// SPDX-License-Identifier: GPL-3.0-or-later
//
// Autostart XDG de la GUI : `$XDG_CONFIG_HOME/autostart/…desktop`.
//
// Aucun privilège, aucun shell, aucune écriture hors du répertoire
// utilisateur : `XDG_CONFIG_HOME` doit être absolu et sous `$HOME` (sinon
// `PathInvalid`). Le fichier est écrit de façon atomique (fichier temporaire,
// `fsync`, `rename`, permissions 0644) et supprimé sur désactivation.
//
// Le contenu est minimal et stable : `Exec=/usr/bin/tuxedo-rust-center --tray`,
// ce qui démarre la GUI dans la barre système, ou en fenêtre si le tray n'est
// pas disponible (géré côté QML).

use std::ffi::OsStr;
use std::fs::{DirBuilder, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

/// Nom du fichier autostart.
pub const AUTOSTART_FILE: &str = "tuxedo-rust-center.desktop";
/// Binaire installé (FHS), jamais un chemin dev.
pub const INSTALLED_BINARY: &str = "/usr/bin/tuxedo-rust-center";
/// Taille maximale lue pour déterminer l'état (fichier utilisateur).
const MAX_ENTRY_BYTES: u64 = 64 * 1024;

/// Erreur typée de l'autostart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartError {
    /// Ni `$HOME` absolu ni chemin utilisable.
    NoHome,
    /// `XDG_CONFIG_HOME` hors du répertoire utilisateur, symlink ou chemin
    /// relatif.
    PathInvalid,
    /// Lecture du fichier existant impossible.
    ReadFailed,
    /// Répertoire/fichier possédé par un autre utilisateur ou accessible en
    /// écriture au groupe/autres.
    InsecurePermissions,
    /// Écriture ou suppression impossible.
    WriteFailed,
}

impl AutostartError {
    /// Code stable pour l'UI.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NoHome => "NoHome",
            Self::PathInvalid => "PathInvalid",
            Self::ReadFailed => "ReadFailed",
            Self::InsecurePermissions => "InsecurePermissions",
            Self::WriteFailed => "WriteFailed",
        }
    }
}

/// État exposé à QML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutostartState {
    /// Un chemin d'autostart utilisable existe.
    pub available: bool,
    /// Le fichier existe et n'est pas désactivé.
    pub enabled: bool,
    /// Code d'erreur (`""` si aucun).
    pub error: &'static str,
}

/// Résout le répertoire de configuration XDG.
///
/// `XDG_CONFIG_HOME` n'est utilisé que s'il est absolu, sans `..` et **sous
/// `$HOME`** ; sinon on retombe sur `$HOME/.config` (valeur relative ignorée
/// comme le veut la spécification XDG).
///
/// # Errors
///
/// [`AutostartError::NoHome`] si `$HOME` est absent ou relatif,
/// [`AutostartError::PathInvalid`] si `$HOME` ou la racine XDG contient `..`
/// ou si la racine XDG sort du répertoire utilisateur.
pub fn resolve_config_home(
    xdg_config: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, AutostartError> {
    let home = home
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or(AutostartError::NoHome)?;
    if contains_parent(home) {
        return Err(AutostartError::PathInvalid);
    }
    if let Some(xdg) = xdg_config.map(Path::new).filter(|path| path.is_absolute()) {
        if contains_parent(xdg) || !xdg.starts_with(home) {
            return Err(AutostartError::PathInvalid);
        }
        return Ok(xdg.to_path_buf());
    }
    Ok(home.join(".config"))
}

/// Chemin du fichier autostart pour une racine de configuration donnée.
///
/// # Errors
///
/// [`AutostartError::PathInvalid`] si la racine n'est pas absolue, contient
/// `..` ou est vide.
pub fn autostart_path(config_home: &Path) -> Result<PathBuf, AutostartError> {
    if !config_home.is_absolute() || contains_parent(config_home) {
        return Err(AutostartError::PathInvalid);
    }
    Ok(config_home.join("autostart").join(AUTOSTART_FILE))
}

/// Contenu minimal du fichier autostart.
#[must_use]
pub fn desktop_entry() -> String {
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

/// Vrai si le fichier existe et n'est pas explicitement désactivé.
///
/// # Errors
///
/// [`AutostartError::ReadFailed`] (lecture/taille), [`AutostartError::PathInvalid`]
/// (symlink, dossier, fichier non régulier).
pub fn is_enabled(path: &Path) -> Result<bool, AutostartError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(AutostartError::ReadFailed),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AutostartError::PathInvalid);
    }
    if metadata.len() > MAX_ENTRY_BYTES {
        return Err(AutostartError::ReadFailed);
    }
    let content = std::fs::read_to_string(path).map_err(|_| AutostartError::ReadFailed)?;
    let mut has_type = false;
    let mut has_exec = false;
    let mut disabled = false;
    for line in content.lines().map(str::trim) {
        if line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
            || line.eq_ignore_ascii_case("Hidden=true")
        {
            disabled = true;
        }
        if line == "Type=Application" {
            has_type = true;
        }
        if line == format!("Exec={INSTALLED_BINARY} --tray") {
            has_exec = true;
        }
    }
    // Un fichier utilisateur sans les clés attendues n'est pas notre entrée :
    // il peut exister, mais l'autostart TCC est considéré désactivé.
    Ok(!disabled && has_type && has_exec)
}

/// Active ou désactive l'autostart.
///
/// # Errors
///
/// [`AutostartError::PathInvalid`] (symlink, chemin hors utilisateur),
/// [`AutostartError::WriteFailed`] (création/rename/suppression).
pub fn set_enabled(path: &Path, enabled: bool) -> Result<(), AutostartError> {
    if enabled {
        write_entry(path)
    } else {
        remove_entry(path)
    }
}

fn write_entry(path: &Path) -> Result<(), AutostartError> {
    let directory = path.parent().ok_or(AutostartError::PathInvalid)?;
    if !directory.is_absolute() || contains_parent(directory) {
        return Err(AutostartError::PathInvalid);
    }
    // `$XDG_CONFIG_HOME` et `autostart` ne doivent jamais être accessibles en
    // écriture au groupe/autres (T2-02). Le répertoire autostart est créé 0700.
    if let Some(config_home) = directory.parent() {
        ensure_secure_directory(config_home)?;
    }
    ensure_secure_directory(directory)?;
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AutostartError::PathInvalid);
        }
        if metadata.uid() != current_uid() {
            return Err(AutostartError::InsecurePermissions);
        }
    }

    let temporary = directory.join(format!(".{AUTOSTART_FILE}.tmp.{}", std::process::id()));
    // Un temporaire d'un crash précédent (même PID recyclé) ne doit pas
    // bloquer `create_new` : le répertoire est 0700, la suppression est sûre.
    let _ = std::fs::remove_file(&temporary);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| AutostartError::WriteFailed)?;
        file.write_all(desktop_entry().as_bytes())
            .map_err(|_| AutostartError::WriteFailed)?;
        file.sync_all().map_err(|_| AutostartError::WriteFailed)?;
        drop(file);
        std::fs::set_permissions(&temporary, Permissions::from_mode(0o644))
            .map_err(|_| AutostartError::WriteFailed)?;
        std::fs::rename(&temporary, path).map_err(|_| AutostartError::WriteFailed)?;
        // Durabilité de l'entrée de répertoire (best-effort : le rename a
        // déjà réussi, ne pas transformer un fsync indisponible en échec).
        if let Ok(directory_file) = std::fs::File::open(directory) {
            let _ = directory_file.sync_all();
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|_| AutostartError::WriteFailed)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.uid() != current_uid()
            || metadata.permissions().mode() & 0o777 != 0o644
        {
            return Err(AutostartError::InsecurePermissions);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Vérifie (ou crée en 0700) un répertoire utilisateur sûr.
fn ensure_secure_directory(path: &Path) -> Result<(), AutostartError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AutostartError::PathInvalid);
            }
            if metadata.uid() != current_uid() || metadata.permissions().mode() & 0o022 != 0 {
                return Err(AutostartError::InsecurePermissions);
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| AutostartError::WriteFailed)?;
            let metadata =
                std::fs::symlink_metadata(path).map_err(|_| AutostartError::WriteFailed)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != current_uid()
                || metadata.permissions().mode() & 0o022 != 0
            {
                return Err(AutostartError::InsecurePermissions);
            }
            Ok(())
        }
        Err(_) => Err(AutostartError::ReadFailed),
    }
}

#[allow(unsafe_code)]
fn current_uid() -> u32 {
    // `geteuid` est sûr dans ce contexte (aucun pointeur, aucun état partagé).
    unsafe { libc::geteuid() }
}

fn remove_entry(path: &Path) -> Result<(), AutostartError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AutostartError::PathInvalid),
        Ok(metadata) if metadata.is_file() => {
            std::fs::remove_file(path).map_err(|_| AutostartError::WriteFailed)
        }
        Ok(_) => Err(AutostartError::PathInvalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AutostartError::ReadFailed),
    }
}

fn contains_parent(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

/// Racine de configuration de l'utilisateur courant.
///
/// # Errors
///
/// Voir [`resolve_config_home`].
pub fn current_config_home() -> Result<PathBuf, AutostartError> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    resolve_config_home(xdg.as_deref(), home.as_deref())
}

/// Chemin autostart de l'utilisateur courant.
///
/// # Errors
///
/// Voir [`current_config_home`] et [`autostart_path`].
pub fn current_path() -> Result<PathBuf, AutostartError> {
    autostart_path(&current_config_home()?)
}

/// État courant pour QML (jamais d'erreur fatale).
#[must_use]
pub fn current_state() -> AutostartState {
    let path = match current_path() {
        Ok(path) => path,
        Err(error) => {
            return AutostartState {
                available: false,
                enabled: false,
                error: error.code(),
            };
        }
    };
    match is_enabled(&path) {
        Ok(enabled) => AutostartState {
            available: true,
            enabled,
            error: "",
        },
        Err(error) => AutostartState {
            available: true,
            enabled: false,
            error: error.code(),
        },
    }
}

/// Active/désactive l'autostart courant.
///
/// # Errors
///
/// Voir [`current_path`] et [`set_enabled`].
pub fn set_enabled_current(enabled: bool) -> Result<(), AutostartError> {
    set_enabled(&current_path()?, enabled)
}

#[cfg(test)]
mod tests {
    use super::{
        AUTOSTART_FILE, AutostartError, autostart_path, desktop_entry, is_enabled,
        resolve_config_home, set_enabled,
    };
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use tempfile::TempDir;

    fn config_dir(temp: &TempDir) -> std::path::PathBuf {
        temp.path().join("config")
    }

    fn entry_path(temp: &TempDir) -> std::path::PathBuf {
        config_dir(temp).join("autostart").join(AUTOSTART_FILE)
    }

    #[test]
    fn desktop_entry_has_required_keys_and_tray_flag() {
        let entry = desktop_entry();
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Type=Application\n"));
        assert!(entry.contains("Exec=/usr/bin/tuxedo-rust-center --tray\n"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true\n"));
        assert!(entry.contains("Terminal=false\n"));
        assert!(!entry.contains("shell"));
    }

    #[test]
    fn write_enable_read_remove_is_atomic_and_idempotent() {
        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        assert!(!is_enabled(&path).expect("absent"));

        set_enabled(&path, true).expect("activation");
        assert!(is_enabled(&path).expect("actif"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("lecture"),
            desktop_entry()
        );
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);

        // Réécriture idempotente.
        set_enabled(&path, true).expect("réécriture");
        assert!(is_enabled(&path).expect("actif"));

        set_enabled(&path, false).expect("désactivation");
        assert!(!is_enabled(&path).expect("absent après suppression"));
        // Suppression répétée : idempotente.
        set_enabled(&path, false).expect("désactivation idempotente");
    }

    #[test]
    fn explicit_disable_or_hidden_is_reported_as_off() {
        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dossier");
        std::fs::write(&path, "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n")
            .expect("écriture");
        assert!(!is_enabled(&path).expect("désactivé"));

        std::fs::write(&path, "[Desktop Entry]\nHidden=true\n").expect("écriture");
        assert!(!is_enabled(&path).expect("masqué"));
    }

    #[test]
    fn insecure_directories_are_refused() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        // Répertoire autostart préexistant accessible en écriture au groupe.
        let temp = TempDir::new().expect("tempdir");
        let config = config_dir(&temp);
        let autostart = config.join("autostart");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&autostart)
            .expect("dossiers");
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700))
            .expect("config 0700");
        std::fs::set_permissions(&autostart, std::fs::Permissions::from_mode(0o775))
            .expect("autostart 0775");
        let path = entry_path(&temp);
        assert_eq!(
            set_enabled(&path, true).unwrap_err(),
            AutostartError::InsecurePermissions
        );
        assert!(!path.exists(), "aucun fichier ne doit être écrit");

        // Parent `$XDG_CONFIG_HOME` accessible en écriture au groupe.
        let temp = TempDir::new().expect("tempdir");
        let config = config_dir(&temp);
        std::fs::DirBuilder::new()
            .mode(0o775)
            .create(&config)
            .expect("config");
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o775))
            .expect("config 0775");
        assert_eq!(
            set_enabled(&entry_path(&temp), true).unwrap_err(),
            AutostartError::InsecurePermissions
        );
        assert!(!entry_path(&temp).exists());
    }

    #[test]
    fn secure_directories_are_created_0700() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        set_enabled(&path, true).expect("activation");
        let autostart = path.parent().expect("parent");
        let config = autostart.parent().expect("config");
        for directory in [config, autostart] {
            let mode = std::fs::metadata(directory)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "répertoire: {}", directory.display());
        }
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[test]
    fn entry_requires_type_and_expected_exec() {
        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dossier");
        for content in [
            "[Desktop Entry]\nX-GNOME-Autostart-enabled=true\n",
            "[Desktop Entry]\nType=Application\n",
            "[Desktop Entry]\nExec=/usr/bin/tuxedo-rust-center --tray\n",
            "[Desktop Entry]\nType=Application\nExec=/usr/bin/other --tray\n",
        ] {
            std::fs::write(&path, content).expect("écriture");
            assert!(
                !is_enabled(&path).expect("lecture"),
                "contenu inattendu accepté: {content}"
            );
        }
        std::fs::write(&path, desktop_entry()).expect("écriture");
        assert!(is_enabled(&path).expect("lecture"));
    }

    #[test]
    fn temporary_file_is_removed_on_write_failure() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        let autostart = path.parent().expect("parent");
        std::fs::create_dir_all(autostart).expect("dossier");
        std::fs::set_permissions(autostart, std::fs::Permissions::from_mode(0o500))
            .expect("dossier non inscriptible");
        assert_eq!(
            set_enabled(&path, true).unwrap_err(),
            AutostartError::WriteFailed
        );
        let entries: Vec<_> = std::fs::read_dir(autostart)
            .expect("lecture")
            .filter_map(Result::ok)
            .collect();
        assert!(entries.is_empty(), "fichier temporaire laissé: {entries:?}");
        std::fs::set_permissions(autostart, std::fs::Permissions::from_mode(0o700))
            .expect("restauration");
    }

    #[test]
    fn symlinks_and_directories_are_refused() {
        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dossier");
        // Dossier à la place du fichier.
        std::fs::create_dir(&path).expect("dossier cible");
        assert_eq!(is_enabled(&path).unwrap_err(), AutostartError::PathInvalid);
        assert_eq!(
            set_enabled(&path, true).unwrap_err(),
            AutostartError::PathInvalid
        );
        assert_eq!(
            set_enabled(&path, false).unwrap_err(),
            AutostartError::PathInvalid
        );
        std::fs::remove_dir(&path).expect("nettoyage");

        // Lien symbolique refusé (pas suivi, pas remplacé silencieusement).
        let target = temp.path().join("target.desktop");
        std::fs::write(&target, desktop_entry()).expect("cible");
        std::os::unix::fs::symlink(&target, &path).expect("symlink");
        assert_eq!(is_enabled(&path).unwrap_err(), AutostartError::PathInvalid);
        assert_eq!(
            set_enabled(&path, true).unwrap_err(),
            AutostartError::PathInvalid
        );
    }

    #[test]
    fn config_home_resolution_is_strict() {
        assert_eq!(
            resolve_config_home(None, Some(OsStr::new("/home/alice"))).expect("défaut"),
            Path::new("/home/alice/.config")
        );
        assert_eq!(
            resolve_config_home(
                Some(OsStr::new("/home/alice/.config-x")),
                Some(OsStr::new("/home/alice"))
            )
            .expect("xdg"),
            Path::new("/home/alice/.config-x")
        );
        // Chemin relatif : ignoré, repli sur ~/.config.
        assert_eq!(
            resolve_config_home(
                Some(OsStr::new("relative")),
                Some(OsStr::new("/home/alice"))
            )
            .expect("relatif ignoré"),
            Path::new("/home/alice/.config")
        );
        // Hors du répertoire utilisateur : refusé.
        assert_eq!(
            resolve_config_home(
                Some(OsStr::new("/etc/xdg")),
                Some(OsStr::new("/home/alice"))
            )
            .unwrap_err(),
            AutostartError::PathInvalid
        );
        // `..` refusé.
        assert_eq!(
            resolve_config_home(
                Some(OsStr::new("/home/alice/../etc")),
                Some(OsStr::new("/home/alice"))
            )
            .unwrap_err(),
            AutostartError::PathInvalid
        );
        // HOME absent ou relatif.
        assert_eq!(
            resolve_config_home(None, None).unwrap_err(),
            AutostartError::NoHome
        );
        assert_eq!(
            resolve_config_home(None, Some(OsStr::new("relative"))).unwrap_err(),
            AutostartError::NoHome
        );
        assert_eq!(
            resolve_config_home(
                Some(OsStr::new("/home/alice")),
                Some(OsStr::new("/home/alice"))
            )
            .expect("racine home"),
            Path::new("/home/alice")
        );
        assert!(
            resolve_config_home(
                Some(OsStr::new("/home/alice")),
                Some(OsStr::new("/home/alice"))
            )
            .is_ok()
        );
    }

    #[test]
    fn autostart_path_validation() {
        assert_eq!(
            autostart_path(Path::new("/home/alice/.config")).expect("valide"),
            Path::new("/home/alice/.config/autostart/tuxedo-rust-center.desktop")
        );
        for path in ["relative", "/home/alice/../etc", ""] {
            assert_eq!(
                autostart_path(Path::new(path)).unwrap_err(),
                AutostartError::PathInvalid,
                "chemin: {path}"
            );
        }
    }

    #[test]
    fn oversized_entry_is_refused() {
        let temp = TempDir::new().expect("tempdir");
        let path = entry_path(&temp);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dossier");
        std::fs::write(&path, "x".repeat(64 * 1024 + 1)).expect("écriture");
        assert_eq!(is_enabled(&path).unwrap_err(), AutostartError::ReadFailed);
    }
}
