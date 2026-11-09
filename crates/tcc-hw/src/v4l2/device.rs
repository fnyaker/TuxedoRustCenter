// SPDX-License-Identifier: GPL-3.0-or-later
//! Validation du nœud V4L2 et identité de la caméra.
//!
//! Toutes les fonctions de ce module sont **pures ou en lecture sysfs** :
//! aucun `unsafe`, aucune ouverture de `/dev`.
//!
//! Le nœud est validé pour une ouverture **en lecture ou en écriture** :
//! périphérique caractère, major `81`, major/minor identiques à
//! `/sys/class/video4linux/videoN/dev`, nœud possédé par root. Le nœud reste
//! root-owned même lorsque `tccd-user` écrit ; l'autorisation réelle de
//! l'écriture est l'ouverture `O_RDWR` (ACL `uaccess` ou groupe `video`), pas
//! un UID appelant différent.
//!
//! Le bit d'écriture monde (`0o002`) n'est **pas** refusé : il ne donne aucun
//! droit d'écriture au descripteur ouvert en `O_RDONLY`, et les distributions
//! posent couramment `root:video 0666` avec une ACL `uaccess`
//! (`crwxrwxrwx+`), ce qui rendrait la lecture impossible. Le mode et le
//! groupe sont journalisés (décision V-09 : dépendre de NSS/LDAP casserait des
//! distributions).
//!
//! Le chemin d'écriture (`tccd-user`, `O_RDWR`) réutilise cette validation
//! (caractère, major/minor sysfs, propriétaire root) ; l'autorisation
//! d'écriture réelle reste l'ouverture `O_RDWR`, filtrée par l'ACL
//! `uaccess`/le groupe `video`. Aucun `S_CTRL` n'est émis si l'ouverture
//! échoue.

#![forbid(unsafe_code)]

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;
use crate::webcam::VideoDevice;

use super::abi;
use super::control::sanitize_text;

/// Métadonnées d'un nœud, extraites de `fstat` par [`super::real`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeMetadata {
    pub is_char: bool,
    pub major: u32,
    pub minor: u32,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

/// Identité stable d'une caméra pour l'UI et les futurs préréglages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UvcIdentity {
    /// Vendor USB (`1bcf`), si exposé.
    pub vendor: Option<String>,
    /// Produit USB (`2c99`), si exposé.
    pub product: Option<String>,
    /// Numéro de série, si exposé.
    pub serial: Option<String>,
    /// Repli lisible (`Integrated Camera`).
    pub fallback_name: String,
}

/// Valide un nœud V4L2 pour une ouverture en lecture ou en écriture.
///
/// # Permissions
///
/// Le nœud doit appartenir à root ; c'est le nœud qui est root-owned, pas
/// l'appelant. Le bit d'écriture monde peut être présent : pour la lecture,
/// `O_RDONLY` ne confère aucun droit d'écriture ; pour l'écriture, le noyau
/// applique l'ACL `uaccess`/le groupe `video` à l'ouverture `O_RDWR`. Le mode
/// est journalisé en debug ; il n'est pas un critère de refus.
///
/// # Errors
///
/// - [`HwError::NotSupported`] si ce n'est pas un périphérique caractère
///   majeur 81 ;
/// - [`HwError::InvalidValue`] si le minor ne correspond pas au nœud sysfs ;
/// - [`HwError::PolicyDenied`] si le nœud n'appartient pas à root.
pub(crate) fn validate_node(meta: &NodeMetadata, sysfs_dev: Option<&str>) -> Result<()> {
    if !meta.is_char {
        return Err(HwError::NotSupported("nœud V4L2 (non caractère)"));
    }
    if meta.major != abi::VIDEO_MAJOR {
        return Err(HwError::NotSupported("nœud V4L2 (major)"));
    }
    let Some((expected_major, expected_minor)) = sysfs_dev.and_then(parse_sysfs_dev) else {
        return Err(HwError::InvalidValue {
            context: "nœud V4L2 (major/minor sysfs absent)",
            value: sysfs_dev.unwrap_or("<absent>").to_owned(),
        });
    };
    if meta.major != expected_major {
        return Err(HwError::InvalidValue {
            context: "nœud V4L2 (major sysfs)",
            value: format!("{} != {expected_major}", meta.major),
        });
    }
    if meta.minor != expected_minor {
        return Err(HwError::InvalidValue {
            context: "nœud V4L2 (minor)",
            value: format!("{} != {expected_minor}", meta.minor),
        });
    }
    if meta.uid != 0 {
        return Err(HwError::PolicyDenied {
            resource: "nœud vidéo",
            reason: "propriétaire non root",
        });
    }
    // V-09 : information, pas un refus. `0o002` est courant (`root:video
    // 0666` + ACL `uaccess`) et inoffensif pour `O_RDONLY` ; en écriture,
    // l'ouverture `O_RDWR` est filtrée par l'ACL/le groupe du noyau.
    tracing::debug!(
        gid = meta.gid,
        mode = format_args!("{:o}", meta.mode),
        world_writable = meta.mode & 0o002 != 0,
        "nœud V4L2 validé (lecture/écriture selon l'ouverture)"
    );
    Ok(())
}

/// Parse l'attribut sysfs `dev` (`81:12`).
#[must_use]
pub(crate) fn parse_sysfs_dev(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = value.trim().split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Lit l'identité d'une caméra depuis sysfs (aucun ioctl).
///
/// `uevent` d'une interface USB expose `PRODUCT=vvvv/pppp/bbbb`. Sur un
/// périphérique non USB (MIPI, virtuel), l'identité se réduit au nom lisible.
#[must_use]
pub(crate) fn identity(root: &SysfsRoot, device: &VideoDevice) -> UvcIdentity {
    let base = format!("class/video4linux/{}", device.name);
    let uevent = root
        .read_optional_string(format!("{base}/device/uevent"))
        .ok()
        .flatten()
        .unwrap_or_default();
    let (vendor, product) = parse_product(&uevent);
    let serial = root
        .read_optional_string(format!("{base}/device/serial"))
        .ok()
        .flatten()
        .map(|value| sanitize_text(&value, 64))
        .filter(|value| !value.is_empty());
    let fallback_name = device
        .device_name
        .as_deref()
        .map(|value| sanitize_text(value, 128))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| sanitize_text(&device.name, 64));
    UvcIdentity {
        vendor,
        product,
        serial,
        fallback_name,
    }
}

/// Parse `PRODUCT=` d'un `uevent` USB.
#[must_use]
pub(crate) fn parse_product(uevent: &str) -> (Option<String>, Option<String>) {
    for line in uevent.lines() {
        let Some(value) = line.strip_prefix("PRODUCT=") else {
            continue;
        };
        let mut parts = value.trim().split('/');
        let vendor = parts.next().map(|part| sanitize_text(part, 16));
        let product = parts.next().map(|part| sanitize_text(part, 16));
        return (
            vendor
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase()),
            product
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase()),
        );
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn node(major: u32, minor: u32, uid: u32, mode: u32) -> NodeMetadata {
        NodeMetadata {
            is_char: true,
            major,
            minor,
            uid,
            gid: 44,
            mode,
        }
    }

    #[test]
    fn accepts_standard_root_video_node() {
        assert!(validate_node(&node(81, 0, 0, 0o660), Some("81:0")).is_ok());
        assert!(validate_node(&node(81, 12, 0, 0o600), Some("81:12")).is_ok());
    }

    #[test]
    fn accepts_world_writable_node_for_read_only_use() {
        // Cas réel : `crwxrwxrwx+ root video` (0666 + ACL `uaccess`) sur une
        // webcam FHD. L'ouverture est `O_RDONLY` : le bit d'écriture monde ne
        // donne aucun droit à notre descripteur.
        assert!(validate_node(&node(81, 0, 0, 0o666), Some("81:0")).is_ok());
        assert!(validate_node(&node(81, 1, 0, 0o664), Some("81:1")).is_ok());
        assert!(validate_node(&node(81, 2, 0, 0o777), Some("81:2")).is_ok());
    }

    #[test]
    fn rejects_wrong_type_major_minor_or_owner() {
        let mut wrong_type = node(81, 0, 0, 0o660);
        wrong_type.is_char = false;
        assert!(matches!(
            validate_node(&wrong_type, Some("81:0")),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            validate_node(&node(80, 0, 0, 0o660), Some("80:0")),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            validate_node(&node(81, 1, 0, 0o660), Some("81:0")),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            validate_node(&node(81, 0, 0, 0o660), Some("80:0")),
            Err(HwError::InvalidValue { .. })
        ));
        // Un mode permissif n'affaiblit pas les autres contrôles.
        assert!(matches!(
            validate_node(&node(81, 0, 1000, 0o660), Some("81:0")),
            Err(HwError::PolicyDenied { .. })
        ));
        assert!(matches!(
            validate_node(&node(81, 0, 1000, 0o666), Some("81:0")),
            Err(HwError::PolicyDenied { .. })
        ));
        assert!(matches!(
            validate_node(&node(81, 0, 0, 0o660), None),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn parses_sysfs_dev() {
        assert_eq!(parse_sysfs_dev("81:0\n"), Some((81, 0)));
        assert_eq!(parse_sysfs_dev("81:12"), Some((81, 12)));
        assert_eq!(parse_sysfs_dev("garbage"), None);
        assert_eq!(parse_sysfs_dev("81:"), None);
        assert_eq!(parse_sysfs_dev("81:12:3"), None);
    }

    #[test]
    fn parses_usb_product() {
        assert_eq!(
            parse_product("DEVTYPE=usb_interface\nPRODUCT=1bcf/2c99/7200\n"),
            (Some("1bcf".to_owned()), Some("2c99".to_owned()))
        );
        assert_eq!(
            parse_product("PRODUCT=1BCF/2C99/7200\n"),
            (Some("1bcf".to_owned()), Some("2c99".to_owned()))
        );
        assert_eq!(parse_product("DRIVER=uvcvideo\n"), (None, None));
        assert_eq!(parse_product("PRODUCT=\n"), (None, None));
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn device(name: &str) -> VideoDevice {
        VideoDevice {
            name: name.to_owned(),
            device_path: Some(format!("/dev/{name}")),
            device_name: Some("Integrated Camera\n".to_owned()),
            index: Some(0),
            dev: Some("81:0".to_owned()),
        }
    }

    #[test]
    fn reads_identity_from_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/video4linux/video0/device/uevent",
            "DEVTYPE=usb_interface\nPRODUCT=1bcf/2c99/7200\n",
        );
        write(
            dir.path(),
            "class/video4linux/video0/device/serial",
            "SN123\n",
        );
        let identity = identity(&SysfsRoot::new(dir.path()), &device("video0"));
        assert_eq!(identity.vendor.as_deref(), Some("1bcf"));
        assert_eq!(identity.product.as_deref(), Some("2c99"));
        assert_eq!(identity.serial.as_deref(), Some("SN123"));
        assert_eq!(identity.fallback_name, "Integrated Camera");
    }

    #[test]
    fn identity_falls_back_on_missing_uevent() {
        let dir = tempfile::tempdir().unwrap();
        let identity = identity(&SysfsRoot::new(dir.path()), &device("video3"));
        assert_eq!(identity.vendor, None);
        assert_eq!(identity.product, None);
        assert_eq!(identity.serial, None);
        assert_eq!(identity.fallback_name, "Integrated Camera");
    }
}
