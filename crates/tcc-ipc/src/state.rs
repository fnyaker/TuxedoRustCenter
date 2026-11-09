// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Snapshot d'état partagé entre le daemon, la GUI et l'agent de session.

use serde::{Deserialize, Serialize};
use tcc_core::{
    MAX_PROFILE_NAME_LEN, PowerSource, PpdProfile, Profile, ProfileId, ResolutionSource,
    validate_profile_description, validate_profile_id, validate_profile_name,
};

use crate::IPC_CONTRACT_VERSION;
use crate::error::{IpcError, IpcResult};

/// Profil actif exposé aux clients.
///
/// Copie volontairement légère : l'UI récupère la liste complète des profils
/// séparément. `source` explique pourquoi ce profil est actif (matrice,
/// `stateMap`, override manuel ou défaut), ce qui évite à la GUI de deviner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProfile {
    /// Identifiant stable.
    pub id: ProfileId,
    /// Nom affiché.
    pub name: String,
    /// Description courte.
    pub description: String,
    /// Vrai si le profil est un preset TCC.
    pub is_preset: bool,
    /// Origine de la sélection.
    pub source: ResolutionSource,
}

impl ActiveProfile {
    /// Construit un profil actif à partir d'un profil complet.
    #[must_use]
    pub fn from_profile(profile: &Profile, source: ResolutionSource) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            description: profile.description.clone(),
            is_preset: profile.is_preset(),
            source,
        }
    }

    /// Valide les champs exposés.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si l'identifiant, le nom ou la description ne
    /// respectent pas les règles de `tcc-core`.
    pub fn validate(&self) -> IpcResult<()> {
        validate_profile_id(&self.id)
            .map_err(|error| IpcError::field("activeProfile.id", error.to_string()))?;
        validate_profile_name(&self.name)
            .map_err(|error| IpcError::field("activeProfile.name", error.to_string()))?;
        if self.name.chars().count() > MAX_PROFILE_NAME_LEN {
            return Err(IpcError::field(
                "activeProfile.name",
                format!("ne doit pas dépasser {MAX_PROFILE_NAME_LEN} caractères"),
            ));
        }
        validate_profile_description(&self.description)
            .map_err(|error| IpcError::field("activeProfile.description", error.to_string()))?;
        Ok(())
    }
}

/// Snapshot d'état exposé par le daemon.
///
/// `revision` est un compteur monotone incrémenté à chaque changement
/// effectif. Les clients peuvent ignorer un snapshot dont la révision n'a pas
/// augmenté, et détecter une perte de messages D-Bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    /// Version du contrat IPC.
    pub contract_version: u32,
    /// Compteur monotone de révision.
    pub revision: u64,
    /// Profil PPD courant, absent si PPD n'est pas installé.
    pub ppd_profile: Option<PpdProfile>,
    /// Source d'alimentation courante.
    pub power_source: PowerSource,
    /// Vrai si un override manuel temporaire est actif.
    pub manual_override: bool,
    /// Profil actif, `None` avant la première application.
    pub active_profile: Option<ActiveProfile>,
}

impl StateSnapshot {
    /// Crée un snapshot avec la version de contrat courante.
    #[must_use]
    pub fn new(
        revision: u64,
        ppd_profile: Option<PpdProfile>,
        power_source: PowerSource,
        active_profile: Option<ActiveProfile>,
    ) -> Self {
        Self {
            contract_version: IPC_CONTRACT_VERSION,
            revision,
            ppd_profile,
            power_source,
            manual_override: active_profile
                .as_ref()
                .is_some_and(|profile| profile.source == ResolutionSource::ManualOverride),
            active_profile,
        }
    }

    /// Valide le snapshot.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si la version de contrat diffère, si le profil
    /// actif est invalide ou si `manualOverride` ne correspond pas à la source
    /// du profil actif.
    pub fn validate(&self) -> IpcResult<()> {
        crate::ensure_contract_compatible(self.contract_version)?;
        if let Some(profile) = &self.active_profile {
            profile.validate()?;
        }
        let expected_manual = self
            .active_profile
            .as_ref()
            .is_some_and(|profile| profile.source == ResolutionSource::ManualOverride);
        if self.manual_override != expected_manual {
            return Err(IpcError::field(
                "manualOverride",
                "doit correspondre à la source ManualOverride du profil actif",
            ));
        }
        Ok(())
    }

    /// Vrai si le snapshot annonce un override manuel cohérent avec le profil.
    #[must_use]
    pub fn is_manual_override(&self) -> bool {
        self.manual_override
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveProfile, StateSnapshot};
    use crate::{IPC_CONTRACT_VERSION, IpcError};
    use tcc_core::{PowerSource, PpdProfile, Profile, ResolutionSource};

    fn active() -> ActiveProfile {
        ActiveProfile::from_profile(
            &Profile::new("office", "Office"),
            ResolutionSource::StateMatrix,
        )
    }

    #[test]
    fn active_profile_from_core_profile() {
        let active = active();
        assert_eq!(active.id.as_str(), "office");
        assert_eq!(active.name, "Office");
        assert_eq!(active.source, ResolutionSource::StateMatrix);
        active.validate().unwrap();
    }

    #[test]
    fn snapshot_roundtrips() {
        let snapshot = StateSnapshot::new(
            42,
            Some(PpdProfile::Balanced),
            PowerSource::Battery,
            Some(active()),
        );
        snapshot.validate().unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: StateSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snapshot);
        assert!(!parsed.is_manual_override());
    }

    #[test]
    fn snapshot_preserves_unknown_ppd_value() {
        let snapshot = JsonModel {
            ppd: Some(PpdProfile::Unknown("oem-mode".into())),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("oem-mode"));
    }

    #[derive(serde::Serialize)]
    struct JsonModel {
        ppd: Option<PpdProfile>,
    }

    #[test]
    fn manual_override_is_flagged() {
        let active = ActiveProfile::from_profile(
            &Profile::new("manual", "Manual"),
            ResolutionSource::ManualOverride,
        );
        let snapshot = StateSnapshot::new(7, None, PowerSource::Ac, Some(active));
        assert!(snapshot.is_manual_override());
    }

    #[test]
    fn incompatible_contract_is_rejected() {
        let mut snapshot = StateSnapshot::new(1, None, PowerSource::Unknown, None);
        snapshot.contract_version = IPC_CONTRACT_VERSION + 1;
        assert!(matches!(
            snapshot.validate(),
            Err(IpcError::ContractMismatch { .. })
        ));
    }

    #[test]
    fn invalid_active_profile_is_rejected() {
        let mut active = active();
        active.name.clear();
        let error = active.validate().unwrap_err();
        assert!(matches!(error, IpcError::InvalidField { .. }));
    }

    #[test]
    fn manual_flag_must_match_source() {
        let mut snapshot = StateSnapshot::new(1, None, PowerSource::Ac, Some(active()));
        snapshot.manual_override = true;
        assert!(matches!(
            snapshot.validate(),
            Err(IpcError::InvalidField { .. })
        ));

        snapshot.manual_override = false;
        snapshot.validate().unwrap();
    }
}
