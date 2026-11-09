// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Le code est une
// réimplémentation originale des réglages globaux de TUXEDO Control Center
// (GPL-3.0-or-later), étendue à la matrice PPD × alimentation.

//! Réglages globaux persistants.
//!
//! Le format reprend les clés de l'original (`fahrenheit`, `stateMap`,
//! `shutdownTime`, `keyboardStates`, `lightbarStates`, …) et ajoute :
//!
//! - `schemaVersion` : version de schéma obligatoire ;
//! - `stateMatrix` : matrice `(PPD, alimentation) → profil` ;
//! - `deletedProfileIds` : identifiants de profils supprimés qui ne doivent
//!   jamais être réutilisés (tombstones) ;
//! - `chargingStartThreshold` / `chargingEndThreshold` : seuils optionnels,
//!   laissés vides tant que le matériel ne les expose pas ;
//! - `chargingType` / `fnLockStatus` : contrôles matériels persistants
//!   optionnels (`None` = ne pas toucher) ;
//! - les champs inconnus sont conservés dans [`Settings::extra`].
//!
//! # Migration des états clavier
//!
//! `keyboardBacklightStates` est l'ancien nom de `keyboardStates`. Il n'est pas
//! un champ typé : il atterrit dans [`Settings::extra`] et la migration
//! ([`crate::migration::migrate_settings`]) le convertit si nécessaire. Cela
//! évite de perdre la valeur si elle est mal formée ou si l'utilisateur
//! revient à une version antérieure.

use std::collections::BTreeMap;

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::profile::ProfileId;
use crate::state::{LegacyStateMap, StateMatrix};

/// Version courante du schéma de `settings.json`.
///
/// À incrémenter pour tout changement incompatible. La migration lit les
/// versions antérieures ; une version future est refusée par la validation
/// plutôt que mal interprétée.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Mode de couleur d'un clavier RGB.
///
/// L'original stocke un enum numérique (`static = 0`, `breathing = 1`). La
/// sérialisation conserve ce format pour rester compatible ; la
/// désérialisation accepte aussi les chaînes `static` / `breathing`, plus
/// lisibles dans un fichier édité à la main ou dans l'API typée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyboardColorMode {
    /// Couleur fixe.
    #[default]
    Static,
    /// Respiration.
    Breathing,
}

impl KeyboardColorMode {
    /// Valeur numérique historique.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Static => 0,
            Self::Breathing => 1,
        }
    }

    /// Convertit une valeur numérique historique.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Static),
            1 => Some(Self::Breathing),
            _ => None,
        }
    }

    /// Représentation textuelle stable.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Breathing => "breathing",
        }
    }

    /// Convertit une chaîne (`static` / `breathing`, insensible à la casse).
    #[must_use]
    pub fn from_str_ignore_case(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "static" => Some(Self::Static),
            "breathing" => Some(Self::Breathing),
            _ => None,
        }
    }
}

impl Serialize for KeyboardColorMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.as_u8())
    }
}

impl<'de> Deserialize<'de> for KeyboardColorMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Number(u8),
            Text(String),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Number(value) => Self::from_u8(value)
                .ok_or_else(|| de::Error::custom(format!("mode clavier inconnu: {value}"))),
            Repr::Text(value) => Self::from_str_ignore_case(&value)
                .ok_or_else(|| de::Error::custom(format!("mode clavier inconnu: {value}"))),
        }
    }
}

/// État d'une zone de rétroéclairage clavier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyboardState {
    /// Mode statique ou respiration.
    pub mode: KeyboardColorMode,
    /// Luminosité (`0..=255`, plafonnée par le matériel).
    pub brightness: u8,
    /// Composante rouge (`-1` = non supporté, sinon `0..=255`).
    pub red: i16,
    /// Composante verte (`-1` = non supporté, sinon `0..=255`).
    pub green: i16,
    /// Composante bleue (`-1` = non supporté, sinon `0..=255`).
    pub blue: i16,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            mode: KeyboardColorMode::Static,
            brightness: 0,
            red: -1,
            green: -1,
            blue: -1,
        }
    }
}

impl KeyboardState {
    /// Crée un état de zone.
    #[must_use]
    pub fn new(mode: KeyboardColorMode, brightness: u8, red: i16, green: i16, blue: i16) -> Self {
        Self {
            mode,
            brightness,
            red,
            green,
            blue,
        }
    }
}

/// État de la barre lumineuse (lightbar).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LightbarState {
    /// Luminosité (`0..=255`, plafonnée par le matériel).
    pub brightness: u8,
    /// Composante rouge.
    pub red: i16,
    /// Composante verte.
    pub green: i16,
    /// Composante bleue.
    pub blue: i16,
}

/// Réglages globaux.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Version du schéma. `0` signale un fichier legacy non migré.
    #[serde(default)]
    pub schema_version: u32,
    /// Afficher les températures en Fahrenheit.
    #[serde(default)]
    pub fahrenheit: bool,
    /// `stateMap` historique AC/batterie.
    #[serde(default)]
    pub state_map: LegacyStateMap,
    /// Matrice `(PPD, alimentation)`, vide si PPD n'est pas utilisé.
    #[serde(default)]
    pub state_matrix: StateMatrix,
    /// Identifiants de profils supprimés qui ne doivent jamais être réutilisés
    /// (tombstones).
    ///
    /// L'ordre d'insertion est conservé et sérialisé tel quel ;
    /// [`Settings::mark_profile_deleted`] ajoute en fin de liste. Les doublons,
    /// les identifiants invalides et les identifiants gérés par TCC (presets,
    /// profil personnalisé par défaut) sont refusés par
    /// [`Settings::validate`]. Limite : [`crate::MAX_DELETED_PROFILE_IDS`].
    #[serde(default)]
    pub deleted_profile_ids: Vec<ProfileId>,
    /// Heure d'extinction programmée, format `HH:MM`, ou `None`.
    #[serde(default)]
    pub shutdown_time: Option<String>,
    /// Activer l'application des réglages CPU des profils.
    #[serde(default = "default_true")]
    pub cpu_settings_enabled: bool,
    /// Activer le contrôle des ventilateurs par TCC.
    #[serde(default = "default_true")]
    pub fan_control_enabled: bool,
    /// Activer le contrôle du rétroéclairage clavier/lightbar.
    #[serde(default = "default_true")]
    pub keyboard_backlight_control_enabled: bool,
    /// Workaround YCbCr 4:2:0 par carte/port.
    ///
    /// Chaque carte est un objet `{ "<port>": bool }`, comme dans l'original.
    #[serde(default)]
    pub ycbcr420_workaround: Vec<BTreeMap<String, bool>>,
    /// Profil de charge courant (`high`, `medium`, `low`, …) si supporté.
    #[serde(default)]
    pub charging_profile: Option<String>,
    /// Priorité de charge (`charging`, `performance`, …) si supportée.
    #[serde(default)]
    pub charging_priority: Option<String>,
    /// Seuil de début de charge en pourcentage, si supporté.
    #[serde(default)]
    pub charging_start_threshold: Option<u8>,
    /// Seuil de fin de charge en pourcentage, si supporté.
    #[serde(default)]
    pub charging_end_threshold: Option<u8>,
    /// Type de charge demandé (`fast`, `standard`, `adaptive`, …), si le
    /// matériel l'expose.
    ///
    /// Liste **ouverte** : `tcc-core` ne connaît pas les valeurs du firmware.
    /// La forme est validée (≤ 64 caractères, `[A-Za-z0-9._:-]`), mais la
    /// validité matérielle est vérifiée par le daemon contre les capacités
    /// réelles avant toute écriture. `None` = ne pas toucher au type courant ;
    /// matériel absent ou type non supporté ⇒ no-op journalisé, jamais
    /// d'écriture arbitraire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_type: Option<String>,
    /// État demandé du Fn-Lock.
    ///
    /// `None` = ne pas toucher. `Some(bool)` = état appliqué au chargement et
    /// à chaque rechargement. Le daemon vérifie la disponibilité et écrit le
    /// sysfs ; si le Fn-Lock est absent ou en lecture seule, il journalise et
    /// ne fait rien. Ce champ ne force jamais une écriture matérielle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fn_lock_status: Option<bool>,
    /// États des zones RGB du clavier.
    #[serde(default)]
    pub keyboard_states: Vec<KeyboardState>,
    /// État de la lightbar, `None` si absente.
    #[serde(default)]
    pub lightbar_states: Option<LightbarState>,
    /// Consentement persistant pour que TCC écrive `platform_profile` même
    /// lorsque power-profiles-daemon en est propriétaire.
    ///
    /// `false` par défaut : PPD reste propriétaire de `platform_profile` et de
    /// l'EPP (D4), et l'admin refuse toute écriture de ce champ. `true`
    /// autorise **uniquement** la méthode phase 2
    /// `Admin.Hw.SetOdmProfileOverride` (polkit systématique) à écrire
    /// `platform_profile` ; ce booléen ne déclenche à lui seul aucune écriture
    /// et n'est pas encore lu par `tccd` (prérequis
    /// `docs/security/ADMIN_PHASE2_DESIGN.md` §7.4).
    ///
    /// Le champ est un booléen JSON strict (`true`/`false`) : une chaîne ou un
    /// nombre est refusé par la désérialisation, y compris via la migration
    /// legacy. Aucun alias snake_case n'est accepté.
    #[serde(default)]
    pub ppd_override: bool,
    /// Champs inconnus, préservés à l'écriture.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            fahrenheit: false,
            state_map: LegacyStateMap::default(),
            state_matrix: StateMatrix::default(),
            deleted_profile_ids: Vec::new(),
            shutdown_time: None,
            cpu_settings_enabled: true,
            fan_control_enabled: true,
            keyboard_backlight_control_enabled: true,
            ycbcr420_workaround: Vec::new(),
            charging_profile: None,
            charging_priority: None,
            charging_start_threshold: None,
            charging_end_threshold: None,
            charging_type: None,
            fn_lock_status: None,
            keyboard_states: Vec::new(),
            lightbar_states: None,
            ppd_override: false,
            extra: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Réglages par défaut avec la version de schéma courante.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Vrai si l'identifiant figure dans les tombstones.
    ///
    /// Recherche linéaire volontaire : la liste est bornée à
    /// [`crate::MAX_DELETED_PROFILE_IDS`] et rarement lue dans une boucle
    /// chaude. Aucun index supplémentaire n'est sérialisé.
    #[must_use]
    pub fn is_profile_id_deleted(&self, id: &ProfileId) -> bool {
        self.deleted_profile_ids.iter().any(|deleted| deleted == id)
    }

    /// Marque un identifiant de profil comme supprimé.
    ///
    /// L'opération est **idempotente** : marquer deux fois le même ID réussit
    /// sans créer de doublon. L'ID est ajouté en fin de liste (ordre
    /// d'insertion conservé). Les identifiants gérés par TCC (presets, presets
    /// legacy, profil personnalisé par défaut) ne peuvent pas être tombstonés :
    /// TCC les recrée ou les fusionne toujours.
    ///
    /// Il n'existe volontairement **pas** d'`unmark` : un tombstone est
    /// permanent. Réutiliser un ID est une décision de réparation
    /// administrateur hors API (édition root de `/var/lib`), jamais une
    /// opération offerte au parser `Admin1`.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si l'identifiant est syntaxiquement invalide, s'il
    /// est géré par TCC, ou si la liste a déjà atteint
    /// [`crate::MAX_DELETED_PROFILE_IDS`] entrées.
    pub fn mark_profile_deleted(&mut self, id: &ProfileId) -> Result<(), crate::ValidationError> {
        if let Err(error) = crate::validate_profile_id(id) {
            let mut wrapped = crate::ValidationError::new();
            wrapped.push("deletedProfileIds", error.to_string());
            return Err(wrapped);
        }
        if id.is_tcc_managed() {
            let mut error = crate::ValidationError::new();
            error.push(
                "deletedProfileIds",
                "un identifiant fourni par TCC ne peut pas être tombstoné",
            );
            return Err(error);
        }
        if self.is_profile_id_deleted(id) {
            return Ok(());
        }
        if self.deleted_profile_ids.len() >= crate::MAX_DELETED_PROFILE_IDS {
            let mut error = crate::ValidationError::new();
            error.push(
                "deletedProfileIds",
                format!(
                    "ne doit pas dépasser {} entrées",
                    crate::MAX_DELETED_PROFILE_IDS
                ),
            );
            return Err(error);
        }
        self.deleted_profile_ids.push(id.clone());
        Ok(())
    }

    /// Initialise la matrice avec la proposition par défaut.
    ///
    /// Les presets référencés sont fournis par
    /// [`crate::presets::default_profiles`] ; fusionner les profils chargés
    /// avec `merge_with_default_profiles` avant d'écrire la configuration
    /// garantit qu'aucune référence ne manque.
    pub fn with_suggested_matrix(&mut self) -> &mut Self {
        self.state_matrix = StateMatrix::suggested_default();
        self
    }

    /// Vrai si le fichier a été produit par une version antérieure.
    #[must_use]
    pub fn needs_migration(&self) -> bool {
        self.schema_version != SETTINGS_SCHEMA_VERSION
    }

    /// Valide les réglages.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur agrégée décrivant tous les champs invalides.
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        crate::validate::validate_settings(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyboardColorMode, KeyboardState, SETTINGS_SCHEMA_VERSION, Settings};
    use crate::MAX_DELETED_PROFILE_IDS;
    use crate::profile::{DEFAULT_CUSTOM_PROFILE_ID, OFFICE_PROFILE_ID, ProfileId};

    const LEGACY_SETTINGS: &str = r#"{
        "fahrenheit": true,
        "stateMap": { "power_ac": "__office__", "power_bat": "__profile_silent__" },
        "shutdownTime": "22:30",
        "cpuSettingsEnabled": false,
        "fanControlEnabled": true,
        "keyboardBacklightControlEnabled": true,
        "ycbcr420Workaround": [{ "card0-DP-1": true }],
        "chargingProfile": "high",
        "chargingPriority": "charging",
        "keyboardStates": [
            { "mode": 0, "brightness": 255, "red": 255, "green": 0, "blue": 0 }
        ],
        "lightbarStates": { "brightness": 10, "red": 1, "green": 2, "blue": 3 },
        "keyboardBacklightStates": []
    }"#;

    #[test]
    fn legacy_settings_parse_with_defaults() {
        let settings: Settings = serde_json::from_str(LEGACY_SETTINGS).unwrap();
        assert!(settings.fahrenheit);
        assert_eq!(settings.state_map.power_ac, ProfileId::new("__office__"));
        assert_eq!(settings.shutdown_time.as_deref(), Some("22:30"));
        assert!(!settings.cpu_settings_enabled);
        assert!(settings.fan_control_enabled);
        assert_eq!(settings.schema_version, 0);
        assert!(settings.needs_migration());
        assert_eq!(settings.keyboard_states.len(), 1);
        assert_eq!(settings.keyboard_states[0].red, 255);
        assert_eq!(settings.lightbar_states.as_ref().unwrap().blue, 3);
        // Champ legacy non typé, conservé pour la migration.
        assert!(settings.extra.contains_key("keyboardBacklightStates"));
    }

    #[test]
    fn default_settings_have_current_schema_and_toggles() {
        let settings = Settings::default();
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.cpu_settings_enabled);
        assert!(settings.fan_control_enabled);
        assert!(settings.keyboard_backlight_control_enabled);
        assert!(!settings.ppd_override);
        assert!(settings.deleted_profile_ids.is_empty());
        assert!(settings.charging_type.is_none());
        assert!(settings.fn_lock_status.is_none());
        assert!(settings.state_matrix.is_empty());
        assert!(!settings.fahrenheit);
    }

    #[test]
    fn settings_roundtrip_preserves_unknown_fields() {
        let mut settings: Settings = serde_json::from_str(LEGACY_SETTINGS).unwrap();
        settings
            .extra
            .insert("future".into(), serde_json::json!(42));
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.extra.get("future"), Some(&serde_json::json!(42)));
        assert_eq!(parsed.keyboard_states, settings.keyboard_states);
    }

    #[test]
    fn missing_booleans_default_to_enabled() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.cpu_settings_enabled);
        assert!(settings.fan_control_enabled);
        assert!(settings.keyboard_backlight_control_enabled);
        assert!(!settings.ppd_override);
    }

    #[test]
    fn ppd_override_defaults_to_false_and_roundtrips() {
        let absent: Settings = serde_json::from_str("{}").unwrap();
        assert!(!absent.ppd_override);

        for value in [false, true] {
            let settings = Settings {
                ppd_override: value,
                ..Settings::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            assert!(json.contains("\"ppdOverride\":"));
            assert!(!json.contains("\"ppd_override\""));
            let parsed: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.ppd_override, value);
            assert_eq!(parsed, settings);
            assert!(!parsed.extra.contains_key("ppdOverride"));
        }

        // Booléen strict : ni chaîne ni nombre ne sont coercés.
        assert!(serde_json::from_str::<Settings>(r#"{"ppdOverride":"true"}"#).is_err());
        assert!(serde_json::from_str::<Settings>(r#"{"ppdOverride":1}"#).is_err());
    }

    #[test]
    fn validate_accepts_both_ppd_override_values() {
        for value in [false, true] {
            let settings = Settings {
                ppd_override: value,
                ..Settings::default()
            };
            settings.validate().unwrap();
        }
    }

    #[test]
    fn charging_type_and_fn_lock_status_default_to_none_and_roundtrip() {
        let absent: Settings = serde_json::from_str("{}").unwrap();
        assert!(absent.charging_type.is_none());
        assert!(absent.fn_lock_status.is_none());

        let settings = Settings {
            charging_type: Some("fast.charge:1-A".to_owned()),
            fn_lock_status: Some(true),
            ..Settings::default()
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            json.get("chargingType"),
            Some(&serde_json::json!("fast.charge:1-A"))
        );
        assert_eq!(json.get("fnLockStatus"), Some(&serde_json::json!(true)));
        assert!(json.get("charging_type").is_none());
        assert!(json.get("fn_lock_status").is_none());

        let parsed: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, settings);
        parsed.validate().unwrap();

        // `None` est omis à la sérialisation et relu comme `None`.
        let default_json = serde_json::to_value(Settings::default()).unwrap();
        assert!(default_json.get("chargingType").is_none());
        assert!(default_json.get("fnLockStatus").is_none());
        let reread: Settings = serde_json::from_value(default_json).unwrap();
        assert!(reread.charging_type.is_none());
        assert!(reread.fn_lock_status.is_none());

        // Types stricts : ni chaîne pour le booléen, ni nombre pour le type.
        assert!(serde_json::from_str::<Settings>(r#"{"fnLockStatus":"yes"}"#).is_err());
        assert!(serde_json::from_str::<Settings>(r#"{"chargingType":3}"#).is_err());
    }

    #[test]
    fn charging_and_fn_lock_preserve_unknown_fields() {
        let raw = r#"{
            "chargingType": "adaptive",
            "fnLockStatus": false,
            "futureSetting": 42
        }"#;
        let settings: Settings = serde_json::from_str(raw).unwrap();
        assert_eq!(settings.charging_type.as_deref(), Some("adaptive"));
        assert_eq!(settings.fn_lock_status, Some(false));
        assert_eq!(
            settings.extra.get("futureSetting"),
            Some(&serde_json::json!(42))
        );

        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json.get("futureSetting"), Some(&serde_json::json!(42)));
        assert_eq!(json.get("fnLockStatus"), Some(&serde_json::json!(false)));
    }

    #[test]
    fn deleted_profile_ids_default_empty_and_roundtrip() {
        let absent: Settings = serde_json::from_str("{}").unwrap();
        assert!(absent.deleted_profile_ids.is_empty());

        let mut settings = Settings::default();
        settings
            .mark_profile_deleted(&ProfileId::new("custom-old"))
            .unwrap();
        settings
            .mark_profile_deleted(&ProfileId::new("custom-older"))
            .unwrap();

        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            json.get("deletedProfileIds"),
            Some(&serde_json::json!(["custom-old", "custom-older"]))
        );
        assert!(json.get("deleted_profile_ids").is_none());

        let parsed: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.deleted_profile_ids, settings.deleted_profile_ids);
        assert!(parsed.is_profile_id_deleted(&ProfileId::new("custom-old")));
        assert!(!parsed.is_profile_id_deleted(&ProfileId::new("custom-new")));
        assert!(!parsed.extra.contains_key("deletedProfileIds"));
        parsed.validate().unwrap();
    }

    #[test]
    fn mark_profile_deleted_is_idempotent() {
        let mut settings = Settings::default();
        let id = ProfileId::new("custom-gone");
        settings.mark_profile_deleted(&id).unwrap();
        settings.mark_profile_deleted(&id).unwrap();
        assert_eq!(settings.deleted_profile_ids, vec![id]);
    }

    #[test]
    fn mark_profile_deleted_refuses_tcc_managed_ids() {
        for id in [
            OFFICE_PROFILE_ID,
            DEFAULT_CUSTOM_PROFILE_ID,
            "__legacy_default__",
        ] {
            let mut settings = Settings::default();
            let error = settings
                .mark_profile_deleted(&ProfileId::new(id))
                .unwrap_err();
            assert!(error.to_string().contains("TCC"), "id={id}");
            assert!(settings.deleted_profile_ids.is_empty());
        }
    }

    #[test]
    fn mark_profile_deleted_refuses_invalid_ids() {
        let mut settings = Settings::default();
        for id in ["", "bad id", "bad/id"] {
            assert!(settings.mark_profile_deleted(&ProfileId::new(id)).is_err());
        }
        assert!(
            settings
                .mark_profile_deleted(&ProfileId::new("a".repeat(65)))
                .is_err()
        );
        assert!(settings.deleted_profile_ids.is_empty());
    }

    #[test]
    fn mark_profile_deleted_enforces_the_bound() {
        let mut settings = Settings::default();
        for index in 0..MAX_DELETED_PROFILE_IDS {
            settings
                .mark_profile_deleted(&ProfileId::new(format!("custom-{index}")))
                .unwrap();
        }
        assert_eq!(settings.deleted_profile_ids.len(), MAX_DELETED_PROFILE_IDS);

        // Idempotence au plafond : un ID déjà présent reste accepté.
        let existing = settings.deleted_profile_ids[0].clone();
        settings.mark_profile_deleted(&existing).unwrap();

        let error = settings
            .mark_profile_deleted(&ProfileId::new("custom-overflow"))
            .unwrap_err();
        assert!(error.to_string().contains("1024"));
        assert_eq!(settings.deleted_profile_ids.len(), MAX_DELETED_PROFILE_IDS);
    }

    #[test]
    fn keyboard_mode_accepts_numbers_and_strings() {
        let numeric: KeyboardColorMode = serde_json::from_str("1").unwrap();
        assert_eq!(numeric, KeyboardColorMode::Breathing);
        let text: KeyboardColorMode = serde_json::from_str("\"static\"").unwrap();
        assert_eq!(text, KeyboardColorMode::Static);
        assert!(serde_json::from_str::<KeyboardColorMode>("2").is_err());
        assert_eq!(
            serde_json::to_string(&KeyboardColorMode::Breathing).unwrap(),
            "1"
        );
    }

    #[test]
    fn keyboard_default_colors_mark_unsupported() {
        let state = KeyboardState::default();
        assert_eq!(state.red, -1);
        assert_eq!(state.green, -1);
        assert_eq!(state.blue, -1);
    }

    #[test]
    fn suggested_matrix_is_opt_in() {
        let mut settings = Settings::new();
        assert!(settings.state_matrix.is_empty());
        settings.with_suggested_matrix();
        assert_eq!(settings.state_matrix.len(), 6);
    }
}
