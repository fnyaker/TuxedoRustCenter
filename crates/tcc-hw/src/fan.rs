// SPDX-License-Identifier: GPL-3.0-or-later
//! Ventilateurs : hwmon `tuxi` / `pwm` et `/dev/tuxedo_io`.
//!
//! Priorité identique à `FanControlWorker` de l'original :
//!
//! 1. **`tuxi`** — hwmon `tuxedo_tuxi_sensors` ;
//! 2. **`pwm`** — hwmon `tuxedo` ;
//! 3. **`tuxedo-io`** — `/dev/tuxedo_io` (Clevo/Uniwill).
//!
//! ```text
//! /sys/class/hwmon/hwmonN/name
//! /sys/class/hwmon/hwmonN/fanM_input     (tr/min)
//! /sys/class/hwmon/hwmonN/fanM_max       (tr/min, optionnel)
//! /sys/class/hwmon/hwmonN/fanM_label     (cpu, gpu1, ...)
//! /sys/class/hwmon/hwmonN/tempM_input    (millidegrés)
//! /sys/class/hwmon/hwmonN/tempM_label
//! /sys/bus/platform/devices/tuxedo_fan_control/fanM_pwm
//! /sys/bus/platform/devices/tuxedo_fan_control/fanM_pwm_enable
//! ```
//!
//! `pwm_enable` : `1` = manuel, `2` = automatique (contrat hwmon standard).
//!
//! Ce module ne contient **aucune** boucle de régulation : les courbes, la
//! temporisation et l'hystérésis appartiennent à `tcc-core` et `tcc-daemon`.
//! Il expose uniquement des lectures et des écritures validées, derrière le
//! trait [`FanControl`] afin de pouvoir injecter un backend factice en test.
//!
//! # Dégradation
//!
//! - Aucun backend disponible → [`detect`] retourne `None` ; l'appelant
//!   n'écrit rien et expose « non disponible ».
//! - Un capteur manquant retourne [`HwError::SensorUnavailable`], jamais une
//!   valeur inventée : le daemon décide de conserver la vitesse courante.
//! - L'original arrêtait le daemon (`process.exit(0)`) quand un label de
//!   ventilateur ne correspondait à aucun capteur ; ici on journalise et on
//!   retombe sur le premier capteur disponible.
//!
//! # Quirk Sirius
//!
//! Les Sirius 16 Gen1/Gen2 exposent deux chemins hwmon. L'original n'utilise
//! que `tuxi` pour éviter une double écriture et la charge CPU associée
//! (`FanControlTuxi.checkAvailable`). Le backend `pwm` est donc ignoré sur ces
//! SKU (`SIRIUS1601`, `SIRIUS1602`).

use std::sync::Arc;

use crate::dmi::DmiInfo;
use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;
use crate::tuxedo_io::{Interface, TuxedoIo};

/// Répertoire des classes hwmon.
const CLASS_HWMON: &str = "class/hwmon";
/// Répertoire des attributs PWM TUXEDO (module `tuxedo_fan_control`).
const FAN_CONTROL_PATH: &str = "bus/platform/devices/tuxedo_fan_control";
/// Nom hwmon du module `tuxedo_tuxi_sensors`.
const TUXI_HWMON_NAME: &str = "tuxedo_tuxi_sensors";
/// Nom hwmon du module historique `tuxedo`.
const PWM_HWMON_NAME: &str = "tuxedo";
/// Valeur PWM maximale du contrat hwmon.
const PWM_MAX: u32 = 255;
/// SKU des machines où seul `tuxi` doit être utilisé.
const SIRIUS_SKUS: [&str; 2] = ["SIRIUS1601", "SIRIUS1602"];

/// Backend de contrôle des ventilateurs sélectionné.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanBackendKind {
    /// hwmon `tuxedo_tuxi_sensors`.
    Tuxi,
    /// hwmon `tuxedo`.
    Pwm,
    /// `/dev/tuxedo_io`.
    TuxedoIo,
}

impl FanBackendKind {
    /// Nom stable exposé à l'UI et aux logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tuxi => "tuxi",
            Self::Pwm => "pwm",
            Self::TuxedoIo => "tuxedo-io",
        }
    }
}

/// Rôle logique d'un ventilateur.
///
/// Le crate ne dépend pas de `tcc-core` : le daemon convertit vers
/// `tcc_core::fan::FanTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanTarget {
    /// Courbe CPU.
    Cpu,
    /// Courbe GPU.
    Gpu,
}

impl FanTarget {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// Contrôle des ventilateurs, indépendant du backend.
///
/// Toutes les méthodes prennent un index **0-based**. Un index hors plage
/// retourne [`HwError::OutOfRange`] ; un backend sans écriture retourne
/// [`HwError::NotSupported`].
pub trait FanControl: std::fmt::Debug + Send + Sync {
    /// Backend effectivement sélectionné.
    fn backend(&self) -> FanBackendKind;

    /// Nombre de ventilateurs exposés.
    fn fan_count(&self) -> u8;

    /// Libellé du ventilateur (`cpu`, `gpu`, `fan1`, ...).
    fn fan_label(&self, fan: u8) -> String;

    /// Rôle logique utilisé pour choisir la courbe.
    fn fan_target(&self, fan: u8) -> FanTarget;

    /// Température associée au ventilateur, en degrés Celsius.
    fn fan_temperature(&self, fan: u8) -> Result<i32>;

    /// Vitesse actuelle en pourcentage (`0..=100`).
    fn fan_speed_percent(&self, fan: u8) -> Result<u8>;

    /// Fixe la vitesse d'un ventilateur en pourcentage (`0..=100`).
    fn set_fan_speed_percent(&self, fan: u8, percent: u8) -> Result<()>;

    /// Active ou désactive le mode manuel.
    ///
    /// Sur `tuxedo-io`, seules les machines Uniwill exposent un commutateur
    /// explicite ; Clevo est un no-op (l'écriture de vitesse suffit), comme
    /// dans l'original.
    fn set_manual_mode(&self, manual: bool) -> Result<()>;

    /// Rend tous les ventilateurs au firmware.
    fn set_fans_auto(&self) -> Result<()>;

    /// Vitesse minimale garantie par le matériel, en pourcentage.
    fn fans_min_speed(&self) -> u8;

    /// Vrai si l'arrêt complet des ventilateurs est supporté.
    fn fans_off_available(&self) -> bool;

    /// Vrai si le backend accepte les écritures de vitesse.
    fn has_write_support(&self) -> bool;

    /// Vrai si l'EC dérive et qu'il faut réécrire la vitesse à chaque tick.
    ///
    /// L'original réécrit systématiquement sur Uniwill
    /// (`FanControlWorker.writeFanSpeed`, quirk documenté par TUXEDO).
    fn writes_every_tick(&self) -> bool {
        false
    }
}

/// Sélectionne le backend selon la priorité de l'original.
///
/// Ouvre `/dev/tuxedo_io` si nécessaire. Aucun panic : une machine non-TUXEDO
/// sans hwmon et sans module retourne `None`.
#[must_use]
pub fn detect(root: &SysfsRoot) -> Option<Arc<dyn FanControl>> {
    let io = match TuxedoIo::open() {
        Ok(io) => Some(io),
        Err(error) => {
            if !error.is_unavailable() {
                tracing::debug!(%error, "tuxedo-io indisponible pour les ventilateurs");
            }
            None
        }
    };
    detect_with(root, io)
}

/// Comme [`detect`], avec une interface `tuxedo-io` déjà construite.
///
/// Sert aux tests et aux simulations : `None` désactive le fallback ioctl.
#[must_use]
pub fn detect_with(root: &SysfsRoot, io: Option<TuxedoIo>) -> Option<Arc<dyn FanControl>> {
    match HwmonFanControl::find(root, FanBackendKind::Tuxi) {
        Ok(Some(control)) => {
            tracing::debug!(fans = control.fan_count(), "ventilateurs : backend tuxi");
            return Some(Arc::new(control));
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "détection du backend hwmon tuxi impossible"),
    }

    if !is_sirius(root) {
        match HwmonFanControl::find(root, FanBackendKind::Pwm) {
            Ok(Some(control)) => {
                tracing::debug!(fans = control.fan_count(), "ventilateurs : backend pwm");
                return Some(Arc::new(control));
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "détection du backend hwmon pwm impossible"),
        }
    }

    match io {
        Some(io) => {
            tracing::debug!(
                interface = io.interface().as_str(),
                fans = io.fan_count(),
                "ventilateurs : backend tuxedo-io"
            );
            Some(Arc::new(TuxedoIoFanControl::new(io)))
        }
        None => None,
    }
}

/// Vrai si la machine est un Sirius 16 Gen1/Gen2 (quirk hwmon).
fn is_sirius(root: &SysfsRoot) -> bool {
    let Ok(dmi) = DmiInfo::read(root) else {
        return false;
    };
    dmi.sku()
        .is_some_and(|sku| SIRIUS_SKUS.contains(&sku.trim()))
}

// ---------------------------------------------------------------------------
// Backend hwmon (tuxi / pwm)
// ---------------------------------------------------------------------------

/// Capteur de température hwmon.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TempSensor {
    label: String,
    path: String,
}

/// Ventilateur hwmon et ses attributs associés.
#[derive(Debug, Clone)]
struct HwmonFan {
    label: String,
    input_path: String,
    max_path: Option<String>,
    temperature: Option<TempSensor>,
    pwm_path: Option<String>,
    pwm_enable_path: Option<String>,
}

/// Backend hwmon (`tuxedo_tuxi_sensors` ou `tuxedo`).
#[derive(Debug)]
pub struct HwmonFanControl {
    kind: FanBackendKind,
    fans: Vec<HwmonFan>,
    root: SysfsRoot,
    writable: bool,
}

impl HwmonFanControl {
    /// Cherche le hwmon correspondant au backend et construit le contrôle.
    fn find(root: &SysfsRoot, kind: FanBackendKind) -> Result<Option<Self>> {
        let name = match kind {
            FanBackendKind::Tuxi => TUXI_HWMON_NAME,
            FanBackendKind::Pwm => PWM_HWMON_NAME,
            FanBackendKind::TuxedoIo => return Ok(None),
        };
        let Some(hwmon) = find_hwmon(root, name)? else {
            return Ok(None);
        };
        Self::build(root, kind, hwmon)
    }

    fn build(root: &SysfsRoot, kind: FanBackendKind, hwmon: String) -> Result<Option<Self>> {
        let indices = numbered_attributes(root, &hwmon, "fan", "_input")?;
        if indices.is_empty() {
            tracing::debug!(hwmon = %hwmon, backend = kind.as_str(), "hwmon sans ventilateur");
            return Ok(None);
        }

        let temps = read_temperatures(root, &hwmon)?;
        // Les attributs PWM vivent dans le module `tuxedo_fan_control`, pas
        // dans le hwmon : un fichier absent signifie « lecture seule ».
        let controls = root.list_dir(FAN_CONTROL_PATH).unwrap_or_default();

        let mut fans = Vec::with_capacity(indices.len());
        for index in indices {
            let label = root
                .read_optional_string(format!("{hwmon}/fan{index}_label"))?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("fan{index}"));
            let max_path = root
                .is_file(format!("{hwmon}/fan{index}_max"))
                .then(|| format!("{hwmon}/fan{index}_max"));
            let temperature = match_temperature(&label, &temps);
            let pwm_name = format!("fan{index}_pwm");
            let pwm_enable_name = format!("fan{index}_pwm_enable");
            fans.push(HwmonFan {
                label,
                input_path: format!("{hwmon}/fan{index}_input"),
                max_path,
                temperature,
                pwm_path: controls
                    .contains(&pwm_name)
                    .then(|| format!("{FAN_CONTROL_PATH}/{pwm_name}")),
                pwm_enable_path: controls
                    .contains(&pwm_enable_name)
                    .then(|| format!("{FAN_CONTROL_PATH}/{pwm_enable_name}")),
            });
        }

        let writable = fans.iter().any(|fan| fan.pwm_enable_path.is_some());
        Ok(Some(Self {
            kind,
            fans,
            root: root.clone(),
            writable,
        }))
    }

    fn fan(&self, fan: u8) -> Result<&HwmonFan> {
        self.fans
            .get(usize::from(fan))
            .ok_or_else(|| HwError::OutOfRange {
                context: "index ventilateur",
                value: i64::from(fan),
                min: 0,
                max: self.fans.len().saturating_sub(1) as i64,
            })
    }
}

impl FanControl for HwmonFanControl {
    fn backend(&self) -> FanBackendKind {
        self.kind
    }

    fn fan_count(&self) -> u8 {
        self.fans.len().min(usize::from(u8::MAX)) as u8
    }

    fn fan_label(&self, fan: u8) -> String {
        self.fans
            .get(usize::from(fan))
            .map(|slot| slot.label.clone())
            .unwrap_or_else(|| format!("fan{}", u16::from(fan) + 1))
    }

    fn fan_target(&self, fan: u8) -> FanTarget {
        // Un label inconnu est traité comme un ventilateur CPU, comme
        // `FanControlHwmon.mapLogicToFans`.
        let label = self.fan_label(fan).to_ascii_lowercase();
        if label.contains("gpu") {
            FanTarget::Gpu
        } else {
            FanTarget::Cpu
        }
    }

    fn fan_temperature(&self, fan: u8) -> Result<i32> {
        let slot = self.fan(fan)?;
        let Some(sensor) = &slot.temperature else {
            return Err(HwError::SensorUnavailable {
                sensor: "température ventilateur hwmon",
            });
        };
        let millidegrees = self.root.read_i64(&sensor.path)?;
        // L'original considère une valeur nulle comme un capteur absent
        // (`if (readValue)` dans `FanControlHwmon.getFanTemperature`).
        if millidegrees <= 0 {
            return Err(HwError::SensorUnavailable {
                sensor: "température ventilateur hwmon",
            });
        }
        let celsius = millidegrees / 1000;
        i32::try_from(celsius).map_err(|_| HwError::InvalidValue {
            context: "température ventilateur hwmon",
            value: celsius.to_string(),
        })
    }

    fn fan_speed_percent(&self, fan: u8) -> Result<u8> {
        let slot = self.fan(fan)?;
        let Some(max_path) = &slot.max_path else {
            return Err(HwError::SensorUnavailable {
                sensor: "vitesse maximale ventilateur hwmon",
            });
        };
        let input = self.root.read_u64(&slot.input_path)?;
        let max = self.root.read_u64(max_path)?;
        if max == 0 {
            return Err(HwError::SensorUnavailable {
                sensor: "vitesse maximale ventilateur hwmon",
            });
        }
        let input = input.min(max);
        let percent = (input * 100 + max / 2) / max;
        Ok(percent.min(100) as u8)
    }

    fn set_fan_speed_percent(&self, fan: u8, percent: u8) -> Result<()> {
        if percent > 100 {
            return Err(HwError::OutOfRange {
                context: "vitesse ventilateur",
                value: i64::from(percent),
                min: 0,
                max: 100,
            });
        }
        let slot = self.fan(fan)?;
        let Some(path) = &slot.pwm_path else {
            return Err(HwError::NotSupported("écriture PWM ventilateur"));
        };
        let raw = (u32::from(percent) * PWM_MAX + 50) / 100;
        self.root.write_i64(path, i64::from(raw))
    }

    fn set_manual_mode(&self, manual: bool) -> Result<()> {
        self.write_pwm_enable(if manual { 1 } else { 2 })
    }

    fn set_fans_auto(&self) -> Result<()> {
        self.write_pwm_enable(2)
    }

    fn fans_min_speed(&self) -> u8 {
        // Le contrat hwmon n'expose pas de vitesse minimale : l'original ne
        // l'applique pas non plus pour ce backend (`_fansMinSpeedHWLimit = 0`).
        0
    }

    fn fans_off_available(&self) -> bool {
        true
    }

    fn has_write_support(&self) -> bool {
        self.writable
    }
}

impl HwmonFanControl {
    fn write_pwm_enable(&self, value: i64) -> Result<()> {
        let mut written = false;
        let mut last_error = None;
        for fan in &self.fans {
            let Some(path) = &fan.pwm_enable_path else {
                continue;
            };
            match self.root.write_i64(path, value) {
                Ok(()) => written = true,
                Err(error) => {
                    tracing::warn!(path = %path, %error, "écriture pwm_enable échouée");
                    last_error = Some(error);
                }
            }
        }
        if written {
            return Ok(());
        }
        match last_error {
            Some(error) => Err(error),
            None => Err(HwError::NotSupported("pwm_enable ventilateur")),
        }
    }
}

/// Cherche `hwmonN/name == name` et retourne le chemin relatif.
fn find_hwmon(root: &SysfsRoot, name: &str) -> Result<Option<String>> {
    for entry in root.list_dir(CLASS_HWMON)? {
        let path = format!("{CLASS_HWMON}/{entry}");
        if !root.is_file(format!("{path}/name")) {
            continue;
        }
        let found = root.read_optional_string(format!("{path}/name"))?;
        if found.as_deref() == Some(name) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Indices des attributs numérotés `prefixNNN_suffix`, triés.
fn numbered_attributes(
    root: &SysfsRoot,
    directory: &str,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<u8>> {
    let mut indices = Vec::new();
    for entry in root.list_dir(directory)? {
        let Some(rest) = entry.strip_prefix(prefix) else {
            continue;
        };
        let Some(number) = rest.strip_suffix(suffix) else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        match number.parse::<u8>() {
            Ok(index) if index > 0 => indices.push(index),
            _ => {
                tracing::debug!(entry = %entry, "attribut hwmon ignoré (index hors plage)");
            }
        }
    }
    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

/// Lit les capteurs de température d'un hwmon.
fn read_temperatures(root: &SysfsRoot, hwmon: &str) -> Result<Vec<TempSensor>> {
    let mut sensors = Vec::new();
    for index in numbered_attributes(root, hwmon, "temp", "_input")? {
        let label = root
            .read_optional_string(format!("{hwmon}/temp{index}_label"))?
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("temp{index}"));
        sensors.push(TempSensor {
            label,
            path: format!("{hwmon}/temp{index}_input"),
        });
    }
    Ok(sensors)
}

/// Associe un ventilateur à un capteur de température.
///
/// Reprend `FanControlHwmon.matchLabels` : le label du capteur doit commencer
/// par le label du ventilateur, les suffixes numériques `cpuN` étant
/// normalisés en `cpu`. À défaut, le premier capteur est utilisé (l'original
/// arrêtait le daemon ; on préfère une régulation approximative à un arrêt).
fn match_temperature(label: &str, sensors: &[TempSensor]) -> Option<TempSensor> {
    let normalized = normalize_label(label);
    let matched = sensors
        .iter()
        .find(|sensor| sensor.label.to_ascii_lowercase().starts_with(&normalized));
    match matched {
        Some(sensor) => Some(sensor.clone()),
        None => {
            let fallback = sensors.first().cloned();
            if fallback.is_some() {
                tracing::debug!(
                    fan_label = label,
                    "aucun capteur de température ne correspond, utilisation du premier"
                );
            }
            fallback
        }
    }
}

/// Normalise `cpu0`/`cpu1` en `cpu` (insensible à la casse), comme l'original.
fn normalize_label(label: &str) -> String {
    let lower = label.trim().to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("cpu") {
        if rest.is_empty() || rest.bytes().all(|byte| byte.is_ascii_digit()) {
            return "cpu".to_owned();
        }
    }
    lower
}

// ---------------------------------------------------------------------------
// Backend tuxedo-io
// ---------------------------------------------------------------------------

/// Backend `/dev/tuxedo_io`.
#[derive(Debug)]
pub struct TuxedoIoFanControl {
    io: TuxedoIo,
    min_speed: u8,
    off_available: bool,
    /// Nombre de capteurs consécutifs exploitables, utilisé pour le mapping
    /// CPU/GPU de l'original (`getNumberTempsAvailable`).
    temperature_count: u8,
}

impl TuxedoIoFanControl {
    /// Construit le backend autour d'une interface déjà ouverte.
    #[must_use]
    pub fn new(io: TuxedoIo) -> Self {
        let min_speed = io.fans_min_speed().unwrap_or_else(|error| {
            tracing::debug!(%error, "vitesse minimale des ventilateurs inconnue, 20 % par défaut");
            20
        });
        let off_available = io.fans_off_available().unwrap_or(true);
        let mut temperature_count = 0u8;
        for index in 0..io.fan_count() {
            if io.fan_temperature(index).is_ok() {
                temperature_count = temperature_count.saturating_add(1);
            } else {
                break;
            }
        }
        Self {
            io,
            min_speed,
            off_available,
            temperature_count,
        }
    }

    fn validate(&self, fan: u8) -> Result<()> {
        if fan >= self.io.fan_count() {
            return Err(HwError::OutOfRange {
                context: "index ventilateur",
                value: i64::from(fan),
                min: 0,
                max: i64::from(self.io.fan_count()) - 1,
            });
        }
        Ok(())
    }
}

impl FanControl for TuxedoIoFanControl {
    fn backend(&self) -> FanBackendKind {
        FanBackendKind::TuxedoIo
    }

    fn fan_count(&self) -> u8 {
        self.io.fan_count()
    }

    fn fan_label(&self, fan: u8) -> String {
        format!("fan{}", u16::from(fan) + 1)
    }

    fn fan_target(&self, fan: u8) -> FanTarget {
        // `FanControlTuxedoIO.mapLogicToFans` : 1 = CPU, 2 = GPU seulement
        // lorsqu'un second capteur existe, sinon CPU, 3 = GPU.
        match fan {
            0 => FanTarget::Cpu,
            1 if self.temperature_count >= 2 => FanTarget::Gpu,
            1 => FanTarget::Cpu,
            _ => FanTarget::Gpu,
        }
    }

    fn fan_temperature(&self, fan: u8) -> Result<i32> {
        self.validate(fan)?;
        self.io.fan_temperature(fan)
    }

    fn fan_speed_percent(&self, fan: u8) -> Result<u8> {
        self.validate(fan)?;
        self.io.fan_speed_percent(fan)
    }

    fn set_fan_speed_percent(&self, fan: u8, percent: u8) -> Result<()> {
        self.validate(fan)?;
        self.io.set_fan_speed_percent(fan, percent)
    }

    fn set_manual_mode(&self, manual: bool) -> Result<()> {
        self.io.set_mode_enable(manual)
    }

    fn set_fans_auto(&self) -> Result<()> {
        self.io.set_fans_auto()
    }

    fn fans_min_speed(&self) -> u8 {
        self.min_speed
    }

    fn fans_off_available(&self) -> bool {
        self.off_available
    }

    fn has_write_support(&self) -> bool {
        // Une interface Clevo/Uniwill détectée expose toujours l'écriture.
        true
    }

    fn writes_every_tick(&self) -> bool {
        // Quirk TUXEDO : sur Uniwill, l'EC dérive après quelques heures et la
        // vitesse change après écriture ; l'original réécrit à chaque tick.
        self.io.interface() == Interface::Uniwill
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuxedo_io::fake::FakeTransport;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, contents).expect("write");
    }

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .expect("read")
            .trim()
            .to_owned()
    }

    /// hwmon `tuxi` avec deux ventilateurs et deux capteurs.
    fn tuxi_fixture(root: &Path) {
        write(root, "class/hwmon/hwmon0/name", "tuxedo_tuxi_sensors\n");
        write(root, "class/hwmon/hwmon0/fan1_input", "1500\n");
        write(root, "class/hwmon/hwmon0/fan1_max", "3000\n");
        write(root, "class/hwmon/hwmon0/fan1_label", "cpu\n");
        write(root, "class/hwmon/hwmon0/fan2_input", "900\n");
        write(root, "class/hwmon/hwmon0/fan2_max", "3000\n");
        write(root, "class/hwmon/hwmon0/fan2_label", "gpu1\n");
        write(root, "class/hwmon/hwmon0/temp1_input", "45000\n");
        write(root, "class/hwmon/hwmon0/temp1_label", "cpu\n");
        write(root, "class/hwmon/hwmon0/temp2_input", "55000\n");
        write(root, "class/hwmon/hwmon0/temp2_label", "gpu1\n");
        write(
            root,
            "bus/platform/devices/tuxedo_fan_control/fan1_pwm",
            "0\n",
        );
        write(
            root,
            "bus/platform/devices/tuxedo_fan_control/fan1_pwm_enable",
            "2\n",
        );
        write(
            root,
            "bus/platform/devices/tuxedo_fan_control/fan2_pwm",
            "0\n",
        );
        write(
            root,
            "bus/platform/devices/tuxedo_fan_control/fan2_pwm_enable",
            "2\n",
        );
    }

    #[test]
    fn detects_tuxi_and_reads_sensors() {
        let dir = tempfile::tempdir().expect("tempdir");
        tuxi_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");

        assert_eq!(control.backend(), FanBackendKind::Tuxi);
        assert_eq!(control.fan_count(), 2);
        assert_eq!(control.fan_label(0), "cpu");
        assert_eq!(control.fan_label(1), "gpu1");
        assert_eq!(control.fan_target(0), FanTarget::Cpu);
        assert_eq!(control.fan_target(1), FanTarget::Gpu);
        assert_eq!(control.fan_temperature(0).expect("temp cpu"), 45);
        assert_eq!(control.fan_temperature(1).expect("temp gpu"), 55);
        assert_eq!(control.fan_speed_percent(0).expect("speed"), 50);
        assert!(control.has_write_support());
        assert_eq!(control.fans_min_speed(), 0);
        assert!(control.fans_off_available());
    }

    #[test]
    fn writes_pwm_and_enable() {
        let dir = tempfile::tempdir().expect("tempdir");
        tuxi_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");

        control.set_manual_mode(true).expect("manuel");
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_fan_control/fan1_pwm_enable"
            ),
            "1"
        );
        control.set_fan_speed_percent(0, 50).expect("vitesse");
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_fan_control/fan1_pwm"
            ),
            "128"
        );
        assert!(matches!(
            control.set_fan_speed_percent(0, 101),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            control.set_fan_speed_percent(9, 50),
            Err(HwError::OutOfRange { .. })
        ));
        control.set_fans_auto().expect("auto");
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_fan_control/fan2_pwm_enable"
            ),
            "2"
        );
    }

    #[test]
    fn read_only_hwmon_is_supported() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "class/hwmon/hwmon0/name",
            "tuxedo_tuxi_sensors\n",
        );
        write(dir.path(), "class/hwmon/hwmon0/fan1_input", "1000\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_max", "2000\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_input", "40000\n");
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");

        assert!(!control.has_write_support());
        assert!(matches!(
            control.set_fan_speed_percent(0, 50),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            control.set_fans_auto(),
            Err(HwError::NotSupported(_))
        ));
        assert_eq!(control.fan_temperature(0).expect("temp"), 40);
    }

    #[test]
    fn prefers_tuxi_over_pwm() {
        let dir = tempfile::tempdir().expect("tempdir");
        tuxi_fixture(dir.path());
        // Second hwmon `tuxedo` avec un seul ventilateur.
        write(dir.path(), "class/hwmon/hwmon1/name", "tuxedo\n");
        write(dir.path(), "class/hwmon/hwmon1/fan1_input", "0\n");
        write(dir.path(), "class/hwmon/hwmon1/fan1_max", "255\n");
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");
        assert_eq!(control.backend(), FanBackendKind::Tuxi);
        assert_eq!(control.fan_count(), 2);
    }

    #[test]
    fn falls_back_to_pwm_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "class/hwmon/hwmon0/name", "tuxedo\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_input", "255\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_max", "255\n");
        let root = SysfsRoot::new(dir.path());
        assert_eq!(
            detect_with(&root, None).expect("backend").backend(),
            FanBackendKind::Pwm
        );
    }

    #[test]
    fn sirius_skips_pwm_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "class/dmi/id/product_sku", "SIRIUS1602\n");
        write(dir.path(), "class/hwmon/hwmon0/name", "tuxedo\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_input", "255\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_max", "255\n");
        let root = SysfsRoot::new(dir.path());
        assert!(detect_with(&root, None).is_none());
    }

    #[test]
    fn missing_temperature_falls_back_to_first_sensor() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "class/hwmon/hwmon0/name",
            "tuxedo_tuxi_sensors\n",
        );
        write(dir.path(), "class/hwmon/hwmon0/fan1_input", "1000\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_max", "2000\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_label", "gpu1\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_input", "38000\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_label", "cpu\n");
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");
        assert_eq!(control.fan_temperature(0).expect("temp"), 38);
    }

    #[test]
    fn zero_temperature_is_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "class/hwmon/hwmon0/name",
            "tuxedo_tuxi_sensors\n",
        );
        write(dir.path(), "class/hwmon/hwmon0/fan1_input", "1000\n");
        write(dir.path(), "class/hwmon/hwmon0/fan1_max", "2000\n");
        write(dir.path(), "class/hwmon/hwmon0/temp1_input", "0\n");
        let root = SysfsRoot::new(dir.path());
        let control = detect_with(&root, None).expect("backend");
        assert!(matches!(
            control.fan_temperature(0),
            Err(HwError::SensorUnavailable { .. })
        ));
    }

    #[test]
    fn tuxedo_io_backend_maps_and_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = SysfsRoot::new(dir.path());
        let io = TuxedoIo::with_transport(Box::new(FakeTransport::uniwill())).expect("io");
        let control = detect_with(&root, Some(io)).expect("backend");

        assert_eq!(control.backend(), FanBackendKind::TuxedoIo);
        assert_eq!(control.fan_count(), 2);
        assert_eq!(control.fan_label(0), "fan1");
        assert_eq!(control.fan_target(0), FanTarget::Cpu);
        assert_eq!(control.fan_target(1), FanTarget::Gpu);
        assert_eq!(control.fan_temperature(0).expect("temp"), 45);
        assert_eq!(control.fans_min_speed(), 20);
        assert!(control.fans_off_available());
        assert!(control.writes_every_tick());
        assert!(control.has_write_support());

        control.set_manual_mode(false).expect("mode");
        control.set_fan_speed_percent(1, 50).expect("vitesse");
        assert_eq!(control.fan_speed_percent(1).expect("lecture"), 50);
        assert!(matches!(
            control.set_fan_speed_percent(2, 10),
            Err(HwError::OutOfRange { .. })
        ));
    }

    #[test]
    fn clevo_backend_does_not_rewrite_every_tick() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = SysfsRoot::new(dir.path());
        let io = TuxedoIo::with_transport(Box::new(FakeTransport::clevo())).expect("io");
        let control = detect_with(&root, Some(io)).expect("backend");
        assert_eq!(control.fan_count(), 3);
        assert!(!control.writes_every_tick());
    }

    #[test]
    fn no_hardware_means_no_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = SysfsRoot::new(dir.path());
        assert!(detect_with(&root, None).is_none());
        // En production, l'absence de /dev/tuxedo_io ne doit pas paniquer.
        let _ = detect(&root);
    }

    #[test]
    fn label_normalization_matches_original() {
        assert_eq!(normalize_label("CPU0"), "cpu");
        assert_eq!(normalize_label("cpu"), "cpu");
        assert_eq!(normalize_label("gpu1"), "gpu1");
        assert_eq!(normalize_label(""), "");
    }

    #[test]
    fn numbered_attributes_ignores_unrelated_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "hwmon/fan1_input", "1\n");
        write(dir.path(), "hwmon/fan2_input", "1\n");
        write(dir.path(), "hwmon/fanx_input", "1\n");
        write(dir.path(), "hwmon/fan3_min", "1\n");
        let root = SysfsRoot::new(dir.path());
        assert_eq!(
            numbered_attributes(&root, "hwmon", "fan", "_input").expect("liste"),
            vec![1, 2]
        );
    }
}
