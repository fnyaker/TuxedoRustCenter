// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Service D-Bus **de session** exposant les capacités d'affichage et
//! d'écriture UVC de `tccd-user`.
//!
//! La GUI ne peut pas interroger directement le compositeur : ce service
//! publie les modes réellement disponibles sur l'écran interne, en lecture
//! seule. Il porte aussi l'unique voie d'écriture des contrôles UVC
//! (`SetUvcControl`), exécutée dans le processus non privilégié de l'agent.
//! Il est servi sur le bus de session, sous le nom
//! `com.tuxedocomputers.tccd.User` et l'interface `…User1`.
//!
//! # Contraintes
//!
//! - Les profils passent par `Admin1` ; l'agent applique le mode d'affichage.
//! - `SetUvcControl` n'est **pas** une opération privilégiée : service
//!   utilisateur, sans polkit, avec l'ACL `uaccess`/groupe `video`.
//! - Le device UVC n'est ouvert qu'à l'appel, jamais au démarrage.
//! - Le chemin doit être un nœud réellement énuméré par `tcc-hw` ; aucune
//!   écriture de fichier arbitraire et aucun chemin client.
//! - Un cache court (1 s) évite d'énumérer les modes du compositeur à chaque
//!   appel GUI.
//! - Si le bus de session ou le nom est indisponible, l'agent continue sans ce
//!   service : seule la GUI perd la liste des modes réels.
//! - Aucun chemin, aucune commande et aucun secret ne traverse l'interface.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tcc_hw::display::{DisplayBackendKind, DisplayMode, OutputInfo, select_internal_output};
use tcc_ipc::{
    DBUS_SESSION_AGENT_INTERFACE, DBUS_SESSION_AGENT_PATH, DBUS_SESSION_AGENT_SERVICE,
    SESSION_CONTRACT_VERSION,
};

use crate::backend;
use crate::session::SessionInfo;
use crate::uvc::{UvcRuntime, UvcWriteError};

/// Durée du cache des capacités d'affichage.
pub const DISPLAY_INFO_CACHE: Duration = Duration::from_secs(1);

/// Erreur de démarrage du service de session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionServiceError {
    /// Bus de session inaccessible (pas de session graphique).
    #[error("bus de session indisponible: {0}")]
    SessionBus(String),
    /// Nom D-Bus déjà possédé par un autre agent.
    #[error("nom D-Bus déjà pris: {0}")]
    NameUnavailable(String),
    /// Enregistrement de l'objet/interface impossible.
    #[error("enregistrement du service de session: {0}")]
    Serve(String),
}

/// Rapport d'affichage exposé à la GUI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// Un backend de session a répondu.
    pub available: bool,
    pub session: SessionReport,
    pub backend: BackendReport,
    pub internal_output: InternalOutputReport,
    /// Un écran interne est présent et un mode peut être appliqué.
    pub supported: bool,
    /// Toujours faux dans l'agent (les commandes CLI ont leur propre dry-run).
    pub dry_run: bool,
    /// Erreur générique de backend, sans secret.
    pub error: Option<String>,
}

/// Contexte de session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReport {
    #[serde(rename = "type")]
    pub session_type: String,
    pub desktop: Option<String>,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
}

/// Backend d'affichage effectif.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    pub kind: &'static str,
    pub name: String,
    pub available: bool,
}

/// Écran interne et ses modes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalOutputReport {
    pub name: String,
    pub connected: bool,
    pub enabled: bool,
    pub has_internal_display: bool,
    pub current_mode: Option<ModeReport>,
    pub modes: Vec<ModeReport>,
}

/// Un mode d'affichage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeReport {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    pub preferred: bool,
    pub current: bool,
}

impl From<&DisplayMode> for ModeReport {
    fn from(mode: &DisplayMode) -> Self {
        Self {
            width: mode.width,
            height: mode.height,
            refresh_hz: mode.refresh_hz,
            preferred: mode.preferred,
            current: mode.current,
        }
    }
}

impl DisplayInfo {
    /// Construit un rapport vide pour un backend indisponible.
    #[must_use]
    pub fn unavailable(session: &SessionInfo, error: impl Into<String>) -> Self {
        Self::build(
            session,
            session.backend,
            session.backend.as_str(),
            false,
            None,
            Some(error.into()),
            false,
        )
    }

    /// Construit le rapport depuis les sorties du backend.
    ///
    /// Seul l'écran interne est retenu ; les modes d'un écran externe ne
    /// peuvent pas apparaître dans `internalOutput.modes`.
    #[must_use]
    pub fn build(
        session: &SessionInfo,
        backend_kind: DisplayBackendKind,
        backend_name: &str,
        backend_available: bool,
        outputs: Option<Vec<OutputInfo>>,
        error: Option<String>,
        dry_run: bool,
    ) -> Self {
        let internal = outputs.as_deref().and_then(select_internal_output);
        let available = backend_available && outputs.is_some() && error.is_none();
        let supported = available && internal.is_some();
        let internal_output = match internal {
            Some(output) => {
                let info = output.info();
                InternalOutputReport {
                    name: info.name.clone(),
                    connected: info.connected,
                    enabled: info.enabled,
                    has_internal_display: true,
                    current_mode: info.current_mode.as_ref().map(ModeReport::from),
                    modes: info.modes.iter().map(ModeReport::from).collect(),
                }
            }
            None => InternalOutputReport {
                name: String::new(),
                connected: false,
                enabled: false,
                has_internal_display: false,
                current_mode: None,
                modes: Vec::new(),
            },
        };
        Self {
            available,
            session: SessionReport {
                session_type: session.session_type.as_str().to_owned(),
                desktop: session.env.xdg_current_desktop.clone(),
                wayland_display: session.env.wayland_display.clone(),
                display: session.env.display.clone(),
            },
            backend: BackendReport {
                kind: backend_kind.as_str(),
                name: backend_name.to_owned(),
                available: backend_available,
            },
            internal_output,
            supported,
            dry_run,
            error,
        }
    }
}

/// Source injectable des capacités d'affichage (tests).
pub trait DisplayInfoSource: Send + Sync {
    /// Retourne le rapport courant ; peut utiliser un cache interne.
    fn display_info(&self) -> DisplayInfo;
}

/// Source réelle : backend de session + cache court.
#[derive(Debug)]
pub struct RealDisplayInfoSource {
    session: SessionInfo,
    uid: u32,
    cache: Mutex<Option<(Instant, DisplayInfo)>>,
}

impl RealDisplayInfoSource {
    /// Construit la source pour une session et un UID non privilégié.
    #[must_use]
    pub fn new(session: SessionInfo, uid: u32) -> Self {
        Self {
            session,
            uid,
            cache: Mutex::new(None),
        }
    }

    fn load(&self) -> DisplayInfo {
        match backend::create(&self.session, self.uid) {
            Ok(mut backend) => {
                let kind = backend.kind();
                let name = backend.backend_name().to_owned();
                let available = backend.is_available();
                match backend.list_outputs() {
                    Ok(outputs) => DisplayInfo::build(
                        &self.session,
                        kind,
                        &name,
                        available,
                        Some(outputs),
                        None,
                        false,
                    ),
                    Err(error) => DisplayInfo::build(
                        &self.session,
                        kind,
                        &name,
                        available,
                        None,
                        Some(error.to_string()),
                        false,
                    ),
                }
            }
            Err(error) => DisplayInfo::unavailable(&self.session, error.to_string()),
        }
    }
}

impl DisplayInfoSource for RealDisplayInfoSource {
    fn display_info(&self) -> DisplayInfo {
        let now = Instant::now();
        if let Ok(cache) = self.cache.lock()
            && let Some((created, info)) = cache.as_ref()
            && now.duration_since(*created) < DISPLAY_INFO_CACHE
        {
            return info.clone();
        }
        let info = self.load();
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some((now, info.clone()));
        }
        info
    }
}

/// Interface D-Bus `User1` : affichage en lecture, écriture UVC contrôlée.
pub struct SessionService {
    source: Arc<dyn DisplayInfoSource>,
    uvc: Option<Arc<UvcRuntime>>,
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionService")
            .field("contract", &SESSION_CONTRACT_VERSION)
            .field("uvc", &self.uvc.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionService {
    /// Construit le service sur une source d'affichage, sans écriture UVC.
    ///
    /// Utilisé par les tests et par les déploiements qui ne compilent pas la
    /// couche d'écriture : `GetUvcWriteSupported` retourne alors `false` et
    /// `SetUvcControl` échoue en `NotSupported`.
    #[must_use]
    pub fn new(source: Arc<dyn DisplayInfoSource>) -> Self {
        Self { source, uvc: None }
    }

    /// Construit le service avec écriture UVC.
    #[must_use]
    pub fn with_uvc(source: Arc<dyn DisplayInfoSource>, uvc: Arc<UvcRuntime>) -> Self {
        Self {
            source,
            uvc: Some(uvc),
        }
    }

    /// Service de production : affichage réel et écriture UVC réelle.
    ///
    /// Aucun device n'est ouvert ici : `UvcRuntime` ne fait qu'instancier les
    /// fabriques ; l'énumération et l'ouverture ont lieu à l'appel.
    #[must_use]
    pub fn production(source: Arc<dyn DisplayInfoSource>) -> Self {
        Self::with_uvc(source, Arc::new(UvcRuntime::production()))
    }
}

#[zbus::interface(name = "com.tuxedocomputers.tccd.User1")]
impl SessionService {
    /// Diagnostic de vivacité.
    #[zbus(name = "Ping")]
    async fn ping(&self) -> bool {
        true
    }

    /// Version de l'implémentation.
    #[zbus(name = "GetVersion")]
    async fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Version du contrat de session.
    #[zbus(name = "GetContractVersion")]
    async fn get_contract_version(&self) -> u32 {
        SESSION_CONTRACT_VERSION
    }

    /// Rapport JSON des capacités d'affichage (lecture seule).
    #[zbus(name = "GetDisplayInfoJSON")]
    async fn get_display_info_json(&self) -> zbus::fdo::Result<String> {
        let source = Arc::clone(&self.source);
        let info = tokio::task::spawn_blocking(move || source.display_info())
            .await
            .map_err(|_| zbus::fdo::Error::Failed("tâche d'affichage interrompue".to_owned()))?;
        serde_json::to_string(&info).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    /// Vrai si au moins un device UVC est énuméré et si le backend d'écriture
    /// est instancié.
    ///
    /// Aucun périphérique n'est ouvert : seule l'énumération sysfs est lue.
    /// Retourne `false` si le service a été construit sans couche d'écriture,
    /// afin que la GUI n'affiche jamais d'écriture non supportée.
    #[zbus(name = "GetUvcWriteSupported")]
    async fn get_uvc_write_supported(&self) -> bool {
        let Some(uvc) = self.uvc.as_ref().map(Arc::clone) else {
            return false;
        };
        match tokio::task::spawn_blocking(move || uvc.write_supported()).await {
            Ok(supported) => supported,
            Err(error) => {
                tracing::warn!(%error, "tccd-user : énumération UVC interrompue");
                false
            }
        }
    }

    /// Écrit un contrôle UVC sur un device énuméré.
    ///
    /// Retourne `true` si la valeur a été écrite, `false` si elle était déjà
    /// courante (idempotence). Le débit est limité par UID appelant.
    #[zbus(name = "SetUvcControl")]
    async fn set_uvc_control(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        device_path: String,
        control_key: String,
        value: i32,
    ) -> Result<bool, UvcWriteError> {
        let Some(uvc) = self.uvc.as_ref().map(Arc::clone) else {
            tracing::warn!(
                result = "not_supported",
                "écriture UVC refusée : backend absent"
            );
            return Err(UvcWriteError::NotSupported(
                "écriture UVC non disponible dans ce processus".to_owned(),
            ));
        };

        // 1. Syntaxe d'abord : aucune I/O, aucun jeton consommé pour un
        //    message malformé, aucune donnée brute journalisée.
        if let Err(error) = UvcRuntime::validate_request(&device_path, &control_key) {
            tracing::warn!(
                result = error.label(),
                "écriture UVC refusée : demande invalide"
            );
            return Err(error);
        }

        // 2. Débit par UID appelant. Si l'identité ne peut pas être résolue,
        //    le nom unique D-Bus évite un bucket global partagé.
        let caller = header
            .sender()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".to_owned());
        let caller_uid = resolve_caller_uid(connection, &header).await;
        let limiter_key = match caller_uid {
            Some(uid) => format!("uid:{uid}"),
            None => format!("name:{caller}"),
        };
        if !uvc.allow(&limiter_key) {
            tracing::warn!(
                method = "SetUvcControl",
                caller = %caller,
                caller_uid = ?caller_uid,
                device = %device_path,
                key = %control_key,
                value,
                result = "rate_limited",
                "écriture UVC refusée : limite de débit"
            );
            return Err(UvcWriteError::RateLimited);
        }

        // 3. Énumération + écriture dans une unique tâche bloquante : aucune
        //    ouverture de device sur le runtime async.
        let path = device_path.clone();
        let key = control_key.clone();
        let runtime = Arc::clone(&uvc);
        let result =
            tokio::task::spawn_blocking(move || runtime.write(&path, &key, i64::from(value))).await;

        match result {
            Ok(Ok(changed)) => {
                tracing::info!(
                    method = "SetUvcControl",
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    device = %device_path,
                    key = %control_key,
                    value,
                    changed,
                    result = "ok",
                    "écriture UVC appliquée"
                );
                Ok(changed)
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    method = "SetUvcControl",
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    device = %device_path,
                    key = %control_key,
                    value,
                    result = error.label(),
                    "écriture UVC refusée"
                );
                Err(error)
            }
            Err(error) => {
                tracing::error!(
                    method = "SetUvcControl",
                    caller = %caller,
                    caller_uid = ?caller_uid,
                    device = %device_path,
                    key = %control_key,
                    value,
                    %error,
                    result = "failed",
                    "tâche d'écriture UVC interrompue"
                );
                Err(UvcWriteError::Failed(
                    "tâche d'écriture UVC interrompue".to_owned(),
                ))
            }
        }
    }
}

/// Délai maximal de résolution de l'UID appelant.
const UID_TIMEOUT: Duration = Duration::from_millis(500);

/// Taille maximale du corps D-Bus accepté par le service `User1`.
///
/// Les arguments légitimes tiennent dans quelques dizaines d'octets (64 octets
/// de chemin, 64 octets de clé, un `i32`). Ce garde s'applique **avant** la
/// désérialisation des arguments par l'interface : un `ss` surdimensionné est
/// refusé avec `InvalidArgs("message.size")` sans allouer de `String`
/// secondaire. Voir UVCW-06 dans `docs/security/REVIEW_V4L2.md`.
pub const MAX_SESSION_MESSAGE_BODY_BYTES: usize = 64 * 1024;

/// Enveloppe l'interface `User1` avec un garde de taille de corps.
#[derive(Debug)]
pub struct BoundedSessionService(SessionService);

impl BoundedSessionService {
    /// Construit l'enveloppe autour du service.
    #[must_use]
    pub fn new(service: SessionService) -> Self {
        Self(service)
    }
}

fn oversized_session<'call>(
    connection: &'call zbus::Connection,
    msg: &'call zbus::Message,
) -> Option<zbus::object_server::DispatchResult<'call>> {
    if msg.body().len() > MAX_SESSION_MESSAGE_BODY_BYTES {
        return Some(zbus::object_server::DispatchResult::new_async(
            connection,
            msg,
            async {
                Err::<(), zbus::fdo::Error>(zbus::fdo::Error::InvalidArgs(
                    "message.size".to_owned(),
                ))
            },
        ));
    }
    None
}

#[async_trait::async_trait]
impl zbus::object_server::Interface for BoundedSessionService {
    fn name() -> zbus::names::InterfaceName<'static> {
        <SessionService as zbus::object_server::Interface>::name()
    }

    fn spawn_tasks_for_methods(&self) -> bool {
        <SessionService as zbus::object_server::Interface>::spawn_tasks_for_methods(&self.0)
    }

    async fn get(
        &self,
        property_name: &str,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> Option<zbus::fdo::Result<zbus::zvariant::OwnedValue>> {
        <SessionService as zbus::object_server::Interface>::get(
            &self.0,
            property_name,
            server,
            connection,
            header,
            emitter,
        )
        .await
    }

    async fn get_all(
        &self,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> {
        <SessionService as zbus::object_server::Interface>::get_all(
            &self.0, server, connection, header, emitter,
        )
        .await
    }

    fn set<'call>(
        &'call self,
        property_name: &'call str,
        value: &'call zbus::zvariant::Value<'_>,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        header: Option<&'call zbus::message::Header<'_>>,
        emitter: &'call zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::object_server::DispatchResult<'call> {
        <SessionService as zbus::object_server::Interface>::set(
            &self.0,
            property_name,
            value,
            server,
            connection,
            header,
            emitter,
        )
    }

    async fn set_mut(
        &mut self,
        property_name: &str,
        value: &zbus::zvariant::Value<'_>,
        server: &zbus::object_server::ObjectServer,
        connection: &zbus::Connection,
        header: Option<&zbus::message::Header<'_>>,
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> Option<zbus::fdo::Result<()>> {
        <SessionService as zbus::object_server::Interface>::set_mut(
            &mut self.0,
            property_name,
            value,
            server,
            connection,
            header,
            emitter,
        )
        .await
    }

    fn call<'call>(
        &'call self,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        msg: &'call zbus::Message,
        name: zbus::names::MemberName<'call>,
    ) -> zbus::object_server::DispatchResult<'call> {
        if let Some(error) = oversized_session(connection, msg) {
            return error;
        }
        <SessionService as zbus::object_server::Interface>::call(
            &self.0, server, connection, msg, name,
        )
    }

    fn call_mut<'call>(
        &'call mut self,
        server: &'call zbus::object_server::ObjectServer,
        connection: &'call zbus::Connection,
        msg: &'call zbus::Message,
        name: zbus::names::MemberName<'call>,
    ) -> zbus::object_server::DispatchResult<'call> {
        if let Some(error) = oversized_session(connection, msg) {
            return error;
        }
        <SessionService as zbus::object_server::Interface>::call_mut(
            &mut self.0,
            server,
            connection,
            msg,
            name,
        )
    }

    fn introspect_to_writer(&self, writer: &mut dyn std::fmt::Write, level: usize) {
        <SessionService as zbus::object_server::Interface>::introspect_to_writer(
            &self.0, writer, level,
        );
    }
}

/// Résout l'UID de l'appelant via `GetConnectionUnixUser`.
///
/// L'échec est non fatal : le limiteur retombe sur le nom unique D-Bus.
async fn resolve_caller_uid(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<u32> {
    let sender = header.sender()?;
    let bus_name = zbus::names::BusName::try_from(sender.as_str()).ok()?;
    let proxy = tokio::time::timeout(UID_TIMEOUT, zbus::fdo::DBusProxy::new(connection))
        .await
        .ok()?
        .ok()?;
    tokio::time::timeout(UID_TIMEOUT, proxy.get_connection_unix_user(bus_name))
        .await
        .ok()?
        .ok()
}

/// Enregistre le service sur le bus de session et attend l'arrêt.
///
/// `shutdown` permet aux tests de fermer proprement le service ; en production
/// la tâche est annulée avec le runtime de l'agent.
///
/// # Errors
///
/// [`SessionServiceError`] si le bus de session, le nom ou l'objet est
/// indisponible. L'appelant agent journalise et continue.
pub async fn serve_with(
    source: Arc<dyn DisplayInfoSource>,
    name: String,
    shutdown: Option<Arc<tokio::sync::Notify>>,
) -> Result<(), SessionServiceError> {
    serve_service(SessionService::new(source), name, shutdown).await
}

/// Enregistre le service de session avec écriture UVC et attend l'arrêt.
///
/// Variante utilisée par l'agent de production : `GetUvcWriteSupported`
/// reflète l'énumération réelle et `SetUvcControl` écrit via `tcc-hw`.
///
/// # Errors
///
/// [`SessionServiceError`] si le bus de session, le nom ou l'objet est
/// indisponible.
pub async fn serve_with_uvc(
    source: Arc<dyn DisplayInfoSource>,
    uvc: Arc<UvcRuntime>,
    name: String,
    shutdown: Option<Arc<tokio::sync::Notify>>,
) -> Result<(), SessionServiceError> {
    serve_service(SessionService::with_uvc(source, uvc), name, shutdown).await
}

async fn serve_service(
    service: SessionService,
    name: String,
    shutdown: Option<Arc<tokio::sync::Notify>>,
) -> Result<(), SessionServiceError> {
    let uvc_enabled = service.uvc.is_some();
    // Vérification explicite avant `build` : selon la version de zbus, une
    // demande de nom déjà possédé peut rester bloquée. Un nom pris n'empêche
    // jamais l'agent de continuer.
    let probe = zbus::Connection::session()
        .await
        .map_err(|error| SessionServiceError::SessionBus(error.to_string()))?;
    let dbus = zbus::fdo::DBusProxy::new(&probe)
        .await
        .map_err(|error| SessionServiceError::Serve(error.to_string()))?;
    let bus_name = zbus::names::BusName::try_from(name.clone())
        .map_err(|error| SessionServiceError::Serve(error.to_string()))?;
    let owned = dbus
        .name_has_owner(bus_name)
        .await
        .map_err(|error| SessionServiceError::Serve(error.to_string()))?;
    drop(probe);
    if owned {
        return Err(SessionServiceError::NameUnavailable(name));
    }

    let builder = zbus::connection::Builder::session()
        .map_err(|error| SessionServiceError::SessionBus(error.to_string()))?;
    let builder = builder
        .name(name.clone())
        .map_err(|error| map_name_error(error, &name))?;
    let builder = builder
        .serve_at(DBUS_SESSION_AGENT_PATH, BoundedSessionService::new(service))
        .map_err(|error| SessionServiceError::Serve(error.to_string()))?;
    let connection = tokio::time::timeout(Duration::from_secs(2), builder.build())
        .await
        .map_err(|_| SessionServiceError::NameUnavailable(name.clone()))?
        .map_err(|error| {
            if matches!(error, zbus::Error::NameTaken) {
                SessionServiceError::NameUnavailable(name.clone())
            } else {
                SessionServiceError::Serve(error.to_string())
            }
        })?;
    tracing::info!(
        service = %name,
        path = DBUS_SESSION_AGENT_PATH,
        interface = DBUS_SESSION_AGENT_INTERFACE,
        uvc_write = uvc_enabled,
        "service de session enregistré"
    );

    match shutdown {
        Some(notify) => notify.notified().await,
        None => std::future::pending().await,
    }
    drop(connection);
    tracing::info!("service de session arrêté");
    Ok(())
}

fn map_name_error(error: zbus::Error, name: &str) -> SessionServiceError {
    if matches!(error, zbus::Error::NameTaken) {
        SessionServiceError::NameUnavailable(name.to_owned())
    } else {
        SessionServiceError::Serve(error.to_string())
    }
}

/// Démarre le service en tâche de fond sur le runtime de l'agent.
///
/// Un échec (bus absent, nom déjà pris) est journalisé et n'arrête **jamais**
/// l'agent : la GUI perd seulement les modes réels.
pub fn spawn(session: SessionInfo, uid: u32) {
    let source = Arc::new(RealDisplayInfoSource::new(session, uid));
    let service = SessionService::production(source);
    tokio::spawn(async move {
        if let Err(error) =
            serve_service(service, DBUS_SESSION_AGENT_SERVICE.to_owned(), None).await
        {
            tracing::warn!(
                %error,
                "service de session indisponible ; l'agent continue sans interface de modes"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::session::SessionEnv;

    struct FakeSource {
        info: DisplayInfo,
        calls: AtomicUsize,
    }

    impl DisplayInfoSource for FakeSource {
        fn display_info(&self) -> DisplayInfo {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.info.clone()
        }
    }

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

    fn fake_source(info: DisplayInfo) -> Arc<FakeSource> {
        Arc::new(FakeSource {
            info,
            calls: AtomicUsize::new(0),
        })
    }

    fn info_with_outputs(outputs: Vec<OutputInfo>) -> DisplayInfo {
        DisplayInfo::build(
            &session(),
            DisplayBackendKind::X11,
            "x11",
            true,
            Some(outputs),
            None,
            false,
        )
    }

    fn output(
        name: &str,
        modes: &[(u32, u32, f32)],
        current: Option<(u32, u32, f32)>,
    ) -> OutputInfo {
        let mut output = OutputInfo::new(name);
        output.connected = true;
        output.enabled = current.is_some();
        output.modes = modes
            .iter()
            .map(|(width, height, refresh_hz)| DisplayMode {
                width: *width,
                height: *height,
                refresh_hz: *refresh_hz,
                preferred: false,
                current: current.is_some_and(|(w, h, r)| {
                    w == *width && h == *height && (r - *refresh_hz).abs() < f32::EPSILON
                }),
            })
            .collect();
        output.current_mode = current.map(|(width, height, refresh_hz)| DisplayMode {
            width,
            height,
            refresh_hz,
            preferred: false,
            current: true,
        });
        output
    }

    #[test]
    fn json_contains_only_internal_modes() {
        let outputs = vec![
            output("HDMI-A-1", &[(1920, 1080, 60.0)], Some((1920, 1080, 60.0))),
            output(
                "eDP-1",
                &[(2560, 1600, 60.0), (2560, 1600, 165.0)],
                Some((2560, 1600, 60.0)),
            ),
        ];
        let info = info_with_outputs(outputs);
        let json = serde_json::to_string(&info).expect("JSON");
        assert!(json.contains("\"internalOutput\""));
        assert!(json.contains("\"hasInternalDisplay\":true"));
        assert!(json.contains("\"name\":\"eDP-1\""));
        assert!(!json.contains("HDMI-A-1"));
        assert_eq!(info.internal_output.modes.len(), 2);
        assert!((info.internal_output.modes[1].refresh_hz - 165.0).abs() < f32::EPSILON);
        assert!(info.supported);
        assert!(info.available);
    }

    #[test]
    fn no_internal_display_is_reported() {
        let outputs = vec![output(
            "DP-1",
            &[(1920, 1080, 60.0)],
            Some((1920, 1080, 60.0)),
        )];
        let info = info_with_outputs(outputs);
        assert!(!info.internal_output.has_internal_display);
        assert!(info.internal_output.modes.is_empty());
        assert!(!info.supported);
        assert!(info.available);
    }

    #[test]
    fn unavailable_backend_is_reported() {
        let info = DisplayInfo::unavailable(&session(), "backend absent");
        assert!(!info.available);
        assert!(!info.supported);
        assert_eq!(info.error.as_deref(), Some("backend absent"));
        assert_eq!(info.internal_output.name, "");
        assert!(info.internal_output.modes.is_empty());
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

    #[tokio::test]
    async fn serves_contract_and_display_info_on_session_bus() {
        let Ok(client) = zbus::Connection::session().await else {
            eprintln!("bus de session absent : test service de session ignoré");
            return;
        };
        let name = unique_name("serve");
        let fake = fake_source(info_with_outputs(vec![
            output("HDMI-A-1", &[(1920, 1080, 60.0)], Some((1920, 1080, 60.0))),
            output(
                "eDP-1",
                &[(2560, 1600, 60.0), (2560, 1600, 165.0)],
                Some((2560, 1600, 60.0)),
            ),
        ]));
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(serve_with(
            Arc::clone(&fake) as Arc<dyn DisplayInfoSource>,
            name.clone(),
            Some(Arc::clone(&shutdown)),
        ));
        assert!(
            wait_for_owner(&client, &name, true).await,
            "le service doit posséder son nom"
        );

        let proxy = zbus::Proxy::new(
            &client,
            name.clone(),
            DBUS_SESSION_AGENT_PATH,
            DBUS_SESSION_AGENT_INTERFACE,
        )
        .await
        .expect("proxy User1");
        let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
        assert!(pong);
        let version: String = proxy.call("GetVersion", &()).await.expect("version");
        assert!(!version.is_empty());
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
        assert_eq!(value["internalOutput"]["name"], "eDP-1");
        assert_eq!(
            value["internalOutput"]["modes"].as_array().unwrap().len(),
            2
        );
        assert_eq!(value["supported"], true);
        assert!(!json.contains("HDMI-A-1"));
        assert_eq!(fake.calls.load(Ordering::Relaxed), 1);

        shutdown.notify_one();
        handle.await.expect("arrêt").expect("service");
        assert!(
            wait_for_owner(&client, &name, false).await,
            "le nom doit être libéré après l'arrêt"
        );
    }

    #[tokio::test]
    async fn unavailable_backend_is_reported_over_dbus() {
        let Ok(client) = zbus::Connection::session().await else {
            eprintln!("bus de session absent : test service de session ignoré");
            return;
        };
        let name = unique_name("unavailable");
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(serve_with(
            fake_source(DisplayInfo::unavailable(&session(), "backend absent"))
                as Arc<dyn DisplayInfoSource>,
            name.clone(),
            Some(Arc::clone(&shutdown)),
        ));
        assert!(wait_for_owner(&client, &name, true).await);
        let proxy = zbus::Proxy::new(
            &client,
            name,
            DBUS_SESSION_AGENT_PATH,
            DBUS_SESSION_AGENT_INTERFACE,
        )
        .await
        .expect("proxy User1");
        let json: String = proxy
            .call("GetDisplayInfoJSON", &())
            .await
            .expect("display info");
        let value: serde_json::Value = serde_json::from_str(&json).expect("JSON");
        assert_eq!(value["available"], false);
        assert_eq!(value["supported"], false);
        assert_eq!(value["error"], "backend absent");
        assert_eq!(value["internalOutput"]["hasInternalDisplay"], false);
        assert_eq!(
            value["internalOutput"]["modes"].as_array().unwrap().len(),
            0
        );
        shutdown.notify_one();
        handle.await.expect("arrêt").expect("service");
    }

    #[tokio::test]
    async fn name_taken_is_reported_and_first_service_stays_alive() {
        let Ok(client) = zbus::Connection::session().await else {
            eprintln!("bus de session absent : test service de session ignoré");
            return;
        };
        let name = unique_name("taken");
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let first = tokio::spawn(serve_with(
            fake_source(info_with_outputs(Vec::new())) as Arc<dyn DisplayInfoSource>,
            name.clone(),
            Some(Arc::clone(&shutdown)),
        ));
        assert!(wait_for_owner(&client, &name, true).await);

        let second = serve_with(
            fake_source(info_with_outputs(Vec::new())) as Arc<dyn DisplayInfoSource>,
            name.clone(),
            Some(Arc::new(tokio::sync::Notify::new())),
        )
        .await;
        assert!(matches!(
            second,
            Err(SessionServiceError::NameUnavailable(_))
        ));

        let proxy = zbus::Proxy::new(
            &client,
            name,
            DBUS_SESSION_AGENT_PATH,
            DBUS_SESSION_AGENT_INTERFACE,
        )
        .await
        .expect("proxy User1");
        let pong: bool = proxy.call("Ping", &()).await.expect("Ping");
        assert!(pong, "le premier service doit rester vivant");
        shutdown.notify_one();
        first.await.expect("arrêt").expect("service");
    }

    #[test]
    fn json_uses_expected_field_names() {
        let info = info_with_outputs(vec![output(
            "eDP-1",
            &[(2560, 1600, 60.0)],
            Some((2560, 1600, 60.0)),
        )]);
        let json = serde_json::to_string(&info).expect("JSON");
        for field in [
            "\"available\"",
            "\"session\"",
            "\"type\"",
            "\"desktop\"",
            "\"waylandDisplay\"",
            "\"backend\"",
            "\"kind\"",
            "\"name\"",
            "\"internalOutput\"",
            "\"connected\"",
            "\"enabled\"",
            "\"hasInternalDisplay\"",
            "\"currentMode\"",
            "\"refreshHz\"",
            "\"modes\"",
            "\"preferred\"",
            "\"supported\"",
            "\"dryRun\"",
            "\"error\"",
        ] {
            assert!(json.contains(field), "champ absent: {field}");
        }
        assert!(!json.contains("HDMI"));
    }
}
