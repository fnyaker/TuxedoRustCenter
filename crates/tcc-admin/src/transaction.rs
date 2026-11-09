// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Transaction de persistance de `tccd-admin` (AD-5, AD-6).
//!
//! Séquence appliquée à chaque écriture :
//!
//! 1. lire l'ancienne version (≤ 1 Mio, `O_NOFOLLOW`) pour pouvoir revenir ;
//! 2. écrire via `ConfigStore::save_*_with_backup` (`.bak` 0600, cible 0644,
//!    `fsync` + `rename`) ;
//! 3. demander `Control1.ReloadConfig` à `tccd` ;
//! 4. si le rapport n'est pas `ok`, restaurer l'ancien contenu par écriture
//!    atomique puis redemander un rechargement.
//!
//! Aucun chemin ne provient d'un client : la cible est toujours
//! `ConfigStore::paths()`.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tcc_core::config::{DEFAULT_MAX_CONFIG_BYTES, LoadPolicy};
use tcc_core::{ConfigStore, ProfileCollection, Settings};
use tcc_ipc::AdminReloadReport;

use crate::error::AdminError;

/// Taille maximale d'un payload ou d'une ancienne version relue.
pub const MAX_PAYLOAD_BYTES: usize = DEFAULT_MAX_CONFIG_BYTES as usize;

/// Notification de rechargement vers `tccd`.
///
/// Le contrat est volontairement minimal : un succès porte le rapport
/// `AdminReloadReport`, une erreur de transport ne contient aucun détail
/// exploitable par un client.
#[async_trait]
pub trait ReloadNotifier: Send + Sync {
    /// Demande à `tccd` de relire la configuration.
    ///
    /// # Errors
    ///
    /// [`ReloadError`] si le service est absent, en timeout ou si la réponse
    /// est invalide.
    async fn reload(&self) -> Result<AdminReloadReport, ReloadError>;
}

/// Échec de transport vers `Control1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("tccd n'a pas pu recharger la configuration")]
pub struct ReloadError;

/// État de la sauvegarde `.bak` après une écriture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupStatus {
    /// Aucune sauvegarde (fichier absent, contenu identique, ou précédent
    /// inexploitable sans `.bak` existant).
    None,
    /// Une nouvelle sauvegarde a été écrite.
    Written,
    /// Un `.bak` sain existait déjà et a été conservé : le contenu courant
    /// remplacé n'était pas exploitable (A1-07).
    Preserved,
}

impl BackupStatus {
    /// Libellé stable pour l'audit (`backup=…`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Written => "written",
            Self::Preserved => "preserved",
        }
    }
}

/// Résultat d'une écriture acceptée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedInfo {
    /// Taille du payload validé (jamais son contenu).
    pub bytes: usize,
    /// État de la sauvegarde `.bak`.
    pub backup: BackupStatus,
    /// Faux si le payload a été accepté sans écriture (presets uniquement).
    pub applied: bool,
}

impl SavedInfo {
    /// Libellé d'audit : `ok` si un rechargement a eu lieu, `skipped` pour un
    /// no-op légitime. Ne jamais annoncer `ok` sans rechargement (A1-05).
    #[must_use]
    pub const fn reload_label(&self) -> &'static str {
        if self.applied { "ok" } else { "skipped" }
    }
}

/// Type de fichier relu avant écriture, pour valider le précédent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviousKind {
    Settings,
    Profiles,
}

impl PreviousKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Profiles => "profiles",
        }
    }

    /// Vrai si les octets sont un document valide du type cible.
    fn is_usable(self, bytes: &[u8]) -> bool {
        match self {
            Self::Settings => serde_json::from_slice::<Settings>(bytes)
                .is_ok_and(|value| value.validate().is_ok()),
            Self::Profiles => serde_json::from_slice::<ProfileCollection>(bytes)
                .is_ok_and(|value| value.validate().is_ok()),
        }
    }
}

fn backup_status(saved: &tcc_core::Saved) -> BackupStatus {
    if saved.backup.is_some() {
        BackupStatus::Written
    } else if saved.preserved_backup.is_some() {
        BackupStatus::Preserved
    } else {
        BackupStatus::None
    }
}

/// État d'un fichier avant la transaction, pour un rollback exact.
#[derive(Debug, Clone)]
struct RestoreTarget {
    path: std::path::PathBuf,
    /// Vrai si le fichier existait avant l'écriture.
    existed: bool,
    /// Octets exacts à restaurer, ou `None` si le fichier était absent ou si
    /// son contenu ne doit pas être restauré (transaction mono-fichier avec
    /// précédent illisible, A1-07).
    previous: Option<Vec<u8>>,
    /// Supprimer le fichier au rollback s'il était absent avant (T1-01).
    ///
    /// Activé uniquement pour la transaction double fichier : si
    /// `settings.json` n'existait pas et que le rechargement échoue, le
    /// fichier créé doit disparaître pour ne pas laisser un tombstone
    /// partiel. La transaction mono-fichier conserve volontairement le
    /// nouveau fichier valide (`failed_save_without_previous_keeps_valid_file`).
    remove_if_absent: bool,
}

/// Résultat d'un rollback.
#[derive(Debug, Clone, Copy, Default)]
struct RollbackOutcome {
    /// Au moins un fichier a été restauré ou supprimé.
    restored: bool,
    /// Au moins une restauration a échoué.
    failed: bool,
}

/// Sauvegarde les réglages puis recharge `tccd`, avec rollback.
///
/// # Errors
///
/// Voir [`AdminError`]. En cas de `ReloadFailed`, l'ancien contenu a été
/// restauré si une version précédente existait.
pub async fn save_settings(
    store: &ConfigStore,
    reloader: &dyn ReloadNotifier,
    settings: &Settings,
    payload_bytes: usize,
) -> Result<SavedInfo, AdminError> {
    let path = store.paths().settings.clone();
    let policy = store.policy().clone();
    let previous = read_previous_blocking(path, policy, PreviousKind::Settings).await?;
    let store = store.clone();
    let settings = settings.clone();
    let saved = tokio::task::spawn_blocking(move || store.save_settings_with_backup(&settings))
        .await
        .map_err(|_| AdminError::WriteFailed)?
        .map_err(|_| AdminError::WriteFailed)?;
    let backup = backup_status(&saved);
    if backup == BackupStatus::Preserved {
        tracing::warn!(
            kind = "settings",
            backup = backup.as_str(),
            "contenu courant non exploitable : sauvegarde .bak saine conservée"
        );
    }
    commit(&[previous], payload_bytes, backup, reloader).await
}

/// Sauvegarde les profils puis recharge `tccd`, avec rollback.
///
/// # Errors
///
/// Voir [`save_settings`].
pub async fn save_profiles(
    store: &ConfigStore,
    reloader: &dyn ReloadNotifier,
    profiles: &ProfileCollection,
    payload_bytes: usize,
) -> Result<SavedInfo, AdminError> {
    let path = store.paths().profiles.clone();
    let policy = store.policy().clone();
    let previous = read_previous_blocking(path, policy, PreviousKind::Profiles).await?;
    let store = store.clone();
    let profiles = profiles.clone();
    let saved = tokio::task::spawn_blocking(move || store.save_profiles_with_backup(&profiles))
        .await
        .map_err(|_| AdminError::WriteFailed)?
        .map_err(|_| AdminError::WriteFailed)?;
    let backup = backup_status(&saved);
    if backup == BackupStatus::Preserved {
        tracing::warn!(
            kind = "profiles",
            backup = backup.as_str(),
            "contenu courant non exploitable : sauvegarde .bak saine conservée"
        );
    }
    commit(&[previous], payload_bytes, backup, reloader).await
}

/// Sauvegarde les profils **et** les réglages dans une seule transaction.
///
/// Utilisé quand une suppression de profil ajoute des tombstones : les deux
/// fichiers doivent être appliqués ensemble, ou restaurés ensemble si `tccd`
/// refuse le rechargement. Les profils sont écrits en premier ; si l'écriture
/// des réglages échoue, les profils sont restaurés.
///
/// Un seul rechargement est demandé après l'écriture des deux fichiers.
///
/// # Errors
///
/// Voir [`save_settings`]. Une écriture partielle est annulée avant de
/// renvoyer [`AdminError::WriteFailed`].
pub async fn save_profiles_with_settings(
    store: &ConfigStore,
    reloader: &dyn ReloadNotifier,
    profiles: &ProfileCollection,
    settings: &Settings,
    payload_bytes: usize,
) -> Result<SavedInfo, AdminError> {
    let profiles_path = store.paths().profiles.clone();
    let settings_path = store.paths().settings.clone();
    let policy = store.policy().clone();
    let previous_profiles = read_previous_target_blocking(
        profiles_path,
        policy.clone(),
        PreviousKind::Profiles,
        true,
        true,
    )
    .await?;
    let previous_settings =
        read_previous_target_blocking(settings_path, policy, PreviousKind::Settings, true, true)
            .await?;

    let store_for_write = store.clone();
    let profiles_for_write = profiles.clone();
    let settings_for_write = settings.clone();
    let written = tokio::task::spawn_blocking(move || {
        let profiles_saved = store_for_write
            .save_profiles_with_backup(&profiles_for_write)
            .map_err(|_| ())?;
        let settings_saved = store_for_write
            .save_settings_with_backup(&settings_for_write)
            .map_err(|_| ())?;
        Ok::<_, ()>((profiles_saved, settings_saved))
    })
    .await
    .map_err(|_| AdminError::WriteFailed)?;

    let (profiles_saved, settings_saved) = match written {
        Ok(saved) => saved,
        Err(()) => {
            // L'écriture du second fichier a échoué : elle a pu laisser
            // `settings.json` dans un état inconnu (rename suivi d'une erreur
            // de synchronisation). Restaure les **deux** fichiers à leur état
            // initial, y compris en supprimant ceux qui étaient absents.
            let _ = rollback(&[previous_profiles, previous_settings]).await;
            return Err(AdminError::WriteFailed);
        }
    };

    let profiles_backup = backup_status(&profiles_saved);
    let settings_backup = backup_status(&settings_saved);
    for (kind, backup) in [("profiles", profiles_backup), ("settings", settings_backup)] {
        if backup == BackupStatus::Preserved {
            tracing::warn!(
                kind,
                backup = backup.as_str(),
                "contenu courant non exploitable : sauvegarde .bak saine conservée"
            );
        }
    }
    let backup = merge_backup(profiles_backup, settings_backup);

    commit(
        &[previous_profiles, previous_settings],
        payload_bytes,
        backup,
        reloader,
    )
    .await
}

/// Fusionne les états de sauvegarde de deux écritures : `written` prime sur
/// `preserved`, qui prime sur `none`.
const fn merge_backup(first: BackupStatus, second: BackupStatus) -> BackupStatus {
    if matches!(first, BackupStatus::Written) || matches!(second, BackupStatus::Written) {
        BackupStatus::Written
    } else if matches!(first, BackupStatus::Preserved) || matches!(second, BackupStatus::Preserved)
    {
        BackupStatus::Preserved
    } else {
        BackupStatus::None
    }
}

async fn restore(path: &Path, bytes: &[u8]) -> Result<(), AdminError> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || restore_previous(&path, &bytes))
        .await
        .map_err(|_| AdminError::WriteFailed)?
}

/// Supprime un fichier créé par la transaction, de façon durable.
fn remove_file(path: &Path) -> Result<(), AdminError> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent()
                && let Ok(directory) = File::open(parent)
            {
                directory.sync_all().map_err(|_| AdminError::WriteFailed)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AdminError::WriteFailed),
    }
}

async fn remove_if_exists(path: &Path) -> Result<(), AdminError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || remove_file(&path))
        .await
        .map_err(|_| AdminError::WriteFailed)?
}

/// Restaure une cible. Renvoie `true` si une action a été effectuée.
async fn restore_target(target: &RestoreTarget) -> Result<bool, AdminError> {
    if let Some(bytes) = &target.previous {
        restore(&target.path, bytes).await?;
        Ok(true)
    } else if !target.existed && target.remove_if_absent {
        remove_if_exists(&target.path).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Applique le rollback à toutes les cibles, même si l'une échoue.
async fn rollback(targets: &[RestoreTarget]) -> RollbackOutcome {
    let mut outcome = RollbackOutcome::default();
    for target in targets {
        match restore_target(target).await {
            Ok(performed) => outcome.restored |= performed,
            Err(error) => {
                outcome.failed = true;
                tracing::error!(%error, "restauration de la configuration impossible");
            }
        }
    }
    outcome
}

async fn commit(
    targets: &[RestoreTarget],
    payload_bytes: usize,
    backup: BackupStatus,
    reloader: &dyn ReloadNotifier,
) -> Result<SavedInfo, AdminError> {
    let report = reloader.reload().await;
    if let Ok(report) = &report
        && report.is_ok()
    {
        return Ok(SavedInfo {
            bytes: payload_bytes,
            backup,
            applied: true,
        });
    }

    tracing::warn!(
        reload = if report.is_ok() { "warning" } else { "error" },
        "rechargement refusé : rollback de la configuration"
    );

    let outcome = rollback(targets).await;

    if outcome.restored {
        // Après restauration, l'ancien contenu est sur disque. Une seconde
        // demande de rechargement est un effort de resynchronisation : son
        // échec ne remet pas en cause le rollback, qui a déjà réussi.
        match reloader.reload().await {
            Ok(resync) if resync.is_ok() => {}
            Ok(_) => tracing::warn!("rechargement après rollback avec avertissements"),
            Err(_) => tracing::warn!("rechargement après rollback indisponible"),
        }
    }

    if outcome.failed {
        // Un fichier au moins n'a pas pu être restauré : l'état disque est
        // potentiellement mixte, ce qui est plus grave qu'un rechargement
        // refusé.
        Err(AdminError::WriteFailed)
    } else {
        // Rollback complet : soit un contenu précédent a été restauré, soit
        // des fichiers créés ont été supprimés. En transaction mono-fichier
        // sans précédent, le nouveau fichier reste valide et sera chargé au
        // prochain rechargement.
        Err(AdminError::ReloadFailed)
    }
}

/// Enveloppe [`read_previous`] dans `spawn_blocking` (I/O disque).
///
/// Transaction mono-fichier : un précédent illisible n'est pas restauré
/// (A1-07) et une absence n'est jamais transformée en suppression.
async fn read_previous_blocking(
    path: std::path::PathBuf,
    policy: LoadPolicy,
    kind: PreviousKind,
) -> Result<RestoreTarget, AdminError> {
    read_previous_target_blocking(path, policy, kind, false, false).await
}

/// Lit l'état initial d'un fichier pour le rollback.
///
/// `restore_unusable` force la restauration des octets exacts même si le
/// contenu n'est pas exploitable (transaction double fichier, T1-01).
/// `remove_if_absent` autorise la suppression du fichier créé si la cible
/// n'existait pas avant la transaction.
async fn read_previous_target_blocking(
    path: std::path::PathBuf,
    policy: LoadPolicy,
    kind: PreviousKind,
    restore_unusable: bool,
    remove_if_absent: bool,
) -> Result<RestoreTarget, AdminError> {
    tokio::task::spawn_blocking(move || {
        let read_path = path.clone();
        let previous = read_previous(&read_path, &policy)?;
        match previous {
            Some(bytes) => {
                let usable = kind.is_usable(&bytes);
                if !usable && !restore_unusable {
                    // Un précédent corrompu ne doit jamais être restauré après
                    // un échec de rechargement. Le `.bak` sain éventuel reste
                    // géré par `ConfigStore` (A1-07).
                    tracing::warn!(
                        kind = kind.as_str(),
                        "contenu précédent illisible : rollback désactivé"
                    );
                }
                Ok(RestoreTarget {
                    path,
                    existed: true,
                    previous: (usable || restore_unusable).then_some(bytes),
                    remove_if_absent,
                })
            }
            None => Ok(RestoreTarget {
                path,
                existed: false,
                previous: None,
                remove_if_absent,
            }),
        }
    })
    .await
    .map_err(|_| AdminError::WriteFailed)?
}

/// Lit l'ancienne version, bornée et sans suivre de lien symbolique.
///
/// Réplique les contrôles de `ConfigStore` (A1-04) : fichier régulier,
/// propriétaire attendu par la politique, pas d'écriture groupe/monde. Le fd
/// est ouvert avec `O_CLOEXEC` : il ne sera jamais hérité par un futur
/// processus enfant.
fn read_previous(path: &Path, policy: &LoadPolicy) -> Result<Option<Vec<u8>>, AdminError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AdminError::WriteFailed),
    };
    let metadata = file.metadata().map_err(|_| AdminError::WriteFailed)?;
    if !metadata.file_type().is_file() {
        return Err(AdminError::WriteFailed);
    }
    if let Some(expected) = policy.expected_owner
        && metadata.uid() != expected
    {
        return Err(AdminError::WriteFailed);
    }
    if policy.enforce_permissions && metadata.mode() & 0o022 != 0 {
        return Err(AdminError::WriteFailed);
    }
    if metadata.len() > policy.max_file_bytes {
        return Err(AdminError::WriteFailed);
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(policy.max_file_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| AdminError::WriteFailed)?;
    if bytes.len() as u64 > policy.max_file_bytes {
        return Err(AdminError::WriteFailed);
    }
    Ok(Some(bytes))
}

static RESTORE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Restaure exactement les anciens octets par écriture atomique.
///
/// Le temporaire est créé en `0600`, jamais suivi s'il existe déjà. Son mode
/// est passé à `0644` sur le descripteur (`fchmod`) avant `fsync`/`rename`,
/// comme `ConfigStore::write_atomic`.
fn restore_previous(path: &Path, bytes: &[u8]) -> Result<(), AdminError> {
    let parent = path.parent().ok_or(AdminError::WriteFailed)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(AdminError::WriteFailed)?;

    for _ in 0..8 {
        let counter = RESTORE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{file_name}.restore.{}.{counter}",
            std::process::id()
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(AdminError::WriteFailed),
        };

        let result = (|| -> std::io::Result<()> {
            file.write_all(bytes)?;
            // `fchmod` sur le fd avant `rename`, comme `ConfigStore` : la
            // cible naît en 0644, sans fenêtre 0600 ni dépendance à l'umask.
            file.set_permissions(fs::Permissions::from_mode(0o644))?;
            file.sync_all()?;
            fs::rename(&temp, path)?;
            // `sync_all` sur le répertoire rend le renommage durable.
            if let Ok(directory) = File::open(parent) {
                directory.sync_all()?;
            }
            Ok(())
        })();

        if let Err(error) = result {
            let _ = fs::remove_file(&temp);
            tracing::error!(%error, "restauration de la configuration impossible");
            return Err(AdminError::WriteFailed);
        }
        return Ok(());
    }
    Err(AdminError::WriteFailed)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use tcc_core::config::LoadPolicy;

    use super::{MAX_PAYLOAD_BYTES, read_previous, restore_previous};
    use crate::error::AdminError;

    #[test]
    fn audit_label_distinguishes_noop_and_write() {
        let noop = super::SavedInfo {
            bytes: 10,
            backup: super::BackupStatus::None,
            applied: false,
        };
        assert_eq!(noop.reload_label(), "skipped");
        assert_eq!(super::BackupStatus::None.as_str(), "none");
        assert_eq!(super::BackupStatus::Written.as_str(), "written");
        assert_eq!(super::BackupStatus::Preserved.as_str(), "preserved");
        let written = super::SavedInfo {
            bytes: 10,
            backup: super::BackupStatus::Written,
            applied: true,
        };
        assert_eq!(written.reload_label(), "ok");
    }

    #[test]
    fn payload_limit_matches_one_mebibyte() {
        assert_eq!(MAX_PAYLOAD_BYTES, 1024 * 1024);
    }

    #[test]
    fn previous_file_is_read_and_absent_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        assert_eq!(
            read_previous(&path, &LoadPolicy::default()).expect("absent"),
            None
        );

        std::fs::write(&path, b"{\"ok\":true}").expect("écriture");
        let bytes = read_previous(&path, &LoadPolicy::default())
            .expect("lecture")
            .expect("contenu");
        assert_eq!(bytes, b"{\"ok\":true}");
    }

    #[test]
    fn symlink_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.json");
        std::fs::write(&target, b"secret").expect("cible");
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&target, &link).expect("lien");
        assert_eq!(
            read_previous(&link, &LoadPolicy::default()),
            Err(AdminError::WriteFailed)
        );
    }

    #[test]
    fn oversize_previous_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let file = std::fs::File::create(&path).expect("création");
        file.set_len(MAX_PAYLOAD_BYTES as u64 + 1).expect("taille");
        drop(file);
        assert_eq!(
            read_previous(&path, &LoadPolicy::default()),
            Err(AdminError::WriteFailed)
        );
    }

    #[test]
    fn group_writable_previous_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{}").expect("écriture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))
            .expect("permissions");
        assert_eq!(
            read_previous(&path, &LoadPolicy::default()),
            Err(AdminError::WriteFailed)
        );
    }

    #[test]
    fn previous_with_unexpected_owner_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{}").expect("écriture");
        let uid = std::fs::metadata(&path).expect("metadata").uid();
        let policy = LoadPolicy {
            expected_owner: Some(uid.wrapping_add(1)),
            ..LoadPolicy::default()
        };
        assert_eq!(read_previous(&path, &policy), Err(AdminError::WriteFailed));

        let accepted = LoadPolicy {
            expected_owner: Some(uid),
            ..LoadPolicy::default()
        };
        assert_eq!(
            read_previous(&path, &accepted).expect("lecture"),
            Some(b"{}".to_vec())
        );
    }

    #[test]
    fn restore_rewrites_exact_bytes_and_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("profiles.json");
        std::fs::write(&path, b"nouveau").expect("écriture");
        restore_previous(&path, b"ancien").expect("restauration");
        assert_eq!(std::fs::read(&path).expect("lecture"), b"ancien");
        let mode = std::fs::metadata(&path)
            .expect("métadonnées")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
        // Aucun temporaire résiduel.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("répertoire")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".restore."))
            .collect();
        assert!(leftovers.is_empty());
    }
}
