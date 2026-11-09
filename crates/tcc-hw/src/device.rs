// SPDX-License-Identifier: GPL-3.0-or-later
//! Identification du modèle TUXEDO et quirks associés.
//!
//! L'original (`TuxedoControlCenterDaemon.identifyDevice`) combine deux
//! sources :
//!
//! 1. le **SKU DMI** `/sys/class/dmi/id/product_sku` (table
//!    `dmiSKUDeviceMap`) ;
//! 2. à défaut, l'**identifiant de modèle Uniwill** `R_UW_MODEL_ID` du module
//!    `tuxedo_io` (table `uwidDeviceMap`, valeurs `0x12..=0x17`).
//!
//! Ce module porte uniquement l'**identification** et les prédicats de quirk
//! factuels. Les décisions fonctionnelles (ne pas écrire `platform_profile`,
//! restreindre PRIME, ...) restent dans les modules qui les appliquent, de
//! façon explicite et testée.
//!
//! Un SKU inconnu retourne [`TuxedoDevice::Unknown`] : ce n'est jamais une
//! erreur. Le daemon doit fonctionner sur une machine non-TUXEDO.

use crate::dmi::DmiInfo;
use crate::sysfs::SysfsRoot;

/// Modèle TUXEDO identifié.
///
/// Les variantes reprennent exactement les clés de `TUXEDODevice` de
/// `tuxedo-control-center-master/src/common/models/DefaultProfiles.ts`, afin
/// que les logs et la documentation restent comparables à l'original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TuxedoDevice {
    /// SKU absent ou non reconnu.
    #[default]
    Unknown,
    Ibp17G6,
    Pulse1501,
    Pulse1502,
    Pulse1403,
    Pulse1404,
    Ibp14G6Tux,
    Ibp14G6Trx,
    Ibp14G6Tqf,
    Ibp14G7AqfArx,
    IbpG8,
    IbpG10Amd,
    IbpG10Intel,
    Ibm15A10,
    Ibm15I10,
    Ibm16A10,
    Aura14G3,
    Aura15G3,
    Polaris1Xa02,
    Polaris1Xi02,
    Polaris1Xa03,
    Polaris1Xi03,
    Polaris1Xa05,
    Stellaris1Xa03,
    Stellaris1Xi03,
    Stellaris1Xi04,
    Stepol1Xa04,
    Stellaris1Xi05,
    Stellaris1Xa05,
    Stellaris16I06,
    Stellaris17I06,
    Stellsl15A06,
    Stellsl15I06,
    Stellaris16A07,
    Stellaris16I07,
    Stellaris16I08Ips,
    Stellaris16I08Mled,
    Stellaris16I08Oled,
    Sirius1601,
    Sirius1602,
    Gemini17I04,
    Ba1510,
}

impl TuxedoDevice {
    /// Nom stable, identique à la clé `TUXEDODevice` de l'original.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Ibp17G6 => "IBP17G6",
            Self::Pulse1501 => "PULSE1501",
            Self::Pulse1502 => "PULSE1502",
            Self::Pulse1403 => "PULSE1403",
            Self::Pulse1404 => "PULSE1404",
            Self::Ibp14G6Tux => "IBP14G6_TUX",
            Self::Ibp14G6Trx => "IBP14G6_TRX",
            Self::Ibp14G6Tqf => "IBP14G6_TQF",
            Self::Ibp14G7AqfArx => "IBP14G7_AQF_ARX",
            Self::IbpG8 => "IBPG8",
            Self::IbpG10Amd => "IBPG10AMD",
            Self::IbpG10Intel => "IBPG10INTEL",
            Self::Ibm15A10 => "IBM15A10",
            Self::Ibm15I10 => "IBM15I10",
            Self::Ibm16A10 => "IBM16A10",
            Self::Aura14G3 => "AURA14G3",
            Self::Aura15G3 => "AURA15G3",
            Self::Polaris1Xa02 => "POLARIS1XA02",
            Self::Polaris1Xi02 => "POLARIS1XI02",
            Self::Polaris1Xa03 => "POLARIS1XA03",
            Self::Polaris1Xi03 => "POLARIS1XI03",
            Self::Polaris1Xa05 => "POLARIS1XA05",
            Self::Stellaris1Xa03 => "STELLARIS1XA03",
            Self::Stellaris1Xi03 => "STELLARIS1XI03",
            Self::Stellaris1Xi04 => "STELLARIS1XI04",
            Self::Stepol1Xa04 => "STEPOL1XA04",
            Self::Stellaris1Xi05 => "STELLARIS1XI05",
            Self::Stellaris1Xa05 => "STELLARIS1XA05",
            Self::Stellaris16I06 => "STELLARIS16I06",
            Self::Stellaris17I06 => "STELLARIS17I06",
            Self::Stellsl15A06 => "STELLSL15A06",
            Self::Stellsl15I06 => "STELLSL15I06",
            Self::Stellaris16A07 => "STELLARIS16A07",
            Self::Stellaris16I07 => "STELLARIS16I07",
            Self::Stellaris16I08Ips => "STELLARIS16I08_IPS",
            Self::Stellaris16I08Mled => "STELLARIS16I08_MLED",
            Self::Stellaris16I08Oled => "STELLARIS16I08_OLED",
            Self::Sirius1601 => "SIRIUS1601",
            Self::Sirius1602 => "SIRIUS1602",
            Self::Gemini17I04 => "GEMINI17I04",
            Self::Ba1510 => "BA1510",
        }
    }

    /// Vrai pour un modèle TUXEDO reconnu.
    #[must_use]
    pub const fn is_tuxedo(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Identifie le modèle depuis le DMI et, en secours, l'identifiant Uniwill.
    ///
    /// La table DMI de l'original contient deux entrées dont la clé littérale
    /// comporte un espace (`"IBP14A10MK1 / IBP15A10MK1"`). Ces valeurs ne
    /// peuvent pas correspondre à un fichier sysfs réel : la machine était
    /// donc identifiée `UNKNOWN` et le quirk « pas d'ACPI platform_profile »
    /// n'était jamais appliqué sur ces modèles. Les deux SKU réels
    /// (`IBP14A10MK1`, `IBP15A10MK1`) sont ajoutés comme alias explicites vers
    /// [`TuxedoDevice::IbpG10Amd`] — correction documentée d'un bug de
    /// l'original, les clés littérales restant acceptées pour la traçabilité.
    #[must_use]
    pub fn identify(dmi: &DmiInfo, uniwill_model_id: Option<i32>) -> Self {
        if let Some(sku) = dmi.sku() {
            let model = Self::from_sku(sku.trim());
            if model.is_tuxedo() {
                return model;
            }
        }
        uniwill_model_id
            .and_then(Self::from_uniwill_model_id)
            .unwrap_or(Self::Unknown)
    }

    /// Identifie depuis un SKU DMI brut.
    #[must_use]
    pub fn from_sku(sku: &str) -> Self {
        let sku = sku.trim();
        match sku {
            "IBS1706" => Self::Ibp17G6,
            "IBP1XI08MK1" | "IBP1XI08MK2" | "IBP14I08MK2" | "IBP16I08MK2" | "OMNIA08IMK2" => {
                Self::IbpG8
            }
            "IBP14A10MK1 / IBP15A10MK1"
            | "IIBP14A10MK1 / IBP15A10MK1"
            | "IBP14A10MK1"
            | "IBP15A10MK1" => Self::IbpG10Amd,
            "IBP15I10MK1" => Self::IbpG10Intel,
            "IBM15A10" => Self::Ibm15A10,
            "IBM15I10" => Self::Ibm15I10,
            "IBM16A10" => Self::Ibm16A10,
            "POLARIS1XA02" => Self::Polaris1Xa02,
            "POLARIS1XI02" => Self::Polaris1Xi02,
            "POLARIS1XA03" => Self::Polaris1Xa03,
            "POLARIS1XI03" => Self::Polaris1Xi03,
            "STELLARIS1XA03" => Self::Stellaris1Xa03,
            "STEPOL1XA04" => Self::Stepol1Xa04,
            "STELLARIS1XI03" => Self::Stellaris1Xi03,
            "STELLARIS1XI04" => Self::Stellaris1Xi04,
            "PULSE1501" => Self::Pulse1501,
            "PULSE1502" => Self::Pulse1502,
            "PULSE1403" => Self::Pulse1403,
            "PULSE1404" => Self::Pulse1404,
            "STELLARIS1XI05" => Self::Stellaris1Xi05,
            "POLARIS1XA05" => Self::Polaris1Xa05,
            "STELLARIS1XA05" => Self::Stellaris1Xa05,
            "STELLARIS16I06" => Self::Stellaris16I06,
            "STELLARIS17I06" => Self::Stellaris17I06,
            "STELLARIS16G8I-IPS" => Self::Stellaris16I08Ips,
            "STELLARIS16G8I-MLED" => Self::Stellaris16I08Mled,
            "STELLARIS16G8I-OLED" => Self::Stellaris16I08Oled,
            "STELLSL15A06" => Self::Stellsl15A06,
            "STELLSL15I06" => Self::Stellsl15I06,
            "AURA14GEN3" => Self::Aura14G3,
            "AURA15GEN3" => Self::Aura15G3,
            "STELLARIS16A07" => Self::Stellaris16A07,
            "STELLARIS16I07" => Self::Stellaris16I07,
            "SIRIUS1601" => Self::Sirius1601,
            "SIRIUS1602" => Self::Sirius1602,
            "GEMINI17I04" | "GEMINIGEN4I" => Self::Gemini17I04,
            "BA1510" => Self::Ba1510,
            _ => Self::Unknown,
        }
    }

    /// Identifie depuis l'identifiant de modèle Uniwill (`R_UW_MODEL_ID`).
    ///
    /// La table de l'original ne couvre que quatre valeurs ; le module noyau
    /// renvoie un entier décimal (`0x12` = 18, ...). Toute autre valeur est
    /// inconnue.
    #[must_use]
    pub fn from_uniwill_model_id(model_id: i32) -> Option<Self> {
        match model_id {
            0x12 => Some(Self::Ibp14G6Trx),
            0x13 => Some(Self::Ibp14G6Tux),
            0x14 => Some(Self::Ibp14G6Tqf),
            0x17 => Some(Self::Ibp14G7AqfArx),
            _ => None,
        }
    }

    /// Lit le DMI puis identifie le modèle.
    ///
    /// Une erreur DMI (fichier absent, contenu non UTF-8) retourne
    /// [`TuxedoDevice::Unknown`] : l'identification ne doit jamais empêcher le
    /// démarrage.
    #[must_use]
    pub fn detect(root: &SysfsRoot, uniwill_model_id: Option<i32>) -> Self {
        match DmiInfo::read(root) {
            Ok(dmi) => Self::identify(&dmi, uniwill_model_id),
            Err(error) => {
                tracing::debug!(%error, "DMI illisible, modèle TUXEDO inconnu");
                uniwill_model_id
                    .and_then(Self::from_uniwill_model_id)
                    .unwrap_or(Self::Unknown)
            }
        }
    }

    /// Vrai si l'ACPI `platform_profile` ne doit pas être utilisé sur ce
    /// modèle (`ODMProfileWorker.hasQuirkNoPlatformProfile`).
    ///
    /// L'original passe alors directement au fallback `tuxedo-io` : ces SKU
    /// exposent un `platform_profile` ACPI que le firmware n'honore pas.
    #[must_use]
    pub const fn blocks_acpi_platform_profile(self) -> bool {
        matches!(self, Self::IbpG10Amd | Self::Ibm15A10 | Self::Ibm16A10)
    }

    /// Vrai si l'EPP ne doit pas être écrit sur ce modèle
    /// (`CpuWorker.noEPPWriteQuirk`).
    #[must_use]
    pub const fn skips_epp_write(self) -> bool {
        matches!(self, Self::Sirius1602 | Self::Stellsl15A06)
    }

    /// Vrai si le modèle impose la préférence EPP `performance`
    /// (`CpuWorker.hasEPPPerformanceQuirk`).
    #[must_use]
    pub const fn forces_epp_performance(self) -> bool {
        matches!(self, Self::Gemini17I04 | Self::IbpG10Intel | Self::Ibm15I10)
    }

    /// Vrai si PRIME n'est utilisable que lorsque l'écran interne est relié à
    /// l'iGPU (`PrimeWorker.checkPrimeSupported`).
    #[must_use]
    pub const fn prime_requires_internal_on_igpu(self) -> bool {
        matches!(self, Self::Ibm15A10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dmi_with_sku(sku: &str) -> DmiInfo {
        DmiInfo {
            product_sku: Some(sku.to_owned()),
            ..DmiInfo::default()
        }
    }

    #[test]
    fn sku_identification_covers_original_table() {
        assert_eq!(TuxedoDevice::from_sku("IBM16A10"), TuxedoDevice::Ibm16A10);
        assert_eq!(
            TuxedoDevice::from_sku("STELLARIS16G8I-OLED"),
            TuxedoDevice::Stellaris16I08Oled
        );
        assert_eq!(
            TuxedoDevice::from_sku("GEMINIGEN4I"),
            TuxedoDevice::Gemini17I04
        );
        assert_eq!(
            TuxedoDevice::from_sku(" SIRIUS1601\n"),
            TuxedoDevice::Sirius1601
        );
        // Alias réels du quirk IBP G10 AMD (clés originales inmatchables).
        assert_eq!(
            TuxedoDevice::from_sku("IBP14A10MK1"),
            TuxedoDevice::IbpG10Amd
        );
        assert_eq!(
            TuxedoDevice::from_sku("IBP15A10MK1"),
            TuxedoDevice::IbpG10Amd
        );
        assert_eq!(TuxedoDevice::from_sku("nope"), TuxedoDevice::Unknown);
        assert_eq!(TuxedoDevice::from_sku(""), TuxedoDevice::Unknown);
    }

    #[test]
    fn dmi_takes_priority_over_uniwill_model() {
        let dmi = dmi_with_sku("SIRIUS1602");
        assert_eq!(
            TuxedoDevice::identify(&dmi, Some(0x13)),
            TuxedoDevice::Sirius1602
        );
    }

    #[test]
    fn uniwill_model_is_the_fallback() {
        let dmi = DmiInfo::default();
        assert_eq!(
            TuxedoDevice::identify(&dmi, Some(0x12)),
            TuxedoDevice::Ibp14G6Trx
        );
        assert_eq!(
            TuxedoDevice::identify(&dmi, Some(0x99)),
            TuxedoDevice::Unknown
        );
        assert_eq!(TuxedoDevice::identify(&dmi, None), TuxedoDevice::Unknown);
    }

    #[test]
    fn detect_reads_sysfs_and_never_panics() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("class/dmi/id")).unwrap();
        std::fs::write(dir.path().join("class/dmi/id/product_sku"), "BA1510\n").unwrap();
        let root = SysfsRoot::new(dir.path());
        assert_eq!(TuxedoDevice::detect(&root, None), TuxedoDevice::Ba1510);
        assert_eq!(
            TuxedoDevice::detect(&SysfsRoot::new("/nonexistent-tcc-root"), Some(0x14)),
            TuxedoDevice::Ibp14G6Tqf
        );
    }

    #[test]
    fn platform_profile_quirk_matches_original() {
        for device in [
            TuxedoDevice::IbpG10Amd,
            TuxedoDevice::Ibm15A10,
            TuxedoDevice::Ibm16A10,
        ] {
            assert!(device.blocks_acpi_platform_profile(), "{device:?}");
        }
        assert!(!TuxedoDevice::Ibm15I10.blocks_acpi_platform_profile());
        assert!(!TuxedoDevice::Unknown.blocks_acpi_platform_profile());
    }

    #[test]
    fn epp_quirks_match_original() {
        assert!(TuxedoDevice::Sirius1602.skips_epp_write());
        assert!(TuxedoDevice::Stellsl15A06.skips_epp_write());
        assert!(!TuxedoDevice::Sirius1601.skips_epp_write());
        assert!(TuxedoDevice::Gemini17I04.forces_epp_performance());
        assert!(TuxedoDevice::IbpG10Intel.forces_epp_performance());
        assert!(TuxedoDevice::Ibm15I10.forces_epp_performance());
        assert!(!TuxedoDevice::Ibm16A10.forces_epp_performance());
    }

    #[test]
    fn prime_quirk_is_ibm15a10_only() {
        assert!(TuxedoDevice::Ibm15A10.prime_requires_internal_on_igpu());
        assert!(!TuxedoDevice::Ibm16A10.prime_requires_internal_on_igpu());
    }

    #[test]
    fn device_names_are_unique() {
        let devices = [
            TuxedoDevice::Unknown,
            TuxedoDevice::Ibp17G6,
            TuxedoDevice::Pulse1501,
            TuxedoDevice::Pulse1502,
            TuxedoDevice::Pulse1403,
            TuxedoDevice::Pulse1404,
            TuxedoDevice::Ibp14G6Tux,
            TuxedoDevice::Ibp14G6Trx,
            TuxedoDevice::Ibp14G6Tqf,
            TuxedoDevice::Ibp14G7AqfArx,
            TuxedoDevice::IbpG8,
            TuxedoDevice::IbpG10Amd,
            TuxedoDevice::IbpG10Intel,
            TuxedoDevice::Ibm15A10,
            TuxedoDevice::Ibm15I10,
            TuxedoDevice::Ibm16A10,
            TuxedoDevice::Aura14G3,
            TuxedoDevice::Aura15G3,
            TuxedoDevice::Polaris1Xa02,
            TuxedoDevice::Polaris1Xi02,
            TuxedoDevice::Polaris1Xa03,
            TuxedoDevice::Polaris1Xi03,
            TuxedoDevice::Polaris1Xa05,
            TuxedoDevice::Stellaris1Xa03,
            TuxedoDevice::Stellaris1Xi03,
            TuxedoDevice::Stellaris1Xi04,
            TuxedoDevice::Stepol1Xa04,
            TuxedoDevice::Stellaris1Xi05,
            TuxedoDevice::Stellaris1Xa05,
            TuxedoDevice::Stellaris16I06,
            TuxedoDevice::Stellaris17I06,
            TuxedoDevice::Stellsl15A06,
            TuxedoDevice::Stellsl15I06,
            TuxedoDevice::Stellaris16A07,
            TuxedoDevice::Stellaris16I07,
            TuxedoDevice::Stellaris16I08Ips,
            TuxedoDevice::Stellaris16I08Mled,
            TuxedoDevice::Stellaris16I08Oled,
            TuxedoDevice::Sirius1601,
            TuxedoDevice::Sirius1602,
            TuxedoDevice::Gemini17I04,
            TuxedoDevice::Ba1510,
        ];
        let mut names: Vec<&str> = devices.iter().map(|device| device.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
