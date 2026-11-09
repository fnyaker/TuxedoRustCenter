// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend Hyprland via socket IPC natif.
//!
//! - lecture : commande `j/monitors` (JSON) ;
//! - application : `keyword monitor <nom>,<W>x<H>@<Hz>,<X>x<Y>,<scale>,transform,<t>`
//!   en préservant position, scale et transform du moniteur courant.
//!
//! Aucun appel à `hyprctl`, aucun shell. Le socket est
//! `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` et ses
//! permissions/credentials sont vérifiées par `security`.

use std::path::PathBuf;

use serde::Deserialize;
use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    rates_match,
};
use tcc_hw::error::{HwError, Result as HwResult};

use crate::backend::{ensure_internal, format_hz, validate_internal_output_name};
use crate::error::{Result, UserError};
use crate::ipc;
use crate::security;
use crate::session::SessionInfo;

const TOLERANCE_HZ: f32 = 0.5;

/// Moniteur Hyprland tel que retourné par `j/monitors`.
#[derive(Debug, Clone, Deserialize)]
struct HyprMonitor {
    name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(rename = "refreshRate", default)]
    refresh_rate: f32,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default = "default_scale")]
    scale: f32,
    #[serde(default)]
    transform: u8,
    #[serde(default)]
    disabled: bool,
    #[serde(rename = "availableModes", default)]
    available_modes: Vec<String>,
}

fn default_scale() -> f32 {
    1.0
}

impl HyprMonitor {
    /// Convertit en type neutre `tcc-hw`.
    fn to_output(&self) -> OutputInfo {
        let mut output = OutputInfo::new(&self.name);
        // Hyprland ne liste que les moniteurs connus/connectés.
        output.connected = true;
        output.enabled = !self.disabled;
        let current = self.current_mode();
        output.current_mode = current.clone();
        let mut modes = self.parsed_modes();
        if let Some(current_mode) = current {
            if !modes.iter().any(|mode| same_mode(mode, &current_mode)) {
                modes.push(current_mode);
            }
        }
        output.modes = modes;
        output
    }

    fn current_mode(&self) -> Option<DisplayMode> {
        if self.disabled || self.width == 0 || self.height == 0 || self.refresh_rate <= 0.0 {
            return None;
        }
        Some(DisplayMode {
            width: self.width,
            height: self.height,
            refresh_hz: self.refresh_rate,
            preferred: false,
            current: true,
        })
    }

    fn parsed_modes(&self) -> Vec<DisplayMode> {
        let mut modes: Vec<DisplayMode> = Vec::new();
        for raw in &self.available_modes {
            let Some(mut mode) = parse_hyprland_mode(raw) else {
                continue;
            };
            if let Some(current) = self.current_mode() {
                mode.current = mode.width == current.width
                    && mode.height == current.height
                    && rates_match(mode.refresh_hz, current.refresh_hz, 0.01);
            }
            if !modes.iter().any(|existing| same_mode(existing, &mode)) {
                modes.push(mode);
            }
        }
        modes
    }

    fn mode_at_rate(&self, target: &DisplayMode) -> Option<DisplayMode> {
        self.parsed_modes()
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

/// Parse `2560x1440@165.00Hz`.
fn parse_hyprland_mode(raw: &str) -> Option<DisplayMode> {
    let (resolution, rate) = raw.split_once('@')?;
    let (width, height) = resolution.split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    let rate = rate
        .trim()
        .trim_end_matches("Hz")
        .trim_end_matches("hz")
        .trim()
        .parse::<f32>()
        .ok()?;
    if width == 0 || height == 0 || !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    Some(DisplayMode {
        width,
        height,
        refresh_hz: rate,
        preferred: false,
        current: false,
    })
}

/// Backend Hyprland.
#[derive(Debug)]
pub struct HyprlandBackend {
    socket: PathBuf,
    uid: u32,
}

impl HyprlandBackend {
    /// Construit le backend depuis la session détectée.
    pub fn new(session: &SessionInfo, uid: u32) -> Result<Self> {
        let runtime_dir =
            session.env.xdg_runtime_dir.as_deref().ok_or_else(|| {
                UserError::UnsupportedSession("XDG_RUNTIME_DIR absent".to_owned())
            })?;
        let runtime_dir = PathBuf::from(runtime_dir);
        security::validate_runtime_dir(&runtime_dir, uid)?;

        let signature = session
            .env
            .hyprland_instance_signature
            .as_deref()
            .ok_or_else(|| {
                UserError::UnsupportedSession("HYPRLAND_INSTANCE_SIGNATURE absent".to_owned())
            })?;
        if !valid_signature(signature) {
            return Err(UserError::InvalidSocket(format!(
                "signature d'instance Hyprland invalide : {signature}"
            )));
        }
        let socket = runtime_dir
            .join("hypr")
            .join(signature)
            .join(".socket.sock");
        Ok(Self { socket, uid })
    }

    fn request(&self, command: &str) -> Result<String> {
        ipc::hyprland_request(&self.socket, self.uid, command)
    }

    fn monitors(&self) -> Result<Vec<HyprMonitor>> {
        let response = self.request("j/monitors")?;
        serde_json::from_str(&response).map_err(|err| UserError::Ipc {
            protocol: "hyprland",
            message: format!("JSON monitors invalide : {err}"),
        })
    }
}

fn valid_signature(signature: &str) -> bool {
    !signature.is_empty()
        && signature.len() <= 128
        && signature != "."
        && signature != ".."
        && signature
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl DisplayModeBackend for HyprlandBackend {
    fn backend_name(&self) -> &'static str {
        "hyprland"
    }

    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Hyprland
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        let monitors = self
            .monitors()
            .map_err(|err| HwError::Display(err.to_string()))?;
        Ok(monitors.iter().map(HyprMonitor::to_output).collect())
    }

    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        ensure_internal(output)?;
        let monitors = self
            .monitors()
            .map_err(|err| HwError::Display(err.to_string()))?;
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.name == output.name())
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!("moniteur {} introuvable", output.name()))
            })?;
        if monitor.disabled {
            return Err(HwError::Display(format!(
                "écran interne {} désactivé",
                output.name()
            )));
        }
        let selected = monitor.mode_at_rate(mode).ok_or_else(|| {
            HwError::DisplayModeNotFound(format!(
                "{} Hz à {}x{} sur {}",
                mode.refresh_hz,
                mode.width,
                mode.height,
                output.name()
            ))
        })?;
        let command = monitor_command(monitor, &selected)?;
        let response = self
            .request(&command)
            .map_err(|err| HwError::Display(err.to_string()))?;
        let response = response.trim();
        if response.eq_ignore_ascii_case("ok") {
            Ok(ApplyOutcome::Applied)
        } else {
            Err(HwError::Display(format!(
                "Hyprland a refusé la commande : {response}"
            )))
        }
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Construit la commande Hyprland en préservant position/scale/transform.
fn monitor_command(monitor: &HyprMonitor, mode: &DisplayMode) -> HwResult<String> {
    if !validate_internal_output_name(&monitor.name) {
        return Err(HwError::NotInternalOutput(monitor.name.clone()));
    }
    if monitor.transform > 7 {
        return Err(HwError::Display(format!(
            "transform Hyprland invalide : {}",
            monitor.transform
        )));
    }
    let scale = if monitor.scale.is_finite() && monitor.scale > 0.0 {
        monitor.scale
    } else {
        1.0
    };
    Ok(format!(
        "keyword monitor {},{w}x{h}@{rate},{x}x{y},{scale},transform,{transform}",
        monitor.name,
        w = mode.width,
        h = mode.height,
        rate = format_hz(mode.refresh_hz),
        x = monitor.x,
        y = monitor.y,
        scale = scale,
        transform = monitor.transform,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::internal_output;

    const SAMPLE: &str = r#"
    [
      {
        "id": 0,
        "name": "eDP-1",
        "description": "TUXEDO 2560x1440",
        "make": "TUXEDO",
        "model": "Panel",
        "serial": "0x00000000",
        "width": 2560,
        "height": 1440,
        "refreshRate": 60.00,
        "x": 0,
        "y": 0,
        "activeWorkspace": {"id": 1, "name": "1"},
        "specialWorkspace": {"id": 0, "name": ""},
        "reserved": [0, 0, 0, 0],
        "scale": 1.25,
        "transform": 1,
        "focused": true,
        "dpmsStatus": true,
        "vrr": false,
        "solitary": "0",
        "activelyTearing": false,
        "disabled": false,
        "currentFormat": "XRGB8888",
        "mirrorOf": "none",
        "availableModes": [
          "2560x1440@165.00Hz",
          "2560x1440@60.00Hz",
          "1920x1080@60.00Hz"
        ]
      },
      {
        "id": 1,
        "name": "HDMI-A-1",
        "description": "External",
        "make": "Dell",
        "model": "U2720Q",
        "serial": "0x12345678",
        "width": 3840,
        "height": 2160,
        "refreshRate": 60.00,
        "x": 2560,
        "y": 0,
        "scale": 1.0,
        "transform": 0,
        "disabled": false,
        "availableModes": ["3840x2160@60.00Hz"]
      }
    ]
    "#;

    fn monitors() -> Vec<HyprMonitor> {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn parses_realistic_monitors_json() {
        let monitors = monitors();
        assert_eq!(monitors.len(), 2);
        let internal = internal_output(
            &monitors
                .iter()
                .map(HyprMonitor::to_output)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(internal.name(), "eDP-1");
        assert_eq!(internal.current_refresh_hz(), Some(60.0));
        assert_eq!(internal.modes_at_current_resolution().len(), 2);
        assert_eq!(internal.modes_at_current_resolution()[1].refresh_hz, 165.0);
    }

    #[test]
    fn external_output_is_not_selected() {
        let outputs: Vec<OutputInfo> = monitors().iter().map(HyprMonitor::to_output).collect();
        assert_eq!(outputs.len(), 2);
        assert!(outputs[1].name == "HDMI-A-1");
        assert!(!outputs[1].internal);
    }

    #[test]
    fn mode_parser_handles_hz_suffix() {
        let mode = parse_hyprland_mode("2560x1440@59.94Hz").unwrap();
        assert_eq!(mode.width, 2560);
        assert_eq!(mode.height, 1440);
        assert!((mode.refresh_hz - 59.94).abs() < 0.001);
        assert!(parse_hyprland_mode("bad").is_none());
        assert!(parse_hyprland_mode("0x0@60Hz").is_none());
    }

    #[test]
    fn command_preserves_position_scale_transform() {
        let monitor = &monitors()[0];
        let selected = monitor
            .mode_at_rate(&DisplayMode::new(2560, 1440, 165.0))
            .unwrap();
        let command = monitor_command(monitor, &selected).unwrap();
        assert_eq!(
            command,
            "keyword monitor eDP-1,2560x1440@165,0x0,1.25,transform,1"
        );
    }

    #[test]
    fn command_rejects_external_monitor() {
        let monitor = &monitors()[1];
        let mode = DisplayMode::new(3840, 2160, 60.0);
        assert!(matches!(
            monitor_command(monitor, &mode),
            Err(HwError::NotInternalOutput(_))
        ));
    }

    #[test]
    fn signature_validation() {
        assert!(valid_signature("abc123"));
        assert!(valid_signature("0x1234abcd"));
        assert!(!valid_signature(".."));
        assert!(!valid_signature("../x"));
        assert!(!valid_signature("a/b"));
        assert!(!valid_signature(""));
    }

    #[test]
    fn disabled_monitor_has_no_current_mode() {
        let json = r#"[{"name":"eDP-1","disabled":true,"availableModes":[]}]"#;
        let monitors: Vec<HyprMonitor> = serde_json::from_str(json).unwrap();
        let output = monitors[0].to_output();
        assert!(!output.enabled);
        assert!(output.current_mode.is_none());
    }
}
