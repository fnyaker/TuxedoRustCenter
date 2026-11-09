// SPDX-License-Identifier: GPL-3.0-or-later
//! Mode agent `tccd-user` : lecture du profil actif sur le bus D-Bus et
//! application de la résolution et/ou du taux de l'écran interne.
//!
//! Le daemon expose du JSON (`GetActiveProfileJSON`, `GetProfilesJSON`) traité
//! comme **non fiable** : bornes de taille, timeouts, parsing `serde`, puis
//! `ActiveProfile::validate` et `ProfileCollection::validate`.
//!
//! Aucune sortie d'affichage n'est acceptée depuis D-Bus : le backend choisit
//! lui-même l'écran interne (eDP/LVDS/DSI), applique la résolution demandée
//! (ou conserve la résolution courante) et une tolérance de 0,5 Hz.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::de::DeserializeOwned;
use tcc_core::{
    MAX_REFRESH_RATE_HZ, MAX_RESOLUTION, MIN_REFRESH_RATE_HZ, Profile, ProfileCollection, ProfileId,
};
use tcc_hw::display::{
    ApplyOutcome, DEFAULT_REFRESH_TOLERANCE_HZ, DisplayModeBackend, apply_internal_display_mode,
};
use tcc_hw::error::HwError;
use tcc_ipc::ActiveProfile;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::backend;
use crate::error::{Result, UserError};
use crate::session::SessionInfo;

/// Taille maximale d'une réponse JSON du daemon (protection mémoire avant
/// parsing).
pub const MAX_REMOTE_JSON_BYTES: usize = 4 * 1024 * 1024;
/// Délai maximal d'une connexion ou d'un appel D-Bus.
pub const DBUS_CALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Intervalle de lecture du profil actif quand le daemon répond.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Backoff initial en cas d'absence du daemon ou d'échec d'application.
pub const BASE_BACKOFF: Duration = Duration::from_secs(2);
/// Backoff maximal.
pub const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// Seuil de comparaison des taux (évite les micro-variations flottantes).
const TARGET_EPSILON_HZ: f32 = 0.001;

/// Cible d'affichage issue du profil actif.
///
/// `refresh_hz` et `resolution` sont indépendamment optionnels : le profil
/// peut ne demander qu'un taux, qu'une résolution, ou les deux. Une cible
/// `None`/`None` n'est jamais produite (`resolve_target` renvoie `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefreshTarget {
    pub profile_id: ProfileId,
    pub refresh_hz: Option<f32>,
    pub resolution: Option<(u32, u32)>,
}

/// Source de la cible, abstraite pour les tests (daemon réel ou simulé).
pub trait TargetSource: Send + Sync {
    /// Lit la cible courante. `Ok(None)` signifie « ne pas changer ».
    fn fetch(&self) -> Pin<Box<dyn Future<Output = Result<Option<RefreshTarget>>> + Send + '_>>;
}

/// Bus utilisé par le client de développement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BusKind {
    #[default]
    System,
    Session,
}

/// Source réelle : bus D-Bus + méthodes JSON du daemon.
///
/// La connexion et le proxy sont conservés entre les cycles de polling
/// (R-10) : une seule authentification D-Bus à la première lecture, puis
/// réutilisation tant que le daemon répond. En cas d'erreur, le proxy est
/// invalidé et reconstruit au cycle suivant.
#[derive(Debug)]
pub struct DbusSource {
    service: String,
    bus: BusKind,
    proxy: tokio::sync::Mutex<Option<zbus::Proxy<'static>>>,
}

impl Default for DbusSource {
    fn default() -> Self {
        Self {
            service: service_name(),
            bus: bus_kind(),
            proxy: tokio::sync::Mutex::new(None),
        }
    }
}

impl DbusSource {
    /// Source pointant explicitement vers un nom de service (tests, dev).
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            bus: BusKind::System,
            proxy: tokio::sync::Mutex::new(None),
        }
    }

    /// Injecte un proxy déjà construit (tests sans daemon).
    #[cfg(test)]
    fn with_proxy(proxy: zbus::Proxy<'static>) -> Self {
        Self {
            service: service_name(),
            bus: BusKind::System,
            proxy: tokio::sync::Mutex::new(Some(proxy)),
        }
    }

    async fn ensure_proxy(&self) -> Result<zbus::Proxy<'static>> {
        let mut guard = self.proxy.lock().await;
        if let Some(proxy) = guard.as_ref() {
            return Ok(proxy.clone());
        }

        let connect = async {
            match self.bus {
                BusKind::System => zbus::Connection::system().await,
                BusKind::Session => zbus::Connection::session().await,
            }
        };
        let connection = connect
            .await
            .map_err(|err| UserError::Dbus(format!("connexion au bus D-Bus : {err}")))?;
        let proxy = zbus::Proxy::new_owned(
            connection,
            self.service.clone(),
            tcc_ipc::DBUS_PATH,
            tcc_ipc::DBUS_INTERFACE,
        )
        .await
        .map_err(|err| UserError::Dbus(format!("proxy tccd : {err}")))?;
        *guard = Some(proxy.clone());
        Ok(proxy)
    }

    async fn invalidate_proxy(&self) {
        let mut guard = self.proxy.lock().await;
        *guard = None;
    }
}

impl TargetSource for DbusSource {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = Result<Option<RefreshTarget>>> + Send + '_>> {
        Box::pin(async move {
            // Un seul budget de 3 s pour tout le cycle (connexion + proxy +
            // deux appels) : une absence ou un daemon bloqué ne fige pas
            // l'agent plus longtemps que prévu.
            let result = tokio::time::timeout(DBUS_CALL_TIMEOUT, async {
                let proxy = self.ensure_proxy().await?;
                let (active_json, profiles_json) = fetch_payloads_with(&proxy).await?;
                resolve_target(&active_json, &profiles_json)
            })
            .await;

            match result {
                Ok(Ok(target)) => Ok(target),
                Ok(Err(error)) => {
                    if matches!(&error, UserError::Dbus(_) | UserError::Timeout(_)) {
                        self.invalidate_proxy().await;
                    }
                    Err(error)
                }
                Err(_) => {
                    self.invalidate_proxy().await;
                    Err(UserError::Timeout("cycle de lecture tccd"))
                }
            }
        })
    }
}

/// Nom de service D-Bus, avec override de développement `TCC_DBUS_SERVICE`.
///
/// Les clients sont non privilégiés : cette variable ne peut pas élever leurs
/// droits. Le daemon root, lui, n'accepte un nom alternatif que via
/// `--bus-name` (voir `tccd`).
fn service_name() -> String {
    std::env::var("TCC_DBUS_SERVICE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| tcc_ipc::DBUS_SERVICE.to_owned())
}

/// Bus de développement : `TCC_DBUS_BUS=session` pour le mode `--dev-session`.
fn bus_kind() -> BusKind {
    if std::env::var("TCC_DBUS_BUS").is_ok_and(|value| value.eq_ignore_ascii_case("session")) {
        BusKind::Session
    } else {
        BusKind::System
    }
}

/// Référence au profil actif, selon le format renvoyé par le daemon.
///
/// Le contrat documenté est `Option<tcc_ipc::ActiveProfile>`. Pour rester
/// compatible avec un daemon plus ancien qui renverrait le `Profile` complet,
/// le fallback n'extrait que l'identifiant ; les réglages sont ensuite relus
/// dans `ProfileCollection`, qui fait foi.
#[derive(Debug)]
enum ActiveRef {
    Typed(ActiveProfile),
    ProfileId(ProfileId),
}

/// Résout la cible depuis les deux payloads JSON.
///
/// `GetActiveProfileJSON` peut renvoyer `null` avant la première application
/// ou lorsqu'aucun profil n'est actif : `Option<...>` est donc utilisé.
pub fn resolve_target(active_json: &str, profiles_json: &str) -> Result<Option<RefreshTarget>> {
    check_payload_size(active_json, "active profile")?;
    check_payload_size(profiles_json, "profiles")?;

    // Les deux payloads sont parsés même si le profil actif est `null` :
    // aucune réponse du daemon n'est supposée bien formée.
    let active = parse_active_profile(active_json)?;
    let profiles: ProfileCollection = parse_payload(profiles_json, "profiles")?;

    let Some(active) = active else {
        return Ok(None);
    };
    let profile_id = match active {
        ActiveRef::Typed(active) => active.id,
        ActiveRef::ProfileId(id) => id,
    };

    let Some(profile) = profiles.get(&profile_id) else {
        return Err(UserError::InvalidRemoteData(format!(
            "profil actif '{}' absent de la collection",
            profile_id.as_str()
        )));
    };
    let rate = profile.display.effective_refresh_rate();
    let resolution = profile.display.effective_resolution();

    // `useRefRate=false`, `refreshRate=-1` (None), `useResolution=false` ou
    // aucune résolution exploitable : aucune modification. On ne valide pas
    // toute la collection dans ce cas puisqu'aucune donnée n'est utilisée pour
    // le matériel (compatibilité originale `refreshRate: -1`).
    if rate.is_none() && resolution.is_none() {
        return Ok(None);
    }

    // Validation complète avant d'utiliser la collection pour appliquer un
    // mode. Le profil actif typé a déjà été validé ci-dessus.
    profiles
        .validate()
        .map_err(|err| UserError::InvalidRemoteData(format!("profiles: {err}")))?;

    if let Some(rate) = rate
        && (!rate.is_finite() || !(MIN_REFRESH_RATE_HZ..=MAX_REFRESH_RATE_HZ).contains(&rate))
    {
        return Err(UserError::InvalidRemoteData(format!(
            "taux de rafraîchissement invalide : {rate} Hz"
        )));
    }
    if let Some((width, height)) = resolution
        && (width == 0 || height == 0 || width > MAX_RESOLUTION || height > MAX_RESOLUTION)
    {
        return Err(UserError::InvalidRemoteData(format!(
            "résolution invalide : {width}x{height}"
        )));
    }
    Ok(Some(RefreshTarget {
        profile_id,
        refresh_hz: rate,
        resolution,
    }))
}

fn parse_active_profile(payload: &str) -> Result<Option<ActiveRef>> {
    match serde_json::from_str::<Option<ActiveProfile>>(payload) {
        Ok(Some(active)) => {
            active
                .validate()
                .map_err(|err| UserError::InvalidRemoteData(format!("active profile: {err}")))?;
            Ok(Some(ActiveRef::Typed(active)))
        }
        Ok(None) => Ok(None),
        Err(typed_error) => {
            // Compatibilité : le daemon peut renvoyer un `Profile` complet.
            // Seul l'identifiant est utilisé ; la collection reste la source
            // de vérité pour les réglages, et elle est validée avant usage.
            let full: Option<Profile> = serde_json::from_str(payload).map_err(|full_error| {
                UserError::InvalidRemoteData(format!(
                    "active profile: JSON invalide (ActiveProfile: {typed_error}; \
                     Profile: {full_error})"
                ))
            })?;
            Ok(full.map(|profile| ActiveRef::ProfileId(profile.id)))
        }
    }
}

fn parse_payload<T: DeserializeOwned>(payload: &str, what: &'static str) -> Result<T> {
    serde_json::from_str(payload)
        .map_err(|err| UserError::InvalidRemoteData(format!("{what}: JSON invalide : {err}")))
}

fn check_payload_size(payload: &str, what: &'static str) -> Result<()> {
    if payload.len() > MAX_REMOTE_JSON_BYTES {
        return Err(UserError::InvalidRemoteData(format!(
            "{what}: réponse de {} octets supérieure à {MAX_REMOTE_JSON_BYTES}",
            payload.len()
        )));
    }
    Ok(())
}

async fn fetch_payloads_with(proxy: &zbus::Proxy<'_>) -> Result<(String, String)> {
    let active_json: String = proxy
        .call("GetActiveProfileJSON", &())
        .await
        .map_err(|err| UserError::Dbus(format!("GetActiveProfileJSON : {err}")))?;

    let profiles_json: String = proxy
        .call("GetProfilesJSON", &())
        .await
        .map_err(|err| UserError::Dbus(format!("GetProfilesJSON : {err}")))?;

    check_payload_size(&active_json, "active profile")?;
    check_payload_size(&profiles_json, "profiles")?;
    Ok((active_json, profiles_json))
}

#[derive(Default)]
struct AgentState {
    backend: Option<Box<dyn DisplayModeBackend>>,
    last: Option<RefreshTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cycle {
    Idle,
    Applied,
    Unsupported(String),
    ApplyFailed(String),
}

/// Boucle de service principale.
pub async fn run(uid: u32, session: SessionInfo) -> Result<()> {
    // Refus root avant toute publication D-Bus, même en lecture seule.
    if uid == 0 {
        return Err(UserError::RunningAsRoot);
    }
    // Service de session en lecture seule pour la GUI. Un échec (bus absent,
    // nom déjà pris) est journalisé par `session_service::spawn` et n'empêche
    // jamais l'agent de continuer.
    crate::session_service::spawn(session.clone(), uid);
    let source = DbusSource::default();
    run_with_source(uid, session, &source).await
}

async fn run_with_source(uid: u32, session: SessionInfo, source: &dyn TargetSource) -> Result<()> {
    // Défense en profondeur : le CLI vérifie déjà, mais l'agent ne doit jamais
    // tourner en root même s'il est appelé par un autre binaire.
    if uid == 0 {
        return Err(UserError::RunningAsRoot);
    }

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut state = AgentState::default();
    let mut daemon_backoff = BASE_BACKOFF;
    let mut apply_backoff = BASE_BACKOFF;

    tracing::info!(
        session = session.session_type.as_str(),
        backend = session.backend.as_str(),
        "tccd-user : agent démarré (lecture tccd sur le bus D-Bus)"
    );

    loop {
        let cycle = tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("tccd-user : SIGTERM reçu, arrêt");
                return Ok(());
            }
            _ = sigint.recv() => {
                tracing::info!("tccd-user : SIGINT reçu, arrêt");
                return Ok(());
            }
            cycle = poll_once(source, &mut state, &session, uid) => cycle,
        };

        let delay = match cycle {
            Ok(Cycle::Idle) | Ok(Cycle::Applied) => {
                daemon_backoff = BASE_BACKOFF;
                apply_backoff = BASE_BACKOFF;
                POLL_INTERVAL
            }
            Ok(Cycle::Unsupported(message)) => {
                daemon_backoff = BASE_BACKOFF;
                tracing::warn!(
                    %message,
                    "tccd-user : changement de taux non supporté, nouvelle tentative plus tard"
                );
                let delay = apply_backoff;
                apply_backoff = next_backoff(apply_backoff);
                delay
            }
            Ok(Cycle::ApplyFailed(message)) => {
                daemon_backoff = BASE_BACKOFF;
                tracing::warn!(
                    %message,
                    "tccd-user : application du taux échouée, nouvelle tentative plus tard"
                );
                let delay = apply_backoff;
                apply_backoff = next_backoff(apply_backoff);
                delay
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    "tccd-user : daemon absent ou réponse invalide, nouvelle tentative"
                );
                let delay = daemon_backoff;
                daemon_backoff = next_backoff(daemon_backoff);
                delay
            }
        };

        if wait_or_shutdown(delay, &mut sigterm, &mut sigint).await {
            return Ok(());
        }
    }
}

async fn poll_once(
    source: &dyn TargetSource,
    state: &mut AgentState,
    session: &SessionInfo,
    uid: u32,
) -> Result<Cycle> {
    let target = source.fetch().await?;
    let Some(target) = target else {
        // Profil sans taux applicable : ne rien modifier, et oublier l'ancienne
        // cible pour qu'un retour au même taux soit réappliqué.
        state.last = None;
        return Ok(Cycle::Idle);
    };

    if !target_changed(state.last.as_ref(), &target) {
        return Ok(Cycle::Idle);
    }

    if state.backend.is_none() {
        match create_backend(session.clone(), uid).await {
            Ok(backend) => state.backend = Some(backend),
            Err(error) => {
                tracing::warn!(%error, "tccd-user : backend d'affichage indisponible");
                return Ok(Cycle::ApplyFailed(error.to_string()));
            }
        }
    }

    let Some(backend) = state.backend.take() else {
        return Ok(Cycle::ApplyFailed("backend absent".to_owned()));
    };
    let (result, returned) = apply_target(backend, target.clone()).await;
    match result {
        Ok(ApplyOutcome::Applied) => {
            state.backend = returned;
            tracing::info!(
                profile = target.profile_id.as_str(),
                hz = ?target.refresh_hz,
                resolution = ?target.resolution,
                "tccd-user : mode d'affichage appliqué"
            );
            state.last = Some(target);
            Ok(Cycle::Applied)
        }
        Ok(ApplyOutcome::AlreadyApplied) => {
            state.backend = returned;
            tracing::debug!(
                profile = target.profile_id.as_str(),
                hz = ?target.refresh_hz,
                resolution = ?target.resolution,
                "tccd-user : mode déjà appliqué"
            );
            state.last = Some(target);
            Ok(Cycle::Idle)
        }
        Ok(ApplyOutcome::Unsupported) => {
            state.backend = returned;
            Ok(Cycle::Unsupported(format!(
                "backend non supporté pour le profil '{}' (hz={:?}, resolution={:?})",
                target.profile_id.as_str(),
                target.refresh_hz,
                target.resolution
            )))
        }
        Err(error) => {
            // Le backend peut être dans un état incohérent (compositeur
            // redémarré, connexion X11 perdue) : le recréer au prochain essai.
            state.backend = None;
            Ok(Cycle::ApplyFailed(error.to_string()))
        }
    }
}

fn option_rate_changed(previous: Option<f32>, next: Option<f32>) -> bool {
    match (previous, next) {
        (Some(previous), Some(next)) => (previous - next).abs() > TARGET_EPSILON_HZ,
        (None, None) => false,
        _ => true,
    }
}

fn target_changed(last: Option<&RefreshTarget>, next: &RefreshTarget) -> bool {
    match last {
        None => true,
        Some(previous) => {
            previous.profile_id != next.profile_id
                || option_rate_changed(previous.refresh_hz, next.refresh_hz)
                || previous.resolution != next.resolution
        }
    }
}

async fn create_backend(session: SessionInfo, uid: u32) -> Result<Box<dyn DisplayModeBackend>> {
    match tokio::task::spawn_blocking(move || backend::create(&session, uid)).await {
        Ok(result) => result,
        Err(error) => Err(UserError::AgentTask(format!(
            "création du backend interrompue : {error}"
        ))),
    }
}

async fn apply_target(
    mut backend: Box<dyn DisplayModeBackend>,
    target: RefreshTarget,
) -> (
    tcc_hw::Result<ApplyOutcome>,
    Option<Box<dyn DisplayModeBackend>>,
) {
    match tokio::task::spawn_blocking(move || {
        let (width, height) = target
            .resolution
            .map_or((None, None), |(width, height)| (Some(width), Some(height)));
        let outcome = apply_internal_display_mode(
            &mut *backend,
            width,
            height,
            target.refresh_hz,
            DEFAULT_REFRESH_TOLERANCE_HZ,
        );
        (outcome, backend)
    })
    .await
    {
        Ok((outcome, backend)) => (outcome, Some(backend)),
        Err(error) => (
            Err(HwError::Display(format!(
                "tâche d'application interrompue : {error}"
            ))),
            None,
        ),
    }
}

fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_BACKOFF)
}

async fn wait_or_shutdown(delay: Duration, sigterm: &mut Signal, sigint: &mut Signal) -> bool {
    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("tccd-user : SIGTERM reçu, arrêt");
            true
        }
        _ = sigint.recv() => {
            tracing::info!("tccd-user : SIGINT reçu, arrêt");
            true
        }
        _ = tokio::time::sleep(delay) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use tcc_core::{Profile, ResolutionSource};
    use tcc_ipc::ActiveProfile;

    struct FailingSource;

    impl TargetSource for FailingSource {
        fn fetch(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Option<RefreshTarget>>> + Send + '_>> {
            Box::pin(async { Err(UserError::Dbus("daemon absent".to_owned())) })
        }
    }

    struct FixedSource(Option<RefreshTarget>);

    impl TargetSource for FixedSource {
        fn fetch(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Option<RefreshTarget>>> + Send + '_>> {
            let target = self.0.clone();
            Box::pin(async move { Ok(target) })
        }
    }

    fn profile(id: &str, refresh: Option<f32>, use_refresh: bool) -> Profile {
        let mut profile = Profile::new(id, format!("Profil {id}"));
        profile.display.refresh_rate = refresh;
        profile.display.use_refresh_rate = use_refresh;
        profile
    }

    fn json_for(profile: &Profile) -> (String, String) {
        let active = ActiveProfile::from_profile(profile, ResolutionSource::StateMatrix);
        let active_json = serde_json::to_string(&active).unwrap();
        let profiles_json =
            serde_json::to_string(&ProfileCollection::new(vec![profile.clone()])).unwrap();
        (active_json, profiles_json)
    }

    #[test]
    fn resolves_active_profile_refresh_rate() {
        let profile = profile("office", Some(165.0), true);
        let (active_json, profiles_json) = json_for(&profile);
        let target = resolve_target(&active_json, &profiles_json)
            .unwrap()
            .unwrap();
        assert_eq!(target.profile_id.as_str(), "office");
        assert!((target.refresh_hz.expect("taux") - 165.0).abs() < 0.001);
        assert_eq!(target.resolution, None);
    }

    #[test]
    fn resolves_resolution_only() {
        let mut profile = profile("office", None, false);
        profile.display.use_resolution = true;
        profile.display.x_resolution = Some(1920);
        profile.display.y_resolution = Some(1080);
        let (active_json, profiles_json) = json_for(&profile);
        let target = resolve_target(&active_json, &profiles_json)
            .expect("résolution")
            .expect("cible");
        assert_eq!(target.refresh_hz, None);
        assert_eq!(target.resolution, Some((1920, 1080)));
    }

    #[test]
    fn resolves_resolution_and_rate() {
        let mut profile = profile("office", Some(165.0), true);
        profile.display.use_resolution = true;
        profile.display.x_resolution = Some(1920);
        profile.display.y_resolution = Some(1080);
        let (active_json, profiles_json) = json_for(&profile);
        let target = resolve_target(&active_json, &profiles_json)
            .expect("résolution")
            .expect("cible");
        assert_eq!(target.refresh_hz, Some(165.0));
        assert_eq!(target.resolution, Some((1920, 1080)));
    }

    #[test]
    fn resolution_change_invalidates_cached_target() {
        let base = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(60.0),
            resolution: Some((2560, 1440)),
        };
        let same = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(60.0),
            resolution: Some((2560, 1440)),
        };
        let other = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(60.0),
            resolution: Some((1920, 1080)),
        };
        assert!(!target_changed(Some(&base), &same));
        assert!(target_changed(Some(&base), &other));
    }

    #[test]
    fn resolves_full_profile_compatibility_shape() {
        // Ancien daemon : GetActiveProfileJSON renvoie le Profile complet
        // (sans `isPreset`). Seul l'identifiant doit être utilisé.
        let profile = profile("office", Some(165.0), true);
        let active_json = serde_json::to_string(&profile).unwrap();
        let profiles_json = serde_json::to_string(&ProfileCollection::new(vec![profile])).unwrap();
        let target = resolve_target(&active_json, &profiles_json)
            .unwrap()
            .unwrap();
        assert_eq!(target.profile_id.as_str(), "office");
        assert!((target.refresh_hz.expect("taux") - 165.0).abs() < 0.001);
    }

    #[test]
    fn use_ref_rate_false_means_no_change() {
        let profile = profile("office", Some(165.0), false);
        let (active_json, profiles_json) = json_for(&profile);
        assert_eq!(resolve_target(&active_json, &profiles_json).unwrap(), None);
    }

    #[test]
    fn refresh_rate_minus_one_means_no_change() {
        let profile = profile("office", None, true);
        let (active_json, profiles_json) = json_for(&profile);
        // La sérialisation de `None` utilise -1, compatible original.
        assert!(profiles_json.contains("-1"));
        assert_eq!(resolve_target(&active_json, &profiles_json).unwrap(), None);
    }

    #[test]
    fn null_active_profile_means_no_change() {
        assert_eq!(resolve_target("null", "[]").unwrap(), None);
    }

    #[test]
    fn missing_active_profile_is_rejected() {
        let profile = profile("office", Some(165.0), true);
        let active_json = serde_json::to_string(&ActiveProfile::from_profile(
            &profile,
            ResolutionSource::StateMatrix,
        ))
        .unwrap();
        let error = resolve_target(&active_json, "[]").unwrap_err();
        assert!(matches!(error, UserError::InvalidRemoteData(_)));
    }

    #[test]
    fn invalid_active_profile_is_rejected() {
        let profile = profile("office", Some(165.0), true);
        let mut active = ActiveProfile::from_profile(&profile, ResolutionSource::StateMatrix);
        active.name.clear();
        let active_json = serde_json::to_string(&active).unwrap();
        let profiles_json = serde_json::to_string(&ProfileCollection::new(vec![profile])).unwrap();
        assert!(matches!(
            resolve_target(&active_json, &profiles_json),
            Err(UserError::InvalidRemoteData(_))
        ));
    }

    #[test]
    fn malformed_json_is_rejected_without_panic() {
        assert!(matches!(
            resolve_target("{", "[]"),
            Err(UserError::InvalidRemoteData(_))
        ));
        assert!(matches!(
            resolve_target("null", "{"),
            Err(UserError::InvalidRemoteData(_))
        ));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let huge = " ".repeat(MAX_REMOTE_JSON_BYTES + 1);
        assert!(matches!(
            resolve_target(&huge, "[]"),
            Err(UserError::InvalidRemoteData(_))
        ));
    }

    #[test]
    fn invalid_refresh_rate_is_rejected() {
        let mut profile = profile("office", Some(165.0), true);
        // Court-circuite la validation de collection pour tester la borne
        // locale de l'agent.
        profile.display.refresh_rate = Some(0.0);
        let active = ActiveProfile::from_profile(&profile, ResolutionSource::StateMatrix);
        let active_json = serde_json::to_string(&active).unwrap();
        let profiles_json = serde_json::to_string(&ProfileCollection::new(vec![profile])).unwrap();
        assert!(resolve_target(&active_json, &profiles_json).is_err());
    }

    #[test]
    fn target_change_compares_profile_and_rate() {
        let base = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(60.0),
            resolution: None,
        };
        let same = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(60.0005),
            resolution: None,
        };
        let other_rate = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(165.0),
            resolution: None,
        };
        let other_profile = RefreshTarget {
            profile_id: ProfileId::new("quiet"),
            refresh_hz: Some(60.0),
            resolution: None,
        };

        assert!(target_changed(None, &base));
        assert!(!target_changed(Some(&base), &same));
        assert!(target_changed(Some(&base), &other_rate));
        assert!(target_changed(Some(&base), &other_profile));
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(next_backoff(BASE_BACKOFF), Duration::from_secs(4));
        assert_eq!(next_backoff(Duration::from_secs(8)), MAX_BACKOFF);
        assert_eq!(next_backoff(MAX_BACKOFF), MAX_BACKOFF);
    }

    #[tokio::test]
    async fn absent_daemon_cycle_is_bounded_by_timeout() {
        let Ok(connection) = zbus::Connection::session().await else {
            // Pas de bus de session dans cet environnement : test non applicable.
            return;
        };
        let service = format!("com.tuxedocomputers.tccd.AbsentTest{}", std::process::id());
        let Ok(proxy) = zbus::Proxy::new_owned(
            connection,
            service,
            tcc_ipc::DBUS_PATH,
            tcc_ipc::DBUS_INTERFACE,
        )
        .await
        else {
            return;
        };
        let source = DbusSource::with_proxy(proxy);
        let start = Instant::now();
        let result = source.fetch().await;
        assert!(
            result.is_err(),
            "un service absent doit produire une erreur"
        );
        assert!(
            start.elapsed() <= DBUS_CALL_TIMEOUT + Duration::from_secs(1),
            "cycle trop long sans daemon : {:?}",
            start.elapsed()
        );
    }

    struct FakeDisplayBackend {
        outputs: Vec<tcc_hw::display::OutputInfo>,
        applied: Arc<std::sync::Mutex<Option<tcc_hw::display::DisplayMode>>>,
    }

    impl DisplayModeBackend for FakeDisplayBackend {
        fn backend_name(&self) -> &'static str {
            "fake-agent"
        }

        fn kind(&self) -> tcc_hw::display::DisplayBackendKind {
            tcc_hw::display::DisplayBackendKind::X11
        }

        fn list_outputs(&mut self) -> tcc_hw::Result<Vec<tcc_hw::display::OutputInfo>> {
            Ok(self.outputs.clone())
        }

        fn apply_refresh_rate(
            &mut self,
            _output: &tcc_hw::display::InternalOutput,
            mode: &tcc_hw::display::DisplayMode,
        ) -> tcc_hw::Result<ApplyOutcome> {
            *self
                .applied
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(mode.clone());
            Ok(ApplyOutcome::Applied)
        }
    }

    #[tokio::test]
    async fn applies_resolution_and_rate_in_one_operation() {
        let mut output = tcc_hw::display::OutputInfo::new("eDP-1");
        output.connected = true;
        output.enabled = true;
        output.current_mode = Some(tcc_hw::display::DisplayMode::new(2560, 1440, 60.0));
        output.modes = vec![
            tcc_hw::display::DisplayMode::new(2560, 1440, 60.0),
            tcc_hw::display::DisplayMode::new(1920, 1080, 60.0),
            tcc_hw::display::DisplayMode::new(1920, 1080, 165.0),
        ];
        let applied = Arc::new(std::sync::Mutex::new(None));
        let backend = FakeDisplayBackend {
            outputs: vec![output],
            applied: Arc::clone(&applied),
        };
        let target = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(165.0),
            resolution: Some((1920, 1080)),
        };
        let (result, returned) = apply_target(Box::new(backend), target).await;
        assert_eq!(result.expect("application"), ApplyOutcome::Applied);
        assert!(returned.is_some());
        let mode = applied
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("mode appliqué");
        assert_eq!((mode.width, mode.height), (1920, 1080));
        assert_eq!(mode.refresh_hz, 165.0);
    }

    #[tokio::test]
    async fn absent_daemon_does_not_panic() {
        let session = SessionInfo::from_env(crate::SessionEnv::default());
        let mut state = AgentState::default();
        let result = poll_once(&FailingSource, &mut state, &session, 1000).await;
        assert!(matches!(result, Err(UserError::Dbus(_))));
        assert!(state.last.is_none());
    }

    #[tokio::test]
    async fn unchanged_target_does_not_create_backend() {
        let target = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(165.0),
            resolution: None,
        };
        let session = SessionInfo::from_env(crate::SessionEnv::default());
        let mut state = AgentState {
            backend: None,
            last: Some(target.clone()),
        };
        let cycle = poll_once(&FixedSource(Some(target)), &mut state, &session, 1000)
            .await
            .unwrap();
        assert_eq!(cycle, Cycle::Idle);
        assert!(state.backend.is_none());
    }

    #[tokio::test]
    async fn unsupported_backend_is_reported_not_panicking() {
        let target = RefreshTarget {
            profile_id: ProfileId::new("office"),
            refresh_hz: Some(165.0),
            resolution: None,
        };
        let session = SessionInfo::from_env(crate::SessionEnv {
            xdg_session_type: Some("tty".to_owned()),
            ..crate::SessionEnv::default()
        });
        assert_eq!(
            session.backend,
            tcc_hw::display::DisplayBackendKind::Unsupported,
            "session détectée : {:?}",
            session
        );
        let mut state = AgentState::default();
        let cycle = poll_once(&FixedSource(Some(target)), &mut state, &session, 1000)
            .await
            .unwrap();
        assert!(
            matches!(cycle, Cycle::Unsupported(_)),
            "cycle inattendu : {cycle:?}"
        );
        assert!(state.last.is_none());
    }

    #[tokio::test]
    async fn no_target_clears_previous_target() {
        let session = SessionInfo::from_env(crate::SessionEnv::default());
        let mut state = AgentState {
            backend: None,
            last: Some(RefreshTarget {
                profile_id: ProfileId::new("office"),
                refresh_hz: Some(165.0),
                resolution: None,
            }),
        };
        let cycle = poll_once(&FixedSource(None), &mut state, &session, 1000)
            .await
            .unwrap();
        assert_eq!(cycle, Cycle::Idle);
        assert!(state.last.is_none());
    }
}
