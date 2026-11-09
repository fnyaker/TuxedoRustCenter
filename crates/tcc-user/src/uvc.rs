// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Écriture des contrôles UVC pour le service de session `User1`.
//!
//! Ce module est le seul point d'entrée d'écriture de `tccd-user`. Il s'appuie
//! sur `tcc_hw::v4l2::write`, qui valide déjà :
//!
//! - le chemin canonique `/dev/videoN` (caractère, major 81, propriétaire
//!   root) ;
//! - l'allowlist fermée des 15 CIDs UVC ;
//! - le type, les bornes, le pas et les menus du contrôle ;
//! - la relecture après `VIDIOC_S_CTRL`, le budget de 64 ioctl et le délai de
//!   2 s ;
//! - le verrou par device et la quarantaine de 30 s après `EIO`.
//!
//! Ce module ajoute les contrôles propres au service de session :
//!
//! - le chemin demandé doit correspondre à un nœud **réellement énuméré** par
//!   `tcc-hw` (aucun chemin arbitraire, aucun `/dev/videoN` inventé) ;
//! - la clé est bornée à 64 octets ASCII `[a-z0-9_]+` ;
//! - le débit est limité par UID appelant (voir [`crate::rate_limit`]) ;
//! - les erreurs sont stables et documentées dans `docs/DBUS_API.md`.
//!
//! Aucun périphérique n'est ouvert au démarrage : l'énumération et l'écriture
//! sont exécutées à l'appel, dans une tâche bloquante bornée.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tcc_hw::error::HwError;
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::v4l2::{UvcWriteReport, UvcWriterController};
use tcc_hw::webcam;

use crate::rate_limit::UvcWriteRateLimiter;

/// Longueur maximale d'une clé de contrôle UVC acceptée par le service.
pub const MAX_UVC_CONTROL_KEY_BYTES: usize = 64;
/// Longueur maximale d'un chemin de device UVC accepté par le service.
pub const MAX_UVC_DEVICE_PATH_BYTES: usize = 64;
/// Durée de validité du cache de `GetUvcWriteSupported`.
///
/// Choix : un cache court **global** plutôt qu'un rate limit par UID. Une
/// méthode de capability ne doit pas introduire une erreur `LimitsExceeded`
/// côté GUI ; le cache borne l'énumération V4L2 à au plus une fois par seconde,
/// même si plusieurs clients de session interrogent la capacité.
pub const UVC_WRITE_SUPPORTED_CACHE: Duration = Duration::from_secs(1);
/// Préfixe des erreurs applicatives du service `User1`.
pub const UVC_ERROR_PREFIX: &str = "org.tuxedocomputers.tccd.User1.Error";
/// Erreur stable : valeur refusée par les bornes, le pas ou le menu.
pub const UVC_ERROR_OUT_OF_RANGE: &str = "org.tuxedocomputers.tccd.User1.Error.OutOfRange";
/// Erreur stable : device déjà utilisé par une autre écriture ou `EBUSY`.
pub const UVC_ERROR_BUSY: &str = "org.tuxedocomputers.tccd.User1.Error.Busy";
/// Erreur stable : device en quarantaine après `EIO`/débranchement.
pub const UVC_ERROR_STALLED: &str = "org.tuxedocomputers.tccd.User1.Error.Stalled";
/// Erreur standard : argument de méthode invalide.
pub const UVC_ERROR_INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";
/// Erreur standard : contrôle absent de l'allowlist ou non inscriptible.
pub const UVC_ERROR_NOT_SUPPORTED: &str = "org.freedesktop.DBus.Error.NotSupported";
/// Erreur standard : échec générique du backend.
pub const UVC_ERROR_FAILED: &str = "org.freedesktop.DBus.Error.Failed";
/// Erreur standard : limite de débit atteinte.
pub const UVC_ERROR_LIMITS_EXCEEDED: &str = "org.freedesktop.DBus.Error.LimitsExceeded";

/// Entrée d'énumération UVC minimale utilisée pour valider un chemin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcDeviceEntry {
    /// Nom sysfs (`video0`).
    pub node: String,
    /// Chemin canonique (`/dev/video0`).
    pub path: String,
}

/// Source d'énumération V4L2 (sysfs), injectable en test.
pub trait UvcDeviceSource: Send + Sync + std::fmt::Debug {
    /// Liste les nœuds de capture réellement énumérés.
    fn devices(&self) -> tcc_hw::Result<Vec<UvcDeviceEntry>>;
}

/// Source réelle : `tcc_hw::webcam::enumerate`, sans ouverture de device.
#[derive(Debug, Clone)]
pub struct SysfsUvcDeviceSource {
    root: SysfsRoot,
}

impl SysfsUvcDeviceSource {
    /// Source de production (`/sys`).
    #[must_use]
    pub fn system() -> Self {
        Self {
            root: SysfsRoot::system(),
        }
    }

    /// Source sur une racine injectée (tests, simulation).
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }
}

impl UvcDeviceSource for SysfsUvcDeviceSource {
    fn devices(&self) -> tcc_hw::Result<Vec<UvcDeviceEntry>> {
        let entries = webcam::enumerate(&self.root)?;
        Ok(entries
            .into_iter()
            .filter(|device| device.is_capture())
            .filter_map(|device| {
                let path = device.device_path?;
                Some(UvcDeviceEntry {
                    node: device.name,
                    path,
                })
            })
            .collect())
    }
}

/// Écriture d'un contrôle UVC, injectable en test.
pub trait UvcControlWriter: Send + Sync + std::fmt::Debug {
    /// Écrit `value` et retourne le rapport du backend.
    ///
    /// # Errors
    ///
    /// Erreurs typées de `tcc-hw` (`Busy`, `Stalled`, `OutOfRange`,
    /// `InvalidValue`, `NotSupported`, `BudgetExceeded`, ...).
    fn set_control(
        &self,
        device: &str,
        control_key: &str,
        value: i64,
    ) -> tcc_hw::Result<UvcWriteReport>;
}

/// Écriture réelle : [`UvcWriterController`] du crate `tcc-hw`.
#[derive(Debug)]
pub struct SystemUvcWriter {
    controller: UvcWriterController,
}

impl SystemUvcWriter {
    /// Écrivain de production (`/sys`, ouverture de `/dev/videoN` à l'appel).
    #[must_use]
    pub fn system() -> Self {
        Self {
            controller: UvcWriterController::system(SysfsRoot::system()),
        }
    }

    /// Écrivain sur un contrôleur injecté (tests avec `FakeWriter`).
    #[must_use]
    pub fn with_controller(controller: UvcWriterController) -> Self {
        Self { controller }
    }
}

impl UvcControlWriter for SystemUvcWriter {
    fn set_control(
        &self,
        device: &str,
        control_key: &str,
        value: i64,
    ) -> tcc_hw::Result<UvcWriteReport> {
        self.controller.set_control(device, control_key, value)
    }
}

/// Erreur D-Bus stable du service `User1.SetUvcControl`.
///
/// Les variantes `InvalidArgs`, `NotSupported` et `Failed` utilisent les noms
/// standard D-Bus ; `OutOfRange`, `Busy` et `Stalled` utilisent le namespace
/// applicatif [`UVC_ERROR_PREFIX`] ; la limite de débit utilise
/// `org.freedesktop.DBus.Error.LimitsExceeded`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UvcWriteError {
    /// Chemin ou clé syntaxiquement invalide, ou device non énuméré.
    #[error("argument UVC invalide : {0}")]
    InvalidArgs(String),
    /// Contrôle hors allowlist, en lecture seule ou backend absent.
    #[error("écriture UVC non supportée : {0}")]
    NotSupported(String),
    /// Valeur refusée par les bornes, le pas ou le menu du contrôle.
    #[error("valeur UVC refusée : {0}")]
    OutOfRange(String),
    /// Device déjà en cours d'écriture ou occupé par une autre application.
    #[error("périphérique UVC occupé")]
    Busy,
    /// Device en quarantaine après `EIO` ou débranchement.
    #[error("périphérique UVC bloqué")]
    Stalled,
    /// Échec générique du backend.
    #[error("écriture UVC échouée : {0}")]
    Failed(String),
    /// Limite de débit atteinte pour cet appelant.
    #[error("limite de débit atteinte")]
    RateLimited,
}

impl UvcWriteError {
    /// Libellé stable pour l'audit (`result=…`).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::InvalidArgs(_) => "invalid_args",
            Self::NotSupported(_) => "not_supported",
            Self::OutOfRange(_) => "out_of_range",
            Self::Busy => "busy",
            Self::Stalled => "stalled",
            Self::Failed(_) => "failed",
            Self::RateLimited => "rate_limited",
        }
    }
}

impl zbus::DBusError for UvcWriteError {
    fn name(&self) -> zbus::names::ErrorName<'_> {
        zbus::names::ErrorName::from_static_str_unchecked(match self {
            Self::InvalidArgs(_) => UVC_ERROR_INVALID_ARGS,
            Self::NotSupported(_) => UVC_ERROR_NOT_SUPPORTED,
            Self::OutOfRange(_) => UVC_ERROR_OUT_OF_RANGE,
            Self::Busy => UVC_ERROR_BUSY,
            Self::Stalled => UVC_ERROR_STALLED,
            Self::Failed(_) => UVC_ERROR_FAILED,
            Self::RateLimited => UVC_ERROR_LIMITS_EXCEEDED,
        })
    }

    fn description(&self) -> Option<&str> {
        match self {
            Self::InvalidArgs(message)
            | Self::NotSupported(message)
            | Self::OutOfRange(message)
            | Self::Failed(message) => Some(message.as_str()),
            Self::Busy => Some("périphérique UVC occupé"),
            Self::Stalled => Some("périphérique UVC bloqué"),
            Self::RateLimited => Some("limite de débit UVC atteinte"),
        }
    }

    fn create_reply(
        &self,
        header: &zbus::message::Header<'_>,
    ) -> zbus::Result<zbus::message::Message> {
        let name = self.name();
        match self {
            Self::InvalidArgs(message)
            | Self::NotSupported(message)
            | Self::OutOfRange(message)
            | Self::Failed(message) => zbus::message::Message::error(header, name)?.build(message),
            Self::Busy => {
                zbus::message::Message::error(header, name)?.build(&"périphérique UVC occupé")
            }
            Self::Stalled => {
                zbus::message::Message::error(header, name)?.build(&"périphérique UVC bloqué")
            }
            Self::RateLimited => {
                zbus::message::Message::error(header, name)?.build(&"limite de débit UVC atteinte")
            }
        }
    }
}

/// Runtime d'écriture UVC partagé par le service `User1`.
///
/// Une seule instance par processus : le verrou par device et la quarantaine
/// de `tcc-hw` sont internes au contrôleur.
#[derive(Debug)]
pub struct UvcRuntime {
    devices: Arc<dyn UvcDeviceSource>,
    writer: Arc<dyn UvcControlWriter>,
    limiter: Mutex<UvcWriteRateLimiter>,
    write_supported_cache: Mutex<Option<(Instant, bool)>>,
}

impl UvcRuntime {
    /// Runtime de production : sysfs réel, écriture à l'appel uniquement.
    #[must_use]
    pub fn production() -> Self {
        Self::new(
            Arc::new(SysfsUvcDeviceSource::system()),
            Arc::new(SystemUvcWriter::system()),
        )
    }

    /// Runtime avec dépendances injectées (tests).
    #[must_use]
    pub fn new(devices: Arc<dyn UvcDeviceSource>, writer: Arc<dyn UvcControlWriter>) -> Self {
        Self {
            devices,
            writer,
            limiter: Mutex::new(UvcWriteRateLimiter::default()),
            write_supported_cache: Mutex::new(None),
        }
    }

    /// Runtime avec une limite de débit personnalisée (tests, simulation).
    ///
    /// La production utilise [`UvcRuntime::production`] ou
    /// [`UvcRuntime::new`] (20 écritures/s, burst 20). Ce constructeur permet
    /// de rendre les tests de limite déterministes sans dépendre de la durée
    /// réelle des appels.
    #[must_use]
    pub fn with_rate_limit(
        devices: Arc<dyn UvcDeviceSource>,
        writer: Arc<dyn UvcControlWriter>,
        rate_per_second: f64,
        burst: f64,
    ) -> Self {
        Self {
            devices,
            writer,
            limiter: Mutex::new(UvcWriteRateLimiter::new(rate_per_second, burst)),
            write_supported_cache: Mutex::new(None),
        }
    }

    /// Vrai si l'écriture est disponible : au moins un nœud UVC énuméré et un
    /// backend d'écriture instancié.
    ///
    /// Aucun device n'est ouvert : seule l'énumération sysfs est lue. Le
    /// résultat est mis en cache pendant [`UVC_WRITE_SUPPORTED_CACHE`] afin
    /// qu'un client ne puisse pas marteler l'énumération V4L2.
    #[must_use]
    pub fn write_supported(&self) -> bool {
        self.write_supported_at(Instant::now())
    }

    fn write_supported_at(&self, now: Instant) -> bool {
        {
            let cache = lock_or_recover(&self.write_supported_cache);
            if let Some((cached_at, supported)) = cache.as_ref()
                && now.saturating_duration_since(*cached_at) < UVC_WRITE_SUPPORTED_CACHE
            {
                return *supported;
            }
        }
        let supported = self
            .devices
            .devices()
            .map(|devices| devices.iter().any(|device| !device.path.is_empty()))
            .unwrap_or(false);
        *lock_or_recover(&self.write_supported_cache) = Some((now, supported));
        supported
    }

    /// Vérifie la syntaxe d'une demande avant toute I/O ou consommation de
    /// jeton.
    ///
    /// # Errors
    ///
    /// [`UvcWriteError::InvalidArgs`] si le chemin ou la clé ne respecte pas
    /// le contrat.
    pub fn validate_request(device_path: &str, control_key: &str) -> Result<(), UvcWriteError> {
        validate_device_path(device_path)?;
        validate_control_key(control_key)
    }

    /// Consomme un jeton de débit pour `caller`.
    pub fn allow(&self, caller: &str) -> bool {
        lock_or_recover(&self.limiter).allow(caller, Instant::now())
    }

    /// Écrit un contrôle sur un device **réellement énuméré**.
    ///
    /// # Errors
    ///
    /// - [`UvcWriteError::InvalidArgs`] : chemin non énuméré ou clé invalide ;
    /// - [`UvcWriteError::Failed`] : énumération ou backend en échec ;
    /// - erreurs typées du backend (`Busy`, `Stalled`, `OutOfRange`,
    ///   `NotSupported`).
    ///
    /// Le booléen vaut `true` si la valeur a été écrite, `false` si elle était
    /// déjà courante (idempotence).
    pub fn write(
        &self,
        device_path: &str,
        control_key: &str,
        value: i64,
    ) -> Result<bool, UvcWriteError> {
        Self::validate_request(device_path, control_key)?;
        let devices = self.devices.devices().map_err(|error| {
            UvcWriteError::Failed(format!("énumération V4L2 indisponible : {error}"))
        })?;
        if !devices.iter().any(|device| device.path == device_path) {
            return Err(UvcWriteError::InvalidArgs(
                "devicePath non énuméré par tcc-hw".to_owned(),
            ));
        }
        self.writer
            .set_control(device_path, control_key, value)
            .map(|report| report.changed)
            .map_err(map_hw_error)
    }
}

/// Valide un chemin `/dev/videoN` canonique.
fn validate_device_path(path: &str) -> Result<(), UvcWriteError> {
    if path.is_empty() || path.len() > MAX_UVC_DEVICE_PATH_BYTES || !path.is_ascii() {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    }
    let Some(name) = path.strip_prefix("/dev/") else {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    };
    let Some(digits) = name.strip_prefix("video") else {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    };
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    }
    let Ok(number) = digits.parse::<u32>() else {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    };
    if number > 4096 {
        return Err(UvcWriteError::InvalidArgs("devicePath".to_owned()));
    }
    Ok(())
}

/// Valide une clé stable `[a-z0-9_]+` de 64 octets au plus.
fn validate_control_key(key: &str) -> Result<(), UvcWriteError> {
    if key.is_empty()
        || key.len() > MAX_UVC_CONTROL_KEY_BYTES
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(UvcWriteError::InvalidArgs("controlKey".to_owned()));
    }
    Ok(())
}

/// Convertit une erreur `tcc-hw` en erreur D-Bus stable.
fn map_hw_error(error: HwError) -> UvcWriteError {
    // `EACCES`/`EPERM` à l'ouverture ou sur un ioctl ne sont pas une panne du
    // backend : l'ACL `uaccess` ou le groupe `video` manque pour cet
    // utilisateur. La GUI doit afficher « non supporté / permission » et non
    // « échec ».
    if is_permission_error(&error) {
        return UvcWriteError::NotSupported(
            "accès au périphérique UVC refusé (ACL uaccess/groupe video absent)".to_owned(),
        );
    }
    match error {
        HwError::Busy { .. } => UvcWriteError::Busy,
        HwError::Stalled { .. } => UvcWriteError::Stalled,
        HwError::OutOfRange { .. } | HwError::InvalidValue { .. } => {
            UvcWriteError::OutOfRange("valeur ou menu UVC refusé".to_owned())
        }
        HwError::NotSupported(_) | HwError::UnsupportedDevice => {
            UvcWriteError::NotSupported("contrôle UVC non disponible en écriture".to_owned())
        }
        HwError::PolicyDenied { .. } => {
            UvcWriteError::NotSupported("écriture UVC refusée par la politique".to_owned())
        }
        HwError::InvalidPath { .. } => UvcWriteError::InvalidArgs("devicePath".to_owned()),
        HwError::BudgetExceeded { .. } => {
            UvcWriteError::Failed("budget d'ioctl UVC épuisé".to_owned())
        }
        other => UvcWriteError::Failed(format!("backend UVC : {other}")),
    }
}

/// Vrai si l'erreur signifie un refus d'autorisation du système de fichiers.
fn is_permission_error(error: &HwError) -> bool {
    match error {
        HwError::Io { source, .. }
        | HwError::Unavailable { source, .. }
        | HwError::Ioctl { source, .. } => source.kind() == std::io::ErrorKind::PermissionDenied,
        _ => false,
    }
}

fn lock_or_recover<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Debug)]
    struct FakeDevices {
        devices: Vec<UvcDeviceEntry>,
    }

    impl UvcDeviceSource for FakeDevices {
        fn devices(&self) -> tcc_hw::Result<Vec<UvcDeviceEntry>> {
            Ok(self.devices.clone())
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum FakeFailure {
        Busy,
        Stalled,
        OutOfRange,
        NotSupported,
        Budget,
        NotFound,
    }

    impl FakeFailure {
        fn into_hw_error(self) -> HwError {
            match self {
                Self::Busy => HwError::Busy {
                    device: "video0".to_owned(),
                },
                Self::Stalled => HwError::Stalled {
                    device: "video0".to_owned(),
                },
                Self::OutOfRange => HwError::OutOfRange {
                    context: "valeur UVC",
                    value: 300,
                    min: 0,
                    max: 255,
                },
                Self::NotSupported => HwError::NotSupported("contrôle UVC"),
                Self::Budget => HwError::BudgetExceeded {
                    operation: "UVC",
                    limit: 64,
                },
                Self::NotFound => HwError::NotFound {
                    path: "/dev/video0".into(),
                },
            }
        }
    }

    #[derive(Debug)]
    struct CountingDevices {
        devices: Vec<UvcDeviceEntry>,
        calls: AtomicUsize,
    }

    impl UvcDeviceSource for CountingDevices {
        fn devices(&self) -> tcc_hw::Result<Vec<UvcDeviceEntry>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.devices.clone())
        }
    }

    #[derive(Debug)]
    struct FakeWriter {
        report: Mutex<Option<UvcWriteReport>>,
        failure: Mutex<Option<FakeFailure>>,
        calls: AtomicUsize,
        last: Mutex<Option<(String, String, i64)>>,
    }

    impl FakeWriter {
        fn changed(changed: bool) -> Self {
            Self::with_report(UvcWriteReport {
                device: "/dev/video0".to_owned(),
                key: "brightness".to_owned(),
                previous: if changed { 10 } else { 42 },
                value: 42,
                changed,
            })
        }

        fn failing(failure: FakeFailure) -> Self {
            Self {
                report: Mutex::new(None),
                failure: Mutex::new(Some(failure)),
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }

        fn with_report(report: UvcWriteReport) -> Self {
            Self {
                report: Mutex::new(Some(report)),
                failure: Mutex::new(None),
                calls: AtomicUsize::new(0),
                last: Mutex::new(None),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn last(&self) -> Option<(String, String, i64)> {
            match self.last.lock() {
                Ok(guard) => guard.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }
    }

    impl UvcControlWriter for FakeWriter {
        fn set_control(
            &self,
            device: &str,
            control_key: &str,
            value: i64,
        ) -> tcc_hw::Result<UvcWriteReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.last.lock() {
                Ok(mut last) => {
                    *last = Some((device.to_owned(), control_key.to_owned(), value));
                }
                Err(poisoned) => {
                    *poisoned.into_inner() =
                        Some((device.to_owned(), control_key.to_owned(), value));
                }
            }
            if let Ok(failure) = self.failure.lock()
                && let Some(failure) = *failure
            {
                return Err(failure.into_hw_error());
            }
            match self.report.lock() {
                Ok(report) => report.clone().ok_or(HwError::NotSupported("fake report")),
                Err(poisoned) => poisoned
                    .into_inner()
                    .clone()
                    .ok_or(HwError::NotSupported("fake report")),
            }
        }
    }

    fn entry(path: &str) -> UvcDeviceEntry {
        UvcDeviceEntry {
            node: path.trim_start_matches("/dev/").to_owned(),
            path: path.to_owned(),
        }
    }

    fn build_runtime(devices: Vec<UvcDeviceEntry>, writer: FakeWriter) -> UvcRuntime {
        UvcRuntime::new(
            Arc::new(FakeDevices { devices }),
            Arc::new(writer) as Arc<dyn UvcControlWriter>,
        )
    }

    #[test]
    fn device_paths_are_canonical() {
        for path in ["/dev/video0", "/dev/video12", "/dev/video4096"] {
            assert!(validate_device_path(path).is_ok(), "{path}");
        }
        for path in [
            "",
            "video0",
            "/dev/video",
            "/dev/videoX",
            "/dev/video0/",
            "/tmp/video0",
            "/dev/video01",
            "/dev/../video0",
            "/dev/video99999",
            "/dev/video4097",
            "/dev/vidéo0",
        ] {
            assert!(validate_device_path(path).is_err(), "{path}");
        }
        assert!(validate_device_path(&format!("/dev/video{}", "0".repeat(60))).is_err());
    }

    #[test]
    fn keys_are_bounded_and_lowercase() {
        for key in [
            "brightness",
            "focus_auto",
            "power_line_frequency",
            "a",
            "a_1",
        ] {
            assert!(validate_control_key(key).is_ok(), "{key}");
        }
        for key in [
            "",
            "Brightness",
            "focus-auto",
            "focus auto",
            "focus.auto",
            "../x",
        ] {
            assert!(validate_control_key(key).is_err(), "{key}");
        }
        assert!(validate_control_key(&"a".repeat(MAX_UVC_CONTROL_KEY_BYTES)).is_ok());
        assert!(validate_control_key(&"a".repeat(MAX_UVC_CONTROL_KEY_BYTES + 1)).is_err());
    }

    #[test]
    fn write_supported_requires_an_enumerated_device() {
        assert!(!build_runtime(Vec::new(), FakeWriter::changed(true)).write_supported());
        assert!(
            build_runtime(vec![entry("/dev/video0")], FakeWriter::changed(true)).write_supported()
        );
    }

    #[test]
    fn write_supported_is_cached_for_one_second() {
        let devices = Arc::new(CountingDevices {
            devices: vec![entry("/dev/video0")],
            calls: AtomicUsize::new(0),
        });
        let runtime = UvcRuntime::new(
            Arc::clone(&devices) as Arc<dyn UvcDeviceSource>,
            Arc::new(FakeWriter::changed(true)),
        );
        let start = Instant::now();
        assert!(runtime.write_supported_at(start));
        assert!(runtime.write_supported_at(start + Duration::from_millis(999)));
        assert_eq!(devices.calls.load(Ordering::SeqCst), 1);

        // Après le cache, l'énumération est rafraîchie.
        assert!(
            runtime
                .write_supported_at(start + UVC_WRITE_SUPPORTED_CACHE + Duration::from_millis(1))
        );
        assert_eq!(devices.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn permission_errors_are_not_supported() {
        let permission = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let errors = [
            HwError::Io {
                path: "/dev/video0".into(),
                source: permission(),
            },
            HwError::Unavailable {
                device: "video0",
                source: permission(),
            },
            HwError::Ioctl {
                name: "VIDIOC_S_CTRL",
                request: 0,
                source: permission(),
            },
        ];
        for error in errors {
            let mapped = map_hw_error(error);
            assert!(
                matches!(mapped, UvcWriteError::NotSupported(_)),
                "{mapped:?}"
            );
        }
    }

    #[test]
    fn non_enumerated_device_does_not_call_backend() {
        let writer = Arc::new(FakeWriter::changed(true));
        let runtime = UvcRuntime::new(
            Arc::new(FakeDevices {
                devices: vec![entry("/dev/video0")],
            }),
            Arc::clone(&writer) as Arc<dyn UvcControlWriter>,
        );
        assert!(runtime.write("/dev/video1", "brightness", 42).is_err());
        assert_eq!(writer.calls(), 0);
    }

    #[test]
    fn write_forwards_validated_device_and_key() {
        let writer = Arc::new(FakeWriter::changed(true));
        let runtime = UvcRuntime::new(
            Arc::new(FakeDevices {
                devices: vec![entry("/dev/video0")],
            }),
            Arc::clone(&writer) as Arc<dyn UvcControlWriter>,
        );
        assert!(runtime.write("/dev/video0", "brightness", 42).unwrap());
        assert_eq!(
            writer.last(),
            Some(("/dev/video0".to_owned(), "brightness".to_owned(), 42))
        );
    }

    #[test]
    fn changed_and_unchanged_are_reported() {
        let runtime = build_runtime(vec![entry("/dev/video0")], FakeWriter::changed(true));
        assert!(runtime.write("/dev/video0", "brightness", 42).unwrap());

        let runtime = build_runtime(vec![entry("/dev/video0")], FakeWriter::changed(false));
        assert!(!runtime.write("/dev/video0", "brightness", 42).unwrap());
    }

    #[test]
    fn backend_errors_are_mapped_to_stable_errors() {
        for (failure, expected) in [
            (FakeFailure::Busy, "busy"),
            (FakeFailure::Stalled, "stalled"),
            (FakeFailure::OutOfRange, "out_of_range"),
            (FakeFailure::NotSupported, "not_supported"),
            (FakeFailure::Budget, "failed"),
            (FakeFailure::NotFound, "failed"),
        ] {
            let runtime = build_runtime(vec![entry("/dev/video0")], FakeWriter::failing(failure));
            let error = runtime
                .write("/dev/video0", "brightness", 42)
                .expect_err("erreur backend");
            assert_eq!(error.label(), expected, "{failure:?}");
        }
    }

    #[test]
    fn invalid_requests_do_not_call_backend() {
        let writer = Arc::new(FakeWriter::changed(true));
        let runtime = UvcRuntime::new(
            Arc::new(FakeDevices {
                devices: vec![entry("/dev/video0")],
            }),
            Arc::clone(&writer) as Arc<dyn UvcControlWriter>,
        );
        for (path, key) in [
            ("../video0", "brightness"),
            ("/dev/video0", "Brightness"),
            ("", "brightness"),
        ] {
            assert!(
                runtime.write(path, key, 42).is_err(),
                "accepté à tort: {path} {key}"
            );
        }
        assert_eq!(writer.calls(), 0);
    }

    #[test]
    fn rate_limit_is_per_caller() {
        // Débit volontairement très bas : le test ne dépend pas du temps réel
        // de la machine.
        let runtime = UvcRuntime::with_rate_limit(
            Arc::new(FakeDevices {
                devices: vec![entry("/dev/video0")],
            }),
            Arc::new(FakeWriter::changed(true)) as Arc<dyn UvcControlWriter>,
            0.001,
            2.0,
        );
        assert!(runtime.allow("uid:1000"));
        assert!(runtime.allow("uid:1000"));
        assert!(!runtime.allow("uid:1000"));
        assert!(runtime.allow("uid:1001"));
        assert!(runtime.allow("uid:1001"));
        assert!(!runtime.allow("uid:1001"));
    }

    #[test]
    fn error_names_are_stable() {
        use zbus::DBusError;

        let cases = [
            (
                UvcWriteError::InvalidArgs("x".to_owned()),
                UVC_ERROR_INVALID_ARGS,
            ),
            (
                UvcWriteError::NotSupported("x".to_owned()),
                UVC_ERROR_NOT_SUPPORTED,
            ),
            (
                UvcWriteError::OutOfRange("x".to_owned()),
                UVC_ERROR_OUT_OF_RANGE,
            ),
            (UvcWriteError::Busy, UVC_ERROR_BUSY),
            (UvcWriteError::Stalled, UVC_ERROR_STALLED),
            (UvcWriteError::Failed("x".to_owned()), UVC_ERROR_FAILED),
            (UvcWriteError::RateLimited, UVC_ERROR_LIMITS_EXCEEDED),
        ];
        for (error, expected) in cases {
            assert_eq!(error.name().as_str(), expected);
        }
    }
}
