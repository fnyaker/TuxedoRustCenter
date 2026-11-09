// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Le projet est distribué sous GPL-3.0-or-later.

//! Contrat `Admin1` partagé entre `tccd-admin`, `tccd` et la GUI.
//!
//! Ce module ne contient **aucun** proxy D-Bus, aucun appel polkit et aucune
//! logique d'autorisation. Il fixe uniquement les types sérialisables et les
//! codes du contrat décrit par `docs/security/ADMIN_DESIGN.md` (AD-2) :
//!
//! - [`AdminReloadReport`] : réponse de `tccd.Control1.ReloadConfig` ;
//! - [`AdminVersionInfo`] : identité de version exposée par l'admin ;
//! - [`AdminErrorCode`] : namespace d'erreurs `Admin1.Error.*` ;
//! - [`AdminAction`] : libellés stables pour l'audit.
//!
//! Les proxies `zbus`, l'implémentation du service et l'appel
//! `CheckAuthorization` appartiennent à `linux-platform` (`tcc-admin` et
//! `tcc-authz`). `tcc-ipc` reste sans dépendance `zbus`, conformément à sa
//! charte.
//!
//! # Règles de sérialisation
//!
//! - les rapports utilisent `camelCase`, comme le reste de `tcc-ipc` ;
//! - toute chaîne reçue du bus est **bornée** avant usage
//!   ([`AdminReloadReport::validate`], [`AdminVersionInfo::validate`]) ;
//! - aucune structure ne contient de `serde_json::Value` libre, de `a{sv}`
//!   ouvert ni de chemin de fichier.
//!
//! Il n'existe volontairement pas d'`AdminRequest` générique : chaque méthode
//! `Admin1` a une signature D-Bus concrète (`s`, `ai`, `i`, `hh`, …). Un enum
//! de requête serait un type de dispatch interne, pas un type de transport,
//! et exposerait un point d'extension non versionné.

use serde::{Deserialize, Serialize};

use crate::ADMIN_CONTRACT_VERSION;
use crate::error::{IpcError, IpcResult};

/// Nombre maximal d'avertissements dans [`AdminReloadReport`].
pub const MAX_ADMIN_RELOAD_WARNINGS: usize = 64;
/// Longueur maximale (en caractères) d'un avertissement de rechargement.
pub const MAX_ADMIN_RELOAD_WARNING_CHARS: usize = 512;
/// Longueur maximale d'une chaîne de version d'implémentation.
pub const MAX_ADMIN_VERSION_CHARS: usize = 64;

/// Rapport renvoyé par `tccd.Control1.ReloadConfig`.
///
/// `ok` n'est vrai que si **aucun** avertissement n'a été produit : un
/// fichier ignoré, corrompu ou trop volumineux doit provoquer un rollback
/// côté `tccd-admin` (AD-6). Les messages d'avertissement ne doivent contenir
/// ni secret ni contenu de fichier ; ils sont bornés et refusent les
/// caractères de contrôle pour éviter l'injection de lignes de journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReloadReport {
    /// Vrai si le rechargement a réussi sans aucun avertissement.
    pub ok: bool,
    /// Avertissements de chargement, dans l'ordre de détection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl AdminReloadReport {
    /// Rapport de succès sans avertissement.
    #[must_use]
    pub fn clean() -> Self {
        Self {
            ok: true,
            warnings: Vec::new(),
        }
    }

    /// Rapport dérivé d'une liste d'avertissements.
    ///
    /// `ok` est vrai uniquement si la liste est vide. Aucun avertissement
    /// n'est tronqué ni filtré silencieusement : un avertissement trop long
    /// fait échouer [`AdminReloadReport::validate`].
    #[must_use]
    pub fn from_warnings(warnings: Vec<String>) -> Self {
        Self {
            ok: warnings.is_empty(),
            warnings,
        }
    }

    /// Vrai si le rechargement est un succès sans avertissement.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.ok
    }

    /// Vérifie les bornes et l'invariant `ok == warnings.is_empty()`.
    ///
    /// # Errors
    ///
    /// Renvoie [`IpcError::InvalidField`] si un avertissement est trop long,
    /// contient un caractère de contrôle, si la liste dépasse
    /// [`MAX_ADMIN_RELOAD_WARNINGS`], ou si `ok` est vrai alors qu'un
    /// avertissement est présent.
    pub fn validate(&self) -> IpcResult<()> {
        if self.warnings.len() > MAX_ADMIN_RELOAD_WARNINGS {
            return Err(IpcError::field(
                "reloadReport.warnings",
                format!(
                    "{} avertissements, maximum {MAX_ADMIN_RELOAD_WARNINGS}",
                    self.warnings.len()
                ),
            ));
        }
        if self.ok && !self.warnings.is_empty() {
            return Err(IpcError::field(
                "reloadReport.ok",
                "doit être false lorsqu'un avertissement est présent",
            ));
        }
        for (index, warning) in self.warnings.iter().enumerate() {
            if warning.chars().count() > MAX_ADMIN_RELOAD_WARNING_CHARS {
                // Ne jamais reprendre le contenu : il peut être sensible.
                return Err(IpcError::field(
                    format!("reloadReport.warnings[{index}]"),
                    format!("dépasse {MAX_ADMIN_RELOAD_WARNING_CHARS} caractères"),
                ));
            }
            if warning.chars().any(char::is_control) {
                return Err(IpcError::field(
                    format!("reloadReport.warnings[{index}]"),
                    "contient un caractère de contrôle",
                ));
            }
        }
        Ok(())
    }

    /// Décode un rapport depuis un JSON borné.
    ///
    /// La taille est vérifiée **avant** `serde_json::from_str` (AD-2).
    ///
    /// # Errors
    ///
    /// Renvoie [`IpcError::PayloadTooLarge`] au-delà de `max_bytes` ou une
    /// erreur de champ si le JSON est invalide ou hors bornes.
    pub fn from_json_bounded(json: &str, max_bytes: usize) -> IpcResult<Self> {
        ensure_payload_size(json, max_bytes)?;
        let report: Self = serde_json::from_str(json)
            .map_err(|error| IpcError::field("reloadReport", error.to_string()))?;
        report.validate()?;
        Ok(report)
    }

    /// Sérialise le rapport après validation.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur de validation ou de sérialisation.
    pub fn to_json(&self) -> IpcResult<String> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| IpcError::field("reloadReport", error.to_string()))
    }
}

/// Identité de version de `tccd-admin`.
///
/// Le contrat est strictement comparé (comme [`crate::IPC_CONTRACT_VERSION`]) :
/// un client dont la version diffère doit refuser d'appeler
/// ([`crate::ensure_admin_contract_compatible`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminVersionInfo {
    /// Version du contrat `Admin1`, attendue égale à
    /// [`crate::ADMIN_CONTRACT_VERSION`].
    pub contract_version: u32,
    /// Version de l'implémentation (par exemple `0.1.0`).
    pub implementation_version: String,
}

impl AdminVersionInfo {
    /// Construit une identité de version.
    #[must_use]
    pub fn new(contract_version: u32, implementation_version: impl Into<String>) -> Self {
        Self {
            contract_version,
            implementation_version: implementation_version.into(),
        }
    }

    /// Construit l'identité de version du contrat courant.
    #[must_use]
    pub fn for_current(implementation_version: impl Into<String>) -> Self {
        Self::new(ADMIN_CONTRACT_VERSION, implementation_version)
    }

    /// Vérifie la version d'implémentation et la cohérence du contrat.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si la version est vide, dépasse
    /// [`MAX_ADMIN_VERSION_CHARS`], contient des caractères non graphiques ou
    /// si `contract_version` diffère de [`crate::ADMIN_CONTRACT_VERSION`].
    pub fn validate(&self) -> IpcResult<()> {
        crate::ensure_admin_contract_compatible(self.contract_version)?;
        if self.implementation_version.is_empty() {
            return Err(IpcError::field(
                "versionInfo.implementationVersion",
                "ne doit pas être vide",
            ));
        }
        if self.implementation_version.chars().count() > MAX_ADMIN_VERSION_CHARS {
            return Err(IpcError::field(
                "versionInfo.implementationVersion",
                format!("dépasse {MAX_ADMIN_VERSION_CHARS} caractères"),
            ));
        }
        if !self
            .implementation_version
            .chars()
            .all(|character| character.is_ascii_graphic())
        {
            return Err(IpcError::field(
                "versionInfo.implementationVersion",
                "doit être ASCII graphique",
            ));
        }
        Ok(())
    }
}

/// Codes d'erreur du namespace `org.tuxedocomputers.tccd.Admin1.Error.*`.
///
/// Ce type est volontairement indépendant de `zbus` : `tcc-admin` mappe ces
/// variantes sur son enum `DBusError` (`linux-platform`). Les valeurs
/// sérialisées sont stables et ne doivent pas être renommées sans incrément
/// de [`crate::ADMIN_CONTRACT_VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AdminErrorCode {
    /// polkit a refusé, est absent, a expiré ou l'identité est inconnue.
    NotAuthorized,
    /// Entrée syntaxiquement invalide (champ nommé uniquement).
    InvalidArgs,
    /// Charge utile au-delà de la limite de 1 Mio.
    PayloadTooLarge,
    /// Modèle valide syntaxiquement mais rejeté par la validation.
    Validation,
    /// Un profil référencé par l'état disparaîtrait.
    InUse,
    /// `tccd` n'a pas pu recharger ; le fichier précédent a été restauré.
    ReloadFailed,
    /// Écriture impossible (détail dans le journal uniquement).
    WriteFailed,
    /// Limite de débit atteinte.
    RateLimited,
    /// Une écriture est déjà en cours.
    Busy,
    /// Matériel absent ou opération non supportée.
    NotSupported,
}

impl AdminErrorCode {
    /// Toutes les variantes, dans l'ordre du contrat.
    pub const ALL: [Self; 10] = [
        Self::NotAuthorized,
        Self::InvalidArgs,
        Self::PayloadTooLarge,
        Self::Validation,
        Self::InUse,
        Self::ReloadFailed,
        Self::WriteFailed,
        Self::RateLimited,
        Self::Busy,
        Self::NotSupported,
    ];

    /// Nom D-Bus complet, dans le namespace applicatif (jamais
    /// `org.freedesktop.DBus.Error.AccessDenied` pour un refus polkit).
    #[must_use]
    pub fn dbus_name(self) -> String {
        format!("{}.{}", crate::ADMIN_ERROR_NAMESPACE, self.as_str())
    }

    /// Suffixe stable de l'erreur (`NotAuthorized`, `PayloadTooLarge`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAuthorized => "NotAuthorized",
            Self::InvalidArgs => "InvalidArgs",
            Self::PayloadTooLarge => "PayloadTooLarge",
            Self::Validation => "Validation",
            Self::InUse => "InUse",
            Self::ReloadFailed => "ReloadFailed",
            Self::WriteFailed => "WriteFailed",
            Self::RateLimited => "RateLimited",
            Self::Busy => "Busy",
            Self::NotSupported => "NotSupported",
        }
    }
}

/// Action `Admin1` de phase 1 ou 2, avec un libellé stable pour l'audit.
///
/// Les méthodes de phase 3 (Tomte, WebFAI) ne sont pas listées : leur design
/// exige une revue `security` dédiée (AD-10). Le changement de mot de passe
/// de chiffrement LUKS est **hors périmètre** (décision utilisateur) et ne
/// doit pas être ajouté ici.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdminAction {
    /// `SaveProfilesJSON` (phase 1).
    SaveProfiles,
    /// `SaveSettingsJSON` (phase 1).
    SaveSettings,
    /// `ReloadConfig` (phase 1).
    ReloadConfig,
    /// `SetTdpValues` (phase 2, transitoire).
    SetTdpValues,
    /// `SetOdmProfile` (phase 2, transitoire).
    SetOdmProfile,
    /// `SetNvidiaCtgpOffset` (phase 2, transitoire).
    SetNvidiaCtgpOffset,
    /// `SetPrimeMode` (phase 2, persistant par nature).
    SetPrimeMode,
}

impl AdminAction {
    /// Toutes les variantes de phase 1 et 2.
    pub const ALL: [Self; 7] = [
        Self::SaveProfiles,
        Self::SaveSettings,
        Self::ReloadConfig,
        Self::SetTdpValues,
        Self::SetOdmProfile,
        Self::SetNvidiaCtgpOffset,
        Self::SetPrimeMode,
    ];

    /// Libellé kebab-case utilisé dans les lignes d'audit (`action=…`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SaveProfiles => "save-profiles",
            Self::SaveSettings => "save-settings",
            Self::ReloadConfig => "reload-config",
            Self::SetTdpValues => "set-tdp-values",
            Self::SetOdmProfile => "set-odm-profile",
            Self::SetNvidiaCtgpOffset => "set-nvidia-ctgp-offset",
            Self::SetPrimeMode => "set-prime-mode",
        }
    }

    /// Nom exact de la méthode D-Bus correspondante.
    #[must_use]
    pub const fn method_name(self) -> &'static str {
        match self {
            Self::SaveProfiles => "SaveProfilesJSON",
            Self::SaveSettings => "SaveSettingsJSON",
            Self::ReloadConfig => "ReloadConfig",
            Self::SetTdpValues => "SetTdpValues",
            Self::SetOdmProfile => "SetOdmProfile",
            Self::SetNvidiaCtgpOffset => "SetNvidiaCtgpOffset",
            Self::SetPrimeMode => "SetPrimeMode",
        }
    }
}

/// Nombre maximal de valeurs TDP acceptées par `SetTdpValues`.
pub const MAX_TDP_VALUES: usize = 3;
/// Longueur maximale des chaînes phase 2 (`SetOdmProfile`).
pub const MAX_HW_STRING_CHARS: usize = 64;
/// Borne statique de l'offset cTGP si les limites NVIDIA sont inconnues.
pub const MAX_CTGP_STATIC_OFFSET_WATTS: i32 = 200;

/// Codes d'erreur du namespace
/// `org.tuxedocomputers.tccd.AdminHw1.Error.*`.
///
/// Indépendant de `zbus`, comme [`AdminErrorCode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AdminHwErrorCode {
    /// polkit a refusé, est absent, a expiré ou l'identité est inconnue.
    NotAuthorized,
    /// Entrée syntaxiquement invalide (code de raison uniquement).
    InvalidArgs,
    /// Valeur syntaxiquement valide mais refusée.
    Validation,
    /// PPD propriétaire ou override non autorisé.
    PolicyDenied,
    /// Matériel ou outil absent.
    NotSupported,
    /// Écriture ou vérification en échec.
    WriteFailed,
    /// État matériel intermédiaire non restauré.
    PartialApply,
    /// Limite de débit atteinte.
    RateLimited,
    /// Une écriture ou un dialogue est déjà en cours.
    Busy,
}

impl AdminHwErrorCode {
    /// Toutes les variantes, dans l'ordre du contrat.
    pub const ALL: [Self; 9] = [
        Self::NotAuthorized,
        Self::InvalidArgs,
        Self::Validation,
        Self::PolicyDenied,
        Self::NotSupported,
        Self::WriteFailed,
        Self::PartialApply,
        Self::RateLimited,
        Self::Busy,
    ];

    /// Nom D-Bus complet.
    #[must_use]
    pub fn dbus_name(self) -> String {
        format!("{}.{}", crate::ADMIN_HW_ERROR_NAMESPACE, self.as_str())
    }

    /// Suffixe stable de l'erreur.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAuthorized => "NotAuthorized",
            Self::InvalidArgs => "InvalidArgs",
            Self::Validation => "Validation",
            Self::PolicyDenied => "PolicyDenied",
            Self::NotSupported => "NotSupported",
            Self::WriteFailed => "WriteFailed",
            Self::PartialApply => "PartialApply",
            Self::RateLimited => "RateLimited",
            Self::Busy => "Busy",
        }
    }
}

/// Vérifie qu'une chaîne ne dépasse pas `max_bytes` avant tout parsing.
fn ensure_payload_size(json: &str, max_bytes: usize) -> IpcResult<()> {
    if json.len() > max_bytes {
        return Err(IpcError::PayloadTooLarge {
            size: json.len(),
            max: max_bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AdminAction, AdminErrorCode, AdminHwErrorCode, AdminReloadReport, AdminVersionInfo,
        MAX_ADMIN_RELOAD_WARNING_CHARS, MAX_ADMIN_RELOAD_WARNINGS, MAX_ADMIN_VERSION_CHARS,
    };
    use crate::IpcError;
    use crate::admin::ensure_payload_size;

    #[test]
    fn clean_report_serializes_without_warnings() {
        let report = AdminReloadReport::clean();
        let json = report.to_json().unwrap();
        assert_eq!(json, r#"{"ok":true}"#);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn report_roundtrip_preserves_warnings() {
        let report = AdminReloadReport::from_warnings(vec![
            "profils: fichier ignoré".to_owned(),
            "réglages: validation échouée".to_owned(),
        ]);
        assert!(!report.is_ok());
        let json = report.to_json().unwrap();
        let decoded = AdminReloadReport::from_json_bounded(&json, 1024).unwrap();
        assert_eq!(decoded, report);
    }

    #[test]
    fn missing_warnings_field_defaults_to_empty() {
        let report: AdminReloadReport = serde_json::from_str(r#"{"ok":true}"#).unwrap();
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn report_rejects_inconsistent_ok() {
        let report = AdminReloadReport {
            ok: true,
            warnings: vec!["avertissement".to_owned()],
        };
        assert!(matches!(
            report.validate(),
            Err(IpcError::InvalidField { .. })
        ));
    }

    #[test]
    fn report_rejects_too_many_warnings() {
        let report = AdminReloadReport::from_warnings(
            (0..=MAX_ADMIN_RELOAD_WARNINGS)
                .map(|index| format!("avertissement {index}"))
                .collect(),
        );
        assert!(matches!(
            report.validate(),
            Err(IpcError::InvalidField { .. })
        ));
    }

    #[test]
    fn report_rejects_oversized_warning_without_echoing_it() {
        let report =
            AdminReloadReport::from_warnings(vec!["x".repeat(MAX_ADMIN_RELOAD_WARNING_CHARS + 1)]);
        let error = report.validate().unwrap_err();
        let message = error.to_string();
        assert!(message.contains("warnings[0]"));
        assert!(!message.contains("xxxx"));
    }

    #[test]
    fn report_rejects_control_characters() {
        let report = AdminReloadReport::from_warnings(vec!["ligne\nsuivante".to_owned()]);
        assert!(report.validate().is_err());
    }

    #[test]
    fn report_rejects_payload_before_parsing() {
        let error = AdminReloadReport::from_json_bounded(r#"{"ok":true}"#, 4).unwrap_err();
        match error {
            IpcError::PayloadTooLarge { size, max } => {
                assert_eq!(size, 11);
                assert_eq!(max, 4);
            }
            other => panic!("erreur inattendue: {other:?}"),
        }
    }

    #[test]
    fn size_check_accepts_exact_limit() {
        assert!(ensure_payload_size("abc", 3).is_ok());
        assert!(matches!(
            ensure_payload_size("abcd", 3),
            Err(IpcError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn version_info_roundtrip_and_validation() {
        let info = AdminVersionInfo::for_current("0.1.0");
        assert_eq!(info.contract_version, crate::ADMIN_CONTRACT_VERSION);
        assert!(info.validate().is_ok());
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("contractVersion"));
        assert!(json.contains("implementationVersion"));
        let decoded: AdminVersionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn version_info_rejects_empty_spaces_and_long_values() {
        assert!(AdminVersionInfo::for_current("").validate().is_err());
        assert!(AdminVersionInfo::for_current("0.1 0").validate().is_err());
        assert!(
            AdminVersionInfo::for_current("x".repeat(MAX_ADMIN_VERSION_CHARS + 1))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn version_info_rejects_other_contract_versions() {
        let info = AdminVersionInfo::new(crate::ADMIN_CONTRACT_VERSION + 1, "0.1.0");
        assert!(matches!(
            info.validate(),
            Err(IpcError::AdminContractMismatch { .. })
        ));
    }

    #[test]
    fn error_codes_use_application_namespace() {
        for code in AdminErrorCode::ALL {
            let name = code.dbus_name();
            assert!(name.starts_with("org.tuxedocomputers.tccd.Admin1.Error."));
            assert!(name.ends_with(code.as_str()));
        }
        assert_eq!(
            AdminErrorCode::NotAuthorized.dbus_name(),
            "org.tuxedocomputers.tccd.Admin1.Error.NotAuthorized"
        );
    }

    #[test]
    fn actions_have_stable_labels_and_serde_keys() {
        for action in AdminAction::ALL {
            assert_eq!(
                serde_json::to_string(&action).unwrap(),
                format!("\"{}\"", action.as_str())
            );
            assert!(!action.method_name().is_empty());
        }
        assert_eq!(AdminAction::SaveProfiles.as_str(), "save-profiles");
        assert_eq!(AdminAction::SaveProfiles.method_name(), "SaveProfilesJSON");
    }

    #[test]
    fn admin_hw_error_codes_use_their_own_namespace() {
        for code in AdminHwErrorCode::ALL {
            let name = code.dbus_name();
            assert!(name.starts_with("org.tuxedocomputers.tccd.AdminHw1.Error."));
            assert!(name.ends_with(code.as_str()));
        }
        assert_eq!(
            AdminHwErrorCode::PolicyDenied.dbus_name(),
            "org.tuxedocomputers.tccd.AdminHw1.Error.PolicyDenied"
        );
        assert_eq!(
            AdminHwErrorCode::PartialApply.dbus_name(),
            "org.tuxedocomputers.tccd.AdminHw1.Error.PartialApply"
        );
    }
}
