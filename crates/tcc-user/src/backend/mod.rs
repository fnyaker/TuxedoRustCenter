// SPDX-License-Identifier: GPL-3.0-or-later
//! Backends d'affichage de `tccd-user`.
//!
//! Chaque backend implémente [`tcc_hw::display::DisplayModeBackend`]. La
//! sélection de l'écran interne et le matching des taux sont communs
//! (`tcc-hw`) ; les backends ne font que parler au compositeur/X11.
//!
//! | Session | Backend | État |
//! |---|---|---|
//! | X11 | `x11.rs` (x11rb / XRandR) | implémenté |
//! | Hyprland | `hyprland.rs` (socket IPC) | implémenté |
//! | Sway | `sway.rs` (i3-ipc) | implémenté |
//! | GNOME Wayland | `gnome.rs` (Mutter DisplayConfig) | implémenté |
//! | KDE | `kde.rs` (`kscreen-doctor -j`) | implémenté |
//! | Wayland générique | `stub.rs` (wlr-output-management) | `NotSupported` |

pub mod gnome;
pub mod hyprland;
pub mod kde;
pub mod stub;
pub mod sway;
pub mod x11;

use tcc_hw::display::{
    DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    needs_rate_change, select_internal_output,
};
use tcc_hw::error::{HwError, Result as HwResult};

use crate::error::Result;
use crate::session::SessionInfo;

/// Construit le backend correspondant à la session détectée.
pub fn create(session: &SessionInfo, uid: u32) -> Result<Box<dyn DisplayModeBackend>> {
    match session.backend {
        DisplayBackendKind::X11 => Ok(Box::new(x11::X11Backend::connect()?)),
        DisplayBackendKind::Hyprland => Ok(Box::new(hyprland::HyprlandBackend::new(session, uid)?)),
        DisplayBackendKind::Sway => Ok(Box::new(sway::SwayBackend::new(session, uid)?)),
        DisplayBackendKind::Gnome => {
            // Mutter DisplayConfig n'existe que sous Wayland ; une session
            // X11 avec GNOME doit passer par XRandR.
            if session.session_type == crate::session::SessionType::X11
                || session.env.wayland_display.is_none()
            {
                return Err(crate::error::UserError::UnsupportedSession(
                    "GNOME Mutter nécessite une session Wayland".to_owned(),
                ));
            }
            Ok(Box::new(gnome::GnomeBackend::new()?))
        }
        DisplayBackendKind::Kde => Ok(Box::new(kde::KdeBackend::new()?)),
        DisplayBackendKind::Wlroots => Ok(Box::new(stub::StubBackend::wayland_generic())),
        DisplayBackendKind::Unsupported => Ok(Box::new(stub::StubBackend::unsupported())),
    }
}

/// Décision de `--dry-run` / planification avant application.
#[derive(Debug, Clone, PartialEq)]
pub enum RefreshPlan {
    /// Le taux courant est déjà dans la tolérance.
    AlreadyApplied { output: String, current_hz: f32 },
    /// Un mode à la résolution courante sera appliqué.
    Apply { output: String, mode: DisplayMode },
    /// Aucun écran interne connecté.
    NoInternalOutput,
    /// Aucun mode à la résolution courante ne correspond au taux demandé.
    NotFound { output: String, target_hz: f32 },
}

/// Planifie une demande de taux sans toucher au matériel.
pub fn plan_refresh(outputs: &[OutputInfo], target_hz: f32, tolerance_hz: f32) -> RefreshPlan {
    let Some(internal) = internal_output(outputs) else {
        return RefreshPlan::NoInternalOutput;
    };
    if !needs_rate_change(internal.current_refresh_hz(), target_hz, tolerance_hz) {
        return RefreshPlan::AlreadyApplied {
            output: internal.name().to_owned(),
            current_hz: internal.current_refresh_hz().unwrap_or(target_hz),
        };
    }
    match internal.mode_for_rate(target_hz, tolerance_hz) {
        Some(mode) => RefreshPlan::Apply {
            output: internal.name().to_owned(),
            mode: mode.clone(),
        },
        None => RefreshPlan::NotFound {
            output: internal.name().to_owned(),
            target_hz,
        },
    }
}

/// Vérifie qu'un nom de connecteur est sûr pour une commande IPC.
///
/// Les connecteurs DRM sont de la forme `eDP-1`, `DP-2`, `HDMI-A-1`,
/// `DSI-1`. On refuse tout caractère qui pourrait altérer la grammaire d'une
/// commande compositeur (espace, virgule, point-virgule, saut de ligne, ...).
pub fn validate_output_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

/// Vérifie à la fois le format et le caractère interne du connecteur.
pub fn validate_internal_output_name(name: &str) -> bool {
    validate_output_name(name) && tcc_hw::display::is_internal_connector(name)
}

/// Formate un taux en Hz pour les commandes compositeur, sans zéros inutiles.
///
/// `165.0` → `"165"`, `59.94` → `"59.94"`, `59.994` → `"59.994"`.
pub fn format_hz(hz: f32) -> String {
    if !hz.is_finite() || hz <= 0.0 {
        return "0".to_owned();
    }
    let mut value = format!("{hz:.3}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

/// Extrait l'écran interne d'une liste, en validant le nom.
pub(crate) fn internal_output(outputs: &[OutputInfo]) -> Option<InternalOutput> {
    select_internal_output(outputs).filter(|output| validate_internal_output_name(output.name()))
}

/// Refuse explicitement un écran qui n'est pas interne.
pub(crate) fn ensure_internal(output: &InternalOutput) -> HwResult<()> {
    if !validate_internal_output_name(output.name()) {
        return Err(HwError::NotInternalOutput(output.name().to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, current: Option<f32>, rates: &[f32]) -> OutputInfo {
        let mut output = OutputInfo::new(name);
        output.connected = true;
        output.enabled = current.is_some();
        output.modes = rates
            .iter()
            .map(|rate| DisplayMode {
                width: 2560,
                height: 1440,
                refresh_hz: *rate,
                preferred: *rate == rates[0],
                current: false,
            })
            .collect();
        output.current_mode = current.map(|rate| DisplayMode {
            width: 2560,
            height: 1440,
            refresh_hz: rate,
            preferred: false,
            current: true,
        });
        output
    }

    #[test]
    fn output_name_validation() {
        assert!(validate_output_name("eDP-1"));
        assert!(validate_output_name("HDMI-A-1"));
        assert!(validate_output_name("DP-2"));
        assert!(!validate_output_name("eDP-1,scale=2"));
        assert!(!validate_output_name("eDP-1;reload"));
        assert!(!validate_output_name("eDP 1"));
        assert!(!validate_output_name("../eDP-1"));
        assert!(!validate_internal_output_name("HDMI-A-1"));
    }

    #[test]
    fn hz_formatting() {
        assert_eq!(format_hz(165.0), "165");
        assert_eq!(format_hz(59.94), "59.94");
        assert_eq!(format_hz(59.994), "59.994");
        assert_eq!(format_hz(60.0), "60");
        assert_eq!(format_hz(f32::NAN), "0");
    }

    #[test]
    fn plan_already_applied() {
        let outputs = vec![output("eDP-1", Some(165.0), &[60.0, 165.0])];
        assert!(matches!(
            plan_refresh(&outputs, 165.2, 0.5),
            RefreshPlan::AlreadyApplied { .. }
        ));
    }

    #[test]
    fn plan_applies_internal_only() {
        let outputs = vec![
            output("HDMI-A-1", Some(60.0), &[60.0]),
            output("eDP-1", Some(60.0), &[60.0, 165.0]),
        ];
        match plan_refresh(&outputs, 165.0, 0.5) {
            RefreshPlan::Apply { output, mode } => {
                assert_eq!(output, "eDP-1");
                assert_eq!(mode.refresh_hz, 165.0);
            }
            other => panic!("plan inattendu : {other:?}"),
        }
    }

    #[test]
    fn plan_external_only_is_no_internal() {
        let outputs = vec![output("DP-1", Some(60.0), &[60.0, 165.0])];
        assert_eq!(
            plan_refresh(&outputs, 165.0, 0.5),
            RefreshPlan::NoInternalOutput
        );
    }

    #[test]
    fn plan_missing_rate_is_not_found() {
        let outputs = vec![output("eDP-1", Some(60.0), &[60.0])];
        assert!(matches!(
            plan_refresh(&outputs, 165.0, 0.5),
            RefreshPlan::NotFound { .. }
        ));
    }
}
