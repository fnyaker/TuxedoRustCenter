// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. La migration lit les formats
// de TUXEDO Control Center (GPL-3.0-or-later) et les convertit vers les
// modèles Rust. Aucun code source de l'implémentation Electron/Node.js n'a
// été copié.

//! Migration depuis les fichiers de l'original (`/etc/tcc/*`).
//!
//! La migration est séparée de la persistance : les fonctions de ce module
//! sont **pures** (JSON en entrée, modèles en sortie) et [`read_legacy_config`]
//! ne fait que lire les fichiers historiques. C'est `tcc-daemon` qui décide
//! d'écrire ou non le résultat dans le magasin d'état, après validation.
//!
//! # Ce qui est conservé
//!
//! - les clés connues de `settings`, `profiles` et `fantables` ;
//! - les champs inconnus, grâce à `serde(flatten)` dans [`Profile`] et
//!   [`Settings`] ;
//! - les identifiants existants, sauf s'ils sont absents ou dupliqués (un
//!   nouvel identifiant est alors généré pour garantir l'unicité) ;
//! - `refreshRate: -1`, converti en `None` par le désérialiseur de
//!   [`crate::profile::DisplayProfile`] ;
//! - `scalingMaxFrequency: -1`, converti en
//!   [`crate::profile::FrequencyLimit::Reduced`] ;
//! - l'ancien champ `keyboardBacklightStates`, renommé en `keyboardStates` ;
//!   s'il est illisible (mode inconnu), il est conservé tel quel dans les
//!   champs inconnus au lieu d'être perdu (BUG-QA-001).
//!
//! # Ce qui n'est pas migré ici
//!
//! `/etc/tcc/webcam` contient des préréglages V4L2 dont les types appartiennent
//! à `tcc-hw`. Le fichier est laissé intact ; l'agent webcam le migrera avec
//! ses propres modèles. De même, les identifiants de profils supprimés ne sont
//! pas réutilisés : un nouvel identifiant est toujours généré.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::DEFAULT_MAX_CONFIG_BYTES;
use crate::error::CoreError;
use crate::fan::{FanCurve, FanTableSet};
use crate::profile::{Profile, ProfileCollection, ProfileId};
use crate::settings::{KeyboardState, SETTINGS_SCHEMA_VERSION, Settings};

/// Répertoire historique de TCC.
pub const LEGACY_CONFIG_DIR: &str = "/etc/tcc";
/// Fichier historique des réglages.
pub const LEGACY_SETTINGS_FILE: &str = "settings";
/// Fichier historique des profils.
pub const LEGACY_PROFILES_FILE: &str = "profiles";
/// Fichier historique des courbes de ventilation.
pub const LEGACY_FAN_TABLES_FILE: &str = "fantables";

/// Chemins des fichiers historiques.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPaths {
    /// Répertoire historique.
    pub root: PathBuf,
    /// Fichier `settings`.
    pub settings: PathBuf,
    /// Fichier `profiles`.
    pub profiles: PathBuf,
    /// Fichier `fantables`.
    pub fan_tables: PathBuf,
}

impl LegacyPaths {
    /// Construit les chemins à partir d'un répertoire.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            settings: root.join(LEGACY_SETTINGS_FILE),
            profiles: root.join(LEGACY_PROFILES_FILE),
            fan_tables: root.join(LEGACY_FAN_TABLES_FILE),
            root,
        }
    }

    /// Chemins historiques système (`/etc/tcc`).
    #[must_use]
    pub fn default_system() -> Self {
        Self::new(LEGACY_CONFIG_DIR)
    }
}

impl Default for LegacyPaths {
    fn default() -> Self {
        Self::default_system()
    }
}

/// Résultat d'une migration : valeur convertie, changement et notes.
#[derive(Debug, Clone, PartialEq)]
pub struct Migration<T> {
    /// Valeur convertie.
    pub value: T,
    /// Vrai si la valeur diffère sémantiquement de l'entrée.
    pub changed: bool,
    /// Notes lisibles à journaliser (identifiants générés, champs renommés).
    pub notes: Vec<String>,
}

/// Configuration historique lue depuis un répertoire.
#[derive(Debug, Clone, Default)]
pub struct LegacyConfig {
    /// Réglages migrés, si le fichier existait.
    pub settings: Option<Migration<Settings>>,
    /// Profils migrés, si le fichier existait.
    pub profiles: Option<Migration<ProfileCollection>>,
    /// Courbes migrées, si le fichier existait.
    pub fan_tables: Option<Migration<FanTableSet>>,
    /// Notes globales (fichiers absents, conversions de masse).
    pub notes: Vec<String>,
}

impl LegacyConfig {
    /// Vrai si aucun fichier historique n'a été trouvé.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.settings.is_none() && self.profiles.is_none() && self.fan_tables.is_none()
    }
}

/// Lit et migre les fichiers d'un répertoire historique.
///
/// Les fichiers absents sont simplement ignorés. Un fichier présent mais
/// invalide remonte une erreur contextualisée : l'appelant peut conserver
/// l'ancien répertoire intact et démarrer avec les valeurs par défaut.
///
/// # Errors
///
/// Renvoie une erreur d'entrée/sortie, de taille (`> 1 Mio`), de JSON ou de
/// validation.
pub fn read_legacy_config(paths: &LegacyPaths) -> Result<LegacyConfig, CoreError> {
    let mut config = LegacyConfig::default();

    if paths.settings.exists() {
        let raw = read_legacy_file(&paths.settings)?;
        config.settings =
            Some(migrate_settings(&raw).map_err(|error| error.with_path(&paths.settings))?);
    }
    if paths.profiles.exists() {
        let raw = read_legacy_file(&paths.profiles)?;
        config.profiles =
            Some(migrate_profiles(&raw).map_err(|error| error.with_path(&paths.profiles))?);
    }
    if paths.fan_tables.exists() {
        let raw = read_legacy_file(&paths.fan_tables)?;
        config.fan_tables =
            Some(migrate_fan_tables(&raw).map_err(|error| error.with_path(&paths.fan_tables))?);
    }

    if config.is_empty() {
        config.notes.push(format!(
            "aucun fichier historique trouvé dans {}",
            paths.root.display()
        ));
    }

    Ok(config)
}

fn read_legacy_file(path: &Path) -> Result<String, CoreError> {
    let bytes = crate::config::read_file_bounded_no_follow(path, DEFAULT_MAX_CONFIG_BYTES)?;
    String::from_utf8(bytes).map_err(|error| {
        CoreError::config(format!("{}: contenu non UTF-8: {error}", path.display()))
    })
}

/// Migre un fichier `settings` historique.
///
/// # Errors
///
/// Renvoie une erreur si le JSON est invalide ou si les réglages migrés ne
/// passent pas la validation.
pub fn migrate_settings(raw: &str) -> Result<Migration<Settings>, CoreError> {
    let mut settings: Settings = serde_json::from_str(raw)?;
    let mut changed = settings.schema_version != SETTINGS_SCHEMA_VERSION;
    let mut notes = Vec::new();

    if settings.schema_version != SETTINGS_SCHEMA_VERSION {
        notes.push(format!(
            "schemaVersion {} → {SETTINGS_SCHEMA_VERSION}",
            settings.schema_version
        ));
        settings.schema_version = SETTINGS_SCHEMA_VERSION;
    }

    if let Some(value) = settings.extra.remove("keyboardBacklightStates") {
        changed = true;
        // La désérialisation emprunte `value` (`&Value` est un `Deserializer`)
        // au lieu de le consommer : la valeur d'origine reste disponible pour
        // la branche `Err`, sans copie intermédiaire.
        match Vec::<KeyboardState>::deserialize(&value) {
            Ok(states) if settings.keyboard_states.is_empty() && !states.is_empty() => {
                settings.keyboard_states = states;
                notes.push("keyboardBacklightStates → keyboardStates".to_owned());
            }
            Ok(_) => {
                // keyboardStates est déjà renseigné : l'ancien champ est un
                // doublon, sa suppression est la migration attendue.
                notes.push("keyboardBacklightStates obsolète supprimé".to_owned());
            }
            Err(_) => {
                // Ne perd pas la donnée : on réinsère la valeur d'origine telle
                // quelle dans `extra`, la validation ne la regarde pas. Un
                // fichier legacy invalide reste ainsi récupérable si
                // l'utilisateur revient à l'ancien TCC.
                settings
                    .extra
                    .insert("keyboardBacklightStates".to_owned(), value);
                notes.push(
                    "keyboardBacklightStates illisible, conservé dans les champs inconnus"
                        .to_owned(),
                );
            }
        }
    }

    settings.validate()?;
    Ok(Migration {
        value: settings,
        changed,
        notes,
    })
}

/// Migre un fichier `profiles` historique (tableau JSON).
///
/// # Errors
///
/// Renvoie une erreur si le JSON est invalide ou si les profils migrés ne
/// passent pas la validation.
pub fn migrate_profiles(raw: &str) -> Result<Migration<ProfileCollection>, CoreError> {
    let profiles: Vec<Profile> = serde_json::from_str(raw)?;
    let mut changed = false;
    let mut notes = Vec::new();
    let mut seen_ids: BTreeSet<ProfileId> = BTreeSet::new();
    let mut migrated = Vec::with_capacity(profiles.len());

    for (index, mut profile) in profiles.into_iter().enumerate() {
        if profile.id.is_empty() {
            let generated = ProfileId::generate();
            notes.push(format!(
                "profil #{} ({}) sans identifiant → {}",
                index, profile.name, generated
            ));
            profile.id = generated;
            changed = true;
        }

        if !seen_ids.insert(profile.id.clone()) {
            let old_id = profile.id.clone();
            let generated = ProfileId::generate();
            notes.push(format!(
                "identifiant dupliqué {old_id} (profil {}) → {generated}",
                profile.name
            ));
            profile.id = generated;
            seen_ids.insert(profile.id.clone());
            changed = true;
        }

        if profile.fan.custom_fan_curve.normalize() {
            notes.push(format!(
                "courbe personnalisée du profil {} triée par température",
                profile.name
            ));
            changed = true;
        }

        migrated.push(profile);
    }

    let collection = ProfileCollection::new(migrated);
    collection.validate()?;
    Ok(Migration {
        value: collection,
        changed,
        notes,
    })
}

/// Migre un fichier `fantables` historique (tableau JSON).
///
/// Les courbes sans nom reçoivent un nom `Imported N`, les doublons de nom
/// sont suffixés, et les tables sont triées. Aucune courbe n'est supprimée.
///
/// # Errors
///
/// Renvoie une erreur si le JSON est invalide ou si les courbes migrées ne
/// passent pas la validation.
pub fn migrate_fan_tables(raw: &str) -> Result<Migration<FanTableSet>, CoreError> {
    let curves: Vec<FanCurve> = serde_json::from_str(raw)?;
    let mut changed = false;
    let mut notes = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut migrated = Vec::with_capacity(curves.len());

    for (index, mut curve) in curves.into_iter().enumerate() {
        let sorted = curve.normalize();
        if sorted {
            changed = true;
            if let Some(name) = curve.name.clone() {
                notes.push(format!("courbe {name} triée par température"));
            }
        }

        let base_name = curve
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let mut candidate = match base_name {
            Some(name) => name,
            None => {
                changed = true;
                let generated = format!("Imported {}", index + 1);
                notes.push(format!("courbe #{} sans nom → {generated}", index));
                generated
            }
        };

        if names.contains(&candidate) {
            let mut suffix = 2_u32;
            loop {
                let next = format!("{candidate} ({suffix})");
                if !names.contains(&next) {
                    notes.push(format!("nom de courbe dupliqué {candidate} → {next}"));
                    candidate = next;
                    changed = true;
                    break;
                }
                suffix += 1;
            }
        }
        names.insert(candidate.clone());
        curve.name = Some(candidate);
        migrated.push(curve);
    }

    let tables = FanTableSet::new(migrated);
    tables.validate()?;
    Ok(Migration {
        value: tables,
        changed,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        LegacyPaths, migrate_fan_tables, migrate_profiles, migrate_settings, read_legacy_config,
    };
    use crate::profile::{FrequencyLimit, ProfileCollection};
    use crate::settings::SETTINGS_SCHEMA_VERSION;

    const LEGACY_SETTINGS: &str = r#"{
        "fahrenheit": true,
        "stateMap": { "power_ac": "__office__", "power_bat": "__profile_silent__" },
        "shutdownTime": null,
        "cpuSettingsEnabled": true,
        "fanControlEnabled": false,
        "keyboardBacklightControlEnabled": true,
        "ycbcr420Workaround": [],
        "chargingProfile": null,
        "chargingPriority": null,
        "keyboardBacklightStates": [
            { "mode": 1, "brightness": 20, "red": 10, "green": 20, "blue": 30 }
        ],
        "lightbarStates": null
    }"#;

    const LEGACY_PROFILES: &str = r#"[
        {
            "id": "__office__",
            "name": "Office",
            "description": "Profil bureau",
            "display": {
                "brightness": 70,
                "useBrightness": true,
                "refreshRate": 60,
                "useRefRate": true,
                "xResolution": -1,
                "yResolution": -1,
                "useResolution": false
            },
            "cpu": {
                "onlineCores": 8,
                "useMaxPerfGov": false,
                "scalingMinFrequency": 400000,
                "scalingMaxFrequency": -1,
                "governor": "powersave",
                "energyPerformancePreference": "balance_performance",
                "noTurbo": false
            },
            "webcam": { "status": true, "useStatus": true },
            "fan": {
                "useControl": true,
                "fanProfile": "Balanced",
                "minimumFanspeed": 0,
                "maximumFanspeed": 100,
                "offsetFanspeed": 0,
                "customFanCurve": {
                    "tableCPU": [{ "temp": 60, "speed": 40 }, { "temp": 20, "speed": 10 }],
                    "tableGPU": [{ "temp": 20, "speed": 10 }]
                }
            },
            "odmProfile": { "name": "balanced" },
            "odmPowerLimits": { "tdpValues": [15, 25, 35] },
            "nvidiaPowerCTRLProfile": { "cTGPOffset": 0 },
            "customField": { "future": true }
        },
        {
            "name": "Sans identifiant",
            "description": "",
            "display": {},
            "cpu": {},
            "webcam": {},
            "fan": {},
            "odmProfile": {},
            "odmPowerLimits": {},
            "nvidiaPowerCTRLProfile": {}
        }
    ]"#;

    const LEGACY_FANTABLES: &str = r#"[
        {
            "name": "Silent",
            "tableCPU": [{ "temp": 60, "speed": 30 }, { "temp": 20, "speed": 10 }],
            "tableGPU": [{ "temp": 20, "speed": 10 }]
        },
        {
            "tableCPU": [{ "temp": 20, "speed": 10 }],
            "tableGPU": []
        }
    ]"#;

    #[test]
    fn settings_are_migrated_and_renamed() {
        let migration = migrate_settings(LEGACY_SETTINGS).unwrap();
        assert!(migration.changed);
        assert_eq!(migration.value.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migration.value.keyboard_states.len(), 1);
        assert_eq!(migration.value.keyboard_states[0].brightness, 20);
        assert!(
            !migration
                .value
                .extra
                .contains_key("keyboardBacklightStates")
        );
        assert!(
            migration
                .notes
                .iter()
                .any(|note| note.contains("schemaVersion"))
        );
    }

    #[test]
    fn ppd_override_is_migrated_and_unknown_fields_are_preserved() {
        let raw = r#"{
            "ppdOverride": true,
            "futureField": { "nested": 42 }
        }"#;
        let migration = migrate_settings(raw).unwrap();
        assert!(migration.value.ppd_override);
        assert_eq!(
            migration.value.extra.get("futureField"),
            Some(&serde_json::json!({ "nested": 42 }))
        );
        let roundtrip = serde_json::to_value(&migration.value).unwrap();
        assert_eq!(
            roundtrip.get("futureField"),
            Some(&serde_json::json!({ "nested": 42 })),
            "le champ inconnu doit survivre à la sérialisation"
        );
        assert_eq!(roundtrip.get("ppdOverride"), Some(&serde_json::json!(true)));

        let absent = migrate_settings("{}").unwrap();
        assert!(!absent.value.ppd_override);
        assert!(absent.value.extra.is_empty());
    }

    #[test]
    fn invalid_ppd_override_is_refused_by_legacy_migration() {
        let raw = r#"{
            "ppdOverride": "yes",
            "futureField": { "nested": 42 }
        }"#;
        assert!(migrate_settings(raw).is_err());
    }

    #[test]
    fn deleted_profile_ids_are_migrated_and_unknown_fields_preserved() {
        let raw = r#"{
            "deletedProfileIds": ["custom-old", "custom-older"],
            "futureField": { "nested": 42 }
        }"#;
        let migration = migrate_settings(raw).unwrap();
        assert_eq!(
            migration
                .value
                .deleted_profile_ids
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>(),
            vec!["custom-old", "custom-older"]
        );
        assert_eq!(
            migration.value.extra.get("futureField"),
            Some(&serde_json::json!({ "nested": 42 }))
        );

        let absent = migrate_settings("{}").unwrap();
        assert!(absent.value.deleted_profile_ids.is_empty());
    }

    #[test]
    fn invalid_deleted_profile_ids_are_refused_by_legacy_migration() {
        // Type invalide, identifiant invalide, doublon, preset : refus, jamais
        // de coercition silencieuse.
        for raw in [
            r#"{"deletedProfileIds": "custom-old"}"#,
            r#"{"deletedProfileIds": ["bad id"]}"#,
            r#"{"deletedProfileIds": ["custom-a", "custom-a"]}"#,
            r#"{"deletedProfileIds": ["__office__"]}"#,
        ] {
            assert!(migrate_settings(raw).is_err(), "payload accepté: {raw}");
        }
    }

    #[test]
    fn charging_type_and_fn_lock_are_migrated_and_unknown_fields_preserved() {
        let raw = r#"{
            "chargingType": "fast.charge:1-A",
            "fnLockStatus": true,
            "futureField": { "nested": 42 }
        }"#;
        let migration = migrate_settings(raw).unwrap();
        assert_eq!(
            migration.value.charging_type.as_deref(),
            Some("fast.charge:1-A")
        );
        assert_eq!(migration.value.fn_lock_status, Some(true));
        assert_eq!(
            migration.value.extra.get("futureField"),
            Some(&serde_json::json!({ "nested": 42 }))
        );

        let absent = migrate_settings("{}").unwrap();
        assert!(absent.value.charging_type.is_none());
        assert!(absent.value.fn_lock_status.is_none());
    }

    #[test]
    fn invalid_charging_control_fields_are_refused_by_legacy_migration() {
        for raw in [
            r#"{"chargingType": 3}"#,
            r#"{"chargingType": ""}"#,
            r#"{"chargingType": "bad type"}"#,
            r#"{"chargingType": "bad/type"}"#,
            r#"{"fnLockStatus": "yes"}"#,
            r#"{"fnLockStatus": 1}"#,
        ] {
            assert!(migrate_settings(raw).is_err(), "payload accepté: {raw}");
        }

        let too_long = format!(r#"{{"chargingType": "{}"}}"#, "a".repeat(65));
        assert!(migrate_settings(&too_long).is_err());
    }

    /// BUG-QA-001 : une valeur `keyboardBacklightStates` illisible ne doit pas
    /// être perdue. Elle est conservée **telle quelle** dans `extra`, survit à
    /// la sérialisation et reste intacte à la migration suivante.
    #[test]
    fn unreadable_keyboard_backlight_states_are_preserved_verbatim() {
        // `mode: 9` n'est pas une valeur de `KeyboardColorMode` : la conversion
        // vers `Vec<KeyboardState>` échoue et la branche `Err` est prise.
        let raw = r#"{
            "keyboardBacklightStates": [
                { "mode": 9, "brightness": 10, "red": 0, "green": 0, "blue": 0 }
            ]
        }"#;
        let original: serde_json::Value = serde_json::json!([
            { "mode": 9, "brightness": 10, "red": 0, "green": 0, "blue": 0 }
        ]);

        let migration = migrate_settings(raw).unwrap();
        assert!(migration.changed, "la migration a bien été tentée");
        assert!(migration.value.keyboard_states.is_empty());
        assert_eq!(
            migration.value.extra.get("keyboardBacklightStates"),
            Some(&original),
            "la valeur d'origine doit être réinsérée sans transformation"
        );
        assert!(
            migration.notes.iter().any(|note| note.contains("conservé")),
            "la note doit annoncer la conservation: {:?}",
            migration.notes
        );

        // La clé survit à l'écriture JSON et à une seconde migration.
        let json = serde_json::to_string(&migration.value).unwrap();
        assert!(json.contains("keyboardBacklightStates"));
        let second = migrate_settings(&json).unwrap();
        assert_eq!(
            second.value.extra.get("keyboardBacklightStates"),
            Some(&original)
        );
    }

    #[test]
    fn settings_migration_is_idempotent() {
        let first = migrate_settings(LEGACY_SETTINGS).unwrap();
        let json = serde_json::to_string(&first.value).unwrap();
        let second = migrate_settings(&json).unwrap();
        assert!(!second.changed);
        assert_eq!(second.value, first.value);
    }

    #[test]
    fn profiles_migrate_ids_and_keep_unknown_fields() {
        let migration = migrate_profiles(LEGACY_PROFILES).unwrap();
        assert!(migration.changed);
        assert_eq!(migration.value.len(), 2);

        let office = migration.value.get_by_id("__office__").unwrap();
        assert_eq!(office.display.refresh_rate, Some(60.0));
        assert_eq!(office.display.effective_refresh_rate(), Some(60.0));
        assert_eq!(
            office.cpu.scaling_max_frequency,
            Some(FrequencyLimit::Reduced)
        );
        assert!(office.extra.contains_key("customField"));
        assert_eq!(office.fan.custom_fan_curve.table_cpu[0].temp, 20.0);

        let generated = &migration.value.as_slice()[1];
        assert!(!generated.id.is_empty());
        assert!(generated.id.as_str().starts_with("custom-"));
        crate::validate::validate_profile_id(&generated.id).unwrap();
    }

    #[test]
    fn profiles_migration_is_idempotent() {
        let first = migrate_profiles(LEGACY_PROFILES).unwrap();
        let json = serde_json::to_string(&first.value).unwrap();
        let second = migrate_profiles(&json).unwrap();
        assert!(!second.changed);
    }

    #[test]
    fn duplicate_profile_ids_are_regenerated() {
        let raw = r#"[
            { "id": "same", "name": "A" },
            { "id": "same", "name": "B" }
        ]"#;
        let migration = migrate_profiles(raw).unwrap();
        let ids: Vec<&str> = migration.value.ids().map(|id| id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(migration.changed);
    }

    #[test]
    fn fan_tables_get_names_and_sorted_tables() {
        let migration = migrate_fan_tables(LEGACY_FANTABLES).unwrap();
        assert!(migration.changed);
        assert_eq!(migration.value.len(), 2);
        assert!(migration.value.get("Silent").is_some());
        let imported = migration
            .value
            .names()
            .find(|name| name.starts_with("Imported"));
        assert!(imported.is_some(), "courbe sans nom non renommée");

        let silent = migration.value.get("Silent").unwrap();
        assert_eq!(silent.table_cpu[0].temp, 20.0);
    }

    #[test]
    fn fan_tables_migration_is_idempotent() {
        let first = migrate_fan_tables(LEGACY_FANTABLES).unwrap();
        let json = serde_json::to_string(&first.value).unwrap();
        let second = migrate_fan_tables(&json).unwrap();
        assert!(!second.changed);
    }

    #[test]
    fn invalid_legacy_settings_are_rejected() {
        let raw = r#"{ "stateMap": { "power_ac": "../escape", "power_bat": "x" } }"#;
        assert!(migrate_settings(raw).is_err());
    }

    #[test]
    fn read_legacy_directory_handles_missing_and_present_files() {
        let temp = TempDir::new().unwrap();
        let paths = LegacyPaths::new(temp.path());

        let empty = read_legacy_config(&paths).unwrap();
        assert!(empty.is_empty());
        assert!(!empty.notes.is_empty());

        fs::write(&paths.settings, LEGACY_SETTINGS).unwrap();
        fs::write(&paths.profiles, LEGACY_PROFILES).unwrap();
        fs::write(&paths.fan_tables, LEGACY_FANTABLES).unwrap();

        let config = read_legacy_config(&paths).unwrap();
        assert!(config.settings.is_some());
        assert!(config.profiles.is_some());
        assert!(config.fan_tables.is_some());
        assert!(!config.is_empty());
    }

    #[test]
    fn invalid_profile_in_legacy_file_is_rejected() {
        let temp = TempDir::new().unwrap();
        let paths = LegacyPaths::new(temp.path());
        fs::write(&paths.profiles, r#"[{"id": "../escape", "name": "Bad"}]"#).unwrap();

        let error = read_legacy_config(&paths).unwrap_err();
        assert!(matches!(error, crate::CoreError::Validation(_)));
    }

    #[test]
    fn migration_preserves_profile_collection_order() {
        let migration = migrate_profiles(LEGACY_PROFILES).unwrap();
        let names: Vec<&str> = migration
            .value
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(names, vec!["Office", "Sans identifiant"]);
    }

    #[test]
    fn empty_profile_file_is_valid() {
        let migration = migrate_profiles("[]").unwrap();
        assert!(!migration.changed);
        assert!(migration.value.is_empty());
        assert_eq!(migration.value, ProfileCollection::default());
    }

    #[test]
    fn oversized_legacy_file_is_refused() {
        let temp = TempDir::new().unwrap();
        let paths = LegacyPaths::new(temp.path());
        let oversized = vec![b' '; crate::config::DEFAULT_MAX_CONFIG_BYTES as usize + 1];
        fs::write(&paths.settings, oversized).unwrap();

        let error = read_legacy_config(&paths).unwrap_err();
        assert!(error.to_string().contains("limite"));
    }
}
