// SPDX-License-Identifier: GPL-3.0-or-later
//! Écriture des contrôles UVC — étape 2, réservée à `tccd-user`.
//!
//! Ce module est le **seul** point d'écriture V4L2. Il n'est jamais appelé par
//! `tccd` (daemon root) ni par la GUI. `tccd-user`, agent de session non
//! privilégié, l'expose via `User1.SetUvcControl` et
//! `User1.GetUvcWriteSupported` (contrat `docs/DBUS_API.md`) ; aucune action
//! polkit n'est requise, l'accès au device reposant sur l'ACL `uaccess` ou le
//! groupe `video`.
//!
//! # Contrat
//!
//! `set_control(device_path, control_key, value)` :
//! - `device_path` doit être exactement `/dev/videoN` (chemin canonique) ;
//! - `control_key` est une clé stable de l'allowlist de lecture ;
//! - `value` est un entier : valeur bornée, `0`/`1` pour un booléen, index pour
//!   un menu.
//!
//! # Sécurité
//!
//! - ouverture `O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC` dans `real_write.rs` ;
//! - validation `fstat` (caractère, major 81, minor sysfs, uid root) et
//!   `device_caps` capture ;
//! - allowlist fermée des 15 CIDs : aucun CID privé, aucun contrôle hors
//!   liste ;
//! - `QUERYCTRL`/`G_CTRL` avant écriture, validation type/min/max/step/menu,
//!   `S_CTRL` puis relecture `G_CTRL` ;
//! - budget de [`MAX_IOCTLS_PER_WRITE`] ioctl et délai [`WRITE_TIMEOUT`] ;
//! - **une seule écriture en vol par processus** (porte globale) et
//!   `try_lock` par device ; `EBUSY` ⇒ [`HwError::Busy`] ;
//!   `EIO`/débranchement ⇒ [`HwError::Stalled`] avec quarantaine
//!   [`STALL_COOLDOWN`]. Un `S_CTRL` bloqué ne peut donc immobiliser qu'un
//!   thread `spawn_blocking` ; un watchdog côté service reste nécessaire pour
//!   borner un pilote pathologique.
//!
//! `User1.SetUvcControl` retourne `true` si la valeur a changé et `false` si
//! elle était déjà courante (no-op). Le service `tccd-user` ajoute la
//! validation du chemin énuméré, le rate limit par UID et les erreurs D-Bus
//! stables (`InvalidArgs`, `NotSupported`, `OutOfRange`, `Busy`, `Stalled`,
//! `Failed`, `LimitsExceeded`).
//!
//! **Aucun `S_CTRL` dans les chemins de lecture** : `real.rs` et le daemon
//! n'utilisent que `QUERYCAP`/`QUERYCTRL`/`QUERYMENU`/`G_CTRL`.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;
use crate::webcam::VideoDevice;

use super::{
    ControlQuery, DeviceCapabilities, MenuQuery, abi, check_capabilities, control, is_interrupted,
    unique_key,
};

/// Nombre maximal d'ioctl pour une écriture complète.
pub const MAX_IOCTLS_PER_WRITE: u32 = 64;
/// Délai maximal d'une écriture complète (hors blocage noyau imprévisible).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Durée pendant laquelle un device en erreur `EIO`/débranché reste `stalled`.
pub const STALL_COOLDOWN: Duration = Duration::from_secs(30);
/// Nombre maximal de devices suivis pour les verrous et l'état `stalled`.
/// Aligné sur [`super::MAX_DEVICES`] (16).
const MAX_TRACKED_DEVICES: usize = super::MAX_DEVICES;
/// Tentatives en cas d'`EINTR`.
const EINTR_ATTEMPTS: u32 = 3;
/// Délai entre deux tentatives `EINTR`.
const EINTR_DELAY: Duration = Duration::from_millis(5);

/// Transport d'écriture V4L2, injectable en test.
pub trait UvcWriter: Send + Sync + std::fmt::Debug {
    /// `VIDIOC_QUERYCAP`.
    fn capabilities(&self) -> Result<DeviceCapabilities>;
    /// `VIDIOC_QUERYCTRL`.
    fn query_control(&self, id: u32) -> Result<ControlQuery>;
    /// `VIDIOC_QUERYMENU`.
    fn query_menu(&self, id: u32, index: i32) -> Result<MenuQuery>;
    /// `VIDIOC_G_CTRL`.
    fn get_control(&self, id: u32) -> Result<i32>;
    /// `VIDIOC_S_CTRL` — **seul point d'écriture**.
    fn set_control(&self, id: u32, value: i32) -> Result<()>;
}

/// Fabrique de transports d'écriture (production : `real_write`).
pub type WriterFactory = Arc<dyn Fn(&VideoDevice) -> Result<Box<dyn UvcWriter>> + Send + Sync>;

/// Résultat d'une écriture réussie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcWriteReport {
    /// Chemin demandé (`/dev/video0`).
    pub device: String,
    /// Clé stable du contrôle.
    pub key: String,
    /// Valeur brute avant écriture.
    pub previous: i32,
    /// Valeur brute demandée/relue.
    pub value: i32,
    /// Faux si la valeur était déjà celle du matériel (aucun `S_CTRL`).
    pub changed: bool,
}

/// Contrôleur d'écriture UVC.
///
/// `tccd-user` doit partager **une seule instance** : `try_lock` et l'état
/// `stalled` sont internes au contrôleur. Deux instances dans le même
/// processus ne se sérialisent pas entre elles (le noyau arbitre l'accès au
/// device, mais le `Busy` applicatif n'est pas garanti).
pub struct UvcWriterController {
    root: SysfsRoot,
    factory: WriterFactory,
    /// Porte globale : une seule séquence d'écriture complète par processus.
    gate: Mutex<()>,
    devices: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    stalled: Mutex<HashMap<String, Instant>>,
}

impl std::fmt::Debug for UvcWriterController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UvcWriterController")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl UvcWriterController {
    /// Contrôleur de production (ouvre `/dev/videoN` en `O_RDWR`).
    #[must_use]
    pub fn system(root: SysfsRoot) -> Self {
        Self::with_factory(root, Arc::new(super::real_write::open))
    }

    /// Contrôleur avec une fabrique injectée (tests, simulation).
    #[must_use]
    pub fn with_factory(root: SysfsRoot, factory: WriterFactory) -> Self {
        Self {
            root,
            factory,
            gate: Mutex::new(()),
            devices: Mutex::new(HashMap::new()),
            stalled: Mutex::new(HashMap::new()),
        }
    }

    /// Écrit un contrôle sur un device.
    ///
    /// `device_path` doit être **exactement** `/dev/videoN` ; aucun chemin
    /// arbitraire, aucun lien symbolique. `value` est validé contre
    /// `QUERYCTRL` avant toute écriture.
    ///
    /// # Errors
    ///
    /// - [`HwError::InvalidValue`]/[`HwError::OutOfRange`] : chemin, clé ou
    ///   valeur invalide ;
    /// - [`HwError::NotSupported`] : contrôle absent de l'allowlist ou non
    ///   inscriptible ;
    /// - [`HwError::Busy`] : device déjà en cours d'écriture ou `EBUSY` ;
    /// - [`HwError::Stalled`] : `EIO`, débranchement, timeout ou budget ;
    /// - [`HwError::Ioctl`]/[`HwError::Io`] : erreur noyau.
    pub fn set_control(
        &self,
        device_path: &str,
        control_key: &str,
        value: i64,
    ) -> Result<UvcWriteReport> {
        let (name, canonical) = parse_device_path(device_path)?;
        validate_control_key(control_key)?;
        self.expire_stalls();
        if self.is_stalled(&name) {
            return Err(HwError::Stalled { device: name });
        }

        // Une seule séquence `QUERYCAP` → `S_CTRL` → relecture à la fois dans
        // le processus. Un appel concurrent ne doit pas consommer un second
        // thread `spawn_blocking` ni attendre un ioctl bloqué.
        let _gate = match self.gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(HwError::Busy { device: name });
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };

        let lock = self.device_lock(&name)?;
        let _guard = match lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(HwError::Busy { device: name });
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };

        let result = self.set_control_locked(&name, &canonical, control_key, value);
        if matches!(result, Err(HwError::Stalled { .. })) {
            self.mark_stalled(&name);
        }
        result
    }

    fn set_control_locked(
        &self,
        name: &str,
        canonical: &str,
        control_key: &str,
        value: i64,
    ) -> Result<UvcWriteReport> {
        let device = read_device(&self.root, name, canonical)?;
        let writer = (self.factory)(&device)?;
        let capabilities = writer.capabilities()?;
        check_capabilities(&capabilities, device.index)?;
        verify_identity(&self.root, name, &capabilities)?;

        let mut budget = WriteBudget::new();
        let found = find_control(writer.as_ref(), control_key, &mut budget, name)?;
        validate_requested_value(writer.as_ref(), &found, value, &mut budget, name)?;
        let requested = i32::try_from(value).map_err(|_| HwError::OutOfRange {
            context: "valeur UVC",
            value,
            min: i64::from(found.query.min),
            max: i64::from(found.query.max),
        })?;

        let previous = call_budgeted(|| writer.get_control(found.cid), &mut budget, name)?;
        if previous == requested {
            return Ok(UvcWriteReport {
                device: canonical.to_owned(),
                key: control_key.to_owned(),
                previous,
                value: requested,
                changed: false,
            });
        }

        call_budgeted(
            || writer.set_control(found.cid, requested),
            &mut budget,
            name,
        )?;
        let after = call_budgeted(|| writer.get_control(found.cid), &mut budget, name)?;
        if after != requested {
            // Le firmware peut clamper/arrondir : tenter de restaurer la
            // valeur précédente, sans masquer l'écart constaté.
            let restored = call_budgeted(
                || writer.set_control(found.cid, previous),
                &mut budget,
                name,
            )
            .is_ok();
            let restore = if restored {
                "restauration précédente OK"
            } else {
                "restauration précédente en échec"
            };
            return Err(HwError::InvalidValue {
                context: "relecture UVC après écriture",
                value: format!(
                    "après={after} demandé={requested} précédent={previous} ; {restore}"
                ),
            });
        }

        Ok(UvcWriteReport {
            device: canonical.to_owned(),
            key: control_key.to_owned(),
            previous,
            value: after,
            changed: true,
        })
    }

    fn device_lock(&self, name: &str) -> Result<Arc<Mutex<()>>> {
        let mut devices = lock_or_recover(&self.devices);
        devices.retain(|_, lock| Arc::strong_count(lock) > 1);
        if let Some(lock) = devices.get(name) {
            return Ok(Arc::clone(lock));
        }
        if devices.len() >= MAX_TRACKED_DEVICES {
            return Err(HwError::Busy {
                device: name.to_owned(),
            });
        }
        let lock = Arc::new(Mutex::new(()));
        devices.insert(name.to_owned(), Arc::clone(&lock));
        Ok(lock)
    }

    fn is_stalled(&self, name: &str) -> bool {
        let stalled = lock_or_recover(&self.stalled);
        stalled
            .get(name)
            .is_some_and(|at| at.elapsed() < STALL_COOLDOWN)
    }

    fn mark_stalled(&self, name: &str) {
        let mut stalled = lock_or_recover(&self.stalled);
        stalled.retain(|_, at| at.elapsed() < STALL_COOLDOWN);
        if stalled.len() >= MAX_TRACKED_DEVICES
            && let Some(oldest) = stalled
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(name, _)| name.clone())
        {
            stalled.remove(&oldest);
        }
        stalled.insert(name.to_owned(), Instant::now());
    }

    fn expire_stalls(&self) {
        let mut stalled = lock_or_recover(&self.stalled);
        stalled.retain(|_, at| at.elapsed() < STALL_COOLDOWN);
    }
}

/// Contrôle retenu pour l'écriture.
#[derive(Debug)]
struct FoundControl {
    cid: u32,
    query: ControlQuery,
    kind: control::UvcControlKind,
}

/// Trouve un contrôle inscriptible par sa clé stable.
fn find_control(
    writer: &dyn UvcWriter,
    control_key: &str,
    budget: &mut WriteBudget,
    device: &str,
) -> Result<FoundControl> {
    let mut seen = HashSet::new();
    let mut last_id = 0u32;
    let mut id = abi::V4L2_CTRL_FLAG_NEXT_CTRL;

    for _ in 0..super::MAX_CONTROLS {
        let query = match call_budgeted(|| writer.query_control(id), budget, device) {
            Ok(query) => query,
            Err(HwError::Ioctl { source, .. })
                if source.kind() == std::io::ErrorKind::InvalidInput =>
            {
                break;
            }
            Err(error) => return Err(error),
        };
        let cid = abi::mask_query_flags(query.id);
        if cid == 0 || cid == last_id {
            break;
        }
        last_id = cid;

        if control::cid_allowed(cid) {
            let Some(kind) = control::kind_from_raw(query.kind_raw) else {
                id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            };
            let flags = control::UvcFlags::from_raw(query.flags);
            if !flags.is_writable() || control::UvcFlags::is_action_or_payload(query.flags) {
                id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            }
            let name = control::sanitize_text(&query.name, 64);
            let key = unique_key(&control::to_key(&name, cid), cid, &mut seen);
            if key == control_key {
                return Ok(FoundControl { cid, query, kind });
            }
        }
        id = cid | abi::V4L2_CTRL_FLAG_NEXT_CTRL;
    }

    Err(HwError::NotSupported("contrôle UVC"))
}

/// Valide la valeur demandée contre les métadonnées du contrôle.
fn validate_requested_value(
    writer: &dyn UvcWriter,
    found: &FoundControl,
    value: i64,
    budget: &mut WriteBudget,
    device: &str,
) -> Result<()> {
    let min = i64::from(found.query.min);
    let max = i64::from(found.query.max);
    let raw = i32::try_from(value).map_err(|_| HwError::OutOfRange {
        context: "valeur UVC (i32)",
        value,
        min,
        max,
    })?;

    // Pour INTEGER_MENU, `min..=max` est l'intervalle des **index**, pas des
    // valeurs : la valeur demandée doit correspondre à `QUERYMENU.value`.
    let value_range = !matches!(found.kind, control::UvcControlKind::IntegerMenu);
    if value_range && (value < min || value > max) {
        return Err(HwError::OutOfRange {
            context: "valeur UVC",
            value,
            min,
            max,
        });
    }

    match found.kind {
        control::UvcControlKind::Boolean => {
            if value != 0 && value != 1 {
                return Err(HwError::OutOfRange {
                    context: "booléen UVC",
                    value,
                    min: 0,
                    max: 1,
                });
            }
        }
        control::UvcControlKind::Integer => {
            let step = i64::from(found.query.step);
            if step <= 0 {
                return Err(HwError::InvalidValue {
                    context: "step UVC",
                    value: found.query.step.to_string(),
                });
            }
            if (value - min) % step != 0 {
                return Err(HwError::InvalidValue {
                    context: "valeur UVC incompatible avec le pas",
                    value: value.to_string(),
                });
            }
        }
        control::UvcControlKind::Menu => {
            match call_budgeted(|| writer.query_menu(found.cid, raw), budget, device) {
                Ok(_) => {}
                Err(HwError::Ioctl { source, .. })
                    if source.kind() == std::io::ErrorKind::InvalidInput =>
                {
                    return Err(HwError::InvalidValue {
                        context: "index de menu UVC",
                        value: value.to_string(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        control::UvcControlKind::IntegerMenu => {
            let mut found_value = false;
            for index in found.query.min..=found.query.max {
                match call_budgeted(|| writer.query_menu(found.cid, index), budget, device) {
                    Ok(entry) if entry.value == Some(value) => {
                        found_value = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(HwError::Ioctl { source, .. })
                        if source.kind() == std::io::ErrorKind::InvalidInput =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            if !found_value {
                return Err(HwError::InvalidValue {
                    context: "valeur absente du menu entier UVC",
                    value: value.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Accesseur de la valeur courante, avec budget et `EINTR` borné.
fn call_budgeted<T>(
    mut operation: impl FnMut() -> Result<T>,
    budget: &mut WriteBudget,
    device: &str,
) -> Result<T> {
    let mut last = None;
    for attempt in 0..EINTR_ATTEMPTS {
        budget.consume(device)?;
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_interrupted(&error) && attempt + 1 < EINTR_ATTEMPTS => {
                std::thread::sleep(EINTR_DELAY);
                last = Some(error);
            }
            Err(error) => return Err(classify_ioctl(error, device)),
        }
    }
    Err(last.unwrap_or_else(|| HwError::Stalled {
        device: device.to_owned(),
    }))
}

/// Convertit les erreurs d'ioctl en erreurs de contrat.
fn classify_ioctl(error: HwError, device: &str) -> HwError {
    if let HwError::Ioctl { source, .. } = &error {
        match source.raw_os_error() {
            Some(libc::EBUSY) => {
                return HwError::Busy {
                    device: device.to_owned(),
                };
            }
            Some(libc::EIO | libc::ENODEV | libc::ENXIO) => {
                return HwError::Stalled {
                    device: device.to_owned(),
                };
            }
            _ => {}
        }
    }
    error
}

/// Vérifie que le fd ouvert correspond toujours au device énuméré dans sysfs.
///
/// `fstat` couvre déjà le major/minor ; `QUERYCAP` fournit en plus le pilote,
/// la carte et le `bus_info`. Un remplacement de device avec le même minor
/// entre la lecture sysfs et l'ouverture est ainsi détecté avant `S_CTRL`.
fn verify_identity(root: &SysfsRoot, name: &str, capabilities: &DeviceCapabilities) -> Result<()> {
    let base = format!("class/video4linux/{name}");
    let uevent = root
        .read_optional_string(format!("{base}/device/uevent"))
        .ok()
        .flatten()
        .unwrap_or_default();
    let expected_driver = uevent.lines().find_map(|line| {
        line.strip_prefix("DRIVER=")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    let expected_card = root
        .read_optional_string(format!("{base}/name"))
        .ok()
        .flatten()
        .map(|value| control::sanitize_text(&value, 128))
        .filter(|value| !value.is_empty());
    let expected_bus = expected_bus_info(root, name);

    if let Some(expected) = expected_driver
        && !capabilities.driver.eq_ignore_ascii_case(&expected)
    {
        tracing::warn!(
            device = name,
            expected,
            actual = %capabilities.driver,
            "identité UVC incohérente : pilote différent"
        );
        return Err(HwError::Stalled {
            device: name.to_owned(),
        });
    }
    if let Some(expected) = expected_card {
        let prefix: String = expected.chars().take(31).collect();
        if !capabilities.card.starts_with(&prefix) {
            tracing::warn!(
                device = name,
                expected = %expected,
                actual = %capabilities.card,
                "identité UVC incohérente : carte différente"
            );
            return Err(HwError::Stalled {
                device: name.to_owned(),
            });
        }
    }
    if let Some(expected) = expected_bus
        && !capabilities.bus_info.eq_ignore_ascii_case(&expected)
    {
        tracing::warn!(
            device = name,
            expected,
            actual = %capabilities.bus_info,
            "identité UVC incohérente : bus différent"
        );
        return Err(HwError::Stalled {
            device: name.to_owned(),
        });
    }
    Ok(())
}

/// Dérive `usb-<pci>-<port>` du lien `/sys/class/video4linux/videoN/device`.
fn expected_bus_info(root: &SysfsRoot, name: &str) -> Option<String> {
    let link = root
        .resolve(format!("class/video4linux/{name}/device"))
        .ok()?;
    let canonical = std::fs::canonicalize(link).ok()?;
    let parts: Vec<&str> = canonical
        .iter()
        .filter_map(|component| component.to_str())
        .collect();
    let usb_index = parts.iter().position(|part| {
        part.strip_prefix("usb")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
    })?;
    let pci = parts.get(usb_index.checked_sub(1)?)?;
    let device = parts.get(usb_index + 1)?;
    let (_, devpath) = device.split_once('-')?;
    Some(format!("usb-{pci}-{devpath}"))
}

/// Budget d'ioctl et échéance d'une écriture.
#[derive(Debug)]
struct WriteBudget {
    remaining: u32,
    deadline: Instant,
}

impl WriteBudget {
    fn new() -> Self {
        Self {
            remaining: MAX_IOCTLS_PER_WRITE,
            deadline: Instant::now() + WRITE_TIMEOUT,
        }
    }

    fn consume(&mut self, device: &str) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(HwError::Stalled {
                device: device.to_owned(),
            });
        }
        if self.remaining == 0 {
            return Err(HwError::BudgetExceeded {
                operation: "UVC",
                limit: MAX_IOCTLS_PER_WRITE,
            });
        }
        self.remaining -= 1;
        Ok(())
    }
}

/// Parse un chemin `/dev/videoN` canonique.
fn parse_device_path(path: &str) -> Result<(String, String)> {
    let name = path
        .strip_prefix("/dev/")
        .ok_or_else(|| HwError::InvalidPath { path: path.into() })?;
    let number = name
        .strip_prefix("video")
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| HwError::InvalidPath { path: path.into() })?;
    let number: u32 = number
        .parse()
        .map_err(|_| HwError::InvalidPath { path: path.into() })?;
    // Refuse les zéros de tête (`video01`) et les indices absurdes.
    if number > 4096 || number.to_string() != name.trim_start_matches("video") {
        return Err(HwError::InvalidPath { path: path.into() });
    }
    Ok((name.to_owned(), path.to_owned()))
}

/// Valide la forme d'une clé stable.
fn validate_control_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(HwError::InvalidValue {
            context: "clé de contrôle UVC",
            value: key.chars().take(64).collect(),
        });
    }
    Ok(())
}

/// Lit l'identité sysfs du nœud demandé.
fn read_device(root: &SysfsRoot, name: &str, canonical: &str) -> Result<VideoDevice> {
    let base = format!("class/video4linux/{name}");
    let dev = root
        .read_optional_string(format!("{base}/dev"))
        .map_err(|_| HwError::NotFound {
            path: root.resolve(&base).unwrap_or_else(|_| canonical.into()),
        })?
        .ok_or_else(|| HwError::NotFound {
            path: root.resolve(&base).unwrap_or_else(|_| canonical.into()),
        })?;
    let index = root
        .read_optional_string(format!("{base}/index"))
        .ok()
        .flatten()
        .and_then(|value| value.trim().parse::<u32>().ok());
    let device_name = root
        .read_optional_string(format!("{base}/name"))
        .ok()
        .flatten();
    Ok(VideoDevice {
        name: name.to_owned(),
        device_path: Some(canonical.to_owned()),
        device_name,
        index,
        dev: Some(dev),
    })
}

fn lock_or_recover<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HwError;
    use crate::v4l2::fake::{FakeControl, FakeWriter};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex as StdMutex};
    use std::thread;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn fixture(root: &Path) {
        write(root, "class/video4linux/video0/dev", "81:0\n");
        write(root, "class/video4linux/video0/index", "0\n");
        write(root, "class/video4linux/video0/name", "Camera\n");
    }

    fn fixture_two(root: &Path) {
        fixture(root);
        write(root, "class/video4linux/video1/dev", "81:1\n");
        write(root, "class/video4linux/video1/index", "0\n");
        write(root, "class/video4linux/video1/name", "Camera\n");
    }

    fn identity_fixture(root: &Path) {
        let interface = root.join("devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0");
        fs::create_dir_all(&interface).unwrap();
        fs::write(interface.join("uevent"), "DRIVER=uvcvideo\n").unwrap();
        let link = root.join("class/video4linux/video0/device");
        if !link.exists() {
            std::os::unix::fs::symlink(&interface, &link).unwrap();
        }
    }

    fn caps() -> DeviceCapabilities {
        DeviceCapabilities {
            driver: "uvcvideo".to_owned(),
            card: "Camera".to_owned(),
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
        .value(10)
    }

    fn writer_controller(
        root: &Path,
        writer: FakeWriter,
    ) -> (UvcWriterController, Arc<AtomicUsize>) {
        let writes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&writes);
        let writer = Arc::new(writer);
        let controller = UvcWriterController::with_factory(
            SysfsRoot::new(root),
            Arc::new(move |_device| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(FakeWriter::clone_state(&writer)))
            }),
        );
        (controller, writes)
    }

    #[test]
    fn parses_only_canonical_video_paths() {
        assert_eq!(
            parse_device_path("/dev/video0").unwrap(),
            ("video0".to_owned(), "/dev/video0".to_owned())
        );
        for path in [
            "video0",
            "/dev/video",
            "/dev/videoX",
            "/dev/video0/",
            "/tmp/video0",
            "/dev/video01",
            "/dev/../video0",
        ] {
            assert!(parse_device_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn writes_allowlisted_integer_and_reports_previous() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let (controller, writes) = writer_controller(
            dir.path(),
            FakeWriter::new(caps()).with_control(brightness()),
        );
        let report = controller
            .set_control("/dev/video0", "brightness", 42)
            .unwrap();
        assert!(report.changed);
        assert_eq!(report.previous, 10);
        assert_eq!(report.value, 42);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn same_value_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps()).with_control(brightness());
        let (controller, _) = writer_controller(dir.path(), fake);
        let report = controller
            .set_control("/dev/video0", "brightness", 10)
            .unwrap();
        assert!(!report.changed);
    }

    #[test]
    fn rejects_out_of_range_and_non_step_values() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps()).with_control(
            FakeControl::new(
                control::CID_BRIGHTNESS,
                "Brightness",
                abi::V4L2_CTRL_TYPE_INTEGER,
            )
            .range(0, 100, 5)
            .value(10),
        );
        let (controller, _) = writer_controller(dir.path(), fake);
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 101),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 12),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn boolean_and_menu_values_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps())
            .with_control(
                FakeControl::new(0x0098_0911, "Exposure, Auto", abi::V4L2_CTRL_TYPE_BOOLEAN)
                    .range(0, 1, 1)
                    .value(0),
            )
            .with_control(
                FakeControl::new(
                    control::CID_POWER_LINE_FREQUENCY,
                    "Power Line Frequency",
                    abi::V4L2_CTRL_TYPE_MENU,
                )
                .range(0, 2, 1)
                .value(1)
                .menu(&[(0, "Disabled"), (1, "50 Hz")]),
            );
        let (controller, _) = writer_controller(dir.path(), fake);
        assert!(matches!(
            controller.set_control("/dev/video0", "exposure_auto", 2),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(
            controller
                .set_control("/dev/video0", "power_line_frequency", 1)
                .is_ok()
        );
        assert!(matches!(
            controller.set_control("/dev/video0", "power_line_frequency", 2),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn integer_menu_matches_values_not_indexes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps()).with_control(
            FakeControl::new(
                control::CID_WHITE_BALANCE_TEMPERATURE,
                "White Balance Temperature",
                abi::V4L2_CTRL_TYPE_INTEGER_MENU,
            )
            .range(0, 1, 1)
            .value(10)
            .integer_menu(&[(0, 10), (1, 20)]),
        );
        let (controller, _) = writer_controller(dir.path(), fake);
        assert!(
            controller
                .set_control("/dev/video0", "white_balance_temperature", 20)
                .is_ok()
        );
        // 1 est un index valide mais pas une valeur du menu.
        assert!(matches!(
            controller.set_control("/dev/video0", "white_balance_temperature", 1),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            controller.set_control("/dev/video0", "white_balance_temperature", 30),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn disallowed_or_unknown_controls_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps()).with_control(
            FakeControl::new(0x0800_0001, "Private", abi::V4L2_CTRL_TYPE_INTEGER).range(0, 10, 1),
        );
        let (controller, _) = writer_controller(dir.path(), fake);
        assert!(matches!(
            controller.set_control("/dev/video0", "private", 1),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            controller.set_control("/dev/video0", "unknown", 1),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn identity_mismatch_is_stalled_before_write() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        identity_fixture(dir.path());
        let mut capabilities = caps();
        capabilities.bus_info = "usb-0000:00:14.0-2".to_owned();
        let (controller, _) = writer_controller(
            dir.path(),
            FakeWriter::new(capabilities).with_control(brightness()),
        );
        let result = controller.set_control("/dev/video0", "brightness", 20);
        assert!(matches!(result, Err(HwError::Stalled { .. })), "{result:?}");
    }

    #[test]
    fn matching_identity_allows_write() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        identity_fixture(dir.path());
        let (controller, _) = writer_controller(
            dir.path(),
            FakeWriter::new(caps()).with_control(brightness()),
        );
        let report = controller
            .set_control("/dev/video0", "brightness", 20)
            .expect("identité cohérente");
        assert!(report.changed);
    }

    #[test]
    fn absent_device_is_unavailable_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (controller, writes) = writer_controller(dir.path(), FakeWriter::new(caps()));
        let result = controller.set_control("/dev/video0", "brightness", 10);
        assert!(matches!(result, Err(HwError::NotFound { .. })));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn busy_and_eio_are_typed_and_eio_stalls_device() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let busy = FakeWriter::new(caps())
            .with_control(brightness())
            .fail_capabilities(std::io::ErrorKind::WouldBlock);
        let (controller, _) = writer_controller(dir.path(), busy);
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 10),
            Err(HwError::Busy { .. })
        ));

        let eio = FakeWriter::new(caps())
            .with_control(brightness())
            .fail_set_errno(libc::EIO);
        let (controller, _) = writer_controller(dir.path(), eio);
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 20),
            Err(HwError::Stalled { .. })
        ));
        // Quarantaine : le deuxième appel est refusé avant toute ouverture.
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 21),
            Err(HwError::Stalled { .. })
        ));
    }

    #[test]
    fn budget_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut writer = FakeWriter::new(caps());
        for index in 0..100u32 {
            writer = writer.with_control(
                FakeControl::new(0x0900_0000 + index, "Private", abi::V4L2_CTRL_TYPE_INTEGER)
                    .range(0, 10, 1),
            );
        }
        let (controller, _) = writer_controller(dir.path(), writer);
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 1),
            Err(HwError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn readback_mismatch_attempts_restore_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps())
            .with_control(brightness().value(10))
            .clamp_writes_to(40);
        let writes = fake.writes_handle();
        let fake = Arc::new(fake);
        let controller = UvcWriterController::with_factory(
            SysfsRoot::new(dir.path()),
            Arc::new(move |_| Ok(Box::new(FakeWriter::clone_state(&fake)))),
        );
        let result = controller.set_control("/dev/video0", "brightness", 42);
        match result {
            Err(HwError::InvalidValue { value, .. }) => {
                assert!(value.contains("après=40"), "{value}");
                assert!(value.contains("précédent=10"), "{value}");
                assert!(value.contains("restauration précédente OK"), "{value}");
            }
            other => panic!("attendu InvalidValue, obtenu {other:?}"),
        }
        // Écriture demandée (42→40) puis restauration (10).
        assert_eq!(
            writes.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "la restauration doit être tentée"
        );
    }

    #[test]
    fn readback_mismatch_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let fake = FakeWriter::new(caps())
            .with_control(brightness())
            .ignore_writes();
        let (controller, _) = writer_controller(dir.path(), fake);
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 20),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[derive(Default)]
    struct Gate {
        state: StdMutex<(bool, bool)>,
        cv: Condvar,
    }

    impl Gate {
        fn wait_started(&self) {
            let mut state = self.state.lock().expect("gate");
            while !state.0 {
                state = self.cv.wait(state).expect("gate wait");
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().expect("gate");
            state.1 = true;
            self.cv.notify_all();
        }
    }

    #[test]
    fn stalled_map_evicts_oldest_not_everything() {
        let dir = tempfile::tempdir().unwrap();
        let controller = UvcWriterController::with_factory(
            SysfsRoot::new(dir.path()),
            Arc::new(|_| Ok(Box::new(FakeWriter::new(caps())))),
        );
        for index in 0..=MAX_TRACKED_DEVICES {
            controller.mark_stalled(&format!("video{index}"));
        }
        let stalled = controller.stalled.lock().expect("stalled");
        assert_eq!(stalled.len(), MAX_TRACKED_DEVICES);
        assert!(
            !stalled.contains_key("video0"),
            "la plus ancienne est évincée"
        );
        assert!(stalled.contains_key(&format!("video{MAX_TRACKED_DEVICES}")));
    }

    #[test]
    fn concurrent_write_on_same_device_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let gate = Arc::new(Gate::default());
        let factory_gate = Arc::clone(&gate);
        let controller = Arc::new(UvcWriterController::with_factory(
            SysfsRoot::new(dir.path()),
            Arc::new(move |_| {
                let mut state = factory_gate.state.lock().expect("gate");
                state.0 = true;
                factory_gate.cv.notify_all();
                while !state.1 {
                    state = factory_gate.cv.wait(state).expect("gate wait");
                }
                Ok(Box::new(FakeWriter::new(caps()).with_control(brightness())))
            }),
        ));

        let first_controller = Arc::clone(&controller);
        let first =
            thread::spawn(move || first_controller.set_control("/dev/video0", "brightness", 20));
        gate.wait_started();

        let second = controller.set_control("/dev/video0", "brightness", 30);
        assert!(matches!(second, Err(HwError::Busy { .. })));

        gate.release();
        let first = first.join().expect("thread writer");
        assert!(first.is_ok(), "{first:?}");
    }

    #[test]
    fn global_gate_serializes_different_devices() {
        let dir = tempfile::tempdir().unwrap();
        fixture_two(dir.path());
        let gate = Arc::new(Gate::default());
        let factory_gate = Arc::clone(&gate);
        let controller = Arc::new(UvcWriterController::with_factory(
            SysfsRoot::new(dir.path()),
            Arc::new(move |_| {
                let mut state = factory_gate.state.lock().expect("gate");
                state.0 = true;
                factory_gate.cv.notify_all();
                while !state.1 {
                    state = factory_gate.cv.wait(state).expect("gate wait");
                }
                Ok(Box::new(FakeWriter::new(caps()).with_control(brightness())))
            }),
        ));

        let first_controller = Arc::clone(&controller);
        let first =
            thread::spawn(move || first_controller.set_control("/dev/video0", "brightness", 20));
        gate.wait_started();

        // Device différent, même processus : la porte globale refuse sans
        // attendre ni ouvrir un second device.
        let second = controller.set_control("/dev/video1", "brightness", 30);
        assert!(matches!(second, Err(HwError::Busy { .. })));

        gate.release();
        let first = first.join().expect("thread writer");
        assert!(first.is_ok(), "{first:?}");

        // Après relâchement, une nouvelle écriture passe.
        let third = controller.set_control("/dev/video1", "brightness", 30);
        assert!(third.is_ok(), "{third:?}");
    }

    #[test]
    fn writer_factory_busy_is_propagated() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let controller = UvcWriterController::with_factory(
            SysfsRoot::new(dir.path()),
            Arc::new(|_| {
                Err(HwError::Busy {
                    device: "video0".to_owned(),
                })
            }),
        );
        assert!(matches!(
            controller.set_control("/dev/video0", "brightness", 1),
            Err(HwError::Busy { .. })
        ));
    }
}
