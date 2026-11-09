// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! Contrats IPC/D-Bus partagés entre les binaires TuxedoRustCenter.
//!
//! Ce crate est la **seule** frontière de types entre `tcc-daemon`, `tcc-user`
//! et `tcc-gui`. Il ne dépend ni de `tcc-hw`, ni de Qt, ni d'un runtime
//! asynchrone : uniquement de `serde`, `serde_json`, `thiserror` et de
//! `tcc-core` pour réutiliser les enums de domaine (`PowerSource`,
//! `PpdProfile`, `ProfileId`, `ResolutionSource`). Cela évite qu'une copie
//! locale des enums diverge de la logique de bascule.
//!
//! # Version de contrat
//!
//! [`IPC_CONTRACT_VERSION`] est un entier strictement comparé. Toute
//! modification incompatible — suppression/renommage de champ, changement de
//! sémantique, passage d'un champ optionnel à obligatoire — impose un
//! incrément. Un client qui reçoit une autre version doit refuser le message
//! via [`ensure_contract_compatible`] plutôt que d'ignorer le problème.
//!
//! # Compatibilité `zbus`/`zvariant`
//!
//! Les types sont sérialisables `serde` et n'utilisent que des chaînes, des
//! entiers, des booléens, des options et des listes : ils se projettent
//! naturellement en signatures D-Bus (`s`, `u`, `b`, `a{sv}`, `a(su)`). Les
//! valeurs flottantes (`refresh_rate_hz`) devront être validées comme finies
//! avant tout encodage `zvariant` (`f64`). Les enums sont sérialisées en
//! chaînes stables pour rester lisibles dans `busctl` et dans les tests ; la
//! couche D-Bus pourra les mapper en `u32` si nécessaire sans changer les
//! types Rust.
//!
//! Aucun compteur, timestamp ou `serde_json::Value` ne figure dans les
//! signatures exposées : le daemon doit rester maître du typage de son API.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod admin;
pub mod display;
pub mod error;
pub mod session;
pub mod state;

pub use admin::{
    AdminAction, AdminErrorCode, AdminHwErrorCode, AdminReloadReport, AdminVersionInfo,
    MAX_ADMIN_RELOAD_WARNING_CHARS, MAX_ADMIN_RELOAD_WARNINGS, MAX_ADMIN_VERSION_CHARS,
    MAX_CTGP_STATIC_OFFSET_WATTS, MAX_HW_STRING_CHARS, MAX_TDP_VALUES,
};
pub use display::{
    DisplayBackend, DisplayModeInfo, DisplayModeRequest, DisplayResolution, is_internal_output_name,
};
pub use error::{IpcError, IpcResult};
pub use session::{
    SessionAgentAction, SessionAgentOutcome, SessionAgentRequest, SessionAgentResponse,
};
pub use state::{ActiveProfile, StateSnapshot};

/// Nom D-Bus historique du daemon.
pub const DBUS_SERVICE: &str = "com.tuxedocomputers.tccd";
/// Objet D-Bus historique.
pub const DBUS_PATH: &str = "/com/tuxedocomputers/tccd";
/// Interface D-Bus historique (JSON de compatibilité).
pub const DBUS_INTERFACE: &str = "com.tuxedocomputers.tccd";
/// Interface typée versionnée du daemon.
pub const DBUS_MANAGER_INTERFACE: &str = "com.tuxedocomputers.tccd.Manager1";
/// Service D-Bus de l'agent de session, sur le **bus de session**.
pub const DBUS_SESSION_AGENT_SERVICE: &str = "com.tuxedocomputers.tccd.User";
/// Objet D-Bus de l'agent de session.
pub const DBUS_SESSION_AGENT_PATH: &str = "/com/tuxedocomputers/tccd/User";
/// Interface D-Bus de l'agent de session (lecture seule).
pub const DBUS_SESSION_AGENT_INTERFACE: &str = "com.tuxedocomputers.tccd.User1";
/// Version du contrat de l'interface de session.
pub const SESSION_CONTRACT_VERSION: u32 = 1;
/// Version du contrat IPC.
///
/// À incrémenter pour tout changement incompatible. La comparaison est
/// stricte, sans négociation mineure : un champ ajouté avec valeur par défaut
/// reste compatible si tous les producteurs le renseignent correctement, mais
/// un champ requis ou une sémantique modifiée ne l'est pas.
pub const IPC_CONTRACT_VERSION: u32 = 1;

/// Service D-Bus de l'administration à la demande (D1/AD-1).
///
/// Distinct de [`DBUS_SERVICE`] : un bug de parsing dans `tccd`, qui traite
/// des messages en continu, ne doit pas devenir une primitive root
/// d'administration.
pub const DBUS_ADMIN_SERVICE: &str = "com.tuxedocomputers.tccd.Admin";
/// Objet D-Bus de l'administration, distinct de [`DBUS_PATH`].
pub const DBUS_ADMIN_PATH: &str = "/com/tuxedocomputers/tccd/Admin";
/// Interface admin versionnée (AD-2).
pub const DBUS_ADMIN_INTERFACE: &str = "com.tuxedocomputers.tccd.Admin1";
/// Interface de contrôle interne de `tccd`, réservée à root (AD-6).
///
/// `tccd` re-vérifie l'UID de l'appelant en plus de la politique D-Bus.
pub const DBUS_CONTROL_INTERFACE: &str = "com.tuxedocomputers.tccd.Control1";
/// Namespace des erreurs applicatives `Admin1` (AD-2).
pub const ADMIN_ERROR_NAMESPACE: &str = "org.tuxedocomputers.tccd.Admin1.Error";
/// Version du contrat admin, comparée strictement.
///
/// Indépendante de [`IPC_CONTRACT_VERSION`] : l'admin peut évoluer sans
/// invalider les clients de l'API IPC historique, et inversement.
pub const ADMIN_CONTRACT_VERSION: u32 = 1;

/// Service D-Bus des limites matérielles phase 2 (AD-12 B2).
pub const DBUS_ADMIN_HW_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Hw";
/// Objet D-Bus du service `Admin.Hw`.
pub const DBUS_ADMIN_HW_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Hw";
/// Interface versionnée `AdminHw1`.
pub const DBUS_ADMIN_HW_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminHw1";
/// Namespace des erreurs applicatives `AdminHw1`.
pub const ADMIN_HW_ERROR_NAMESPACE: &str = "org.tuxedocomputers.tccd.AdminHw1.Error";
/// Version du contrat `AdminHw1`, comparée strictement.
pub const ADMIN_HW_CONTRACT_VERSION: u32 = 1;

/// Service D-Bus de bascule PRIME phase 2 (AD-12 B2, non implémenté ici).
pub const DBUS_ADMIN_PRIME_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Prime";
/// Objet D-Bus du service `Admin.Prime`.
pub const DBUS_ADMIN_PRIME_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Prime";
/// Interface versionnée `AdminPrime1`.
pub const DBUS_ADMIN_PRIME_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminPrime1";
/// Namespace des erreurs applicatives `AdminPrime1`.
pub const ADMIN_PRIME_ERROR_NAMESPACE: &str = "org.tuxedocomputers.tccd.AdminPrime1.Error";
/// Version du contrat `AdminPrime1`, comparée strictement.
pub const ADMIN_PRIME_CONTRACT_VERSION: u32 = 1;

/// Vérifie la compatibilité d'une version du contrat `AdminHw1`.
///
/// # Errors
///
/// Renvoie [`IpcError::AdminContractMismatch`] si `remote` diffère de
/// [`ADMIN_HW_CONTRACT_VERSION`].
pub fn ensure_admin_hw_contract_compatible(remote: u32) -> IpcResult<()> {
    if remote == ADMIN_HW_CONTRACT_VERSION {
        Ok(())
    } else {
        Err(IpcError::AdminContractMismatch {
            local: ADMIN_HW_CONTRACT_VERSION,
            remote,
        })
    }
}

/// Vérifie la compatibilité d'une version du contrat `Admin1`.
///
/// # Errors
///
/// Renvoie [`IpcError::AdminContractMismatch`] si `remote` diffère de
/// [`ADMIN_CONTRACT_VERSION`].
pub fn ensure_admin_contract_compatible(remote: u32) -> IpcResult<()> {
    if remote == ADMIN_CONTRACT_VERSION {
        Ok(())
    } else {
        Err(IpcError::AdminContractMismatch {
            local: ADMIN_CONTRACT_VERSION,
            remote,
        })
    }
}

/// Vérifie la compatibilité d'une version reçue.
///
/// # Errors
///
/// Renvoie [`IpcError::ContractMismatch`] si `remote` diffère de
/// [`IPC_CONTRACT_VERSION`].
pub fn ensure_contract_compatible(remote: u32) -> IpcResult<()> {
    if remote == IPC_CONTRACT_VERSION {
        Ok(())
    } else {
        Err(IpcError::ContractMismatch {
            local: IPC_CONTRACT_VERSION,
            remote,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADMIN_CONTRACT_VERSION, DBUS_ADMIN_INTERFACE, DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE,
        DBUS_CONTROL_INTERFACE, IPC_CONTRACT_VERSION, ensure_admin_contract_compatible,
        ensure_contract_compatible,
    };
    use crate::IpcError;

    #[test]
    fn session_agent_contract_is_stable() {
        use super::{
            DBUS_SESSION_AGENT_INTERFACE, DBUS_SESSION_AGENT_PATH, DBUS_SESSION_AGENT_SERVICE,
            SESSION_CONTRACT_VERSION,
        };
        assert_eq!(DBUS_SESSION_AGENT_SERVICE, "com.tuxedocomputers.tccd.User");
        assert_eq!(DBUS_SESSION_AGENT_PATH, "/com/tuxedocomputers/tccd/User");
        assert_eq!(
            DBUS_SESSION_AGENT_INTERFACE,
            "com.tuxedocomputers.tccd.User1"
        );
        assert_eq!(SESSION_CONTRACT_VERSION, 1);
        assert_ne!(DBUS_SESSION_AGENT_SERVICE, super::DBUS_SERVICE);
        assert_ne!(DBUS_SESSION_AGENT_PATH, super::DBUS_PATH);
    }

    #[test]
    fn current_version_is_compatible() {
        ensure_contract_compatible(IPC_CONTRACT_VERSION).unwrap();
    }

    #[test]
    fn other_versions_are_rejected() {
        let error = ensure_contract_compatible(IPC_CONTRACT_VERSION + 1).unwrap_err();
        match error {
            IpcError::ContractMismatch { local, remote } => {
                assert_eq!(local, IPC_CONTRACT_VERSION);
                assert_eq!(remote, IPC_CONTRACT_VERSION + 1);
            }
            other => panic!("erreur inattendue: {other:?}"),
        }
    }

    #[test]
    fn admin_contract_version_is_strict() {
        ensure_admin_contract_compatible(ADMIN_CONTRACT_VERSION).unwrap();
        let error = ensure_admin_contract_compatible(ADMIN_CONTRACT_VERSION + 1).unwrap_err();
        assert!(matches!(
            error,
            IpcError::AdminContractMismatch { local, remote }
                if local == ADMIN_CONTRACT_VERSION && remote == ADMIN_CONTRACT_VERSION + 1
        ));
    }

    #[test]
    fn admin_names_are_distinct_from_daemon_names() {
        assert_ne!(DBUS_ADMIN_SERVICE, super::DBUS_SERVICE);
        assert_ne!(DBUS_ADMIN_PATH, super::DBUS_PATH);
        assert_ne!(DBUS_ADMIN_INTERFACE, super::DBUS_MANAGER_INTERFACE);
        assert!(DBUS_ADMIN_SERVICE.ends_with(".Admin"));
        assert!(DBUS_ADMIN_PATH.ends_with("/Admin"));
        assert!(DBUS_ADMIN_INTERFACE.ends_with(".Admin1"));
        assert!(DBUS_CONTROL_INTERFACE.ends_with(".Control1"));
    }
}
