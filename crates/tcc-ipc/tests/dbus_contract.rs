// SPDX-License-Identifier: GPL-3.0-or-later
//
// Test d'intégration du contrat D-Bus (`tcc-ipc`) sur des payloads versionnés.
// Vérifie la frontière tcc-core ↔ tcc-ipc : un profil legacy migré doit
// produire une demande d'affichage valide, et toute réponse non fiable doit
// être rejetée avant usage.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use tcc_core::{
    MAX_PROFILE_NAME_LEN, PowerSource, PpdProfile, ProfileCollection, ProfileId, ResolutionSource,
};
use tcc_ipc::{
    ActiveProfile, DisplayBackend, DisplayModeInfo, DisplayModeRequest, DisplayResolution,
    IPC_CONTRACT_VERSION, IpcError, SessionAgentAction, SessionAgentRequest, StateSnapshot,
    ensure_contract_compatible,
};

#[test]
fn valid_state_snapshots_parse_validate_and_roundtrip() {
    for name in [
        "dbus/state_snapshot.json",
        "dbus/state_snapshot_manual_override.json",
        "dbus/state_snapshot_pre_first_apply.json",
    ] {
        let raw = fixtures::read_fixture(name);
        let snapshot: StateSnapshot = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{name}: JSON invalide: {error}"));
        snapshot
            .validate()
            .unwrap_or_else(|error| panic!("{name}: validation: {error}"));
        assert_eq!(snapshot.contract_version, IPC_CONTRACT_VERSION);

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let reparsed: StateSnapshot = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reparsed, snapshot, "{name}: aller-retour non fidèle");
    }
}

#[test]
fn state_snapshot_fields_are_camel_case_and_consistent() {
    let raw = fixtures::read_fixture("dbus/state_snapshot.json");
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(json["ppdProfile"], "balanced");
    assert_eq!(json["powerSource"], "battery");
    assert_eq!(json["activeProfile"]["source"], "stateMatrix");
    assert_eq!(json["manualOverride"], false);

    let snapshot: StateSnapshot = serde_json::from_str(&raw).unwrap();
    assert_eq!(snapshot.power_source, PowerSource::Battery);
    assert_eq!(snapshot.ppd_profile, Some(PpdProfile::Balanced));
    assert!(!snapshot.is_manual_override());
    let active = snapshot.active_profile.expect("profil actif");
    assert_eq!(active.id.as_str(), "__office__");
    assert_eq!(active.source, ResolutionSource::StateMatrix);
    assert!(active.is_preset);

    let manual: StateSnapshot = serde_json::from_str(&fixtures::read_fixture(
        "dbus/state_snapshot_manual_override.json",
    ))
    .unwrap();
    assert!(manual.is_manual_override());
    assert_eq!(
        manual.active_profile.unwrap().source,
        ResolutionSource::ManualOverride
    );
}

#[test]
fn active_profile_fixture_is_valid() {
    let active: ActiveProfile =
        serde_json::from_str(&fixtures::read_fixture("dbus/active_profile.json")).unwrap();
    active.validate().unwrap();
    assert_eq!(active.id.as_str(), "__office__");
    assert!(active.is_preset);

    let gaming: ActiveProfile =
        serde_json::from_str(&fixtures::read_fixture("dbus/active_profile_gaming.json")).unwrap();
    gaming.validate().unwrap();
    assert!(!gaming.is_preset);
}

#[test]
fn profile_collection_crosses_the_ipc_boundary() {
    let profiles: ProfileCollection =
        serde_json::from_str(&fixtures::read_fixture("dbus/profile_collection.json")).unwrap();
    profiles.validate().unwrap();
    assert_eq!(profiles.len(), 3);

    let office = profiles.get_by_id("__office__").expect("office");
    assert!(office.extra.contains_key("x-extension"));
    let active = ActiveProfile::from_profile(office, ResolutionSource::StateMatrix);
    active.validate().unwrap();
    assert!(active.is_preset);

    // Une demande d'affichage construite depuis le profil doit être valide et
    // ne toucher que le taux (pas la résolution).
    let rate = office.display.effective_refresh_rate().expect("taux");
    let request = DisplayModeRequest::new(office.id.clone())
        .with_refresh_rate(rate)
        .with_internal_output("eDP-1");
    request.validate().unwrap();
    assert!(request.refresh_rate_only());
    assert_eq!(request.refresh_rate_hz, Some(120.0));

    let gaming = profiles.get_by_id("custom-gaming-0001").expect("gaming");
    let (width, height) = gaming.display.effective_resolution().expect("résolution");
    let request = DisplayModeRequest::new(gaming.id.clone())
        .with_refresh_rate(gaming.display.effective_refresh_rate().unwrap())
        .with_resolution(DisplayResolution::new(width, height))
        .with_internal_output("eDP-1");
    request.validate().unwrap();
    assert!(!request.refresh_rate_only());
}

#[test]
fn session_agent_requests_are_validated_end_to_end() {
    let request = SessionAgentRequest::apply_display_mode(
        42,
        DisplayModeRequest::new("custom-gaming-0001")
            .with_refresh_rate(165.0)
            .with_internal_output("eDP-1"),
    );
    request.validate().unwrap();
    assert_eq!(request.action, SessionAgentAction::ApplyDisplayMode);
    assert_eq!(request.profile_id().unwrap().as_str(), "custom-gaming-0001");

    let mut external = request.clone();
    external.display_mode = Some(
        DisplayModeRequest::new("custom-gaming-0001")
            .with_refresh_rate(165.0)
            .with_internal_output("HDMI-A-1"),
    );
    assert!(matches!(
        external.validate(),
        Err(IpcError::NonInternalOutput(_))
    ));

    SessionAgentRequest::get_display_mode_info(1)
        .validate()
        .unwrap();
    SessionAgentRequest::reset_display_mode(2)
        .validate()
        .unwrap();
    SessionAgentRequest::new(3, SessionAgentAction::Shutdown)
        .validate()
        .unwrap();
}

#[test]
fn display_info_fixture_shape_is_stable() {
    let info = DisplayModeInfo {
        supported: true,
        backend: DisplayBackend::Kde,
        internal_output: Some("eDP-1".into()),
        current_resolution: Some(DisplayResolution::new(2560, 1600)),
        current_refresh_rate_hz: Some(240.0),
        available_resolutions: vec![DisplayResolution::new(2560, 1600)],
        available_refresh_rates_hz: vec![60.0, 120.0, 240.0],
        error: None,
    };
    info.validate().unwrap();
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["internalOutput"], "eDP-1");
    assert_eq!(json["currentRefreshRateHz"], 240.0);
    let parsed: DisplayModeInfo = serde_json::from_value(json).unwrap();
    assert_eq!(parsed, info);
}

#[test]
fn malformed_or_structurally_invalid_payloads_are_rejected() {
    let malformed = fixtures::read_fixture("dbus/invalid/malformed.json");
    assert!(serde_json::from_str::<StateSnapshot>(&malformed).is_err());

    let missing = fixtures::read_fixture("dbus/invalid/state_snapshot_missing_fields.json");
    assert!(serde_json::from_str::<StateSnapshot>(&missing).is_err());

    let bad_enum = fixtures::read_fixture("dbus/invalid/state_snapshot_bad_enum.json");
    assert!(serde_json::from_str::<StateSnapshot>(&bad_enum).is_err());

    let not_a_collection = fixtures::read_fixture("dbus/invalid/not_a_collection.json");
    assert!(serde_json::from_str::<ProfileCollection>(&not_a_collection).is_err());
}

#[test]
fn semantically_invalid_payloads_are_rejected_by_validation() {
    let contract: StateSnapshot = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/state_snapshot_contract_mismatch.json",
    ))
    .unwrap();
    assert!(matches!(
        contract.validate(),
        Err(IpcError::ContractMismatch { .. })
    ));

    let manual: StateSnapshot = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/state_snapshot_manual_mismatch.json",
    ))
    .unwrap();
    assert!(matches!(
        manual.validate(),
        Err(IpcError::InvalidField { .. })
    ));

    let bad_id: ActiveProfile = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/active_profile_bad_id.json",
    ))
    .unwrap();
    assert!(matches!(
        bad_id.validate(),
        Err(IpcError::InvalidField { .. })
    ));

    let bad_name: ActiveProfile = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/active_profile_bad_name.json",
    ))
    .unwrap();
    assert!(matches!(
        bad_name.validate(),
        Err(IpcError::InvalidField { .. })
    ));

    let duplicates: ProfileCollection = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/profile_collection_duplicate_ids.json",
    ))
    .unwrap();
    let error = duplicates.validate().unwrap_err();
    assert!(
        error
            .issues()
            .iter()
            .any(|issue| issue.field.contains("id")),
        "anomalies: {:?}",
        error.issues()
    );
}

#[test]
fn out_of_range_profile_values_are_aggregated() {
    let profiles: ProfileCollection = serde_json::from_str(&fixtures::read_fixture(
        "dbus/invalid/profile_collection_bad_values.json",
    ))
    .unwrap();
    let error = profiles.validate().unwrap_err();
    let fields: Vec<&str> = error
        .issues()
        .iter()
        .map(|issue| issue.field.as_str())
        .collect();

    for expected in [
        "display.brightness",
        "display.refreshRate",
        "cpu.onlineCores",
        "fan.minimumFanspeed",
        "odmPowerLimits.tdpValues[1]",
        "nvidiaPowerCTRLProfile.cTGPOffset",
    ] {
        assert!(
            fields.contains(&expected),
            "champ {expected} non signalé: {fields:?}"
        );
    }
}

#[test]
fn name_length_limit_is_enforced_at_the_ipc_boundary() {
    let mut active = ActiveProfile {
        id: ProfileId::new("__office__"),
        name: "x".repeat(MAX_PROFILE_NAME_LEN + 1),
        description: String::new(),
        is_preset: true,
        source: ResolutionSource::StateMatrix,
    };
    assert!(active.validate().is_err());

    active.name = "x".repeat(MAX_PROFILE_NAME_LEN);
    active.validate().expect("longueur maximale acceptée");
}

#[test]
fn contract_version_is_strictly_compared() {
    ensure_contract_compatible(IPC_CONTRACT_VERSION).unwrap();
    assert!(ensure_contract_compatible(IPC_CONTRACT_VERSION + 1).is_err());
    assert!(ensure_contract_compatible(0).is_err());
}
