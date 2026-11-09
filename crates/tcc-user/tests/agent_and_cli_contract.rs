// SPDX-License-Identifier: GPL-3.0-or-later
//
// Tests d'intégration de l'agent de session et de son CLI. Ils couvrent les
// fonctions publiques testables sans compositeur ni D-Bus : résolution des
// payloads du daemon, parsing d'arguments, planification du taux et invariants
// « écran interne uniquement ».

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use tcc_core::{ProfileCollection, ResolutionSource};
use tcc_hw::display::{DisplayMode, OutputInfo};
use tcc_ipc::ActiveProfile;
use tcc_user::agent::{MAX_REMOTE_JSON_BYTES, resolve_target};
use tcc_user::backend::{
    RefreshPlan, format_hz, plan_refresh, validate_internal_output_name, validate_output_name,
};
use tcc_user::cli::{Command, parse_args};
use tcc_user::security::ensure_not_root_for;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn profiles_payload() -> String {
    fixtures::read_fixture("dbus/profile_collection.json")
}

#[test]
fn valid_daemon_payloads_resolve_the_internal_refresh_target() {
    let target = resolve_target(
        &fixtures::read_fixture("dbus/active_profile.json"),
        &profiles_payload(),
    )
    .expect("payloads valides")
    .expect("cible");

    assert_eq!(target.profile_id.as_str(), "__office__");
    assert_eq!(target.refresh_hz, Some(120.0));
}

#[test]
fn gaming_profile_target_is_165_hz() {
    let target = resolve_target(
        &fixtures::read_fixture("dbus/active_profile_gaming.json"),
        &profiles_payload(),
    )
    .expect("payloads valides")
    .expect("cible");
    assert_eq!(target.profile_id.as_str(), "custom-gaming-0001");
    assert_eq!(target.refresh_hz, Some(165.0));
    assert_eq!(target.resolution, Some((2560, 1600)));
}

/// Construit la référence active d'un profil de la collection de résolution.
fn active_for(id: &str) -> (String, String) {
    let profiles_json = fixtures::read_fixture("dbus/profile_collection_resolution.json");
    let profiles: ProfileCollection =
        serde_json::from_str(&profiles_json).expect("collection résolution");
    let profile = profiles.get_by_id(id).expect("profil de résolution");
    let active = ActiveProfile::from_profile(profile, ResolutionSource::StateMatrix);
    (
        serde_json::to_string(&active).expect("active profile"),
        profiles_json,
    )
}

#[test]
fn resolution_only_target_keeps_refresh_none() {
    let (active, profiles) = active_for("custom-resolution-only");
    let target = resolve_target(&active, &profiles)
        .expect("payloads valides")
        .expect("cible");
    assert_eq!(target.profile_id.as_str(), "custom-resolution-only");
    assert_eq!(target.refresh_hz, None);
    assert_eq!(target.resolution, Some((1920, 1200)));
}

#[test]
fn resolution_and_rate_target_are_both_resolved() {
    let (active, profiles) = active_for("custom-resolution-rate");
    let target = resolve_target(&active, &profiles)
        .expect("payloads valides")
        .expect("cible");
    assert_eq!(target.profile_id.as_str(), "custom-resolution-rate");
    assert_eq!(target.refresh_hz, Some(165.0));
    assert_eq!(target.resolution, Some((2560, 1600)));
}

#[test]
fn rate_only_target_keeps_resolution_none() {
    let (active, profiles) = active_for("custom-rate-only");
    let target = resolve_target(&active, &profiles)
        .expect("payloads valides")
        .expect("cible");
    assert_eq!(target.profile_id.as_str(), "custom-rate-only");
    assert_eq!(target.refresh_hz, Some(120.0));
    assert_eq!(target.resolution, None);
}

#[test]
fn profile_without_display_change_is_a_noop() {
    let (active, profiles) = active_for("custom-no-display");
    assert_eq!(resolve_target(&active, &profiles).expect("payloads"), None);
}

#[test]
fn refresh_only_regression_stays_refresh_only() {
    // Profil `Office` historique : taux seul, aucune résolution.
    let target = resolve_target(
        &fixtures::read_fixture("dbus/active_profile.json"),
        &profiles_payload(),
    )
    .expect("payloads valides")
    .expect("cible");
    assert_eq!(target.refresh_hz, Some(120.0));
    assert_eq!(target.resolution, None);
}

#[test]
fn invalid_resolution_in_collection_is_rejected_before_apply() {
    let profiles_json =
        fixtures::read_fixture("dbus/invalid/profile_collection_bad_resolution.json");
    let profiles: ProfileCollection = serde_json::from_str(&profiles_json).expect("collection");
    let profile = profiles.get_by_id("custom-bad-resolution").expect("profil");
    let active = ActiveProfile::from_profile(profile, ResolutionSource::StateMatrix);
    let error = resolve_target(&serde_json::to_string(&active).unwrap(), &profiles_json)
        .expect_err("résolution hors bornes");
    assert!(
        error.to_string().contains("profiles"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn null_active_profile_is_a_noop() {
    let result = resolve_target("null", &profiles_payload()).expect("null accepté");
    assert!(result.is_none());
}

#[test]
fn profile_without_effective_rate_is_a_noop() {
    let active = r#"{
        "id": "__profile_silent__",
        "name": "Quiet",
        "description": "",
        "isPreset": true,
        "source": "legacyStateMap"
    }"#;
    let target = resolve_target(active, &profiles_payload()).expect("payloads valides");
    assert!(
        target.is_none(),
        "useRefRate=false ne doit jamais déclencher de changement"
    );
}

#[test]
fn legacy_full_profile_payload_is_accepted() {
    // Un ancien daemon peut renvoyer le `Profile` complet au lieu d'un
    // `ActiveProfile`. Seul l'identifiant est retenu ; la collection fait foi.
    let profiles: ProfileCollection = serde_json::from_str(&profiles_payload()).unwrap();
    let gaming = profiles.get_by_id("custom-gaming-0001").expect("gaming");
    let full_profile = serde_json::to_string(gaming).unwrap();

    let target = resolve_target(&full_profile, &profiles_payload())
        .expect("payload legacy accepté")
        .expect("cible");
    assert_eq!(target.profile_id.as_str(), "custom-gaming-0001");
    assert_eq!(target.refresh_hz, Some(165.0));
}

#[test]
fn unknown_active_profile_is_rejected() {
    let mut active: serde_json::Value =
        serde_json::from_str(&fixtures::read_fixture("dbus/active_profile.json")).unwrap();
    active["id"] = serde_json::json!("does-not-exist");

    let error = resolve_target(&active.to_string(), &profiles_payload()).unwrap_err();
    assert!(
        error.to_string().contains("absent de la collection"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn malformed_profiles_and_invalid_collections_are_rejected() {
    let active = fixtures::read_fixture("dbus/active_profile.json");

    let malformed = fixtures::read_fixture("dbus/invalid/malformed.json");
    let error = resolve_target(&active, &malformed).unwrap_err();
    assert!(
        error.to_string().contains("profiles"),
        "erreur inattendue: {error}"
    );

    // La collection contient un doublon d'identifiant : elle est structurellement
    // lisible mais invalide, et le taux demandé force sa validation complète.
    let duplicates = fixtures::read_fixture("dbus/invalid/profile_collection_duplicate_ids.json");
    let error = resolve_target(&active, &duplicates).unwrap_err();
    assert!(
        error.to_string().contains("profiles"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn oversized_daemon_response_is_refused_before_parsing() {
    let oversized = " ".repeat(MAX_REMOTE_JSON_BYTES + 1);
    let error = resolve_target(&oversized, &profiles_payload()).unwrap_err();
    assert!(
        error.to_string().contains("supérieure"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn cli_parses_diagnostic_and_apply_modes() {
    assert_eq!(parse_args(args(&[])).unwrap(), Command::Agent);
    assert_eq!(
        parse_args(args(&["--list", "--dry-run"])).unwrap(),
        Command::List { dry_run: true }
    );
    assert_eq!(
        parse_args(args(&["--set-refresh", "165"])).unwrap(),
        Command::SetRefresh {
            hz: 165.0,
            dry_run: false
        }
    );
    assert_eq!(
        parse_args(args(&["--dry-run", "--set-refresh=59.94"])).unwrap(),
        Command::SetRefresh {
            hz: 59.94,
            dry_run: true
        }
    );
    assert_eq!(
        parse_args(args(&["--set-resolution", "1920x1080"])).unwrap(),
        Command::SetResolution {
            width: 1920,
            height: 1080,
            refresh_hz: None,
            dry_run: false
        }
    );
    assert_eq!(
        parse_args(args(&[
            "--set-resolution=2560x1600",
            "--set-refresh",
            "165",
            "--dry-run"
        ]))
        .unwrap(),
        Command::SetResolution {
            width: 2560,
            height: 1600,
            refresh_hz: Some(165.0),
            dry_run: true
        }
    );
    assert_eq!(parse_args(args(&["--help"])).unwrap(), Command::Help);
    assert_eq!(parse_args(args(&["-V"])).unwrap(), Command::Version);
}

#[test]
fn cli_rejects_conflicts_unknown_options_and_bad_rates() {
    for values in [
        vec!["--list", "--set-refresh", "60"],
        vec!["--dry-run"],
        vec!["--unknown"],
        vec!["--set-refresh"],
        vec!["--set-refresh", "60", "--set-refresh", "120"],
        vec!["--list", "--list"],
        vec!["--list", "--set-resolution", "1920x1080"],
        vec!["--set-resolution", "0x1080"],
        vec!["--set-resolution", "1920x1080x2"],
    ] {
        assert!(
            parse_args(args(&values)).is_err(),
            "accepté à tort: {values:?}"
        );
    }

    for rate in ["0", "-1", "nan", "inf", "1001", "abc", ""] {
        assert!(
            parse_args(args(&["--set-refresh", rate])).is_err(),
            "taux accepté à tort: {rate}"
        );
    }
}

#[test]
fn refresh_planning_never_targets_an_external_output() {
    let outputs = vec![
        output("HDMI-A-1", Some(60.0), &[60.0, 120.0, 165.0]),
        output("eDP-1", Some(60.0), &[60.0, 120.0, 165.0]),
    ];
    match plan_refresh(&outputs, 165.0, 0.5) {
        RefreshPlan::Apply { output, mode } => {
            assert_eq!(output, "eDP-1");
            assert_eq!(mode.refresh_hz, 165.0);
            assert_eq!((mode.width, mode.height), (2560, 1600));
        }
        other => panic!("plan inattendu: {other:?}"),
    }

    let external_only = vec![output("DP-1", Some(60.0), &[60.0, 165.0])];
    assert_eq!(
        plan_refresh(&external_only, 165.0, 0.5),
        RefreshPlan::NoInternalOutput
    );
}

#[test]
fn refresh_planning_honours_tolerance_and_missing_modes() {
    let outputs = vec![output("eDP-1", Some(165.0), &[60.0, 165.0])];
    assert!(matches!(
        plan_refresh(&outputs, 165.2, 0.5),
        RefreshPlan::AlreadyApplied { .. }
    ));
    assert!(matches!(
        plan_refresh(&outputs, 120.0, 0.5),
        RefreshPlan::NotFound { .. }
    ));
    assert!(matches!(
        plan_refresh(&outputs, 164.0, 0.5),
        RefreshPlan::NotFound { .. }
    ));
}

#[test]
fn compositor_output_names_and_rates_are_strictly_formatted() {
    assert_eq!(format_hz(165.0), "165");
    assert_eq!(format_hz(59.94), "59.94");
    assert_eq!(format_hz(59.994), "59.994");
    assert_eq!(format_hz(f32::NAN), "0");

    assert!(validate_output_name("eDP-1"));
    assert!(validate_output_name("HDMI-A-1"));
    assert!(!validate_output_name("eDP-1;rm -rf /"));
    assert!(!validate_output_name("eDP-1,scale=2"));
    assert!(!validate_output_name("../eDP-1"));

    assert!(validate_internal_output_name("eDP-1"));
    assert!(validate_internal_output_name("LVDS-1"));
    assert!(!validate_internal_output_name("HDMI-A-1"));
}

#[test]
fn agent_refuses_root_without_touching_the_system() {
    assert!(ensure_not_root_for(0).is_err());
    assert_eq!(ensure_not_root_for(1000).unwrap(), 1000);
}

fn output(name: &str, current: Option<f32>, rates: &[f32]) -> OutputInfo {
    let mut output = OutputInfo::new(name);
    output.connected = true;
    output.enabled = current.is_some();
    output.modes = rates
        .iter()
        .map(|rate| DisplayMode {
            width: 2560,
            height: 1600,
            refresh_hz: *rate,
            preferred: *rate == rates[0],
            current: false,
        })
        .collect();
    output.current_mode = current.map(|rate| DisplayMode {
        width: 2560,
        height: 1600,
        refresh_hz: rate,
        preferred: false,
        current: true,
    });
    output
}
