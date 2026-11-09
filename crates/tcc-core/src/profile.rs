// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Le code est une
// réimplémentation originale des modèles de profils de TUXEDO Control
// Center (GPL-3.0-or-later). Les noms de champs et les valeurs par défaut
// sont conservés pour assurer la migration depuis `/etc/tcc/profiles`.

//! Modèles de profils matériels.
//!
//! Les noms JSON sont ceux de l'original (`useBrightness`, `refreshRate`,
//! `scalingMinFrequency`, `cTGPOffset`, …) afin qu'un fichier existant reste
//! lisible sans transformation. Les champs inconnus sont conservés dans
//! [`Profile::extra`] et réécrits tels quels : une version future de TCC ou
//! un profil importé ne perd pas de données lors d'un aller-retour.
//!
//! # Taux de rafraîchissement
//!
//! L'original stocke `refreshRate: -1` pour « ne pas changer ». Le modèle Rust
//! expose [`DisplayProfile::refresh_rate`] en `Option<f32>` : `-1`, une valeur
//! négative, `null` ou un champ absent deviennent `None`. À la sérialisation,
//! `None` redevient `-1` pour rester compatible avec les lecteurs existants.
//! [`DisplayProfile::effective_refresh_rate`] tient compte de `useRefRate` :
//! même si un taux est renseigné, il n'est appliqué que si
//! [`DisplayProfile::use_refresh_rate`] est vrai.
//!
//! # Fréquences CPU
//!
//! `scalingMaxFrequency: -1` (`FrequencyConfig.ReducedFrequency` dans
//! l'original) est représenté par [`FrequencyLimit::Reduced`]. Le matériel
//! décide ensuite de la fréquence concrète ; `tcc-core` ne l'invente pas.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::fan::FanCurve;

/// Identifiant du profil personnalisé par défaut de TCC.
pub const DEFAULT_CUSTOM_PROFILE_ID: &str = "__default_custom_profile__";
/// Identifiant du profil preset « économie d'énergie maximale ».
pub const MAX_ENERGY_SAVE_PROFILE_ID: &str = "__profile_max_energy_save__";
/// Identifiant du profil preset « silencieux ».
pub const QUIET_PROFILE_ID: &str = "__profile_silent__";
/// Identifiant du profil preset « bureau ».
pub const OFFICE_PROFILE_ID: &str = "__office__";
/// Identifiant du profil preset « hautes performances ».
pub const HIGH_PERFORMANCE_PROFILE_ID: &str = "__high_performance__";

const LEGACY_PRESET_IDS: [&str; 3] = [
    "__legacy_default__",
    "__legacy_cool_and_breezy__",
    "__legacy_powersave_extreme__",
];

const PRESET_IDS: [&str; 4] = [
    MAX_ENERGY_SAVE_PROFILE_ID,
    QUIET_PROFILE_ID,
    OFFICE_PROFILE_ID,
    HIGH_PERFORMANCE_PROFILE_ID,
];

/// Identifiant de profil.
///
/// Newtype transparent autour de `String` : la sérialisation JSON reste une
/// chaîne simple, compatible avec les identifiants historiques
/// (`__office__`, identifiants aléatoires, profils importés).
///
/// Un `ProfileId` peut être construit sans validation ; la validation
/// centralisée ([`crate::validate_profile_id`]) est appliquée au chargement et
/// à la sauvegarde. Cela permet de lire puis de réparer un fichier legacy
/// (identifiant généré pour un profil qui n'en a pas) avant de le rejeter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileId(String);

impl ProfileId {
    /// Construit un identifiant sans validation.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Construit un identifiant validé.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si l'identifiant est vide, trop long ou contient des
    /// caractères non autorisés.
    pub fn try_new(id: impl Into<String>) -> Result<Self, crate::ValidationError> {
        let id = Self(id.into());
        crate::validate::validate_profile_id(&id)?;
        Ok(id)
    }

    /// Génère un identifiant unique pour un nouveau profil.
    ///
    /// Le préfixe `custom-` rend les identifiants générés reconnaissables dans
    /// les logs ; l'unicité repose sur un UUID v4.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("custom-{}", uuid::Uuid::new_v4().simple()))
    }

    /// Chaîne sous-jacente.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Vrai si l'identifiant est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Vrai si l'identifiant fait partie des presets fournis par TCC.
    #[must_use]
    pub fn is_builtin_preset(&self) -> bool {
        PRESET_IDS.contains(&self.0.as_str()) || LEGACY_PRESET_IDS.contains(&self.0.as_str())
    }

    /// Vrai si l'identifiant est fourni ou géré par TCC : les quatre presets,
    /// les presets legacy et le profil personnalisé par défaut.
    ///
    /// Ces identifiants ne peuvent pas être tombstonés : TCC les recrée ou les
    /// fusionne ([`crate::merge_with_default_profiles`]), et un tombstone
    /// rendrait la configuration incohérente.
    #[must_use]
    pub fn is_tcc_managed(&self) -> bool {
        self.is_builtin_preset() || self.0 == DEFAULT_CUSTOM_PROFILE_ID
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ProfileId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProfileId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProfileId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<ProfileId> for String {
    fn from(value: ProfileId) -> Self {
        value.0
    }
}

/// Réglages d'affichage d'un profil.
///
/// Le taux et la résolution ne concernent que l'écran interne. La conversion
/// vers le contrat IPC est faite par `tcc-daemon` : `refresh_rate` +
/// `use_refresh_rate` deviennent `DisplayModeRequest.refresh_rate_hz`, et
/// `x_resolution`/`y_resolution` ne sont transmis que si `use_resolution` est
/// vrai. `tcc-core` ne dépend pas de `tcc-ipc` pour conserver la frontière de
/// dépendance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplayProfile {
    /// Luminosité cible en pourcentage (`0..=100`).
    pub brightness: u8,
    /// Vrai si la luminosité doit être appliquée à l'activation du profil.
    pub use_brightness: bool,
    /// Taux de rafraîchissement cible de l'écran interne.
    ///
    /// `None` signifie « ne pas changer ». La sérialisation utilise `-1` pour
    /// rester compatible avec l'original.
    #[serde(
        deserialize_with = "de_optional_f32_sentinel",
        serialize_with = "ser_optional_f32_sentinel"
    )]
    pub refresh_rate: Option<f32>,
    /// Vrai si le taux de rafraîchissement doit être appliqué.
    ///
    /// La clé originale est `useRefRate` ; `useRefreshRate` est accepté en
    /// lecture pour les configurations futures ou écrites à la main.
    #[serde(rename = "useRefRate", alias = "useRefreshRate")]
    pub use_refresh_rate: bool,
    /// Largeur cible de l'écran interne.
    #[serde(
        deserialize_with = "de_optional_u32_sentinel",
        serialize_with = "ser_optional_u32_sentinel"
    )]
    pub x_resolution: Option<u32>,
    /// Hauteur cible de l'écran interne.
    #[serde(
        deserialize_with = "de_optional_u32_sentinel",
        serialize_with = "ser_optional_u32_sentinel"
    )]
    pub y_resolution: Option<u32>,
    /// Vrai si la résolution doit être appliquée.
    pub use_resolution: bool,
}

impl Default for DisplayProfile {
    fn default() -> Self {
        Self {
            brightness: 100,
            use_brightness: false,
            refresh_rate: None,
            use_refresh_rate: false,
            x_resolution: None,
            y_resolution: None,
            use_resolution: false,
        }
    }
}

impl DisplayProfile {
    /// Taux de rafraîchissement réellement applicable.
    ///
    /// Renvoie `None` si `useRefRate` est faux, même si un taux est renseigné :
    /// c'est la sémantique de l'original.
    #[must_use]
    pub fn effective_refresh_rate(&self) -> Option<f32> {
        if self.use_refresh_rate {
            self.refresh_rate
        } else {
            None
        }
    }

    /// Résolution réellement applicable.
    #[must_use]
    pub fn effective_resolution(&self) -> Option<(u32, u32)> {
        if self.use_resolution {
            match (self.x_resolution, self.y_resolution) {
                (Some(width), Some(height)) => Some((width, height)),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Vrai si une résolution partielle est renseignée.
    #[must_use]
    pub fn has_partial_resolution(&self) -> bool {
        self.x_resolution.is_some() != self.y_resolution.is_some()
    }
}

/// Limite de fréquence CPU.
///
/// Représente `scalingMaxFrequency` : soit une fréquence absolue en kHz, soit
/// le marqueur `-1` de l'original qui laisse le matériel choisir une fréquence
/// réduite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyLimit {
    /// Fréquence réduite décidée par la couche matérielle (`-1` historique).
    Reduced,
    /// Fréquence absolue en kHz.
    Hz(u64),
}

impl FrequencyLimit {
    /// Fréquence en kHz, ou `None` pour [`FrequencyLimit::Reduced`].
    #[must_use]
    pub fn hz(self) -> Option<u64> {
        match self {
            Self::Reduced => None,
            Self::Hz(hz) => Some(hz),
        }
    }
}

impl Serialize for FrequencyLimit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Reduced => serializer.serialize_i64(-1),
            Self::Hz(hz) => serializer.serialize_u64(*hz),
        }
    }
}

impl<'de> Deserialize<'de> for FrequencyLimit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = i64::deserialize(deserializer)?;
        match raw {
            -1 => Ok(Self::Reduced),
            value if value >= 0 => Ok(Self::Hz(value.unsigned_abs())),
            value => Err(de::Error::custom(format!(
                "fréquence CPU négative inattendue: {value}"
            ))),
        }
    }
}

/// Réglages CPU d'un profil.
///
/// Les fréquences sont exprimées en **kHz**, la même unité que
/// `tcc_hw::cpu::CpuPolicy` (`scaling_min_khz`, `scaling_max_khz`,
/// `available_frequencies_khz`). Une conversion `MHz → kHz` ne doit donc pas
/// être appliquée entre `tcc-hw` et ce modèle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CpuProfile {
    /// Nombre de cœurs à laisser en ligne, `None` pour ne pas changer.
    #[serde(deserialize_with = "de_optional_u32_sentinel")]
    pub online_cores: Option<u32>,
    /// Vrai pour appliquer le governor de performances maximales.
    pub use_max_perf_gov: bool,
    /// Fréquence minimale de scaling, en kHz.
    #[serde(deserialize_with = "de_optional_u64_sentinel")]
    pub scaling_min_frequency: Option<u64>,
    /// Fréquence maximale de scaling, en kHz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaling_max_frequency: Option<FrequencyLimit>,
    /// Governor cpufreq demandé.
    ///
    /// L'original ne l'utilise plus directement (`useMaxPerfGov` pilote le
    /// governor), mais le champ est conservé pour la compatibilité et l'export.
    pub governor: String,
    /// Préférence d'énergie (`energy_performance_preference`).
    pub energy_performance_preference: String,
    /// Vrai pour désactiver le turbo (`no_turbo`).
    pub no_turbo: bool,
}

impl Default for CpuProfile {
    fn default() -> Self {
        Self {
            online_cores: None,
            use_max_perf_gov: false,
            scaling_min_frequency: None,
            scaling_max_frequency: None,
            governor: "powersave".to_owned(),
            energy_performance_preference: "balance_performance".to_owned(),
            no_turbo: false,
        }
    }
}

/// État de la webcam à l'activation du profil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebcamProfile {
    /// Vrai pour activer la webcam, faux pour la couper.
    pub status: bool,
    /// Vrai si `status` doit être appliqué.
    pub use_status: bool,
}

impl Default for WebcamProfile {
    fn default() -> Self {
        Self {
            status: true,
            use_status: true,
        }
    }
}

/// Réglages de ventilation d'un profil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanProfile {
    /// Vrai si le contrôle des ventilateurs est actif pour ce profil.
    pub use_control: bool,
    /// Nom de la courbe sélectionnée (`Silent`, `Balanced`, `Custom`, …).
    pub fan_profile: String,
    /// Vitesse minimale imposée, en pourcentage.
    pub minimum_fanspeed: f32,
    /// Vitesse maximale imposée, en pourcentage.
    pub maximum_fanspeed: f32,
    /// Offset ajouté à la vitesse calculée, en pourcentage.
    pub offset_fanspeed: f32,
    /// Courbe personnalisée utilisée quand `fan_profile` vaut `Custom`.
    pub custom_fan_curve: FanCurve,
}

impl Default for FanProfile {
    fn default() -> Self {
        Self {
            use_control: true,
            fan_profile: "Balanced".to_owned(),
            minimum_fanspeed: 0.0,
            maximum_fanspeed: 100.0,
            offset_fanspeed: 0.0,
            custom_fan_curve: FanCurve::default_custom(),
        }
    }
}

impl FanProfile {
    /// Vrai si ce profil demande la courbe personnalisée.
    #[must_use]
    pub fn uses_custom_curve(&self) -> bool {
        self.fan_profile == "Custom"
    }
}

/// Profil ODM (`platform_profile` / `tuxedo_platform_profile`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OdmProfile {
    /// Nom du profil ODM, `None` pour ne pas changer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Limites de puissance TDP d'un profil.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OdmPowerLimits {
    /// Valeurs PL1/PL2/PL4 en watts, dans l'ordre fourni par le matériel.
    pub tdp_values: Vec<f64>,
}

/// Offset cTGP NVIDIA d'un profil.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NvidiaPowerCtrlProfile {
    /// Offset cTGP en watts, ajouté à la limite de puissance par défaut.
    #[serde(rename = "cTGPOffset")]
    pub ctgp_offset: f64,
}

/// Profil matériel complet.
///
/// Les champs inconnus sont conservés dans [`Profile::extra`] (aplatissement
/// `serde`) pour ne pas perdre les extensions d'une version future ou les
/// données d'un profil importé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Identifiant stable du profil.
    #[serde(default)]
    pub id: ProfileId,
    /// Nom affiché et unique.
    #[serde(default)]
    pub name: String,
    /// Description libre.
    #[serde(default)]
    pub description: String,
    /// Réglages d'affichage.
    #[serde(default)]
    pub display: DisplayProfile,
    /// Réglages CPU.
    #[serde(default)]
    pub cpu: CpuProfile,
    /// Réglages webcam.
    #[serde(default)]
    pub webcam: WebcamProfile,
    /// Réglages de ventilation.
    #[serde(default)]
    pub fan: FanProfile,
    /// Profil ODM.
    #[serde(default)]
    pub odm_profile: OdmProfile,
    /// Limites TDP.
    #[serde(default)]
    pub odm_power_limits: OdmPowerLimits,
    /// Offset cTGP NVIDIA.
    #[serde(default, rename = "nvidiaPowerCTRLProfile")]
    pub nvidia_power_ctrl_profile: NvidiaPowerCtrlProfile,
    /// Champs JSON inconnus, préservés à l'écriture.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
            name: "TUXEDO Defaults".to_owned(),
            description: "Edit profile to change behaviour".to_owned(),
            display: DisplayProfile::default(),
            cpu: CpuProfile::default(),
            webcam: WebcamProfile::default(),
            fan: FanProfile::default(),
            odm_profile: OdmProfile::default(),
            odm_power_limits: OdmPowerLimits::default(),
            nvidia_power_ctrl_profile: NvidiaPowerCtrlProfile::default(),
            extra: BTreeMap::new(),
        }
    }
}

impl Profile {
    /// Crée un profil avec des valeurs par défaut et un identifiant donné.
    #[must_use]
    pub fn new(id: impl Into<ProfileId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            ..Self::default()
        }
    }

    /// Vrai si l'identifiant correspond à un preset TCC.
    #[must_use]
    pub fn is_preset(&self) -> bool {
        self.id.is_builtin_preset()
    }

    /// Valide le profil.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur agrégée décrivant tous les champs invalides.
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        crate::validate::validate_profile(self)
    }
}

/// Collection ordonnée de profils.
///
/// L'ordre est celui du fichier JSON : la GUI conserve ainsi l'ordre choisi
/// par l'utilisateur. Les recherches sont linéaires (quelques dizaines de
/// profils au maximum), ce qui évite d'allouer un index supplémentaire.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileCollection {
    profiles: Vec<Profile>,
}

impl ProfileCollection {
    /// Construit une collection à partir d'une liste.
    #[must_use]
    pub fn new(profiles: Vec<Profile>) -> Self {
        Self { profiles }
    }

    /// Profils contenus dans la collection.
    #[must_use]
    pub fn as_slice(&self) -> &[Profile] {
        &self.profiles
    }

    /// Itère sur les profils.
    pub fn iter(&self) -> std::slice::Iter<'_, Profile> {
        self.profiles.iter()
    }

    /// Itère mutuellement sur les profils.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Profile> {
        self.profiles.iter_mut()
    }

    /// Nombre de profils.
    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Vrai si la collection est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Cherche un profil par identifiant.
    #[must_use]
    pub fn get(&self, id: &ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| &profile.id == id)
    }

    /// Cherche un profil par identifiant sous forme de chaîne.
    #[must_use]
    pub fn get_by_id(&self, id: &str) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|profile| profile.id.as_str() == id)
    }

    /// Cherche un profil mutable par identifiant.
    #[must_use]
    pub fn get_mut(&mut self, id: &ProfileId) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|profile| &profile.id == id)
    }

    /// Vrai si un profil porte cet identifiant.
    #[must_use]
    pub fn contains(&self, id: &ProfileId) -> bool {
        self.get(id).is_some()
    }

    /// Itère sur les identifiants.
    pub fn ids(&self) -> impl Iterator<Item = &ProfileId> {
        self.profiles.iter().map(|profile| &profile.id)
    }

    /// Ensemble des identifiants, utilisé pour la résolution d'état.
    #[must_use]
    pub fn id_set(&self) -> std::collections::BTreeSet<ProfileId> {
        self.profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect()
    }

    /// Ajoute un profil, ou remplace celui qui porte le même identifiant.
    ///
    /// Renvoie le profil remplacé, s'il existait. L'identifiant n'est pas
    /// validé ici ; [`ProfileCollection::validate`] s'en charge.
    pub fn upsert(&mut self, profile: Profile) -> Option<Profile> {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            return Some(std::mem::replace(existing, profile));
        }
        self.profiles.push(profile);
        None
    }

    /// Retire et renvoie le profil portant cet identifiant.
    pub fn remove(&mut self, id: &ProfileId) -> Option<Profile> {
        let index = self.profiles.iter().position(|profile| &profile.id == id)?;
        Some(self.profiles.remove(index))
    }

    /// Valide tous les profils, l'unicité des identifiants et des noms.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur agrégée si au moins un profil est invalide ou si
    /// deux profils partagent un identifiant ou un nom.
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        crate::validate::validate_profiles(self)
    }
}

impl From<Vec<Profile>> for ProfileCollection {
    fn from(profiles: Vec<Profile>) -> Self {
        Self::new(profiles)
    }
}

impl FromIterator<Profile> for ProfileCollection {
    fn from_iter<T: IntoIterator<Item = Profile>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl IntoIterator for ProfileCollection {
    type Item = Profile;
    type IntoIter = std::vec::IntoIter<Profile>;

    fn into_iter(self) -> Self::IntoIter {
        self.profiles.into_iter()
    }
}

impl<'a> IntoIterator for &'a ProfileCollection {
    type Item = &'a Profile;
    type IntoIter = std::slice::Iter<'a, Profile>;

    fn into_iter(self) -> Self::IntoIter {
        self.profiles.iter()
    }
}

fn de_optional_f32_sentinel<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f32>, D::Error> {
    let raw = Option::<f32>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(value) if !value.is_finite() => {
            Err(de::Error::custom("valeur de refresh rate non finie"))
        }
        Some(value) if value < 0.0 => Ok(None),
        Some(value) => Ok(Some(value)),
    }
}

fn ser_optional_f32_sentinel<S: Serializer>(
    value: &Option<f32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) if value.is_finite() && *value >= 0.0 => {
            if value.fract() == 0.0 {
                serializer.serialize_i64(*value as i64)
            } else {
                serializer.serialize_f32(*value)
            }
        }
        _ => serializer.serialize_i64(-1),
    }
}

fn de_optional_u32_sentinel<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    let raw = Option::<i64>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(value) if value < 0 => Ok(None),
        Some(value) => u32::try_from(value)
            .map(Some)
            .map_err(|_| de::Error::custom(format!("valeur u32 hors bornes: {value}"))),
    }
}

fn ser_optional_u32_sentinel<S: Serializer>(
    value: &Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.serialize_u32(*value),
        None => serializer.serialize_i64(-1),
    }
}

fn de_optional_u64_sentinel<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    let raw = Option::<i64>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(value) if value < 0 => Ok(None),
        Some(value) => Ok(Some(value.unsigned_abs())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CpuProfile, DEFAULT_CUSTOM_PROFILE_ID, DisplayProfile, FrequencyLimit, Profile,
        ProfileCollection, ProfileId,
    };

    const LEGACY_PROFILE: &str = r#"{
        "id": "__office__",
        "name": "Office",
        "description": "Bureau",
        "display": {
            "brightness": 60,
            "useBrightness": true,
            "refreshRate": -1,
            "useRefRate": false,
            "xResolution": -1,
            "yResolution": -1,
            "useResolution": false
        },
        "cpu": {
            "onlineCores": 8,
            "useMaxPerfGov": false,
            "scalingMinFrequency": 400000,
            "scalingMaxFrequency": -1,
            "governor": "powersave",
            "energyPerformancePreference": "balance_performance",
            "noTurbo": false
        },
        "webcam": { "status": true, "useStatus": true },
        "fan": {
            "useControl": true,
            "fanProfile": "Balanced",
            "minimumFanspeed": 0,
            "maximumFanspeed": 100,
            "offsetFanspeed": 0,
            "customFanCurve": {
                "tableCPU": [{ "temp": 20, "speed": 12 }],
                "tableGPU": [{ "temp": 20, "speed": 12 }]
            }
        },
        "odmProfile": { "name": "balanced" },
        "odmPowerLimits": { "tdpValues": [15, 25, 35] },
        "nvidiaPowerCTRLProfile": { "cTGPOffset": 10 },
        "futureField": { "kept": true }
    }"#;

    #[test]
    fn legacy_refresh_rate_minus_one_becomes_none() {
        let profile: Profile = serde_json::from_str(LEGACY_PROFILE).unwrap();
        assert_eq!(profile.display.refresh_rate, None);
        assert_eq!(profile.display.x_resolution, None);
        assert_eq!(profile.display.y_resolution, None);
        assert_eq!(profile.display.effective_refresh_rate(), None);
        assert_eq!(
            profile.cpu.scaling_max_frequency,
            Some(FrequencyLimit::Reduced)
        );
        assert_eq!(profile.cpu.scaling_min_frequency, Some(400_000));
    }

    #[test]
    fn unknown_fields_survive_roundtrip() {
        let profile: Profile = serde_json::from_str(LEGACY_PROFILE).unwrap();
        assert!(profile.extra.contains_key("futureField"));

        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("futureField"));
        assert!(json.contains("\"kept\":true"));
    }

    #[test]
    fn serialization_keeps_original_keys_and_sentinels() {
        let profile = Profile::default();
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["display"]["refreshRate"], -1);
        assert_eq!(json["display"]["useRefRate"], false);
        assert_eq!(json["cpu"]["scalingMinFrequency"], serde_json::Value::Null);
        assert_eq!(json["cpu"]["scalingMaxFrequency"], serde_json::Value::Null);
        assert_eq!(json["nvidiaPowerCTRLProfile"]["cTGPOffset"], 0.0);
        assert_eq!(json["fan"]["fanProfile"], "Balanced");
    }

    #[test]
    fn refresh_rate_roundtrips_with_fraction() {
        let mut profile = Profile::default();
        profile.display.refresh_rate = Some(59.94);
        profile.display.use_refresh_rate = true;

        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("59.94"));
        let parsed: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.display.refresh_rate, Some(59.94));
        assert_eq!(parsed.display.effective_refresh_rate(), Some(59.94));
    }

    #[test]
    fn refresh_rate_disabled_is_not_effective() {
        let display = DisplayProfile {
            refresh_rate: Some(144.0),
            use_refresh_rate: false,
            ..DisplayProfile::default()
        };
        assert_eq!(display.effective_refresh_rate(), None);
    }

    #[test]
    fn use_refresh_rate_alias_is_accepted() {
        let profile: Profile = serde_json::from_str(
            r#"{
                "id": "x",
                "name": "X",
                "display": { "refreshRate": 120, "useRefreshRate": true }
            }"#,
        )
        .unwrap();
        assert!(profile.display.use_refresh_rate);
        assert_eq!(profile.display.refresh_rate, Some(120.0));
    }

    #[test]
    fn missing_optional_sections_use_defaults() {
        let profile: Profile = serde_json::from_str(r#"{"id":"x","name":"X"}"#).unwrap();
        assert_eq!(
            profile,
            Profile {
                id: ProfileId::new("x"),
                name: "X".to_owned(),
                description: String::new(),
                ..Profile::default()
            }
        );
    }

    #[test]
    fn profile_id_generation_is_unique_and_valid() {
        let first = ProfileId::generate();
        let second = ProfileId::generate();
        assert_ne!(first, second);
        crate::validate::validate_profile_id(&first).unwrap();
    }

    #[test]
    fn preset_detection_matches_historical_ids() {
        assert!(ProfileId::new("__office__").is_builtin_preset());
        assert!(ProfileId::new("__legacy_default__").is_builtin_preset());
        assert!(!ProfileId::new("custom-123").is_builtin_preset());

        assert!(ProfileId::new("__office__").is_tcc_managed());
        assert!(ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID).is_tcc_managed());
        assert!(!ProfileId::new("custom-123").is_tcc_managed());
    }

    #[test]
    fn collection_upsert_replaces_same_id() {
        let mut collection = ProfileCollection::default();
        collection.upsert(Profile::new("a", "A"));
        let replaced = collection.upsert(Profile::new("a", "A2"));
        assert_eq!(replaced.unwrap().name, "A");
        assert_eq!(collection.len(), 1);
        assert_eq!(collection.get_by_id("a").unwrap().name, "A2");
    }

    #[test]
    fn collection_roundtrips_as_array() {
        let collection =
            ProfileCollection::new(vec![Profile::new("a", "A"), Profile::new("b", "B")]);
        let json = serde_json::to_string(&collection).unwrap();
        assert!(json.starts_with('['));
        let parsed: ProfileCollection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, collection);
    }

    #[test]
    fn cpu_profile_defaults_match_original() {
        let cpu = CpuProfile::default();
        assert_eq!(cpu.governor, "powersave");
        assert_eq!(cpu.energy_performance_preference, "balance_performance");
        assert!(!cpu.no_turbo);
    }

    #[test]
    fn default_profile_uses_known_id() {
        assert_eq!(Profile::default().id.as_str(), DEFAULT_CUSTOM_PROFILE_ID);
    }
}
