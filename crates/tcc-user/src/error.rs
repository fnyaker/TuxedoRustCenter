// SPDX-License-Identifier: GPL-3.0-or-later
//! Erreurs typées de l'agent de session.

use tcc_hw::HwError;

/// Erreur de `tccd-user`.
#[derive(Debug, thiserror::Error)]
pub enum UserError {
    /// L'agent doit tourner dans la session de l'utilisateur, pas en root.
    #[error("tccd-user doit s'exécuter en utilisateur, pas en root (UID effectif 0)")]
    RunningAsRoot,

    /// L'UID effectif n'a pas pu être déterminé de façon fiable.
    #[error("impossible de déterminer l'UID effectif : {0}")]
    EffectiveUid(String),

    /// La session ne permet pas d'appliquer un mode d'affichage.
    #[error("session non supportée : {0}")]
    UnsupportedSession(String),

    /// Le backend n'a pas pu être construit ou initialisé.
    #[error("backend {backend} indisponible : {reason}")]
    BackendUnavailable {
        backend: &'static str,
        reason: String,
    },

    /// Le socket IPC de session est absent, d'un autre propriétaire ou
    /// dangereux (lien symbolique, permissions).
    #[error("socket de session invalide : {0}")]
    InvalidSocket(String),

    /// Erreur de protocole IPC (Hyprland, Sway, ...).
    #[error("erreur IPC {protocol} : {message}")]
    Ipc {
        protocol: &'static str,
        message: String,
    },

    /// Argument de ligne de commande invalide.
    #[error("argument invalide : {0}")]
    InvalidArgument(String),

    /// Erreur de communication avec le daemon sur le bus système.
    #[error("D-Bus : {0}")]
    Dbus(String),

    /// Le daemon n'a pas répondu dans le délai imparti.
    #[error("délai dépassé : {0}")]
    Timeout(&'static str),

    /// Une réponse du daemon ne respecte pas le contrat attendu. Le JSON et
    /// les identifiants sont traités comme non fiables.
    #[error("réponse daemon invalide : {0}")]
    InvalidRemoteData(String),

    /// Une tâche bloquante interne à l'agent a échoué ou paniqué.
    #[error("tâche interne de l'agent : {0}")]
    AgentTask(String),

    /// Erreur de la couche matérielle/affichage.
    #[error(transparent)]
    Hardware(#[from] HwError),

    /// Erreur JSON (réponse compositeur ou rapport CLI).
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Erreur d'entrée/sortie.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl UserError {
    /// Indique si l'erreur signifie « backend non supporté dans cette
    /// session », cas normal pour GNOME/KDE/Wayland générique.
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            Self::UnsupportedSession(_)
                | Self::BackendUnavailable { .. }
                | Self::Hardware(HwError::NotSupported(_))
                | Self::Hardware(HwError::NoDisplayBackend)
        )
    }
}

/// Résultat de l'agent de session.
pub type Result<T> = std::result::Result<T, UserError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_supported_is_unsupported() {
        let err = UserError::Hardware(HwError::NotSupported("gnome"));
        assert!(err.is_unsupported());
    }

    #[test]
    fn invalid_argument_is_not_unsupported() {
        assert!(!UserError::InvalidArgument("x".to_owned()).is_unsupported());
    }
}
