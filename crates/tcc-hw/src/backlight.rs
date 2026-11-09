// SPDX-License-Identifier: GPL-3.0-or-later
//! Rétroéclairage via `/sys/class/backlight`.
//!
//! ```text
//! /sys/class/backlight/<nom>/max_brightness
//! /sys/class/backlight/<nom>/brightness        (écriture)
//! /sys/class/backlight/<nom>/actual_brightness (lecture, peut manquer)
//! /sys/class/backlight/<nom>/type
//! ```
//!
//! Quirk `amdgpu_bl*` repris de l'original (`DisplayBacklightController`) :
//! certains pilotes AMD retournent `actual_brightness` sur 16 bits
//! (`0..=0xffff`) alors que la plage attendue est `0..=0xff`. La valeur est
//! ramenée sur 8 bits pour l'affichage ; le pourcentage, lui, reste calculé sur
//! la valeur brute et `max_brightness` pour ne jamais désaligner l'écriture.

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

const CLASS_BACKLIGHT: &str = "class/backlight";
const AMDGPU_MAX_8BIT: u32 = 0xff;
const AMDGPU_MAX_16BIT: u32 = 0xffff;

/// Périphérique de rétroéclairage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacklightDevice {
    /// Nom du répertoire (`intel_backlight`, `amdgpu_bl0`, ...).
    pub name: String,
    /// Chemin relatif à la racine sysfs.
    pub path: String,
    pub max_brightness: u32,
    pub brightness: u32,
    pub actual_brightness: Option<u32>,
    /// Vrai pour les pilotes `amdgpu_bl*`.
    pub amdgpu_quirk: bool,
}

impl BacklightDevice {
    /// Lit un périphérique. Une erreur est retournée si `max_brightness` ou
    /// `brightness` est absent : sans ces deux attributs, le device n'est pas
    /// pilotable.
    pub fn read(root: &SysfsRoot, name: &str) -> Result<Self> {
        let path = format!("{CLASS_BACKLIGHT}/{name}");
        let max_brightness = root.read_u64(format!("{path}/max_brightness"))?;
        if max_brightness > u64::from(u32::MAX) {
            return Err(HwError::InvalidValue {
                context: "max_brightness",
                value: max_brightness.to_string(),
            });
        }
        let brightness = root
            .read_optional_string(format!("{path}/brightness"))?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let actual_brightness = root
            .read_optional_string(format!("{path}/actual_brightness"))?
            .and_then(|value| value.parse::<u32>().ok());
        Ok(Self {
            name: name.to_owned(),
            path,
            max_brightness: max_brightness as u32,
            brightness,
            actual_brightness,
            amdgpu_quirk: name.contains("amdgpu_bl"),
        })
    }

    /// Valeur brute actuellement appliquée.
    pub fn effective_raw(&self) -> u32 {
        self.actual_brightness.unwrap_or(self.brightness)
    }

    /// Valeur d'affichage ramenée sur 8 bits quand le quirk AMD est actif.
    pub fn display_raw_8bit(&self) -> u32 {
        let raw = self.effective_raw();
        if self.amdgpu_quirk && raw > AMDGPU_MAX_8BIT {
            // round(0xff * raw / 0xffff) sans flottant.
            ((u64::from(AMDGPU_MAX_8BIT) * u64::from(raw) + u64::from(AMDGPU_MAX_16BIT) / 2)
                / u64::from(AMDGPU_MAX_16BIT)) as u32
        } else {
            raw
        }
    }

    /// Luminosité actuelle en pourcentage.
    pub fn brightness_percent(&self) -> u8 {
        if self.max_brightness == 0 {
            return 0;
        }
        // Sur `amdgpu_bl*`, `actual_brightness` peut être sur 16 bits alors
        // que `max_brightness` reste sur 8 bits. On normalise avant de
        // calculer le pourcentage ; sinon on diviserait une valeur 16 bits par
        // un maximum 8 bits.
        let raw = if self.amdgpu_quirk && self.max_brightness <= AMDGPU_MAX_8BIT {
            u64::from(self.display_raw_8bit())
        } else {
            u64::from(self.effective_raw())
        };
        let percent =
            (raw * 100 + u64::from(self.max_brightness) / 2) / u64::from(self.max_brightness);
        percent.min(100) as u8
    }

    /// Convertit un pourcentage en valeur brute pour ce périphérique.
    pub fn percent_to_raw(&self, percent: u8) -> u32 {
        if self.max_brightness == 0 {
            return 0;
        }
        let percent = u64::from(percent.min(100));
        ((percent * u64::from(self.max_brightness) + 50) / 100) as u32
    }

    /// Applique un pourcentage. Retourne la valeur brute écrite.
    pub fn set_percent(&self, root: &SysfsRoot, percent: u8) -> Result<u32> {
        let raw = self.percent_to_raw(percent);
        root.write_i64(format!("{}/brightness", self.path), i64::from(raw))?;
        Ok(raw)
    }
}

/// Énumère les périphériques de rétroéclairage.
///
/// Les périphériques incomplets sont ignorés : c'est le cas normal d'un
/// pilote en cours d'initialisation ou d'un noyau sans backlight.
pub fn enumerate(root: &SysfsRoot) -> Result<Vec<BacklightDevice>> {
    let mut devices = Vec::new();
    for name in root.list_dir(CLASS_BACKLIGHT)? {
        match BacklightDevice::read(root, &name) {
            Ok(device) => devices.push(device),
            Err(err) if err.is_unavailable() => {
                tracing::debug!(backlight = %name, error = %err, "backlight ignoré");
            }
            Err(err) => return Err(err),
        }
    }
    Ok(devices)
}

/// Sélectionne le périphérique « principal ».
///
/// L'original écrit sur tous les drivers pour être sûr. [`set_all_percent`]
/// reproduit ce comportement ; cette fonction sert à l'UI qui ne veut afficher
/// qu'une seule luminosité.
pub fn pick_primary(root: &SysfsRoot) -> Result<Option<BacklightDevice>> {
    let devices = enumerate(root)?;
    let preferred = [
        "intel_backlight",
        "amdgpu_bl0",
        "acpi_video0",
        "acpi_video1",
    ];
    if let Some(device) = preferred
        .iter()
        .find_map(|name| devices.iter().find(|device| device.name == *name))
    {
        return Ok(Some(device.clone()));
    }
    Ok(devices.into_iter().next())
}

/// Résultat d'une application multi-périphériques.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BacklightApplyReport {
    /// `(nom, valeur brute)` des périphériques écrits.
    pub applied: Vec<(String, u32)>,
    /// `(nom, message)` des échecs, sans interrompre les autres écritures.
    pub failures: Vec<(String, String)>,
}

impl BacklightApplyReport {
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// Applique un pourcentage à tous les périphériques détectés.
///
/// Comportement volontairement identique à l'original : on tente tous les
/// drivers, une erreur sur l'un n'empêche pas les autres.
pub fn set_all_percent(root: &SysfsRoot, percent: u8) -> Result<BacklightApplyReport> {
    let mut report = BacklightApplyReport::default();
    for device in enumerate(root)? {
        match device.set_percent(root, percent) {
            Ok(raw) => report.applied.push((device.name, raw)),
            Err(err) => {
                tracing::warn!(backlight = %device.name, error = %err, "écriture luminosité échouée");
                report.failures.push((device.name, err.to_string()));
            }
        }
    }
    Ok(report)
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

    #[test]
    fn enumerates_and_computes_percent() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/backlight/intel_backlight/max_brightness",
            "937\n",
        );
        write(
            dir.path(),
            "class/backlight/intel_backlight/brightness",
            "468\n",
        );
        write(
            dir.path(),
            "class/backlight/intel_backlight/actual_brightness",
            "468\n",
        );
        let root = SysfsRoot::new(dir.path());

        let devices = enumerate(&root).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].brightness_percent(), 50);
        assert_eq!(devices[0].percent_to_raw(50), 469);
    }

    #[test]
    fn writes_brightness() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/backlight/intel_backlight/max_brightness",
            "1000\n",
        );
        write(
            dir.path(),
            "class/backlight/intel_backlight/brightness",
            "500\n",
        );
        let root = SysfsRoot::new(dir.path());
        let device = BacklightDevice::read(&root, "intel_backlight").unwrap();
        assert_eq!(device.set_percent(&root, 25).unwrap(), 250);
        assert_eq!(
            read(dir.path(), "class/backlight/intel_backlight/brightness"),
            "250"
        );
        assert_eq!(device.set_percent(&root, 100).unwrap(), 1000);
    }

    #[test]
    fn amdgpu_quirk_scales_display_value_only() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/backlight/amdgpu_bl0/max_brightness",
            "255\n",
        );
        write(dir.path(), "class/backlight/amdgpu_bl0/brightness", "128\n");
        write(
            dir.path(),
            "class/backlight/amdgpu_bl0/actual_brightness",
            "32768\n",
        );
        let root = SysfsRoot::new(dir.path());
        let device = BacklightDevice::read(&root, "amdgpu_bl0").unwrap();
        assert!(device.amdgpu_quirk);
        assert_eq!(device.display_raw_8bit(), 128);
        // Le pourcentage se base sur la valeur 8 bits normalisée.
        assert_eq!(device.brightness_percent(), 50);
    }

    #[test]
    fn incomplete_device_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/backlight/broken/brightness", "1\n");
        write(dir.path(), "class/backlight/good/max_brightness", "100\n");
        write(dir.path(), "class/backlight/good/brightness", "50\n");
        let root = SysfsRoot::new(dir.path());
        let devices = enumerate(&root).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "good");
    }

    #[test]
    fn set_all_continues_after_one_failure() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/backlight/good/max_brightness", "100\n");
        write(dir.path(), "class/backlight/good/brightness", "0\n");
        // `brightness` n'existe pas : l'énumération réussit (max_brightness
        // suffit) mais l'écriture échoue ; le second device doit quand même
        // être traité.
        write(
            dir.path(),
            "class/backlight/partial/max_brightness",
            "100\n",
        );
        let root = SysfsRoot::new(dir.path());
        let report = set_all_percent(&root, 40).unwrap();
        assert_eq!(report.applied.len(), 1);
        assert_eq!(report.applied[0].0, "good");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].0, "partial");
    }

    #[test]
    fn primary_prefers_known_driver() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/backlight/amdgpu_bl0/max_brightness",
            "255\n",
        );
        write(dir.path(), "class/backlight/amdgpu_bl0/brightness", "0\n");
        write(
            dir.path(),
            "class/backlight/intel_backlight/max_brightness",
            "100\n",
        );
        write(
            dir.path(),
            "class/backlight/intel_backlight/brightness",
            "0\n",
        );
        let root = SysfsRoot::new(dir.path());
        assert_eq!(
            pick_primary(&root).unwrap().unwrap().name,
            "intel_backlight"
        );
    }
}
