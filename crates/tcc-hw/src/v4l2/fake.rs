// SPDX-License-Identifier: GPL-3.0-or-later
//! Transport V4L2 factice pour les tests.
//!
//! Aucun `/dev`, aucun ioctl, aucun privilège : [`FakeReader`] renvoie des
//! capacités, contrôles et menus scriptés. Il permet de tester l'allowlist, les
//! bornes, les menus, `EINTR`, `EBUSY`/`ENODEV`, la troncature et le budget en
//! CI, sans caméra.
//!
//! Le type est aussi utilisable par les tests d'intégration du crate
//! (`tests/fake_v4l2.rs`) et par `tcc-daemon` via une fabrique injectée.

#![forbid(unsafe_code)]

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::abi;
use super::write::UvcWriter;
use super::{ControlQuery, DeviceCapabilities, MenuQuery, Result, V4l2Reader};
use crate::error::HwError;

/// Contrôle scripté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeControl {
    id: u32,
    name: String,
    kind_raw: u32,
    min: i32,
    max: i32,
    step: i32,
    default: i32,
    value: i32,
    flags: u32,
    menu: Vec<MenuQuery>,
}

impl FakeControl {
    /// Crée un contrôle entier par défaut (`0..=100`, pas 1, valeur 0).
    #[must_use]
    pub fn new(id: u32, name: &str, kind_raw: u32) -> Self {
        Self {
            id,
            name: name.to_owned(),
            kind_raw,
            min: 0,
            max: 100,
            step: 1,
            default: 0,
            value: 0,
            flags: 0,
            menu: Vec::new(),
        }
    }

    /// Définit les bornes.
    #[must_use]
    pub fn range(mut self, min: i32, max: i32, step: i32) -> Self {
        self.min = min;
        self.max = max;
        self.step = step;
        self
    }

    /// Définit la valeur par défaut.
    #[must_use]
    pub fn default(mut self, value: i32) -> Self {
        self.default = value;
        self
    }

    /// Définit la valeur courante.
    #[must_use]
    pub fn value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }

    /// Définit les drapeaux bruts.
    #[must_use]
    pub fn flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    /// Définit un menu à libellés.
    #[must_use]
    pub fn menu(mut self, entries: &[(i32, &str)]) -> Self {
        self.menu = entries
            .iter()
            .map(|(_index, label)| MenuQuery {
                label: Some((*label).to_owned()),
                value: None,
            })
            .collect();
        self
    }

    /// Définit un menu à valeurs entières (`INTEGER_MENU`).
    #[must_use]
    pub fn integer_menu(mut self, entries: &[(i32, i64)]) -> Self {
        self.menu = entries
            .iter()
            .map(|(_index, value)| MenuQuery {
                label: None,
                value: Some(*value),
            })
            .collect();
        self
    }

    fn query(&self) -> ControlQuery {
        ControlQuery {
            id: self.id,
            kind_raw: self.kind_raw,
            name: self.name.clone(),
            min: self.min,
            max: self.max,
            step: self.step,
            default: self.default,
            flags: self.flags,
        }
    }
}

/// Lecteur factice.
#[derive(Debug)]
pub struct FakeReader {
    capabilities: DeviceCapabilities,
    controls: Mutex<Vec<FakeControl>>,
    fail_capabilities: Option<io::ErrorKind>,
    fail_query: Option<io::ErrorKind>,
    fail_menu: Option<io::ErrorKind>,
    fail_get: Option<io::ErrorKind>,
    eintr_once: AtomicBool,
}

impl FakeReader {
    /// Crée un lecteur avec les capacités données.
    #[must_use]
    pub fn new(capabilities: DeviceCapabilities) -> Self {
        Self {
            capabilities,
            controls: Mutex::new(Vec::new()),
            fail_capabilities: None,
            fail_query: None,
            fail_menu: None,
            fail_get: None,
            eintr_once: AtomicBool::new(false),
        }
    }

    /// Ajoute un contrôle.
    #[must_use]
    pub fn with_control(self, control: FakeControl) -> Self {
        if let Ok(mut controls) = self.controls.lock() {
            controls.push(control);
        }
        self
    }

    /// Fait échouer le prochain `QUERYCTRL` avec `EINTR` (une seule fois).
    #[must_use]
    pub fn with_eintr_once(self) -> Self {
        self.eintr_once.store(true, Ordering::SeqCst);
        self
    }

    /// Fait échouer `QUERYCAP` avec le type d'erreur donné.
    #[must_use]
    pub fn fail_capabilities(mut self, kind: io::ErrorKind) -> Self {
        self.fail_capabilities = Some(kind);
        self
    }

    /// Fait échouer `QUERYCTRL` avec le type d'erreur donné.
    #[must_use]
    pub fn fail_query(mut self, kind: io::ErrorKind) -> Self {
        self.fail_query = Some(kind);
        self
    }

    /// Fait échouer `QUERYMENU` avec le type d'erreur donné.
    #[must_use]
    pub fn fail_menu(mut self, kind: io::ErrorKind) -> Self {
        self.fail_menu = Some(kind);
        self
    }

    /// Fait échouer `G_CTRL` avec le type d'erreur donné.
    #[must_use]
    pub fn fail_get(mut self, kind: io::ErrorKind) -> Self {
        self.fail_get = Some(kind);
        self
    }

    /// Copie l'état pour une nouvelle ouverture simulée.
    #[must_use]
    pub fn clone_state(other: &FakeReader) -> Self {
        let controls = match other.controls.lock() {
            Ok(controls) => controls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        Self {
            capabilities: other.capabilities.clone(),
            controls: Mutex::new(controls),
            fail_capabilities: other.fail_capabilities,
            fail_query: other.fail_query,
            fail_menu: other.fail_menu,
            fail_get: other.fail_get,
            eintr_once: AtomicBool::new(other.eintr_once.load(Ordering::SeqCst)),
        }
    }

    fn lock_controls(&self) -> std::sync::MutexGuard<'_, Vec<FakeControl>> {
        match self.controls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl V4l2Reader for FakeReader {
    fn capabilities(&self) -> Result<DeviceCapabilities> {
        match self.fail_capabilities {
            Some(io::ErrorKind::WouldBlock) => Err(HwError::Busy {
                device: "fake".to_owned(),
            }),
            Some(kind) => Err(HwError::Unavailable {
                device: "fake",
                source: io::Error::from(kind),
            }),
            None => Ok(self.capabilities.clone()),
        }
    }

    fn query_control(&self, id: u32) -> Result<ControlQuery> {
        if self.eintr_once.swap(false, Ordering::SeqCst) {
            return Err(fake_ioctl_error(io::ErrorKind::Interrupted));
        }
        if let Some(kind) = self.fail_query {
            return Err(fake_ioctl_error(kind));
        }
        let bare = abi::mask_query_flags(id);
        let controls = self.lock_controls();
        let found = if id & abi::V4L2_CTRL_FLAG_NEXT_CTRL != 0 {
            controls
                .iter()
                .filter(|control| control.id > bare)
                .min_by_key(|control| control.id)
        } else {
            controls.iter().find(|control| control.id == bare)
        };
        found
            .map(FakeControl::query)
            .ok_or_else(|| fake_ioctl_error(io::ErrorKind::InvalidInput))
    }

    fn query_menu(&self, id: u32, index: i32) -> Result<MenuQuery> {
        if let Some(kind) = self.fail_menu {
            return Err(fake_ioctl_error(kind));
        }
        let controls = self.lock_controls();
        let Some(control) = controls.iter().find(|control| control.id == id) else {
            return Err(fake_ioctl_error(io::ErrorKind::InvalidInput));
        };
        let position = i64::from(index) - i64::from(control.min);
        let Some(entry) = usize::try_from(position)
            .ok()
            .and_then(|position| control.menu.get(position))
        else {
            return Err(fake_ioctl_error(io::ErrorKind::InvalidInput));
        };
        Ok(entry.clone())
    }

    fn get_control(&self, id: u32) -> Result<i32> {
        if let Some(kind) = self.fail_get {
            return Err(fake_ioctl_error(kind));
        }
        let controls = self.lock_controls();
        controls
            .iter()
            .find(|control| control.id == id)
            .map(|control| control.value)
            .ok_or_else(|| fake_ioctl_error(io::ErrorKind::InvalidInput))
    }
}

/// Écrivain factice, pour les tests de l'étape 2.
#[derive(Debug)]
pub struct FakeWriter {
    capabilities: DeviceCapabilities,
    controls: Mutex<Vec<FakeControl>>,
    fail_capabilities: Option<io::ErrorKind>,
    fail_query: Option<io::ErrorKind>,
    fail_menu: Option<io::ErrorKind>,
    fail_get: Option<io::ErrorKind>,
    fail_set: Option<io::ErrorKind>,
    fail_set_errno: Option<i32>,
    ignore_writes: bool,
    clamp_writes_to: Option<i32>,
    writes: Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeWriter {
    /// Crée un écrivain avec les capacités données.
    #[must_use]
    pub fn new(capabilities: DeviceCapabilities) -> Self {
        Self {
            capabilities,
            controls: Mutex::new(Vec::new()),
            fail_capabilities: None,
            fail_query: None,
            fail_menu: None,
            fail_get: None,
            fail_set: None,
            fail_set_errno: None,
            ignore_writes: false,
            clamp_writes_to: None,
            writes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Ajoute un contrôle.
    #[must_use]
    pub fn with_control(self, control: FakeControl) -> Self {
        if let Ok(mut controls) = self.controls.lock() {
            controls.push(control);
        }
        self
    }

    /// Simule un noyau qui accepte `S_CTRL` mais ne modifie pas la valeur.
    #[must_use]
    pub fn ignore_writes(mut self) -> Self {
        self.ignore_writes = true;
        self
    }

    /// Simule un firmware qui borne toute écriture à `max`.
    #[must_use]
    pub fn clamp_writes_to(mut self, max: i32) -> Self {
        self.clamp_writes_to = Some(max);
        self
    }

    /// Compteur d'écritures partagé (observable même après `clone_state`).
    #[must_use]
    pub fn writes_handle(&self) -> Arc<std::sync::atomic::AtomicUsize> {
        Arc::clone(&self.writes)
    }

    /// Fait échouer `QUERYCAP`.
    #[must_use]
    pub fn fail_capabilities(mut self, kind: io::ErrorKind) -> Self {
        self.fail_capabilities = Some(kind);
        self
    }

    /// Fait échouer `S_CTRL` avec un errno précis.
    #[must_use]
    pub fn fail_set_errno(mut self, errno: i32) -> Self {
        self.fail_set_errno = Some(errno);
        self
    }

    /// Fait échouer `S_CTRL`.
    #[must_use]
    pub fn fail_set(mut self, kind: io::ErrorKind) -> Self {
        self.fail_set = Some(kind);
        self
    }

    /// Nombre d'écritures réellement effectuées.
    #[must_use]
    pub fn writes(&self) -> usize {
        self.writes.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Copie l'état pour une nouvelle ouverture simulée.
    #[must_use]
    pub fn clone_state(other: &FakeWriter) -> Self {
        let controls = match other.controls.lock() {
            Ok(controls) => controls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        Self {
            capabilities: other.capabilities.clone(),
            controls: Mutex::new(controls),
            fail_capabilities: other.fail_capabilities,
            fail_query: other.fail_query,
            fail_menu: other.fail_menu,
            fail_get: other.fail_get,
            fail_set: other.fail_set,
            fail_set_errno: other.fail_set_errno,
            ignore_writes: other.ignore_writes,
            clamp_writes_to: other.clamp_writes_to,
            writes: Arc::clone(&other.writes),
        }
    }

    fn lock_controls(&self) -> std::sync::MutexGuard<'_, Vec<FakeControl>> {
        match self.controls.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl UvcWriter for FakeWriter {
    fn capabilities(&self) -> Result<DeviceCapabilities> {
        match self.fail_capabilities {
            Some(io::ErrorKind::WouldBlock) => Err(HwError::Busy {
                device: "fake".to_owned(),
            }),
            Some(kind) => Err(HwError::Unavailable {
                device: "fake",
                source: io::Error::from(kind),
            }),
            None => Ok(self.capabilities.clone()),
        }
    }

    fn query_control(&self, id: u32) -> Result<ControlQuery> {
        if let Some(kind) = self.fail_query {
            return Err(fake_ioctl_error(kind));
        }
        let bare = abi::mask_query_flags(id);
        let controls = self.lock_controls();
        let found = if id & abi::V4L2_CTRL_FLAG_NEXT_CTRL != 0 {
            controls
                .iter()
                .filter(|control| control.id > bare)
                .min_by_key(|control| control.id)
        } else {
            controls.iter().find(|control| control.id == bare)
        };
        found
            .map(FakeControl::query)
            .ok_or_else(|| fake_ioctl_error(io::ErrorKind::InvalidInput))
    }

    fn query_menu(&self, id: u32, index: i32) -> Result<MenuQuery> {
        if let Some(kind) = self.fail_menu {
            return Err(fake_ioctl_error(kind));
        }
        let controls = self.lock_controls();
        let Some(control) = controls.iter().find(|control| control.id == id) else {
            return Err(fake_ioctl_error(io::ErrorKind::InvalidInput));
        };
        let position = i64::from(index) - i64::from(control.min);
        let Some(entry) = usize::try_from(position)
            .ok()
            .and_then(|position| control.menu.get(position))
        else {
            return Err(fake_ioctl_error(io::ErrorKind::InvalidInput));
        };
        Ok(entry.clone())
    }

    fn get_control(&self, id: u32) -> Result<i32> {
        if let Some(kind) = self.fail_get {
            return Err(fake_ioctl_error(kind));
        }
        let controls = self.lock_controls();
        controls
            .iter()
            .find(|control| control.id == id)
            .map(|control| control.value)
            .ok_or_else(|| fake_ioctl_error(io::ErrorKind::InvalidInput))
    }

    fn set_control(&self, id: u32, value: i32) -> Result<()> {
        if let Some(errno) = self.fail_set_errno {
            return Err(fake_ioctl_error_errno(errno));
        }
        if let Some(kind) = self.fail_set {
            return Err(fake_ioctl_error(kind));
        }
        if self.ignore_writes {
            return Ok(());
        }
        let mut controls = self.lock_controls();
        let Some(control) = controls.iter_mut().find(|control| control.id == id) else {
            return Err(fake_ioctl_error(io::ErrorKind::InvalidInput));
        };
        control.value = self.clamp_writes_to.map_or(value, |max| value.min(max));
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn fake_ioctl_error(kind: io::ErrorKind) -> HwError {
    HwError::Ioctl {
        name: "fake V4L2",
        request: 0,
        source: io::Error::from(kind),
    }
}

fn fake_ioctl_error_errno(errno: i32) -> HwError {
    HwError::Ioctl {
        name: "fake V4L2",
        request: 0,
        source: io::Error::from_raw_os_error(errno),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> DeviceCapabilities {
        DeviceCapabilities::default()
    }

    #[test]
    fn enumerates_in_increasing_id_order() {
        let reader = FakeReader::new(caps())
            .with_control(FakeControl::new(0x0098_0901, "Contrast", 1))
            .with_control(FakeControl::new(0x0098_0900, "Brightness", 1));
        let first = reader.query_control(abi::V4L2_CTRL_FLAG_NEXT_CTRL).unwrap();
        assert_eq!(first.id, 0x0098_0900);
        let second = reader
            .query_control(first.id | abi::V4L2_CTRL_FLAG_NEXT_CTRL)
            .unwrap();
        assert_eq!(second.id, 0x0098_0901);
        assert!(matches!(
            reader.query_control(second.id | abi::V4L2_CTRL_FLAG_NEXT_CTRL),
            Err(HwError::Ioctl { .. })
        ));
    }

    #[test]
    fn menu_entries_are_indexed() {
        let reader = FakeReader::new(caps()).with_control(
            FakeControl::new(0x0098_0918, "Power Line Frequency", 3)
                .range(0, 2, 1)
                .menu(&[(0, "Disabled"), (1, "50 Hz")]),
        );
        let entry = reader.query_menu(0x0098_0918, 1).unwrap();
        assert_eq!(entry.label.as_deref(), Some("50 Hz"));
        assert!(reader.query_menu(0x0098_0918, 2).is_err());
    }

    #[test]
    fn clone_state_is_independent() {
        let reader = FakeReader::new(caps()).with_control(FakeControl::new(1, "A", 1));
        let clone = FakeReader::clone_state(&reader);
        assert_eq!(clone.get_control(1).unwrap(), 0);
    }
}
