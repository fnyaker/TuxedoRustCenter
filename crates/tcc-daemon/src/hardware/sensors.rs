// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker de capteurs, à la demande.
//!
//! Reprend `GpuInfoWorker` / `CpuPowerWorker` de l'original : la collecte n'est
//! active que si un client l'a demandée (`SetSensorDataCollectionStatus`).
//! `linux-platform` fournit un `AtomicBool` partagé, mis à jour par le setter
//! D-Bus. Tant qu'il est faux, le worker ne lit **aucun** fichier : il publie
//! seulement `enabled: false` et les valeurs par défaut.
//!
//! Le timer de 2 s reste actif mais son coût est nul hors collecte (une lecture
//! atomique). C'est le compromis retenu pour que l'activation soit immédiate
//! sans notification supplémentaire.
//!
//! Les valeurs manquantes sont `None`, jamais `0` inventé. Le premier
//! échantillon RAPL ne fournit pas de puissance (pas de delta) : l'UI affiche
//! « indisponible » pendant un tick, comme l'original.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::sensors::{GpuRole, GpuSensors, SensorCollector, SensorSnapshot};
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de collecte (identique à `GpuInfoWorker`).
pub const SENSOR_TICK: Duration = Duration::from_secs(2);
/// Nombre de ticks sans GPU avant une nouvelle détection de topologie.
const RESCAN_TICKS: u32 = 15;

/// Capteurs CPU exposés à l'UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuData {
    pub temperature_c: Option<f64>,
    pub frequency_mhz: Option<u64>,
    pub power_w: Option<f64>,
}

/// Capteurs d'une GPU.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuData {
    /// `amd`, `intel`, `nvidia`, `unknown`.
    pub vendor: String,
    /// `integrated`, `discrete`, `unknown`.
    pub role: String,
    pub name: String,
    pub card: String,
    pub temperature_c: Option<f64>,
    pub frequency_mhz: Option<u64>,
    pub max_frequency_mhz: Option<u64>,
    pub power_w: Option<f64>,
}

impl From<&GpuSensors> for GpuData {
    fn from(gpu: &GpuSensors) -> Self {
        Self {
            vendor: gpu.vendor.as_str().to_owned(),
            role: gpu.role.as_str().to_owned(),
            name: gpu.name.clone(),
            card: gpu.card.clone(),
            temperature_c: gpu.temperature_c,
            frequency_mhz: gpu.frequency_mhz,
            max_frequency_mhz: gpu.max_frequency_mhz,
            power_w: gpu.power_w,
        }
    }
}

/// Instantané des capteurs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorData {
    /// Collecte demandée par un client.
    pub enabled: bool,
    /// Vrai si au moins un capteur est disponible.
    pub available: bool,
    pub cpu: CpuData,
    /// iGPU détectée (commodité UI).
    pub igpu: Option<GpuData>,
    /// dGPU détectée (commodité UI).
    pub dgpu: Option<GpuData>,
    /// Toutes les GPU détectées.
    pub gpus: Vec<GpuData>,
}

impl SensorData {
    /// État publié quand la collecte est désactivée.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    fn from_snapshot(snapshot: &SensorSnapshot) -> Self {
        let gpus: Vec<GpuData> = snapshot.gpus.iter().map(GpuData::from).collect();
        let igpu = snapshot
            .gpus
            .iter()
            .find(|gpu| gpu.role == GpuRole::Integrated)
            .map(GpuData::from);
        let dgpu = snapshot
            .gpus
            .iter()
            .find(|gpu| gpu.role == GpuRole::Discrete)
            .map(GpuData::from);
        let available = snapshot.cpu.temperature_c.is_some()
            || snapshot.cpu.package_power_w.is_some()
            || !gpus.is_empty();
        Self {
            enabled: true,
            available,
            cpu: CpuData {
                temperature_c: snapshot.cpu.temperature_c,
                frequency_mhz: snapshot.cpu.frequency_mhz,
                power_w: snapshot.cpu.package_power_w,
            },
            igpu,
            dgpu,
            gpus,
        }
    }
}

/// Runtime de collecte.
#[derive(Debug)]
pub struct SensorRuntime {
    root: SysfsRoot,
    /// Construit au premier tick, dans `spawn_blocking` : la construction
    /// énumère hwmon, RAPL et DRM, ce qui ne doit pas bloquer le runtime
    /// asynchrone au démarrage.
    collector: Option<SensorCollector>,
    enabled: Arc<AtomicBool>,
    idle_ticks: u32,
}

impl SensorRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot, enabled: Arc<AtomicBool>) -> Self {
        Self {
            root,
            collector: None,
            enabled,
            idle_ticks: 0,
        }
    }

    /// Exécute un tick de collecte.
    pub fn tick(&mut self, _input: &HardwareInputs) -> SensorData {
        if !self.enabled.load(Ordering::Relaxed) {
            if let Some(collector) = self.collector.as_mut() {
                collector.reset();
            }
            return SensorData::disabled();
        }
        let root = self.root.clone();
        let collector = self
            .collector
            .get_or_insert_with(|| SensorCollector::new(root));
        let snapshot = collector.collect();
        if snapshot.gpus.is_empty() {
            self.idle_ticks = self.idle_ticks.saturating_add(1);
            if self.idle_ticks >= RESCAN_TICKS {
                self.idle_ticks = 0;
                collector.rescan();
            }
        } else {
            self.idle_ticks = 0;
        }
        SensorData::from_snapshot(&snapshot)
    }
}

/// Démarre le worker de capteurs.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    enabled: Option<Arc<AtomicBool>>,
) -> tokio::task::JoinHandle<()> {
    let flag = enabled.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let runtime = SensorRuntime::new(root, Arc::clone(&flag));
    // Pas de `gate` : le runtime vérifie le drapeau lui-même pour publier
    // `enabled: false` immédiatement quand la collecte est coupée.
    spawn_worker(
        "capteurs",
        SENSOR_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_sensors,
        SensorRuntime::tick,
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

    fn fixture(root: &Path) {
        write(root, "class/hwmon/hwmon0/name", "coretemp\n");
        write(root, "class/hwmon/hwmon0/temp1_input", "50000\n");
        write(root, "devices/system/cpu/online", "0\n");
        write(
            root,
            "devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
            "1200000\n",
        );
        write(
            root,
            "class/drm/card0/device/uevent",
            "DRIVER=amdgpu\nPCI_ID=1002:1638\n",
        );
        write(root, "class/drm/card0/device/hwmon/hwmon1/name", "amdgpu\n");
        write(
            root,
            "class/drm/card0/device/hwmon/hwmon1/temp1_input",
            "48000\n",
        );
    }

    #[test]
    fn disabled_collection_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = SensorRuntime::new(SysfsRoot::new(dir.path()), flag);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.enabled);
        assert!(!data.available);
        assert_eq!(data.cpu, CpuData::default());
        assert!(data.gpus.is_empty());
    }

    #[test]
    fn enabled_collection_publishes_values() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let flag = Arc::new(AtomicBool::new(true));
        let mut runtime = SensorRuntime::new(SysfsRoot::new(dir.path()), flag);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.enabled);
        assert!(data.available);
        assert_eq!(data.cpu.temperature_c, Some(50.0));
        assert_eq!(data.cpu.frequency_mhz, Some(1200));
        assert_eq!(data.gpus.len(), 1);
        assert_eq!(data.igpu.as_ref().expect("igpu").vendor, "amd");
        assert!(data.dgpu.is_none());
        assert_eq!(data.igpu.as_ref().expect("igpu").temperature_c, Some(48.0));
    }

    #[test]
    fn disabled_state_resets_rapl_delta() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let flag = Arc::new(AtomicBool::new(true));
        let mut runtime = SensorRuntime::new(SysfsRoot::new(dir.path()), flag.clone());
        let _ = runtime.tick(&HardwareInputs::default());
        flag.store(false, Ordering::Relaxed);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.enabled);
        flag.store(true, Ordering::Relaxed);
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.cpu.power_w, None, "le delta RAPL a été réinitialisé");
    }

    #[test]
    fn missing_hardware_is_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let flag = Arc::new(AtomicBool::new(true));
        let mut runtime = SensorRuntime::new(SysfsRoot::new(dir.path()), flag);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.enabled);
        assert!(!data.available);
        assert!(data.gpus.is_empty());
    }

    #[test]
    fn serializes_camel_case() {
        let data = SensorData {
            enabled: true,
            available: true,
            cpu: CpuData {
                temperature_c: Some(45.5),
                frequency_mhz: Some(2500),
                power_w: Some(12.0),
            },
            igpu: None,
            dgpu: None,
            gpus: Vec::new(),
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"temperatureC\":45.5"));
        assert!(json.contains("\"frequencyMhz\":2500"));
        assert!(json.contains("\"powerW\":12.0"));
    }
}
