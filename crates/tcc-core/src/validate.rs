// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Il s'agit d'une
// réimplémentation originale de la validation des profils et réglages de
// TUXEDO Control Center (GPL-3.0-or-later), durcie selon
// `docs/security/DECISIONS.md` (bornes explicites, refus des valeurs non
// finies, identifiants sûrs).

//! Validation centralisée des profils, réglages et courbes de ventilation.
//!
//! Toute entrée non fiable — fichier migré, import utilisateur, futur message
//! D-Bus — passe par ces fonctions avant d'être appliquée ou écrite. Les
//! bornes statiques sont volontairement larges : `tcc-core` ne connaît pas le
//! matériel, et `tcc-hw` affine ensuite avec les capacités réelles (par
//! exemple `maxBrightness`). Une valeur hors de ces bornes est **rejetée**,
//! jamais silencieusement ramenée dans la plage : un clamp caché masque un
//! bug et fausse l'UI.
//!
//! Les anomalies sont agrégées : un import peut signaler tous ses champs
//! fautifs en une seule passe. Aucune fonction ne panique sur une entrée
//! invalide.

use std::collections::BTreeSet;

use crate::error::{ValidationError, ValidationIssue};
use crate::fan::{FanCurve, FanTableSet, MAX_FAN_TABLE_ENTRIES};
use crate::profile::{Profile, ProfileCollection, ProfileId};
use crate::settings::{SETTINGS_SCHEMA_VERSION, Settings};

/// Longueur maximale d'un identifiant de profil.
pub const MAX_PROFILE_ID_LEN: usize = 64;
/// Longueur maximale d'un nom de profil.
pub const MAX_PROFILE_NAME_LEN: usize = 128;
/// Longueur maximale d'une description de profil.
pub const MAX_DESCRIPTION_LEN: usize = 4096;
/// Nombre maximal de profils dans une collection.
pub const MAX_PROFILES: usize = 256;
/// Nombre maximal d'identifiants de profils supprimés (tombstones).
///
/// Un ID supprimé reste réservé : la liste peut dépasser le nombre de profils
/// vivants, mais reste bornée pour éviter un fichier de configuration
/// illimité.
pub const MAX_DELETED_PROFILE_IDS: usize = 1024;
/// Longueur maximale d'un type de charge (`chargingType`).
pub const MAX_CHARGING_TYPE_LEN: usize = 64;
/// Nombre maximal de courbes de ventilation nommées.
pub const MAX_FAN_CURVES: usize = 64;
/// Nombre maximal d'états de zones clavier.
pub const MAX_KEYBOARD_STATES: usize = 64;
/// Nombre maximal de cartes dans le workaround YCbCr.
pub const MAX_YCBCR_CARDS: usize = 16;
/// Nombre maximal de ports par carte dans le workaround YCbCr.
pub const MAX_YCBCR_PORTS: usize = 64;
/// Nombre maximal de valeurs TDP dans un profil.
pub const MAX_TDP_VALUES: usize = 16;

/// Taux de rafraîchissement minimal accepté, en Hz.
///
/// La borne sécurité est `1..=1000` ; `tcc-hw`/`tcc-user` vérifient ensuite
/// que le mode existe réellement dans la liste du compositeur.
pub const MIN_REFRESH_RATE_HZ: f32 = 1.0;
/// Taux de rafraîchissement maximal accepté, en Hz.
pub const MAX_REFRESH_RATE_HZ: f32 = 1000.0;
/// Résolution maximale acceptée pour un écran interne.
pub const MAX_RESOLUTION: u32 = 16_384;
/// Nombre maximal de cœurs CPU adressables.
pub const MAX_CPU_ONLINE_CORES: u32 = 1024;
/// Fréquence CPU minimale acceptée, en kHz (100 MHz).
pub const MIN_CPU_FREQUENCY_KHZ: u64 = 100_000;
/// Fréquence CPU maximale acceptée, en kHz (10 GHz).
pub const MAX_CPU_FREQUENCY_KHZ: u64 = 10_000_000;
/// Puissance TDP minimale acceptée, en watts.
pub const MIN_TDP_WATTS: f64 = 1.0;
/// Puissance TDP maximale acceptée, en watts.
pub const MAX_TDP_WATTS: f64 = 1000.0;
/// Offset cTGP maximal accepté, en watts.
pub const MAX_CTGP_OFFSET_WATTS: f64 = 200.0;
/// Température minimale acceptée dans une courbe, en °C.
pub const MIN_FAN_TEMP_C: f32 = -40.0;
/// Température maximale acceptée dans une courbe, en °C.
pub const MAX_FAN_TEMP_C: f32 = 200.0;

fn issue(issues: &mut Vec<ValidationIssue>, field: impl Into<String>, message: impl Into<String>) {
    issues.push(ValidationIssue::new(field, message));
}

fn validate_finite_f32(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: f32,
    min: f32,
    max: f32,
) {
    if !value.is_finite() {
        issue(issues, field, "doit être une valeur finie");
    } else if !(min..=max).contains(&value) {
        issue(
            issues,
            field,
            format!("doit être compris entre {min} et {max} (reçu {value})"),
        );
    }
}

fn validate_finite_f64(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: f64,
    min: f64,
    max: f64,
) {
    if !value.is_finite() {
        issue(issues, field, "doit être une valeur finie");
    } else if !(min..=max).contains(&value) {
        issue(
            issues,
            field,
            format!("doit être compris entre {min} et {max} (reçu {value})"),
        );
    }
}

fn validate_optional_u32_range(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: Option<u32>,
    min: u32,
    max: u32,
) {
    if let Some(value) = value
        && !(min..=max).contains(&value)
    {
        issue(
            issues,
            field,
            format!("doit être compris entre {min} et {max} (reçu {value})"),
        );
    }
}

fn validate_optional_u64_range(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: Option<u64>,
    min: u64,
    max: u64,
) {
    if let Some(value) = value
        && !(min..=max).contains(&value)
    {
        issue(
            issues,
            field,
            format!("doit être compris entre {min} et {max} (reçu {value})"),
        );
    }
}

fn has_control_chars(value: &str, allow_layout: bool) -> bool {
    value.chars().any(|character| {
        character.is_control() && !(allow_layout && matches!(character, '\n' | '\t'))
    })
}

fn validate_label(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: &str,
    max_len: usize,
    required: bool,
) {
    let trimmed = value.trim();
    if required && trimmed.is_empty() {
        issue(issues, field, "ne doit pas être vide");
        return;
    }
    if value.chars().count() > max_len {
        issue(
            issues,
            field,
            format!("ne doit pas dépasser {max_len} caractères"),
        );
    }
    if has_control_chars(value, false) {
        issue(
            issues,
            field,
            "ne doit contenir aucun caractère de contrôle",
        );
    }
}

fn validate_token(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: &str,
    max_len: usize,
    required: bool,
) {
    let trimmed = value.trim();
    if required && trimmed.is_empty() {
        issue(issues, field, "ne doit pas être vide");
        return;
    }
    if value.chars().count() > max_len {
        issue(
            issues,
            field,
            format!("ne doit pas dépasser {max_len} caractères"),
        );
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        issue(
            issues,
            field,
            "ne doit contenir que des lettres ASCII, chiffres, '_' ou '-'",
        );
    }
}

fn validate_optional_token(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: Option<&str>,
    max_len: usize,
) {
    if let Some(value) = value {
        validate_token(issues, field, value, max_len, true);
    }
}

fn validate_profile_id_into(id: &ProfileId, issues: &mut Vec<ValidationIssue>) {
    validate_profile_id_field_into(id, "id", issues);
}

/// Variante de [`validate_profile_id_into`] avec un nom de champ explicite
/// (`deletedProfileIds[i]`, `stateMap.power_ac`, …).
fn validate_profile_id_field_into(id: &ProfileId, field: &str, issues: &mut Vec<ValidationIssue>) {
    let value = id.as_str();
    if value.is_empty() {
        issue(issues, field, "ne doit pas être vide");
        return;
    }
    if value.chars().count() > MAX_PROFILE_ID_LEN {
        issue(
            issues,
            field,
            format!("ne doit pas dépasser {MAX_PROFILE_ID_LEN} caractères"),
        );
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        issue(
            issues,
            field,
            "ne doit contenir que des lettres ASCII, chiffres, '_' ou '-'",
        );
    }
}

/// Valide un identifiant de profil.
///
/// Règles : `^[A-Za-z0-9_-]{1,64}$`, ce qui exclut les séparateurs de chemin,
/// les points, les espaces et les caractères de contrôle.
///
/// # Errors
///
/// Renvoie une erreur si l'identifiant est vide, trop long ou contient un
/// caractère interdit.
pub fn validate_profile_id(id: &ProfileId) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    validate_profile_id_into(id, &mut issues);
    ValidationError::from_issues(issues)
}

/// Valide un nom de profil.
///
/// Le nom peut être Unicode (i18n), mais doit être non vide après `trim`, ne
/// pas dépasser [`MAX_PROFILE_NAME_LEN`] caractères et ne contenir aucun
/// caractère de contrôle.
///
/// # Errors
///
/// Renvoie une erreur si le nom est invalide.
pub fn validate_profile_name(name: &str) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    validate_label(&mut issues, "name", name, MAX_PROFILE_NAME_LEN, true);
    ValidationError::from_issues(issues)
}

/// Valide une description de profil.
///
/// Les retours à la ligne et tabulations sont autorisés ; les autres
/// caractères de contrôle sont refusés.
///
/// # Errors
///
/// Renvoie une erreur si la description est trop longue ou contient un
/// caractère interdit.
pub fn validate_profile_description(description: &str) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        issue(
            &mut issues,
            "description",
            format!("ne doit pas dépasser {MAX_DESCRIPTION_LEN} caractères"),
        );
    }
    if has_control_chars(description, true) {
        issue(
            &mut issues,
            "description",
            "ne doit contenir aucun caractère de contrôle (hors retour à la ligne et tabulation)",
        );
    }
    ValidationError::from_issues(issues)
}

fn validate_fan_curve_into(
    curve: &FanCurve,
    field_prefix: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(name) = &curve.name {
        validate_label(issues, &format!("{field_prefix}.name"), name, 64, true);
    }

    for (table_name, table) in [
        ("tableCPU", &curve.table_cpu),
        ("tableGPU", &curve.table_gpu),
    ] {
        if table.len() > MAX_FAN_TABLE_ENTRIES {
            issue(
                issues,
                format!("{field_prefix}.{table_name}"),
                format!("ne doit pas dépasser {MAX_FAN_TABLE_ENTRIES} points"),
            );
        }
        let mut seen_temps = BTreeSet::new();
        for (index, entry) in table.iter().enumerate() {
            let field = format!("{field_prefix}.{table_name}[{index}]");
            validate_finite_f32(
                issues,
                &format!("{field}.temp"),
                entry.temp,
                MIN_FAN_TEMP_C,
                MAX_FAN_TEMP_C,
            );
            validate_finite_f32(
                issues,
                &format!("{field}.speed"),
                entry.speed,
                crate::fan::FAN_SPEED_MIN_PERCENT,
                crate::fan::FAN_SPEED_MAX_PERCENT,
            );
            if entry.temp.is_finite() && !seen_temps.insert(entry.temp.to_bits()) {
                issue(
                    issues,
                    format!("{field}.temp"),
                    "température dupliquée dans la table",
                );
            }
        }
    }
}

/// Valide une courbe de ventilation (profil personnalisé).
///
/// Une courbe vide est acceptée : elle signifie « pas de courbe
/// personnalisée » et n'est utilisée que si le profil sélectionne `Custom`.
///
/// # Errors
///
/// Renvoie une erreur si un point est hors bornes, non fini ou dupliqué.
pub fn validate_fan_curve(curve: &FanCurve) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    validate_fan_curve_into(curve, "fan.customFanCurve", &mut issues);
    ValidationError::from_issues(issues)
}

fn validate_fan_limits_into(fan: &crate::profile::FanProfile, issues: &mut Vec<ValidationIssue>) {
    validate_finite_f32(
        issues,
        "fan.minimumFanspeed",
        fan.minimum_fanspeed,
        crate::fan::FAN_SPEED_MIN_PERCENT,
        crate::fan::FAN_SPEED_MAX_PERCENT,
    );
    validate_finite_f32(
        issues,
        "fan.maximumFanspeed",
        fan.maximum_fanspeed,
        crate::fan::FAN_SPEED_MIN_PERCENT,
        crate::fan::FAN_SPEED_MAX_PERCENT,
    );
    validate_finite_f32(
        issues,
        "fan.offsetFanspeed",
        fan.offset_fanspeed,
        crate::fan::FAN_OFFSET_MIN_PERCENT,
        crate::fan::FAN_OFFSET_MAX_PERCENT,
    );
    if fan.minimum_fanspeed.is_finite()
        && fan.maximum_fanspeed.is_finite()
        && fan.minimum_fanspeed > fan.maximum_fanspeed
    {
        issue(
            issues,
            "fan.minimumFanspeed",
            "doit être inférieure ou égale à maximumFanspeed",
        );
    }
    if fan.use_control {
        validate_label(issues, "fan.fanProfile", &fan.fan_profile, 64, true);
    } else if !fan.fan_profile.is_empty() {
        validate_label(issues, "fan.fanProfile", &fan.fan_profile, 64, false);
    }
    validate_fan_curve_into(&fan.custom_fan_curve, "fan.customFanCurve", issues);
}

fn validate_profile_into(profile: &Profile, issues: &mut Vec<ValidationIssue>) {
    validate_profile_id_into(&profile.id, issues);
    validate_label(issues, "name", &profile.name, MAX_PROFILE_NAME_LEN, true);
    if profile.description.chars().count() > MAX_DESCRIPTION_LEN {
        issue(
            issues,
            "description",
            format!("ne doit pas dépasser {MAX_DESCRIPTION_LEN} caractères"),
        );
    }
    if has_control_chars(&profile.description, true) {
        issue(
            issues,
            "description",
            "ne doit contenir aucun caractère de contrôle (hors retour à la ligne et tabulation)",
        );
    }

    // Affichage.
    if profile.display.brightness > 100 {
        issue(
            issues,
            "display.brightness",
            "doit être comprise entre 0 et 100",
        );
    }
    if let Some(rate) = profile.display.refresh_rate {
        validate_finite_f32(
            issues,
            "display.refreshRate",
            rate,
            MIN_REFRESH_RATE_HZ,
            MAX_REFRESH_RATE_HZ,
        );
    }
    if profile.display.use_refresh_rate && profile.display.refresh_rate.is_none() {
        issue(
            issues,
            "display.refreshRate",
            "un taux est requis quand useRefRate est vrai",
        );
    }
    validate_optional_u32_range(
        issues,
        "display.xResolution",
        profile.display.x_resolution,
        1,
        MAX_RESOLUTION,
    );
    validate_optional_u32_range(
        issues,
        "display.yResolution",
        profile.display.y_resolution,
        1,
        MAX_RESOLUTION,
    );
    if profile.display.has_partial_resolution() {
        issue(
            issues,
            "display",
            "xResolution et yResolution doivent être renseignés ensemble",
        );
    }
    if profile.display.use_resolution && profile.display.effective_resolution().is_none() {
        issue(
            issues,
            "display",
            "une résolution complète est requise quand useResolution est vrai",
        );
    }

    // CPU.
    validate_optional_u32_range(
        issues,
        "cpu.onlineCores",
        profile.cpu.online_cores,
        1,
        MAX_CPU_ONLINE_CORES,
    );
    validate_optional_u64_range(
        issues,
        "cpu.scalingMinFrequency",
        profile.cpu.scaling_min_frequency,
        MIN_CPU_FREQUENCY_KHZ,
        MAX_CPU_FREQUENCY_KHZ,
    );
    validate_optional_u64_range(
        issues,
        "cpu.scalingMaxFrequency",
        profile
            .cpu
            .scaling_max_frequency
            .and_then(|limit| limit.hz()),
        MIN_CPU_FREQUENCY_KHZ,
        MAX_CPU_FREQUENCY_KHZ,
    );
    if let (Some(min), Some(limit)) = (
        profile.cpu.scaling_min_frequency,
        profile.cpu.scaling_max_frequency,
    ) && let Some(max) = limit.hz()
        && min > max
    {
        issue(
            issues,
            "cpu.scalingMinFrequency",
            "doit être inférieure ou égale à scalingMaxFrequency",
        );
    }
    validate_token(issues, "cpu.governor", &profile.cpu.governor, 64, true);
    validate_token(
        issues,
        "cpu.energyPerformancePreference",
        &profile.cpu.energy_performance_preference,
        64,
        true,
    );

    // Ventilation.
    validate_fan_limits_into(&profile.fan, issues);

    // ODM / TDP / NVIDIA.
    validate_optional_token(
        issues,
        "odmProfile.name",
        profile.odm_profile.name.as_deref(),
        64,
    );
    if profile.odm_power_limits.tdp_values.len() > MAX_TDP_VALUES {
        issue(
            issues,
            "odmPowerLimits.tdpValues",
            format!("ne doit pas dépasser {MAX_TDP_VALUES} valeurs"),
        );
    }
    let mut previous_tdp = None;
    for (index, value) in profile.odm_power_limits.tdp_values.iter().enumerate() {
        validate_finite_f64(
            issues,
            &format!("odmPowerLimits.tdpValues[{index}]"),
            *value,
            MIN_TDP_WATTS,
            MAX_TDP_WATTS,
        );
        if let Some(previous) = previous_tdp
            && value.is_finite()
            && *value < previous
        {
            issue(
                issues,
                format!("odmPowerLimits.tdpValues[{index}]"),
                "les valeurs TDP doivent être croissantes",
            );
        }
        if value.is_finite() {
            previous_tdp = Some(*value);
        }
    }
    validate_finite_f64(
        issues,
        "nvidiaPowerCTRLProfile.cTGPOffset",
        profile.nvidia_power_ctrl_profile.ctgp_offset,
        0.0,
        MAX_CTGP_OFFSET_WATTS,
    );
}

/// Valide un profil complet.
///
/// # Errors
///
/// Renvoie une erreur agrégée si un champ est invalide.
pub fn validate_profile(profile: &Profile) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    validate_profile_into(profile, &mut issues);
    ValidationError::from_issues(issues)
}

/// Clé de comparaison des noms de profils.
///
/// Insensible à la casse et aux espaces de bord, comme `validate_profiles`.
/// Partagée avec [`crate::presets::merge_with_default_profiles`] pour que la
/// fusion et la validation appliquent exactement la même règle.
pub(crate) fn normalized_name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Valide une collection de profils.
///
/// Vérifie en plus l'unicité des identifiants et des noms (comparaison
/// insensible à la casse après `trim`) et le nombre maximal de profils.
///
/// # Errors
///
/// Renvoie une erreur agrégée si au moins un profil est invalide ou si deux
/// profils partagent un identifiant ou un nom.
pub fn validate_profiles(profiles: &ProfileCollection) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    if profiles.len() > MAX_PROFILES {
        issue(
            &mut issues,
            "profiles",
            format!("ne doit pas dépasser {MAX_PROFILES} profils"),
        );
    }

    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for (index, profile) in profiles.iter().enumerate() {
        validate_profile_into(profile, &mut issues);
        if !profile.id.is_empty() && !ids.insert(profile.id.clone()) {
            issue(
                &mut issues,
                format!("profiles[{index}].id"),
                format!("identifiant dupliqué: {}", profile.id),
            );
        }
        if !profile.name.trim().is_empty() && !names.insert(normalized_name_key(&profile.name)) {
            issue(
                &mut issues,
                format!("profiles[{index}].name"),
                format!("nom dupliqué: {}", profile.name),
            );
        }
    }

    ValidationError::from_issues(issues)
}

fn validate_shutdown_time(issues: &mut Vec<ValidationIssue>, value: &str) {
    let Some((hours, minutes)) = value.split_once(':') else {
        issue(issues, "shutdownTime", "format attendu HH:MM");
        return;
    };
    if hours.len() != 2
        || minutes.len() != 2
        || !hours.bytes().all(|byte| byte.is_ascii_digit())
        || !minutes.bytes().all(|byte| byte.is_ascii_digit())
    {
        issue(issues, "shutdownTime", "format attendu HH:MM");
        return;
    }
    match (hours.parse::<u8>(), minutes.parse::<u8>()) {
        (Ok(hours), Ok(minutes)) if hours <= 23 && minutes <= 59 => {}
        _ => issue(issues, "shutdownTime", "heure hors bornes (00:00–23:59)"),
    }
}

fn validate_charging_descriptor(
    issues: &mut Vec<ValidationIssue>,
    field: &str,
    value: Option<&str>,
) {
    let Some(value) = value else {
        return;
    };
    if value.is_empty() {
        issue(issues, field, "ne doit pas être vide");
        return;
    }
    if value.chars().count() > 64 {
        issue(issues, field, "ne doit pas dépasser 64 caractères");
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_graphic() && character != '\\')
    {
        issue(
            issues,
            field,
            "ne doit contenir que des caractères ASCII imprimables sans antislash",
        );
    }
}

/// Valide un type de charge (`chargingType`).
///
/// Liste ouverte : `tcc-core` ne connaît pas les types exposés par le
/// firmware. Seule la forme est validée ; le daemon confronte la valeur aux
/// capacités réelles de `tcc-hw` avant toute écriture.
fn validate_charging_type(issues: &mut Vec<ValidationIssue>, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    if value.is_empty() {
        issue(issues, "chargingType", "ne doit pas être vide");
        return;
    }
    if value.chars().count() > MAX_CHARGING_TYPE_LEN {
        issue(
            issues,
            "chargingType",
            format!("ne doit pas dépasser {MAX_CHARGING_TYPE_LEN} caractères"),
        );
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
    }) {
        issue(
            issues,
            "chargingType",
            "ne doit contenir que des lettres ASCII, chiffres, '.', '_', ':' ou '-'",
        );
    }
}

/// Valide les réglages globaux.
///
/// # Errors
///
/// Renvoie une erreur agrégée si un champ est invalide ou si la version de
/// schéma n'est pas celle attendue.
pub fn validate_settings(settings: &Settings) -> Result<(), ValidationError> {
    let mut issues = Vec::new();

    if settings.schema_version != SETTINGS_SCHEMA_VERSION {
        issue(
            &mut issues,
            "schemaVersion",
            format!(
                "version {} non supportée (attendu {SETTINGS_SCHEMA_VERSION})",
                settings.schema_version
            ),
        );
    }

    validate_profile_id_into(&settings.state_map.power_ac, &mut issues);
    validate_profile_id_into(&settings.state_map.power_bat, &mut issues);
    for (key, profile_id) in settings.state_matrix.iter() {
        validate_profile_id_into(profile_id, &mut issues);
        if key.matrix_key().is_none() {
            issue(
                &mut issues,
                "stateMatrix",
                "clé non représentable (PPD inconnu)",
            );
        }
    }

    if settings.deleted_profile_ids.len() > MAX_DELETED_PROFILE_IDS {
        issue(
            &mut issues,
            "deletedProfileIds",
            format!("ne doit pas dépasser {MAX_DELETED_PROFILE_IDS} entrées"),
        );
    }
    let mut deleted = BTreeSet::new();
    for (index, profile_id) in settings.deleted_profile_ids.iter().enumerate() {
        let field = format!("deletedProfileIds[{index}]");
        validate_profile_id_field_into(profile_id, &field, &mut issues);
        if profile_id.is_tcc_managed() {
            issue(
                &mut issues,
                &field,
                "un identifiant fourni par TCC ne peut pas être tombstoné",
            );
        }
        if !deleted.insert(profile_id.as_str()) {
            issue(
                &mut issues,
                &field,
                format!("identifiant dupliqué: {profile_id}"),
            );
        }
    }

    if let Some(shutdown_time) = &settings.shutdown_time {
        validate_shutdown_time(&mut issues, shutdown_time);
    }

    validate_charging_descriptor(
        &mut issues,
        "chargingProfile",
        settings.charging_profile.as_deref(),
    );
    validate_charging_descriptor(
        &mut issues,
        "chargingPriority",
        settings.charging_priority.as_deref(),
    );
    validate_charging_type(&mut issues, settings.charging_type.as_deref());
    // `fnLockStatus` est un `Option<bool>` strict : serde refuse tout autre
    // type, et aucune plage n'est à contrôler. La disponibilité matérielle
    // est vérifiée par le daemon, jamais exigée ici.

    if let Some(start) = settings.charging_start_threshold
        && start > 100
    {
        issue(
            &mut issues,
            "chargingStartThreshold",
            "doit être comprise entre 0 et 100",
        );
    }
    if let Some(end) = settings.charging_end_threshold
        && end > 100
    {
        issue(
            &mut issues,
            "chargingEndThreshold",
            "doit être comprise entre 0 et 100",
        );
    }
    if let (Some(start), Some(end)) = (
        settings.charging_start_threshold,
        settings.charging_end_threshold,
    ) && start >= end
    {
        issue(
            &mut issues,
            "chargingStartThreshold",
            "doit être strictement inférieur à chargingEndThreshold",
        );
    }

    if settings.keyboard_states.len() > MAX_KEYBOARD_STATES {
        issue(
            &mut issues,
            "keyboardStates",
            format!("ne doit pas dépasser {MAX_KEYBOARD_STATES} zones"),
        );
    }
    for (index, state) in settings.keyboard_states.iter().enumerate() {
        for (name, value) in [
            ("red", state.red),
            ("green", state.green),
            ("blue", state.blue),
        ] {
            if !(-1..=255).contains(&value) {
                issue(
                    &mut issues,
                    format!("keyboardStates[{index}].{name}"),
                    "doit être -1 (non supporté) ou compris entre 0 et 255",
                );
            }
        }
    }

    if let Some(lightbar) = &settings.lightbar_states {
        for (name, value) in [
            ("red", lightbar.red),
            ("green", lightbar.green),
            ("blue", lightbar.blue),
        ] {
            if !(0..=255).contains(&value) {
                issue(
                    &mut issues,
                    format!("lightbarStates.{name}"),
                    "doit être compris entre 0 et 255",
                );
            }
        }
    }

    if settings.ycbcr420_workaround.len() > MAX_YCBCR_CARDS {
        issue(
            &mut issues,
            "ycbcr420Workaround",
            format!("ne doit pas dépasser {MAX_YCBCR_CARDS} cartes"),
        );
    }
    for (card_index, ports) in settings.ycbcr420_workaround.iter().enumerate() {
        if ports.len() > MAX_YCBCR_PORTS {
            issue(
                &mut issues,
                format!("ycbcr420Workaround[{card_index}]"),
                format!("ne doit pas dépasser {MAX_YCBCR_PORTS} ports"),
            );
        }
        for port in ports.keys() {
            if port.is_empty() || port.chars().count() > 64 {
                issue(
                    &mut issues,
                    format!("ycbcr420Workaround[{card_index}]"),
                    "nom de port vide ou trop long",
                );
            }
            if has_control_chars(port, false) {
                issue(
                    &mut issues,
                    format!("ycbcr420Workaround[{card_index}]"),
                    "nom de port avec caractère de contrôle",
                );
            }
        }
    }

    ValidationError::from_issues(issues)
}

/// Vérifie que les profils référencés par les réglages existent.
///
/// Utilisé par le daemon après chargement des deux fichiers. Un profil manquant
/// n'empêche pas le démarrage (le résolveur retombe sur le profil par défaut),
/// mais doit être signalé.
///
/// # Errors
///
/// Renvoie une erreur si `stateMap` ou `stateMatrix` référence un identifiant
/// absent de la collection.
pub fn validate_settings_references(
    settings: &Settings,
    profiles: &ProfileCollection,
) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    let mut check = |field: &str, id: &ProfileId| {
        if !id.is_empty() && !profiles.contains(id) {
            issue(&mut issues, field, format!("profil inconnu: {id}"));
        }
    };

    check("stateMap.power_ac", &settings.state_map.power_ac);
    check("stateMap.power_bat", &settings.state_map.power_bat);
    for (key, profile_id) in settings.state_matrix.iter() {
        let field = match key.matrix_key() {
            Some(token) => format!("stateMatrix[{token}]"),
            None => "stateMatrix".to_owned(),
        };
        check(&field, profile_id);
    }
    drop(check);

    ValidationError::from_issues(issues)
}

/// Valide un ensemble de courbes de ventilation nommées.
///
/// Chaque courbe doit avoir un nom non vide (sinon elle ne peut pas être
/// sélectionnée par `fan.fanProfile`), et les noms doivent être uniques.
///
/// # Errors
///
/// Renvoie une erreur agrégée si une courbe est invalide, sans nom, ou si deux
/// courbes partagent un nom.
pub fn validate_fan_tables(tables: &FanTableSet) -> Result<(), ValidationError> {
    let mut issues = Vec::new();
    if tables.len() > MAX_FAN_CURVES {
        issue(
            &mut issues,
            "fanTables",
            format!("ne doit pas dépasser {MAX_FAN_CURVES} courbes"),
        );
    }

    let mut names = BTreeSet::new();
    for (index, curve) in tables.iter().enumerate() {
        let prefix = format!("fanTables[{index}]");
        match curve.name.as_deref() {
            Some(name) if !name.trim().is_empty() => {
                validate_label(&mut issues, &format!("{prefix}.name"), name, 64, true);
                if !names.insert(name.to_owned()) {
                    issue(
                        &mut issues,
                        format!("{prefix}.name"),
                        format!("nom de courbe dupliqué: {name}"),
                    );
                }
            }
            _ => issue(
                &mut issues,
                format!("{prefix}.name"),
                "un nom de courbe non vide est requis",
            ),
        }
        validate_fan_curve_into(curve, &prefix, &mut issues);
    }

    ValidationError::from_issues(issues)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        MAX_CHARGING_TYPE_LEN, MAX_DELETED_PROFILE_IDS, MAX_PROFILE_ID_LEN, validate_fan_curve,
        validate_fan_tables, validate_profile, validate_profile_description, validate_profile_id,
        validate_profile_name, validate_profiles, validate_settings, validate_settings_references,
    };
    use crate::error::ValidationIssue;
    use crate::fan::{FanCurve, FanTableEntry, FanTableSet};
    use crate::profile::{
        DEFAULT_CUSTOM_PROFILE_ID, DisplayProfile, OFFICE_PROFILE_ID, Profile, ProfileCollection,
        ProfileId,
    };
    use crate::settings::{KeyboardState, Settings};

    fn valid_profile() -> Profile {
        Profile::new("office", "Office")
    }

    fn fields(error: &crate::ValidationError) -> Vec<&str> {
        error
            .issues()
            .iter()
            .map(|issue| issue.field.as_str())
            .collect()
    }

    #[test]
    fn validates_known_good_profile() {
        validate_profile(&valid_profile()).unwrap();
    }

    #[test]
    fn rejects_empty_and_unsafe_ids() {
        for value in ["", "../x", "a/b", "a.b", "é"] {
            assert!(
                validate_profile_id(&ProfileId::new(value)).is_err(),
                "identifiant accepté à tort: {value:?}"
            );
        }
        let long = "a".repeat(MAX_PROFILE_ID_LEN + 1);
        assert!(validate_profile_id(&ProfileId::new(long)).is_err());
    }

    #[test]
    fn accepts_original_preset_ids() {
        for value in [
            "__profile_max_energy_save__",
            "__office__",
            "__legacy_cool_and_breezy__",
            "custom-0123456789abcdef",
        ] {
            validate_profile_id(&ProfileId::new(value)).unwrap();
        }
    }

    #[test]
    fn profile_name_rules() {
        validate_profile_name("Bureau").unwrap();
        validate_profile_name("Büro").unwrap();
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("   ").is_err());
        assert!(validate_profile_name("a\u{0}b").is_err());
    }

    #[test]
    fn description_allows_newlines_but_not_other_controls() {
        validate_profile_description("Ligne 1\nLigne 2\tok").unwrap();
        assert!(validate_profile_description("bell\u{7}").is_err());
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_values() {
        let mut profile = valid_profile();
        profile.display.refresh_rate = Some(f32::NAN);
        profile.display.use_refresh_rate = true;
        profile.fan.minimum_fanspeed = 90.0;
        profile.fan.maximum_fanspeed = 10.0;
        profile.cpu.scaling_min_frequency = Some(5_000_000);
        profile.cpu.scaling_max_frequency = Some(crate::profile::FrequencyLimit::Hz(1_000_000));
        profile.nvidia_power_ctrl_profile.ctgp_offset = f64::INFINITY;

        let error = validate_profile(&profile).unwrap_err();
        let fields = fields(&error);
        assert!(fields.contains(&"display.refreshRate"));
        assert!(fields.contains(&"fan.minimumFanspeed"));
        assert!(fields.contains(&"cpu.scalingMinFrequency"));
        assert!(fields.contains(&"nvidiaPowerCTRLProfile.cTGPOffset"));
    }

    #[test]
    fn refresh_rate_requires_value_when_enabled() {
        let mut profile = valid_profile();
        profile.display.use_refresh_rate = true;
        profile.display.refresh_rate = None;
        let error = validate_profile(&profile).unwrap_err();
        assert!(fields(&error).contains(&"display.refreshRate"));
    }

    #[test]
    fn partial_resolution_is_rejected() {
        let mut profile = valid_profile();
        profile.display.x_resolution = Some(1920);
        let error = validate_profile(&profile).unwrap_err();
        assert!(fields(&error).contains(&"display"));
    }

    #[test]
    fn tdp_values_must_be_increasing_and_bounded() {
        let mut profile = valid_profile();
        profile.odm_power_limits.tdp_values = vec![30.0, 20.0, 5000.0];
        let error = validate_profile(&profile).unwrap_err();
        let fields = fields(&error);
        assert!(fields.contains(&"odmPowerLimits.tdpValues[1]"));
        assert!(fields.contains(&"odmPowerLimits.tdpValues[2]"));
    }

    #[test]
    fn fan_curve_rejects_duplicate_and_non_finite_points() {
        let curve = FanCurve {
            name: Some("bad".into()),
            table_cpu: vec![
                FanTableEntry::new(50.0, 20.0),
                FanTableEntry::new(50.0, 30.0),
            ],
            table_gpu: vec![FanTableEntry::new(f32::NAN, f32::INFINITY)],
        };
        let error = validate_fan_curve(&curve).unwrap_err();
        assert!(
            error
                .issues()
                .iter()
                .any(|issue: &ValidationIssue| issue.message.contains("dupliquée"))
        );
    }

    #[test]
    fn settings_require_current_schema() {
        let mut settings = Settings {
            schema_version: 0,
            ..Settings::default()
        };
        let error = validate_settings(&settings).unwrap_err();
        assert!(fields(&error).contains(&"schemaVersion"));

        settings.schema_version = 99;
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn deleted_profile_ids_are_bounded_unique_and_not_tcc_managed() {
        let mut settings = Settings {
            deleted_profile_ids: vec![ProfileId::new("custom-old"), ProfileId::new("custom-older")],
            ..Settings::default()
        };
        validate_settings(&settings).unwrap();

        // Doublon localisé.
        settings.deleted_profile_ids =
            vec![ProfileId::new("custom-old"), ProfileId::new("custom-old")];
        let error = validate_settings(&settings).unwrap_err();
        assert!(fields(&error).contains(&"deletedProfileIds[1]"));
        assert!(
            error
                .issues()
                .iter()
                .any(|issue| issue.message.contains("dupliqué"))
        );

        // Identifiant syntaxiquement invalide.
        settings.deleted_profile_ids = vec![ProfileId::new("bad id")];
        let error = validate_settings(&settings).unwrap_err();
        assert!(fields(&error).contains(&"deletedProfileIds[0]"));

        // Preset et profil personnalisé par défaut refusés.
        for id in [OFFICE_PROFILE_ID, DEFAULT_CUSTOM_PROFILE_ID] {
            settings.deleted_profile_ids = vec![ProfileId::new(id)];
            let error = validate_settings(&settings).unwrap_err();
            assert!(
                error
                    .issues()
                    .iter()
                    .any(|issue| issue.message.contains("fourni par TCC")),
                "id={id}"
            );
        }

        // Borne stricte.
        settings.deleted_profile_ids = (0..=MAX_DELETED_PROFILE_IDS)
            .map(|index| ProfileId::new(format!("custom-{index}")))
            .collect();
        let error = validate_settings(&settings).unwrap_err();
        assert!(fields(&error).contains(&"deletedProfileIds"));
    }

    #[test]
    fn charging_thresholds_are_ordered() {
        let mut settings = Settings {
            charging_start_threshold: Some(80),
            charging_end_threshold: Some(70),
            ..Settings::default()
        };
        let error = validate_settings(&settings).unwrap_err();
        assert!(fields(&error).contains(&"chargingStartThreshold"));

        settings.charging_start_threshold = Some(60);
        settings.charging_end_threshold = Some(80);
        validate_settings(&settings).unwrap();
    }

    #[test]
    fn charging_type_and_fn_lock_validation() {
        let mut settings = Settings {
            charging_type: None,
            fn_lock_status: None,
            ..Settings::default()
        };
        validate_settings(&settings).unwrap();

        for valid in [
            "fast".to_owned(),
            "fast.charge:1-A".to_owned(),
            "standard_2".to_owned(),
            "a".repeat(MAX_CHARGING_TYPE_LEN),
        ] {
            settings.charging_type = Some(valid);
            validate_settings(&settings).unwrap();
        }

        for invalid in ["", "bad type", "bad/type", "bad\\type", "é", "\n"] {
            settings.charging_type = Some(invalid.to_owned());
            let error = validate_settings(&settings).unwrap_err();
            assert!(
                fields(&error).contains(&"chargingType"),
                "valeur acceptée à tort: {invalid:?}"
            );
        }
        settings.charging_type = Some("a".repeat(MAX_CHARGING_TYPE_LEN + 1));
        assert!(validate_settings(&settings).is_err());

        settings.charging_type = None;
        for value in [false, true] {
            settings.fn_lock_status = Some(value);
            validate_settings(&settings).unwrap();
        }
        settings.fn_lock_status = None;
        validate_settings(&settings).unwrap();
    }

    #[test]
    fn shutdown_time_format_is_checked() {
        let mut settings = Settings::default();
        for valid in ["00:00", "23:59", "07:05"] {
            settings.shutdown_time = Some(valid.to_owned());
            validate_settings(&settings).unwrap();
        }
        for invalid in ["24:00", "7:05", "12:60", "12", "aa:bb", "12:5"] {
            settings.shutdown_time = Some(invalid.to_owned());
            assert!(
                validate_settings(&settings).is_err(),
                "accepté à tort: {invalid}"
            );
        }
    }

    #[test]
    fn keyboard_and_lightbar_colors_are_bounded() {
        let mut settings = Settings::default();
        settings.keyboard_states = vec![KeyboardState::new(
            crate::settings::KeyboardColorMode::Static,
            255,
            300,
            0,
            0,
        )];
        assert!(validate_settings(&settings).is_err());

        settings.keyboard_states = vec![KeyboardState::default()];
        settings.lightbar_states = Some(crate::settings::LightbarState {
            brightness: 1,
            red: 256,
            green: 0,
            blue: 0,
        });
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn ycbcr_workaround_keys_are_checked() {
        let mut settings = Settings::default();
        let mut ports = BTreeMap::new();
        ports.insert(String::new(), true);
        settings.ycbcr420_workaround = vec![ports];
        assert!(validate_settings(&settings).is_err());

        let mut ports = BTreeMap::new();
        ports.insert("card0-DP-1".to_owned(), true);
        settings.ycbcr420_workaround = vec![ports];
        validate_settings(&settings).unwrap();
    }

    #[test]
    fn profile_collection_rejects_duplicates() {
        let mut first = valid_profile();
        first.name = "Office".to_owned();
        let mut second = valid_profile();
        second.name = " office ".to_owned();
        let collection = ProfileCollection::new(vec![first, second]);
        let error = validate_profiles(&collection).unwrap_err();
        let fields = fields(&error);
        assert!(fields.iter().any(|field| field.ends_with(".name")));
        assert!(fields.iter().any(|field| field.ends_with(".id")));
    }

    #[test]
    fn profile_count_is_bounded() {
        let mut profiles = Vec::new();
        for index in 0..=super::MAX_PROFILES {
            profiles.push(Profile::new(
                format!("p{index}"),
                format!("Profile {index}"),
            ));
        }
        let collection = ProfileCollection::new(profiles);
        assert!(validate_profiles(&collection).is_err());
    }

    #[test]
    fn fan_tables_require_unique_non_empty_names() {
        let mut unnamed = FanTableSet::default();
        unnamed.upsert(FanCurve {
            name: None,
            table_cpu: vec![FanTableEntry::new(20.0, 10.0)],
            table_gpu: Vec::new(),
        });
        assert!(validate_fan_tables(&unnamed).is_err());

        let mut named = FanTableSet::default();
        named.upsert(FanCurve {
            name: Some("Silent".into()),
            table_cpu: vec![FanTableEntry::new(20.0, 10.0)],
            table_gpu: Vec::new(),
        });
        validate_fan_tables(&named).unwrap();
    }

    #[test]
    fn settings_references_are_checked_against_collection() {
        let mut settings = Settings::default();
        settings.state_map.power_ac = ProfileId::new("office");
        settings.state_map.power_bat = ProfileId::new("missing");
        let profiles = ProfileCollection::new(vec![valid_profile()]);
        let error = validate_settings_references(&settings, &profiles).unwrap_err();
        assert!(fields(&error).contains(&"stateMap.power_bat"));
        assert!(!fields(&error).contains(&"stateMap.power_ac"));
    }

    #[test]
    fn display_profile_default_is_valid() {
        let profile = Profile::new("custom", "Custom");
        assert_eq!(profile.display, DisplayProfile::default());
        validate_profile(&profile).unwrap();
    }
}
