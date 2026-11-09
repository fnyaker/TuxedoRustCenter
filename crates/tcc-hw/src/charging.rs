// SPDX-License-Identifier: GPL-3.0-or-later
//! Charge de la batterie : profils/priorités TUXEDO et seuils standard.
//!
//! # Chemins
//!
//! ```text
//! /sys/devices/platform/tuxedo_keyboard/charging_profile/charging_profile
//! /sys/devices/platform/tuxedo_keyboard/charging_profile/charging_profiles_available
//! /sys/devices/platform/tuxedo_keyboard/charging_priority/charging_prio
//! /sys/devices/platform/tuxedo_keyboard/charging_priority/charging_prios_available
//! /sys/class/power_supply/<batterie>/charge_control_start_threshold
//! /sys/class/power_supply/<batterie>/charge_control_end_threshold
//! /sys/class/power_supply/<batterie>/charge_control_start_available_thresholds
//! /sys/class/power_supply/<batterie>/charge_control_end_available_thresholds
//! /sys/class/power_supply/<batterie>/charge_type
//! ```
//!
//! Sources : `ChargingWorker`, `ChargingProfileController`,
//! `ChargingPriorityController` et `PowerSupplyController` de l'original, et
//! `Documentation/ABI/testing/sysfs-class-power` du noyau.
//!
//! # Comportement repris de l'original
//!
//! - Les profils TUXEDO sont exposés dans un sous-répertoire
//!   (`charging_profile/charging_profile`) et non directement sous
//!   `tuxedo_keyboard`.
//! - Une valeur n'est écrite que si elle figure dans la liste
//!   `*_available_*` et diffère de la valeur courante
//!   (`ChargingWorker.applyChargingProfile`).
//! - Les seuils ne sont appliqués par le firmware que lorsque
//!   `charge_type == Custom` : l'interface d'origine écrivait `Custom` avant
//!   les seuils (`thresholdRadioGroupChange`). [`ChargingController::set_thresholds`]
//!   reproduit cette séquence explicitement.
//! - `start < end` est refusé (l'UI d'origine affiche un dialogue d'erreur).
//!
//! # Différences assumées
//!
//! - Les valeurs disponibles sont **toujours** validées avant écriture. Si le
//!   noyau n'expose pas `*_available_thresholds` (attribut non standard), la
//!   validation se limite à `0..=100` et `start < end`, comme le repli de
//!   l'original (`getChargeStartAvailableThresholds` retourne `0..=100`).
//! - Une écriture identique à la valeur courante est ignorée : l'original le
//!   faisait aussi pour les profils/priorités, mais écrivait systématiquement
//!   les seuils. Éviter l'écriture inutile réduit l'usure et le bruit.

use std::fmt;

use crate::error::{HwError, Result};
use crate::power_supply::PowerSupplies;
use crate::sysfs::SysfsRoot;

/// Base des attributs TUXEDO du module `tuxedo_keyboard`.
pub const TUXEDO_KEYBOARD_BASE: &str = "devices/platform/tuxedo_keyboard";
/// Sous-répertoire du profil de charge TUXEDO.
pub const CHARGING_PROFILE_BASE: &str = "devices/platform/tuxedo_keyboard/charging_profile";
/// Sous-répertoire de la priorité de charge TUXEDO.
pub const CHARGING_PRIORITY_BASE: &str = "devices/platform/tuxedo_keyboard/charging_priority";
/// Classe des alimentations.
const CLASS_POWER_SUPPLY: &str = "class/power_supply";

/// Longueur maximale acceptée pour un descripteur de charge.
const MAX_DESCRIPTOR_LEN: usize = 64;

/// Type de charge standard (`charge_type` du noyau).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeType {
    /// Inconnu (`Unknown`).
    Unknown,
    /// Non disponible (`N/A`).
    NotAvailable,
    /// Charge lente de maintien (`Trickle`).
    Trickle,
    /// Charge rapide (`Fast`).
    Fast,
    /// Charge standard (`Standard`).
    Standard,
    /// Charge adaptative (`Adaptive`).
    Adaptive,
    /// Seuils personnalisés (`Custom`).
    Custom,
    /// Durée de vie prolongée (`LongLife`).
    LongLife,
    /// Alimentation directe sans charge (`Bypass`).
    Bypass,
    /// Valeur non reconnue, conservée telle quelle.
    Other(String),
}

impl ChargeType {
    /// Convertit une valeur `charge_type` du noyau.
    #[must_use]
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim() {
            "Unknown" => Self::Unknown,
            "N/A" => Self::NotAvailable,
            "Trickle" => Self::Trickle,
            "Fast" => Self::Fast,
            "Standard" => Self::Standard,
            "Adaptive" => Self::Adaptive,
            "Custom" => Self::Custom,
            "LongLife" => Self::LongLife,
            "Bypass" => Self::Bypass,
            "" => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Représentation sysfs.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown => "Unknown",
            Self::NotAvailable => "N/A",
            Self::Trickle => "Trickle",
            Self::Fast => "Fast",
            Self::Standard => "Standard",
            Self::Adaptive => "Adaptive",
            Self::Custom => "Custom",
            Self::LongLife => "LongLife",
            Self::Bypass => "Bypass",
            Self::Other(value) => value,
        }
    }

    /// Vrai si la valeur est écrite telle quelle par le noyau.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl fmt::Display for ChargeType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// État complet de la charge, prêt à être exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChargingCapabilities {
    /// Un profil TUXEDO est exposé et lisible.
    pub profile_available: bool,
    /// Profil TUXEDO courant.
    pub profile: Option<String>,
    /// Profils TUXEDO disponibles (ordre du noyau).
    pub profiles: Vec<String>,
    /// Une priorité TUXEDO est exposée et lisible.
    pub priority_available: bool,
    /// Priorité TUXEDO courante.
    pub priority: Option<String>,
    /// Priorités TUXEDO disponibles.
    pub priorities: Vec<String>,
    /// Nom de la première batterie trouvée.
    pub battery: Option<String>,
    /// Seuil de début courant, en pourcentage.
    pub start_threshold: Option<u8>,
    /// Seuil de fin courant, en pourcentage.
    pub end_threshold: Option<u8>,
    /// Liste des seuils de début acceptés (vide si non exposée).
    pub start_thresholds: Vec<u8>,
    /// Liste des seuils de fin acceptés (vide si non exposée).
    pub end_thresholds: Vec<u8>,
    /// Type de charge courant, si exposé.
    pub charge_type: Option<String>,
    /// Vrai si les seuils sont pilotables (au moins de début ou de fin).
    pub thresholds_available: bool,
}

impl ChargingCapabilities {
    /// Vrai si aucun réglage de charge n'est exposé (machine non-TUXEDO,
    /// batterie virtuelle, ...).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
            && self.priorities.is_empty()
            && !self.thresholds_available
            && self.charge_type.is_none()
    }
}

/// Contrôleur de charge adossé à une racine sysfs.
#[derive(Debug, Clone)]
pub struct ChargingController {
    root: SysfsRoot,
}

impl ChargingController {
    /// Crée un contrôleur. Aucun accès disque n'est fait ici.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }

    /// Racine sysfs utilisée.
    #[must_use]
    pub fn root(&self) -> &SysfsRoot {
        &self.root
    }

    /// Lit les capacités et les valeurs courantes.
    ///
    /// Un attribut absent produit un champ `None` ou une liste vide, jamais une
    /// erreur : c'est le cas normal sur une machine sans module TUXEDO.
    ///
    /// # Errors
    ///
    /// Seules les erreurs d'E/S réelles (permissions, I/O noyau) sont remontées.
    pub fn capabilities(&self) -> Result<ChargingCapabilities> {
        let profiles = self.read_list(format!(
            "{CHARGING_PROFILE_BASE}/charging_profiles_available"
        ))?;
        let profile = self
            .root
            .read_optional_string(format!("{CHARGING_PROFILE_BASE}/charging_profile"))?
            .filter(|value| !value.is_empty());
        let priorities =
            self.read_list(format!("{CHARGING_PRIORITY_BASE}/charging_prios_available"))?;
        let priority = self
            .root
            .read_optional_string(format!("{CHARGING_PRIORITY_BASE}/charging_prio"))?
            .filter(|value| !value.is_empty());

        let battery = self.battery_name()?;
        let mut capabilities = ChargingCapabilities {
            profile_available: profile.is_some() && !profiles.is_empty(),
            profile,
            profiles,
            priority_available: priority.is_some() && !priorities.is_empty(),
            priority,
            priorities,
            battery: battery.clone(),
            ..ChargingCapabilities::default()
        };

        let Some(battery) = battery else {
            return Ok(capabilities);
        };
        let base = format!("{CLASS_POWER_SUPPLY}/{battery}");
        capabilities.start_threshold =
            self.read_percent(format!("{base}/charge_control_start_threshold"))?;
        capabilities.end_threshold =
            self.read_percent(format!("{base}/charge_control_end_threshold"))?;
        capabilities.start_thresholds =
            self.read_percent_list(format!("{base}/charge_control_start_available_thresholds"))?;
        capabilities.end_thresholds =
            self.read_percent_list(format!("{base}/charge_control_end_available_thresholds"))?;
        capabilities.charge_type = self
            .root
            .read_optional_string(format!("{base}/charge_type"))?
            .filter(|value| !value.is_empty());
        capabilities.thresholds_available =
            capabilities.start_threshold.is_some() || capabilities.end_threshold.is_some();
        Ok(capabilities)
    }

    /// Applique un profil de charge TUXEDO.
    ///
    /// La valeur doit appartenir à `charging_profiles_available`. Une valeur
    /// identique à la valeur courante n'est pas réécrite.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si les attributs TUXEDO sont absents ;
    /// - [`HwError::InvalidValue`] si le descripteur est invalide ou inconnu ;
    /// - [`HwError::Io`] en cas d'échec d'écriture.
    pub fn set_profile(&self, value: &str) -> Result<()> {
        self.set_choice(
            value,
            format!("{CHARGING_PROFILE_BASE}/charging_profiles_available"),
            format!("{CHARGING_PROFILE_BASE}/charging_profile"),
            "profil de charge",
        )
    }

    /// Applique une priorité de charge TUXEDO.
    ///
    /// # Errors
    ///
    /// Voir [`ChargingController::set_profile`].
    pub fn set_priority(&self, value: &str) -> Result<()> {
        self.set_choice(
            value,
            format!("{CHARGING_PRIORITY_BASE}/charging_prios_available"),
            format!("{CHARGING_PRIORITY_BASE}/charging_prio"),
            "priorité de charge",
        )
    }

    /// Applique les seuils de début et de fin.
    ///
    /// Ordre reproduit de l'interface d'origine : `charge_type = Custom` (si
    /// l'attribut existe), puis seuil de début, puis seuil de fin. Les valeurs
    /// sont validées **avant** la première écriture : si l'une des deux est
    /// refusée, rien n'est modifié.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si aucun seuil n'est exposé ;
    /// - [`HwError::OutOfRange`] si une valeur sort de `0..=100` ;
    /// - [`HwError::InvalidValue`] si `start >= end` ou si une valeur n'est pas
    ///   dans la liste disponible.
    pub fn set_thresholds(&self, start: u8, end: u8) -> Result<()> {
        let capabilities = self.capabilities()?;
        if !capabilities.thresholds_available {
            return Err(HwError::NotSupported("seuils de charge"));
        }
        validate_threshold(start, "seuil de charge (début)")?;
        validate_threshold(end, "seuil de charge (fin)")?;
        if start >= end {
            return Err(HwError::InvalidValue {
                context: "seuils de charge (début >= fin)",
                value: format!("{start} >= {end}"),
            });
        }
        if !capabilities.start_thresholds.is_empty()
            && !capabilities.start_thresholds.contains(&start)
        {
            return Err(HwError::InvalidValue {
                context: "seuil de charge (début non disponible)",
                value: start.to_string(),
            });
        }
        if !capabilities.end_thresholds.is_empty() && !capabilities.end_thresholds.contains(&end) {
            return Err(HwError::InvalidValue {
                context: "seuil de charge (fin non disponible)",
                value: end.to_string(),
            });
        }

        let Some(battery) = capabilities.battery.as_deref() else {
            return Err(HwError::NotSupported("batterie"));
        };
        let base = format!("{CLASS_POWER_SUPPLY}/{battery}");

        // `charge_type=Custom` est un prérequis du pilote pour honorer les
        // seuils. L'original le faisait depuis l'UI avant d'écrire.
        if capabilities.charge_type.is_some() {
            self.set_charge_type(&ChargeType::Custom)?;
        }
        self.write_threshold(
            &format!("{base}/charge_control_start_threshold"),
            start,
            capabilities.start_threshold,
        )?;
        self.write_threshold(
            &format!("{base}/charge_control_end_threshold"),
            end,
            capabilities.end_threshold,
        )?;
        Ok(())
    }

    /// Écrit un seuil unique après validation contre sa liste disponible.
    ///
    /// Utilisé par le daemon pour un setter D-Bus unitaire. Pour appliquer les
    /// deux seuils de façon cohérente, préférer [`ChargingController::set_thresholds`].
    ///
    /// # Errors
    ///
    /// Voir [`ChargingController::set_thresholds`].
    pub fn set_start_threshold(&self, value: u8) -> Result<()> {
        let capabilities = self.capabilities()?;
        self.apply_single_threshold(value, &capabilities, true)
    }

    /// Écrit le seuil de fin après validation.
    ///
    /// # Errors
    ///
    /// Voir [`ChargingController::set_thresholds`].
    pub fn set_end_threshold(&self, value: u8) -> Result<()> {
        let capabilities = self.capabilities()?;
        self.apply_single_threshold(value, &capabilities, false)
    }

    /// Écrit `charge_type`.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si l'attribut est absent ;
    /// - [`HwError::InvalidValue`] si la valeur n'appartient pas à l'ABI
    ///   `sysfs-class-power`.
    pub fn set_charge_type(&self, charge_type: &ChargeType) -> Result<()> {
        if !charge_type.is_known() {
            return Err(HwError::InvalidValue {
                context: "type de charge inconnu",
                value: charge_type.as_str().to_owned(),
            });
        }
        let Some(battery) = self.battery_name()? else {
            return Err(HwError::NotSupported("batterie"));
        };
        let path = format!("{CLASS_POWER_SUPPLY}/{battery}/charge_type");
        let current = self.root.read_optional_string(&path)?;
        let Some(current) = current else {
            return Err(HwError::NotSupported("type de charge"));
        };
        if current.trim() == charge_type.as_str() {
            return Ok(());
        }
        self.root.write_string(&path, charge_type.as_str())
    }

    /// Nom de la première batterie exposée, si elle existe.
    ///
    /// # Errors
    ///
    /// Erreur d'E/S réelle lors de l'énumération de
    /// `/sys/class/power_supply`.
    pub fn battery_name(&self) -> Result<Option<String>> {
        Ok(PowerSupplies::read(&self.root)?
            .first_battery()
            .map(|battery| battery.name.clone()))
    }

    fn apply_single_threshold(
        &self,
        value: u8,
        capabilities: &ChargingCapabilities,
        is_start: bool,
    ) -> Result<()> {
        let (available, current, path_suffix, start) = if is_start {
            (
                &capabilities.start_thresholds,
                capabilities.start_threshold,
                "charge_control_start_threshold",
                "seuil de charge (début)",
            )
        } else {
            (
                &capabilities.end_thresholds,
                capabilities.end_threshold,
                "charge_control_end_threshold",
                "seuil de charge (fin)",
            )
        };
        if current.is_none() {
            return Err(HwError::NotSupported(start));
        }
        validate_threshold(value, start)?;
        if !available.is_empty() && !available.contains(&value) {
            return Err(HwError::InvalidValue {
                context: start,
                value: value.to_string(),
            });
        }
        // Cohérence des deux seuils : on relit l'autre valeur pour refuser une
        // configuration incohérente avant d'écrire.
        let other = if is_start {
            capabilities.end_threshold
        } else {
            capabilities.start_threshold
        };
        if let Some(other) = other {
            let (new_start, new_end) = if is_start {
                (value, other)
            } else {
                (other, value)
            };
            if new_start >= new_end {
                return Err(HwError::InvalidValue {
                    context: "seuils de charge (début >= fin)",
                    value: format!("{new_start} >= {new_end}"),
                });
            }
        }
        if let Some(battery) = capabilities.battery.as_deref() {
            if capabilities.charge_type.is_some() {
                self.set_charge_type(&ChargeType::Custom)?;
            }
            let path = format!("{CLASS_POWER_SUPPLY}/{battery}/{path_suffix}");
            return self.write_threshold(&path, value, current);
        }
        Err(HwError::NotSupported("batterie"))
    }

    fn set_choice(
        &self,
        value: &str,
        available_path: String,
        value_path: String,
        context: &'static str,
    ) -> Result<()> {
        validate_descriptor(value, context)?;
        let available = self.read_list(available_path)?;
        if available.is_empty() {
            return Err(HwError::NotSupported(context));
        }
        if !available.iter().any(|candidate| candidate == value) {
            return Err(HwError::InvalidValue {
                context,
                value: value.to_owned(),
            });
        }
        let current = self
            .root
            .read_optional_string(&value_path)?
            .filter(|candidate| !candidate.is_empty());
        let Some(current) = current else {
            return Err(HwError::NotSupported(context));
        };
        if current == value {
            return Ok(());
        }
        self.root.write_string(&value_path, value)
    }

    fn write_threshold(&self, path: &str, value: u8, current: Option<u8>) -> Result<()> {
        if current == Some(value) {
            return Ok(());
        }
        self.root.write_i64(path, i64::from(value))
    }

    fn read_list(&self, path: String) -> Result<Vec<String>> {
        let Some(raw) = self.root.read_optional_string(path)? else {
            return Ok(Vec::new());
        };
        Ok(raw
            .split_whitespace()
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn read_percent(&self, path: String) -> Result<Option<u8>> {
        let Some(raw) = self.root.read_optional_string(path)? else {
            return Ok(None);
        };
        // Un firmware exotique peut exposer `-1` pour « non supporté ». On ne
        // fait pas échouer tout l'état de charge pour un attribut illisible :
        // la valeur devient `None` et l'UI l'affiche comme indisponible.
        let Ok(value) = raw.parse::<i64>() else {
            tracing::debug!(value = %raw, "seuil de charge non numérique, ignoré");
            return Ok(None);
        };
        if !(0..=100).contains(&value) {
            tracing::debug!(value, "seuil de charge hors plage, ignoré");
            return Ok(None);
        }
        Ok(Some(value as u8))
    }

    fn read_percent_list(&self, path: String) -> Result<Vec<u8>> {
        let Some(raw) = self.root.read_optional_string(path)? else {
            return Ok(Vec::new());
        };
        let mut values = Vec::new();
        for token in raw.split_whitespace() {
            let value = token.parse::<i64>().map_err(|_| HwError::InvalidValue {
                context: "seuils de charge disponibles",
                value: token.to_owned(),
            })?;
            if (0..=100).contains(&value) {
                values.push(value as u8);
            }
        }
        values.sort_unstable();
        values.dedup();
        Ok(values)
    }
}

fn validate_threshold(value: u8, context: &'static str) -> Result<()> {
    if value > 100 {
        return Err(HwError::OutOfRange {
            context,
            value: i64::from(value),
            min: 0,
            max: 100,
        });
    }
    Ok(())
}

/// Valide un descripteur de charge avant écriture sysfs.
///
/// Les listes `*_available_*` du noyau sont la source de vérité ; cette
/// validation est une seconde barrière contre les entrées D-Bus arbitraires.
fn validate_descriptor(value: &str, context: &'static str) -> Result<()> {
    if value.is_empty() {
        return Err(HwError::InvalidValue {
            context,
            value: String::new(),
        });
    }
    if value.chars().count() > MAX_DESCRIPTOR_LEN {
        return Err(HwError::InvalidValue {
            context,
            value: format!("<{} caractères>", value.chars().count()),
        });
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_graphic() && character != '\\')
    {
        return Err(HwError::InvalidValue {
            context,
            value: value.escape_default().to_string(),
        });
    }
    Ok(())
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

    fn tuxedo_fixture(root: &Path) {
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profiles_available",
            "high_capacity balanced stationary\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profile",
            "balanced\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prios_available",
            "charge_battery performance\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prio",
            "performance\n",
        );
    }

    fn battery_fixture(root: &Path) {
        write(root, "class/power_supply/AC/type", "Mains\n");
        write(root, "class/power_supply/AC/online", "1\n");
        write(root, "class/power_supply/BAT0/type", "Battery\n");
        write(
            root,
            "class/power_supply/BAT0/charge_control_start_available_thresholds",
            "40 50 60\n",
        );
        write(
            root,
            "class/power_supply/BAT0/charge_control_end_available_thresholds",
            "80 85 90 100\n",
        );
        write(
            root,
            "class/power_supply/BAT0/charge_control_start_threshold",
            "60\n",
        );
        write(
            root,
            "class/power_supply/BAT0/charge_control_end_threshold",
            "90\n",
        );
        write(root, "class/power_supply/BAT0/charge_type", "Standard\n");
    }

    fn controller(root: &Path) -> ChargingController {
        ChargingController::new(SysfsRoot::new(root))
    }

    #[test]
    fn reads_tuxedo_profiles_and_battery() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        battery_fixture(dir.path());
        let capabilities = controller(dir.path()).capabilities().unwrap();

        assert!(capabilities.profile_available);
        assert_eq!(capabilities.profile.as_deref(), Some("balanced"));
        assert_eq!(
            capabilities.profiles,
            vec!["high_capacity", "balanced", "stationary"]
        );
        assert!(capabilities.priority_available);
        assert_eq!(capabilities.priority.as_deref(), Some("performance"));
        assert_eq!(capabilities.battery.as_deref(), Some("BAT0"));
        assert_eq!(capabilities.start_threshold, Some(60));
        assert_eq!(capabilities.end_threshold, Some(90));
        assert_eq!(capabilities.start_thresholds, vec![40, 50, 60]);
        assert_eq!(capabilities.end_thresholds, vec![80, 85, 90, 100]);
        assert_eq!(capabilities.charge_type.as_deref(), Some("Standard"));
        assert!(capabilities.thresholds_available);
        assert!(!capabilities.is_empty());
    }

    #[test]
    fn missing_hardware_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let capabilities = controller(dir.path()).capabilities().unwrap();
        assert!(!capabilities.profile_available);
        assert!(capabilities.profiles.is_empty());
        assert!(capabilities.is_empty());
        assert!(matches!(
            controller(dir.path()).set_profile("balanced"),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            controller(dir.path()).set_thresholds(50, 90),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn applies_profile_only_when_available_and_different() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        let controller = controller(dir.path());

        controller.set_profile("stationary").unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "stationary"
        );

        // Valeur identique : pas de nouvelle écriture (le fichier garde la
        // valeur, ce qui suffit à prouver l'idempotence fonctionnelle).
        controller.set_profile("stationary").unwrap();

        let error = controller.set_profile("turbo").unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "stationary"
        );
    }

    #[test]
    fn rejects_malformed_descriptor_before_io() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        let controller = controller(dir.path());
        let error = controller.set_priority("bad\nvalue").unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        let error = controller.set_priority(&"a".repeat(65)).unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
    }

    #[test]
    fn applies_priority() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        let controller = controller(dir.path());
        controller.set_priority("charge_battery").unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_priority/charging_prio"
            ),
            "charge_battery"
        );
    }

    #[test]
    fn thresholds_are_validated_and_set_custom_first() {
        let dir = tempfile::tempdir().unwrap();
        battery_fixture(dir.path());
        let controller = controller(dir.path());

        // Seuil non disponible dans la liste.
        let error = controller.set_thresholds(55, 90).unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Standard",
            "aucune écriture ne doit avoir lieu après un refus"
        );

        // Début >= fin.
        let error = controller.set_thresholds(60, 60).unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));

        controller.set_thresholds(50, 85).unwrap();
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Custom"
        );
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_start_threshold"
            ),
            "50"
        );
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_end_threshold"
            ),
            "85"
        );
    }

    #[test]
    fn thresholds_fall_back_to_range_when_lists_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_control_start_threshold",
            "60\n",
        );
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_control_end_threshold",
            "90\n",
        );
        let controller = controller(dir.path());
        controller.set_thresholds(40, 80).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_start_threshold"
            ),
            "40"
        );
        // Pas de charge_type : aucun prérequis à écrire.
        assert!(
            !dir.path()
                .join("class/power_supply/BAT0/charge_type")
                .exists()
        );
    }

    #[test]
    fn single_threshold_keeps_pair_coherent() {
        let dir = tempfile::tempdir().unwrap();
        battery_fixture(dir.path());
        let controller = controller(dir.path());

        let error = controller.set_end_threshold(50).unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        let error = controller.set_start_threshold(90).unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));

        controller.set_start_threshold(40).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_start_threshold"
            ),
            "40"
        );
        controller.set_end_threshold(100).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_end_threshold"
            ),
            "100"
        );
    }

    #[test]
    fn charge_type_is_validated_against_kernel_abi() {
        let dir = tempfile::tempdir().unwrap();
        battery_fixture(dir.path());
        let controller = controller(dir.path());

        controller.set_charge_type(&ChargeType::Custom).unwrap();
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Custom"
        );
        let error = controller
            .set_charge_type(&ChargeType::Other("Bogus".to_owned()))
            .unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Custom"
        );
    }

    #[test]
    fn charge_type_parsing_is_stable() {
        assert_eq!(ChargeType::from_sysfs("N/A"), ChargeType::NotAvailable);
        assert_eq!(ChargeType::from_sysfs("Custom\n"), ChargeType::Custom);
        assert_eq!(
            ChargeType::from_sysfs("Future"),
            ChargeType::Other("Future".to_owned())
        );
        assert!(!ChargeType::from_sysfs("Future").is_known());
        assert_eq!(ChargeType::LongLife.as_str(), "LongLife");
    }

    #[test]
    fn out_of_range_threshold_value_is_ignored_on_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/power_supply/BAT0/type", "Battery\n");
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_control_start_threshold",
            "150\n",
        );
        let capabilities = controller(dir.path()).capabilities().unwrap();
        // Un attribut illisible ne doit pas faire tomber tout le domaine.
        assert_eq!(capabilities.start_threshold, None);
    }
}
