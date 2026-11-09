// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend X11 via XRandR natif (`x11rb`, 100 % Rust).
//!
//! Aucun appel à `xrandr`, aucun scraping de `/proc`. L'agent tourne dans la
//! session et utilise `DISPLAY`/`XAUTHORITY` de son propre environnement.
//!
//! Sécurité :
//! - seuls les outputs dont le nom commence par `eDP`/`LVDS`/`DSI` sont
//!   applicables ;
//! - si le CRTC de l'écran interne est partagé (miroir) avec un autre output,
//!   l'application est refusée plutôt que de modifier un écran externe ;
//! - la résolution, la position et la rotation courantes sont conservées.

use std::collections::{HashMap, HashSet};

use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
    rates_match,
};
use tcc_hw::error::{HwError, Result as HwResult};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::backend::ensure_internal;
use crate::error::{Result, UserError};

/// Tolérance utilisée pour retrouver un mode XRandR.
const TOLERANCE_HZ: f32 = 0.5;

/// Sortie X11 mise en cache pour l'application.
#[derive(Debug, Clone)]
struct CachedOutput {
    output_id: randr::Output,
    crtc: randr::Crtc,
    crtc_outputs: Vec<randr::Output>,
    x: i16,
    y: i16,
    rotation: randr::Rotation,
    enabled: bool,
    mode_ids: Vec<(randr::Mode, DisplayMode)>,
}

/// Cache rafraîchi à chaque `list_outputs`.
#[derive(Debug, Clone, Default)]
struct X11Cache {
    config_timestamp: u32,
    outputs: HashMap<String, CachedOutput>,
}

/// Backend X11/XRandR.
#[derive(Debug)]
pub struct X11Backend {
    conn: RustConnection,
    root: x11rb::protocol::xproto::Window,
    cache: X11Cache,
}

impl X11Backend {
    /// Ouvre la connexion X11 et vérifie la disponibilité de RandR.
    pub fn connect() -> Result<Self> {
        let (conn, screen_num) =
            x11rb::connect(None).map_err(|err| UserError::BackendUnavailable {
                backend: "x11",
                reason: format!("connexion X11 : {err}"),
            })?;
        let screen =
            conn.setup()
                .roots
                .get(screen_num)
                .ok_or_else(|| UserError::BackendUnavailable {
                    backend: "x11",
                    reason: format!("écran X11 {screen_num} introuvable"),
                })?;
        let root = screen.root;
        let backend = Self {
            conn,
            root,
            cache: X11Cache::default(),
        };
        let cookie = backend
            .conn
            .randr_query_version(1, 6)
            .map_err(|err| x11_err("RandR QueryVersion", err))?;
        let version = cookie
            .reply()
            .map_err(|err| x11_err("RandR QueryVersion reply", err))?;
        if version.major_version < 1 || (version.major_version == 1 && version.minor_version < 2) {
            return Err(UserError::BackendUnavailable {
                backend: "x11",
                reason: format!(
                    "RandR {}.{} trop ancien (1.2 minimum)",
                    version.major_version, version.minor_version
                ),
            });
        }
        Ok(backend)
    }

    fn list(&mut self) -> HwResult<Vec<OutputInfo>> {
        let resources = self
            .conn
            .randr_get_screen_resources_current(self.root)
            .map_err(|err| x11_err("GetScreenResourcesCurrent", err))?
            .reply()
            .map_err(|err| x11_err("GetScreenResourcesCurrent reply", err))?;

        let mode_infos: HashMap<randr::Mode, &randr::ModeInfo> =
            resources.modes.iter().map(|mode| (mode.id, mode)).collect();
        let mut cache = X11Cache {
            config_timestamp: resources.config_timestamp,
            outputs: HashMap::new(),
        };
        let mut outputs = Vec::with_capacity(resources.outputs.len());

        for output_id in &resources.outputs {
            let info = self
                .conn
                .randr_get_output_info(*output_id, resources.config_timestamp)
                .map_err(|err| x11_err("GetOutputInfo", err))?
                .reply()
                .map_err(|err| x11_err("GetOutputInfo reply", err))?;
            let name = output_name(&info.name);
            let mut output = OutputInfo::new(&name);
            output.connected = info.connection == randr::Connection::CONNECTED;

            let preferred: HashSet<randr::Mode> = info
                .modes
                .iter()
                .take(usize::from(info.num_preferred))
                .copied()
                .collect();

            let mut cached = CachedOutput {
                output_id: *output_id,
                crtc: 0,
                crtc_outputs: Vec::new(),
                x: 0,
                y: 0,
                rotation: randr::Rotation::default(),
                enabled: false,
                mode_ids: Vec::new(),
            };

            if info.crtc != 0 {
                let crtc = self
                    .conn
                    .randr_get_crtc_info(info.crtc, resources.config_timestamp)
                    .map_err(|err| x11_err("GetCrtcInfo", err))?
                    .reply()
                    .map_err(|err| x11_err("GetCrtcInfo reply", err))?;
                cached.crtc = info.crtc;
                cached.x = crtc.x;
                cached.y = crtc.y;
                cached.rotation = crtc.rotation;
                cached.crtc_outputs = crtc.outputs.clone();
                cached.enabled = crtc.mode != 0;
                output.enabled = cached.enabled;
                if crtc.mode != 0 {
                    if let Some(mode_info) = mode_infos.get(&crtc.mode) {
                        if let Some(rate) = mode_refresh(mode_info) {
                            output.current_mode = Some(DisplayMode {
                                width: u32::from(mode_info.width),
                                height: u32::from(mode_info.height),
                                refresh_hz: rate,
                                preferred: preferred.contains(&crtc.mode),
                                current: true,
                            });
                        }
                    }
                }
            }

            for mode_id in &info.modes {
                let Some(mode_info) = mode_infos.get(mode_id) else {
                    continue;
                };
                let Some(rate) = mode_refresh(mode_info) else {
                    continue;
                };
                let mode = DisplayMode {
                    width: u32::from(mode_info.width),
                    height: u32::from(mode_info.height),
                    refresh_hz: rate,
                    preferred: preferred.contains(mode_id),
                    current: output.current_mode.as_ref().is_some_and(|current| {
                        current.width == u32::from(mode_info.width)
                            && current.height == u32::from(mode_info.height)
                            && rates_match(current.refresh_hz, rate, 0.01)
                    }),
                };
                output.modes.push(mode.clone());
                cached.mode_ids.push((*mode_id, mode));
            }

            // Le mode courant peut ne pas figurer dans la liste `modes` de
            // l'output sur certains serveurs : l'ajouter pour que le matching
            // à la résolution courante reste possible.
            if let Some(current) = output.current_mode.clone() {
                if !output.modes.iter().any(|mode| same_mode(mode, &current)) {
                    output.modes.push(current);
                }
            }

            cache.outputs.insert(name, cached);
            outputs.push(output);
        }

        self.cache = cache;
        Ok(outputs)
    }
}

impl DisplayModeBackend for X11Backend {
    fn backend_name(&self) -> &'static str {
        "x11"
    }

    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::X11
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        self.list()
    }

    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        ensure_internal(output)?;
        // Relecture fraîche : le mode courant a pu changer depuis le dernier
        // listage. `list()` met aussi le cache à jour.
        let outputs = self.list()?;
        if !outputs
            .iter()
            .any(|candidate| candidate.name == output.name())
        {
            return Err(HwError::DisplayModeNotFound(format!(
                "output {} disparu",
                output.name()
            )));
        }
        let cached = self
            .cache
            .outputs
            .get(output.name())
            .cloned()
            .ok_or_else(|| HwError::DisplayModeNotFound(output.name().to_owned()))?;

        if !cached.enabled || cached.crtc == 0 {
            return Err(HwError::Display(format!(
                "écran interne {} inactif",
                output.name()
            )));
        }
        // Un CRTC partagé signifie miroir : changer le mode toucherait aussi
        // l'autre écran, potentiellement externe. Refus explicite.
        if cached.crtc_outputs.len() != 1 || cached.crtc_outputs[0] != cached.output_id {
            return Err(HwError::Display(format!(
                "CRTC de {} partagé avec {:?} : application refusée pour ne pas \
                 modifier un écran externe",
                output.name(),
                cached.crtc_outputs
            )));
        }

        let target = cached
            .mode_ids
            .iter()
            .filter(|(_, candidate)| {
                candidate.width == mode.width && candidate.height == mode.height
            })
            .min_by(|(_, left), (_, right)| {
                (left.refresh_hz - mode.refresh_hz)
                    .abs()
                    .total_cmp(&(right.refresh_hz - mode.refresh_hz).abs())
            })
            .filter(|(_, candidate)| {
                rates_match(candidate.refresh_hz, mode.refresh_hz, TOLERANCE_HZ)
            })
            .map(|(id, _)| *id)
            .ok_or_else(|| {
                HwError::DisplayModeNotFound(format!(
                    "{} Hz à {}x{} sur {}",
                    mode.refresh_hz,
                    mode.width,
                    mode.height,
                    output.name()
                ))
            })?;

        let reply = self
            .conn
            .randr_set_crtc_config(
                cached.crtc,
                x11rb::CURRENT_TIME,
                self.cache.config_timestamp,
                cached.x,
                cached.y,
                target,
                cached.rotation,
                &cached.crtc_outputs,
            )
            .map_err(|err| x11_err("SetCrtcConfig", err))?
            .reply()
            .map_err(|err| x11_err("SetCrtcConfig reply", err))?;
        if reply.status != randr::SetConfig::SUCCESS {
            return Err(HwError::Display(format!(
                "SetCrtcConfig refusé par le serveur X (status {:?})",
                reply.status
            )));
        }
        self.conn.flush().map_err(|err| x11_err("flush", err))?;
        Ok(ApplyOutcome::Applied)
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn x11_err(context: &str, err: impl std::fmt::Display) -> HwError {
    HwError::Display(format!("X11 {context} : {err}"))
}

/// Calcule le taux réel d'un mode XRandR.
///
/// `dot_clock` est en Hz ; `htotal * vtotal` donne le nombre de pixels par
/// trame. Les modes entrelacés (TV) ne sont pas ciblés (écrans internes eDP).
fn mode_refresh(info: &randr::ModeInfo) -> Option<f32> {
    let total = u64::from(info.htotal) * u64::from(info.vtotal);
    if total == 0 || info.dot_clock == 0 {
        return None;
    }
    let rate = f64::from(info.dot_clock) / total as f64;
    if rate.is_finite() && rate > 0.0 {
        Some(rate as f32)
    } else {
        None
    }
}

fn output_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_owned()
}

fn same_mode(left: &DisplayMode, right: &DisplayMode) -> bool {
    left.width == right.width
        && left.height == right.height
        && rates_match(left.refresh_hz, right.refresh_hz, 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_info(
        width: u16,
        height: u16,
        dot_clock: u32,
        htotal: u16,
        vtotal: u16,
    ) -> randr::ModeInfo {
        randr::ModeInfo {
            id: 1,
            width,
            height,
            dot_clock,
            hsync_start: 0,
            hsync_end: 0,
            htotal,
            hskew: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal,
            name_len: 0,
            mode_flags: randr::ModeFlag::default(),
        }
    }

    #[test]
    fn refresh_is_dot_clock_over_total() {
        // 2560x1440 @ 165 Hz : 650 MHz / (2720 * 1448) ≈ 165.03
        let info = mode_info(2560, 1440, 650_000_000, 2720, 1448);
        let rate = mode_refresh(&info).unwrap();
        assert!((rate - 165.0).abs() < 1.0, "rate = {rate}");
    }

    #[test]
    fn zero_totals_are_rejected() {
        assert!(mode_refresh(&mode_info(1920, 1080, 0, 0, 0)).is_none());
    }

    #[test]
    fn output_name_trims_nul() {
        assert_eq!(output_name(b"eDP-1\0"), "eDP-1");
        assert_eq!(output_name(b"HDMI-A-1"), "HDMI-A-1");
    }
}
