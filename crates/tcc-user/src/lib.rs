// SPDX-License-Identifier: GPL-3.0-or-later
//! Agent de session `tccd-user` : détection de session, backends d'affichage et
//! CLI de diagnostic.
//!
//! L'agent est **non privilégié**. Il applique la résolution et/ou le taux de
//! l'écran interne via le compositeur ou XRandR, et publie ses capacités
//! d'affichage et d'écriture UVC sur le **bus de session**
//! (`com.tuxedocomputers.tccd.User`, interface `User1`).
//!
//! L'écriture UVC (`User1.SetUvcControl`) reste dans ce processus : le daemon
//! root `tccd` ne compile ni n'appelle jamais `VIDIOC_S_CTRL`. Le device n'est
//! ouvert qu'à l'appel, après validation du chemin énuméré et de la clé.
//!
//! Règles :
//!
//! - refus d'exécution en root ;
//! - jamais d'écran externe modifié ;
//! - résolution optionnelle, jamais un écran externe ;
//! - aucun shell, aucun `/proc/*/environ` ;
//! - erreurs typées, pas de panic sur les chemins de production.

#![deny(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![warn(missing_debug_implementations)]

pub mod agent;
pub mod backend;
pub mod cli;
pub mod error;
pub mod ipc;
mod rate_limit;
pub mod security;
pub mod session;
pub mod session_service;
pub mod uvc;

pub use error::{Result, UserError};
pub use session::{SessionEnv, SessionInfo, SessionType};
pub use session_service::{
    DISPLAY_INFO_CACHE, DisplayInfo, DisplayInfoSource, MAX_SESSION_MESSAGE_BODY_BYTES,
    RealDisplayInfoSource, SessionService, SessionServiceError,
};
pub use uvc::{
    MAX_UVC_CONTROL_KEY_BYTES, MAX_UVC_DEVICE_PATH_BYTES, SysfsUvcDeviceSource, SystemUvcWriter,
    UVC_ERROR_BUSY, UVC_ERROR_FAILED, UVC_ERROR_INVALID_ARGS, UVC_ERROR_LIMITS_EXCEEDED,
    UVC_ERROR_NOT_SUPPORTED, UVC_ERROR_OUT_OF_RANGE, UVC_ERROR_PREFIX, UVC_ERROR_STALLED,
    UVC_WRITE_SUPPORTED_CACHE, UvcControlWriter, UvcDeviceEntry, UvcDeviceSource, UvcRuntime,
    UvcWriteError,
};

/// Version du crate.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
