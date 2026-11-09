// SPDX-License-Identifier: GPL-3.0-or-later
//
// Tests de non-régression des bugs identifiés par la QA.
//
// BUG-QA-001 et BUG-QA-002 sont corrigés dans `tcc-core` : ces tests ne sont
// plus `#[ignore]` et protègent les invariants correspondants. Les analyses
// d'origine restent dans `docs/qa/BUGS.md`.

use tcc_core::{Profile, ProfileCollection, merge_with_default_profiles, migrate_settings};

/// BUG-QA-001 — `migrate_settings` retire `keyboardBacklightStates` de `extra`
/// avant de tenter la conversion. Une valeur illisible doit être réinsérée
/// telle quelle au lieu d'être perdue.
#[test]
fn migrate_settings_preserves_unreadable_keyboard_backlight_states() {
    // Mode 9 : invalide pour `KeyboardColorMode`, donc `Vec<KeyboardState>`
    // échoue et la branche `Err` de la migration est prise.
    let raw = r#"{
        "keyboardBacklightStates": [
            { "mode": 9, "brightness": 10, "red": 0, "green": 0, "blue": 0 }
        ]
    }"#;

    let migration = migrate_settings(raw).expect("settings migrés");
    assert!(
        migration
            .value
            .extra
            .contains_key("keyboardBacklightStates"),
        "la valeur illisible doit être conservée dans les champs inconnus"
    );
    assert!(
        migration.notes.iter().any(|note| note.contains("conservé")),
        "la note doit annoncer la conservation: {:?}",
        migration.notes
    );
}

/// BUG-QA-002 — la fusion des presets peut créer un doublon de nom que la
/// validation refuse, ce qui fait rejeter toute la collection par la GUI.
///
/// Décision : l'identifiant reste la clé d'unicité, le profil chargé en
/// conflit est renommé (`Office (custom)`), les presets gardent leur nom.
#[test]
fn merged_presets_keep_profile_names_unique() {
    let loaded = ProfileCollection::new(vec![Profile::new("custom-office", "Office")]);
    let merged = merge_with_default_profiles(loaded);

    match merged.validate() {
        Ok(()) => {}
        Err(error) => panic!("collection fusionnée invalide: {error}"),
    }

    // Les deux profils survivent, seul le nom du profil utilisateur change.
    assert_eq!(
        merged
            .get_by_id("custom-office")
            .map(|profile| profile.name.as_str()),
        Some("Office (custom)")
    );
    assert_eq!(
        merged
            .get_by_id("__office__")
            .map(|profile| profile.name.as_str()),
        Some("Office")
    );
}
