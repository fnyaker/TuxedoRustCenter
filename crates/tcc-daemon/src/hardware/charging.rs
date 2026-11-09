// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker de charge de la batterie.
//!
//! Reprend `ChargingWorker` de l'original :
//!
//! - applique le profil et la priorité TUXEDO au démarrage et à chaque
//!   changement de réglages ;
//! - expose les listes disponibles et les seuils pour l'UI ;
//! - ne réécrit **pas** les valeurs à chaque tick périodique : un tick ne
//!   rafraîchit que la télémétrie. Le firmware et l'utilisateur restent
//!   maîtres entre deux changements de configuration (contrairement à une
//!   réconciliation permanente qui écraserait un réglage fait via sysfs).
//!
//! Différence assumée : les seuils persistés sont réappliqués au démarrage
//! **uniquement** lorsque `charge_type == Custom`, c'est-à-dire quand
//! l'utilisateur a explicitement activé les seuils. Cela restaure l'intention
//! après une coupure de courant sans réactiver un mode que l'utilisateur aurait
//! désactivé.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::charging::{ChargeType, ChargingCapabilities, ChargingController};
use tcc_hw::error::HwError;
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de rafraîchissement de la télémétrie.
pub const CHARGING_TICK: Duration = Duration::from_secs(10);

/// Instantané de charge exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChargingData {
    /// Un réglage de charge est exposé.
    pub available: bool,
    /// Au moins un réglage est pilotable par TCC (hors `dry_run`).
    pub write_supported: bool,
    /// Le profil TUXEDO peut être écrit.
    pub profile_writable: bool,
    /// La priorité TUXEDO peut être écrite.
    pub priority_writable: bool,
    /// Les seuils peuvent être écrits.
    pub thresholds_writable: bool,
    /// `charge_type` est exposé par le noyau.
    pub charge_type_available: bool,
    /// `Settings.chargingType` peut être appliqué (attribut présent, hors
    /// `dry_run`).
    pub charge_type_writable: bool,
    /// Types de charge annoncés par le backend. Vide : l'ABI noyau expose le
    /// type courant, pas une liste ; ne rien inventer côté UI.
    pub charge_types: Vec<String>,
    /// Profils TUXEDO disponibles.
    pub profiles: Vec<String>,
    /// Profil courant (matériel).
    pub profile: Option<String>,
    /// Priorités TUXEDO disponibles.
    pub priorities: Vec<String>,
    /// Priorité courante (matériel).
    pub priority: Option<String>,
    /// Batterie utilisée pour les seuils.
    pub battery: Option<String>,
    /// Seuil de début courant.
    pub start_threshold: Option<u8>,
    /// Seuil de fin courant.
    pub end_threshold: Option<u8>,
    /// Seuils de début acceptés.
    pub start_thresholds: Vec<u8>,
    /// Seuils de fin acceptés.
    pub end_thresholds: Vec<u8>,
    /// Type de charge courant.
    pub charge_type: Option<String>,
    /// Vrai si les seuils sont pilotables.
    pub thresholds_available: bool,
    /// Vrai si la dernière réconciliation a réussi.
    pub applied: bool,
    /// Erreurs de la dernière réconciliation (bornées).
    pub errors: Vec<String>,
}

impl ChargingData {
    fn from_capabilities(capabilities: &ChargingCapabilities, writable: bool) -> Self {
        let profile_writable = writable && capabilities.profile_available;
        let priority_writable = writable && capabilities.priority_available;
        let thresholds_writable = writable && capabilities.thresholds_available;
        let charge_type_available = capabilities.charge_type.is_some();
        let charge_type_writable = writable && charge_type_available;
        Self {
            available: !capabilities.is_empty(),
            write_supported: profile_writable
                || priority_writable
                || thresholds_writable
                || charge_type_writable,
            profile_writable,
            priority_writable,
            thresholds_writable,
            charge_type_available,
            charge_type_writable,
            charge_types: Vec::new(),
            profiles: capabilities.profiles.clone(),
            profile: capabilities.profile.clone(),
            priorities: capabilities.priorities.clone(),
            priority: capabilities.priority.clone(),
            battery: capabilities.battery.clone(),
            start_threshold: capabilities.start_threshold,
            end_threshold: capabilities.end_threshold,
            start_thresholds: capabilities.start_thresholds.clone(),
            end_thresholds: capabilities.end_thresholds.clone(),
            charge_type: capabilities.charge_type.clone(),
            thresholds_available: capabilities.thresholds_available,
            applied: true,
            errors: Vec::new(),
        }
    }
}

/// Exécution d'un tick de charge.
#[derive(Debug)]
pub struct ChargingRuntime {
    controller: ChargingController,
    dry_run: bool,
    last_revision: Option<u64>,
    pending: bool,
    last_error_logged: bool,
    apply_error_logged: bool,
    charge_type_warned: Option<String>,
}

impl ChargingRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            controller: ChargingController::new(root),
            dry_run,
            last_revision: None,
            pending: true,
            last_error_logged: false,
            apply_error_logged: false,
            charge_type_warned: None,
        }
    }

    /// Exécute un tick : rafraîchit l'état et applique les réglages si la
    /// révision a changé (ou si une écriture précédente a échoué).
    pub fn tick(&mut self, input: &HardwareInputs) -> ChargingData {
        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.pending = true;
            self.last_revision = Some(input.revision);
            self.apply_error_logged = false;
        }

        let capabilities = match self.controller.capabilities() {
            Ok(capabilities) => capabilities,
            Err(error) => {
                if !self.last_error_logged {
                    self.last_error_logged = true;
                    tracing::warn!(%error, "état de charge illisible");
                }
                return ChargingData {
                    applied: false,
                    errors: vec![format!("lecture: {error}")],
                    ..ChargingData::default()
                };
            }
        };
        self.last_error_logged = false;
        let writable = !self.dry_run;
        let mut data = ChargingData::from_capabilities(&capabilities, writable);

        let applied_now = self.pending;
        if applied_now {
            let mut errors = Vec::new();
            self.apply(&capabilities, input, &mut errors);
            if errors.is_empty() {
                self.apply_error_logged = false;
            } else if !self.apply_error_logged {
                self.apply_error_logged = true;
                tracing::warn!(errors = ?errors, "application de la charge partielle");
            }
            data.errors = errors.clone();
            data.applied = errors.is_empty();
            // On réessaie au prochain tick uniquement en cas d'échec : un
            // succès reste stable jusqu'au prochain changement de révision.
            self.pending = !errors.is_empty();
        } else {
            data.applied = true;
        }

        // Relit les valeurs après application pour que l'UI voie l'état réel.
        if applied_now && let Ok(refreshed) = self.controller.capabilities() {
            let mut refreshed = ChargingData::from_capabilities(&refreshed, writable);
            refreshed.errors = data.errors;
            refreshed.applied = data.applied;
            data = refreshed;
        }
        data
    }

    fn apply(
        &mut self,
        capabilities: &ChargingCapabilities,
        input: &HardwareInputs,
        errors: &mut Vec<String>,
    ) {
        let settings = &input.settings;
        if self.dry_run {
            return;
        }
        if let Some(profile) = settings.charging_profile.as_deref()
            && capabilities.profile_available
        {
            match self.controller.set_profile(profile) {
                Ok(()) => {}
                Err(error) if error.is_unavailable() => {
                    tracing::debug!(%error, "profil de charge non supporté");
                }
                Err(error) => errors.push(format!("profil '{profile}': {error}")),
            }
        }
        if let Some(priority) = settings.charging_priority.as_deref()
            && capabilities.priority_available
        {
            match self.controller.set_priority(priority) {
                Ok(()) => {}
                Err(error) if error.is_unavailable() => {
                    tracing::debug!(%error, "priorité de charge non supportée");
                }
                Err(error) => errors.push(format!("priorité '{priority}': {error}")),
            }
        }
        self.apply_thresholds(capabilities, settings, errors);
        self.apply_charge_type(capabilities, settings, errors);
    }

    /// Applique `settings.charging_type` si le backend expose l'attribut.
    ///
    /// L'ABI noyau n'expose pas de liste de types disponibles : seules les
    /// valeurs standard de [`ChargeType`] sont acceptées, puis le firmware
    /// décide. Un type inconnu ou un attribut absent ne produit **aucune
    /// écriture** ; l'avertissement n'est journalisé qu'une fois par valeur.
    fn apply_charge_type(
        &mut self,
        capabilities: &ChargingCapabilities,
        settings: &tcc_core::Settings,
        errors: &mut Vec<String>,
    ) {
        let Some(value) = settings.charging_type.as_deref() else {
            return;
        };
        let Some(charge_type) = writable_charge_type(value) else {
            self.warn_charge_type_once(value, "type non standard ou en lecture seule");
            errors.push(format!("type de charge '{value}': non supporté"));
            return;
        };
        if capabilities.charge_type.is_none() {
            self.warn_charge_type_once(value, "attribut charge_type absent");
            errors.push(format!("type de charge '{value}': attribut absent"));
            return;
        }
        match self.controller.set_charge_type(&charge_type) {
            Ok(()) => {
                self.charge_type_warned = None;
            }
            Err(HwError::NotSupported(_)) => {
                self.warn_charge_type_once(value, "type de charge non supporté");
                errors.push(format!("type de charge '{value}': non supporté"));
            }
            Err(HwError::InvalidValue { .. }) => {
                self.warn_charge_type_once(value, "valeur refusée par le noyau");
                errors.push(format!("type de charge '{value}': refusé par le noyau"));
            }
            Err(error) => {
                self.warn_charge_type_once(value, "écriture impossible");
                errors.push(format!("type de charge '{value}': {error}"));
            }
        }
    }

    /// Journalise un avertissement unique pour une valeur de type de charge.
    fn warn_charge_type_once(&mut self, value: &str, reason: &str) {
        if self.charge_type_warned.as_deref() == Some(value) {
            return;
        }
        self.charge_type_warned = Some(value.to_owned());
        tracing::warn!(value, reason, "type de charge non appliqué");
    }

    fn apply_thresholds(
        &self,
        capabilities: &ChargingCapabilities,
        settings: &tcc_core::Settings,
        errors: &mut Vec<String>,
    ) {
        let (Some(start), Some(end)) = (
            settings.charging_start_threshold,
            settings.charging_end_threshold,
        ) else {
            return;
        };
        if !capabilities.thresholds_available {
            return;
        }
        // Ne pas réactiver un mode que l'utilisateur a désactivé : les seuils
        // ne sont réappliqués que si `charge_type` est déjà `Custom`.
        let custom = capabilities
            .charge_type
            .as_deref()
            .is_some_and(|value| ChargeType::from_sysfs(value) == ChargeType::Custom);
        if !custom {
            return;
        }
        match self.controller.set_thresholds(start, end) {
            Ok(()) => {}
            Err(HwError::NotSupported(_)) => {}
            Err(error) => errors.push(format!("seuils {start}/{end}: {error}")),
        }
    }
}

/// Valide une valeur de type de charge contre l'ABI standard.
///
/// `Unknown`/`N/A` et les valeurs hors ABI ne sont jamais écrites : ce sont
/// des états de lecture, pas des choix utilisateur.
fn writable_charge_type(value: &str) -> Option<ChargeType> {
    match canonical_charge_type(value) {
        ChargeType::Unknown | ChargeType::NotAvailable | ChargeType::Other(_) => None,
        charge_type => Some(charge_type),
    }
}

/// Normalise la casse des valeurs connues (`fast` → `Fast`).
///
/// `Settings.chargingType` est une liste ouverte validée en forme par
/// `tcc-core` ; le noyau attend les noms canoniques (`Fast`, `Standard`, ...).
/// Les valeurs inconnues restent [`ChargeType::Other`] et sont refusées.
fn canonical_charge_type(value: &str) -> ChargeType {
    let trimmed = value.trim();
    match ChargeType::from_sysfs(trimmed) {
        ChargeType::Other(_) => match trimmed.to_ascii_lowercase().as_str() {
            "unknown" => ChargeType::Unknown,
            "n/a" => ChargeType::NotAvailable,
            "trickle" => ChargeType::Trickle,
            "fast" => ChargeType::Fast,
            "standard" => ChargeType::Standard,
            "adaptive" => ChargeType::Adaptive,
            "custom" => ChargeType::Custom,
            "longlife" => ChargeType::LongLife,
            "bypass" => ChargeType::Bypass,
            _ => ChargeType::Other(trimmed.to_owned()),
        },
        charge_type => charge_type,
    }
}

/// Démarre le worker de charge.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = ChargingRuntime::new(root, dry_run);
    spawn_worker(
        "charge",
        CHARGING_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_charging,
        ChargingRuntime::tick,
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

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn fixture(root: &Path) {
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profiles_available",
            "high_capacity balanced stationary\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profile",
            "high_capacity\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prios_available",
            "charge_battery performance\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prio",
            "charge_battery\n",
        );
        write(root, "class/power_supply/BAT0/type", "Battery\n");
        write(
            root,
            "class/power_supply/BAT0/charge_control_start_available_thresholds",
            "40 60\n",
        );
        write(
            root,
            "class/power_supply/BAT0/charge_control_end_available_thresholds",
            "80 90\n",
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
        write(root, "class/power_supply/BAT0/charge_type", "Custom\n");
    }

    fn settings() -> tcc_core::Settings {
        tcc_core::Settings {
            charging_profile: Some("stationary".to_owned()),
            charging_priority: Some("performance".to_owned()),
            charging_start_threshold: Some(40),
            charging_end_threshold: Some(80),
            ..tcc_core::Settings::default()
        }
    }

    fn input(revision: u64) -> HardwareInputs {
        HardwareInputs::new(settings(), None, false, revision)
    }

    #[test]
    fn applies_settings_on_first_tick_then_only_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));

        assert!(data.available);
        assert!(data.applied, "{data:?}");
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "stationary"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_priority/charging_prio"
            ),
            "performance"
        );
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_start_threshold"
            ),
            "40"
        );
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_end_threshold"
            ),
            "80"
        );

        // Un changement externe n'est pas écrasé tant que la révision ne change
        // pas.
        write(
            dir.path(),
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profile",
            "balanced\n",
        );
        let data = runtime.tick(&input(1));
        assert_eq!(data.profile.as_deref(), Some("balanced"));
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "balanced"
        );

        // Une nouvelle révision réapplique les réglages.
        runtime.tick(&input(2));
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "stationary"
        );
    }

    #[test]
    fn dry_run_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&input(1));
        assert!(data.applied);
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/charging_profile/charging_profile"
            ),
            "high_capacity"
        );
    }

    #[test]
    fn charge_type_is_applied_reflected_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut settings = settings();
        settings.charging_type = Some("fast".to_owned());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);

        let data = runtime.tick(&HardwareInputs::new(settings.clone(), None, false, 1));
        assert!(data.applied, "{data:?}");
        assert_eq!(data.charge_type.as_deref(), Some("Fast"));
        assert!(data.charge_type_available);
        assert!(data.charge_type_writable);
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Fast"
        );

        // Même révision : une modification externe n'est pas écrasée.
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_type",
            "Standard\n",
        );
        let data = runtime.tick(&HardwareInputs::new(settings.clone(), None, false, 1));
        assert_eq!(data.charge_type.as_deref(), Some("Standard"));
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Standard"
        );

        // Nouvelle révision : la valeur demandée est réappliquée.
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 2));
        assert_eq!(data.charge_type.as_deref(), Some("Fast"));
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Fast"
        );
    }

    #[test]
    fn invalid_charge_type_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut settings = settings();
        settings.charging_type = Some("Bogus".to_owned());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(!data.applied);
        assert_eq!(data.charge_type.as_deref(), Some("Custom"));
        assert!(
            data.errors
                .iter()
                .any(|error| error.contains("non supporté")),
            "{data:?}"
        );
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Custom"
        );
    }

    #[test]
    fn missing_charge_type_backend_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        fs::remove_file(dir.path().join("class/power_supply/BAT0/charge_type")).unwrap();
        let mut settings = settings();
        settings.charging_type = Some("fast".to_owned());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(!data.charge_type_available);
        assert!(!data.charge_type_writable);
        assert!(!data.applied);
        assert!(
            data.errors
                .iter()
                .any(|error| error.contains("attribut absent")),
            "{data:?}"
        );
        assert!(
            !dir.path()
                .join("class/power_supply/BAT0/charge_type")
                .exists()
        );
    }

    #[test]
    fn missing_hardware_is_reported_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(!data.available);
        assert!(data.applied);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn thresholds_are_not_reactivated_when_charge_type_is_standard() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(
            dir.path(),
            "class/power_supply/BAT0/charge_type",
            "Standard\n",
        );
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(data.applied);
        // Le profil est appliqué, mais pas les seuils.
        assert_eq!(
            read(dir.path(), "class/power_supply/BAT0/charge_type"),
            "Standard"
        );
        assert_eq!(
            read(
                dir.path(),
                "class/power_supply/BAT0/charge_control_start_threshold"
            ),
            "60"
        );
    }

    #[test]
    fn invalid_descriptor_is_reported_and_retried() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut settings = settings();
        settings.charging_profile = Some("does-not-exist".to_owned());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings.clone(), None, false, 1));
        assert!(!data.applied);
        assert_eq!(data.errors.len(), 1);
        assert!(runtime.pending, "un échec doit être réessayé");

        // La correction permet de converger au tick suivant sans changer de
        // révision (cas d'un module qui se charge tardivement).
        settings.charging_profile = Some("balanced".to_owned());
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(data.applied, "{data:?}");
        assert!(!runtime.pending);
    }

    #[test]
    fn capability_writability_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(data.write_supported);
        assert!(data.profile_writable);
        assert!(data.priority_writable);
        assert!(data.thresholds_writable);
        assert!(data.charge_type_available);
        assert!(data.charge_type_writable);
        assert!(data.charge_types.is_empty());

        let mut runtime = ChargingRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&input(1));
        assert!(!data.write_supported);
        assert!(!data.profile_writable);
        assert!(!data.priority_writable);
        assert!(!data.thresholds_writable);
        assert!(!data.charge_type_writable);
    }

    #[test]
    fn serializes_camel_case() {
        let data = ChargingData {
            available: true,
            write_supported: true,
            charge_type_writable: false,
            profile: Some("balanced".to_owned()),
            ..ChargingData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"writeSupported\":true"));
        assert!(json.contains("\"chargeTypeWritable\":false"));
        assert!(json.contains("\"profile\":\"balanced\""));
    }
}
