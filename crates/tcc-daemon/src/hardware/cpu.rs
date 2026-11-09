// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Capacités CPU en lecture seule.
//!
//! La GUI a besoin de la topologie réelle et des listes de governors/EPP pour
//! l'éditeur de profil (`docs/qa/FEATURE_AUDIT.md`, D4/E8). L'application des
//! réglages CPU reste dans `apply.rs` (`linux-platform`) ; ce worker ne fait
//! que lire `tcc-hw::cpu` et publier des capacités.
//!
//! `writable` signifie « TCC peut appliquer un profil CPU » : au moins une
//! politique cpufreq active **et** le daemon n'est pas en `dry_run`. Ce n'est
//! pas une mesure des permissions POSIX : le daemon tourne en root et la
//! validation réelle est faite au moment de l'écriture par `tcc-hw`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::cpu::{CpuController, CpuPolicy, ScalingDriver};
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture (les capacités changent rarement).
pub const CPU_TICK: Duration = Duration::from_secs(15);

/// Capacités CPU exposées à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CpuCapabilitiesData {
    /// Nombre de CPU présents (`possible`/`present`).
    pub total_cores: usize,
    /// Nombre de CPU en ligne.
    pub online_cores: usize,
    /// Governors acceptés par au moins une politique cpufreq.
    pub available_governors: Vec<String>,
    /// Préférences EPP acceptées, si exposées par le pilote.
    pub available_epp: Vec<String>,
    /// Au moins une politique cpufreq active.
    pub has_cpu_control: bool,
    /// TCC peut appliquer un profil CPU (contrôle présent et hors `dry_run`).
    pub writable: bool,
    /// Pilote cpufreq détecté (`intel_pstate`, `amd-pstate-epp`, ...).
    pub scaling_driver: Option<String>,
    /// Erreurs de lecture (bornées).
    pub errors: Vec<String>,
}

impl CpuCapabilitiesData {
    fn from_controller(controller: &CpuController, dry_run: bool) -> Self {
        let topology = controller.topology();
        let policies = controller.policies();
        let total_cores = if topology.present.is_empty() {
            policies.len()
        } else {
            topology.present.len()
        };
        let online_cores = if topology.online.is_empty() {
            controller.online_cores().len()
        } else {
            topology.online.len()
        };
        let has_cpu_control = policies.iter().any(CpuPolicy::is_active);
        let scaling_driver = match controller.scaling_driver() {
            ScalingDriver::Unknown(_) => None,
            driver => Some(driver.to_string()),
        };
        Self {
            total_cores,
            online_cores,
            available_governors: union(policies.iter().map(|policy| &policy.available_governors)),
            available_epp: union(policies.iter().map(|policy| &policy.available_epps)),
            has_cpu_control,
            writable: has_cpu_control && !dry_run,
            scaling_driver,
            errors: Vec::new(),
        }
    }
}

/// Union ordonnée et dédupliquée de listes.
fn union<'a>(lists: impl IntoIterator<Item = &'a Vec<String>>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for list in lists {
        for value in list {
            if !result.contains(value) {
                result.push(value.clone());
            }
        }
    }
    result
}

/// Runtime des capacités CPU.
#[derive(Debug)]
pub struct CpuRuntime {
    root: SysfsRoot,
    dry_run: bool,
    last_error_logged: bool,
}

impl CpuRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            root,
            dry_run,
            last_error_logged: false,
        }
    }

    /// Lit la topologie et les politiques cpufreq.
    pub fn tick(&mut self, _input: &HardwareInputs) -> CpuCapabilitiesData {
        match CpuController::new(self.root.clone()) {
            Ok(controller) => {
                self.last_error_logged = false;
                CpuCapabilitiesData::from_controller(&controller, self.dry_run)
            }
            Err(error) => {
                if !self.last_error_logged {
                    self.last_error_logged = true;
                    tracing::debug!(%error, "capacités CPU illisibles");
                }
                CpuCapabilitiesData {
                    errors: vec![format!("CPU: {error}")],
                    ..CpuCapabilitiesData::default()
                }
            }
        }
    }
}

/// Démarre le worker des capacités CPU.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = CpuRuntime::new(root, dry_run);
    spawn_worker(
        "cpu",
        CPU_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_cpu,
        CpuRuntime::tick,
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

    fn cpu_policy(root: &Path, cpu: u32) {
        let base = format!("devices/system/cpu/cpu{cpu}/cpufreq");
        write(root, &format!("{base}/scaling_driver"), "intel_pstate\n");
        write(root, &format!("{base}/scaling_governor"), "powersave\n");
        write(
            root,
            &format!("{base}/scaling_available_governors"),
            "performance powersave\n",
        );
        write(
            root,
            &format!("{base}/energy_performance_preference"),
            "balance_power\n",
        );
        write(
            root,
            &format!("{base}/energy_performance_available_preferences"),
            "default performance balance_performance balance_power power\n",
        );
        write(root, &format!("{base}/cpuinfo_min_freq"), "400000\n");
        write(root, &format!("{base}/cpuinfo_max_freq"), "5000000\n");
        write(root, &format!("{base}/scaling_min_freq"), "400000\n");
        write(root, &format!("{base}/scaling_max_freq"), "5000000\n");
        write(root, &format!("{base}/scaling_cur_freq"), "2000000\n");
    }

    #[test]
    fn reads_topology_governors_and_epp() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "devices/system/cpu/possible", "0-3\n");
        write(dir.path(), "devices/system/cpu/present", "0-3\n");
        write(dir.path(), "devices/system/cpu/online", "0-1\n");
        cpu_policy(dir.path(), 0);
        cpu_policy(dir.path(), 1);
        for cpu in 2..=3 {
            write(
                dir.path(),
                &format!("devices/system/cpu/cpu{cpu}/online"),
                "0\n",
            );
        }
        let mut runtime = CpuRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.total_cores, 4);
        assert_eq!(data.online_cores, 2);
        assert_eq!(
            data.available_governors,
            vec!["performance".to_owned(), "powersave".to_owned()]
        );
        assert_eq!(
            data.available_epp,
            vec![
                "default".to_owned(),
                "performance".to_owned(),
                "balance_performance".to_owned(),
                "balance_power".to_owned(),
                "power".to_owned()
            ]
        );
        assert!(data.has_cpu_control);
        assert!(data.writable);
        assert_eq!(data.scaling_driver.as_deref(), Some("intel_pstate"));
        assert!(data.errors.is_empty());
    }

    #[test]
    fn dry_run_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "devices/system/cpu/possible", "0\n");
        write(dir.path(), "devices/system/cpu/present", "0\n");
        write(dir.path(), "devices/system/cpu/online", "0\n");
        cpu_policy(dir.path(), 0);
        let mut runtime = CpuRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.has_cpu_control);
        assert!(!data.writable);
    }

    #[test]
    fn no_cpufreq_means_no_control() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "devices/system/cpu/possible", "0\n");
        write(dir.path(), "devices/system/cpu/present", "0\n");
        write(dir.path(), "devices/system/cpu/online", "0\n");
        let mut runtime = CpuRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.total_cores, 1);
        assert!(!data.has_cpu_control);
        assert!(!data.writable);
        assert!(data.available_governors.is_empty());
    }

    #[test]
    fn missing_sysfs_degrades_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = CpuRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.total_cores, 0);
        assert!(!data.writable);
        assert_eq!(data.errors.len(), 1);
        // Le deuxième tick ne duplique pas l'erreur (log unique).
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.errors.len(), 1);
    }

    #[test]
    fn serializes_camel_case() {
        let data = CpuCapabilitiesData {
            total_cores: 8,
            online_cores: 4,
            available_governors: vec!["powersave".to_owned()],
            available_epp: vec!["power".to_owned()],
            has_cpu_control: true,
            writable: true,
            ..CpuCapabilitiesData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"totalCores\":8"));
        assert!(json.contains("\"onlineCores\":4"));
        assert!(json.contains("\"availableGovernors\":[\"powersave\"]"));
        assert!(json.contains("\"availableEpp\":[\"power\"]"));
        assert!(json.contains("\"hasCpuControl\":true"));
        assert!(json.contains("\"writable\":true"));
    }
}
