// SPDX-License-Identifier: GPL-3.0-or-later
//! Capteurs : température/fréquence/puissance CPU et GPU.
//!
//! # Sources
//!
//! ```text
//! /sys/class/hwmon/hwmonN/name                         coretemp, k10temp, amdgpu, ...
//! /sys/class/hwmon/hwmonN/temp1_input                  millidegrés
//! /sys/devices/system/cpu/online                       liste CPU
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq
//! /sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/{name,energy_uj,max_energy_range_uj}
//! /sys/class/drm/cardN/device/uevent                   PCI_ID=VVVV:DDDD, DRIVER=...
//! /sys/class/drm/cardN/device/hwmon/hwmonM/...
//! /sys/class/drm/cardN/gt_act_freq_mhz                 i915
//! /sys/class/drm/cardN/device/pp_dpm_sclk              amdgpu (fréquence max)
//! nvidia-smi --query-gpu=...                           (allowlist, lecture seule)
//! ```
//!
//! Sources : `GpuInfoWorker`, `IntelRAPLController`, `PowerController` et
//! `AvailabilityService` de l'original.
//!
//! # Règles
//!
//! - **Aucune valeur inventée** : un capteur absent vaut `None`, jamais `0`.
//! - **RAPL** : la puissance est calculée par différence d'énergie entre deux
//!   appels. Le compteur peut déborder ; `max_energy_range_uj` est utilisé
//!   pour corriger le *wrap*. Le premier appel ne produit pas de puissance
//!   (pas de delta).
//! - **Classification GPU** : vendor PCI + liste d'identifiants AMD iGPU
//!   reprise de `AmdDeviceIDs.ts`. Intel est considéré intégré, NVIDIA discret.
//!   Une AMD hors liste est considérée discrète, comme
//!   `AvailabilityService`. Un GPU inclassable porte `role = unknown`.
//! - **NVIDIA** : `nvidia-smi` est appelé au plus une fois par collecte, et
//!   seulement si une GPU NVIDIA est détectée. Un échec dégrade les champs en
//!   `None` sans faire échouer la collecte.
//! - **Coût** : [`SensorCollector::collect`] lit un nombre borné de fichiers
//!   (≈ 20). Le daemon ne l'appelle qu'après `SetSensorDataCollectionStatus`
//!   (`docs/linux/PLATFORM_DESIGN.md` §7).
//!
//! Le wrapper d'échantillonnage (`hardware::sensors::SensorWorker`) applique
//! cette politique côté daemon.

use std::time::{Duration, Instant};

use crate::nvidia::{NvidiaSensors, NvidiaSmi};
use crate::sysfs::{SysfsRoot, parse_cpu_list};

/// Classe hwmon.
const CLASS_HWMON: &str = "class/hwmon";
/// Classe DRM.
const CLASS_DRM: &str = "class/drm";
/// Base CPU.
const CPU_BASE: &str = "devices/system/cpu";
/// Racines RAPL possibles (noyau récent puis ancien).
const RAPL_ROOTS: [&str; 2] = [
    "devices/virtual/powercap/intel-rapl/intel-rapl:0",
    "devices/virtual/powercap/intel-rapl:0",
];
/// Noms hwmon CPU acceptés, par ordre de préférence.
const CPU_HWMON_NAMES: [&str; 4] = ["coretemp", "k10temp", "zenpower", "cpu_thermal"];
/// Identifiants PCI des iGPU AMD (repris de `AmdDeviceIDs.ts`).
const AMD_IGPU_IDS: [&str; 10] = [
    "15D8", "1636", "164C", "1638", "1681", "15BF", "1900", "13C0", "1114", "150E",
];
/// Zone RAPL utilisée pour la consommation iGPU Intel.
const IGPU_RAPL_NAMES: [&str; 2] = ["uncore", "i915"];
/// Nombre maximal de GPU exposées.
const MAX_GPUS: usize = 8;

/// Vendor d'une GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Amd,
    Intel,
    Nvidia,
    Unknown,
}

impl GpuVendor {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Amd => "amd",
            Self::Intel => "intel",
            Self::Nvidia => "nvidia",
            Self::Unknown => "unknown",
        }
    }

    fn from_pci_vendor(vendor: &str) -> Self {
        match vendor.to_ascii_uppercase().as_str() {
            "1002" => Self::Amd,
            "8086" => Self::Intel,
            "10DE" => Self::Nvidia,
            _ => Self::Unknown,
        }
    }
}

/// Rôle d'une GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuRole {
    Integrated,
    Discrete,
    Unknown,
}

impl GpuRole {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Integrated => "integrated",
            Self::Discrete => "discrete",
            Self::Unknown => "unknown",
        }
    }
}

/// Capteurs d'une GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuSensors {
    /// Vendor PCI.
    pub vendor: GpuVendor,
    /// Rôle iGPU/dGPU.
    pub role: GpuRole,
    /// Nom du pilote/hwmon (`amdgpu`, `i915`, `nvidia`).
    pub name: String,
    /// Nom de la carte DRM (`card0`).
    pub card: String,
    /// Température en °C.
    pub temperature_c: Option<f64>,
    /// Fréquence courante en MHz.
    pub frequency_mhz: Option<u64>,
    /// Fréquence maximale en MHz.
    pub max_frequency_mhz: Option<u64>,
    /// Puissance en watts.
    pub power_w: Option<f64>,
}

impl GpuSensors {
    fn empty(vendor: GpuVendor, role: GpuRole, name: String, card: String) -> Self {
        Self {
            vendor,
            role,
            name,
            card,
            temperature_c: None,
            frequency_mhz: None,
            max_frequency_mhz: None,
            power_w: None,
        }
    }
}

/// Présence de GPU par rôle, sans lecture de capteur.
///
/// Dérivée de `/sys/class/drm` et des identifiants PCI ; aucune valeur de
/// capteur, aucun `nvidia-smi`, aucun bail de collecte n'est nécessaire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuPresence {
    /// Au moins une GPU intégrée.
    pub integrated: bool,
    /// Au moins une GPU discrète.
    pub discrete: bool,
}

/// Détecte la présence d'iGPU/dGPU sans lire de capteurs.
///
/// Utile aux capabilities du Dashboard même quand la collecte de capteurs est
/// désactivée. Le coût est une énumération DRM/Pci, sans commande externe.
#[must_use]
pub fn gpu_presence(root: &SysfsRoot) -> GpuPresence {
    let mut presence = GpuPresence::default();
    for device in enumerate_gpus(root) {
        match device.role {
            GpuRole::Integrated => presence.integrated = true,
            GpuRole::Discrete => presence.discrete = true,
            GpuRole::Unknown => {}
        }
    }
    presence
}

/// Capteurs CPU.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CpuSensors {
    /// Température du paquet en °C.
    pub temperature_c: Option<f64>,
    /// Fréquence moyenne des cœurs en ligne, en MHz.
    pub frequency_mhz: Option<u64>,
    /// Puissance du paquet RAPL en watts.
    pub package_power_w: Option<f64>,
}

/// Instantané de tous les capteurs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SensorSnapshot {
    pub cpu: CpuSensors,
    pub gpus: Vec<GpuSensors>,
}

impl SensorSnapshot {
    /// GPU intégrée, si détectée.
    #[must_use]
    pub fn integrated_gpu(&self) -> Option<&GpuSensors> {
        self.gpus.iter().find(|gpu| gpu.role == GpuRole::Integrated)
    }

    /// GPU discrète, si détectée.
    #[must_use]
    pub fn discrete_gpu(&self) -> Option<&GpuSensors> {
        self.gpus.iter().find(|gpu| gpu.role == GpuRole::Discrete)
    }
}

/// Compteur RAPL avec gestion du *wrap*.
#[derive(Debug, Clone)]
struct RaplCounter {
    base: String,
    last_energy_uj: Option<u64>,
    last_instant: Option<Instant>,
}

impl RaplCounter {
    fn new(base: String) -> Self {
        Self {
            base,
            last_energy_uj: None,
            last_instant: None,
        }
    }

    fn energy_uj(&self, root: &SysfsRoot) -> Option<u64> {
        root.read_i64(format!("{}/energy_uj", self.base))
            .ok()
            .and_then(|value| u64::try_from(value).ok())
    }

    fn max_range_uj(&self, root: &SysfsRoot) -> u64 {
        root.read_i64(format!("{}/max_energy_range_uj", self.base))
            .ok()
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX)
    }

    /// Échantillonne la puissance moyenne depuis le dernier appel.
    fn sample(&mut self, root: &SysfsRoot, now: Instant) -> Option<f64> {
        let energy = self.energy_uj(root)?;
        let previous_energy = self.last_energy_uj;
        let previous_instant = self.last_instant;
        self.last_energy_uj = Some(energy);
        self.last_instant = Some(now);
        let (Some(previous_energy), Some(previous_instant)) = (previous_energy, previous_instant)
        else {
            return None;
        };
        let elapsed = now.checked_duration_since(previous_instant)?;
        rapl_power_watts(previous_energy, energy, self.max_range_uj(root), elapsed)
    }
}

/// Calcule une puissance moyenne en watts à partir d'un delta d'énergie.
///
/// `energy_uj` est en microjoules : `µJ / s = µW`, d'où la division par
/// `1_000_000`. Le *wrap* est corrigé avec `max_range_uj`.
#[must_use]
fn rapl_power_watts(previous: u64, current: u64, max_range: u64, elapsed: Duration) -> Option<f64> {
    if elapsed.is_zero() {
        return None;
    }
    let delta = if current >= previous {
        current - previous
    } else {
        // Compteur remis à zéro : `max_range` peut être `u64::MAX` si le
        // fichier est absent ; la soustraction saturante évite tout panic.
        max_range.saturating_sub(previous).saturating_add(current)
    };
    if delta == 0 {
        return Some(0.0);
    }
    Some(delta as f64 / elapsed.as_secs_f64() / 1_000_000.0)
}

/// Collecteur de capteurs.
#[derive(Debug)]
pub struct SensorCollector {
    root: SysfsRoot,
    cpu_hwmon: Option<String>,
    rapl_package: Option<RaplCounter>,
    rapl_igpu: Option<RaplCounter>,
    nvidia: NvidiaSmi,
}

impl SensorCollector {
    /// Construit le collecteur et détecte les sources stables.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        let cpu_hwmon = find_cpu_hwmon(&root);
        let rapl_package = find_rapl_package(&root).map(RaplCounter::new);
        let rapl_igpu = rapl_package
            .as_ref()
            .and_then(|package| find_igpu_rapl(&root, &package.base))
            .map(RaplCounter::new);
        Self {
            root,
            cpu_hwmon,
            rapl_package,
            rapl_igpu,
            nvidia: NvidiaSmi::detect(),
        }
    }

    /// Racine sysfs utilisée.
    #[must_use]
    pub fn root(&self) -> &SysfsRoot {
        &self.root
    }

    /// Oublie l'état RAPL (veille/reprise, changement de profil).
    pub fn reset(&mut self) {
        for counter in [&mut self.rapl_package, &mut self.rapl_igpu]
            .into_iter()
            .flatten()
        {
            counter.last_energy_uj = None;
            counter.last_instant = None;
        }
    }

    /// Relit la topologie (hotplug, chargement tardif d'un pilote).
    pub fn rescan(&mut self) {
        self.cpu_hwmon = find_cpu_hwmon(&self.root);
        self.rapl_package = find_rapl_package(&self.root).map(RaplCounter::new);
        self.rapl_igpu = self
            .rapl_package
            .as_ref()
            .and_then(|package| find_igpu_rapl(&self.root, &package.base))
            .map(RaplCounter::new);
        self.nvidia = NvidiaSmi::detect();
    }

    /// Collecte un instantané complet.
    #[must_use]
    pub fn collect(&mut self) -> SensorSnapshot {
        let now = Instant::now();
        let cpu = self.collect_cpu(now);
        let gpus = self.collect_gpus();
        SensorSnapshot { cpu, gpus }
    }

    fn collect_cpu(&mut self, now: Instant) -> CpuSensors {
        let temperature_c = self.cpu_temperature();
        let frequency_mhz = self.cpu_frequency();
        let package_power_w = self
            .rapl_package
            .as_mut()
            .and_then(|counter| counter.sample(&self.root, now));
        CpuSensors {
            temperature_c,
            frequency_mhz,
            package_power_w,
        }
    }

    fn cpu_temperature(&mut self) -> Option<f64> {
        let hwmon = match &self.cpu_hwmon {
            Some(hwmon) if self.root.is_file(format!("{hwmon}/temp1_input")) => hwmon.clone(),
            _ => {
                self.cpu_hwmon = find_cpu_hwmon(&self.root);
                self.cpu_hwmon.clone()?
            }
        };
        let millidegrees = self.root.read_i64(format!("{hwmon}/temp1_input")).ok()?;
        if millidegrees <= 0 {
            return None;
        }
        Some(millidegrees as f64 / 1000.0)
    }

    fn cpu_frequency(&self) -> Option<u64> {
        let online = self
            .root
            .read_string(format!("{CPU_BASE}/online"))
            .ok()
            .and_then(|value| parse_cpu_list(&value).ok())
            .filter(|cpus| !cpus.is_empty())?;
        let mut total = 0u64;
        let mut count = 0u64;
        for cpu in online {
            let base = format!("{CPU_BASE}/cpu{cpu}/cpufreq");
            let Some(khz) = self
                .root
                .read_i64(format!("{base}/scaling_cur_freq"))
                .ok()
                .or_else(|| self.root.read_i64(format!("{base}/cpuinfo_cur_freq")).ok())
            else {
                continue;
            };
            if khz <= 0 {
                continue;
            }
            total = total.saturating_add(khz as u64);
            count += 1;
        }
        (count > 0).then(|| total / count / 1000)
    }

    fn collect_gpus(&mut self) -> Vec<GpuSensors> {
        let devices = enumerate_gpus(&self.root);
        let mut nvidia_sensors: Option<NvidiaSensors> = None;
        let mut gpus = Vec::with_capacity(devices.len());
        for device in devices {
            let mut gpu = GpuSensors::empty(
                device.vendor,
                device.role,
                device.name.clone(),
                device.card.clone(),
            );
            if let Some(hwmon) = &device.hwmon {
                gpu.temperature_c = read_temperature(&self.root, hwmon);
                gpu.power_w = read_power(&self.root, hwmon);
                gpu.frequency_mhz = read_frequency(&self.root, hwmon);
            }
            if device.vendor == GpuVendor::Intel {
                gpu.frequency_mhz = gpu
                    .frequency_mhz
                    .or_else(|| read_intel_frequency(&self.root, &device.card));
                gpu.max_frequency_mhz = read_optional_mhz(
                    &self.root,
                    format!("{CLASS_DRM}/{}/gt_RP0_freq_mhz", device.card),
                );
                gpu.power_w = gpu.power_w.or_else(|| {
                    self.rapl_igpu
                        .as_mut()
                        .and_then(|counter| counter.sample(&self.root, Instant::now()))
                });
            }
            if device.vendor == GpuVendor::Amd {
                gpu.max_frequency_mhz =
                    read_amd_max_frequency(&self.root, &device.card).or(gpu.max_frequency_mhz);
            }
            if device.vendor == GpuVendor::Nvidia {
                if nvidia_sensors.is_none() {
                    nvidia_sensors = match self.nvidia.sensors() {
                        Ok(sensors) => sensors,
                        Err(error) => {
                            tracing::debug!(%error, "capteurs nvidia-smi indisponibles");
                            None
                        }
                    };
                }
                if let Some(sensors) = nvidia_sensors {
                    gpu.power_w = gpu.power_w.or(sensors.power_draw_watts);
                    gpu.frequency_mhz = gpu
                        .frequency_mhz
                        .or(sensors.core_clock_mhz.map(|value| value as u64));
                    gpu.max_frequency_mhz = gpu
                        .max_frequency_mhz
                        .or(sensors.max_core_clock_mhz.map(|value| value as u64));
                }
            }
            gpus.push(gpu);
        }
        gpus
    }
}

/// GPU découverte dans `/sys/class/drm`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuDevice {
    card: String,
    vendor: GpuVendor,
    role: GpuRole,
    name: String,
    hwmon: Option<String>,
}

fn enumerate_gpus(root: &SysfsRoot) -> Vec<GpuDevice> {
    let Ok(entries) = root.list_dir(CLASS_DRM) else {
        return Vec::new();
    };
    let mut gpus = Vec::new();
    for card in entries {
        if !is_card_name(&card) {
            continue;
        }
        let base = format!("{CLASS_DRM}/{card}");
        let uevent = root
            .read_optional_string(format!("{base}/device/uevent"))
            .ok()
            .flatten()
            .unwrap_or_default();
        let (vendor_id, device_id, driver) = parse_pci_uevent(&uevent);
        if vendor_id.is_empty() {
            // Carte DRM sans identité PCI (virtio, simpledrm) : ce n'est pas
            // une GPU pilotable, on l'ignore.
            continue;
        }
        let vendor = GpuVendor::from_pci_vendor(&vendor_id);
        let role = match vendor {
            GpuVendor::Intel => GpuRole::Integrated,
            GpuVendor::Nvidia => GpuRole::Discrete,
            GpuVendor::Amd => {
                if AMD_IGPU_IDS.contains(&device_id.as_str()) {
                    GpuRole::Integrated
                } else {
                    GpuRole::Discrete
                }
            }
            GpuVendor::Unknown => GpuRole::Unknown,
        };
        let hwmon = find_card_hwmon(root, &card);
        let name = hwmon
            .as_ref()
            .and_then(|hwmon| {
                root.read_optional_string(format!("{hwmon}/name"))
                    .ok()
                    .flatten()
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                if driver.is_empty() {
                    "unknown".to_owned()
                } else {
                    driver
                }
            });
        gpus.push(GpuDevice {
            card,
            vendor,
            role,
            name,
            hwmon,
        });
        if gpus.len() >= MAX_GPUS {
            break;
        }
    }
    gpus
}

fn is_card_name(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Parse `PCI_ID=1002:1638` et `DRIVER=amdgpu` d'un `uevent` noyau.
fn parse_pci_uevent(uevent: &str) -> (String, String, String) {
    let mut vendor = String::new();
    let mut device = String::new();
    let mut driver = String::new();
    for line in uevent.lines() {
        if let Some(value) = line.strip_prefix("PCI_ID=") {
            if let Some((vendor_part, device_part)) = value.split_once(':') {
                vendor = vendor_part.trim().to_ascii_uppercase();
                device = device_part.trim().to_ascii_uppercase();
            }
        } else if let Some(value) = line.strip_prefix("DRIVER=") {
            driver = value.trim().to_owned();
        }
    }
    (vendor, device, driver)
}

fn find_card_hwmon(root: &SysfsRoot, card: &str) -> Option<String> {
    let base = format!("{CLASS_DRM}/{card}/device/hwmon");
    let entries = root.list_dir(&base).ok()?;
    entries
        .into_iter()
        .find(|entry| root.is_file(format!("{base}/{entry}/name")))
        .map(|entry| format!("{base}/{entry}"))
}

fn find_cpu_hwmon(root: &SysfsRoot) -> Option<String> {
    let entries = root.list_dir(CLASS_HWMON).ok()?;
    for wanted in CPU_HWMON_NAMES {
        for entry in &entries {
            let base = format!("{CLASS_HWMON}/{entry}");
            let name = root
                .read_optional_string(format!("{base}/name"))
                .ok()
                .flatten()
                .unwrap_or_default();
            if name == wanted && root.is_file(format!("{base}/temp1_input")) {
                return Some(base);
            }
        }
    }
    None
}

fn find_rapl_package(root: &SysfsRoot) -> Option<String> {
    for base in RAPL_ROOTS {
        let name = root
            .read_optional_string(format!("{base}/name"))
            .ok()
            .flatten()
            .unwrap_or_default();
        if name == "package-0" && root.is_file(format!("{base}/energy_uj")) {
            return Some(base.to_owned());
        }
    }
    None
}

fn find_igpu_rapl(root: &SysfsRoot, package: &str) -> Option<String> {
    let entries = root.list_dir(package).ok()?;
    for entry in entries {
        if !entry.starts_with("intel-rapl:0:") {
            continue;
        }
        let base = format!("{package}/{entry}");
        let name = root
            .read_optional_string(format!("{base}/name"))
            .ok()
            .flatten()
            .unwrap_or_default();
        if IGPU_RAPL_NAMES.contains(&name.as_str()) && root.is_file(format!("{base}/energy_uj")) {
            return Some(base);
        }
    }
    None
}

fn read_temperature(root: &SysfsRoot, hwmon: &str) -> Option<f64> {
    let millidegrees = root.read_i64(format!("{hwmon}/temp1_input")).ok()?;
    (millidegrees > 0).then(|| millidegrees as f64 / 1000.0)
}

fn read_power(root: &SysfsRoot, hwmon: &str) -> Option<f64> {
    let microwatts = root
        .read_i64(format!("{hwmon}/power1_average"))
        .ok()
        .or_else(|| root.read_i64(format!("{hwmon}/power1_input")).ok())?;
    (microwatts >= 0).then(|| microwatts as f64 / 1_000_000.0)
}

fn read_frequency(root: &SysfsRoot, hwmon: &str) -> Option<u64> {
    let hertz = root.read_i64(format!("{hwmon}/freq1_input")).ok()?;
    (hertz > 0).then_some(hertz as u64 / 1_000_000)
}

fn read_intel_frequency(root: &SysfsRoot, card: &str) -> Option<u64> {
    read_optional_mhz(root, format!("{CLASS_DRM}/{card}/gt_act_freq_mhz"))
}

fn read_optional_mhz(root: &SysfsRoot, path: String) -> Option<u64> {
    let mhz = root.read_i64(path).ok()?;
    (mhz > 0).then_some(mhz as u64)
}

/// Fréquence maximale AMD depuis `pp_dpm_sclk` (`1234Mhz`).
fn read_amd_max_frequency(root: &SysfsRoot, card: &str) -> Option<u64> {
    let content = root
        .read_optional_string(format!("{CLASS_DRM}/{card}/device/pp_dpm_sclk"))
        .ok()
        .flatten()?;
    parse_amd_max_mhz(&content)
}

fn parse_amd_max_mhz(content: &str) -> Option<u64> {
    let mut max = None;
    for line in content.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(position) = lower.find("mhz") else {
            continue;
        };
        let prefix = &line[..position];
        let digits: String = prefix
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(value) = digits.parse::<u64>() {
            max = Some(max.map_or(value, |current: u64| current.max(value)));
        }
    }
    max
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

    fn cpu_fixture(root: &Path) {
        write(root, "class/hwmon/hwmon0/name", "k10temp\n");
        write(root, "class/hwmon/hwmon0/temp1_input", "45500\n");
        write(root, "devices/system/cpu/online", "0-1\n");
        write(
            root,
            "devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
            "2000000\n",
        );
        write(
            root,
            "devices/system/cpu/cpu1/cpufreq/scaling_cur_freq",
            "3000000\n",
        );
    }

    fn rapl_fixture(root: &Path) {
        let base = "devices/virtual/powercap/intel-rapl/intel-rapl:0";
        write(root, &format!("{base}/name"), "package-0\n");
        write(root, &format!("{base}/energy_uj"), "1000000\n");
        write(
            root,
            &format!("{base}/max_energy_range_uj"),
            "262143328850\n",
        );
        let igpu = format!("{base}/intel-rapl:0:1");
        write(root, &format!("{igpu}/name"), "uncore\n");
        write(root, &format!("{igpu}/energy_uj"), "500000\n");
        write(
            root,
            &format!("{igpu}/max_energy_range_uj"),
            "262143328850\n",
        );
    }

    fn amd_gpu_fixture(root: &Path, device_id: &str) {
        write(
            root,
            "class/drm/card0/device/uevent",
            &format!("DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:{device_id}\n"),
        );
        write(root, "class/drm/card0/device/hwmon/hwmon1/name", "amdgpu\n");
        write(
            root,
            "class/drm/card0/device/hwmon/hwmon1/temp1_input",
            "52000\n",
        );
        write(
            root,
            "class/drm/card0/device/hwmon/hwmon1/freq1_input",
            "1100000000\n",
        );
        write(
            root,
            "class/drm/card0/device/hwmon/hwmon1/power1_average",
            "12500000\n",
        );
        write(
            root,
            "class/drm/card0/device/pp_dpm_sclk",
            "0: 500Mhz\n1: 2100Mhz *\n",
        );
    }

    #[test]
    fn cpu_temperature_and_frequency() {
        let dir = tempfile::tempdir().unwrap();
        cpu_fixture(dir.path());
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        assert_eq!(snapshot.cpu.temperature_c, Some(45.5));
        assert_eq!(snapshot.cpu.frequency_mhz, Some(2500));
        assert_eq!(snapshot.cpu.package_power_w, None, "premier appel RAPL");
        assert!(snapshot.gpus.is_empty());
    }

    #[test]
    fn gpu_presence_uses_drm_without_collecting_sensors() {
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
        let presence = gpu_presence(&SysfsRoot::new(dir.path()));
        assert!(presence.integrated);
        assert!(presence.discrete);
    }

    #[test]
    fn gpu_presence_is_empty_without_drm() {
        let dir = tempfile::tempdir().unwrap();
        let presence = gpu_presence(&SysfsRoot::new(dir.path()));
        assert!(!presence.integrated);
        assert!(!presence.discrete);
    }

    #[test]
    fn absent_cpu_sensors_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        assert_eq!(snapshot.cpu, CpuSensors::default());
    }

    #[test]
    fn zero_temperature_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/hwmon/hwmon0/name", "coretemp\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_input", "0\n");
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        assert_eq!(collector.collect().cpu.temperature_c, None);
    }

    #[test]
    fn cpu_hwmon_preference_order() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/hwmon/hwmon0/name", "zenpower\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_input", "30000\n");
        write(dir.path(), "class/hwmon/hwmon1/name", "coretemp\n");
        write(dir.path(), "class/hwmon/hwmon1/temp1_input", "40000\n");
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        assert_eq!(collector.collect().cpu.temperature_c, Some(40.0));
    }

    #[test]
    fn rapl_power_delta_and_wrap() {
        assert_eq!(
            rapl_power_watts(1_000_000, 3_000_000, u64::MAX, Duration::from_secs(1)),
            Some(2.0)
        );
        assert_eq!(
            rapl_power_watts(3_000_000, 3_000_000, u64::MAX, Duration::from_secs(1)),
            Some(0.0)
        );
        // Wrap : 10 → 2 sur une plage de 12.
        assert_eq!(
            rapl_power_watts(10, 2, 12, Duration::from_secs(1)),
            Some(4.0 / 1_000_000.0)
        );
        assert_eq!(rapl_power_watts(1, 2, u64::MAX, Duration::ZERO), None);
    }

    #[test]
    fn rapl_power_is_measured_between_two_calls() {
        let dir = tempfile::tempdir().unwrap();
        cpu_fixture(dir.path());
        rapl_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let mut collector = SensorCollector::new(root);
        // Premier échantillon : pas de puissance.
        assert_eq!(collector.collect().cpu.package_power_w, None);
        // Deuxième lecture avec énergie accrue et temps écoulé : on ne peut pas
        // contrôler `Instant`, mais la valeur doit être positive ou nulle.
        std::thread::sleep(Duration::from_millis(10));
        write(
            dir.path(),
            "devices/virtual/powercap/intel-rapl/intel-rapl:0/energy_uj",
            "3000000\n",
        );
        let power = collector.collect().cpu.package_power_w;
        assert!(power.is_some_and(|value| value >= 0.0), "{power:?}");
    }

    #[test]
    fn amd_igpu_is_classified_and_read() {
        let dir = tempfile::tempdir().unwrap();
        amd_gpu_fixture(dir.path(), "1638");
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        assert_eq!(snapshot.gpus.len(), 1);
        let gpu = &snapshot.gpus[0];
        assert_eq!(gpu.vendor, GpuVendor::Amd);
        assert_eq!(gpu.role, GpuRole::Integrated);
        assert_eq!(gpu.name, "amdgpu");
        assert_eq!(gpu.temperature_c, Some(52.0));
        assert_eq!(gpu.frequency_mhz, Some(1100));
        assert_eq!(gpu.max_frequency_mhz, Some(2100));
        assert_eq!(gpu.power_w, Some(12.5));
        assert!(snapshot.integrated_gpu().is_some());
        assert!(snapshot.discrete_gpu().is_none());
    }

    #[test]
    fn amd_unknown_id_is_discrete() {
        let dir = tempfile::tempdir().unwrap();
        amd_gpu_fixture(dir.path(), "7480");
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        assert_eq!(snapshot.gpus[0].role, GpuRole::Discrete);
    }

    #[test]
    fn intel_igpu_uses_drm_frequencies() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/drm/card0/device/uevent",
            "DRIVER=i915\nPCI_ID=8086:7D55\n",
        );
        write(dir.path(), "class/drm/card0/gt_act_freq_mhz", "800\n");
        write(dir.path(), "class/drm/card0/gt_RP0_freq_mhz", "2400\n");
        write(
            dir.path(),
            "class/drm/card0/device/hwmon/hwmon0/name",
            "i915\n",
        );
        write(
            dir.path(),
            "class/drm/card0/device/hwmon/hwmon0/temp1_input",
            "41000\n",
        );
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        let gpu = &snapshot.gpus[0];
        assert_eq!(gpu.vendor, GpuVendor::Intel);
        assert_eq!(gpu.role, GpuRole::Integrated);
        assert_eq!(gpu.frequency_mhz, Some(800));
        assert_eq!(gpu.max_frequency_mhz, Some(2400));
        assert_eq!(gpu.temperature_c, Some(41.0));
    }

    #[test]
    fn nvidia_gpu_without_nvidia_smi_has_no_sensors() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/drm/card0/device/uevent",
            "DRIVER=nvidia\nPCI_ID=10DE:28E0\n",
        );
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let snapshot = collector.collect();
        assert_eq!(snapshot.gpus[0].vendor, GpuVendor::Nvidia);
        assert_eq!(snapshot.gpus[0].role, GpuRole::Discrete);
        if !NvidiaSmi::detect().is_available() {
            assert_eq!(snapshot.gpus[0].power_w, None);
            assert_eq!(snapshot.gpus[0].frequency_mhz, None);
        }
    }

    #[test]
    fn connectors_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/drm/card0-eDP-1/device/uevent",
            "PCI_ID=1002:1638\n",
        );
        write(
            dir.path(),
            "class/drm/card0/device/uevent",
            "PCI_ID=1002:1638\n",
        );
        let gpus = enumerate_gpus(&SysfsRoot::new(dir.path()));
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].card, "card0");
    }

    #[test]
    fn uevent_parsing() {
        assert_eq!(
            parse_pci_uevent("DRIVER=amdgpu\nPCI_ID=1002:1638\n"),
            ("1002".to_owned(), "1638".to_owned(), "amdgpu".to_owned())
        );
        assert_eq!(
            parse_pci_uevent("garbage"),
            (String::new(), String::new(), String::new())
        );
        assert_eq!(
            parse_pci_uevent("PCI_ID=broken"),
            (String::new(), String::new(), String::new())
        );
    }

    #[test]
    fn amd_max_frequency_parsing() {
        assert_eq!(parse_amd_max_mhz("0: 500Mhz\n1: 2100Mhz *\n"), Some(2100));
        assert_eq!(parse_amd_max_mhz("no frequencies"), None);
        assert_eq!(parse_amd_max_mhz(""), None);
    }

    #[test]
    fn gpu_vendor_mapping() {
        assert_eq!(GpuVendor::from_pci_vendor("1002"), GpuVendor::Amd);
        assert_eq!(GpuVendor::from_pci_vendor("8086"), GpuVendor::Intel);
        assert_eq!(GpuVendor::from_pci_vendor("10de"), GpuVendor::Nvidia);
        assert_eq!(GpuVendor::from_pci_vendor("1234"), GpuVendor::Unknown);
    }

    #[test]
    fn reset_clears_rapl_state() {
        let dir = tempfile::tempdir().unwrap();
        cpu_fixture(dir.path());
        rapl_fixture(dir.path());
        let mut collector = SensorCollector::new(SysfsRoot::new(dir.path()));
        let _ = collector.collect();
        assert!(
            collector
                .rapl_package
                .as_ref()
                .unwrap()
                .last_energy_uj
                .is_some()
        );
        collector.reset();
        assert!(
            collector
                .rapl_package
                .as_ref()
                .unwrap()
                .last_energy_uj
                .is_none()
        );
    }
}
