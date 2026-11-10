// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests D-Bus de `Admin1` sur le bus de session.
//!
//! Le service réel est monté dans le processus de test avec un authorizer et
//! un notifier factices : aucun polkit, aucun `tccd` et aucun privilège ne
//! sont nécessaires. Sans bus de session, les tests sont ignorés.
//!
//! Isolation : chaque test ouvre **sa propre connexion** sur le bus de
//! session et sert l'objet sur cette connexion. Le proxy cible le **nom
//! unique** de cette connexion (`:1.x`), jamais un nom bien connu fixe : deux
//! tests ne peuvent donc pas se disputer un nom, même exécutés en parallèle.
//! Aucun verrou global n'est nécessaire.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tcc_admin::transaction::{MAX_PAYLOAD_BYTES, ReloadError, ReloadNotifier};
use tcc_admin::{Activity, AdminService, Authorizer, AuthzDecision};
use tcc_core::{ConfigStore, Profile, ProfileCollection, ProfileId, Settings};
use tcc_ipc::AdminReloadReport;
use zbus::proxy;

#[proxy(
    interface = "com.tuxedocomputers.tccd.Admin1",
    default_path = "/com/tuxedocomputers/tccd/Admin"
)]
trait Admin1 {
    fn ping(&self) -> zbus::Result<bool>;
    fn get_version(&self) -> zbus::Result<String>;
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "SaveSettingsJSON")]
    fn save_settings_json(&self, json: &str) -> zbus::Result<bool>;
    #[zbus(name = "SaveProfilesJSON")]
    fn save_profiles_json(&self, json: &str) -> zbus::Result<bool>;
    fn reload_config(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn contract_version(&self) -> zbus::Result<u32>;
}

struct AllowAll;
#[async_trait]
impl Authorizer for AllowAll {
    async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
        AuthzDecision::Authorized
    }
}

struct DenyAll;
#[async_trait]
impl Authorizer for DenyAll {
    async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
        AuthzDecision::Denied(tcc_authz::DenialReason::PolicyDenied)
    }
}

struct Unavailable;
#[async_trait]
impl Authorizer for Unavailable {
    async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
        AuthzDecision::Unavailable
    }
}

struct FakeReloader {
    report: AdminReloadReport,
    calls: AtomicUsize,
}

impl FakeReloader {
    fn clean() -> Self {
        Self {
            report: AdminReloadReport::clean(),
            calls: AtomicUsize::new(0),
        }
    }

    fn warning() -> Self {
        Self {
            report: AdminReloadReport::from_warnings(vec!["invalide".to_owned()]),
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
        Ok(self.report.clone())
    }
}

/// Notifier qui bloque le premier rechargement jusqu'à autorisation du test.
///
/// Il remplace le `sleep` du test de concurrence : le test sait de façon
/// déterministe quand le verrou d'écriture est détenu, sans dépendre de la
/// charge machine.
struct GatedReloader {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    calls: AtomicUsize,
}

impl GatedReloader {
    fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ReloadNotifier for GatedReloader {
    async fn reload(&self) -> Result<AdminReloadReport, ReloadError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // `notify_one` conserve un permis si le test n'attend pas encore.
        self.entered.notify_one();
        self.release.notified().await;
        Ok(AdminReloadReport::clean())
    }
}

fn store(root: &std::path::Path) -> ConfigStore {
    ConfigStore::with_admin_roots(root, root.join("admin"))
        .with_policy(tcc_core::config::LoadPolicy::default())
        .with_write_root(root)
}

fn error_name(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => panic!("erreur D-Bus inattendue: {other:?}"),
    }
}

async fn serve(
    store: ConfigStore,
    reloader: Arc<dyn ReloadNotifier>,
    authorizer: Arc<dyn Authorizer>,
) -> Option<zbus::Connection> {
    let connection = zbus::Connection::session().await.ok()?;
    let service = AdminService::new(store, reloader, authorizer, Arc::new(Activity::new()));
    connection
        .object_server()
        .at(tcc_ipc::DBUS_ADMIN_PATH, service)
        .await
        .ok()?;
    Some(connection)
}

async fn proxy<'a>(
    client: &'a zbus::Connection,
    service: &zbus::Connection,
) -> Option<Admin1Proxy<'a>> {
    let destination = service.unique_name()?.to_string();
    Admin1Proxy::builder(client)
        .destination(destination)
        .ok()?
        .path(tcc_ipc::DBUS_ADMIN_PATH)
        .ok()?
        .build()
        .await
        .ok()
}

#[tokio::test]
async fn ping_version_and_property_are_available() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(connection) = serve(
        store(dir.path()),
        Arc::new(FakeReloader::clean()),
        Arc::new(AllowAll),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    assert!(proxy.ping().await.expect("Ping"));
    assert!(
        proxy
            .get_version()
            .await
            .expect("GetVersion")
            .contains(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        proxy
            .get_contract_version()
            .await
            .expect("GetContractVersion"),
        1
    );
    // Propriété conservée pour l'introspection interne uniquement.
    assert_eq!(proxy.contract_version().await.expect("ContractVersion"), 1);
}

#[tokio::test]
async fn save_settings_writes_and_reloads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reloader = Arc::new(FakeReloader::clean());
    let Some(connection) = serve(store(dir.path()), reloader.clone(), Arc::new(AllowAll)).await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let json = serde_json::to_string(&Settings::default()).expect("JSON");
    assert!(proxy.save_settings_json(&json).await.expect("SaveSettings"));
    assert!(dir.path().join("settings.json").exists());
    assert_eq!(reloader.calls(), 1);
    assert!(proxy.reload_config().await.expect("ReloadConfig"));
    assert_eq!(reloader.calls(), 2);
}

#[tokio::test]
async fn malformed_payload_is_rejected_with_typed_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(connection) = serve(
        store(dir.path()),
        Arc::new(FakeReloader::clean()),
        Arc::new(AllowAll),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let error = proxy
        .save_settings_json("not-json")
        .await
        .expect_err("JSON invalide");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.InvalidArgs"
    );
    assert!(!dir.path().join("settings.json").exists());
}

#[tokio::test]
async fn oversized_payload_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(connection) = serve(
        store(dir.path()),
        Arc::new(FakeReloader::clean()),
        Arc::new(AllowAll),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let payload = "x".repeat(MAX_PAYLOAD_BYTES + 1);
    let error = proxy
        .save_settings_json(&payload)
        .await
        .expect_err("payload trop grand");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.PayloadTooLarge"
    );
}

#[tokio::test]
async fn denied_polkit_is_not_authorized() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(connection) = serve(
        store(dir.path()),
        Arc::new(FakeReloader::clean()),
        Arc::new(DenyAll),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let json = serde_json::to_string(&Settings::default()).expect("JSON");
    let error = proxy
        .save_settings_json(&json)
        .await
        .expect_err("refus polkit");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.NotAuthorized"
    );
    assert!(!dir.path().join("settings.json").exists());
}

#[tokio::test]
async fn unavailable_polkit_is_fail_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(connection) = serve(
        store(dir.path()),
        Arc::new(FakeReloader::clean()),
        Arc::new(Unavailable),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let json = serde_json::to_string(&Settings::default()).expect("JSON");
    let error = proxy
        .save_settings_json(&json)
        .await
        .expect_err("polkit indisponible");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.NotAuthorized"
    );
    assert!(!dir.path().join("settings.json").exists());
}

#[tokio::test]
async fn removing_a_referenced_profile_is_in_use_over_dbus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_profiles(&ProfileCollection::new(vec![Profile::new(
            "custom-in-use",
            "In use",
        )]))
        .expect("profils initiaux");

    let mut settings = Settings::default();
    settings.state_map.power_ac = ProfileId::new("custom-in-use");
    settings.state_map.power_bat = ProfileId::new("custom-in-use");
    store.save_settings(&settings).expect("settings initiaux");
    let before = std::fs::read(&store.paths().profiles).expect("profils avant");

    let Some(connection) = serve(
        store.clone(),
        Arc::new(FakeReloader::clean()),
        Arc::new(AllowAll),
    )
    .await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    // Suppression explicite de tous les profils personnalisés alors que
    // `stateMap` référence encore `custom-in-use` : la transaction doit être
    // refusée avant toute écriture.
    let error = proxy
        .save_profiles_json("[]")
        .await
        .expect_err("profil référencé");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.InUse"
    );
    assert_eq!(
        std::fs::read(&store.paths().profiles).expect("profils après"),
        before
    );
}

#[tokio::test]
async fn admin_validation_does_not_quarantine_a_corrupt_settings_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let settings_path = store.paths().settings.clone();
    std::fs::write(&settings_path, b"{ pas du json").expect("settings corrompus");

    let Some(connection) = serve(store, Arc::new(FakeReloader::clean()), Arc::new(AllowAll)).await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let error = proxy
        .save_profiles_json("[]")
        .await
        .expect_err("settings corrompus");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.Validation"
    );
    assert!(
        settings_path.exists(),
        "la validation croisée a renommé le fichier corrompu"
    );
    assert_eq!(
        std::fs::read(&settings_path).expect("fichier d'origine"),
        b"{ pas du json"
    );
    assert!(
        !dir.path().join("settings.json.bak").exists(),
        "la validation croisée ne doit pas renommer le fichier corrompu"
    );
}

#[tokio::test]
async fn admin_settings_validation_does_not_quarantine_a_corrupt_profiles_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    let profiles_path = store.paths().profiles.clone();
    std::fs::write(&profiles_path, b"{ pas du json").expect("profils corrompus");

    let Some(connection) = serve(store, Arc::new(FakeReloader::clean()), Arc::new(AllowAll)).await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let json = serde_json::to_string(&Settings::default()).expect("JSON");
    let error = proxy
        .save_settings_json(&json)
        .await
        .expect_err("profils corrompus");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.Validation"
    );
    assert!(
        profiles_path.exists(),
        "la validation croisée a renommé le fichier corrompu"
    );
    assert!(
        !dir.path().join("profiles.json.bak").exists(),
        "la validation croisée ne doit pas renommer le fichier corrompu"
    );
}

#[tokio::test]
async fn concurrent_write_returns_busy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reloader = Arc::new(GatedReloader::new());
    let Some(connection) = serve(store(dir.path()), reloader.clone(), Arc::new(AllowAll)).await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let json = serde_json::to_string(&Settings::default()).expect("JSON");
    let first_proxy = proxy.clone();
    let first = first_proxy.save_settings_json(&json);
    tokio::pin!(first);

    // Attendre que le premier handler soit entré dans le rechargement : le
    // verrou d'écriture est alors détenu de façon certaine, sans `sleep`.
    tokio::select! {
        _ = reloader.entered.notified() => {}
        result = &mut first => {
            panic!("première écriture terminée avant le verrou: {result:?}");
        }
    }

    let error = proxy
        .save_settings_json(&json)
        .await
        .expect_err("écriture concurrente");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.Busy"
    );
    assert_eq!(reloader.calls(), 1);

    reloader.release.notify_one();
    assert!(first.await.expect("première écriture"));
    assert_eq!(reloader.calls(), 1);
}

#[tokio::test]
async fn concurrent_profile_transaction_returns_busy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_profiles(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Ancien",
        )]))
        .expect("profils initiaux");
    let reloader = Arc::new(GatedReloader::new());
    let Some(connection) = serve(store.clone(), reloader.clone(), Arc::new(AllowAll)).await else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    // La suppression écrit deux fichiers puis attend le rechargement : le
    // verrou est tenu de façon certaine pendant cette attente.
    let first_proxy = proxy.clone();
    let first = first_proxy.save_profiles_json("[]");
    tokio::pin!(first);

    tokio::select! {
        _ = reloader.entered.notified() => {}
        result = &mut first => {
            panic!("première écriture terminée avant le verrou: {result:?}");
        }
    }

    let error = proxy
        .save_profiles_json("[]")
        .await
        .expect_err("transaction concurrente");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.Busy"
    );
    assert_eq!(reloader.calls(), 1);

    reloader.release.notify_one();
    assert!(first.await.expect("première écriture"));
    assert_eq!(reloader.calls(), 1);

    // État final cohérent : profils supprimés et tombstone persisté ensemble.
    let profiles: ProfileCollection =
        serde_json::from_slice(&std::fs::read(store.paths().profiles.clone()).expect("profils"))
            .expect("JSON");
    assert!(profiles.is_empty());
    let settings: Settings =
        serde_json::from_slice(&std::fs::read(store.paths().settings.clone()).expect("settings"))
            .expect("JSON");
    assert!(settings.is_profile_id_deleted(&ProfileId::new("custom-old")));
}

#[tokio::test]
async fn reload_warning_rolls_back_over_dbus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = store(dir.path());
    store
        .save_settings(&Settings::default())
        .expect("settings initiaux");
    let old_bytes = std::fs::read(dir.path().join("settings.json")).expect("avant");

    let Some(connection) =
        serve(store, Arc::new(FakeReloader::warning()), Arc::new(AllowAll)).await
    else {
        eprintln!("bus de session absent : test Admin1 ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &connection).await else {
        panic!("proxy Admin1");
    };

    let new = Settings {
        fahrenheit: true,
        ..Settings::default()
    };
    let json = serde_json::to_string(&new).expect("JSON");
    let error = proxy
        .save_settings_json(&json)
        .await
        .expect_err("rechargement refusé");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.Admin1.Error.ReloadFailed"
    );
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).expect("après"),
        old_bytes
    );
}
