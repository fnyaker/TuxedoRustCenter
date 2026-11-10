// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Logique pure de l'interface : formats, validation locale des profils,
//! import/export, modes d'affichage. Aucune E/S ni dépendance à gpui.
//!
//! Le daemon reste seul juge : ces règles reproduisent celles de l'interface
//! Qt (`ProfileRules.qml`, `profile_edit.rs`, `profile_transfer.rs`) pour
//! signaler une erreur avant l'envoi, jamais pour la contourner.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tcc_core::{
    FanCurve, MAX_DESCRIPTION_LEN, MAX_PROFILE_NAME_LEN, MAX_PROFILES, Profile, ProfileCollection,
    ProfileId, Settings, StateKey,
};

use crate::i18n::{t, tf};

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// Tiret affiché quand une valeur est absente.
pub const DASH: &str = "—";

/// Nombre avec `decimals` décimales.
#[must_use]
pub fn fmt_num(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}")
}

/// Température en °C ou °F.
#[must_use]
pub fn temp_value(celsius: f64, fahrenheit: bool) -> f64 {
    if fahrenheit {
        celsius * 9. / 5. + 32.
    } else {
        celsius
    }
}

/// Unité de température.
#[must_use]
pub fn temp_unit(fahrenheit: bool) -> &'static str {
    if fahrenheit { "°F" } else { "°C" }
}

/// Température formatée, ou un tiret.
#[must_use]
pub fn fmt_temp(celsius: Option<f64>, fahrenheit: bool) -> String {
    celsius.map_or_else(
        || DASH.to_owned(),
        |c| format!("{:.0} {}", temp_value(c, fahrenheit), temp_unit(fahrenheit)),
    )
}

/// Taux de rafraîchissement : `60 Hz`, `59.94 Hz`.
#[must_use]
pub fn fmt_hz(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.01 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

/// Chaîne d'un champ JSON (vide si absent ou d'un autre type).
#[must_use]
pub fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Chaîne d'un champ JSON, ou « Non disponible ».
#[must_use]
pub fn text_or_unavailable(value: &Value) -> String {
    match value.as_str() {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => t("common.unavailable"),
    }
}

/// Liste de chaînes d'un tableau JSON.
#[must_use]
pub fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Liste d'entiers d'un tableau JSON.
#[must_use]
pub fn ints(value: &Value) -> Vec<i64> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

/// Libellé d'un mode ODM (`power_save` → « Économie d'énergie »), sinon la
/// valeur brute.
#[must_use]
pub fn odm_label(name: &str) -> String {
    let key = format!("odm.mode.{name}");
    if crate::i18n::has(&key) {
        t(&key)
    } else {
        name.to_owned()
    }
}

/// Libellé du profil PPD.
#[must_use]
pub fn ppd_label(ppd: &str) -> String {
    t(match ppd {
        "power-saver" => "settings.ppdSaver",
        "balanced" => "settings.ppdBalanced",
        "performance" => "settings.ppdPerformance",
        _ => "settings.ppdUnknown",
    })
}

/// Libellé de la source d'alimentation.
#[must_use]
pub fn power_label(power: &str) -> String {
    t(match power {
        "ac" => "dash.ac",
        "battery" => "dash.battery",
        _ => "state.unavailable",
    })
}

/// Libellé du mode graphique PRIME.
#[must_use]
pub fn prime_label(mode: &str) -> String {
    t(match mode.to_ascii_lowercase().as_str() {
        "igpu" => "settings.graphics.igpu",
        "dgpu" => "settings.graphics.dgpu",
        "on-demand" | "ondemand" => "settings.graphics.ondemand",
        "off" => "settings.graphics.off",
        _ => "settings.ppdUnknown",
    })
}

// ---------------------------------------------------------------------------
// Historique des capteurs
// ---------------------------------------------------------------------------

/// Nombre de points conservés (les capteurs sont publiés toutes les 2 s).
pub const HISTORY_LEN: usize = 48;

/// Série temporelle locale : le contrat n'expose pas d'historique.
#[derive(Debug, Clone, Default)]
pub struct History(Vec<f32>);

impl History {
    /// Ajoute un point, en oubliant le plus ancien au-delà de
    /// [`HISTORY_LEN`]. Une valeur absente n'ajoute rien.
    pub fn push(&mut self, value: Option<f64>) {
        let Some(value) = value.filter(|v| v.is_finite()) else {
            return;
        };
        if self.0.len() >= HISTORY_LEN {
            self.0.remove(0);
        }
        self.0.push(value as f32);
    }

    /// Vide la série (matériel disparu).
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Points, du plus ancien au plus récent.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Profils
// ---------------------------------------------------------------------------

/// Modes de ventilation proposés, dans l'ordre du sélecteur.
pub const FAN_MODES: [&str; 6] = [
    "silent",
    "quiet",
    "balanced",
    "performance",
    "custom",
    "auto",
];

/// Clé du mode de ventilation d'un profil (`auto` si le contrôle est coupé).
#[must_use]
pub fn fan_mode(profile: &Profile) -> &'static str {
    if !profile.fan.use_control {
        return "auto";
    }
    let name = profile.fan.fan_profile.to_ascii_lowercase();
    FAN_MODES
        .into_iter()
        .find(|mode| *mode == name && *mode != "auto")
        .unwrap_or("auto")
}

/// Nom du contrat pour une clé de mode (`quiet` → `Quiet`).
#[must_use]
pub fn canonical_fan_profile(key: &str) -> &'static str {
    match key {
        "silent" => "Silent",
        "quiet" => "Quiet",
        "performance" => "Performance",
        "custom" => "Custom",
        _ => "Balanced",
    }
}

/// kHz → GHz arrondi au centième.
#[must_use]
pub fn khz_to_ghz(khz: u64) -> f64 {
    (khz as f64 / 1e4).round() / 100.
}

/// GHz → kHz, par pas de 100 MHz.
#[must_use]
pub fn ghz_to_khz(ghz: f64) -> u64 {
    ((ghz * 10.).round() * 100_000.).max(0.) as u64
}

/// Nom comparé sans casse ni espaces de bord.
#[must_use]
pub fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn unique_name(taken: &BTreeSet<String>, source: &str, word: &str) -> String {
    let source = source.trim();
    for counter in 1..=MAX_PROFILES + 1 {
        let suffix = if counter == 1 {
            format!(" ({word})")
        } else {
            format!(" ({word} {counter})")
        };
        let keep = MAX_PROFILE_NAME_LEN.saturating_sub(suffix.chars().count());
        let candidate = format!("{}{suffix}", truncate_chars(source, keep));
        if !taken.contains(&normalized_name(&candidate)) {
            return candidate;
        }
    }
    format!("{} ({word})", truncate_chars(source, 32))
}

fn names_of(collection: &ProfileCollection) -> BTreeSet<String> {
    collection
        .iter()
        .map(|profile| normalized_name(&profile.name))
        .collect()
}

/// Nom unique pour une copie : `X (copie)`, `X (copie 2)`…
#[must_use]
pub fn unique_copy_name(collection: &ProfileCollection, source: &str) -> String {
    unique_name(&names_of(collection), source, "copie")
}

/// Profil vierge proposé par « Nouveau profil ».
#[must_use]
pub fn new_profile() -> Profile {
    let mut profile = Profile::new(ProfileId::generate(), t("editor.newProfileName"));
    profile.fan.custom_fan_curve = FanCurve::default_custom();
    profile
}

/// Erreurs de validation locale : `(champ, clé i18n)`.
pub type FieldErrors = Vec<(&'static str, &'static str)>;

/// Première erreur parmi `fields`.
#[must_use]
pub fn first_error(errors: &FieldErrors, fields: &[&str]) -> Option<&'static str> {
    errors
        .iter()
        .find(|(field, _)| fields.contains(field))
        .map(|(_, key)| *key)
}

/// Valide un brouillon de profil. `other_names` : noms normalisés des autres
/// profils ; `ctgp_max` : borne de l'offset cTGP exposée par le matériel.
#[must_use]
pub fn validate_profile(
    profile: &Profile,
    other_names: &BTreeSet<String>,
    ctgp_max: f64,
) -> FieldErrors {
    let mut errors = FieldErrors::new();
    let name = profile.name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        errors.push(("name", "editor.error.nameEmpty"));
    } else if name.chars().count() > MAX_PROFILE_NAME_LEN {
        errors.push(("name", "editor.error.nameTooLong"));
    } else if other_names.contains(&normalized_name(name)) {
        errors.push(("name", "editor.error.nameDuplicate"));
    }
    if profile.description.chars().count() > MAX_DESCRIPTION_LEN {
        errors.push(("description", "editor.error.descriptionTooLong"));
    }
    let display = &profile.display;
    if display.brightness > 100 {
        errors.push(("brightness", "editor.error.brightnessRange"));
    }
    if display.use_refresh_rate {
        match display.refresh_rate {
            Some(rate) if rate.is_finite() && (1.0..=1000.0).contains(&rate) => {}
            _ => errors.push(("refreshRate", "editor.error.refreshRange")),
        }
    }
    if display.use_resolution {
        let valid = matches!(
            (display.x_resolution, display.y_resolution),
            (Some(w), Some(h)) if (1..=16_384).contains(&w) && (1..=16_384).contains(&h)
        );
        if !valid {
            errors.push(("resolution", "editor.error.resolutionRange"));
        }
    }
    let cpu = &profile.cpu;
    if cpu
        .online_cores
        .is_some_and(|cores| cores == 0 || cores > tcc_core::MAX_CPU_ONLINE_CORES)
    {
        errors.push(("cores", "editor.error.coresRange"));
    }
    let range = tcc_core::MIN_CPU_FREQUENCY_KHZ..=tcc_core::MAX_CPU_FREQUENCY_KHZ;
    if cpu
        .scaling_min_frequency
        .is_some_and(|min| !range.contains(&min))
    {
        errors.push(("freqMin", "editor.error.freqRange"));
    }
    if let Some(max) = cpu
        .scaling_max_frequency
        .and_then(tcc_core::FrequencyLimit::hz)
    {
        if !range.contains(&max) {
            errors.push(("freqMax", "editor.error.freqRange"));
        } else if cpu.scaling_min_frequency.is_some_and(|min| min > max) {
            errors.push(("freqMax", "editor.error.freqOrder"));
        }
    }
    let fan = &profile.fan;
    let speed_ok = |v: f32| v.is_finite() && (0.0..=100.0).contains(&v);
    if !speed_ok(fan.minimum_fanspeed) {
        errors.push(("fanMin", "editor.error.fanRange"));
    }
    if !speed_ok(fan.maximum_fanspeed) {
        errors.push(("fanMax", "editor.error.fanRange"));
    } else if fan.minimum_fanspeed > fan.maximum_fanspeed {
        errors.push(("fanMin", "editor.error.fanOrder"));
    }
    if !fan.offset_fanspeed.is_finite() || !(-100.0..=100.0).contains(&fan.offset_fanspeed) {
        errors.push(("fanOffset", "editor.error.fanOffsetRange"));
    }
    if let Some(key) = curve_error(&fan.custom_fan_curve.table_cpu) {
        errors.push(("fanCurve", key));
    }
    if let Some(name) = profile.odm_profile.name.as_deref()
        && !odm_name_valid(name)
    {
        errors.push(("odmProfile", "editor.error.odmProfile"));
    }
    let tdp = &profile.odm_power_limits.tdp_values;
    let mut previous = f64::NEG_INFINITY;
    let tdp_valid = tdp.len() <= 3
        && tdp.iter().all(|value| {
            let ok = value.is_finite() && (1.0..=1000.0).contains(value) && *value > previous;
            previous = *value;
            ok
        });
    if !tdp_valid {
        errors.push(("tdpValues", "editor.error.tdpValues"));
    }
    let ctgp = profile.nvidia_power_ctrl_profile.ctgp_offset;
    if !ctgp.is_finite() || !(0.0..=ctgp_max.max(0.)).contains(&ctgp) {
        errors.push(("ctgpOffset", "editor.error.ctgpOffset"));
    }
    errors
}

/// Nom ODM accepté par le service matériel.
#[must_use]
pub fn odm_name_valid(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Erreur d'une courbe de ventilation (clé i18n), ou `None` si elle est
/// valide : au moins deux points, bornés, températures strictement
/// croissantes.
#[must_use]
pub fn curve_error(points: &[tcc_core::FanTableEntry]) -> Option<&'static str> {
    if points.len() < 2 {
        return Some("fanCurve.error.min");
    }
    if points.len() > tcc_core::MAX_FAN_TABLE_ENTRIES {
        return Some("fanCurve.error.max");
    }
    let mut previous = f32::NEG_INFINITY;
    for point in points {
        if !point.temp.is_finite() || !point.speed.is_finite() {
            return Some("fanCurve.error.number");
        }
        if !(0.0..=100.0).contains(&point.temp) || !(0.0..=100.0).contains(&point.speed) {
            return Some("fanCurve.error.range");
        }
        if point.temp <= previous {
            return Some("fanCurve.error.order");
        }
        previous = point.temp;
    }
    None
}

// ---------------------------------------------------------------------------
// Matrice d'états
// ---------------------------------------------------------------------------

/// Lignes de la matrice (profils PPD).
pub const MATRIX_PPD: [&str; 3] = ["power-saver", "balanced", "performance"];
/// Colonnes de la matrice (alimentation).
pub const MATRIX_POWER: [&str; 2] = ["ac", "battery"];

/// Profil assigné à une cellule (`balanced/ac`), ou `None` pour le repli.
#[must_use]
pub fn matrix_cell(settings: &Settings, cell: &str) -> Option<String> {
    let key = StateKey::from_matrix_key(cell)?;
    settings
        .state_matrix
        .get(&key)
        .map(|id| id.as_str().to_owned())
}

/// Assigne (`Some`) ou libère (`None`) une cellule.
pub fn set_matrix_cell(settings: &mut Settings, cell: &str, profile: Option<&str>) {
    let Some(key) = StateKey::from_matrix_key(cell) else {
        return;
    };
    match profile {
        Some(id) => {
            settings.state_matrix.insert(key, ProfileId::new(id));
        }
        None => {
            settings.state_matrix.remove(&key);
        }
    }
}

/// Vrai si `id` est assigné à au moins une cellule de la colonne `power`.
#[must_use]
pub fn assigned_to(settings: &Settings, id: &str, power: &str) -> bool {
    MATRIX_PPD
        .iter()
        .any(|ppd| matrix_cell(settings, &format!("{ppd}/{power}")).as_deref() == Some(id))
}

// ---------------------------------------------------------------------------
// Import / export
// ---------------------------------------------------------------------------

/// Version du format d'échange.
pub const EXPORT_VERSION: u32 = 1;
/// Taille maximale d'un fichier d'import ou d'export (1 Mio).
pub const MAX_TRANSFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportEnvelope {
    export_version: u32,
    profiles: Vec<Profile>,
}

/// Compteurs présentés après un import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Profils ajoutés.
    pub imported: usize,
    /// Profils renommés à cause d'un conflit de nom.
    pub renamed: usize,
    /// Presets ignorés.
    pub skipped: usize,
}

/// Erreur d'import/export : clé i18n à afficher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferError(pub &'static str);

/// Exporte les profils personnalisés : `(json, nombre)`.
///
/// # Errors
///
/// Rien à exporter, ou export trop volumineux.
pub fn build_export(collection: &ProfileCollection) -> Result<(String, usize), TransferError> {
    let profiles: Vec<Profile> = collection
        .iter()
        .filter(|profile| !profile.is_preset())
        .cloned()
        .collect();
    if profiles.is_empty() {
        return Err(TransferError("admin.error.nothingToExport"));
    }
    let count = profiles.len();
    let json = serde_json::to_string_pretty(&ExportEnvelope {
        export_version: EXPORT_VERSION,
        profiles,
    })
    .map_err(|_| TransferError("admin.error.invalidPayload"))?;
    if json.len() > MAX_TRANSFER_BYTES {
        return Err(TransferError("admin.error.payloadTooLarge"));
    }
    Ok((json, count))
}

/// Lit une enveloppe versionnée ou un tableau de profils (format historique).
///
/// # Errors
///
/// Fichier trop volumineux, JSON invalide ou version inconnue.
pub fn parse_import(text: &str) -> Result<Vec<Profile>, TransferError> {
    const INVALID: TransferError = TransferError("admin.error.invalidImport");
    if text.len() > MAX_TRANSFER_BYTES {
        return Err(TransferError("admin.error.payloadTooLarge"));
    }
    let value: Value = serde_json::from_str(text).map_err(|_| INVALID)?;
    if value.is_object() {
        let envelope: ExportEnvelope = serde_json::from_value(value).map_err(|_| INVALID)?;
        if envelope.export_version != EXPORT_VERSION {
            return Err(INVALID);
        }
        Ok(envelope.profiles)
    } else if value.is_array() {
        serde_json::from_value(value).map_err(|_| INVALID)
    } else {
        Err(INVALID)
    }
}

/// Ajoute des profils importés à `collection` : presets ignorés, nouvel
/// identifiant pour chacun (jamais d'écrasement), nom suffixé en cas de
/// conflit. Un profil invalide rejette l'import en bloc.
///
/// # Errors
///
/// Profil invalide ou limite de profils atteinte.
pub fn merge_import(
    collection: &mut ProfileCollection,
    imported: Vec<Profile>,
) -> Result<ImportSummary, TransferError> {
    let mut summary = ImportSummary::default();
    let mut candidates = Vec::new();
    for profile in imported {
        if profile.is_preset() {
            summary.skipped += 1;
            continue;
        }
        profile
            .validate()
            .map_err(|_| TransferError("admin.error.validation"))?;
        candidates.push(profile);
    }
    if collection.len() + candidates.len() > MAX_PROFILES {
        return Err(TransferError("admin.error.tooManyProfiles"));
    }
    let mut names = names_of(collection);
    for mut profile in candidates {
        profile.id = ProfileId::generate();
        if names.contains(&normalized_name(&profile.name)) {
            profile.name = unique_name(&names, &profile.name, "importé");
            summary.renamed += 1;
        }
        names.insert(normalized_name(&profile.name));
        collection.upsert(profile);
        summary.imported += 1;
    }
    Ok(summary)
}

/// Résumé d'import lisible.
#[must_use]
pub fn import_message(summary: ImportSummary) -> String {
    use crate::i18n::plural;
    if summary.imported == 0 {
        return t("profiles.importEmpty");
    }
    let count =
        |n: usize, one: &str, many: &str| plural(n, one, many).replacen("%1", &n.to_string(), 1);
    let mut parts = vec![count(
        summary.imported,
        "profiles.importedOne",
        "profiles.importedMany",
    )];
    if summary.renamed > 0 {
        parts.push(count(
            summary.renamed,
            "profiles.renamedOne",
            "profiles.renamedMany",
        ));
    }
    if summary.skipped > 0 {
        parts.push(count(
            summary.skipped,
            "profiles.skippedOne",
            "profiles.skippedMany",
        ));
    }
    parts.join(" · ")
}

/// Résumé d'export lisible.
#[must_use]
pub fn export_message(count: usize) -> String {
    let key = if count > 1 {
        "profiles.exportedMany"
    } else {
        "profiles.exportedOne"
    };
    tf(key, &[&count.to_string()])
}

// ---------------------------------------------------------------------------
// Modes d'affichage (agent de session)
// ---------------------------------------------------------------------------

/// Mode de l'écran interne exposé par l'agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub hz: f64,
    pub current: bool,
}

/// Tolérance de comparaison des taux (contrat : ≈ 0,5 Hz).
pub const RATE_TOLERANCE: f64 = 0.5;

/// Modes de l'écran interne d'un rapport `GetDisplayInfoJSON`.
#[must_use]
pub fn modes(display: &Value) -> Vec<Mode> {
    display["internalOutput"]["modes"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|mode| {
                    Some(Mode {
                        width: u32::try_from(mode["width"].as_u64()?).ok()?,
                        height: u32::try_from(mode["height"].as_u64()?).ok()?,
                        hz: mode["refreshHz"].as_f64().filter(|hz| *hz > 0.)?,
                        current: mode["current"] == true,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Résolutions distinctes, de la plus grande à la plus petite.
#[must_use]
pub fn resolutions(modes: &[Mode]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for mode in modes {
        if !out.contains(&(mode.width, mode.height)) {
            out.push((mode.width, mode.height));
        }
    }
    out.sort_by_key(|res| std::cmp::Reverse((res.0 * res.1, res.0)));
    out
}

/// Taux distincts d'une résolution, du plus rapide au plus lent.
#[must_use]
pub fn rates(modes: &[Mode], resolution: (u32, u32)) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for mode in modes {
        if (mode.width, mode.height) == resolution
            && !out.iter().any(|hz| (hz - mode.hz).abs() < 0.01)
        {
            out.push(mode.hz);
        }
    }
    out.sort_by(|a, b| b.total_cmp(a));
    out
}

/// Taux de `rates` le plus proche de `wanted`, dans la tolérance.
#[must_use]
pub fn matching_rate(rates: &[f64], wanted: f64) -> Option<f64> {
    rates
        .iter()
        .copied()
        .filter(|hz| (hz - wanted).abs() <= RATE_TOLERANCE)
        .min_by(|a, b| (a - wanted).abs().total_cmp(&(b - wanted).abs()))
}

/// Mode courant de l'écran interne.
#[must_use]
pub fn current_mode(modes: &[Mode]) -> Option<Mode> {
    modes.iter().copied().find(|mode| mode.current)
}

/// `2560 × 1600 · 240 Hz`.
#[must_use]
pub fn mode_label(mode: Mode) -> String {
    format!(
        "{} · {}",
        tf(
            "display.resolutionFormat",
            &[&mode.width.to_string(), &mode.height.to_string()]
        ),
        fmt_hz(mode.hz)
    )
}

/// `HH:MM` valide → `(heure, minute)`.
#[must_use]
pub fn parse_shutdown_time(value: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = value.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some((hours, minutes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tcc_core::FanTableEntry;

    #[test]
    fn formats_values() {
        assert_eq!(fmt_temp(Some(50.), false), "50 °C");
        assert_eq!(fmt_temp(Some(50.), true), "122 °F");
        assert_eq!(fmt_temp(None, false), DASH);
        assert_eq!(fmt_hz(240.), "240 Hz");
        assert_eq!(fmt_hz(59.94), "59.94 Hz");
        assert_eq!(khz_to_ghz(3_500_000), 3.5);
        assert_eq!(ghz_to_khz(3.5), 3_500_000);
        assert_eq!(strings(&json!(["a", "", 3, "b"])), ["a", "b"]);
    }

    #[test]
    fn history_is_bounded_and_skips_missing_values() {
        let mut history = History::default();
        history.push(None);
        for value in 0..HISTORY_LEN + 5 {
            history.push(Some(value as f64));
        }
        assert_eq!(history.values().len(), HISTORY_LEN);
        assert_eq!(history.values().first(), Some(&5.));
    }

    #[test]
    fn fan_modes_follow_the_contract() {
        let mut profile = Profile::new("custom-a", "A");
        profile.fan.fan_profile = "Quiet".to_owned();
        assert_eq!(fan_mode(&profile), "quiet");
        profile.fan.use_control = false;
        assert_eq!(fan_mode(&profile), "auto");
        assert_eq!(canonical_fan_profile("custom"), "Custom");
        assert_eq!(canonical_fan_profile("auto"), "Balanced");
    }

    #[test]
    fn validates_a_profile_draft() {
        let mut profile = new_profile();
        profile.name = "Travail".to_owned();
        let none = BTreeSet::new();
        assert!(validate_profile(&profile, &none, 200.).is_empty());

        let taken: BTreeSet<String> = ["travail".to_owned()].into();
        let errors = validate_profile(&profile, &taken, 200.);
        assert_eq!(
            first_error(&errors, &["name"]),
            Some("editor.error.nameDuplicate")
        );

        profile.name = "  ".to_owned();
        profile.fan.minimum_fanspeed = 80.;
        profile.fan.maximum_fanspeed = 20.;
        profile.cpu.scaling_min_frequency = Some(4_000_000);
        profile.cpu.scaling_max_frequency = Some(tcc_core::FrequencyLimit::Hz(2_000_000));
        profile.odm_power_limits.tdp_values = vec![45., 35.];
        profile.nvidia_power_ctrl_profile.ctgp_offset = 201.;
        let errors = validate_profile(&profile, &none, 200.);
        assert_eq!(
            first_error(&errors, &["name"]),
            Some("editor.error.nameEmpty")
        );
        assert_eq!(
            first_error(&errors, &["fanMin"]),
            Some("editor.error.fanOrder")
        );
        assert_eq!(
            first_error(&errors, &["freqMax"]),
            Some("editor.error.freqOrder")
        );
        assert_eq!(
            first_error(&errors, &["tdpValues"]),
            Some("editor.error.tdpValues")
        );
        assert_eq!(
            first_error(&errors, &["ctgpOffset"]),
            Some("editor.error.ctgpOffset")
        );
    }

    #[test]
    fn curve_needs_two_increasing_bounded_points() {
        let point = FanTableEntry::new;
        assert_eq!(curve_error(&[point(30., 20.)]), Some("fanCurve.error.min"));
        assert_eq!(
            curve_error(&[point(50., 20.), point(40., 30.)]),
            Some("fanCurve.error.order")
        );
        assert_eq!(
            curve_error(&[point(50., 20.), point(60., 130.)]),
            Some("fanCurve.error.range")
        );
        assert_eq!(curve_error(&[point(30., 20.), point(80., 80.)]), None);
    }

    #[test]
    fn copy_names_are_unique() {
        let collection = ProfileCollection::new(vec![
            Profile::new("a", "Jeu"),
            Profile::new("b", "Jeu (copie)"),
        ]);
        assert_eq!(unique_copy_name(&collection, "Jeu"), "Jeu (copie 2)");
        assert_eq!(unique_copy_name(&collection, "Autre"), "Autre (copie)");
    }

    #[test]
    fn matrix_cells_roundtrip() {
        let mut settings = Settings::default();
        assert_eq!(matrix_cell(&settings, "balanced/ac"), None);
        set_matrix_cell(&mut settings, "balanced/ac", Some("custom-a"));
        assert_eq!(
            matrix_cell(&settings, "balanced/ac").as_deref(),
            Some("custom-a")
        );
        assert!(assigned_to(&settings, "custom-a", "ac"));
        assert!(!assigned_to(&settings, "custom-a", "battery"));
        set_matrix_cell(&mut settings, "balanced/ac", None);
        assert_eq!(matrix_cell(&settings, "balanced/ac"), None);
    }

    #[test]
    fn export_then_import_adds_renamed_copies() {
        let mut collection = ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Jeu"),
        ]);
        let (json, count) = build_export(&collection).unwrap();
        assert_eq!(count, 1);
        let imported = parse_import(&json).unwrap();
        let summary = merge_import(&mut collection, imported).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                imported: 1,
                renamed: 1,
                skipped: 0
            }
        );
        assert_eq!(collection.len(), 3);
        assert!(collection.iter().any(|p| p.name == "Jeu (importé)"));
    }

    #[test]
    fn import_rejects_garbage_and_skips_presets() {
        assert!(parse_import("42").is_err());
        assert!(parse_import("{\"exportVersion\": 9, \"profiles\": []}").is_err());
        assert!(parse_import("{\"exportVersion\": 1, \"profiles\": [], \"x\": 1}").is_err());
        let presets = ProfileCollection::new(vec![Profile::new("__office__", "Office")]);
        assert_eq!(
            build_export(&presets).unwrap_err(),
            TransferError("admin.error.nothingToExport")
        );
        let mut collection = ProfileCollection::new(Vec::new());
        let bare = serde_json::to_string(&presets).unwrap();
        let summary = merge_import(&mut collection, parse_import(&bare).unwrap()).unwrap();
        assert_eq!(summary.skipped, 1);
        assert!(collection.is_empty());
    }

    #[test]
    fn display_modes_group_by_resolution() {
        let display = json!({"internalOutput": {"modes": [
            {"width": 2560, "height": 1600, "refreshHz": 240.0, "current": true},
            {"width": 2560, "height": 1600, "refreshHz": 60.0, "current": false},
            {"width": 1920, "height": 1200, "refreshHz": 59.95, "current": false},
            {"width": 1920, "height": 1200, "refreshHz": 0.0}
        ]}});
        let modes = modes(&display);
        assert_eq!(modes.len(), 3);
        assert_eq!(resolutions(&modes), [(2560, 1600), (1920, 1200)]);
        assert_eq!(rates(&modes, (2560, 1600)), [240., 60.]);
        assert_eq!(matching_rate(&[240., 60.], 59.9), Some(60.));
        assert_eq!(matching_rate(&[240., 60.], 120.), None);
        assert_eq!(current_mode(&modes).map(|m| m.hz), Some(240.));
    }

    #[test]
    fn parses_shutdown_time() {
        assert_eq!(parse_shutdown_time("22:30"), Some((22, 30)));
        assert_eq!(parse_shutdown_time("24:00"), None);
        assert_eq!(parse_shutdown_time("7:5"), None);
    }
}
