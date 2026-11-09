// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests boîte noire de `tccd-user --set-resolution`.
//!
//! Aucun compositeur, aucun écran réel : la session est forcée en TTY et le
//! bus de session pointe vers un socket inexistant. Les commandes qui
//! atteignent le backend échouent proprement (code 1) ; les arguments
//! invalides sont des erreurs d'usage (code 2), sans panic ni écriture.

use std::process::{Command, Output};

const USER: &str = env!("CARGO_BIN_EXE_tcc-user");
const NO_BUS: &str = "unix:path=/nonexistent/tuxedo-rust-center-test-bus";

fn run(args: &[&str]) -> Output {
    Command::new(USER)
        .args(args)
        .env("XDG_SESSION_TYPE", "tty")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("SWAYSOCK")
        .env("TCC_DBUS_BUS", "session")
        .env("DBUS_SESSION_BUS_ADDRESS", NO_BUS)
        .env("RUST_LOG", "error")
        .output()
        .expect("exécution de tccd-user")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_and_version_do_not_require_a_backend() {
    let help = run(&["--help"]);
    assert!(help.status.success(), "stderr: {}", stderr(&help));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--set-resolution"));

    let version = run(&["--version"]);
    assert!(version.status.success(), "stderr: {}", stderr(&version));
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn invalid_resolution_arguments_are_usage_errors() {
    for arguments in [
        vec!["--set-resolution"],
        vec!["--set-resolution", "0x1080"],
        vec!["--set-resolution", "1920x0"],
        vec!["--set-resolution", "1920x1080x2"],
        vec!["--set-resolution", "16385x1080"],
        vec!["--set-resolution", "1920x16385"],
        vec!["--set-resolution", "-1920x1080"],
        vec![
            "--set-resolution",
            "1920x1080",
            "--set-resolution",
            "2560x1600",
        ],
    ] {
        let output = run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} devrait être une erreur d'usage, stderr: {}",
            stderr(&output)
        );
        let message = stderr(&output);
        assert!(
            message.contains("tccd-user"),
            "{arguments:?}: stderr inattendu: {message}"
        );
    }
}

#[test]
fn set_resolution_accepts_both_forms_and_reaches_the_backend_cleanly() {
    for arguments in [
        vec!["--set-resolution", "1920x1080", "--dry-run"],
        vec!["--set-resolution=2560x1600", "--dry-run"],
        vec![
            "--set-resolution",
            "1920x1080",
            "--set-refresh",
            "60",
            "--dry-run",
        ],
    ] {
        let output = run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{arguments:?} doit échouer proprement sans écran interne, stderr: {}",
            stderr(&output)
        );
        let message = stderr(&output);
        assert!(
            !message.contains("panicked"),
            "{arguments:?}: panic inattendu: {message}"
        );
        assert!(
            message.contains("tccd-user"),
            "{arguments:?}: stderr inattendu: {message}"
        );
    }
}

#[test]
fn set_resolution_without_dry_run_fails_cleanly_without_backend() {
    let output = run(&["--set-resolution", "1920x1080"]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("non supporté") || message.contains("écran interne"),
        "stderr inattendu: {message}"
    );
    assert!(!message.contains("panicked"));
}

#[test]
fn dry_run_alone_is_a_usage_error() {
    let output = run(&["--dry-run"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--dry-run"));
}
