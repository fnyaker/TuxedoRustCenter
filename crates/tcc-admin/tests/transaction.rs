// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests d'intégration de la transaction admin : `.bak`, permissions,
//! rollback exact et échec de transport.

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tcc_admin::AdminError;
use tcc_admin::transaction::{
    BackupStatus, ReloadError, ReloadNotifier, save_profiles, save_profiles_with_settings,
    save_settings,
};
use tcc_core::{ConfigStore, Profile, ProfileCollection, ProfileId, Settings};
use tcc_ipc::AdminReloadReport;

struct FakeReloader {
    report: Result<AdminReloadReport, ReloadError>,
    calls: AtomicUsize,
}

impl FakeReloader {
    fn clean() -> Self {
        Self {
            report: Ok(AdminReloadReport::clean()),
            calls: AtomicUsize::new(0),
        }
    }

    fn warning() -> Self {
        Self {
            report: Ok(AdminReloadReport::from_warnings(vec![
                "configuration invalide".to_owned(),
            ])),
            calls: AtomicUsize::new(0),
        }
    }

    fn unavailable() -> Self {
        Self {
            report: Err(ReloadError),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ReloadNotifier for FakeReloader {
    async fn reload(&self) -> Result<AdminReloadReport, ReloadError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.report.clone()
    }
}

fn store(root: &std::path::Path) -> ConfigStore {
    ConfigStore::with_admin_roots(root, root.join("admin"))
        .with_policy(tcc_core::config::LoadPolicy::default())
        .with_write_root(root)
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path)
        .expect("métadonnées")
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn successful_save_writes_backup_0600() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let old = Settings::default();
    store.save_settings(&old).expect("settings initiaux");
    let old_bytes = std::fs::read(dir.path().join("settings.json")).expect("avant");

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let reloader = FakeReloader::clean();
    let saved = save_settings(&store, &reloader, &new, 123)
        .await
        .expect("écriture");
    assert_eq!(saved.bytes, 123);
    assert_eq!(saved.backup, BackupStatus::Written);

    let target = dir.path().join("settings.json");
    let backup = dir.path().join("settings.json.bak");
    assert_eq!(mode(&target), 0o644);
    assert_eq!(mode(&backup), 0o600);
    assert_eq!(std::fs::read(&backup).expect("bak"), old_bytes);
    let written: Settings =
        serde_json::from_str(&std::fs::read_to_string(&target).expect("après")).expect("JSON");
    assert!(written.fahrenheit);
    assert_eq!(reloader.calls(), 1);
}

#[tokio::test]
async fn successful_profiles_save_writes_backup_0600() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let old = ProfileCollection::new(vec![Profile::new("custom-old", "Ancien")]);
    store.save_profiles(&old).expect("profils initiaux");
    let old_bytes = std::fs::read(dir.path().join("profiles.json")).expect("avant");

    let new = ProfileCollection::new(vec![
        Profile::new("custom-old", "Ancien"),
        Profile::new("custom-new", "Nouveau"),
    ]);
    let reloader = FakeReloader::clean();
    let saved = save_profiles(&store, &reloader, &new, 456)
        .await
        .expect("écriture");
    assert_eq!(saved.bytes, 456);
    assert_eq!(saved.backup, BackupStatus::Written);

    let target = dir.path().join("profiles.json");
    let backup = dir.path().join("profiles.json.bak");
    assert_eq!(mode(&target), 0o644);
    assert_eq!(mode(&backup), 0o600);
    assert_eq!(std::fs::read(&backup).expect("bak"), old_bytes);
    let written: ProfileCollection =
        serde_json::from_str(&std::fs::read_to_string(&target).expect("après")).expect("JSON");
    assert_eq!(written, new);
    assert_eq!(reloader.calls(), 1);
}

#[tokio::test]
async fn profiles_reload_warning_rolls_back_exact_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let old = ProfileCollection::new(vec![Profile::new("custom-old", "Ancien")]);
    store.save_profiles(&old).expect("profils initiaux");
    let old_bytes = std::fs::read(dir.path().join("profiles.json")).expect("avant");

    let new = ProfileCollection::new(vec![Profile::new("custom-new", "Nouveau")]);
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_profiles(&store, &reloader, &new, 100).await,
        Err(AdminError::ReloadFailed)
    );
    assert_eq!(
        std::fs::read(dir.path().join("profiles.json")).expect("après"),
        old_bytes,
        "l'ancienne collection doit être restaurée à l'octet près"
    );
    assert_eq!(reloader.calls(), 2, "rollback + resynchronisation");
    assert_eq!(mode(&dir.path().join("profiles.json.bak")), 0o600);
}

#[tokio::test]
async fn reload_warning_rolls_back_exact_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_settings(&Settings::default())
        .expect("settings initiaux");
    let old_bytes = std::fs::read(dir.path().join("settings.json")).expect("avant");

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_settings(&store, &reloader, &new, 100).await,
        Err(AdminError::ReloadFailed)
    );
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).expect("après"),
        old_bytes,
        "l'ancien contenu doit être restauré à l'octet près"
    );
    assert_eq!(reloader.calls(), 2, "rollback + resynchronisation");
    assert_eq!(mode(&dir.path().join("settings.json.bak")), 0o600);
}

#[tokio::test]
async fn transport_failure_also_rolls_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_settings(&Settings::default())
        .expect("settings initiaux");
    let old_bytes = std::fs::read(dir.path().join("settings.json")).expect("avant");

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let reloader = FakeReloader::unavailable();
    assert_eq!(
        save_settings(&store, &reloader, &new, 100).await,
        Err(AdminError::ReloadFailed)
    );
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).expect("après"),
        old_bytes
    );
    assert_eq!(reloader.calls(), 2);
}

#[tokio::test]
async fn failed_save_without_previous_keeps_valid_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_settings(&store, &reloader, &Settings::default(), 50).await,
        Err(AdminError::ReloadFailed)
    );
    let written: Settings = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("settings.json")).expect("fichier"),
    )
    .expect("JSON valide");
    assert!(!written.fahrenheit);
    assert!(!dir.path().join("settings.json.bak").exists());
}

#[tokio::test]
async fn corrupt_current_preserves_healthy_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());

    // Deux écritures valides créent un `.bak` sain (le premier contenu).
    store
        .save_settings(&Settings {
            fahrenheit: true,
            ..Settings::default()
        })
        .expect("premier");
    store
        .save_settings_with_backup(&Settings::default())
        .expect("second");
    let backup = dir.path().join("settings.json.bak");
    let healthy_backup = std::fs::read(&backup).expect("sauvegarde");
    let target = dir.path().join("settings.json");

    // Le fichier courant devient illisible : la transaction ne doit pas
    // remplacer la sauvegarde saine par ces octets (A1-07).
    std::fs::write(&target, b"{ pas du json").expect("corruption");

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let reloader = FakeReloader::clean();
    let saved = save_settings(&store, &reloader, &new, 12)
        .await
        .expect("écriture");
    assert_eq!(saved.backup, BackupStatus::Preserved);
    assert_eq!(
        std::fs::read(&backup).expect("sauvegarde après"),
        healthy_backup,
        "le .bak sain doit rester intact"
    );
    let written: Settings =
        serde_json::from_str(&std::fs::read_to_string(&target).expect("cible")).expect("JSON");
    assert!(written.fahrenheit, "la nouvelle valeur doit être écrite");
    assert_eq!(mode(&backup), 0o600);
}

#[tokio::test]
async fn corrupt_previous_is_not_restored_after_reload_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let target = dir.path().join("settings.json");
    std::fs::write(&target, b"{ pas du json").expect("corruption initiale");

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_settings(&store, &reloader, &new, 12).await,
        Err(AdminError::ReloadFailed)
    );

    let written: Settings =
        serde_json::from_str(&std::fs::read_to_string(&target).expect("cible")).expect("JSON");
    assert!(
        written.fahrenheit,
        "le nouveau fichier valide doit être conservé, pas le précédent corrompu"
    );
    assert!(
        !dir.path().join("settings.json.bak").exists(),
        "aucune sauvegarde ne doit être créée à partir d'un contenu corrompu"
    );
}

#[tokio::test]
async fn identical_content_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let settings = Settings::default();
    store.save_settings(&settings).expect("initiaux");
    let before = std::fs::read(dir.path().join("settings.json")).expect("avant");
    let reloader = FakeReloader::clean();

    let saved = save_settings(&store, &reloader, &settings, 50)
        .await
        .expect("idempotent");
    assert_eq!(saved.backup, BackupStatus::None);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).expect("après"),
        before
    );
    // `ConfigStore` ne réécrit pas un contenu identique ; le rechargement est
    // tout de même demandé (l'appelant peut avoir changé autre chose).
    assert_eq!(reloader.calls(), 1);
}

#[tokio::test]
async fn profiles_and_settings_are_written_in_one_transaction() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_settings(&Settings::default())
        .expect("settings initiaux");
    store
        .save_profiles(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Ancien",
        )]))
        .expect("profils initiaux");
    let old_settings = std::fs::read(dir.path().join("settings.json")).expect("settings avant");
    let old_profiles = std::fs::read(dir.path().join("profiles.json")).expect("profils avant");

    let mut settings = Settings::default();
    settings
        .mark_profile_deleted(&ProfileId::new("custom-old"))
        .expect("tombstone");
    let profiles = ProfileCollection::new(Vec::new());
    let reloader = FakeReloader::clean();
    let saved = save_profiles_with_settings(&store, &reloader, &profiles, &settings, 42)
        .await
        .expect("écriture");

    assert_eq!(saved.bytes, 42);
    assert_eq!(saved.backup, BackupStatus::Written);
    let settings_target = dir.path().join("settings.json");
    let profiles_target = dir.path().join("profiles.json");
    assert_eq!(mode(&settings_target), 0o644);
    assert_eq!(mode(&profiles_target), 0o644);
    assert_eq!(mode(&dir.path().join("settings.json.bak")), 0o600);
    assert_eq!(mode(&dir.path().join("profiles.json.bak")), 0o600);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json.bak")).expect("bak settings"),
        old_settings
    );
    assert_eq!(
        std::fs::read(dir.path().join("profiles.json.bak")).expect("bak profils"),
        old_profiles
    );
    assert_eq!(
        reloader.calls(),
        1,
        "un seul rechargement pour deux fichiers"
    );

    let written: Settings =
        serde_json::from_slice(&std::fs::read(&settings_target).expect("settings")).expect("JSON");
    assert!(written.is_profile_id_deleted(&ProfileId::new("custom-old")));
    let written: ProfileCollection =
        serde_json::from_slice(&std::fs::read(&profiles_target).expect("profils")).expect("JSON");
    assert!(written.is_empty());
}

#[tokio::test]
async fn reload_failure_rolls_back_profiles_and_settings_together() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_settings(&Settings::default())
        .expect("settings initiaux");
    store
        .save_profiles(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Ancien",
        )]))
        .expect("profils initiaux");
    let old_settings = std::fs::read(dir.path().join("settings.json")).expect("settings avant");
    let old_profiles = std::fs::read(dir.path().join("profiles.json")).expect("profils avant");

    let mut settings = Settings::default();
    settings
        .mark_profile_deleted(&ProfileId::new("custom-old"))
        .expect("tombstone");
    let profiles = ProfileCollection::new(Vec::new());
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_profiles_with_settings(&store, &reloader, &profiles, &settings, 42).await,
        Err(AdminError::ReloadFailed)
    );
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).expect("settings après"),
        old_settings,
        "les réglages doivent être restaurés à l'octet près"
    );
    assert_eq!(
        std::fs::read(dir.path().join("profiles.json")).expect("profils après"),
        old_profiles,
        "les profils doivent être restaurés à l'octet près"
    );
    let restored: Settings =
        serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).expect("settings"))
            .expect("JSON");
    assert!(!restored.is_profile_id_deleted(&ProfileId::new("custom-old")));
    assert_eq!(reloader.calls(), 2, "rollback + resynchronisation");
}

#[tokio::test]
async fn reload_failure_restores_absence_of_settings_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_profiles(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Ancien",
        )]))
        .expect("profils initiaux");
    let old_profiles = std::fs::read(dir.path().join("profiles.json")).expect("profils avant");
    assert!(
        !dir.path().join("settings.json").exists(),
        "settings absent au départ"
    );

    let mut settings = Settings::default();
    settings
        .mark_profile_deleted(&ProfileId::new("custom-old"))
        .expect("tombstone");
    let profiles = ProfileCollection::new(Vec::new());
    let reloader = FakeReloader::warning();
    assert_eq!(
        save_profiles_with_settings(&store, &reloader, &profiles, &settings, 42).await,
        Err(AdminError::ReloadFailed)
    );
    assert!(
        !dir.path().join("settings.json").exists(),
        "le settings créé par la transaction doit être supprimé au rollback"
    );
    assert_eq!(
        std::fs::read(dir.path().join("profiles.json")).expect("profils après"),
        old_profiles,
        "les profils doivent être restaurés à l'octet près"
    );
    assert_eq!(reloader.calls(), 2, "rollback + resynchronisation");
}
