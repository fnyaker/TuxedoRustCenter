// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend Sway via IPC i3-ipc.
//!
//! - lecture : `GET_OUTPUTS` (type 3), réponse JSON ;
//! - application : `RUN_COMMAND` (type 0) avec
//!   `output <nom> mode <W>x<H>@<Hz>Hz`.
//!
//! Le mode courant, la position, l'échelle et la transformation sont conservés
//! par Sway : la commande ne modifie que le mode. Aucun shell, aucun `swaymsg`.

use std::path::PathBuf;

use serde::Deserialize;
use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    rates_match,
};
use tcc_hw::error::{HwError, Result as HwResult};

use crate::backend::{ensure_internal, format_hz};
use crate::error::{Result, UserError};
use crate::ipc::{self, sway_message};
use crate::security;
use crate::session::SessionInfo;

const TOLERANCE_HZ: f32 = 0.5;

#[derive(Debug, Clone, Copy, Deserialize)]
struct SwayMode {
    width: u32,
    height: u32,
    /// Millihertz dans l'IPC Sway.
    refresh: u32,
}

impl SwayMode {
    fn to_display(self) -> DisplayMode {
        DisplayMode {
            width: self.width,
            height: self.height,
            refresh_hz: self.refresh as f32 / 1000.0,
            preferred: false,
            current: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SwayOutput {
    name: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    power: Option<bool>,
    #[serde(default)]
    modes: Vec<SwayMode>,
    #[serde(default)]
    current_mode: Option<SwayMode>,
}

impl SwayOutput {
    fn enabled(&self) -> bool {
        self.active && self.power.unwrap_or(true)
    }

    fn to_output(&self) -> OutputInfo {
        let mut output = OutputInfo::new(&self.name);
        output.connected = true;
        output.enabled = self.enabled();
        let current = self.current_display();
        output.current_mode = current.clone();

        let mut modes: Vec<DisplayMode> = Vec::new();
        for mode in &self.modes {
            let mut display = mode.to_display();
            if let Some(current) = current.as_ref() {
                display.current = same_mode(&display, current);
            }
            if !modes.iter().any(|existing| same_mode(existing, &display)) {
                modes.push(display);
            }
        }
        if let Some(current) = current {
            if !modes.iter().any(|mode| same_mode(mode, &current)) {
                modes.push(current);
            }
        }
        output.modes = modes;
        output
    }

    fn current_display(&self) -> Option<DisplayMode> {
        if !self.enabled() {
            return None;
        }
        let current = self.current_mode?;
        let mut mode = current.to_display();
        mode.current = true;
        Some(mode)
    }

    fn mode_at_rate(&self, target: &DisplayMode) -> Option<DisplayMode> {
        let mut candidates: Vec<DisplayMode> = self.modes.iter().map(|m| m.to_display()).collect();
        if let Some(current) = self.current_display() {
            if !candidates.iter().any(|mode| same_mode(mode, &current)) {
                candidates.push(current);
            }
        }
        candidates
            .into_iter()
            .filter(|mode| mode.width == target.width && mode.height == target.height)
            .min_by(|left, right| {
                (left.refresh_hz - target.refresh_hz)
                    .abs()
                    .total_cmp(&(right.refresh_hz - target.refresh_hz).abs())
            })
            .filter(|mode| rates_match(mode.refresh_hz, target.refresh_hz, TOLERANCE_HZ))
    }
}

fn same_mode(left: &DisplayMode, right: &DisplayMode) -> bool {
    left.width == right.width
        && left.height == right.height
        && rates_match(left.refresh_hz, right.refresh_hz, 0.01)
}

#[derive(Debug, Deserialize)]
struct SwayReply {
    success: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Backend Sway.
#[derive(Debug)]
pub struct SwayBackend {
    socket: PathBuf,
    uid: u32,
}

impl SwayBackend {
    /// Construit le backend et valide le socket `$SWAYSOCK`.
    pub fn new(session: &SessionInfo, uid: u32) -> Result<Self> {
        let socket = session
            .env
            .sway_sock
            .as_deref()
            .ok_or_else(|| UserError::UnsupportedSession("SWAYSOCK absent".to_owned()))?;
        let socket = PathBuf::from(socket);
        security::validate_socket_path(&socket, uid)?;
        Ok(Self { socket, uid })
    }

    fn outputs(&self) -> Result<Vec<SwayOutput>> {
        let (_, body) = ipc::sway_request(&self.socket, self.uid, sway_message::GET_OUTPUTS, b"")?;
        serde_json::from_slice(&body).map_err(|err| UserError::Ipc {
            protocol: "sway",
            message: format!("JSON GET_OUTPUTS invalide : {err}"),
        })
    }

    fn run_command(&self, command: &str) -> Result<()> {
        let (_, body) = ipc::sway_request(
            &self.socket,
            self.uid,
            sway_message::RUN_COMMAND,
            command.as_bytes(),
        )?;
        let reply: SwayReply = serde_json::from_slice(&body).map_err(|err| UserError::Ipc {
            protocol: "sway",
            message: format!("JSON RUN_COMMAND invalide : {err}"),
        })?;
        if reply.success {
            Ok(())
        } else {
            Err(UserError::Ipc {
                protocol: "sway",
                message: reply.error.unwrap_or_else(|| "commande refusée".to_owned()),
            })
        }
    }
}

impl DisplayModeBackend for SwayBackend {
    fn backend_name(&self) -> &'static str {
        "sway"
    }

    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Sway
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        let outputs = self
            .outputs()
            .map_err(|err| HwError::Display(err.to_string()))?;
        Ok(outputs.iter().map(SwayOutput::to_output).collect())
    }

    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        ensure_internal(output)?;
        let outputs = self
            .outputs()
            .map_err(|err| HwError::Display(err.to_string()))?;
        let sway_output = outputs
            .iter()
            .find(|candidate| candidate.name == output.name())
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!("output {} introuvable", output.name()))
            })?;
        if !sway_output.enabled() {
            return Err(HwError::Display(format!(
                "écran interne {} inactif",
                output.name()
            )));
        }
        let selected = sway_output.mode_at_rate(mode).ok_or_else(|| {
            HwError::DisplayModeNotFound(format!(
                "{} Hz à {}x{} sur {}",
                mode.refresh_hz,
                mode.width,
                mode.height,
                output.name()
            ))
        })?;
        let command = format!(
            "output {} mode {}x{}@{}Hz",
            sway_output.name,
            selected.width,
            selected.height,
            format_hz(selected.refresh_hz)
        );
        self.run_command(&command)
            .map_err(|err| HwError::Display(err.to_string()))?;
        Ok(ApplyOutcome::Applied)
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::internal_output;

    const SAMPLE: &str = r#"
    [
      {
        "name": "eDP-1",
        "make": "Sharp Corporation",
        "model": "0x148B",
        "serial": "0x00000000",
        "active": true,
        "dpms": true,
        "power": true,
        "primary": false,
        "scale": 1.0,
        "transform": "normal",
        "current_workspace": "1",
        "modes": [
          {"width": 2560, "height": 1440, "refresh": 165000},
          {"width": 2560, "height": 1440, "refresh": 60000},
          {"width": 1920, "height": 1080, "refresh": 60000}
        ],
        "current_mode": {"width": 2560, "height": 1440, "refresh": 60000},
        "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
        "focused": true
      },
      {
        "name": "HDMI-A-2",
        "make": "Unknown",
        "model": "NS-19E310A13",
        "serial": "0x00000001",
        "active": true,
        "power": true,
        "primary": false,
        "scale": 1.0,
        "transform": "normal",
        "current_workspace": "2",
        "modes": [
          {"width": 1920, "height": 1080, "refresh": 60000}
        ],
        "current_mode": {"width": 1920, "height": 1080, "refresh": 60000}
      }
    ]
    "#;

    fn outputs() -> Vec<SwayOutput> {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn parses_realistic_get_outputs() {
        let outputs = outputs();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "eDP-1");
        assert_eq!(outputs[0].current_display().unwrap().refresh_hz, 60.0);
        assert_eq!(outputs[0].modes.len(), 3);
    }

    #[test]
    fn selects_internal_output_only() {
        let outputs: Vec<OutputInfo> = outputs().iter().map(SwayOutput::to_output).collect();
        let internal = internal_output(&outputs).unwrap();
        assert_eq!(internal.name(), "eDP-1");
        assert_eq!(internal.modes_at_current_resolution().len(), 2);
    }

    #[test]
    fn mode_matching_uses_millihz() {
        let output = &outputs()[0];
        let selected = output
            .mode_at_rate(&DisplayMode::new(2560, 1440, 165.0))
            .unwrap();
        assert_eq!(selected.width, 2560);
        assert_eq!(selected.height, 1440);
        assert!((selected.refresh_hz - 165.0).abs() < 0.001);
        assert!(
            output
                .mode_at_rate(&DisplayMode::new(2560, 1440, 144.0))
                .is_none()
        );
    }

    #[test]
    fn command_uses_selected_mode() {
        let output = &outputs()[0];
        let selected = output
            .mode_at_rate(&DisplayMode::new(2560, 1440, 165.0))
            .unwrap();
        let command = format!(
            "output {} mode {}x{}@{}Hz",
            output.name,
            selected.width,
            selected.height,
            format_hz(selected.refresh_hz)
        );
        assert_eq!(command, "output eDP-1 mode 2560x1440@165Hz");
    }

    #[test]
    fn disabled_output_has_no_current_mode() {
        let json =
            r#"[{"name":"eDP-1","active":false,"power":false,"modes":[],"current_mode":null}]"#;
        let outputs: Vec<SwayOutput> = serde_json::from_str(json).unwrap();
        let output = outputs[0].to_output();
        assert!(!output.enabled);
        assert!(output.current_mode.is_none());
    }
}
