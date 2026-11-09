// SPDX-License-Identifier: GPL-3.0-or-later
//! Profil de plateforme (ODM) : `tuxedo_platform_profile` puis ACPI.
//!
//! # Chemins
//!
//! ```text
//! /sys/bus/platform/devices/tuxedo_platform_profile/platform_profile
//! /sys/bus/platform/devices/tuxedo_platform_profile/platform_profile_choices
//! /sys/firmware/acpi/platform_profile
//! /sys/firmware/acpi/platform_profile_choices
//! ```
//!
//! Ordre de l'original (`ODMProfileWorker.onStart`) : module TUXEDO, puis
//! ACPI si le modèle n'est pas dans la liste de quirk, puis fallback
//! `tuxedo-io` (traité par le daemon, voir [`crate::tdp`] et
//! [`crate::tuxedo_io`]).
//!
//! # Conflit avec power-profiles-daemon (D4)
//!
//! `platform_profile` est aussi écrit par PPD. Par défaut PPD en est
//! propriétaire : [`PlatformProfile::apply`] exige une
//! [`ProfileOwnership`] explicite et refuse l'écriture en mode
//! [`ProfileOwnership::PpdOwned`]. Le daemon construit cette valeur à partir
//! de la présence de PPD et de l'option utilisateur « TCC prioritaire » ;
//! toute dérogation est journalisée par l'appelant.
//!
//! # Quirks
//!
//! Les modèles IBP G10 AMD, IBM15A10 et IBM16A10 exposent un
//! `platform_profile` ACPI que le firmware n'honore pas
//! (`ODMProfileWorker.hasQuirkNoPlatformProfile`). L'ACPI est alors ignoré au
//! profit du fallback `tuxedo-io`.

use crate::device::TuxedoDevice;
use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Base du module TUXEDO.
pub const TUXEDO_PLATFORM_PROFILE_BASE: &str = "bus/platform/devices/tuxedo_platform_profile";
/// Base ACPI générique (deux fichiers, pas un répertoire).
pub const ACPI_PLATFORM_PROFILE_PATH: &str = "firmware/acpi/platform_profile";
/// Liste ACPI des profils disponibles.
pub const ACPI_PLATFORM_PROFILE_CHOICES: &str = "firmware/acpi/platform_profile_choices";

/// Longueur maximale d'un nom de profil.
const MAX_PROFILE_NAME: usize = 64;

/// Origine du profil de plateforme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSource {
    /// Module `tuxedo_platform_profile`.
    Tuxedo,
    /// `platform_profile` ACPI standard.
    Acpi,
}

impl ProfileSource {
    /// Nom stable exposé à l'UI et aux logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tuxedo => "tuxedo_platform_profile",
            Self::Acpi => "acpi_platform_profile",
        }
    }
}

/// Propriétaire du profil de plateforme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileOwnership {
    /// PPD est actif : TCC ne doit pas écrire `platform_profile`.
    PpdOwned,
    /// TCC est propriétaire (PPD absent, ou override explicite et journalisé).
    TccOwned,
}

/// Vue du profil de plateforme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformProfile {
    source: Option<ProfileSource>,
    current: Option<String>,
    choices: Vec<String>,
    /// Vrai si l'ACPI a été ignoré à cause d'un quirk de modèle.
    acpi_quirk_blocked: bool,
}

impl PlatformProfile {
    /// Détecte la source disponible.
    ///
    /// # Errors
    ///
    /// Erreur d'E/S réelle lors des lectures.
    pub fn detect(root: &SysfsRoot, device: TuxedoDevice) -> Result<Self> {
        let tuxedo_current =
            root.read_optional_string(format!("{TUXEDO_PLATFORM_PROFILE_BASE}/platform_profile"))?;
        let tuxedo_choices = read_choices(
            root,
            format!("{TUXEDO_PLATFORM_PROFILE_BASE}/platform_profile_choices"),
        )?;
        if let (Some(current), false) = (&tuxedo_current, tuxedo_choices.is_empty()) {
            return Ok(Self {
                source: Some(ProfileSource::Tuxedo),
                current: Some(current.clone()),
                choices: tuxedo_choices,
                acpi_quirk_blocked: false,
            });
        }

        let blocked = device.blocks_acpi_platform_profile();
        if !blocked {
            let acpi_current = root.read_optional_string(ACPI_PLATFORM_PROFILE_PATH)?;
            let acpi_choices = read_choices(root, ACPI_PLATFORM_PROFILE_CHOICES.to_owned())?;
            if let (Some(current), false) = (&acpi_current, acpi_choices.is_empty()) {
                return Ok(Self {
                    source: Some(ProfileSource::Acpi),
                    current: Some(current.clone()),
                    choices: acpi_choices,
                    acpi_quirk_blocked: false,
                });
            }
        }

        Ok(Self {
            source: None,
            current: None,
            choices: Vec::new(),
            acpi_quirk_blocked: blocked,
        })
    }

    /// Source active, `None` si aucun `platform_profile` utilisable.
    #[must_use]
    pub fn source(&self) -> Option<ProfileSource> {
        self.source
    }

    /// Profil courant.
    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Profils disponibles (ordre du noyau).
    #[must_use]
    pub fn choices(&self) -> &[String] {
        &self.choices
    }

    /// Vrai si un profil de plateforme est pilotable par sysfs.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.source.is_some() && !self.choices.is_empty()
    }

    /// Vrai si l'ACPI a été ignoré à cause d'un quirk de modèle.
    #[must_use]
    pub fn acpi_quirk_blocked(&self) -> bool {
        self.acpi_quirk_blocked
    }

    /// Profil par défaut, comme l'original : la **dernière** valeur de la liste
    /// de choix (généralement `performance`/`overboost`).
    #[must_use]
    pub fn default_profile(&self) -> Option<&str> {
        self.choices.last().map(String::as_str)
    }

    /// Applique un profil.
    ///
    /// # Errors
    ///
    /// - [`HwError::PolicyDenied`] si `ownership == PpdOwned` ;
    /// - [`HwError::NotSupported`] si aucun `platform_profile` n'est exposé ;
    /// - [`HwError::InvalidValue`] si le nom est invalide ou absent de la liste.
    pub fn apply(&self, root: &SysfsRoot, name: &str, ownership: ProfileOwnership) -> Result<()> {
        if ownership == ProfileOwnership::PpdOwned {
            return Err(HwError::PolicyDenied {
                resource: "platform_profile",
                reason: "power-profiles-daemon est propriétaire (D4)",
            });
        }
        let Some(source) = self.source else {
            return Err(HwError::NotSupported("platform_profile"));
        };
        validate_name(name)?;
        if self.choices.is_empty() {
            return Err(HwError::NotSupported("platform_profile_choices"));
        }
        if !self.choices.iter().any(|candidate| candidate == name) {
            return Err(HwError::InvalidValue {
                context: "profil de plateforme",
                value: name.to_owned(),
            });
        }
        if self.current.as_deref() == Some(name) {
            return Ok(());
        }
        let path = match source {
            ProfileSource::Tuxedo => format!("{TUXEDO_PLATFORM_PROFILE_BASE}/platform_profile"),
            ProfileSource::Acpi => ACPI_PLATFORM_PROFILE_PATH.to_owned(),
        };
        root.write_string(&path, name)
    }
}

fn read_choices(root: &SysfsRoot, path: String) -> Result<Vec<String>> {
    let Some(raw) = root.read_optional_string(path)? else {
        return Ok(Vec::new());
    };
    Ok(raw
        .split_whitespace()
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect())
}

fn validate_name(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(HwError::InvalidValue {
            context: "profil de plateforme",
            value: String::new(),
        });
    }
    if value.chars().count() > MAX_PROFILE_NAME {
        return Err(HwError::InvalidValue {
            context: "profil de plateforme (trop long)",
            value: format!("<{} caractères>", value.chars().count()),
        });
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_graphic() && character != '\\')
    {
        return Err(HwError::InvalidValue {
            context: "profil de plateforme",
            value: value.escape_default().to_string(),
        });
    }
    Ok(())
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

    fn tuxedo_fixture(root: &Path) {
        write(
            root,
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            root,
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
    }

    fn acpi_fixture(root: &Path) {
        write(
            root,
            "firmware/acpi/platform_profile_choices",
            "low-power balanced performance\n",
        );
        write(root, "firmware/acpi/platform_profile", "low-power\n");
    }

    fn detect(root: &Path, device: TuxedoDevice) -> PlatformProfile {
        PlatformProfile::detect(&SysfsRoot::new(root), device).unwrap()
    }

    #[test]
    fn tuxedo_module_has_priority_over_acpi() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        acpi_fixture(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Unknown);

        assert_eq!(profile.source(), Some(ProfileSource::Tuxedo));
        assert_eq!(profile.current(), Some("balanced"));
        assert_eq!(profile.choices(), ["power_save", "balanced", "performance"]);
        assert_eq!(profile.default_profile(), Some("performance"));
        assert!(profile.is_available());
    }

    #[test]
    fn acpi_is_used_when_no_tuxedo_module() {
        let dir = tempfile::tempdir().unwrap();
        acpi_fixture(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Ibm16A10);
        assert_eq!(profile.source(), None);
        assert!(profile.acpi_quirk_blocked());

        let profile = detect(dir.path(), TuxedoDevice::Unknown);
        assert_eq!(profile.source(), Some(ProfileSource::Acpi));
        assert_eq!(profile.default_profile(), Some("performance"));
    }

    #[test]
    fn quirk_only_blocks_acpi_not_tuxedo() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        acpi_fixture(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Ibm15A10);
        assert_eq!(profile.source(), Some(ProfileSource::Tuxedo));
        assert!(!profile.acpi_quirk_blocked());
    }

    #[test]
    fn absent_hardware_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let profile = detect(dir.path(), TuxedoDevice::Unknown);
        assert!(!profile.is_available());
        assert_eq!(profile.source(), None);
        assert!(matches!(
            profile.apply(
                &SysfsRoot::new(dir.path()),
                "performance",
                ProfileOwnership::TccOwned
            ),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn ppd_ownership_denies_write() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Unknown);
        let error = profile
            .apply(&root, "performance", ProfileOwnership::PpdOwned)
            .unwrap_err();
        assert!(error.is_policy_denied());
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "balanced"
        );
    }

    #[test]
    fn applies_known_profile_and_validates_unknown() {
        let dir = tempfile::tempdir().unwrap();
        tuxedo_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Unknown);

        profile
            .apply(&root, "performance", ProfileOwnership::TccOwned)
            .unwrap();
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "performance"
        );

        let error = profile
            .apply(&root, "turbo", ProfileOwnership::TccOwned)
            .unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
        let error = profile
            .apply(&root, "bad value", ProfileOwnership::TccOwned)
            .unwrap_err();
        assert!(matches!(error, HwError::InvalidValue { .. }));
    }

    #[test]
    fn acpi_write_uses_acpi_file() {
        let dir = tempfile::tempdir().unwrap();
        acpi_fixture(dir.path());
        let root = SysfsRoot::new(dir.path());
        let profile = detect(dir.path(), TuxedoDevice::Unknown);
        profile
            .apply(&root, "performance", ProfileOwnership::TccOwned)
            .unwrap();
        assert_eq!(
            read(dir.path(), "firmware/acpi/platform_profile"),
            "performance"
        );
    }
}
