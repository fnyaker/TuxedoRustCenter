// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker webcam : kill switch, énumération V4L2 et **lecture seule** des
//! contrôles UVC.
//!
//! Reprend `WebcamWorker` :
//!
//! - publie la disponibilité du commutateur et son état ;
//! - applique le statut du **profil actif** à chaque changement de profil
//!   (comportement original conservé) ;
//! - énumère les nœuds `/dev/video*` (sysfs) ;
//! - lit les contrôles UVC à la demande via [`UvcReaderController`] (étape 1
//!   du design `docs/hardware/V4L2_DESIGN.md`).
//!
//! # Lecture à la demande
//!
//! Aucun ioctl n'est émis à chaque tick de 2 s. Les contrôles ne sont lus que
//! si le **bail de collecte** est actif (même drapeau que les capteurs, armé
//! par `GetHardwareSnapshotJSON`/`SetSensorDataCollectionStatus`). Une lecture
//! est conservée 30 s et rafraîchie si la liste des devices change. Sans
//! client, aucun contrôle n'est lu.
//!
//! `dry_run` n'ouvre aucun device et n'appelle aucun ioctl : la télémétrie
//! reste vide, ce qui permet `--check` et `--dev-session`.
//!
//! **Aucune écriture** : le module `tcc-hw::v4l2` de cette étape ne définit pas
//! `S_CTRL`. Les contrôles sont exposés en lecture, avec leurs bornes, leur
//! valeur courante et leurs menus.
//!
//! Uniwill n'expose pas le kill switch : `switchAvailable: false`, aucune
//! écriture, aucune erreur remontée à l'UI.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::tuxedo_io::TuxedoIo;
use tcc_hw::v4l2::{
    UvcControlInfo, UvcFlags, UvcIdentity, UvcMenuEntry, UvcReaderController, UvcValue,
};
use tcc_hw::webcam::{self, VideoDevice, WebcamSwitch};

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de rafraîchissement du kill switch et de l'énumération sysfs.
pub const WEBCAM_TICK: Duration = Duration::from_secs(2);
/// Intervalle minimal entre deux lectures de contrôles UVC (bail actif).
pub const CONTROLS_REFRESH: Duration = Duration::from_secs(30);
/// Nombre de ticks entre deux tentatives d'ouverture de `/dev/tuxedo_io`.
const IO_RETRY_TICKS: u32 = 15;
/// Nombre maximal d'erreurs de contrôle conservées dans le payload.
const MAX_CONTROL_ERRORS: usize = 8;

/// Identité d'une caméra exposée à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvcIdentityData {
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub fallback_name: String,
}

impl From<&UvcIdentity> for UvcIdentityData {
    fn from(identity: &UvcIdentity) -> Self {
        Self {
            vendor: identity.vendor.clone(),
            product: identity.product.clone(),
            serial: identity.serial.clone(),
            fallback_name: identity.fallback_name.clone(),
        }
    }
}

/// Entrée de menu exposée à l'UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvcMenuData {
    pub index: i64,
    pub key: String,
    pub label: String,
}

impl From<&UvcMenuEntry> for UvcMenuData {
    fn from(entry: &UvcMenuEntry) -> Self {
        Self {
            index: entry.index,
            key: entry.key.clone(),
            label: entry.label.clone(),
        }
    }
}

/// Valeur courante d'un contrôle.
///
/// Le JSON est tagué par `kind` : `{"kind":"integer","value":128}`,
/// `{"kind":"boolean","value":true}` ou
/// `{"kind":"menu","index":1,"key":"50_hz","label":"50 Hz"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UvcValueData {
    Integer {
        value: i64,
    },
    Boolean {
        value: bool,
    },
    Menu {
        index: i64,
        key: String,
        label: String,
    },
}

impl From<&UvcValue> for UvcValueData {
    fn from(value: &UvcValue) -> Self {
        match value {
            UvcValue::Integer(value) => Self::Integer { value: *value },
            UvcValue::Boolean(value) => Self::Boolean { value: *value },
            UvcValue::Menu { index, key, label } => Self::Menu {
                index: *index,
                key: key.clone(),
                label: label.clone(),
            },
        }
    }
}

/// Drapeaux d'un contrôle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvcFlagsData {
    pub inactive: bool,
    pub read_only: bool,
    pub write_only: bool,
    pub update: bool,
    pub disabled: bool,
    pub grabbed: bool,
}

impl From<UvcFlags> for UvcFlagsData {
    fn from(flags: UvcFlags) -> Self {
        Self {
            inactive: flags.inactive,
            read_only: flags.read_only,
            write_only: flags.write_only,
            update: flags.update,
            disabled: flags.disabled,
            grabbed: flags.grabbed,
        }
    }
}

/// Contrôle UVC exposé à l'UI (lecture seule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvcControlData {
    /// Clé stable (`brightness`, `focus_auto`, ...).
    pub key: String,
    /// Nom noyau sanitisé.
    pub name: String,
    /// Type (`integer`, `boolean`, `menu`, `integerMenu`).
    pub kind: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub default: i64,
    pub current: Option<UvcValueData>,
    pub menu: Vec<UvcMenuData>,
    pub flags: UvcFlagsData,
    /// Catégorie (`basic`, `exposure`, `color`, `advanced`, `other`).
    pub category: String,
    /// Vrai si le contrôle est visible en mode simple.
    pub simple: bool,
}

impl From<&UvcControlInfo> for UvcControlData {
    fn from(control: &UvcControlInfo) -> Self {
        Self {
            key: control.key.clone(),
            name: control.name.clone(),
            kind: control.kind.as_str().to_owned(),
            min: control.min,
            max: control.max,
            step: control.step,
            default: control.default,
            current: control.current.as_ref().map(UvcValueData::from),
            menu: control.menu.iter().map(UvcMenuData::from).collect(),
            flags: control.flags.into(),
            category: control.category.as_str().to_owned(),
            simple: control.simple,
        }
    }
}

/// Nœud V4L2 exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDeviceData {
    pub name: String,
    pub device_path: Option<String>,
    pub device_name: Option<String>,
    pub index: Option<u32>,
    /// Vrai pour un nœud de métadonnées UVC.
    pub metadata: bool,
    /// Identité stable (vendor/produit USB), si lue.
    pub identity: Option<UvcIdentityData>,
    /// Bus V4L2 (`usb-0000:00:14.0-1`), si lu.
    pub bus_info: Option<String>,
    /// Contrôles UVC (allowlist), vide si non lus ou non supportés.
    pub controls: Vec<UvcControlData>,
    /// Erreur de lecture des contrôles pour ce device.
    pub controls_error: Option<String>,
}

impl From<&VideoDevice> for VideoDeviceData {
    fn from(device: &VideoDevice) -> Self {
        Self {
            name: device.name.clone(),
            device_path: device.device_path.clone(),
            device_name: device
                .device_name
                .as_deref()
                .map(|value| sanitize_display(value, 128)),
            index: device.index,
            metadata: device.is_metadata(),
            identity: None,
            bus_info: None,
            controls: Vec::new(),
            controls_error: None,
        }
    }
}

/// Instantané webcam.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebcamData {
    /// Kill switch matériel disponible.
    pub switch_available: bool,
    /// Écriture des contrôles UVC : **toujours faux** dans `tccd`. L'écriture
    /// appartient à une future étape `tccd-user` ; ce champ évite à la GUI de
    /// déduire une capacité d'écriture de `controlsSupported`.
    pub controls_writable: bool,
    /// État du kill switch (`true` = webcam activée).
    pub enabled: Option<bool>,
    /// Vrai si au moins un contrôle UVC a été lu.
    pub controls_supported: bool,
    /// Vrai si le budget ou un plafond a tronqué la lecture.
    pub controls_truncated: bool,
    /// Nœuds V4L2 détectés.
    pub devices: Vec<VideoDeviceData>,
    /// Statut du kill switch appliqué par la dernière réconciliation.
    pub applied: Option<bool>,
    pub errors: Vec<String>,
}

/// Contrôles en cache pour un device.
#[derive(Debug, Clone, Default)]
struct CachedControls {
    node: String,
    identity: Option<UvcIdentityData>,
    bus_info: Option<String>,
    controls: Vec<UvcControlData>,
    error: Option<String>,
}

impl CachedControls {
    fn from_controls(controls: &tcc_hw::v4l2::UvcControls) -> Self {
        Self {
            node: controls.device.node.clone(),
            identity: Some(UvcIdentityData::from(&controls.device.identity)),
            bus_info: controls.device.bus_info.clone(),
            controls: controls.controls.iter().map(UvcControlData::from).collect(),
            error: controls.error.clone(),
        }
    }
}

/// Runtime webcam.
#[derive(Debug)]
pub struct WebcamRuntime {
    root: SysfsRoot,
    dry_run: bool,
    io: Option<Arc<TuxedoIo>>,
    switch: Option<WebcamSwitch>,
    io_retry: u32,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
    io_error_logged: bool,
    controls_enabled: Option<Arc<AtomicBool>>,
    uvc: Option<UvcReaderController>,
    cached_controls: Vec<CachedControls>,
    last_controls_read: Option<Instant>,
    controls_signature: Vec<String>,
    controls_truncated: bool,
    controls_error_logged: bool,
}

impl WebcamRuntime {
    /// Construit le runtime sans lecture de contrôles.
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            root,
            dry_run,
            io: None,
            switch: None,
            io_retry: 0,
            last_revision: None,
            applied_revision: None,
            io_error_logged: false,
            controls_enabled: None,
            uvc: None,
            cached_controls: Vec::new(),
            last_controls_read: None,
            controls_signature: Vec::new(),
            controls_truncated: false,
            controls_error_logged: false,
        }
    }

    /// Construit le runtime avec un bail de lecture et/ou un contrôleur injecté.
    ///
    /// `enabled` est le drapeau du bail (capteurs/Dashboard). `controller`
    /// permet aux tests d'injecter un `FakeReader` ; en production il est
    /// `None` et le contrôleur réel est créé paresseusement.
    #[must_use]
    pub fn with_controls(
        root: SysfsRoot,
        dry_run: bool,
        enabled: Option<Arc<AtomicBool>>,
        controller: Option<UvcReaderController>,
    ) -> Self {
        let mut runtime = Self::new(root, dry_run);
        runtime.controls_enabled = enabled;
        runtime.uvc = controller;
        runtime
    }

    /// Exécute un tick.
    pub fn tick(&mut self, input: &HardwareInputs) -> WebcamData {
        self.ensure_switch();
        let mut errors = Vec::new();
        let mut devices: Vec<VideoDeviceData> = webcam::enumerate(&self.root)
            .map(|devices| devices.iter().map(VideoDeviceData::from).collect())
            .unwrap_or_else(|error| {
                errors.push(format!("énumération V4L2: {error}"));
                Vec::new()
            });
        let nodes: Vec<String> = devices.iter().map(|device| device.name.clone()).collect();

        if self.controls_requested() {
            self.refresh_controls(&nodes, &mut errors);
        }
        self.apply_controls(&mut devices);

        let mut data = WebcamData {
            switch_available: self.switch.is_some(),
            controls_writable: false,
            enabled: self.switch.as_ref().and_then(|switch| switch.status().ok()),
            controls_supported: devices.iter().any(|device| !device.controls.is_empty()),
            controls_truncated: self.controls_truncated,
            devices,
            ..WebcamData::default()
        };

        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }
        let already_applied =
            self.applied_revision == self.last_revision && self.last_revision.is_some();
        if self.dry_run || already_applied {
            data.errors = errors;
            return data;
        }

        if let Some(switch) = &self.switch
            && let Some(profile) = &input.profile
        {
            let desired = profile.webcam.status;
            match switch.status() {
                Ok(current) if current == desired => data.applied = Some(desired),
                Ok(_) => match switch.set(desired) {
                    Ok(()) => data.applied = Some(desired),
                    Err(error) => errors.push(format!("kill switch: {error}")),
                },
                Err(error) => errors.push(format!("kill switch: {error}")),
            }
        }
        data.errors = errors;
        if data.errors.is_empty() {
            self.applied_revision = self.last_revision;
        }
        data
    }

    fn controls_requested(&self) -> bool {
        !self.dry_run
            && self
                .controls_enabled
                .as_ref()
                .is_some_and(|enabled| enabled.load(Ordering::Relaxed))
    }

    fn refresh_controls(&mut self, nodes: &[String], errors: &mut Vec<String>) {
        let fresh = self
            .last_controls_read
            .is_some_and(|last| last.elapsed() < CONTROLS_REFRESH);
        if fresh && self.controls_signature == nodes {
            return;
        }
        let report = {
            let controller = self
                .uvc
                .get_or_insert_with(|| UvcReaderController::system(self.root.clone()));
            controller.read_all()
        };
        self.cached_controls = report
            .controls
            .iter()
            .map(CachedControls::from_controls)
            .collect();
        self.controls_truncated = report.truncated;
        self.last_controls_read = Some(Instant::now());
        self.controls_signature = nodes.to_vec();
        if !report.errors.is_empty() && !self.controls_error_logged {
            self.controls_error_logged = true;
            tracing::debug!(
                errors = ?report.errors,
                "lecture des contrôles UVC partielle"
            );
        }
        errors.extend(report.errors.into_iter().take(MAX_CONTROL_ERRORS));
    }

    fn apply_controls(&self, devices: &mut [VideoDeviceData]) {
        for device in devices {
            let Some(cached) = self
                .cached_controls
                .iter()
                .find(|cached| cached.node == device.name)
            else {
                continue;
            };
            device.identity = cached.identity.clone();
            device.bus_info = cached.bus_info.clone();
            device.controls = cached.controls.clone();
            device.controls_error = cached.error.clone();
        }
    }

    fn ensure_switch(&mut self) {
        if self.switch.is_some() {
            return;
        }
        if self.io_retry > 0 {
            self.io_retry -= 1;
            return;
        }
        self.io_retry = IO_RETRY_TICKS;
        let io = match &self.io {
            Some(io) => Some(Arc::clone(io)),
            None => match TuxedoIo::open() {
                Ok(io) => {
                    let io = Arc::new(io);
                    self.io = Some(Arc::clone(&io));
                    Some(io)
                }
                Err(error) => {
                    if !self.io_error_logged {
                        self.io_error_logged = true;
                        tracing::debug!(%error, "tuxedo-io indisponible pour la webcam");
                    }
                    None
                }
            },
        };
        if let Some(io) = io {
            self.io_error_logged = false;
            let switch = WebcamSwitch::new(io);
            if switch.is_available() {
                self.switch = Some(switch);
            }
        }
    }
}

/// Nettoie une chaîne d'affichage issue du matériel.
fn sanitize_display(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Démarre le worker webcam.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
    controls_enabled: Option<Arc<AtomicBool>>,
) -> tokio::task::JoinHandle<()> {
    let runtime = WebcamRuntime::with_controls(root, dry_run, controls_enabled, None);
    spawn_worker(
        "webcam",
        WEBCAM_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_webcam,
        WebcamRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use tcc_core::presets::office_profile;
    use tcc_hw::v4l2::fake::{FakeControl, FakeReader};
    use tcc_hw::v4l2::{DeviceCapabilities, UvcCategory, UvcControlKind};

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn input(status: bool) -> HardwareInputs {
        let mut profile = office_profile();
        profile.webcam.status = status;
        HardwareInputs::new(tcc_core::Settings::default(), Some(profile), false, 1)
    }

    fn fixture(root: &Path) {
        write(root, "class/video4linux/video0/name", "Integrated Camera\n");
        write(root, "class/video4linux/video0/index", "0\n");
        write(root, "class/video4linux/video0/dev", "81:0\n");
        write(
            root,
            "class/video4linux/video0/device/uevent",
            "PRODUCT=1bcf/2c99/7200\n",
        );
    }

    fn capabilities() -> DeviceCapabilities {
        DeviceCapabilities {
            driver: "uvcvideo".to_owned(),
            card: "Integrated Camera".to_owned(),
            bus_info: "usb-0000:00:14.0-1".to_owned(),
            capabilities: 0x8000_0001,
            device_caps: 0x0000_0001,
        }
    }

    fn fake_controller(root: &Path) -> (UvcReaderController, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let fake = Arc::new(
            FakeReader::new(capabilities()).with_control(
                FakeControl::new(0x0098_0900, "Brightness", 1)
                    .range(0, 255, 1)
                    .default(128)
                    .value(42),
            ),
        );
        let controller = UvcReaderController::with_factory(
            SysfsRoot::new(root),
            Arc::new(move |_device| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(FakeReader::clone_state(&fake)))
            }),
        );
        (controller, calls)
    }

    #[test]
    fn enumerates_devices_without_switch() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/video4linux/video0/name",
            "Integrated Camera\n",
        );
        write(dir.path(), "class/video4linux/video0/index", "0\n");
        let mut runtime = WebcamRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(true));
        assert!(!data.switch_available);
        assert_eq!(data.devices.len(), 1);
        assert_eq!(data.devices[0].name, "video0");
        assert!(!data.controls_supported);
        assert!(!data.controls_truncated);
        assert_eq!(data.applied, None);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn missing_hardware_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = WebcamRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(false));
        assert!(!data.switch_available);
        assert!(data.devices.is_empty());
        assert!(data.applied.is_none());
        assert!(!data.controls_supported);
    }

    #[test]
    fn dry_run_does_not_touch_switch_or_read_controls() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let (controller, calls) = fake_controller(dir.path());
        let enabled = Arc::new(AtomicBool::new(true));
        let mut runtime = WebcamRuntime::with_controls(
            SysfsRoot::new(dir.path()),
            true,
            Some(enabled),
            Some(controller),
        );
        let first = runtime.tick(&input(true));
        let second = runtime.tick(&input(true));
        assert_eq!(first, second);
        assert!(first.errors.is_empty());
        assert!(!first.controls_supported);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn controls_are_read_on_demand_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let (controller, calls) = fake_controller(dir.path());
        let enabled = Arc::new(AtomicBool::new(true));
        let mut runtime = WebcamRuntime::with_controls(
            SysfsRoot::new(dir.path()),
            false,
            Some(enabled),
            Some(controller),
        );

        let data = runtime.tick(&input(true));
        assert!(data.controls_supported);
        assert_eq!(data.devices.len(), 1);
        let device = &data.devices[0];
        assert_eq!(
            device.identity.as_ref().unwrap().vendor.as_deref(),
            Some("1bcf")
        );
        assert_eq!(device.bus_info.as_deref(), Some("usb-0000:00:14.0-1"));
        assert_eq!(device.controls.len(), 1);
        assert_eq!(device.controls[0].key, "brightness");
        assert_eq!(
            device.controls[0].current,
            Some(UvcValueData::Integer { value: 42 })
        );
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"controlsSupported\":true"));
        assert!(json.contains("\"kind\":\"integer\""));
        assert!(!json.contains("\"id\":"));

        // Deuxième tick dans la fenêtre de 30 s : aucun nouvel appel.
        let calls_after_first = calls.load(Ordering::SeqCst);
        let _ = runtime.tick(&input(true));
        assert_eq!(calls.load(Ordering::SeqCst), calls_after_first);
    }

    #[test]
    fn disabled_lease_does_not_read_controls() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let (controller, calls) = fake_controller(dir.path());
        let enabled = Arc::new(AtomicBool::new(false));
        let mut runtime = WebcamRuntime::with_controls(
            SysfsRoot::new(dir.path()),
            false,
            Some(enabled),
            Some(controller),
        );
        let data = runtime.tick(&input(true));
        assert!(!data.controls_supported);
        assert!(data.devices[0].controls.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn serializes_camel_case() {
        let data = WebcamData {
            switch_available: true,
            enabled: Some(false),
            controls_supported: true,
            devices: vec![VideoDeviceData {
                name: "video0".to_owned(),
                metadata: false,
                controls: vec![UvcControlData {
                    key: "brightness".to_owned(),
                    name: "Brightness".to_owned(),
                    kind: "integer".to_owned(),
                    min: 0,
                    max: 255,
                    step: 1,
                    default: 128,
                    current: Some(UvcValueData::Integer { value: 128 }),
                    menu: Vec::new(),
                    flags: UvcFlagsData::default(),
                    category: "color".to_owned(),
                    simple: true,
                }],
                ..VideoDeviceData::default()
            }],
            ..WebcamData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"controlsSupported\":true"));
        assert!(json.contains("\"controlsWritable\":false"));
        assert!(json.contains("\"controlsTruncated\":false"));
        assert!(json.contains("\"controls\":["));
        assert!(!json.contains("\"id\":"));
        assert!(json.contains("\"kind\":\"integer\""));
    }

    #[test]
    fn control_categories_are_mapped() {
        let control = UvcControlInfo {
            key: "brightness".to_owned(),
            name: "Brightness".to_owned(),
            kind: UvcControlKind::Integer,
            min: 0,
            max: 255,
            step: 1,
            default: 128,
            current: Some(UvcValue::Integer(64)),
            menu: Vec::new(),
            flags: UvcFlags::default(),
            category: UvcCategory::Color,
            simple: true,
        };
        let data = UvcControlData::from(&control);
        assert_eq!(data.kind, "integer");
        assert_eq!(data.category, "color");
        assert!(data.simple);
    }

    /// Test opt-in sur matériel réel : vérifie que le worker expose bien les
    /// contrôles UVC quand une caméra est présente.
    ///
    /// ```text
    /// cargo test -p tcc-daemon --bins real_hardware_exposes_uvc_controls \
    ///     -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "ouvre /dev/video* : nécessite une caméra et un accès en lecture"]
    fn real_hardware_exposes_uvc_controls() {
        let enabled = Arc::new(AtomicBool::new(true));
        let mut runtime =
            WebcamRuntime::with_controls(SysfsRoot::system(), false, Some(enabled), None);
        let data = runtime.tick(&HardwareInputs::default());
        println!(
            "{}",
            serde_json::to_string_pretty(&data)
                .unwrap_or_else(|_| "<json indisponible>".to_owned())
        );
        if data.devices.is_empty() {
            println!("aucune caméra V4L2 détectée : test sans objet");
            return;
        }
        assert!(
            data.controls_supported,
            "aucun contrôle UVC lu : {:?}",
            data.errors
        );
        assert!(
            data.devices
                .iter()
                .any(|device| !device.controls.is_empty())
        );
    }
}
