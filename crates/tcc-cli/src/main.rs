// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! CLI de diagnostic du daemon TuxedoRustCenter.
//!
//! Le CLI est non privilégié et parle au daemon via le bus système. Il sert à
//! vérifier l'état, les profils et les réglages sans lancer la GUI.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use anyhow::{Context, Result};
use tcc_ipc::{DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE, DBUS_PATH, DBUS_SERVICE};

/// Nom de service D-Bus, avec override de développement `TCC_DBUS_SERVICE`.
fn service_name() -> String {
    std::env::var("TCC_DBUS_SERVICE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DBUS_SERVICE.to_owned())
}

/// Bus de développement : `TCC_DBUS_BUS=session` pour le mode `--dev-session`.
fn session_bus() -> bool {
    std::env::var("TCC_DBUS_BUS").is_ok_and(|value| value.eq_ignore_ascii_case("session"))
}

#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd",
    default_service = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Tccd {
    #[zbus(name = "GetStateJSON")]
    fn get_state_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetProfilesJSON")]
    fn get_profiles_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetSettingsJSON")]
    fn get_settings_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetFanDataJSON")]
    fn get_fan_data_json(&self) -> zbus::Result<String>;
    #[zbus(name = "SetTempProfile")]
    fn set_temp_profile(&self, id: &str) -> zbus::Result<bool>;
    #[zbus(name = "ClearTempProfile")]
    fn clear_temp_profile(&self) -> zbus::Result<bool>;
    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
}

/// Service administratif à la demande (`tccd-admin`).
#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.Admin1",
    default_service = "com.tuxedocomputers.tccd.Admin",
    default_path = "/com/tuxedocomputers/tccd/Admin"
)]
trait Admin1 {
    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetVersion")]
    fn get_version(&self) -> zbus::Result<String>;
    /// Voie supportée pour le contrat ; ne pas passer par `Properties.Get`.
    #[zbus(name = "GetContractVersion")]
    fn get_contract_version(&self) -> zbus::Result<u32>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Status,
    Profiles,
    Settings,
    Fans,
    Hardware,
    Timer,
    Sensors(Option<bool>),
    Admin,
    SetTemp(String),
    ClearTemp,
    Ping,
    Help,
    Version,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command> {
    let mut iter = args.into_iter();
    let Some(first) = iter.next() else {
        return Ok(Command::Status);
    };
    match first.as_str() {
        "status" | "state" => Ok(Command::Status),
        "profiles" => Ok(Command::Profiles),
        "settings" => Ok(Command::Settings),
        "fans" | "fan" => Ok(Command::Fans),
        "hardware" | "hw" => Ok(Command::Hardware),
        "timer" | "shutdown-timer" => Ok(Command::Timer),
        "admin" => Ok(Command::Admin),
        "sensors" | "sensor" => {
            let action = iter.next();
            match action.as_deref() {
                None | Some("status") => Ok(Command::Sensors(None)),
                Some("on") | Some("true") => Ok(Command::Sensors(Some(true))),
                Some("off") | Some("false") => Ok(Command::Sensors(Some(false))),
                Some(other) => anyhow::bail!("sensors attend on|off|status, pas {other}"),
            }
        }
        "set-temp" => {
            let id = iter
                .next()
                .context("set-temp attend un identifiant de profil")?;
            if iter.next().is_some() {
                anyhow::bail!("set-temp n'accepte qu'un seul identifiant");
            }
            Ok(Command::SetTemp(id))
        }
        "clear-temp" => Ok(Command::ClearTemp),
        "ping" => Ok(Command::Ping),
        "--help" | "-h" | "help" => Ok(Command::Help),
        "--version" | "-V" => Ok(Command::Version),
        other => anyhow::bail!("commande inconnue: {other} (essayez --help)"),
    }
}

fn main() -> Result<()> {
    let command = parse_args(std::env::args().skip(1))?;
    match command {
        Command::Help => {
            println!(
                "tcc-cli {}\n\n\
                 USAGE:\n    tcc-cli [status|profiles|settings|fans|hardware|timer|admin|sensors [on|off|status]|ping|set-temp <id>|clear-temp]\n\n\
                 Le CLI est non privilégié et interroge le daemon sur le bus système.",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        Command::Version => {
            println!("tcc-cli {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("runtime tokio")?;
    runtime.block_on(run(command))
}

async fn run(command: Command) -> Result<()> {
    let service = service_name();
    let connection = if session_bus() {
        zbus::Connection::session().await
    } else {
        zbus::Connection::system().await
    }
    .with_context(|| format!("connexion au bus D-Bus ({service}, {DBUS_PATH})"))?;
    let proxy = TccdProxy::builder(&connection)
        .destination(service.clone())
        .context("destination D-Bus tccd")?
        .path(DBUS_PATH)
        .context("objet D-Bus tccd")?
        .build()
        .await
        .context("proxy D-Bus tccd")?;

    match command {
        Command::Status => print_json(proxy.get_state_json().await?)?,
        Command::Profiles => print_json(proxy.get_profiles_json().await?)?,
        Command::Settings => print_json(proxy.get_settings_json().await?)?,
        Command::Fans => print_json(proxy.get_fan_data_json().await?)?,
        Command::Hardware => {
            let manager = zbus::Proxy::new(
                &connection,
                service_name(),
                DBUS_PATH,
                "com.tuxedocomputers.tccd.Manager1",
            )
            .await
            .context("proxy Manager1")?;
            let raw: String = manager
                .call("GetHardwareSnapshotJSON", &())
                .await
                .context("GetHardwareSnapshotJSON")?;
            print_json(raw)?;
        }
        Command::Timer => {
            let manager = zbus::Proxy::new(
                &connection,
                service_name(),
                DBUS_PATH,
                "com.tuxedocomputers.tccd.Manager1",
            )
            .await
            .context("proxy Manager1")?;
            let raw: String = manager
                .call("GetShutdownTimerJSON", &())
                .await
                .context("GetShutdownTimerJSON")?;
            print_json(raw)?;
        }
        Command::Sensors(action) => {
            let manager = zbus::Proxy::new(
                &connection,
                service_name(),
                DBUS_PATH,
                "com.tuxedocomputers.tccd.Manager1",
            )
            .await
            .context("proxy Manager1")?;
            match action {
                Some(enabled) => {
                    let _: bool = manager
                        .call("SetSensorDataCollectionStatus", &enabled)
                        .await
                        .context("SetSensorDataCollectionStatus")?;
                    println!(
                        "collecte capteurs: {}",
                        if enabled {
                            "activée (bail 10 s)"
                        } else {
                            "désactivée"
                        }
                    );
                }
                None => {
                    let enabled: bool = manager
                        .call("GetSensorDataCollectionStatus", &())
                        .await
                        .context("GetSensorDataCollectionStatus")?;
                    println!(
                        "collecte capteurs: {}",
                        if enabled { "active" } else { "inactive" }
                    );
                }
            }
        }
        Command::Admin => {
            let admin = Admin1Proxy::builder(&connection)
                .destination(DBUS_ADMIN_SERVICE)
                .context("destination D-Bus admin")?
                .path(DBUS_ADMIN_PATH)
                .context("chemin D-Bus admin")?
                .build()
                .await
                .context("service admin non installé ou non activé (TCC_WITH_ADMIN=1)")?;
            let ping = admin
                .ping()
                .await
                .context("service admin non installé ou non activé (TCC_WITH_ADMIN=1)")?;
            let version = admin.get_version().await.context("Admin1.GetVersion")?;
            let contract = admin
                .get_contract_version()
                .await
                .context("Admin1.GetContractVersion")?;
            println!("admin: ping={ping}, version={version}, contrat={contract}");
        }
        Command::SetTemp(id) => {
            let ok = proxy.set_temp_profile(&id).await?;
            println!("profil temporaire '{id}': {ok}");
        }
        Command::ClearTemp => {
            let ok = proxy.clear_temp_profile().await?;
            println!("override temporaire supprimé: {ok}");
        }
        Command::Ping => {
            let ok = proxy.ping().await?;
            println!("tccd: {}", if ok { "vivant" } else { "injoignable" });
        }
        Command::Help | Command::Version => {}
    }
    Ok(())
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn parses_fans() {
        assert_eq!(parse_args(args(&["fans"])).unwrap(), Command::Fans);
        assert_eq!(parse_args(args(&["fan"])).unwrap(), Command::Fans);
    }

    #[test]
    fn parses_admin_and_timer() {
        assert_eq!(parse_args(args(&["admin"])).unwrap(), Command::Admin);
        assert_eq!(parse_args(args(&["timer"])).unwrap(), Command::Timer);
        assert_eq!(
            parse_args(args(&["shutdown-timer"])).unwrap(),
            Command::Timer
        );
    }

    #[test]
    fn parses_hardware_and_sensors() {
        assert_eq!(parse_args(args(&["hardware"])).unwrap(), Command::Hardware);
        assert_eq!(
            parse_args(args(&["sensors"])).unwrap(),
            Command::Sensors(None)
        );
        assert_eq!(
            parse_args(args(&["sensors", "on"])).unwrap(),
            Command::Sensors(Some(true))
        );
        assert_eq!(
            parse_args(args(&["sensors", "off"])).unwrap(),
            Command::Sensors(Some(false))
        );
        assert!(parse_args(args(&["sensors", "nope"])).is_err());
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
}

fn print_json(raw: String) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(&raw).context("réponse D-Bus JSON invalide")?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn default_is_status() {
        assert_eq!(parse_args(args(&[])).unwrap(), Command::Status);
    }

    #[test]
    fn parses_set_temp() {
        assert_eq!(
            parse_args(args(&["set-temp", "__office__"])).unwrap(),
            Command::SetTemp("__office__".to_owned())
        );
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(parse_args(args(&["nope"])).is_err());
        assert!(parse_args(args(&["set-temp"])).is_err());
        assert!(parse_args(args(&["set-temp", "a", "b"])).is_err());
    }
}
