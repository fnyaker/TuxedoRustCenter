// SPDX-License-Identifier: GPL-3.0-or-later
//! Éclairage du clavier et de la barre lumineuse (LED sysfs).
//!
//! # Chemins
//!
//! ```text
//! /sys/devices/platform/tuxedo_keyboard/leds/white:kbd_backlight
//! /sys/bus/platform/devices/tuxedo_nb05_kbd_backlight/leds/white:kbd_backlight
//! /sys/devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight[_N]
//! /sys/class/leds/rgb:lightbar
//! <led>/brightness
//! <led>/max_brightness
//! <led>/multi_intensity          ("R G B", 0..=255 par composante)
//! <led>/device/controls/buffer_input   (0/1)
//! /sys/bus/hid/drivers/tuxedo-keyboard-ite/<dev>/leds/rgb:kbd_backlight[*]
//! /sys/bus/hid/drivers/ite_829x/<dev>/leds/...
//! /sys/bus/hid/drivers/ite_8291/<dev>/leds/...
//! /sys/bus/platform/drivers/tuxedo_nb04_kbd_backlight/<dev>/leds/...
//! ```
//!
//! Sources : `LightingListener` de l'original et
//! `Documentation/leds/led-class.rgb` du noyau.
//!
//! # Reproduction du comportement original
//!
//! 1. Détection **blanc d'abord** : si `white:kbd_backlight` expose
//!    `max_brightness`, le clavier est blanc (une zone, pas de couleur).
//! 2. Sinon, zones RGB découvertes via les pilotes HID ITE, avec repli sur les
//!    trois chemins statiques `rgb:kbd_backlight[_1|_2]` (Polaris Gen 1).
//! 3. `zones > 4` ⇒ per-key ; `4`, `3`, `1` zones ⇒ RGB classique.
//! 4. Écriture clavier : `buffer_input = 1`, puis `multi_intensity` par zone,
//!    puis `buffer_input = 0` (`writeKeyboardStates`). La luminosité est
//!    écrite une fois, sur la première LED.
//! 5. Lightbar : `brightness` + `multi_intensity` (`writeLightbarStates`).
//!
//! # Différences assumées
//!
//! - L'original écrit la luminosité du clavier via UPower
//!   (`org.freedesktop.UPower.KbdBacklight.SetBrightness`), qui agit sur la
//!   même LED côté pilote. Ici l'écriture se fait directement dans
//!   `<led>/brightness` : pas de dépendance D-Bus dans la couche matérielle,
//!   testable en CI, et l'effet matériel est identique. Si un modèle exige le
//!   chemin UPower, `linux-platform` peut le brancher ; le risque résiduel est
//!   documenté dans `docs/hardware/HARDWARE_MATRIX.md`.
//! - Le **mode respiration n'existe pas** dans l'ABI LED utilisée : l'original
//!   ne l'écrit jamais non plus (`writeKeyboardStates` ignore `state.mode`).
//!   Le champ reste donc porté par les réglages et exposé à l'UI, sans effet
//!   matériel — parité stricte avec l'original.
//! - En cas d'échec d'une zone, `buffer_input` est **toujours** remis à `0`.
//!   L'original laissait le buffer actif si une exception survenait, ce qui
//!   pouvait figer la couleur affichée jusqu'au prochain cycle.
//! - Les zones manquantes dans le tableau fourni sont ignorées au lieu de
//!   lever (l'original levait puis journalisait).

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Chemins blancs connus, dans l'ordre de l'original.
const WHITE_LED_PATHS: [&str; 2] = [
    "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight",
    "bus/platform/devices/tuxedo_nb05_kbd_backlight/leds/white:kbd_backlight",
];

/// Chemins RGB statiques de repli (Polaris Gen 1 et claviers simples).
const STATIC_RGB_PATHS: [&str; 3] = [
    "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight",
    "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_1",
    "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_2",
];

/// Racines des pilotes HID/platform exposant des zones RGB.
const RGB_DRIVER_ROOTS: [&str; 4] = [
    "bus/hid/drivers/tuxedo-keyboard-ite",
    "bus/hid/drivers/ite_829x",
    "bus/hid/drivers/ite_8291",
    "bus/platform/drivers/tuxedo_nb04_kbd_backlight",
];

/// Chemin de la lightbar.
pub const LIGHTBAR_PATH: &str = "class/leds/rgb:lightbar";
/// Nombre maximal de zones accepté (per-key inclus).
const MAX_ZONES: usize = 128;
/// Composante couleur maximale du contrat LED.
const MAX_COLOR: u16 = 0xff;

/// Type de clavier détecté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardKind {
    /// Aucun rétroéclairage clavier détecté.
    None,
    /// Clavier blanc (une seule intensité, pas de couleur).
    WhiteOnly,
    /// Clavier RGB par zones (1 à 4 zones).
    Rgb,
    /// Clavier RGB par touche (plus de 4 zones).
    PerKey,
}

impl KeyboardKind {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WhiteOnly => "white",
            Self::Rgb => "rgb",
            Self::PerKey => "per-key",
        }
    }
}

/// Couleur RGB (`0..=255` par composante).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl RgbColor {
    /// Construit une couleur.
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// Formate la valeur attendue par `multi_intensity`.
    #[must_use]
    pub fn as_multi_intensity(&self) -> String {
        format!("{} {} {}", self.red, self.green, self.blue)
    }
}

/// Capacités du rétroéclairage clavier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardCapabilities {
    /// Type détecté.
    pub kind: KeyboardKind,
    /// Nombre de zones pilotables (0 si absent).
    pub zones: u8,
    /// Luminosité maximale (0 si absent).
    pub max_brightness: u32,
    /// Composante rouge maximale (`None` pour un clavier blanc).
    pub max_red: Option<u16>,
    /// Composante verte maximale.
    pub max_green: Option<u16>,
    /// Composante bleue maximale.
    pub max_blue: Option<u16>,
    /// Vrai si `buffer_input` est écrit autour des couleurs.
    pub uses_buffer_input: bool,
}

impl Default for KeyboardCapabilities {
    fn default() -> Self {
        Self {
            kind: KeyboardKind::None,
            zones: 0,
            max_brightness: 0,
            max_red: None,
            max_green: None,
            max_blue: None,
            uses_buffer_input: false,
        }
    }
}

impl KeyboardCapabilities {
    /// Vrai si un rétroéclairage est pilotable.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.kind != KeyboardKind::None && self.max_brightness > 0 && self.zones > 0
    }
}

/// Capacités de la barre lumineuse.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LightbarCapabilities {
    pub available: bool,
    pub max_brightness: u32,
    pub max_red: u16,
    pub max_green: u16,
    pub max_blue: u16,
}

/// Résultat de l'application d'un état clavier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LightingApplyReport {
    /// Luminosité appliquée, si le clavier est pilotable.
    pub brightness: Option<u8>,
    /// Nombre de zones RGB écrites.
    pub zones_written: u8,
    /// Erreurs par zone (`multi_intensity`), la première erreur bloquante.
    pub errors: Vec<String>,
    /// Vrai si l'état a été appliqué sans erreur.
    pub applied: bool,
}

/// Périphérique LED découvert.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LedDevice {
    /// Chemin relatif à la racine sysfs.
    path: String,
    /// Luminosité maximale.
    max_brightness: u32,
}

/// Contrôleur d'éclairage.
#[derive(Debug, Clone)]
pub struct LightingController {
    root: SysfsRoot,
    keyboard: KeyboardCapabilities,
    keyboard_leds: Vec<LedDevice>,
    lightbar: LightbarCapabilities,
    lightbar_led: Option<LedDevice>,
}

impl LightingController {
    /// Détecte clavier et barre lumineuse.
    ///
    /// Aucun matériel absent n'est une erreur : les capacités sont simplement
    /// `is_available() == false`.
    ///
    /// # Errors
    ///
    /// Seules les erreurs d'E/S réelles (énumération impossible) sont remontées.
    pub fn detect(root: SysfsRoot) -> Result<Self> {
        let mut controller = Self {
            root,
            keyboard: KeyboardCapabilities::default(),
            keyboard_leds: Vec::new(),
            lightbar: LightbarCapabilities::default(),
            lightbar_led: None,
        };
        controller.rescan()?;
        Ok(controller)
    }

    /// Relit la topologie (hotplug udev, chargement tardif du pilote).
    ///
    /// # Errors
    ///
    /// Voir [`LightingController::detect`].
    pub fn rescan(&mut self) -> Result<()> {
        let (kind, leds) = self.detect_keyboard()?;
        self.keyboard = self.build_keyboard_capabilities(kind, &leds)?;
        self.keyboard_leds = leds;

        let lightbar = self.read_led(LIGHTBAR_PATH)?;
        self.lightbar = match &lightbar {
            Some(led) => {
                // La lightbar n'est pilotable que si `multi_intensity` existe,
                // comme `detectLightbar` de l'original.
                if self
                    .root
                    .is_file(format!("{LIGHTBAR_PATH}/multi_intensity"))
                {
                    LightbarCapabilities {
                        available: true,
                        max_brightness: led.max_brightness,
                        max_red: MAX_COLOR,
                        max_green: MAX_COLOR,
                        max_blue: MAX_COLOR,
                    }
                } else {
                    LightbarCapabilities::default()
                }
            }
            None => LightbarCapabilities::default(),
        };
        self.lightbar_led = lightbar.filter(|_| self.lightbar.available);
        Ok(())
    }

    /// Capacités du clavier.
    #[must_use]
    pub fn keyboard_capabilities(&self) -> &KeyboardCapabilities {
        &self.keyboard
    }

    /// Capacités de la barre lumineuse.
    #[must_use]
    pub fn lightbar_capabilities(&self) -> &LightbarCapabilities {
        &self.lightbar
    }

    /// Vrai si la luminosité clavier est pilotable.
    #[must_use]
    pub fn keyboard_available(&self) -> bool {
        self.keyboard.is_available() && !self.keyboard_leds.is_empty()
    }

    /// Vrai si la lightbar est pilotable.
    #[must_use]
    pub fn lightbar_available(&self) -> bool {
        self.lightbar.available && self.lightbar_led.is_some()
    }

    /// Applique un état clavier (luminosité commune + couleurs par zone).
    ///
    /// `brightness` est la valeur **demandée** (échelle `0..=255` du contrat
    /// `tcc-core`) ; elle est plafonnée à `max_brightness` avant écriture et
    /// retournée telle quelle dans le rapport. Chaque couleur est
    /// écrite dans la zone correspondante ; les zones au-delà du nombre détecté
    /// sont ignorées. Sur un clavier blanc, les couleurs sont sans effet.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si aucun clavier n'est détecté ;
    /// - [`HwError::Io`] si la luminosité ne peut pas être écrite.
    ///
    /// Les erreurs de couleur par zone ne sont pas fatales : elles sont
    /// retournées dans [`LightingApplyReport::errors`] et les autres zones
    /// continuent d'être écrites.
    pub fn apply_keyboard(
        &self,
        brightness: u8,
        colors: &[RgbColor],
    ) -> Result<LightingApplyReport> {
        if !self.keyboard_available() {
            return Err(HwError::NotSupported("rétroéclairage clavier"));
        }
        let Some(first) = self.keyboard_leds.first() else {
            return Err(HwError::NotSupported("rétroéclairage clavier"));
        };
        let raw_brightness = scale_brightness(brightness, first.max_brightness);
        self.write_brightness(&first.path, raw_brightness)?;

        let mut report = LightingApplyReport {
            brightness: Some(brightness),
            ..LightingApplyReport::default()
        };
        if self.keyboard.kind == KeyboardKind::WhiteOnly {
            report.applied = true;
            return Ok(report);
        }

        let buffer_path = self
            .keyboard
            .uses_buffer_input
            .then(|| format!("{}/device/controls/buffer_input", first.path));
        let buffer_guard = BufferGuard::new(&self.root, buffer_path.as_deref());

        let zones = usize::from(self.keyboard.zones);
        for (index, led) in self.keyboard_leds.iter().take(zones).enumerate() {
            let Some(color) = colors.get(index) else {
                continue;
            };
            let path = format!("{}/multi_intensity", led.path);
            if !self.root.is_file(&path) {
                continue;
            }
            match self.root.write_string(&path, &color.as_multi_intensity()) {
                Ok(()) => report.zones_written = report.zones_written.saturating_add(1),
                Err(error) => report.errors.push(format!("zone {index}: {error}")),
            }
        }
        if let Some(guard) = buffer_guard
            && let Err(error) = guard.release()
        {
            report.errors.push(format!("buffer_input: {error}"));
        }
        report.applied = report.errors.is_empty() && report.zones_written > 0;
        Ok(report)
    }

    /// Applique un état de barre lumineuse.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si la lightbar est absente ;
    /// - [`HwError::Io`] en cas d'échec d'écriture.
    pub fn apply_lightbar(&self, brightness: u8, color: RgbColor) -> Result<()> {
        let Some(led) = self.lightbar_led.as_ref() else {
            return Err(HwError::NotSupported("lightbar"));
        };
        if !self.lightbar.available {
            return Err(HwError::NotSupported("lightbar"));
        }
        let raw_brightness = scale_brightness(brightness, led.max_brightness);
        self.write_brightness(&led.path, raw_brightness)?;
        self.root.write_string(
            format!("{}/multi_intensity", led.path),
            &color.as_multi_intensity(),
        )
    }

    /// Écrit la luminosité brute, sans écrire si la valeur est déjà appliquée.
    fn write_brightness(&self, led_path: &str, raw: u32) -> Result<()> {
        let path = format!("{led_path}/brightness");
        let current = self
            .root
            .read_optional_string(&path)?
            .and_then(|value| value.trim().parse::<u32>().ok());
        if current == Some(raw) {
            return Ok(());
        }
        self.root.write_i64(&path, i64::from(raw))
    }

    fn detect_keyboard(&self) -> Result<(KeyboardKind, Vec<LedDevice>)> {
        for path in WHITE_LED_PATHS {
            if let Some(led) = self.read_led(path)? {
                return Ok((KeyboardKind::WhiteOnly, vec![led]));
            }
        }

        let zones = self.detect_rgb_zones()?;
        if zones.is_empty() {
            return Ok((KeyboardKind::None, Vec::new()));
        }
        // `read_led` ne retient que les zones qui exposent `max_brightness` :
        // une zone non pilotable n'est jamais comptée (l'original désactivait
        // tout le clavier si la dernière zone manquait ; ici les zones valides
        // restent utilisables).
        let kind = if zones.len() > 4 {
            KeyboardKind::PerKey
        } else {
            KeyboardKind::Rgb
        };
        Ok((kind, zones))
    }

    fn build_keyboard_capabilities(
        &self,
        kind: KeyboardKind,
        leds: &[LedDevice],
    ) -> Result<KeyboardCapabilities> {
        if kind == KeyboardKind::None {
            return Ok(KeyboardCapabilities::default());
        }
        let Some(first) = leds.first() else {
            return Ok(KeyboardCapabilities::default());
        };
        let uses_buffer_input = self
            .root
            .is_file(format!("{}/device/controls/buffer_input", first.path));
        let (max_red, max_green, max_blue) = match kind {
            KeyboardKind::WhiteOnly => (None, None, None),
            _ => (Some(MAX_COLOR), Some(MAX_COLOR), Some(MAX_COLOR)),
        };
        Ok(KeyboardCapabilities {
            kind,
            zones: leds.len().min(MAX_ZONES).min(usize::from(u8::MAX)) as u8,
            max_brightness: first.max_brightness,
            max_red,
            max_green,
            max_blue,
            uses_buffer_input,
        })
    }

    /// Découvre les zones RGB, pilotes HID d'abord puis chemins statiques.
    fn detect_rgb_zones(&self) -> Result<Vec<LedDevice>> {
        let mut discovered: Vec<(u32, String)> = Vec::new();
        for driver in RGB_DRIVER_ROOTS {
            for device in self.root.list_dir(driver)? {
                let leds = format!("{driver}/{device}/leds");
                if !self.root.exists(&leds) {
                    continue;
                }
                for led in self.root.list_dir(&leds)? {
                    if !led.contains("rgb:kbd_backlight") {
                        continue;
                    }
                    discovered.push((rgb_index(&led), format!("{leds}/{led}")));
                }
            }
        }
        discovered.sort_by_key(|entry| entry.0);
        discovered.dedup_by(|left, right| left.0 == right.0);

        let mut zones = Vec::new();
        if !discovered.is_empty() {
            for (_, path) in discovered.into_iter().take(MAX_ZONES) {
                if let Some(led) = self.read_led(&path)? {
                    zones.push(led);
                }
            }
        } else {
            // Repli de l'original : seuls les chemins déjà connus qui existent.
            for path in STATIC_RGB_PATHS {
                if let Some(led) = self.read_led(path)? {
                    zones.push(led);
                }
            }
        }
        Ok(zones)
    }

    fn read_led(&self, path: &str) -> Result<Option<LedDevice>> {
        let Some(raw) = self
            .root
            .read_optional_string(format!("{path}/max_brightness"))?
        else {
            return Ok(None);
        };
        let value = raw.parse::<u64>().map_err(|_| HwError::InvalidValue {
            context: "luminosité maximale LED",
            value: raw,
        })?;
        if value == 0 || value > u64::from(u32::MAX) {
            return Err(HwError::OutOfRange {
                context: "luminosité maximale LED",
                value: i64::try_from(value).unwrap_or(i64::MAX),
                min: 1,
                max: i64::from(u32::MAX),
            });
        }
        Ok(Some(LedDevice {
            path: path.to_owned(),
            max_brightness: value as u32,
        }))
    }
}

/// Convertit une luminosité `0..=255` vers l'échelle de la LED.
///
/// Le contrat `tcc-core` exprime la luminosité sur `0..=255` ; UPower et
/// l'original utilisent la même échelle que `max_brightness`. On plafonne donc
/// à `max_brightness` sans remise à l'échelle (comme `SetBrightness`).
fn scale_brightness(brightness: u8, max_brightness: u32) -> u32 {
    u32::from(brightness).min(max_brightness)
}

/// Garde qui force `buffer_input = 0` même si une écriture échoue.
struct BufferGuard<'a> {
    root: &'a SysfsRoot,
    path: Option<String>,
    released: bool,
}

impl<'a> BufferGuard<'a> {
    fn new(root: &'a SysfsRoot, path: Option<&str>) -> Option<Self> {
        let path = path?;
        let guard = Self {
            root,
            path: Some(path.to_owned()),
            released: false,
        };
        if let Err(error) = root.write_string(path, "1") {
            tracing::debug!(%error, "activation du buffer d'entrée LED impossible");
            return Some(Self {
                root,
                path: None,
                released: true,
            });
        }
        Some(guard)
    }

    fn release(mut self) -> Result<()> {
        self.released = true;
        match &self.path {
            Some(path) => self.root.write_string(path, "0"),
            None => Ok(()),
        }
    }
}

impl Drop for BufferGuard<'_> {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if let Some(path) = &self.path {
            if let Err(error) = self.root.write_string(path, "0") {
                tracing::warn!(%error, "désactivation du buffer d'entrée LED impossible");
            }
        }
    }
}

/// Extrait l'index d'une zone `rgb:kbd_backlight[_N]`.
fn rgb_index(name: &str) -> u32 {
    let Some(position) = name.find("rgb:kbd_backlight") else {
        return u32::MAX;
    };
    name[position + "rgb:kbd_backlight".len()..]
        .trim_start_matches('_')
        .parse::<u32>()
        .unwrap_or(0)
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

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn white_fixture(root: &Path) {
        write(
            root,
            "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/max_brightness",
            "3\n",
        );
        write(
            root,
            "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/brightness",
            "0\n",
        );
    }

    fn rgb_fixture(root: &Path, zones: usize, buffer_input: bool) {
        for index in 0..zones {
            let name = if index == 0 {
                "rgb:kbd_backlight".to_owned()
            } else {
                format!("rgb:kbd_backlight_{index}")
            };
            let base = format!("devices/platform/tuxedo_keyboard/leds/{name}");
            write(root, &format!("{base}/max_brightness"), "255\n");
            write(root, &format!("{base}/brightness"), "0\n");
            write(root, &format!("{base}/multi_intensity"), "255 255 255\n");
        }
        if buffer_input {
            write(
                root,
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input",
                "0\n",
            );
        }
    }

    fn hid_rgb_fixture(root: &Path, zones: usize) {
        for index in 0..zones {
            let name = if index == 0 {
                "rgb:kbd_backlight".to_owned()
            } else {
                format!("rgb:kbd_backlight_{index}")
            };
            let base = format!("bus/hid/drivers/tuxedo-keyboard-ite/0003:1:0001/leds/{name}");
            write(root, &format!("{base}/max_brightness"), "255\n");
            write(root, &format!("{base}/brightness"), "0\n");
            write(root, &format!("{base}/multi_intensity"), "255 255 255\n");
        }
    }

    fn fixture_controller(root: &Path) -> LightingController {
        LightingController::detect(SysfsRoot::new(root)).unwrap()
    }

    #[test]
    fn detects_white_only_keyboard() {
        let dir = tempfile::tempdir().unwrap();
        white_fixture(dir.path());
        let controller = fixture_controller(dir.path());
        let capabilities = controller.keyboard_capabilities();

        assert_eq!(capabilities.kind, KeyboardKind::WhiteOnly);
        assert_eq!(capabilities.zones, 1);
        assert_eq!(capabilities.max_brightness, 3);
        assert_eq!(capabilities.max_red, None);
        assert!(!capabilities.uses_buffer_input);
        assert!(controller.keyboard_available());
        assert!(!controller.lightbar_available());
    }

    #[test]
    fn applies_white_keyboard_brightness_only() {
        let dir = tempfile::tempdir().unwrap();
        white_fixture(dir.path());
        let controller = fixture_controller(dir.path());

        let report = controller
            .apply_keyboard(9, &[RgbColor::new(255, 0, 0)])
            .unwrap();
        assert!(report.applied);
        assert_eq!(report.brightness, Some(9));
        assert_eq!(report.zones_written, 0);
        // Plafonnée à `max_brightness`.
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/brightness"
            ),
            "3"
        );
    }

    #[test]
    fn detects_bang_and_white_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        white_fixture(dir.path());
        rgb_fixture(dir.path(), 3, true);
        let controller = fixture_controller(dir.path());
        assert_eq!(
            controller.keyboard_capabilities().kind,
            KeyboardKind::WhiteOnly
        );
    }

    #[test]
    fn detects_three_zone_rgb_and_applies_colors() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path(), 3, true);
        let controller = fixture_controller(dir.path());
        let capabilities = controller.keyboard_capabilities();
        assert_eq!(capabilities.kind, KeyboardKind::Rgb);
        assert_eq!(capabilities.zones, 3);
        assert_eq!(capabilities.max_red, Some(255));
        assert!(capabilities.uses_buffer_input);

        let report = controller
            .apply_keyboard(
                40,
                &[
                    RgbColor::new(1, 2, 3),
                    RgbColor::new(4, 5, 6),
                    RgbColor::new(7, 8, 9),
                ],
            )
            .unwrap();
        assert!(report.applied, "{report:?}");
        assert_eq!(report.zones_written, 3);
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "1 2 3"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_2/multi_intensity"
            ),
            "7 8 9"
        );
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input"
            ),
            "0",
            "buffer_input doit être relâché"
        );
    }

    #[test]
    fn per_key_keyboard_uses_all_zones() {
        let dir = tempfile::tempdir().unwrap();
        // Le per-key n'est découvert que via les pilotes HID : les trois
        // chemins statiques ne couvrent que les claviers 1 à 3 zones.
        hid_rgb_fixture(dir.path(), 6);
        let controller = fixture_controller(dir.path());
        assert_eq!(
            controller.keyboard_capabilities().kind,
            KeyboardKind::PerKey
        );
        assert_eq!(controller.keyboard_capabilities().zones, 6);
        assert!(!controller.keyboard_capabilities().uses_buffer_input);
    }

    #[test]
    fn hid_driver_zones_are_sorted_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "rgb:kbd_backlight_2",
            "rgb:kbd_backlight",
            "rgb:kbd_backlight_1",
        ] {
            let base = format!("bus/hid/drivers/tuxedo-keyboard-ite/0003:1:0001/leds/{name}");
            write(dir.path(), &format!("{base}/max_brightness"), "255\n");
            write(dir.path(), &format!("{base}/brightness"), "0\n");
            write(dir.path(), &format!("{base}/multi_intensity"), "0 0 0\n");
        }
        let controller = fixture_controller(dir.path());
        assert_eq!(controller.keyboard_capabilities().kind, KeyboardKind::Rgb);
        assert_eq!(controller.keyboard_capabilities().zones, 3);
        let report = controller
            .apply_keyboard(10, &[RgbColor::new(9, 9, 9)])
            .unwrap();
        assert_eq!(report.zones_written, 1);
        assert_eq!(
            read(
                dir.path(),
                "bus/hid/drivers/tuxedo-keyboard-ite/0003:1:0001/leds/rgb:kbd_backlight/multi_intensity"
            ),
            "9 9 9"
        );
    }

    #[test]
    fn missing_zones_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        rgb_fixture(dir.path(), 3, false);
        let controller = fixture_controller(dir.path());
        let report = controller
            .apply_keyboard(10, &[RgbColor::new(1, 1, 1)])
            .unwrap();
        assert_eq!(report.zones_written, 1);
        assert!(report.applied);
    }

    #[test]
    fn lightbar_reads_capabilities_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let base = "class/leds/rgb:lightbar";
        write(dir.path(), &format!("{base}/max_brightness"), "10\n");
        write(dir.path(), &format!("{base}/brightness"), "0\n");
        write(dir.path(), &format!("{base}/multi_intensity"), "0 0 0\n");
        let controller = fixture_controller(dir.path());
        let capabilities = controller.lightbar_capabilities();
        assert!(capabilities.available);
        assert_eq!(capabilities.max_brightness, 10);
        assert_eq!(capabilities.max_red, 255);

        controller
            .apply_lightbar(200, RgbColor::new(10, 20, 30))
            .unwrap();
        assert_eq!(read(dir.path(), &format!("{base}/brightness")), "10");
        assert_eq!(
            read(dir.path(), &format!("{base}/multi_intensity")),
            "10 20 30"
        );
    }

    #[test]
    fn lightbar_without_multi_intensity_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/leds/rgb:lightbar/max_brightness", "10\n");
        let controller = fixture_controller(dir.path());
        assert!(!controller.lightbar_available());
        assert!(matches!(
            controller.apply_lightbar(10, RgbColor::default()),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn no_hardware_degrades_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let controller = fixture_controller(dir.path());
        assert_eq!(controller.keyboard_capabilities().kind, KeyboardKind::None);
        assert!(!controller.keyboard_available());
        assert!(matches!(
            controller.apply_keyboard(10, &[]),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn brightness_is_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        white_fixture(dir.path());
        write(
            dir.path(),
            "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/brightness",
            "2\n",
        );
        let controller = fixture_controller(dir.path());
        controller.apply_keyboard(2, &[]).unwrap();
        assert_eq!(
            read(
                dir.path(),
                "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/brightness"
            ),
            "2"
        );
    }

    #[test]
    fn rgb_index_parsing() {
        assert_eq!(rgb_index("rgb:kbd_backlight"), 0);
        assert_eq!(rgb_index("rgb:kbd_backlight_3"), 3);
        assert_eq!(rgb_index("something_else"), u32::MAX);
    }

    #[test]
    fn color_formatting() {
        assert_eq!(RgbColor::new(1, 2, 3).as_multi_intensity(), "1 2 3");
    }
}
