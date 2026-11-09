// SPDX-License-Identifier: GPL-3.0-or-later
//! Identification DMI/SMBIOS.
//!
//! ```text
//! /sys/class/dmi/id/product_sku
//! /sys/class/dmi/id/product_name
//! /sys/class/dmi/id/board_vendor
//! /sys/class/dmi/id/chassis_vendor
//! /sys/class/dmi/id/sys_vendor
//! /sys/class/dmi/id/board_name
//! ```
//!
//! L'original (`DMIController`) lit `product_sku`, `board_vendor`,
//! `chassis_vendor` et `sys_vendor`. Tous les champs sont optionnels : une VM,
//! un conteneur ou un ARM peuvent ne pas exposer DMI. La détection TUXEDO est
//! volontairement tolérante (vendor ou board vendor), le daemon ne doit jamais
//! refuser de démarrer sur une machine non-TUXEDO.

use crate::error::Result;
use crate::sysfs::SysfsRoot;

const DMI_CLASS: &str = "class/dmi/id";

/// Informations DMI utiles au daemon.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DmiInfo {
    pub product_sku: Option<String>,
    pub product_name: Option<String>,
    pub board_name: Option<String>,
    pub board_vendor: Option<String>,
    pub chassis_vendor: Option<String>,
    pub sys_vendor: Option<String>,
}

impl DmiInfo {
    /// Lit les champs DMI disponibles.
    pub fn read(root: &SysfsRoot) -> Result<Self> {
        Ok(Self {
            product_sku: root.read_optional_string(format!("{DMI_CLASS}/product_sku"))?,
            product_name: root.read_optional_string(format!("{DMI_CLASS}/product_name"))?,
            board_name: root.read_optional_string(format!("{DMI_CLASS}/board_name"))?,
            board_vendor: root.read_optional_string(format!("{DMI_CLASS}/board_vendor"))?,
            chassis_vendor: root.read_optional_string(format!("{DMI_CLASS}/chassis_vendor"))?,
            sys_vendor: root.read_optional_string(format!("{DMI_CLASS}/sys_vendor"))?,
        })
    }

    /// Vrai si l'un des champs constructeur mentionne TUXEDO.
    ///
    /// Sert uniquement à activer des chemins supplémentaires ; tous les
    /// modules restent pilotés par la présence réelle des fichiers sysfs.
    pub fn is_tuxedo(&self) -> bool {
        [&self.sys_vendor, &self.board_vendor, &self.chassis_vendor]
            .into_iter()
            .flatten()
            .any(|vendor| vendor.to_ascii_lowercase().contains("tuxedo"))
    }

    /// SKU produit, utilisée pour les quirks de modèle.
    pub fn sku(&self) -> Option<&str> {
        self.product_sku.as_deref()
    }
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

    #[test]
    fn reads_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/product_sku", "SKU-42\n");
        write(dir.path(), "class/dmi/id/product_name", "TUXEDO Book X\n");
        write(dir.path(), "class/dmi/id/board_name", "Board\n");
        write(
            dir.path(),
            "class/dmi/id/board_vendor",
            "TUXEDO Computers\n",
        );
        write(dir.path(), "class/dmi/id/chassis_vendor", "TUXEDO\n");
        write(
            dir.path(),
            "class/dmi/id/sys_vendor",
            "TUXEDO Computers GmbH\n",
        );
        let dmi = DmiInfo::read(&SysfsRoot::new(dir.path())).unwrap();
        assert_eq!(dmi.sku(), Some("SKU-42"));
        assert_eq!(dmi.product_name.as_deref(), Some("TUXEDO Book X"));
        assert!(dmi.is_tuxedo());
    }

    #[test]
    fn missing_dmi_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dmi = DmiInfo::read(&SysfsRoot::new(dir.path())).unwrap();
        assert_eq!(dmi, DmiInfo::default());
        assert!(!dmi.is_tuxedo());
    }

    #[test]
    fn detection_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/sys_vendor", "tuxedo computers\n");
        assert!(
            DmiInfo::read(&SysfsRoot::new(dir.path()))
                .unwrap()
                .is_tuxedo()
        );
    }

    #[test]
    fn other_vendor_is_not_tuxedo() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/sys_vendor", "LENOVO\n");
        write(dir.path(), "class/dmi/id/board_vendor", "LENOVO\n");
        assert!(
            !DmiInfo::read(&SysfsRoot::new(dir.path()))
                .unwrap()
                .is_tuxedo()
        );
    }
}
