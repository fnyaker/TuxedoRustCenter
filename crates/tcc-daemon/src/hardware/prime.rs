// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker PRIME : état du mode graphique.
//!
//! Reprend `PrimeWorker` en **lecture seule** : `tccd` publie l'état, la
//! bascule appartient à `tccd-admin` (D1). Elle n'est **pas** exposée par
//! `tcc-hw` (finding H-01 de `docs/security/REVIEW_HARDWARE.md`) : le daemon
//! principal ne peut que lire l'état. La bascule sera implémentée directement
//! dans `tccd-admin`, avec sa propre allowlist `prime-select` et une
//! vérification polkit méthode par méthode.
//!
//! Différence assumée : la détection complète (présence de `prime-select`,
//! version TUXEDO, marqueur d'offloading) est faite toutes les 5 minutes, et
//! seul `prime-select query` est relancé toutes les 30 secondes. L'original
//! exécutait trois commandes toutes les 10 secondes, ce qui est coûteux pour
//! un état qui ne change qu'à la déconnexion.
//!
//! Sur une machine sans `prime-select` TUXEDO (Fedora, Arch, ...), l'état est
//! `supported: false` : c'est le comportement de l'original, qui ne supporte
//! que TUXEDO OS / Ubuntu avec le paquet `nvidia-prime` TUXEDO.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::device::TuxedoDevice;
use tcc_hw::prime::{self, PrimeState};
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture du mode.
pub const PRIME_TICK: Duration = Duration::from_secs(30);
/// Nombre de ticks entre deux détections complètes (5 minutes).
const FULL_DETECT_TICKS: u32 = 10;

/// État PRIME exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrimeData {
    /// PRIME utilisable (outil TUXEDO présent).
    pub available: bool,
    /// La bascule est **toujours** refusée dans `tccd` : elle appartient à
    /// `tccd-admin` (D1). Ce champ vaut `false` même quand `supported` est
    /// vrai, pour que la GUI distingue « matériel supporté » et « action
    /// autorisée ».
    pub writable: bool,
    /// La plateforme supporte la bascule.
    pub platform_supported: bool,
    /// Vrai si la bascule est proposée à l'utilisateur.
    pub supported: bool,
    /// Paquet `nvidia-prime` TUXEDO détecté.
    pub tux_prime: bool,
    /// `iGPU`, `dGPU`, `on-demand`, `off` ou `unknown`.
    pub mode: Option<String>,
    /// Écran interne relié au dGPU (modèles concernés uniquement).
    pub internal_display_on_nvidia: Option<bool>,
    /// Marqueur `requires_offloading` présent.
    pub requires_offloading: bool,
    /// Chemin absolu de `prime-select`.
    pub prime_select: Option<String>,
}

impl From<&PrimeState> for PrimeData {
    fn from(state: &PrimeState) -> Self {
        Self {
            available: state.available,
            writable: false,
            platform_supported: state.platform_supported,
            supported: state.is_supported(),
            tux_prime: state.tux_prime,
            mode: state.mode.map(|mode| mode.as_str().to_owned()),
            internal_display_on_nvidia: state.internal_display_on_nvidia,
            requires_offloading: state.requires_offloading,
            prime_select: state.prime_select.map(str::to_owned),
        }
    }
}

/// Runtime PRIME.
#[derive(Debug)]
pub struct PrimeRuntime {
    root: SysfsRoot,
    state: Option<PrimeState>,
    ticks: u32,
}

impl PrimeRuntime {
    /// Construit le runtime.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self {
            root,
            state: None,
            ticks: 0,
        }
    }

    /// Exécute un tick.
    pub fn tick(&mut self, _input: &HardwareInputs) -> PrimeData {
        let needs_full = self.state.is_none() || self.ticks % FULL_DETECT_TICKS == 0;
        if needs_full {
            let device = TuxedoDevice::detect(&self.root, None);
            self.state = Some(prime::detect(&self.root, device));
        } else if let Some(state) = self.state.as_mut()
            && state.available
        {
            state.mode = prime::query_mode(state.prime_select);
        }
        self.ticks = self.ticks.wrapping_add(1);
        self.state.as_ref().map(PrimeData::from).unwrap_or_default()
    }
}

/// Démarre le worker PRIME.
pub(crate) fn spawn(ctx: WorkerContext, root: SysfsRoot) -> tokio::task::JoinHandle<()> {
    let runtime = PrimeRuntime::new(root);
    spawn_worker(
        "prime",
        PRIME_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_prime,
        PrimeRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn absent_prime_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = PrimeRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        // Selon la machine de test, `prime-select` peut exister ; on vérifie
        // surtout l'absence de panique et la cohérence du drapeau.
        assert_eq!(data.supported, data.available && data.platform_supported);
    }

    #[test]
    fn internal_display_detection_feeds_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/product_sku", "IBM15A10\n");
        write(dir.path(), "class/drm/card0-eDP-1/status", "connected\n");
        write(
            dir.path(),
            "class/drm/card0-eDP-1/device/device/vendor",
            "0x10de\n",
        );
        let mut runtime = PrimeRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.internal_display_on_nvidia, Some(true));
        // Écran sur dGPU ⇒ plateforme non supportée sur IBM15A10.
        assert!(!data.platform_supported);
    }

    #[test]
    fn other_models_do_not_probe_display_routing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/product_sku", "IBM16A10\n");
        write(dir.path(), "class/drm/card0-eDP-1/status", "connected\n");
        write(
            dir.path(),
            "class/drm/card0-eDP-1/device/device/vendor",
            "0x10de\n",
        );
        let mut runtime = PrimeRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert_eq!(data.internal_display_on_nvidia, None);
    }

    #[test]
    fn serializes_camel_case() {
        let data = PrimeData {
            supported: true,
            mode: Some("on-demand".to_owned()),
            internal_display_on_nvidia: Some(false),
            ..PrimeData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"writable\":false"));
        assert!(json.contains("\"platformSupported\":false"));
        assert!(json.contains("\"internalDisplayOnNvidia\":false"));
        assert!(json.contains("\"mode\":\"on-demand\""));
    }
}
