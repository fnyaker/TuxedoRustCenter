// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Service D-Bus `Admin1` de `tccd-admin` (phase 1, AD-1 à AD-6).
//!
//! Le service expose quatre opérations :
//!
//! - `Ping`, `GetVersion` : diagnostic sans polkit ni écriture ;
//! - `SaveProfilesJSON`, `SaveSettingsJSON` : écriture persistante sous
//!   polkit, avec validation, sérialisation des écritures et rollback ;
//! - `ReloadConfig` : rechargement de `tccd`, sans écriture.
//!
//! Toute méthode d'écriture suit le même ordre : résolution de l'identité,
//! borne de taille, limiteur de débit, polkit, verrou d'écriture, validation,
//! transaction, audit. Aucun paramètre de chemin ni de commande n'est accepté.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tcc_authz::{
    AuthorizationLimiter, AuthorizationRequest, DenialReason, DialogGate, DialogGateError,
};
use tcc_core::{
    ConfigStore, Profile, ProfileCollection, Settings, StrictLoadError,
    merge_with_default_profiles, validate_settings_references,
};
use tcc_ipc::{ADMIN_CONTRACT_VERSION, AdminReloadReport};

use crate::error::{AdminError, in_use_profile, validation_error};
use crate::transaction::{
    self, BackupStatus, MAX_PAYLOAD_BYTES, ReloadError, ReloadNotifier, SavedInfo,
};

/// Action polkit pour `SaveProfilesJSON`.
pub const ACTION_PROFILES: &str = "com.tuxedocomputers.tccd1.profiles";
/// Action polkit pour `SaveSettingsJSON` et `ReloadConfig`.
pub const ACTION_SETTINGS: &str = "com.tuxedocomputers.tccd1.settings";
/// Sortie après cette durée sans appel.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Attente maximale du dialogue polkit unique avant `Busy`.
pub const DIALOG_WAIT: Duration = Duration::from_secs(2);
/// Débit maximal des méthodes d'écriture (AD-4).
pub const WRITE_RATE_PER_SECOND: f64 = 2.0;
/// Burst initial des méthodes d'écriture (AD-4).
pub const WRITE_BURST: f64 = 4.0;
/// Timeout de l'appel `Control1.ReloadConfig`.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout de résolution de l'UID de l'appelant.
const UID_TIMEOUT: Duration = Duration::from_millis(500);

/// Horloge d'activité du service.
///
/// Chaque appel D-Bus (même refusé) repousse l'échéance d'inactivité. Aucun
/// autre état n'est conservé.
#[derive(Debug)]
pub struct Activity {
    last: std::sync::Mutex<Instant>,
    in_flight: std::sync::atomic::AtomicUsize,
}

impl Activity {
    /// Crée une horloge active à l'instant présent.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last: std::sync::Mutex::new(Instant::now()),
            in_flight: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Marque une activité.
    pub fn touch(&self) {
        let mut last = match self.last.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *last = Instant::now();
    }

    /// Marque un appel en cours.
    ///
    /// Tant que le garde vit, la supervision d'inactivité ne doit pas arrêter
    /// le processus : un dialogue polkit légitime peut dépasser 30 s. Le garde
    /// ne modifie pas `last` (A1-03).
    #[must_use]
    pub fn enter(&self) -> ActiveCall<'_> {
        self.in_flight
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ActiveCall { activity: self }
    }

    /// Vrai si le service est inactif depuis au moins `timeout`.
    #[must_use]
    pub fn is_idle(&self, timeout: Duration) -> bool {
        if self.in_flight.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            return false;
        }
        let last = match self.last.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        last.elapsed() >= timeout
    }

    /// Dernier instant d'activité (tests).
    #[cfg(test)]
    fn last_for_test(&self) -> Instant {
        match self.last.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

impl Default for Activity {
    fn default() -> Self {
        Self::new()
    }
}

/// Garde d'un appel en cours : empêche l'arrêt sur inactivité pendant le
/// traitement, sans repousser `last`.
#[derive(Debug)]
pub struct ActiveCall<'a> {
    activity: &'a Activity,
}

impl Drop for ActiveCall<'_> {
    fn drop(&mut self) {
        self.activity
            .in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Surveille l'inactivité et demande l'arrêt du processus.
///
/// Une seule tâche de supervision (1 s), sans thread dédié.
pub fn spawn_idle_watch(
    activity: Arc<Activity>,
    shutdown: Arc<tokio::sync::Notify>,
    timeout: Duration,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if activity.is_idle(timeout) {
                tracing::info!(
                    timeout_seconds = timeout.as_secs(),
                    "tccd-admin inactif : sortie"
                );
                shutdown.notify_waiters();
                break;
            }
        }
    });
}

/// Décision d'autorisation consommée par le service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzDecision {
    /// L'appelant est autorisé.
    Authorized,
    /// polkit a refusé ; le motif est destiné à l'audit.
    Denied(DenialReason),
    /// Un dialogue est déjà en cours.
    Busy,
    /// polkit est absent ou en erreur.
    Unavailable,
}

/// Autorisation injectable (polkit en production, faux en test).
#[async_trait]
pub trait Authorizer: Send + Sync {
    /// Vérifie l'autorisation de `caller` pour `action`.
    async fn authorize(&self, caller: &str, uid: u32, action: &str) -> AuthzDecision;
}

/// Autorisation polkit réelle avec un seul dialogue global.
#[derive(Debug, Default)]
pub struct PolkitAuthorizer {
    dialog: DialogGate,
}

impl PolkitAuthorizer {
    /// Construit l'autorisation de production.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Authorizer for PolkitAuthorizer {
    async fn authorize(&self, caller: &str, uid: u32, action: &str) -> AuthzDecision {
        // Un seul dialogue à la fois et un seul par appelant (AD-4). Le
        // permis est rendu au Drop, y compris si polkit échoue.
        let _permit = match self.dialog.acquire(caller, DIALOG_WAIT).await {
            Ok(permit) => permit,
            Err(DialogGateError::CallerBusy | DialogGateError::GlobalBusy) => {
                return AuthzDecision::Busy;
            }
        };
        let request = AuthorizationRequest::new(caller, action).with_caller_uid(uid);
        match tcc_authz::check_authorization(&request).await {
            Ok(result) if result.is_authorized() => AuthzDecision::Authorized,
            Ok(tcc_authz::AuthorizationResult::Denied(reason)) => AuthzDecision::Denied(reason),
            Ok(_) => AuthzDecision::Denied(DenialReason::PolicyDenied),
            Err(_) => AuthzDecision::Unavailable,
        }
    }
}

/// Client `Control1.ReloadConfig` réel.
#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.Control1",
    default_service = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Control1 {
    #[zbus(name = "ReloadConfig")]
    fn reload_config(&self) -> zbus::Result<String>;
}

/// Implémentation de production de [`ReloadNotifier`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ControlReloader;

#[async_trait]
impl ReloadNotifier for ControlReloader {
    async fn reload(&self) -> Result<AdminReloadReport, ReloadError> {
        let connection = zbus::Connection::system().await.map_err(|_| ReloadError)?;
        let proxy = Control1Proxy::new(&connection)
            .await
            .map_err(|_| ReloadError)?;
        let json = tokio::time::timeout(CONTROL_TIMEOUT, proxy.reload_config())
            .await
            .map_err(|_| ReloadError)?
            .map_err(|_| ReloadError)?;
        AdminReloadReport::from_json_bounded(&json, MAX_PAYLOAD_BYTES).map_err(|_| ReloadError)
    }
}

/// Contexte d'un appel, pour l'audit et le limiteur.
struct CallContext {
    method: &'static str,
    action: &'static str,
    caller: String,
    uid: u32,
    started: Instant,
}

/// Compteurs d'audit des tombstones d'une écriture de profils.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TombstoneCounts {
    /// Profils supprimés par cette écriture.
    deleted: usize,
    /// Tombstones nouvellement ajoutés.
    added: usize,
    /// Tombstones déjà présents (idempotence).
    preserved: usize,
}

/// Résultat d'une écriture de profils.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProfilesSave {
    /// Résultat commun de la transaction.
    saved: SavedInfo,
    /// Compteurs de tombstones.
    tombstones: TombstoneCounts,
}

/// Service `Admin1` phase 1.
pub struct AdminService {
    store: ConfigStore,
    reloader: Arc<dyn ReloadNotifier>,
    authorizer: Arc<dyn Authorizer>,
    limiter: AuthorizationLimiter,
    write_gate: Arc<tokio::sync::Semaphore>,
    activity: Arc<Activity>,
}

impl AdminService {
    /// Construit un service avec dépendances injectées (tests et production).
    #[must_use]
    pub fn new(
        store: ConfigStore,
        reloader: Arc<dyn ReloadNotifier>,
        authorizer: Arc<dyn Authorizer>,
        activity: Arc<Activity>,
    ) -> Self {
        Self {
            store,
            reloader,
            authorizer,
            limiter: AuthorizationLimiter::new(WRITE_RATE_PER_SECOND, WRITE_BURST),
            write_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            activity,
        }
    }

    /// Construit le service de production (`/var/lib` + polkit + `Control1`).
    #[must_use]
    pub fn production(activity: Arc<Activity>) -> Self {
        Self::new(
            ConfigStore::system(),
            Arc::new(ControlReloader),
            Arc::new(PolkitAuthorizer::new()),
            activity,
        )
    }

    /// Horloge d'activité partagée avec la supervision d'inactivité.
    #[must_use]
    pub fn activity(&self) -> Arc<Activity> {
        Arc::clone(&self.activity)
    }

    /// Résout le nom unique et l'UID de l'appelant.
    async fn caller_identity(
        &self,
        header: &zbus::message::Header<'_>,
        connection: &zbus::Connection,
    ) -> Result<(String, u32), AdminError> {
        let sender = header.sender().ok_or(AdminError::NotAuthorized)?;
        let caller = sender.to_string();
        let bus_name = zbus::names::BusName::try_from(sender.as_str())
            .map_err(|_| AdminError::NotAuthorized)?;
        let proxy = tokio::time::timeout(UID_TIMEOUT, zbus::fdo::DBusProxy::new(connection))
            .await
            .map_err(|_| AdminError::NotAuthorized)?
            .map_err(|_| AdminError::NotAuthorized)?;
        let uid = tokio::time::timeout(UID_TIMEOUT, proxy.get_connection_unix_user(bus_name))
            .await
            .map_err(|_| AdminError::NotAuthorized)?
            .map_err(|_| AdminError::NotAuthorized)?;
        Ok((caller, uid))
    }

    /// Applique le limiteur puis polkit, et audite le refus.
    async fn authorize_write(&self, context: &CallContext) -> Result<(), AdminError> {
        if !self
            .limiter
            .allow(&context.caller, context.uid, Instant::now())
        {
            tracing::warn!(
                method = context.method,
                action = context.action,
                caller = %context.caller,
                caller_uid = context.uid,
                result = "rate_limited",
                duration_ms = context.started.elapsed().as_millis() as u64,
                "appel admin refusé"
            );
            return Err(AdminError::RateLimited);
        }

        match self
            .authorizer
            .authorize(&context.caller, context.uid, context.action)
            .await
        {
            AuthzDecision::Authorized => {
                // A1-03 : seul un appel autorisé prolonge la vie du processus
                // root. Un refus (polkit, limiteur, identité) ne le fait pas.
                self.activity.touch();
                Ok(())
            }
            AuthzDecision::Busy => {
                tracing::warn!(
                    method = context.method,
                    action = context.action,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = "busy",
                    duration_ms = context.started.elapsed().as_millis() as u64,
                    "dialogue d'autorisation déjà en cours"
                );
                Err(AdminError::Busy)
            }
            AuthzDecision::Denied(reason) => {
                tracing::warn!(
                    method = context.method,
                    action = context.action,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = ?reason,
                    duration_ms = context.started.elapsed().as_millis() as u64,
                    "autorisation refusée"
                );
                Err(AdminError::NotAuthorized)
            }
            AuthzDecision::Unavailable => {
                tracing::error!(
                    method = context.method,
                    action = context.action,
                    caller = %context.caller,
                    caller_uid = context.uid,
                    result = "polkit_unavailable",
                    duration_ms = context.started.elapsed().as_millis() as u64,
                    "polkit indisponible : refus fail-closed"
                );
                Err(AdminError::NotAuthorized)
            }
        }
    }

    fn audit_success(&self, context: &CallContext, saved: SavedInfo, reload: &'static str) {
        tracing::info!(
            method = context.method,
            action = context.action,
            caller = %context.caller,
            caller_uid = context.uid,
            result = "ok",
            duration_ms = context.started.elapsed().as_millis() as u64,
            bytes = saved.bytes,
            backup = saved.backup.as_str(),
            reload,
            "appel admin accepté"
        );
    }

    /// Audite les compteurs de tombstones, sans identifiant ni payload.
    fn audit_tombstones(&self, context: &CallContext, counts: TombstoneCounts) {
        if counts.deleted == 0 && counts.added == 0 && counts.preserved == 0 {
            return;
        }
        tracing::info!(
            method = context.method,
            action = context.action,
            caller = %context.caller,
            caller_uid = context.uid,
            deletedProfiles = counts.deleted,
            tombstonesAdded = counts.added,
            tombstonesPreserved = counts.preserved,
            "tombstones de profils enregistrés"
        );
    }

    fn audit_failure(&self, context: &CallContext, bytes: Option<usize>, error: &AdminError) {
        tracing::warn!(
            method = context.method,
            action = context.action,
            caller = %context.caller,
            caller_uid = context.uid,
            result = error.label(),
            duration_ms = context.started.elapsed().as_millis() as u64,
            bytes = bytes.unwrap_or(0),
            "appel admin refusé"
        );
    }

    /// Borne la taille **avant** tout parsing (AD-2).
    fn ensure_payload_size(size: usize) -> Result<(), AdminError> {
        if size > MAX_PAYLOAD_BYTES {
            Err(AdminError::PayloadTooLarge)
        } else {
            Ok(())
        }
    }

    async fn save_profiles_inner(
        &self,
        payload: &str,
        bytes: usize,
    ) -> Result<ProfilesSave, AdminError> {
        Self::ensure_payload_size(payload.len())?;
        let parsed: ProfileCollection = serde_json::from_str(payload)
            .map_err(|_| AdminError::InvalidArgs("profiles".to_owned()))?;

        // Les presets ne sont jamais persistés : une mise à jour logicielle
        // doit pouvoir les remplacer. Un payload composé uniquement de
        // presets est accepté sans effet.
        let custom: Vec<Profile> = parsed
            .iter()
            .filter(|profile| !profile.is_preset())
            .cloned()
            .collect();
        // Un payload non vide composé uniquement de presets est accepté sans
        // effet : il ne doit pas effacer les profils personnalisés existants.
        // En revanche, une collection vide est une demande explicite de
        // suppression de tous les profils personnalisés (elle passe alors par
        // la validation des références).
        if custom.is_empty() && !parsed.is_empty() {
            return Ok(ProfilesSave {
                saved: SavedInfo {
                    bytes,
                    backup: BackupStatus::None,
                    applied: false,
                },
                tombstones: TombstoneCounts::default(),
            });
        }
        let profiles = ProfileCollection::new(custom);
        profiles
            .validate()
            .map_err(|error| validation_error("profiles", &error))?;

        // Lecture stricte : ne jamais quarantaine un fichier corrompu pendant
        // une validation croisée (A1-07/BUG-QA-003). Un fichier absent est le
        // premier démarrage : on valide contre les réglages par défaut.
        let mut settings = match self.store.load_settings_no_recovery() {
            Ok(settings) => settings,
            Err(StrictLoadError::NotFound { .. }) => Settings::default(),
            Err(_) => return Err(AdminError::Validation("settings".to_owned())),
        };
        // AD-R13 : le diff de tombstones ne porte que sur l'état `/var/lib`.
        // Les profils `/etc` sont un simple fallback de lecture : les
        // comparer ferait tombstoner des profils qu'un client n'a jamais vus.
        let previous = match self.store.load_profiles_state_only_no_recovery() {
            Ok(previous) => previous,
            Err(StrictLoadError::NotFound { .. }) => ProfileCollection::new(Vec::new()),
            Err(StrictLoadError::Invalid { .. } | StrictLoadError::TooLarge { .. }) => {
                return Err(AdminError::Validation("profiles".to_owned()));
            }
            Err(StrictLoadError::Io { .. }) => return Err(AdminError::WriteFailed),
        };

        // Un identifiant tombstoné ne peut jamais être recréé via l'API. Un ID
        // déjà présent dans l'ancienne collection reste accepté : le profil
        // peut avoir été conservé lors d'une réparation administrateur.
        for id in profiles.ids() {
            if !previous.contains(id) && settings.is_profile_id_deleted(id) {
                return Err(AdminError::Validation("deletedProfileIds".to_owned()));
            }
        }

        // Les profils supprimés deviennent des tombstones permanents. Un
        // identifiant géré par TCC (preset, profil personnalisé par défaut)
        // ne peut pas être supprimé par l'API.
        // Les presets ne sont jamais persistés par l'API (cf. filtre plus haut) :
        // un état hérité (migration, ancienne version) peut pourtant en
        // contenir. Leur absence d'un payload non vide n'est pas une
        // suppression ; un payload vide reste une demande explicite refusée.
        let deleted: Vec<tcc_core::ProfileId> = previous
            .iter()
            .filter(|profile| {
                (parsed.is_empty() || !profile.is_preset()) && !profiles.contains(&profile.id)
            })
            .map(|profile| profile.id.clone())
            .collect();
        if deleted.iter().any(tcc_core::ProfileId::is_tcc_managed) {
            return Err(AdminError::Validation("deletedProfileIds".to_owned()));
        }

        let mut tombstones_added = 0;
        let mut tombstones_preserved = 0;
        for id in &deleted {
            if settings.is_profile_id_deleted(id) {
                tombstones_preserved += 1;
            } else {
                settings
                    .mark_profile_deleted(id)
                    .map_err(|_| AdminError::Validation("deletedProfileIds".to_owned()))?;
                tombstones_added += 1;
            }
        }
        settings
            .validate()
            .map_err(|error| validation_error("settings", &error))?;

        let merged = merge_with_default_profiles(profiles.clone());
        validate_settings_references(&settings, &merged).map_err(|error| in_use_profile(&error))?;

        let tombstones = TombstoneCounts {
            deleted: deleted.len(),
            added: tombstones_added,
            preserved: tombstones_preserved,
        };
        let saved = if deleted.is_empty() {
            transaction::save_profiles(&self.store, self.reloader.as_ref(), &profiles, bytes)
                .await?
        } else {
            // Settings et profils sont écrits dans la même transaction : un
            // tombstone ne doit jamais être persisté sans la suppression
            // correspondante.
            transaction::save_profiles_with_settings(
                &self.store,
                self.reloader.as_ref(),
                &profiles,
                &settings,
                bytes,
            )
            .await?
        };
        Ok(ProfilesSave { saved, tombstones })
    }

    async fn save_settings_inner(
        &self,
        payload: &str,
        bytes: usize,
    ) -> Result<SavedInfo, AdminError> {
        Self::ensure_payload_size(payload.len())?;
        let mut settings: Settings = serde_json::from_str(payload)
            .map_err(|_| AdminError::InvalidArgs("settings".to_owned()))?;
        settings
            .validate()
            .map_err(|error| validation_error("settings", &error))?;

        // Les tombstones sont append-only via l'API : `SaveSettingsJSON` ne
        // peut ni en ajouter ni en retirer. Seule une suppression de profil
        // via `SaveProfilesJSON` en crée. Un fichier courant illisible est
        // refusé plutôt que remplacé : les tombstones existants sont inconnus.
        let current = match self.store.load_settings_no_recovery() {
            Ok(current) => current,
            Err(StrictLoadError::NotFound { .. }) => Settings::default(),
            Err(_) => return Err(AdminError::Validation("settings".to_owned())),
        };
        for id in &settings.deleted_profile_ids {
            if !current.is_profile_id_deleted(id) {
                return Err(AdminError::Validation("deletedProfileIds".to_owned()));
            }
        }
        for id in &current.deleted_profile_ids {
            if !settings.is_profile_id_deleted(id) {
                settings
                    .mark_profile_deleted(id)
                    .map_err(|_| AdminError::Validation("deletedProfileIds".to_owned()))?;
            }
        }
        settings
            .validate()
            .map_err(|error| validation_error("settings", &error))?;

        // Même règle : lecture stricte, aucun effet de bord, absence tolérée.
        let profiles = match self.store.load_profiles_no_recovery() {
            Ok(profiles) => profiles,
            Err(StrictLoadError::NotFound { .. }) => ProfileCollection::new(Vec::new()),
            Err(_) => return Err(AdminError::Validation("profiles".to_owned())),
        };
        let merged = merge_with_default_profiles(profiles);
        validate_settings_references(&settings, &merged).map_err(|error| in_use_profile(&error))?;

        transaction::save_settings(&self.store, self.reloader.as_ref(), &settings, bytes).await
    }
}

impl std::fmt::Debug for AdminService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdminService")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

#[zbus::interface(name = "com.tuxedocomputers.tccd.Admin1")]
impl AdminService {
    /// Version du contrat `Admin1`.
    #[zbus(property, name = "ContractVersion")]
    fn contract_version(&self) -> u32 {
        ADMIN_CONTRACT_VERSION
    }

    /// Diagnostic sans effet de bord.
    #[zbus(name = "Ping")]
    async fn ping(&self) -> bool {
        true
    }

    /// Version de l'implémentation.
    #[zbus(name = "GetVersion")]
    async fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Version du contrat `Admin1`, voie supportée pour la négociation.
    ///
    /// La propriété `ContractVersion` reste déclarée pour l'introspection
    /// interne, mais les clients ne doivent pas dépendre de `Properties.Get`
    /// (A1-11).
    #[zbus(name = "GetContractVersion")]
    async fn get_contract_version(&self) -> u32 {
        ADMIN_CONTRACT_VERSION
    }

    /// Enregistre la collection de profils personnalisés.
    ///
    /// Les presets sont filtrés. Le fichier n'est écrit que si le rechargement
    /// de `tccd` réussit sans avertissement ; sinon l'ancien contenu est
    /// restauré.
    #[zbus(name = "SaveProfilesJSON")]
    async fn save_profiles_json(
        &self,
        json: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self.caller_identity(&header, connection).await?;
        let context = CallContext {
            method: "SaveProfilesJSON",
            action: ACTION_PROFILES,
            caller,
            uid,
            started,
        };

        if let Err(error) = Self::ensure_payload_size(json.len()) {
            self.audit_failure(&context, Some(json.len()), &error);
            return Err(error);
        }
        self.authorize_write(&context).await?;

        let permit = match self.write_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.audit_failure(&context, Some(json.len()), &AdminError::Busy);
                return Err(AdminError::Busy);
            }
        };
        let bytes = json.len();
        let result = self.save_profiles_inner(&json, bytes).await;
        drop(permit);
        match &result {
            Ok(saved) => {
                self.audit_success(&context, saved.saved, saved.saved.reload_label());
                self.audit_tombstones(&context, saved.tombstones);
            }
            Err(error) => self.audit_failure(&context, Some(bytes), error),
        }
        result.map(|_| true)
    }

    /// Enregistre les réglages globaux.
    ///
    /// Le modèle doit être valide et toutes ses références de profils doivent
    /// exister sur disque. Transaction et rollback identiques à
    /// `SaveProfilesJSON`.
    #[zbus(name = "SaveSettingsJSON")]
    async fn save_settings_json(
        &self,
        json: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self.caller_identity(&header, connection).await?;
        let context = CallContext {
            method: "SaveSettingsJSON",
            action: ACTION_SETTINGS,
            caller,
            uid,
            started,
        };

        if let Err(error) = Self::ensure_payload_size(json.len()) {
            self.audit_failure(&context, Some(json.len()), &error);
            return Err(error);
        }
        self.authorize_write(&context).await?;

        let permit = match self.write_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.audit_failure(&context, Some(json.len()), &AdminError::Busy);
                return Err(AdminError::Busy);
            }
        };
        let bytes = json.len();
        let result = self.save_settings_inner(&json, bytes).await;
        drop(permit);
        match &result {
            Ok(saved) => self.audit_success(&context, *saved, saved.reload_label()),
            Err(error) => self.audit_failure(&context, Some(bytes), error),
        }
        result.map(|_| true)
    }

    /// Demande à `tccd` de relire la configuration sur disque.
    ///
    /// Retourne `true` si le rechargement n'a produit aucun avertissement.
    #[zbus(name = "ReloadConfig")]
    async fn reload_config(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> Result<bool, AdminError> {
        let _active = self.activity.enter();
        let started = Instant::now();
        let (caller, uid) = self.caller_identity(&header, connection).await?;
        let context = CallContext {
            method: "ReloadConfig",
            action: ACTION_SETTINGS,
            caller,
            uid,
            started,
        };
        self.authorize_write(&context).await?;

        match self.reloader.reload().await {
            Ok(report) => {
                let ok = report.is_ok();
                self.audit_success(
                    &context,
                    SavedInfo {
                        bytes: 0,
                        backup: BackupStatus::None,
                        applied: true,
                    },
                    if ok { "ok" } else { "warning" },
                );
                Ok(ok)
            }
            Err(_) => {
                self.audit_failure(&context, None, &AdminError::ReloadFailed);
                Err(AdminError::ReloadFailed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use tcc_core::{ConfigStore, Profile, ProfileCollection, ProfileId, Settings};
    use tcc_ipc::AdminReloadReport;

    use super::{
        Activity, AdminService, Authorizer, AuthzDecision, BackupStatus, IDLE_TIMEOUT,
        MAX_PAYLOAD_BYTES, ReloadError, ReloadNotifier, TombstoneCounts,
    };
    use crate::error::AdminError;

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

    struct FixedReloader {
        report: AdminReloadReport,
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl ReloadNotifier for FixedReloader {
        async fn reload(&self) -> Result<AdminReloadReport, ReloadError> {
            *self.calls.lock().expect("calls") += 1;
            Ok(self.report.clone())
        }
    }

    fn store(root: &std::path::Path) -> ConfigStore {
        // Politique par défaut : le test tourne avec l'UID du développeur,
        // contrairement à la production `ConfigStore::system()` qui exige
        // root. La portée d'écriture reste bornée au tempdir.
        ConfigStore::with_admin_roots(root, root.join("admin"))
            .with_policy(tcc_core::config::LoadPolicy::default())
            .with_write_root(root)
    }

    fn service(
        store: ConfigStore,
        report: AdminReloadReport,
        authorizer: std::sync::Arc<dyn Authorizer>,
    ) -> (AdminService, std::sync::Arc<FixedReloader>) {
        let reloader = std::sync::Arc::new(FixedReloader {
            report,
            calls: Mutex::new(0),
        });
        let activity = std::sync::Arc::new(Activity::new());
        let service = AdminService::new(store, reloader.clone(), authorizer, activity);
        (service, reloader)
    }

    #[test]
    fn activity_idle_detection() {
        let activity = Activity::new();
        assert!(!activity.is_idle(IDLE_TIMEOUT));
        activity.touch();
        assert!(!activity.is_idle(IDLE_TIMEOUT));
    }

    #[tokio::test]
    async fn idle_watch_requests_shutdown() {
        let activity = std::sync::Arc::new(Activity::new());
        let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
        super::spawn_idle_watch(activity, shutdown.clone(), std::time::Duration::ZERO);
        tokio::time::timeout(std::time::Duration::from_secs(3), shutdown.notified())
            .await
            .expect("la supervision doit demander l'arrêt");
    }

    fn context() -> super::CallContext {
        super::CallContext {
            method: "SaveSettingsJSON",
            action: "com.tuxedocomputers.tccd1.settings",
            caller: ":1.42".to_owned(),
            uid: 1000,
            started: std::time::Instant::now(),
        }
    }

    #[tokio::test]
    async fn denied_call_does_not_extend_activity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, _) = service(
            store(dir.path()),
            AdminReloadReport::clean(),
            std::sync::Arc::new(DenyAll),
        );
        let activity = service.activity();
        let before = activity.last_for_test();
        assert_eq!(
            service.authorize_write(&context()).await,
            Err(AdminError::NotAuthorized)
        );
        assert_eq!(
            activity.last_for_test(),
            before,
            "un refus ne doit pas prolonger le processus root"
        );
    }

    #[tokio::test]
    async fn authorized_call_extends_activity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, _) = service(
            store(dir.path()),
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let activity = service.activity();
        let before = activity.last_for_test();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        service.authorize_write(&context()).await.expect("autorisé");
        assert!(
            activity.last_for_test() > before,
            "un appel autorisé doit prolonger l'activité"
        );
    }

    #[test]
    fn in_flight_call_prevents_idle_shutdown() {
        let activity = Activity::new();
        assert!(activity.is_idle(std::time::Duration::ZERO));
        let guard = activity.enter();
        assert!(
            !activity.is_idle(std::time::Duration::ZERO),
            "un appel en cours ne doit pas être interrompu par l'inactivité"
        );
        drop(guard);
        assert!(activity.is_idle(std::time::Duration::ZERO));
    }

    #[test]
    fn payload_size_is_bounded_before_parse() {
        assert!(AdminService::ensure_payload_size(0).is_ok());
        assert!(AdminService::ensure_payload_size(MAX_PAYLOAD_BYTES).is_ok());
        assert_eq!(
            AdminService::ensure_payload_size(MAX_PAYLOAD_BYTES + 1),
            Err(AdminError::PayloadTooLarge)
        );
    }

    #[tokio::test]
    async fn preset_only_payload_is_a_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let custom = ProfileCollection::new(vec![Profile::new("custom-keep", "Keep")]);
        store.save_profiles(&custom).expect("profils existants");
        let before = std::fs::read(dir.path().join("profiles.json")).expect("avant");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let presets: Vec<Profile> = tcc_core::default_profiles()
            .iter()
            .filter(|profile| profile.is_preset())
            .cloned()
            .collect();
        assert_eq!(presets.len(), 4);
        let payload = serde_json::to_string(&ProfileCollection::new(presets)).expect("JSON");
        let saved = temp
            .0
            .save_profiles_inner(&payload, payload.len())
            .await
            .expect("noop");
        assert_eq!(saved.saved.bytes, payload.len());
        assert_eq!(saved.saved.backup, BackupStatus::None);
        assert!(!saved.saved.applied);
        assert_eq!(saved.saved.reload_label(), "skipped");
        assert_eq!(saved.tombstones, TombstoneCounts::default());
        assert_eq!(*temp.1.calls.lock().expect("calls"), 0);
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("après"),
            before
        );
    }

    #[tokio::test]
    async fn empty_payload_deletes_custom_profiles() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-old",
                "Old",
            )]))
            .expect("profils existants");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        let saved = temp
            .0
            .save_profiles_inner(payload, payload.len())
            .await
            .expect("écriture");
        assert_eq!(saved.saved.backup, BackupStatus::Written);
        assert_eq!(
            saved.tombstones,
            TombstoneCounts {
                deleted: 1,
                added: 1,
                preserved: 0
            }
        );
        let written: ProfileCollection = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("profiles.json")).expect("lecture"),
        )
        .expect("JSON");
        assert!(written.is_empty());
        let settings: Settings = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).expect("settings"),
        )
        .expect("JSON");
        assert!(
            settings.is_profile_id_deleted(&ProfileId::new("custom-old")),
            "le profil supprimé doit être tombstoné"
        );
        assert_eq!(*temp.1.calls.lock().expect("calls"), 1);
        let backup = dir.path().join("profiles.json.bak");
        assert!(backup.exists(), "sauvegarde attendue");
        assert!(
            dir.path().join("settings.json").exists(),
            "les réglages sont écrits dans la même transaction"
        );
        assert!(
            !dir.path().join("settings.json.bak").exists(),
            "aucune sauvegarde pour un premier fichier de réglages"
        );
    }

    #[tokio::test]
    async fn deleted_profile_id_cannot_be_recreated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-old",
                "Old",
            )]))
            .expect("profils existants");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        temp.0
            .save_profiles_inner(payload, payload.len())
            .await
            .expect("suppression");
        let profiles_before = std::fs::read(dir.path().join("profiles.json")).expect("profils");
        let settings_before = std::fs::read(dir.path().join("settings.json")).expect("settings");

        let recreate = serde_json::to_string(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Again",
        )]))
        .expect("JSON");
        assert_eq!(
            temp.0.save_profiles_inner(&recreate, recreate.len()).await,
            Err(AdminError::Validation("deletedProfileIds".to_owned()))
        );
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("profils"),
            profiles_before,
            "aucune écriture ne doit avoir lieu"
        );
        assert_eq!(
            std::fs::read(dir.path().join("settings.json")).expect("settings"),
            settings_before,
            "aucun tombstone ne doit être ajouté"
        );
        assert_eq!(
            *temp.1.calls.lock().expect("calls"),
            1,
            "un refus ne déclenche aucun rechargement"
        );
    }

    #[tokio::test]
    async fn new_profile_id_is_accepted_next_to_tombstones() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let mut settings = Settings::default();
        settings
            .mark_profile_deleted(&ProfileId::new("custom-gone"))
            .expect("tombstone");
        store.save_settings(&settings).expect("settings");
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-live",
                "Live",
            )]))
            .expect("profils");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = serde_json::to_string(&ProfileCollection::new(vec![
            Profile::new("custom-live", "Live"),
            Profile::new("custom-new", "New"),
        ]))
        .expect("JSON");
        let saved = temp
            .0
            .save_profiles_inner(&payload, payload.len())
            .await
            .expect("création autorisée");
        assert_eq!(saved.tombstones, TombstoneCounts::default());
    }

    #[tokio::test]
    async fn legacy_presets_on_disk_do_not_block_a_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let mut legacy: Vec<Profile> = tcc_core::default_profiles()
            .iter()
            .filter(|profile| profile.is_preset())
            .cloned()
            .collect();
        legacy.push(Profile::new("custom-live", "Live"));
        store
            .save_profiles(&ProfileCollection::new(legacy))
            .expect("profils");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = serde_json::to_string(&ProfileCollection::new(vec![
            Profile::new("custom-live", "Live"),
            Profile::new("custom-new", "New"),
        ]))
        .expect("JSON");
        let saved = temp
            .0
            .save_profiles_inner(&payload, payload.len())
            .await
            .expect("sauvegarde autorisée");
        assert_eq!(saved.tombstones, TombstoneCounts::default());
    }

    #[tokio::test]
    async fn deleting_a_preset_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let preset = tcc_core::default_profiles()
            .iter()
            .find(|profile| profile.is_preset())
            .cloned()
            .expect("preset");
        store
            .save_profiles(&ProfileCollection::new(vec![preset]))
            .expect("profil preset");
        let before = std::fs::read(dir.path().join("profiles.json")).expect("profils");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        assert_eq!(
            temp.0.save_profiles_inner(payload, payload.len()).await,
            Err(AdminError::Validation("deletedProfileIds".to_owned()))
        );
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("profils"),
            before
        );
    }

    #[tokio::test]
    async fn etc_profile_is_not_tombstoned_by_state_diff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let admin_profiles_path = store.admin_paths().expect("racine admin").profiles.clone();
        std::fs::create_dir_all(admin_profiles_path.parent().expect("parent"))
            .expect("répertoire admin");
        let admin_profiles = ProfileCollection::new(vec![Profile::new("custom-admin", "Admin")]);
        let admin_bytes = serde_json::to_vec(&admin_profiles).expect("JSON");
        std::fs::write(&admin_profiles_path, &admin_bytes).expect("profil /etc");
        // Aucun `profiles.json` d'état : le repli `/etc` est la seule source
        // visible avant la transaction.
        assert!(!store.paths().profiles.exists());

        let temp = service(
            store.clone(),
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = serde_json::to_string(&ProfileCollection::new(vec![Profile::new(
            "custom-state",
            "State",
        )]))
        .expect("JSON");
        let saved = temp
            .0
            .save_profiles_inner(&payload, payload.len())
            .await
            .expect("écriture");
        assert_eq!(saved.tombstones, TombstoneCounts::default());

        let settings_path = dir.path().join("settings.json");
        let settings = if settings_path.exists() {
            serde_json::from_slice(&std::fs::read(&settings_path).expect("settings")).expect("JSON")
        } else {
            Settings::default()
        };
        assert!(
            !settings.is_profile_id_deleted(&ProfileId::new("custom-admin")),
            "un profil /etc ne doit jamais être tombstoné par un diff d'état"
        );
        assert_eq!(
            std::fs::read(&admin_profiles_path).expect("profil /etc intact"),
            admin_bytes
        );

        // L'ID `/etc` n'est pas tombstoné : il reste importable dans l'état.
        let readd = serde_json::to_string(&ProfileCollection::new(vec![
            Profile::new("custom-state", "State"),
            Profile::new("custom-admin", "Admin"),
        ]))
        .expect("JSON");
        temp.0
            .save_profiles_inner(&readd, readd.len())
            .await
            .expect("profil /etc encore disponible");
    }

    #[tokio::test]
    async fn tombstone_limit_is_enforced_before_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let mut settings = Settings::default();
        for index in 0..tcc_core::MAX_DELETED_PROFILE_IDS {
            settings
                .mark_profile_deleted(&ProfileId::new(format!("custom-gone-{index}")))
                .expect("tombstone");
        }
        store.save_settings(&settings).expect("settings");
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-extra",
                "Extra",
            )]))
            .expect("profils");
        let profiles_before = std::fs::read(dir.path().join("profiles.json")).expect("profils");
        let settings_before = std::fs::read(dir.path().join("settings.json")).expect("settings");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        assert_eq!(
            temp.0.save_profiles_inner(payload, payload.len()).await,
            Err(AdminError::Validation("deletedProfileIds".to_owned()))
        );
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("profils"),
            profiles_before
        );
        assert_eq!(
            std::fs::read(dir.path().join("settings.json")).expect("settings"),
            settings_before
        );
    }

    #[tokio::test]
    async fn reload_failure_rolls_back_profiles_and_settings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        store
            .save_settings(&Settings::default())
            .expect("settings initiaux");
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-old",
                "Old",
            )]))
            .expect("profils initiaux");
        let settings_before = std::fs::read(dir.path().join("settings.json")).expect("settings");
        let profiles_before = std::fs::read(dir.path().join("profiles.json")).expect("profils");

        let temp = service(
            store,
            AdminReloadReport::from_warnings(vec!["configuration invalide".to_owned()]),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        assert_eq!(
            temp.0.save_profiles_inner(payload, payload.len()).await,
            Err(AdminError::ReloadFailed)
        );
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("profils"),
            profiles_before,
            "les profils doivent être restaurés"
        );
        assert_eq!(
            std::fs::read(dir.path().join("settings.json")).expect("settings"),
            settings_before,
            "les réglages doivent être restaurés"
        );
        let restored: Settings = serde_json::from_slice(
            &std::fs::read(dir.path().join("settings.json")).expect("settings"),
        )
        .expect("JSON");
        assert!(
            !restored.is_profile_id_deleted(&ProfileId::new("custom-old")),
            "aucun tombstone partiel ne doit subsister"
        );
        assert_eq!(*temp.1.calls.lock().expect("calls"), 2);
    }

    #[tokio::test]
    async fn reload_failure_without_previous_settings_restores_absence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-old",
                "Old",
            )]))
            .expect("profils initiaux");
        let profiles_before = std::fs::read(dir.path().join("profiles.json")).expect("profils");
        assert!(
            !dir.path().join("settings.json").exists(),
            "settings absent au départ"
        );

        let failing = service(
            store.clone(),
            AdminReloadReport::from_warnings(vec!["configuration invalide".to_owned()]),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        assert_eq!(
            failing.0.save_profiles_inner(payload, payload.len()).await,
            Err(AdminError::ReloadFailed)
        );
        assert!(
            !dir.path().join("settings.json").exists(),
            "aucun settings.json résiduel ne doit contenir un tombstone partiel"
        );
        assert_eq!(
            std::fs::read(dir.path().join("profiles.json")).expect("profils"),
            profiles_before,
            "les profils doivent être restaurés"
        );

        // Aucun tombstone n'a été persisté : après disparition de l'état, le
        // profil est de nouveau recréable.
        std::fs::remove_file(dir.path().join("profiles.json")).expect("suppression état");
        let clean = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let recreate = serde_json::to_string(&ProfileCollection::new(vec![Profile::new(
            "custom-old",
            "Old",
        )]))
        .expect("JSON");
        let saved = clean
            .0
            .save_profiles_inner(&recreate, recreate.len())
            .await
            .expect("profil recréable");
        assert_eq!(saved.tombstones, TombstoneCounts::default());
    }

    #[tokio::test]
    async fn tombstones_cannot_be_removed_via_settings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let mut settings = Settings::default();
        settings
            .mark_profile_deleted(&ProfileId::new("custom-gone"))
            .expect("tombstone");
        store.save_settings(&settings).expect("settings");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = serde_json::to_string(&Settings::default()).expect("JSON");
        temp.0
            .save_settings_inner(&payload, payload.len())
            .await
            .expect("écriture");
        let written: Settings = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).expect("settings"),
        )
        .expect("JSON");
        assert!(
            written.is_profile_id_deleted(&ProfileId::new("custom-gone")),
            "un tombstone existant ne peut pas être retiré via SaveSettingsJSON"
        );
    }

    #[tokio::test]
    async fn tombstones_cannot_be_added_via_settings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let mut settings = Settings::default();
        settings
            .mark_profile_deleted(&ProfileId::new("custom-x"))
            .expect("tombstone");
        let payload = serde_json::to_string(&settings).expect("JSON");
        assert_eq!(
            temp.0.save_settings_inner(&payload, payload.len()).await,
            Err(AdminError::Validation("deletedProfileIds".to_owned()))
        );
        assert!(!dir.path().join("settings.json").exists());
    }

    #[tokio::test]
    async fn removing_a_referenced_profile_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        store
            .save_profiles(&ProfileCollection::new(vec![Profile::new(
                "custom-in-use",
                "In Use",
            )]))
            .expect("profils");
        let settings = Settings {
            state_map: tcc_core::LegacyStateMap {
                power_ac: ProfileId::new("custom-in-use"),
                ..tcc_core::LegacyStateMap::default()
            },
            ..Settings::default()
        };
        store.save_settings(&settings).expect("settings");

        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let payload = "[]";
        assert_eq!(
            temp.0.save_profiles_inner(payload, payload.len()).await,
            Err(AdminError::InUse("custom-in-use".to_owned()))
        );
        assert_eq!(*temp.1.calls.lock().expect("calls"), 0);
        let on_disk: ProfileCollection = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("profiles.json")).expect("lecture"),
        )
        .expect("JSON");
        assert_eq!(on_disk.len(), 1);
    }

    #[tokio::test]
    async fn invalid_model_is_rejected_before_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let error = temp
            .0
            .save_settings_inner("{\"schemaVersion\": 999}", 25)
            .await
            .expect_err("schéma invalide");
        assert!(matches!(error, AdminError::Validation(_)));
        assert!(!dir.path().join("settings.json").exists());
    }

    #[tokio::test]
    async fn malformed_json_is_invalid_args() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        assert_eq!(
            temp.0.save_settings_inner("not-json", 8).await,
            Err(AdminError::InvalidArgs("settings".to_owned()))
        );
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn write_gate_is_exclusive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let temp = service(
            store(dir.path()),
            AdminReloadReport::clean(),
            std::sync::Arc::new(AllowAll),
        );
        let first = temp
            .0
            .write_gate
            .clone()
            .try_acquire_owned()
            .expect("premier verrou");
        assert!(
            temp.0.write_gate.clone().try_acquire_owned().is_err(),
            "une seconde écriture doit être refusée"
        );
        drop(first);
        assert!(temp.0.write_gate.clone().try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn deny_does_not_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store(dir.path());
        let temp = service(
            store,
            AdminReloadReport::clean(),
            std::sync::Arc::new(DenyAll),
        );
        assert_eq!(
            temp.0
                .authorize_write(&super::CallContext {
                    method: "SaveSettingsJSON",
                    action: "action",
                    caller: ":1.1".to_owned(),
                    uid: 1000,
                    started: std::time::Instant::now(),
                })
                .await,
            Err(AdminError::NotAuthorized)
        );
        assert!(!dir.path().join("settings.json").exists());
    }
}
