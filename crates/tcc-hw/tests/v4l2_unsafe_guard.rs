// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Garde statique de la couche V4L2.
//!
//! Vérifie sans matériel :
//! - `unsafe`/`libc::ioctl` uniquement dans `real.rs` (lecture) et
//!   `real_write.rs` (écriture) ;
//! - `#![forbid(unsafe_code)]` sur les modules sûrs ;
//! - `S_CTRL` uniquement dans `abi.rs` et `real_write.rs` ;
//! - aucun ioctl de flux (`REQBUFS`, `QBUF`, `STREAMON`, `mmap`).
//!
//! Ce test échoue si un futur diff réintroduit `S_CTRL` dans le chemin de
//! lecture (`real.rs`, `mod.rs`, `write.rs`, `fake.rs`).

use std::path::{Path, PathBuf};

fn source_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/v4l2")
}

fn rust_files() -> Vec<PathBuf> {
    let dir = source_dir();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Retourne les lignes de code, commentaires retirés.
fn code_lines(contents: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_block = false;
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if in_block {
            if trimmed.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            if !trimmed.contains("*/") {
                in_block = true;
            }
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let code = match line.find("//") {
            Some(position) => &line[..position],
            None => line,
        };
        lines.push(code.to_owned());
    }
    lines
}

fn file_name(file: &Path) -> &str {
    file.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
}

#[test]
fn unsafe_is_confined_to_real_transports() {
    let files = rust_files();
    assert!(!files.is_empty(), "aucun fichier sous src/v4l2");

    for file in &files {
        let relative = file_name(file);
        let contents = std::fs::read_to_string(file).expect("lecture source V4L2");
        let code = code_lines(&contents);
        let unsafe_usage = code
            .iter()
            .filter(|line| line.contains("unsafe {") || line.contains("unsafe fn"))
            .count();
        let ioctl = code
            .iter()
            .filter(|line| line.contains("libc::ioctl"))
            .count();

        if matches!(relative, "real.rs" | "real_write.rs") {
            assert!(
                contents.contains("#![allow(unsafe_code)]"),
                "{relative} doit porter #![allow(unsafe_code)]"
            );
            assert!(unsafe_usage > 0, "{relative} doit contenir l'appel unsafe");
            assert_eq!(ioctl, 1, "un seul helper ioctl attendu dans {relative}");
        } else {
            assert_eq!(unsafe_usage, 0, "{relative} ne doit pas contenir d'unsafe");
            assert_eq!(ioctl, 0, "{relative} ne doit pas appeler libc::ioctl");
            if relative != "mod.rs" {
                assert!(
                    contents.contains("#![forbid(unsafe_code)]"),
                    "{relative} doit porter #![forbid(unsafe_code)]"
                );
            }
        }
    }
}

#[test]
fn s_ctrl_is_only_in_abi_and_real_write() {
    for file in rust_files() {
        let relative = file_name(&file);
        let contents = std::fs::read_to_string(&file).expect("lecture source V4L2");
        for line in code_lines(&contents) {
            if line.contains("S_CTRL") || line.contains("VIDIOC_S_CTRL") {
                assert!(
                    matches!(relative, "abi.rs" | "real_write.rs"),
                    "{relative} ne doit pas référencer S_CTRL : {}",
                    line.trim()
                );
            }
        }
    }
}

#[test]
fn no_streaming_ioctl() {
    let forbidden = ["REQBUFS", "QBUF", "DQBUF", "STREAMON", "STREAMOFF", "mmap"];
    for file in rust_files() {
        let relative = file_name(&file);
        let contents = std::fs::read_to_string(&file).expect("lecture source V4L2");
        for line in code_lines(&contents) {
            for token in forbidden {
                assert!(
                    !line.contains(token),
                    "{relative} contient {token} : {}",
                    line.trim()
                );
            }
        }
    }
}

#[test]
fn real_transports_have_no_streaming_helper() {
    for relative in ["real.rs", "real_write.rs"] {
        let contents =
            std::fs::read_to_string(source_dir().join(relative)).expect("lecture transport");
        for token in ["read_exact", "write_all"] {
            assert!(
                !contents.contains(token),
                "{relative} ne doit pas lire/écrire le flux ({token})"
            );
        }
    }
}
