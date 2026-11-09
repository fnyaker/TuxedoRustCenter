// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Contrat D-Bus de `User1.SetUvcControl` et de `GetUvcWriteSupported`.
//!
//! Les tests utilisent le **vrai** contrôleur d'écriture `tcc-hw`
//! (`UvcWriterController` + `FakeWriter`) sur une racine sysfs temporaire :
//! aucun `/dev/video*`, aucun privilège, aucune caméra. Ils couvrent :
//!
//! - l'introspection (méthode présente, signature `(ssi) -> b`) ;
//! - l'écriture réelle et l'idempotence (`true`/`false`) ;
//! - les erreurs stables (`InvalidArgs`, `NotSupported`, `OutOfRange`,
//!   `Busy`, `Stalled`, `LimitsExceeded`) ;
//! - la limite de débit par UID ;
//! - le cache de `GetUvcWriteSupported` et le garde de corps D-Bus ;
//! - le mapping `EACCES`/`EPERM` en `NotSupported` (ACL absente) ;
//! - l'absence de `S_CTRL` dans le graphe de `tccd`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::v4l2::fake::{FakeControl, FakeWriter};
use tcc_hw::v4l2::{ControlQuery, DeviceCapabilities, MenuQuery, UvcWriter, UvcWriterController};
use tcc_ipc::{DBUS_SESSION_AGENT_INTERFACE, DBUS_SESSION_AGENT_PATH};
use tcc_user::session_service::{serve_with, serve_with_uvc};
use tcc_user::{
    DisplayInfo, DisplayInfoSource, SessionEnv, SessionInfo, SysfsUvcDeviceSource, SystemUvcWriter,
    UVC_ERROR_BUSY, UVC_ERROR_INVALID_ARGS, UVC_ERROR_LIMITS_EXCEEDED, UVC_ERROR_NOT_SUPPORTED,
    UVC_ERROR_OUT_OF_RANGE, UVC_ERROR_STALLED, UvcRuntime,
};

const CID_BRIGHTNESS: u32 = 0x0098_0900;
const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("répertoire sysfs de test");
    }
    std::fs::write(path, contents).expect("fichier sysfs de test");
}

fn fixture(root: &Path) {
    write_file(root, "class/video4linux/video0/dev", "81:0\n");
    write_file(root, "class/video4linux/video0/index", "0\n");
    write_file(root, "class/video4linux/video0/name", "Integrated Camera\n");
    write_file(
        root,
        "class/video4linux/video0/device/uevent",
        "PRODUCT=1bcf/2c99/7200\n",
    );
}

fn capabilities() -> DeviceCapabilities {
    DeviceCapabilities {
        driver: "uvcvideo".to_owned(),
        card: "Integrated Camera".to_owned(),
        bus_info: "usb-0000:00:14.0-1".to_owned(),
        capabilities: V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_DEVICE_CAPS,
        device_caps: V4L2_CAP_VIDEO_CAPTURE,
    }
}

fn brightness_writer(initial: i32) -> FakeWriter {
    FakeWriter::new(capabilities()).with_control(
        FakeControl::new(CID_BRIGHTNESS, "Brightness", 1)
            .range(0, 255, 1)
            .default(128)
            .value(initial),
    )
}

/// Écrivain partagé entre les ouvertures : l'état des contrôles persiste,
/// comme sur un vrai device.
#[derive(Debug)]
struct SharedFakeWriter(Arc<FakeWriter>);

impl UvcWriter for SharedFakeWriter {
    fn capabilities(&self) -> tcc_hw::Result<DeviceCapabilities> {
        self.0.capabilities()
    }

    fn query_control(&self, id: u32) -> tcc_hw::Result<ControlQuery> {
        self.0.query_control(id)
    }

    fn query_menu(&self, id: u32, index: i32) -> tcc_hw::Result<MenuQuery> {
        self.0.query_menu(id, index)
    }

    fn get_control(&self, id: u32) -> tcc_hw::Result<i32> {
        self.0.get_control(id)
    }

    fn set_control(&self, id: u32, value: i32) -> tcc_hw::Result<()> {
        self.0.set_control(id, value)
    }
}

fn controller(root: &Path, writer: Arc<FakeWriter>) -> UvcWriterController {
    UvcWriterController::with_factory(
        SysfsRoot::new(root),
        Arc::new(move |_device| {
            Ok(Box::new(SharedFakeWriter(Arc::clone(&writer))) as Box<dyn UvcWriter>)
        }),
    )
}

/// Source d'énumération statique qui compte les appels (cache/rate limit).
#[derive(Debug)]
struct CountingDevices {
    devices: Vec<tcc_user::UvcDeviceEntry>,
    calls: Arc<AtomicUsize>,
}

impl tcc_user::UvcDeviceSource for CountingDevices {
    fn devices(&self) -> tcc_hw::Result<Vec<tcc_user::UvcDeviceEntry>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.devices.clone())
    }
}

/// Écrivain qui simule une ACL/groupe `video` absent.
#[derive(Debug)]
struct PermissionWriter;

impl tcc_user::UvcControlWriter for PermissionWriter {
    fn set_control(
        &self,
        _device: &str,
        _control_key: &str,
        _value: i64,
    ) -> tcc_hw::Result<tcc_hw::v4l2::UvcWriteReport> {
        Err(tcc_hw::HwError::Io {
            path: "/dev/video0".into(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        })
    }
}

/// Source d'affichage inerte : les tests UVC ne dépendent d'aucun compositeur.
struct EmptyDisplay;

impl DisplayInfoSource for EmptyDisplay {
    fn display_info(&self) -> DisplayInfo {
        DisplayInfo::unavailable(
            &SessionInfo::from_env(SessionEnv::default()),
            "test UVC sans affichage",
        )
    }
}

struct TestService {
    client: zbus::Connection,
    name: String,
    shutdown: Arc<tokio::sync::Notify>,
    handle: tokio::task::JoinHandle<Result<(), tcc_user::SessionServiceError>>,
}

impl TestService {
    async fn proxy(&self) -> zbus::Proxy<'_> {
        zbus::Proxy::new(
            &self.client,
            self.name.clone(),
            DBUS_SESSION_AGENT_PATH,
            DBUS_SESSION_AGENT_INTERFACE,
        )
        .await
        .expect("proxy User1")
    }
}

impl Drop for TestService {
    fn drop(&mut self) {
        self.shutdown.notify_one();
        self.handle.abort();
    }
}

fn unique_name(suffix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    format!(
        "com.tuxedocomputers.tccd.User.UvcTest{}.{}{}",
        std::process::id(),
        suffix,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

async fn wait_for_owner(client: &zbus::Connection, name: &str, expected: bool) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(client).await else {
        return false;
    };
    let Ok(bus_name) = zbus::names::BusName::try_from(name.to_owned()) else {
        return false;
    };
    for _ in 0..50 {
        if dbus.name_has_owner(bus_name.clone()).await.unwrap_or(false) == expected {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn serve_runtime(uvc: UvcRuntime, suffix: &str) -> Option<TestService> {
    let client = zbus::Connection::session().await.ok()?;
    let name = unique_name(suffix);
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let handle = tokio::spawn(serve_with_uvc(
        Arc::new(EmptyDisplay) as Arc<dyn DisplayInfoSource>,
        Arc::new(uvc),
        name.clone(),
        Some(Arc::clone(&shutdown)),
    ));
    if !wait_for_owner(&client, &name, true).await {
        handle.abort();
        return None;
    }
    Some(TestService {
        client,
        name,
        shutdown,
        handle,
    })
}

async fn serve_fixture(root: &Path, writer: Arc<FakeWriter>, suffix: &str) -> Option<TestService> {
    let uvc = UvcRuntime::new(
        Arc::new(SysfsUvcDeviceSource::new(SysfsRoot::new(root))),
        Arc::new(SystemUvcWriter::with_controller(controller(root, writer))),
    );
    serve_runtime(uvc, suffix).await
}

fn error_name(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => panic!("erreur D-Bus inattendue: {other:?}"),
    }
}

#[tokio::test]
async fn introspection_lists_uvc_methods_and_signature() {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = Arc::new(brightness_writer(10));
    let Some(service) = serve_fixture(dir.path(), writer, "introspect").await else {
        eprintln!("bus de session absent : test User1.SetUvcControl ignoré");
        return;
    };
    let introspectable = zbus::Proxy::new(
        &service.client,
        service.name.clone(),
        DBUS_SESSION_AGENT_PATH,
        "org.freedesktop.DBus.Introspectable",
    )
    .await
    .expect("proxy introspection");
    let xml: String = introspectable
        .call("Introspect", &())
        .await
        .expect("Introspect");

    for method in [
        "Ping",
        "GetVersion",
        "GetContractVersion",
        "GetDisplayInfoJSON",
        "GetUvcWriteSupported",
        "SetUvcControl",
    ] {
        assert!(xml.contains(method), "méthode absente: {method}");
    }

    let method = xml
        .split("<method name=\"SetUvcControl\">")
        .nth(1)
        .and_then(|rest| rest.split("</method>").next())
        .expect("bloc SetUvcControl");
    assert_eq!(
        method.matches("type=\"s\"").count(),
        2,
        "deux chaînes attendues : {method}"
    );
    assert!(method.contains("type=\"i\""), "entier attendu : {method}");
    assert!(
        method.contains("type=\"b\"") && method.contains("direction=\"out\""),
        "retour booléen attendu : {method}"
    );

    let capability = xml
        .split("<method name=\"GetUvcWriteSupported\">")
        .nth(1)
        .and_then(|rest| rest.split("</method>").next())
        .expect("bloc GetUvcWriteSupported");
    assert!(
        capability.contains("type=\"b\"") && !capability.contains("direction=\"in\""),
        "capability sans entrée et booléenne : {capability}"
    );
}

#[tokio::test]
async fn write_supported_reflects_enumeration_and_backend() {
    // 1. Service sans couche d'écriture : jamais de fausse promesse.
    let Ok(client) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test writeSupported ignoré");
        return;
    };
    let name = unique_name("no-uvc");
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let handle = tokio::spawn(serve_with(
        Arc::new(EmptyDisplay) as Arc<dyn DisplayInfoSource>,
        name.clone(),
        Some(Arc::clone(&shutdown)),
    ));
    if !wait_for_owner(&client, &name, true).await {
        eprintln!("bus de session absent : test writeSupported ignoré");
        return;
    }
    let proxy = zbus::Proxy::new(
        &client,
        name.clone(),
        DBUS_SESSION_AGENT_PATH,
        DBUS_SESSION_AGENT_INTERFACE,
    )
    .await
    .expect("proxy sans UVC");
    let supported: bool = proxy
        .call("GetUvcWriteSupported", &())
        .await
        .expect("capability sans UVC");
    assert!(!supported);
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "brightness", 1))
        .await
        .expect_err("écriture sans backend");
    assert_eq!(error_name(&error), UVC_ERROR_NOT_SUPPORTED);
    drop(proxy);
    shutdown.notify_one();
    let _ = handle.await;

    // 2. Backend présent mais aucun device énuméré.
    let empty = tempfile::tempdir().expect("tempdir");
    let writer = Arc::new(brightness_writer(10));
    let Some(service) = serve_fixture(empty.path(), writer, "empty").await else {
        eprintln!("bus de session absent : test writeSupported ignoré");
        return;
    };
    let proxy = service.proxy().await;
    let supported: bool = proxy
        .call("GetUvcWriteSupported", &())
        .await
        .expect("capability sans device");
    assert!(!supported);

    // 3. Device énuméré : écriture annoncée.
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let writer = Arc::new(brightness_writer(10));
    let Some(service) = serve_fixture(dir.path(), writer, "camera").await else {
        eprintln!("bus de session absent : test writeSupported ignoré");
        return;
    };
    let proxy = service.proxy().await;
    let supported: bool = proxy
        .call("GetUvcWriteSupported", &())
        .await
        .expect("capability avec device");
    assert!(supported);
}

#[tokio::test]
async fn writes_are_idempotent_and_validated() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let writer = Arc::new(brightness_writer(10));
    let Some(service) = serve_fixture(dir.path(), writer, "write").await else {
        eprintln!("bus de session absent : test SetUvcControl ignoré");
        return;
    };
    let proxy = service.proxy().await;

    // Valeur déjà courante : aucun `S_CTRL`, retour false.
    let changed: bool = proxy
        .call("SetUvcControl", &("/dev/video0", "brightness", 10))
        .await
        .expect("valeur courante");
    assert!(!changed);

    // Écriture réelle puis idempotence.
    let changed: bool = proxy
        .call("SetUvcControl", &("/dev/video0", "brightness", 42))
        .await
        .expect("écriture 42");
    assert!(changed);
    let changed: bool = proxy
        .call("SetUvcControl", &("/dev/video0", "brightness", 42))
        .await
        .expect("réécriture 42");
    assert!(!changed);

    // Contrôle inconnu : hors allowlist.
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "unknown", 1))
        .await
        .expect_err("contrôle inconnu");
    assert_eq!(error_name(&error), UVC_ERROR_NOT_SUPPORTED);

    // Hors plage et valeur négative (borne basse 0).
    for value in [300, -1] {
        let error = proxy
            .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "brightness", value))
            .await
            .expect_err("valeur hors plage");
        assert_eq!(error_name(&error), UVC_ERROR_OUT_OF_RANGE, "value={value}");
    }

    // Chemin non énuméré et évasion de chemin.
    for path in ["/dev/video9", "../video0", "/dev/video01"] {
        let error = proxy
            .call::<_, _, bool>("SetUvcControl", &(path, "brightness", 1))
            .await
            .expect_err("chemin invalide");
        assert_eq!(error_name(&error), UVC_ERROR_INVALID_ARGS, "path={path}");
    }

    // Clé invalide.
    for key in [
        "",
        "Brightness",
        "focus-auto",
        "focus auto",
        "../brightness",
    ] {
        let error = proxy
            .call::<_, _, bool>("SetUvcControl", &("/dev/video0", key, 1))
            .await
            .expect_err("clé invalide");
        assert_eq!(error_name(&error), UVC_ERROR_INVALID_ARGS, "key={key}");
    }

    // Le service reste vivant après tous les refus.
    let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
    assert!(pong);
}

#[tokio::test]
async fn busy_and_stalled_are_mapped() {
    // Device occupé (`EBUSY` simulé sur QUERYCAP).
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let busy = Arc::new(brightness_writer(10).fail_capabilities(std::io::ErrorKind::WouldBlock));
    let Some(service) = serve_fixture(dir.path(), busy, "busy").await else {
        eprintln!("bus de session absent : test Busy ignoré");
        return;
    };
    let proxy = service.proxy().await;
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "brightness", 1))
        .await
        .expect_err("device occupé");
    assert_eq!(error_name(&error), UVC_ERROR_BUSY);
    drop(proxy);
    drop(service);

    // Device débranché (`EIO`), puis quarantaine.
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let stalled = Arc::new(brightness_writer(10).fail_set_errno(libc::EIO));
    let Some(service) = serve_fixture(dir.path(), stalled, "stalled").await else {
        eprintln!("bus de session absent : test Stalled ignoré");
        return;
    };
    let proxy = service.proxy().await;
    for attempt in 0..2 {
        let error = proxy
            .call::<_, _, bool>(
                "SetUvcControl",
                &("/dev/video0", "brightness", 20 + attempt),
            )
            .await
            .expect_err("device bloqué");
        assert_eq!(error_name(&error), UVC_ERROR_STALLED, "essai {attempt}");
    }
}

#[tokio::test]
async fn oversized_dbus_body_is_rejected_before_deserialization() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let writer = Arc::new(brightness_writer(10));
    let Some(service) = serve_fixture(dir.path(), writer, "oversized").await else {
        eprintln!("bus de session absent : test corps surdimensionné ignoré");
        return;
    };
    let proxy = service.proxy().await;

    let huge = "a".repeat(1024 * 1024);
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &(huge.as_str(), "brightness", 1))
        .await
        .expect_err("corps surdimensionné");
    assert_eq!(error_name(&error), UVC_ERROR_INVALID_ARGS);

    // Le service reste vivant et n'a pas désérialisé la méthode.
    let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
    assert!(pong);
}

#[tokio::test]
async fn write_supported_is_cached_over_dbus() {
    let calls = Arc::new(AtomicUsize::new(0));
    let uvc = UvcRuntime::new(
        Arc::new(CountingDevices {
            devices: vec![tcc_user::UvcDeviceEntry {
                node: "video0".to_owned(),
                path: "/dev/video0".to_owned(),
            }],
            calls: Arc::clone(&calls),
        }),
        Arc::new(PermissionWriter),
    );
    let Some(service) = serve_runtime(uvc, "cache").await else {
        eprintln!("bus de session absent : test cache writeSupported ignoré");
        return;
    };
    let proxy = service.proxy().await;
    for _ in 0..3 {
        let supported: bool = proxy
            .call("GetUvcWriteSupported", &())
            .await
            .expect("capability");
        assert!(supported);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "l'énumération doit être mise en cache 1 s"
    );
}

#[tokio::test]
async fn permission_denied_is_not_supported_over_dbus() {
    let uvc = UvcRuntime::new(
        Arc::new(CountingDevices {
            devices: vec![tcc_user::UvcDeviceEntry {
                node: "video0".to_owned(),
                path: "/dev/video0".to_owned(),
            }],
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(PermissionWriter),
    );
    let Some(service) = serve_runtime(uvc, "permission").await else {
        eprintln!("bus de session absent : test EACCES ignoré");
        return;
    };
    let proxy = service.proxy().await;
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "brightness", 20))
        .await
        .expect_err("ACL absente");
    assert_eq!(error_name(&error), UVC_ERROR_NOT_SUPPORTED);
}

#[tokio::test]
async fn rate_limit_is_per_uid_over_dbus() {
    let dir = tempfile::tempdir().expect("tempdir");
    fixture(dir.path());
    let writer = Arc::new(brightness_writer(0));
    let uvc = UvcRuntime::with_rate_limit(
        Arc::new(SysfsUvcDeviceSource::new(SysfsRoot::new(dir.path()))),
        Arc::new(SystemUvcWriter::with_controller(controller(
            dir.path(),
            writer,
        ))),
        0.001,
        2.0,
    );
    let Some(service) = serve_runtime(uvc, "rate").await else {
        eprintln!("bus de session absent : test rate limit UVC ignoré");
        return;
    };
    let proxy = service.proxy().await;

    for value in 0..2 {
        let changed: bool = proxy
            .call("SetUvcControl", &("/dev/video0", "brightness", value))
            .await
            .unwrap_or_else(|error| panic!("écriture {value} refusée: {error}"));
        assert!(changed || value == 0);
    }
    let error = proxy
        .call::<_, _, bool>("SetUvcControl", &("/dev/video0", "brightness", 2))
        .await
        .expect_err("limite de débit");
    assert_eq!(error_name(&error), UVC_ERROR_LIMITS_EXCEEDED);

    // La capability reste lisible : bucket et lecture sont indépendants.
    let supported: bool = proxy
        .call("GetUvcWriteSupported", &())
        .await
        .expect("capability malgré limite");
    assert!(supported);
}

#[test]
fn tccd_does_not_reference_uvc_write() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tcc-daemon/src");
    let mut files = Vec::new();
    collect_rust_files(&source, &mut files);
    assert!(!files.is_empty(), "aucune source tcc-daemon trouvée");

    for file in files {
        let contents = std::fs::read_to_string(&file).expect("lecture source tcc-daemon");
        for line in code_lines(&contents) {
            for token in [
                "UvcWriterController",
                "UvcControlWriter",
                "UvcWriteReport",
                "v4l2::write",
                "S_CTRL",
                "VIDIOC_S_CTRL",
            ] {
                assert!(
                    !line.contains(token),
                    "{} référence {token} : {}",
                    file.display(),
                    line.trim()
                );
            }
        }
    }
}

fn collect_rust_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files.sort();
}

/// Retire les commentaires de ligne et de bloc, comme la garde V4L2 de
/// `tcc-hw`. Les `S_CTRL` documentaires ne doivent pas faire échouer le test.
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
