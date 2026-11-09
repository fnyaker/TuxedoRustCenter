// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Backend GNOME Wayland via `org.gnome.Mutter.DisplayConfig`.
//!
//! # API Mutter utilisée
//!
//! - `GetCurrentState() -> (u serial, monitors, logical_monitors, properties)` ;
//! - `ApplyMonitorsConfig(u serial, u method, logical_monitors, properties)`
//!   avec `method = 1` (temporaire, jamais persistant dans `monitors.xml`).
//!
//! Le mode est identifié par la **chaîne exacte** fournie par Mutter dans
//! `GetCurrentState` (`a(siiddada{sv})`) ; il n'est ni recalculé ni formaté.
//! Le connecteur interne (eDP/LVDS/DSI) est le seul dont l'identifiant de mode
//! est remplacé. Les autres moniteurs conservent leur mode courant ; si ce
//! mode n'est pas exposé, l'application est refusée plutôt que de risquer de
//! modifier un écran externe.
//!
//! En cas de serial périmé (Mutter répond `AccessDenied`/`InvalidArgs`),
//! l'état est relu et l'application retentée **une seule fois**.
//!
//! Le backend est synchrone (trait `DisplayModeBackend`) et embarque un
//! runtime Tokio courant-thread : il doit être appelé depuis un thread
//! bloquant, comme les autres backends.

use std::collections::HashMap;
use std::time::Duration;

use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    rates_match,
};
use tcc_hw::error::{HwError, Result as HwResult};
use zbus::zvariant::OwnedValue;

use crate::backend::{ensure_internal, validate_internal_output_name};
use crate::error::{Result, UserError};

/// Service D-Bus de Mutter.
pub const MUTTER_SERVICE: &str = "org.gnome.Mutter.DisplayConfig";
/// Objet D-Bus de Mutter.
pub const MUTTER_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
/// Interface D-Bus de Mutter.
pub const MUTTER_INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";

/// Délai maximal d'un appel Mutter (lecture ou application).
const DBUS_TIMEOUT: Duration = Duration::from_secs(3);
/// Méthode `temporary` : ne persiste pas la configuration dans `monitors.xml`.
const APPLY_METHOD_TEMPORARY: u32 = 1;
/// Taille maximale d'un identifiant de mode Mutter.
const MAX_MODE_ID_LEN: usize = 256;

type MonitorKey = (String, String, String, String);
type RawMode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);
type RawMonitor = (MonitorKey, Vec<RawMode>, HashMap<String, OwnedValue>);
type RawLogical = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<MonitorKey>,
    HashMap<String, OwnedValue>,
);
type RawState = (
    u32,
    Vec<RawMonitor>,
    Vec<RawLogical>,
    HashMap<String, OwnedValue>,
);

/// Moniteur envoyé à `ApplyMonitorsConfig` : connecteur, mode exact, propriétés.
type ApplyMonitor = (String, String, HashMap<String, OwnedValue>);
/// Moniteur logique envoyé à `ApplyMonitorsConfig`.
type ApplyLogical = (i32, i32, f64, u32, bool, Vec<ApplyMonitor>);

#[derive(Debug, Clone)]
struct MutterMode {
    id: String,
    width: u32,
    height: u32,
    refresh_hz: f32,
    preferred: bool,
    current: bool,
}

#[derive(Debug, Clone)]
struct MutterMonitor {
    key: MonitorKey,
    modes: Vec<MutterMode>,
    builtin: bool,
}

#[derive(Debug, Clone)]
struct MutterLogical {
    x: i32,
    y: i32,
    scale: f64,
    transform: u32,
    primary: bool,
    monitors: Vec<MonitorKey>,
}

#[derive(Debug, Clone)]
struct MutterState {
    serial: u32,
    monitors: Vec<MutterMonitor>,
    logical: Vec<MutterLogical>,
}

impl MutterState {
    fn from_raw(raw: RawState) -> HwResult<Self> {
        let (serial, raw_monitors, raw_logical, _properties) = raw;
        let monitors = raw_monitors
            .into_iter()
            .filter_map(|(key, modes, properties)| {
                if key.0.is_empty() {
                    return None;
                }
                let modes = modes
                    .into_iter()
                    .filter_map(|(id, width, height, refresh_hz, _scale, _scales, props)| {
                        if id.is_empty()
                            || id.len() > MAX_MODE_ID_LEN
                            || width <= 0
                            || height <= 0
                            || !refresh_hz.is_finite()
                            || refresh_hz <= 0.0
                        {
                            return None;
                        }
                        Some(MutterMode {
                            id,
                            width: width as u32,
                            height: height as u32,
                            refresh_hz: refresh_hz as f32,
                            preferred: bool_property(&props, "is-preferred").unwrap_or(false),
                            current: bool_property(&props, "is-current").unwrap_or(false),
                        })
                    })
                    .collect();
                Some(MutterMonitor {
                    key,
                    modes,
                    builtin: bool_property(&properties, "is-builtin").unwrap_or(false),
                })
            })
            .collect();

        let logical = raw_logical
            .into_iter()
            .map(
                |(x, y, scale, transform, primary, monitors, _props)| MutterLogical {
                    x,
                    y,
                    scale,
                    transform,
                    primary,
                    monitors,
                },
            )
            .collect();

        Ok(Self {
            serial,
            monitors,
            logical,
        })
    }

    fn to_outputs(&self) -> Vec<OutputInfo> {
        let enabled: std::collections::HashSet<&str> = self
            .logical
            .iter()
            .flat_map(|logical| logical.monitors.iter().map(|key| key.0.as_str()))
            .collect();

        self.monitors
            .iter()
            .map(|monitor| {
                let mut output = OutputInfo::new(&monitor.key.0);
                output.connected = true;
                output.enabled = enabled.contains(monitor.key.0.as_str())
                    || monitor.modes.iter().any(|mode| mode.current);
                output.current_mode = monitor
                    .modes
                    .iter()
                    .find(|mode| mode.current)
                    .map(display_mode_from_mutter);
                output.modes = monitor.modes.iter().map(display_mode_from_mutter).collect();
                if !monitor.builtin && output.internal {
                    // Le nom du connecteur reste la source de vérité (règle
                    // commune `is_internal_connector`) ; `is-builtin` n'est
                    // qu'une indication supplémentaire.
                }
                output
            })
            .collect()
    }

    fn current_mode_id(&self, connector: &str) -> Option<&str> {
        self.monitors
            .iter()
            .find(|monitor| monitor.key.0 == connector)
            .and_then(|monitor| monitor.modes.iter().find(|mode| mode.current))
            .map(|mode| mode.id.as_str())
    }

    /// Construit la configuration Mutter qui ne change que le connecteur
    /// interne.
    fn plan(&self, output: &InternalOutput, target: &DisplayMode) -> HwResult<ApplyPlan> {
        let name = output.name();
        let monitor = self
            .monitors
            .iter()
            .find(|monitor| monitor.key.0 == name)
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!("moniteur Mutter {name} introuvable"))
            })?;
        let selected = monitor
            .modes
            .iter()
            .filter(|mode| mode.width == target.width && mode.height == target.height)
            .min_by(|left, right| {
                (left.refresh_hz - target.refresh_hz)
                    .abs()
                    .total_cmp(&(right.refresh_hz - target.refresh_hz).abs())
            })
            .filter(|mode| rates_match(mode.refresh_hz, target.refresh_hz, 0.5))
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!(
                    "{} Hz à {}x{} sur {name}",
                    target.refresh_hz, target.width, target.height
                ))
            })?;
        validate_mode_id(&selected.id)?;

        let mut applied = Vec::with_capacity(self.logical.len());
        let mut found = false;
        for logical in &self.logical {
            let mut monitors = Vec::with_capacity(logical.monitors.len());
            for key in &logical.monitors {
                let connector = &key.0;
                let mode_id = if connector == name {
                    found = true;
                    selected.id.clone()
                } else {
                    self.current_mode_id(connector)
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            HwError::Display(format!(
                                "mode courant de l'écran externe {connector} introuvable ; \
                                 configuration refusée pour ne pas le modifier"
                            ))
                        })?
                };
                monitors.push((connector.clone(), mode_id, HashMap::new()));
            }
            applied.push((
                logical.x,
                logical.y,
                logical.scale,
                logical.transform,
                logical.primary,
                monitors,
            ));
        }

        if !found {
            return Err(HwError::Display(format!(
                "{name} n'apparaît dans aucun moniteur logique Mutter"
            )));
        }
        Ok(ApplyPlan {
            serial: self.serial,
            logical: applied,
        })
    }
}

fn display_mode_from_mutter(mode: &MutterMode) -> DisplayMode {
    DisplayMode {
        width: mode.width,
        height: mode.height,
        refresh_hz: mode.refresh_hz,
        preferred: mode.preferred,
        current: mode.current,
    }
}

#[derive(Debug, Clone)]
struct ApplyPlan {
    serial: u32,
    logical: Vec<ApplyLogical>,
}

/// Backend GNOME Wayland.
#[derive(Debug)]
pub struct GnomeBackend {
    runtime: tokio::runtime::Runtime,
    proxy: zbus::Proxy<'static>,
    timeout: Duration,
}

impl GnomeBackend {
    /// Construit le backend sur le service Mutter de la session.
    pub fn new() -> Result<Self> {
        Self::with_service(MUTTER_SERVICE, DBUS_TIMEOUT)
    }

    /// Construit le backend avec un service/timeout explicites (tests).
    fn with_service(service: &str, timeout: Duration) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| UserError::BackendUnavailable {
                backend: "gnome",
                reason: format!("runtime Tokio : {error}"),
            })?;
        let proxy = runtime
            .block_on(connect_proxy(service, timeout))
            .map_err(|reason| UserError::BackendUnavailable {
                backend: "gnome",
                reason,
            })?;
        Ok(Self {
            runtime,
            proxy,
            timeout,
        })
    }

    async fn fetch_state_async(&self) -> HwResult<MutterState> {
        let result =
            tokio::time::timeout(self.timeout, self.proxy.call_method("GetCurrentState", &()))
                .await
                .map_err(|_| HwError::Display("Mutter GetCurrentState : délai dépassé".to_owned()))?
                .map_err(|error| map_dbus_error("GetCurrentState", &error))?;
        let raw: RawState = result
            .body()
            .deserialize()
            .map_err(|error| HwError::Display(format!("GetCurrentState invalide : {error}")))?;
        MutterState::from_raw(raw)
    }

    async fn apply_plan_async(&self, plan: &ApplyPlan) -> std::result::Result<(), zbus::Error> {
        let properties: HashMap<String, OwnedValue> = HashMap::new();
        let body = (
            plan.serial,
            APPLY_METHOD_TEMPORARY,
            plan.logical.clone(),
            properties,
        );
        tokio::time::timeout(
            self.timeout,
            self.proxy.call_method("ApplyMonitorsConfig", &body),
        )
        .await
        .map_err(|_| zbus::Error::Failure("Mutter ApplyMonitorsConfig : délai dépassé".to_owned()))?
        .map(|_| ())
    }

    #[cfg(test)]
    fn with_service_for_test(service: &str, timeout: Duration) -> Result<Self> {
        Self::with_service(service, timeout)
    }
}

async fn connect_proxy(
    service: &str,
    timeout: Duration,
) -> std::result::Result<zbus::Proxy<'static>, String> {
    let connection = tokio::time::timeout(timeout, zbus::Connection::session())
        .await
        .map_err(|_| "connexion au bus de session : délai dépassé".to_owned())?
        .map_err(|error| format!("connexion au bus de session : {error}"))?;
    tokio::time::timeout(
        timeout,
        zbus::Proxy::new_owned(
            connection,
            service.to_owned(),
            MUTTER_PATH.to_owned(),
            MUTTER_INTERFACE.to_owned(),
        ),
    )
    .await
    .map_err(|_| "proxy Mutter : délai dépassé".to_owned())?
    .map_err(|error| format!("proxy Mutter : {error}"))
}

impl DisplayModeBackend for GnomeBackend {
    fn backend_name(&self) -> &'static str {
        "gnome"
    }

    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Gnome
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        let state = self.runtime.block_on(self.fetch_state_async())?;
        Ok(state.to_outputs())
    }

    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        ensure_internal(output)?;
        if !validate_internal_output_name(output.name()) {
            return Err(HwError::NotInternalOutput(output.name().to_owned()));
        }
        self.runtime.block_on(async {
            let state = self.fetch_state_async().await?;
            let plan = state.plan(output, mode)?;
            match self.apply_plan_async(&plan).await {
                Ok(()) => Ok(ApplyOutcome::Applied),
                Err(error) if is_retryable_apply_error(&error) => {
                    // Serial périmé (ou configuration changée entre-temps) :
                    // relire l'état et réessayer une seule fois.
                    tracing::debug!(%error, "Mutter ApplyMonitorsConfig : relecture après serial périmé");
                    let state = self.fetch_state_async().await?;
                    let plan = state.plan(output, mode)?;
                    self.apply_plan_async(&plan).await.map_err(|error| {
                        map_dbus_error("ApplyMonitorsConfig (retry)", &error)
                    })?;
                    Ok(ApplyOutcome::Applied)
                }
                Err(error) => Err(map_dbus_error("ApplyMonitorsConfig", &error)),
            }
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn bool_property(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    properties
        .get(key)
        .and_then(|value| value.downcast_ref::<bool>().ok())
}

fn validate_mode_id(mode_id: &str) -> HwResult<()> {
    if mode_id.is_empty()
        || mode_id.len() > MAX_MODE_ID_LEN
        || mode_id.chars().any(char::is_control)
    {
        return Err(HwError::Display(
            "identifiant de mode Mutter invalide".to_owned(),
        ));
    }
    Ok(())
}

fn is_retryable_apply_error(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.DBus.Error.InvalidArgs"
        ),
        _ => false,
    }
}

fn map_dbus_error(context: &str, error: &zbus::Error) -> HwError {
    match error {
        zbus::Error::MethodError(name, _, _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
                    | "org.freedesktop.DBus.Error.UnknownMethod"
            ) =>
        {
            let _ = context;
            HwError::NotSupported("Mutter DisplayConfig indisponible")
        }
        other => HwError::Display(format!("{context} : {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use tcc_hw::display::apply_internal_refresh_rate;
    use zbus::zvariant::Value;

    use super::*;

    fn owned(value: bool) -> OwnedValue {
        OwnedValue::try_from(Value::from(value)).expect("OwnedValue")
    }

    fn mode(id: &str, width: i32, height: i32, hz: f64, current: bool, preferred: bool) -> RawMode {
        let mut properties = HashMap::new();
        if current {
            properties.insert("is-current".to_owned(), owned(true));
        }
        if preferred {
            properties.insert("is-preferred".to_owned(), owned(true));
        }
        (id.to_owned(), width, height, hz, 1.0, vec![1.0], properties)
    }

    fn monitor(connector: &str, builtin: bool, modes: Vec<RawMode>) -> RawMonitor {
        let mut properties = HashMap::new();
        if builtin {
            properties.insert("is-builtin".to_owned(), owned(true));
        }
        (
            (
                connector.to_owned(),
                "vendor".to_owned(),
                "product".to_owned(),
                "serial".to_owned(),
            ),
            modes,
            properties,
        )
    }

    fn logical(connectors: &[&str]) -> RawLogical {
        (
            0,
            0,
            1.0,
            0,
            true,
            connectors
                .iter()
                .map(|connector| {
                    (
                        (*connector).to_owned(),
                        "vendor".to_owned(),
                        "product".to_owned(),
                        "serial".to_owned(),
                    )
                })
                .collect(),
            HashMap::new(),
        )
    }

    fn sample_state() -> RawState {
        (
            7,
            vec![
                monitor(
                    "eDP-1",
                    true,
                    vec![
                        mode("edp60", 2560, 1600, 60.0, true, false),
                        mode("edp120", 2560, 1600, 120.0, false, true),
                        mode("edp1920", 1920, 1080, 60.0, false, false),
                    ],
                ),
                monitor(
                    "HDMI-A-1",
                    false,
                    vec![mode("hdmi60", 1920, 1080, 60.0, true, false)],
                ),
            ],
            vec![logical(&["eDP-1"]), logical(&["HDMI-A-1"])],
            HashMap::new(),
        )
    }

    #[test]
    fn parses_state_and_selects_internal_output() {
        let state = MutterState::from_raw(sample_state()).expect("state");
        let outputs = state.to_outputs();
        assert_eq!(outputs.len(), 2);
        let internal = crate::backend::internal_output(&outputs).expect("internal");
        assert_eq!(internal.name(), "eDP-1");
        assert_eq!(internal.current_refresh_hz(), Some(60.0));
        assert_eq!(internal.modes_at_current_resolution().len(), 2);
        assert_eq!(state.serial, 7);
    }

    #[test]
    fn malformed_modes_are_filtered() {
        let raw = (
            1,
            vec![monitor(
                "eDP-1",
                true,
                vec![
                    mode("", 2560, 1600, 60.0, true, false),
                    mode("zero", 0, 1600, 60.0, false, false),
                    mode("nan", 2560, 1600, f64::NAN, false, false),
                    mode("ok", 2560, 1600, 60.0, true, false),
                ],
            )],
            vec![logical(&["eDP-1"])],
            HashMap::new(),
        );
        let state = MutterState::from_raw(raw).expect("state");
        assert_eq!(state.monitors[0].modes.len(), 1);
        assert_eq!(state.monitors[0].modes[0].id, "ok");
    }

    #[test]
    fn external_only_has_no_internal_output() {
        let raw = (
            1,
            vec![monitor(
                "HDMI-A-1",
                false,
                vec![mode("hdmi60", 1920, 1080, 60.0, true, false)],
            )],
            vec![logical(&["HDMI-A-1"])],
            HashMap::new(),
        );
        let state = MutterState::from_raw(raw).expect("state");
        assert!(
            crate::backend::internal_output(&state.to_outputs()).is_none(),
            "aucun écran interne ne doit être sélectionné"
        );
    }

    #[test]
    fn plan_preserves_external_mode() {
        let state = MutterState::from_raw(sample_state()).expect("state");
        let outputs = state.to_outputs();
        let internal = crate::backend::internal_output(&outputs).expect("internal");
        let target = DisplayMode::new(2560, 1600, 120.0);
        let plan = state.plan(&internal, &target).expect("plan");
        assert_eq!(plan.serial, 7);
        let edp = plan
            .logical
            .iter()
            .flat_map(|logical| &logical.5)
            .find(|monitor| monitor.0 == "eDP-1")
            .expect("eDP");
        assert_eq!(edp.1, "edp120");
        let hdmi = plan
            .logical
            .iter()
            .flat_map(|logical| &logical.5)
            .find(|monitor| monitor.0 == "HDMI-A-1")
            .expect("HDMI");
        assert_eq!(hdmi.1, "hdmi60", "l'écran externe ne change pas");
    }

    #[test]
    fn missing_rate_is_not_found() {
        let state = MutterState::from_raw(sample_state()).expect("state");
        let outputs = state.to_outputs();
        let internal = crate::backend::internal_output(&outputs).expect("internal");
        let target = DisplayMode::new(2560, 1600, 144.0);
        assert!(matches!(
            state.plan(&internal, &target),
            Err(HwError::DisplayModeNotFound(_))
        ));
    }

    #[test]
    fn invalid_mode_ids_are_rejected() {
        assert!(validate_mode_id("").is_err());
        assert!(validate_mode_id("mode\ninjection").is_err());
        assert!(validate_mode_id(&"x".repeat(MAX_MODE_ID_LEN + 1)).is_err());
        assert!(validate_mode_id("edp120").is_ok());
    }

    #[derive(Debug)]
    struct FakeInner {
        state: Mutex<RawState>,
        attempts: Mutex<Vec<u32>>,
        applied: Mutex<Vec<(u32, u32, Vec<ApplyLogical>)>>,
        stale_once: AtomicBool,
        deny: AtomicBool,
        delay: Duration,
        calls: AtomicUsize,
    }

    #[derive(Debug, Clone)]
    struct FakeMutter {
        inner: std::sync::Arc<FakeInner>,
    }

    impl FakeMutter {
        fn new(state: RawState) -> Self {
            Self::with_delay(state, Duration::ZERO)
        }

        fn with_delay(state: RawState, delay: Duration) -> Self {
            Self {
                inner: std::sync::Arc::new(FakeInner {
                    state: Mutex::new(state),
                    attempts: Mutex::new(Vec::new()),
                    applied: Mutex::new(Vec::new()),
                    stale_once: AtomicBool::new(false),
                    deny: AtomicBool::new(false),
                    delay,
                    calls: AtomicUsize::new(0),
                }),
            }
        }

        fn stale_once(&self) -> &AtomicBool {
            &self.inner.stale_once
        }

        fn deny(&self) -> &AtomicBool {
            &self.inner.deny
        }

        fn attempts(&self) -> Vec<u32> {
            self.inner
                .attempts
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }

        fn applied(&self) -> Vec<(u32, u32, Vec<ApplyLogical>)> {
            self.inner
                .applied
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    #[zbus::interface(name = "org.gnome.Mutter.DisplayConfig")]
    impl FakeMutter {
        fn get_current_state(&self) -> RawState {
            self.inner.state.lock().expect("state").clone()
        }

        async fn apply_monitors_config(
            &self,
            serial: u32,
            method: u32,
            logical: Vec<ApplyLogical>,
            _properties: HashMap<String, OwnedValue>,
        ) -> zbus::fdo::Result<()> {
            self.inner.calls.fetch_add(1, Ordering::SeqCst);
            if !self.inner.delay.is_zero() {
                tokio::time::sleep(self.inner.delay).await;
            }
            if let Ok(mut attempts) = self.inner.attempts.lock() {
                attempts.push(serial);
            }
            if self.inner.deny.load(Ordering::SeqCst) {
                return Err(zbus::fdo::Error::AccessDenied(
                    "session verrouillée".to_owned(),
                ));
            }
            if self.inner.stale_once.swap(false, Ordering::SeqCst) {
                if let Ok(mut state) = self.inner.state.lock() {
                    state.0 = state.0.saturating_add(1);
                }
                return Err(zbus::fdo::Error::AccessDenied("serial périmé".to_owned()));
            }
            let expected = self
                .inner
                .state
                .lock()
                .map(|state| state.0)
                .unwrap_or_default();
            if serial != expected {
                return Err(zbus::fdo::Error::AccessDenied("serial périmé".to_owned()));
            }
            if let Ok(mut applied) = self.inner.applied.lock() {
                applied.push((serial, method, logical));
            }
            Ok(())
        }
    }

    static NEXT_TEST: AtomicU32 = AtomicU32::new(0);

    async fn start_fake(fake: FakeMutter) -> Option<(zbus::Connection, String)> {
        let connection = zbus::Connection::session().await.ok()?;
        let name = format!(
            "com.example.MutterTest{}_{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::SeqCst)
        );
        connection.request_name(name.clone()).await.ok()?;
        connection
            .object_server()
            .at(MUTTER_PATH, fake)
            .await
            .ok()?;
        Some((connection, name))
    }

    async fn run_backend<T, F>(name: String, timeout: Duration, operation: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut GnomeBackend) -> T + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let mut backend = GnomeBackend::with_service_for_test(&name, timeout).ok()?;
            Some(operation(&mut backend))
        })
        .await
        .unwrap_or_default()
    }

    #[tokio::test]
    async fn applies_internal_mode_only() {
        let fake = FakeMutter::new(sample_state());
        let Some((_service, name)) = start_fake(fake.clone()).await else {
            eprintln!("bus de session absent : test Mutter ignoré");
            return;
        };
        let Some(outcome) = run_backend(name, Duration::from_secs(1), |backend| {
            apply_internal_refresh_rate(backend, 120.0, 0.5)
        })
        .await
        else {
            panic!("backend Mutter");
        };
        assert_eq!(outcome.expect("application"), ApplyOutcome::Applied);
        let applied = fake.applied();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].1, APPLY_METHOD_TEMPORARY);
        let logical = &applied[0].2;
        let edp = logical
            .iter()
            .flat_map(|entry| &entry.5)
            .find(|monitor| monitor.0 == "eDP-1")
            .expect("eDP");
        assert_eq!(edp.1, "edp120");
        let hdmi = logical
            .iter()
            .flat_map(|entry| &entry.5)
            .find(|monitor| monitor.0 == "HDMI-A-1")
            .expect("HDMI");
        assert_eq!(hdmi.1, "hdmi60");
    }

    #[tokio::test]
    async fn stale_serial_retries_once() {
        let fake = FakeMutter::new(sample_state());
        fake.stale_once().store(true, Ordering::SeqCst);
        let Some((_service, name)) = start_fake(fake.clone()).await else {
            eprintln!("bus de session absent : test Mutter ignoré");
            return;
        };
        let Some(outcome) = run_backend(name, Duration::from_secs(1), |backend| {
            apply_internal_refresh_rate(backend, 120.0, 0.5)
        })
        .await
        else {
            panic!("backend Mutter");
        };
        assert_eq!(
            outcome.expect("application après retry"),
            ApplyOutcome::Applied
        );
        assert_eq!(fake.attempts(), vec![7, 8]);
        assert_eq!(fake.applied().len(), 1);
        assert_eq!(fake.applied()[0].0, 8);
    }

    #[tokio::test]
    async fn denied_apply_fails() {
        let fake = FakeMutter::new(sample_state());
        fake.deny().store(true, Ordering::SeqCst);
        let Some((_service, name)) = start_fake(fake.clone()).await else {
            eprintln!("bus de session absent : test Mutter ignoré");
            return;
        };
        let Some(outcome) = run_backend(name, Duration::from_secs(1), |backend| {
            apply_internal_refresh_rate(backend, 120.0, 0.5)
        })
        .await
        else {
            panic!("backend Mutter");
        };
        let error = outcome.expect_err("refus Mutter");
        assert!(error.to_string().contains("ApplyMonitorsConfig"));
        assert!(fake.applied().is_empty());
    }

    #[tokio::test]
    async fn missing_mode_is_reported_without_apply() {
        let fake = FakeMutter::new(sample_state());
        let Some((_service, name)) = start_fake(fake.clone()).await else {
            eprintln!("bus de session absent : test Mutter ignoré");
            return;
        };
        let Some(outcome) = run_backend(name, Duration::from_secs(1), |backend| {
            apply_internal_refresh_rate(backend, 144.0, 0.5)
        })
        .await
        else {
            panic!("backend Mutter");
        };
        assert!(matches!(outcome, Err(HwError::DisplayModeNotFound(_))));
        assert!(fake.applied().is_empty());
    }

    #[tokio::test]
    async fn timeout_is_bounded() {
        let fake = FakeMutter::with_delay(sample_state(), Duration::from_secs(5));
        // Le service est volontairement lent : le client doit abandonner.
        let Some((_service, name)) = start_fake(fake).await else {
            eprintln!("bus de session absent : test Mutter ignoré");
            return;
        };
        let started = Instant::now();
        let Some(outcome) = run_backend(name, Duration::from_millis(100), |backend| {
            apply_internal_refresh_rate(backend, 120.0, 0.5)
        })
        .await
        else {
            panic!("backend Mutter");
        };
        assert!(outcome.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "le timeout Mutter n'est pas borné"
        );
    }
}
