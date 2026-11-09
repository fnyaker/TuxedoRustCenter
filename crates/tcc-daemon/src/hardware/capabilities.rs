// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Capabilities simples pour le Dashboard et la page Info.
//!
//! Ce worker ne lit **aucun capteur** et n'ouvre aucun périphérique
//! privilégié. Il expose uniquement :
//!
//! - `hasBattery` : au moins une batterie présente (`tcc-hw::power_supply`) ;
//! - `hasAc` : au moins un adaptateur secteur exposé ;
//! - `hasIgpu` / `hasDedicatedGpu` : rôles détectés dans `/sys/class/drm`
//!   (`tcc-hw::sensors::gpu_presence`), sans valeur de capteur ni
//!   `nvidia-smi`.
//!
//! La présence de l'écran interne est publiée dans
//! [`super::display::DisplayData::has_internal_display`].

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::power_supply::PowerSupplies;
use tcc_hw::sensors::gpu_presence;
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture (branchement secteur, hotplug GPU).
pub const CAPABILITIES_TICK: Duration = Duration::from_secs(30);

/// Capabilities matérielles simples.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CapabilitiesData {
    /// Au moins une batterie est présente.
    pub has_battery: bool,
    /// Au moins un adaptateur secteur est exposé.
    pub has_ac: bool,
    /// Au moins une GPU intégrée est détectée.
    pub has_igpu: bool,
    /// Au moins une GPU discrète est détectée.
    pub has_dedicated_gpu: bool,
    /// Erreurs de lecture (bornées).
    pub errors: Vec<String>,
}

/// Runtime des capabilities.
#[derive(Debug)]
pub struct CapabilitiesRuntime {
    root: SysfsRoot,
    last_error_logged: bool,
}

impl CapabilitiesRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self {
            root,
            last_error_logged: false,
        }
    }

    /// Lit les alimentations et la présence des GPU.
    pub fn tick(&mut self, _input: &HardwareInputs) -> CapabilitiesData {
        let mut errors = Vec::new();
        let (has_battery, has_ac) = match PowerSupplies::read(&self.root) {
            Ok(supplies) => (
                supplies.batteries.iter().any(|battery| battery.present),
                !supplies.adapters.is_empty(),
            ),
            Err(error) => {
                if !self.last_error_logged {
                    self.last_error_logged = true;
                    tracing::debug!(%error, "alimentations illisibles");
                }
                errors.push(format!("power_supply: {error}"));
                (false, false)
            }
        };
        self.last_error_logged = false;

        let presence = gpu_presence(&self.root);
        CapabilitiesData {
            has_battery,
            has_ac,
            has_igpu: presence.integrated,
            has_dedicated_gpu: presence.discrete,
            errors,
        }
    }
}

/// Démarre le worker des capabilities.
pub(crate) fn spawn(ctx: WorkerContext, root: SysfsRoot) -> tokio::task::JoinHandle<()> {
    let runtime = CapabilitiesRuntime::new(root);
    spawn_worker(
        "capabilities",
        CAPABILITIES_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_capabilities,
        CapabilitiesRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn detects_battery_and_ac() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(dir.path(), "class/power_supply/BAT0/present", "1\n");
        write(dir.path(), "class/power_supply/AC/type", "Mains\n");
        write(dir.path(), "class/power_supply/AC/online", "1\n");
        let mut runtime = CapabilitiesRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.has_battery);
        assert!(data.has_ac);
        assert!(!data.has_igpu);
        assert!(!data.has_dedicated_gpu);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn absent_power_supplies_are_false() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = CapabilitiesRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.has_battery);
        assert!(!data.has_ac);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn removed_battery_is_not_a_capability() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(dir.path(), "class/power_supply/BAT0/present", "0\n");
        let mut runtime = CapabilitiesRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.has_battery);
    }

    #[test]
    fn detects_gpu_roles_without_sensor_collection() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/drm/card0/device/uevent",
            "DRIVER=i915\nPCI_ID=8086:7D55\n",
        );
        write(
            dir.path(),
            "class/drm/card1/device/uevent",
            "DRIVER=nvidia\nPCI_ID=10DE:28E0\n",
        );
        let mut runtime = CapabilitiesRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.has_igpu);
        assert!(data.has_dedicated_gpu);
    }

    #[test]
    fn serializes_camel_case() {
        let data = CapabilitiesData {
            has_battery: true,
            has_ac: true,
            has_igpu: true,
            has_dedicated_gpu: true,
            ..CapabilitiesData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"hasBattery\":true"));
        assert!(json.contains("\"hasAc\":true"));
        assert!(json.contains("\"hasIgpu\":true"));
        assert!(json.contains("\"hasDedicatedGpu\":true"));
    }
}
