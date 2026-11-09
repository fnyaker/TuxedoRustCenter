// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Le moteur de résolution d'état
// est une réimplémentation originale de la logique `StateSwitcherWorker`
// (GPL-3.0-or-later), étendue à power-profiles-daemon.

//! Résolution de l'état `(PPD, alimentation)`.
//!
//! Le moteur est **pur** : il ne lit ni D-Bus, ni sysfs, ni horloge système.
//! L'appelant fournit les entrées et un temps monotone sous forme de
//! [`Duration`]. Cette séparation permet de tester le debounce, les
//! changements rapprochés et les overrides sans temporisation réelle, et
//! laisse `tcc-daemon` seul responsable des watchers PPD/UPower.
//!
//! # Ordre de résolution
//!
//! 1. override manuel, s'il est encore valide pour la clé courante ;
//! 2. matrice `(PPD, alimentation)` du [`StateMatrix`] ;
//! 3. `stateMap` historique `power_ac` / `power_bat`, uniquement si
//!    l'alimentation est connue (AC ou batterie) ;
//! 4. profil par défaut.
//!
//! Si un identifiant de la matrice ou du `stateMap` ne correspond à aucun
//! profil connu (voir [`StateResolver::set_known_profiles`]), la résolution
//! continue vers l'étape suivante au lieu de renvoyer un profil inexistant.
//!
//! # Debounce et override
//!
//! Un changement de clé est confirmé après [`StateResolver::debounce`] (750 ms
//! par défaut). Tant que la clé n'est pas confirmée, l'état actif ne change
//! pas. Un retour à la clé courante pendant la fenêtre annule la transition.
//! L'override manuel est invalidé **au moment où la nouvelle clé est
//! confirmée**, pas pendant la fenêtre de debounce : un rebond transitoire de
//! PPD ne doit pas effacer le choix explicite de l'utilisateur. Ce choix est
//! documenté et testé.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use crate::profile::{
    DEFAULT_CUSTOM_PROFILE_ID, HIGH_PERFORMANCE_PROFILE_ID, MAX_ENERGY_SAVE_PROFILE_ID,
    OFFICE_PROFILE_ID, ProfileId, QUIET_PROFILE_ID,
};

/// Debounce par défaut entre un changement d'entrée et l'application.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(750);

/// Source d'alimentation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    /// Secteur connecté.
    #[serde(alias = "AC", alias = "power_ac", alias = "mains")]
    Ac,
    /// Batterie.
    #[serde(alias = "BAT", alias = "power_bat")]
    Battery,
    /// État inconnu (pas de batterie détectée, lecture en échec, …).
    #[default]
    Unknown,
}

impl PowerSource {
    /// Représentation canonique (`ac`, `battery`, `unknown`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ac => "ac",
            Self::Battery => "battery",
            Self::Unknown => "unknown",
        }
    }

    /// Vrai si la source est connue et peut utiliser le `stateMap` historique.
    #[must_use]
    pub fn is_known(self) -> bool {
        matches!(self, Self::Ac | Self::Battery)
    }

    /// Déduit la source de l'état `online` d'un adaptateur secteur.
    ///
    /// Conversion directe depuis `tcc_hw::power_supply::PowerState::Ac` /
    /// `Battery` ou depuis l'attribut sysfs `online` d'une alimentation
    /// `Mains`. `None` (attribut absent ou lecture impossible) donne
    /// [`PowerSource::Unknown`] et non un défaut AC silencieux : l'original
    /// supposait AC, mais un état inconnu ne doit pas appliquer un profil
    /// secteur sur batterie.
    #[must_use]
    pub fn from_ac_online(online: Option<bool>) -> Self {
        match online {
            Some(true) => Self::Ac,
            Some(false) => Self::Battery,
            None => Self::Unknown,
        }
    }

    /// Déduit la source de la propriété UPower `OnBattery`.
    ///
    /// `None` (UPower absent) donne [`PowerSource::Unknown`] : le daemon
    /// bascule alors sur la lecture sysfs ou le profil par défaut.
    #[must_use]
    pub fn from_on_battery(on_battery: Option<bool>) -> Self {
        match on_battery {
            Some(true) => Self::Battery,
            Some(false) => Self::Ac,
            None => Self::Unknown,
        }
    }
}

impl fmt::Display for PowerSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Profil power-profiles-daemon.
///
/// Les valeurs inconnues sont conservées ([`PpdProfile::Unknown`]) pour les
/// logs et l'UI. Elles ne participent pas à la matrice, ce qui déclenche le
/// fallback AC/batterie.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PpdProfile {
    /// `power-saver`.
    PowerSaver,
    /// `balanced`.
    Balanced,
    /// `performance`.
    Performance,
    /// Toute autre valeur retournée par PPD.
    Unknown(String),
}

impl PpdProfile {
    /// Convertit la valeur D-Bus de PPD.
    #[must_use]
    pub fn from_dbus(value: &str) -> Self {
        match value {
            "power-saver" => Self::PowerSaver,
            "balanced" => Self::Balanced,
            "performance" => Self::Performance,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// Convertit une valeur optionnelle de PPD (propriété absente).
    #[must_use]
    pub fn from_optional_dbus(value: Option<&str>) -> Option<Self> {
        value.map(Self::from_dbus)
    }

    /// Représentation canonique, y compris pour une valeur inconnue.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::PowerSaver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::Unknown(value) => value,
        }
    }

    /// Jeton utilisable comme clé de matrice.
    ///
    /// `None` pour une valeur inconnue : la matrice ne peut pas représenter
    /// une valeur arbitraire de PPD, et la résolution doit alors utiliser le
    /// fallback AC/batterie.
    #[must_use]
    pub fn matrix_token(&self) -> Option<&'static str> {
        match self {
            Self::PowerSaver => Some("power-saver"),
            Self::Balanced => Some("balanced"),
            Self::Performance => Some("performance"),
            Self::Unknown(_) => None,
        }
    }

    /// Vrai si la valeur correspond à un profil PPD standard.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl fmt::Display for PpdProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for PpdProfile {
    fn from(value: &str) -> Self {
        Self::from_dbus(value)
    }
}

impl From<String> for PpdProfile {
    fn from(value: String) -> Self {
        Self::from_dbus(&value)
    }
}

impl Serialize for PpdProfile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PpdProfile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_dbus(&value))
    }
}

/// Clé d'état : profil PPD (optionnel) et alimentation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateKey {
    /// Profil PPD, ou `None` si PPD est absent.
    pub ppd: Option<PpdProfile>,
    /// Source d'alimentation.
    pub power: PowerSource,
}

impl StateKey {
    /// Construit une clé d'état.
    #[must_use]
    pub fn new(ppd: Option<PpdProfile>, power: PowerSource) -> Self {
        Self { ppd, power }
    }

    /// Clé de matrice canonique, par exemple `balanced/ac`.
    ///
    /// Renvoie `None` si le profil PPD est inconnu ; une telle clé ne peut pas
    /// être stockée dans la matrice.
    #[must_use]
    pub fn matrix_key(&self) -> Option<String> {
        let ppd = match &self.ppd {
            None => "none",
            Some(profile) => profile.matrix_token()?,
        };
        Some(format!("{ppd}/{}", self.power.as_str()))
    }

    /// Analyse une clé de matrice canonique.
    ///
    /// Accepte aussi `power-saver`, `powersaver`, `balanced`, `performance`,
    /// `none` côté PPD, et `ac`, `battery`, `unknown` côté alimentation.
    #[must_use]
    pub fn from_matrix_key(value: &str) -> Option<Self> {
        let (raw_ppd, raw_power) = value.split_once('/')?;
        let ppd = match raw_ppd {
            "none" | "" => None,
            "power-saver" | "power_saver" | "powersaver" => Some(PpdProfile::PowerSaver),
            "balanced" => Some(PpdProfile::Balanced),
            "performance" => Some(PpdProfile::Performance),
            _ => return None,
        };
        let power = match raw_power {
            "ac" | "AC" => PowerSource::Ac,
            "battery" | "bat" => PowerSource::Battery,
            "unknown" => PowerSource::Unknown,
            _ => return None,
        };
        Some(Self { ppd, power })
    }
}

/// Matrice `(PPD, alimentation) → profil`.
///
/// Sérialisée comme un objet JSON `{ "balanced/ac": "__office__", … }`, ce qui
/// reste lisible et éditable dans la GUI. Les clés non canoniques sont
/// rejetées au chargement ; le `ConfigStore` sauvegarde alors le fichier en
/// `.bak` et repart des valeurs par défaut.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateMatrix {
    entries: std::collections::BTreeMap<StateKey, ProfileId>,
}

impl StateMatrix {
    /// Matrice vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Matrice proposée par défaut par l'architecture.
    ///
    /// Les identifiants référencés sont les presets fournis par
    /// [`crate::presets::default_profiles`]. Le daemon peut donc l'utiliser
    /// directement une fois les profils par défaut fusionnés avec ceux
    /// chargés. Une entrée manquante retombe proprement sur `stateMap` puis
    /// sur le profil par défaut.
    #[must_use]
    pub fn suggested_default() -> Self {
        let mut matrix = Self::new();
        for (ppd, power, profile) in [
            (
                PpdProfile::PowerSaver,
                PowerSource::Ac,
                MAX_ENERGY_SAVE_PROFILE_ID,
            ),
            (
                PpdProfile::PowerSaver,
                PowerSource::Battery,
                MAX_ENERGY_SAVE_PROFILE_ID,
            ),
            (PpdProfile::Balanced, PowerSource::Ac, OFFICE_PROFILE_ID),
            (PpdProfile::Balanced, PowerSource::Battery, QUIET_PROFILE_ID),
            (
                PpdProfile::Performance,
                PowerSource::Ac,
                HIGH_PERFORMANCE_PROFILE_ID,
            ),
            (
                PpdProfile::Performance,
                PowerSource::Battery,
                OFFICE_PROFILE_ID,
            ),
        ] {
            matrix.insert(StateKey::new(Some(ppd), power), ProfileId::new(profile));
        }
        matrix
    }

    /// Insère une entrée.
    ///
    /// Renvoie `false` si la clé n'est pas représentable (PPD inconnu) ;
    /// l'entrée n'est alors pas modifiée.
    pub fn insert(&mut self, key: StateKey, profile_id: ProfileId) -> bool {
        if key.matrix_key().is_none() {
            return false;
        }
        self.entries.insert(key, profile_id);
        true
    }

    /// Retire une entrée.
    pub fn remove(&mut self, key: &StateKey) -> Option<ProfileId> {
        self.entries.remove(key)
    }

    /// Cherche le profil associé à une clé.
    #[must_use]
    pub fn get(&self, key: &StateKey) -> Option<&ProfileId> {
        self.entries.get(key)
    }

    /// Vrai si la matrice contient cette clé.
    #[must_use]
    pub fn contains_key(&self, key: &StateKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Itère sur les entrées.
    pub fn iter(&self) -> impl Iterator<Item = (&StateKey, &ProfileId)> {
        self.entries.iter()
    }

    /// Nombre d'entrées.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Vrai si la matrice est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Serialize for StateMatrix {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (key, profile_id) in &self.entries {
            let token = key
                .matrix_key()
                .ok_or_else(|| serde::ser::Error::custom("clé de matrice non représentable"))?;
            map.serialize_entry(&token, profile_id)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StateMatrix {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MatrixVisitor;

        impl<'de> Visitor<'de> for MatrixVisitor {
            type Value = StateMatrix;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("un objet JSON de clés « ppd/alimentation »")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = std::collections::BTreeMap::new();
                while let Some((raw_key, profile_id)) = map.next_entry::<String, ProfileId>()? {
                    let key = StateKey::from_matrix_key(&raw_key).ok_or_else(|| {
                        de::Error::custom(format!(
                            "clé de matrice invalide: {raw_key} (attendu « ppd/alimentation »)"
                        ))
                    })?;
                    entries.insert(key, profile_id);
                }
                Ok(StateMatrix { entries })
            }
        }

        deserializer.deserialize_map(MatrixVisitor)
    }
}

/// `stateMap` historique de l'original (`power_ac` / `power_bat`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LegacyStateMap {
    /// Profil à activer sur secteur.
    #[serde(default = "default_custom_profile")]
    pub power_ac: ProfileId,
    /// Profil à activer sur batterie.
    #[serde(default = "default_custom_profile")]
    pub power_bat: ProfileId,
}

fn default_custom_profile() -> ProfileId {
    ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID)
}

impl Default for LegacyStateMap {
    fn default() -> Self {
        Self {
            power_ac: default_custom_profile(),
            power_bat: default_custom_profile(),
        }
    }
}

impl LegacyStateMap {
    /// Profil historique pour une source connue.
    #[must_use]
    pub fn get(&self, power: PowerSource) -> Option<&ProfileId> {
        match power {
            PowerSource::Ac => Some(&self.power_ac),
            PowerSource::Battery => Some(&self.power_bat),
            PowerSource::Unknown => None,
        }
    }

    /// Affecte un profil à une source connue.
    ///
    /// Renvoie `false` pour [`PowerSource::Unknown`], qui n'a pas d'entrée
    /// historique.
    pub fn set(&mut self, power: PowerSource, profile_id: ProfileId) -> bool {
        match power {
            PowerSource::Ac => {
                self.power_ac = profile_id;
                true
            }
            PowerSource::Battery => {
                self.power_bat = profile_id;
                true
            }
            PowerSource::Unknown => false,
        }
    }
}

/// Entrées du moteur de résolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInput {
    /// Profil PPD courant, `None` si PPD est absent.
    pub ppd: Option<PpdProfile>,
    /// Source d'alimentation courante.
    pub power: PowerSource,
}

impl StateInput {
    /// Construit une entrée.
    #[must_use]
    pub fn new(ppd: Option<PpdProfile>, power: PowerSource) -> Self {
        Self { ppd, power }
    }

    /// Clé d'état correspondante.
    #[must_use]
    pub fn key(&self) -> StateKey {
        StateKey::new(self.ppd.clone(), self.power)
    }
}

/// Origine du profil sélectionné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionSource {
    /// Override manuel temporaire (tray ou GUI).
    ManualOverride,
    /// Matrice `(PPD, alimentation)`.
    StateMatrix,
    /// `stateMap` historique AC/batterie.
    LegacyStateMap,
    /// Profil par défaut.
    #[default]
    DefaultProfile,
}

/// Override manuel temporaire.
///
/// L'override reste valide tant que la clé d'état n'a pas changé. Il n'est
/// jamais persisté : un redémarrage du daemon repart de la matrice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualOverride {
    /// Profil choisi manuellement.
    pub profile_id: ProfileId,
    /// Clé d'état pour laquelle le choix a été fait.
    pub key: StateKey,
}

impl ManualOverride {
    /// Vrai si l'override s'applique encore à cette clé.
    #[must_use]
    pub fn is_valid_for(&self, key: &StateKey) -> bool {
        self.key == *key
    }
}

/// État résolu, avec sa provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedState {
    /// Clé d'état ayant produit cette résolution.
    pub key: StateKey,
    /// Profil à appliquer.
    pub profile_id: ProfileId,
    /// Origine du choix.
    pub source: ResolutionSource,
}

impl ResolvedState {
    /// Vrai si le profil vient d'un override manuel.
    #[must_use]
    pub fn is_manual_override(&self) -> bool {
        self.source == ResolutionSource::ManualOverride
    }
}

/// Résultat d'une observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserveOutcome {
    /// Le profil actif change (ou doit être réappliqué).
    Changed {
        /// État précédent, `None` au premier démarrage.
        previous: Option<ResolvedState>,
        /// Nouvel état à appliquer.
        current: ResolvedState,
    },
    /// Rien à appliquer : le profil actif reste le même.
    Unchanged(ResolvedState),
    /// Changement détecté mais encore dans la fenêtre de debounce.
    Pending {
        /// État actuellement actif.
        current: Option<ResolvedState>,
        /// Clé candidate qui sera confirmée après le debounce.
        candidate: StateKey,
    },
}

impl ObserveOutcome {
    /// État actif associé au résultat.
    #[must_use]
    pub fn current(&self) -> Option<&ResolvedState> {
        match self {
            Self::Changed { current, .. } | Self::Unchanged(current) => Some(current),
            Self::Pending { current, .. } => current.as_ref(),
        }
    }

    /// Vrai si une transition est en attente de debounce.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }
}

/// État interne d'une clé en attente de confirmation.
#[derive(Debug, Clone)]
struct PendingKey {
    key: StateKey,
    since: Duration,
}

/// Moteur de résolution d'état.
#[derive(Debug, Clone)]
pub struct StateResolver {
    matrix: StateMatrix,
    legacy_state_map: LegacyStateMap,
    default_profile: ProfileId,
    known_profiles: Option<BTreeSet<ProfileId>>,
    debounce: Duration,
    current: Option<ResolvedState>,
    pending: Option<PendingKey>,
    manual_override: Option<ManualOverride>,
    force_reapply: bool,
}

impl StateResolver {
    /// Crée un moteur avec le debounce par défaut.
    #[must_use]
    pub fn new(
        matrix: StateMatrix,
        legacy_state_map: LegacyStateMap,
        default_profile: ProfileId,
    ) -> Self {
        Self {
            matrix,
            legacy_state_map,
            default_profile,
            known_profiles: None,
            debounce: DEFAULT_DEBOUNCE,
            current: None,
            pending: None,
            manual_override: None,
            force_reapply: false,
        }
    }

    /// Change la durée de debounce.
    #[must_use]
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// Durée de debounce courante.
    #[must_use]
    pub fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Définit la durée de debounce.
    pub fn set_debounce(&mut self, debounce: Duration) {
        self.debounce = debounce;
    }

    /// Matrice courante.
    #[must_use]
    pub fn matrix(&self) -> &StateMatrix {
        &self.matrix
    }

    /// Remplace la matrice (rechargement de configuration).
    ///
    /// L'appelant doit ensuite appeler [`StateResolver::force_reapply`] s'il
    /// veut réappliquer le profil courant avec les nouvelles règles.
    pub fn set_matrix(&mut self, matrix: StateMatrix) {
        self.matrix = matrix;
    }

    /// `stateMap` historique courant.
    #[must_use]
    pub fn legacy_state_map(&self) -> &LegacyStateMap {
        &self.legacy_state_map
    }

    /// Remplace le `stateMap` historique.
    pub fn set_legacy_state_map(&mut self, state_map: LegacyStateMap) {
        self.legacy_state_map = state_map;
    }

    /// Profil par défaut courant.
    #[must_use]
    pub fn default_profile(&self) -> &ProfileId {
        &self.default_profile
    }

    /// Remplace le profil par défaut.
    pub fn set_default_profile(&mut self, profile_id: ProfileId) {
        self.default_profile = profile_id;
    }

    /// Restreint la résolution aux profils réellement présents.
    ///
    /// `None` désactive le filtrage (utile aux tests et aux clients qui n'ont
    /// pas la liste des profils). Le daemon doit fournir l'ensemble chargé
    /// pour éviter d'appliquer un identifiant supprimé.
    ///
    /// La matrice et le `stateMap` sont filtrés ; le profil par défaut ne
    /// l'est pas, car il doit toujours exister (le daemon le crée au besoin).
    pub fn set_known_profiles(&mut self, known: Option<BTreeSet<ProfileId>>) {
        self.known_profiles = known;
    }

    /// État actuellement actif.
    #[must_use]
    pub fn current(&self) -> Option<&ResolvedState> {
        self.current.as_ref()
    }

    /// Clé en attente de debounce, si elle existe.
    #[must_use]
    pub fn pending_key(&self) -> Option<&StateKey> {
        self.pending.as_ref().map(|pending| &pending.key)
    }

    /// Override manuel courant, même s'il n'est plus valide.
    #[must_use]
    pub fn manual_override(&self) -> Option<&ManualOverride> {
        self.manual_override.as_ref()
    }

    /// Demande une réapplication du profil courant au prochain `observe`.
    ///
    /// Utilisé après un rechargement de configuration ou un `SIGHUP` : sans
    /// changement de clé, `observe` renvoie normalement `Unchanged`, ce qui ne
    /// déclencherait pas la réapplication des réglages matériels.
    pub fn force_reapply(&mut self) {
        self.force_reapply = true;
    }

    /// Définit un override manuel pour la clé active.
    ///
    /// Renvoie `false` si aucun état n'a encore été observé : l'appelant doit
    /// d'abord appeler [`StateResolver::observe`] au démarrage.
    pub fn set_manual_override(&mut self, profile_id: ProfileId) -> bool {
        let Some(current) = self.current.clone() else {
            return false;
        };
        self.manual_override = Some(ManualOverride {
            profile_id: profile_id.clone(),
            key: current.key.clone(),
        });
        self.current = Some(ResolvedState {
            key: current.key,
            profile_id,
            source: ResolutionSource::ManualOverride,
        });
        true
    }

    /// Supprime l'override manuel et recalcule l'état courant.
    ///
    /// Renvoie le nouvel état s'il y avait effectivement un override.
    pub fn clear_manual_override(&mut self) -> Option<ResolvedState> {
        let key = self.current.as_ref()?.key.clone();
        self.manual_override.take()?;
        let resolved = self.resolve_for_key(&key);
        self.current = Some(resolved.clone());
        Some(resolved)
    }

    /// Observe un nouvel état et renvoie l'action à effectuer.
    ///
    /// `now` doit être monotone (par exemple `Instant::elapsed` depuis le
    /// démarrage du daemon). Le premier appel est appliqué immédiatement afin
    /// de ne pas retarder le profil au boot.
    pub fn observe(&mut self, input: StateInput, now: Duration) -> ObserveOutcome {
        let key = input.key();
        let force = std::mem::take(&mut self.force_reapply);

        if let Some(pending) = self.pending.clone() {
            if pending.key == key {
                if force || now.saturating_sub(pending.since) >= self.debounce {
                    self.pending = None;
                    return self.commit(key, force);
                }
                return ObserveOutcome::Pending {
                    current: self.current.clone(),
                    candidate: key,
                };
            }

            if self
                .current
                .as_ref()
                .is_some_and(|current| current.key == key)
            {
                self.pending = None;
                if force {
                    return self.commit(key, true);
                }
                if let Some(current) = self.current.clone() {
                    return ObserveOutcome::Unchanged(current);
                }
            }

            self.pending = Some(PendingKey {
                key: key.clone(),
                since: now,
            });
            return ObserveOutcome::Pending {
                current: self.current.clone(),
                candidate: key,
            };
        }

        match &self.current {
            None => self.commit(key, true),
            Some(current) if current.key == key => {
                if force {
                    self.commit(key, true)
                } else {
                    ObserveOutcome::Unchanged(current.clone())
                }
            }
            Some(_) => {
                self.pending = Some(PendingKey {
                    key: key.clone(),
                    since: now,
                });
                ObserveOutcome::Pending {
                    current: self.current.clone(),
                    candidate: key,
                }
            }
        }
    }

    /// Résout une clé sans tenir compte du debounce.
    ///
    /// Utilisé par [`StateResolver::observe`] et par les clients qui veulent
    /// prévisualiser l'effet d'un état (GUI, tests).
    #[must_use]
    pub fn resolve_for_key(&self, key: &StateKey) -> ResolvedState {
        if let Some(override_) = &self.manual_override
            && override_.is_valid_for(key)
        {
            return ResolvedState {
                key: key.clone(),
                profile_id: override_.profile_id.clone(),
                source: ResolutionSource::ManualOverride,
            };
        }

        if let Some(profile_id) = self.matrix.get(key)
            && self.is_known(profile_id)
        {
            return ResolvedState {
                key: key.clone(),
                profile_id: profile_id.clone(),
                source: ResolutionSource::StateMatrix,
            };
        }

        if let Some(profile_id) = self.legacy_state_map.get(key.power)
            && self.is_known(profile_id)
        {
            return ResolvedState {
                key: key.clone(),
                profile_id: profile_id.clone(),
                source: ResolutionSource::LegacyStateMap,
            };
        }

        ResolvedState {
            key: key.clone(),
            profile_id: self.default_profile.clone(),
            source: ResolutionSource::DefaultProfile,
        }
    }

    fn is_known(&self, profile_id: &ProfileId) -> bool {
        self.known_profiles
            .as_ref()
            .is_none_or(|known| known.contains(profile_id))
    }

    fn commit(&mut self, key: StateKey, force: bool) -> ObserveOutcome {
        let key_changed = self
            .current
            .as_ref()
            .is_none_or(|current| current.key != key);
        if key_changed {
            // Un changement d'état confirmé invalide le choix temporaire,
            // comme `StateSwitcherWorker` le fait dans l'original.
            self.manual_override = None;
        }

        let resolved = self.resolve_for_key(&key);
        let previous = self.current.replace(resolved.clone());
        match previous {
            Some(previous) if !force && previous == resolved => ObserveOutcome::Unchanged(previous),
            previous => ObserveOutcome::Changed {
                previous,
                current: resolved,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use super::{
        LegacyStateMap, ObserveOutcome, PowerSource, PpdProfile, ResolutionSource, StateInput,
        StateKey, StateMatrix, StateResolver,
    };
    use crate::profile::ProfileId;

    fn id(value: &str) -> ProfileId {
        ProfileId::new(value)
    }

    fn branch_resolver() -> StateResolver {
        let mut matrix = StateMatrix::new();
        matrix.insert(
            StateKey::new(Some(PpdProfile::Balanced), PowerSource::Ac),
            id("office"),
        );
        matrix.insert(
            StateKey::new(Some(PpdProfile::Balanced), PowerSource::Battery),
            id("quiet"),
        );
        matrix.insert(
            StateKey::new(Some(PpdProfile::Performance), PowerSource::Ac),
            id("high"),
        );
        matrix.insert(
            StateKey::new(Some(PpdProfile::Performance), PowerSource::Battery),
            id("office"),
        );

        let legacy = LegacyStateMap {
            power_ac: id("legacy-ac"),
            power_bat: id("legacy-bat"),
        };

        StateResolver::new(matrix, legacy, id("fallback"))
    }

    fn input(ppd: Option<PpdProfile>, power: PowerSource) -> StateInput {
        StateInput::new(ppd, power)
    }

    #[test]
    fn first_observation_is_applied_immediately() {
        let mut resolver = branch_resolver();
        let outcome = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        match outcome {
            ObserveOutcome::Changed { previous, current } => {
                assert!(previous.is_none());
                assert_eq!(current.profile_id, id("office"));
                assert_eq!(current.source, ResolutionSource::StateMatrix);
            }
            other => panic!("résultat inattendu: {other:?}"),
        }
        assert!(
            !resolver
                .observe(
                    input(Some(PpdProfile::Balanced), PowerSource::Ac),
                    Duration::from_millis(10),
                )
                .is_pending()
        );
    }

    #[test]
    fn matrix_is_used_for_known_ppd() {
        let mut resolver = branch_resolver();
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Battery),
            Duration::ZERO,
        );
        assert_eq!(resolver.current().unwrap().profile_id, id("office"));
        assert_eq!(
            resolver.current().unwrap().source,
            ResolutionSource::StateMatrix
        );
    }

    #[test]
    fn ppd_absent_falls_back_to_legacy_state_map() {
        let mut resolver = branch_resolver();
        resolver.observe(input(None, PowerSource::Ac), Duration::ZERO);
        let current = resolver.current().unwrap();
        assert_eq!(current.profile_id, id("legacy-ac"));
        assert_eq!(current.source, ResolutionSource::LegacyStateMap);
    }

    #[test]
    fn unknown_ppd_falls_back_to_legacy_state_map() {
        let mut resolver = branch_resolver();
        resolver.observe(
            input(
                Some(PpdProfile::Unknown("custom".into())),
                PowerSource::Battery,
            ),
            Duration::ZERO,
        );
        let current = resolver.current().unwrap();
        assert_eq!(current.profile_id, id("legacy-bat"));
        assert_eq!(current.source, ResolutionSource::LegacyStateMap);
    }

    #[test]
    fn unknown_power_uses_default_profile() {
        let mut resolver = branch_resolver();
        let outcome = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Unknown),
            Duration::ZERO,
        );
        match outcome {
            ObserveOutcome::Changed { current, .. } => {
                assert_eq!(current.profile_id, id("fallback"));
                assert_eq!(current.source, ResolutionSource::DefaultProfile);
            }
            other => panic!("résultat inattendu: {other:?}"),
        }
    }

    #[test]
    fn debounce_delays_key_change() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );

        let pending = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        assert!(pending.is_pending());
        assert_eq!(resolver.current().unwrap().profile_id, id("office"));

        let still_pending = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(400),
        );
        assert!(still_pending.is_pending());

        let changed = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(600),
        );
        match changed {
            ObserveOutcome::Changed { previous, current } => {
                assert_eq!(previous.unwrap().profile_id, id("office"));
                assert_eq!(current.profile_id, id("high"));
            }
            other => panic!("résultat inattendu: {other:?}"),
        }
    }

    #[test]
    fn debounce_boundary_is_inclusive() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        let changed = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(600),
        );
        assert!(matches!(changed, ObserveOutcome::Changed { .. }));
    }

    #[test]
    fn flapping_back_to_current_key_cancels_pending_change() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        let back = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::from_millis(200),
        );
        assert!(matches!(back, ObserveOutcome::Unchanged(_)));
        assert!(resolver.pending_key().is_none());
        assert_eq!(resolver.current().unwrap().profile_id, id("office"));
    }

    #[test]
    fn rapid_successive_changes_restart_debounce() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );

        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Battery),
            Duration::from_millis(300),
        );

        // 500 ms après le dernier changement, la clé batterie est confirmée.
        let outcome = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Battery),
            Duration::from_millis(800),
        );
        match outcome {
            ObserveOutcome::Changed { current, .. } => {
                assert_eq!(current.profile_id, id("quiet"));
                assert_eq!(current.key.power, PowerSource::Battery);
            }
            other => panic!("résultat inattendu: {other:?}"),
        }
    }

    #[test]
    fn manual_override_survives_until_key_change_is_confirmed() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        assert!(resolver.set_manual_override(id("manual")));
        assert_eq!(resolver.current().unwrap().profile_id, id("manual"));
        assert!(resolver.current().unwrap().is_manual_override());

        // Même clé : l'override reste actif.
        let unchanged = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::from_millis(50),
        );
        assert!(matches!(unchanged, ObserveOutcome::Unchanged(_)));
        assert_eq!(resolver.current().unwrap().profile_id, id("manual"));

        // Changement encore en debounce : l'override reste actif tant que la
        // nouvelle clé n'est pas confirmée.
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        assert_eq!(resolver.current().unwrap().profile_id, id("manual"));

        // Confirmation : l'override est invalidé et la matrice reprend.
        let outcome = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(700),
        );
        match outcome {
            ObserveOutcome::Changed { current, .. } => {
                assert_eq!(current.profile_id, id("high"));
                assert_eq!(current.source, ResolutionSource::StateMatrix);
            }
            other => panic!("résultat inattendu: {other:?}"),
        }
        assert!(resolver.manual_override().is_none());
    }

    #[test]
    fn transient_flap_keeps_manual_override() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.set_manual_override(id("manual"));
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::from_millis(200),
        );
        assert_eq!(resolver.current().unwrap().profile_id, id("manual"));
        assert!(resolver.manual_override().is_some());
    }

    #[test]
    fn manual_override_before_first_observation_is_refused() {
        let mut resolver = branch_resolver();
        assert!(!resolver.set_manual_override(id("manual")));
    }

    #[test]
    fn clear_manual_override_restores_matrix_profile() {
        let mut resolver = branch_resolver();
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.set_manual_override(id("manual"));
        let restored = resolver.clear_manual_override().unwrap();
        assert_eq!(restored.profile_id, id("office"));
        assert_eq!(restored.source, ResolutionSource::StateMatrix);
        assert!(resolver.clear_manual_override().is_none());
    }

    #[test]
    fn unknown_matrix_profile_falls_back_to_legacy() {
        let mut resolver = branch_resolver();
        resolver.set_known_profiles(Some(
            [id("legacy-ac"), id("legacy-bat"), id("fallback")]
                .into_iter()
                .collect::<BTreeSet<_>>(),
        ));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        assert_eq!(resolver.current().unwrap().profile_id, id("legacy-ac"));
        assert_eq!(
            resolver.current().unwrap().source,
            ResolutionSource::LegacyStateMap
        );
    }

    #[test]
    fn force_reapply_returns_changed_even_for_same_key() {
        let mut resolver = branch_resolver();
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.force_reapply();
        let outcome = resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::from_millis(1),
        );
        assert!(matches!(outcome, ObserveOutcome::Changed { .. }));
    }

    #[test]
    fn force_reapply_can_bypass_debounce_for_pending_key() {
        let mut resolver = branch_resolver().with_debounce(Duration::from_millis(500));
        resolver.observe(
            input(Some(PpdProfile::Balanced), PowerSource::Ac),
            Duration::ZERO,
        );
        resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(100),
        );
        resolver.force_reapply();
        let outcome = resolver.observe(
            input(Some(PpdProfile::Performance), PowerSource::Ac),
            Duration::from_millis(150),
        );
        assert!(matches!(outcome, ObserveOutcome::Changed { .. }));
    }

    #[test]
    fn matrix_serializes_with_canonical_keys() {
        let matrix = branch_resolver().matrix().clone();
        let json = serde_json::to_value(&matrix).unwrap();
        assert_eq!(json["balanced/ac"], "office");
        assert_eq!(json["performance/battery"], "office");

        let parsed: StateMatrix = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, matrix);
    }

    #[test]
    fn matrix_rejects_unknown_ppd_keys() {
        let key = StateKey::new(Some(PpdProfile::Unknown("weird".into())), PowerSource::Ac);
        let mut matrix = StateMatrix::new();
        assert!(!matrix.insert(key, id("x")));
        assert!(matrix.is_empty());
    }

    #[test]
    fn suggested_matrix_references_presets() {
        let matrix = StateMatrix::suggested_default();
        assert_eq!(matrix.len(), 6);
        assert_eq!(
            matrix
                .get(&StateKey::new(
                    Some(PpdProfile::PowerSaver),
                    PowerSource::Battery
                ))
                .unwrap()
                .as_str(),
            "__profile_max_energy_save__"
        );
    }

    #[test]
    fn pdpprofile_serde_preserves_unknown_values() {
        let parsed: PpdProfile = serde_json::from_str("\"custom-profile\"").unwrap();
        assert_eq!(parsed, PpdProfile::Unknown("custom-profile".into()));
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            "\"custom-profile\""
        );
    }

    #[test]
    fn powersource_serde_and_legacy_aliases() {
        let ac: PowerSource = serde_json::from_str("\"AC\"").unwrap();
        assert_eq!(ac, PowerSource::Ac);
        let bat: PowerSource = serde_json::from_str("\"power_bat\"").unwrap();
        assert_eq!(bat, PowerSource::Battery);
        assert_eq!(
            serde_json::to_string(&PowerSource::Unknown).unwrap(),
            "\"unknown\""
        );
    }

    #[test]
    fn powersource_conversions_match_upower_and_sysfs() {
        assert_eq!(PowerSource::from_ac_online(Some(true)), PowerSource::Ac);
        assert_eq!(
            PowerSource::from_ac_online(Some(false)),
            PowerSource::Battery
        );
        assert_eq!(PowerSource::from_ac_online(None), PowerSource::Unknown);
        assert_eq!(
            PowerSource::from_on_battery(Some(true)),
            PowerSource::Battery
        );
        assert_eq!(PowerSource::from_on_battery(Some(false)), PowerSource::Ac);
        assert_eq!(PowerSource::from_on_battery(None), PowerSource::Unknown);
    }

    #[test]
    fn state_matrix_invalid_key_is_rejected_on_load() {
        let result: Result<StateMatrix, _> = serde_json::from_str(r#"{"turbo/ac":"x"}"#);
        assert!(result.is_err());
    }
}
