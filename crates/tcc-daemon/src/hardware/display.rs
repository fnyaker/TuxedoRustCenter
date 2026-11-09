// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Connecteur interne, lu depuis DRM.
//!
//! Le daemon expose uniquement ce que `/sys/class/drm` permet de savoir :
//! présence et état du connecteur interne (`eDP-*`, `LVDS-*`, `DSI-*`). Les
//! **modes**, le backend de session, le taux courant et l'application du
//! refresh restent fournis par `tccd-user` (agent de session). `tccd` ne doit
//! jamais inventer un backend ou un taux : la GUI doit interroger l'agent et
//! afficher « non supporté » s'il est absent.
//!
//! Ce worker est en lecture seule et sans effet sur la session graphique.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::display;
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture (les connecteurs changent au branchement/veille).
pub const DISPLAY_TICK: Duration = Duration::from_secs(30);

/// Informations DRM exposées à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisplayData {
    /// Un connecteur interne a été trouvé dans `/sys/class/drm`.
    pub available: bool,
    /// Capability explicite : un panneau interne est exposé (même déconnecté).
    pub has_internal_display: bool,
    /// Connecteur interne (`eDP-1`, `LVDS-1`, `DSI-1`).
    pub internal_connector: Option<String>,
    /// Panneau interne détecté comme connecté.
    pub connected: bool,
    /// Source de l'information (`drm`), `None` si aucun connecteur.
    pub source: Option<String>,
    /// Erreurs de lecture (bornées).
    pub errors: Vec<String>,
}

impl DisplayData {
    fn from_connector(connector: Option<display::DrmConnector>) -> Self {
        match connector {
            Some(connector) => Self {
                available: true,
                has_internal_display: true,
                internal_connector: Some(connector.name),
                connected: connector.connected,
                source: Some("drm".to_owned()),
                errors: Vec::new(),
            },
            None => Self::default(),
        }
    }
}

/// Runtime du connecteur interne.
#[derive(Debug)]
pub struct DisplayRuntime {
    root: SysfsRoot,
}

impl DisplayRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }

    /// Lit le connecteur interne.
    pub fn tick(&mut self, _input: &HardwareInputs) -> DisplayData {
        match display::read_internal_connector(&self.root) {
            Ok(connector) => DisplayData::from_connector(connector),
            Err(error) => {
                tracing::debug!(%error, "connecteur DRM illisible");
                DisplayData {
                    errors: vec![format!("DRM: {error}")],
                    ..DisplayData::default()
                }
            }
        }
    }
}

/// Démarre le worker d'affichage.
pub(crate) fn spawn(ctx: WorkerContext, root: SysfsRoot) -> tokio::task::JoinHandle<()> {
    let runtime = DisplayRuntime::new(root);
    spawn_worker(
        "display",
        DISPLAY_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_display,
        DisplayRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write_drm(root: &Path, entry: &str, status: &str) {
        let path = root.join("class/drm").join(entry);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("status"), format!("{status}\n")).unwrap();
    }

    #[test]
    fn exposes_connected_internal_connector() {
        let dir = tempfile::tempdir().unwrap();
        write_drm(dir.path(), "card0-eDP-1", "connected");
        let mut runtime = DisplayRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.available);
        assert!(data.has_internal_display);
        assert_eq!(data.internal_connector.as_deref(), Some("eDP-1"));
        assert!(data.connected);
        assert_eq!(data.source.as_deref(), Some("drm"));
        assert!(data.errors.is_empty());
    }

    #[test]
    fn disconnected_connector_is_still_exposed() {
        let dir = tempfile::tempdir().unwrap();
        write_drm(dir.path(), "card1-LVDS-1", "disconnected");
        let mut runtime = DisplayRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.available);
        assert_eq!(data.internal_connector.as_deref(), Some("LVDS-1"));
        assert!(!data.connected);
    }

    #[test]
    fn external_or_absent_drm_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        write_drm(dir.path(), "card0-HDMI-A-1", "connected");
        let mut runtime = DisplayRuntime::new(SysfsRoot::new(dir.path()));
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.available);
        assert!(!data.has_internal_display);
        assert_eq!(data.internal_connector, None);
        assert!(!data.connected);
        assert_eq!(data.source, None);
    }

    #[test]
    fn serializes_camel_case() {
        let data = DisplayData {
            available: true,
            has_internal_display: true,
            internal_connector: Some("eDP-1".to_owned()),
            connected: true,
            source: Some("drm".to_owned()),
            ..DisplayData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"hasInternalDisplay\":true"));
        assert!(json.contains("\"internalConnector\":\"eDP-1\""));
        assert!(json.contains("\"connected\":true"));
        assert!(json.contains("\"source\":\"drm\""));
    }
}
