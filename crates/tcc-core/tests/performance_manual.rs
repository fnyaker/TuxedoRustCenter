// SPDX-License-Identifier: GPL-3.0-or-later
//
// Mesures manuelles, ignorées par défaut : les budgets de performance ne sont
// pas des gates CI tant que la variance des runners n'est pas caractérisée.
//
//   cargo test -p tcc-core --release --test performance_manual -- --ignored --nocapture
//
// Voir `docs/qa/TEST_PLAN.md` section Performance.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use std::time::{Duration, Instant};

use tcc_core::{
    DEFAULT_CUSTOM_PROFILE_ID, LegacyPaths, PowerSource, PpdProfile, ProfileCollection, ProfileId,
    Settings, StateInput, StateResolver, merge_with_default_profiles, read_legacy_config,
};

#[test]
#[ignore = "mesure manuelle (release): migration + validation d'une fixture"]
fn migration_and_validation_stay_under_budget() {
    let paths = LegacyPaths::new(fixtures::fixture_path("legacy"));
    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        let config = read_legacy_config(&paths).expect("migration");
        config.settings.expect("settings").value.validate().unwrap();
        config.profiles.expect("profils").value.validate().unwrap();
        config.fan_tables.expect("tables").value.validate().unwrap();
    }
    let total = start.elapsed();
    let per_iteration = total / iterations;
    println!("migration + validation: {per_iteration:?} par itération ({iterations} itérations)");
    assert!(
        per_iteration < Duration::from_millis(100),
        "budget manuel dépassé: {per_iteration:?} par itération"
    );
}

#[test]
#[ignore = "mesure manuelle (release): résolution d'état à chaud"]
fn state_resolution_stays_under_budget() {
    let mut settings = Settings::new();
    settings.with_suggested_matrix();
    let profiles = merge_with_default_profiles(ProfileCollection::default());
    let mut resolver = StateResolver::new(
        settings.state_matrix,
        settings.state_map,
        ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
    );
    resolver.set_known_profiles(Some(profiles.id_set()));
    let input = StateInput::new(Some(PpdProfile::Balanced), PowerSource::Battery);

    let iterations = 1_000_000_u32;
    let start = Instant::now();
    for _ in 0..iterations {
        let outcome = resolver.observe(input.clone(), Duration::ZERO);
        assert!(outcome.current().is_some());
    }
    let total = start.elapsed();
    println!("résolution d'état: {total:?} pour {iterations} appels");
    assert!(
        total < Duration::from_secs(2),
        "budget manuel dépassé: {total:?}"
    );
}
