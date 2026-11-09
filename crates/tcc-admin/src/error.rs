// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Erreurs typées du service `Admin1`.
//!
//! Les messages sont **génériques** : aucun chemin, aucune valeur brute,
//! aucun contenu de fichier. Le namespace applicatif permet au client de
//! distinguer un refus polkit d'une erreur de transport
//! (`org.freedesktop.DBus.Error.AccessDenied` n'est jamais utilisé pour un
//! refus polkit, voir AD-2).

use tcc_ipc::AdminErrorCode;

/// Erreur D-Bus du service admin.
#[derive(Debug, Clone, PartialEq, Eq, zbus::DBusError)]
#[zbus(prefix = "org.tuxedocomputers.tccd.Admin1.Error")]
pub enum AdminError {
    /// polkit a refusé, est absent, a expiré, ou l'identité est inconnue.
    NotAuthorized,
    /// Entrée syntaxiquement invalide (nom de champ uniquement).
    InvalidArgs(String),
    /// Charge utile au-delà de la limite de 1 Mio.
    PayloadTooLarge,
    /// Modèle valide syntaxiquement mais rejeté par la validation.
    Validation(String),
    /// Un profil référencé par l'état disparaîtrait.
    InUse(String),
    /// `tccd` n'a pas pu recharger ; le fichier précédent a été restauré.
    ReloadFailed,
    /// Écriture ou rollback impossible. Détail dans le journal uniquement.
    WriteFailed,
    /// Limite de débit atteinte.
    RateLimited,
    /// Une écriture est déjà en cours.
    Busy,
    /// Matériel absent ou opération non supportée (phases ultérieures).
    NotSupported,
}

impl AdminError {
    /// Libellé stable pour l'audit (`result=…`).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NotAuthorized => "not_authorized",
            Self::InvalidArgs(_) => "invalid_args",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Validation(_) => "validation",
            Self::InUse(_) => "in_use",
            Self::ReloadFailed => "reload_failed",
            Self::WriteFailed => "write_failed",
            Self::RateLimited => "rate_limited",
            Self::Busy => "busy",
            Self::NotSupported => "not_supported",
        }
    }

    /// Code de contrat correspondant, pour les tests et la documentation.
    #[must_use]
    pub const fn code(&self) -> AdminErrorCode {
        match self {
            Self::NotAuthorized => AdminErrorCode::NotAuthorized,
            Self::InvalidArgs(_) => AdminErrorCode::InvalidArgs,
            Self::PayloadTooLarge => AdminErrorCode::PayloadTooLarge,
            Self::Validation(_) => AdminErrorCode::Validation,
            Self::InUse(_) => AdminErrorCode::InUse,
            Self::ReloadFailed => AdminErrorCode::ReloadFailed,
            Self::WriteFailed => AdminErrorCode::WriteFailed,
            Self::RateLimited => AdminErrorCode::RateLimited,
            Self::Busy => AdminErrorCode::Busy,
            Self::NotSupported => AdminErrorCode::NotSupported,
        }
    }
}

/// Convertit une erreur de validation `tcc-core` en erreur D-Bus bornée.
///
/// Seul le **nom du champ** est repris : jamais la valeur fautive ni le
/// message détaillé, qui peuvent contenir des données utilisateur.
#[must_use]
pub fn validation_error(prefix: &str, error: &tcc_core::ValidationError) -> AdminError {
    let field = error
        .issues()
        .first()
        .map(|issue| issue.field.as_str())
        .unwrap_or("modèle");
    AdminError::Validation(format!("{prefix}.{field}"))
}

/// Extrait un identifiant de profil sûr d'une erreur de référence.
///
/// Le message de `tcc-core` est `profil inconnu: <id>`. L'identifiant n'est
/// repris que s'il respecte la grammaire des IDs ; sinon un libellé générique
/// évite de relayer une donnée arbitraire.
#[must_use]
pub fn in_use_profile(error: &tcc_core::ValidationError) -> AdminError {
    let candidate = error
        .issues()
        .first()
        .and_then(|issue| issue.message.strip_prefix("profil inconnu: "));
    match candidate {
        Some(id)
            if !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                }) =>
        {
            AdminError::InUse(id.to_owned())
        }
        _ => AdminError::InUse("profil référencé".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use tcc_core::ValidationIssue;

    use super::{AdminError, in_use_profile, validation_error};
    use tcc_ipc::AdminErrorCode;

    #[test]
    fn labels_and_codes_match_contract() {
        for error in [
            AdminError::NotAuthorized,
            AdminError::PayloadTooLarge,
            AdminError::ReloadFailed,
            AdminError::WriteFailed,
            AdminError::RateLimited,
            AdminError::Busy,
            AdminError::NotSupported,
        ] {
            assert_eq!(
                error.code().as_str(),
                match error {
                    AdminError::NotAuthorized => "NotAuthorized",
                    AdminError::PayloadTooLarge => "PayloadTooLarge",
                    AdminError::ReloadFailed => "ReloadFailed",
                    AdminError::WriteFailed => "WriteFailed",
                    AdminError::RateLimited => "RateLimited",
                    AdminError::Busy => "Busy",
                    AdminError::NotSupported => "NotSupported",
                    _ => unreachable!(),
                }
            );
        }
        assert_eq!(AdminError::Busy.code(), AdminErrorCode::Busy);
    }

    #[test]
    fn validation_keeps_only_the_field() {
        let error = tcc_core::ValidationError::from_vec(vec![ValidationIssue::new(
            "display.refreshRate",
            "valeur secrète 123",
        )]);
        let mapped = validation_error("profil", &error);
        assert_eq!(
            mapped,
            AdminError::Validation("profil.display.refreshRate".to_owned())
        );
        assert!(!format!("{mapped:?}").contains("secrète"));
    }

    #[test]
    fn in_use_keeps_safe_ids_only() {
        let safe = tcc_core::ValidationError::from_vec(vec![ValidationIssue::new(
            "stateMap.power_ac",
            "profil inconnu: custom-abc_123",
        )]);
        assert_eq!(
            in_use_profile(&safe),
            AdminError::InUse("custom-abc_123".to_owned())
        );

        let unsafe_id = tcc_core::ValidationError::from_vec(vec![ValidationIssue::new(
            "stateMap.power_ac",
            "profil inconnu: /etc/shadow",
        )]);
        assert_eq!(
            in_use_profile(&unsafe_id),
            AdminError::InUse("profil référencé".to_owned())
        );
    }
}
