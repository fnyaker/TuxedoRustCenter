// SPDX-License-Identifier: GPL-3.0-or-later
//! Contrôle CPU via cpufreq.
//!
//! Chemins noyau utilisés :
//!
//! ```text
//! /sys/devices/system/cpu/{possible,present,online,kernel_max}
//! /sys/devices/system/cpu/cpuN/online
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_driver
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_governor
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_available_governors
//! /sys/devices/system/cpu/cpuN/cpufreq/energy_performance_preference
//! /sys/devices/system/cpu/cpuN/cpufreq/energy_performance_available_preferences
//! /sys/devices/system/cpu/cpuN/cpufreq/{cpuinfo,scaling}_{min,max}_freq
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq
//! /sys/devices/system/cpu/cpuN/cpufreq/scaling_available_frequencies
//! /sys/devices/system/cpu/intel_pstate/{no_turbo,max_perf_pct,min_perf_pct,status,...}
//! /sys/devices/system/cpu/amd_pstate/status
//! /sys/devices/system/cpu/cpufreq/boost
//! ```
//!
//! Toutes les fréquences sont en kHz, comme dans sysfs. Les écritures sont
//! validées contre les valeurs réellement exposées par le noyau, puis
//! appliquées aux politiques en ligne uniquement.

use std::fmt;

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Pilote cpufreq détecté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalingDriver {
    AcpiCpufreq,
    IntelPstate,
    AmdPstate,
    AmdPstateEpp,
    Unknown(String),
}

impl ScalingDriver {
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim() {
            "acpi-cpufreq" => Self::AcpiCpufreq,
            "intel_pstate" => Self::IntelPstate,
            "amd-pstate" => Self::AmdPstate,
            "amd-pstate-epp" => Self::AmdPstateEpp,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// Vrai si le pilote gère lui-même le turbo/EPP.
    pub const fn is_pstate(&self) -> bool {
        matches!(
            self,
            Self::IntelPstate | Self::AmdPstate | Self::AmdPstateEpp
        )
    }
}

impl fmt::Display for ScalingDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcpiCpufreq => formatter.write_str("acpi-cpufreq"),
            Self::IntelPstate => formatter.write_str("intel_pstate"),
            Self::AmdPstate => formatter.write_str("amd-pstate"),
            Self::AmdPstateEpp => formatter.write_str("amd-pstate-epp"),
            Self::Unknown(value) => formatter.write_str(value),
        }
    }
}

/// Topologie logique annoncée par le noyau.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuTopology {
    pub possible: Vec<u32>,
    pub present: Vec<u32>,
    /// Liste globale `online` au moment de la lecture. Peut être vide si le
    /// noyau ne l'expose pas.
    pub online: Vec<u32>,
}

impl CpuTopology {
    pub fn read(root: &SysfsRoot) -> Result<Self> {
        let online = match root.read_optional_string("devices/system/cpu/online")? {
            Some(value) => crate::sysfs::parse_cpu_list(&value)?,
            None => Vec::new(),
        };
        Ok(Self {
            possible: root.read_cpu_list("devices/system/cpu/possible")?,
            present: root.read_cpu_list("devices/system/cpu/present")?,
            online,
        })
    }

    /// Nombre de CPU présents.
    pub fn cpu_count(&self) -> usize {
        self.present.len()
    }
}

/// État d'une politique cpufreq (un ou plusieurs CPU logiques).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuPolicy {
    pub cpu: u32,
    pub online: Option<bool>,
    pub scaling_driver: Option<String>,
    pub governor: Option<String>,
    pub available_governors: Vec<String>,
    pub epp: Option<String>,
    pub available_epps: Vec<String>,
    pub cpuinfo_min_khz: Option<u64>,
    pub cpuinfo_max_khz: Option<u64>,
    pub scaling_min_khz: Option<u64>,
    pub scaling_max_khz: Option<u64>,
    pub scaling_cur_khz: Option<u64>,
    pub available_frequencies_khz: Vec<u64>,
    pub has_cpufreq: bool,
}

impl CpuPolicy {
    fn read(root: &SysfsRoot, cpu: u32) -> Result<Self> {
        let base = format!("devices/system/cpu/cpu{cpu}");
        let cpufreq = format!("{base}/cpufreq");
        let has_cpufreq = root.exists(&cpufreq);
        let mut policy = Self {
            cpu,
            online: None,
            scaling_driver: None,
            governor: None,
            available_governors: Vec::new(),
            epp: None,
            available_epps: Vec::new(),
            cpuinfo_min_khz: None,
            cpuinfo_max_khz: None,
            scaling_min_khz: None,
            scaling_max_khz: None,
            scaling_cur_khz: None,
            available_frequencies_khz: Vec::new(),
            has_cpufreq,
        };

        // `online` n'existe pas pour cpu0 : il est toujours en ligne.
        if cpu != 0 {
            policy.online = optional_bool(root, format!("{base}/online"));
        }
        if !has_cpufreq {
            return Ok(policy);
        }

        policy.scaling_driver = root.read_optional_string(format!("{cpufreq}/scaling_driver"))?;
        policy.governor = root.read_optional_string(format!("{cpufreq}/scaling_governor"))?;
        policy.available_governors =
            optional_string_list(root, format!("{cpufreq}/scaling_available_governors"))?;
        policy.epp =
            root.read_optional_string(format!("{cpufreq}/energy_performance_preference"))?;
        policy.available_epps = optional_string_list(
            root,
            format!("{cpufreq}/energy_performance_available_preferences"),
        )?;
        policy.cpuinfo_min_khz = optional_u64(root, format!("{cpufreq}/cpuinfo_min_freq"))?;
        policy.cpuinfo_max_khz = optional_u64(root, format!("{cpufreq}/cpuinfo_max_freq"))?;
        policy.scaling_min_khz = optional_u64(root, format!("{cpufreq}/scaling_min_freq"))?;
        policy.scaling_max_khz = optional_u64(root, format!("{cpufreq}/scaling_max_freq"))?;
        policy.scaling_cur_khz = optional_u64(root, format!("{cpufreq}/scaling_cur_freq"))?;
        if let Some(value) =
            root.read_optional_string(format!("{cpufreq}/scaling_available_frequencies"))?
        {
            policy.available_frequencies_khz = parse_frequency_list(&value);
        }
        Ok(policy)
    }

    /// Vrai si la politique peut recevoir des réglages (en ligne et cpufreq).
    pub fn is_active(&self) -> bool {
        self.has_cpufreq && (self.cpu == 0 || self.online.unwrap_or(true))
    }

    /// Fréquence maximale physique.
    pub fn max_khz(&self) -> Option<u64> {
        self.cpuinfo_max_khz.or(self.scaling_max_khz)
    }

    /// Fréquence minimale physique.
    pub fn min_khz(&self) -> Option<u64> {
        self.cpuinfo_min_khz.or(self.scaling_min_khz)
    }
}

/// État du pilote `intel_pstate`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntelPstate {
    pub no_turbo: Option<bool>,
    pub max_perf_pct: Option<u64>,
    pub min_perf_pct: Option<u64>,
    pub num_pstates: Option<u64>,
    pub status: Option<String>,
    pub turbo_pct: Option<u64>,
}

impl IntelPstate {
    pub fn read(root: &SysfsRoot) -> Result<Self> {
        const BASE: &str = "devices/system/cpu/intel_pstate";
        Ok(Self {
            no_turbo: optional_bool(root, format!("{BASE}/no_turbo")),
            max_perf_pct: optional_u64(root, format!("{BASE}/max_perf_pct"))?,
            min_perf_pct: optional_u64(root, format!("{BASE}/min_perf_pct"))?,
            num_pstates: optional_u64(root, format!("{BASE}/num_pstates"))?,
            status: root.read_optional_string(format!("{BASE}/status"))?,
            turbo_pct: optional_u64(root, format!("{BASE}/turbo_pct"))?,
        })
    }

    /// Vrai si le répertoire `intel_pstate` existe.
    pub fn is_available(root: &SysfsRoot) -> bool {
        root.exists("devices/system/cpu/intel_pstate")
    }
}

/// Contrôleur CPU principal.
#[derive(Debug, Clone)]
pub struct CpuController {
    root: SysfsRoot,
    topology: CpuTopology,
    policies: Vec<CpuPolicy>,
}

impl CpuController {
    /// Construit le contrôleur et lit la topologie.
    pub fn new(root: SysfsRoot) -> Result<Self> {
        let topology = CpuTopology::read(&root)?;
        let mut policies = Vec::with_capacity(topology.present.len());
        for cpu in &topology.present {
            policies.push(CpuPolicy::read(&root, *cpu)?);
        }
        Ok(Self {
            root,
            topology,
            policies,
        })
    }

    pub fn topology(&self) -> &CpuTopology {
        &self.topology
    }

    pub fn policies(&self) -> &[CpuPolicy] {
        &self.policies
    }

    /// Relit les politiques (après un changement de governor externe, par ex.).
    pub fn refresh(&mut self) -> Result<()> {
        *self = Self::new(self.root.clone())?;
        Ok(())
    }

    /// Premier pilote cpufreq annoncé.
    pub fn scaling_driver(&self) -> ScalingDriver {
        self.policies
            .iter()
            .find_map(|policy| policy.scaling_driver.as_deref())
            .map(ScalingDriver::from_sysfs)
            .unwrap_or_else(|| ScalingDriver::Unknown(String::new()))
    }

    /// État `intel_pstate` s'il est exposé.
    pub fn intel_pstate(&self) -> Result<Option<IntelPstate>> {
        if IntelPstate::is_available(&self.root) {
            Ok(Some(IntelPstate::read(&self.root)?))
        } else {
            Ok(None)
        }
    }

    /// Statut `amd_pstate` (`active`, `passive`, `guided`, `off`).
    pub fn amd_pstate_status(&self) -> Result<Option<String>> {
        self.root
            .read_optional_string("devices/system/cpu/amd_pstate/status")
    }

    /// CPU en ligne d'après le dernier rafraîchissement.
    pub fn online_cores(&self) -> Vec<u32> {
        self.policies
            .iter()
            .filter(|policy| policy.cpu == 0 || policy.online.unwrap_or(true))
            .map(|policy| policy.cpu)
            .collect()
    }

    /// Met `count` CPU en ligne (cpu0 reste toujours en ligne).
    ///
    /// Retourne le nombre de CPU effectivement en ligne après l'opération.
    pub fn set_online_cores(&self, count: usize) -> Result<usize> {
        if count == 0 || count > self.topology.present.len() {
            return Err(HwError::OutOfRange {
                context: "nombre de cœurs en ligne",
                value: count as i64,
                min: 1,
                max: self.topology.present.len() as i64,
            });
        }
        let mut writable = false;
        for (index, cpu) in self.topology.present.iter().enumerate() {
            if *cpu == 0 {
                continue;
            }
            let path = format!("devices/system/cpu/cpu{cpu}/online");
            if !self.root.is_file(&path) {
                continue;
            }
            writable = true;
            let desired = index < count;
            let current = optional_bool(&self.root, path.clone());
            if current != Some(desired) {
                self.root.write_bool(&path, desired)?;
            }
        }
        if !writable && self.topology.present.len() > 1 {
            return Err(HwError::NotSupported("online cores"));
        }
        // Recalcul après écriture : ne pas se fier au cache `policies`.
        let mut online = 0usize;
        for cpu in &self.topology.present {
            let host = *cpu == 0
                || optional_bool(&self.root, format!("devices/system/cpu/cpu{cpu}/online"))
                    .unwrap_or(true);
            if host {
                online += 1;
            }
        }
        Ok(online)
    }

    /// Applique un governor s'il est annoncé par toutes les politiques
    /// concernées. Aucune écriture partielle : la validation précède les
    /// écritures.
    pub fn set_governor(&self, governor: &str) -> Result<()> {
        validate_token(governor, "governor")?;
        let mut targets = Vec::new();
        for policy in self.policies.iter().filter(|policy| policy.is_active()) {
            if policy.available_governors.is_empty() {
                continue;
            }
            if !policy
                .available_governors
                .iter()
                .any(|item| item == governor)
            {
                return Err(HwError::InvalidValue {
                    context: "governor non disponible",
                    value: governor.to_owned(),
                });
            }
            targets.push(policy);
        }
        if targets.is_empty() {
            return Err(HwError::NotSupported("scaling_governor"));
        }
        for policy in targets {
            // On écrit toujours : la valeur en cache peut être obsolète si un
            // autre acteur (PPD, TLP) a modifié sysfs entre-temps.
            self.root.write_string(
                format!(
                    "devices/system/cpu/cpu{}/cpufreq/scaling_governor",
                    policy.cpu
                ),
                governor,
            )?;
        }
        Ok(())
    }

    /// Applique une préférence d'énergie (`energy_performance_preference`).
    pub fn set_energy_performance_preference(&self, preference: &str) -> Result<()> {
        validate_token(preference, "energy_performance_preference")?;
        let mut targets = Vec::new();
        for policy in self.policies.iter().filter(|policy| policy.is_active()) {
            if policy.available_epps.is_empty() {
                continue;
            }
            if !policy.available_epps.iter().any(|item| item == preference) {
                return Err(HwError::InvalidValue {
                    context: "energy_performance_preference non disponible",
                    value: preference.to_owned(),
                });
            }
            targets.push(policy);
        }
        if targets.is_empty() {
            return Err(HwError::NotSupported("energy_performance_preference"));
        }
        for policy in targets {
            self.root.write_string(
                format!(
                    "devices/system/cpu/cpu{}/cpufreq/energy_performance_preference",
                    policy.cpu
                ),
                preference,
            )?;
        }
        Ok(())
    }

    /// Applique `scaling_max_freq`, borné et aligné sur les fréquences
    /// réellement disponibles.
    pub fn set_scaling_max_khz(&self, target_khz: u64) -> Result<()> {
        let mut written = 0usize;
        for policy in self.policies.iter().filter(|policy| policy.is_active()) {
            let Some(max) = policy.max_khz() else {
                continue;
            };
            let min = policy.scaling_min_khz.or(policy.min_khz()).unwrap_or(0);
            let mut value = target_khz.clamp(min, max);
            if !policy.available_frequencies_khz.is_empty() {
                let candidates: Vec<u64> = policy
                    .available_frequencies_khz
                    .iter()
                    .copied()
                    .filter(|freq| *freq >= min)
                    .collect();
                if let Some(nearest) = nearest_frequency(value, &candidates) {
                    value = nearest;
                }
            }
            self.root.write_i64(
                format!(
                    "devices/system/cpu/cpu{}/cpufreq/scaling_max_freq",
                    policy.cpu
                ),
                value as i64,
            )?;
            written += 1;
        }
        if written == 0 {
            return Err(HwError::NotSupported("scaling_max_freq"));
        }
        Ok(())
    }

    /// Applique `scaling_min_freq`, borné et aligné sur les fréquences
    /// réellement disponibles.
    pub fn set_scaling_min_khz(&self, target_khz: u64) -> Result<()> {
        let mut written = 0usize;
        for policy in self.policies.iter().filter(|policy| policy.is_active()) {
            let Some(min) = policy.min_khz() else {
                continue;
            };
            let max = policy.scaling_max_khz.or(policy.max_khz()).unwrap_or(min);
            let mut value = target_khz.clamp(min, max);
            if !policy.available_frequencies_khz.is_empty() {
                let candidates: Vec<u64> = policy
                    .available_frequencies_khz
                    .iter()
                    .copied()
                    .filter(|freq| *freq <= max)
                    .collect();
                if let Some(nearest) = nearest_frequency(value, &candidates) {
                    value = nearest;
                }
            }
            self.root.write_i64(
                format!(
                    "devices/system/cpu/cpu{}/cpufreq/scaling_min_freq",
                    policy.cpu
                ),
                value as i64,
            )?;
            written += 1;
        }
        if written == 0 {
            return Err(HwError::NotSupported("scaling_min_freq"));
        }
        Ok(())
    }

    /// Active ou désactive le turbo via `intel_pstate/no_turbo`.
    pub fn set_no_turbo(&self, no_turbo: bool) -> Result<()> {
        let path = "devices/system/cpu/intel_pstate/no_turbo";
        if !self.root.is_file(path) {
            return Err(HwError::NotSupported("intel_pstate/no_turbo"));
        }
        self.root.write_bool(path, no_turbo)
    }

    /// Active ou désactive le boost via `cpufreq/boost` (acpi-cpufreq/amd).
    pub fn set_boost(&self, enabled: bool) -> Result<()> {
        let path = "devices/system/cpu/cpufreq/boost";
        if !self.root.is_file(path) {
            return Err(HwError::NotSupported("cpufreq/boost"));
        }
        self.root.write_bool(path, enabled)
    }
}

fn validate_token(value: &str, context: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'+' | b'.'))
    {
        return Err(HwError::InvalidValue {
            context,
            value: value.escape_default().to_string(),
        });
    }
    Ok(())
}

/// Choisit la fréquence disponible la plus proche, en préférant la plus basse
/// en cas d'égalité.
pub fn nearest_frequency(target: u64, candidates: &[u64]) -> Option<u64> {
    candidates.iter().copied().min_by_key(|freq| {
        let distance = freq.abs_diff(target);
        (distance, *freq)
    })
}

fn parse_frequency_list(value: &str) -> Vec<u64> {
    value
        .split_whitespace()
        .filter_map(|item| item.parse::<u64>().ok())
        .collect()
}

fn optional_bool(root: &SysfsRoot, path: String) -> Option<bool> {
    root.read_optional_string(path)
        .ok()
        .flatten()
        .and_then(|value| match value.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        })
}

fn optional_u64(root: &SysfsRoot, path: String) -> Result<Option<u64>> {
    match root.read_optional_string(path)? {
        Some(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| HwError::InvalidValue {
                context: "fréquence CPU",
                value,
            }),
        None => Ok(None),
    }
}

fn optional_string_list(root: &SysfsRoot, path: String) -> Result<Vec<String>> {
    match root.read_optional_string(path)? {
        Some(value) => Ok(value.split_whitespace().map(str::to_owned).collect()),
        None => Ok(Vec::new()),
    }
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

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Construit 4 CPU, avec cpufreq sur chacun.
    fn fixture() -> (tempfile::TempDir, SysfsRoot) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write(base, "devices/system/cpu/possible", "0-3\n");
        write(base, "devices/system/cpu/present", "0-3\n");
        write(base, "devices/system/cpu/online", "0-3\n");
        write(base, "devices/system/cpu/intel_pstate/no_turbo", "0\n");
        write(base, "devices/system/cpu/intel_pstate/status", "active\n");
        write(base, "devices/system/cpu/cpufreq/boost", "0\n");
        for cpu in 0..4 {
            let base_cpu = format!("devices/system/cpu/cpu{cpu}");
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_driver"),
                "intel_pstate\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_governor"),
                "powersave\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_available_governors"),
                "performance powersave\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/energy_performance_preference"),
                "balance_performance\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/energy_performance_available_preferences"),
                "default performance balance_performance balance_power power\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/cpuinfo_min_freq"),
                "400000\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/cpuinfo_max_freq"),
                "4800000\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_min_freq"),
                "400000\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_max_freq"),
                "4800000\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_cur_freq"),
                "2200000\n",
            );
            write(
                base,
                &format!("{base_cpu}/cpufreq/scaling_available_frequencies"),
                "400000 800000 1200000 1600000 2000000 2400000 2800000 3200000 3600000 4000000 4400000 4800000\n",
            );
            if cpu != 0 {
                write(base, &format!("{base_cpu}/online"), "1\n");
            }
        }
        let root = SysfsRoot::new(base);
        (dir, root)
    }

    #[test]
    fn topology_and_policy_reading() {
        let (_dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        assert_eq!(cpu.topology().cpu_count(), 4);
        assert_eq!(cpu.topology().online, vec![0, 1, 2, 3]);
        assert_eq!(cpu.policies().len(), 4);
        assert_eq!(cpu.scaling_driver(), ScalingDriver::IntelPstate);
        assert_eq!(cpu.intel_pstate().unwrap().unwrap().no_turbo, Some(false));
        assert_eq!(cpu.amd_pstate_status().unwrap(), None);
        assert!(
            cpu.policies()[0]
                .available_governors
                .contains(&"powersave".to_owned())
        );
        assert_eq!(cpu.online_cores(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn set_governor_writes_only_when_valid() {
        let (dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        cpu.set_governor("performance").unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu0/cpufreq/scaling_governor"
            ),
            "performance"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu3/cpufreq/scaling_governor"
            ),
            "performance"
        );

        let err = cpu.set_governor("schedutil").unwrap_err();
        assert!(matches!(err, HwError::InvalidValue { .. }));
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu0/cpufreq/scaling_governor"
            ),
            "performance"
        );
    }

    #[test]
    fn set_epp_validates_available_preferences() {
        let (dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        cpu.set_energy_performance_preference("power").unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu2/cpufreq/energy_performance_preference"
            ),
            "power"
        );
        assert!(cpu.set_energy_performance_preference("turbo").is_err());
    }

    #[test]
    fn max_frequency_is_clamped_and_snapped() {
        let (dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        // 1_700_000 tombe entre 1_600_000 et 2_000_000 : la plus proche est 1.6 GHz.
        cpu.set_scaling_max_khz(1_700_000).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu1/cpufreq/scaling_max_freq"
            ),
            "1600000"
        );
        // Au-delà du maximum physique : borné.
        cpu.set_scaling_max_khz(9_000_000).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu1/cpufreq/scaling_max_freq"
            ),
            "4800000"
        );
    }

    #[test]
    fn min_frequency_is_clamped_to_scaling_max() {
        let (dir, root) = fixture();
        // scaling_max réduit à 2 GHz.
        write(
            dir.path(),
            "devices/system/cpu/cpu0/cpufreq/scaling_max_freq",
            "2000000\n",
        );
        let cpu = CpuController::new(root).unwrap();
        cpu.set_scaling_min_khz(3_000_000).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu0/cpufreq/scaling_min_freq"
            ),
            "2000000"
        );
    }

    #[test]
    fn online_cores_writes_only_secondary_cores() {
        let (dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        assert_eq!(cpu.set_online_cores(2).unwrap(), 2);
        assert_eq!(read(dir.path(), "devices/system/cpu/cpu1/online"), "1");
        assert_eq!(read(dir.path(), "devices/system/cpu/cpu2/online"), "0");
        assert_eq!(read(dir.path(), "devices/system/cpu/cpu3/online"), "0");
        assert!(cpu.set_online_cores(0).is_err());
        assert!(cpu.set_online_cores(9).is_err());
    }

    #[test]
    fn no_turbo_and_boost_are_validated() {
        let (dir, root) = fixture();
        let cpu = CpuController::new(root).unwrap();
        cpu.set_no_turbo(true).unwrap();
        assert_eq!(
            read(dir.path(), "devices/system/cpu/intel_pstate/no_turbo"),
            "1"
        );
        cpu.set_boost(true).unwrap();
        assert_eq!(read(dir.path(), "devices/system/cpu/cpufreq/boost"), "1");
    }

    #[test]
    fn missing_cpufreq_degrades_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "devices/system/cpu/possible", "0\n");
        write(dir.path(), "devices/system/cpu/present", "0\n");
        let root = SysfsRoot::new(dir.path());
        let cpu = CpuController::new(root).unwrap();
        assert_eq!(cpu.topology().cpu_count(), 1);
        assert!(matches!(
            cpu.set_governor("powersave"),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            cpu.set_scaling_max_khz(1_000_000),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn driver_parsing_covers_supported_pilots() {
        assert_eq!(
            ScalingDriver::from_sysfs("intel_pstate"),
            ScalingDriver::IntelPstate
        );
        assert_eq!(
            ScalingDriver::from_sysfs("amd-pstate-epp"),
            ScalingDriver::AmdPstateEpp
        );
        assert_eq!(
            ScalingDriver::from_sysfs("acpi-cpufreq"),
            ScalingDriver::AcpiCpufreq
        );
        assert!(ScalingDriver::from_sysfs("intel_pstate").is_pstate());
        assert!(!ScalingDriver::from_sysfs("acpi-cpufreq").is_pstate());
        assert_eq!(ScalingDriver::from_sysfs("weird").to_string(), "weird");
    }

    #[test]
    fn offline_cores_are_not_written() {
        let (dir, root) = fixture();
        write(dir.path(), "devices/system/cpu/cpu3/online", "0\n");
        let cpu = CpuController::new(root).unwrap();
        // cpu3 est hors ligne : son governor ne doit pas être touché.
        cpu.set_governor("performance").unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu3/cpufreq/scaling_governor"
            ),
            "powersave"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/system/cpu/cpu2/cpufreq/scaling_governor"
            ),
            "performance"
        );
    }

    #[test]
    fn amd_pstate_status_is_optional() {
        let (dir, root) = fixture();
        write(
            dir.path(),
            "devices/system/cpu/amd_pstate/status",
            "active\n",
        );
        let cpu = CpuController::new(root).unwrap();
        assert_eq!(cpu.amd_pstate_status().unwrap().as_deref(), Some("active"));
    }

    #[test]
    fn nearest_frequency_prefers_lower_on_tie() {
        assert_eq!(nearest_frequency(1500, &[1000, 2000]), Some(1000));
        assert_eq!(nearest_frequency(1600, &[1000, 2000]), Some(2000));
        assert_eq!(nearest_frequency(1600, &[]), None);
    }
}
