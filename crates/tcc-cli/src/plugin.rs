// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Installation du plugin de barre Omarchy (`tuxedo.tcc-stats`) dans la
//! configuration de l'utilisateur. Le plugin est du QML (seul format accepté
//! par le shell Omarchy) ; il ne contient aucune logique : il appelle `tcc-cli`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Identifiant du plugin.
pub const PLUGIN_ID: &str = "tuxedo.tcc-stats";
/// Emplacement installé par les paquets.
pub const INSTALLED_SOURCE: &str = "/usr/share/tuxedo-rust-center/omarchy-plugin/tuxedo.tcc-stats";

const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 32;

/// `$XDG_CONFIG_HOME/omarchy/plugins` (ou `~/.config/omarchy/plugins`).
pub fn plugins_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
        .context("HOME et XDG_CONFIG_HOME absents")?;
    Ok(base.join("omarchy").join("plugins"))
}

/// Fichiers réguliers du plugin (relatifs), sans lien symbolique.
fn collect(source: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        let dir = source.join(&relative);
        for entry in fs::read_dir(&dir).with_context(|| format!("lecture de {}", dir.display()))? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let child = relative.join(entry.file_name());
            if kind.is_symlink() {
                bail!("lien symbolique refusé : {}", child.display());
            } else if kind.is_dir() {
                if child.components().count() > 2 {
                    bail!("arborescence trop profonde : {}", child.display());
                }
                stack.push(child);
            } else if kind.is_file() {
                if entry.metadata()?.len() > MAX_FILE_BYTES {
                    bail!("fichier trop volumineux : {}", child.display());
                }
                files.push(child);
                if files.len() > MAX_FILES {
                    bail!("trop de fichiers dans le plugin");
                }
            }
        }
    }
    if !files.iter().any(|f| f == Path::new("manifest.json")) {
        bail!("manifest.json absent de {}", source.display());
    }
    files.sort();
    Ok(files)
}

/// Copie le plugin de `source` vers `<plugins_dir>/<PLUGIN_ID>` (remplacé
/// atomiquement). Renvoie le dossier installé.
pub fn install(source: &Path, plugins_dir: &Path) -> Result<PathBuf> {
    let files = collect(source)?;
    fs::create_dir_all(plugins_dir)
        .with_context(|| format!("création de {}", plugins_dir.display()))?;
    let staging = plugins_dir.join(format!(".{PLUGIN_ID}.tmp"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging)?;
    let copied = (|| -> Result<()> {
        for relative in &files {
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source.join(relative), &target)
                .with_context(|| format!("copie de {}", relative.display()))?;
        }
        Ok(())
    })();
    if let Err(error) = copied {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let destination = plugins_dir.join(PLUGIN_ID);
    let _ = fs::remove_dir_all(&destination);
    fs::rename(&staging, &destination).context("installation du plugin")?;
    Ok(destination)
}

/// Supprime le plugin installé ; `false` s'il n'y en avait pas.
pub fn uninstall(plugins_dir: &Path) -> Result<bool> {
    let destination = plugins_dir.join(PLUGIN_ID);
    match fs::symlink_metadata(&destination) {
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(&destination)?;
            Ok(true)
        }
        Ok(_) => bail!("{} n'est pas un dossier", destination.display()),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_plugin(root: &Path) -> PathBuf {
        let dir = root.join("src");
        fs::create_dir_all(dir.join("i18n")).unwrap();
        fs::write(dir.join("manifest.json"), "{}").unwrap();
        fs::write(dir.join("Panel.qml"), "Item {}").unwrap();
        fs::write(dir.join("i18n/en.json"), "{}").unwrap();
        dir
    }

    #[test]
    fn installs_and_replaces_atomically() {
        let root = tempfile::tempdir().unwrap();
        let source = fake_plugin(root.path());
        let plugins = root.path().join("plugins");
        let dest = install(&source, &plugins).unwrap();
        assert!(dest.join("manifest.json").is_file());
        assert!(dest.join("i18n/en.json").is_file());
        fs::write(dest.join("stale.txt"), "x").unwrap();
        install(&source, &plugins).unwrap();
        assert!(!dest.join("stale.txt").exists());
        assert!(!plugins.join(format!(".{PLUGIN_ID}.tmp")).exists());
    }

    #[test]
    fn rejects_missing_manifest_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let source = fake_plugin(root.path());
        fs::remove_file(source.join("manifest.json")).unwrap();
        assert!(install(&source, &root.path().join("p")).is_err());
        fs::write(source.join("manifest.json"), "{}").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", source.join("link")).unwrap();
        assert!(install(&source, &root.path().join("p")).is_err());
    }

    #[test]
    fn uninstall_reports_presence() {
        let root = tempfile::tempdir().unwrap();
        let source = fake_plugin(root.path());
        let plugins = root.path().join("plugins");
        assert!(!uninstall(&plugins).unwrap());
        install(&source, &plugins).unwrap();
        assert!(uninstall(&plugins).unwrap());
        assert!(!plugins.join(PLUGIN_ID).exists());
    }
}
