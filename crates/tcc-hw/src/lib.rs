// SPDX-License-Identifier: GPL-3.0-or-later
//! Couche matérielle Linux de TuxedoRustCenter.
//!
//! Ce crate est volontairement synchrone et sans dépendance à un runtime
//! asynchrone : le daemon l'appelle depuis des workers dédiés ou via
//! `spawn_blocking`. Il ne dépend ni de `tcc-core`, ni de `tcc-ipc`, ni de Qt.
//!
//! # Principes
//!
//! - **Racine sysfs injectable** : [`SysfsRoot`] pointe sur `/sys` en
//!   production, sur un répertoire temporaire en test. La variable
//!   `TCC_SYSFS_ROOT` n'est lue que si la feature `dev-overrides` est compilée
//!   et que le processus n'est pas root (D7) ; elle est absente des paquets de
//!   release.
//! - **Écritures contrôlées** : `O_NOFOLLOW`, vérification `fstat`, chemin
//!   relatif sans `..`, propriétaire root et absence de bit world-writable sur
//!   la racine `/sys`. Voir [`sysfs`].
//! - **Erreurs typées** : [`HwError`] distingue l'absence matérielle (cas
//!   normal) des erreurs réelles.
//! - **Dégradation propre** : un matériel non-TUXEDO ne doit jamais empêcher
//!   le daemon de démarrer ; les fonctions absentes retournent
//!   [`HwError::NotSupported`] ou une liste vide.
//! - **`unsafe` isolé** : uniquement dans [`tuxedo_io::real`] (ioctl
//!   Clevo/Uniwill), `v4l2::real` (quatre ioctl V4L2 de lecture) et
//!   `v4l2::real_write` (`S_CTRL` de l'agent de session), avec commentaires
//!   de sûreté et validation préalable des index/plages. Ce sont les seules
//!   dérogations à l'interdiction `unsafe` du crate, revues explicitement ;
//!   l'override de développement n'utilise pas `unsafe`.
//!
//! # Feature `dev-overrides`
//!
//! Active `TCC_SYSFS_ROOT` pour les tests d'intégration. Elle est refusée en
//! root, n'est jamais activée dans les paquets, et les unités systemd du
//! dépôt ne transmettent aucune variable d'override.
//!
//! # Conversions vers `tcc-core`
//!
//! Ce crate n'importe pas les types de `tcc-core` (frontière de dépendance).
//! Le daemon devra convertir :
//!
//! - [`power_supply::PowerState`] → `tcc_core::PowerSource` (même
//!   sémantique `Ac`/`Battery`/`Unknown`) ;
//! - les champs `Option`/`Vec` de [`cpu::CpuPolicy`] vers le profil CPU
//!   (les fréquences sont en kHz dans les deux sens) ;
//! - [`fan::FanTarget`] vers `tcc_core::fan::FanTarget`, et les pourcentages de
//!   [`fan::FanControl`] vers le profil de ventilation ;
//! - [`display::DisplayMode`] vers le payload `tcc-ipc` du refresh rate ;
//! - [`v4l2::UvcControlInfo`] vers le payload `WebcamData` (lecture seule) ;
//! - [`charging::ChargingCapabilities`] et [`charging::ChargeType`] vers les
//!   setters `SetChargingProfile`/`SetChargeType` et le JSON de statut ;
//! - [`lighting::RgbColor`] et [`lighting::KeyboardCapabilities`] vers
//!   `tcc_core::KeyboardState`/`LightbarState` (la luminosité reste sur
//!   `0..=255`, plafonnée par le matériel) ;
//! - [`platform_profile::PlatformProfile`] et [`tdp::TdpLimit`] vers les
//!   payloads ODM (`GetODMProfilesJSON`, `GetODMTDPJSON`) ;
//! - [`sensors::SensorSnapshot`] vers les payloads `GetIGpuInfoValuesJSON` /
//!   `GetDGpuInfoValuesJSON` / `GetCpuPowerValuesJSON` ;
//! - [`device::TuxedoDevice`] vers le champ `device` du diagnostic.
//!
//! Voir `docs/hardware/HARDWARE_MATRIX.md` pour le tableau complet des APIs et
//! des points de branchement, et `docs/linux/PLATFORM_DESIGN.md` pour les
//! décisions matérielles.

#![deny(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![warn(missing_debug_implementations)]

pub mod backlight;
pub mod charging;
pub mod command;
pub mod cpu;
pub mod device;
pub mod display;
pub mod dmi;
pub mod error;
pub mod fan;
pub mod fn_lock;
pub mod lighting;
pub mod nvidia;
pub mod platform_profile;
pub mod power_supply;
pub mod prime;
pub mod sensors;
pub mod sysfs;
pub mod tdp;
pub mod tuxedo_io;
pub mod v4l2;
pub mod webcam;
pub mod ycbcr;

pub use error::{HwError, Result};
pub use sysfs::SysfsRoot;

/// Version du crate.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn sysfs_root_rejects_absolute_paths() {
        let root = SysfsRoot::system();
        assert!(root.resolve("/etc/passwd").is_err());
    }
}
