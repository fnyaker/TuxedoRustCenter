// SPDX-License-Identifier: GPL-3.0-or-later
//! Transport V4L2 réel d'écriture — **seul module avec `S_CTRL`**.
//!
//! Ouverture `O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC` sur `/dev/videoN`
//! reconstruit depuis le nom sysfs, validation `fstat` (caractère, major 81,
//! minor sysfs, uid root), puis `QUERYCAP` via le contrôleur. L'autorisation
//! d'ouverture (ACL `uaccess`, groupe `video`) est celle de l'utilisateur de
//! session : `tccd-user` n'est ni root ni privilégié et l'expose via
//! `User1.SetUvcControl`. `tccd` n'ouvre jamais ce transport.
//!
//! Les ioctl utilisés sont `QUERYCAP`, `QUERYCTRL`, `QUERYMENU`, `G_CTRL` et
//! `S_CTRL`. Aucun `REQBUFS`/`QBUF`/`STREAMON`, aucun `mmap`, aucun flux.

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
use super::write::UvcWriter;
use super::{ControlQuery, DeviceCapabilities, MenuQuery};

/// Flags d'ouverture : lecture + écriture, non bloquant, sans suivi de lien,
/// fermé à l'`exec`.
pub(crate) const OPEN_FLAGS: i32 = libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;

/// Ouvre un device V4L2 en lecture/écriture.
pub(crate) fn open(device: &VideoDevice) -> Result<Box<dyn UvcWriter>> {
    Ok(Box::new(RealWriter::open(device)?))
}

/// Transport réel d'écriture sur un fd V4L2.
#[derive(Debug)]
pub(crate) struct RealWriter {
    file: File,
}

impl RealWriter {
    fn open(device: &VideoDevice) -> Result<Self> {
        let Some(number) = device
            .name
            .strip_prefix("video")
            .and_then(|value| value.parse::<u32>().ok())
        else {
            return Err(HwError::NotSupported("nom de nœud V4L2"));
        };
        let path = PathBuf::from(format!("/dev/video{number}"));
        let file = match open_options().open(&path) {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(HwError::UnsafeTarget { path });
            }
            Err(error) => return Err(HwError::io_or_not_found(&path, error)),
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
        // correspond à `T`. `argument` est vivant pendant tout l'appel et le
        // noyau écrit au plus `size_of::<T>()` octets. Le fd de `file` reste
        // ouvert et valide pendant l'appel.
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

fn open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).custom_flags(OPEN_FLAGS);
    options
}

impl UvcWriter for RealWriter {
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
            (!value.is_empty()).then_some(value)
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

    fn set_control(&self, id: u32, value: i32) -> Result<()> {
        let mut raw = abi::V4l2Control { id, value };
        self.ioctl(abi::VIDIOC_S_CTRL, "VIDIOC_S_CTRL", &mut raw)
    }
}

/// Décode une chaîne C bornée en texte sanitisé.
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
    use std::path::Path;

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
    fn open_flags_are_read_write_no_follow() {
        assert_ne!(OPEN_FLAGS & libc::O_NOFOLLOW, 0);
        assert_ne!(OPEN_FLAGS & libc::O_CLOEXEC, 0);
        assert_ne!(OPEN_FLAGS & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn open_options_allow_read_and_write() {
        use std::io::{Read, Seek, SeekFrom, Write};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("file");
        std::fs::write(&path, b"data").expect("write");
        let mut file = open_options().open(&path).expect("open");
        let written = file.write(b"X").expect("write access");
        assert_eq!(written, 1);
        file.seek(SeekFrom::Start(0)).expect("seek");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("read");
        assert_eq!(contents, "Xata");
    }

    #[test]
    fn symlink_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        std::fs::write(&target, b"").expect("write");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let error = open_options().open(&link).expect_err("symlink accepté");
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
    }

    #[test]
    fn missing_device_is_not_found() {
        let path = Path::new("/dev/video9999");
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
}
