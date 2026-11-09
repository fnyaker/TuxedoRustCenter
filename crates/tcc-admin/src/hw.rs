// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Service D-Bus `Admin.Hw` phase 2 (AD-12 B2, P2-01…P2-07).
//!
//! Ce service séparé porte **uniquement** les limites matérielles
//! transitoires : TDP (PL1/PL2/PL4), profil ODM et offset cTGP NVIDIA. Il ne
//! lit ni n'écrit de configuration, n'exécute aucune commande externe et
//! n'ouvre `/dev/tuxedo_io` qu'à la demande, dans une méthode explicite.
//!
//! # Sécurité
//!
//! Ordre appliqué à chaque écriture :
//! identité → validation syntaxique → rate limit → polkit → `Semaphore(1)` →
//! backend → vérification → rollback best-effort → audit. Le service est
//! activé à la demande par D-Bus, exige root et sort après 30 s d'inactivité.
//!
//! Aucun chemin, device, variable d'environnement ou commande ne traverse
//! l'interface : les seules entrées sont `ai`, `i` et une chaîne bornée.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tcc_authz::AuthorizationLimiter;
use tcc_core::{ConfigStore, StrictLoadError};
use tcc_hw::device::TuxedoDevice;
use tcc_hw::error::HwError;
use tcc_hw::nvidia::CtgpControl;
use tcc_hw::platform_profile::{PlatformProfile, ProfileOwnership};
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::tdp::{TdpControl, TdpLimit};
use tcc_hw::tuxedo_io::TuxedoIo;
use tcc_ipc::{
    ADMIN_HW_CONTRACT_VERSION, AdminHwErrorCode, MAX_CTGP_STATIC_OFFSET_WATTS, MAX_HW_STRING_CHARS,
    MAX_TDP_VALUES,
};

use crate::service::{Activity, Authorizer, AuthzDecision, PolkitAuthorizer};

/// Action polkit des écritures thermiques/ODM/cTGP.
pub const ACTION_POWER_LIMITS: &str = "com.tuxedocomputers.tccd1.power-limits";
/// Sortie après cette durée sans appel.
pub const HW_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Débit des écritures : 2 appels/s, burst 4.
pub const HW_WRITE_RATE_PER_SECOND: f64 = 2.0;
/// Burst des écritures.
pub const HW_WRITE_BURST: f64 = 4.0;
/// Débit des lectures TDP : 2 appels/s, burst 5.
pub const HW_READ_RATE_PER_SECOND: f64 = 2.0;
/// Burst des lectures TDP.
pub const HW_READ_BURST: f64 = 5.0;
/// Timeout de résolution de l'UID appelant.
const UID_TIMEOUT: Duration = Duration::from_millis(500);
/// Bornes de syntaxe TDP (les bornes firmware restent la première barrière).
const TDP_MIN_WATTS: i32 = 1;
const TDP_MAX_WATTS: i32 = 1000;

/// Erreur D-Bus du service `Admin.Hw`.
#[derive(Debug, Clone, PartialEq, Eq, zbus::DBusError)]
#[zbus(prefix = "org.tuxedocomputers.tccd.AdminHw1.Error")]
pub enum AdminHwError {
    /// polkit a refusé, est absent, a expiré ou l'identité est inconnue.
    NotAuthorized,
    /// Entrée syntaxiquement invalide (`tdp.count`, `odm.name`, …).
    InvalidArgs(String),
    /// Valeur refusée par la validation ou les bornes firmware.
    Validation(String),
    /// PPD possède `platform_profile`, ou l'override n'est pas consenti.
    PolicyDenied(String),
    /// Matériel ou fonctionnalité absente.
    NotSupported(String),
    /// Écriture ou vérification en échec.
    WriteFailed,
    /// État matériel intermédiaire non restauré (descripteurs bornés).
    PartialApply(String),
    /// Limite de débit atteinte.
    RateLimited,
    /// Une écriture ou un dialogue est déjà en cours.
    Busy,
}

impl AdminHwError {
    /// Libellé stable pour l'audit.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NotAuthorized => "not_authorized",
            Self::InvalidArgs(_) => "invalid_args",
            Self::Validation(_) => "validation",
            Self::PolicyDenied(_) => "policy_denied",
            Self::NotSupported(_) => "not_supported",
            Self::WriteFailed => "write_failed",
            Self::PartialApply(_) => "partial_apply",
            Self::RateLimited => "rate_limited",
            Self::Busy => "busy",
        }
    }

    /// Code de contrat correspondant.
    #[must_use]
    pub const fn code(&self) -> AdminHwErrorCode {
        match self {
            Self::NotAuthorized => AdminHwErrorCode::NotAuthorized,
            Self::InvalidArgs(_) => AdminHwErrorCode::InvalidArgs,
            Self::Validation(_) => AdminHwErrorCode::Validation,
            Self::PolicyDenied(_) => AdminHwErrorCode::PolicyDenied,
            Self::NotSupported(_) => AdminHwErrorCode::NotSupported,
            Self::WriteFailed => AdminHwErrorCode::WriteFailed,
            Self::PartialApply(_) => AdminHwErrorCode::PartialApply,
            Self::RateLimited => AdminHwErrorCode::RateLimited,
            Self::Busy => AdminHwErrorCode::Busy,
        }
    }
}

/// Propriété de `platform_profile` par power-profiles-daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpdOwnership {
    /// PPD possède le nom : TCC ne doit pas écrire sans override.
    Owned,
    /// PPD est installé mais endormi.
    Activatable,
    /// PPD absent.
    Absent,
    /// Bus en erreur : traité comme `Owned` (fail-closed).
    Unknown,
}

impl PpdOwnership {
    /// Libellé stable pour l'audit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Activatable => "activatable",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }

    /// Vrai si une écriture `platform_profile` sans override est interdite.
    #[must_use]
    pub const fn blocks_platform_write(self) -> bool {
        matches!(self, Self::Owned | Self::Unknown)
    }
}

/// Backend TDP injectable.
pub trait TdpBackend: Send + Sync {
    /// Limites courantes, bornes firmware incluses.
    fn limits(&self) -> tcc_hw::Result<Vec<TdpLimit>>;
    /// Applique les valeurs (validation firmware incluse).
    fn set_values(&self, values: &[f64]) -> tcc_hw::Result<Vec<TdpLimit>>;
}

/// Vue ODM détectée (source `platform_profile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdmPlatform {
    /// Vrai si un `platform_profile` exploitable est exposé.
    pub available: bool,
    /// Profil courant.
    pub current: Option<String>,
    /// Profils disponibles (allowlist runtime).
    pub choices: Vec<String>,
}

/// Backend ODM injectable.
pub trait OdmBackend: Send + Sync {
    /// Détecte `platform_profile` (source TUXEDO ou ACPI) ; `None` si absent.
    fn platform(&self) -> tcc_hw::Result<Option<OdmPlatform>>;
    /// Profils ODM du module `tuxedo_io`.
    fn io_profiles(&self) -> tcc_hw::Result<Vec<String>>;
    /// Écrit `platform_profile`.
    fn apply_platform(&self, name: &str) -> tcc_hw::Result<()>;
    /// Écrit le profil ODM via `tuxedo_io`.
    fn apply_io(&self, name: &str) -> tcc_hw::Result<()>;
}

/// Backend cTGP injectable.
pub trait CtgpBackend: Send + Sync {
    /// Offset courant, `None` si l'attribut est absent.
    fn offset(&self) -> tcc_hw::Result<Option<i32>>;
    /// Applique un offset en watts.
    fn set_offset(&self, watts: i32) -> tcc_hw::Result<()>;
    /// Borne dynamique, `None` si inconnue (repli statique).
    fn max_offset(&self) -> tcc_hw::Result<Option<i32>>;
}

/// Fournisseur de borne dynamique cTGP (instantané `tccd`).
#[async_trait]
pub trait CtgpBounds: Send + Sync {
    /// Offset maximal en watts, `None` si indisponible (repli statique).
    async fn max_offset(&self) -> Option<i32>;
}

/// Fournisseur statique : aucune borne dynamique.
#[derive(Debug, Default, Clone, Copy)]
struct StaticCtgpBounds;

#[async_trait]
impl CtgpBounds for StaticCtgpBounds {
    async fn max_offset(&self) -> Option<i32> {
        None
    }
}

/// Lit la borne dynamique depuis `Manager1.GetHardwareSnapshotJSON`.
///
/// Timeout court (2 s), JSON borné à 4 Mio, aucun `nvidia-smi` exécuté. En cas
/// d'absence de `tccd`, de timeout ou de champ manquant, le service retombe sur
/// la borne statique 200 W.
#[derive(Debug, Default, Clone, Copy)]
pub struct TccdCtgpBounds;

#[async_trait]
impl CtgpBounds for TccdCtgpBounds {
    async fn max_offset(&self) -> Option<i32> {
        const TIMEOUT: Duration = Duration::from_secs(2);
        const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
        let connection = tokio::time::timeout(TIMEOUT, zbus::Connection::system())
            .await
            .ok()?
            .ok()?;
        let proxy = tokio::time::timeout(
            TIMEOUT,
            zbus::Proxy::new(
                &connection,
                tcc_ipc::DBUS_SERVICE,
                tcc_ipc::DBUS_PATH,
                tcc_ipc::DBUS_MANAGER_INTERFACE,
            ),
        )
        .await
        .ok()?
        .ok()?;
        let json: String =
            tokio::time::timeout(TIMEOUT, proxy.call("GetHardwareSnapshotJSON", &()))
                .await
                .ok()?
                .ok()?;
        if json.len() > MAX_SNAPSHOT_BYTES {
            return None;
        }
        let snapshot: serde_json::Value = serde_json::from_str(&json).ok()?;
        let nvidia = snapshot.get("nvidia")?;
        let default = nvidia.get("defaultPowerLimit")?.as_f64()?;
        let max = nvidia.get("maxPowerLimit")?.as_f64()?;
        let offset = (max - default).floor();
        if !offset.is_finite() {
            return None;
        }
        Some(offset.clamp(0.0, f64::from(MAX_CTGP_STATIC_OFFSET_WATTS)) as i32)
    }
}

/// Sonde PPD injectable.
#[async_trait]
pub trait PpdProbe: Send + Sync {
    /// Propriété courante de `platform_profile`.
    async fn ownership(&self) -> PpdOwnership;
}

/// Consentement persistant pour l'override `platform_profile`.
pub trait OverrideConsent: Send + Sync {
    /// Vrai si `Settings.ppd_override` est vrai sur disque.
    fn enabled(&self) -> bool;
}

/// Backend TDP de production : ouvre `/dev/tuxedo_io` à chaque appel.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealTdpBackend;

impl TdpBackend for RealTdpBackend {
    fn limits(&self) -> tcc_hw::Result<Vec<TdpLimit>> {
        let io = Arc::new(TuxedoIo::open()?);
        TdpControl::new(io).limits()
    }

    fn set_values(&self, values: &[f64]) -> tcc_hw::Result<Vec<TdpLimit>> {
        let io = Arc::new(TuxedoIo::open()?);
        TdpControl::new(io).set_values(values)
    }
}

/// Backend ODM de production (sysfs + fallback ioctl).
#[derive(Debug, Clone)]
pub struct RealOdmBackend {
    root: SysfsRoot,
}

impl RealOdmBackend {
    /// Construit le backend sur une racine sysfs.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }
}

impl OdmBackend for RealOdmBackend {
    fn platform(&self) -> tcc_hw::Result<Option<OdmPlatform>> {
        let device = TuxedoDevice::detect(&self.root, None);
        let profile = PlatformProfile::detect(&self.root, device)?;
        Ok(Some(OdmPlatform {
            available: profile.is_available(),
            current: profile.current().map(str::to_owned),
            choices: profile.choices().to_vec(),
        }))
    }

    fn io_profiles(&self) -> tcc_hw::Result<Vec<String>> {
        TuxedoIo::open()?.odm_profiles()
    }

    fn apply_platform(&self, name: &str) -> tcc_hw::Result<()> {
        let device = TuxedoDevice::detect(&self.root, None);
        let profile = PlatformProfile::detect(&self.root, device)?;
        profile.apply(&self.root, name, ProfileOwnership::TccOwned)
    }

    fn apply_io(&self, name: &str) -> tcc_hw::Result<()> {
        TuxedoIo::open()?.set_odm_profile(name)
    }
}

/// Backend cTGP de production.
///
/// `max_offset` renvoie volontairement `None` : `Admin.Hw` n'exécute **aucune
/// commande externe** (`nvidia-smi`), sa borne dynamique viendra plus tard
/// d'un instantané `tccd`. La borne statique de sécurité s'applique.
#[derive(Debug, Clone)]
pub struct RealCtgpBackend {
    root: SysfsRoot,
}

impl RealCtgpBackend {
    /// Construit le backend sur une racine sysfs.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }
}

impl CtgpBackend for RealCtgpBackend {
    fn offset(&self) -> tcc_hw::Result<Option<i32>> {
        CtgpControl::new(self.root.clone()).offset()
    }

    fn set_offset(&self, watts: i32) -> tcc_hw::Result<()> {
        CtgpControl::new(self.root.clone()).set_offset_watts(f64::from(watts))
    }

    fn max_offset(&self) -> tcc_hw::Result<Option<i32>> {
        Ok(None)
    }
}

/// Sonde PPD réelle sur le bus système.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZbusPpdProbe;

#[async_trait]
impl PpdProbe for ZbusPpdProbe {
    async fn ownership(&self) -> PpdOwnership {
        const TIMEOUT: Duration = Duration::from_secs(1);
        let name = match zbus::names::BusName::try_from("net.hadess.PowerProfiles".to_owned()) {
            Ok(name) => name,
            Err(_) => return PpdOwnership::Unknown,
        };
        let connection = match tokio::time::timeout(TIMEOUT, zbus::Connection::system()).await {
            Ok(Ok(connection)) => connection,
            _ => return PpdOwnership::Unknown,
        };
        let proxy =
            match tokio::time::timeout(TIMEOUT, zbus::fdo::DBusProxy::new(&connection)).await {
                Ok(Ok(proxy)) => proxy,
                _ => return PpdOwnership::Unknown,
            };
        match tokio::time::timeout(TIMEOUT, proxy.name_has_owner(name.clone())).await {
            Ok(Ok(true)) => PpdOwnership::Owned,
            Ok(Ok(false)) => {
                match tokio::time::timeout(TIMEOUT, proxy.list_activatable_names()).await {
                    Ok(Ok(names))
                        if names
                            .iter()
                            .any(|candidate| candidate.as_str() == "net.hadess.PowerProfiles") =>
                    {
                        PpdOwnership::Activatable
                    }
                    Ok(Ok(_)) => PpdOwnership::Absent,
                    _ => PpdOwnership::Unknown,
                }
            }
            _ => PpdOwnership::Unknown,
        }
    }
}

/// Consentement lu depuis `/var/lib` à chaque appel.
#[derive(Debug, Clone)]
pub struct SystemOverrideConsent {
    store: ConfigStore,
}

impl SystemOverrideConsent {
    /// Construit le lecteur sur le magasin système.
    #[must_use]
    pub fn new(store: ConfigStore) -> Self {
        Self { store }
    }

    /// Variante de production.
    #[must_use]
    pub fn system() -> Self {
        Self::new(ConfigStore::system())
    }
}

impl OverrideConsent for SystemOverrideConsent {
    fn enabled(&self) -> bool {
        match self.store.load_settings_no_recovery() {
            Ok(settings) => settings.ppd_override,
            Err(StrictLoadError::NotFound { .. }) => false,
            Err(error) => {
                tracing::warn!(%error, "réglages illisibles : override PPD refusé");
                false
            }
        }
    }
}

/// Contexte d'un appel pour l'audit.
struct HwCallContext {
    method: &'static str,
    caller: String,
    uid: u32,
    started: Instant,
}

/// Service `Admin.Hw`.
pub struct AdminHwService {
    tdp: Arc<dyn TdpBackend>,
    odm: Arc<dyn OdmBackend>,
    ctgp: Arc<dyn CtgpBackend>,
    bounds: Arc<dyn CtgpBounds>,
    ppd: Arc<dyn PpdProbe>,
    consent: Arc<dyn OverrideConsent>,
    authorizer: Arc<dyn Authorizer>,
    limiter: AuthorizationLimiter,
    read_limiter: AuthorizationLimiter,
    write_gate: Arc<tokio::sync::Semaphore>,
    activity: Arc<Activity>,
}

impl std::fmt::Debug for AdminHwService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdminHwService")
            .finish_non_exhaustive()
    }
}

impl AdminHwService {
    /// Construit le service avec des backends injectés (tests et production).
    #[must_use]
    pub fn new(
        tdp: Arc<dyn TdpBackend>,
        odm: Arc<dyn OdmBackend>,
        ctgp: Arc<dyn CtgpBackend>,
        ppd: Arc<dyn PpdProbe>,
        consent: Arc<dyn OverrideConsent>,
        authorizer: Arc<dyn Authorizer>,
        activity: Arc<Activity>,
    ) -> Self {
        Self {
            tdp,
            odm,
            ctgp,
            bounds: Arc::new(StaticCtgpBounds),
            ppd,
            consent,
            authorizer,
            limiter: AuthorizationLimiter::new(HW_WRITE_RATE_PER_SECOND, HW_WRITE_BURST),
            read_limiter: AuthorizationLimiter::new(HW_READ_RATE_PER_SECOND, HW_READ_BURST),
            write_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            activity,
        }
    }

    /// Remplace le fournisseur de borne cTGP (tests, futures sources).
    #[must_use]
    pub fn with_ctgp_bounds(mut self, bounds: Arc<dyn CtgpBounds>) -> Self {
        self.bounds = bounds;
        self
    }

    /// Service de production : backends réels, polkit, `/var/lib` en lecture.
    #[must_use]
    pub fn production(activity: Arc<Activity>) -> Self {
        let root = SysfsRoot::system();
        Self::new(
            Arc::new(RealTdpBackend),
            Arc::new(RealOdmBackend::new(root.clone())),
            Arc::new(RealCtgpBackend::new(root)),
            Arc::new(ZbusPpdProbe),
            Arc::new(SystemOverrideConsent::system()),
            Arc::new(PolkitAuthorizer::new()),
            activity,
        )
        .with_ctgp_bounds(Arc::new(TccdCtgpBounds))
    }

    /// Horloge d'activité partagée avec la supervision d'inactivité.
    #[must_use]
    pub fn activity(&self) -> Arc<Activity> {
        Arc::clone(&self.activity)
    }

    async fn caller_identity(
        &self,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
    ) -> Result<(String, u32), AdminHwError> {
        let sender = header.sender().ok_or(AdminHwError::NotAuthorized)?;
        let caller = sender.to_string();
        let bus_name = zbus::names::BusName::try_from(sender.as_str())
            .map_err(|_| AdminHwError::NotAuthorized)?;
        let proxy = tokio::time::timeout(UID_TIMEOUT, zbus::fdo::DBusProxy::new(connection))
            .await
            .map_err(|_| AdminHwError::NotAuthorized)?
            .map_err(|_| AdminHwError::NotAuthorized)?;
        let uid = tokio::time::timeout(UID_TIMEOUT, proxy.get_connection_unix_user(bus_name))
            .await
            .map_err(|_| AdminHwError::NotAuthorized)?
            .map_err(|_| AdminHwError::NotAuthorized)?;
        Ok((caller, uid))
    }

    async fn identity_or_audit(
        &self,
        method: &'static str,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
    ) -> Result<(String, u32), AdminHwError> {
        match self.caller_identity(header, connection).await {
            Ok(identity) => Ok(identity),
            Err(error) => {
                tracing::warn!(
                    method,
                    action = ACTION_POWER_LIMITS,
                    caller = "unknown",
                    result = error.label(),
                    "appel Admin.Hw refusé"
                );
                Err(error)
            }
        }
    }

    fn consume_write_token(&self, context: &HwCallContext) -> Result<(), AdminHwError> {
        if self
            .limiter
            .allow(&context.caller, context.uid, Instant::now())
        {
            Ok(())
        } else {
            self.audit(context, Some(&AdminHwError::RateLimited));
            Err(AdminHwError::RateLimited)
        }
    }

    async fn authorize_polkit(&self, context: &HwCallContext) -> Result<(), AdminHwError> {
        match self
            .authorizer
            .authorize(&context.caller, context.uid, ACTION_POWER_LIMITS)
            .await
        {
            AuthzDecision::Authorized => {
                self.activity.touch();
                Ok(())
            }
            AuthzDecision::Busy => {
                tracing::warn!(
                    method = context.method,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = "busy",
                    "dialogue d'autorisation déjà en cours"
                );
                Err(AdminHwError::Busy)
            }
            AuthzDecision::Denied(reason) => {
                tracing::warn!(
                    method = context.method,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = ?reason,
                    "autorisation refusée"
                );
                Err(AdminHwError::NotAuthorized)
            }
            AuthzDecision::Unavailable => {
                tracing::error!(
                    method = context.method,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = "polkit_unavailable",
                    "polkit indisponible : refus fail-closed"
                );
                Err(AdminHwError::NotAuthorized)
            }
        }
    }

    async fn authorize_write(&self, context: &HwCallContext) -> Result<(), AdminHwError> {
        self.consume_write_token(context)?;
        self.authorize_polkit(context).await
    }

    async fn override_consent(&self) -> bool {
        let consent = Arc::clone(&self.consent);
        tokio::task::spawn_blocking(move || consent.enabled())
            .await
            .unwrap_or(false)
    }

    async fn run_blocking<T, F>(&self, operation: F) -> Result<tcc_hw::Result<T>, AdminHwError>
    where
        T: Send + 'static,
        F: FnOnce() -> tcc_hw::Result<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(operation)
            .await
            .map_err(|_| AdminHwError::WriteFailed)
    }

    async fn tdp_limits(&self) -> Result<Vec<TdpLimit>, AdminHwError> {
        let backend = Arc::clone(&self.tdp);
        self.run_blocking(move || backend.limits())
            .await?
            .map_err(map_tdp_error)
    }

    async fn tdp_set(&self, values: Vec<f64>) -> Result<Vec<TdpLimit>, AdminHwError> {
        let backend = Arc::clone(&self.tdp);
        self.run_blocking(move || backend.set_values(&values))
            .await?
            .map_err(map_tdp_error)
    }

    async fn odm_platform(&self) -> Result<Option<OdmPlatform>, AdminHwError> {
        let backend = Arc::clone(&self.odm);
        match self.run_blocking(move || backend.platform()).await? {
            Ok(platform) => Ok(platform),
            Err(error) if error.is_unavailable() => Ok(None),
            Err(_) => Err(AdminHwError::WriteFailed),
        }
    }

    async fn odm_io_profiles(&self) -> Result<Vec<String>, AdminHwError> {
        let backend = Arc::clone(&self.odm);
        match self.run_blocking(move || backend.io_profiles()).await? {
            Ok(profiles) => Ok(profiles),
            Err(error) if error.is_unavailable() => Ok(Vec::new()),
            Err(_) => Err(AdminHwError::WriteFailed),
        }
    }

    async fn ctgp_offset(&self) -> Result<Option<i32>, AdminHwError> {
        let backend = Arc::clone(&self.ctgp);
        self.run_blocking(move || backend.offset())
            .await?
            .map_err(map_ctgp_error)
    }

    async fn ctgp_set(&self, watts: i32) -> Result<(), AdminHwError> {
        let backend = Arc::clone(&self.ctgp);
        self.run_blocking(move || backend.set_offset(watts))
            .await?
            .map_err(map_ctgp_error)
    }

    async fn ctgp_max(&self) -> (i32, &'static str) {
        let backend = Arc::clone(&self.ctgp);
        if let Ok(Ok(Some(max))) = self.run_blocking(move || backend.max_offset()).await {
            return (max.clamp(0, MAX_CTGP_STATIC_OFFSET_WATTS), "dynamic");
        }
        match self.bounds.max_offset().await {
            Some(max) => (max.clamp(0, MAX_CTGP_STATIC_OFFSET_WATTS), "dynamic"),
            None => (MAX_CTGP_STATIC_OFFSET_WATTS, "static"),
        }
    }

    async fn set_tdp(
        &self,
        context: HwCallContext,
        values: Vec<i32>,
    ) -> Result<bool, AdminHwError> {
        // `[]` est une consigne explicite : appliquer les maxima firmware
        // (comportement original / `TdpControl::set_values(&[])`). Seule la
        // borne haute du nombre de valeurs est refusée ici.
        if values.len() > MAX_TDP_VALUES {
            let error = AdminHwError::InvalidArgs("tdp.count".to_owned());
            self.audit(&context, Some(&error));
            return Err(error);
        }
        if values
            .iter()
            .any(|value| !(TDP_MIN_WATTS..=TDP_MAX_WATTS).contains(value))
        {
            let error = AdminHwError::Validation("tdp.values".to_owned());
            self.audit(&context, Some(&error));
            return Err(error);
        }
        self.authorize_write(&context).await?;
        let permit = match self.write_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.audit(&context, Some(&AdminHwError::Busy));
                return Err(AdminHwError::Busy);
            }
        };
        let result = self.set_tdp_locked(&context, &values).await;
        drop(permit);
        self.audit(&context, result.as_ref().err());
        result
    }

    async fn set_tdp_locked(
        &self,
        context: &HwCallContext,
        values: &[i32],
    ) -> Result<bool, AdminHwError> {
        let limits = self.tdp_limits().await?;
        if limits.is_empty() {
            return Err(AdminHwError::NotSupported("tdp".to_owned()));
        }
        if values.len() > limits.len() {
            return Err(AdminHwError::Validation("tdp.values".to_owned()));
        }
        let previous: Vec<i32> = limits.iter().map(|limit| limit.current).collect();
        // `[]` cible les maxima du firmware, comme `TdpControl::set_values`.
        let maxima = values.is_empty();
        let target: Vec<i32> = if maxima {
            limits.iter().map(|limit| limit.max).collect()
        } else {
            values.to_vec()
        };
        let noop = if maxima {
            previous == target
        } else {
            target
                .iter()
                .enumerate()
                .all(|(index, value)| previous.get(index) == Some(value))
        };
        if noop {
            tracing::info!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                duration_ms = context.started.elapsed().as_millis() as u64,
                caller_uid = context.uid,
                result = "ok",
                count = target.len(),
                values = if maxima { "empty" } else { "set" },
                maxima,
                noop = true,
                "appel Admin.Hw"
            );
            return Ok(true);
        }

        let float_values: Vec<f64> = if maxima {
            Vec::new()
        } else {
            target.iter().map(|value| f64::from(*value)).collect()
        };
        if let Err(error) = self.tdp_set(float_values).await {
            if matches!(
                error,
                AdminHwError::Validation(_) | AdminHwError::NotSupported(_)
            ) {
                return Err(error);
            }
            return self.rollback_tdp(context, &previous, &limits).await;
        }

        let after = self.tdp_limits().await?;
        let applied = if maxima {
            after.iter().all(|limit| limit.current == limit.max)
        } else {
            target
                .iter()
                .enumerate()
                .all(|(index, value)| after.get(index).map(|limit| limit.current) == Some(*value))
        };
        if applied {
            let descriptors = if maxima {
                descriptors(limits.len(), &limits)
            } else {
                descriptors(target.len(), &limits)
            };
            tracing::info!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                duration_ms = context.started.elapsed().as_millis() as u64,
                caller_uid = context.uid,
                result = "ok",
                count = target.len(),
                values = if maxima { "empty" } else { "set" },
                maxima,
                applied = %descriptors,
                noop = false,
                restore = "none",
                "appel Admin.Hw"
            );
            return Ok(true);
        }

        self.rollback_tdp(context, &previous, &limits).await
    }

    async fn rollback_tdp(
        &self,
        context: &HwCallContext,
        previous: &[i32],
        limits: &[TdpLimit],
    ) -> Result<bool, AdminHwError> {
        let float_previous: Vec<f64> = previous.iter().map(|value| f64::from(*value)).collect();
        let restore_ok = self.tdp_set(float_previous).await.is_ok();
        let after = self.tdp_limits().await;
        let restored = after
            .as_ref()
            .map(|current| {
                current
                    .iter()
                    .zip(previous.iter())
                    .all(|(limit, value)| limit.current == *value)
            })
            .unwrap_or(false);
        if restore_ok && restored {
            tracing::warn!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                caller_uid = context.uid,
                duration_ms = context.started.elapsed().as_millis() as u64,
                result = "write_failed",
                restore = "ok",
                "rollback TDP"
            );
            return Err(AdminHwError::WriteFailed);
        }
        let failed = after
            .as_ref()
            .map(|current| failed_descriptors(limits, current))
            .unwrap_or_else(|_| descriptors(limits.len(), limits));
        tracing::warn!(
            method = context.method,
            action = ACTION_POWER_LIMITS,
            polkit = ACTION_POWER_LIMITS,
            caller = %context.caller,
            caller_uid = context.uid,
            duration_ms = context.started.elapsed().as_millis() as u64,
            result = "partial_apply",
            restore = "failed",
            failed = %failed,
            "rollback TDP"
        );
        Err(AdminHwError::PartialApply(failed))
    }

    async fn get_tdp(&self, context: HwCallContext) -> Result<Vec<i32>, AdminHwError> {
        if !self
            .read_limiter
            .allow(&context.caller, context.uid, Instant::now())
        {
            self.audit(&context, Some(&AdminHwError::RateLimited));
            return Err(AdminHwError::RateLimited);
        }
        let limits = match self.tdp_limits().await {
            Ok(limits) => limits,
            Err(error) => {
                self.audit(&context, Some(&error));
                return Err(error);
            }
        };
        if limits.is_empty() {
            let error = AdminHwError::NotSupported("tdp".to_owned());
            self.audit(&context, Some(&error));
            return Err(error);
        }
        let values: Vec<i32> = limits.iter().map(|limit| limit.current).collect();
        tracing::info!(
            method = context.method,
            action = ACTION_POWER_LIMITS,
            polkit = ACTION_POWER_LIMITS,
            caller = %context.caller,
            duration_ms = context.started.elapsed().as_millis() as u64,
            caller_uid = context.uid,
            result = "ok",
            count = values.len(),
            "appel Admin.Hw"
        );
        Ok(values)
    }

    async fn set_odm(
        &self,
        context: HwCallContext,
        name: String,
        override_mode: bool,
    ) -> Result<bool, AdminHwError> {
        if let Err(error) = validate_odm_name(&name) {
            self.audit(&context, Some(&error));
            return Err(error);
        }
        // AH-06 : le rate limit reste avant le consentement et polkit. Le
        // consentement persistant est bon marché et lisible avant le dialogue ;
        // un refus évite une authentification inutile.
        if override_mode {
            self.consume_write_token(&context)?;
            if !self.override_consent().await {
                let error = AdminHwError::PolicyDenied("ppd.override_disabled".to_owned());
                self.audit(&context, Some(&error));
                return Err(error);
            }
            self.authorize_polkit(&context).await?;
        } else {
            self.authorize_write(&context).await?;
        }
        let permit = match self.write_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.audit(&context, Some(&AdminHwError::Busy));
                return Err(AdminHwError::Busy);
            }
        };
        let result = self.set_odm_locked(&context, &name, override_mode).await;
        drop(permit);
        self.audit(&context, result.as_ref().err());
        result
    }

    async fn set_odm_locked(
        &self,
        context: &HwCallContext,
        name: &str,
        override_mode: bool,
    ) -> Result<bool, AdminHwError> {
        let platform = self.odm_platform().await?;
        let io_profiles = self.odm_io_profiles().await?;
        let previous_platform = platform
            .as_ref()
            .and_then(|profile| profile.current.clone())
            .filter(|current| !current.is_empty());
        let choices: Vec<String> = platform
            .as_ref()
            .filter(|profile| profile.available)
            .map(|profile| profile.choices.clone())
            .unwrap_or_default();
        let in_platform = choices.iter().any(|candidate| candidate == name);
        let in_io = io_profiles.iter().any(|candidate| candidate == name);
        if !in_platform && !in_io {
            return Err(AdminHwError::Validation("odm.name".to_owned()));
        }

        let ppd = self.ppd.ownership().await;
        if in_platform {
            if !override_mode && ppd.blocks_platform_write() {
                return Err(AdminHwError::PolicyDenied(
                    "ppd.platform_profile".to_owned(),
                ));
            }
            if override_mode && ppd == PpdOwnership::Unknown {
                return Err(AdminHwError::PolicyDenied(
                    "ppd.platform_profile".to_owned(),
                ));
            }
            let backend = Arc::clone(&self.odm);
            let target = name.to_owned();
            self.run_blocking(move || backend.apply_platform(&target))
                .await?
                .map_err(|error| map_odm_error(error, "platform_profile"))?;
            let after = self.odm_platform().await?;
            let verified = after
                .as_ref()
                .and_then(|profile| profile.current.as_deref())
                .is_some_and(|current| current == name);
            if !verified {
                // AH-04 : restaurer l'ancien profil connu, puis vérifier.
                let Some(previous) = previous_platform else {
                    return Err(AdminHwError::WriteFailed);
                };
                let backend = Arc::clone(&self.odm);
                let previous_for_write = previous.clone();
                let restored = match self
                    .run_blocking(move || backend.apply_platform(&previous_for_write))
                    .await
                {
                    Ok(Ok(())) => self
                        .odm_platform()
                        .await
                        .ok()
                        .flatten()
                        .and_then(|profile| profile.current)
                        .is_some_and(|current| current == previous),
                    _ => false,
                };
                if restored {
                    return Err(AdminHwError::WriteFailed);
                }
                return Err(AdminHwError::PartialApply("platform_profile".to_owned()));
            }
            let ppd_after = self.ppd.ownership().await;
            let ppd_race = ppd_after == PpdOwnership::Owned && ppd != PpdOwnership::Owned;
            tracing::info!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                duration_ms = context.started.elapsed().as_millis() as u64,
                caller_uid = context.uid,
                result = "ok",
                source = "platform",
                name = %name,
                ppd = ppd.as_str(),
                ppd_race,
                override_mode,
                "appel Admin.Hw"
            );
            return Ok(true);
        }

        // AH-02 : PPD ne possède que `platform_profile`. Les profils ODM du
        // module `tuxedo_io` ne sont pas gérés par PPD : le fallback reste
        // autorisé même si PPD est `Owned` ou `Unknown`. L'audit ci-dessous
        // conserve `source=io` et `ppd=owned|unknown` pour le rendre explicite.
        let backend = Arc::clone(&self.odm);
        let target = name.to_owned();
        self.run_blocking(move || backend.apply_io(&target))
            .await?
            .map_err(|error| map_odm_error(error, "platform_profile"))?;
        tracing::info!(
            method = context.method,
            action = ACTION_POWER_LIMITS,
            polkit = ACTION_POWER_LIMITS,
            caller = %context.caller,
            duration_ms = context.started.elapsed().as_millis() as u64,
            caller_uid = context.uid,
            result = "ok",
            source = "io",
            name = %name,
            ppd = ppd.as_str(),
            override_mode,
            verify = "none",
            "appel Admin.Hw"
        );
        Ok(true)
    }

    async fn set_ctgp(&self, context: HwCallContext, watts: i32) -> Result<bool, AdminHwError> {
        if !(0..=MAX_CTGP_STATIC_OFFSET_WATTS).contains(&watts) {
            let error = AdminHwError::Validation("nvidia.ctgpOffset".to_owned());
            self.audit(&context, Some(&error));
            return Err(error);
        }
        self.authorize_write(&context).await?;
        let permit = match self.write_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.audit(&context, Some(&AdminHwError::Busy));
                return Err(AdminHwError::Busy);
            }
        };
        let result = self.set_ctgp_locked(&context, watts).await;
        drop(permit);
        self.audit(&context, result.as_ref().err());
        result
    }

    async fn set_ctgp_locked(
        &self,
        context: &HwCallContext,
        watts: i32,
    ) -> Result<bool, AdminHwError> {
        let (max, bound) = self.ctgp_max().await;
        if watts > max {
            return Err(AdminHwError::Validation("nvidia.ctgpOffset".to_owned()));
        }
        let current = self.ctgp_offset().await?;
        let Some(previous) = current else {
            return Err(AdminHwError::NotSupported("ctgp_offset".to_owned()));
        };
        if previous == watts {
            tracing::info!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                duration_ms = context.started.elapsed().as_millis() as u64,
                caller_uid = context.uid,
                result = "ok",
                bound,
                noop = true,
                "appel Admin.Hw"
            );
            return Ok(true);
        }
        match self.ctgp_set(watts).await {
            Ok(()) => {}
            Err(error @ AdminHwError::WriteFailed) => {
                let _ = self.ctgp_set(previous).await;
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        let after = self.ctgp_offset().await?;
        if after == Some(watts) {
            tracing::info!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                polkit = ACTION_POWER_LIMITS,
                caller = %context.caller,
                duration_ms = context.started.elapsed().as_millis() as u64,
                caller_uid = context.uid,
                result = "ok",
                bound,
                noop = false,
                "appel Admin.Hw"
            );
            return Ok(true);
        }

        // Best-effort : restaurer l'ancienne valeur une seule fois.
        let _ = self.ctgp_set(previous).await;
        Err(AdminHwError::WriteFailed)
    }

    fn audit(&self, context: &HwCallContext, error: Option<&AdminHwError>) {
        if let Some(error) = error {
            let reason = match error {
                AdminHwError::InvalidArgs(reason)
                | AdminHwError::Validation(reason)
                | AdminHwError::PolicyDenied(reason)
                | AdminHwError::NotSupported(reason)
                | AdminHwError::PartialApply(reason) => reason.as_str(),
                AdminHwError::NotAuthorized
                | AdminHwError::WriteFailed
                | AdminHwError::RateLimited
                | AdminHwError::Busy => "",
            };
            tracing::warn!(
                method = context.method,
                action = ACTION_POWER_LIMITS,
                caller = %context.caller,
                caller_uid = context.uid,
                polkit = ACTION_POWER_LIMITS,
                result = error.label(),
                reason,
                duration_ms = context.started.elapsed().as_millis() as u64,
                "appel Admin.Hw"
            );
        }
    }
}

fn validate_odm_name(name: &str) -> Result<(), AdminHwError> {
    if name.is_empty()
        || name.chars().count() > MAX_HW_STRING_CHARS
        || !name
            .chars()
            .all(|character| character.is_ascii_graphic() && character != '\\')
    {
        return Err(AdminHwError::InvalidArgs("odm.name".to_owned()));
    }
    Ok(())
}

fn descriptors(count: usize, limits: &[TdpLimit]) -> String {
    limits
        .iter()
        .take(count)
        .map(|limit| limit.descriptor)
        .collect::<Vec<_>>()
        .join(",")
}

fn failed_descriptors(limits: &[TdpLimit], current: &[TdpLimit]) -> String {
    limits
        .iter()
        .zip(current.iter())
        .filter(|(before, after)| before.current != after.current)
        .map(|(before, _)| before.descriptor)
        .collect::<Vec<_>>()
        .join(",")
}

fn map_tdp_error(error: HwError) -> AdminHwError {
    match error {
        error if error.is_unavailable() => AdminHwError::NotSupported("tdp".to_owned()),
        HwError::InvalidValue { .. } | HwError::OutOfRange { .. } => {
            AdminHwError::Validation("tdp.values".to_owned())
        }
        _ => AdminHwError::WriteFailed,
    }
}

fn map_odm_error(error: HwError, resource: &'static str) -> AdminHwError {
    match error {
        error if error.is_unavailable() => AdminHwError::NotSupported(resource.to_owned()),
        HwError::PolicyDenied { .. } => {
            AdminHwError::PolicyDenied("ppd.platform_profile".to_owned())
        }
        HwError::InvalidValue { .. } | HwError::OutOfRange { .. } => {
            AdminHwError::Validation("odm.name".to_owned())
        }
        _ => AdminHwError::WriteFailed,
    }
}

fn map_ctgp_error(error: HwError) -> AdminHwError {
    match error {
        error if error.is_unavailable() => AdminHwError::NotSupported("ctgp_offset".to_owned()),
        HwError::InvalidValue { .. } | HwError::OutOfRange { .. } => {
            AdminHwError::Validation("nvidia.ctgpOffset".to_owned())
        }
        _ => AdminHwError::WriteFailed,
    }
}

#[zbus::interface(name = "com.tuxedocomputers.tccd.AdminHw1")]
impl AdminHwService {
    /// Version du contrat `AdminHw1` (propriété d'introspection).
    #[zbus(property, name = "ContractVersion")]
    fn contract_version(&self) -> u32 {
        ADMIN_HW_CONTRACT_VERSION
    }

    /// Diagnostic sans effet de bord.
    #[zbus(name = "Ping")]
    async fn ping(&self) -> bool {
        true
    }

    /// Version de l'implémentation.
    #[zbus(name = "GetVersion")]
    async fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Version du contrat `AdminHw1`, voie supportée.
    #[zbus(name = "GetContractVersion")]
    async fn get_contract_version(&self) -> u32 {
        ADMIN_HW_CONTRACT_VERSION
    }

    /// Limites TDP courantes (lecture, sans polkit).
    #[zbus(name = "GetTdpValues")]
    async fn get_tdp_values(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<Vec<i32>, AdminHwError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self
            .identity_or_audit("GetTdpValues", &header, connection)
            .await?;
        self.get_tdp(HwCallContext {
            method: "GetTdpValues",
            caller,
            uid,
            started,
        })
        .await
    }

    /// Applique PL1/PL2/PL4 dans l'ordre du firmware.
    #[zbus(name = "SetTdpValues")]
    async fn set_tdp_values(
        &self,
        values: Vec<i32>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminHwError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self
            .identity_or_audit("SetTdpValues", &header, connection)
            .await?;
        self.set_tdp(
            HwCallContext {
                method: "SetTdpValues",
                caller,
                uid,
                started,
            },
            values,
        )
        .await
    }

    /// Applique un profil ODM (refus si PPD possède `platform_profile`).
    #[zbus(name = "SetOdmProfile")]
    async fn set_odm_profile(
        &self,
        name: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminHwError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self
            .identity_or_audit("SetOdmProfile", &header, connection)
            .await?;
        self.set_odm(
            HwCallContext {
                method: "SetOdmProfile",
                caller,
                uid,
                started,
            },
            name,
            false,
        )
        .await
    }

    /// Applique un profil ODM malgré PPD, si `Settings.ppd_override` est vrai.
    #[zbus(name = "SetOdmProfileOverride")]
    async fn set_odm_profile_override(
        &self,
        name: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminHwError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self
            .identity_or_audit("SetOdmProfileOverride", &header, connection)
            .await?;
        self.set_odm(
            HwCallContext {
                method: "SetOdmProfileOverride",
                caller,
                uid,
                started,
            },
            name,
            true,
        )
        .await
    }

    /// Applique l'offset cTGP NVIDIA en watts.
    #[zbus(name = "SetNvidiaCtgpOffset")]
    async fn set_nvidia_ctgp_offset(
        &self,
        watts: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminHwError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self
            .identity_or_audit("SetNvidiaCtgpOffset", &header, connection)
            .await?;
        self.set_ctgp(
            HwCallContext {
                method: "SetNvidiaCtgpOffset",
                caller,
                uid,
                started,
            },
            watts,
        )
        .await
    }
}

/// Taille maximale du corps D-Bus accepté par `Admin.Hw`.
///
/// Les arguments légitimes tiennent dans quelques dizaines d'octets (3 `i32`
/// ou 64 octets de nom). Ce garde s'applique **avant** la désérialisation des
/// arguments par l'interface : un `ai`/`s` surdimensionné est refusé avec
/// `InvalidArgs("message.size")` sans allouer de `Vec`/`String` secondaire.
///
/// zbus 5 fige la limite de trame socket à 128 Mio et ne l'expose pas ; ce
/// garde applicatif est la barrière disponible avant le dispatch. Voir
/// `docs/DBUS_API.md` pour la limite résiduelle et `MemoryMax`.
pub const MAX_HW_MESSAGE_BODY_BYTES: usize = 64 * 1024;

/// Enveloppe l'interface `AdminHw1` avec un garde de taille de corps.
pub struct BoundedAdminHwInterface(AdminHwService);

impl BoundedAdminHwInterface {
    /// Construit l'enveloppe autour du service.
    #[must_use]
    pub fn new(service: AdminHwService) -> Self {
        Self(service)
    }
}

fn oversized<'call>(
    connection: &'call zbus::Connection,
    msg: &'call zbus::Message,
) -> Option<zbus::object_server::DispatchResult<'call>> {
    if msg.body().len() > MAX_HW_MESSAGE_BODY_BYTES {
        return Some(zbus::object_server::DispatchResult::new_async(
            connection,
            msg,
            async { Err::<(), AdminHwError>(AdminHwError::InvalidArgs("message.size".to_owned())) },
        ));
    }
    None
}

#[async_trait::async_trait]
impl zbus::object_server::Interface for BoundedAdminHwInterface {
    fn name() -> zbus::names::InterfaceName<'static> {
        <AdminHwService as zbus::object_server::Interface>::name()
    }

    fn spawn_tasks_for_methods(&self) -> bool {
        <AdminHwService as zbus::object_server::Interface>::spawn_tasks_for_methods(&self.0)
    }

    async fn get(
        &self,
        property_name: &str,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> Option<zbus::fdo::Result<zbus::zvariant::OwnedValue>> {
        <AdminHwService as zbus::object_server::Interface>::get(
            &self.0,
            property_name,
            server,
            connection,
            header,
            emitter,
        )
        .await
    }

    async fn get_all(
        &self,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> {
        <AdminHwService as zbus::object_server::Interface>::get_all(
            &self.0, server, connection, header, emitter,
        )
        .await
    }

    fn set<'call>(
        &'call self,
        property_name: &'call str,
        value: &'call zbus::zvariant::Value<'_>,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        header: Option<&'call zbus::message::Header<'_>>,
        emitter: &'call zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::object_server::DispatchResult<'call> {
        <AdminHwService as zbus::object_server::Interface>::set(
            &self.0,
            property_name,
            value,
            server,
            connection,
            header,
            emitter,
        )
    }

    async fn set_mut(
        &mut self,
        property_name: &str,
        value: &zbus::zvariant::Value<'_>,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> Option<zbus::fdo::Result<()>> {
        <AdminHwService as zbus::object_server::Interface>::set_mut(
            &mut self.0,
            property_name,
            value,
            server,
            connection,
            header,
            emitter,
        )
        .await
    }

    fn call<'call>(
        &'call self,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        msg: &'call zbus::Message,
        name: zbus::names::MemberName<'call>,
    ) -> zbus::object_server::DispatchResult<'call> {
        if let Some(error) = oversized(connection, msg) {
            return error;
        }
        <AdminHwService as zbus::object_server::Interface>::call(
            &self.0, server, connection, msg, name,
        )
    }

    fn call_mut<'call>(
        &'call mut self,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        msg: &'call zbus::Message,
        name: zbus::names::MemberName<'call>,
    ) -> zbus::object_server::DispatchResult<'call> {
        if let Some(error) = oversized(connection, msg) {
            return error;
        }
        <AdminHwService as zbus::object_server::Interface>::call_mut(
            &mut self.0,
            server,
            connection,
            msg,
            name,
        )
    }

    fn introspect_to_writer(&self, writer: &mut dyn std::fmt::Write, level: usize) {
        <AdminHwService as zbus::object_server::Interface>::introspect_to_writer(
            &self.0, writer, level,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use tcc_authz::DenialReason;
    use tcc_hw::tdp::TDP_DESCRIPTORS;

    use super::*;

    struct AllowAll;
    #[async_trait]
    impl Authorizer for AllowAll {
        async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
            AuthzDecision::Authorized
        }
    }

    struct DenyAll;
    #[async_trait]
    impl Authorizer for DenyAll {
        async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
            AuthzDecision::Denied(DenialReason::PolicyDenied)
        }
    }

    #[derive(Debug)]
    struct FakeTdp {
        limits: Mutex<Vec<TdpLimit>>,
        writes: Mutex<Vec<Vec<f64>>>,
        fail_after: Option<usize>,
        rollback_fails: bool,
    }

    impl FakeTdp {
        fn new(values: &[(i32, i32, i32)]) -> Self {
            Self {
                limits: Mutex::new(
                    values
                        .iter()
                        .enumerate()
                        .map(|(index, (current, min, max))| TdpLimit {
                            descriptor: TDP_DESCRIPTORS[index],
                            current: *current,
                            min: *min,
                            max: *max,
                        })
                        .collect(),
                ),
                writes: Mutex::new(Vec::new()),
                fail_after: None,
                rollback_fails: false,
            }
        }

        fn with_failure(mut self, after: usize, rollback_fails: bool) -> Self {
            self.fail_after = Some(after);
            self.rollback_fails = rollback_fails;
            self
        }

        fn writes(&self) -> Vec<Vec<f64>> {
            self.writes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }

        fn currents(&self) -> Vec<i32> {
            self.limits
                .lock()
                .map(|guard| guard.iter().map(|limit| limit.current).collect())
                .unwrap_or_default()
        }
    }

    impl TdpBackend for FakeTdp {
        fn limits(&self) -> tcc_hw::Result<Vec<TdpLimit>> {
            Ok(self
                .limits
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default())
        }

        fn set_values(&self, values: &[f64]) -> tcc_hw::Result<Vec<TdpLimit>> {
            let attempt = self.writes.lock().map(|guard| guard.len()).unwrap_or(0);
            if let Ok(mut writes) = self.writes.lock() {
                writes.push(values.to_vec());
            }
            if attempt > 0 && self.rollback_fails {
                return Err(write_error());
            }
            let fail_after = if attempt == 0 { self.fail_after } else { None };
            let mut limits = self
                .limits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if values.is_empty() {
                // Même contrat que `TdpControl::set_values(&[])` : maxima
                // firmware pour tous les descripteurs.
                for (index, limit) in limits.iter_mut().enumerate() {
                    if fail_after == Some(index) {
                        return Err(write_error());
                    }
                    limit.current = limit.max;
                }
                return Ok(limits.clone());
            }
            for (index, value) in values.iter().enumerate() {
                if fail_after == Some(index) {
                    return Err(write_error());
                }
                if let Some(limit) = limits.get_mut(index) {
                    limit.current = *value as i32;
                }
            }
            Ok(limits.clone())
        }
    }

    fn write_error() -> HwError {
        HwError::Io {
            path: PathBuf::from("/dev/test"),
            source: std::io::Error::other("test"),
        }
    }

    #[derive(Debug)]
    struct FakeOdm {
        platform: Mutex<Option<OdmPlatform>>,
        io: Vec<String>,
        platform_writes: Mutex<Vec<String>>,
        io_writes: Mutex<Vec<String>>,
        fail_platform: bool,
        verify_mismatch: bool,
        restore_fails: bool,
    }

    impl FakeOdm {
        fn new(choices: &[&str], current: Option<&str>, io: &[&str]) -> Self {
            Self {
                platform: Mutex::new(Some(OdmPlatform {
                    available: !choices.is_empty(),
                    current: current.map(str::to_owned),
                    choices: choices.iter().map(|choice| (*choice).to_owned()).collect(),
                })),
                io: io.iter().map(|choice| (*choice).to_owned()).collect(),
                platform_writes: Mutex::new(Vec::new()),
                io_writes: Mutex::new(Vec::new()),
                fail_platform: false,
                verify_mismatch: false,
                restore_fails: false,
            }
        }

        fn with_verify_mismatch(mut self) -> Self {
            self.verify_mismatch = true;
            self
        }

        fn with_restore_fails(mut self) -> Self {
            self.restore_fails = true;
            self
        }

        fn platform_writes(&self) -> Vec<String> {
            self.platform_writes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }

        fn io_writes(&self) -> Vec<String> {
            self.io_writes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    impl OdmBackend for FakeOdm {
        fn platform(&self) -> tcc_hw::Result<Option<OdmPlatform>> {
            Ok(self
                .platform
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default())
        }

        fn io_profiles(&self) -> tcc_hw::Result<Vec<String>> {
            Ok(self.io.clone())
        }

        fn apply_platform(&self, name: &str) -> tcc_hw::Result<()> {
            let attempt = self
                .platform_writes
                .lock()
                .map(|guard| guard.len())
                .unwrap_or(0);
            if self.fail_platform || (self.restore_fails && attempt > 0) {
                return Err(write_error());
            }
            if let Ok(mut writes) = self.platform_writes.lock() {
                writes.push(name.to_owned());
            }
            if let Ok(mut platform) = self.platform.lock()
                && let Some(platform) = platform.as_mut()
            {
                platform.current = if self.verify_mismatch && attempt == 0 {
                    Some("unexpected".to_owned())
                } else {
                    Some(name.to_owned())
                };
            }
            Ok(())
        }

        fn apply_io(&self, name: &str) -> tcc_hw::Result<()> {
            if let Ok(mut writes) = self.io_writes.lock() {
                writes.push(name.to_owned());
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeCtgp {
        offset: Mutex<Option<i32>>,
        max: Option<i32>,
        writes: Mutex<Vec<i32>>,
        fail: bool,
    }

    impl FakeCtgp {
        fn new(offset: Option<i32>, max: Option<i32>) -> Self {
            Self {
                offset: Mutex::new(offset),
                max,
                writes: Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn writes(&self) -> Vec<i32> {
            self.writes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    impl CtgpBackend for FakeCtgp {
        fn offset(&self) -> tcc_hw::Result<Option<i32>> {
            Ok(*self
                .offset
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()))
        }

        fn set_offset(&self, watts: i32) -> tcc_hw::Result<()> {
            if self.fail {
                return Err(write_error());
            }
            if let Ok(mut writes) = self.writes.lock() {
                writes.push(watts);
            }
            *self
                .offset
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(watts);
            Ok(())
        }

        fn max_offset(&self) -> tcc_hw::Result<Option<i32>> {
            Ok(self.max)
        }
    }

    #[derive(Debug)]
    struct FakePpd(PpdOwnership);

    #[async_trait]
    impl PpdProbe for FakePpd {
        async fn ownership(&self) -> PpdOwnership {
            self.0
        }
    }

    #[derive(Debug, Default)]
    struct CountingAuthorizer {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Authorizer for CountingAuthorizer {
        async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            AuthzDecision::Authorized
        }
    }

    #[derive(Debug)]
    struct FakeConsent(bool);

    impl OverrideConsent for FakeConsent {
        fn enabled(&self) -> bool {
            self.0
        }
    }

    fn context(method: &'static str) -> HwCallContext {
        HwCallContext {
            method,
            caller: ":1.42".to_owned(),
            uid: 1000,
            started: Instant::now(),
        }
    }

    fn service_with(
        tdp: Arc<FakeTdp>,
        odm: Arc<FakeOdm>,
        ctgp: Arc<FakeCtgp>,
        ppd: PpdOwnership,
        consent: bool,
        authorizer: Arc<dyn Authorizer>,
    ) -> AdminHwService {
        AdminHwService::new(
            tdp,
            odm,
            ctgp,
            Arc::new(FakePpd(ppd)),
            Arc::new(FakeConsent(consent)),
            authorizer,
            Arc::new(Activity::new()),
        )
    }

    fn default_odm() -> Arc<FakeOdm> {
        Arc::new(FakeOdm::new(
            &["power_save", "balanced", "performance"],
            Some("balanced"),
            &["power_save", "enthusiast"],
        ))
    }

    fn default_tdp() -> Arc<FakeTdp> {
        Arc::new(FakeTdp::new(&[(35, 5, 100), (45, 5, 100), (60, 5, 120)]))
    }

    fn default_ctgp() -> Arc<FakeCtgp> {
        Arc::new(FakeCtgp::new(Some(10), Some(200)))
    }

    #[tokio::test]
    async fn contract_version_and_ping() {
        let service = service_with(
            default_tdp(),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(service.contract_version(), ADMIN_HW_CONTRACT_VERSION);
        assert_eq!(
            service.get_contract_version().await,
            ADMIN_HW_CONTRACT_VERSION
        );
        assert!(service.ping().await);
        assert!(!service.get_version().await.is_empty());
    }

    #[tokio::test]
    async fn tdp_rejects_arity_and_syntax_before_backend() {
        let tdp = default_tdp();
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![0]).await,
            Err(AdminHwError::Validation("tdp.values".to_owned()))
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![1001]).await,
            Err(AdminHwError::Validation("tdp.values".to_owned()))
        );
        assert_eq!(
            service
                .set_tdp(context("SetTdpValues"), vec![50, 60, 70, 80])
                .await,
            Err(AdminHwError::InvalidArgs("tdp.count".to_owned()))
        );
        assert!(tdp.writes().is_empty());
    }

    #[tokio::test]
    async fn tdp_empty_values_apply_firmware_maxima() {
        let tdp = default_tdp();
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), Vec::new()).await,
            Ok(true)
        );
        assert_eq!(tdp.currents(), vec![100, 100, 120]);
        assert_eq!(tdp.writes(), vec![Vec::<f64>::new()]);
    }

    #[tokio::test]
    async fn tdp_empty_values_are_noop_when_already_at_maxima() {
        let tdp = Arc::new(FakeTdp::new(&[(100, 5, 100), (100, 5, 100), (120, 5, 120)]));
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), Vec::new()).await,
            Ok(true)
        );
        assert!(tdp.writes().is_empty());
    }

    #[tokio::test]
    async fn tdp_empty_values_backend_error_rolls_back() {
        let tdp = Arc::new(FakeTdp::new(&[(35, 5, 100), (45, 5, 100)]).with_failure(0, false));
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), Vec::new()).await,
            Err(AdminHwError::WriteFailed)
        );
        assert_eq!(tdp.currents(), vec![35, 45]);
    }

    #[tokio::test]
    async fn tdp_without_device_is_not_supported() {
        let tdp = Arc::new(FakeTdp::new(&[]));
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![50]).await,
            Err(AdminHwError::NotSupported("tdp".to_owned()))
        );
        assert_eq!(
            service.get_tdp(context("GetTdpValues")).await,
            Err(AdminHwError::NotSupported("tdp".to_owned()))
        );
    }

    #[tokio::test]
    async fn tdp_noop_does_not_write() {
        let tdp = default_tdp();
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![35, 45]).await,
            Ok(true)
        );
        assert!(tdp.writes().is_empty());
    }

    #[tokio::test]
    async fn tdp_applies_and_verifies() {
        let tdp = default_tdp();
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_tdp(context("SetTdpValues"), vec![40, 50, 70])
                .await,
            Ok(true)
        );
        assert_eq!(tdp.currents(), vec![40, 50, 70]);
        assert_eq!(tdp.writes().len(), 1);
        assert_eq!(
            service.get_tdp(context("GetTdpValues")).await,
            Ok(vec![40, 50, 70])
        );
    }

    #[tokio::test]
    async fn tdp_partial_failure_rolls_back() {
        let tdp = Arc::new(FakeTdp::new(&[(35, 5, 100), (45, 5, 100)]).with_failure(1, false));
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![50, 60]).await,
            Err(AdminHwError::WriteFailed)
        );
        assert_eq!(tdp.currents(), vec![35, 45]);
    }

    #[tokio::test]
    async fn tdp_partial_failure_without_rollback_is_partial_apply() {
        let tdp = Arc::new(FakeTdp::new(&[(35, 5, 100), (45, 5, 100)]).with_failure(1, true));
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![50, 60]).await,
            Err(AdminHwError::PartialApply("pl1".to_owned()))
        );
    }

    #[tokio::test]
    async fn odm_ppd_owned_refuses_platform_write() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "balanced".to_owned(), false)
                .await,
            Err(AdminHwError::PolicyDenied(
                "ppd.platform_profile".to_owned()
            ))
        );
        assert!(odm.platform_writes().is_empty());
        assert!(odm.io_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_io_fallback_is_allowed_with_ppd_owned() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "enthusiast".to_owned(), false)
                .await,
            Ok(true)
        );
        assert_eq!(odm.io_writes(), vec!["enthusiast".to_owned()]);
        assert!(odm.platform_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_io_fallback_is_allowed_with_ppd_unknown() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Unknown,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "enthusiast".to_owned(), false)
                .await,
            Ok(true)
        );
        assert_eq!(odm.io_writes(), vec!["enthusiast".to_owned()]);
        assert!(odm.platform_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_override_requires_persistent_consent() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(
                    context("SetOdmProfileOverride"),
                    "balanced".to_owned(),
                    true
                )
                .await,
            Err(AdminHwError::PolicyDenied(
                "ppd.override_disabled".to_owned()
            ))
        );
        assert!(odm.platform_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_override_without_consent_skips_polkit() {
        let authorizer = Arc::new(CountingAuthorizer::default());
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            false,
            authorizer.clone(),
        );
        assert_eq!(
            service
                .set_odm(
                    context("SetOdmProfileOverride"),
                    "balanced".to_owned(),
                    true
                )
                .await,
            Err(AdminHwError::PolicyDenied(
                "ppd.override_disabled".to_owned()
            ))
        );
        assert_eq!(
            authorizer.calls.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "aucun dialogue polkit ne doit être déclenché"
        );
        assert!(odm.platform_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_override_writes_when_consented() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            true,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(
                    context("SetOdmProfileOverride"),
                    "balanced".to_owned(),
                    true
                )
                .await,
            Ok(true)
        );
        assert_eq!(odm.platform_writes(), vec!["balanced".to_owned()]);
    }

    #[tokio::test]
    async fn odm_unknown_ppd_blocks_platform_even_with_override() {
        let odm = default_odm();
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Unknown,
            true,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(
                    context("SetOdmProfileOverride"),
                    "balanced".to_owned(),
                    true
                )
                .await,
            Err(AdminHwError::PolicyDenied(
                "ppd.platform_profile".to_owned()
            ))
        );
        assert!(odm.platform_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_name_in_both_lists_is_not_silently_routed_to_io() {
        let odm = Arc::new(FakeOdm::new(&["balanced"], Some("balanced"), &["balanced"]));
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Owned,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "balanced".to_owned(), false)
                .await,
            Err(AdminHwError::PolicyDenied(
                "ppd.platform_profile".to_owned()
            ))
        );
        assert!(odm.io_writes().is_empty());
    }

    #[tokio::test]
    async fn odm_platform_verification_failure_restores_previous() {
        let odm = Arc::new(
            FakeOdm::new(
                &["power_save", "balanced", "performance"],
                Some("balanced"),
                &[],
            )
            .with_verify_mismatch(),
        );
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "performance".to_owned(), false)
                .await,
            Err(AdminHwError::WriteFailed)
        );
        assert_eq!(
            odm.platform_writes(),
            vec!["performance".to_owned(), "balanced".to_owned()],
            "l'ancien profil doit être restauré après vérification ratée"
        );
        assert_eq!(
            odm.platform
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|platform| platform.current.as_deref()),
            Some("balanced")
        );
    }

    #[tokio::test]
    async fn odm_platform_rollback_failure_is_partial_apply() {
        let odm = Arc::new(
            FakeOdm::new(
                &["power_save", "balanced", "performance"],
                Some("balanced"),
                &[],
            )
            .with_verify_mismatch()
            .with_restore_fails(),
        );
        let service = service_with(
            default_tdp(),
            Arc::clone(&odm),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "performance".to_owned(), false)
                .await,
            Err(AdminHwError::PartialApply("platform_profile".to_owned()))
        );
        assert_eq!(odm.platform_writes(), vec!["performance".to_owned()]);
    }

    #[tokio::test]
    async fn odm_unknown_name_is_validation() {
        let service = service_with(
            default_tdp(),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "turbo".to_owned(), false)
                .await,
            Err(AdminHwError::Validation("odm.name".to_owned()))
        );
        assert_eq!(
            service
                .set_odm(context("SetOdmProfile"), "bad\\name".to_owned(), false)
                .await,
            Err(AdminHwError::InvalidArgs("odm.name".to_owned()))
        );
    }

    #[tokio::test]
    async fn ctgp_validates_and_is_idempotent() {
        let ctgp = Arc::new(FakeCtgp::new(Some(10), Some(150)));
        let service = service_with(
            default_tdp(),
            default_odm(),
            Arc::clone(&ctgp),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), -1).await,
            Err(AdminHwError::Validation("nvidia.ctgpOffset".to_owned()))
        );
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 151).await,
            Err(AdminHwError::Validation("nvidia.ctgpOffset".to_owned()))
        );
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 10).await,
            Ok(true)
        );
        assert!(ctgp.writes().is_empty());
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 40).await,
            Ok(true)
        );
        assert_eq!(ctgp.writes(), vec![40]);
    }

    #[derive(Debug)]
    struct FakeBounds(Option<i32>);

    #[async_trait]
    impl CtgpBounds for FakeBounds {
        async fn max_offset(&self) -> Option<i32> {
            self.0
        }
    }

    #[tokio::test]
    async fn ctgp_dynamic_bound_from_provider_is_used() {
        let ctgp = Arc::new(FakeCtgp::new(Some(10), None));
        let service = service_with(
            default_tdp(),
            default_odm(),
            Arc::clone(&ctgp),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        )
        .with_ctgp_bounds(Arc::new(FakeBounds(Some(150))));

        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 151).await,
            Err(AdminHwError::Validation("nvidia.ctgpOffset".to_owned()))
        );
        assert!(ctgp.writes().is_empty());
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 150).await,
            Ok(true)
        );
        assert_eq!(ctgp.writes(), vec![150]);
    }

    #[tokio::test]
    async fn ctgp_static_bound_applies_when_provider_unavailable() {
        let ctgp = Arc::new(FakeCtgp::new(Some(10), None));
        let service = service_with(
            default_tdp(),
            default_odm(),
            Arc::clone(&ctgp),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        )
        .with_ctgp_bounds(Arc::new(FakeBounds(None)));

        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 151).await,
            Ok(true)
        );
        assert_eq!(ctgp.writes(), vec![151]);
    }

    #[tokio::test]
    async fn ctgp_absent_is_not_supported() {
        let ctgp = Arc::new(FakeCtgp::new(None, None));
        let service = service_with(
            default_tdp(),
            default_odm(),
            Arc::clone(&ctgp),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        assert_eq!(
            service.set_ctgp(context("SetNvidiaCtgpOffset"), 40).await,
            Err(AdminHwError::NotSupported("ctgp_offset".to_owned()))
        );
        assert!(ctgp.writes().is_empty());
    }

    #[tokio::test]
    async fn denied_authorization_never_reaches_backend() {
        let tdp = default_tdp();
        let service = service_with(
            Arc::clone(&tdp),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(DenyAll),
        );
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![50]).await,
            Err(AdminHwError::NotAuthorized)
        );
        assert!(tdp.writes().is_empty());
    }

    #[tokio::test]
    async fn concurrent_write_is_busy() {
        let service = service_with(
            default_tdp(),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        let permit = service
            .write_gate
            .clone()
            .try_acquire_owned()
            .expect("verrou");
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![50]).await,
            Err(AdminHwError::Busy)
        );
        drop(permit);
    }

    #[tokio::test]
    async fn write_rate_limit_is_enforced() {
        let service = service_with(
            default_tdp(),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        for _ in 0..4 {
            assert_eq!(
                service.set_tdp(context("SetTdpValues"), vec![35]).await,
                Ok(true)
            );
        }
        assert_eq!(
            service.set_tdp(context("SetTdpValues"), vec![35]).await,
            Err(AdminHwError::RateLimited)
        );
    }

    #[tokio::test]
    async fn read_rate_limit_is_enforced() {
        let service = service_with(
            default_tdp(),
            default_odm(),
            default_ctgp(),
            PpdOwnership::Absent,
            false,
            Arc::new(AllowAll),
        );
        for _ in 0..5 {
            assert_eq!(
                service.get_tdp(context("GetTdpValues")).await,
                Ok(vec![35, 45, 60])
            );
        }
        assert_eq!(
            service.get_tdp(context("GetTdpValues")).await,
            Err(AdminHwError::RateLimited)
        );
    }

    #[test]
    fn error_codes_match_contract() {
        use tcc_ipc::AdminHwErrorCode;
        assert_eq!(
            AdminHwError::PolicyDenied("ppd.platform_profile".to_owned()).code(),
            AdminHwErrorCode::PolicyDenied
        );
        assert_eq!(
            AdminHwError::PartialApply("pl1".to_owned()).code(),
            AdminHwErrorCode::PartialApply
        );
        assert_eq!(AdminHwError::Busy.code(), AdminHwErrorCode::Busy);
    }
}
