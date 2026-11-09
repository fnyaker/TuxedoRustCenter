// SPDX-License-Identifier: GPL-3.0-or-later
//! CLI de diagnostic et d'application manuelle de `tccd-user`.
//!
//! Le mode agent (sans argument) lit le profil actif sur le bus D-Bus et
//! applique automatiquement le taux de rafraîchissement de l'écran interne.
//! Les commandes `--list`/`--set-refresh` restent disponibles pour le
//! diagnostic et les tests manuels.

use std::process::ExitCode;

use serde::Serialize;
use tcc_hw::display::{
    DEFAULT_REFRESH_TOLERANCE_HZ, DisplayMode, DisplayModePlan, MAX_DISPLAY_DIMENSION,
    apply_internal_display_mode, apply_internal_refresh_rate, plan_internal_display_mode,
};
use tcc_hw::error::HwError;

use crate::backend::{self, RefreshPlan};
use crate::error::{Result, UserError};
use crate::security;
use crate::session::SessionInfo;

/// Commande CLI demandée.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    List {
        dry_run: bool,
    },
    SetRefresh {
        hz: f32,
        dry_run: bool,
    },
    SetResolution {
        width: u32,
        height: u32,
        refresh_hz: Option<f32>,
        dry_run: bool,
    },
    Agent,
    Help,
    Version,
}

/// Parse les arguments (hors nom du binaire).
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command> {
    let mut list = false;
    let mut set_refresh: Option<f32> = None;
    let mut set_resolution: Option<(u32, u32)> = None;
    let mut dry_run = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--list" => {
                if list {
                    return Err(UserError::InvalidArgument(
                        "--list spécifié plusieurs fois".to_owned(),
                    ));
                }
                list = true;
            }
            "--set-refresh" => {
                let value = iter.next().ok_or_else(|| {
                    UserError::InvalidArgument("--set-refresh attend une valeur en Hz".to_owned())
                })?;
                if set_refresh.is_some() {
                    return Err(UserError::InvalidArgument(
                        "--set-refresh spécifié plusieurs fois".to_owned(),
                    ));
                }
                set_refresh = Some(parse_refresh(&value)?);
            }
            "--set-resolution" => {
                let value = iter.next().ok_or_else(|| {
                    UserError::InvalidArgument("--set-resolution attend une valeur WxH".to_owned())
                })?;
                if set_resolution.is_some() {
                    return Err(UserError::InvalidArgument(
                        "--set-resolution spécifié plusieurs fois".to_owned(),
                    ));
                }
                set_resolution = Some(parse_resolution(&value)?);
            }
            "--dry-run" => dry_run = true,
            "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            other if other.starts_with("--set-refresh=") => {
                if set_refresh.is_some() {
                    return Err(UserError::InvalidArgument(
                        "--set-refresh spécifié plusieurs fois".to_owned(),
                    ));
                }
                let value = other.trim_start_matches("--set-refresh=");
                set_refresh = Some(parse_refresh(value)?);
            }
            other if other.starts_with("--set-resolution=") => {
                if set_resolution.is_some() {
                    return Err(UserError::InvalidArgument(
                        "--set-resolution spécifié plusieurs fois".to_owned(),
                    ));
                }
                let value = other.trim_start_matches("--set-resolution=");
                set_resolution = Some(parse_resolution(value)?);
            }
            other => {
                return Err(UserError::InvalidArgument(format!(
                    "option inconnue : {other}"
                )));
            }
        }
    }

    if list && (set_refresh.is_some() || set_resolution.is_some()) {
        return Err(UserError::InvalidArgument(
            "--list est incompatible avec --set-refresh/--set-resolution".to_owned(),
        ));
    }
    if list {
        return Ok(Command::List { dry_run });
    }
    if let Some((width, height)) = set_resolution {
        return Ok(Command::SetResolution {
            width,
            height,
            refresh_hz: set_refresh,
            dry_run,
        });
    }
    if let Some(hz) = set_refresh {
        return Ok(Command::SetRefresh { hz, dry_run });
    }
    if dry_run {
        return Err(UserError::InvalidArgument(
            "--dry-run nécessite --list, --set-refresh ou --set-resolution".to_owned(),
        ));
    }
    Ok(Command::Agent)
}

fn parse_resolution(raw: &str) -> Result<(u32, u32)> {
    let trimmed = raw.trim();
    let (width, height) = trimmed
        .split_once(['x', 'X'])
        .ok_or_else(|| UserError::InvalidArgument(format!("résolution invalide : {raw}")))?;
    let parse_dimension = |value: &str| -> Result<u32> {
        let parsed = value
            .parse::<u32>()
            .map_err(|_| UserError::InvalidArgument(format!("résolution invalide : {raw}")))?;
        if parsed == 0 || parsed > MAX_DISPLAY_DIMENSION {
            return Err(UserError::InvalidArgument(format!(
                "résolution hors plage (1..={MAX_DISPLAY_DIMENSION}) : {raw}"
            )));
        }
        Ok(parsed)
    };
    Ok((parse_dimension(width)?, parse_dimension(height)?))
}

fn parse_refresh(raw: &str) -> Result<f32> {
    let value = raw.trim().parse::<f32>().map_err(|_| {
        UserError::InvalidArgument(format!("taux de rafraîchissement invalide : {raw}"))
    })?;
    if !value.is_finite() || value <= 0.0 || value > 1000.0 {
        return Err(UserError::InvalidArgument(format!(
            "taux de rafraîchissement hors plage (0 < Hz <= 1000) : {raw}"
        )));
    }
    Ok(value)
}

/// Point d'entrée CLI, appelé par `main`.
pub fn run(args: Vec<String>) -> ExitCode {
    match execute(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("tccd-user : {err}");
            ExitCode::FAILURE
        }
    }
}

fn execute(args: Vec<String>) -> Result<ExitCode> {
    let command = match parse_args(args) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("tccd-user : {err}");
            eprintln!();
            eprintln!("{}", usage());
            return Ok(ExitCode::from(2));
        }
    };

    match command {
        Command::Help => {
            println!("{}", usage());
            return Ok(ExitCode::SUCCESS);
        }
        Command::Version => {
            println!("tccd-user {}", crate::version());
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }

    // Refus root pour toutes les commandes opérationnelles (D7/ moindre
    // privilège). `--help`/`--version` restent disponibles.
    let uid = security::ensure_not_root()?;
    let session = SessionInfo::detect();

    match command {
        Command::List { dry_run } => print_list(&session, uid, dry_run),
        Command::SetRefresh { hz, dry_run } => set_refresh(&session, uid, hz, dry_run),
        Command::SetResolution {
            width,
            height,
            refresh_hz,
            dry_run,
        } => set_resolution(&session, uid, width, height, refresh_hz, dry_run),
        Command::Agent => {
            tracing::info!(
                session = session.session_type.as_str(),
                backend = session.backend.as_str(),
                "mode agent : connexion au daemon sur le bus D-Bus"
            );
            run_agent(uid, session)
        }
        Command::Help | Command::Version => Ok(ExitCode::SUCCESS),
    }
}

fn run_agent(uid: u32, session: SessionInfo) -> Result<ExitCode> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| UserError::AgentTask(format!("runtime tokio : {err}")))?;
    runtime.block_on(crate::agent::run(uid, session))?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct ListReport {
    version: &'static str,
    session: SessionReport,
    backend: BackendReport,
    internal_output: Option<OutputReport>,
    supported: bool,
    dry_run: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionReport {
    session_type: String,
    desktop: Option<String>,
    wayland_display: Option<String>,
    display: Option<String>,
    uid: u32,
}

#[derive(Debug, Serialize)]
struct BackendReport {
    kind: &'static str,
    name: &'static str,
    available: bool,
}

#[derive(Debug, Serialize)]
struct OutputReport {
    name: String,
    connected: bool,
    enabled: bool,
    current_mode: Option<ModeReport>,
    modes: Vec<ModeReport>,
}

#[derive(Debug, Clone, Serialize)]
struct ModeReport {
    width: u32,
    height: u32,
    refresh_hz: f32,
    preferred: bool,
    current: bool,
}

impl From<&DisplayMode> for ModeReport {
    fn from(mode: &DisplayMode) -> Self {
        Self {
            width: mode.width,
            height: mode.height,
            refresh_hz: mode.refresh_hz,
            preferred: mode.preferred,
            current: mode.current,
        }
    }
}

fn print_list(session: &SessionInfo, uid: u32, dry_run: bool) -> Result<ExitCode> {
    let mut backend_kind = session.backend;
    let mut backend_name = session.backend.as_str();
    let mut available = false;
    let mut outputs = Vec::new();
    let mut error = None;

    match backend::create(session, uid) {
        Ok(mut backend) => {
            backend_kind = backend.kind();
            backend_name = backend.backend_name();
            available = backend.is_available();
            match backend.list_outputs() {
                Ok(list) => outputs = list,
                Err(err) => {
                    available = false;
                    error = Some(err.to_string());
                }
            }
        }
        Err(err) => error = Some(err.to_string()),
    }

    let internal = backend::internal_output(&outputs);
    let supported = available && internal.is_some();
    if internal.is_none() && error.is_none() {
        error = Some("aucun écran interne connecté".to_owned());
    }

    let report = ListReport {
        version: crate::version(),
        session: SessionReport {
            session_type: session.session_type.as_str().to_owned(),
            desktop: session.env.xdg_current_desktop.clone(),
            wayland_display: session.env.wayland_display.clone(),
            display: session.env.display.clone(),
            uid,
        },
        backend: BackendReport {
            kind: backend_kind.as_str(),
            name: backend_name,
            available,
        },
        internal_output: internal.map(|internal| {
            let info = internal.info();
            OutputReport {
                name: info.name.clone(),
                connected: info.connected,
                enabled: info.enabled,
                current_mode: info.current_mode.as_ref().map(ModeReport::from),
                modes: info.modes.iter().map(ModeReport::from).collect(),
            }
        }),
        supported,
        dry_run,
        error,
    };

    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");
    Ok(ExitCode::SUCCESS)
}

fn set_refresh(session: &SessionInfo, uid: u32, hz: f32, dry_run: bool) -> Result<ExitCode> {
    let mut backend = match backend::create(session, uid) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("tccd-user : {err}");
            return Ok(ExitCode::FAILURE);
        }
    };

    if dry_run {
        let outputs = match backend.list_outputs() {
            Ok(outputs) => outputs,
            Err(err) => {
                eprintln!("tccd-user : simulation impossible : {err}");
                return Ok(ExitCode::FAILURE);
            }
        };
        return Ok(report_plan(&outputs, hz));
    }

    match apply_internal_refresh_rate(&mut *backend, hz, DEFAULT_REFRESH_TOLERANCE_HZ) {
        Ok(tcc_hw::display::ApplyOutcome::Applied) => {
            println!("tccd-user : {hz} Hz appliqué sur l'écran interne");
            Ok(ExitCode::SUCCESS)
        }
        Ok(tcc_hw::display::ApplyOutcome::AlreadyApplied) => {
            println!(
                "tccd-user : l'écran interne est déjà à {hz} Hz \
                 (tolérance {} Hz)",
                DEFAULT_REFRESH_TOLERANCE_HZ
            );
            Ok(ExitCode::SUCCESS)
        }
        Ok(tcc_hw::display::ApplyOutcome::Unsupported) => {
            eprintln!("tccd-user : changement de taux non supporté par ce backend");
            Ok(ExitCode::FAILURE)
        }
        Err(HwError::NotSupported(reason)) => {
            eprintln!("tccd-user : non supporté : {reason}");
            Ok(ExitCode::FAILURE)
        }
        Err(err) => {
            eprintln!("tccd-user : échec : {err}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn set_resolution(
    session: &SessionInfo,
    uid: u32,
    width: u32,
    height: u32,
    refresh_hz: Option<f32>,
    dry_run: bool,
) -> Result<ExitCode> {
    let mut backend = match backend::create(session, uid) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("tccd-user : {err}");
            return Ok(ExitCode::FAILURE);
        }
    };

    if dry_run {
        let outputs = match backend.list_outputs() {
            Ok(outputs) => outputs,
            Err(err) => {
                eprintln!("tccd-user : simulation impossible : {err}");
                return Ok(ExitCode::FAILURE);
            }
        };
        return Ok(report_display_plan(plan_internal_display_mode(
            &outputs,
            Some(width),
            Some(height),
            refresh_hz,
            DEFAULT_REFRESH_TOLERANCE_HZ,
        )));
    }

    match apply_internal_display_mode(
        &mut *backend,
        Some(width),
        Some(height),
        refresh_hz,
        DEFAULT_REFRESH_TOLERANCE_HZ,
    ) {
        Ok(tcc_hw::display::ApplyOutcome::Applied) => {
            match refresh_hz {
                Some(hz) => {
                    println!("tccd-user : {width}x{height} @ {hz} Hz appliqué sur l'écran interne")
                }
                None => println!(
                    "tccd-user : {width}x{height} appliqué sur l'écran interne (taux conservé)"
                ),
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(tcc_hw::display::ApplyOutcome::AlreadyApplied) => {
            println!("tccd-user : l'écran interne est déjà dans le mode demandé");
            Ok(ExitCode::SUCCESS)
        }
        Ok(tcc_hw::display::ApplyOutcome::Unsupported) => {
            eprintln!("tccd-user : changement de mode non supporté par ce backend");
            Ok(ExitCode::FAILURE)
        }
        Err(HwError::NotSupported(reason)) => {
            eprintln!("tccd-user : non supporté : {reason}");
            Ok(ExitCode::FAILURE)
        }
        Err(err) => {
            eprintln!("tccd-user : échec : {err}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn report_display_plan(plan: tcc_hw::Result<DisplayModePlan>) -> ExitCode {
    match plan {
        Ok(DisplayModePlan::AlreadyApplied { output, mode }) => {
            println!(
                "tccd-user : aucun changement, {} est déjà en {}x{} @ {} Hz",
                output.name(),
                mode.width,
                mode.height,
                mode.refresh_hz
            );
            ExitCode::SUCCESS
        }
        Ok(DisplayModePlan::Apply { output, mode }) => {
            println!(
                "tccd-user : appliquerait {}x{} @ {} Hz sur {}",
                mode.width,
                mode.height,
                mode.refresh_hz,
                output.name()
            );
            ExitCode::SUCCESS
        }
        Err(HwError::NoDisplayBackend) => {
            eprintln!("tccd-user : aucun écran interne connecté");
            ExitCode::FAILURE
        }
        Err(HwError::DisplayModeNotFound(detail)) => {
            eprintln!("tccd-user : mode interne introuvable : {detail}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("tccd-user : simulation impossible : {err}");
            ExitCode::FAILURE
        }
    }
}

fn report_plan(outputs: &[tcc_hw::display::OutputInfo], hz: f32) -> ExitCode {
    match backend::plan_refresh(outputs, hz, DEFAULT_REFRESH_TOLERANCE_HZ) {
        RefreshPlan::AlreadyApplied { output, current_hz } => {
            println!("tccd-user : aucun changement, {output} est déjà à ~{current_hz} Hz");
            ExitCode::SUCCESS
        }
        RefreshPlan::Apply { output, mode } => {
            println!(
                "tccd-user : appliquerait {} Hz ({}x{}) sur {output}",
                mode.refresh_hz, mode.width, mode.height
            );
            ExitCode::SUCCESS
        }
        RefreshPlan::NoInternalOutput => {
            eprintln!("tccd-user : aucun écran interne connecté");
            ExitCode::FAILURE
        }
        RefreshPlan::NotFound { output, target_hz } => {
            eprintln!(
                "tccd-user : aucun mode à {target_hz} Hz à la résolution courante de {output}"
            );
            ExitCode::FAILURE
        }
    }
}

fn usage() -> &'static str {
    "tccd-user — agent de session TuxedoRustCenter (affichage interne)

Usage :
  tccd-user --list [--dry-run]
  tccd-user --set-refresh <hz> [--dry-run]
  tccd-user --set-resolution <WxH> [--set-refresh <hz>] [--dry-run]
  tccd-user --help | --version
  tccd-user                     # mode agent : applique le profil actif

Options :
  --list          Affiche un rapport JSON : session, backend, écran interne,
                  modes disponibles et mode courant.
  --set-refresh   Applique le taux demandé à l'écran interne uniquement.
  --set-resolution Applique la résolution WxH à l'écran interne. Sans
                  --set-refresh, le taux courant est conservé.
  --dry-run       N'applique rien et décrit la décision.
  --help, -h      Affiche cette aide.
  --version, -V   Affiche la version.

L'agent refuse de tourner en root et ne modifie jamais un écran externe.
Sans argument, il reste actif, lit le profil actif de tccd et applique le mode
(taux et/ou résolution) de l'écran interne (SIGTERM/SIGINT pour l'arrêter)."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn no_arguments_is_agent_mode() {
        assert_eq!(parse_args(args(&[])).unwrap(), Command::Agent);
    }

    #[test]
    fn parses_list_and_dry_run() {
        assert_eq!(
            parse_args(args(&["--list", "--dry-run"])).unwrap(),
            Command::List { dry_run: true }
        );
    }

    #[test]
    fn parses_set_refresh_both_forms() {
        assert_eq!(
            parse_args(args(&["--set-refresh", "165"])).unwrap(),
            Command::SetRefresh {
                hz: 165.0,
                dry_run: false
            }
        );
        assert_eq!(
            parse_args(args(&["--dry-run", "--set-refresh=59.94"])).unwrap(),
            Command::SetRefresh {
                hz: 59.94,
                dry_run: true
            }
        );
    }

    #[test]
    fn parses_set_resolution_both_forms() {
        assert_eq!(
            parse_args(args(&["--set-resolution", "1920x1080"])).unwrap(),
            Command::SetResolution {
                width: 1920,
                height: 1080,
                refresh_hz: None,
                dry_run: false
            }
        );
        assert_eq!(
            parse_args(args(&["--dry-run", "--set-resolution=2560X1440"])).unwrap(),
            Command::SetResolution {
                width: 2560,
                height: 1440,
                refresh_hz: None,
                dry_run: true
            }
        );
    }

    #[test]
    fn parses_resolution_and_refresh_together() {
        assert_eq!(
            parse_args(args(&[
                "--set-resolution",
                "1920x1080",
                "--set-refresh",
                "165",
                "--dry-run"
            ]))
            .unwrap(),
            Command::SetResolution {
                width: 1920,
                height: 1080,
                refresh_hz: Some(165.0),
                dry_run: true
            }
        );
    }

    #[test]
    fn rejects_invalid_resolutions() {
        for value in [
            "1920",
            "0x1080",
            "1920x0",
            "1920x1080x2",
            "20000x1080",
            "axb",
            "",
        ] {
            assert!(
                parse_args(args(&["--set-resolution", value])).is_err(),
                "résolution acceptée à tort : {value}"
            );
        }
    }

    #[test]
    fn resolution_conflicts_with_list() {
        assert!(parse_args(args(&["--list", "--set-resolution", "1920x1080"])).is_err());
    }

    #[test]
    fn rejects_conflicting_commands() {
        assert!(parse_args(args(&["--list", "--set-refresh", "60"])).is_err());
        assert!(parse_args(args(&["--dry-run"])).is_err());
        assert!(parse_args(args(&["--unknown"])).is_err());
        assert!(parse_args(args(&["--set-refresh"])).is_err());
    }

    #[test]
    fn rejects_invalid_rates() {
        for value in ["0", "-1", "nan", "inf", "1001", "abc"] {
            assert!(
                parse_args(args(&["--set-refresh", value])).is_err(),
                "valeur acceptée à tort : {value}"
            );
        }
    }

    #[test]
    fn help_and_version_win() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["-h"])).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["--version"])).unwrap(), Command::Version);
        assert_eq!(parse_args(args(&["-V"])).unwrap(), Command::Version);
    }

    #[test]
    fn mode_report_conversion() {
        let mode = DisplayMode {
            width: 2560,
            height: 1440,
            refresh_hz: 165.0,
            preferred: true,
            current: false,
        };
        let report = ModeReport::from(&mode);
        assert_eq!(report.width, 2560);
        assert_eq!(report.refresh_hz, 165.0);
        assert!(report.preferred);
    }
}
