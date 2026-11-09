// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Watchers D-Bus et sysfs des deux conditions de bascule :
//! power-profiles-daemon et l'état AC/batterie.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tcc_core::PowerSource;
use tcc_hw::power_supply::PowerSupplies;
use tcc_hw::sysfs::SysfsRoot;
use tokio::sync::{Notify, RwLock};

use crate::runtime::{Inputs, power_from_hw};

/// Intervalle de secours pour relire sysfs si UPower est absent.
const POWER_FALLBACK_INTERVAL: Duration = Duration::from_secs(5);
/// Délai avant de retenter la connexion à un service D-Bus absent.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

#[zbus::proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

/// Lit le profil PPD initial, sans attendre de signal.
pub async fn read_ppd_initial() -> Option<tcc_core::PpdProfile> {
    let connection = zbus::Connection::system().await.ok()?;
    let proxy = PowerProfilesProxy::new(&connection).await.ok()?;
    let value = proxy.active_profile().await.ok()?;
    Some(tcc_core::PpdProfile::from_dbus(&value))
}

/// Lit l'état UPower initial, sans attendre de signal.
///
/// Retourne `None` si UPower est absent ou en erreur ; l'appelant utilise
/// alors sysfs comme source de secours.
pub async fn read_power_initial() -> Option<PowerSource> {
    let connection = zbus::Connection::system().await.ok()?;
    let proxy = UPowerProxy::new(&connection).await.ok()?;
    let on_battery = proxy.on_battery().await.ok()?;
    Some(PowerSource::from_on_battery(Some(on_battery)))
}

/// Observe power-profiles-daemon en continu, avec reconnexion.
pub async fn watch_ppd(inputs: Arc<RwLock<Inputs>>, notify: Arc<Notify>) {
    loop {
        match run_ppd(&inputs, &notify).await {
            Ok(()) => tracing::warn!("PPD: flux terminé, reconnexion"),
            Err(error) => tracing::debug!(%error, "PPD indisponible"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn run_ppd(
    inputs: &RwLock<Inputs>,
    notify: &Notify,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = zbus::Connection::system().await?;
    let proxy = PowerProfilesProxy::new(&connection).await?;

    match proxy.active_profile().await {
        Ok(value) => update_ppd(inputs, notify, Some(value.as_str())).await,
        Err(error) => tracing::debug!(%error, "PPD: ActiveProfile illisible"),
    }

    let mut changes = proxy.receive_active_profile_changed().await;
    while let Some(change) = changes.next().await {
        match change.get().await {
            Ok(value) => update_ppd(inputs, notify, Some(value.as_str())).await,
            Err(error) => tracing::warn!(%error, "PPD: changement de propriété invalide"),
        }
    }
    Ok(())
}

/// Observe UPower `OnBattery`.
pub async fn watch_upower(inputs: Arc<RwLock<Inputs>>, notify: Arc<Notify>) {
    loop {
        match run_upower(&inputs, &notify).await {
            Ok(()) => tracing::warn!("UPower: flux terminé, reconnexion"),
            Err(error) => tracing::debug!(%error, "UPower indisponible"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn run_upower(
    inputs: &RwLock<Inputs>,
    notify: &Notify,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = zbus::Connection::system().await?;
    let proxy = UPowerProxy::new(&connection).await?;

    if let Ok(on_battery) = proxy.on_battery().await {
        update_power(
            inputs,
            notify,
            PowerSource::from_on_battery(Some(on_battery)),
        )
        .await;
    }

    let mut changes = proxy.receive_on_battery_changed().await;
    while let Some(change) = changes.next().await {
        match change.get().await {
            Ok(on_battery) => {
                update_power(
                    inputs,
                    notify,
                    PowerSource::from_on_battery(Some(on_battery)),
                )
                .await;
            }
            Err(error) => tracing::warn!(%error, "UPower: changement invalide"),
        }
    }
    Ok(())
}

/// Relit sysfs périodiquement. Sert de source de vérité de secours et corrige
/// une valeur UPower obsolète.
pub async fn watch_sysfs_power(inputs: Arc<RwLock<Inputs>>, notify: Arc<Notify>) {
    let root = SysfsRoot::from_env_or_system();
    let mut interval = tokio::time::interval(POWER_FALLBACK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        match PowerSupplies::read(&root) {
            Ok(supplies) => {
                update_power(&inputs, &notify, power_from_hw(supplies.power_state())).await;
            }
            Err(error) => tracing::debug!(%error, "lecture power_supply échouée"),
        }
    }
}

async fn update_ppd(inputs: &RwLock<Inputs>, notify: &Notify, value: Option<&str>) {
    let changed = {
        let mut guard = inputs.write().await;
        guard.set_ppd(value)
    };
    if changed {
        notify.notify_one();
    }
}

async fn update_power(inputs: &RwLock<Inputs>, notify: &Notify, power: PowerSource) {
    let changed = {
        let mut guard = inputs.write().await;
        guard.set_power(power)
    };
    if changed {
        notify.notify_one();
    }
}
