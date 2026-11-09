// SPDX-License-Identifier: GPL-3.0-or-later
//
// Édition locale d'un profil et application sur la collection validée.
//
// Ce module ne touche ni à D-Bus ni à Qt : il applique un **DTO d'édition
// borné** (champs de la phase 1) sur la dernière `ProfileCollection` validée
// par `tcc-core`, puis revalide l'ensemble. La GUI n'envoie jamais un profil
// complet : elle décrit uniquement les champs modifiables, ce qui évite de
// réécrire des champs qu'elle ne comprend pas (TDP, ODM, cTGP…).
//
// Règles :
// - les presets ne sont jamais éditables (lecture seule, jamais renvoyés comme
//   personnalisés) ;
// - les valeurs sont bornées avant application (mêmes limites que `tcc-core`
//   quand elles existent) ;
// - aucune correction silencieuse : une valeur invalide produit une erreur
//   typée avec le nom du champ, jamais une valeur de remplacement ;
// - le payload sérialisé est borné à 1 Mio avant l'envoi à `tccd-admin`.

use serde::Deserialize;
use tcc_core::{
    FAN_SPEED_MAX_PERCENT, FAN_SPEED_MIN_PERCENT, FanTableEntry, FrequencyLimit,
    MAX_CPU_FREQUENCY_KHZ, MAX_CPU_ONLINE_CORES, MAX_DESCRIPTION_LEN, MAX_FAN_TABLE_ENTRIES,
    MAX_PROFILE_NAME_LEN, MAX_PROFILES, MIN_CPU_FREQUENCY_KHZ, Profile, ProfileCollection,
    ProfileId, validate_profile_id,
};

/// Taille maximale du DTO d'édition envoyé par QML (64 Kio).
pub const MAX_EDIT_BYTES: usize = 64 * 1024;
/// Dimension maximale acceptée pour une résolution d'écran (pixels).
const MAX_RESOLUTION_DIMENSION: u32 = 16_384;
/// Le profil ne transporte que PL1/PL2/PL4 (le core tolère jusqu'à 16 pour
/// les fichiers historiques).
const MAX_PROFILE_TDP_VALUES: usize = 3;
/// Bornes core du TDP/cTGP.
const MIN_TDP_WATTS: f64 = 1.0;
const MAX_TDP_WATTS: f64 = 1000.0;
const MAX_CTGP_WATTS: f64 = 200.0;
const MAX_ODM_NAME_CHARS: usize = 64;
/// Taille maximale de la collection sérialisée (1 Mio, contrat `Admin1`).
pub const MAX_PROFILES_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Longueur maximale d'un détail d'erreur relayé à l'UI.
const MAX_ERROR_DETAIL_CHARS: usize = 120;

/// Champs modifiables en phase 1.
///
/// `deny_unknown_fields` : un champ non prévu est un bug de la GUI, pas une
/// extension à ignorer. Tous les champs sont obligatoires pour éviter les
/// valeurs par défaut implicites.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileEdit {
    pub name: String,
    pub description: String,
    pub brightness: u8,
    pub use_brightness: bool,
    /// `None` = « ne pas changer ».
    pub refresh_rate: Option<f32>,
    pub use_refresh_rate: bool,
    /// Résolution cible de l'écran interne. `None` = ne pas modifier ;
    /// `Some(false)` désactive l'application sans effacer les valeurs.
    #[serde(default)]
    pub use_resolution: Option<bool>,
    #[serde(default)]
    pub x_resolution: Option<u32>,
    #[serde(default)]
    pub y_resolution: Option<u32>,
    /// `None` = ne pas modifier ce champ (profil sans valeur explicite).
    pub online_cores: Option<u32>,
    pub use_max_perf_gov: bool,
    /// Fréquence minimale en kHz, `None` = ne pas modifier.
    pub scaling_min_khz: Option<u64>,
    /// Fréquence maximale : `None` = ne pas modifier, `-1` = marqueur
    /// « réduit » du firmware, valeur positive = kHz.
    pub scaling_max_khz: Option<i64>,
    pub energy_performance_preference: String,
    pub no_turbo: bool,
    pub webcam_status: bool,
    pub use_webcam_status: bool,
    pub fan_use_control: bool,
    pub fan_profile: String,
    pub fan_min: f32,
    pub fan_max: f32,
    pub fan_offset: f32,
    /// Courbe personnalisée CPU. `None` = ne pas modifier la courbe existante
    /// (nom et table GPU toujours préservés).
    #[serde(default)]
    pub custom_fan_curve: Option<FanCurveEdit>,
    /// Profil ODM du profil matériel. `None` = ne pas changer.
    #[serde(default)]
    pub odm_profile: Option<String>,
    /// Limites TDP PL1/PL2/PL4. `None` = ne pas changer ; `[]` efface.
    #[serde(default)]
    pub tdp_values: Option<Vec<f64>>,
    /// Offset cTGP NVIDIA. `None` = ne pas changer.
    #[serde(default)]
    pub ctgp_offset: Option<f64>,
}

/// Courbe personnalisée éditée par la GUI (CPU uniquement).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanCurveEdit {
    /// Table CPU (`tableCPU` dans le JSON historique).
    #[serde(rename = "tableCPU")]
    pub table_cpu: Vec<FanPointEdit>,
}

/// Point de courbe fourni par QML.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanPointEdit {
    pub temp: f32,
    pub speed: f32,
}

impl ProfileEdit {
    /// Vérifie les bornes locales et la cohérence des champs.
    ///
    /// Renvoie le nom stable du premier champ invalide (jamais la valeur, qui
    /// peut contenir des données utilisateur).
    fn validate(&self) -> Result<(), SaveError> {
        let name = self.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_PROFILE_NAME_LEN
            || name.chars().any(char::is_control)
        {
            return Err(SaveError::Validation {
                field: "name".to_owned(),
            });
        }
        if self.description.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(SaveError::Validation {
                field: "description".to_owned(),
            });
        }
        if self.brightness > 100 {
            return Err(SaveError::Validation {
                field: "brightness".to_owned(),
            });
        }
        if let Some(rate) = self.refresh_rate {
            if !rate.is_finite() || !(1.0..=1000.0).contains(&rate) {
                return Err(SaveError::Validation {
                    field: "refreshRate".to_owned(),
                });
            }
        }
        if self.use_refresh_rate && self.refresh_rate.is_none() {
            return Err(SaveError::Validation {
                field: "refreshRate".to_owned(),
            });
        }
        if self.use_resolution == Some(true) {
            let valid = match (self.x_resolution, self.y_resolution) {
                (Some(width), Some(height)) => {
                    (1..=MAX_RESOLUTION_DIMENSION).contains(&width)
                        && (1..=MAX_RESOLUTION_DIMENSION).contains(&height)
                }
                _ => false,
            };
            if !valid {
                return Err(SaveError::Validation {
                    field: "resolution".to_owned(),
                });
            }
        }
        if let Some(cores) = self.online_cores {
            if cores == 0 || cores > MAX_CPU_ONLINE_CORES {
                return Err(SaveError::Validation {
                    field: "onlineCores".to_owned(),
                });
            }
        }
        if let Some(min) = self.scaling_min_khz {
            if !(MIN_CPU_FREQUENCY_KHZ..=MAX_CPU_FREQUENCY_KHZ).contains(&min) {
                return Err(SaveError::Validation {
                    field: "scalingMinFrequency".to_owned(),
                });
            }
        }
        if let Some(max) = self.scaling_max_khz {
            // -1 est le marqueur historique « réduit » ; toute autre valeur est
            // une fréquence en kHz et doit respecter les bornes.
            if max != -1 {
                let valid = max >= 0
                    && u64::try_from(max).is_ok_and(|hz| {
                        (MIN_CPU_FREQUENCY_KHZ..=MAX_CPU_FREQUENCY_KHZ).contains(&hz)
                    });
                let ordered = self
                    .scaling_min_khz
                    .is_none_or(|min| max >= 0 && (max as u64) >= min);
                if !valid || !ordered {
                    return Err(SaveError::Validation {
                        field: "scalingMaxFrequency".to_owned(),
                    });
                }
            }
        }
        let epp = self.energy_performance_preference.as_str();
        if epp.is_empty()
            || epp.len() > 32
            || !epp
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(SaveError::Validation {
                field: "energyPerformancePreference".to_owned(),
            });
        }
        if let Some(name) = self.odm_profile.as_deref() {
            let name = name.trim();
            if name.is_empty()
                || name.chars().count() > MAX_ODM_NAME_CHARS
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                return Err(SaveError::Validation {
                    field: "odmProfile".to_owned(),
                });
            }
        }
        if let Some(values) = self.tdp_values.as_ref() {
            if values.len() > MAX_PROFILE_TDP_VALUES {
                return Err(SaveError::Validation {
                    field: "tdpValues".to_owned(),
                });
            }
            let mut previous = None;
            for value in values {
                if !value.is_finite()
                    || !(MIN_TDP_WATTS..=MAX_TDP_WATTS).contains(value)
                    || previous.is_some_and(|prev| *value <= prev)
                {
                    return Err(SaveError::Validation {
                        field: "tdpValues".to_owned(),
                    });
                }
                previous = Some(*value);
            }
        }
        if let Some(offset) = self.ctgp_offset {
            if !offset.is_finite() || !(0.0..=MAX_CTGP_WATTS).contains(&offset) {
                return Err(SaveError::Validation {
                    field: "ctgpOffset".to_owned(),
                });
            }
        }
        if !matches!(
            self.fan_profile.as_str(),
            "silent" | "quiet" | "balanced" | "performance" | "custom" | "auto"
        ) {
            return Err(SaveError::Validation {
                field: "fanProfile".to_owned(),
            });
        }
        for (field, value) in [
            ("minimumFanspeed", self.fan_min),
            ("maximumFanspeed", self.fan_max),
        ] {
            if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                return Err(SaveError::Validation {
                    field: field.to_owned(),
                });
            }
        }
        if self.fan_min > self.fan_max {
            return Err(SaveError::Validation {
                field: "minimumFanspeed".to_owned(),
            });
        }
        if !self.fan_offset.is_finite() || !(-100.0..=100.0).contains(&self.fan_offset) {
            return Err(SaveError::Validation {
                field: "offsetFanspeed".to_owned(),
            });
        }
        if let Some(curve) = &self.custom_fan_curve {
            let invalid = || SaveError::Validation {
                field: "fanCurve".to_owned(),
            };
            if curve.table_cpu.len() < 2 || curve.table_cpu.len() > MAX_FAN_TABLE_ENTRIES {
                return Err(invalid());
            }
            let mut previous = f32::NEG_INFINITY;
            for point in &curve.table_cpu {
                if !point.temp.is_finite()
                    || !(0.0..=100.0).contains(&point.temp)
                    || !point.speed.is_finite()
                    || !(FAN_SPEED_MIN_PERCENT..=FAN_SPEED_MAX_PERCENT).contains(&point.speed)
                    || point.temp <= previous
                {
                    return Err(invalid());
                }
                previous = point.temp;
            }
        }
        Ok(())
    }
}

/// Erreur d'édition ou d'enregistrement.
///
/// `code()` est stable et destiné à l'i18n côté QML ; `detail()` est un
/// identifiant de champ ou de profil, jamais un chemin ni un contenu de
/// fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Service `Admin1` absent ou désactivé.
    AdminUnavailable,
    /// Payload ou DTO au-delà de la borne.
    PayloadTooLarge,
    /// DTO illisible (champ manquant, type faux, clé inconnue).
    InvalidPayload,
    /// Aucune collection validée en mémoire (première lecture non faite).
    NoData,
    /// Profil demandé absent de la collection.
    NotFound,
    /// Profil preset : lecture seule.
    PresetReadOnly,
    /// Collection modifiée par un autre client depuis la base d'édition.
    Conflict,
    /// Limite `MAX_PROFILES` atteinte.
    TooManyProfiles,
    /// Fichier d'import invalide (JSON, enveloppe ou version).
    InvalidImport,
    /// Chemin fourni par le `FileDialog` refusé.
    PathInvalid,
    /// Lecture du fichier d'import impossible.
    ReadFailed,
    /// Écriture du fichier d'export impossible.
    FileWriteFailed,
    /// Aucun profil personnalisé à exporter.
    NothingToExport,
    /// Valeur refusée par la validation locale.
    Validation { field: String },
    /// Refus ou erreur du service admin (`code` = suffixe `Admin1.Error.*`).
    Admin { code: &'static str, detail: String },
    /// Erreur de transport D-Bus (détail volontairement générique).
    Transport,
    /// Délai dépassé (polkit ou service bloqué).
    Timeout,
}

impl SaveError {
    /// Code stable pour l'UI (`AdminUnavailable`, `NotAuthorized`, …).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AdminUnavailable => "AdminUnavailable",
            Self::PayloadTooLarge => "PayloadTooLarge",
            Self::InvalidPayload => "InvalidPayload",
            Self::NoData => "NoData",
            Self::NotFound => "NotFound",
            Self::PresetReadOnly => "PresetReadOnly",
            Self::Conflict => "Conflict",
            Self::TooManyProfiles => "TooManyProfiles",
            Self::InvalidImport => "InvalidImport",
            Self::PathInvalid => "PathInvalid",
            Self::ReadFailed => "ReadFailed",
            Self::FileWriteFailed => "FileWriteFailed",
            Self::NothingToExport => "NothingToExport",
            Self::Validation { .. } => "Validation",
            Self::Admin { code, .. } => code,
            Self::Transport => "Transport",
            Self::Timeout => "Timeout",
        }
    }

    /// Détail sûr à afficher (champ ou identifiant), ou chaîne vide.
    #[must_use]
    pub fn detail(&self) -> String {
        let raw = match self {
            Self::Validation { field } => field.as_str(),
            Self::Admin { detail, .. } => detail.as_str(),
            _ => return String::new(),
        };
        sanitize_detail(raw)
    }
}

/// Normalise un détail d'erreur : ASCII graphique, sans séparateur de chemin,
/// borné. Toute valeur suspecte est remplacée par une chaîne vide.
fn sanitize_detail(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(char::is_control)
        || !trimmed.is_ascii()
    {
        return String::new();
    }
    trimmed.chars().take(MAX_ERROR_DETAIL_CHARS).collect()
}

/// Décode un DTO d'édition borné.
///
/// # Errors
///
/// [`SaveError::PayloadTooLarge`] au-delà de [`MAX_EDIT_BYTES`],
/// [`SaveError::InvalidPayload`] si le JSON ne correspond pas exactement au
/// DTO.
pub fn decode_edit(json: &str) -> Result<ProfileEdit, SaveError> {
    if json.len() > MAX_EDIT_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    serde_json::from_str(json).map_err(|_| SaveError::InvalidPayload)
}

/// Résultat d'une édition : collection complète prête à être enregistrée.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedProfiles {
    /// Collection validée avec la modification appliquée.
    pub collection: ProfileCollection,
    /// JSON canonique envoyé à `SaveProfilesJSON`.
    pub json: String,
    /// Identifiant du profil créé ou dupliqué (sélection de l'éditeur).
    pub new_profile_id: Option<ProfileId>,
}

/// Applique le DTO sur la collection et revalide l'ensemble.
///
/// # Errors
///
/// [`SaveError::NotFound`], [`SaveError::PresetReadOnly`],
/// [`SaveError::Validation`] (champ local ou `tcc-core`) ou
/// [`SaveError::PayloadTooLarge`] si la collection sérialisée dépasse 1 Mio.
pub fn apply_edit(
    collection: &ProfileCollection,
    id: &str,
    edit: &ProfileEdit,
) -> Result<PreparedProfiles, SaveError> {
    let id = ProfileId::new(id);
    if validate_profile_id(&id).is_err() {
        return Err(SaveError::Validation {
            field: "id".to_owned(),
        });
    }
    edit.validate()?;

    let mut updated = collection.clone();
    let Some(profile) = updated.get_mut(&id) else {
        return Err(SaveError::NotFound);
    };
    if profile.is_preset() {
        return Err(SaveError::PresetReadOnly);
    }
    apply_fields(profile, edit);

    updated.validate().map_err(|error| SaveError::Validation {
        field: error
            .issues()
            .first()
            .map(|issue| issue.field.clone())
            .unwrap_or_else(|| "profiles".to_owned()),
    })?;

    let json = serde_json::to_string(&updated).map_err(|_| SaveError::InvalidPayload)?;
    if json.len() > MAX_PROFILES_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    Ok(PreparedProfiles {
        collection: updated,
        json,
        new_profile_id: None,
    })
}

/// Vérifie que la collection n'a pas changé puis prépare l'édition.
///
/// `base` est la collection sur laquelle le formulaire a été chargé, `current`
/// la relecture juste avant l'envoi. Une différence annule l'enregistrement
/// (`Conflict`) : sans révision/etag côté `Admin1`, c'est la seule protection
/// contre l'écrasement d'un autre client (course résiduelle documentée).
///
/// # Errors
///
/// [`SaveError::Conflict`] si les collections diffèrent, puis les erreurs de
/// [`apply_edit`].
pub fn prepare_save(
    base: &ProfileCollection,
    current: &ProfileCollection,
    id: &str,
    edit_json: &str,
) -> Result<PreparedProfiles, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    let edit = decode_edit(edit_json)?;
    apply_edit(base, id, &edit)
}

/// Nom normalisé pour la comparaison d'unicité (comme `tcc-core`).
fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Vrai si un autre profil porte déjà ce nom.
fn name_taken(collection: &ProfileCollection, name: &str) -> bool {
    let key = normalized_name(name);
    collection
        .iter()
        .any(|profile| normalized_name(&profile.name) == key)
}

/// Tronque à `max` caractères (pas octets), pour garder le suffixe.
fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Nom unique pour une copie : `X (copie)`, `X (copie 2)`, …
///
/// Le nom final respecte [`MAX_PROFILE_NAME_LEN`] : le nom source est tronqué
/// si nécessaire pour laisser la place au suffixe.
#[must_use]
pub fn unique_copy_name(collection: &ProfileCollection, source_name: &str) -> String {
    let source = source_name.trim();
    for counter in 1..=MAX_PROFILES + 1 {
        let suffix = if counter == 1 {
            " (copie)".to_owned()
        } else {
            format!(" (copie {counter})")
        };
        let keep = MAX_PROFILE_NAME_LEN.saturating_sub(suffix.chars().count());
        let candidate = format!("{}{}", truncate_chars(source, keep), suffix);
        if !name_taken(collection, &candidate) {
            return candidate;
        }
    }
    // Inatteignable avec `MAX_PROFILES` profils, mais reste déterministe.
    format!("{}{}", truncate_chars(source, 32), " (copie)")
}

/// Vérifie la base, la limite de collection et sérialise la collection mise à jour.
fn finish_prepared(
    base: &ProfileCollection,
    current: &ProfileCollection,
    updated: ProfileCollection,
    new_profile_id: Option<ProfileId>,
) -> Result<PreparedProfiles, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    if updated.len() > MAX_PROFILES {
        return Err(SaveError::TooManyProfiles);
    }
    updated.validate().map_err(|error| SaveError::Validation {
        field: error
            .issues()
            .first()
            .map(|issue| issue.field.clone())
            .unwrap_or_else(|| "profiles".to_owned()),
    })?;
    let json = serde_json::to_string(&updated).map_err(|_| SaveError::InvalidPayload)?;
    if json.len() > MAX_PROFILES_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    Ok(PreparedProfiles {
        collection: updated,
        json,
        new_profile_id,
    })
}

/// Ajoute un nouveau profil (ID généré côté Rust).
///
/// Le DTO décrit les champs phase 1 ; les autres sections gardent les valeurs
/// par défaut de `tcc_core::Profile`. Le nom doit être unique (comparaison
/// insensible à la casse et aux espaces).
///
/// # Errors
///
/// [`SaveError::Conflict`], [`SaveError::TooManyProfiles`],
/// [`SaveError::Validation`] (nom dupliqué, bornes) ou
/// [`SaveError::PayloadTooLarge`].
pub fn prepare_create(
    base: &ProfileCollection,
    current: &ProfileCollection,
    edit_json: &str,
) -> Result<PreparedProfiles, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    if base.len() >= MAX_PROFILES {
        return Err(SaveError::TooManyProfiles);
    }
    let edit = decode_edit(edit_json)?;
    edit.validate()?;
    if name_taken(base, &edit.name) {
        return Err(SaveError::Validation {
            field: "name".to_owned(),
        });
    }

    let mut profile = Profile {
        id: ProfileId::generate(),
        ..Profile::default()
    };
    apply_fields(&mut profile, &edit);
    let new_id = profile.id.clone();

    let mut updated = base.clone();
    updated.upsert(profile);
    finish_prepared(base, current, updated, Some(new_id))
}

/// Duplique un profil personnalisé : clone profond, nouvel ID, nom unique.
///
/// # Errors
///
/// [`SaveError::Conflict`], [`SaveError::NotFound`],
/// [`SaveError::PresetReadOnly`] ou [`SaveError::TooManyProfiles`].
pub fn prepare_duplicate(
    base: &ProfileCollection,
    current: &ProfileCollection,
    source_id: &str,
) -> Result<PreparedProfiles, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    if base.len() >= MAX_PROFILES {
        return Err(SaveError::TooManyProfiles);
    }
    let id = ProfileId::new(source_id);
    if validate_profile_id(&id).is_err() {
        return Err(SaveError::NotFound);
    }
    let source = base.get(&id).ok_or(SaveError::NotFound)?;
    if source.is_preset() {
        return Err(SaveError::PresetReadOnly);
    }

    let mut clone = source.clone();
    clone.id = ProfileId::generate();
    clone.name = unique_copy_name(base, &source.name);
    let new_id = clone.id.clone();

    let mut updated = base.clone();
    updated.upsert(clone);
    finish_prepared(base, current, updated, Some(new_id))
}

/// Supprime un profil personnalisé.
///
/// Les références d'état (`stateMap`/`stateMatrix`) ne sont pas connues de la
/// GUI au moment de la suppression : `tccd-admin` refuse avec `InUse` et rien
/// n'est écrit. Les IDs supprimés ne sont pas réutilisés (pas de tombstones).
///
/// # Errors
///
/// [`SaveError::Conflict`], [`SaveError::NotFound`] ou
/// [`SaveError::PresetReadOnly`].
pub fn prepare_delete(
    base: &ProfileCollection,
    current: &ProfileCollection,
    id: &str,
) -> Result<PreparedProfiles, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    let profile_id = ProfileId::new(id);
    if validate_profile_id(&profile_id).is_err() {
        return Err(SaveError::NotFound);
    }
    let profile = base.get(&profile_id).ok_or(SaveError::NotFound)?;
    if profile.is_preset() {
        return Err(SaveError::PresetReadOnly);
    }
    let mut updated = base.clone();
    updated.remove(&profile_id);
    finish_prepared(base, current, updated, None)
}

/// Applique les champs phase 1 sur un profil déjà validé par l'appelant.
fn apply_fields(profile: &mut Profile, edit: &ProfileEdit) {
    profile.name = edit.name.trim().to_owned();
    profile.description = edit.description.trim_end().to_owned();

    profile.display.brightness = edit.brightness;
    profile.display.use_brightness = edit.use_brightness;
    profile.display.refresh_rate = edit.refresh_rate;
    profile.display.use_refresh_rate = edit.use_refresh_rate;
    if let Some(use_resolution) = edit.use_resolution {
        profile.display.use_resolution = use_resolution;
        // La validation impose les deux dimensions quand la résolution est
        // demandée ; `Some(false)`/`None` conservent les valeurs existantes.
        if use_resolution {
            if let (Some(width), Some(height)) = (edit.x_resolution, edit.y_resolution) {
                profile.display.x_resolution = Some(width);
                profile.display.y_resolution = Some(height);
            }
        }
    }

    // Champs `Option` de `tcc-core` : on ne les réécrit que si l'utilisateur
    // les a explicitement fournis. Un renommage ne doit jamais brider le CPU
    // ni inventer une fréquence.
    if let Some(cores) = edit.online_cores {
        profile.cpu.online_cores = Some(cores);
    }
    profile.cpu.use_max_perf_gov = edit.use_max_perf_gov;
    if let Some(min) = edit.scaling_min_khz {
        profile.cpu.scaling_min_frequency = Some(min);
    }
    if let Some(max) = edit.scaling_max_khz {
        profile.cpu.scaling_max_frequency = Some(if max == -1 {
            FrequencyLimit::Reduced
        } else {
            FrequencyLimit::Hz(u64::try_from(max).unwrap_or(0))
        });
    }
    profile.cpu.energy_performance_preference = edit.energy_performance_preference.clone();
    profile.cpu.no_turbo = edit.no_turbo;

    profile.webcam.status = edit.webcam_status;
    profile.webcam.use_status = edit.use_webcam_status;

    profile.fan.use_control = edit.fan_use_control;
    profile.fan.fan_profile = canonical_fan_profile(&edit.fan_profile);
    profile.fan.minimum_fanspeed = edit.fan_min;
    profile.fan.maximum_fanspeed = edit.fan_max;
    profile.fan.offset_fanspeed = edit.fan_offset;
    if let Some(curve) = &edit.custom_fan_curve {
        // Seule la table CPU est éditée : le nom et la table GPU de la courbe
        // existante sont conservés tels quels.
        profile.fan.custom_fan_curve.table_cpu = curve
            .table_cpu
            .iter()
            .map(|point| FanTableEntry::new(point.temp, point.speed))
            .collect();
    }

    // ODM/TDP/cTGP : `None` = ne pas changer ; les valeurs fournies sont
    // appliquées telles quelles après validation.
    if let Some(name) = &edit.odm_profile {
        profile.odm_profile.name = Some(name.trim().to_owned());
    }
    if let Some(values) = &edit.tdp_values {
        profile.odm_power_limits.tdp_values = values.clone();
    }
    if let Some(offset) = edit.ctgp_offset {
        profile.nvidia_power_ctrl_profile.ctgp_offset = offset;
    }
}

/// Convertit la clé i18n du profil de ventilateur en nom du contrat.
///
/// `auto` (contrôle désactivé) conserve un nom neutre : le daemon ignore le
/// nom quand `useControl` est faux.
fn canonical_fan_profile(key: &str) -> String {
    match key {
        "silent" => "Silent",
        "quiet" => "Quiet",
        "performance" => "Performance",
        "custom" => "Custom",
        _ => "Balanced",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_EDIT_BYTES, MAX_PROFILES_PAYLOAD_BYTES, ProfileEdit, SaveError, apply_edit,
        decode_edit, prepare_save,
    };
    use tcc_core::{Profile, ProfileCollection, ProfileId};

    fn collection() -> ProfileCollection {
        ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Studio"),
            Profile::new("custom-b", "Gaming"),
        ])
    }

    fn edit_json(name: &str) -> String {
        format!(
            r#"{{
                "name": "{name}",
                "description": "Description",
                "brightness": 70,
                "useBrightness": true,
                "refreshRate": 120,
                "useRefreshRate": true,
                "onlineCores": 6,
                "useMaxPerfGov": false,
                "scalingMinKhz": 800000,
                "scalingMaxKhz": 3500000,
                "energyPerformancePreference": "balance_power",
                "noTurbo": true,
                "webcamStatus": false,
                "useWebcamStatus": true,
                "fanUseControl": true,
                "fanProfile": "quiet",
                "fanMin": 10,
                "fanMax": 55,
                "fanOffset": 0
            }}"#
        )
    }

    fn edit(name: &str) -> ProfileEdit {
        decode_edit(&edit_json(name)).expect("DTO valide")
    }

    #[test]
    fn dto_roundtrip_and_apply() {
        let edit = edit("Studio 2");
        let prepared = apply_edit(&collection(), "custom-a", &edit).unwrap();
        let profile = prepared
            .collection
            .get_by_id("custom-a")
            .expect("profil modifié");
        assert_eq!(profile.name, "Studio 2");
        assert_eq!(profile.display.brightness, 70);
        assert_eq!(profile.display.refresh_rate, Some(120.0));
        assert_eq!(profile.cpu.online_cores, Some(6));
        assert_eq!(profile.cpu.scaling_min_frequency, Some(800_000));
        assert_eq!(
            profile.cpu.scaling_max_frequency,
            Some(tcc_core::FrequencyLimit::Hz(3_500_000))
        );
        assert_eq!(profile.fan.fan_profile, "Quiet");
        assert_eq!(profile.fan.minimum_fanspeed, 10.0);
        assert!(prepared.json.len() <= MAX_PROFILES_PAYLOAD_BYTES);
        // Les autres profils sont préservés.
        assert!(prepared.collection.get_by_id("custom-b").is_some());
        assert!(prepared.collection.get_by_id("__office__").is_some());
    }

    #[test]
    fn preset_is_read_only() {
        let error = apply_edit(&collection(), "__office__", &edit("Renamed")).unwrap_err();
        assert_eq!(error, SaveError::PresetReadOnly);
    }

    #[test]
    fn unknown_profile_is_rejected() {
        let error = apply_edit(&collection(), "custom-zzz", &edit("X")).unwrap_err();
        assert_eq!(error, SaveError::NotFound);
    }

    #[test]
    fn duplicate_name_is_rejected() {
        // « gaming » existe déjà (comparaison insensible à la casse).
        let error = apply_edit(&collection(), "custom-a", &edit("GAMING")).unwrap_err();
        assert_eq!(
            error,
            SaveError::Validation {
                field: "profiles[2].name".to_owned()
            }
        );
    }

    #[test]
    fn empty_name_is_rejected_locally() {
        let error = apply_edit(&collection(), "custom-a", &edit("   ")).unwrap_err();
        assert_eq!(
            error,
            SaveError::Validation {
                field: "name".to_owned()
            }
        );
    }

    #[test]
    fn inverted_frequencies_are_rejected() {
        let mut edit = edit("Studio");
        edit.scaling_min_khz = Some(4_000_000);
        edit.scaling_max_khz = Some(2_000_000);
        let error = apply_edit(&collection(), "custom-a", &edit).unwrap_err();
        assert_eq!(
            error,
            SaveError::Validation {
                field: "scalingMaxFrequency".to_owned()
            }
        );
    }

    #[test]
    fn inverted_fan_speeds_are_rejected() {
        let mut edit = edit("Studio");
        edit.fan_min = 80.0;
        edit.fan_max = 20.0;
        let error = apply_edit(&collection(), "custom-a", &edit).unwrap_err();
        assert_eq!(
            error,
            SaveError::Validation {
                field: "minimumFanspeed".to_owned()
            }
        );
    }

    #[test]
    fn unknown_field_in_dto_is_rejected() {
        let json = edit_json("Studio").replace('}', r#", "tdp": 42}"#);
        assert_eq!(decode_edit(&json).unwrap_err(), SaveError::InvalidPayload);
    }

    #[test]
    fn oversized_edit_is_rejected_before_parsing() {
        let json = "x".repeat(MAX_EDIT_BYTES + 1);
        assert_eq!(decode_edit(&json).unwrap_err(), SaveError::PayloadTooLarge);
    }

    #[test]
    fn truncated_dto_is_rejected() {
        // `brightness` est obligatoire (contrairement aux champs Option).
        let json = edit_json("Studio").replace("\"brightness\": 70,", "");
        assert_eq!(decode_edit(&json).unwrap_err(), SaveError::InvalidPayload);
    }

    #[test]
    fn reduced_max_frequency_is_preserved() {
        let mut edit = edit("Studio");
        edit.scaling_max_khz = Some(-1);
        let prepared = apply_edit(&collection(), "custom-a", &edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert_eq!(
            profile.cpu.scaling_max_frequency,
            Some(tcc_core::FrequencyLimit::Reduced)
        );
    }

    #[test]
    fn resolution_is_applied_only_when_requested() {
        let mut first_edit = edit("Studio");
        first_edit.use_resolution = Some(true);
        first_edit.x_resolution = Some(2880);
        first_edit.y_resolution = Some(1800);
        let prepared = apply_edit(&collection(), "custom-a", &first_edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert!(profile.display.use_resolution);
        assert_eq!(profile.display.x_resolution, Some(2880));
        assert_eq!(profile.display.y_resolution, Some(1800));

        // Sans demande, les valeurs existantes sont préservées.
        let base = prepared.collection;
        let mut next_edit = edit("Studio");
        next_edit.use_resolution = Some(false);
        next_edit.x_resolution = None;
        next_edit.y_resolution = None;
        let prepared = apply_edit(&base, "custom-a", &next_edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert!(!profile.display.use_resolution);
        assert_eq!(profile.display.x_resolution, Some(2880));
        assert_eq!(profile.display.y_resolution, Some(1800));
    }

    #[test]
    fn resolution_dto_is_decoded_and_preserved_when_absent() {
        let json = r#"{
                "name": "Studio",
                "description": "",
                "brightness": 70,
                "useBrightness": true,
                "refreshRate": 165,
                "useRefreshRate": true,
                "useResolution": true,
                "xResolution": 2880,
                "yResolution": 1800,
                "useMaxPerfGov": false,
                "energyPerformancePreference": "balance_power",
                "noTurbo": false,
                "webcamStatus": true,
                "useWebcamStatus": true,
                "fanUseControl": true,
                "fanProfile": "balanced",
                "fanMin": 0,
                "fanMax": 100,
                "fanOffset": 0
            }"#;
        let edit = decode_edit(json).expect("DTO résolution");
        let prepared = apply_edit(&collection(), "custom-a", &edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert!(profile.display.use_resolution);
        assert_eq!(profile.display.x_resolution, Some(2880));
        assert_eq!(profile.display.y_resolution, Some(1800));

        // DTO sans résolution (ancien client) : le drapeau et les dimensions
        // existants ne sont pas effacés.
        let mut base = Profile::new("custom-a", "Studio");
        base.display.use_resolution = true;
        base.display.x_resolution = Some(2880);
        base.display.y_resolution = Some(1800);
        let base = ProfileCollection::new(vec![base]);
        let minimal = edit_json("Studio");
        let edit = decode_edit(&minimal).expect("DTO minimal");
        let prepared = apply_edit(&base, "custom-a", &edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert!(profile.display.use_resolution);
        assert_eq!(profile.display.x_resolution, Some(2880));
        assert_eq!(profile.display.y_resolution, Some(1800));
    }

    fn odm_edit(fields: &str) -> ProfileEdit {
        let json = format!(
            r#"{{
                "name": "Studio",
                "description": "",
                "brightness": 70,
                "useBrightness": true,
                "refreshRate": 120,
                "useRefreshRate": true,
                "useMaxPerfGov": false,
                "energyPerformancePreference": "balance_power",
                "noTurbo": false,
                "webcamStatus": true,
                "useWebcamStatus": true,
                "fanUseControl": true,
                "fanProfile": "balanced",
                "fanMin": 0,
                "fanMax": 100,
                "fanOffset": 0,
                {fields}
            }}"#
        );
        decode_edit(&json).expect("DTO ODM")
    }

    #[test]
    fn odm_tdp_and_ctgp_are_applied_and_preserved() {
        let mut profile = Profile::new("custom-a", "Studio");
        profile.odm_profile.name = Some("balanced".to_owned());
        profile.odm_power_limits.tdp_values = vec![35.0, 45.0];
        profile.nvidia_power_ctrl_profile.ctgp_offset = 5.0;
        let base = ProfileCollection::new(vec![profile]);

        let edit = odm_edit(
            r#""odmProfile": "enthusiast",
               "tdpValues": [40.0, 50.0, 65.0],
               "ctgpOffset": 12.0"#,
        );
        let prepared = apply_edit(&base, "custom-a", &edit).expect("application");
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert_eq!(profile.odm_profile.name.as_deref(), Some("enthusiast"));
        assert_eq!(profile.odm_power_limits.tdp_values, vec![40.0, 50.0, 65.0]);
        assert_eq!(profile.nvidia_power_ctrl_profile.ctgp_offset, 12.0);

        // `None`/absent préserve les trois champs.
        let edit = odm_edit(r#""odmProfile": null, "tdpValues": null, "ctgpOffset": null"#);
        let prepared = apply_edit(&base, "custom-a", &edit).expect("préservation");
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert_eq!(profile.odm_profile.name.as_deref(), Some("balanced"));
        assert_eq!(profile.odm_power_limits.tdp_values, vec![35.0, 45.0]);
        assert_eq!(profile.nvidia_power_ctrl_profile.ctgp_offset, 5.0);

        // `[]` efface explicitement les limites TDP.
        let edit = odm_edit(r#""tdpValues": []"#);
        let prepared = apply_edit(&base, "custom-a", &edit).expect("effacement");
        assert!(
            prepared
                .collection
                .get_by_id("custom-a")
                .unwrap()
                .odm_power_limits
                .tdp_values
                .is_empty()
        );
    }

    #[test]
    fn odm_tdp_and_ctgp_bounds_are_enforced() {
        for fields in [
            r#""odmProfile": """#,
            r#""odmProfile": "bad name""#,
            r#""odmProfile": "a.b""#,
            r#""tdpValues": [10.0, 20.0, 30.0, 40.0]"#,
            r#""tdpValues": [30.0, 20.0]"#,
            r#""tdpValues": [30.0, 30.0]"#,
            r#""tdpValues": [0.0]"#,
            r#""tdpValues": [1001.0]"#,
            r#""ctgpOffset": -1.0"#,
            r#""ctgpOffset": 201.0"#,
        ] {
            let edit = odm_edit(fields);
            let error = apply_edit(&collection(), "custom-a", &edit).expect_err("refus");
            assert_eq!(error.code(), "Validation");
        }
    }

    #[test]
    fn resolution_requires_both_bounded_dimensions() {
        for (width, height) in [
            (None, Some(1080)),
            (Some(1920), None),
            (Some(0), Some(1080)),
            (Some(19_200), Some(1080)),
        ] {
            let mut edit = edit("Studio");
            edit.use_resolution = Some(true);
            edit.x_resolution = width;
            edit.y_resolution = height;
            let error = apply_edit(&collection(), "custom-a", &edit).expect_err("refus");
            assert_eq!(error.code(), "Validation");
            assert_eq!(error.detail(), "resolution");
        }
    }

    #[test]
    fn rename_only_keeps_absent_optional_fields() {
        // Profil sans `onlineCores` ni fréquences explicites.
        let bare = ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Studio"),
        ]);
        // DTO minimal : seuls les champs non optionnels sont fournis.
        let json = r#"{
            "name": "Studio renommé",
            "description": "Description",
            "brightness": 70,
            "useBrightness": true,
            "refreshRate": null,
            "useRefreshRate": false,
            "useMaxPerfGov": false,
            "energyPerformancePreference": "balance_power",
            "noTurbo": false,
            "webcamStatus": true,
            "useWebcamStatus": true,
            "fanUseControl": true,
            "fanProfile": "balanced",
            "fanMin": 0,
            "fanMax": 100,
            "fanOffset": 0
        }"#;
        let edit = decode_edit(json).expect("DTO minimal");
        let prepared = apply_edit(&bare, "custom-a", &edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert_eq!(profile.name, "Studio renommé");
        // Aucune valeur inventée : les `Option` restent `None`.
        assert_eq!(profile.cpu.online_cores, None);
        assert_eq!(profile.cpu.scaling_min_frequency, None);
        assert_eq!(profile.cpu.scaling_max_frequency, None);
    }

    #[test]
    fn rename_only_keeps_known_optional_fields() {
        let mut profile = Profile::new("custom-a", "Studio");
        profile.cpu.online_cores = Some(8);
        profile.cpu.scaling_min_frequency = Some(800_000);
        profile.cpu.scaling_max_frequency = Some(tcc_core::FrequencyLimit::Hz(3_500_000));
        let base = ProfileCollection::new(vec![profile]);

        let json = r#"{
            "name": "Studio renommé",
            "description": "",
            "brightness": 70,
            "useBrightness": true,
            "refreshRate": null,
            "useRefreshRate": false,
            "useMaxPerfGov": false,
            "energyPerformancePreference": "balance_power",
            "noTurbo": false,
            "webcamStatus": true,
            "useWebcamStatus": true,
            "fanUseControl": true,
            "fanProfile": "balanced",
            "fanMin": 0,
            "fanMax": 100,
            "fanOffset": 0
        }"#;
        let edit = decode_edit(json).expect("DTO minimal");
        let prepared = apply_edit(&base, "custom-a", &edit).unwrap();
        let profile = prepared.collection.get_by_id("custom-a").unwrap();
        assert_eq!(profile.cpu.online_cores, Some(8));
        assert_eq!(profile.cpu.scaling_min_frequency, Some(800_000));
        assert_eq!(
            profile.cpu.scaling_max_frequency,
            Some(tcc_core::FrequencyLimit::Hz(3_500_000))
        );
    }

    #[test]
    fn invalid_reduced_sentinel_is_rejected() {
        let mut edit = edit("Studio");
        edit.scaling_max_khz = Some(-2);
        assert_eq!(
            apply_edit(&collection(), "custom-a", &edit).unwrap_err(),
            SaveError::Validation {
                field: "scalingMaxFrequency".to_owned()
            }
        );
    }

    #[test]
    fn concurrent_change_is_a_conflict() {
        let base = collection();
        let mut current = base.clone();
        // Un autre client renomme un profil non édité.
        current
            .get_mut(&ProfileId::new("custom-b"))
            .expect("profil")
            .name = "Gaming (autre client)".to_owned();
        let error = prepare_save(&base, &current, "custom-a", &edit_json("Studio"))
            .expect_err("conflit attendu");
        assert_eq!(error, SaveError::Conflict);
        // Collections identiques : l'édition est préparée.
        assert!(prepare_save(&base, &base, "custom-a", &edit_json("Studio")).is_ok());
    }

    #[test]
    fn error_codes_and_details_are_safe() {
        assert_eq!(SaveError::PresetReadOnly.code(), "PresetReadOnly");
        assert_eq!(SaveError::PresetReadOnly.detail(), "");
        assert_eq!(
            SaveError::Admin {
                code: "Validation",
                detail: "profiles[0].name".to_owned()
            }
            .detail(),
            "profiles[0].name"
        );
        // Un chemin ou un contenu de fichier n'est jamais relayé.
        assert_eq!(
            SaveError::Admin {
                code: "WriteFailed",
                detail: "/var/lib/tuxedo-rust-center/profiles.json".to_owned()
            }
            .detail(),
            ""
        );
        assert_eq!(
            SaveError::Validation {
                field: "a\u{7}b".to_owned()
            }
            .detail(),
            ""
        );
    }

    fn edit_json_with_curve(points: &[(f32, f32)]) -> String {
        let points_json: Vec<String> = points
            .iter()
            .map(|(temp, speed)| format!(r#"{{"temp":{temp},"speed":{speed}}}"#))
            .collect();
        edit_json("Studio").replace(
            r#""fanOffset": 0"#,
            &format!(
                r#""fanOffset": 0, "customFanCurve": {{"tableCPU":[{}]}}"#,
                points_json.join(",")
            ),
        )
    }

    fn curve_profile() -> ProfileCollection {
        let mut collection = collection();
        let profile = collection
            .get_mut(&ProfileId::new("custom-a"))
            .expect("profil");
        profile.fan.custom_fan_curve.name = Some("Studio curve".to_owned());
        profile.fan.custom_fan_curve.table_gpu = vec![tcc_core::FanTableEntry::new(40.0, 30.0)];
        collection
    }

    #[test]
    fn custom_curve_updates_cpu_and_preserves_name_and_gpu() {
        let base = curve_profile();
        let json = edit_json_with_curve(&[(30.0, 20.0), (55.0, 45.0), (80.0, 90.0)]);
        let prepared = prepare_save(&base, &base, "custom-a", &json).expect("édition");
        let profile = prepared
            .collection
            .get(&ProfileId::new("custom-a"))
            .expect("profil");
        assert_eq!(
            profile
                .fan
                .custom_fan_curve
                .table_cpu
                .iter()
                .map(|point| (point.temp, point.speed))
                .collect::<Vec<_>>(),
            vec![(30.0, 20.0), (55.0, 45.0), (80.0, 90.0)]
        );
        // GPU et nom conservés.
        assert_eq!(
            profile.fan.custom_fan_curve.name.as_deref(),
            Some("Studio curve")
        );
        assert_eq!(
            profile.fan.custom_fan_curve.table_gpu,
            vec![tcc_core::FanTableEntry::new(40.0, 30.0)]
        );
        // Sérialisation attendue par le daemon.
        assert!(prepared.json.contains(r#""customFanCurve""#));
        assert!(prepared.json.contains(r#""tableCPU""#));
        assert!(prepared.json.contains(r#""tableGPU""#));
    }

    #[test]
    fn absent_curve_never_changes_existing_curve() {
        let base = curve_profile();
        let before = base
            .get(&ProfileId::new("custom-a"))
            .expect("profil")
            .fan
            .custom_fan_curve
            .clone();
        let prepared =
            prepare_save(&base, &base, "custom-a", &edit_json("Studio")).expect("édition");
        let after = &prepared
            .collection
            .get(&ProfileId::new("custom-a"))
            .expect("profil")
            .fan
            .custom_fan_curve;
        assert_eq!(after, &before);
    }

    #[test]
    fn invalid_custom_curves_are_refused() {
        let base = curve_profile();
        let mut too_many: Vec<(f32, f32)> =
            (0..257).map(|index| (index as f32 * 0.3, 50.0)).collect();
        too_many.truncate(tcc_core::MAX_FAN_TABLE_ENTRIES + 1);
        for points in [
            vec![(50.0, 40.0)],
            vec![(60.0, 40.0), (50.0, 45.0)],
            vec![(50.0, 40.0), (50.0, 45.0)],
            vec![(50.0, 40.0), (101.0, 45.0)],
            vec![(50.0, 40.0), (60.0, 101.0)],
            too_many,
        ] {
            let json = edit_json_with_curve(&points);
            let error = prepare_save(&base, &base, "custom-a", &json).unwrap_err();
            assert_eq!(
                error,
                SaveError::Validation {
                    field: "fanCurve".to_owned()
                }
            );
        }
    }

    #[test]
    fn custom_curve_shape_is_strict() {
        for payload in [
            r#"{"tableGPU":[{"temp":40,"speed":30}]}"#,
            r#"[{"temp":40,"speed":30}]"#,
            r#"{"tableCPU":[{"temp":"chaud","speed":30}]}"#,
            r#"{"tableCPU":[{"temp":40,"speed":30,"extra":1}]}"#,
        ] {
            let json = edit_json("Studio").replace(
                r#""fanOffset": 0"#,
                &format!(r#""fanOffset": 0, "customFanCurve": {payload}"#),
            );
            assert_eq!(decode_edit(&json).unwrap_err(), SaveError::InvalidPayload);
        }
    }

    #[test]
    fn cancelled_edit_keeps_original_collection() {
        let original = collection();
        let before = original.clone();
        assert!(apply_edit(&original, "custom-a", &edit("Gaming")).is_err());
        assert_eq!(original, before);
    }

    #[test]
    fn id_must_be_valid() {
        let error = apply_edit(&collection(), "../etc", &edit("X")).unwrap_err();
        assert_eq!(
            error,
            SaveError::Validation {
                field: "id".to_owned()
            }
        );
        assert!(ProfileId::new("custom-a").as_str() == "custom-a");
    }
}
