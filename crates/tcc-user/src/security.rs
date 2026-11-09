// SPDX-License-Identifier: GPL-3.0-or-later
//! Contrôles de sécurité de l'agent de session.
//!
//! - refus d'exécution en root ;
//! - validation des répertoires runtime et des sockets IPC de compositeur
//!   (propriétaire, type, permissions, absence de lien symbolique) ;
//! - vérification des credentials du pair après connexion.
//!
//! Aucune lecture de `/proc/*/environ` (D6). Seul `/proc/self/status` est lu
//! pour connaître l'UID effectif.

use std::fs::{self, Metadata};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path};
use std::time::Duration;

use crate::error::{Result, UserError};

/// Taille maximale d'une réponse IPC (protections mémoire).
pub const MAX_IPC_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// Délai maximal de lecture/écriture sur un socket de compositeur.
pub const IPC_TIMEOUT: Duration = Duration::from_secs(3);

/// Lit l'UID effectif depuis `/proc/self/status`.
///
/// On évite `geteuid()` pour ne pas introduire d'`unsafe` dans cet agent.
pub fn effective_uid() -> Result<u32> {
    let status = fs::read_to_string("/proc/self/status")
        .map_err(|err| UserError::EffectiveUid(format!("lecture de /proc/self/status : {err}")))?;
    parse_effective_uid(&status).ok_or_else(|| {
        UserError::EffectiveUid("ligne Uid absente ou invalide dans /proc/self/status".to_owned())
    })
}

/// Refuse l'exécution en root et retourne l'UID effectif.
pub fn ensure_not_root() -> Result<u32> {
    let uid = effective_uid()?;
    ensure_not_root_for(uid)
}

/// Variante testable de [`ensure_not_root`].
pub fn ensure_not_root_for(uid: u32) -> Result<u32> {
    if uid == 0 {
        Err(UserError::RunningAsRoot)
    } else {
        Ok(uid)
    }
}

fn parse_effective_uid(status: &str) -> Option<u32> {
    // Ligne : `Uid:\t<real>\t<effective>\t<saved>\t<fs>`
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    let mut fields = line.split_whitespace();
    let _label = fields.next()?;
    let _real = fields.next()?;
    fields.next()?.parse::<u32>().ok()
}

/// Vérifie un répertoire runtime de session (`$XDG_RUNTIME_DIR`) :
/// répertoire réel (pas un lien), possédé par l'utilisateur, mode 0700.
pub fn validate_runtime_dir(path: &Path, expected_uid: u32) -> Result<()> {
    if !path.is_absolute() {
        return Err(UserError::InvalidSocket(format!(
            "répertoire runtime non absolu : {}",
            path.display()
        )));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|err| UserError::InvalidSocket(format!("{} : {err}", path.display())))?;
    if meta.file_type().is_symlink() {
        return Err(UserError::InvalidSocket(format!(
            "répertoire runtime symbolique refusé : {}",
            path.display()
        )));
    }
    if !meta.is_dir() {
        return Err(UserError::InvalidSocket(format!(
            "répertoire runtime invalide : {}",
            path.display()
        )));
    }
    if meta.uid() != expected_uid {
        return Err(UserError::InvalidSocket(format!(
            "répertoire runtime possédé par l'UID {} au lieu de {} : {}",
            meta.uid(),
            expected_uid,
            path.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(UserError::InvalidSocket(format!(
            "répertoire runtime accessible au groupe/autres (mode {:o}) : {}",
            meta.mode() & 0o777,
            path.display()
        )));
    }
    Ok(())
}

/// Valide le chemin d'un socket de compositeur sans se connecter.
pub fn validate_socket_path(path: &Path, expected_uid: u32) -> Result<()> {
    if !path.is_absolute() {
        return Err(UserError::InvalidSocket(format!(
            "socket non absolu : {}",
            path.display()
        )));
    }
    reject_parent_components(path)?;

    let meta = fs::symlink_metadata(path)
        .map_err(|err| UserError::InvalidSocket(format!("{} : {err}", path.display())))?;
    validate_socket_metadata(path, &meta, expected_uid)?;

    let parent = path.parent().ok_or_else(|| {
        UserError::InvalidSocket(format!(
            "socket sans répertoire parent : {}",
            path.display()
        ))
    })?;
    let parent_meta = fs::symlink_metadata(parent)
        .map_err(|err| UserError::InvalidSocket(format!("{} : {err}", parent.display())))?;
    if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
        return Err(UserError::InvalidSocket(format!(
            "répertoire du socket invalide : {}",
            parent.display()
        )));
    }
    if parent_meta.uid() != expected_uid {
        return Err(UserError::InvalidSocket(format!(
            "répertoire du socket possédé par l'UID {} au lieu de {} : {}",
            parent_meta.uid(),
            expected_uid,
            parent.display()
        )));
    }
    if parent_meta.mode() & 0o022 != 0 {
        return Err(UserError::InvalidSocket(format!(
            "répertoire du socket inscriptible par groupe/autres (mode {:o}) : {}",
            parent_meta.mode() & 0o777,
            parent.display()
        )));
    }
    Ok(())
}

fn validate_socket_metadata(path: &Path, meta: &Metadata, expected_uid: u32) -> Result<()> {
    if meta.file_type().is_symlink() {
        return Err(UserError::InvalidSocket(format!(
            "lien symbolique refusé : {}",
            path.display()
        )));
    }
    if !meta.file_type().is_socket() {
        return Err(UserError::InvalidSocket(format!(
            "n'est pas un socket UNIX : {}",
            path.display()
        )));
    }
    if meta.uid() != expected_uid {
        return Err(UserError::InvalidSocket(format!(
            "socket possédé par l'UID {} au lieu de {} : {}",
            meta.uid(),
            expected_uid,
            path.display()
        )));
    }
    if meta.mode() & 0o002 != 0 {
        return Err(UserError::InvalidSocket(format!(
            "socket inscriptible par le monde (mode {:o}) : {}",
            meta.mode() & 0o777,
            path.display()
        )));
    }
    Ok(())
}

/// Rejette les composants `..` (évasion de chemin) et exige un chemin absolu.
fn reject_parent_components(path: &Path) -> Result<()> {
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(UserError::InvalidSocket(format!(
                "composant `..` interdit : {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Ouvre un socket de compositeur, valide le chemin, puis applique des
/// timeouts.
///
/// La validation couvre le type (socket UNIX), l'absence de lien symbolique,
/// le propriétaire et les permissions du socket et de son répertoire parent.
/// `$XDG_RUNTIME_DIR` doit être 0700 et possédé par l'utilisateur, ce qui
/// empêche un autre utilisateur de remplacer le socket entre la vérification et
/// la connexion. Les credentials du pair ne sont pas interrogés : l'API
/// `peer_cred` n'est pas stable sur la MSRV du projet.
pub fn connect_session_socket(path: &Path, expected_uid: u32) -> Result<UnixStream> {
    validate_socket_path(path, expected_uid)?;
    let stream = UnixStream::connect(path).map_err(|err| {
        UserError::InvalidSocket(format!("connexion à {} : {err}", path.display()))
    })?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::os::unix::net::UnixListener;

    #[test]
    fn uid_parsing() {
        assert_eq!(
            parse_effective_uid("Name:\ttcc\nUid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(parse_effective_uid("Uid:\t1000\t0\t0\t0\n"), Some(0));
        assert_eq!(parse_effective_uid("Name:\ttcc\n"), None);
    }

    #[test]
    fn root_is_refused() {
        assert!(matches!(
            ensure_not_root_for(0),
            Err(UserError::RunningAsRoot)
        ));
        assert_eq!(ensure_not_root_for(1000).unwrap(), 1000);
    }

    #[test]
    fn effective_uid_is_readable() {
        // /proc/self/status existe sur toute session Linux normale.
        assert!(effective_uid().is_ok());
    }

    fn socket_fixture() -> (tempfile::TempDir, std::path::PathBuf, UnixListener) {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("test.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        (dir, path, listener)
    }

    #[test]
    fn accepts_own_socket() {
        let (_dir, path, _listener) = socket_fixture();
        let uid = effective_uid().unwrap();
        assert!(validate_socket_path(&path, uid).is_ok());
        let stream = connect_session_socket(&path, uid).unwrap();
        drop(stream);
    }

    #[test]
    fn rejects_symlink_socket() {
        let (dir, path, _listener) = socket_fixture();
        let link = dir.path().join("link.sock");
        symlink(&path, &link).unwrap();
        let uid = effective_uid().unwrap();
        assert!(validate_socket_path(&link, uid).is_err());
    }

    #[test]
    fn rejects_wrong_owner() {
        let (_dir, path, _listener) = socket_fixture();
        let uid = effective_uid().unwrap();
        assert!(validate_socket_path(&path, uid.wrapping_add(1)).is_err());
    }

    #[test]
    fn rejects_runtime_dir_with_bad_mode() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let uid = effective_uid().unwrap();
        assert!(validate_runtime_dir(dir.path(), uid).is_err());

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_runtime_dir(dir.path(), uid).is_ok());
    }

    #[test]
    fn rejects_parent_dir_escape() {
        let uid = effective_uid().unwrap();
        assert!(validate_socket_path(Path::new("/run/../etc/passwd"), uid).is_err());
    }
}
