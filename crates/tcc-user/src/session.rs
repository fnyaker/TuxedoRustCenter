// SPDX-License-Identifier: GPL-3.0-or-later
//! Détection de session graphique.
//!
//! Sources : variables d'environnement de l'agent (`XDG_SESSION_TYPE`,
//! `XDG_CURRENT_DESKTOP`, `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_RUNTIME_DIR`,
//! `HYPRLAND_INSTANCE_SIGNATURE`, `SWAYSOCK`). Aucune lecture de
//! `/proc/*/environ` (D6) : l'agent ne voit que son propre environnement, que
//! systemd lui transmet.

use tcc_hw::display::DisplayBackendKind;

/// Type de session annoncé par logind/systemd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionType {
    X11,
    Wayland,
    Tty,
    Unknown(String),
}

impl SessionType {
    /// Convertit `XDG_SESSION_TYPE`.
    pub fn from_raw(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("x11") | Some("X11") => Self::X11,
            Some("wayland") | Some("Wayland") => Self::Wayland,
            Some("tty") | Some("TTY") => Self::Tty,
            Some(other) if !other.is_empty() => Self::Unknown(other.to_ascii_lowercase()),
            _ => Self::Unknown(String::new()),
        }
    }

    /// Valeur stable pour les logs/JSON.
    pub fn as_str(&self) -> &str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Tty => "tty",
            Self::Unknown(value) if value.is_empty() => "unknown",
            Self::Unknown(value) => value,
        }
    }
}

/// Environnement de session utile à la détection.
///
/// Construit depuis l'environnement réel par [`SessionEnv::from_env`], ou
/// directement dans les tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEnv {
    pub xdg_session_type: Option<String>,
    pub xdg_current_desktop: Option<String>,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
    pub xdg_runtime_dir: Option<String>,
    pub hyprland_instance_signature: Option<String>,
    pub sway_sock: Option<String>,
}

impl SessionEnv {
    /// Lit l'environnement du processus.
    pub fn from_env() -> Self {
        Self {
            xdg_session_type: env_string("XDG_SESSION_TYPE"),
            xdg_current_desktop: env_string("XDG_CURRENT_DESKTOP"),
            wayland_display: env_string("WAYLAND_DISPLAY"),
            display: env_string("DISPLAY"),
            xdg_runtime_dir: env_string("XDG_RUNTIME_DIR"),
            hyprland_instance_signature: env_string("HYPRLAND_INSTANCE_SIGNATURE"),
            sway_sock: env_string("SWAYSOCK"),
        }
    }

    /// Vrai si `XDG_CURRENT_DESKTOP` contient l'un des fragments donnés
    /// (`:`-séparé, insensible à la casse).
    pub fn desktop_contains(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.xdg_current_desktop
            .as_deref()
            .unwrap_or_default()
            .split(':')
            .map(|token| token.trim().to_ascii_lowercase())
            .any(|token| token.contains(&needle))
    }
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Session détectée et backend pressenti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub env: SessionEnv,
    pub session_type: SessionType,
    pub backend: DisplayBackendKind,
}

impl SessionInfo {
    /// Détecte la session depuis l'environnement réel.
    pub fn detect() -> Self {
        Self::from_env(SessionEnv::from_env())
    }

    /// Détection pure, utilisée par les tests.
    pub fn from_env(env: SessionEnv) -> Self {
        let session_type = SessionType::from_raw(env.xdg_session_type.as_deref());
        let backend = detect_backend(&env, &session_type);
        Self {
            env,
            session_type,
            backend,
        }
    }

    /// Vrai si un backend **implémenté dans cet agent** est disponible.
    ///
    /// Wayland générique reste un stub. GNOME n'est supporté que sous
    /// Wayland (vérifié aussi par la factory).
    pub fn has_supported_backend(&self) -> bool {
        matches!(
            self.backend,
            DisplayBackendKind::X11
                | DisplayBackendKind::Hyprland
                | DisplayBackendKind::Sway
                | DisplayBackendKind::Kde
                | DisplayBackendKind::Gnome
        )
    }
}

fn detect_backend(env: &SessionEnv, session_type: &SessionType) -> DisplayBackendKind {
    match session_type {
        SessionType::Tty => DisplayBackendKind::Unsupported,
        SessionType::X11 => DisplayBackendKind::X11,
        SessionType::Wayland => detect_wayland_backend(env),
        SessionType::Unknown(_) => {
            if env.wayland_display.is_some() {
                detect_wayland_backend(env)
            } else if env.display.is_some() {
                DisplayBackendKind::X11
            } else {
                DisplayBackendKind::Unsupported
            }
        }
    }
}

fn detect_wayland_backend(env: &SessionEnv) -> DisplayBackendKind {
    if env.desktop_contains("hyprland") || env.hyprland_instance_signature.is_some() {
        DisplayBackendKind::Hyprland
    } else if env.desktop_contains("sway") {
        DisplayBackendKind::Sway
    } else if env.desktop_contains("kde") || env.desktop_contains("plasma") {
        DisplayBackendKind::Kde
    } else if env.desktop_contains("gnome") {
        DisplayBackendKind::Gnome
    } else {
        DisplayBackendKind::Wlroots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(session_type: &str, desktop: &str) -> SessionEnv {
        SessionEnv {
            xdg_session_type: Some(session_type.to_owned()),
            xdg_current_desktop: Some(desktop.to_owned()),
            wayland_display: (session_type == "wayland").then(|| "wayland-1".to_owned()),
            display: (session_type == "x11").then(|| ":0".to_owned()),
            ..SessionEnv::default()
        }
    }

    #[test]
    fn x11_session() {
        let info = SessionInfo::from_env(env("x11", "GNOME"));
        assert_eq!(info.session_type, SessionType::X11);
        assert_eq!(info.backend, DisplayBackendKind::X11);
        assert!(info.has_supported_backend());
    }

    #[test]
    fn hyprland_session() {
        let mut e = env("wayland", "Hyprland");
        e.hyprland_instance_signature = Some("abc123".to_owned());
        let info = SessionInfo::from_env(e);
        assert_eq!(info.session_type, SessionType::Wayland);
        assert_eq!(info.backend, DisplayBackendKind::Hyprland);
    }

    #[test]
    fn hyprland_detected_by_signature_even_without_desktop() {
        let mut e = env("wayland", "");
        e.hyprland_instance_signature = Some("abc123".to_owned());
        assert_eq!(
            SessionInfo::from_env(e).backend,
            DisplayBackendKind::Hyprland
        );
    }

    #[test]
    fn sway_session() {
        let info = SessionInfo::from_env(env("wayland", "sway"));
        assert_eq!(info.backend, DisplayBackendKind::Sway);
    }

    #[test]
    fn gnome_and_kde_sessions_are_supported() {
        assert_eq!(
            SessionInfo::from_env(env("wayland", "GNOME")).backend,
            DisplayBackendKind::Gnome
        );
        assert_eq!(
            SessionInfo::from_env(env("wayland", "KDE")).backend,
            DisplayBackendKind::Kde
        );
        assert_eq!(
            SessionInfo::from_env(env("wayland", "Plasma")).backend,
            DisplayBackendKind::Kde
        );
        assert!(SessionInfo::from_env(env("wayland", "GNOME")).has_supported_backend());
        assert!(SessionInfo::from_env(env("wayland", "KDE")).has_supported_backend());
        // KDE est testé avant GNOME dans l'ordre de détection.
        let both = SessionEnv {
            xdg_session_type: Some("wayland".to_owned()),
            xdg_current_desktop: Some("GNOME:KDE".to_owned()),
            wayland_display: Some("wayland-1".to_owned()),
            ..SessionEnv::default()
        };
        assert_eq!(SessionInfo::from_env(both).backend, DisplayBackendKind::Kde);
    }

    #[test]
    fn unknown_wayland_is_wlroots() {
        let info = SessionInfo::from_env(env("wayland", "some-compositor"));
        assert_eq!(info.backend, DisplayBackendKind::Wlroots);
        assert!(!info.has_supported_backend());
    }

    #[test]
    fn tty_is_unsupported() {
        let info = SessionInfo::from_env(env("tty", ""));
        assert_eq!(info.session_type, SessionType::Tty);
        assert_eq!(info.backend, DisplayBackendKind::Unsupported);
    }

    #[test]
    fn missing_session_type_is_inferred() {
        let e = SessionEnv {
            wayland_display: Some("wayland-0".to_owned()),
            hyprland_instance_signature: Some("sig".to_owned()),
            ..SessionEnv::default()
        };
        assert_eq!(
            SessionInfo::from_env(e).session_type,
            SessionType::Unknown(String::new())
        );
        assert_eq!(
            SessionInfo::from_env(SessionEnv {
                wayland_display: Some("wayland-0".to_owned()),
                hyprland_instance_signature: Some("sig".to_owned()),
                ..SessionEnv::default()
            })
            .backend,
            DisplayBackendKind::Hyprland
        );

        let x11 = SessionEnv {
            display: Some(":0".to_owned()),
            ..SessionEnv::default()
        };
        assert_eq!(SessionInfo::from_env(x11).backend, DisplayBackendKind::X11);
    }

    #[test]
    fn empty_environment_is_unsupported() {
        let info = SessionInfo::from_env(SessionEnv::default());
        assert_eq!(info.backend, DisplayBackendKind::Unsupported);
    }

    #[test]
    fn desktop_tokens_are_split_on_colon() {
        let e = SessionEnv {
            xdg_current_desktop: Some("KDE:Plasma".to_owned()),
            ..SessionEnv::default()
        };
        assert!(e.desktop_contains("kde"));
        assert!(e.desktop_contains("plasma"));
        assert!(!e.desktop_contains("gnome"));
    }
}
