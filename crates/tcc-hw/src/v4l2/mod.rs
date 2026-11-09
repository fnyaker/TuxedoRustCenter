// SPDX-License-Identifier: GPL-3.0-or-later
//! Contrôles UVC V4L2 — lecture (`tccd`) et écriture (`tccd-user`).
//!
//! Le chemin de lecture expose `VIDIOC_QUERYCAP`, `VIDIOC_QUERYCTRL`,
//! `VIDIOC_QUERYMENU` et `VIDIOC_G_CTRL` uniquement. L'écriture est isolée
//! dans [`mod@write`]/`real_write` et utilise `VIDIOC_S_CTRL` ; elle n'est jamais
//! construite ni appelée par `tccd`, la GUI ou les helpers de lecture (garde
//! statique `tests/v4l2_unsafe_guard.rs`). Voir
//! `docs/hardware/V4L2_DESIGN.md`.
//!
//! # Principes
//!
//! - **Allowlist normative** : seuls 15 CIDs sont exposés
//!   ([`control::cid_allowed`]) ; tout autre contrôle énuméré est ignoré.
//! - **Nœud dérivé de sysfs** : [`crate::webcam::enumerate`] fournit `videoN`,
//!   le chemin `/dev/videoN` est reconstruit et validé (caractère, major 81,
//!   minor sysfs, root ; l'écriture monde est journalisée et acceptée pour
//!   une ouverture en lecture seule).
//! - **Budget** : au plus [`MAX_IOCTLS_PER_READ`] ioctl par cycle, 16 devices,
//!   256 contrôles, 64 entrées de menu. Un dépassement produit
//!   `truncated: true`, jamais une boucle infinie.
//! - **`unsafe` confiné** : uniquement dans les transports internes `real`
//!   (lecture) et `real_write` (écriture) ; les autres modules sont
//!   `#![forbid(unsafe_code)]`. Aucun `mmap`, aucun flux. `S_CTRL` n'existe
//!   que dans `real_write` et `abi`.
//! - **Pas de lecture périodique** : ce module est synchrone et appelé à la
//!   demande par le worker (`spawn_blocking`). Les menus ne sont lus que
//!   lorsque les contrôles sont demandés.
//!
//! # Transport factice
//!
//! [`fake::FakeReader`] et [`fake::FakeWriter`] implémentent les traits sans
//! matériel ; la CI teste l'énumération, l'allowlist, les menus, les erreurs
//! (`EINTR`, `EBUSY`, `ENODEV`, `EIO`), le budget, `try_lock`/`stalled` et la
//! relecture après écriture.

#![deny(unsafe_code)]

mod abi;
pub mod control;
mod device;
pub mod fake;
mod real;
mod real_write;
pub mod write;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;
use crate::webcam::{self, VideoDevice};

pub use control::{UvcCategory, UvcControlInfo, UvcControlKind, UvcFlags, UvcMenuEntry, UvcValue};
pub use device::UvcIdentity;
pub use write::{
    MAX_IOCTLS_PER_WRITE, STALL_COOLDOWN, UvcWriteReport, UvcWriter, UvcWriterController,
    WRITE_TIMEOUT, WriterFactory,
};

/// Nombre maximal de devices exposés.
pub const MAX_DEVICES: usize = 16;
/// Nombre maximal de contrôles par device.
pub const MAX_CONTROLS: usize = 256;
/// Nombre maximal d'entrées par menu.
pub const MAX_MENU_ENTRIES: usize = 64;
/// Nombre maximal d'ioctl par cycle de lecture.
pub const MAX_IOCTLS_PER_READ: u32 = 512;
/// Nombre de tentatives en cas d'`EINTR`.
const EINTR_ATTEMPTS: u32 = 3;
/// Délai entre deux tentatives `EINTR`.
const EINTR_DELAY: Duration = Duration::from_millis(10);

/// Capacités d'un périphérique V4L2, décodées de `VIDIOC_QUERYCAP`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceCapabilities {
    /// Pilote (`uvcvideo`).
    pub driver: String,
    /// Nom de la carte.
    pub card: String,
    /// Bus (`usb-0000:00:14.0-1`).
    pub bus_info: String,
    /// `capabilities` brut.
    pub capabilities: u32,
    /// `device_caps` brut.
    pub device_caps: u32,
}

impl DeviceCapabilities {
    /// Capacités effectives : `device_caps` est autoritaire quand
    /// `V4L2_CAP_DEVICE_CAPS` est posé (V-07).
    #[must_use]
    pub fn effective_caps(&self) -> u32 {
        if self.capabilities & abi::V4L2_CAP_DEVICE_CAPS != 0 {
            self.device_caps
        } else {
            self.capabilities
        }
    }
}

/// Métadonnées brutes d'un contrôle, décodées de `VIDIOC_QUERYCTRL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlQuery {
    /// Identifiant noyau (flags masqués par le transport).
    pub id: u32,
    /// Type brut (`V4L2_CTRL_TYPE_*`).
    pub kind_raw: u32,
    /// Nom noyau.
    pub name: String,
    /// Borne basse.
    pub min: i32,
    /// Borne haute.
    pub max: i32,
    /// Pas.
    pub step: i32,
    /// Valeur par défaut.
    pub default: i32,
    /// Drapeaux bruts.
    pub flags: u32,
}

/// Entrée de menu brute décodée de `VIDIOC_QUERYMENU`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuQuery {
    /// Libellé (`MENU`).
    pub label: Option<String>,
    /// Valeur entière (`INTEGER_MENU`).
    pub value: Option<i64>,
}

/// Accès bas niveau à un device V4L2, injectable en test.
pub trait V4l2Reader: Send + Sync + std::fmt::Debug {
    /// `VIDIOC_QUERYCAP`.
    fn capabilities(&self) -> Result<DeviceCapabilities>;

    /// `VIDIOC_QUERYCTRL` pour un `id` (éventuellement flaggé `NEXT_CTRL`).
    fn query_control(&self, id: u32) -> Result<ControlQuery>;

    /// `VIDIOC_QUERYMENU`.
    fn query_menu(&self, id: u32, index: i32) -> Result<MenuQuery>;

    /// `VIDIOC_G_CTRL`.
    fn get_control(&self, id: u32) -> Result<i32>;
}

/// Périphérique exposé à l'UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcDevice {
    /// Nom sysfs (`video0`).
    pub node: String,
    /// Chemin du nœud (`/dev/video0`).
    pub path: String,
    /// Nom lisible sanitisé.
    pub name: String,
    /// Identité stable.
    pub identity: UvcIdentity,
    /// `bus_info` de `QUERYCAP`, si l'ouverture a réussi.
    pub bus_info: Option<String>,
    /// Vrai si le device a été ouvert et validé.
    pub available: bool,
}

/// Contrôles lus sur un device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcControls {
    /// Device concerné.
    pub device: UvcDevice,
    /// Contrôles exposés (allowlist uniquement).
    pub controls: Vec<UvcControlInfo>,
    /// Vrai si le budget ou un plafond a tronqué la lecture.
    pub truncated: bool,
    /// Erreur non fatale (ouverture, permission, device occupé).
    pub error: Option<String>,
}

/// Résultat d'énumération des devices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UvcDevicesReport {
    /// Devices capture trouvés (available ou non).
    pub devices: Vec<UvcDevice>,
    /// Erreurs bornées.
    pub errors: Vec<String>,
    /// Vrai si plus de [`MAX_DEVICES`] devices ont été trouvés.
    pub truncated: bool,
}

/// Résultat d'une lecture complète (tous devices).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UvcAllReport {
    /// Contrôles par device.
    pub controls: Vec<UvcControls>,
    /// Erreurs bornées.
    pub errors: Vec<String>,
    /// Vrai si le budget ou un plafond a tronqué la lecture.
    pub truncated: bool,
}

/// Fabrique de transports (production : module interne `real`, tests : [`fake`]).
pub type ReaderFactory = Arc<dyn Fn(&VideoDevice) -> Result<Box<dyn V4l2Reader>> + Send + Sync>;

/// Contrôleur de lecture UVC.
pub struct UvcReaderController {
    root: SysfsRoot,
    factory: ReaderFactory,
}

impl std::fmt::Debug for UvcReaderController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UvcReaderController")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl UvcReaderController {
    /// Contrôleur de production (ouvre `/dev/videoN` en lecture seule).
    #[must_use]
    pub fn system(root: SysfsRoot) -> Self {
        Self {
            root,
            factory: Arc::new(real::open),
        }
    }

    /// Contrôleur avec une fabrique injectée (tests, simulation).
    ///
    /// La fabrique ne doit jamais ouvrir un chemin fourni par un client : elle
    /// reçoit un [`VideoDevice`] issu de [`crate::webcam::enumerate`].
    #[must_use]
    pub fn with_factory(root: SysfsRoot, factory: ReaderFactory) -> Self {
        Self { root, factory }
    }

    /// Énumère les devices capture et lit leurs capacités.
    ///
    /// Aucun contrôle n'est lu ici : seuls `QUERYCAP` et l'identité sysfs sont
    /// utilisés. Un device est listé même s'il ne peut pas être ouvert
    /// (`available: false`, erreur dans le rapport).
    #[must_use]
    pub fn devices(&self) -> UvcDevicesReport {
        let mut report = UvcDevicesReport::default();
        let entries = match webcam::enumerate(&self.root) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push(format!("énumération sysfs: {error}"));
                return report;
            }
        };
        let mut capture: Vec<VideoDevice> = entries
            .into_iter()
            .filter(|device| device.is_capture())
            .collect();
        if capture.len() > MAX_DEVICES {
            report.truncated = true;
            capture.truncate(MAX_DEVICES);
        }

        let mut budget = Budget::new(MAX_IOCTLS_PER_READ);
        for device in capture {
            let identity = device::identity(&self.root, &device);
            let mut uvc = UvcDevice {
                node: device.name.clone(),
                path: device
                    .device_path
                    .clone()
                    .unwrap_or_else(|| format!("/dev/{}", device.name)),
                name: identity.fallback_name.clone(),
                identity,
                bus_info: None,
                available: false,
            };
            if !budget.consume() {
                report.truncated = true;
                report.devices.push(uvc);
                break;
            }
            match (self.factory)(&device) {
                Ok(reader) => match retry_eintr(|| reader.capabilities()) {
                    Ok(capabilities) => {
                        if let Err(error) = check_capabilities(&capabilities, device.index) {
                            report.errors.push(format!("{}: {error}", device.name));
                        } else {
                            uvc.available = true;
                            let bus_info = control::sanitize_text(&capabilities.bus_info, 64);
                            uvc.bus_info = (!bus_info.is_empty()).then_some(bus_info);
                            let card = control::sanitize_text(&capabilities.card, 128);
                            if !card.is_empty() {
                                uvc.name = card;
                            }
                        }
                    }
                    Err(error) => report
                        .errors
                        .push(format!("{}: capabilities: {error}", device.name)),
                },
                Err(error) => report
                    .errors
                    .push(format!("{}: ouverture: {error}", device.name)),
            }
            report.devices.push(uvc);
        }
        trim_errors(&mut report.errors);
        report
    }

    /// Lit les contrôles d'un device avec un budget neuf.
    ///
    /// # Errors
    ///
    /// - device absent/occupé/refusé ;
    /// - `QUERYCAP`/`QUERYCTRL` en erreur (hors fin d'énumération) ;
    /// - contrôle non capture.
    pub fn read_controls(&self, device: &UvcDevice) -> Result<UvcControls> {
        let mut budget = Budget::new(MAX_IOCTLS_PER_READ);
        self.read_controls_budget(device, &mut budget)
    }

    /// Lit tous les devices disponibles avec un **budget global**.
    #[must_use]
    pub fn read_all(&self) -> UvcAllReport {
        let devices = self.devices();
        let mut report = UvcAllReport {
            errors: devices.errors,
            truncated: devices.truncated,
            ..UvcAllReport::default()
        };
        let mut budget = Budget::new(MAX_IOCTLS_PER_READ);
        for device in devices.devices {
            if !device.available {
                report.controls.push(UvcControls {
                    error: Some("device indisponible".to_owned()),
                    device,
                    controls: Vec::new(),
                    truncated: false,
                });
                continue;
            }
            match self.read_controls_budget(&device, &mut budget) {
                Ok(controls) => {
                    report.truncated |= controls.truncated;
                    report.controls.push(controls);
                }
                Err(error) => {
                    report
                        .errors
                        .push(format!("{}: contrôles: {error}", device.node));
                    report.controls.push(UvcControls {
                        error: Some(format!("{error}")),
                        device,
                        controls: Vec::new(),
                        truncated: false,
                    });
                }
            }
            if budget.exhausted() {
                report.truncated = true;
                break;
            }
        }
        trim_errors(&mut report.errors);
        report
    }

    fn read_controls_budget(&self, device: &UvcDevice, budget: &mut Budget) -> Result<UvcControls> {
        let source = VideoDevice {
            name: device.node.clone(),
            device_path: Some(device.path.clone()),
            device_name: Some(device.name.clone()),
            index: Some(0),
            dev: sysfs_dev(&self.root, &device.node),
        };
        let reader = (self.factory)(&source)?;
        let capabilities = retry_eintr(|| reader.capabilities())?;
        check_capabilities(&capabilities, source.index)?;

        let mut controls = Vec::new();
        let mut truncated = false;
        let mut seen = std::collections::HashSet::new();
        let mut last_id: u32 = 0;
        let mut id = abi::V4L2_CTRL_FLAG_NEXT_CTRL;
        let mut iterations = 0usize;

        for _ in 0..MAX_CONTROLS {
            iterations += 1;
            if !budget.consume() {
                truncated = true;
                break;
            }
            let query = match retry_eintr(|| reader.query_control(id)) {
                Ok(query) => query,
                Err(error) if is_invalid_input(&error) => break,
                Err(error) => return Err(error),
            };
            let cid = abi::mask_query_flags(query.id);
            if cid == 0 || cid == last_id {
                break;
            }
            last_id = cid;

            if !control::cid_allowed(cid) {
                id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            }
            let Some(kind) = control::kind_from_raw(query.kind_raw) else {
                id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            };
            let flags = UvcFlags::from_raw(query.flags);
            if !flags.is_readable() || UvcFlags::is_action_or_payload(query.flags) {
                id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            }
            let kind = control::normalize_kind(kind, query.min, query.max, query.step);
            let step = i64::from(query.step.max(1));

            let (menu, menu_truncated) =
                if matches!(kind, UvcControlKind::Menu | UvcControlKind::IntegerMenu) {
                    let (menu, truncated) =
                        read_menu(reader.as_ref(), cid, query.min, query.max, kind, budget);
                    (menu, truncated)
                } else {
                    (Vec::new(), false)
                };
            truncated |= menu_truncated;

            let current = if budget.consume() {
                match retry_eintr(|| reader.get_control(cid)) {
                    Ok(raw) => Some(value_from_raw(kind, raw, &menu)),
                    Err(error) => {
                        tracing::debug!(cid = format_args!("0x{cid:08x}"), %error, "valeur de contrôle illisible");
                        None
                    }
                }
            } else {
                truncated = true;
                None
            };

            let name = control::sanitize_text(&query.name, 64);
            let base_key = control::to_key(&name, cid);
            let key = unique_key(&base_key, cid, &mut seen);
            controls.push(UvcControlInfo {
                key,
                name,
                kind,
                min: i64::from(query.min),
                max: i64::from(query.max),
                step,
                default: i64::from(query.default),
                current,
                menu,
                flags,
                category: control::category_for(cid),
                simple: control::is_simple(cid),
            });

            id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
        }
        if iterations >= MAX_CONTROLS {
            truncated = true;
        }
        Ok(UvcControls {
            device: device.clone(),
            controls,
            truncated,
            error: None,
        })
    }
}

/// Lit les entrées d'un menu, avec cap et budget.
fn read_menu(
    reader: &dyn V4l2Reader,
    cid: u32,
    min: i32,
    max: i32,
    kind: UvcControlKind,
    budget: &mut Budget,
) -> (Vec<UvcMenuEntry>, bool) {
    let mut menu = Vec::new();
    let mut truncated = false;
    if max < min {
        return (menu, true);
    }
    let count = i64::from(max) - i64::from(min) + 1;
    if count > MAX_MENU_ENTRIES as i64 {
        truncated = true;
    }
    for index in min..=max {
        if menu.len() >= MAX_MENU_ENTRIES {
            truncated = true;
            break;
        }
        if !budget.consume() {
            truncated = true;
            break;
        }
        let entry = match retry_eintr(|| reader.query_menu(cid, index)) {
            Ok(entry) => entry,
            Err(error) if is_invalid_input(&error) => continue,
            Err(error) => {
                tracing::debug!(
                    cid = format_args!("0x{cid:08x}"),
                    index,
                    %error,
                    "entrée de menu illisible"
                );
                continue;
            }
        };
        let (key, label) = match kind {
            UvcControlKind::IntegerMenu => {
                let value = entry.value.unwrap_or(i64::from(index));
                (value.to_string(), value.to_string())
            }
            _ => {
                let label = control::sanitize_text(entry.label.as_deref().unwrap_or(""), 64);
                let key = if label.is_empty() {
                    format!("menu_{index}")
                } else {
                    control::to_key(&label, cid)
                };
                let label = if label.is_empty() { key.clone() } else { label };
                (key, label)
            }
        };
        menu.push(UvcMenuEntry {
            index: i64::from(index),
            key,
            label,
        });
    }
    (menu, truncated)
}

/// Désambiguïse une clé dupliquée sans exposer l'identifiant noyau.
pub(crate) fn unique_key(
    base: &str,
    cid: u32,
    seen: &mut std::collections::HashSet<String>,
) -> String {
    if seen.insert(base.to_owned()) {
        return base.to_owned();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}_{suffix}");
        if seen.insert(candidate.clone()) {
            tracing::debug!(
                cid = format_args!("0x{cid:08x}"),
                key = %candidate,
                "clé de contrôle dupliquée désambiguïsée"
            );
            return candidate;
        }
        suffix = suffix.saturating_add(1);
        if suffix > 64 {
            return format!("{base}_dup");
        }
    }
}

fn value_from_raw(kind: UvcControlKind, raw: i32, menu: &[UvcMenuEntry]) -> UvcValue {
    match kind {
        UvcControlKind::Boolean => UvcValue::Boolean(raw != 0),
        UvcControlKind::Integer => UvcValue::Integer(i64::from(raw)),
        UvcControlKind::Menu | UvcControlKind::IntegerMenu => {
            let index = i64::from(raw);
            if let Some(entry) = menu.iter().find(|entry| entry.index == index) {
                UvcValue::Menu {
                    index,
                    key: entry.key.clone(),
                    label: entry.label.clone(),
                }
            } else {
                UvcValue::Menu {
                    index,
                    key: raw.to_string(),
                    label: raw.to_string(),
                }
            }
        }
    }
}

/// Vérifie les capacités et le type de nœud (V-07).
pub(crate) fn check_capabilities(
    capabilities: &DeviceCapabilities,
    index: Option<u32>,
) -> Result<()> {
    if index.is_some_and(|index| index > 0) {
        return Err(HwError::NotSupported("nœud metadata V4L2"));
    }
    let caps = capabilities.effective_caps();
    if caps & abi::V4L2_CAP_META_CAPTURE != 0 {
        return Err(HwError::NotSupported("nœud meta-capture V4L2"));
    }
    if caps & abi::V4L2_CAP_VIDEO_CAPTURE == 0 {
        return Err(HwError::NotSupported("nœud V4L2 sans capture vidéo"));
    }
    Ok(())
}

/// Compteur d'ioctl borné.
#[derive(Debug)]
struct Budget {
    remaining: u32,
}

impl Budget {
    fn new(remaining: u32) -> Self {
        Self { remaining }
    }

    fn consume(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }

    fn exhausted(&self) -> bool {
        self.remaining == 0
    }
}

/// Rejoue un appel en cas d'`EINTR`, de façon bornée.
fn retry_eintr<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    let mut attempt = 0;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if attempt + 1 >= EINTR_ATTEMPTS || !is_interrupted(&error) {
                    return Err(error);
                }
                attempt += 1;
                std::thread::sleep(EINTR_DELAY);
            }
        }
    }
}

pub(crate) fn is_interrupted(error: &HwError) -> bool {
    matches!(error, HwError::Ioctl { source, .. } if source.kind() == io::ErrorKind::Interrupted)
}

pub(crate) fn is_invalid_input(error: &HwError) -> bool {
    matches!(error, HwError::Ioctl { source, .. } if source.kind() == io::ErrorKind::InvalidInput)
}

fn sysfs_dev(root: &SysfsRoot, node: &str) -> Option<String> {
    root.read_optional_string(format!("class/video4linux/{node}/dev"))
        .ok()
        .flatten()
}

fn trim_errors(errors: &mut Vec<String>) {
    const MAX_ERRORS: usize = 8;
    errors.truncate(MAX_ERRORS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v4l2::fake::{FakeControl, FakeReader};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn fixture(root: &Path, count: usize) {
        for index in 0..count {
            let node = format!("video{index}");
            write(
                root,
                &format!("class/video4linux/{node}/name"),
                "Integrated Camera\n",
            );
            write(root, &format!("class/video4linux/{node}/index"), "0\n");
            write(
                root,
                &format!("class/video4linux/{node}/dev"),
                &format!("81:{index}\n"),
            );
            write(
                root,
                &format!("class/video4linux/{node}/device/uevent"),
                "PRODUCT=1bcf/2c99/7200\n",
            );
        }
    }

    fn caps() -> DeviceCapabilities {
        DeviceCapabilities {
            driver: "uvcvideo".to_owned(),
            card: "Integrated Camera".to_owned(),
            bus_info: "usb-0000:00:14.0-1".to_owned(),
            capabilities: abi::V4L2_CAP_VIDEO_CAPTURE | abi::V4L2_CAP_DEVICE_CAPS,
            device_caps: abi::V4L2_CAP_VIDEO_CAPTURE,
        }
    }

    fn brightness() -> FakeControl {
        FakeControl::new(
            control::CID_BRIGHTNESS,
            "Brightness",
            abi::V4L2_CTRL_TYPE_INTEGER,
        )
        .range(0, 255, 1)
        .default(128)
        .value(42)
    }

    fn controller(root: &Path, reader: FakeReader) -> (UvcReaderController, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let reader = Arc::new(reader);
        let controller = UvcReaderController::with_factory(
            SysfsRoot::new(root),
            Arc::new(move |_device| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(FakeReader::clone_state(&reader)))
            }),
        );
        (controller, calls)
    }

    #[test]
    fn enumerates_capture_devices_and_identity() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let (controller, _) = controller(
            dir.path(),
            FakeReader::new(caps()).with_control(brightness()),
        );
        let report = controller.devices();
        assert_eq!(report.devices.len(), 1);
        assert!(report.devices[0].available);
        assert_eq!(report.devices[0].identity.vendor.as_deref(), Some("1bcf"));
        assert_eq!(report.devices[0].identity.product.as_deref(), Some("2c99"));
        assert_eq!(report.devices[0].name, "Integrated Camera");
        assert_eq!(
            report.devices[0].bus_info.as_deref(),
            Some("usb-0000:00:14.0-1")
        );
    }

    #[test]
    fn reads_only_allowlisted_controls() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let reader = FakeReader::new(caps())
            .with_control(brightness())
            .with_control(
                FakeControl::new(0x009f_0903, "Test Pattern", abi::V4L2_CTRL_TYPE_MENU)
                    .range(0, 1, 1),
            )
            .with_control(
                FakeControl::new(0x0800_0001, "Private", abi::V4L2_CTRL_TYPE_INTEGER)
                    .range(0, 10, 1),
            )
            .with_control(
                FakeControl::new(
                    0x0098_0904,
                    "Auto White Balance",
                    abi::V4L2_CTRL_TYPE_BOOLEAN,
                )
                .range(0, 1, 1),
            );
        let (controller, _) = controller(dir.path(), reader);
        let report = controller.read_all();
        assert_eq!(report.controls.len(), 1);
        let controls = &report.controls[0].controls;
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].key, "brightness");
        assert_eq!(controls[0].current, Some(UvcValue::Integer(42)));
        assert!(controls[0].simple);
        assert_eq!(controls[0].category, UvcCategory::Color);
    }

    #[test]
    fn reads_menu_entries_lazily_and_maps_value() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let reader = FakeReader::new(caps()).with_control(
            FakeControl::new(
                control::CID_POWER_LINE_FREQUENCY,
                "Power Line Frequency",
                abi::V4L2_CTRL_TYPE_MENU,
            )
            .range(0, 2, 1)
            .value(1)
            .menu(&[(0, "Disabled"), (1, "50 Hz"), (2, "60 Hz")]),
        );
        let (controller, _) = controller(dir.path(), reader);
        let report = controller.read_all();
        let control = &report.controls[0].controls[0];
        assert_eq!(control.key, "power_line_frequency");
        assert_eq!(control.menu.len(), 3);
        assert_eq!(control.menu[1].index, 1);
        assert_eq!(control.menu[1].key, "50_hz");
        assert_eq!(
            control.current,
            Some(UvcValue::Menu {
                index: 1,
                key: "50_hz".to_owned(),
                label: "50 Hz".to_owned()
            })
        );
    }

    #[test]
    fn eintr_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let reader = FakeReader::new(caps())
            .with_control(brightness())
            .with_eintr_once();
        let (controller, _) = controller(dir.path(), reader);
        let report = controller.read_all();
        assert_eq!(report.controls[0].controls.len(), 1);
        assert_eq!(
            report.controls[0].controls[0].current,
            Some(UvcValue::Integer(42))
        );
    }

    #[test]
    fn missing_capture_or_meta_caps_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let mut meta = caps();
        meta.device_caps = abi::V4L2_CAP_META_CAPTURE;
        let (controller, _) = controller(dir.path(), FakeReader::new(meta));
        let report = controller.devices();
        assert!(!report.devices[0].available);
        assert!(report.errors.iter().any(|error| error.contains("meta")));
    }

    #[test]
    fn absent_device_and_busy_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let (controller, _) = controller(
            dir.path(),
            FakeReader::new(caps()).fail_capabilities(io::ErrorKind::WouldBlock),
        );
        let report = controller.devices();
        assert!(!report.devices[0].available);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("capabilities"))
        );

        let report = controller.read_all();
        assert_eq!(report.controls.len(), 1);
        assert!(report.controls[0].error.is_some());
    }

    #[test]
    fn budget_truncates_long_control_lists() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let mut reader = FakeReader::new(caps());
        // 600 contrôles : la boucle est bornée à MAX_CONTROLS et le budget
        // global à MAX_IOCTLS_PER_READ. Le résultat doit être tronqué, sans
        // panic ni boucle.
        for index in 0..600u32 {
            reader = reader.with_control(
                FakeControl::new(
                    0x0098_0900 + index,
                    &format!("Control {index}"),
                    abi::V4L2_CTRL_TYPE_INTEGER,
                )
                .range(0, 10, 1)
                .value(1),
            );
        }
        let (controller, _) = controller(dir.path(), reader);
        let report = controller.read_all();
        assert_eq!(report.controls.len(), 1);
        assert!(report.controls[0].truncated || report.truncated);
        assert!(report.controls[0].controls.len() <= MAX_CONTROLS);
    }

    #[test]
    fn menu_entries_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 1);
        let mut menu = Vec::new();
        for index in 0..200i32 {
            menu.push((index, format!("Entry {index}")));
        }
        let entries: Vec<(i32, &str)> = menu
            .iter()
            .map(|(index, label)| (*index, label.as_str()))
            .collect();
        let reader = FakeReader::new(caps()).with_control(
            FakeControl::new(
                control::CID_POWER_LINE_FREQUENCY,
                "Power Line Frequency",
                abi::V4L2_CTRL_TYPE_MENU,
            )
            .range(0, 199, 1)
            .value(1)
            .menu(&entries),
        );
        let (controller, _) = controller(dir.path(), reader);
        let report = controller.read_all();
        let control = &report.controls[0].controls[0];
        assert_eq!(control.menu.len(), MAX_MENU_ENTRIES);
        assert!(report.controls[0].truncated);
    }

    #[test]
    fn caps_devices_at_sixteen() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), 20);
        let (controller, _) = controller(
            dir.path(),
            FakeReader::new(caps()).with_control(brightness()),
        );
        let report = controller.devices();
        assert_eq!(report.devices.len(), MAX_DEVICES);
        assert!(report.truncated);
    }

    #[test]
    fn no_camera_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (controller, _) = controller(dir.path(), FakeReader::new(caps()));
        let report = controller.devices();
        assert!(report.devices.is_empty());
        assert!(report.errors.is_empty());
        let all = controller.read_all();
        assert!(all.controls.is_empty());
        assert!(!all.truncated);
    }

    #[test]
    fn factory_is_not_called_without_devices() {
        let dir = tempfile::tempdir().unwrap();
        let (controller, calls) = controller(dir.path(), FakeReader::new(caps()));
        let _ = controller.read_all();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn capabilities_effective_device_caps() {
        let mut capabilities = caps();
        capabilities.capabilities = abi::V4L2_CAP_DEVICE_CAPS; // capture absente
        capabilities.device_caps = abi::V4L2_CAP_VIDEO_CAPTURE;
        assert_eq!(
            capabilities.effective_caps() & abi::V4L2_CAP_VIDEO_CAPTURE,
            abi::V4L2_CAP_VIDEO_CAPTURE
        );
        capabilities.capabilities = abi::V4L2_CAP_VIDEO_CAPTURE;
        capabilities.device_caps = abi::V4L2_CAP_META_CAPTURE;
        assert_eq!(
            capabilities.effective_caps() & abi::V4L2_CAP_VIDEO_CAPTURE,
            abi::V4L2_CAP_VIDEO_CAPTURE
        );
    }

    #[test]
    fn unique_key_never_exposes_raw_id() {
        let mut seen = std::collections::HashSet::new();
        assert_eq!(unique_key("brightness", 0x1234, &mut seen), "brightness");
        assert_eq!(unique_key("brightness", 0x1234, &mut seen), "brightness_2");
        assert_eq!(unique_key("brightness", 0x1234, &mut seen), "brightness_3");
    }
}
