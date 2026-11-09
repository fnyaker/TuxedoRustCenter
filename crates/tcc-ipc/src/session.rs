// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Requêtes et réponses entre le daemon et l'agent de session.
//!
//! L'agent `tccd-user` ne fait qu'appliquer la partie affichage. Il ne reçoit
//! jamais de chemin, de commande ni de secret : uniquement une action, un
//! identifiant de requête et, pour l'application, une [`DisplayModeRequest`]
//! déjà validée.

use serde::{Deserialize, Serialize};
use tcc_core::ProfileId;

use crate::IPC_CONTRACT_VERSION;
use crate::display::{DisplayModeInfo, DisplayModeRequest};
use crate::error::{IpcError, IpcResult};

/// Longueur maximale d'un message de réponse.
pub const MAX_SESSION_MESSAGE_LEN: usize = 2048;

/// Action demandée à l'agent de session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionAgentAction {
    /// Lire l'état d'affichage et le renvoyer.
    GetDisplayModeInfo,
    /// Appliquer un mode sur l'écran interne.
    ApplyDisplayMode,
    /// Revenir au mode initial mémorisé (au mieux).
    ResetDisplayMode,
    /// Arrêter proprement l'agent (utilisé par systemd/user).
    Shutdown,
}

/// Requête du daemon vers l'agent de session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAgentRequest {
    /// Version du contrat IPC.
    pub contract_version: u32,
    /// Identifiant de corrélation, choisi par le daemon.
    pub request_id: u64,
    /// Action demandée.
    pub action: SessionAgentAction,
    /// Demande d'affichage, uniquement pour
    /// [`SessionAgentAction::ApplyDisplayMode`].
    pub display_mode: Option<DisplayModeRequest>,
}

impl SessionAgentRequest {
    /// Crée une requête avec la version de contrat courante.
    #[must_use]
    pub fn new(request_id: u64, action: SessionAgentAction) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            request_id,
            action,
            display_mode: None,
        }
    }

    /// Demande l'état d'affichage.
    #[must_use]
    pub fn get_display_mode_info(request_id: u64) -> Self {
        Self::new(request_id, SessionAgentAction::GetDisplayModeInfo)
    }

    /// Demande l'application d'un mode.
    #[must_use]
    pub fn apply_display_mode(request_id: u64, request: DisplayModeRequest) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            request_id,
            action: SessionAgentAction::ApplyDisplayMode,
            display_mode: Some(request),
        }
    }

    /// Demande la restauration du mode initial.
    #[must_use]
    pub fn reset_display_mode(request_id: u64) -> Self {
        Self::new(request_id, SessionAgentAction::ResetDisplayMode)
    }

    /// Profil à l'origine de la demande, s'il y en a un.
    #[must_use]
    pub fn profile_id(&self) -> Option<&ProfileId> {
        self.display_mode.as_ref().map(|mode| &mode.profile_id)
    }

    /// Valide la requête.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si la version diffère, si une demande d'application
    /// n'a pas de mode valide, ou si une action sans application transporte
    /// malgré tout un mode.
    pub fn validate(&self) -> IpcResult<()> {
        crate::ensure_contract_compatible(self.contract_version)?;
        match self.action {
            SessionAgentAction::ApplyDisplayMode => {
                let Some(display_mode) = &self.display_mode else {
                    return Err(IpcError::field(
                        "displayMode",
                        "requis pour ApplyDisplayMode",
                    ));
                };
                display_mode.validate()?;
            }
            SessionAgentAction::GetDisplayModeInfo
            | SessionAgentAction::ResetDisplayMode
            | SessionAgentAction::Shutdown => {
                if self.display_mode.is_some() {
                    return Err(IpcError::field("displayMode", "interdit pour cette action"));
                }
            }
        }
        Ok(())
    }
}

/// Résultat d'une requête de session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionAgentOutcome {
    /// Le mode a été appliqué.
    Applied,
    /// Le mode demandé était déjà actif.
    AlreadyApplied,
    /// Le backend ne sait pas appliquer cette demande.
    Unsupported,
    /// L'application a échoué ; `message` explique pourquoi.
    Failed,
    /// Réponse informative (par exemple `GetDisplayModeInfo`).
    Info,
}

/// Réponse de l'agent de session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAgentResponse {
    /// Version du contrat IPC.
    pub contract_version: u32,
    /// Identifiant de la requête d'origine.
    pub request_id: u64,
    /// Résultat.
    pub outcome: SessionAgentOutcome,
    /// État d'affichage, si l'agent peut le fournir.
    pub display: Option<DisplayModeInfo>,
    /// Message lisible (erreur ou information).
    pub message: Option<String>,
}

impl SessionAgentResponse {
    /// Crée une réponse avec la version de contrat courante.
    #[must_use]
    pub fn new(
        request_id: u64,
        outcome: SessionAgentOutcome,
        display: Option<DisplayModeInfo>,
        message: Option<String>,
    ) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            request_id,
            outcome,
            display,
            message,
        }
    }

    /// Réponse informative contenant l'état d'affichage.
    #[must_use]
    pub fn info(request_id: u64, display: DisplayModeInfo) -> Self {
        Self::new(request_id, SessionAgentOutcome::Info, Some(display), None)
    }

    /// Réponse d'échec.
    #[must_use]
    pub fn failure(request_id: u64, message: impl Into<String>) -> Self {
        Self::new(
            request_id,
            SessionAgentOutcome::Failed,
            None,
            Some(message.into()),
        )
    }

    /// Vrai si l'opération a réussi ou était déjà effective.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(
            self.outcome,
            SessionAgentOutcome::Applied
                | SessionAgentOutcome::AlreadyApplied
                | SessionAgentOutcome::Info
        )
    }

    /// Valide la réponse avant émission.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si la version diffère, si un échec n'a pas de
    /// message, si une info n'a pas d'état d'affichage, ou si un message est
    /// trop long.
    pub fn validate(&self) -> IpcResult<()> {
        crate::ensure_contract_compatible(self.contract_version)?;
        if let Some(display) = &self.display {
            display.validate()?;
        }
        match self.outcome {
            SessionAgentOutcome::Failed if self.message.is_none() => {
                return Err(IpcError::field("message", "requis lorsque outcome=Failed"));
            }
            SessionAgentOutcome::Info if self.display.is_none() => {
                return Err(IpcError::field("display", "requis lorsque outcome=Info"));
            }
            _ => {}
        }
        if let Some(message) = &self.message
            && message.chars().count() > MAX_SESSION_MESSAGE_LEN
        {
            return Err(IpcError::field(
                "message",
                format!("ne doit pas dépasser {MAX_SESSION_MESSAGE_LEN} caractères"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionAgentAction, SessionAgentOutcome, SessionAgentRequest, SessionAgentResponse,
    };
    use crate::IpcError;
    use crate::display::{DisplayModeInfo, DisplayModeRequest};

    fn request() -> DisplayModeRequest {
        DisplayModeRequest::new("office")
            .with_refresh_rate(165.0)
            .with_internal_output("eDP-1")
    }

    #[test]
    fn apply_request_roundtrips() {
        let request = SessionAgentRequest::apply_display_mode(7, request());
        request.validate().unwrap();
        assert_eq!(request.profile_id().unwrap().as_str(), "office");

        let json = serde_json::to_string(&request).unwrap();
        let parsed: SessionAgentRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
        assert_eq!(parsed.action, SessionAgentAction::ApplyDisplayMode);
    }

    #[test]
    fn info_request_has_no_display_mode() {
        let request = SessionAgentRequest::get_display_mode_info(1);
        request.validate().unwrap();
        assert!(request.display_mode.is_none());
    }

    #[test]
    fn apply_without_mode_is_rejected() {
        let request = SessionAgentRequest::new(1, SessionAgentAction::ApplyDisplayMode);
        assert!(matches!(
            request.validate(),
            Err(IpcError::InvalidField { .. })
        ));
    }

    #[test]
    fn get_with_mode_is_rejected() {
        let mut agent_request = SessionAgentRequest::get_display_mode_info(1);
        agent_request.display_mode = Some(request());
        assert!(agent_request.validate().is_err());
    }

    #[test]
    fn request_with_invalid_mode_is_rejected() {
        let request = SessionAgentRequest::apply_display_mode(
            1,
            DisplayModeRequest::new("office")
                .with_refresh_rate(60.0)
                .with_internal_output("HDMI-1"),
        );
        assert!(matches!(
            request.validate(),
            Err(IpcError::NonInternalOutput(_))
        ));
    }

    #[test]
    fn response_roundtrips() {
        let response = SessionAgentResponse::info(
            3,
            DisplayModeInfo::ready(crate::DisplayBackend::X11, "eDP-1"),
        );
        response.validate().unwrap();
        let json = serde_json::to_string(&response).unwrap();
        let parsed: SessionAgentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, response);
        assert!(parsed.is_success());
    }

    #[test]
    fn failure_response_requires_message() {
        let mut response = SessionAgentResponse::failure(1, "backend absent");
        response.validate().unwrap();

        response.message = None;
        assert!(response.validate().is_err());
    }

    #[test]
    fn info_response_requires_display_info() {
        let response = SessionAgentResponse::new(1, SessionAgentOutcome::Info, None, None);
        assert!(response.validate().is_err());
    }

    #[test]
    fn incompatible_version_is_rejected() {
        let mut request = SessionAgentRequest::get_display_mode_info(1);
        request.contract_version = 99;
        assert!(matches!(
            request.validate(),
            Err(IpcError::ContractMismatch { .. })
        ));
    }
}
