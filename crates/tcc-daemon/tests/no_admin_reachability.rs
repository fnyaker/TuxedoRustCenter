// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Garde statique : `tccd` ne doit pas pouvoir basculer PRIME.
//!
//! Finding **H-01** de `docs/security/REVIEW_HARDWARE.md` : `tccd` dépend de
//! `tcc-hw`, donc toute fonction publique du crate (par exemple
//! `prime::set_mode`) serait joignable depuis le daemon principal. D1 exige que
//! la bascule PRIME vive dans `tccd-admin`, derrière polkit.
//!
//! `prime::set_mode`, `PRIME_SWITCH_TIMEOUT` et l'allowlist d'écriture ont été
//! retirés de `tcc-hw`. Ce test verrouille la décision : il échoue si du code
//! du binaire `tccd` réintroduit un appel de bascule ou une commande externe
//! directe.
//!
//! Les commentaires sont ignorés (la documentation peut mentionner la
//! décision). `src/bin/` est exclu : le futur binaire `tccd-admin` y vivra et
//! pourra, lui, exécuter `prime-select`.
//!
//! Équivalent manuel :
//!
//! ```sh
//! grep -rn --include='*.rs' -E 'prime::set_mode|PRIME_SWITCH_TIMEOUT' \
//!     crates/tcc-daemon/src --exclude-dir=bin
//! ```

use std::path::{Path, PathBuf};

/// Jetons interdits dans les sources de `tccd`.
const FORBIDDEN: [&str; 2] = ["prime::set_mode", "PRIME_SWITCH_TIMEOUT"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("bin") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Vrai si la ligne est un commentaire (doc incluse).
fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
}

fn offenders(sources: &[PathBuf], needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources {
        let Ok(contents) = std::fs::read_to_string(source) else {
            continue;
        };
        for (index, line) in contents.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            for needle in needles {
                if line.contains(needle) {
                    found.push(format!(
                        "{}:{}: {}",
                        source.display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    found
}

#[test]
fn tccd_never_calls_prime_switch() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    rust_files(&src, &mut sources);
    assert!(
        !sources.is_empty(),
        "aucune source trouvée sous {}",
        src.display()
    );

    let found = offenders(&sources, &FORBIDDEN);
    assert!(
        found.is_empty(),
        "tccd ne doit jamais basculer PRIME (D1, finding H-01) :\n{}",
        found.join("\n")
    );
}

#[test]
fn tcc_hw_does_not_export_prime_switch() {
    // Le crate partagé ne doit plus contenir l'implémentation de bascule.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let prime = manifest.join("../tcc-hw/src/prime.rs");
    let Ok(contents) = std::fs::read_to_string(&prime) else {
        // Le chemin relatif peut varier si le test est déplacé : ne pas
        // produire de faux échec, la garantie principale est le test ci-dessus.
        return;
    };
    for (index, line) in contents.lines().enumerate() {
        if is_comment(line) {
            continue;
        }
        assert!(
            !line.contains("pub fn set_mode"),
            "prime::set_mode doit rester absent de tcc-hw : {}:{}",
            prime.display(),
            index + 1
        );
        assert!(
            !line.contains("PRIME_SWITCH_TIMEOUT"),
            "PRIME_SWITCH_TIMEOUT doit rester absent de tcc-hw : {}:{}",
            prime.display(),
            index + 1
        );
    }
}

#[test]
fn tccd_does_not_spawn_external_commands_directly() {
    // Toute commande externe passe par les allowlists de `tcc-hw` (ou vit dans
    // `tccd-admin`). Ce test protège la propriété vérifiée lors de la revue.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    rust_files(&src, &mut sources);
    let found = offenders(&sources, &["Command::new"]);
    assert!(
        found.is_empty(),
        "tccd ne doit pas lancer de commande externe directement :\n{}",
        found.join("\n")
    );
}
