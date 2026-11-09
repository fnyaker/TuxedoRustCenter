// SPDX-License-Identifier: GPL-3.0-or-later
//
// Helpers partagés par les tests d'intégration. Ce fichier est inclus via
// `#[path = "../../../tests/support/fixtures.rs"]` : il est compilé dans le
// crate de test de chaque paquet, jamais dans le code de production.
//
// `CARGO_MANIFEST_DIR` désigne le paquet en cours de compilation, toujours
// situé à `crates/<nom>` ; `../..` pointe donc sur la racine du workspace.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

/// Racine du workspace, indépendante du répertoire courant du test.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("racine du workspace introuvable")
}

/// Chemin absolu d'une fixture sous `tests/fixtures/`.
pub fn fixture_path(relative: impl AsRef<Path>) -> PathBuf {
    workspace_root()
        .join("tests")
        .join("fixtures")
        .join(relative)
}

/// Lit une fixture texte.
pub fn read_fixture(relative: impl AsRef<Path>) -> String {
    let path = fixture_path(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "lecture de la fixture {} impossible: {error}",
            path.display()
        )
    })
}

/// Lit une fixture binaire (JSON non UTF-8, par exemple).
pub fn read_fixture_bytes(relative: impl AsRef<Path>) -> Vec<u8> {
    let path = fixture_path(relative);
    fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "lecture de la fixture {} impossible: {error}",
            path.display()
        )
    })
}
