// SPDX-License-Identifier: GPL-3.0-or-later
//! PRIME : **état** du mode graphique (iGPU / dGPU / on-demand).
//!
//! Sources : `PrimeWorker` de l'original et `/usr/bin/prime-select`.
//!
//! # Détection
//!
//! ```text
//! /usr/bin/prime-select                                    (allowlist)
//! dpkg-query -W -f='${Version}\n' nvidia-prime             (allowlist)
//! /var/lib/ubuntu-drivers-common/requires_offloading       (existence)
//! /sys/class/drm/*eDP*/status                              (connecteur interne)
//! /sys/class/drm/*eDP*/device/device/vendor                (0x10de = NVIDIA)
//! ```
//!
//! L'original ne considère PRIME disponible que si `prime-select` est présent
//! **et** fourni par le paquet TUXEDO (`nvidia-prime` dont la version contient
//! `tux`) : le `prime-select` d'Ubuntu n'a pas de mode dGPU fiable sous
//! Wayland. Cette vérification est conservée.
//!
//! Sur IBM15A10, PRIME n'est supporté que si l'écran interne n'est **pas**
//! relié au dGPU (`getDisplayConnectedToNvidia`). Ailleurs, la présence de
//! `/var/lib/ubuntu-drivers-common/requires_offloading` fait foi.
//!
//! # Bascule : volontairement absente de `tcc-hw`
//!
//! Ce crate ne dépend pas de `tccd` mais il est **lié dans** `tccd` : exposer
//! `prime-select <mode>` ici rendrait la bascule joignable depuis le daemon
//! principal, alors que D1 exige qu'elle reste dans `tccd-admin`, derrière
//! polkit. Le finding H-01 de `docs/security/REVIEW_HARDWARE.md` a conduit au
//! retrait de `set_mode`/`PRIME_SWITCH_TIMEOUT`.
//!
//! La bascule sera implémentée **directement dans `tccd-admin`** (allowlist
//! `prime-select`, arguments séparés, vérification polkit méthode par
//! méthode). Aucune dépendance partagée ne doit la réintroduire dans `tccd` ;
//! `tests/no_admin_reachability.rs` échoue si `tccd` la référence.
//!
//! `tccd` ne fait que lire l'état et ne redémarre ni X11 ni Wayland : le
//! changement prend effet à la prochaine ouverture de session, comme dans
//! l'original.

use std::path::Path;
use std::time::Duration;

use crate::command::{self, CommandOutput};
use crate::sysfs::SysfsRoot;

/// Fichier marqueur d'`ubuntu-drivers-common` (chemin absolu fixe).
pub const REQUIRES_OFFLOADING_PATH: &str = "/var/lib/ubuntu-drivers-common/requires_offloading";
/// Délai maximal pour `prime-select query`.
pub const PRIME_QUERY_TIMEOUT: Duration = Duration::from_secs(3);
/// Arguments de `dpkg-query`, figés (aucune interpolation).
const DPKG_QUERY_ARGS: [&str; 3] = ["-W", "-f=${Version}\n", "nvidia-prime"];
/// Vendor PCI NVIDIA.
const NVIDIA_VENDOR_ID: &str = "0x10de";

/// Mode PRIME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimeMode {
    /// Rendu sur l'iGPU (`intel`).
    Igpu,
    /// Rendu sur le dGPU (`nvidia`).
    Dgpu,
    /// Rendu à la demande (`on-demand`).
    OnDemand,
    /// PRIME présent mais mode inconnu.
    Off,
    /// Mode illisible.
    Unknown,
}

impl PrimeMode {
    /// Nom stable exposé à l'UI (`D-Bus GetPrimeState` de l'original).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Igpu => "iGPU",
            Self::Dgpu => "dGPU",
            Self::OnDemand => "on-demand",
            Self::Off => "off",
            Self::Unknown => "unknown",
        }
    }

    /// Convertit la sortie de `prime-select query`.
    #[must_use]
    pub fn from_query(value: &str) -> Self {
        match value.trim() {
            "intel" => Self::Igpu,
            "nvidia" => Self::Dgpu,
            "on-demand" => Self::OnDemand,
            "" => Self::Unknown,
            _ => Self::Off,
        }
    }
}

/// État PRIME consolidé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeState {
    /// `prime-select` trouvé à un chemin allowlisté.
    pub prime_select: Option<&'static str>,
    /// Le paquet `nvidia-prime` est la version TUXEDO.
    pub tux_prime: bool,
    /// PRIME est utilisable (outil TUXEDO présent).
    pub available: bool,
    /// La plateforme supporte la bascule (marqueur offloading / écran sur iGPU).
    pub platform_supported: bool,
    /// Mode courant, si lisible.
    pub mode: Option<PrimeMode>,
    /// Vrai si l'écran interne est relié au dGPU.
    pub internal_display_on_nvidia: Option<bool>,
    /// Marqueur `requires_offloading` présent.
    pub requires_offloading: bool,
}

impl PrimeState {
    /// État initial : PRIME indisponible.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            prime_select: None,
            tux_prime: false,
            available: false,
            platform_supported: false,
            mode: None,
            internal_display_on_nvidia: None,
            requires_offloading: false,
        }
    }

    /// Vrai si le mode peut être affiché et changé.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.available && self.platform_supported
    }
}

/// Détecte l'état PRIME.
///
/// Fonction best-effort : aucune erreur n'est remontée, un état
/// [`PrimeState::unavailable`] est retourné si l'outil ou les fichiers
/// manquent. C'est le comportement de `PrimeWorker` (l'UI affiche `-1`).
#[must_use]
pub fn detect(root: &SysfsRoot, device: crate::device::TuxedoDevice) -> PrimeState {
    let confining_display = device.prime_requires_internal_on_igpu();
    let internal_display_on_nvidia = if confining_display {
        Some(internal_display_on_nvidia(root))
    } else {
        None
    };

    let prime_select = command::resolve(&command::PRIME_SELECT);
    let requires_offloading = Path::new(REQUIRES_OFFLOADING_PATH).exists();
    let tux_prime = prime_select.is_some() && is_tux_prime();
    let available = prime_select.is_some() && tux_prime;
    let platform_supported = if confining_display {
        internal_display_on_nvidia == Some(false)
    } else {
        requires_offloading
    };
    let mode = available.then(|| query_mode(prime_select)).flatten();
    PrimeState {
        prime_select,
        tux_prime,
        available,
        platform_supported,
        mode,
        internal_display_on_nvidia,
        requires_offloading,
    }
}

/// Lit le mode courant via `prime-select query`.
#[must_use]
pub fn query_mode(prime_select: Option<&'static str>) -> Option<PrimeMode> {
    let program = prime_select?;
    match command::run(program, &["query"], PRIME_QUERY_TIMEOUT) {
        Ok(output) => output
            .first_line()
            .map(PrimeMode::from_query)
            .or(Some(PrimeMode::Unknown)),
        Err(error) => {
            tracing::debug!(%error, "prime-select query a échoué");
            None
        }
    }
}

/// Vrai si l'écran interne est relié au dGPU NVIDIA.
///
/// Parcourt les connecteurs `eDP` de `/sys/class/drm` et vérifie
/// `device/device/vendor == 0x10de` avec `status == connected`
/// (`PrimeWorker.getDisplayConnectedToNvidia`).
#[must_use]
pub fn internal_display_on_nvidia(root: &SysfsRoot) -> bool {
    let Ok(entries) = root.list_dir("class/drm") else {
        return false;
    };
    for entry in entries {
        if !entry.contains("eDP") {
            continue;
        }
        let base = format!("class/drm/{entry}");
        let vendor = root.read_optional_string(format!("{base}/device/device/vendor"));
        let status = root.read_optional_string(format!("{base}/status"));
        let (Ok(Some(vendor)), Ok(Some(status))) = (vendor, status) else {
            continue;
        };
        if vendor.trim().eq_ignore_ascii_case(NVIDIA_VENDOR_ID) && status.trim() == "connected" {
            return true;
        }
    }
    false
}

/// Vrai si le paquet `nvidia-prime` installé est celui de TUXEDO.
fn is_tux_prime() -> bool {
    let Some(program) = command::resolve(&command::DPKG_QUERY) else {
        return false;
    };
    match command::run(program, &DPKG_QUERY_ARGS, PRIME_QUERY_TIMEOUT) {
        Ok(CommandOutput { stdout, .. }) => stdout.to_ascii_lowercase().contains("tux"),
        Err(error) => {
            tracing::debug!(%error, "version de nvidia-prime illisible");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::TuxedoDevice;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn mode_parsing_matches_original() {
        assert_eq!(PrimeMode::from_query("nvidia\n"), PrimeMode::Dgpu);
        assert_eq!(PrimeMode::from_query("intel"), PrimeMode::Igpu);
        assert_eq!(PrimeMode::from_query("on-demand"), PrimeMode::OnDemand);
        assert_eq!(PrimeMode::from_query("other"), PrimeMode::Off);
        assert_eq!(PrimeMode::from_query(""), PrimeMode::Unknown);
        assert_eq!(PrimeMode::Dgpu.as_str(), "dGPU");
        assert_eq!(PrimeMode::Off.as_str(), "off");
    }

    #[test]
    fn internal_display_detection() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/drm/card0-eDP-1/status", "connected\n");
        write(
            dir.path(),
            "class/drm/card0-eDP-1/device/device/vendor",
            "0x10de\n",
        );
        let root = SysfsRoot::new(dir.path());
        assert!(internal_display_on_nvidia(&root));

        write(dir.path(), "class/drm/card0-eDP-1/status", "disconnected\n");
        assert!(!internal_display_on_nvidia(&root));

        write(dir.path(), "class/drm/card0-eDP-1/status", "connected\n");
        write(
            dir.path(),
            "class/drm/card0-eDP-1/device/device/vendor",
            "0x8086\n",
        );
        assert!(!internal_display_on_nvidia(&root));
    }

    #[test]
    fn absent_hardware_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let root = SysfsRoot::new(dir.path());
        let state = detect(&root, TuxedoDevice::Ibm15A10);
        // Sur une machine sans prime-select ni marqueur, tout est faux.
        assert!(!state.available);
        assert!(!state.is_supported());
        assert_eq!(state.internal_display_on_nvidia, Some(false));
        // Hors IBM15A10, la détection d'écran n'est pas faite.
        let state = detect(&root, TuxedoDevice::Ibm16A10);
        assert_eq!(state.internal_display_on_nvidia, None);
    }

    #[test]
    fn unavailable_state_shape() {
        let state = PrimeState::unavailable();
        assert!(!state.is_supported());
        assert_eq!(state.mode, None);
        assert_eq!(state.prime_select, None);
    }
}
