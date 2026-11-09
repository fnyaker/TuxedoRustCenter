// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker NVIDIA cTGP.
//!
//! Reprend `NVIDIAPowerCTRLListener` :
//!
//! - applique `nvidiaPowerCTRLProfile.cTGPOffset` du profil actif quand
//!   `ctgp_offset` **et** `nvidia-smi` sont disponibles ;
//! - publie l'offset courant et les limites de puissance NVIDIA pour l'UI.
//!
//! Différences assumées :
//!
//! - l'original écrivait l'offset à chaque apparition d'un événement sysfs et
//!   ne dédupliquait pas ; ici la valeur n'est écrite que si elle diffère de
//!   la valeur courante, et seulement au changement de révision ;
//! - les limites `nvidia-smi` sont relues toutes les 5 minutes au lieu d'être
//!   figées au démarrage (un changement de vBIOS ou de driver est ainsi pris en
//!   compte), tout en évitant de lancer deux processus toutes les 5 secondes.
//!
//! Aucune écriture de limite de puissance n'est effectuée : le levier matériel
//! reste `ctgp_offset`, comme dans l'original.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::error::HwError;
use tcc_hw::nvidia::{CtgpControl, MAX_CTGP_OFFSET_WATTS, NvidiaPowerLimits};
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de rafraîchissement.
pub const NVIDIA_TICK: Duration = Duration::from_secs(5);
/// Nombre de ticks entre deux lectures de `nvidia-smi` (5 minutes).
const LIMITS_REFRESH_TICKS: u32 = 60;

/// Instantané NVIDIA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NvidiaData {
    /// `ctgp_offset` **et** `nvidia-smi` disponibles.
    pub available: bool,
    /// TCC peut écrire l'offset cTGP (fonctionnalité complète, hors
    /// `dry_run`). `attributeAvailable` seul ne suffit pas.
    pub writable: bool,
    /// `ctgp_offset` exposé par le firmware.
    pub attribute_available: bool,
    /// `nvidia-smi` trouvé dans l'allowlist.
    pub nvidia_smi_available: bool,
    /// Offset courant en watts.
    pub ctgp_offset: Option<i32>,
    /// Limite de puissance par défaut (watts).
    pub default_power_limit: Option<f64>,
    /// Limite de puissance maximale (watts).
    pub max_power_limit: Option<f64>,
    /// Borne d'offset cTGP applicable, en watts.
    ///
    /// Dynamique (`maxPowerLimit - defaultPowerLimit`, bornée à 200 W) quand
    /// `nvidia-smi` expose les deux limites ; sinon borne statique de sécurité
    /// de 200 W. `maxCtgpOffsetSource` indique la provenance.
    pub max_ctgp_offset: i32,
    /// `dynamic` (limites NVIDIA) ou `static` (borne 200 W, pas matérielle).
    pub max_ctgp_offset_source: String,
    /// Offset appliqué par la dernière réconciliation.
    pub applied_offset: Option<i32>,
    pub errors: Vec<String>,
}

impl Default for NvidiaData {
    fn default() -> Self {
        Self {
            available: false,
            writable: false,
            attribute_available: false,
            nvidia_smi_available: false,
            ctgp_offset: None,
            default_power_limit: None,
            max_power_limit: None,
            max_ctgp_offset: MAX_CTGP_OFFSET_WATTS as i32,
            max_ctgp_offset_source: "static".to_owned(),
            applied_offset: None,
            errors: Vec::new(),
        }
    }
}

/// Runtime NVIDIA cTGP.
#[derive(Debug)]
pub struct NvidiaRuntime {
    control: CtgpControl,
    dry_run: bool,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
    limits: Option<NvidiaPowerLimits>,
    limits_ticks: u32,
    limits_error_logged: bool,
    ctgp_warned: Option<String>,
}

impl NvidiaRuntime {
    /// Construit le runtime.
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            control: CtgpControl::new(root),
            dry_run,
            last_revision: None,
            applied_revision: None,
            limits: None,
            limits_ticks: 0,
            limits_error_logged: false,
            ctgp_warned: None,
        }
    }

    /// Exécute un tick.
    pub fn tick(&mut self, input: &HardwareInputs) -> NvidiaData {
        let mut errors = Vec::new();
        let available = self.control.is_available();
        let mut data = NvidiaData {
            available,
            writable: available && !self.dry_run,
            attribute_available: self.control.attribute_available(),
            nvidia_smi_available: self.control.smi().is_available(),
            ctgp_offset: match self.control.offset() {
                Ok(offset) => offset,
                Err(error) => {
                    errors.push(format!("ctgp_offset: {error}"));
                    None
                }
            },
            ..NvidiaData::default()
        };

        if data.available
            && (self.limits.is_none() || self.limits_ticks % LIMITS_REFRESH_TICKS == 0)
        {
            match self.control.smi().power_limits() {
                Ok(limits) => {
                    self.limits = limits;
                    self.limits_error_logged = false;
                }
                Err(error) => {
                    if !self.limits_error_logged {
                        self.limits_error_logged = true;
                        tracing::debug!(%error, "limites NVIDIA indisponibles");
                    }
                }
            }
        }
        self.limits_ticks = self.limits_ticks.wrapping_add(1);
        if let Some(limits) = self.limits {
            data.default_power_limit = Some(limits.default_watts);
            data.max_power_limit = Some(limits.max_watts);
        }
        let (max_ctgp_offset, source) = resolve_max_ctgp_offset(self.limits);
        data.max_ctgp_offset = max_ctgp_offset;
        data.max_ctgp_offset_source = source.to_owned();

        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }
        let already_applied =
            self.applied_revision == self.last_revision && self.last_revision.is_some();
        if self.dry_run || already_applied {
            data.errors = errors;
            return data;
        }

        if let Some(profile) = &input.profile
            && data.available
        {
            let offset = profile.nvidia_power_ctrl_profile.ctgp_offset;
            match validate_ctgp_offset(offset, data.max_ctgp_offset) {
                Ok(()) => match self.control.set_offset_watts(offset) {
                    Ok(()) => {
                        self.ctgp_warned = None;
                        data.applied_offset = self.control.offset().ok().flatten();
                    }
                    Err(HwError::PolicyDenied { .. } | HwError::NotSupported(_)) => {}
                    Err(error) => {
                        errors.push(format!("ctgp_offset {offset}: {error}"));
                        self.warn_ctgp_once(
                            offset,
                            data.max_ctgp_offset,
                            data.max_ctgp_offset_source.as_str(),
                            &error.to_string(),
                        );
                    }
                },
                Err(message) => {
                    errors.push(message.clone());
                    self.warn_ctgp_once(
                        offset,
                        data.max_ctgp_offset,
                        data.max_ctgp_offset_source.as_str(),
                        &message,
                    );
                }
            }
        }
        data.errors = errors;
        if data.errors.is_empty() {
            self.applied_revision = self.last_revision;
        }
        data
    }

    /// Journalise un refus/échec cTGP une fois par (offset, borne).
    fn warn_ctgp_once(&mut self, offset: f64, max: i32, source: &str, reason: &str) {
        let key = format!("{offset}:{max}");
        if self.ctgp_warned.as_deref() == Some(key.as_str()) {
            return;
        }
        self.ctgp_warned = Some(key);
        tracing::warn!(
            offset,
            max,
            source,
            reason,
            "offset cTGP refusé ou non appliqué"
        );
    }
}

/// Calcule la borne cTGP réelle.
///
/// `dynamic` seulement si les deux limites NVIDIA sont disponibles et finies ;
/// sinon borne statique de sécurité 200 W, explicitement signalée `static`.
fn resolve_max_ctgp_offset(limits: Option<NvidiaPowerLimits>) -> (i32, &'static str) {
    match limits {
        Some(limits) if limits.default_watts.is_finite() && limits.max_watts.is_finite() => {
            let bound = (limits.max_watts - limits.default_watts)
                .floor()
                .clamp(0.0, MAX_CTGP_OFFSET_WATTS);
            (bound as i32, "dynamic")
        }
        _ => (MAX_CTGP_OFFSET_WATTS as i32, "static"),
    }
}

/// Valide un offset demandé avant toute écriture.
fn validate_ctgp_offset(offset: f64, max: i32) -> Result<(), String> {
    if !offset.is_finite() {
        return Err(format!("ctgp_offset {offset}: valeur non finie"));
    }
    let rounded = offset.round();
    if (offset - rounded).abs() > 0.001 {
        return Err(format!("ctgp_offset {offset}: valeur non entière"));
    }
    if !(0.0..=f64::from(max)).contains(&offset) {
        return Err(format!("ctgp_offset {offset}: hors borne 0..={max} W"));
    }
    Ok(())
}

/// Démarre le worker NVIDIA.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = NvidiaRuntime::new(root, dry_run);
    spawn_worker(
        "nvidia",
        NVIDIA_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_nvidia,
        NvidiaRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tcc_core::presets::office_profile;

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

    fn input(offset: f64, revision: u64) -> HardwareInputs {
        let mut profile = office_profile();
        profile.nvidia_power_ctrl_profile.ctgp_offset = offset;
        HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            revision,
        )
    }

    #[test]
    fn absent_attribute_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = NvidiaRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(10.0, 1));
        assert!(!data.available);
        assert!(!data.attribute_available);
        assert_eq!(data.ctgp_offset, None);
        assert!(data.applied_offset.is_none());
        assert!(data.errors.is_empty());
    }

    #[test]
    fn offset_is_applied_only_when_full_feature_is_available() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH, "0\n");
        let mut runtime = NvidiaRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(15.0, 1));
        assert!(data.attribute_available);
        // `nvidia-smi` est absent en CI : la fonctionnalité complète est
        // indisponible et rien n'est écrit (comportement original).
        if !data.nvidia_smi_available {
            assert!(!data.available);
            assert_eq!(read(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH), "0");
        } else {
            assert_eq!(read(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH), "15");
        }
    }

    #[test]
    fn dry_run_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH, "5\n");
        let mut runtime = NvidiaRuntime::new(SysfsRoot::new(dir.path()), true);
        let _ = runtime.tick(&input(20.0, 1));
        assert_eq!(read(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH), "5");
    }

    #[test]
    fn invalid_offset_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), tcc_hw::nvidia::CTGP_OFFSET_PATH, "0\n");
        let mut runtime = NvidiaRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(500.0, 1));
        if data.available {
            assert_eq!(data.errors.len(), 1);
        } else {
            // Sans nvidia-smi, l'offset n'est même pas tenté.
            assert!(data.errors.is_empty());
        }
    }

    #[test]
    fn ctgp_dynamic_bound_uses_limits() {
        let (max, source) = resolve_max_ctgp_offset(Some(NvidiaPowerLimits {
            default_watts: 150.0,
            max_watts: 300.0,
        }));
        assert_eq!(max, 150);
        assert_eq!(source, "dynamic");
        assert!(validate_ctgp_offset(150.0, max).is_ok());
        assert!(validate_ctgp_offset(151.0, max).is_err());
        assert!(validate_ctgp_offset(150.5, max).is_err());
        assert!(validate_ctgp_offset(-1.0, max).is_err());
    }

    #[test]
    fn tick_exposes_cached_dynamic_bound() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = NvidiaRuntime::new(SysfsRoot::new(dir.path()), false);
        runtime.limits = Some(NvidiaPowerLimits {
            default_watts: 150.0,
            max_watts: 300.0,
        });
        let data = runtime.tick(&input(0.0, 1));
        assert_eq!(data.max_ctgp_offset, 150);
        assert_eq!(data.max_ctgp_offset_source, "dynamic");
        assert_eq!(data.default_power_limit, Some(150.0));
        assert_eq!(data.max_power_limit, Some(300.0));
    }

    #[test]
    fn ctgp_bound_without_limits_is_static() {
        let (max, source) = resolve_max_ctgp_offset(None);
        assert_eq!(max, 200);
        assert_eq!(source, "static");
        assert!(validate_ctgp_offset(200.0, max).is_ok());
        assert!(validate_ctgp_offset(201.0, max).is_err());

        let default = NvidiaData::default();
        assert_eq!(default.max_ctgp_offset, 200);
        assert_eq!(default.max_ctgp_offset_source, "static");
    }

    #[test]
    fn ctgp_bound_is_clamped() {
        let below = NvidiaPowerLimits {
            default_watts: 100.0,
            max_watts: 90.0,
        };
        assert_eq!(resolve_max_ctgp_offset(Some(below)).0, 0);

        let above = NvidiaPowerLimits {
            default_watts: 0.0,
            max_watts: 1000.0,
        };
        assert_eq!(resolve_max_ctgp_offset(Some(above)).0, 200);
        assert_eq!(
            resolve_max_ctgp_offset(Some(NvidiaPowerLimits {
                default_watts: f64::NAN,
                max_watts: 100.0,
            }))
            .1,
            "static"
        );
    }

    #[test]
    fn serializes_camel_case() {
        let data = NvidiaData {
            available: true,
            writable: true,
            ctgp_offset: Some(25),
            default_power_limit: Some(80.0),
            max_power_limit: Some(300.0),
            max_ctgp_offset: 150,
            max_ctgp_offset_source: "dynamic".to_owned(),
            ..NvidiaData::default()
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"writable\":true"));
        assert!(json.contains("\"ctgpOffset\":25"));
        assert!(json.contains("\"defaultPowerLimit\":80.0"));
        assert!(json.contains("\"maxCtgpOffset\":150"));
        assert!(json.contains("\"maxCtgpOffsetSource\":\"dynamic\""));
    }
}
