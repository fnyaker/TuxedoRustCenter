// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker d'éclairage clavier / lightbar.
//!
//! Reprend `LightingListener` de l'original :
//!
//! - détecte le clavier (blanc, RGB, per-key) et la lightbar au démarrage ;
//! - applique `settings.keyboard_states` et `settings.lightbar_states` quand
//!   les réglages changent (révision) ;
//! - respecte `settings.keyboard_backlight_control_enabled` : désactivé, aucune
//!   écriture n'est faite ;
//! - réessaie la détection à chaque tick si le pilote n'est pas encore chargé
//!   (l'original réessayait 10 fois à 3 s d'intervalle).
//!
//! Le mode « respiration » n'a pas d'équivalent matériel dans l'ABI LED :
//! seul l'état statique est appliqué, exactement comme `writeKeyboardStates`
//! qui ignore `state.mode`.
//!
//! Les couleurs `-1` (non supporté) de `tcc-core` sont ramenées à `0` avant
//! écriture ; l'original aurait écrit `-1` et laissé échouer l'écriture.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_core::{KeyboardState, LightbarState};
use tcc_hw::error::HwError;
use tcc_hw::lighting::{LightingController, RgbColor};
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de vérification (détection tardive, relecture des capacités).
pub const LIGHTING_TICK: Duration = Duration::from_secs(10);

/// Vérifie que l'utilisateur autorise le pilotage de l'éclairage.
///
/// `settings.keyboard_backlight_control_enabled` est un interrupteur global
/// (finding H-06 de `docs/security/REVIEW_HARDWARE.md`). Le worker le respecte
/// déjà ; **tout setter D-Bus doit aussi l'appeler avant d'écrire**. Le setter
/// actuel de `linux-platform` contourne encore ce contrôle : c'est un point
/// d'intégration documenté dans `docs/hardware/HARDWARE_MATRIX.md` §3.2 et
/// `docs/hardware/INTEGRATION_NOTES.md` §3.
///
/// # Errors
///
/// [`HwError::PolicyDenied`] si le contrôle est désactivé.
pub fn ensure_control_enabled(settings: &tcc_core::Settings) -> Result<(), HwError> {
    if settings.keyboard_backlight_control_enabled {
        Ok(())
    } else {
        Err(HwError::PolicyDenied {
            resource: "rétroéclairage clavier",
            reason: "contrôle désactivé dans les réglages",
        })
    }
}

/// Capacités du clavier exposées à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyboardData {
    pub available: bool,
    /// TCC peut appliquer un état (matériel présent, contrôle activé, hors
    /// `dry_run`). L'autorisation D-Bus est une politique séparée.
    pub writable: bool,
    /// Modes réellement appliqués. Seul `static` est supporté par l'ABI LED ;
    /// le mode « respiration » de l'original n'a pas d'équivalent matériel.
    pub modes: Vec<String>,
    /// `white`, `rgb`, `per-key` ou `none`.
    pub kind: String,
    pub zones: u8,
    pub max_brightness: u32,
    /// `None` pour un clavier blanc.
    pub max_red: Option<u16>,
    pub max_green: Option<u16>,
    pub max_blue: Option<u16>,
    /// Vrai si `buffer_input` est piloté.
    pub buffer_input: bool,
}

/// Capacités de la lightbar exposées à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LightbarData {
    pub available: bool,
    /// TCC peut appliquer un état (matériel présent, contrôle activé, hors
    /// `dry_run`).
    pub writable: bool,
    /// Modes réellement appliqués (`static` uniquement).
    pub modes: Vec<String>,
    pub max_brightness: u32,
    pub max_red: u16,
    pub max_green: u16,
    pub max_blue: u16,
}

/// État d'une zone tel qu'appliqué.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ZoneData {
    pub brightness: u8,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<&KeyboardState> for ZoneData {
    fn from(state: &KeyboardState) -> Self {
        Self {
            brightness: state.brightness,
            red: clamp_color(state.red),
            green: clamp_color(state.green),
            blue: clamp_color(state.blue),
        }
    }
}

impl From<&LightbarState> for ZoneData {
    fn from(state: &LightbarState) -> Self {
        Self {
            brightness: state.brightness,
            red: clamp_color(state.red),
            green: clamp_color(state.green),
            blue: clamp_color(state.blue),
        }
    }
}

impl ZoneData {
    fn color(&self) -> RgbColor {
        RgbColor::new(self.red, self.green, self.blue)
    }
}

fn clamp_color(value: i16) -> u8 {
    value.clamp(0, 255) as u8
}

/// Modes réellement supportés par l'ABI LED.
fn supported_modes(available: bool) -> Vec<String> {
    if available {
        vec!["static".to_owned()]
    } else {
        Vec::new()
    }
}

/// Instantané d'éclairage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LightingData {
    /// Vrai si au moins un périphérique est pilotable.
    pub available: bool,
    /// Contrôle activé dans les réglages.
    pub enabled: bool,
    /// Vrai si le matériel n'est pas exposé.
    pub unsupported: bool,
    pub keyboard: KeyboardData,
    pub lightbar: LightbarData,
    /// États demandés par les réglages (0 à N zones).
    pub states: Vec<ZoneData>,
    /// État demandé pour la lightbar.
    pub lightbar_state: Option<ZoneData>,
    /// Vrai si la dernière application a réussi.
    pub applied: bool,
    /// Erreurs de la dernière application.
    pub errors: Vec<String>,
}

/// Runtime du worker d'éclairage.
#[derive(Debug)]
pub struct LightingRuntime {
    root: SysfsRoot,
    dry_run: bool,
    controller: Option<LightingController>,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
    last_error_logged: bool,
}

impl LightingRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            root,
            dry_run,
            controller: None,
            last_revision: None,
            applied_revision: None,
            last_error_logged: false,
        }
    }

    /// Exécute un tick.
    pub fn tick(&mut self, input: &HardwareInputs) -> LightingData {
        self.ensure_controller();
        let Some(controller) = self.controller.as_ref() else {
            return LightingData {
                enabled: ensure_control_enabled(&input.settings).is_ok(),
                unsupported: true,
                applied: true,
                ..LightingData::default()
            };
        };

        let control_enabled = ensure_control_enabled(&input.settings).is_ok();
        let can_write = control_enabled && !self.dry_run;
        let keyboard = controller.keyboard_capabilities();
        let lightbar = controller.lightbar_capabilities();
        let mut data = LightingData {
            available: controller.keyboard_available() || controller.lightbar_available(),
            enabled: control_enabled,
            unsupported: !controller.keyboard_available() && !controller.lightbar_available(),
            keyboard: KeyboardData {
                available: controller.keyboard_available(),
                writable: can_write && controller.keyboard_available(),
                modes: supported_modes(controller.keyboard_available()),
                kind: keyboard.kind.as_str().to_owned(),
                zones: keyboard.zones,
                max_brightness: keyboard.max_brightness,
                max_red: keyboard.max_red,
                max_green: keyboard.max_green,
                max_blue: keyboard.max_blue,
                buffer_input: keyboard.uses_buffer_input,
            },
            lightbar: LightbarData {
                available: controller.lightbar_available(),
                writable: can_write && controller.lightbar_available(),
                modes: supported_modes(controller.lightbar_available()),
                max_brightness: lightbar.max_brightness,
                max_red: lightbar.max_red,
                max_green: lightbar.max_green,
                max_blue: lightbar.max_blue,
            },
            states: input
                .settings
                .keyboard_states
                .iter()
                .map(ZoneData::from)
                .collect(),
            lightbar_state: input.settings.lightbar_states.as_ref().map(ZoneData::from),
            applied: true,
            errors: Vec::new(),
        };

        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }
        if self.dry_run || !control_enabled {
            self.applied_revision = self.last_revision;
            return data;
        }
        if self.applied_revision == self.last_revision && self.last_revision.is_some() {
            return data;
        }

        let mut errors = Vec::new();
        self.apply_keyboard(controller, &input.settings.keyboard_states, &mut errors);
        self.apply_lightbar(
            controller,
            input.settings.lightbar_states.as_ref(),
            &mut errors,
        );
        data.errors = errors;
        data.applied = data.errors.is_empty();
        if data.applied {
            self.applied_revision = self.last_revision;
        }
        data
    }

    fn ensure_controller(&mut self) {
        // Une détection peut réussir sans rien trouver (module pas encore
        // chargé). On retente à chaque tick tant qu'aucun périphérique n'est
        // pilotable ; la détection ne lit qu'une poignée d'attributs.
        let usable = self.controller.as_ref().is_some_and(|controller| {
            controller.keyboard_available() || controller.lightbar_available()
        });
        if usable {
            return;
        }
        match LightingController::detect(self.root.clone()) {
            Ok(controller) => {
                if controller.keyboard_available() || controller.lightbar_available() {
                    tracing::info!(
                        keyboard = controller.keyboard_capabilities().kind.as_str(),
                        zones = controller.keyboard_capabilities().zones,
                        lightbar = controller.lightbar_available(),
                        "éclairage détecté"
                    );
                } else {
                    tracing::debug!("aucun éclairage pilotable détecté");
                }
                self.controller = Some(controller);
            }
            Err(error) => {
                if !self.last_error_logged {
                    self.last_error_logged = true;
                    tracing::warn!(%error, "détection de l'éclairage impossible");
                }
            }
        }
    }

    fn apply_keyboard(
        &self,
        controller: &LightingController,
        states: &[KeyboardState],
        errors: &mut Vec<String>,
    ) {
        if states.is_empty() || !controller.keyboard_available() {
            return;
        }
        let Some(first) = states.first() else {
            return;
        };
        let colors: Vec<RgbColor> = states
            .iter()
            .map(ZoneData::from)
            .map(|zone| zone.color())
            .collect();
        match controller.apply_keyboard(first.brightness, &colors) {
            Ok(report) => {
                if !report.errors.is_empty() {
                    errors.extend(report.errors);
                }
            }
            Err(error) => errors.push(format!("clavier: {error}")),
        }
    }

    fn apply_lightbar(
        &self,
        controller: &LightingController,
        state: Option<&LightbarState>,
        errors: &mut Vec<String>,
    ) {
        let Some(state) = state else {
            return;
        };
        if !controller.lightbar_available() {
            return;
        }
        let zone = ZoneData::from(state);
        if let Err(error) = controller.apply_lightbar(zone.brightness, zone.color()) {
            errors.push(format!("lightbar: {error}"));
        }
    }
}

/// Démarre le worker d'éclairage.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = LightingRuntime::new(root, dry_run);
    spawn_worker(
        "éclairage",
        LIGHTING_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_lighting,
        LightingRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tcc_core::KeyboardColorMode;

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

    fn rgb_fixture(root: &Path) {
        for index in 0..3 {
            let name = if index == 0 {
                "rgb:kbd_backlight".to_owned()
            } else {
                format!("rgb:kbd_backlight_{index}")
            };
            let base = format!("devices/platform/tuxedo_keyboard/leds/{name}");
            write(root, &format!("{base}/max_brightness"), "255\n");
            write(root, &format!("{base}/brightness"), "0\n");
            write(root, &format!("{base}/multi_intensity"), "0 0 0\n");
        }
        write(
            root,
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input",
            "0\n",
        );
    }

    fn settings() -> tcc_core::Settings {
        tcc_core::Settings {
            keyboard_states: vec![
                KeyboardState::new(KeyboardColorMode::Static, 40, 10, 20, 30),
                KeyboardState::new(KeyboardColorMode::Static, 40, 40, 50, 60),
                KeyboardState::new(KeyboardColorMode::Static, 40, 70, 80, 90),
            ],
            lightbar_states: Some(LightbarState {
                brightness: 10,
                red: 1,
                green: 2,
                blue: 3,
            }),
            ..tcc_core::Settings::default()
        }
    }

    fn input(revision: u64) -> HardwareInputs {
        HardwareInputs::new(settings(), None, false, revision)
    }

    #[test]
    fn applies_keyboard_states_once_per_revision() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path());
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));

        assert!(data.available);
        assert!(data.applied, "{data:?}");
        assert_eq!(data.keyboard.kind, "rgb");
        assert!(data.keyboard.writable);
        assert_eq!(data.keyboard.modes, vec!["static".to_owned()]);
        assert_eq!(data.keyboard.zones, 3);
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "10 20 30"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_2/multi_intensity"
            ),
            "70 80 90"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input"
            ),
            "0"
        );

        // Changement externe : pas réécrit sans nouvelle révision.
        write(
            dir.path(),
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity",
            "1 1 1\n",
        );
        runtime.tick(&input(1));
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "1 1 1"
        );

        runtime.tick(&input(2));
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "10 20 30"
        );
    }

    #[test]
    fn disabled_control_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path());
        let mut settings = settings();
        settings.keyboard_backlight_control_enabled = false;
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(!data.enabled);
        assert!(!data.keyboard.writable);
        assert!(!data.lightbar.writable);
        assert!(data.applied);
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "0 0 0"
        );
    }

    #[test]
    fn ensure_control_enabled_guards_setters() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path());
        let enabled = tcc_core::Settings::default();
        assert!(ensure_control_enabled(&enabled).is_ok());

        let disabled = tcc_core::Settings {
            keyboard_backlight_control_enabled: false,
            ..tcc_core::Settings::default()
        };
        let error = ensure_control_enabled(&disabled).expect_err("contrôle désactivé");
        assert!(error.is_policy_denied());
    }

    #[test]
    fn disabled_control_is_reported_even_without_hardware() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings();
        settings.keyboard_backlight_control_enabled = false;
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(data.unsupported);
        assert!(!data.enabled);
    }

    #[test]
    fn white_keyboard_ignores_colors() {
        let dir = tempfile::tempdir().unwrap();
        let base = "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight";
        write(dir.path(), &format!("{base}/max_brightness"), "3\n");
        write(dir.path(), &format!("{base}/brightness"), "0\n");
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert_eq!(data.keyboard.kind, "white");
        assert_eq!(data.keyboard.zones, 1);
        assert_eq!(data.keyboard.max_red, None);
        assert_eq!(read(dir.path(), &format!("{base}/brightness")), "3");
    }

    #[test]
    fn missing_hardware_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(!data.available);
        assert!(data.unsupported);
        assert!(data.applied);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn lazy_detection_picks_up_late_driver() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(data.unsupported);
        // Le pilote apparaît après le démarrage.
        rgb_fixture(dir.path());
        let data = runtime.tick(&input(1));
        assert!(!data.unsupported, "{data:?}");
        assert!(data.available);
        assert!(data.applied);
    }

    #[test]
    fn dry_run_and_negative_colors_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path());
        let mut settings = settings();
        settings.keyboard_states[0].red = -1;
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&HardwareInputs::new(settings, None, false, 1));
        assert!(data.applied);
        assert_eq!(data.states[0].red, 0);
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "0 0 0"
        );
    }

    #[test]
    fn lightbar_state_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let base = "class/leds/rgb:lightbar";
        write(dir.path(), &format!("{base}/max_brightness"), "10\n");
        write(dir.path(), &format!("{base}/brightness"), "0\n");
        write(dir.path(), &format!("{base}/multi_intensity"), "0 0 0\n");
        let mut runtime = LightingRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(1));
        assert!(data.lightbar.available);
        assert!(data.lightbar.writable);
        assert_eq!(data.lightbar.modes, vec!["static".to_owned()]);
        assert!(data.applied);
        assert_eq!(read(dir.path(), &format!("{base}/brightness")), "10");
        assert_eq!(
            read(dir.path(), &format!("{base}/multi_intensity")),
            "1 2 3"
        );
    }
}
