// SPDX-License-Identifier: GPL-3.0-or-later
//! ABI V4L2 minimale : ioctl de lecture et `S_CTRL` d'écriture, structures
//! `#[repr(C)]`.
//!
//! Ce module est **sans `unsafe`** : il ne contient que des constantes, des
//! structures de données et des assertions de disposition. Les appels `ioctl`
//! sont dans [`super::real`] (lecture) et `super::real_write` (écriture).
//!
//! Les valeurs sont recopiées de `linux/videodev2.h` et vérifiées par les
//! assertions de taille/offset et les tests d'égalité des constantes.
//!
//! # ioctl exposés
//!
//! | Constante | Requête | Taille | Sens |
//! |---|---|---|---|
//! | `VIDIOC_QUERYCAP` | `0x80685600` | 104 | lecture |
//! | `VIDIOC_QUERYCTRL` | `0xC0445624` | 68 | lecture/écriture |
//! | `VIDIOC_QUERYMENU` | `0xC02C5625` | 44 | lecture/écriture |
//! | `VIDIOC_G_CTRL` | `0xC008561B` | 8 | lecture/écriture |
//! | `VIDIOC_S_CTRL` | `0xC008561C` | 8 | écriture (module `write` uniquement) |
//!
//! `VIDIOC_S_CTRL` n'est utilisé que par [`super::write`] / `real_write.rs` ;
//! le lecteur (`real.rs`) et le daemon ne l'appellent jamais. Un test statique
//! (`tests/v4l2_unsafe_guard.rs`) vérifie cette séparation.
//!
//! `VIDIOC_QUERYCAP` est un `_IOR`, mais reçoit un pointeur mutable comme tous
//! les ioctl V4L2.

#![forbid(unsafe_code)]

use std::mem::{offset_of, size_of};

// ---------------------------------------------------------------------------
// Encodage ioctl Linux
// ---------------------------------------------------------------------------

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;
/// Type `'V'` de `videodev2.h`.
const IOC_TYPE_V: u8 = b'V';

const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> u64 {
    ((dir as u64) << IOC_DIRSHIFT)
        | ((size as u64) << IOC_SIZESHIFT)
        | ((kind as u64) << IOC_TYPESHIFT)
        | ((nr as u64) << IOC_NRSHIFT)
}

const fn ior(kind: u8, nr: u8, size: usize) -> u64 {
    ioc(IOC_READ, kind, nr, size)
}

const fn iowr(kind: u8, nr: u8, size: usize) -> u64 {
    ioc(IOC_READ | IOC_WRITE, kind, nr, size)
}

/// `VIDIOC_QUERYCAP` — capacités du périphérique.
pub(crate) const VIDIOC_QUERYCAP: u64 = ior(IOC_TYPE_V, 0x00, size_of::<V4l2Capability>());
/// `VIDIOC_QUERYCTRL` — métadonnées d'un contrôle.
pub(crate) const VIDIOC_QUERYCTRL: u64 = iowr(IOC_TYPE_V, 0x24, size_of::<V4l2QueryCtrl>());
/// `VIDIOC_QUERYMENU` — entrée de menu d'un contrôle.
pub(crate) const VIDIOC_QUERYMENU: u64 = iowr(IOC_TYPE_V, 0x25, size_of::<V4l2QueryMenu>());
/// `VIDIOC_G_CTRL` — valeur courante d'un contrôle.
pub(crate) const VIDIOC_G_CTRL: u64 = iowr(IOC_TYPE_V, 0x1b, size_of::<V4l2Control>());
/// `VIDIOC_S_CTRL` — écriture d'un contrôle (module `write` uniquement).
pub(crate) const VIDIOC_S_CTRL: u64 = iowr(IOC_TYPE_V, 0x1c, size_of::<V4l2Control>());

// ---------------------------------------------------------------------------
// Structures noyau
// ---------------------------------------------------------------------------

/// `struct v4l2_capability` (104 octets).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct V4l2Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

/// `struct v4l2_control` (8 octets).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct V4l2Control {
    pub id: u32,
    pub value: i32,
}

/// `struct v4l2_queryctrl` (68 octets).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct V4l2QueryCtrl {
    pub id: u32,
    pub ctrl_type: u32,
    pub name: [u8; 32],
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default_value: i32,
    pub flags: u32,
    pub reserved: [u32; 2],
}

/// `struct v4l2_querymenu` (44 octets, `packed` côté noyau).
///
/// Le noyau utilise une union `name[32]` / `__s64 value`. On la représente par
/// un tableau de 32 octets, ce qui donne exactement la même taille (44) sans
/// union Rust ni `unsafe` supplémentaire. Le champ `reserved` doit être mis à
/// zéro avant l'appel.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct V4l2QueryMenu {
    pub id: u32,
    pub index: u32,
    pub payload: [u8; 32],
    pub reserved: u32,
}

// ---------------------------------------------------------------------------
// Constantes de contrôle
// ---------------------------------------------------------------------------

/// `V4L2_CTRL_FLAG_NEXT_CTRL` : énumération du contrôle suivant.
pub(crate) const V4L2_CTRL_FLAG_NEXT_CTRL: u32 = 0x8000_0000;
/// `V4L2_CTRL_FLAG_NEXT_COMPOUND` : contrôles composés. Jamais demandé.
pub(crate) const V4L2_CTRL_FLAG_NEXT_COMPOUND: u32 = 0x4000_0000;

/// `V4L2_CTRL_FLAG_DISABLED`.
pub(crate) const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;
/// `V4L2_CTRL_FLAG_GRABBED`.
pub(crate) const V4L2_CTRL_FLAG_GRABBED: u32 = 0x0002;
/// `V4L2_CTRL_FLAG_READ_ONLY`.
pub(crate) const V4L2_CTRL_FLAG_READ_ONLY: u32 = 0x0004;
/// `V4L2_CTRL_FLAG_UPDATE`.
pub(crate) const V4L2_CTRL_FLAG_UPDATE: u32 = 0x0008;
/// `V4L2_CTRL_FLAG_INACTIVE`.
pub(crate) const V4L2_CTRL_FLAG_INACTIVE: u32 = 0x0010;
/// `V4L2_CTRL_FLAG_WRITE_ONLY`.
pub(crate) const V4L2_CTRL_FLAG_WRITE_ONLY: u32 = 0x0040;
/// `V4L2_CTRL_FLAG_HAS_PAYLOAD` : contrôle composé, non supporté.
pub(crate) const V4L2_CTRL_FLAG_HAS_PAYLOAD: u32 = 0x0100;
/// `V4L2_CTRL_FLAG_EXECUTE_ON_WRITE` : action implicite, non supporté.
pub(crate) const V4L2_CTRL_FLAG_EXECUTE_ON_WRITE: u32 = 0x0200;

/// `V4L2_CTRL_TYPE_INTEGER`.
pub(crate) const V4L2_CTRL_TYPE_INTEGER: u32 = 1;
/// `V4L2_CTRL_TYPE_BOOLEAN`.
pub(crate) const V4L2_CTRL_TYPE_BOOLEAN: u32 = 2;
/// `V4L2_CTRL_TYPE_MENU`.
pub(crate) const V4L2_CTRL_TYPE_MENU: u32 = 3;
/// `V4L2_CTRL_TYPE_BUTTON` (jamais exposé).
pub(crate) const V4L2_CTRL_TYPE_BUTTON: u32 = 4;
/// `V4L2_CTRL_TYPE_INTEGER_MENU`.
pub(crate) const V4L2_CTRL_TYPE_INTEGER_MENU: u32 = 9;

/// `V4L2_CAP_VIDEO_CAPTURE`.
pub(crate) const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
/// `V4L2_CAP_DEVICE_CAPS` : `device_caps` est autoritaire.
pub(crate) const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;
/// `V4L2_CAP_META_CAPTURE`.
pub(crate) const V4L2_CAP_META_CAPTURE: u32 = 0x0080_0000;

/// Major des périphériques vidéo (`VIDEO_MAJOR`).
pub(crate) const VIDEO_MAJOR: u32 = 81;

/// Masque les flags de requête pour ne garder que l'identifiant du contrôle.
pub(crate) const QUERY_FLAG_MASK: u32 = V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;

/// Retire les flags `NEXT_*` d'un `id` retourné par `QUERYCTRL`.
#[must_use]
pub(crate) const fn mask_query_flags(id: u32) -> u32 {
    id & !QUERY_FLAG_MASK
}

// ---------------------------------------------------------------------------
// Assertions de disposition (vérifiées à la compilation)
// ---------------------------------------------------------------------------

const _: () = {
    assert!(size_of::<V4l2Capability>() == 104);
    assert!(size_of::<V4l2Control>() == 8);
    assert!(size_of::<V4l2QueryCtrl>() == 68);
    assert!(size_of::<V4l2QueryMenu>() == 44);
    assert!(std::mem::align_of::<V4l2QueryMenu>() == 4);

    assert!(offset_of!(V4l2Control, id) == 0);
    assert!(offset_of!(V4l2Control, value) == 4);
    assert!(offset_of!(V4l2QueryCtrl, id) == 0);
    assert!(offset_of!(V4l2QueryCtrl, ctrl_type) == 4);
    assert!(offset_of!(V4l2QueryCtrl, name) == 8);
    assert!(offset_of!(V4l2QueryCtrl, minimum) == 40);
    assert!(offset_of!(V4l2QueryCtrl, maximum) == 44);
    assert!(offset_of!(V4l2QueryCtrl, step) == 48);
    assert!(offset_of!(V4l2QueryCtrl, default_value) == 52);
    assert!(offset_of!(V4l2QueryCtrl, flags) == 56);

    assert!(offset_of!(V4l2QueryMenu, payload) == 8);
    assert!(offset_of!(V4l2QueryMenu, reserved) == 40);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_kernel_header() {
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        assert_eq!(VIDIOC_G_CTRL, 0xC008_561B);
        assert_eq!(VIDIOC_S_CTRL, 0xC008_561C);
        assert_eq!(VIDIOC_QUERYCTRL, 0xC044_5624);
        assert_eq!(VIDIOC_QUERYMENU, 0xC02C_5625);
    }

    #[test]
    fn query_flags_are_masked() {
        assert_eq!(
            mask_query_flags(V4L2_CTRL_FLAG_NEXT_CTRL | 0x0098_0900),
            0x0098_0900
        );
        assert_eq!(
            mask_query_flags(V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND | 0x0098_0901),
            0x0098_0901
        );
        assert_eq!(mask_query_flags(0), 0);
    }

    #[test]
    fn control_flags_match_kernel_header() {
        assert_eq!(V4L2_CTRL_FLAG_DISABLED, 0x0001);
        assert_eq!(V4L2_CTRL_FLAG_GRABBED, 0x0002);
        assert_eq!(V4L2_CTRL_FLAG_READ_ONLY, 0x0004);
        assert_eq!(V4L2_CTRL_FLAG_UPDATE, 0x0008);
        assert_eq!(V4L2_CTRL_FLAG_INACTIVE, 0x0010);
        assert_eq!(V4L2_CTRL_FLAG_WRITE_ONLY, 0x0040);
        assert_eq!(V4L2_CTRL_FLAG_HAS_PAYLOAD, 0x0100);
        assert_eq!(V4L2_CTRL_FLAG_EXECUTE_ON_WRITE, 0x0200);
        assert_eq!(V4L2_CTRL_FLAG_NEXT_COMPOUND, 0x4000_0000);
    }

    #[test]
    fn s_ctrl_is_defined_for_the_write_module_only() {
        // Le test statique vérifie que `S_CTRL` n'est utilisé que par
        // `abi.rs` et `real_write.rs`.
        assert_eq!(VIDIOC_S_CTRL, 0xC008_561C);
    }
}
