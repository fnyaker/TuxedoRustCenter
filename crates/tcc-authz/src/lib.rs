// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! Autorisation polkit partagée par `tccd` et `tccd-admin` (AD-3, AD-4, AD-9).
//!
//! # Contrat
//!
//! - le sujet polkit est toujours le **nom unique de bus** de l'appelant
//!   ([`AuthorizationRequest::caller_bus_name`]), jamais un PID fourni par le
//!   client (`unix-process` est interdit) ;
//! - l'appel `CheckAuthorization` est fait **même pour root** : aucun
//!   court-circuit, aucun fallback ;
//! - polkit absent, en erreur ou en timeout ⇒ refus (fail-closed) ;
//! - aucun cache local : le seul cache est celui décidé par
//!   l'administrateur système via les defaults polkit ;
//! - [`AuthorizationLimiter`] et [`DialogGate`] fournissent les garde-fous
//!   anti-dialog-storm, avec des seuils choisis par le service appelant.
//!
//! # Exemple
//!
//! ```ignore
//! let request = AuthorizationRequest::new(":1.42", "com.tuxedocomputers.tccd1.profiles")
//!     .with_caller_uid(1000);
//! let result = check_authorization(&request).await?;
//! if !result.is_authorized() {
//!     // refus : ne jamais poursuivre l'opération
//! }
//! ```
//!
//! # Frontière
//!
//! Le client D-Bus PolicyKit1 et le timeout vivent dans [`ZbusPolicyKit`].
//! Les tests et les binaires peuvent injecter un autre
//! [`PolicyKitBackend`] via [`check_authorization_with`] sans toucher au bus
//! système.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thiserror::Error;

/// Délai par défaut d'un dialogue polkit (120 s, AD-3).
pub const DEFAULT_AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
/// Délai minimal accepté.
pub const MIN_AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(1);
/// Délai maximal accepté (garde-fou anti-blocage).
pub const MAX_AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(300);
/// Longueur maximale d'un nom unique de bus D-Bus.
pub const MAX_CALLER_NAME_CHARS: usize = 255;
/// Longueur maximale d'un identifiant d'action polkit.
pub const MAX_ACTION_ID_CHARS: usize = 128;
/// Attente maximale par défaut pour obtenir le dialogue polkit unique.
pub const DEFAULT_DIALOG_WAIT: Duration = Duration::from_secs(2);
/// Nombre maximal d'appelants suivis par [`AuthorizationLimiter`].
pub const MAX_LIMITER_ENTRIES: usize = 1024;

/// Service PolicyKit1 sur le bus système.
pub const POLKIT_SERVICE: &str = "org.freedesktop.PolicyKit1";
/// Objet `Authority` de PolicyKit1.
pub const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
/// Interface `Authority` de PolicyKit1.
pub const POLKIT_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";
/// Type du sujet polkit utilisé : nom unique de l'appelant.
pub const SUBJECT_SYSTEM_BUS_NAME: &str = "system-bus-name";

/// Demande d'autorisation adressée à PolicyKit1.
///
/// Le champ [`AuthorizationRequest::caller_bus_name`] doit être le nom unique
/// du **client** (`:1.42`), obtenu depuis l'en-tête D-Bus. Le nom unique de
/// `tccd-admin` ou de `tccd` est interdit : ce serait un confused deputy
/// (TA-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationRequest {
    /// Nom unique de bus du client (`:` suivi de chiffres), jamais un nom
    /// bien connu ni un PID.
    pub caller_bus_name: String,
    /// UID du client, résolu avant l'appel polkit. `None` doit être refusé
    /// par l'implémentation : l'identité inconnue n'est jamais autorisée.
    pub caller_uid: Option<u32>,
    /// Identifiant d'action polkit (`com.tuxedocomputers.tccd1.…`).
    pub action_id: String,
    /// Autorise le dialogue interactif (`AllowUserInteraction`).
    ///
    /// Vrai pour les actions `auth_admin*` ; faux pour une vérification
    /// silencieuse, qui échouera si une authentification est requise.
    pub allow_interaction: bool,
    /// Délai maximal de la réponse polkit.
    pub timeout: Duration,
}

impl AuthorizationRequest {
    /// Construit une demande interactive avec le délai par défaut.
    ///
    /// L'UID n'est pas fourni : l'appelant doit appeler
    /// [`AuthorizationRequest::with_caller_uid`] après résolution
    /// (`GetConnectionUnixUser`).
    #[must_use]
    pub fn new(caller_bus_name: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self {
            caller_bus_name: caller_bus_name.into(),
            caller_uid: None,
            action_id: action_id.into(),
            allow_interaction: true,
            timeout: DEFAULT_AUTHORIZATION_TIMEOUT,
        }
    }

    /// Renseigne l'UID résolu de l'appelant.
    #[must_use]
    pub fn with_caller_uid(mut self, uid: u32) -> Self {
        self.caller_uid = Some(uid);
        self
    }

    /// Remplace le délai d'autorisation.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Active ou désactive le dialogue interactif.
    #[must_use]
    pub fn with_interaction(mut self, allow_interaction: bool) -> Self {
        self.allow_interaction = allow_interaction;
        self
    }

    /// Valide la forme de la requête avant tout appel externe.
    ///
    /// Cette validation est **structurelle** : elle ne remplace pas la
    /// décision polkit et ne vérifie aucun droit.
    ///
    /// # Errors
    ///
    /// Renvoie [`AuthzError::InvalidRequest`] si le nom de bus n'est pas un
    /// nom unique, si l'action est vide ou mal formée, ou si le délai sort de
    /// `[1 s, 300 s]`.
    pub fn validate(&self) -> Result<(), AuthzError> {
        validate_caller_name(&self.caller_bus_name)?;
        validate_action_id(&self.action_id)?;
        if self.timeout < MIN_AUTHORIZATION_TIMEOUT || self.timeout > MAX_AUTHORIZATION_TIMEOUT {
            return Err(AuthzError::InvalidRequest(format!(
                "délai hors bornes: {} ms",
                self.timeout.as_millis()
            )));
        }
        Ok(())
    }
}

/// Résultat d'une vérification d'autorisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthorizationResult {
    /// polkit a répondu `is_authorized = true` **et** `is_challenge = false`.
    Authorized,
    /// Refus explicite, avec un motif typé non détaillé au client.
    Denied(DenialReason),
}

impl AuthorizationResult {
    /// Vrai uniquement pour [`AuthorizationResult::Authorized`].
    ///
    /// Un `is_challenge = true` ne doit jamais être interprété comme un
    /// succès : l'implémentation le convertit en
    /// [`AuthorizationResult::Denied`].
    #[must_use]
    pub const fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized)
    }
}

/// Motif de refus, utilisé pour l'audit interne.
///
/// Le message renvoyé au client reste générique
/// (`Admin1.Error.NotAuthorized`) : ce motif ne doit pas être exposé sur le
/// bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DenialReason {
    /// `is_authorized = false`.
    PolicyDenied,
    /// L'utilisateur a annulé le dialogue (`is_challenge = true`).
    ChallengeCancelled,
    /// Le délai d'autorisation a expiré.
    Timeout,
    /// PolicyKit1 est absent ou en erreur.
    PolicyKitUnavailable,
    /// L'UID de l'appelant n'a pas pu être résolu.
    UnknownCaller,
    /// Un dialogue polkit est déjà en cours pour cet appelant.
    DialogBusy,
}

/// Réponse brute de PolicyKit1.
///
/// La décision finale est centralisée dans
/// [`AuthorizationResult`] : une implémentation de [`PolicyKitBackend`] ne
/// peut pas considérer `is_challenge = true` comme une autorisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawAuthorization {
    /// Champ `is_authorized` de la réponse.
    pub is_authorized: bool,
    /// Champ `is_challenge` de la réponse.
    pub is_challenge: bool,
}

impl RawAuthorization {
    /// Construit une réponse brute.
    #[must_use]
    pub const fn new(is_authorized: bool, is_challenge: bool) -> Self {
        Self {
            is_authorized,
            is_challenge,
        }
    }

    /// Décision sûre : seul `is_authorized && !is_challenge` autorise.
    #[must_use]
    pub const fn decision(self) -> AuthorizationResult {
        if self.is_authorized && !self.is_challenge {
            AuthorizationResult::Authorized
        } else if self.is_challenge {
            AuthorizationResult::Denied(DenialReason::ChallengeCancelled)
        } else {
            AuthorizationResult::Denied(DenialReason::PolicyDenied)
        }
    }
}

/// Erreur d'infrastructure d'autorisation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AuthzError {
    /// La requête est mal formée ; aucun appel externe n'a été effectué.
    #[error("requête d'autorisation invalide: {0}")]
    InvalidRequest(String),
    /// PolicyKit1 est absent, inaccessible ou en erreur.
    #[error("polkit indisponible: {0}")]
    Unavailable(String),
    /// L'identité de l'appelant n'a pas pu être résolue.
    #[error("identité de l'appelant inconnue")]
    UnknownCaller,
}

/// Backend d'autorisation injectable.
///
/// L'implémentation de production est [`ZbusPolicyKit`]. Les tests utilisent
/// un faux backend et [`check_authorization_with`] pour vérifier le sujet, les
/// drapeaux et les décisions sans bus système.
#[async_trait]
pub trait PolicyKitBackend: Send + Sync {
    /// Appelle `CheckAuthorization` et renvoie la réponse brute.
    ///
    /// # Errors
    ///
    /// Renvoie [`AuthzError::Unavailable`] si polkit est absent ou si le
    /// transport échoue.
    async fn check_authorization(
        &self,
        caller_bus_name: &str,
        action_id: &str,
        allow_interaction: bool,
    ) -> Result<RawAuthorization, AuthzError>;
}

/// Client PolicyKit1 réel sur le bus système.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZbusPolicyKit;

#[async_trait]
impl PolicyKitBackend for ZbusPolicyKit {
    async fn check_authorization(
        &self,
        caller_bus_name: &str,
        action_id: &str,
        allow_interaction: bool,
    ) -> Result<RawAuthorization, AuthzError> {
        let connection = zbus::Connection::system()
            .await
            .map_err(|error| AuthzError::Unavailable(error.to_string()))?;
        let proxy = zbus::Proxy::new(&connection, POLKIT_SERVICE, POLKIT_PATH, POLKIT_INTERFACE)
            .await
            .map_err(|error| AuthzError::Unavailable(error.to_string()))?;

        // Sujet `("system-bus-name", {"name": <unique>})` : jamais le PID
        // fourni par le client, jamais notre propre nom.
        let subject = system_bus_subject(caller_bus_name);
        let details: HashMap<String, String> = HashMap::new();
        let flags: u32 = if allow_interaction { 1 } else { 0 };

        let reply = proxy
            .call_method(
                "CheckAuthorization",
                &(subject, action_id, details, flags, ""),
            )
            .await
            .map_err(|error| AuthzError::Unavailable(error.to_string()))?;
        let (is_authorized, is_challenge, _details): (bool, bool, HashMap<String, String>) = reply
            .body()
            .deserialize()
            .map_err(|error| AuthzError::Unavailable(error.to_string()))?;
        Ok(RawAuthorization::new(is_authorized, is_challenge))
    }
}

/// Vérifie une autorisation polkit avec le backend de production.
///
/// # Errors
///
/// - [`AuthzError::InvalidRequest`] si la requête est mal formée ;
/// - [`AuthzError::UnknownCaller`] si l'UID de l'appelant n'est pas résolu.
///
/// Toute indisponibilité de polkit ou tout timeout est converti en
/// [`AuthorizationResult::Denied`] : l'appelant ne doit jamais poursuivre.
pub async fn check_authorization(
    request: &AuthorizationRequest,
) -> Result<AuthorizationResult, AuthzError> {
    check_authorization_with(request, &ZbusPolicyKit).await
}

/// Vérifie une autorisation avec un backend injectable.
///
/// La décision applique toujours `is_authorized && !is_challenge`, même si le
/// backend retourne une combinaison incohérente.
///
/// # Errors
///
/// Voir [`check_authorization`].
pub async fn check_authorization_with<B>(
    request: &AuthorizationRequest,
    backend: &B,
) -> Result<AuthorizationResult, AuthzError>
where
    B: PolicyKitBackend + ?Sized,
{
    request.validate()?;
    if request.caller_uid.is_none() {
        return Err(AuthzError::UnknownCaller);
    }

    match tokio::time::timeout(
        request.timeout,
        backend.check_authorization(
            &request.caller_bus_name,
            &request.action_id,
            request.allow_interaction,
        ),
    )
    .await
    {
        Err(_) => Ok(AuthorizationResult::Denied(DenialReason::Timeout)),
        Ok(Err(AuthzError::UnknownCaller)) => Err(AuthzError::UnknownCaller),
        Ok(Err(_)) => Ok(AuthorizationResult::Denied(
            DenialReason::PolicyKitUnavailable,
        )),
        Ok(Ok(raw)) => Ok(raw.decision()),
    }
}

/// Construit le sujet polkit `("system-bus-name", {"name": <unique>})`.
fn system_bus_subject(
    caller_bus_name: &str,
) -> (String, HashMap<String, zbus::zvariant::Value<'_>>) {
    let mut properties: HashMap<String, zbus::zvariant::Value<'_>> = HashMap::new();
    properties.insert(
        "name".to_owned(),
        zbus::zvariant::Value::from(caller_bus_name),
    );
    (SUBJECT_SYSTEM_BUS_NAME.to_owned(), properties)
}

fn validate_caller_name(caller: &str) -> Result<(), AuthzError> {
    if !caller.starts_with(':') {
        return Err(AuthzError::InvalidRequest(
            "le sujet doit être un nom unique de bus (préfixe ':')".to_owned(),
        ));
    }
    if caller.chars().count() > MAX_CALLER_NAME_CHARS {
        return Err(AuthzError::InvalidRequest(format!(
            "nom de bus de plus de {MAX_CALLER_NAME_CHARS} caractères"
        )));
    }
    if !caller.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '-' | '_')
    }) {
        return Err(AuthzError::InvalidRequest(
            "nom de bus contenant un caractère invalide".to_owned(),
        ));
    }
    Ok(())
}

fn validate_action_id(action_id: &str) -> Result<(), AuthzError> {
    if action_id.is_empty() {
        return Err(AuthzError::InvalidRequest(
            "identifiant d'action vide".to_owned(),
        ));
    }
    if action_id.chars().count() > MAX_ACTION_ID_CHARS {
        return Err(AuthzError::InvalidRequest(format!(
            "identifiant d'action de plus de {MAX_ACTION_ID_CHARS} caractères"
        )));
    }
    if !action_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(AuthzError::InvalidRequest(
            "identifiant d'action contenant un caractère invalide".to_owned(),
        ));
    }
    if !action_id.contains('.') {
        return Err(AuthzError::InvalidRequest(
            "identifiant d'action sans namespace".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(now: Instant, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: now,
        }
    }

    fn allow(&mut self, now: Instant, rate: f64, burst: f64) -> bool {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(burst);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Limiteur de tentatives d'autorisation, par nom unique **et** par UID.
///
/// Il doit être appelé **avant** polkit pour qu'un client en boucle ne
/// déclenche pas de dialogue. Les seuils sont choisis par le service
/// (AD-9) : l'admin utilise 2 appels/s avec un burst de 4.
#[derive(Debug)]
pub struct AuthorizationLimiter {
    rate_per_second: f64,
    burst: f64,
    names: Mutex<HashMap<String, TokenBucket>>,
    uids: Mutex<HashMap<u32, TokenBucket>>,
}

impl AuthorizationLimiter {
    /// Crée un limiteur avec les seuils donnés.
    ///
    /// Les valeurs non finies, nulles ou négatives retombent sur 2/s burst 4.
    #[must_use]
    pub fn new(rate_per_second: f64, burst: f64) -> Self {
        let rate_per_second = if rate_per_second.is_finite() && rate_per_second > 0.0 {
            rate_per_second
        } else {
            2.0
        };
        let burst = if burst.is_finite() && burst >= 1.0 {
            burst
        } else {
            4.0
        };
        Self {
            rate_per_second,
            burst,
            names: Mutex::new(HashMap::new()),
            uids: Mutex::new(HashMap::new()),
        }
    }

    /// Vérifie et consomme un jeton pour `(caller, uid)`.
    ///
    /// Le refus est immédiat si l'un des deux buckets est épuisé. La map est
    /// bornée : au-delà de [`MAX_LIMITER_ENTRIES`], l'entrée la plus ancienne
    /// est évincée.
    pub fn allow(&self, caller: &str, uid: u32, now: Instant) -> bool {
        let rate = self.rate_per_second;
        let burst = self.burst;
        let caller = caller.to_owned();
        let name_allowed = with_bucket(&self.names, &caller, now, rate, burst);
        let uid_allowed = with_bucket(&self.uids, &uid, now, rate, burst);
        name_allowed && uid_allowed
    }
}

impl Default for AuthorizationLimiter {
    fn default() -> Self {
        Self::new(2.0, 4.0)
    }
}

fn with_bucket<K>(
    buckets: &Mutex<HashMap<K, TokenBucket>>,
    key: &K,
    now: Instant,
    rate: f64,
    burst: f64,
) -> bool
where
    K: Eq + std::hash::Hash + Clone,
{
    let mut guard = match buckets.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.len() >= MAX_LIMITER_ENTRIES && !guard.contains_key(key) {
        if let Some(oldest) = guard
            .iter()
            .min_by_key(|(_, bucket)| bucket.last_refill)
            .map(|(key, _)| key.clone())
        {
            guard.remove(&oldest);
        }
    }
    guard
        .entry(key.clone())
        .or_insert_with(|| TokenBucket::new(now, burst))
        .allow(now, rate, burst)
}

/// Résultat de l'acquisition d'un dialogue polkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogGateError {
    /// Un dialogue est déjà en cours pour ce nom unique.
    CallerBusy,
    /// Le dialogue global est occupé au-delà de l'attente autorisée.
    GlobalBusy,
}

/// Verrou qui n'autorise qu'un seul dialogue polkit à la fois.
///
/// Il empêche N dialogues simultanés (AD-4) et refuse immédiatement un second
/// dialogue pour le même nom unique. Le permis est rendu au `Drop`.
#[derive(Debug)]
pub struct DialogGate {
    semaphore: tokio::sync::Semaphore,
    in_flight: Mutex<HashSet<String>>,
}

impl DialogGate {
    /// Crée un verrou de dialogue unique.
    #[must_use]
    pub fn new() -> Self {
        Self {
            semaphore: tokio::sync::Semaphore::new(1),
            in_flight: Mutex::new(HashSet::new()),
        }
    }

    /// Acquiert le droit d'ouvrir un dialogue pour `caller`.
    ///
    /// Attend au plus `wait` avant de renvoyer
    /// [`DialogGateError::GlobalBusy`].
    ///
    /// # Errors
    ///
    /// Voir [`DialogGateError`].
    pub async fn acquire(
        &self,
        caller: &str,
        wait: Duration,
    ) -> Result<DialogPermit<'_>, DialogGateError> {
        // L'entrée est protégée par un garde RAII dès l'insertion : une
        // annulation du future (déconnexion, arrêt, timeout externe) la retire
        // elle aussi. Sans ce garde, `in_flight` fuirait et l'appelant
        // resterait « occupé » jusqu'à la fin du processus (A1-02).
        let slot = self.reserve(caller)?;

        match tokio::time::timeout(wait, self.semaphore.acquire()).await {
            Ok(Ok(permit)) => Ok(DialogPermit {
                _slot: slot,
                _permit: permit,
            }),
            // Le `slot` est abandonné ici : son `Drop` retire l'appelant.
            Ok(Err(_)) | Err(_) => Err(DialogGateError::GlobalBusy),
        }
    }

    fn reserve<'a>(&'a self, caller: &str) -> Result<CallerSlot<'a>, DialogGateError> {
        let mut in_flight = match self.in_flight.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !in_flight.insert(caller.to_owned()) {
            return Err(DialogGateError::CallerBusy);
        }
        Ok(CallerSlot {
            gate: self,
            caller: caller.to_owned(),
        })
    }

    fn remove(&self, caller: &str) {
        let mut in_flight = match self.in_flight.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        in_flight.remove(caller);
    }
}

/// Entrée `in_flight` retirée au `Drop`, y compris en cas d'annulation.
#[derive(Debug)]
struct CallerSlot<'a> {
    gate: &'a DialogGate,
    caller: String,
}

impl Drop for CallerSlot<'_> {
    fn drop(&mut self) {
        self.gate.remove(&self.caller);
    }
}

impl Default for DialogGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Permis de dialogue rendu au `Drop`.
#[derive(Debug)]
pub struct DialogPermit<'a> {
    _slot: CallerSlot<'a>,
    _permit: tokio::sync::SemaphorePermit<'a>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::{
        AuthorizationLimiter, AuthorizationRequest, AuthorizationResult, AuthzError, DenialReason,
        DialogGate, DialogGateError, PolicyKitBackend, RawAuthorization, check_authorization,
        check_authorization_with,
    };

    #[derive(Debug, Default)]
    struct CapturingBackend {
        calls: Mutex<Vec<(String, String, bool)>>,
        response: RawAuthorization,
        unavailable: bool,
    }

    #[async_trait::async_trait]
    impl PolicyKitBackend for CapturingBackend {
        async fn check_authorization(
            &self,
            caller_bus_name: &str,
            action_id: &str,
            allow_interaction: bool,
        ) -> Result<RawAuthorization, AuthzError> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((
                    caller_bus_name.to_owned(),
                    action_id.to_owned(),
                    allow_interaction,
                ));
            }
            if self.unavailable {
                Err(AuthzError::Unavailable("service absent".to_owned()))
            } else {
                Ok(self.response)
            }
        }
    }

    fn request() -> AuthorizationRequest {
        AuthorizationRequest::new(":1.42", "com.tuxedocomputers.tccd1.profiles")
            .with_caller_uid(1000)
    }

    #[test]
    fn valid_request_is_accepted() {
        let request = request();
        assert!(request.validate().is_ok());
        assert!(request.allow_interaction);
    }

    #[test]
    fn well_known_names_are_rejected() {
        let mut request = request();
        request.caller_bus_name = "com.example.Service".to_owned();
        assert!(request.validate().is_err());
        request.caller_bus_name = "".to_owned();
        assert!(request.validate().is_err());
        request.caller_bus_name = ":1.42/evil".to_owned();
        assert!(request.validate().is_err());
    }

    #[test]
    fn timeouts_are_bounded() {
        let mut request = request();
        request.timeout = Duration::ZERO;
        assert!(request.validate().is_err());
        request.timeout = super::MAX_AUTHORIZATION_TIMEOUT + Duration::from_secs(1);
        assert!(request.validate().is_err());
    }

    #[test]
    fn challenge_is_never_authorized() {
        assert_eq!(
            RawAuthorization::new(true, true).decision(),
            AuthorizationResult::Denied(DenialReason::ChallengeCancelled)
        );
        assert_eq!(
            RawAuthorization::new(false, false).decision(),
            AuthorizationResult::Denied(DenialReason::PolicyDenied)
        );
        assert_eq!(
            RawAuthorization::new(true, false).decision(),
            AuthorizationResult::Authorized
        );
        assert_eq!(
            RawAuthorization::new(false, true).decision(),
            AuthorizationResult::Denied(DenialReason::ChallengeCancelled)
        );
    }

    #[test]
    fn subject_is_system_bus_name_and_never_unix_process() {
        let (kind, properties) = super::system_bus_subject(":1.42");
        assert_eq!(kind, "system-bus-name");
        assert!(!kind.contains("unix-process"));
        assert_eq!(properties.len(), 1);
        let value = properties.get("name").expect("propriété name");
        assert_eq!(value, &zbus::zvariant::Value::from(":1.42"));
    }

    #[tokio::test]
    async fn root_is_not_short_circuited() {
        // AD-3 : même root passe par CheckAuthorization. Un refus polkit
        // s'applique, et le backend est bien appelé.
        let backend = CapturingBackend {
            response: RawAuthorization::new(false, false),
            ..CapturingBackend::default()
        };
        let request = AuthorizationRequest::new(":1.42", "com.tuxedocomputers.tccd1.profiles")
            .with_caller_uid(0);
        assert_eq!(
            check_authorization_with(&request, &backend)
                .await
                .expect("réponse"),
            AuthorizationResult::Denied(DenialReason::PolicyDenied)
        );
        assert_eq!(backend.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn subject_and_flags_are_forwarded_without_cache() {
        let backend = CapturingBackend {
            response: RawAuthorization::new(true, false),
            ..CapturingBackend::default()
        };
        let request = request();
        for _ in 0..2 {
            let result = check_authorization_with(&request, &backend)
                .await
                .expect("autorisation");
            assert_eq!(result, AuthorizationResult::Authorized);
        }
        let calls = backend.calls.lock().expect("calls");
        assert_eq!(
            calls.len(),
            2,
            "aucun cache : chaque appel interroge polkit"
        );
        assert_eq!(calls[0].0, ":1.42");
        assert_eq!(calls[0].1, "com.tuxedocomputers.tccd1.profiles");
        assert!(calls[0].2, "flags AllowUserInteraction attendu");
    }

    #[tokio::test]
    async fn non_interactive_request_sets_flags_to_zero() {
        let backend = CapturingBackend {
            response: RawAuthorization::new(true, false),
            ..CapturingBackend::default()
        };
        let request = request().with_interaction(false);
        assert!(check_authorization_with(&request, &backend).await.is_ok());
        let calls = backend.calls.lock().expect("calls");
        assert!(!calls[0].2);
    }

    #[tokio::test]
    async fn denial_and_challenge_are_refused() {
        let denied = CapturingBackend {
            response: RawAuthorization::new(false, false),
            ..CapturingBackend::default()
        };
        assert_eq!(
            check_authorization_with(&request(), &denied)
                .await
                .expect("réponse"),
            AuthorizationResult::Denied(DenialReason::PolicyDenied)
        );

        let challenge = CapturingBackend {
            response: RawAuthorization::new(false, true),
            ..CapturingBackend::default()
        };
        assert_eq!(
            check_authorization_with(&request(), &challenge)
                .await
                .expect("réponse"),
            AuthorizationResult::Denied(DenialReason::ChallengeCancelled)
        );
    }

    #[tokio::test]
    async fn missing_polkit_is_fail_closed() {
        let backend = CapturingBackend {
            unavailable: true,
            ..CapturingBackend::default()
        };
        assert_eq!(
            check_authorization_with(&request(), &backend)
                .await
                .expect("réponse"),
            AuthorizationResult::Denied(DenialReason::PolicyKitUnavailable)
        );
    }

    #[tokio::test]
    async fn timeout_is_fail_closed() {
        struct NeverBackend;

        #[async_trait::async_trait]
        impl PolicyKitBackend for NeverBackend {
            async fn check_authorization(
                &self,
                _caller_bus_name: &str,
                _action_id: &str,
                _allow_interaction: bool,
            ) -> Result<RawAuthorization, AuthzError> {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(RawAuthorization::new(true, false))
            }
        }

        let request = request().with_timeout(super::MIN_AUTHORIZATION_TIMEOUT);
        assert_eq!(
            check_authorization_with(&request, &NeverBackend)
                .await
                .expect("réponse"),
            AuthorizationResult::Denied(DenialReason::Timeout)
        );
    }

    #[tokio::test]
    async fn unknown_caller_uid_is_rejected_before_polkit() {
        let backend = CapturingBackend {
            response: RawAuthorization::new(true, false),
            ..CapturingBackend::default()
        };
        let request = AuthorizationRequest::new(":1.42", "com.example.action");
        assert_eq!(
            check_authorization_with(&request, &backend).await,
            Err(AuthzError::UnknownCaller)
        );
        assert!(backend.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn production_entry_point_is_fail_closed_without_system_polkit() {
        // Sans bus système/polkit, l'appel ne doit jamais autoriser. Sur un
        // système avec polkit, la requête est structurellement valide mais
        // l'UID fourni arbitrairement ne correspond pas au nom unique : polkit
        // refuse. Le test n'exige donc aucun privilège.
        let result = check_authorization(&request()).await;
        if let Ok(decision) = result {
            assert!(!decision.is_authorized());
        }
    }

    #[test]
    fn limiter_enforces_rate_and_uid_budgets() {
        let limiter = AuthorizationLimiter::new(2.0, 4.0);
        let start = std::time::Instant::now();
        for _ in 0..4 {
            assert!(limiter.allow(":1.42", 1000, start));
        }
        assert!(!limiter.allow(":1.42", 1000, start));
        // 500 ms = 1 jeton à 2/s.
        assert!(limiter.allow(":1.42", 1000, start + Duration::from_millis(500)));
        // Un autre nom avec le même UID est limité par le bucket UID.
        assert!(!limiter.allow(":1.43", 1000, start + Duration::from_millis(500)));
        // Un autre UID dispose de son propre bucket.
        assert!(limiter.allow(":1.44", 1001, start));
    }

    #[tokio::test]
    async fn dialog_gate_refuses_same_caller_and_serializes_others() {
        let gate = DialogGate::new();
        let first = gate
            .acquire(":1.42", Duration::from_millis(10))
            .await
            .expect("premier dialogue");
        assert!(matches!(
            gate.acquire(":1.42", Duration::from_millis(10)).await,
            Err(DialogGateError::CallerBusy)
        ));
        assert!(matches!(
            gate.acquire(":1.43", Duration::from_millis(10)).await,
            Err(DialogGateError::GlobalBusy)
        ));
        drop(first);
        assert!(
            gate.acquire(":1.43", Duration::from_millis(10))
                .await
                .is_ok(),
            "le permis doit être rendu au Drop"
        );
    }

    #[tokio::test]
    async fn cancelled_dialog_releases_the_caller_slot() {
        let gate = DialogGate::new();
        let holder = gate
            .acquire(":1.holder", Duration::from_millis(10))
            .await
            .expect("détenteur");

        let mut pending = Box::pin(gate.acquire(":1.42", Duration::from_secs(60)));
        tokio::select! {
            biased;
            _ = &mut pending => panic!("le dialogue ne doit pas aboutir"),
            _ = tokio::task::yield_now() => {}
        }
        // L'appelant est bien marqué occupé pendant l'attente.
        assert!(matches!(
            gate.acquire(":1.42", Duration::from_millis(10)).await,
            Err(DialogGateError::CallerBusy)
        ));

        // Annulation du future en attente : le garde RAII doit retirer
        // l'entrée. Sans cela, l'appel suivant resterait `CallerBusy`.
        drop(pending);
        drop(holder);
        assert!(
            gate.acquire(":1.42", Duration::from_millis(10))
                .await
                .is_ok(),
            "l'annulation doit libérer l'entrée in_flight"
        );
    }

    #[test]
    fn limiter_map_stays_bounded() {
        let limiter = AuthorizationLimiter::new(2.0, 4.0);
        let start = std::time::Instant::now();
        for index in 0..(super::MAX_LIMITER_ENTRIES + 32) {
            let _ = limiter.allow(&format!(":1.{index}"), index as u32, start);
        }
        let names = limiter.names.lock().expect("names");
        assert!(names.len() <= super::MAX_LIMITER_ENTRIES);
        drop(names);
        let uids = limiter.uids.lock().expect("uids");
        assert!(uids.len() <= super::MAX_LIMITER_ENTRIES);
    }

    #[test]
    fn action_validation_is_strict() {
        assert!(super::validate_action_id("com.tuxedocomputers.tccd1.profiles").is_ok());
        assert!(super::validate_action_id("profiles").is_err());
        assert!(super::validate_action_id("com.example action").is_err());
    }

    #[test]
    fn denial_reasons_are_never_authorized() {
        let _ = HashMap::<String, String>::new();
        for reason in [
            DenialReason::PolicyDenied,
            DenialReason::ChallengeCancelled,
            DenialReason::Timeout,
            DenialReason::PolicyKitUnavailable,
            DenialReason::UnknownCaller,
            DenialReason::DialogBusy,
        ] {
            assert!(!AuthorizationResult::Denied(reason).is_authorized());
        }
    }
}
