// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests du binaire `tccd-admin` (arguments et refus non-root).

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tccd-admin"))
        .args(args)
        .output()
        .expect("exécution de tccd-admin")
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(|value| value.parse::<u32>().ok())
        })
        .is_some_and(|uid| uid == 0)
}

#[test]
fn version_prints_contract_and_exits_successfully() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(stdout.contains("tccd-admin"));
    assert!(stdout.contains("com.tuxedocomputers.tccd.Admin1"));
    assert!(stdout.contains("contrat v1"));
}

#[test]
fn help_documents_the_phase_one_contract() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(stdout.contains("USAGE"));
    assert!(stdout.contains("Admin1"));
    assert!(stdout.contains("root"));
    assert!(stdout.contains("SaveProfilesJSON"));
    assert!(stdout.contains("SaveSettingsJSON"));
    assert!(!stdout.contains("encore implémenté"));
}

#[test]
fn unknown_argument_fails_with_usage_code() {
    let output = run(&["--force"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8");
    assert!(stderr.contains("argument inconnu"));
    assert!(stderr.contains("--help"));
}

#[test]
fn no_argument_refuses_to_run_as_non_root() {
    if is_root() {
        eprintln!("test non-root ignoré : processus root");
        return;
    }
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(77));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8");
    assert!(stderr.contains("root requis"));
}
