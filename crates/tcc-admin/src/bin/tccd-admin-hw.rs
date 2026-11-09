// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Binaire `tccd-admin-hw` — limites matérielles phase 2 (TDP/ODM/cTGP).
//!
//! Service root séparé, activé à la demande par D-Bus, sans état persistant.
//! Il n'expose que trois scalaires bornés (`ai`, `i`, `s`) et n'exécute aucune
//! commande externe.
//!
//! # Codes de sortie
//!
//! | Code | Signification |
//! |---|---|
//! | 0 | arrêt propre (`--help`, `--version`, inactivité, signal) |
//! | 1 | erreur de démarrage (bus indisponible, nom pris, etc.) |
//! | 2 | argument invalide |
//! | 77 | processus non root (`EX_NOPERM`) |

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use tcc_admin::{
    Activity, AdminHwService, BoundedAdminHwInterface, HW_IDLE_TIMEOUT, build_runtime,
    spawn_idle_watch,
};
use tcc_ipc::{
    ADMIN_HW_CONTRACT_VERSION, DBUS_ADMIN_HW_INTERFACE, DBUS_ADMIN_HW_PATH, DBUS_ADMIN_HW_SERVICE,
};
use thiserror::Error;

/// Code de sortie pour un argument invalide.
const EXIT_USAGE: u8 = 2;
/// Code de sortie `EX_NOPERM` : processus non root.
const EXIT_NOT_ROOT: u8 = 77;

const HELP: &str = "\
tccd-admin-hw — limites matérielles à la demande de TuxedoRustCenter

USAGE:
    tccd-admin-hw
    tccd-admin-hw --help
    tccd-admin-hw --version

Sans argument, démarre le service D-Bus com.tuxedocomputers.tccd.Admin.Hw
(interface AdminHw1), activé à la demande. Le processus exige root, n'accepte
aucun chemin ou commande et sort après 30 s sans appel.

Méthodes : Ping, GetVersion, GetContractVersion, GetTdpValues, SetTdpValues,
SetOdmProfile, SetOdmProfileOverride, SetNvidiaCtgpOffset. Les écritures sont
autorisées par polkit com.tuxedocomputers.tccd1.power-limits, sérialisées et
vérifiées. Aucune persistance : une écriture peut être réappliquée par tccd au
prochain changement de profil.
";

/// Commande reconnue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Service,
}

/// Erreur de parsing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
enum ArgsError {
    #[error("argument inconnu: {0}")]
    Unknown(String),
    #[error("argument non UTF-8")]
    NonUtf8,
    #[error("argument en trop: {0}")]
    Extra(String),
}

fn parse_args<I>(args: I) -> Result<Command, ArgsError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Service);
    };
    let first = first.into_string().map_err(|_| ArgsError::NonUtf8)?;
    let command = match first.as_str() {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        other => return Err(ArgsError::Unknown(other.to_owned())),
    };
    if let Some(extra) = args.next() {
        let extra = extra.into_string().map_err(|_| ArgsError::NonUtf8)?;
        return Err(ArgsError::Extra(extra));
    }
    Ok(command)
}

fn main() -> ExitCode {
    match parse_args(std::env::args_os().skip(1)) {
        Ok(Command::Help) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!(
                "tccd-admin-hw {} ({} contrat v{})",
                env!("CARGO_PKG_VERSION"),
                DBUS_ADMIN_HW_INTERFACE,
                ADMIN_HW_CONTRACT_VERSION
            );
            ExitCode::SUCCESS
        }
        Ok(Command::Service) => {
            if !is_root() {
                eprintln!(
                    "tccd-admin-hw: root requis (service activé par D-Bus, jamais par un utilisateur)"
                );
                return ExitCode::from(EXIT_NOT_ROOT);
            }
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
                .init();
            let runtime = match build_runtime() {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(%error, "construction du runtime tokio");
                    return ExitCode::FAILURE;
                }
            };
            match runtime.block_on(run_service()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!(%error, "tccd-admin-hw arrêté sur erreur");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("tccd-admin-hw: {error}");
            eprintln!("Essayez « tccd-admin-hw --help ».");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

async fn run_service() -> anyhow::Result<()> {
    let activity = Arc::new(Activity::new());
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let service = BoundedAdminHwInterface::new(AdminHwService::production(Arc::clone(&activity)));

    let connection = zbus::connection::Builder::system()
        .context("connexion au bus système")?
        // Borne la file de messages du service court (AH-01). La taille du
        // corps est gardée à 64 Kio dans `BoundedAdminHwInterface`.
        .max_queued(8)
        .name(DBUS_ADMIN_HW_SERVICE)
        .context("réservation du nom Admin.Hw")?
        .serve_at(DBUS_ADMIN_HW_PATH, service)
        .context("enregistrement de l'objet AdminHw1")?
        .build()
        .await
        .context("démarrage du service D-Bus Admin.Hw")?;
    tracing::info!(
        service = DBUS_ADMIN_HW_SERVICE,
        path = DBUS_ADMIN_HW_PATH,
        interface = DBUS_ADMIN_HW_INTERFACE,
        contract = ADMIN_HW_CONTRACT_VERSION,
        idle_timeout_seconds = HW_IDLE_TIMEOUT.as_secs(),
        "tccd-admin-hw démarré"
    );

    spawn_idle_watch(
        Arc::clone(&activity),
        Arc::clone(&shutdown),
        HW_IDLE_TIMEOUT,
    );

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("handler SIGTERM")?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("handler SIGINT")?;

    tokio::select! {
        _ = shutdown.notified() => tracing::info!("arrêt après inactivité"),
        _ = sigterm.recv() => tracing::info!("arrêt sur SIGTERM"),
        _ = sigint.recv() => tracing::info!("arrêt sur SIGINT"),
    }

    drop(connection);
    tracing::info!("tccd-admin-hw arrêté");
    Ok(())
}

/// Vrai si l'UID effectif est root, sans `unsafe`/libc.
#[must_use]
fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(|value| value.parse::<u32>().ok())
        })
        .is_some_and(|uid| uid == 0)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{ArgsError, Command, parse_args};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn service_is_default() {
        assert_eq!(parse_args(args(&[])), Ok(Command::Service));
    }

    #[test]
    fn help_version_and_unknown() {
        assert_eq!(parse_args(args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse_args(args(&["--version"])), Ok(Command::Version));
        assert!(matches!(
            parse_args(args(&["--dry-run"])),
            Err(ArgsError::Unknown(_))
        ));
    }
}
