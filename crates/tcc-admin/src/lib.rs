// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! `tccd-admin` — service administratif à la demande (phase 1).
//!
//! Le binaire (`src/main.rs`) est volontairement mince : toute la logique
//! testable (autorisation, validation, transaction, inactivité) vit dans
//! cette bibliothèque. Le service expose `Admin1` sur
//! `com.tuxedocomputers.tccd.Admin` et écrit uniquement
//! `/var/lib/tuxedo-rust-center` via [`tcc_core::ConfigStore`].
//!
//! Aucun chemin, aucune commande et aucun secret ne traverse l'interface
//! D-Bus. Les opérations de phase 2/3 (TDP, PRIME) ne sont pas implémentées
//! ici ; le changement de mot de passe LUKS est hors périmètre.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod hw;
pub mod runtime;
pub mod service;
pub mod transaction;

pub use error::AdminError;
pub use hw::{
    ACTION_POWER_LIMITS, AdminHwError, AdminHwService, BoundedAdminHwInterface, CtgpBackend,
    CtgpBounds, HW_IDLE_TIMEOUT, HW_READ_BURST, HW_READ_RATE_PER_SECOND, HW_WRITE_BURST,
    HW_WRITE_RATE_PER_SECOND, MAX_HW_MESSAGE_BODY_BYTES, OdmBackend, OdmPlatform, OverrideConsent,
    PpdOwnership, PpdProbe, RealCtgpBackend, RealOdmBackend, RealTdpBackend, SystemOverrideConsent,
    TdpBackend, ZbusPpdProbe,
};
pub use runtime::build_runtime;
pub use service::{
    ACTION_PROFILES, ACTION_SETTINGS, ActiveCall, Activity, AdminService, Authorizer,
    AuthzDecision, ControlReloader, DIALOG_WAIT, IDLE_TIMEOUT, PolkitAuthorizer, WRITE_BURST,
    WRITE_RATE_PER_SECOND, spawn_idle_watch,
};
pub use transaction::{BackupStatus, MAX_PAYLOAD_BYTES, ReloadError, ReloadNotifier, SavedInfo};
