// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! Logique pure de TuxedoRustCenter.
//!
//! `tcc-core` ne dépend d'aucun matériel, d'aucun bus et d'aucun toolkit
//! graphique. Il contient les modèles partagés, la validation, la migration,
//! la persistance JSON et le moteur de bascule `(PPD, alimentation)`. Cette
//! séparation permet de tester toute la logique sans machine TUXEDO et sans
//! privilège.
//!
//! # Modules
//!
//! - [`profile`] : profils matériels et compatibilité des clés originales ;
//! - [`presets`] : presets TCC et profil personnalisé par défaut ;
//! - [`settings`] : réglages globaux, matrice d'état, clavier/lightbar ;
//! - [`state`] : `(PPD, alimentation)`, debounce, override manuel ;
//! - [`fan`] : courbes de ventilation pures ;
//! - [`validate`] : validation centralisée et bornes de sécurité ;
//! - [`config`] : lecture/écriture atomique et durcie des fichiers ;
//! - [`migration`] : conversion depuis `/etc/tcc` ;
//! - [`error`] : erreurs typées.
//!
//! # Garanties
//!
//! - aucune fonction publique ne panique sur une entrée invalide ;
//! - aucune valeur non finie (`NaN`, infini) n'est acceptée par la validation ;
//! - les fichiers de configuration sont lus avec `O_NOFOLLOW` et écrits de
//!   façon atomique ;
//! - les champs JSON inconnus sont conservés lors d'un aller-retour ;
//! - le code de production n'utilise ni `unwrap` ni `expect`.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod error;
pub mod fan;
pub mod migration;
pub mod presets;
pub mod profile;
pub mod settings;
pub mod state;
pub mod validate;

pub use config::{
    ADMIN_CONFIG_DIR, ConfigPaths, ConfigStore, DEFAULT_MAX_CONFIG_BYTES, FAN_TABLES_FILE,
    LoadPolicy, LoadWarning, LoadWarningKind, Loaded, PROFILES_FILE, SETTINGS_FILE, STATE_DIR,
    Saved, WEBCAM_FILE,
};
pub use error::{CoreError, CoreResult, StrictLoadError, ValidationError, ValidationIssue};
pub use fan::{
    FAN_OFFSET_MAX_PERCENT, FAN_OFFSET_MIN_PERCENT, FAN_SPEED_MAX_PERCENT, FAN_SPEED_MIN_PERCENT,
    FanCurve, FanCurveEvaluator, FanLimits, FanTableEntry, FanTableSet, FanTarget,
    MAX_FAN_TABLE_ENTRIES,
};
pub use migration::{
    LEGACY_CONFIG_DIR, LEGACY_FAN_TABLES_FILE, LEGACY_PROFILES_FILE, LEGACY_SETTINGS_FILE,
    LegacyConfig, LegacyPaths, Migration, migrate_fan_tables, migrate_profiles, migrate_settings,
    read_legacy_config,
};
pub use presets::{
    DEFAULT_PROFILE_IDS, PRESET_PROFILE_IDS, default_custom_profile, default_profiles,
    high_performance_profile, max_energy_save_profile, merge_with_default_profiles, office_profile,
    quiet_profile,
};
pub use profile::{
    CpuProfile, DEFAULT_CUSTOM_PROFILE_ID, DisplayProfile, FanProfile, FrequencyLimit,
    HIGH_PERFORMANCE_PROFILE_ID, MAX_ENERGY_SAVE_PROFILE_ID, NvidiaPowerCtrlProfile,
    OFFICE_PROFILE_ID, OdmPowerLimits, OdmProfile, Profile, ProfileCollection, ProfileId,
    QUIET_PROFILE_ID, WebcamProfile,
};
pub use settings::{
    KeyboardColorMode, KeyboardState, LightbarState, SETTINGS_SCHEMA_VERSION, Settings,
};
pub use state::{
    DEFAULT_DEBOUNCE, LegacyStateMap, ManualOverride, ObserveOutcome, PowerSource, PpdProfile,
    ResolutionSource, ResolvedState, StateInput, StateKey, StateMatrix, StateResolver,
};
pub use validate::{
    MAX_CHARGING_TYPE_LEN, MAX_CPU_FREQUENCY_KHZ, MAX_CPU_ONLINE_CORES, MAX_CTGP_OFFSET_WATTS,
    MAX_DELETED_PROFILE_IDS, MAX_DESCRIPTION_LEN, MAX_FAN_CURVES, MAX_FAN_TEMP_C,
    MAX_KEYBOARD_STATES, MAX_PROFILE_ID_LEN, MAX_PROFILE_NAME_LEN, MAX_PROFILES,
    MAX_REFRESH_RATE_HZ, MAX_RESOLUTION, MAX_TDP_VALUES, MAX_TDP_WATTS, MAX_YCBCR_CARDS,
    MAX_YCBCR_PORTS, MIN_CPU_FREQUENCY_KHZ, MIN_FAN_TEMP_C, MIN_REFRESH_RATE_HZ, MIN_TDP_WATTS,
    validate_fan_curve, validate_fan_tables, validate_profile, validate_profile_description,
    validate_profile_id, validate_profile_name, validate_profiles, validate_settings,
    validate_settings_references,
};

/// Version du crate, issue de `Cargo.toml`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_not_empty() {
        assert!(!super::version().is_empty());
    }
}
