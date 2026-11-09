// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Service D-Bus du daemon.
//!
//! Le jalon 1 expose une API de lecture JSON et un override temporaire. Les
//! méthodes typées et les signaux seront ajoutés au jalon 2, quand la GUI sera
//! branchée. Les noms de méthodes reprennent la convention historique pour
//! faciliter la transition depuis l'original.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, RwLock, broadcast};
use zbus::fdo;

use tcc_core::{KeyboardState, LightbarState, MAX_KEYBOARD_STATES, MAX_PROFILE_ID_LEN, Settings};
use tcc_hw::charging::{ChargeType, ChargingController};
use tcc_hw::error::HwError;
use tcc_hw::fn_lock::FnLock;
use tcc_hw::lighting::{LightingController, RgbColor};
use tcc_hw::sysfs::SysfsRoot;
use tcc_ipc::{
    AdminReloadReport, DBUS_CONTROL_INTERFACE, DBUS_INTERFACE, DBUS_MANAGER_INTERFACE, DBUS_PATH,
    IPC_CONTRACT_VERSION,
};

use crate::fan::FanData;
use crate::hardware::{HardwareSnapshot, HardwareStatus};
use crate::rate_limit::{
    READ_BURST, READ_RATE_PER_SECOND, RateLimiter, ReadRateLimiter, WRITE_BURST,
    WRITE_RATE_PER_SECOND, WriteRateLimiter,
};
use crate::runtime::RuntimeState;
use crate::sensor_lease::SensorLease;
use crate::shutdown_timer::ShutdownTimerStatus;

/// Objet D-Bus du daemon.
pub struct TccDaemon {
    state: Arc<RwLock<RuntimeState>>,
    notify: Arc<Notify>,
    fan: Arc<std::sync::RwLock<FanData>>,
    /// Limite les écritures D-Bus par nom unique d'appelant. Voir R-03/R-05.
    writes: std::sync::Mutex<WriteRateLimiter>,
}

impl TccDaemon {
    /// Construit l'interface D-Bus.
    #[must_use]
    pub fn new(
        state: Arc<RwLock<RuntimeState>>,
        notify: Arc<Notify>,
        fan: Arc<std::sync::RwLock<FanData>>,
    ) -> Self {
        Self {
            state,
            notify,
            fan,
            writes: std::sync::Mutex::new(WriteRateLimiter::default()),
        }
    }
}

#[zbus::interface(name = "com.tuxedocomputers.tccd")]
impl TccDaemon {
    /// Version du contrat IPC.
    #[zbus(property, name = "ContractVersion")]
    fn contract_version(&self) -> u32 {
        IPC_CONTRACT_VERSION
    }

    /// Version du daemon.
    #[zbus(name = "GetVersion")]
    async fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Snapshot complet de l'état, sérialisé en JSON.
    #[zbus(name = "GetStateJSON")]
    async fn get_state_json(&self) -> fdo::Result<String> {
        let state = self.state.read().await;
        serde_json::to_string(&state.snapshot())
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Profil actif, sérialisé en JSON (`null` avant la première application).
    #[zbus(name = "GetActiveProfileJSON")]
    async fn get_active_profile_json(&self) -> fdo::Result<String> {
        let state = self.state.read().await;
        serde_json::to_string(&state.snapshot().active_profile)
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Liste complète des profils, sérialisée en JSON.
    #[zbus(name = "GetProfilesJSON")]
    async fn get_profiles_json(&self) -> fdo::Result<String> {
        let state = self.state.read().await;
        serde_json::to_string(&state.profiles)
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Réglages globaux, sérialisés en JSON.
    #[zbus(name = "GetSettingsJSON")]
    async fn get_settings_json(&self) -> fdo::Result<String> {
        let state = self.state.read().await;
        serde_json::to_string(&state.settings)
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// État des ventilateurs (température, vitesse mesurée et consigne).
    ///
    /// Voir `docs/DBUS_API.md` section « Ventilateurs ». Le payload est un
    /// `crate::fan::FanData` sérialisé ; il ne contient aucune donnée
    /// sensible et reste stable même sans matériel (`available: false`).
    #[zbus(name = "GetFanDataJSON")]
    async fn get_fan_data_json(&self) -> fdo::Result<String> {
        let json = {
            let fan = self
                .fan
                .read()
                .map_err(|_| fdo::Error::Failed("état des ventilateurs indisponible".to_owned()))?;
            serde_json::to_string(&*fan).map_err(|error| fdo::Error::Failed(error.to_string()))?
        };
        Ok(json)
    }

    /// Active un profil temporaire jusqu'au prochain changement d'état.
    ///
    /// **Sécurité (R-03/R-05)** : méthode globale, non authentifiée par polkit
    /// à ce stade (voir `docs/DBUS_API.md`). Elle est bornée en taille,
    /// limitée par nom unique D-Bus (5 écritures/s, burst 10) et auditée
    /// (`caller`, `caller_uid`, `profile_id`). Le payload brut n'est jamais
    /// journalisé.
    #[zbus(name = "SetTempProfile")]
    async fn set_temp_profile(
        &self,
        id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let caller = caller_name(&header);
        let caller_uid = resolve_caller_uid(connection, &header).await;

        if let Err(error) = validate_profile_id_argument(&id) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                profile_id_len = id.len(),
                "SetTempProfile rejeté : identifiant invalide ou trop long"
            );
            return Err(error);
        }

        if let Err(error) = self.consume_write(&caller) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                profile_id = %id,
                "SetTempProfile rejeté : limite de débit atteinte"
            );
            return Err(error);
        }

        tracing::info!(
            caller = %caller,
            caller_uid = ?caller_uid,
            profile_id = %id,
            "SetTempProfile"
        );

        {
            let mut state = self.state.write().await;
            if let Err(error) = state.set_temp_profile(&id) {
                tracing::warn!(
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    profile_id = %id,
                    error = %error,
                    "SetTempProfile refusé"
                );
                return Err(fdo::Error::InvalidArgs(error));
            }
        }
        self.notify.notify_one();
        Ok(true)
    }

    /// Supprime l'override temporaire.
    ///
    /// Même limiteur que `SetTempProfile`. Une méthode référencée par un nom
    /// unique inconnu (appel interne) partage le bucket `unknown`.
    #[zbus(name = "ClearTempProfile")]
    async fn clear_temp_profile(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let caller = caller_name(&header);
        let caller_uid = resolve_caller_uid(connection, &header).await;

        if let Err(error) = self.consume_write(&caller) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                "ClearTempProfile rejeté : limite de débit atteinte"
            );
            return Err(error);
        }

        tracing::info!(
            caller = %caller,
            caller_uid = ?caller_uid,
            "ClearTempProfile"
        );

        let changed = {
            let mut state = self.state.write().await;
            state.clear_temp_profile()
        };
        if changed {
            self.notify.notify_one();
        }
        Ok(changed)
    }

    /// Indique que le daemon est vivant.
    #[zbus(name = "Ping")]
    async fn ping(&self) -> bool {
        true
    }

    /// Signal émis après chaque changement effectif de profil.
    ///
    /// Le payload est le `StateSnapshot` JSON, afin que les clients puissent
    /// ignorer une révision déjà vue.
    #[zbus(signal)]
    async fn state_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        snapshot_json: &str,
    ) -> zbus::Result<()>;
}

impl TccDaemon {
    /// Consomme un jeton d'écriture pour l'appelant.
    ///
    /// Fail-closed si le mutex est empoisonné : on préfère refuser une écriture
    /// que continuer sans limitation.
    fn consume_write(&self, caller: &str) -> fdo::Result<()> {
        let mut limiter = self
            .writes
            .lock()
            .map_err(|_| fdo::Error::Failed("limiteur d'écriture indisponible".to_owned()))?;
        if limiter.allow(caller, Instant::now()) {
            Ok(())
        } else {
            Err(fdo::Error::LimitsExceeded(
                "trop d'écritures de profil ; réessayez plus tard".to_owned(),
            ))
        }
    }
}

/// Nom unique D-Bus de l'appelant, ou `unknown` pour un appel interne.
fn caller_name(header: &zbus::message::Header<'_>) -> String {
    header
        .sender()
        .map(ToString::to_string)
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Borne l'identifiant avant toute résolution de profil (R-03/R-05).
fn validate_profile_id_argument(id: &str) -> fdo::Result<()> {
    if id.is_empty() || id.len() > MAX_PROFILE_ID_LEN {
        return Err(fdo::Error::InvalidArgs(format!(
            "identifiant de profil invalide (1 à {MAX_PROFILE_ID_LEN} octets)"
        )));
    }
    Ok(())
}

/// Résout l'UID de l'appelant via `GetConnectionUnixUser`.
///
/// L'échec est non fatal : la méthode continue, avec `caller_uid = None`.
async fn resolve_caller_uid(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<u32> {
    let sender = header.sender()?;
    let bus_name = zbus::names::BusName::try_from(sender.as_str()).ok()?;
    let proxy = tokio::time::timeout(
        Duration::from_millis(500),
        zbus::fdo::DBusProxy::new(connection),
    )
    .await
    .ok()?
    .ok()?;
    tokio::time::timeout(
        Duration::from_millis(500),
        proxy.get_connection_unix_user(bus_name),
    )
    .await
    .ok()?
    .ok()
}

/// Surveille la connexion D-Bus et signale sa perte.
///
/// Une perte de connexion (bus redémarré, nom perdu) doit provoquer l'arrêt du
/// daemon pour que systemd le redémarre : un `tccd` sans nom D-Bus est
/// inutilisable par la GUI et l'agent. Deux échecs consécutifs du `GetId`
/// (toutes les 2 s) sont nécessaires pour éviter un faux positif transitoire.
pub fn watch_connection(connection: &zbus::Connection) -> tokio::sync::oneshot::Receiver<()> {
    let connection = connection.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failures = 0u8;
        loop {
            interval.tick().await;
            let alive = tokio::time::timeout(Duration::from_secs(2), async {
                let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
                proxy.get_id().await
            })
            .await;
            match alive {
                Ok(Ok(_)) => failures = 0,
                _ => {
                    failures = failures.saturating_add(1);
                    if failures >= 2 {
                        let _ = sender.send(());
                        break;
                    }
                }
            }
        }
    });
    receiver
}

/// Événement d'audit de complétion d'un setter (sans payload ni chemin).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuditCompletion {
    caller: String,
    caller_uid: u32,
    method: &'static str,
    action: &'static str,
    result: &'static str,
    duration_ms: u64,
}

/// Destination d'audit injectable (tracing en production, buffer en test).
trait AuditSink: Send + Sync {
    fn completion(&self, event: AuditCompletion);
}

#[derive(Debug, Default)]
struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn completion(&self, event: AuditCompletion) {
        tracing::info!(
            method = event.method,
            action = event.action,
            caller = %event.caller,
            caller_uid = event.caller_uid,
            result = event.result,
            duration_ms = event.duration_ms,
            "setter matériel terminé"
        );
    }
}

/// Dépendances matérielles injectées dans l'interface `Manager1`.
#[derive(Clone)]
pub(crate) struct HardwareBridge {
    /// Instantané partagé des workers.
    pub(crate) status: Arc<HardwareStatus>,
    /// Bail de collecte des capteurs (10 s, réarmé par lecture).
    pub(crate) sensors: Arc<SensorLease>,
    /// Racine sysfs (réelle ou factice).
    pub(crate) root: SysfsRoot,
    /// Mode `--dev-session` : aucune écriture matérielle.
    pub(crate) dry_run: bool,
    /// Authorizer injecté (tests, `--dev-session`). `None` = polkit réel.
    authorizer: Option<Arc<dyn SettersAuthorizer>>,
    /// Vrai pendant `Preparing`/`Suspended`/`Resuming` (SL-09).
    sleeping: Arc<AtomicBool>,
    /// Sink d'audit (tracing par défaut, buffer en test).
    audit: Arc<dyn AuditSink>,
    /// État de la minuterie d'extinction, partagé avec la boucle principale.
    shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
}

impl HardwareBridge {
    /// Pont de production : polkit réel.
    #[must_use]
    pub(crate) fn production(
        status: Arc<HardwareStatus>,
        sensors: Arc<SensorLease>,
        root: SysfsRoot,
        dry_run: bool,
        sleeping: Arc<AtomicBool>,
        shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
    ) -> Self {
        Self {
            status,
            sensors,
            root,
            dry_run,
            authorizer: None,
            sleeping,
            audit: Arc::new(TracingAuditSink),
            shutdown,
        }
    }

    /// Pont réservé à `--dev-session` : le dry-run est forcé et l'authorizer
    /// permissif ne peut être construit que par ce chemin.
    #[must_use]
    pub(crate) fn dev_session(
        status: Arc<HardwareStatus>,
        sensors: Arc<SensorLease>,
        root: SysfsRoot,
        sleeping: Arc<AtomicBool>,
        shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
    ) -> Self {
        Self {
            status,
            sensors,
            root,
            dry_run: true,
            authorizer: Some(Arc::new(DevSessionAuthorizer)),
            sleeping,
            audit: Arc::new(TracingAuditSink),
            shutdown,
        }
    }

    /// Pont de test avec un authorizer et un dry-run explicites.
    #[cfg(test)]
    fn test_with_authorizer(
        status: Arc<HardwareStatus>,
        sensors: Arc<SensorLease>,
        root: SysfsRoot,
        dry_run: bool,
        authorizer: Arc<dyn SettersAuthorizer>,
        sleeping: Arc<AtomicBool>,
        shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
    ) -> Self {
        Self::test_with_authorizer_and_audit(
            status,
            sensors,
            root,
            dry_run,
            authorizer,
            sleeping,
            Arc::new(TracingAuditSink),
            shutdown,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn test_with_authorizer_and_audit(
        status: Arc<HardwareStatus>,
        sensors: Arc<SensorLease>,
        root: SysfsRoot,
        dry_run: bool,
        authorizer: Arc<dyn SettersAuthorizer>,
        sleeping: Arc<AtomicBool>,
        audit: Arc<dyn AuditSink>,
        shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
    ) -> Self {
        Self {
            status,
            sensors,
            root,
            dry_run,
            authorizer: Some(authorizer),
            sleeping,
            audit,
            shutdown,
        }
    }
}

/// Action polkit des setters de charge.
pub(crate) const ACTION_CHARGING: &str = "com.tuxedocomputers.tccd1.charging";
/// Action polkit des setters lighting/Fn-Lock.
pub(crate) const ACTION_SETTINGS: &str = "com.tuxedocomputers.tccd1.settings";

/// Vérifie l'autorisation d'un setter `Manager1`.
///
/// L'implémentation de production appelle `tcc-authz` (sujet
/// `system-bus-name`, fail-closed, root non court-circuité). Les tests et le
/// mode `--dev-session` injectent une implémentation factice.
#[async_trait::async_trait]
pub(crate) trait SettersAuthorizer: Send + Sync {
    /// Autorise `caller` (nom unique) pour `action`.
    async fn authorize(&self, caller: &str, uid: u32, action: &'static str) -> fdo::Result<()>;
}

/// Authorizer polkit de production.
#[derive(Debug, Default)]
struct PolkitSettersAuthorizer;

#[async_trait::async_trait]
impl SettersAuthorizer for PolkitSettersAuthorizer {
    async fn authorize(&self, caller: &str, uid: u32, action: &'static str) -> fdo::Result<()> {
        let request = tcc_authz::AuthorizationRequest::new(caller, action).with_caller_uid(uid);
        match tcc_authz::check_authorization(&request).await {
            Ok(result) if result.is_authorized() => Ok(()),
            Ok(_) => Err(fdo::Error::AccessDenied("autorisation requise".to_owned())),
            Err(error) => {
                // Le motif détaillé reste dans le journal ; le client reçoit
                // un refus générique.
                tracing::warn!(%error, action, "polkit indisponible pour un setter");
                Err(fdo::Error::AccessDenied("autorisation requise".to_owned()))
            }
        }
    }
}

/// Authorizer permissif réservé à `--dev-session` (bus de session, dry-run)
/// et aux tests. Ne jamais l'injecter sur le bus système.
#[derive(Debug, Default)]
pub(crate) struct DevSessionAuthorizer;

#[async_trait::async_trait]
impl SettersAuthorizer for DevSessionAuthorizer {
    async fn authorize(&self, _caller: &str, _uid: u32, _action: &'static str) -> fdo::Result<()> {
        Ok(())
    }
}

/// Taille maximale d'une valeur de charge/priorité.
const MAX_CHARGE_VALUE_LEN: usize = 128;
/// Taille maximale d'un payload JSON d'éclairage.
const MAX_LIGHTING_JSON_BYTES: usize = 64 * 1024;

/// Interface typée `com.tuxedocomputers.tccd.Manager1`.
///
/// Expose la matrice matérielle en lecture et des setters **non persistants**
/// pour charge/lighting/Fn-Lock. TDP, ODM, cTGP, PRIME et YCbCr restent en
/// lecture seule : leurs écritures sont réservées à `tccd-admin` + polkit
/// (D1/D5).
pub(crate) struct HardwareManager {
    state: Arc<RwLock<RuntimeState>>,
    notify: Arc<Notify>,
    status: Arc<HardwareStatus>,
    sensors: Arc<SensorLease>,
    root: SysfsRoot,
    dry_run: bool,
    /// Limiteur nom unique **et** UID pour les setters (M1-01).
    writes: tcc_authz::AuthorizationLimiter,
    /// Lectures coûteuses du snapshot (2/s, burst 5) par nom unique.
    reads: std::sync::Mutex<ReadRateLimiter>,
    /// Lectures de l'état de la minuterie (2/s, burst 5), bucket séparé.
    shutdown_reads: std::sync::Mutex<ReadRateLimiter>,
    /// Un seul dialogue polkit global (M1-01).
    authz_gate: tcc_authz::DialogGate,
    /// Autorisation polkit des setters (injectable en test).
    authorizer: Arc<dyn SettersAuthorizer>,
    /// Vrai pendant la veille : aucun setter ne doit écrire (SL-09).
    sleeping: Arc<AtomicBool>,
    /// Sink d'audit de complétion.
    audit: Arc<dyn AuditSink>,
    /// État de la minuterie d'extinction.
    shutdown: Arc<std::sync::Mutex<ShutdownTimerStatus>>,
}

/// Contexte d'audit d'un setter autorisé.
#[derive(Debug)]
struct WriteContext {
    caller: String,
    caller_uid: u32,
    method: &'static str,
    action: &'static str,
    started: Instant,
}

impl HardwareManager {
    /// Construit l'interface matérielle.
    #[must_use]
    pub(crate) fn new(
        state: Arc<RwLock<RuntimeState>>,
        notify: Arc<Notify>,
        bridge: HardwareBridge,
    ) -> Self {
        debug_assert!(
            bridge.authorizer.is_none() || bridge.dry_run || cfg!(test),
            "authorizer permissif injecté sans dry-run"
        );
        Self {
            state,
            notify,
            status: bridge.status,
            sensors: bridge.sensors,
            root: bridge.root,
            dry_run: bridge.dry_run,
            writes: tcc_authz::AuthorizationLimiter::new(WRITE_RATE_PER_SECOND, WRITE_BURST),
            reads: std::sync::Mutex::new(RateLimiter::new(READ_RATE_PER_SECOND, READ_BURST)),
            shutdown_reads: std::sync::Mutex::new(RateLimiter::new(
                READ_RATE_PER_SECOND,
                READ_BURST,
            )),
            authz_gate: tcc_authz::DialogGate::new(),
            authorizer: bridge
                .authorizer
                .unwrap_or_else(|| Arc::new(PolkitSettersAuthorizer)),
            sleeping: bridge.sleeping,
            audit: bridge.audit,
            shutdown: bridge.shutdown,
        }
    }

    /// Résout le nom unique et l'UID de l'appelant.
    async fn caller_identity(
        &self,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
    ) -> (String, Option<u32>) {
        let caller = caller_name(header);
        let caller_uid = resolve_caller_uid(connection, header).await;
        (caller, caller_uid)
    }

    /// Refuse les setters lighting si l'utilisateur a désactivé le contrôle
    /// global (`keyboard_backlight_control_enabled`).
    async fn ensure_lighting_enabled(&self, method: &'static str) -> fdo::Result<()> {
        if self
            .state
            .read()
            .await
            .settings
            .keyboard_backlight_control_enabled
        {
            Ok(())
        } else {
            tracing::warn!(
                method,
                result = "disabled",
                "setter lighting refusé : contrôle global désactivé"
            );
            Err(fdo::Error::AccessDenied(
                "contrôle du rétroéclairage désactivé".to_owned(),
            ))
        }
    }

    /// Consomme un jeton d'écriture (5/s, burst 10 par nom unique **et** UID).
    fn rate_limit(
        &self,
        caller: &str,
        caller_uid: Option<u32>,
        method: &'static str,
    ) -> fdo::Result<()> {
        let uid = caller_uid.unwrap_or(u32::MAX);
        if !self.writes.allow(caller, uid, Instant::now()) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                method,
                result = "rate_limited",
                "setter matériel rejeté : limite de débit atteinte"
            );
            return Err(fdo::Error::LimitsExceeded(
                "trop d'écritures matérielles ; réessayez plus tard".to_owned(),
            ));
        }
        Ok(())
    }

    /// Autorise l'écriture via polkit (fail-closed, root non court-circuité).
    async fn authorize_setter(
        &self,
        caller: &str,
        caller_uid: Option<u32>,
        method: &'static str,
        action: &'static str,
    ) -> fdo::Result<()> {
        let Some(uid) = caller_uid else {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                method,
                action,
                result = "denied",
                "setter matériel refusé : identité inconnue"
            );
            return Err(fdo::Error::AccessDenied("autorisation requise".to_owned()));
        };
        // Un seul dialogue polkit global et un seul par appelant (M1-01).
        // Le permis est rendu au Drop, y compris en cas d'annulation.
        let _permit = match self
            .authz_gate
            .acquire(caller, std::time::Duration::from_secs(2))
            .await
        {
            Ok(permit) => permit,
            Err(_) => {
                tracing::warn!(
                    caller = %caller,
                    caller_uid = uid,
                    method,
                    action,
                    result = "busy",
                    "setter matériel refusé : dialogue polkit déjà en cours"
                );
                return Err(fdo::Error::AccessDenied("autorisation requise".to_owned()));
            }
        };
        match self.authorizer.authorize(caller, uid, action).await {
            Ok(()) => {
                tracing::info!(
                    caller = %caller,
                    caller_uid = uid,
                    method,
                    action,
                    result = "authorized",
                    "setter matériel autorisé"
                );
                Ok(())
            }
            Err(error) => {
                // Le détail (polkit absent, timeout, refus) reste dans le
                // journal ; le client reçoit toujours un refus générique.
                tracing::warn!(
                    caller = %caller,
                    caller_uid = uid,
                    method,
                    action,
                    result = "denied",
                    %error,
                    "setter matériel refusé par polkit"
                );
                Err(fdo::Error::AccessDenied("autorisation requise".to_owned()))
            }
        }
    }

    /// Séquence identité → validation → rate limit → polkit.
    ///
    /// La validation est faite avant de consommer un jeton et avant polkit :
    /// une entrée invalide ne déclenche ni limiteur ni dialogue.
    async fn prepare_write<T, F>(
        &self,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
        method: &'static str,
        action: &'static str,
        validate: F,
    ) -> fdo::Result<(T, WriteContext)>
    where
        F: FnOnce() -> fdo::Result<T>,
    {
        let started = Instant::now();
        let (caller, caller_uid) = self.caller_identity(header, connection).await;
        if self.sleeping.load(Ordering::Relaxed) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                method,
                result = "sleeping",
                "setter matériel refusé pendant la veille"
            );
            return Err(fdo::Error::AccessDenied("veille en cours".to_owned()));
        }
        let value = match validate() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    method,
                    result = "invalid_args",
                    "setter matériel refusé : entrée invalide"
                );
                return Err(error);
            }
        };
        self.rate_limit(&caller, caller_uid, method)?;
        self.authorize_setter(&caller, caller_uid, method, action)
            .await?;
        let caller_uid = caller_uid
            .ok_or_else(|| fdo::Error::AccessDenied("autorisation requise".to_owned()))?;
        Ok((
            value,
            WriteContext {
                caller,
                caller_uid,
                method,
                action,
                started,
            },
        ))
    }

    /// Ligne d'audit de complétion, sans payload ni chemin (M1-05).
    fn audit_completion(&self, context: &WriteContext, ok: bool) {
        self.audit.completion(AuditCompletion {
            caller: context.caller.clone(),
            caller_uid: context.caller_uid,
            method: context.method,
            action: context.action,
            result: if ok { "ok" } else { "error" },
            duration_ms: context.started.elapsed().as_millis() as u64,
        });
    }

    async fn update_settings<F>(&self, mutation: F)
    where
        F: FnOnce(&mut Settings),
    {
        {
            let mut state = self.state.write().await;
            mutation(&mut state.settings);
            state.revision = state.revision.saturating_add(1);
        }
        self.notify.notify_one();
    }

    async fn bump_revision(&self) {
        {
            let mut state = self.state.write().await;
            state.revision = state.revision.saturating_add(1);
        }
        self.notify.notify_one();
    }

    /// Exécute une opération `tcc-hw` bloquante hors du runtime asynchrone.
    async fn blocking_hw<T, F>(&self, operation: F) -> fdo::Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, HwError> + Send + 'static,
    {
        match tokio::task::spawn_blocking(operation).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(map_hw_error(error)),
            Err(error) => Err(fdo::Error::Failed(format!(
                "tâche matérielle interrompue : {error}"
            ))),
        }
    }

    /// Consomme un jeton de lecture du snapshot matériel.
    fn consume_read(&self, caller: &str) -> fdo::Result<()> {
        let mut limiter = self
            .reads
            .lock()
            .map_err(|_| fdo::Error::Failed("limiteur de lecture indisponible".to_owned()))?;
        if limiter.allow(caller, Instant::now()) {
            Ok(())
        } else {
            Err(fdo::Error::LimitsExceeded(
                "trop de lectures du snapshot matériel ; réessayez plus tard".to_owned(),
            ))
        }
    }

    /// Consomme un jeton de lecture de l'état de la minuterie.
    fn consume_shutdown_read(&self, caller: &str) -> fdo::Result<()> {
        let mut limiter = self.shutdown_reads.lock().map_err(|_| {
            fdo::Error::Failed("limiteur de lecture de la minuterie indisponible".to_owned())
        })?;
        if limiter.allow(caller, Instant::now()) {
            Ok(())
        } else {
            Err(fdo::Error::LimitsExceeded(
                "trop de lectures de l'état de la minuterie ; réessayez plus tard".to_owned(),
            ))
        }
    }

    /// Snapshot sérialisé ; le bail a déjà été réarmé par l'appelant accepté.
    fn read_snapshot(&self) -> fdo::Result<String> {
        let snapshot: HardwareSnapshot = self.status.snapshot();
        serde_json::to_string(&snapshot).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Consomme un jeton puis produit un snapshot (tests et usages internes).
    #[cfg(test)]
    fn snapshot_for_caller(&self, caller: &str) -> fdo::Result<String> {
        self.consume_read(caller)?;
        self.sensors.touch(Instant::now());
        self.read_snapshot()
    }
}

#[zbus::interface(name = "com.tuxedocomputers.tccd.Manager1")]
impl HardwareManager {
    /// Version du contrat IPC, voie supportée pour les clients.
    ///
    /// La propriété `ContractVersion` de l'interface historique reste
    /// déclarée pour l'introspection, mais les clients ne doivent pas
    /// dépendre de `Properties.Get`/`GetAll` (P-12).
    #[zbus(name = "GetContractVersion")]
    async fn get_contract_version(&self) -> u32 {
        IPC_CONTRACT_VERSION
    }

    /// Instantané complet de la matrice matérielle.
    ///
    /// Une lecture réarme le bail de collecte si celle-ci est active, comme
    /// l'original. La lecture est limitée à 2/s (burst 5) par nom unique : une
    /// rafale est rejetée **avant** de réarmer le bail.
    #[zbus(name = "GetHardwareSnapshotJSON")]
    async fn get_hardware_snapshot_json(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let caller = caller_name(&header);
        // Le jeton est consommé avant toute résolution d'UID : une rafale
        // rejetée ne génère pas d'appel GetConnectionUnixUser.
        if let Err(error) = self.consume_read(&caller) {
            tracing::warn!(
                caller = %caller,
                "GetHardwareSnapshotJSON rejeté : limite de lecture atteinte"
            );
            return Err(error);
        }
        let caller_uid = resolve_caller_uid(connection, &header).await;
        self.sensors.touch(Instant::now());
        match self.read_snapshot() {
            Ok(json) => {
                tracing::debug!(
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    "GetHardwareSnapshotJSON"
                );
                Ok(json)
            }
            Err(error) => Err(error),
        }
    }

    /// Active/désactive la collecte de capteurs. Aucune I/O immédiate.
    ///
    /// `true` ouvre une session limitée à 5 min (bail d'inactivité 10 s,
    /// réarmé par les lectures). Un `true` pendant une session active ne
    /// repousse pas le plafond ; il faut attendre l'expiration pour en ouvrir
    /// une nouvelle. La méthode est rate-limitée et auditée comme les setters
    /// matériels.
    #[zbus(name = "SetSensorDataCollectionStatus")]
    async fn set_sensor_data_collection_status(
        &self,
        status: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (caller, caller_uid) = self.caller_identity(&header, connection).await;
        if self.sleeping.load(Ordering::Relaxed) {
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                method = "SetSensorDataCollectionStatus",
                result = "sleeping",
                "setter refusé pendant la veille"
            );
            return Err(fdo::Error::AccessDenied("veille en cours".to_owned()));
        }
        self.rate_limit(&caller, caller_uid, "SetSensorDataCollectionStatus")?;
        tracing::info!(
            caller = %caller,
            caller_uid = ?caller_uid,
            method = "SetSensorDataCollectionStatus",
            status,
            "bail de collecte des capteurs"
        );
        if status {
            self.sensors.arm(Instant::now());
        } else {
            self.sensors.disarm();
        }
        Ok(true)
    }

    /// État de la collecte de capteurs.
    #[zbus(name = "GetSensorDataCollectionStatus")]
    async fn get_sensor_data_collection_status(&self) -> bool {
        self.sensors.is_enabled()
    }

    /// État de la minuterie d'extinction (lecture seule, sans polkit).
    ///
    /// Limité à 2 lectures/s (burst 5) par nom unique, avec un bucket
    /// indépendant de `GetHardwareSnapshotJSON`.
    #[zbus(name = "GetShutdownTimerJSON")]
    async fn get_shutdown_timer_json(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let caller = caller_name(&header);
        let caller_uid = resolve_caller_uid(connection, &header).await;
        if let Err(error) = self.consume_shutdown_read(&caller) {
            tracing::debug!(
                caller = %caller,
                caller_uid = ?caller_uid,
                method = "GetShutdownTimerJSON",
                result = "rate_limited",
                "lecture de la minuterie rejetée"
            );
            return Err(error);
        }
        self.shutdown_timer_json()
    }

    /// Sérialisation non limitée, pour les usages internes et les tests.
    fn shutdown_timer_json(&self) -> fdo::Result<String> {
        let status = self
            .shutdown
            .lock()
            .map_err(|_| fdo::Error::Failed("état de la minuterie indisponible".to_owned()))?;
        serde_json::to_string(&*status).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Profil de charge TUXEDO (non persistant).
    #[zbus(name = "SetChargingProfile")]
    async fn set_charging_profile(
        &self,
        value: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (value, context) = self
            .prepare_write(
                &header,
                connection,
                "SetChargingProfile",
                ACTION_CHARGING,
                || {
                    validate_charge_value(&value, "profil de charge")?;
                    Ok(value)
                },
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                let target = value.clone();
                self.blocking_hw(move || ChargingController::new(root).set_profile(&target))
                    .await?;
            }
            self.update_settings(|settings| settings.charging_profile = Some(value))
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// Priorité de charge TUXEDO (non persistante).
    #[zbus(name = "SetChargingPriority")]
    async fn set_charging_priority(
        &self,
        value: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (value, context) = self
            .prepare_write(
                &header,
                connection,
                "SetChargingPriority",
                ACTION_CHARGING,
                || {
                    validate_charge_value(&value, "priorité de charge")?;
                    Ok(value)
                },
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                let target = value.clone();
                self.blocking_hw(move || ChargingController::new(root).set_priority(&target))
                    .await?;
            }
            self.update_settings(|settings| settings.charging_priority = Some(value))
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// Seuil de début de charge (0..=100).
    #[zbus(name = "SetChargeStartThreshold")]
    async fn set_charge_start_threshold(
        &self,
        value: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (value, context) = self
            .prepare_write(
                &header,
                connection,
                "SetChargeStartThreshold",
                ACTION_CHARGING,
                || percent_argument(value, "seuil de début de charge"),
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                self.blocking_hw(move || ChargingController::new(root).set_start_threshold(value))
                    .await?;
            }
            self.update_settings(|settings| settings.charging_start_threshold = Some(value))
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// Seuil de fin de charge (0..=100).
    #[zbus(name = "SetChargeEndThreshold")]
    async fn set_charge_end_threshold(
        &self,
        value: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (value, context) = self
            .prepare_write(
                &header,
                connection,
                "SetChargeEndThreshold",
                ACTION_CHARGING,
                || percent_argument(value, "seuil de fin de charge"),
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                self.blocking_hw(move || ChargingController::new(root).set_end_threshold(value))
                    .await?;
            }
            self.update_settings(|settings| settings.charging_end_threshold = Some(value))
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// Type de charge (`Standard`, `Fast`, `Custom`, ...).
    #[zbus(name = "SetChargeType")]
    async fn set_charge_type(
        &self,
        value: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (charge_type, context) = self
            .prepare_write(
                &header,
                connection,
                "SetChargeType",
                ACTION_CHARGING,
                || {
                    validate_charge_value(&value, "type de charge")?;
                    let charge_type = ChargeType::from_sysfs(&value);
                    if !charge_type.is_known() {
                        return Err(fdo::Error::InvalidArgs("type de charge inconnu".to_owned()));
                    }
                    Ok(charge_type)
                },
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                let charge_type = charge_type.clone();
                self.blocking_hw(move || {
                    ChargingController::new(root).set_charge_type(&charge_type)
                })
                .await?;
            }
            // `Settings` n'a pas de champ `charge_type` : réveille seulement
            // les workers pour rafraîchir la télémétrie.
            self.bump_revision().await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// États des zones du clavier (JSON `Vec<KeyboardState>`).
    #[zbus(name = "SetKeyboardStatesJSON")]
    async fn set_keyboard_states_json(
        &self,
        states_json: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        self.ensure_lighting_enabled("SetKeyboardStatesJSON")
            .await?;
        let (states, context) = self
            .prepare_write(
                &header,
                connection,
                "SetKeyboardStatesJSON",
                ACTION_SETTINGS,
                || parse_keyboard_states(&states_json),
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                let states_for_hw = states.clone();
                self.blocking_hw(move || apply_keyboard_states(&root, &states_for_hw))
                    .await?;
            }
            self.update_settings(|settings| settings.keyboard_states = states)
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// État de la lightbar (JSON `LightbarState` ou `null`).
    #[zbus(name = "SetLightbarStatesJSON")]
    async fn set_lightbar_states_json(
        &self,
        state_json: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        self.ensure_lighting_enabled("SetLightbarStatesJSON")
            .await?;
        let (state, context) = self
            .prepare_write(
                &header,
                connection,
                "SetLightbarStatesJSON",
                ACTION_SETTINGS,
                || parse_lightbar_state(&state_json),
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                if let Some(state) = &state {
                    let root = self.root.clone();
                    let state_for_hw = state.clone();
                    self.blocking_hw(move || apply_lightbar_state(&root, &state_for_hw))
                        .await?;
                }
            }
            self.update_settings(|settings| settings.lightbar_states = state)
                .await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }

    /// Active/désactive le Fn-Lock (global, non persistant).
    #[zbus(name = "SetFnLockStatus")]
    async fn set_fn_lock_status(
        &self,
        enabled: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<bool> {
        let (enabled, context) = self
            .prepare_write(
                &header,
                connection,
                "SetFnLockStatus",
                ACTION_SETTINGS,
                || Ok(enabled),
            )
            .await?;
        let result: fdo::Result<bool> = async {
            if !self.dry_run {
                let root = self.root.clone();
                self.blocking_hw(move || FnLock::new(root).set(enabled))
                    .await?;
            }
            self.bump_revision().await;
            Ok(true)
        }
        .await;
        self.audit_completion(&context, result.is_ok());
        result
    }
}

fn validate_charge_value(value: &str, field: &'static str) -> fdo::Result<()> {
    if value.is_empty()
        || value.len() > MAX_CHARGE_VALUE_LEN
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(fdo::Error::InvalidArgs(format!("{field} invalide")));
    }
    Ok(())
}

fn percent_argument(value: i32, field: &'static str) -> fdo::Result<u8> {
    if !(0..=100).contains(&value) {
        return Err(fdo::Error::InvalidArgs(format!(
            "{field} hors plage (0..=100) : {value}"
        )));
    }
    Ok(value as u8)
}

fn parse_keyboard_states(json: &str) -> fdo::Result<Vec<KeyboardState>> {
    if json.len() > MAX_LIGHTING_JSON_BYTES {
        return Err(fdo::Error::InvalidArgs(
            "keyboardStates: payload trop volumineux".to_owned(),
        ));
    }
    let states: Vec<KeyboardState> = serde_json::from_str(json)
        .map_err(|error| fdo::Error::InvalidArgs(format!("keyboardStates: {error}")))?;
    if states.len() > MAX_KEYBOARD_STATES {
        return Err(fdo::Error::InvalidArgs(format!(
            "keyboardStates: au plus {MAX_KEYBOARD_STATES} zones"
        )));
    }
    for (index, state) in states.iter().enumerate() {
        for (name, value) in [
            ("red", state.red),
            ("green", state.green),
            ("blue", state.blue),
        ] {
            if !(-1..=255).contains(&value) {
                return Err(fdo::Error::InvalidArgs(format!(
                    "keyboardStates[{index}].{name}: -1 ou 0..=255 attendu"
                )));
            }
        }
    }
    Ok(states)
}

fn parse_lightbar_state(json: &str) -> fdo::Result<Option<LightbarState>> {
    if json.len() > MAX_LIGHTING_JSON_BYTES {
        return Err(fdo::Error::InvalidArgs(
            "lightbarStates: payload trop volumineux".to_owned(),
        ));
    }
    let state: Option<LightbarState> = serde_json::from_str(json)
        .map_err(|error| fdo::Error::InvalidArgs(format!("lightbarStates: {error}")))?;
    if let Some(state) = &state {
        for (name, value) in [
            ("red", state.red),
            ("green", state.green),
            ("blue", state.blue),
        ] {
            if !(0..=255).contains(&value) {
                return Err(fdo::Error::InvalidArgs(format!(
                    "lightbarStates.{name}: 0..=255 attendu"
                )));
            }
        }
    }
    Ok(state)
}

fn apply_keyboard_states(root: &SysfsRoot, states: &[KeyboardState]) -> Result<(), HwError> {
    let Some(first) = states.first() else {
        return Ok(());
    };
    let controller = LightingController::detect(root.clone())?;
    let colors: Vec<RgbColor> = states
        .iter()
        .map(|state| {
            RgbColor::new(
                clamp_color(state.red),
                clamp_color(state.green),
                clamp_color(state.blue),
            )
        })
        .collect();
    let report = controller.apply_keyboard(first.brightness, &colors)?;
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(HwError::Display(report.errors.join("; ")))
    }
}

fn apply_lightbar_state(root: &SysfsRoot, state: &LightbarState) -> Result<(), HwError> {
    let controller = LightingController::detect(root.clone())?;
    controller.apply_lightbar(
        state.brightness,
        RgbColor::new(
            clamp_color(state.red),
            clamp_color(state.green),
            clamp_color(state.blue),
        ),
    )
}

fn clamp_color(value: i16) -> u8 {
    value.clamp(0, 255) as u8
}

/// Convertit une erreur matérielle en erreur D-Bus **générique**.
///
/// Le détail (chemin sysfs, valeur, ioctl) reste dans journald ; il ne doit
/// jamais atteindre le client (M1-03).
fn map_hw_error(error: HwError) -> fdo::Error {
    tracing::warn!(%error, "erreur matérielle : détail conservé côté daemon");
    match error {
        HwError::UnsafeTarget { .. } | HwError::UnsafePermissions { .. } => {
            fdo::Error::AccessDenied("accès matériel refusé".to_owned())
        }
        HwError::InvalidValue { .. } | HwError::OutOfRange { .. } | HwError::InvalidPath { .. } => {
            fdo::Error::InvalidArgs("entrée matérielle invalide".to_owned())
        }
        HwError::NotFound { .. }
        | HwError::Unavailable { .. }
        | HwError::SensorUnavailable { .. }
        | HwError::UnsupportedDevice
        | HwError::NotSupported(_)
        | HwError::NoDisplayBackend => fdo::Error::NotSupported("matériel indisponible".to_owned()),
        _ => fdo::Error::Failed("erreur matérielle".to_owned()),
    }
}

/// Bus utilisé par le daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusType {
    /// Bus système de production.
    #[default]
    System,
    /// Bus de session, uniquement pour le développement sans root.
    Session,
}

/// Demande de rechargement adressée à la boucle principale du daemon.
///
/// Le handler D-Bus ne recharge jamais lui-même : il envoie la demande et
/// attend le rapport. Cela évite de prendre les verrous d'état depuis une
/// tâche zbus et garde une seule boucle de réconciliation.
#[derive(Debug)]
pub struct ReloadRequest {
    /// Canal de réponse, alimenté par la boucle principale.
    pub reply: tokio::sync::oneshot::Sender<AdminReloadReport>,
}

/// Délai maximal d'attente du rapport de rechargement.
const RELOAD_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Interface `Control1`, strictement réservée à root.
pub struct Control {
    reload_tx: tokio::sync::mpsc::Sender<ReloadRequest>,
    /// Sérialise les rechargements concurrents. Aucun rapport n'est mémorisé :
    /// chaque appel doit relire les fichiers sur disque (A1-01).
    reload_lock: tokio::sync::Mutex<()>,
}

impl Control {
    /// Construit l'interface de contrôle autour du canal de la boucle
    /// principale.
    #[must_use]
    pub fn new(reload_tx: tokio::sync::mpsc::Sender<ReloadRequest>) -> Self {
        Self {
            reload_tx,
            reload_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Demande un rechargement réel et attend son propre rapport.
    ///
    /// Le verrou est tenu pendant l'attente : deux appels concurrents sont
    /// sérialisés, mais le second déclenche bien un nouveau chargement. Un
    /// rapport n'est jamais réutilisé, même si les appels sont rapprochés :
    /// l'admin peut écrire entre deux appels (A1-01).
    async fn request_reload(&self) -> AdminReloadReport {
        let _guard = self.reload_lock.lock().await;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        if self.reload_tx.send(ReloadRequest { reply }).await.is_err() {
            return AdminReloadReport::from_warnings(vec![
                "boucle de rechargement arrêtée".to_owned(),
            ]);
        }

        match tokio::time::timeout(RELOAD_REPLY_TIMEOUT, receiver).await {
            Ok(Ok(report)) => report,
            Ok(Err(_)) => {
                AdminReloadReport::from_warnings(vec!["rechargement interrompu".to_owned()])
            }
            Err(_) => AdminReloadReport::from_warnings(vec!["rechargement expiré".to_owned()]),
        }
    }
}

/// Décision root-only, testable sans bus.
///
/// Un UID inconnu (`None`) est refusé : l'absence d'identité n'autorise
/// jamais.
fn is_root_caller(caller_uid: Option<u32>) -> bool {
    caller_uid == Some(0)
}

#[zbus::interface(name = "com.tuxedocomputers.tccd.Control1")]
impl Control {
    /// Recharge profils/réglages depuis `/var/lib` et réapplique le profil.
    ///
    /// Réponse : [`AdminReloadReport`] JSON. `ok = false` dès qu'un
    /// avertissement de chargement est présent. Réservé à l'UID 0 : la
    /// politique D-Bus est doublée d'une vérification `GetConnectionUnixUser`
    /// côté daemon (défense en profondeur). Chaque appel déclenche un
    /// rechargement réel ; les appels concurrents sont sérialisés, mais aucun
    /// rapport n'est réutilisé.
    #[zbus(name = "ReloadConfig")]
    async fn reload_config(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let caller = caller_name(&header);
        let caller_uid = resolve_caller_uid(connection, &header).await;
        if !is_root_caller(caller_uid) {
            // Pas de détail : ne pas révéler l'UID résolu ni le motif.
            tracing::warn!(
                caller = %caller,
                caller_uid = ?caller_uid,
                result = "denied",
                "Control1.ReloadConfig refusé (root requis)"
            );
            return Err(fdo::Error::AccessDenied(String::new()));
        }

        let report = self.request_reload().await;
        tracing::info!(
            caller = %caller,
            caller_uid = caller_uid,
            result = if report.is_ok() { "ok" } else { "warning" },
            "Control1.ReloadConfig"
        );
        report
            .to_json()
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }
}

/// Construit et enregistre le service.
///
/// Retourne aussi le récepteur des demandes `Control1.ReloadConfig` : la
/// boucle principale doit le servir pour que l'admin obtienne son rapport.
///
/// # Errors
///
/// Renvoie une erreur si le bus est inaccessible ou si le nom est déjà pris
/// (par exemple l'ancien TCC tourne encore sur le bus système).
pub async fn serve(
    state: Arc<RwLock<RuntimeState>>,
    notify: Arc<Notify>,
    fan: Arc<std::sync::RwLock<FanData>>,
    hardware: HardwareBridge,
    service: &str,
    bus: BusType,
) -> anyhow::Result<(zbus::Connection, tokio::sync::mpsc::Receiver<ReloadRequest>)> {
    let interface = TccDaemon::new(state.clone(), notify.clone(), fan);
    let supervisor_lease = Arc::clone(&hardware.sensors);
    let hardware_manager = HardwareManager::new(state, notify, hardware);
    let (reload_tx, reload_rx) = tokio::sync::mpsc::channel::<ReloadRequest>(8);
    let control = Control::new(reload_tx);
    // Une seule tâche de supervision (1 s), détachée avec le runtime.
    crate::sensor_lease::spawn_supervisor(supervisor_lease);
    let builder = match bus {
        BusType::System => zbus::connection::Builder::system()?,
        BusType::Session => zbus::connection::Builder::session()?,
    };
    let connection = builder
        .name(service.to_owned())?
        .serve_at(DBUS_PATH, interface)?
        .serve_at(DBUS_PATH, hardware_manager)?
        .serve_at(DBUS_PATH, control)?
        .build()
        .await?;
    tracing::info!(
        service,
        path = DBUS_PATH,
        interface = DBUS_INTERFACE,
        manager_interface = DBUS_MANAGER_INTERFACE,
        control_interface = DBUS_CONTROL_INTERFACE,
        bus = ?bus,
        "service D-Bus enregistré"
    );
    Ok((connection, reload_rx))
}

/// Transmet les snapshots du daemon vers le signal D-Bus `StateChanged`.
///
/// Une erreur ponctuelle d'émission est journalisée mais ne doit pas arrêter
/// le daemon ; un client absent n'est pas une condition fatale.
pub fn spawn_state_forwarder(
    connection: &zbus::Connection,
    mut snapshots: broadcast::Receiver<String>,
) {
    let connection = connection.clone();
    tokio::spawn(async move {
        loop {
            match snapshots.recv().await {
                Ok(json) => {
                    match connection
                        .object_server()
                        .interface::<_, TccDaemon>(DBUS_PATH)
                        .await
                    {
                        Ok(interface) => {
                            if let Err(error) =
                                TccDaemon::state_changed(interface.signal_emitter(), &json).await
                            {
                                tracing::warn!(%error, "émission de StateChanged échouée");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "interface D-Bus introuvable pour le signal");
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "signal StateChanged en retard, messages ignorés");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use tcc_core::{FanTableSet, Settings, default_profiles};
    use tcc_ipc::IPC_CONTRACT_VERSION;

    use crate::config::LoadedConfig;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Decision {
        Allow,
        Deny,
        Unavailable,
    }

    #[derive(Debug, Default)]
    struct RecordingAuditSink {
        events: Mutex<Vec<AuditCompletion>>,
    }

    impl AuditSink for RecordingAuditSink {
        fn completion(&self, event: AuditCompletion) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }
    }

    impl RecordingAuditSink {
        fn events(&self) -> Vec<AuditCompletion> {
            self.events
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    #[derive(Debug)]
    struct FakeAuthorizer {
        decision: Decision,
        calls: Mutex<Vec<(String, u32, &'static str)>>,
    }

    impl FakeAuthorizer {
        fn new(decision: Decision) -> Self {
            Self {
                decision,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, u32, &'static str)> {
            self.calls
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl SettersAuthorizer for FakeAuthorizer {
        async fn authorize(&self, caller: &str, uid: u32, action: &'static str) -> fdo::Result<()> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((caller.to_owned(), uid, action));
            }
            match self.decision {
                Decision::Allow => Ok(()),
                Decision::Deny => Err(fdo::Error::AccessDenied(String::new())),
                Decision::Unavailable => Err(fdo::Error::Failed("polkit absent".to_owned())),
            }
        }
    }

    async fn authorized_manager(
        authorizer: Arc<dyn SettersAuthorizer>,
    ) -> Option<(
        zbus::Connection,
        zbus::Connection,
        Arc<RwLock<RuntimeState>>,
    )> {
        authorized_manager_with_sleeping(authorizer, false).await
    }

    async fn authorized_manager_with_sleeping(
        authorizer: Arc<dyn SettersAuthorizer>,
        sleeping: bool,
    ) -> Option<(
        zbus::Connection,
        zbus::Connection,
        Arc<RwLock<RuntimeState>>,
    )> {
        let (service, client, state, _audit) =
            authorized_manager_recorded(authorizer, sleeping, true).await?;
        Some((service, client, state))
    }

    async fn authorized_manager_recorded(
        authorizer: Arc<dyn SettersAuthorizer>,
        sleeping: bool,
        dry_run: bool,
    ) -> Option<(
        zbus::Connection,
        zbus::Connection,
        Arc<RwLock<RuntimeState>>,
        Arc<RecordingAuditSink>,
    )> {
        let service = zbus::Connection::session().await.ok()?;
        let state = state();
        let audit = Arc::new(RecordingAuditSink::default());
        let manager = HardwareManager::new(
            state.clone(),
            Arc::new(Notify::new()),
            HardwareBridge::test_with_authorizer_and_audit(
                Arc::new(crate::hardware::HardwareStatus::default()),
                Arc::new(crate::sensor_lease::SensorLease::new()),
                SysfsRoot::new("/nonexistent-tcc-test"),
                dry_run,
                authorizer,
                Arc::new(AtomicBool::new(sleeping)),
                audit.clone(),
                Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
            ),
        );
        service.object_server().at(DBUS_PATH, manager).await.ok()?;
        let client = zbus::Connection::session().await.ok()?;
        Some((service, client, state, audit))
    }

    async fn manager_proxy(
        client: &zbus::Connection,
        service: &zbus::Connection,
    ) -> Option<zbus::Proxy<'static>> {
        let destination = service.unique_name()?.to_string();
        zbus::Proxy::new_owned(
            client.clone(),
            destination,
            DBUS_PATH.to_owned(),
            DBUS_MANAGER_INTERFACE.to_owned(),
        )
        .await
        .ok()
    }

    fn error_name(error: &zbus::Error) -> String {
        match error {
            zbus::Error::MethodError(name, _, _) => name.to_string(),
            other => panic!("erreur D-Bus inattendue: {other:?}"),
        }
    }

    fn state() -> Arc<RwLock<RuntimeState>> {
        let config = LoadedConfig {
            store: tcc_core::ConfigStore::with_admin_roots("/tmp/state", "/tmp/admin"),
            settings: Settings::default(),
            profiles: default_profiles(),
            fan_tables: FanTableSet::default(),
            warnings: Vec::new(),
            report_warnings: Vec::new(),
        };
        Arc::new(RwLock::new(RuntimeState::new(&config)))
    }

    fn fan() -> Arc<std::sync::RwLock<FanData>> {
        Arc::new(std::sync::RwLock::new(FanData::unavailable()))
    }

    #[tokio::test]
    async fn exposes_contract_version() {
        let interface = TccDaemon::new(state(), Arc::new(Notify::new()), fan());
        assert_eq!(interface.contract_version(), IPC_CONTRACT_VERSION);
        assert!(interface.ping().await);
        assert!(interface.get_state_json().await.is_ok());
    }

    #[tokio::test]
    async fn shutdown_timer_status_is_exposed_read_only() {
        let status = Arc::new(std::sync::Mutex::new(ShutdownTimerStatus {
            enabled: true,
            scheduled_at: Some("2026-11-06 22:30".to_owned()),
            source: crate::shutdown_timer::TimerSource::Settings,
            error: None,
        }));
        let manager = HardwareManager::new(
            state(),
            Arc::new(Notify::new()),
            HardwareBridge::test_with_authorizer(
                Arc::new(crate::hardware::HardwareStatus::default()),
                Arc::new(crate::sensor_lease::SensorLease::new()),
                SysfsRoot::new("/nonexistent-tcc-test"),
                true,
                Arc::new(DevSessionAuthorizer),
                Arc::new(AtomicBool::new(false)),
                status,
            ),
        );
        let json = manager.shutdown_timer_json().expect("timer JSON");
        let decoded: ShutdownTimerStatus = serde_json::from_str(&json).expect("JSON");
        assert!(decoded.enabled);
        assert_eq!(decoded.scheduled_at.as_deref(), Some("2026-11-06 22:30"));
        assert_eq!(decoded.source, crate::shutdown_timer::TimerSource::Settings);
        assert_eq!(decoded.error, None);
    }

    #[test]
    fn production_bridge_is_polkit_only() {
        let bridge = HardwareBridge::production(
            Arc::new(HardwareStatus::default()),
            Arc::new(SensorLease::new()),
            SysfsRoot::new("/nonexistent-tcc-test"),
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
        );
        assert!(!bridge.dry_run);
        assert!(bridge.authorizer.is_none());
    }

    #[test]
    fn dev_session_bridge_forces_dry_run_and_authorizer() {
        let bridge = HardwareBridge::dev_session(
            Arc::new(HardwareStatus::default()),
            Arc::new(SensorLease::new()),
            SysfsRoot::new("/nonexistent-tcc-test"),
            Arc::new(AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
        );
        assert!(bridge.dry_run, "le mode dev doit rester dry-run");
        assert!(bridge.authorizer.is_some());
    }

    #[tokio::test]
    async fn manager_exposes_contract_version() {
        let manager = HardwareManager::new(
            state(),
            Arc::new(Notify::new()),
            HardwareBridge::test_with_authorizer(
                Arc::new(crate::hardware::HardwareStatus::default()),
                Arc::new(crate::sensor_lease::SensorLease::new()),
                SysfsRoot::new("/nonexistent-tcc-test"),
                true,
                Arc::new(DevSessionAuthorizer),
                Arc::new(AtomicBool::new(false)),
                Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
            ),
        );
        assert_eq!(manager.get_contract_version().await, IPC_CONTRACT_VERSION);
    }

    #[tokio::test]
    async fn exposes_fan_data_even_without_hardware() {
        let interface = TccDaemon::new(state(), Arc::new(Notify::new()), fan());
        let json = interface.get_fan_data_json().await.expect("fan data");
        let parsed: FanData = serde_json::from_str(&json).expect("fan JSON");
        assert!(!parsed.available);
        assert_eq!(parsed.mode, crate::fan::FanMode::Off);
        assert!(parsed.fans.is_empty());
    }

    #[test]
    fn profile_id_is_bounded_before_resolution() {
        assert!(validate_profile_id_argument("").is_err());
        assert!(validate_profile_id_argument(&"a".repeat(MAX_PROFILE_ID_LEN)).is_ok());
        assert!(validate_profile_id_argument(&"a".repeat(MAX_PROFILE_ID_LEN + 1)).is_err());
        // La borne est en octets : 33 caractères UTF-8 de 2 octets dépassent 64.
        assert!(validate_profile_id_argument(&"é".repeat(33)).is_err());
    }

    #[test]
    fn write_rate_limit_burst_then_limits_then_other_caller() {
        let interface = TccDaemon::new(state(), Arc::new(Notify::new()), fan());
        for _ in 0..10 {
            assert!(interface.consume_write(":1.42").is_ok());
        }
        assert!(matches!(
            interface.consume_write(":1.42"),
            Err(fdo::Error::LimitsExceeded(_))
        ));
        // Un autre nom unique n'est pas pénalisé.
        assert!(interface.consume_write(":1.43").is_ok());
    }

    #[test]
    fn hardware_snapshot_rate_limit_does_not_rearm_lease() {
        let lease = Arc::new(crate::sensor_lease::SensorLease::with_ttl(
            Duration::from_secs(10),
        ));
        let before = Instant::now();
        lease.arm(before);
        let manager = HardwareManager::new(
            state(),
            Arc::new(Notify::new()),
            HardwareBridge::test_with_authorizer(
                Arc::new(crate::hardware::HardwareStatus::default()),
                lease.clone(),
                SysfsRoot::new("/nonexistent-tcc-test"),
                true,
                Arc::new(DevSessionAuthorizer),
                Arc::new(AtomicBool::new(false)),
                Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
            ),
        );

        // Burst de 5 lectures accepté ; chaque lecture réarme le bail.
        for _ in 0..5 {
            let json = manager.snapshot_for_caller(":1.test").expect("snapshot");
            assert!(json.contains("\"sensors\""));
        }
        let deadline_before = lease.deadline_for_test().expect("deadline armé");
        assert!(deadline_before >= before + Duration::from_secs(10));
        assert!(lease.is_enabled());

        // 6e lecture refusée : le bail n'est pas réarmé.
        assert!(matches!(
            manager.snapshot_for_caller(":1.test"),
            Err(fdo::Error::LimitsExceeded(_))
        ));
        assert_eq!(lease.deadline_for_test(), Some(deadline_before));

        // Un autre nom unique possède son propre bucket.
        assert!(manager.snapshot_for_caller(":1.other").is_ok());
    }

    #[tokio::test]
    async fn serves_methods_on_session_bus() {
        let Ok(connection) = zbus::connection::Builder::session()
            .and_then(|builder| {
                builder.serve_at(
                    DBUS_PATH,
                    TccDaemon::new(state(), Arc::new(Notify::new()), fan()),
                )
            })
            .map(|builder| builder.build())
        else {
            return;
        };
        let Ok(connection) = connection.await else {
            // Pas de bus de session (CI headless) : test non applicable.
            return;
        };
        let Some(unique_name) = connection.unique_name().map(ToString::to_string) else {
            return;
        };

        let proxy = zbus::Proxy::new(
            &connection,
            unique_name,
            DBUS_PATH,
            "com.tuxedocomputers.tccd",
        )
        .await
        .expect("proxy");

        let state_json: String = proxy.call("GetStateJSON", &()).await.expect("GetStateJSON");
        let snapshot: tcc_ipc::StateSnapshot =
            serde_json::from_str(&state_json).expect("snapshot JSON");
        assert_eq!(snapshot.contract_version, IPC_CONTRACT_VERSION);

        let fan_json: String = proxy
            .call("GetFanDataJSON", &())
            .await
            .expect("GetFanDataJSON");
        assert!(fan_json.contains("\"available\""));

        let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
        assert!(pong);
    }

    #[tokio::test]
    async fn dry_run_production_service_owns_name_and_serves_reads() {
        // Régression : `--dry-run` était parsé comme `--check`, donc le
        // processus sortait avant d'enregistrer le nom D-Bus. Ici le pont de
        // production en dry-run doit posséder son nom et répondre aux
        // lectures. Le bus de session remplace le bus système (CI sans root).
        let name = format!("com.tuxedocomputers.tccd.TestDryRun.{}", std::process::id());
        let bridge = HardwareBridge::production(
            Arc::new(crate::hardware::HardwareStatus::default()),
            Arc::new(crate::sensor_lease::SensorLease::new()),
            SysfsRoot::new("/nonexistent-tcc-test"),
            true,
            Arc::new(AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(ShutdownTimerStatus::default())),
        );
        let Ok((connection, _reload)) = serve(
            state(),
            Arc::new(Notify::new()),
            fan(),
            bridge,
            &name,
            BusType::Session,
        )
        .await
        else {
            // Pas de bus de session (CI headless) : test non applicable.
            return;
        };

        let dbus = zbus::fdo::DBusProxy::new(&connection)
            .await
            .expect("proxy D-Bus");
        let bus_name = zbus::names::BusName::try_from(name.clone()).expect("nom D-Bus");
        assert!(
            dbus.name_has_owner(bus_name).await.expect("NameHasOwner"),
            "le service dry-run doit posséder son nom"
        );

        let proxy = zbus::Proxy::new(&connection, name, DBUS_PATH, DBUS_INTERFACE)
            .await
            .expect("proxy TccDaemon");
        let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
        assert!(pong);
        let state_json: String = proxy.call("GetStateJSON", &()).await.expect("GetStateJSON");
        assert!(state_json.contains("contractVersion"));
    }

    #[derive(Debug)]
    struct BlockingAuthorizer {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl SettersAuthorizer for BlockingAuthorizer {
        async fn authorize(
            &self,
            _caller: &str,
            _uid: u32,
            _action: &'static str,
        ) -> fdo::Result<()> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn setter_is_refused_while_sleeping() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, state)) =
            authorized_manager_with_sleeping(authorizer.clone(), true).await
        else {
            eprintln!("bus de session absent : test setter pendant veille ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };
        let before = state.read().await.revision;
        let result: Result<bool, zbus::Error> = proxy.call("SetFnLockStatus", &(true,)).await;
        assert_eq!(
            error_name(&result.expect_err("veille en cours")),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
        assert_eq!(state.read().await.revision, before);
        assert!(
            authorizer.calls().is_empty(),
            "aucun dialogue pendant la veille"
        );
    }

    #[tokio::test]
    async fn completion_audit_records_success() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, _state, audit)) =
            authorized_manager_recorded(authorizer, false, true).await
        else {
            eprintln!("bus de session absent : test audit ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };
        let result: Result<bool, zbus::Error> = proxy.call("SetFnLockStatus", &(true,)).await;
        assert!(result.expect("setter autorisé"));
        let events = audit.events();
        let event = events.last().expect("événement de complétion");
        assert_eq!(event.result, "ok");
        assert_eq!(event.method, "SetFnLockStatus");
        assert_eq!(event.action, ACTION_SETTINGS);
        assert!(event.caller.starts_with(':'));
        assert_ne!(event.caller_uid, u32::MAX);
        assert!(event.duration_ms < 5_000);
    }

    #[tokio::test]
    async fn completion_audit_records_hardware_error() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        // dry-run désactivé : le chemin matériel est exercé sur une racine
        // inexistante, donc l'écriture échoue.
        let Some((service, client, _state, audit)) =
            authorized_manager_recorded(authorizer, false, false).await
        else {
            eprintln!("bus de session absent : test audit erreur ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };
        let result: Result<bool, zbus::Error> =
            proxy.call("SetChargingProfile", &("balanced",)).await;
        assert!(result.is_err(), "l'écriture doit échouer sans matériel");
        let events = audit.events();
        let event = events.last().expect("événement de complétion");
        assert_eq!(event.result, "error");
        assert_eq!(event.method, "SetChargingProfile");
        assert_eq!(event.action, ACTION_CHARGING);
    }

    #[tokio::test]
    async fn shutdown_timer_read_is_rate_limited() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, _state)) = authorized_manager(authorizer).await else {
            eprintln!("bus de session absent : test rate limit minuterie ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };

        for index in 0..5 {
            let result: Result<String, zbus::Error> = proxy.call("GetShutdownTimerJSON", &()).await;
            assert!(result.is_ok(), "lecture {index} refusée dans le burst");
        }
        let limited: Result<String, zbus::Error> = proxy.call("GetShutdownTimerJSON", &()).await;
        assert_eq!(
            error_name(&limited.expect_err("rafale de lectures")),
            "org.freedesktop.DBus.Error.LimitsExceeded"
        );

        // Bucket indépendant : le snapshot reste lisible malgré la limite.
        let snapshot: Result<String, zbus::Error> =
            proxy.call("GetHardwareSnapshotJSON", &()).await;
        assert!(snapshot.is_ok(), "le snapshot ne doit pas être impacté");

        // Refill à 2/s : une lecture redevient possible.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let next: Result<String, zbus::Error> = proxy.call("GetShutdownTimerJSON", &()).await;
        assert!(
            next.is_ok(),
            "la lecture légitime suivante doit être acceptée"
        );
    }

    #[tokio::test]
    async fn uid_bucket_limits_multiple_connections() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client_one, _state)) = authorized_manager(authorizer.clone()).await
        else {
            eprintln!("bus de session absent : test quota UID ignoré");
            return;
        };
        let Ok(client_two) = zbus::Connection::session().await else {
            return;
        };
        let Some(proxy_one) = manager_proxy(&client_one, &service).await else {
            panic!("proxy 1");
        };
        let Some(proxy_two) = manager_proxy(&client_two, &service).await else {
            panic!("proxy 2");
        };

        let mut accepted = 0usize;
        let mut limited = 0usize;
        for index in 0..12usize {
            let proxy = if index % 2 == 0 {
                &proxy_one
            } else {
                &proxy_two
            };
            let result: Result<bool, zbus::Error> =
                proxy.call("SetFnLockStatus", &(index % 2 == 0,)).await;
            match result {
                Ok(_) => accepted += 1,
                Err(error) if error_name(&error) == "org.freedesktop.DBus.Error.LimitsExceeded" => {
                    limited += 1;
                }
                Err(error) => panic!("erreur inattendue: {error}"),
            }
        }
        assert_eq!(accepted, 10, "burst partagé par UID");
        assert!(
            limited >= 1,
            "les connexions multiples doivent être limitées"
        );
        assert_eq!(authorizer.calls().len(), accepted);
    }

    #[tokio::test]
    async fn concurrent_dialog_is_refused_by_gate() {
        let authorizer = Arc::new(BlockingAuthorizer {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        });
        let Some((service, client_one, _state)) = authorized_manager(authorizer.clone()).await
        else {
            eprintln!("bus de session absent : test DialogGate ignoré");
            return;
        };
        let Ok(client_two) = zbus::Connection::session().await else {
            return;
        };
        let Some(proxy_one) = manager_proxy(&client_one, &service).await else {
            panic!("proxy 1");
        };
        let Some(proxy_two) = manager_proxy(&client_two, &service).await else {
            panic!("proxy 2");
        };

        let first = proxy_one.call::<_, _, bool>("SetFnLockStatus", &(true,));
        tokio::pin!(first);
        tokio::select! {
            _ = authorizer.entered.notified() => {}
            result = &mut first => panic!("premier dialogue terminé trop tôt: {result:?}"),
        }

        let second: Result<bool, zbus::Error> = proxy_two.call("SetFnLockStatus", &(false,)).await;
        let error = second.expect_err("second dialogue doit être refusé");
        assert_eq!(
            error_name(&error),
            "org.freedesktop.DBus.Error.AccessDenied"
        );

        authorizer.release.notify_one();
        assert!(first.await.expect("premier setter"));
    }

    #[tokio::test]
    async fn lighting_switch_disables_keyboard_and_lightbar_setters() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test interrupteur lighting ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };
        state
            .write()
            .await
            .settings
            .keyboard_backlight_control_enabled = false;

        let keyboard = r#"[{"mode":0,"brightness":10,"red":1,"green":2,"blue":3}]"#;
        let keyboard_result: Result<bool, zbus::Error> =
            proxy.call("SetKeyboardStatesJSON", &(keyboard,)).await;
        assert_eq!(
            error_name(&keyboard_result.expect_err("clavier refusé")),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
        let lightbar_result: Result<bool, zbus::Error> =
            proxy.call("SetLightbarStatesJSON", &("null",)).await;
        assert_eq!(
            error_name(&lightbar_result.expect_err("lightbar refusée")),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
        assert!(
            authorizer.calls().is_empty(),
            "aucun dialogue ne doit être ouvert"
        );
        assert!(state.read().await.settings.keyboard_states.is_empty());

        // Fn-Lock est indépendant du rétroéclairage : il reste autorisé.
        let fn_lock: Result<bool, zbus::Error> = proxy.call("SetFnLockStatus", &(true,)).await;
        assert!(fn_lock.expect("Fn-Lock indépendant"));
        assert_eq!(authorizer.calls().len(), 1);

        state
            .write()
            .await
            .settings
            .keyboard_backlight_control_enabled = true;
        let accepted: Result<bool, zbus::Error> =
            proxy.call("SetKeyboardStatesJSON", &(keyboard,)).await;
        assert!(accepted.expect("clavier autorisé"));
        assert_eq!(state.read().await.settings.keyboard_states.len(), 1);
        assert_eq!(authorizer.calls().len(), 2);
        assert_eq!(authorizer.calls()[1].2, ACTION_SETTINGS);
    }

    #[test]
    fn hw_error_mapping_hides_paths() {
        for error in [
            HwError::InvalidPath {
                path: std::path::PathBuf::from("/sys/class/backlight/foo"),
            },
            HwError::NotFound {
                path: std::path::PathBuf::from("/sys/class/leds/bar"),
            },
            HwError::Io {
                path: std::path::PathBuf::from("/sys/devices/platform/x"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            },
        ] {
            let mapped = map_hw_error(error);
            let detail = format!("{mapped:?}");
            assert!(!detail.contains("/sys"), "chemin fuité: {detail}");
            assert!(
                !detail.contains('/'),
                "séparateur de chemin fuité: {detail}"
            );
        }
    }

    #[tokio::test]
    async fn setter_denied_by_polkit_does_not_write() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Deny));
        let Some((service, client, state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test polkit Manager1 ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };

        let result: Result<bool, zbus::Error> =
            proxy.call("SetChargingProfile", &("balanced",)).await;
        let error = result.expect_err("le refus polkit doit être renvoyé");
        assert_eq!(
            error_name(&error),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
        assert_eq!(state.read().await.settings.charging_profile, None);

        let calls = authorizer.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, ACTION_CHARGING);
        assert!(calls[0].0.starts_with(':'));
    }

    #[tokio::test]
    async fn allowed_setter_writes_and_audits_action() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test polkit Manager1 ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };

        let result: Result<bool, zbus::Error> =
            proxy.call("SetChargingProfile", &("balanced",)).await;
        assert!(result.expect("setter autorisé"));
        assert_eq!(
            state.read().await.settings.charging_profile.as_deref(),
            Some("balanced")
        );
        let calls = authorizer.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, ACTION_CHARGING);
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_before_polkit() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, _state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test polkit Manager1 ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };

        let result: Result<bool, zbus::Error> =
            proxy.call("SetChargeStartThreshold", &(150i32,)).await;
        let error = result.expect_err("valeur hors plage");
        assert_eq!(error_name(&error), "org.freedesktop.DBus.Error.InvalidArgs");
        assert!(
            authorizer.calls().is_empty(),
            "une entrée invalide ne doit pas déclencher polkit"
        );
    }

    #[tokio::test]
    async fn unavailable_polkit_fails_closed() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Unavailable));
        let Some((service, client, state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test polkit Manager1 ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };
        let before = state.read().await.revision;

        let result: Result<bool, zbus::Error> = proxy.call("SetFnLockStatus", &(true,)).await;
        let error = result.expect_err("polkit indisponible");
        assert_eq!(
            error_name(&error),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
        assert_eq!(state.read().await.revision, before);
        assert_eq!(authorizer.calls()[0].2, ACTION_SETTINGS);
    }

    #[tokio::test]
    async fn lighting_setters_use_settings_action() {
        let authorizer = Arc::new(FakeAuthorizer::new(Decision::Allow));
        let Some((service, client, _state)) = authorized_manager(authorizer.clone()).await else {
            eprintln!("bus de session absent : test polkit Manager1 ignoré");
            return;
        };
        let Some(proxy) = manager_proxy(&client, &service).await else {
            panic!("proxy Manager1");
        };

        let result: Result<bool, zbus::Error> = proxy.call("SetFnLockStatus", &(true,)).await;
        assert!(result.expect("Fn-Lock autorisé"));
        let calls = authorizer.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, ACTION_SETTINGS);
    }

    #[test]
    fn control_root_check_rejects_unknown_and_non_root() {
        assert!(is_root_caller(Some(0)));
        assert!(!is_root_caller(Some(1000)));
        assert!(!is_root_caller(None));
    }

    #[tokio::test]
    async fn control_reload_always_requests_a_fresh_report() {
        let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<ReloadRequest>(8);
        let server = tokio::spawn(async move {
            let mut calls = 0usize;
            while let Some(request) = reload_rx.recv().await {
                calls += 1;
                let _ = request
                    .reply
                    .send(AdminReloadReport::from_warnings(vec![format!(
                        "rechargement {calls}"
                    )]));
            }
            calls
        });

        let control = Control::new(reload_tx);
        let first = control.request_reload().await;
        let second = control.request_reload().await;
        assert_ne!(
            first, second,
            "chaque appel doit produire son propre rapport, jamais un cache"
        );
        drop(control);
        assert_eq!(server.await.expect("serveur de test"), 2);
    }

    #[tokio::test]
    async fn control_reload_reflects_a_write_between_calls() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // Le serveur simule la boucle de rechargement : le rapport dépend de
        // l'état courant, comme un fichier modifié entre deux appels.
        let invalid = std::sync::Arc::new(AtomicBool::new(false));
        let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<ReloadRequest>(8);
        let server_invalid = invalid.clone();
        let server = tokio::spawn(async move {
            let mut calls = 0usize;
            while let Some(request) = reload_rx.recv().await {
                calls += 1;
                let report = if server_invalid.load(Ordering::SeqCst) {
                    AdminReloadReport::from_warnings(vec!["invalidé".to_owned()])
                } else {
                    AdminReloadReport::clean()
                };
                let _ = request.reply.send(report);
            }
            calls
        });

        let control = Control::new(reload_tx);
        assert!(control.request_reload().await.is_ok());
        // Écriture entre les deux appels (nouveau contenu, ici invalide).
        invalid.store(true, Ordering::SeqCst);
        let second = control.request_reload().await;
        assert!(
            !second.is_ok(),
            "le second rapport doit refléter le nouveau contenu, pas l'ancien"
        );
        drop(control);
        assert_eq!(server.await.expect("serveur de test"), 2);
    }

    #[tokio::test]
    async fn control_reload_serializes_concurrent_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<ReloadRequest>(8);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let server_calls = calls.clone();
        let server = tokio::spawn(async move {
            let mut entered_tx = Some(entered_tx);
            let mut release_rx = Some(release_rx);
            while let Some(request) = reload_rx.recv().await {
                let call = server_calls.fetch_add(1, Ordering::SeqCst) + 1;
                if call == 1 {
                    if let Some(signal) = entered_tx.take() {
                        let _ = signal.send(());
                    }
                    if let Some(release) = release_rx.take() {
                        let _ = release.await;
                    }
                }
                let _ = request.reply.send(AdminReloadReport::clean());
            }
        });

        let control = std::sync::Arc::new(Control::new(reload_tx));
        let first_control = control.clone();
        let first = tokio::spawn(async move { first_control.request_reload().await });
        entered_rx.await.expect("premier rechargement démarré");

        let second_control = control.clone();
        let second = tokio::spawn(async move { second_control.request_reload().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "le second appel ne doit pas envoyer de demande pendant le premier"
        );

        release_tx.send(()).expect("libération");
        assert!(first.await.expect("premier").is_ok());
        assert!(second.await.expect("second").is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        drop(control);
        server.await.expect("serveur de test");
    }

    #[tokio::test]
    async fn control_reload_reports_closed_channel() {
        let (reload_tx, reload_rx) = tokio::sync::mpsc::channel(1);
        drop(reload_rx);
        let control = Control::new(reload_tx);
        let report = control.request_reload().await;
        assert!(!report.is_ok());
        assert_eq!(report.warnings.len(), 1);
    }
}
