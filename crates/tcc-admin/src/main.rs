// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! Binaire `tccd-admin` — service `Admin1` à la demande (phase 1).
//!
//! Sans argument, le binaire démarre le service D-Bus
//! `com.tuxedocomputers.tccd.Admin` et sort après 30 s sans appel (AD-1).
//! Il refuse de démarrer hors root. Aucun mode `--force`, aucune commande
//! shell, aucun chemin fourni en argument.
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
use tcc_admin::{Activity, AdminService, IDLE_TIMEOUT, build_runtime, spawn_idle_watch};
use tcc_ipc::{ADMIN_CONTRACT_VERSION, DBUS_ADMIN_INTERFACE, DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE};
use thiserror::Error;

/// Code de sortie pour un argument invalide.
const EXIT_USAGE: u8 = 2;
/// Code de sortie `EX_NOPERM` : processus non root.
const EXIT_NOT_ROOT: u8 = 77;

const HELP: &str = "\
tccd-admin — actions administratives à la demande de TuxedoRustCenter

USAGE:
    tccd-admin
    tccd-admin --help
    tccd-admin --version

Sans argument, démarre le service D-Bus com.tuxedocomputers.tccd.Admin
(interface Admin1), activé à la demande. Le processus exige root, n'expose
aucun paramètre de chemin ou de commande et sort après 30 s sans appel.

Méthodes de phase 1 : Ping, GetVersion, SaveProfilesJSON, SaveSettingsJSON,
ReloadConfig. Chaque écriture est autorisée par polkit
(com.tuxedocomputers.tccd1.profiles ou ...settings), validée, sérialisée et
annulée si le rechargement de tccd échoue.
";

/// Commande reconnue par le binaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Affiche l'aide.
    Help,
    /// Affiche la version et la version de contrat.
    Version,
    /// Mode service (défaut).
    Service,
}

/// Erreur de parsing des arguments.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
enum ArgsError {
    /// Argument inconnu.
    #[error("argument inconnu: {0}")]
    Unknown(String),
    /// Argument non UTF-8.
    #[error("argument non UTF-8")]
    NonUtf8,
    /// Argument supplémentaire après `--help`/`--version`.
    #[error("argument en trop: {0}")]
    Extra(String),
}

/// Analyse les arguments, sans jamais paniquer.
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
                "tccd-admin {} ({} contrat v{})",
                env!("CARGO_PKG_VERSION"),
                DBUS_ADMIN_INTERFACE,
                ADMIN_CONTRACT_VERSION
            );
            ExitCode::SUCCESS
        }
        Ok(Command::Service) => {
            if !is_root() {
                eprintln!(
                    "tccd-admin: root requis (service activé par D-Bus, jamais par un utilisateur)"
                );
                return ExitCode::from(EXIT_NOT_ROOT);
            }
            // Filtre fixe : `RUST_LOG` n'est pas honoré, pour qu'un
            // environnement non fiable ne puisse pas activer des traces
            // verbeuses ni faire fuiter des données.
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
                    tracing::error!(%error, "tccd-admin arrêté sur erreur");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("tccd-admin: {error}");
            eprintln!("Essayez « tccd-admin --help ».");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

async fn run_service() -> anyhow::Result<()> {
    let activity = Arc::new(Activity::new());
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let service = AdminService::production(Arc::clone(&activity));

    let connection = zbus::connection::Builder::system()
        .context("connexion au bus système")?
        .name(DBUS_ADMIN_SERVICE)
        .context("réservation du nom admin")?
        .serve_at(DBUS_ADMIN_PATH, service)
        .context("enregistrement de l'objet Admin1")?
        .build()
        .await
        .context("démarrage du service D-Bus admin")?;
    tracing::info!(
        service = DBUS_ADMIN_SERVICE,
        path = DBUS_ADMIN_PATH,
        interface = DBUS_ADMIN_INTERFACE,
        contract = ADMIN_CONTRACT_VERSION,
        idle_timeout_seconds = IDLE_TIMEOUT.as_secs(),
        "tccd-admin démarré"
    );

    spawn_idle_watch(Arc::clone(&activity), Arc::clone(&shutdown), IDLE_TIMEOUT);

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("handler SIGTERM")?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("handler SIGINT")?;

    tokio::select! {
        _ = shutdown.notified() => tracing::info!("arrêt après inactivité"),
        _ = sigterm.recv() => tracing::info!("arrêt sur SIGTERM"),
        _ = sigint.recv() => tracing::info!("arrêt sur SIGINT"),
    }

    // Aucun état à sauver : la fermeture de la connexion relâche le nom.
    drop(connection);
    tracing::info!("tccd-admin arrêté");
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

    use super::{ArgsError, Command, is_root, parse_args};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_argument_selects_service_mode() {
        assert_eq!(parse_args(args(&[])), Ok(Command::Service));
    }

    #[test]
    fn help_and_version_are_recognized() {
        assert_eq!(parse_args(args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse_args(args(&["-h"])), Ok(Command::Help));
        assert_eq!(parse_args(args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse_args(args(&["-V"])), Ok(Command::Version));
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        assert!(matches!(
            parse_args(args(&["--force"])),
            Err(ArgsError::Unknown(_))
        ));
        assert!(matches!(
            parse_args(args(&["save"])),
            Err(ArgsError::Unknown(_))
        ));
    }

    #[test]
    fn extra_arguments_are_rejected() {
        assert!(matches!(
            parse_args(args(&["--version", "--help"])),
            Err(ArgsError::Extra(_))
        ));
    }

    #[test]
    fn root_probe_does_not_panic() {
        let _ = is_root();
    }
}
