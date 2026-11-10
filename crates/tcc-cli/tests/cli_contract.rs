// SPDX-License-Identifier: GPL-3.0-or-later
//
// Tests boîte noire du binaire `tcc-cli` : parsing d'arguments, aide/version
// sans D-Bus, et échec propre lorsque le daemon est absent. Le bus de session
// est volontairement pointé vers un socket inexistant : aucun test ne touche
// le bus système ni un daemon réel.

use std::process::{Command, Output};

const CLI: &str = env!("CARGO_BIN_EXE_tcc-cli");
const NO_BUS: &str = "unix:path=/nonexistent/tuxedo-rust-center-test-bus";

fn run(args: &[&str]) -> Output {
    Command::new(CLI)
        .args(args)
        // Tous les tests utilisent le bus de session avec une adresse
        // impossible : aucun accès au bus système réel.
        .env("TCC_DBUS_BUS", "session")
        .env("DBUS_SESSION_BUS_ADDRESS", NO_BUS)
        .output()
        .expect("exécution de tcc-cli")
}

#[test]
fn help_and_version_do_not_need_a_daemon() {
    let help = run(&["--help"]);
    assert!(
        help.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&help.stderr)
    );
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(stdout.contains("Usage"));
    assert!(stdout.contains("tcc-cli"));

    let version = run(&["--version"]);
    assert!(version.status.success());
    let stdout = String::from_utf8_lossy(&version.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn default_command_is_status_and_fails_cleanly_without_daemon() {
    let output = run(&[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("connexion au bus D-Bus"),
        "stderr inattendu: {stderr}"
    );
}

#[test]
fn unknown_command_and_missing_argument_are_usage_errors() {
    let unknown = run(&["nope"]);
    assert!(!unknown.status.success());
    let stderr = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr inattendu: {stderr}"
    );

    let missing = run(&["set-temp"]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        stderr.contains("required arguments"),
        "stderr inattendu: {stderr}"
    );

    let extra = run(&["set-temp", "a", "b"]);
    assert!(!extra.status.success());
    let stderr = String::from_utf8_lossy(&extra.stderr);
    assert!(
        stderr.contains("unexpected argument"),
        "stderr inattendu: {stderr}"
    );
}

#[test]
fn explicit_commands_also_fail_cleanly_without_daemon() {
    for arguments in [
        vec!["status"],
        vec!["profiles"],
        vec!["settings", "show"],
        vec!["profile", "list"],
        vec!["profile", "use", "x"],
        vec!["profile", "set", "x", "name", "y"],
        vec!["settings", "set", "fahrenheit", "true"],
        vec!["charging", "show"],
        vec!["charging", "end", "80"],
        vec!["tdp", "get"],
        vec!["display"],
        vec!["admin"],
        vec!["reload"],
        vec!["fans"],
        vec!["fan"],
        vec!["ping"],
        vec!["clear-temp"],
        vec!["set-temp", "__office__"],
    ] {
        let output = run(&arguments);
        assert!(
            !output.status.success(),
            "{arguments:?} aurait dû échouer sans daemon"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("connexion au bus D-Bus"),
            "{arguments:?}: stderr inattendu: {stderr}"
        );
    }
}

#[test]
fn stats_degrades_gracefully_without_daemon() {
    // stats reste utile (CPU/batterie locaux) quand le daemon est absent.
    let output = run(&["stats", "--json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"daemon\":false"),
        "stdout inattendu: {stdout}"
    );
}

#[test]
fn completions_do_not_need_a_daemon() {
    let output = run(&["completions", "bash"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("tcc-cli"));
}

#[test]
fn out_of_range_threshold_is_a_usage_error() {
    let output = run(&["charging", "end", "101"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("101"));
}
