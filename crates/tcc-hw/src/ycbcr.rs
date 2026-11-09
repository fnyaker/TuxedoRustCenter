// SPDX-License-Identifier: GPL-3.0-or-later
//! YCbCr 4:2:0 : forçage par port DisplayPort via debugfs.
//!
//! ```text
//! /sys/kernel/debug/dri/<carte>/<port>/force_yuv420_output   (0/1)
//! ```
//!
//! Source : `YCbCr420WorkaroundWorker` de l'original, qui écrit
//! `force_yuv420_output` avec `appendFileSync` sans vérifier le résultat.
//!
//! # Nommage des ports
//!
//! L'original construit la liste des ports avec udev (`card*-*-*`) puis retire
//! le préfixe `<carte>-` : un connecteur `card0-DP-1` donne le port `DP-1`, et
//! le chemin debugfs est `/sys/kernel/debug/dri/0/DP-1/force_yuv420_output`.
//! [`port_from_connector`] applique la même conversion.
//!
//! # Décisions
//!
//! - Le chemin est construit sous [`DEBUGFS_DRM`] (relatif à `/sys`) :
//!   `kernel/debug/dri/<carte>/<port>/force_yuv420_output`. Aucun chemin
//!   absolu fourni par l'appelant n'est accepté.
//! - `card` est un entier (indice de carte DRM) ; `port` doit respecter la
//!   grammaire des noms de port du noyau (`DP-1`, `HDMI-A-1`, `DP-1-2`, ...)
//!   et rester borné. La validation bloque toute tentative de traversée.
//! - Si `debugfs` n'est pas monté, le fichier est absent : l'appel retourne
//!   « non disponible », pas une erreur fatale.
//! - Le changement peut nécessiter une réapplication du mode par le
//!   compositeur (`ModeReapplyPending` dans l'original). `apply` indique si la
//!   valeur a réellement changé ; le daemon expose ce drapeau à l'UI.
//!
//! Une entrée de réglages référence les cartes par index : le daemon convertit
//! `settings.ycbcr420_workaround[card][port]` en appels [`apply`].

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Racine debugfs des cartes DRM, relative à `/sys`.
pub const DEBUGFS_DRM: &str = "kernel/debug/dri";
/// Nombre maximal de cartes accepté.
pub const MAX_CARDS: u32 = 16;
/// Longueur maximale d'un nom de port.
const MAX_PORT_NAME: usize = 64;

/// Port DRM forçable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YcbcrPort {
    /// Index de la carte (`0` pour `card0`).
    pub card: u32,
    /// Nom du port (`card0-DP-1`, ...).
    pub port: String,
    /// Valeur courante lue (`None` si illisible).
    pub forced: Option<bool>,
}

/// Extrait le nom de port debugfs depuis un connecteur DRM.
///
/// `card0-DP-1` → `DP-1`, `card1-HDMI-A-1` → `HDMI-A-1`. Retourne `None` si le
/// nom ne commence pas par `card<chiffres>-`.
#[must_use]
pub fn port_from_connector(connector: &str) -> Option<&str> {
    let rest = connector.strip_prefix("card")?;
    let separator = rest.find('-')?;
    let (digits, port) = rest.split_at(separator);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = &port[1..];
    if port.is_empty() {
        return None;
    }
    Some(port)
}

/// Construit le chemin relatif du forçage pour une carte et un port.
///
/// # Errors
///
/// [`HwError::InvalidValue`] si `card > MAX_CARDS` ou si `port` ne respecte
/// pas la grammaire attendue.
pub fn force_path(card: u32, port: &str) -> Result<String> {
    if card > MAX_CARDS {
        return Err(HwError::OutOfRange {
            context: "carte DRM",
            value: i64::from(card),
            min: 0,
            max: i64::from(MAX_CARDS),
        });
    }
    validate_port(port)?;
    Ok(format!("{DEBUGFS_DRM}/{card}/{port}/force_yuv420_output"))
}

fn validate_port(port: &str) -> Result<()> {
    if port.is_empty() || port.len() > MAX_PORT_NAME {
        return Err(HwError::InvalidValue {
            context: "port DRM",
            value: format!("<{} caractères>", port.len()),
        });
    }
    // Grammaire observée : `card0-DP-1`, `card0-DP-2`, `card0-HDMI-A-1`,
    // `card0-DP-1-2`. Lettres ASCII, chiffres et tirets uniquement.
    if !port
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || port.starts_with('-')
        || port.ends_with('-')
    {
        return Err(HwError::InvalidValue {
            context: "port DRM",
            value: port.to_owned(),
        });
    }
    Ok(())
}

/// Vrai si le forçage est exposé pour cette carte/port.
///
/// # Errors
///
/// Chemin invalide (voir [`force_path`]).
pub fn is_available(root: &SysfsRoot, card: u32, port: &str) -> Result<bool> {
    Ok(root.is_file(force_path(card, port)?))
}

/// Applique ou retire le forçage YCbCr 4:2:0.
///
/// Retourne `true` si la valeur a été modifiée, `false` si elle était déjà
/// correcte.
///
/// # Errors
///
/// - [`HwError::NotSupported`] si le fichier n'existe pas (debugfs non monté ou
///   port non compatible) ;
/// - [`HwError::InvalidValue`] si le chemin est invalide ;
/// - erreur d'E/S en cas d'échec d'écriture.
pub fn apply(root: &SysfsRoot, card: u32, port: &str, enabled: bool) -> Result<bool> {
    let path = force_path(card, port)?;
    let Some(raw) = root.read_optional_string(&path)? else {
        return Err(HwError::NotSupported("force_yuv420_output"));
    };
    let current = match raw.trim() {
        "1" => true,
        "0" => false,
        other => {
            return Err(HwError::InvalidValue {
                context: "force_yuv420_output",
                value: other.to_owned(),
            });
        }
    };
    if current == enabled {
        return Ok(false);
    }
    root.write_bool(&path, enabled)?;
    Ok(true)
}

/// Énumère les ports exposant `force_yuv420_output`.
///
/// Sert au diagnostic et à la découverte de l'UI. Les ports qui ne peuvent pas
/// être lus sont ignorés.
///
/// # Errors
///
/// Erreur d'E/S réelle lors de l'énumération.
pub fn enumerate(root: &SysfsRoot) -> Result<Vec<YcbcrPort>> {
    let mut ports = Vec::new();
    for card_entry in root.list_dir(DEBUGFS_DRM)? {
        // debugfs nomme les cartes par leur index numérique (`0`, `1`, ...),
        // contrairement à `/sys/class/drm/cardN`.
        let Some(card) = card_entry.parse::<u32>().ok() else {
            continue;
        };
        if card > MAX_CARDS {
            continue;
        }
        let card_path = format!("{DEBUGFS_DRM}/{card_entry}");
        for port in root.list_dir(&card_path)? {
            if validate_port(&port).is_err() {
                continue;
            }
            let path = format!("{card_path}/{port}/force_yuv420_output");
            if !root.is_file(&path) {
                continue;
            }
            let forced = root
                .read_optional_string(&path)?
                .and_then(|value| match value.trim() {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                });
            ports.push(YcbcrPort { card, port, forced });
        }
    }
    Ok(ports)
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
    fn path_is_built_and_validated() {
        assert_eq!(
            force_path(0, "DP-1").unwrap(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output"
        );
        assert!(force_path(0, "../etc").is_err());
        assert!(force_path(0, "").is_err());
        assert!(force_path(0, "-DP-1").is_err());
        assert!(force_path(0, "DP/1").is_err());
        assert!(force_path(u32::MAX, "DP-1").is_err());
    }

    #[test]
    fn connector_names_map_to_debugfs_ports() {
        assert_eq!(port_from_connector("card0-DP-1"), Some("DP-1"));
        assert_eq!(port_from_connector("card1-HDMI-A-1"), Some("HDMI-A-1"));
        assert_eq!(port_from_connector("card0-DP-1-2"), Some("DP-1-2"));
        assert_eq!(port_from_connector("card-DP-1"), None);
        assert_eq!(port_from_connector("DP-1"), None);
        assert_eq!(port_from_connector("card0-"), None);
    }

    #[test]
    fn applies_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "0\n",
        );
        let root = SysfsRoot::new(dir.path());
        assert!(is_available(&root, 0, "DP-1").unwrap());
        assert!(apply(&root, 0, "DP-1", true).unwrap());
        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join("kernel/debug/dri/0/DP-1/force_yuv420_output")
            )
            .unwrap(),
            "1"
        );
        // Idempotent.
        assert!(!apply(&root, 0, "DP-1", true).unwrap());
        assert!(apply(&root, 0, "DP-1", false).unwrap());
    }

    #[test]
    fn missing_debugfs_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let root = SysfsRoot::new(dir.path());
        assert!(!is_available(&root, 0, "DP-1").unwrap());
        assert!(matches!(
            apply(&root, 0, "DP-1", true),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn enumerate_skips_invalid_entries() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "1\n",
        );
        write(
            dir.path(),
            "kernel/debug/dri/1/eDP-1/force_yuv420_output",
            "0\n",
        );
        write(dir.path(), "kernel/debug/dri/0/DP-2/other_attribute", "0\n");
        write(dir.path(), "kernel/debug/dri/controlD64/whatever", "0\n");
        let ports = enumerate(&SysfsRoot::new(dir.path())).unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].card, 0);
        assert_eq!(ports[0].port, "DP-1");
        assert_eq!(ports[0].forced, Some(true));
        assert_eq!(ports[1].card, 1);
        assert_eq!(ports[1].forced, Some(false));
    }

    #[test]
    fn corrupt_value_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "kernel/debug/dri/0/DP-1/force_yuv420_output",
            "yes\n",
        );
        let root = SysfsRoot::new(dir.path());
        assert!(matches!(
            apply(&root, 0, "DP-1", true),
            Err(HwError::InvalidValue { .. })
        ));
    }
}
