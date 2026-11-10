// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Lanceur `tuxedo-rust-center` : choisit entre l'interface Qt 6 (officielle)
//! et l'interface gpui (style Omarchy), puis la remplace par `exec`.
//!
//! Ordre de décision : `--ui <mode>` > variable `TCC_UI` > fichier
//! `$XDG_CONFIG_HOME/tuxedo-rust-center/ui` > `auto`.
//!
//! `auto` choisit gpui sous Hyprland (si installée), Qt 6 sinon. Un mode
//! explicite dont le binaire est absent est une erreur ; `auto` retombe sur
//! l'autre interface.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Répertoire installé des interfaces (FHS).
const LIBEXEC_DIR: &str = "/usr/libexec/tuxedo-rust-center";
const QT_BIN: &str = "tcc-gui-qt";
const GPUI_BIN: &str = "tcc-gui-gpui";

/// Mode demandé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Auto,
    Qt6,
    Gpui,
}

/// Interface effective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ui {
    Qt6,
    Gpui,
}

impl Mode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "qt6" | "qt" => Some(Self::Qt6),
            "gpui" | "omarchy" => Some(Self::Gpui),
            _ => None,
        }
    }
}

/// Extrait `--ui <mode>` / `--ui=<mode>` ; le reste est transmis tel quel.
fn split_args(args: Vec<OsString>) -> Result<(Option<Mode>, Vec<OsString>), String> {
    let mut mode = None;
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy().into_owned();
        let value = if text == "--ui" {
            Some(
                iter.next()
                    .ok_or("--ui attend un mode : auto, qt6 ou gpui")?
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            text.strip_prefix("--ui=").map(str::to_owned)
        };
        match value {
            Some(raw) => {
                mode = Some(Mode::parse(&raw).ok_or_else(|| {
                    format!("mode d'interface inconnu « {raw} » (auto, qt6, gpui)")
                })?);
            }
            None => rest.push(arg),
        }
    }
    Ok((mode, rest))
}

/// Mode choisi par l'utilisateur hors ligne de commande.
fn configured_mode(env_ui: Option<&str>, file: Option<&str>) -> Mode {
    env_ui
        .and_then(Mode::parse)
        .or_else(|| file.and_then(Mode::parse))
        .unwrap_or(Mode::Auto)
}

/// Session Hyprland ?
fn is_hyprland(signature: Option<&str>, desktop: Option<&str>) -> bool {
    signature.is_some_and(|s| !s.is_empty())
        || desktop.is_some_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("hyprland")))
}

/// Choix final. `None` si aucune interface n'est installée.
fn resolve(mode: Mode, hyprland: bool, qt: bool, gpui: bool) -> Result<Option<Ui>, String> {
    Ok(match mode {
        Mode::Qt6 if qt => Some(Ui::Qt6),
        Mode::Qt6 => return Err("l'interface Qt 6 n'est pas installée".to_owned()),
        Mode::Gpui if gpui => Some(Ui::Gpui),
        Mode::Gpui => return Err("l'interface gpui n'est pas installée".to_owned()),
        Mode::Auto if hyprland && gpui => Some(Ui::Gpui),
        Mode::Auto if qt => Some(Ui::Qt6),
        Mode::Auto if gpui => Some(Ui::Gpui),
        Mode::Auto => None,
    })
}

fn binary_path(name: &str, _override_var: &str) -> PathBuf {
    #[cfg(feature = "dev-overrides")]
    if let Some(path) = std::env::var_os(_override_var).filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    Path::new(LIBEXEC_DIR).join(name)
}

fn config_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("tuxedo-rust-center").join("ui"))
}

fn main() -> ExitCode {
    let (cli_mode, rest) = match split_args(std::env::args_os().skip(1).collect()) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("tuxedo-rust-center: {message}");
            return ExitCode::from(2);
        }
    };
    let file = config_file().and_then(|path| std::fs::read_to_string(path).ok());
    let mode = cli_mode.unwrap_or_else(|| {
        configured_mode(std::env::var("TCC_UI").ok().as_deref(), file.as_deref())
    });
    let hyprland = is_hyprland(
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
    );
    let qt = binary_path(QT_BIN, "TCC_GUI_QT_BIN");
    let gpui = binary_path(GPUI_BIN, "TCC_GUI_GPUI_BIN");
    let ui = match resolve(mode, hyprland, qt.is_file(), gpui.is_file()) {
        Ok(Some(ui)) => ui,
        Ok(None) => {
            eprintln!("tuxedo-rust-center: aucune interface installée dans {LIBEXEC_DIR}");
            return ExitCode::FAILURE;
        }
        Err(message) => {
            eprintln!("tuxedo-rust-center: {message}");
            return ExitCode::FAILURE;
        }
    };
    let target = match ui {
        Ui::Qt6 => qt,
        Ui::Gpui => gpui,
    };
    // `exec` ne revient qu'en cas d'échec.
    let error = Command::new(&target).args(&rest).exec();
    eprintln!(
        "tuxedo-rust-center: lancement de {} impossible : {error}",
        target.display()
    );
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn extracts_ui_flag_and_forwards_the_rest() {
        let (mode, rest) = split_args(os(&["--ui", "gpui", "--foo", "bar"])).unwrap();
        assert_eq!(mode, Some(Mode::Gpui));
        assert_eq!(rest, os(&["--foo", "bar"]));
        let (mode, rest) = split_args(os(&["--ui=qt6"])).unwrap();
        assert_eq!(mode, Some(Mode::Qt6));
        assert!(rest.is_empty());
        let (mode, rest) = split_args(os(&["--minimized"])).unwrap();
        assert_eq!(mode, None);
        assert_eq!(rest, os(&["--minimized"]));
    }

    #[test]
    fn rejects_bad_ui_values() {
        assert!(split_args(os(&["--ui"])).is_err());
        assert!(split_args(os(&["--ui", "web"])).is_err());
        assert!(split_args(os(&["--ui=html"])).is_err());
    }

    #[test]
    fn env_beats_file_and_unknown_values_are_ignored() {
        assert_eq!(configured_mode(Some("gpui"), Some("qt6")), Mode::Gpui);
        assert_eq!(configured_mode(None, Some("qt6\n")), Mode::Qt6);
        assert_eq!(configured_mode(Some("nope"), Some("gpui")), Mode::Gpui);
        assert_eq!(configured_mode(None, None), Mode::Auto);
    }

    #[test]
    fn detects_hyprland() {
        assert!(is_hyprland(Some("abc"), None));
        assert!(is_hyprland(None, Some("Hyprland")));
        assert!(is_hyprland(None, Some("X-Foo:hyprland")));
        assert!(!is_hyprland(Some(""), Some("GNOME")));
        assert!(!is_hyprland(None, None));
    }

    #[test]
    fn auto_prefers_gpui_on_hyprland_only() {
        assert_eq!(resolve(Mode::Auto, true, true, true), Ok(Some(Ui::Gpui)));
        assert_eq!(resolve(Mode::Auto, false, true, true), Ok(Some(Ui::Qt6)));
        assert_eq!(resolve(Mode::Auto, true, true, false), Ok(Some(Ui::Qt6)));
        assert_eq!(resolve(Mode::Auto, false, false, true), Ok(Some(Ui::Gpui)));
        assert_eq!(resolve(Mode::Auto, true, false, false), Ok(None));
    }

    #[test]
    fn explicit_mode_never_falls_back() {
        assert!(resolve(Mode::Gpui, true, true, false).is_err());
        assert!(resolve(Mode::Qt6, false, false, true).is_err());
        assert_eq!(resolve(Mode::Qt6, true, true, true), Ok(Some(Ui::Qt6)));
    }
}
