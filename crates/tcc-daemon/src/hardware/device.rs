// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker d'identification de l'appareil (DMI).
//!
//! Reprend la partie informative de `TuxedoControlCenterDaemon.identifyDevice` :
//! la GUI a besoin du modèle, du fabricant et du SKU pour l'en-tête du
//! Dashboard et la page Info (`docs/qa/FEATURE_AUDIT.md`, D1/F6).
//!
//! Aucune donnée sensible n'est lue : uniquement les attributs DMI publics de
//! `/sys/class/dmi/id`. Tous les champs sont optionnels (VM, conteneur, ARM) et
//! une machine non-TUXEDO n'est jamais une erreur.
//!
//! La détection Uniwill (`R_UW_MODEL_ID`) n'est pas refaite ici : elle exige
//! `/dev/tuxedo_io` et reste utilisée par les workers ODM/PRIME. Le modèle
//! exposé est celui du SKU DMI, avec `isTuxedo` calculé sur les vendors DMI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::device::TuxedoDevice;
use tcc_hw::dmi::DmiInfo;
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture. Les valeurs DMI ne changent pas à chaud ; une lecture
/// par minute suffit et reste négligeable.
pub const DEVICE_TICK: Duration = Duration::from_secs(60);
/// Taille maximale d'un champ exposé.
const MAX_FIELD_CHARS: usize = 128;
/// Fichier noyau exposant la version du kernel.
const KERNEL_RELEASE_PATH: &str = "/proc/sys/kernel/osrelease";
/// Attribut du module `tuxedo_io` exposant sa version (si compilé avec).
const TUXEDO_DRIVER_VERSION_PATH: &str = "module/tuxedo_io/version";

/// Identification de l'appareil exposée à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeviceData {
    /// Au moins un champ DMI lisible.
    pub available: bool,
    /// `sys_vendor` (`TUXEDO Computers GmbH`, `LENOVO`, ...).
    pub vendor: Option<String>,
    /// `product_name`.
    pub product_name: Option<String>,
    /// `board_name`.
    pub board_name: Option<String>,
    /// `board_vendor`.
    pub board_vendor: Option<String>,
    /// `chassis_vendor`.
    pub chassis_vendor: Option<String>,
    /// `product_sku` (sert aux quirks de modèle).
    pub sku: Option<String>,
    /// Vrai si un vendor DMI ou le SKU désigne un modèle TUXEDO.
    pub is_tuxedo: bool,
    /// Modèle TUXEDO stable (`IBM15A10`, `PULSE1501`, ...), `None` si inconnu.
    pub model: Option<String>,
    /// Version du noyau (`/proc/sys/kernel/osrelease`), bornée et nettoyée.
    pub kernel_release: Option<String>,
    /// Architecture de compilation (`std::env::consts::ARCH`, ex. `x86_64`).
    pub architecture: String,
    /// Version du module noyau `tuxedo_io` si `/sys/module/.../version` existe.
    pub tuxedo_driver_version: Option<String>,
    /// Erreurs de lecture (bornées).
    pub errors: Vec<String>,
}

impl DeviceData {
    fn from_dmi(
        dmi: &DmiInfo,
        kernel_release: Option<String>,
        tuxedo_driver_version: Option<String>,
    ) -> Self {
        let vendor = clean(dmi.sys_vendor.as_deref());
        let product_name = clean(dmi.product_name.as_deref());
        let board_name = clean(dmi.board_name.as_deref());
        let board_vendor = clean(dmi.board_vendor.as_deref());
        let chassis_vendor = clean(dmi.chassis_vendor.as_deref());
        let sku = clean(dmi.product_sku.as_deref());
        let device = TuxedoDevice::identify(dmi, None);
        let is_tuxedo = device.is_tuxedo() || dmi.is_tuxedo();
        let available = vendor.is_some()
            || product_name.is_some()
            || board_name.is_some()
            || board_vendor.is_some()
            || chassis_vendor.is_some()
            || sku.is_some();
        Self {
            available,
            vendor,
            product_name,
            board_name,
            board_vendor,
            chassis_vendor,
            sku,
            is_tuxedo,
            model: device.is_tuxedo().then(|| device.as_str().to_owned()),
            kernel_release,
            architecture: std::env::consts::ARCH.to_owned(),
            tuxedo_driver_version,
            errors: Vec::new(),
        }
    }
}

/// Nettoie un champ DMI : caractères de contrôle retirés, taille bornée.
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(MAX_FIELD_CHARS)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|value| !value.is_empty())
}

/// Runtime d'identification.
#[derive(Debug)]
pub struct DeviceRuntime {
    root: SysfsRoot,
    kernel_release_path: PathBuf,
}

impl DeviceRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self {
            root,
            kernel_release_path: PathBuf::from(KERNEL_RELEASE_PATH),
        }
    }

    /// Injecte un fichier de version noyau (tests).
    #[cfg(test)]
    #[must_use]
    pub fn with_kernel_release_path(root: SysfsRoot, path: impl Into<PathBuf>) -> Self {
        Self {
            root,
            kernel_release_path: path.into(),
        }
    }

    /// Lit les champs DMI, la version noyau et la version du module TUXEDO.
    pub fn tick(&mut self, _input: &HardwareInputs) -> DeviceData {
        let kernel_release = read_kernel_release(&self.kernel_release_path);
        let tuxedo_driver_version = self
            .root
            .read_optional_string(TUXEDO_DRIVER_VERSION_PATH)
            .ok()
            .flatten()
            .and_then(|value| clean(Some(&value)));

        match DmiInfo::read(&self.root) {
            Ok(dmi) => DeviceData::from_dmi(&dmi, kernel_release, tuxedo_driver_version),
            Err(error) => {
                tracing::debug!(%error, "DMI illisible");
                DeviceData {
                    kernel_release,
                    architecture: std::env::consts::ARCH.to_owned(),
                    tuxedo_driver_version,
                    errors: vec![format!("DMI: {error}")],
                    ..DeviceData::default()
                }
            }
        }
    }
}

/// Lit et nettoie la version du noyau.
fn read_kernel_release(path: &Path) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    clean(Some(&value))
}

/// Démarre le worker d'identification.
pub(crate) fn spawn(ctx: WorkerContext, root: SysfsRoot) -> tokio::task::JoinHandle<()> {
    let runtime = DeviceRuntime::new(root);
    spawn_worker(
        "device",
        DEVICE_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_device,
        DeviceRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, field: &str, contents: &str) {
        let path = root.join("class/dmi/id").join(field);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn write_relative(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn reads_dmi_and_identifies_tuxedo_model() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sys_vendor", "TUXEDO Computers GmbH\n");
        write(dir.path(), "product_name", "TUXEDO Book X\n");
        write(dir.path(), "product_sku", "IBM15A10\n");
        write(dir.path(), "board_name", "Board\n");
        write(dir.path(), "board_vendor", "TUXEDO Computers\n");
        write(dir.path(), "chassis_vendor", "TUXEDO\n");
        let mut runtime = DeviceRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.available);
        assert!(data.is_tuxedo);
        assert_eq!(data.model.as_deref(), Some("IBM15A10"));
        assert_eq!(data.vendor.as_deref(), Some("TUXEDO Computers GmbH"));
        assert_eq!(data.product_name.as_deref(), Some("TUXEDO Book X"));
        assert_eq!(data.board_name.as_deref(), Some("Board"));
        assert_eq!(data.board_vendor.as_deref(), Some("TUXEDO Computers"));
        assert_eq!(data.chassis_vendor.as_deref(), Some("TUXEDO"));
        assert_eq!(data.sku.as_deref(), Some("IBM15A10"));
        assert!(data.errors.is_empty());
    }

    #[test]
    fn reads_kernel_release_architecture_and_driver_version() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sys_vendor", "TUXEDO Computers GmbH\n");
        write(dir.path(), "product_sku", "IBM15A10\n");
        write_relative(dir.path(), "module/tuxedo_io/version", "0.3.9\n");
        let kernel = dir.path().join("osrelease");
        fs::write(&kernel, "6.12.0-tuxedo\n").unwrap();
        let mut runtime =
            DeviceRuntime::with_kernel_release_path(SysfsRoot::new(dir.path()), &kernel);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.kernel_release.as_deref(), Some("6.12.0-tuxedo"));
        assert_eq!(data.architecture, std::env::consts::ARCH);
        assert_eq!(data.tuxedo_driver_version.as_deref(), Some("0.3.9"));
    }

    #[test]
    fn missing_kernel_release_and_driver_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let mut runtime =
            DeviceRuntime::with_kernel_release_path(SysfsRoot::new(dir.path()), &missing);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.kernel_release, None);
        assert_eq!(data.tuxedo_driver_version, None);
        assert_eq!(data.architecture, std::env::consts::ARCH);
        assert!(!data.available);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn missing_dmi_degrades_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = DeviceRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.available);
        assert!(!data.is_tuxedo);
        assert_eq!(data.model, None);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn non_tuxedo_is_exposed_without_model() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sys_vendor", "LENOVO\n");
        write(dir.path(), "product_name", "ThinkPad\n");
        let mut runtime = DeviceRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.available);
        assert!(!data.is_tuxedo);
        assert_eq!(data.model, None);
    }

    #[test]
    fn control_characters_are_stripped_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "product_name", "Book\n0000\t");
        let mut runtime = DeviceRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.product_name.as_deref(), Some("Book0000"));

        write(dir.path(), "board_name", &"x".repeat(300));
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(
            data.board_name.as_deref().map(str::len),
            Some(MAX_FIELD_CHARS)
        );
    }

    #[test]
    fn serializes_camel_case() {
        let data = DeviceData {
            available: true,
            product_name: Some("Book".to_owned()),
            is_tuxedo: true,
            model: Some("IBM15A10".to_owned()),
            ..DeviceData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"productName\":\"Book\""));
        assert!(json.contains("\"isTuxedo\":true"));
        assert!(json.contains("\"model\":\"IBM15A10\""));
        assert!(json.contains("\"kernelRelease\":null"));
        assert!(json.contains("\"architecture\":\""));
        assert!(json.contains("\"tuxedoDriverVersion\":null"));
    }
}
