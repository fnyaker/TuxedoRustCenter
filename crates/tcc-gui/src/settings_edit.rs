// SPDX-License-Identifier: GPL-3.0-or-later
//
// Édition locale des réglages globaux et application sur `tcc_core::Settings`.
//
// Champs modifiables : quatre booléens (Fahrenheit, contrôles CPU,
// ventilateurs, rétroéclairage), la charge de la batterie (profil, priorité,
// seuils début/fin, type), la matrice PPD × alimentation, la minuterie
// d'extinction (`HH:MM`), le Fn-Lock et l'éclairage (états clavier/lightbar).
// Tout le reste — `stateMap`, PRIME, YCbCr — est **préservé tel quel**, y
// compris les champs inconnus conservés par `Settings::extra`.
//
// La GUI n'envoie jamais un `Settings` complet : elle décrit uniquement les
// champs autorisés, et Rust les applique sur la version fraîchement relue
// (`run_save_settings`), après comparaison avec la base d'édition (`Conflict`).
//
// Charge : un champ absent ou `null` signifie « ne pas changer » — il n'y a
// jamais d'effacement silencieux. Les valeurs proposées (profils, priorités,
// seuils) sont validées contre les capacités du **dernier instantané
// matériel** ; elles ne sont jamais fournies par QML.

use serde::{Deserialize, Deserializer};
use tcc_core::{
    KeyboardColorMode, KeyboardState, LightbarState, MAX_CHARGING_TYPE_LEN, MAX_KEYBOARD_STATES,
    PowerSource, PpdProfile, ProfileCollection, ProfileId, Settings, StateKey, StateMatrix,
    validate_profile_id,
};

use crate::hardware_view::Charging;
use crate::profile_edit::SaveError;

/// Taille maximale du DTO d'édition envoyé par QML (64 Kio).
pub const MAX_SETTINGS_EDIT_BYTES: usize = 64 * 1024;
/// Taille maximale du `Settings` sérialisé (1 Mio, contrat `Admin1`).
pub const MAX_SETTINGS_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Champs modifiables en phase 1.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsEdit {
    pub fahrenheit: bool,
    pub cpu_settings_enabled: bool,
    pub fan_control_enabled: bool,
    pub keyboard_backlight_control_enabled: bool,
    /// `None` = ne pas changer. Un champ absent et un `null` sont équivalents.
    #[serde(default)]
    pub charging_profile: Option<String>,
    /// `None` = ne pas changer.
    #[serde(default)]
    pub charging_priority: Option<String>,
    /// `None` = ne pas changer.
    #[serde(default)]
    pub charging_start_threshold: Option<u8>,
    /// `None` = ne pas changer.
    #[serde(default)]
    pub charging_end_threshold: Option<u8>,
    /// `None` = ne pas changer ; `Some([])` vide la matrice (fallback daemon).
    #[serde(default)]
    pub state_matrix: Option<Vec<StateMatrixEdit>>,
    /// Heure d'extinction `HH:MM` (24 h) ; `None` = ne pas changer.
    #[serde(default)]
    pub shutdown_time: Option<String>,
    /// Efface `shutdownTime` (fallback : aucune planification). Exclusif avec
    /// un `shutdownTime` non nul.
    #[serde(default)]
    pub clear_shutdown_time: bool,
    /// Consentement persistant pour que TCC écrive `platform_profile` malgré
    /// PPD. `None` = ne pas changer.
    #[serde(default)]
    pub ppd_override: Option<bool>,
    /// Type de charge (`Settings.chargingType`). `None` = ne pas changer.
    #[serde(default)]
    pub charging_type: Option<String>,
    /// État Fn-Lock persistant. `None` = ne pas changer.
    #[serde(default)]
    pub fn_lock_status: Option<bool>,
    /// États clavier complets (`keyboardStates`). `None`/absent = ne pas
    /// changer ; `[]` efface explicitement la liste.
    #[serde(default)]
    pub keyboard_states: Option<Vec<KeyboardStateEdit>>,
    /// État lightbar. Absent = ne pas changer ; `null` explicite = effacer ;
    /// objet = appliquer.
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub lightbar_states: Option<Option<LightbarStateEdit>>,
}

/// État d'une zone clavier fourni par QML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyboardStateEdit {
    /// `static` ou `breathing` ; absent = `static`.
    #[serde(default)]
    pub mode: Option<String>,
    pub brightness: u8,
    pub red: i16,
    pub green: i16,
    pub blue: i16,
}

/// État de la lightbar fourni par QML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LightbarStateEdit {
    pub brightness: u8,
    pub red: i16,
    pub green: i16,
    pub blue: i16,
}

/// Distingue un champ absent (`None`) d'un `null` explicite (`Some(None)`).
fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Types de charge de l'ABI noyau, utilisés quand le backend n'annonce pas de
/// liste.
const STANDARD_CHARGE_TYPES: [&str; 7] = [
    "Trickle", "Fast", "Standard", "Adaptive", "Custom", "LongLife", "Bypass",
];

/// Entrée de matrice PPD × alimentation fournie par QML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateMatrixEdit {
    /// `power-saver`, `balanced` ou `performance`.
    pub ppd: String,
    /// `ac` ou `battery`.
    pub power: String,
    /// Profil cible (preset ou personnalisé), obligatoirement présent dans la
    /// collection live.
    pub profile_id: String,
}

/// Capacités de charge issues du dernier instantané matériel.
///
/// Jamais construites depuis le DTO QML : la liste des valeurs autorisées vient
/// de `GetHardwareSnapshotJSON` (`charging.profiles`, `priorities`,
/// `startThresholds`, `endThresholds`). Une liste absente interdit l'édition du
/// champ correspondant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChargeCapabilities {
    pub profiles: Vec<String>,
    pub priorities: Vec<String>,
    pub start_thresholds: Vec<u8>,
    pub end_thresholds: Vec<u8>,
    pub thresholds_available: bool,
    /// Types annoncés par le backend. Vide : le noyau expose seulement le
    /// type courant, l'UI propose les valeurs ABI standard.
    pub charge_types: Vec<String>,
    pub charge_type_writable: bool,
}

impl ChargeCapabilities {
    #[must_use]
    pub fn from_charging(charging: &Charging) -> Self {
        Self {
            profiles: charging.profiles.clone(),
            priorities: charging.priorities.clone(),
            start_thresholds: charging.start_thresholds.clone(),
            end_thresholds: charging.end_thresholds.clone(),
            thresholds_available: charging.thresholds_available,
            charge_types: charging.charge_types.clone(),
            charge_type_writable: charging.charge_type_writable,
        }
    }
}

/// Réglages validés prêts à être enregistrés.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSettings {
    /// `Settings` complet avec les champs modifiés appliqués.
    pub settings: Settings,
    /// JSON canonique envoyé à `SaveSettingsJSON`.
    pub json: String,
}

/// Décode un DTO de réglages borné.
///
/// # Errors
///
/// [`SaveError::PayloadTooLarge`] au-delà de [`MAX_SETTINGS_EDIT_BYTES`],
/// [`SaveError::InvalidPayload`] si le JSON ne correspond pas exactement au
/// DTO (champ obligatoire manquant, clé inconnue ou type faux).
pub fn decode_settings_edit(json: &str) -> Result<SettingsEdit, SaveError> {
    if json.len() > MAX_SETTINGS_EDIT_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    serde_json::from_str(json).map_err(|_| SaveError::InvalidPayload)
}

/// Applique le DTO sur les réglages et sérialise le résultat.
///
/// Seuls les champs présents (`Some`) sont écrits. Les valeurs de charge sont
/// vérifiées contre les capacités matérielles fournies ; les seuils doivent
/// rester strictement croissants (contrôle final par `Settings::validate`).
///
/// # Errors
///
/// [`SaveError::Validation`] pour une valeur hors listes ou un couple de seuils
/// incohérent, [`SaveError::PayloadTooLarge`] si le `Settings` sérialisé
/// dépasse 1 Mio (improbable, mais borné avant l'envoi D-Bus).
pub fn apply_settings_edit(
    base: &Settings,
    edit: &SettingsEdit,
    capabilities: Option<&ChargeCapabilities>,
    profiles: Option<&ProfileCollection>,
) -> Result<PreparedSettings, SaveError> {
    let mut settings = base.clone();
    settings.fahrenheit = edit.fahrenheit;
    settings.cpu_settings_enabled = edit.cpu_settings_enabled;
    settings.fan_control_enabled = edit.fan_control_enabled;
    settings.keyboard_backlight_control_enabled = edit.keyboard_backlight_control_enabled;

    apply_charge_edit(&mut settings, edit, capabilities)?;
    apply_fn_lock_edit(&mut settings, edit);
    apply_lighting_edit(&mut settings, edit)?;
    apply_state_matrix_edit(&mut settings, edit, profiles)?;
    apply_shutdown_time_edit(&mut settings, edit)?;
    if let Some(value) = edit.ppd_override {
        settings.ppd_override = value;
    }

    // Les seuils (0..=100, début < fin) sont validés par `tcc-core` ; c'est la
    // même règle que celle appliquée par le daemon.
    settings.validate().map_err(|error| SaveError::Validation {
        field: error
            .issues()
            .first()
            .map(|issue| issue.field.clone())
            .unwrap_or_else(|| "settings".to_owned()),
    })?;

    let json = serde_json::to_string(&settings).map_err(|_| SaveError::InvalidPayload)?;
    if json.len() > MAX_SETTINGS_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    Ok(PreparedSettings { settings, json })
}

/// Applique uniquement les champs de charge présents dans le DTO.
fn apply_charge_edit(
    settings: &mut Settings,
    edit: &SettingsEdit,
    capabilities: Option<&ChargeCapabilities>,
) -> Result<(), SaveError> {
    apply_charging_type_edit(settings, edit, capabilities)?;
    if let Some(profile) = edit.charging_profile.as_deref() {
        let capabilities = capabilities_for(capabilities, "chargingProfile")?;
        validate_choice(profile, &capabilities.profiles, "chargingProfile")?;
        settings.charging_profile = Some(profile.to_owned());
    }
    if let Some(priority) = edit.charging_priority.as_deref() {
        let capabilities = capabilities_for(capabilities, "chargingPriority")?;
        validate_choice(priority, &capabilities.priorities, "chargingPriority")?;
        settings.charging_priority = Some(priority.to_owned());
    }
    if let Some(start) = edit.charging_start_threshold {
        let capabilities = capabilities_for(capabilities, "chargingStartThreshold")?;
        if !capabilities.thresholds_available || !capabilities.start_thresholds.contains(&start) {
            return Err(SaveError::Validation {
                field: "chargingStartThreshold".to_owned(),
            });
        }
        settings.charging_start_threshold = Some(start);
    }
    if let Some(end) = edit.charging_end_threshold {
        let capabilities = capabilities_for(capabilities, "chargingEndThreshold")?;
        if !capabilities.thresholds_available || !capabilities.end_thresholds.contains(&end) {
            return Err(SaveError::Validation {
                field: "chargingEndThreshold".to_owned(),
            });
        }
        settings.charging_end_threshold = Some(end);
    }
    Ok(())
}

/// Applique `fnLockStatus` si présent.
fn apply_fn_lock_edit(settings: &mut Settings, edit: &SettingsEdit) {
    if let Some(value) = edit.fn_lock_status {
        settings.fn_lock_status = Some(value);
    }
}

/// Applique le type de charge, validé contre l'ABI et les capacités du
/// backend. Un type inconnu n'est jamais envoyé au daemon.
fn apply_charging_type_edit(
    settings: &mut Settings,
    edit: &SettingsEdit,
    capabilities: Option<&ChargeCapabilities>,
) -> Result<(), SaveError> {
    let Some(raw) = edit.charging_type.as_deref() else {
        return Ok(());
    };
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > MAX_CHARGING_TYPE_LEN
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    {
        return Err(charge_type_error());
    }
    let capabilities = capabilities_for(capabilities, "chargingType")?;
    if !capabilities.charge_type_writable {
        return Err(charge_type_error());
    }
    if capabilities.charge_types.is_empty() {
        let canonical = STANDARD_CHARGE_TYPES
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(value))
            .ok_or_else(charge_type_error)?;
        settings.charging_type = Some((*canonical).to_owned());
    } else if capabilities
        .charge_types
        .iter()
        .any(|candidate| candidate == value)
    {
        settings.charging_type = Some(value.to_owned());
    } else {
        return Err(charge_type_error());
    }
    Ok(())
}

fn charge_type_error() -> SaveError {
    SaveError::Validation {
        field: "chargingType".to_owned(),
    }
}

/// Applique les états clavier/lightbar. Les modes et bornes sont vérifiés
/// avant conversion ; `Some([])`/`Some(None)` sont des effacements explicites.
fn apply_lighting_edit(settings: &mut Settings, edit: &SettingsEdit) -> Result<(), SaveError> {
    if let Some(states) = edit.keyboard_states.as_ref() {
        if states.len() > MAX_KEYBOARD_STATES {
            return Err(lighting_error("keyboardStates"));
        }
        let mut converted = Vec::with_capacity(states.len());
        for (index, state) in states.iter().enumerate() {
            let mode = match state.mode.as_deref().unwrap_or("static") {
                "static" => KeyboardColorMode::Static,
                "breathing" => KeyboardColorMode::Breathing,
                _ => return Err(lighting_error(&format!("keyboardStates[{index}].mode"))),
            };
            if !valid_channel(state.red)
                || !valid_channel(state.green)
                || !valid_channel(state.blue)
            {
                return Err(lighting_error(&format!("keyboardStates[{index}]")));
            }
            converted.push(KeyboardState::new(
                mode,
                state.brightness,
                state.red,
                state.green,
                state.blue,
            ));
        }
        settings.keyboard_states = converted;
    }

    match edit.lightbar_states.as_ref() {
        None => {}
        Some(None) => settings.lightbar_states = None,
        Some(Some(state)) => {
            if !(0..=255).contains(&state.red)
                || !(0..=255).contains(&state.green)
                || !(0..=255).contains(&state.blue)
            {
                return Err(lighting_error("lightbarStates"));
            }
            settings.lightbar_states = Some(LightbarState {
                brightness: state.brightness,
                red: state.red,
                green: state.green,
                blue: state.blue,
            });
        }
    }
    Ok(())
}

/// Un canal est soit `-1` (non supporté), soit `0..=255`.
fn valid_channel(value: i16) -> bool {
    (-1..=255).contains(&value)
}

fn lighting_error(field: &str) -> SaveError {
    SaveError::Validation {
        field: field.to_owned(),
    }
}

fn capabilities_for<'a>(
    capabilities: Option<&'a ChargeCapabilities>,
    field: &'static str,
) -> Result<&'a ChargeCapabilities, SaveError> {
    capabilities.ok_or_else(|| SaveError::Validation {
        field: field.to_owned(),
    })
}

fn validate_choice(value: &str, allowed: &[String], field: &'static str) -> Result<(), SaveError> {
    if value.is_empty() || !allowed.iter().any(|candidate| candidate == value) {
        return Err(SaveError::Validation {
            field: field.to_owned(),
        });
    }
    Ok(())
}

/// Applique la matrice PPD × alimentation si elle est fournie.
///
/// Sémantique de remplacement : la liste fournie devient la matrice complète ;
/// une liste partielle est acceptée (le daemon retombe sur `stateMap`), une
/// liste vide efface la matrice. Chaque clé doit être unique, le PPD et
/// l'alimentation dans les valeurs autorisées, et le profil présent dans la
/// collection live.
fn apply_state_matrix_edit(
    settings: &mut Settings,
    edit: &SettingsEdit,
    profiles: Option<&ProfileCollection>,
) -> Result<(), SaveError> {
    let Some(entries) = edit.state_matrix.as_ref() else {
        return Ok(());
    };

    let mut matrix = StateMatrix::new();
    for entry in entries {
        let ppd = match entry.ppd.as_str() {
            "power-saver" => PpdProfile::PowerSaver,
            "balanced" => PpdProfile::Balanced,
            "performance" => PpdProfile::Performance,
            _ => return Err(matrix_error("stateMatrix")),
        };
        let power = match entry.power.as_str() {
            "ac" => PowerSource::Ac,
            "battery" => PowerSource::Battery,
            _ => return Err(matrix_error("stateMatrix")),
        };
        let key = StateKey::new(Some(ppd), power);
        let cell = key.matrix_key().unwrap_or_else(|| "stateMatrix".to_owned());
        let profile_id = ProfileId::new(entry.profile_id.clone());
        if validate_profile_id(&profile_id).is_err() {
            return Err(matrix_error(&format!("stateMatrix[{cell}]")));
        }
        let profiles = profiles.ok_or_else(|| matrix_error(&format!("stateMatrix[{cell}]")))?;
        if !profiles.contains(&profile_id) {
            return Err(matrix_error(&format!("stateMatrix[{cell}]")));
        }
        if matrix.contains_key(&key) {
            return Err(matrix_error(&format!("stateMatrix[{cell}]")));
        }
        matrix.insert(key, profile_id);
    }
    settings.state_matrix = matrix;
    Ok(())
}

fn matrix_error(field: &str) -> SaveError {
    SaveError::Validation {
        field: field.to_owned(),
    }
}

/// Applique la minuterie d'extinction si elle est fournie.
///
/// `clearShutdownTime` et `shutdownTime` sont exclusifs : une demande qui
/// fournit les deux est refusée (aucune ambiguïté). Un `shutdownTime` vide est
/// invalide ; utiliser `clearShutdownTime` pour effacer.
fn apply_shutdown_time_edit(settings: &mut Settings, edit: &SettingsEdit) -> Result<(), SaveError> {
    if edit.clear_shutdown_time {
        if edit.shutdown_time.is_some() {
            return Err(SaveError::Validation {
                field: "shutdownTime".to_owned(),
            });
        }
        settings.shutdown_time = None;
        return Ok(());
    }
    let Some(value) = edit.shutdown_time.as_deref() else {
        return Ok(());
    };
    settings.shutdown_time = Some(normalize_shutdown_time(value)?);
    Ok(())
}

/// Valide et normalise une heure `HH:MM` sur 24 h.
fn normalize_shutdown_time(value: &str) -> Result<String, SaveError> {
    let value = value.trim();
    let invalid = || SaveError::Validation {
        field: "shutdownTime".to_owned(),
    };
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return Err(invalid());
    }
    if !bytes[..2].iter().all(u8::is_ascii_digit) || !bytes[3..].iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    let hours: u8 = value[..2].parse().map_err(|_| invalid())?;
    let minutes: u8 = value[3..].parse().map_err(|_| invalid())?;
    if hours > 23 || minutes > 59 {
        return Err(invalid());
    }
    Ok(format!("{hours:02}:{minutes:02}"))
}

/// Vérifie que les réglages n'ont pas changé puis prépare l'édition.
///
/// `base` est la version sur laquelle le formulaire a été chargé, `current` la
/// relecture juste avant l'envoi. Une différence annule l'enregistrement
/// (`Conflict`) : sans révision côté `Admin1`, c'est la seule protection
/// contre l'écrasement d'un autre client (course résiduelle documentée).
///
/// # Errors
///
/// [`SaveError::Conflict`] si les réglages diffèrent, puis les erreurs de
/// [`decode_settings_edit`]/[`apply_settings_edit`].
pub fn prepare_settings_save(
    base: &Settings,
    current: &Settings,
    edit_json: &str,
    capabilities: Option<&ChargeCapabilities>,
    profiles: Option<&ProfileCollection>,
) -> Result<PreparedSettings, SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }
    let edit = decode_settings_edit(edit_json)?;
    apply_settings_edit(base, &edit, capabilities, profiles)
}

#[cfg(test)]
mod tests {
    use super::{
        ChargeCapabilities, MAX_SETTINGS_EDIT_BYTES, MAX_SETTINGS_PAYLOAD_BYTES, SettingsEdit,
        apply_settings_edit, decode_settings_edit, prepare_settings_save,
    };
    use crate::profile_edit::SaveError;
    use tcc_core::{
        KeyboardColorMode, KeyboardState, LightbarState, MAX_KEYBOARD_STATES, Settings,
    };

    fn edit_json() -> String {
        r#"{
            "fahrenheit": true,
            "cpuSettingsEnabled": false,
            "fanControlEnabled": true,
            "keyboardBacklightControlEnabled": false
        }"#
        .to_owned()
    }

    fn capabilities() -> ChargeCapabilities {
        ChargeCapabilities {
            profiles: vec!["balanced".to_owned(), "high-capacity".to_owned()],
            priorities: vec!["balanced".to_owned(), "charging".to_owned()],
            start_thresholds: vec![40, 60, 80],
            end_thresholds: vec![50, 70, 90, 100],
            thresholds_available: true,
            charge_types: vec!["Fast".to_owned(), "Standard".to_owned()],
            charge_type_writable: true,
        }
    }

    fn base() -> Settings {
        let mut settings = Settings::default();
        settings.state_map.power_ac = tcc_core::ProfileId::new("custom-ac");
        settings.state_matrix.insert(
            tcc_core::StateKey::new(None, tcc_core::PowerSource::Battery),
            tcc_core::ProfileId::new("custom-bat"),
        );
        settings.charging_profile = Some("balanced".to_owned());
        settings.charging_priority = Some("balanced".to_owned());
        settings.charging_start_threshold = Some(60);
        settings.charging_end_threshold = Some(90);
        settings.shutdown_time = Some("22:30".to_owned());
        settings.extra.insert(
            "futureField".to_owned(),
            serde_json::json!({ "kept": true }),
        );
        settings
    }

    fn charge_edit(fields: &str) -> String {
        format!(
            r#"{{
                "fahrenheit": false,
                "cpuSettingsEnabled": true,
                "fanControlEnabled": true,
                "keyboardBacklightControlEnabled": true,
                {fields}
            }}"#
        )
    }

    #[test]
    fn only_allowed_fields_are_changed() {
        let base = base();
        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");

        assert!(prepared.settings.fahrenheit);
        assert!(!prepared.settings.cpu_settings_enabled);
        assert!(prepared.settings.fan_control_enabled);
        assert!(!prepared.settings.keyboard_backlight_control_enabled);

        // Tout le reste est préservé, y compris les champs inconnus.
        assert_eq!(prepared.settings.state_map, base.state_map);
        assert_eq!(prepared.settings.state_matrix, base.state_matrix);
        assert_eq!(prepared.settings.charging_profile, base.charging_profile);
        assert_eq!(
            prepared.settings.charging_start_threshold,
            base.charging_start_threshold
        );
        assert_eq!(prepared.settings.shutdown_time, base.shutdown_time);
        assert_eq!(prepared.settings.extra, base.extra);
        assert!(prepared.json.contains("futureField"));
        assert!(prepared.json.len() <= MAX_SETTINGS_PAYLOAD_BYTES);
    }

    #[test]
    fn null_charge_fields_never_clear_values() {
        let base = base();
        let json = charge_edit(
            r#""chargingProfile": null,
               "chargingPriority": null,
               "chargingStartThreshold": null,
               "chargingEndThreshold": null"#,
        );
        let edit = decode_settings_edit(&json).expect("DTO");
        let prepared =
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).expect("apply");
        assert_eq!(prepared.settings.charging_profile, base.charging_profile);
        assert_eq!(prepared.settings.charging_priority, base.charging_priority);
        assert_eq!(
            prepared.settings.charging_start_threshold,
            base.charging_start_threshold
        );
        assert_eq!(
            prepared.settings.charging_end_threshold,
            base.charging_end_threshold
        );
    }

    #[test]
    fn omitted_charge_fields_never_clear_values() {
        let base = base();
        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared =
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).expect("apply");
        assert_eq!(prepared.settings.charging_profile, base.charging_profile);
        assert_eq!(prepared.settings.charging_priority, base.charging_priority);
    }

    #[test]
    fn charge_fields_are_applied_selectively() {
        let base = base();
        let json = charge_edit(
            r#""chargingProfile": "high-capacity",
               "chargingStartThreshold": 80"#,
        );
        let edit = decode_settings_edit(&json).expect("DTO");
        let prepared =
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).expect("apply");

        assert_eq!(
            prepared.settings.charging_profile.as_deref(),
            Some("high-capacity")
        );
        assert_eq!(prepared.settings.charging_start_threshold, Some(80));
        // Non fournis : inchangés.
        assert_eq!(prepared.settings.charging_priority, base.charging_priority);
        assert_eq!(
            prepared.settings.charging_end_threshold,
            base.charging_end_threshold
        );
        // Le JSON envoyé contient les autres champs intacts.
        assert!(prepared.json.contains("futureField"));
        assert!(prepared.json.contains("custom-ac"));
    }

    #[test]
    fn charge_values_must_belong_to_hardware_lists() {
        let base = base();
        for (fields, field_name) in [
            (r#""chargingProfile": "turbo""#, "chargingProfile"),
            (r#""chargingPriority": "turbo""#, "chargingPriority"),
            (r#""chargingStartThreshold": 55"#, "chargingStartThreshold"),
            (r#""chargingEndThreshold": 55"#, "chargingEndThreshold"),
        ] {
            let edit = decode_settings_edit(&charge_edit(fields)).expect("DTO");
            let error = apply_settings_edit(&base, &edit, Some(&capabilities()), None).unwrap_err();
            assert_eq!(
                error,
                SaveError::Validation {
                    field: field_name.to_owned()
                }
            );
        }
    }

    #[test]
    fn charge_edit_without_capabilities_is_refused() {
        let base = base();
        let edit =
            decode_settings_edit(&charge_edit(r#""chargingProfile": "balanced""#)).expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, None, None).unwrap_err(),
            SaveError::Validation { .. }
        ));
    }

    #[test]
    fn thresholds_must_stay_strictly_increasing() {
        let base = base();
        // 80 → 70 : couple inversé.
        let edit = decode_settings_edit(&charge_edit(
            r#""chargingStartThreshold": 80, "chargingEndThreshold": 70"#,
        ))
        .expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).unwrap_err(),
            SaveError::Validation { .. }
        ));

        // 60 → 60 : égalité refusée.
        let edit = decode_settings_edit(&charge_edit(
            r#""chargingStartThreshold": 60, "chargingEndThreshold": 60"#,
        ))
        .expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).unwrap_err(),
            SaveError::Validation { .. }
        ));

        // 40 → 100 : accepté.
        let edit = decode_settings_edit(&charge_edit(
            r#""chargingStartThreshold": 40, "chargingEndThreshold": 100"#,
        ))
        .expect("DTO");
        assert!(apply_settings_edit(&base, &edit, Some(&capabilities()), None).is_ok());
    }

    #[test]
    fn threshold_out_of_range_is_refused_before_the_daemon() {
        let base = base();
        // 120 > 100 : refusé par `Settings::validate` (même règle que tccd).
        let edit =
            decode_settings_edit(&charge_edit(r#""chargingEndThreshold": 120"#)).expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).unwrap_err(),
            SaveError::Validation { .. }
        ));
    }

    #[test]
    fn state_matrix_serialization_is_preserved() {
        let base = base();
        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        let json: serde_json::Value = serde_json::from_str(&prepared.json).expect("JSON");
        assert_eq!(
            json["stateMatrix"],
            serde_json::to_value(&base.state_matrix).unwrap()
        );
        assert_eq!(
            json["stateMap"],
            serde_json::to_value(&base.state_map).unwrap()
        );
    }

    fn profiles() -> tcc_core::ProfileCollection {
        tcc_core::ProfileCollection::new(vec![
            tcc_core::Profile::new("__office__", "Office"),
            tcc_core::Profile::new("custom-studio", "Studio"),
        ])
    }

    fn matrix_edit(entries: &str) -> String {
        charge_edit(&format!(r#""stateMatrix": [{entries}]"#))
    }

    #[test]
    fn state_matrix_is_replaced_and_other_fields_preserved() {
        let base = base();
        let json = matrix_edit(
            r#"{"ppd": "balanced", "power": "battery", "profileId": "custom-studio"},
               {"ppd": "power-saver", "power": "ac", "profileId": "__office__"}"#,
        );
        let edit = decode_settings_edit(&json).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, Some(&capabilities()), Some(&profiles()))
            .expect("application");

        // Matrice remplacée (les anciennes entrées de `base` disparaissent).
        assert_eq!(prepared.settings.state_matrix.len(), 2);
        let balanced_bat = tcc_core::StateKey::new(
            Some(tcc_core::PpdProfile::Balanced),
            tcc_core::PowerSource::Battery,
        );
        assert_eq!(
            prepared
                .settings
                .state_matrix
                .get(&balanced_bat)
                .map(|id| id.as_str()),
            Some("custom-studio")
        );
        // Une clé absente retombe sur le fallback daemon.
        let performance_ac = tcc_core::StateKey::new(
            Some(tcc_core::PpdProfile::Performance),
            tcc_core::PowerSource::Ac,
        );
        assert!(
            prepared
                .settings
                .state_matrix
                .get(&performance_ac)
                .is_none()
        );

        // Les autres sections sont intactes (charge, éclairage, extra).
        assert_eq!(prepared.settings.charging_profile, base.charging_profile);
        assert_eq!(
            prepared.settings.charging_start_threshold,
            base.charging_start_threshold
        );
        assert_eq!(prepared.settings.keyboard_states, base.keyboard_states);
        assert_eq!(prepared.settings.lightbar_states, base.lightbar_states);
        assert_eq!(prepared.settings.extra, base.extra);
        assert_eq!(prepared.settings.state_map, base.state_map);
    }

    #[test]
    fn empty_state_matrix_clears_and_absence_keeps_it() {
        let base = base();
        let edit = decode_settings_edit(&matrix_edit("")).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert!(prepared.settings.state_matrix.is_empty());

        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert_eq!(prepared.settings.state_matrix, base.state_matrix);
    }

    #[test]
    fn state_matrix_entries_are_strictly_validated() {
        let base = base();
        let collection = profiles();
        for entries in [
            // PPD inconnu.
            r#"{"ppd": "turbo", "power": "ac", "profileId": "__office__"}"#,
            // Alimentation inconnue.
            r#"{"ppd": "balanced", "power": "mains", "profileId": "__office__"}"#,
            // Profil absent de la collection live.
            r#"{"ppd": "balanced", "power": "ac", "profileId": "custom-gone"}"#,
            // ID invalide.
            r#"{"ppd": "balanced", "power": "ac", "profileId": "../etc"}"#,
            // Doublon de clé.
            r#"{"ppd": "balanced", "power": "ac", "profileId": "__office__"},
               {"ppd": "balanced", "power": "ac", "profileId": "custom-studio"}"#,
        ] {
            let edit = decode_settings_edit(&matrix_edit(entries)).expect("DTO");
            assert!(
                matches!(
                    apply_settings_edit(&base, &edit, None, Some(&collection)).unwrap_err(),
                    SaveError::Validation { .. }
                ),
                "entrées: {entries}"
            );
        }

        // Sans collection live, une entrée non vide est refusée ; une matrice
        // vide reste acceptée (effacement).
        let edit = decode_settings_edit(&matrix_edit(
            r#"{"ppd": "balanced", "power": "ac", "profileId": "__office__"}"#,
        ))
        .expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, None, None).unwrap_err(),
            SaveError::Validation { .. }
        ));
    }

    #[test]
    fn state_matrix_dto_is_strict() {
        for entries in [
            r#"{"ppd": "balanced", "power": "ac"}"#,
            r#"{"ppd": "balanced", "power": "ac", "profileId": 3}"#,
            r#"{"ppd": "balanced", "power": "ac", "profileId": "x", "extra": true}"#,
        ] {
            assert_eq!(
                decode_settings_edit(&matrix_edit(entries)).unwrap_err(),
                SaveError::InvalidPayload,
                "entrées: {entries}"
            );
        }
    }

    fn shutdown_edit(fields: &str) -> String {
        charge_edit(fields)
    }

    #[test]
    fn shutdown_time_is_applied_normalized_and_never_clears_silently() {
        let base = base();
        for (value, expected) in [("07:05", "07:05"), ("23:59", "23:59"), ("00:00", "00:00")] {
            let edit =
                decode_settings_edit(&shutdown_edit(&format!(r#""shutdownTime": "{value}""#)))
                    .expect("DTO");
            let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
            assert_eq!(prepared.settings.shutdown_time.as_deref(), Some(expected));
            // Les autres champs sont préservés.
            assert_eq!(prepared.settings.charging_profile, base.charging_profile);
            assert_eq!(prepared.settings.state_matrix, base.state_matrix);
            assert_eq!(prepared.settings.extra, base.extra);
        }

        // Absent / null : aucune modification.
        let edit = decode_settings_edit(&shutdown_edit(r#""shutdownTime": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert_eq!(prepared.settings.shutdown_time, base.shutdown_time);

        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert_eq!(prepared.settings.shutdown_time, base.shutdown_time);
    }

    #[test]
    fn shutdown_time_is_cleared_only_on_explicit_flag() {
        let base = base();
        let edit = decode_settings_edit(&shutdown_edit(
            r#""shutdownTime": null, "clearShutdownTime": true"#,
        ))
        .expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert_eq!(prepared.settings.shutdown_time, None);
        assert!(prepared.json.contains(r#""shutdownTime":null"#));

        // Effacer une valeur déjà absente reste valide (idempotent).
        let mut empty = base.clone();
        empty.shutdown_time = None;
        let prepared = apply_settings_edit(&empty, &edit, None, None).expect("application");
        assert_eq!(prepared.settings.shutdown_time, None);
    }

    #[test]
    fn shutdown_time_validation_is_strict() {
        let base = base();
        for value in [
            "", "9:00", "24:00", "23:60", "12:5", "123:45", "aa:bb", "12h30", "12:30:00", "+1:00",
            "12:-1",
        ] {
            let edit =
                decode_settings_edit(&shutdown_edit(&format!(r#""shutdownTime": "{value}""#)))
                    .expect("DTO");
            assert!(
                matches!(
                    apply_settings_edit(&base, &edit, None, None).unwrap_err(),
                    SaveError::Validation { .. }
                ),
                "valeur: {value}"
            );
        }

        // clear + time simultanés : ambigu, refusé.
        let edit = decode_settings_edit(&shutdown_edit(
            r#""shutdownTime": "22:30", "clearShutdownTime": true"#,
        ))
        .expect("DTO");
        assert!(matches!(
            apply_settings_edit(&base, &edit, None, None).unwrap_err(),
            SaveError::Validation { .. }
        ));

        // Type strict côté DTO.
        assert_eq!(
            decode_settings_edit(&shutdown_edit(r#""shutdownTime": 2230"#)).unwrap_err(),
            SaveError::InvalidPayload
        );
        assert_eq!(
            decode_settings_edit(&shutdown_edit(r#""clearShutdownTime": "yes""#)).unwrap_err(),
            SaveError::InvalidPayload
        );
    }

    #[test]
    fn ppd_override_is_opt_in_and_preserved() {
        let base = base();
        assert!(!base.ppd_override);
        let edit = decode_settings_edit(&charge_edit(r#""ppdOverride": true"#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert!(prepared.settings.ppd_override);
        assert!(prepared.json.contains(r#""ppdOverride":true"#));

        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("application");
        assert!(!prepared.settings.ppd_override);

        let edit = decode_settings_edit(&charge_edit(r#""ppdOverride": false"#)).expect("DTO");
        let prepared =
            apply_settings_edit(&prepared.settings, &edit, None, None).expect("application");
        assert!(!prepared.settings.ppd_override);
    }

    #[test]
    fn charging_type_is_applied_validated_and_preserved() {
        let base = base();
        let edit = decode_settings_edit(&charge_edit(r#""chargingType": "Fast""#)).expect("DTO");
        let prepared =
            apply_settings_edit(&base, &edit, Some(&capabilities()), None).expect("type de charge");
        assert_eq!(prepared.settings.charging_type.as_deref(), Some("Fast"));
        assert!(prepared.json.contains("\"chargingType\":\"Fast\""));

        // `null`/absent conserve la valeur.
        let edit = decode_settings_edit(&charge_edit(r#""chargingType": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&prepared.settings, &edit, Some(&capabilities()), None)
            .expect("préservation");
        assert_eq!(prepared.settings.charging_type.as_deref(), Some("Fast"));

        // Liste backend vide : les valeurs ABI standard sont acceptées et
        // canonicalisées.
        let mut empty = capabilities();
        empty.charge_types.clear();
        let edit = decode_settings_edit(&charge_edit(r#""chargingType": "fast""#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, Some(&empty), None).expect("ABI");
        assert_eq!(prepared.settings.charging_type.as_deref(), Some("Fast"));

        for (value, caps) in [
            ("Fast!", Some(capabilities())),
            ("Trickle", Some(capabilities())),
            (&"a".repeat(65), Some(capabilities())),
            ("Fast", None),
        ] {
            let edit = decode_settings_edit(&charge_edit(&format!(r#""chargingType": "{value}""#)))
                .expect("DTO");
            let error = apply_settings_edit(&base, &edit, caps.as_ref(), None)
                .expect_err("doit être refusé");
            assert_eq!(error.code(), "Validation");
            assert_eq!(error.detail(), "chargingType");
        }

        let mut read_only = capabilities();
        read_only.charge_type_writable = false;
        let edit = decode_settings_edit(&charge_edit(r#""chargingType": "Fast""#)).expect("DTO");
        assert_eq!(
            apply_settings_edit(&base, &edit, Some(&read_only), None)
                .expect_err("lecture seule")
                .detail(),
            "chargingType"
        );
    }

    #[test]
    fn fn_lock_status_is_applied_and_preserved() {
        let base = base();
        for value in [true, false] {
            let edit = decode_settings_edit(&charge_edit(&format!(r#""fnLockStatus": {value}"#)))
                .expect("DTO");
            let prepared = apply_settings_edit(&base, &edit, None, None).expect("Fn-Lock");
            assert_eq!(prepared.settings.fn_lock_status, Some(value));
        }
        let edit = decode_settings_edit(&charge_edit(r#""fnLockStatus": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("préservation");
        assert_eq!(prepared.settings.fn_lock_status, None);
    }

    #[test]
    fn keyboard_states_are_validated_and_can_be_cleared() {
        let mut base = base();
        base.keyboard_states = vec![KeyboardState::new(KeyboardColorMode::Static, 10, 1, 2, 3)];

        let edit = decode_settings_edit(&charge_edit(
            r#""keyboardStates": [
                { "mode": "static", "brightness": 128, "red": 255,
                  "green": 0, "blue": 10 },
                { "mode": "breathing", "brightness": 20, "red": -1,
                  "green": -1, "blue": -1 }
            ]"#,
        ))
        .expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("clavier");
        assert_eq!(prepared.settings.keyboard_states.len(), 2);
        assert_eq!(
            prepared.settings.keyboard_states[1].mode,
            KeyboardColorMode::Breathing
        );
        assert_eq!(prepared.settings.keyboard_states[1].red, -1);

        // `[]` efface, `null`/absent préserve.
        let edit = decode_settings_edit(&charge_edit(r#""keyboardStates": []"#)).expect("DTO");
        let prepared = apply_settings_edit(&prepared.settings, &edit, None, None).expect("efface");
        assert!(prepared.settings.keyboard_states.is_empty());
        let edit = decode_settings_edit(&charge_edit(r#""keyboardStates": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("préserve");
        assert_eq!(prepared.settings.keyboard_states.len(), 1);
    }

    #[test]
    fn keyboard_state_invalid_values_are_rejected() {
        let base = base();
        for (states, detail) in [
            (
                r#"[{ "mode": "rainbow", "brightness": 1, "red": 0, "green": 0, "blue": 0 }]"#,
                "keyboardStates[0].mode",
            ),
            (
                r#"[{ "mode": "static", "brightness": 1, "red": 256, "green": 0, "blue": 0 }]"#,
                "keyboardStates[0]",
            ),
            (
                r#"[{ "mode": "static", "brightness": 1, "red": -2, "green": 0, "blue": 0 }]"#,
                "keyboardStates[0]",
            ),
        ] {
            let edit =
                decode_settings_edit(&charge_edit(&format!(r#""keyboardStates": {states}"#)))
                    .expect("DTO");
            let error = apply_settings_edit(&base, &edit, None, None).expect_err("refus");
            assert_eq!(error.code(), "Validation");
            assert_eq!(error.detail(), detail);
        }

        let too_many = (0..=MAX_KEYBOARD_STATES)
            .map(|_| r#"{ "brightness": 1, "red": 0, "green": 0, "blue": 0 }"#)
            .collect::<Vec<_>>()
            .join(",");
        let edit =
            decode_settings_edit(&charge_edit(&format!(r#""keyboardStates": [{too_many}]"#)))
                .expect("DTO");
        assert_eq!(
            apply_settings_edit(&base, &edit, None, None)
                .expect_err("trop de zones")
                .detail(),
            "keyboardStates"
        );
    }

    #[test]
    fn lightbar_states_explicit_null_clears_and_object_applies() {
        let mut base = base();
        base.lightbar_states = Some(LightbarState {
            brightness: 10,
            red: 1,
            green: 2,
            blue: 3,
        });

        let edit = decode_settings_edit(&charge_edit(
            r#""lightbarStates": { "brightness": 200, "red": 255,
                                   "green": 128, "blue": 0 }"#,
        ))
        .expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("lightbar");
        assert_eq!(
            prepared.settings.lightbar_states,
            Some(LightbarState {
                brightness: 200,
                red: 255,
                green: 128,
                blue: 0,
            })
        );

        // `null` explicite efface ; absent préserve.
        let edit = decode_settings_edit(&charge_edit(r#""lightbarStates": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&prepared.settings, &edit, None, None).expect("efface");
        assert_eq!(prepared.settings.lightbar_states, None);
        let edit = decode_settings_edit(&charge_edit(r#""chargingProfile": null"#)).expect("DTO");
        let prepared = apply_settings_edit(&base, &edit, None, None).expect("préserve");
        assert_eq!(prepared.settings.lightbar_states, base.lightbar_states);

        let edit = decode_settings_edit(&charge_edit(
            r#""lightbarStates": { "brightness": 1, "red": 256,
                                   "green": 0, "blue": 0 }"#,
        ))
        .expect("DTO");
        assert_eq!(
            apply_settings_edit(&base, &edit, None, None)
                .expect_err("hors bornes")
                .detail(),
            "lightbarStates"
        );
    }

    #[test]
    fn concurrent_change_is_a_conflict() {
        let base = base();
        let mut current = base.clone();
        // Un autre client active Fahrenheit et change un seuil de charge.
        current.fahrenheit = true;
        current.charging_start_threshold = Some(70);
        assert_eq!(
            prepare_settings_save(&base, &current, &edit_json(), None, None).unwrap_err(),
            SaveError::Conflict
        );
        // Identiques : l'édition est préparée.
        assert!(prepare_settings_save(&base, &base, &edit_json(), None, None).is_ok());
    }

    #[test]
    fn unknown_field_is_rejected() {
        let json = edit_json().replace('}', r#", "stateMap": {} }"#);
        assert_eq!(
            decode_settings_edit(&json).unwrap_err(),
            SaveError::InvalidPayload
        );
    }

    #[test]
    fn missing_field_is_rejected() {
        let json = edit_json().replace("\"fahrenheit\": true,", "");
        assert_eq!(
            decode_settings_edit(&json).unwrap_err(),
            SaveError::InvalidPayload
        );
    }

    #[test]
    fn charge_field_types_are_strict() {
        for fields in [
            r#""chargingStartThreshold": "80""#,
            r#""chargingStartThreshold": -1"#,
            r#""chargingEndThreshold": 300"#,
            r#""chargingProfile": 3"#,
        ] {
            let edit = decode_settings_edit(&charge_edit(fields));
            assert_eq!(
                edit.unwrap_err(),
                SaveError::InvalidPayload,
                "champ: {fields}"
            );
        }
    }

    #[test]
    fn oversized_edit_is_rejected_before_parsing() {
        let json = "x".repeat(MAX_SETTINGS_EDIT_BYTES + 1);
        assert_eq!(
            decode_settings_edit(&json).unwrap_err(),
            SaveError::PayloadTooLarge
        );
    }

    #[test]
    fn oversized_settings_payload_is_rejected() {
        let mut base = base();
        // Champ inconnu volumineux : la collection sérialisée dépasse 1 Mio.
        base.extra.insert(
            "huge".to_owned(),
            serde_json::Value::String("x".repeat(MAX_SETTINGS_PAYLOAD_BYTES)),
        );
        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        assert_eq!(
            apply_settings_edit(&base, &edit, None, None).unwrap_err(),
            SaveError::PayloadTooLarge
        );
    }

    #[test]
    fn settings_edit_roundtrips_through_json() {
        let edit = decode_settings_edit(&edit_json()).expect("DTO");
        assert_eq!(
            edit,
            SettingsEdit {
                fahrenheit: true,
                cpu_settings_enabled: false,
                fan_control_enabled: true,
                keyboard_backlight_control_enabled: false,
                charging_profile: None,
                charging_priority: None,
                charging_start_threshold: None,
                charging_end_threshold: None,
                state_matrix: None,
                shutdown_time: None,
                clear_shutdown_time: false,
                ppd_override: None,
                charging_type: None,
                fn_lock_status: None,
                keyboard_states: None,
                lightbar_states: None,
            }
        );
        // Le JSON canonique reste un objet `Settings` complet.
        let prepared = apply_settings_edit(&base(), &edit, None, None).expect("application");
        let decoded: Settings = serde_json::from_str(&prepared.json).expect("relecture");
        assert_eq!(decoded, prepared.settings);
        assert_eq!(decoded.state_matrix.len(), 1);
    }
}
