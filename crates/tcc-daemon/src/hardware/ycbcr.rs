// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker YCbCr 4:2:0.
//!
//! Reprend `YCbCr420WorkaroundWorker` avec deux corrections :
//!
//! - l'écriture passe par `SysfsRoot` (validation, `O_NOFOLLOW`, erreur
//!   vérifiée) au lieu de `appendFileSync` ;
//! - l'état `pendingModeReapply` n'est levé que si une valeur a réellement
//!   changé.
//!
//! Les ports sont les noms de connecteur **sans** préfixe `<carte>-` (`DP-1`),
//! comme les clés de `settings.ycbcr420_workaround`. La conversion depuis un
//! connecteur DRM `card0-DP-1` se fait avec
//! [`tcc_hw::ycbcr::port_from_connector`].

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::error::HwError;
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::ycbcr::{self, YcbcrPort};

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de vérification (le forçage n'est appliqué qu'au changement de
/// révision ; le tick rafraîchit la disponibilité).
pub const YCBCR_TICK: Duration = Duration::from_secs(60);

/// Port debugfs exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YcbcrPortData {
    pub card: u32,
    pub port: String,
    pub forced: Option<bool>,
}

impl From<&YcbcrPort> for YcbcrPortData {
    fn from(port: &YcbcrPort) -> Self {
        Self {
            card: port.card,
            port: port.port.clone(),
            forced: port.forced,
        }
    }
}

/// Instantané YCbCr.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YcbcrData {
    /// Au moins un port est exposé (debugfs monté, ports compatibles).
    pub available: bool,
    /// Ports exposant `force_yuv420_output`.
    pub ports: Vec<YcbcrPortData>,
    /// Vrai si le mode d'affichage doit être réappliqué par le compositeur.
    pub pending_mode_reapply: bool,
    /// Vrai si la dernière application a réussi.
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Runtime YCbCr.
#[derive(Debug)]
pub struct YcbcrRuntime {
    root: SysfsRoot,
    dry_run: bool,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
}

impl YcbcrRuntime {
    /// Construit le runtime.
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            root,
            dry_run,
            last_revision: None,
            applied_revision: None,
        }
    }

    /// Exécute un tick.
    pub fn tick(&mut self, input: &HardwareInputs) -> YcbcrData {
        let mut errors = Vec::new();
        let ports = match ycbcr::enumerate(&self.root) {
            Ok(ports) => ports,
            Err(error) => {
                errors.push(format!("énumération: {error}"));
                Vec::new()
            }
        };
        let available = !ports.is_empty();

        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }
        let already_applied =
            self.applied_revision == self.last_revision && self.last_revision.is_some();
        // Le drapeau décrit une modification **de ce tick** : le daemon émet
        // l'événement `ModeReapplyPending` quand il passe à vrai. L'original
        // le laissait lever jusqu'à réapplication du mode par le compositeur.
        let mut changed_this_tick = false;
        if !already_applied && !self.dry_run {
            for (card, ports_settings) in input.settings.ycbcr420_workaround.iter().enumerate() {
                let Ok(card) = u32::try_from(card) else {
                    continue;
                };
                for (port, enabled) in ports_settings {
                    if !available_for(&ports, card, port) {
                        continue;
                    }
                    match ycbcr::apply(&self.root, card, port, *enabled) {
                        Ok(changed) => {
                            if changed {
                                changed_this_tick = true;
                                tracing::info!(
                                    card,
                                    port = %port,
                                    enabled = *enabled,
                                    "forçage YCbCr 4:2:0 mis à jour"
                                );
                            }
                        }
                        Err(HwError::NotSupported(_)) => {}
                        Err(error) => errors.push(format!("{card}/{port}: {error}")),
                    }
                }
            }
        }
        if !already_applied && errors.is_empty() {
            self.applied_revision = self.last_revision;
        }

        // Relit l'état réel après application.
        let ports_after = ycbcr::enumerate(&self.root).unwrap_or(ports);
        YcbcrData {
            available: available || !ports_after.is_empty(),
            ports: ports_after.iter().map(YcbcrPortData::from).collect(),
            pending_mode_reapply: changed_this_tick,
            applied: errors.is_empty(),
            errors,
        }
    }
}

fn available_for(ports: &[YcbcrPort], card: u32, port: &str) -> bool {
    ports
        .iter()
        .any(|candidate| candidate.card == card && candidate.port == port)
}

/// Démarre le worker YCbCr.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = YcbcrRuntime::new(root, dry_run);
    spawn_worker(
        "ycbcr",
        YCBCR_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_ycbcr,
        YcbcrRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn settings(port: &str, enabled: bool) -> tcc_core::Settings {
        let mut ports = BTreeMap::new();
        ports.insert(port.to_owned(), enabled);
        tcc_core::Settings {
            ycbcr420_workaround: vec![ports],
            ..tcc_core::Settings::default()
        }
    }

    #[test]
    fn applies_enabled_ports_once_per_revision() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "0\n",
        );
        let mut runtime = YcbcrRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings("DP-1", true), None, false, 1));
        assert!(data.available);
        assert!(data.applied, "{data:?}");
        assert!(data.pending_mode_reapply);
        assert_eq!(data.ports.len(), 1);
        assert_eq!(data.ports[0].forced, Some(true));
        assert_eq!(
            read(dir.path(), "kernel/debug/dri/0/DP-1/force_yuv420_output"),
            "1"
        );

        // Tick sans changement : pas de nouveau « pending ».
        let data = runtime.tick(&HardwareInputs::new(settings("DP-1", true), None, false, 1));
        assert!(!data.pending_mode_reapply);

        // Nouvelle révision : désactivation.
        let data = runtime.tick(&HardwareInputs::new(
            settings("DP-1", false),
            None,
            false,
            2,
        ));
        assert!(data.pending_mode_reapply);
        assert_eq!(
            read(dir.path(), "kernel/debug/dri/0/DP-1/force_yuv420_output"),
            "0"
        );
    }

    #[test]
    fn missing_debugfs_is_available_false() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = YcbcrRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings("DP-1", true), None, false, 1));
        assert!(!data.available);
        assert!(!data.pending_mode_reapply);
        assert!(data.applied);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn unknown_port_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "0\n",
        );
        let mut runtime = YcbcrRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(
            settings("HDMI-A-1", true),
            None,
            false,
            1,
        ));
        assert!(data.available);
        assert!(!data.pending_mode_reapply);
        assert!(data.applied);
    }

    #[test]
    fn dry_run_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "0\n",
        );
        let mut runtime = YcbcrRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&HardwareInputs::new(settings("DP-1", true), None, false, 1));
        assert!(data.applied);
        assert!(!data.pending_mode_reapply);
        assert_eq!(
            read(dir.path(), "kernel/debug/dri/0/DP-1/force_yuv420_output"),
            "0"
        );
    }

    #[test]
    fn disconnected_card_index_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut ports = BTreeMap::new();
        ports.insert("DP-1".to_owned(), true);
        // La carte 0 n'existe pas dans debugfs : l'entrée est ignorée.
        let settings = tcc_core::Settings {
            ycbcr420_workaround: vec![BTreeMap::new(), ports],
            ..tcc_core::Settings::default()
        };
        let mut runtime = YcbcrRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(!data.available);
        assert!(data.applied);
    }

    #[test]
    fn serializes_camel_case() {
        let data = YcbcrData {
            available: true,
            ports: vec![YcbcrPortData {
                card: 0,
                port: "DP-1".to_owned(),
                forced: Some(true),
            }],
            pending_mode_reapply: true,
            applied: true,
            errors: Vec::new(),
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"pendingModeReapply\":true"));
        assert!(json.contains("\"forced\":true"));
    }
}
