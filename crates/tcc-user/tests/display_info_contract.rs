// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Contrat du service de session `User1` et du rapport `GetDisplayInfoJSON`.
//!
//! `DisplayInfoView` vit dans `tcc-gui` (module privé, lié à cxx-qt) : ce test
//! ne peut pas l'importer. Il vérifie donc :
//!
//! - que les fixtures `display_info_*.json` sont exactement ce que produit
//!   `tcc_user::DisplayInfo` (sérialisation publique) ;
//! - la forme/les bornes du consommateur avec un miroir de contrat
//!   (`DisplayInfoViewContract`), explicitement limité à 16 384 px, 1–1000 Hz
//!   et 256 modes, comme `AgentData.qml` ;
//! - le contrat `User1` (nom, objet, interface, version) ;
//! - le service sur bus de session : nom possédé, `Ping`/`GetVersion`/
//!   `GetContractVersion`/`GetDisplayInfoJSON`, nom déjà pris, arrêt propre.
//!
//! Aucun compositeur, aucun écran réel : la source est injectée.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use tcc_hw::display::{DisplayBackendKind, DisplayMode, OutputInfo};
use tcc_ipc::{
    DBUS_SESSION_AGENT_INTERFACE, DBUS_SESSION_AGENT_PATH, DBUS_SESSION_AGENT_SERVICE,
    SESSION_CONTRACT_VERSION,
};
use tcc_user::session_service::{SessionServiceError, serve_with};
use tcc_user::{DisplayInfo, DisplayInfoSource, SessionEnv, SessionInfo};

const MAX_MODES: usize = 256;
const MAX_DIMENSION: u32 = 16_384;
const MIN_REFRESH_HZ: f32 = 1.0;
const MAX_REFRESH_HZ: f32 = 1000.0;

/// Miroir du consommateur `DisplayInfoView`/`AgentData.qml`.
///
/// `serde(default)` partout et clés inconnues ignorées : un rapport externe
/// (`externalOutput`) ne doit jamais devenir un mode interne.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DisplayInfoViewContract {
    available: bool,
    session: SessionViewContract,
    backend: BackendViewContract,
    internal_output: InternalOutputViewContract,
    supported: bool,
    dry_run: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SessionViewContract {
    #[serde(rename = "type")]
    session_type: String,
    desktop: Option<String>,
    wayland_display: Option<String>,
    display: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BackendViewContract {
    kind: String,
    name: String,
    available: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct InternalOutputViewContract {
    name: String,
    connected: bool,
    enabled: bool,
    has_internal_display: bool,
    current_mode: Option<ModeViewContract>,
    modes: Vec<ModeViewContract>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ModeViewContract {
    width: u32,
    height: u32,
    refresh_hz: f32,
    preferred: bool,
    current: bool,
}

impl DisplayInfoViewContract {
    fn validate(&self) -> Result<(), String> {
        if self.internal_output.modes.len() > MAX_MODES {
            return Err(format!(
                "trop de modes: {} > {MAX_MODES}",
                self.internal_output.modes.len()
            ));
        }
        for mode in &self.internal_output.modes {
            validate_mode(mode)?;
        }
        if let Some(mode) = &self.internal_output.current_mode {
            validate_mode(mode)?;
        }
        if self.supported && !self.internal_output.has_internal_display {
            return Err("supported=true sans écran interne".to_owned());
        }
        if self.available && !self.backend.available && self.error.is_none() {
            return Err("available=true avec backend indisponible et sans erreur".to_owned());
        }
        Ok(())
    }

    fn canonical(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("re-sérialisation du contrat")
    }
}

fn validate_mode(mode: &ModeViewContract) -> Result<(), String> {
    if mode.width == 0 || mode.width > MAX_DIMENSION {
        return Err(format!("largeur hors bornes: {}", mode.width));
    }
    if mode.height == 0 || mode.height > MAX_DIMENSION {
        return Err(format!("hauteur hors bornes: {}", mode.height));
    }
    if !mode.refresh_hz.is_finite() || !(MIN_REFRESH_HZ..=MAX_REFRESH_HZ).contains(&mode.refresh_hz)
    {
        return Err(format!("taux hors bornes: {}", mode.refresh_hz));
    }
    Ok(())
}

fn read_json(name: &str) -> serde_json::Value {
    let raw = fixtures::read_fixture(name);
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{name}: JSON invalide: {error}"))
}

fn read_view(name: &str) -> DisplayInfoViewContract {
    let raw = fixtures::read_fixture(name);
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{name}: vue invalide: {error}"))
}

// ── Construction de la charge attendue ─────────────────────────────────────

fn session() -> SessionInfo {
    SessionInfo::from_env(SessionEnv {
        xdg_session_type: Some("wayland".to_owned()),
        xdg_current_desktop: Some("Hyprland".to_owned()),
        wayland_display: Some("wayland-1".to_owned()),
        display: None,
        xdg_runtime_dir: Some("/run/user/1000".to_owned()),
        hyprland_instance_signature: Some("sig".to_owned()),
        sway_sock: None,
    })
}

fn session_x11() -> SessionInfo {
    SessionInfo::from_env(SessionEnv {
        xdg_session_type: Some("x11".to_owned()),
        xdg_current_desktop: Some("Xfce".to_owned()),
        wayland_display: None,
        display: Some(":0".to_owned()),
        xdg_runtime_dir: Some("/run/user/1000".to_owned()),
        hyprland_instance_signature: None,
        sway_sock: None,
    })
}

fn mode(width: u32, height: u32, refresh_hz: f32, preferred: bool, current: bool) -> DisplayMode {
    DisplayMode {
        width,
        height,
        refresh_hz,
        preferred,
        current,
    }
}

fn internal_output() -> OutputInfo {
    let mut output = OutputInfo::new("eDP-1");
    output.connected = true;
    output.enabled = true;
    output.current_mode = Some(mode(2560, 1600, 60.0, false, true));
    output.modes = vec![
        mode(2560, 1600, 60.0, false, true),
        mode(2560, 1600, 165.0, true, false),
        mode(1920, 1200, 60.0, false, false),
        mode(1920, 1200, 120.0, false, false),
    ];
    output
}

fn external_output() -> OutputInfo {
    let mut output = OutputInfo::new("HDMI-A-1");
    output.connected = true;
    output.enabled = true;
    output.current_mode = Some(mode(3840, 2160, 60.0, false, true));
    output.modes = vec![mode(3840, 2160, 60.0, false, true)];
    output
}

fn full_info() -> DisplayInfo {
    DisplayInfo::build(
        &session(),
        DisplayBackendKind::Hyprland,
        "hyprland",
        true,
        Some(vec![internal_output(), external_output()]),
        None,
        false,
    )
}

fn unavailable_info() -> DisplayInfo {
    DisplayInfo::unavailable(&session(), "backend absent")
}

fn no_internal_info() -> DisplayInfo {
    DisplayInfo::build(
        &session_x11(),
        DisplayBackendKind::X11,
        "x11",
        true,
        Some(vec![external_output()]),
        None,
        false,
    )
}

// ── Fixtures et contrat ────────────────────────────────────────────────────

#[test]
fn full_fixture_is_canonical_for_production_serialization() {
    let fixture = read_json("dbus/display_info_full.json");
    let produced = serde_json::to_value(full_info()).expect("sérialisation");
    assert_eq!(fixture, produced);
    // L'écran externe est présent dans les sorties du backend mais ne doit
    // jamais apparaître dans le rapport.
    let text = produced.to_string();
    assert!(!text.contains("HDMI-A-1"));
    assert!(!text.contains("3840"));
}

#[test]
fn unavailable_backend_fixture_is_canonical() {
    assert_eq!(
        read_json("dbus/display_info_backend_unavailable.json"),
        serde_json::to_value(unavailable_info()).expect("sérialisation")
    );
}

#[test]
fn no_internal_display_fixture_is_canonical() {
    assert_eq!(
        read_json("dbus/display_info_no_internal.json"),
        serde_json::to_value(no_internal_info()).expect("sérialisation")
    );
}

#[test]
fn full_fixture_has_multiple_resolutions_and_rates() {
    let view = read_view("dbus/display_info_full.json");
    view.validate().expect("contrat valide");
    assert!(view.available);
    assert!(view.supported);
    assert_eq!(view.session.session_type, "wayland");
    assert_eq!(view.backend.kind, "hyprland");
    assert_eq!(view.internal_output.name, "eDP-1");
    assert!(view.internal_output.has_internal_display);
    assert_eq!(view.internal_output.modes.len(), 4);
    assert_eq!(
        view.internal_output.current_mode.as_ref().map(|mode| (
            mode.width,
            mode.height,
            mode.refresh_hz
        )),
        Some((2560, 1600, 60.0))
    );
    let mut resolutions: Vec<(u32, u32)> = view
        .internal_output
        .modes
        .iter()
        .map(|mode| (mode.width, mode.height))
        .collect();
    resolutions.sort_unstable();
    resolutions.dedup();
    assert_eq!(resolutions, vec![(1920, 1200), (2560, 1600)]);
    assert!(
        view.internal_output
            .modes
            .iter()
            .any(|mode| mode.refresh_hz == 165.0 && mode.preferred)
    );
}

#[test]
fn unavailable_and_no_internal_fixtures_are_valid_contracts() {
    let unavailable = read_view("dbus/display_info_backend_unavailable.json");
    unavailable.validate().expect("backend indisponible valide");
    assert!(!unavailable.available);
    assert!(!unavailable.supported);
    assert_eq!(unavailable.error.as_deref(), Some("backend absent"));
    assert!(unavailable.internal_output.modes.is_empty());

    let no_internal = read_view("dbus/display_info_no_internal.json");
    no_internal
        .validate()
        .expect("absence d'écran interne valide");
    assert!(no_internal.available);
    assert!(!no_internal.supported);
    assert!(!no_internal.internal_output.has_internal_display);
    assert!(no_internal.internal_output.modes.is_empty());
    assert_eq!(no_internal.error, None);
}

#[test]
fn contract_fixture_matches_public_constants() {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Contract {
        contract_version: u32,
        service: String,
        path: String,
        interface: String,
        methods: Vec<String>,
        read_only: bool,
    }

    let contract: Contract =
        serde_json::from_str(&fixtures::read_fixture("dbus/display_info_contract.json"))
            .expect("fixture de contrat");
    assert_eq!(contract.contract_version, SESSION_CONTRACT_VERSION);
    assert_eq!(contract.service, DBUS_SESSION_AGENT_SERVICE);
    assert_eq!(contract.path, DBUS_SESSION_AGENT_PATH);
    assert_eq!(contract.interface, DBUS_SESSION_AGENT_INTERFACE);
    assert_eq!(
        contract.methods,
        vec![
            "Ping".to_owned(),
            "GetVersion".to_owned(),
            "GetContractVersion".to_owned(),
            "GetDisplayInfoJSON".to_owned(),
            "GetUvcWriteSupported".to_owned(),
            "SetUvcControl".to_owned(),
        ]
    );
    // L'affichage reste en lecture seule ; l'écriture UVC de session est
    // additive (`SetUvcControl`), d'où `readOnly=false`.
    assert!(!contract.read_only);
}

#[test]
fn wrong_types_are_rejected() {
    let raw = fixtures::read_fixture("dbus/invalid/display_info_wrong_types.json");
    assert!(serde_json::from_str::<DisplayInfoViewContract>(&raw).is_err());
}

#[test]
fn zero_and_oversized_dimensions_are_rejected_by_the_consumer() {
    for (name, expected) in [
        (
            "dbus/invalid/display_info_zero_dimensions.json",
            "hors bornes",
        ),
        (
            "dbus/invalid/display_info_oversized_dimensions.json",
            "hors bornes",
        ),
    ] {
        let view = read_view(name);
        let error = view.validate().expect_err(name);
        assert!(
            error.contains(expected),
            "{name}: erreur inattendue: {error}"
        );
    }
}

#[test]
fn external_outputs_are_dropped_and_modes_stay_internal() {
    let view = read_view("dbus/invalid/display_info_external_modes.json");
    // Les champs `externalOutput`/`externalOutputs` sont inconnus du contrat :
    // ils disparaissent à la re-sérialisation.
    view.validate().expect("modes internes valides");
    let canonical = view.canonical().to_string();
    assert!(!canonical.contains("externalOutput"));
    assert!(!canonical.contains("HDMI-A-1"));
    assert!(!canonical.contains("\"DP-1\""));
    assert!(!canonical.contains("3840"));
    assert!(!canonical.contains("3440"));

    // Le producteur ne copie que les modes de l'écran interne.
    let produced = serde_json::to_value(full_info()).expect("sérialisation");
    assert_eq!(
        produced["internalOutput"]["modes"].as_array().map(Vec::len),
        Some(4)
    );
}

// ── Service `User1` sur bus de session ─────────────────────────────────────

struct FakeSource {
    info: DisplayInfo,
    calls: AtomicUsize,
}

impl FakeSource {
    fn new(info: DisplayInfo) -> Self {
        Self {
            info,
            calls: AtomicUsize::new(0),
        }
    }
}

impl DisplayInfoSource for FakeSource {
    fn display_info(&self) -> DisplayInfo {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.info.clone()
    }
}

fn unique_name(suffix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    format!(
        "com.tuxedocomputers.tccd.User.Test{}.{}{}",
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

async fn proxy<'a>(client: &'a zbus::Connection, name: &str) -> zbus::Proxy<'a> {
    zbus::Proxy::new(
        client,
        name.to_owned(),
        DBUS_SESSION_AGENT_PATH,
        DBUS_SESSION_AGENT_INTERFACE,
    )
    .await
    .expect("proxy User1")
}

#[tokio::test]
async fn user1_serves_contract_and_fixture_payload_on_session_bus() {
    let Ok(client) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test User1 ignoré");
        return;
    };
    let name = unique_name("serve");
    let source = Arc::new(FakeSource::new(full_info()));
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let task_source = Arc::clone(&source) as Arc<dyn DisplayInfoSource>;
    let task_name = name.clone();
    let task_shutdown = Arc::clone(&shutdown);
    let handle = tokio::spawn(async move {
        let result = serve_with(task_source, task_name, Some(task_shutdown)).await;
        if let Err(error) = &result {
            eprintln!("serve_with a échoué: {error:?}");
        }
        result
    });
    assert!(
        wait_for_owner(&client, &name, true).await,
        "le service doit posséder son nom"
    );

    let proxy = proxy(&client, &name).await;
    let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
    assert!(pong);
    let version: String = proxy.call("GetVersion", &()).await.expect("version");
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    let contract: u32 = proxy
        .call("GetContractVersion", &())
        .await
        .expect("contrat");
    assert_eq!(contract, SESSION_CONTRACT_VERSION);

    let json: String = proxy
        .call("GetDisplayInfoJSON", &())
        .await
        .expect("display info");
    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    assert_eq!(value, read_json("dbus/display_info_full.json"));

    // Deux appels successifs restent identiques (pas de fuite d'état).
    let json_again: String = proxy
        .call("GetDisplayInfoJSON", &())
        .await
        .expect("display info 2");
    assert_eq!(json, json_again);
    assert!(source.calls.load(Ordering::Relaxed) >= 2);

    shutdown.notify_one();
    handle.await.expect("arrêt").expect("service");
    assert!(
        wait_for_owner(&client, &name, false).await,
        "le nom doit être libéré après l'arrêt"
    );
}

#[tokio::test]
async fn user1_reports_backend_unavailable_fixture() {
    let Ok(client) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test User1 ignoré");
        return;
    };
    let name = unique_name("unavailable");
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let handle = tokio::spawn(serve_with(
        Arc::new(FakeSource::new(unavailable_info())) as Arc<dyn DisplayInfoSource>,
        name.clone(),
        Some(Arc::clone(&shutdown)),
    ));
    assert!(wait_for_owner(&client, &name, true).await);
    let proxy = proxy(&client, &name).await;
    let json: String = proxy
        .call("GetDisplayInfoJSON", &())
        .await
        .expect("display info");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).expect("JSON"),
        read_json("dbus/display_info_backend_unavailable.json")
    );
    shutdown.notify_one();
    handle.await.expect("arrêt").expect("service");
}

#[tokio::test]
async fn user1_name_taken_is_reported_and_first_service_stays_alive() {
    let Ok(client) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test User1 ignoré");
        return;
    };
    let name = unique_name("taken");
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let first = tokio::spawn(serve_with(
        Arc::new(FakeSource::new(full_info())) as Arc<dyn DisplayInfoSource>,
        name.clone(),
        Some(Arc::clone(&shutdown)),
    ));
    assert!(wait_for_owner(&client, &name, true).await);

    let second = serve_with(
        Arc::new(FakeSource::new(full_info())) as Arc<dyn DisplayInfoSource>,
        name.clone(),
        Some(Arc::new(tokio::sync::Notify::new())),
    )
    .await;
    assert!(matches!(
        second,
        Err(SessionServiceError::NameUnavailable(_))
    ));

    let proxy = proxy(&client, &name).await;
    let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
    assert!(pong, "le premier service doit rester vivant");

    shutdown.notify_one();
    first.await.expect("arrêt").expect("service");
}

#[test]
fn display_fixtures_stay_small() {
    for name in [
        "dbus/display_info_full.json",
        "dbus/display_info_backend_unavailable.json",
        "dbus/display_info_no_internal.json",
        "dbus/display_info_contract.json",
        "dbus/invalid/display_info_wrong_types.json",
        "dbus/invalid/display_info_zero_dimensions.json",
        "dbus/invalid/display_info_oversized_dimensions.json",
        "dbus/invalid/display_info_external_modes.json",
        "dbus/profile_collection_resolution.json",
        "dbus/invalid/profile_collection_bad_resolution.json",
    ] {
        let path = fixtures::fixture_path(name);
        let size = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("métadonnées de {}: {error}", path.display()))
            .len();
        assert!(
            size <= 64 * 1024,
            "{} dépasse la règle des 64 Kio: {size} octets",
            path.display()
        );
    }
}
