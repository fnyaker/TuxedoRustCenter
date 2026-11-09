// SPDX-License-Identifier: GPL-3.0-or-later
//! Alimentation : secteurs et batteries via `/sys/class/power_supply`.
//!
//! Arborescence noyau (Documentation/ABI/testing/sysfs-class-power) :
//!
//! ```text
//! /sys/class/power_supply/<nom>/type          "Mains", "Battery", "USB", ...
//! /sys/class/power_supply/<nom>/online        0/1, uniquement pour Mains
//! /sys/class/power_supply/<nom>/capacity      0..100, batteries
//! /sys/class/power_supply/<nom>/status        Charging/Discharging/Full/...
//! /sys/class/power_supply/<nom>/present       0/1 (batterie amovible)
//! /sys/class/power_supply/<nom>/charge_control_start_threshold
//! /sys/class/power_supply/<nom>/charge_control_end_threshold
//! ```
//!
//! Le daemon utilise UPower en priorité (voir `docs/linux/PLATFORM_DESIGN.md`)
//! et cette lecture sysfs comme source de vérité de secours.

use std::fmt;

use crate::error::Result;
use crate::sysfs::SysfsRoot;

/// Classe de l'état d'alimentation, compatible avec le `ProfileStates` de
/// l'original (`power_ac` / `power_bat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// Secteur connecté.
    Ac,
    /// Sur batterie.
    Battery,
    /// Aucune source identifiable ou lecture impossible.
    Unknown,
}

impl PowerState {
    /// Clé stable pour la matrice d'état du daemon.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ac => "ac",
            Self::Battery => "battery",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for PowerState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Type d'alimentation exposé par le noyau.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerSupplyKind {
    Mains,
    Battery,
    Usb,
    Other(String),
    Unknown,
}

impl PowerSupplyKind {
    /// Convertit le contenu de l'attribut `type`.
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim() {
            "Mains" => Self::Mains,
            "Battery" => Self::Battery,
            "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP" => Self::Usb,
            "Unknown" | "" => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// État de charge d'une batterie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown(String),
}

impl BatteryStatus {
    /// Convertit le contenu de l'attribut `status`.
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim() {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" => Self::Full,
            "Not charging" | "Not_Charging" => Self::NotCharging,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// Vrai si la batterie reçoit de l'énergie (secteur présent).
    pub const fn is_charging(&self) -> bool {
        matches!(self, Self::Charging | Self::Full | Self::NotCharging)
    }
}

impl fmt::Display for BatteryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Charging => formatter.write_str("Charging"),
            Self::Discharging => formatter.write_str("Discharging"),
            Self::Full => formatter.write_str("Full"),
            Self::NotCharging => formatter.write_str("Not charging"),
            Self::Unknown(value) => formatter.write_str(value),
        }
    }
}

/// Secteur alternatif détecté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcAdapter {
    /// Nom du répertoire sous `/sys/class/power_supply`.
    pub name: String,
    /// `online`, ou `None` si le noyau ne l'expose pas.
    pub online: Option<bool>,
}

/// État consolidé d'une batterie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryState {
    pub name: String,
    pub capacity_percent: Option<u8>,
    pub status: Option<BatteryStatus>,
    pub present: bool,
    /// Seuil bas standard (`charge_control_start_threshold`).
    pub charge_control_start: Option<u8>,
    /// Seuil haut standard (`charge_control_end_threshold`).
    pub charge_control_end: Option<u8>,
}

/// Vue consolidée de toutes les alimentations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PowerSupplies {
    pub adapters: Vec<AcAdapter>,
    pub batteries: Vec<BatteryState>,
}

impl PowerSupplies {
    /// Énumère `/sys/class/power_supply`.
    ///
    /// Un attribut illisible sur un périphérique est ignoré (le périphérique
    /// reste listé avec la valeur `None`) : un noyau ou un firmware exotique ne
    /// doit pas empêcher le daemon de démarrer.
    pub fn read(root: &SysfsRoot) -> Result<Self> {
        let mut supplies = Self::default();
        for name in root.list_dir("class/power_supply")? {
            let base = format!("class/power_supply/{name}");
            let kind = match root.read_optional_string(format!("{base}/type"))? {
                Some(value) => PowerSupplyKind::from_sysfs(&value),
                None => PowerSupplyKind::Unknown,
            };
            match kind {
                PowerSupplyKind::Mains => supplies.adapters.push(AcAdapter {
                    name,
                    online: optional_bool(root, format!("{base}/online")),
                }),
                PowerSupplyKind::Battery => {
                    supplies.batteries.push(read_battery(root, &base, name)?)
                }
                // USB-C et autres supplies : le daemon ne les utilise pas pour
                // décider AC/BAT, mais ils restent visibles via `type` si
                // nécessaire dans une version ultérieure.
                _ => {}
            }
        }
        Ok(supplies)
    }

    /// États AC/BAT consolidés.
    ///
    /// Priorité à l'adaptateur secteur, comme l'original (`StateUtils`). Si
    /// aucun adaptateur n'est exposé, l'état est déduit du statut de batterie.
    /// Ni adaptateur ni batterie ⇒ `Unknown`.
    pub fn power_state(&self) -> PowerState {
        if !self.adapters.is_empty() {
            if self
                .adapters
                .iter()
                .any(|adapter| adapter.online == Some(true))
            {
                return PowerState::Ac;
            }
            if self
                .adapters
                .iter()
                .all(|adapter| adapter.online == Some(false))
            {
                return PowerState::Battery;
            }
            return PowerState::Unknown;
        }

        if self.batteries.is_empty() {
            return PowerState::Unknown;
        }
        if self
            .batteries
            .iter()
            .any(|battery| battery.status == Some(BatteryStatus::Discharging))
        {
            return PowerState::Battery;
        }
        if self.batteries.iter().any(|battery| {
            battery
                .status
                .as_ref()
                .is_some_and(BatteryStatus::is_charging)
        }) {
            return PowerState::Ac;
        }
        PowerState::Unknown
    }

    /// Première batterie (l'original utilise aussi la première).
    pub fn first_battery(&self) -> Option<&BatteryState> {
        self.batteries.first()
    }

    /// Capacité moyenne des batteries présentes, si au moins une est lisible.
    pub fn average_capacity(&self) -> Option<u8> {
        let values: Vec<u32> = self
            .batteries
            .iter()
            .filter(|battery| battery.present)
            .filter_map(|battery| battery.capacity_percent.map(u32::from))
            .collect();
        if values.is_empty() {
            None
        } else {
            let sum: u32 = values.iter().sum();
            Some((sum / values.len() as u32).min(100) as u8)
        }
    }
}

fn read_battery(root: &SysfsRoot, base: &str, name: String) -> Result<BatteryState> {
    let capacity_percent = root
        .read_optional_string(format!("{base}/capacity"))?
        .and_then(|value| value.parse::<u16>().ok())
        .map(|value| value.min(100) as u8);
    let status = root
        .read_optional_string(format!("{base}/status"))?
        .map(|value| BatteryStatus::from_sysfs(&value));
    let present = optional_bool(root, format!("{base}/present")).unwrap_or(true);
    let charge_control_start = optional_u8(root, format!("{base}/charge_control_start_threshold"));
    let charge_control_end = optional_u8(root, format!("{base}/charge_control_end_threshold"));
    Ok(BatteryState {
        name,
        capacity_percent,
        status,
        present,
        charge_control_start,
        charge_control_end,
    })
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

fn optional_u8(root: &SysfsRoot, path: String) -> Option<u8> {
    root.read_optional_string(path)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u8>().ok())
}

/// Source d'état d'alimentation pour le daemon.
///
/// L'implémentation réelle lit sysfs ; les tests du daemon peuvent fournir une
/// implémentation simulée ou un [`SysfsPowerStateReader`] pointant sur une
/// arborescence factice.
pub trait PowerStateReader: Send + Sync {
    /// Lit l'état AC/BAT courant.
    fn power_state(&self) -> Result<PowerState>;

    /// Lit l'état détaillé de la première batterie.
    fn battery_state(&self) -> Result<Option<BatteryState>>;
}

/// Lecteur sysfs direct.
#[derive(Debug, Clone)]
pub struct SysfsPowerStateReader {
    root: SysfsRoot,
}

impl SysfsPowerStateReader {
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }

    /// Lit la vue complète (adaptateurs + batteries).
    pub fn read(&self) -> Result<PowerSupplies> {
        PowerSupplies::read(&self.root)
    }
}

impl PowerStateReader for SysfsPowerStateReader {
    fn power_state(&self) -> Result<PowerState> {
        Ok(self.read()?.power_state())
    }

    fn battery_state(&self) -> Result<Option<BatteryState>> {
        Ok(self.read()?.first_battery().cloned())
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

    fn fixture() -> (tempfile::TempDir, SysfsRoot) {
        let dir = tempfile::tempdir().unwrap();
        let root = SysfsRoot::new(dir.path());
        (dir, root)
    }

    #[test]
    fn ac_online_detects_ac() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/AC/type", "Mains\n");
        write(dir.path(), "class/power_supply/AC/online", "1\n");
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(dir.path(), "class/power_supply/BAT0/capacity", "80\n");
        write(dir.path(), "class/power_supply/BAT0/status", "Charging\n");

        let supplies = PowerSupplies::read(&root).unwrap();
        assert_eq!(supplies.power_state(), PowerState::Ac);
        assert_eq!(supplies.first_battery().unwrap().capacity_percent, Some(80));
        assert_eq!(
            supplies.first_battery().unwrap().status,
            Some(BatteryStatus::Charging)
        );
    }

    #[test]
    fn ac_offline_detects_battery() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/AC/type", "Mains\n");
        write(dir.path(), "class/power_supply/AC/online", "0\n");
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Battery
        );
    }

    #[test]
    fn battery_only_falls_back_to_status() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(
            dir.path(),
            "class/power_supply/BAT0/status",
            "Discharging\n",
        );
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Battery
        );

        write(dir.path(), "class/power_supply/BAT0/status", "Full\n");
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Ac
        );
    }

    #[test]
    fn no_supplies_is_unknown() {
        let (_dir, root) = fixture();
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Unknown
        );
    }

    #[test]
    fn corrupt_online_is_unknown_not_a_guess() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/AC/type", "Mains\n");
        write(dir.path(), "class/power_supply/AC/online", "garbage\n");
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Unknown
        );
    }

    #[test]
    fn usb_supplies_are_ignored_for_state() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/ucsi-source/type", "USB\n");
        write(dir.path(), "class/power_supply/ucsi-source/online", "1\n");
        assert_eq!(
            PowerSupplies::read(&root).unwrap().power_state(),
            PowerState::Unknown
        );
    }

    #[test]
    fn thresholds_and_average_capacity() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(dir.path(), "class/power_supply/BAT0/capacity", "40\n");
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_control_end_threshold",
            "80\n",
        );
        write(dir.path(), "class/power_supply/BAT1/type", "Battery\n");
        write(dir.path(), "class/power_supply/BAT1/capacity", "60\n");
        let supplies = PowerSupplies::read(&root).unwrap();
        assert_eq!(supplies.average_capacity(), Some(50));
        assert_eq!(supplies.batteries[0].charge_control_end, Some(80));
    }

    #[test]
    fn battery_status_parsing() {
        assert_eq!(
            BatteryStatus::from_sysfs("Not charging"),
            BatteryStatus::NotCharging
        );
        assert!(BatteryStatus::Full.is_charging());
        assert!(!BatteryStatus::Discharging.is_charging());
        assert_eq!(
            BatteryStatus::from_sysfs("weird"),
            BatteryStatus::Unknown("weird".to_owned())
        );
    }

    #[test]
    fn reader_trait_works() {
        let (dir, root) = fixture();
        write(dir.path(), "class/power_supply/AC/type", "Mains\n");
        write(dir.path(), "class/power_supply/AC/online", "1\n");
        let reader = SysfsPowerStateReader::new(root);
        assert_eq!(reader.power_state().unwrap(), PowerState::Ac);
        assert!(reader.battery_state().unwrap().is_none());
    }
}
