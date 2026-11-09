// SPDX-License-Identifier: GPL-3.0-or-later
//
// Vue typée du rapport `GetDisplayInfoJSON` de `tccd-user` (interface `User1`,
// bus de session). Ce n'est pas le contrat D-Bus : le type canonique vit dans
// `tcc-user`. La GUI ne garde qu'un sous-ensemble d'affichage, tolérant aux
// champs absents et strict sur les types. Les modes exposés sont ceux de
// `internalOutput` uniquement ; aucun écran externe n'est accepté.
//
// Aucune écriture : ce rapport ne sert qu'à proposer des modes réellement
// applicables.

#![allow(dead_code)] // sous-ensemble d'affichage

use serde::{Deserialize, Serialize};

/// Rapport d'affichage de la session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplayInfoView {
    pub available: bool,
    pub session: SessionView,
    pub backend: BackendView,
    pub internal_output: InternalOutputView,
    /// Un écran interne est présent et un mode peut être appliqué.
    pub supported: bool,
    pub dry_run: bool,
    pub error: Option<String>,
}

/// Contexte de session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionView {
    #[serde(rename = "type")]
    pub session_type: String,
    pub desktop: Option<String>,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
}

/// Backend d'affichage effectif.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackendView {
    pub kind: String,
    pub name: String,
    pub available: bool,
}

/// Écran interne et ses modes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InternalOutputView {
    pub name: String,
    pub connected: bool,
    pub enabled: bool,
    pub has_internal_display: bool,
    pub current_mode: Option<ModeView>,
    pub modes: Vec<ModeView>,
}

/// Un mode d'affichage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModeView {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    pub preferred: bool,
    pub current: bool,
}

impl ModeView {
    /// Dimensions/taux dans les bornes d'un écran réel.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        (1..=16_384).contains(&self.width)
            && (1..=16_384).contains(&self.height)
            && self.refresh_hz.is_finite()
            && (1.0..=1000.0).contains(&self.refresh_hz)
    }
}

impl DisplayInfoView {
    /// Écarte les modes invalides et un mode courant incohérent. QML ne voit
    /// donc que des modes réellement applicables.
    pub fn sanitize(&mut self) {
        self.internal_output.modes.retain(ModeView::is_valid);
        if self
            .internal_output
            .current_mode
            .as_ref()
            .is_some_and(|mode| !mode.is_valid())
        {
            self.internal_output.current_mode = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayInfoView;

    const FULL: &str = r#"{
        "available": true,
        "session": { "type": "wayland", "desktop": "Hyprland",
                     "waylandDisplay": "wayland-1", "display": null },
        "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
        "internalOutput": {
            "name": "eDP-1",
            "connected": true,
            "enabled": true,
            "hasInternalDisplay": true,
            "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60.0,
                             "preferred": false, "current": true },
            "modes": [
                { "width": 2560, "height": 1600, "refreshHz": 60.0,
                  "preferred": false, "current": true },
                { "width": 2560, "height": 1600, "refreshHz": 165.0,
                  "preferred": true, "current": false }
            ]
        },
        "supported": true,
        "dryRun": false,
        "error": null,
        "externalOutput": { "name": "DP-1", "modes": [] },
        "unknownTopLevel": true
    }"#;

    #[test]
    fn report_is_parsed_and_external_outputs_are_dropped() {
        let view: DisplayInfoView = serde_json::from_str(FULL).expect("rapport");
        assert!(view.available);
        assert!(view.supported);
        assert!(!view.dry_run);
        assert_eq!(view.session.session_type, "wayland");
        assert_eq!(view.session.desktop.as_deref(), Some("Hyprland"));
        assert_eq!(view.backend.kind, "hyprland");
        assert!(view.backend.available);
        assert_eq!(view.internal_output.name, "eDP-1");
        assert!(view.internal_output.has_internal_display);
        assert_eq!(view.internal_output.modes.len(), 2);
        assert_eq!(view.internal_output.modes[1].refresh_hz, 165.0);
        assert!(view.internal_output.modes[1].preferred);
        assert_eq!(
            view.internal_output
                .current_mode
                .as_ref()
                .map(|mode| mode.refresh_hz),
            Some(60.0)
        );

        let canonical = serde_json::to_string(&view).expect("canonical");

        assert!(!canonical.contains("externalOutput"));
        assert!(!canonical.contains("unknownTopLevel"));
        // Le nom externe ne peut pas se retrouver dans les modes internes
        // (`eDP-1` contient la sous-chaîne « DP-1 », on cherche le nom exact).
        assert!(!canonical.contains("\"DP-1\""));
    }

    #[test]
    fn missing_report_sections_default_to_empty() {
        let view: DisplayInfoView = serde_json::from_str("{}").expect("vide");
        assert!(!view.available);
        assert!(!view.supported);
        assert!(view.internal_output.name.is_empty());
        assert!(view.internal_output.modes.is_empty());
        assert!(view.internal_output.current_mode.is_none());
        assert_eq!(view.error, None);
    }

    #[test]
    fn null_optionals_and_empty_modes_are_tolerated() {
        let view: DisplayInfoView = serde_json::from_str(
            r#"{
                "session": { "desktop": null, "waylandDisplay": null, "display": null },
                "internalOutput": { "name": "eDP-1", "currentMode": null, "modes": [] },
                "error": null
            }"#,
        )
        .expect("tolérant");
        assert_eq!(view.session.desktop, None);
        assert!(view.internal_output.modes.is_empty());
        assert!(view.internal_output.current_mode.is_none());
    }

    #[test]
    fn invalid_modes_are_sanitized_in_rust() {
        let mut view: DisplayInfoView = serde_json::from_str(FULL).expect("rapport");
        view.internal_output.modes.push(super::ModeView {
            width: 0,
            height: 1600,
            refresh_hz: 60.0,
            ..super::ModeView::default()
        });
        view.internal_output.modes.push(super::ModeView {
            width: 2560,
            height: 1600,
            refresh_hz: 0.0,
            ..super::ModeView::default()
        });
        view.internal_output.current_mode = Some(super::ModeView {
            width: 2560,
            height: 1600,
            refresh_hz: 2000.0,
            ..super::ModeView::default()
        });
        view.sanitize();
        assert_eq!(view.internal_output.modes.len(), 2);
        assert!(view.internal_output.current_mode.is_none());
    }

    #[test]
    fn wrong_types_are_rejected() {
        assert!(
            serde_json::from_str::<DisplayInfoView>(r#"{ "internalOutput": { "modes": 3 } }"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<DisplayInfoView>(r#"{ "internalOutput": { "modes": "x" } }"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<DisplayInfoView>(
                r#"{ "internalOutput": { "modes": [{ "width": -1 }] } }"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<DisplayInfoView>(
                r#"{ "internalOutput": { "modes": [{ "refreshHz": "60" }] } }"#
            )
            .is_err()
        );
    }
}
