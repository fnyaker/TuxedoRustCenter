// SPDX-License-Identifier: GPL-3.0-or-later
//! Transport V4L2 réel — **seul module avec `unsafe`**.
//!
//! Quatre ioctl de lecture seulement : `QUERYCAP`, `QUERYCTRL`, `QUERYMENU`,
//! `G_CTRL`. Aucun `S_CTRL`, aucun ioctl de flux (`REQBUFS`, `QBUF`,
//! `STREAMON`), aucun `mmap`/`read`/`write`.
//!
//! # Sûreté
//!
//! Chaque appel `libc::ioctl` est commenté `SAFETY` :
//! - la requête est une constante de [`super::abi`] ;
//! - le descripteur provient d'un [`File`] ouvert et vivant ;
//! - le pointeur désigne une structure `#[repr(C)]` vivante pour la durée de
//!   l'appel, de taille identique à celle encodée dans la requête.
//!
//! L'ouverture est en `O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC` ; le nœud est
//! validé par [`super::device::validate_node`] (caractère, major 81, minor
//! sysfs, root). Le mode `0666` + ACL `uaccess` est courant et accepté : un
//! descripteur `O_RDONLY` n'ouvre aucun droit d'écriture.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::error::{HwError, Result};
use crate::webcam::VideoDevice;

use super::abi;
use super::control;
use super::device::{NodeMetadata, validate_node};
use super::{ControlQuery, DeviceCapabilities, MenuQuery, V4l2Reader};

/// Flags d'ouverture communs : lecture seule, non bloquant, sans suivi de
/// lien symbolique, fermé à l'`exec`.
pub(crate) const OPEN_FLAGS: i32 = libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;

/// Construit les options d'ouverture d'un nœud V4L2.
fn open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(OPEN_FLAGS);
    options
}

/// Ouvre un device V4L2 et retourne le transport de lecture.
pub(crate) fn open(device: &VideoDevice) -> Result<Box<dyn V4l2Reader>> {
    Ok(Box::new(RealReader::open(device)?))
}

/// Transport réel sur un fd V4L2.
#[derive(Debug)]
pub(crate) struct RealReader {
    file: File,
}

impl RealReader {
    fn open(device: &VideoDevice) -> Result<Self> {
        let Some(number) = device
            .name
            .strip_prefix("video")
            .and_then(|value| value.parse::<u32>().ok())
        else {
            return Err(HwError::NotSupported("nom de nœud V4L2"));
        };
        // Le chemin est reconstruit depuis le nom sysfs, jamais depuis un
        // chemin fourni par un client ; `device_path` est ignoré.
        let path = PathBuf::from(format!("/dev/video{number}"));

        let file = match open_options().open(&path) {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(HwError::UnsafeTarget { path });
            }
            Err(error) => return Err(HwError::io_or_not_found(path, error)),
        };

        let metadata = file.metadata().map_err(|error| HwError::io(&path, error))?;
        let metadata = NodeMetadata {
            is_char: metadata.file_type().is_char_device(),
            major: libc::major(metadata.rdev()),
            minor: libc::minor(metadata.rdev()),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
        };
        validate_node(&metadata, device.dev.as_deref())?;
        Ok(Self { file })
    }

    /// Appel ioctl avec un pointeur mutable vers une structure `#[repr(C)]`.
    fn ioctl<T>(&self, request: u64, name: &'static str, argument: &mut T) -> Result<()> {
        // SAFETY: `request` est une constante de `abi` dont la taille encodée
        // correspond à `T` (assertions de `abi`). `argument` est une structure
        // vivante sur la pile pour toute la durée de l'appel ; le noyau écrit
        // au plus `size_of::<T>()` octets. `file` reste ouvert pendant l'appel
        // et son fd est valide.
        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                request as libc::c_ulong,
                argument as *mut T,
            )
        };
        if result < 0 {
            Err(HwError::Ioctl {
                name,
                request,
                source: io::Error::last_os_error(),
            })
        } else {
            Ok(())
        }
    }
}

impl V4l2Reader for RealReader {
    fn capabilities(&self) -> Result<DeviceCapabilities> {
        let mut raw = abi::V4l2Capability::default();
        self.ioctl(abi::VIDIOC_QUERYCAP, "VIDIOC_QUERYCAP", &mut raw)?;
        Ok(DeviceCapabilities {
            driver: cstr(&raw.driver),
            card: cstr(&raw.card),
            bus_info: cstr(&raw.bus_info),
            capabilities: raw.capabilities,
            device_caps: raw.device_caps,
        })
    }

    fn query_control(&self, id: u32) -> Result<ControlQuery> {
        let mut raw = abi::V4l2QueryCtrl {
            id,
            ..abi::V4l2QueryCtrl::default()
        };
        self.ioctl(abi::VIDIOC_QUERYCTRL, "VIDIOC_QUERYCTRL", &mut raw)?;
        Ok(ControlQuery {
            id: abi::mask_query_flags(raw.id),
            kind_raw: raw.ctrl_type,
            name: cstr(&raw.name),
            min: raw.minimum,
            max: raw.maximum,
            step: raw.step,
            default: raw.default_value,
            flags: raw.flags,
        })
    }

    fn query_menu(&self, id: u32, index: i32) -> Result<MenuQuery> {
        let index = u32::try_from(index).map_err(|_| HwError::InvalidValue {
            context: "index de menu V4L2",
            value: index.to_string(),
        })?;
        let mut raw = abi::V4l2QueryMenu {
            id,
            index,
            ..abi::V4l2QueryMenu::default()
        };
        self.ioctl(abi::VIDIOC_QUERYMENU, "VIDIOC_QUERYMENU", &mut raw)?;
        let label = {
            let value = cstr(&raw.payload);
            if value.is_empty() { None } else { Some(value) }
        };
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&raw.payload[..8]);
        Ok(MenuQuery {
            label,
            value: Some(i64::from_ne_bytes(bytes)),
        })
    }

    fn get_control(&self, id: u32) -> Result<i32> {
        let mut raw = abi::V4l2Control { id, value: 0 };
        self.ioctl(abi::VIDIOC_G_CTRL, "VIDIOC_G_CTRL", &mut raw)?;
        Ok(raw.value)
    }
}

/// Décode une chaîne C bornée (32 ou 16 octets) en texte sanitisé.
fn cstr(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    control::sanitize_text(&String::from_utf8_lossy(&bytes[..end]), 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> VideoDevice {
        VideoDevice {
            name: name.to_owned(),
            device_path: Some(format!("/dev/{name}")),
            device_name: None,
            index: Some(0),
            dev: Some("81:0".to_owned()),
        }
    }

    #[test]
    fn missing_device_is_not_found() {
        let path = PathBuf::from("/dev/video9999");
        if path.exists() {
            return;
        }
        let result = open(&device("video9999"));
        assert!(
            matches!(
                result,
                Err(HwError::NotFound { .. }) | Err(HwError::Unavailable { .. })
            ),
            "{result:?}"
        );
    }

    #[test]
    fn invalid_node_name_is_refused_before_open() {
        for name in ["", "camera0", "video", "videoX", "../video0"] {
            let result = open(&device(name));
            assert!(
                matches!(
                    result,
                    Err(HwError::NotSupported(_)) | Err(HwError::InvalidPath { .. })
                ),
                "{name}: {result:?}"
            );
        }
    }

    #[test]
    fn cstr_strips_nul_and_control_characters() {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(b"Camera\n\0");
        assert_eq!(cstr(&bytes), "Camera");
    }

    #[test]
    fn open_flags_include_no_follow_and_cloexec() {
        assert_ne!(OPEN_FLAGS & libc::O_NOFOLLOW, 0);
        assert_ne!(OPEN_FLAGS & libc::O_CLOEXEC, 0);
        assert_ne!(OPEN_FLAGS & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn open_options_are_read_only() {
        use std::io::{Read, Write};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("file");
        std::fs::write(&path, b"data").expect("écriture");
        let mut file = open_options().open(&path).expect("ouverture lecture");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("lecture");
        assert_eq!(contents, "data");
        let error = file.write(b"x").expect_err("écriture acceptée");
        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
    }

    #[test]
    fn symlink_is_refused_by_open_flags() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        std::fs::write(&target, b"").expect("écriture cible");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = open_options().open(&link).expect_err("symlink accepté");
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
    }
}
