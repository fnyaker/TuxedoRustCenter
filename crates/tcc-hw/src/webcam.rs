// SPDX-License-Identifier: GPL-3.0-or-later
//! Webcam : kill switch matériel et énumération V4L2.
//!
//! # Kill switch
//!
//! Le commutateur Clevo est piloté par `/dev/tuxedo_io` :
//!
//! ```text
//! R_CL_WEBCAM_SW    lecture (0/1)
//! W_CL_WEBCAM_SW    écriture (0/1)
//! ```
//!
//! Uniwill n'implémente pas ces requêtes : [`WebcamSwitch::is_available`]
//! retourne `false` et toute écriture échoue en `NotSupported`, comme
//! `TuxedoIOAPI` dans l'original.
//!
//! # Énumération
//!
//! ```text
//! /sys/class/video4linux/videoN/name    nom du périphérique
//! /sys/class/video4linux/videoN/index   index du nœud (0 = capture, 1 = metadata)
//! /sys/class/video4linux/videoN/dev     major:minor
//! ```
//!
//! Le nœud de capture est `/dev/videoN` (dérivé du nom sysfs validé).
//!
//! # Limite assumée : contrôles UVC
//!
//! L'original appliquait les préréglages webcam (luminosité, contraste,
//! exposition, ...) via un script Python `cameractrls.py` appelé par udev.
//! Les contrôles UVC nécessitent des `ioctl` `VIDIOC_QUERYCTRL` /
//! `VIDIOC_G_CTRL` / `VIDIOC_S_CTRL`. La règle du projet confine `unsafe` au
//! wrapper `tuxedo_io` : ce crate **n'implémente donc pas** ces ioctls.
//!
//! Deux options sont proposées à `security` et `linux-platform` (voir
//! `docs/hardware/HARDWARE_MATRIX.md`) :
//!
//! 1. ajouter un module `tcc-hw/src/v4l2.rs` isolé, avec `unsafe` commenté et
//!    revu, sur le modèle de `tuxedo_io::real` ;
//! 2. utiliser une crate V4L2 auditée (licence compatible) et l'envelopper.
//!
//! En attendant, le daemon expose `webcamControlsSupported: false` et l'UI
//! n'affiche pas les réglages UVC. Le kill switch et l'énumération restent
//! pleinement fonctionnels.

use std::sync::Arc;

use crate::error::Result;
use crate::sysfs::SysfsRoot;
use crate::tuxedo_io::TuxedoIo;

/// Classe sysfs des périphériques vidéo.
const CLASS_VIDEO4LINUX: &str = "class/video4linux";
/// Longueur maximale d'un nom de périphérique conservé.
const MAX_DEVICE_NAME: usize = 128;

/// Nœud vidéo V4L2 découvert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDevice {
    /// Nom sysfs (`video0`).
    pub name: String,
    /// Chemin du nœud caractère (`/dev/video0`), `None` si le nom est
    /// inattendu.
    pub device_path: Option<String>,
    /// Nom lisible exposé par le noyau, tronqué à 128 caractères.
    pub device_name: Option<String>,
    /// Index du nœud (0 = capture, 1 = metadata UVC).
    pub index: Option<u32>,
    /// `major:minor`.
    pub dev: Option<String>,
}

impl VideoDevice {
    /// Vrai si le nœud est le flux de capture (index 0).
    #[must_use]
    pub fn is_capture(&self) -> bool {
        self.index.is_none_or(|index| index == 0)
    }

    /// Vrai si le nœud est un nœud de métadonnées UVC.
    #[must_use]
    pub fn is_metadata(&self) -> bool {
        self.index.is_some_and(|index| index > 0)
    }
}

/// Énumère les nœuds `/sys/class/video4linux`.
///
/// # Errors
///
/// Erreur d'E/S réelle lors de l'énumération.
pub fn enumerate(root: &SysfsRoot) -> Result<Vec<VideoDevice>> {
    let mut devices = Vec::new();
    for entry in root.list_dir(CLASS_VIDEO4LINUX)? {
        let Some(number) = valid_video_name(&entry) else {
            continue;
        };
        let base = format!("{CLASS_VIDEO4LINUX}/{entry}");
        let device_name = root
            .read_optional_string(format!("{base}/name"))?
            .map(|value| truncate(&value, MAX_DEVICE_NAME))
            .filter(|value| !value.is_empty());
        let index = root
            .read_optional_string(format!("{base}/index"))?
            .and_then(|value| value.trim().parse::<u32>().ok());
        let dev = root
            .read_optional_string(format!("{base}/dev"))?
            .filter(|value| is_plausible_dev(value));
        devices.push(VideoDevice {
            name: entry,
            device_path: Some(format!("/dev/video{number}")),
            device_name,
            index,
            dev,
        });
    }
    Ok(devices)
}

/// Contrôle du kill switch webcam.
#[derive(Debug, Clone)]
pub struct WebcamSwitch {
    io: Arc<TuxedoIo>,
}

impl WebcamSwitch {
    /// Construit le contrôleur sur une interface déjà ouverte.
    #[must_use]
    pub fn new(io: Arc<TuxedoIo>) -> Self {
        Self { io }
    }

    /// Vrai si l'interface expose le commutateur.
    ///
    /// Uniwill retourne `false` ; Clevo `true` dès qu'un ioctl réussit.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.io.webcam_enabled().is_ok()
    }

    /// État courant (`true` = webcam activée).
    ///
    /// # Errors
    ///
    /// [`crate::HwError::NotSupported`] sur Uniwill ou si l'ioctl échoue.
    pub fn status(&self) -> Result<bool> {
        self.io.webcam_enabled()
    }

    /// Active ou coupe la webcam.
    ///
    /// # Errors
    ///
    /// [`crate::HwError::NotSupported`] sur Uniwill, erreur ioctl sinon.
    pub fn set(&self, enabled: bool) -> Result<()> {
        self.io.set_webcam_enabled(enabled)
    }
}

/// Extrait le numéro d'un nom `videoN`, ou `None` si le nom ne correspond pas.
fn valid_video_name(name: &str) -> Option<u32> {
    name.strip_prefix("video")
        .and_then(|value| value.parse::<u32>().ok())
}

fn is_plausible_dev(value: &str) -> bool {
    let value = value.trim();
    match value.split_once(':') {
        Some((major, minor)) => {
            !major.is_empty()
                && !minor.is_empty()
                && major.bytes().all(|byte| byte.is_ascii_digit())
                && minor.bytes().all(|byte| byte.is_ascii_digit())
        }
        None => false,
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Construit un contrôleur de kill switch depuis une interface `tuxedo-io`.
///
/// Utilitaire pour les tests et le daemon : retourne `None` si l'interface est
/// absente.
#[must_use]
pub fn detect_switch(io: Option<TuxedoIo>) -> Option<WebcamSwitch> {
    let io = io?;
    let switch = WebcamSwitch::new(Arc::new(io));
    switch.is_available().then_some(switch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HwError;
    use crate::tuxedo_io::fake::FakeTransport;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn clevo_switch() -> WebcamSwitch {
        WebcamSwitch::new(Arc::new(
            TuxedoIo::with_transport(Box::new(FakeTransport::clevo())).unwrap(),
        ))
    }

    fn uniwill_switch() -> WebcamSwitch {
        WebcamSwitch::new(Arc::new(
            TuxedoIo::with_transport(Box::new(FakeTransport::uniwill())).unwrap(),
        ))
    }

    #[test]
    fn clevo_switch_reads_and_writes() {
        let switch = clevo_switch();
        assert!(switch.is_available());
        assert!(switch.status().unwrap());
        switch.set(false).unwrap();
        assert!(!switch.status().unwrap());
        switch.set(true).unwrap();
        assert!(switch.status().unwrap());
    }

    #[test]
    fn uniwill_has_no_switch() {
        let switch = uniwill_switch();
        assert!(!switch.is_available());
        assert!(matches!(switch.status(), Err(HwError::NotSupported(_))));
        assert!(matches!(switch.set(false), Err(HwError::NotSupported(_))));
    }

    #[test]
    fn detects_switch_from_optional_io() {
        assert!(detect_switch(None).is_none());
        assert!(
            detect_switch(Some(
                TuxedoIo::with_transport(Box::new(FakeTransport::uniwill())).unwrap()
            ))
            .is_none()
        );
        assert!(
            detect_switch(Some(
                TuxedoIo::with_transport(Box::new(FakeTransport::clevo())).unwrap()
            ))
            .is_some()
        );
    }

    #[test]
    fn enumerates_video_nodes() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "class/video4linux/video0/name",
            "Integrated Camera: Integrated C\n",
        );
        write(dir.path(), "class/video4linux/video0/index", "0\n");
        write(dir.path(), "class/video4linux/video0/dev", "81:0\n");
        write(
            dir.path(),
            "class/video4linux/video1/name",
            "Integrated Camera: Integrated Metadata\n",
        );
        write(dir.path(), "class/video4linux/video1/index", "1\n");
        write(dir.path(), "class/video4linux/video1/dev", "81:1\n");
        // Entrée parasite ignorée.
        write(dir.path(), "class/video4linux/not-a-video/name", "x");

        let devices = enumerate(&SysfsRoot::new(dir.path())).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "video0");
        assert_eq!(devices[0].device_path.as_deref(), Some("/dev/video0"));
        assert_eq!(devices[0].index, Some(0));
        assert!(devices[0].is_capture());
        assert!(!devices[0].is_metadata());
        assert_eq!(devices[0].dev.as_deref(), Some("81:0"));
        assert!(devices[1].is_metadata());
    }

    #[test]
    fn malformed_attributes_degrade_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/video4linux/video2/name", "Cam\n");
        write(
            dir.path(),
            "class/video4linux/video2/index",
            "not-a-number\n",
        );
        write(dir.path(), "class/video4linux/video2/dev", "garbage\n");
        let devices = enumerate(&SysfsRoot::new(dir.path())).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].index, None);
        assert_eq!(devices[0].dev, None);
        assert!(devices[0].is_capture());
    }

    #[test]
    fn empty_enumeration_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(enumerate(&SysfsRoot::new(dir.path())).unwrap().is_empty());
    }

    #[test]
    fn video_name_validation() {
        assert_eq!(valid_video_name("video0"), Some(0));
        assert_eq!(valid_video_name("video12"), Some(12));
        assert_eq!(valid_video_name("video"), None);
        assert_eq!(valid_video_name("videoX"), None);
        assert_eq!(valid_video_name("card0"), None);
    }
}
