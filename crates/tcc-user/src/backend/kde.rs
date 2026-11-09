// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend KDE Plasma via `kscreen-doctor`.
//!
//! KScreen n'expose pas toujours un service D-Bus stable (`org.kde.KScreen`
//! est absent sur certaines installations). Le backend utilise donc l'outil
//! officiel `kscreen-doctor` en mode JSON :
//!
//! - lecture : `kscreen-doctor -j` ;
//! - application : `kscreen-doctor output.<name>.mode.<id>`.
//!
//! Aucun shell n'est utilisé : `Command::new` avec un argument unique validé.
//! L'écran interne est filtré par `tcc-hw` ; un écran externe est refusé avant
//! toute commande.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    rates_match,
};
use tcc_hw::error::{HwError, Result as HwResult};

use crate::backend::{ensure_internal, format_hz, validate_internal_output_name};
use crate::error::{Result, UserError};

const KSCREEN_DOCTOR: &str = "/usr/bin/kscreen-doctor";
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const TOLERANCE_HZ: f32 = 0.5;
/// Délai maximal d'un appel `kscreen-doctor` (lecture ou application).
const KSCREEN_TIMEOUT: Duration = Duration::from_secs(3);
/// Intervalle de vérification de l'enfant.
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Deserialize)]
struct KdeState {
    #[serde(default)]
    outputs: Vec<KdeOutput>,
}

#[derive(Debug, Deserialize)]
struct KdeOutput {
    name: String,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    enabled: bool,
    #[serde(rename = "currentModeId", default)]
    current_mode_id: Option<String>,
    #[serde(default)]
    modes: Vec<KdeMode>,
    #[serde(rename = "preferredModes", default)]
    preferred_modes: Vec<String>,
    #[serde(default)]
    size: KdeSize,
}

#[derive(Debug, Deserialize)]
struct KdeMode {
    id: String,
    #[serde(rename = "refreshRate", default)]
    refresh_rate: f32,
    #[serde(default)]
    size: KdeSize,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct KdeSize {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

impl KdeOutput {
    fn to_output(&self) -> OutputInfo {
        let current_id = self.current_mode_id.as_deref();
        let mut modes: Vec<DisplayMode> = self
            .modes
            .iter()
            .filter_map(|mode| {
                if mode.size.width == 0
                    || mode.size.height == 0
                    || !mode.refresh_rate.is_finite()
                    || mode.refresh_rate <= 0.0
                {
                    return None;
                }
                Some(DisplayMode {
                    width: mode.size.width,
                    height: mode.size.height,
                    refresh_hz: mode.refresh_rate,
                    preferred: self.preferred_modes.iter().any(|id| id == &mode.id),
                    current: current_id.is_some_and(|id| id == mode.id),
                })
            })
            .collect();

        // Si le mode courant n'est pas marqué, tenter de le retrouver par la
        // géométrie courante.
        if !modes.iter().any(|mode| mode.current) && self.size.width > 0 && self.size.height > 0 {
            if let Some(mode) = modes
                .iter_mut()
                .find(|mode| mode.width == self.size.width && mode.height == self.size.height)
            {
                mode.current = true;
            }
        }

        let current = modes.iter().find(|mode| mode.current).cloned();
        let mut output = OutputInfo::new(&self.name);
        output.connected = self.connected;
        output.enabled = self.enabled;
        output.current_mode = current;
        output.modes = modes;
        output
    }

    fn mode_for(&self, target: &DisplayMode) -> Option<&KdeMode> {
        self.modes.iter().find(|mode| {
            mode.size.width == target.width
                && mode.size.height == target.height
                && rates_match(mode.refresh_rate, target.refresh_hz, TOLERANCE_HZ)
        })
    }
}

/// Backend KDE Plasma.
#[derive(Debug)]
pub struct KdeBackend {
    binary: &'static str,
}

impl KdeBackend {
    /// Construit le backend si `kscreen-doctor` est disponible.
    pub fn new() -> Result<Self> {
        if !Path::new(KSCREEN_DOCTOR).is_file() {
            return Err(UserError::BackendUnavailable {
                backend: "kde",
                reason: format!("{KSCREEN_DOCTOR} absent"),
            });
        }
        Ok(Self {
            binary: KSCREEN_DOCTOR,
        })
    }

    fn read_state(&self) -> HwResult<KdeState> {
        let output = run_command_with_timeout(Path::new(self.binary), &["-j"], KSCREEN_TIMEOUT)?;
        if !output.status.success() {
            return Err(HwError::Display(format!(
                "kscreen-doctor -j a échoué: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        if output.stdout.len() > MAX_OUTPUT_BYTES {
            return Err(HwError::Display(
                "sortie kscreen-doctor trop volumineuse".to_owned(),
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| HwError::Display(format!("JSON kscreen-doctor invalide: {error}")))
    }
}

impl DisplayModeBackend for KdeBackend {
    fn backend_name(&self) -> &'static str {
        "kde"
    }

    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Kde
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        Ok(self
            .read_state()?
            .outputs
            .iter()
            .map(KdeOutput::to_output)
            .collect())
    }

    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        ensure_internal(output)?;
        if !validate_internal_output_name(output.name()) {
            return Err(HwError::NotInternalOutput(output.name().to_owned()));
        }

        let state = self.read_state()?;
        let kde_output = state
            .outputs
            .iter()
            .find(|candidate| candidate.name == output.name())
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!("output {} introuvable", output.name()))
            })?;
        if !kde_output.enabled {
            return Err(HwError::Display(format!(
                "écran interne {} désactivé",
                output.name()
            )));
        }
        let selected = kde_output.mode_for(mode).ok_or_else(|| {
            HwError::DisplayModeNotFound(format!(
                "{} Hz à {}x{} sur {}",
                format_hz(mode.refresh_hz),
                mode.width,
                mode.height,
                output.name()
            ))
        })?;
        if selected.id.is_empty()
            || !selected
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(HwError::Display(format!(
                "identifiant de mode KDE invalide: {}",
                selected.id
            )));
        }

        let argument = format!("output.{}.mode.{}", output.name(), selected.id);
        let result =
            run_command_with_timeout(Path::new(self.binary), &[&argument], KSCREEN_TIMEOUT)?;
        if result.status.success() {
            Ok(ApplyOutcome::Applied)
        } else {
            Err(HwError::Display(format!(
                "kscreen-doctor a refusé {argument}: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            )))
        }
    }

    fn is_available(&self) -> bool {
        Path::new(self.binary).is_file()
    }
}

/// Exécute une commande allowlistée avec un délai maximal.
///
/// Le processus enfant est tué en cas de dépassement pour ne jamais immobiliser
/// un thread de `spawn_blocking` indéfiniment (R-11). L'environnement de
/// session est conservé (Wayland/X11, `QT_*`, `DBUS_SESSION_BUS_ADDRESS`), mais
/// les variables de préchargement du loader sont retirées.
fn run_command_with_timeout(program: &Path, args: &[&str], timeout: Duration) -> HwResult<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("LD_PRELOAD")
        .env_remove("LD_AUDIT")
        .spawn()
        .map_err(|error| {
            HwError::Display(format!("lancement de {} : {error}", program.display()))
        })?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child.wait_with_output().map_err(|error| {
                    HwError::Display(format!(
                        "lecture de la sortie de {} : {error}",
                        program.display()
                    ))
                });
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(CHILD_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HwError::Display(format!(
                    "{} n'a pas répondu en {} s ; processus tué",
                    program.display(),
                    timeout.as_secs_f32()
                )));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HwError::Display(format!(
                    "attente de {} : {error}",
                    program.display()
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::internal_output;

    #[test]
    fn command_output_is_captured() {
        let program = Path::new("/bin/echo");
        if !program.is_file() {
            return;
        }
        let output =
            run_command_with_timeout(program, &["kscreen-ok"], Duration::from_secs(1)).unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("kscreen-ok"));
    }

    #[test]
    fn command_timeout_kills_child() {
        let program = Path::new("/bin/sleep");
        if !program.is_file() {
            return;
        }
        let started = Instant::now();
        let error =
            run_command_with_timeout(program, &["5"], Duration::from_millis(100)).unwrap_err();
        assert!(error.to_string().contains("n'a pas répondu"));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "le timeout n'a pas tué l'enfant assez vite : {:?}",
            started.elapsed()
        );
    }

    const SAMPLE: &str = r#"
    {
      "outputs": [
        {
          "name": "eDP-1",
          "connected": true,
          "enabled": true,
          "currentModeId": "1",
          "preferredModes": ["1", "2"],
          "size": {"width": 2560, "height": 1600},
          "modes": [
            {"id": "1", "name": "2560x1600@240", "refreshRate": 240.0, "size": {"width": 2560, "height": 1600}},
            {"id": "2", "name": "2560x1600@60", "refreshRate": 60.0, "size": {"width": 2560, "height": 1600}},
            {"id": "21", "name": "1920x1080@60", "refreshRate": 59.96, "size": {"width": 1920, "height": 1080}}
          ]
        },
        {
          "name": "HDMI-A-1",
          "connected": true,
          "enabled": true,
          "currentModeId": "1",
          "modes": [
            {"id": "1", "name": "1920x1080@60", "refreshRate": 60.0, "size": {"width": 1920, "height": 1080}}
          ]
        }
      ]
    }
    "#;

    fn state() -> KdeState {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn parses_kscreen_json() {
        let outputs: Vec<OutputInfo> = state().outputs.iter().map(KdeOutput::to_output).collect();
        assert_eq!(outputs.len(), 2);
        let internal = internal_output(&outputs).unwrap();
        assert_eq!(internal.name(), "eDP-1");
        assert_eq!(internal.current_refresh_hz(), Some(240.0));
        assert_eq!(internal.modes_at_current_resolution().len(), 2);
    }

    #[test]
    fn selects_mode_by_rate() {
        let output = &state().outputs[0];
        let selected = output
            .mode_for(&DisplayMode::new(2560, 1600, 60.0))
            .unwrap();
        assert_eq!(selected.id, "2");
        assert!(
            output
                .mode_for(&DisplayMode::new(2560, 1600, 144.0))
                .is_none()
        );
    }

    #[test]
    fn external_output_is_not_selected() {
        let outputs: Vec<OutputInfo> = state().outputs.iter().map(KdeOutput::to_output).collect();
        assert_eq!(
            internal_output(&outputs).map(|o| o.name().to_owned()),
            Some("eDP-1".to_owned())
        );
    }

    #[test]
    fn mode_id_is_validated() {
        let mut output = state().outputs.remove(0);
        output.modes[0].id = "1;rm -rf /".to_owned();
        assert!(
            output
                .mode_for(&DisplayMode::new(2560, 1600, 240.0))
                .is_some()
        );
        // La validation a lieu dans apply_refresh_rate, pas dans la recherche.
        assert!(
            !output.modes[0]
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
    }
}
