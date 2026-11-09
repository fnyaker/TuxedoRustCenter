// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Contrat d'affichage : état du mode interne et demandes d'application.
//!
//! L'invariant central est qu'une demande ne peut cibler que l'écran interne.
//! [`is_internal_output_name`] filtre les connecteurs `eDP-*`, `LVDS-*` et
//! `DSI-*` ; [`DisplayModeRequest::validate`] refuse toute autre sortie. Le
//! daemon et l'agent de session doivent tous deux appliquer cette validation :
//! le daemon pour ne pas émettre une demande dangereuse, l'agent pour ne pas
//! faire confiance à un message D-Bus système.

use std::fmt;

use serde::{Deserialize, Serialize};
use tcc_core::{
    MAX_REFRESH_RATE_HZ, MAX_RESOLUTION, MIN_REFRESH_RATE_HZ, ProfileId, validate_profile_id,
};

use crate::error::{IpcError, IpcResult};

/// Nombre maximal de modes listés dans un état d'affichage.
pub const MAX_DISPLAY_MODES: usize = 128;
/// Longueur maximale d'un message d'erreur d'affichage.
pub const MAX_DISPLAY_ERROR_LEN: usize = 1024;

/// Préfixes de connecteurs d'écran interne reconnus.
pub const INTERNAL_OUTPUT_PREFIXES: [&str; 3] = ["eDP-", "LVDS-", "DSI-"];

/// Vrai si le nom désigne un connecteur d'écran interne.
///
/// Règles :
/// - préfixe `eDP-`, `LVDS-` ou `DSI-` (casse du noyau) ;
/// - suffixe non vide, composé de lettres/chiffres ASCII et de `-` seulement ;
/// - commence par un caractère alphanumérique ;
/// - longueur totale bornée.
///
/// `HDMI-1`, `DP-1`, `eDP-1/../../etc`, `eDP-`, une chaîne vide ou un nom
/// contenant un NUL sont refusés.
#[must_use]
pub fn is_internal_output_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 32 || name.trim() != name {
        return false;
    }
    let Some(prefix) = INTERNAL_OUTPUT_PREFIXES
        .iter()
        .find(|prefix| name.starts_with(**prefix))
    else {
        return false;
    };
    let suffix = &name[prefix.len()..];
    suffix
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && suffix
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// Backend d'application du mode d'affichage.
///
/// Reprend les valeurs de `tcc_hw::display::DisplayBackendKind` sans en
/// dépendre : la conversion est faite par `tcc-user`/`tcc-daemon`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayBackend {
    /// X11 via XRandR.
    X11,
    /// Hyprland via son socket IPC.
    Hyprland,
    /// GNOME Mutter via `org.gnome.Mutter.DisplayConfig`.
    Gnome,
    /// KDE via KScreen.
    Kde,
    /// Sway via son IPC.
    Sway,
    /// wlroots générique via `zwlr_output_manager_v1`.
    Wlroots,
    /// Aucun backend disponible.
    #[default]
    Unsupported,
}

impl DisplayBackend {
    /// Nom stable pour les logs et l'UI.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Hyprland => "hyprland",
            Self::Gnome => "gnome",
            Self::Kde => "kde",
            Self::Sway => "sway",
            Self::Wlroots => "wlroots",
            Self::Unsupported => "unsupported",
        }
    }

    /// Vrai si un backend peut appliquer un mode.
    #[must_use]
    pub fn is_supported(self) -> bool {
        self != Self::Unsupported
    }
}

impl fmt::Display for DisplayBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Résolution d'un mode d'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayResolution {
    /// Largeur en pixels.
    pub width: u32,
    /// Hauteur en pixels.
    pub height: u32,
}

impl DisplayResolution {
    /// Crée une résolution.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Valide les bornes.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si une dimension vaut zéro ou dépasse
    /// [`MAX_RESOLUTION`].
    pub fn validate(&self) -> IpcResult<()> {
        if !(1..=MAX_RESOLUTION).contains(&self.width) {
            return Err(IpcError::field(
                "resolution.width",
                format!("doit être comprise entre 1 et {MAX_RESOLUTION}"),
            ));
        }
        if !(1..=MAX_RESOLUTION).contains(&self.height) {
            return Err(IpcError::field(
                "resolution.height",
                format!("doit être comprise entre 1 et {MAX_RESOLUTION}"),
            ));
        }
        Ok(())
    }
}

impl fmt::Display for DisplayResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.width, self.height)
    }
}

/// État du mode d'affichage de l'écran interne.
///
/// `supported == false` est un état normal, pas une erreur silencieuse :
/// l'UI doit afficher « non supporté » avec `error` comme explication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayModeInfo {
    /// Vrai si un backend est disponible et qu'un mode peut être appliqué.
    pub supported: bool,
    /// Backend détecté.
    pub backend: DisplayBackend,
    /// Connecteur interne détecté (`eDP-1`, `LVDS-1`, …).
    pub internal_output: Option<String>,
    /// Résolution actuellement active.
    pub current_resolution: Option<DisplayResolution>,
    /// Taux de rafraîchissement actuellement actif, en Hz.
    pub current_refresh_rate_hz: Option<f32>,
    /// Résolutions proposées sur le connecteur interne.
    pub available_resolutions: Vec<DisplayResolution>,
    /// Taux de rafraîchissement proposés, en Hz.
    pub available_refresh_rates_hz: Vec<f32>,
    /// Explication si `supported` est faux ou si une application a échoué.
    pub error: Option<String>,
}

impl Default for DisplayModeInfo {
    fn default() -> Self {
        Self::unsupported(None::<String>)
    }
}

impl DisplayModeInfo {
    /// État « non supporté », avec explication facultative.
    #[must_use]
    pub fn unsupported(reason: Option<impl Into<String>>) -> Self {
        Self {
            supported: false,
            backend: DisplayBackend::Unsupported,
            internal_output: None,
            current_resolution: None,
            current_refresh_rate_hz: None,
            available_resolutions: Vec::new(),
            available_refresh_rates_hz: Vec::new(),
            error: reason.map(Into::into),
        }
    }

    /// État prêt pour un backend et un connecteur interne.
    #[must_use]
    pub fn ready(backend: DisplayBackend, internal_output: impl Into<String>) -> Self {
        Self {
            supported: backend.is_supported(),
            backend,
            internal_output: Some(internal_output.into()),
            current_resolution: None,
            current_refresh_rate_hz: None,
            available_resolutions: Vec::new(),
            available_refresh_rates_hz: Vec::new(),
            error: None,
        }
    }

    /// Vrai si un mode peut être appliqué.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.supported && self.backend.is_supported() && self.error.is_none()
    }

    /// Valide l'état avant émission sur D-Bus.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si le connecteur n'est pas interne, si un taux ou une
    /// résolution est hors bornes, ou si le message d'erreur est trop long.
    pub fn validate(&self) -> IpcResult<()> {
        if let Some(output) = &self.internal_output
            && !is_internal_output_name(output)
        {
            return Err(IpcError::NonInternalOutput(output.clone()));
        }
        if self.supported && !self.backend.is_supported() {
            return Err(IpcError::field(
                "display.backend",
                "supported=true avec backend Unsupported",
            ));
        }
        if let Some(rate) = self.current_refresh_rate_hz {
            validate_refresh_rate("display.currentRefreshRateHz", rate)?;
        }
        if self.available_refresh_rates_hz.len() > MAX_DISPLAY_MODES {
            return Err(IpcError::field(
                "display.availableRefreshRatesHz",
                format!("ne doit pas dépasser {MAX_DISPLAY_MODES} entrées"),
            ));
        }
        for (index, rate) in self.available_refresh_rates_hz.iter().enumerate() {
            validate_refresh_rate(&format!("display.availableRefreshRatesHz[{index}]"), *rate)?;
        }
        if self.available_resolutions.len() > MAX_DISPLAY_MODES {
            return Err(IpcError::field(
                "display.availableResolutions",
                format!("ne doit pas dépasser {MAX_DISPLAY_MODES} entrées"),
            ));
        }
        for (index, resolution) in self.available_resolutions.iter().enumerate() {
            resolution.validate().map_err(|error| {
                prefix_field(error, &format!("display.availableResolutions[{index}]"))
            })?;
        }
        if let Some(resolution) = &self.current_resolution {
            resolution
                .validate()
                .map_err(|error| prefix_field(error, "display.currentResolution"))?;
        }
        if let Some(error) = &self.error
            && error.chars().count() > MAX_DISPLAY_ERROR_LEN
        {
            return Err(IpcError::field(
                "display.error",
                format!("ne doit pas dépasser {MAX_DISPLAY_ERROR_LEN} caractères"),
            ));
        }
        Ok(())
    }
}

/// Demande d'application d'un mode sur l'écran interne.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayModeRequest {
    /// Profil à l'origine de la demande (traçabilité et anti-rejeu).
    pub profile_id: ProfileId,
    /// Taux de rafraîchissement cible, si le profil le demande.
    pub refresh_rate_hz: Option<f32>,
    /// Résolution cible, si le profil demande explicitement un changement.
    pub resolution: Option<DisplayResolution>,
    /// Connecteur interne ciblé, ou `None` pour laisser l'agent le détecter.
    pub internal_output: Option<String>,
}

impl DisplayModeRequest {
    /// Crée une demande vide pour un profil.
    #[must_use]
    pub fn new(profile_id: impl Into<ProfileId>) -> Self {
        Self {
            profile_id: profile_id.into(),
            refresh_rate_hz: None,
            resolution: None,
            internal_output: None,
        }
    }

    /// Renseigne le taux de rafraîchissement cible.
    #[must_use]
    pub fn with_refresh_rate(mut self, refresh_rate_hz: f32) -> Self {
        self.refresh_rate_hz = Some(refresh_rate_hz);
        self
    }

    /// Renseigne la résolution cible.
    #[must_use]
    pub fn with_resolution(mut self, resolution: DisplayResolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// Renseigne le connecteur interne cible.
    #[must_use]
    pub fn with_internal_output(mut self, output: impl Into<String>) -> Self {
        self.internal_output = Some(output.into());
        self
    }

    /// Valide la demande.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si aucun taux ni résolution n'est demandé, si une
    /// valeur est hors bornes, si le profil est invalide ou si la sortie
    /// n'est pas un connecteur interne.
    pub fn validate(&self) -> IpcResult<()> {
        validate_profile_id(&self.profile_id)
            .map_err(|error| IpcError::field("profileId", error.to_string()))?;

        if self.refresh_rate_hz.is_none() && self.resolution.is_none() {
            return Err(IpcError::field(
                "displayMode",
                "au moins refreshRateHz ou resolution doit être renseigné",
            ));
        }
        if let Some(rate) = self.refresh_rate_hz {
            validate_refresh_rate("refreshRateHz", rate)?;
        }
        if let Some(resolution) = self.resolution {
            resolution.validate()?;
        }
        if let Some(output) = &self.internal_output
            && !is_internal_output_name(output)
        {
            return Err(IpcError::NonInternalOutput(output.clone()));
        }
        Ok(())
    }

    /// Vrai si la demande ne touche que le taux (pas la résolution).
    #[must_use]
    pub fn refresh_rate_only(&self) -> bool {
        self.refresh_rate_hz.is_some() && self.resolution.is_none()
    }
}

fn validate_refresh_rate(field: &str, rate: f32) -> IpcResult<()> {
    if !rate.is_finite() {
        return Err(IpcError::field(field, "doit être une valeur finie"));
    }
    if !(MIN_REFRESH_RATE_HZ..=MAX_REFRESH_RATE_HZ).contains(&rate) {
        return Err(IpcError::field(
            field,
            format!("doit être compris entre {MIN_REFRESH_RATE_HZ} et {MAX_REFRESH_RATE_HZ} Hz"),
        ));
    }
    Ok(())
}

fn prefix_field(error: IpcError, prefix: &str) -> IpcError {
    match error {
        IpcError::InvalidField { field, message } => IpcError::InvalidField {
            field: format!("{prefix}.{field}"),
            message,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DisplayBackend, DisplayModeInfo, DisplayModeRequest, DisplayResolution,
        is_internal_output_name,
    };
    use crate::IpcError;
    use tcc_core::ProfileId;

    #[test]
    fn internal_output_names_are_strictly_filtered() {
        for valid in ["eDP-1", "eDP-2", "LVDS-1", "DSI-1", "eDP-1-1"] {
            assert!(is_internal_output_name(valid), "refusé à tort: {valid}");
        }
        for invalid in [
            "",
            "HDMI-1",
            "DP-1",
            "eDP-",
            "eDP-1/../../x",
            "../../etc/passwd",
            "eDP-1 ",
            " eDP-1",
            "eDP-\0",
            "eDP-é",
            "edp-1",
        ] {
            assert!(
                !is_internal_output_name(invalid),
                "accepté à tort: {invalid:?}"
            );
        }
    }

    #[test]
    fn display_info_roundtrips() {
        let info = DisplayModeInfo {
            supported: true,
            backend: DisplayBackend::Hyprland,
            internal_output: Some("eDP-1".into()),
            current_resolution: Some(DisplayResolution::new(2560, 1600)),
            current_refresh_rate_hz: Some(165.0),
            available_resolutions: vec![DisplayResolution::new(2560, 1600)],
            available_refresh_rates_hz: vec![60.0, 165.0],
            error: None,
        };
        info.validate().unwrap();
        let json = serde_json::to_string(&info).unwrap();
        let parsed: DisplayModeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, info);
        assert_eq!(parsed.backend, DisplayBackend::Hyprland);
    }

    #[test]
    fn display_info_rejects_external_output() {
        let info = DisplayModeInfo {
            internal_output: Some("HDMI-1".into()),
            ..DisplayModeInfo::unsupported(None::<String>)
        };
        assert!(matches!(
            info.validate(),
            Err(IpcError::NonInternalOutput(_))
        ));
    }

    #[test]
    fn request_requires_a_target() {
        let request = DisplayModeRequest::new("office");
        assert!(request.validate().is_err());

        request.validate().unwrap_err();
        let request = request.with_refresh_rate(165.0);
        request.validate().unwrap();
        assert!(request.refresh_rate_only());
    }

    #[test]
    fn request_rejects_non_finite_and_out_of_range_rates() {
        for rate in [f32::NAN, f32::INFINITY, 0.0, 2000.0] {
            let request = DisplayModeRequest::new("office")
                .with_refresh_rate(rate)
                .with_internal_output("eDP-1");
            assert!(request.validate().is_err(), "taux accepté: {rate}");
        }
        DisplayModeRequest::new("office")
            .with_refresh_rate(59.94)
            .with_resolution(DisplayResolution::new(1920, 1080))
            .with_internal_output("eDP-1")
            .validate()
            .unwrap();
    }

    #[test]
    fn request_rejects_external_output() {
        let request = DisplayModeRequest::new("office")
            .with_refresh_rate(60.0)
            .with_internal_output("DP-1");
        assert!(matches!(
            request.validate(),
            Err(IpcError::NonInternalOutput(_))
        ));
    }

    #[test]
    fn request_roundtrips() {
        let request = DisplayModeRequest::new("office")
            .with_refresh_rate(144.0)
            .with_resolution(DisplayResolution::new(2560, 1600))
            .with_internal_output("eDP-1");
        let json = serde_json::to_string(&request).unwrap();
        let parsed: DisplayModeRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn backend_serde_is_stable() {
        assert_eq!(
            serde_json::to_string(&DisplayBackend::X11).unwrap(),
            "\"x11\""
        );
        assert_eq!(
            serde_json::to_string(&DisplayBackend::Unsupported).unwrap(),
            "\"unsupported\""
        );
        let parsed: DisplayBackend = serde_json::from_str("\"gnome\"").unwrap();
        assert_eq!(parsed, DisplayBackend::Gnome);
        assert!(!DisplayBackend::default().is_supported());
    }

    #[test]
    fn resolution_bounds_are_checked() {
        assert!(DisplayResolution::new(0, 1080).validate().is_err());
        assert!(DisplayResolution::new(1920, 0).validate().is_err());
        assert!(
            DisplayResolution::new(super::MAX_RESOLUTION + 1, 1080)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn default_info_is_unsupported() {
        let info = DisplayModeInfo::default();
        assert!(!info.supported);
        assert_eq!(info.backend, DisplayBackend::Unsupported);
        assert!(!info.is_ready());
        info.validate().unwrap();
        assert_eq!(ProfileId::default().as_str(), "");
    }
}
