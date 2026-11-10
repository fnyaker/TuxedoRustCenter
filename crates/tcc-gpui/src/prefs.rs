// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Préférences locales de l'interface et autostart XDG.
//!
//! Tout reste dans le répertoire de configuration de l'utilisateur
//! (`$XDG_CONFIG_HOME`, sinon `~/.config`) : aucun privilège, aucun shell.
//! Les fichiers sont écrits de façon atomique (temporaire puis `rename`).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Fichier autostart, identique à celui de l'interface Qt.
const AUTOSTART_FILE: &str = "tuxedo-rust-center.desktop";
/// Lanceur installé (FHS) : il choisit l'interface et gère `--tray`.
const INSTALLED_BINARY: &str = "/usr/bin/tuxedo-rust-center";
/// Taille maximale lue pour un fichier utilisateur.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Préférences propres à l'interface gpui.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// Langue choisie (`en`, `fr`, `de`) ; absente = langue du système.
    pub language: Option<String>,
    /// Liste de profils en lignes plutôt qu'en grille.
    pub list_mode: bool,
}

/// Répertoire de configuration de l'utilisateur, s'il est absolu.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    let from_env = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    from_env.or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join(".config"))
    })
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::other("chemin sans répertoire"))?;
    std::fs::create_dir_all(directory)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = directory.join(format!(".{name}.tmp.{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn prefs_path(config_home: &Path) -> PathBuf {
    config_home.join("tuxedo-rust-center").join("gpui.json")
}

impl Prefs {
    /// Charge les préférences ; un fichier absent ou illisible donne les
    /// valeurs par défaut.
    #[must_use]
    pub fn load() -> Self {
        config_home()
            .and_then(|home| read_small(&prefs_path(&home)))
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Enregistre les préférences (échec silencieux : elles ne sont qu'un
    /// confort).
    pub fn save(&self) {
        let Some(home) = config_home() else {
            return;
        };
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = write_atomic(&prefs_path(&home), &json);
        }
    }
}

fn autostart_path(config_home: &Path) -> PathBuf {
    config_home.join("autostart").join(AUTOSTART_FILE)
}

fn desktop_entry() -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TUXEDO Control Center\n\
         Comment=TuxedoRustCenter tray\n\
         Exec={INSTALLED_BINARY} --tray\n\
         Icon=tuxedo-rust-center\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

fn entry_enabled(content: &str) -> bool {
    let mut exec = false;
    for line in content.lines().map(str::trim) {
        if line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
        {
            return false;
        }
        exec |= line == format!("Exec={INSTALLED_BINARY} --tray");
    }
    exec
}

fn autostart_enabled_in(config_home: &Path) -> bool {
    read_small(&autostart_path(config_home)).is_some_and(|content| entry_enabled(&content))
}

fn set_autostart_in(config_home: &Path, enabled: bool) -> std::io::Result<()> {
    let path = autostart_path(config_home);
    if enabled {
        write_atomic(&path, &desktop_entry())
    } else {
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// Vrai si le démarrage à l'ouverture de session est actif.
#[must_use]
pub fn autostart_enabled() -> bool {
    config_home().is_some_and(|home| autostart_enabled_in(&home))
}

/// Active ou désactive le démarrage à l'ouverture de session.
///
/// # Errors
///
/// Répertoire de configuration introuvable ou écriture refusée.
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let home = config_home().ok_or_else(|| std::io::Error::other("XDG_CONFIG_HOME introuvable"))?;
    set_autostart_in(&home, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tcc-gpui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn autostart_roundtrip() {
        let home = scratch("autostart");
        assert!(!autostart_enabled_in(&home));
        set_autostart_in(&home, true).unwrap();
        assert!(autostart_enabled_in(&home));
        let content = std::fs::read_to_string(autostart_path(&home)).unwrap();
        assert!(content.starts_with("[Desktop Entry]\n"));
        set_autostart_in(&home, false).unwrap();
        assert!(!autostart_enabled_in(&home));
        // Désactiver deux fois n'est pas une erreur.
        set_autostart_in(&home, false).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn hidden_or_foreign_entries_are_disabled() {
        assert!(entry_enabled(&desktop_entry()));
        assert!(!entry_enabled(&format!("{}Hidden=true\n", desktop_entry())));
        assert!(!entry_enabled("[Desktop Entry]\nExec=/tmp/other --tray\n"));
    }

    #[test]
    fn prefs_roundtrip_and_tolerate_garbage() {
        let home = scratch("prefs");
        let path = prefs_path(&home);
        let prefs = Prefs {
            language: Some("de".to_owned()),
            list_mode: true,
        };
        write_atomic(&path, &serde_json::to_string(&prefs).unwrap()).unwrap();
        let read: Prefs = serde_json::from_str(&read_small(&path).unwrap()).unwrap();
        assert_eq!(read, prefs);
        let partial: Prefs = serde_json::from_str("{\"future\": 1}").unwrap();
        assert_eq!(partial, Prefs::default());
        let _ = std::fs::remove_dir_all(&home);
    }
}
