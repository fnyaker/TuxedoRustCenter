// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Erreurs du contrat IPC.

use thiserror::Error;

/// Résultat des opérations de validation IPC.
pub type IpcResult<T> = Result<T, IpcError>;

/// Erreur de validation ou de compatibilité d'un message IPC.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IpcError {
    /// Le message vient d'une version de contrat différente.
    #[error("version de contrat IPC incompatible: local {local}, reçu {remote}")]
    ContractMismatch {
        /// Version compilée dans ce binaire.
        local: u32,
        /// Version annoncée par le message.
        remote: u32,
    },

    /// Un champ du message ne respecte pas son contrat.
    #[error("champ « {field} » invalide: {message}")]
    InvalidField {
        /// Nom du champ, par exemple `displayMode.refreshRateHz`.
        field: String,
        /// Description du problème.
        message: String,
    },

    /// Une demande d'affichage cible une sortie qui n'est pas l'écran interne.
    #[error("sortie d'affichage non interne refusée: {0}")]
    NonInternalOutput(String),

    /// Une charge utile dépasse la taille maximale autorisée.
    #[error("charge utile de {size} octets, limite {max}")]
    PayloadTooLarge {
        /// Taille reçue en octets.
        size: usize,
        /// Taille maximale acceptée en octets.
        max: usize,
    },

    /// Le message vient d'une version du contrat `Admin1` différente.
    #[error("version de contrat admin incompatible: local {local}, reçu {remote}")]
    AdminContractMismatch {
        /// Version compilée dans ce binaire.
        local: u32,
        /// Version annoncée par le message.
        remote: u32,
    },
}

impl IpcError {
    /// Construit une erreur de champ invalide.
    #[must_use]
    pub fn field(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidField {
            field: field.into(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IpcError;

    #[test]
    fn errors_are_displayable() {
        let error = IpcError::field("revision", "doit être positif");
        assert!(error.to_string().contains("revision"));
        let error = IpcError::NonInternalOutput("HDMI-1".into());
        assert!(error.to_string().contains("HDMI-1"));

        let error = IpcError::PayloadTooLarge { size: 42, max: 16 };
        assert!(error.to_string().contains("42"));
        assert!(error.to_string().contains("16"));

        let error = IpcError::AdminContractMismatch {
            local: 1,
            remote: 2,
        };
        assert!(error.to_string().contains("admin"));
        assert!(error.to_string().contains('2'));
    }
}
