// SPDX-License-Identifier: GPL-3.0-or-later
//
// Test d'intégration : migration d'une arborescence `/etc/tcc` représentative
// vers les modèles `tcc-core`, validation, persistance atomique et
// aller-retour JSON. Complète les tests unitaires de `migration.rs` en
// vérifiant la chaîne complète sur des fixtures versionnées.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use std::fs;

use tcc_core::{
    ConfigStore, LegacyPaths, ProfileCollection, SETTINGS_SCHEMA_VERSION, Settings,
    migrate_fan_tables, migrate_profiles, migrate_settings, read_legacy_config,
    validate_settings_references,
};

fn legacy_paths() -> LegacyPaths {
    LegacyPaths::new(fixtures::fixture_path("legacy"))
}

#[test]
fn legacy_fixture_migrates_settings_profiles_and_fan_tables() {
    let config = read_legacy_config(&legacy_paths()).expect("migration de la fixture");

    assert!(!config.is_empty());
    assert!(
        config.notes.is_empty(),
        "notes inattendues: {:?}",
        config.notes
    );

    let settings = config.settings.expect("settings migrés");
    assert!(settings.changed, "le schéma legacy doit être mis à jour");
    assert_eq!(settings.value.schema_version, SETTINGS_SCHEMA_VERSION);
    assert!(
        settings
            .value
            .extra
            .get("futureSettings")
            .is_some_and(|value| value["telemetry"] == false),
        "les champs inconnus doivent survivre à la migration"
    );
    assert!(
        !settings.value.extra.contains_key("keyboardBacklightStates"),
        "l'ancien champ clavier doit être consommé"
    );
    assert_eq!(settings.value.keyboard_states.len(), 2);
    assert_eq!(settings.value.keyboard_states[0].red, 227);
    settings.value.validate().expect("settings valides");

    let profiles = config.profiles.expect("profils migrés");
    assert!(profiles.changed, "un identifiant doit être généré");
    assert_eq!(profiles.value.len(), 4);
    profiles.value.validate().expect("profils valides");

    let office = profiles
        .value
        .get_by_id("__office__")
        .expect("profil office");
    assert_eq!(office.display.refresh_rate, None);
    assert_eq!(office.display.effective_refresh_rate(), None);
    assert_eq!(
        office.cpu.scaling_max_frequency,
        Some(tcc_core::FrequencyLimit::Reduced)
    );
    assert!(office.extra.contains_key("customField"));
    assert_eq!(
        office.fan.custom_fan_curve.table_cpu[0].temp, 20.0,
        "la courbe doit être triée par température"
    );

    let gaming = profiles
        .value
        .get_by_id("custom-gaming-0001")
        .expect("profil gaming");
    assert_eq!(gaming.display.effective_refresh_rate(), Some(165.0));
    assert_eq!(gaming.display.effective_resolution(), Some((2560, 1600)));

    let generated = profiles
        .value
        .iter()
        .find(|profile| profile.name == "Legacy Sans Identifiant")
        .expect("profil sans identifiant");
    assert!(!generated.id.is_empty());
    assert!(generated.id.as_str().starts_with("custom-"));
    tcc_core::validate_profile_id(&generated.id).expect("identifiant généré valide");

    let tables = config.fan_tables.expect("tables migrées");
    assert!(tables.changed);
    assert_eq!(tables.value.len(), 4);
    assert!(tables.value.get("Silent").is_some());
    assert!(tables.value.get("Silent (2)").is_some());
    assert!(tables.value.get("Imported 3").is_some());
    tables.value.validate().expect("tables valides");
}

#[test]
fn migrated_settings_reference_existing_profiles_after_merge() {
    let config = read_legacy_config(&legacy_paths()).expect("migration");
    let settings = config.settings.expect("settings").value;
    let profiles = tcc_core::merge_with_default_profiles(config.profiles.expect("profils").value);

    // Les presets ajoutés couvrent les identifiants historiques (__office__,
    // __profile_silent__) et le profil par défaut.
    validate_settings_references(&settings, &profiles).expect("références valides");
    profiles.validate().expect("collection fusionnée valide");
}

#[test]
fn migration_is_idempotent_on_the_full_fixture() {
    let settings_raw = fs::read_to_string(fixtures::fixture_path("legacy/settings")).unwrap();
    let profiles_raw = fs::read_to_string(fixtures::fixture_path("legacy/profiles")).unwrap();
    let tables_raw = fs::read_to_string(fixtures::fixture_path("legacy/fantables")).unwrap();

    let first_settings = migrate_settings(&settings_raw).unwrap();
    let second_settings =
        migrate_settings(&serde_json::to_string(&first_settings.value).unwrap()).unwrap();
    assert!(!second_settings.changed);
    assert_eq!(second_settings.value, first_settings.value);

    let first_profiles = migrate_profiles(&profiles_raw).unwrap();
    let second_profiles =
        migrate_profiles(&serde_json::to_string(&first_profiles.value).unwrap()).unwrap();
    assert!(!second_profiles.changed);
    assert_eq!(second_profiles.value, first_profiles.value);

    let first_tables = migrate_fan_tables(&tables_raw).unwrap();
    let second_tables =
        migrate_fan_tables(&serde_json::to_string(&first_tables.value).unwrap()).unwrap();
    assert!(!second_tables.changed);
    assert_eq!(second_tables.value, first_tables.value);
}

#[test]
fn migrated_models_roundtrip_through_atomic_config_store() {
    let config = read_legacy_config(&legacy_paths()).unwrap();
    let settings: Settings = config.settings.unwrap().value;
    let profiles: ProfileCollection = config.profiles.unwrap().value;
    let tables = config.fan_tables.unwrap().value;

    let state_dir = tempfile::tempdir().expect("tempdir état");
    let store = ConfigStore::from_root(state_dir.path());
    store.save_settings(&settings).expect("écriture settings");
    store.save_profiles(&profiles).expect("écriture profils");
    store.save_fan_tables(&tables).expect("écriture tables");

    let loaded_settings = store.load_settings().expect("relecture settings");
    assert!(!loaded_settings.has_warning());
    assert_eq!(loaded_settings.value, settings);

    let loaded_profiles = store.load_profiles().expect("relecture profils");
    assert!(!loaded_profiles.has_warning());
    assert_eq!(loaded_profiles.value, profiles);

    let loaded_tables = store.load_fan_tables().expect("relecture tables");
    assert!(!loaded_tables.has_warning());
    assert_eq!(loaded_tables.value, tables);

    // Les clés historiques restent lisibles par l'ancien TCC.
    let profiles_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.paths().profiles).unwrap()).unwrap();
    let office = profiles_json
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["id"] == "__office__")
        .expect("office sérialisé");
    assert_eq!(office["display"]["refreshRate"], -1);
    assert_eq!(office["display"]["useRefRate"], false);
    assert_eq!(office["cpu"]["scalingMaxFrequency"], -1);
    assert_eq!(office["nvidiaPowerCTRLProfile"]["cTGPOffset"], 0.0);
}

#[test]
fn legacy_symlink_is_refused() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let paths = LegacyPaths::new(dir.path());
    fs::write(&paths.profiles, "[]").unwrap();
    // Un fichier `settings` qui pointe ailleurs ne doit jamais être suivi :
    // la migration pourrait sinon lire un fichier arbitraire.
    symlink(&paths.profiles, &paths.settings).unwrap();

    let error = read_legacy_config(&paths).unwrap_err();
    assert!(
        error.to_string().contains("settings"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn tampered_profile_fixture_is_rejected_without_partial_migration() {
    let raw = fs::read_to_string(fixtures::fixture_path("legacy/profiles")).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let gaming = value
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|profile| profile["id"] == "custom-gaming-0001")
        .expect("gaming");
    gaming["display"]["refreshRate"] = serde_json::json!(5000);

    let error = migrate_profiles(&serde_json::to_string(&value).unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("display.refreshRate"),
        "erreur inattendue: {error}"
    );
}
