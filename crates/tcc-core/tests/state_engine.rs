// SPDX-License-Identifier: GPL-3.0-or-later
//
// Test d'intégration du moteur de bascule sur les types publics de `tcc-core` :
// presets + settings + matrice PPD × alimentation + override + debounce. Aucun
// accès matériel, aucun sommeil : le temps est injecté sous forme de `Duration`.

use std::collections::BTreeSet;
use std::time::Duration;

use tcc_core::{
    DEFAULT_CUSTOM_PROFILE_ID, DEFAULT_DEBOUNCE, HIGH_PERFORMANCE_PROFILE_ID,
    MAX_ENERGY_SAVE_PROFILE_ID, OFFICE_PROFILE_ID, ObserveOutcome, PowerSource, PpdProfile,
    ProfileCollection, ProfileId, QUIET_PROFILE_ID, ResolutionSource, SETTINGS_SCHEMA_VERSION,
    Settings, StateInput, StateKey, StateMatrix, StateResolver, default_profiles,
    merge_with_default_profiles, validate_settings, validate_settings_references,
};

fn profiles() -> ProfileCollection {
    merge_with_default_profiles(ProfileCollection::default())
}

fn settings_with_suggested_matrix() -> Settings {
    let mut settings = Settings::new();
    settings.with_suggested_matrix();
    settings
}

fn resolver(settings: &Settings) -> StateResolver {
    let mut resolver = StateResolver::new(
        settings.state_matrix.clone(),
        settings.state_map.clone(),
        ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
    );
    resolver.set_known_profiles(Some(profiles().id_set()));
    resolver
}

fn key(ppd: Option<PpdProfile>, power: PowerSource) -> StateKey {
    StateKey::new(ppd, power)
}

#[test]
fn suggested_matrix_covers_all_six_ppd_power_combinations() {
    let settings = settings_with_suggested_matrix();
    let profiles = profiles();
    validate_settings(&settings).expect("settings valides");
    let mut resolved = resolver(&settings);

    let cases = [
        (
            PpdProfile::PowerSaver,
            PowerSource::Ac,
            MAX_ENERGY_SAVE_PROFILE_ID,
        ),
        (
            PpdProfile::PowerSaver,
            PowerSource::Battery,
            MAX_ENERGY_SAVE_PROFILE_ID,
        ),
        (PpdProfile::Balanced, PowerSource::Ac, OFFICE_PROFILE_ID),
        (PpdProfile::Balanced, PowerSource::Battery, QUIET_PROFILE_ID),
        (
            PpdProfile::Performance,
            PowerSource::Ac,
            HIGH_PERFORMANCE_PROFILE_ID,
        ),
        (
            PpdProfile::Performance,
            PowerSource::Battery,
            OFFICE_PROFILE_ID,
        ),
    ];

    let mut now = Duration::ZERO;
    for (ppd, power, expected) in cases {
        let input = StateInput::new(Some(ppd.clone()), power);
        // Première observation : immédiate pour la première clé, sinon
        // l'ouverture de la fenêtre de debounce.
        resolved.observe(input.clone(), now);
        now += DEFAULT_DEBOUNCE;
        let outcome = resolved.observe(input.clone(), now);
        let current = outcome.current().expect("état courant");
        assert_eq!(
            current.profile_id.as_str(),
            expected,
            "cas {ppd:?}/{power:?}"
        );
        assert_eq!(current.source, ResolutionSource::StateMatrix);
        assert!(
            profiles.contains(&current.profile_id),
            "profil résolu absent de la collection fusionnée"
        );
        // Un appel supplémentaire avec la même clé ne réapplique rien.
        assert!(matches!(
            resolved.observe(input, now),
            ObserveOutcome::Unchanged(_)
        ));
        now += DEFAULT_DEBOUNCE;
    }
}

#[test]
fn ppd_absent_or_unknown_falls_back_to_legacy_state_map() {
    let mut settings = settings_with_suggested_matrix();
    settings.state_map.power_ac = ProfileId::new(OFFICE_PROFILE_ID);
    settings.state_map.power_bat = ProfileId::new(QUIET_PROFILE_ID);
    let mut resolved = resolver(&settings);

    resolved.observe(StateInput::new(None, PowerSource::Ac), Duration::ZERO);
    assert_eq!(
        resolved.current().unwrap().profile_id.as_str(),
        OFFICE_PROFILE_ID
    );
    assert_eq!(
        resolved.current().unwrap().source,
        ResolutionSource::LegacyStateMap
    );

    let pending = resolved.observe(
        StateInput::new(
            Some(PpdProfile::Unknown("oem-mode".into())),
            PowerSource::Battery,
        ),
        Duration::from_secs(1),
    );
    assert!(pending.is_pending());
    resolved.observe(
        StateInput::new(
            Some(PpdProfile::Unknown("oem-mode".into())),
            PowerSource::Battery,
        ),
        Duration::from_secs(1) + DEFAULT_DEBOUNCE,
    );
    assert_eq!(
        resolved.current().unwrap().profile_id.as_str(),
        QUIET_PROFILE_ID
    );
    assert_eq!(
        resolved.current().unwrap().source,
        ResolutionSource::LegacyStateMap
    );
}

#[test]
fn unknown_power_source_uses_default_profile() {
    let settings = settings_with_suggested_matrix();
    let mut resolved = resolver(&settings);
    resolved.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Unknown),
        Duration::ZERO,
    );
    assert_eq!(
        resolved.current().unwrap().profile_id.as_str(),
        DEFAULT_CUSTOM_PROFILE_ID
    );
    assert_eq!(
        resolved.current().unwrap().source,
        ResolutionSource::DefaultProfile
    );
}

#[test]
fn missing_profile_in_matrix_falls_back_to_legacy_then_default() {
    let mut settings = settings_with_suggested_matrix();
    settings.state_map.power_ac = ProfileId::new("ghost-profile");
    settings.state_map.power_bat = ProfileId::new("ghost-profile");

    let mut matrix = StateMatrix::new();
    matrix.insert(
        key(Some(PpdProfile::Balanced), PowerSource::Ac),
        ProfileId::new("deleted-profile"),
    );
    settings.state_matrix = matrix;

    let mut resolver = StateResolver::new(
        settings.state_matrix.clone(),
        settings.state_map.clone(),
        ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
    );
    resolver.set_known_profiles(Some(profiles().id_set()));

    resolver.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Ac),
        Duration::ZERO,
    );
    assert_eq!(
        resolver.current().unwrap().profile_id.as_str(),
        DEFAULT_CUSTOM_PROFILE_ID,
        "matrice et stateMap pointent vers des profils absents"
    );
    assert_eq!(
        resolver.current().unwrap().source,
        ResolutionSource::DefaultProfile
    );
}

#[test]
fn manual_override_is_invalidated_only_when_new_key_is_confirmed() {
    let settings = settings_with_suggested_matrix();
    let mut resolved = resolver(&settings);

    resolved.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Ac),
        Duration::ZERO,
    );
    assert!(resolved.set_manual_override(ProfileId::new("__high_performance__")));
    assert!(resolved.current().unwrap().is_manual_override());

    // Même clé : l'override survit.
    assert!(matches!(
        resolved.observe(
            StateInput::new(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::from_millis(100)
        ),
        ObserveOutcome::Unchanged(_)
    ));
    assert_eq!(
        resolved.current().unwrap().profile_id.as_str(),
        HIGH_PERFORMANCE_PROFILE_ID
    );

    // Nouvelle clé, encore dans le debounce : l'override reste actif.
    let pending = resolved.observe(
        StateInput::new(Some(PpdProfile::Performance), PowerSource::Ac),
        Duration::from_millis(200),
    );
    assert!(pending.is_pending());
    assert!(resolved.current().unwrap().is_manual_override());

    // Confirmation après le debounce : la matrice reprend la main.
    let changed = resolved.observe(
        StateInput::new(Some(PpdProfile::Performance), PowerSource::Ac),
        Duration::from_millis(200) + DEFAULT_DEBOUNCE,
    );
    match changed {
        ObserveOutcome::Changed { current, .. } => {
            assert_eq!(current.profile_id.as_str(), HIGH_PERFORMANCE_PROFILE_ID);
            assert_eq!(current.source, ResolutionSource::StateMatrix);
        }
        other => panic!("transition attendue, reçu {other:?}"),
    }
    assert!(resolved.manual_override().is_none());
}

#[test]
fn transient_flap_does_not_clear_manual_override() {
    let settings = settings_with_suggested_matrix();
    let mut resolved = resolver(&settings);
    resolved.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Battery),
        Duration::ZERO,
    );
    resolved.set_manual_override(ProfileId::new(HIGH_PERFORMANCE_PROFILE_ID));

    resolved.observe(
        StateInput::new(Some(PpdProfile::Performance), PowerSource::Battery),
        Duration::from_millis(100),
    );
    let back = resolved.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Battery),
        Duration::from_millis(200),
    );
    assert!(matches!(back, ObserveOutcome::Unchanged(_)));
    assert!(resolved.current().unwrap().is_manual_override());
    assert!(resolved.manual_override().is_some());
}

#[test]
fn debounce_boundary_and_force_reapply_are_deterministic() {
    let settings = settings_with_suggested_matrix();
    let mut resolved = resolver(&settings).with_debounce(Duration::from_millis(500));

    resolved.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Ac),
        Duration::ZERO,
    );
    assert!(
        resolved
            .observe(
                StateInput::new(Some(PpdProfile::Performance), PowerSource::Ac),
                Duration::from_millis(100)
            )
            .is_pending()
    );
    // Borne inclusive : 100 + 500.
    assert!(matches!(
        resolved.observe(
            StateInput::new(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(600)
        ),
        ObserveOutcome::Changed { .. }
    ));

    resolved.force_reapply();
    assert!(matches!(
        resolved.observe(
            StateInput::new(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(601)
        ),
        ObserveOutcome::Changed { .. }
    ));
}

#[test]
fn settings_and_matrix_roundtrip_through_json() {
    let settings = settings_with_suggested_matrix();
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["schemaVersion"], SETTINGS_SCHEMA_VERSION);
    assert_eq!(json["stateMatrix"]["balanced/ac"], OFFICE_PROFILE_ID);
    assert_eq!(
        json["stateMatrix"]["performance/battery"],
        OFFICE_PROFILE_ID
    );

    let parsed: Settings = serde_json::from_value(json).unwrap();
    assert_eq!(parsed, settings);
}

#[test]
fn settings_references_reject_unknown_profile_after_merge() {
    let mut settings = settings_with_suggested_matrix();
    settings.state_matrix.insert(
        key(Some(PpdProfile::Performance), PowerSource::Battery),
        ProfileId::new("ghost"),
    );
    let profiles = profiles();

    let error = validate_settings_references(&settings, &profiles).unwrap_err();
    assert!(
        error
            .issues()
            .iter()
            .any(|issue| issue.field == "stateMatrix[performance/battery]"),
        "anomalies: {:?}",
        error.issues()
    );
}

#[test]
fn known_profiles_set_is_only_a_filter_for_matrix_and_state_map() {
    let mut settings = settings_with_suggested_matrix();
    settings.state_map.power_ac = ProfileId::new("ghost");
    let mut resolver = StateResolver::new(
        settings.state_matrix.clone(),
        settings.state_map.clone(),
        ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
    );
    // Même vide, l'ensemble connu ne filtre jamais le profil par défaut :
    // celui-ci doit toujours exister côté daemon.
    resolver.set_known_profiles(Some(BTreeSet::new()));

    resolver.observe(
        StateInput::new(Some(PpdProfile::Balanced), PowerSource::Ac),
        Duration::ZERO,
    );
    assert_eq!(
        resolver.current().unwrap().profile_id.as_str(),
        DEFAULT_CUSTOM_PROFILE_ID
    );
    assert_eq!(
        resolver.current().unwrap().source,
        ResolutionSource::DefaultProfile
    );
}

#[test]
fn default_profiles_are_valid_and_unique() {
    let profiles = default_profiles();
    profiles.validate().expect("presets valides");
    assert_eq!(profiles.len(), 5);
    assert!(profiles.get_by_id(OFFICE_PROFILE_ID).is_some());
    assert!(profiles.get_by_id(QUIET_PROFILE_ID).is_some());
    assert!(profiles.get_by_id(MAX_ENERGY_SAVE_PROFILE_ID).is_some());
    assert!(profiles.get_by_id(HIGH_PERFORMANCE_PROFILE_ID).is_some());
    assert!(profiles.get_by_id(DEFAULT_CUSTOM_PROFILE_ID).is_some());
}
