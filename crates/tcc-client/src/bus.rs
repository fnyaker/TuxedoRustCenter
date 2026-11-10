// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Connexions et appels D-Bus génériques du CLI.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use tcc_ipc::{
    DBUS_ADMIN_HW_INTERFACE, DBUS_ADMIN_HW_PATH, DBUS_ADMIN_HW_SERVICE, DBUS_ADMIN_INTERFACE,
    DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE, DBUS_INTERFACE, DBUS_MANAGER_INTERFACE, DBUS_PATH,
    DBUS_SERVICE, DBUS_SESSION_AGENT_INTERFACE, DBUS_SESSION_AGENT_PATH,
    DBUS_SESSION_AGENT_SERVICE,
};
use zbus::zvariant::DynamicType;

/// Quel démon interroger.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// `tccd` — interface JSON de compatibilité.
    Daemon,
    /// `tccd` — interface typée `Manager1`.
    Manager,
    /// `tccd-admin` (profils, réglages, rechargement).
    Admin,
    /// `tccd-admin-hw` (TDP, ODM, cTGP).
    AdminHw,
    /// `tccd-user`, sur le bus de session.
    Session,
}

/// Client D-Bus.
pub struct Bus {
    system: Option<zbus::Connection>,
    session: Option<zbus::Connection>,
    dev_session: bool,
}

impl Bus {
    /// `dev_session` : `TCC_DBUS_BUS=session` (mode `--dev-session` du daemon).
    pub fn new(dev_session: bool) -> Self {
        Self {
            system: None,
            session: None,
            dev_session,
        }
    }

    async fn connection(&mut self, session: bool) -> Result<zbus::Connection> {
        let slot = if session {
            &mut self.session
        } else {
            &mut self.system
        };
        if let Some(connection) = slot {
            return Ok(connection.clone());
        }
        let connection = if session {
            zbus::Connection::session().await
        } else {
            zbus::Connection::system().await
        }
        .context("connexion au bus D-Bus")?;
        *slot = Some(connection.clone());
        Ok(connection)
    }

    /// Appelle `method` et décode la réponse.
    pub async fn call<B, R>(&mut self, target: Target, method: &str, body: &B) -> Result<R>
    where
        B: serde::Serialize + DynamicType,
        R: DeserializeOwned + zbus::zvariant::Type,
    {
        let (service, path, interface, session) = match target {
            Target::Daemon => (
                daemon_service(),
                DBUS_PATH,
                DBUS_INTERFACE,
                self.dev_session,
            ),
            Target::Manager => (
                daemon_service(),
                DBUS_PATH,
                DBUS_MANAGER_INTERFACE,
                self.dev_session,
            ),
            Target::Admin => (
                DBUS_ADMIN_SERVICE.to_owned(),
                DBUS_ADMIN_PATH,
                DBUS_ADMIN_INTERFACE,
                self.dev_session,
            ),
            Target::AdminHw => (
                DBUS_ADMIN_HW_SERVICE.to_owned(),
                DBUS_ADMIN_HW_PATH,
                DBUS_ADMIN_HW_INTERFACE,
                self.dev_session,
            ),
            Target::Session => (
                DBUS_SESSION_AGENT_SERVICE.to_owned(),
                DBUS_SESSION_AGENT_PATH,
                DBUS_SESSION_AGENT_INTERFACE,
                true,
            ),
        };
        let connection = self.connection(session).await?;
        let proxy = zbus::Proxy::new(&connection, service.clone(), path, interface)
            .await
            .with_context(|| format!("proxy D-Bus {service}"))?;
        proxy
            .call(method, body)
            .await
            .with_context(|| hint(target, method))
    }
}

fn hint(target: Target, method: &str) -> String {
    match target {
        Target::Daemon | Target::Manager => {
            format!("{method} : daemon tccd injoignable ou appel refusé")
        }
        Target::Admin => format!(
            "{method} : service admin (tccd-admin) indisponible ou autorisation polkit refusée"
        ),
        Target::AdminHw => format!(
            "{method} : service matériel (tccd-admin-hw) indisponible ou autorisation polkit refusée"
        ),
        Target::Session => {
            format!("{method} : agent de session tccd-user injoignable (bus de session)")
        }
    }
}

/// Nom de service D-Bus du daemon, avec override `TCC_DBUS_SERVICE`.
pub fn daemon_service() -> String {
    std::env::var("TCC_DBUS_SERVICE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DBUS_SERVICE.to_owned())
}
