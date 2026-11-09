// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter, réécriture en Rust de
// TUXEDO Control Center. Il est distribué sous licence GPL-3.0-or-later.
// Le code est une réimplémentation originale : aucun fichier de
// l'implémentation Electron/Node.js n'a été copié.

//! Erreurs typées de `tcc-core`.
//!
//! Toutes les erreurs publiques du crate sont regroupées ici afin de rester
//! faciles à convertir dans les binaires (`tcc-daemon`, `tcc-cli`, …) sans
//! exposer de dépendance à `anyhow` dans la logique métier.
//!
//! Trois familles coexistent :
//!
//! - [`CoreError`] : erreurs d'entrée/sortie, de configuration et de
//!   sérialisation ;
//! - [`StrictLoadError`] : lecture stricte sans récupération ni quarantaine,
//!   pour les validations qui ne doivent pas modifier le disque ;
//! - [`ValidationError`] : résultat de la validation centralisée, qui peut
//!   contenir plusieurs [`ValidationIssue`] à la fois pour éviter de ne
//!   signaler qu'une seule erreur par chargement.

use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

/// Résultat standard des opérations de `tcc-core`.
pub type CoreResult<T> = Result<T, CoreError>;

/// Erreur de haut niveau du crate.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Une opération d'entrée/sortie a échoué sur un chemin précis.
    #[error("échec d'entrée/sortie sur {path}: {source}")]
    Io {
        /// Chemin concerné (fichier ou répertoire).
        path: PathBuf,
        /// Erreur système d'origine.
        #[source]
        source: std::io::Error,
    },

    /// Un fichier JSON n'a pas pu être décodé.
    #[error("JSON invalide dans {path}: {source}")]
    Json {
        /// Chemin du fichier en cause.
        path: PathBuf,
        /// Erreur `serde_json` d'origine (ligne, colonne, message).
        #[source]
        source: serde_json::Error,
    },

    /// Une valeur n'a pas pu être sérialisée en JSON.
    ///
    /// Ce cas ne devrait concerner que des valeurs non finies (`NaN`,
    /// infini) ou des types non sérialisables, jamais une donnée utilisateur
    /// bien formée.
    #[error("sérialisation JSON impossible: {0}")]
    Serde(#[from] serde_json::Error),

    /// La validation centralisée a rejeté la valeur.
    #[error(transparent)]
    Validation(#[from] ValidationError),

    /// Erreur de configuration ou d'invariant détecté par le crate.
    #[error("configuration invalide: {0}")]
    Config(String),
}

impl CoreError {
    /// Construit une erreur d'entrée/sortie contextualisée.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Construit une erreur de configuration.
    #[must_use]
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// Ajoute un chemin à une erreur de désérialisation.
    ///
    /// Utilisé par la migration et les helpers de lecture en mémoire, qui ne
    /// connaissent pas forcément le chemin du fichier au moment du parsing.
    #[must_use]
    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        match self {
            Self::Serde(source) => Self::Json {
                path: path.into(),
                source,
            },
            other => other,
        }
    }
}

/// Erreur de lecture stricte d'un fichier de configuration.
///
/// Renvoyée par
/// [`ConfigStore::load_settings_no_recovery`](crate::ConfigStore::load_settings_no_recovery)
/// et
/// [`ConfigStore::load_profiles_no_recovery`](crate::ConfigStore::load_profiles_no_recovery).
/// Contrairement à [`crate::ConfigStore::load_settings`], aucune récupération
/// n'est tentée : le fichier n'est ni renommé en `.bak`, ni en
/// `.bak.corrupt`, et aucun temporaire n'est créé. L'appelant décide
/// explicitement quoi faire d'un contenu invalide.
#[derive(Debug, Error)]
pub enum StrictLoadError {
    /// Aucun fichier d'état ni fichier administrateur n'existe.
    #[error("fichier de configuration absent: {path}")]
    NotFound {
        /// Chemin d'état recherché. La racine administrateur a aussi été
        /// consultée avant de conclure à l'absence.
        path: PathBuf,
    },

    /// Le fichier dépasse la limite de lecture configurée.
    #[error("fichier de configuration trop volumineux: {path} ({size} octets, limite {max})")]
    TooLarge {
        /// Fichier concerné.
        path: PathBuf,
        /// Taille observée (métadonnée ou lecture bornée).
        size: u64,
        /// Limite configurée par [`crate::LoadPolicy::max_file_bytes`].
        max: u64,
    },

    /// Lecture impossible ou refus de sécurité.
    ///
    /// Couvre les erreurs d'entrée/sortie, les liens symboliques, les fichiers
    /// non réguliers, un propriétaire inattendu et des permissions trop
    /// larges. Le motif de refus est porté par `source`, jamais par le contenu
    /// du fichier.
    #[error("lecture impossible de {path}: {source}")]
    Io {
        /// Fichier ou répertoire concerné.
        path: PathBuf,
        /// Erreur système d'origine ou refus de sécurité.
        #[source]
        source: std::io::Error,
    },

    /// Contenu non UTF-8, JSON invalide ou rejeté par la validation.
    #[error("contenu de configuration invalide dans {path}: {reason}")]
    Invalid {
        /// Fichier concerné.
        path: PathBuf,
        /// Motif lisible (champ, ligne/colonne), sans contenu brut.
        reason: String,
    },
}

/// Une anomalie de validation localisée sur un champ.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field}: {message}")]
pub struct ValidationIssue {
    /// Nom du champ concerné, par exemple `display.refreshRate`.
    pub field: String,
    /// Description lisible du problème.
    pub message: String,
}

impl ValidationIssue {
    /// Crée une anomalie pour un champ donné.
    #[must_use]
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Ensemble d'anomalies de validation.
///
/// La validation agrège volontairement toutes les anomalies détectées : une
/// GUI peut ainsi afficher les champs fautifs en une seule passe, et la
/// migration n'écrase pas un fichier pour une seule valeur aberrante.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationError {
    issues: Vec<ValidationIssue>,
}

impl ValidationError {
    /// Construit une erreur vide (utile comme accumulateur).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construit une erreur à partir d'une liste d'anomalies.
    ///
    /// Une liste vide donne une erreur vide ; préférez
    /// [`ValidationError::from_issues`] pour obtenir un `Result`.
    #[must_use]
    pub fn from_vec(issues: Vec<ValidationIssue>) -> Self {
        Self { issues }
    }

    /// Convertit une liste d'anomalies en `Result`.
    ///
    /// # Errors
    ///
    /// Renvoie une `ValidationError` si la liste n'est pas vide.
    pub fn from_issues(issues: Vec<ValidationIssue>) -> Result<(), Self> {
        if issues.is_empty() {
            Ok(())
        } else {
            Err(Self { issues })
        }
    }

    /// Ajoute une anomalie localisée sur un champ.
    pub fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue::new(field, message));
    }

    /// Ajoute une anomalie déjà construite.
    pub fn push_issue(&mut self, issue: ValidationIssue) {
        self.issues.push(issue);
    }

    /// Ajoute toutes les anomalies d'une autre erreur.
    pub fn extend(&mut self, other: Self) {
        self.issues.extend(other.issues);
    }

    /// Liste des anomalies, dans l'ordre de détection.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }

    /// Vrai si aucune anomalie n'a été enregistrée.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// Nombre d'anomalies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    /// Consomme l'erreur et renvoie ses anomalies.
    #[must_use]
    pub fn into_issues(self) -> Vec<ValidationIssue> {
        self.issues
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            return f.write_str("validation échouée (aucun détail)");
        }
        for (index, issue) in self.issues.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl From<ValidationIssue> for ValidationError {
    fn from(issue: ValidationIssue) -> Self {
        Self {
            issues: vec![issue],
        }
    }
}

impl IntoIterator for ValidationError {
    type Item = ValidationIssue;
    type IntoIter = std::vec::IntoIter<ValidationIssue>;

    fn into_iter(self) -> Self::IntoIter {
        self.issues.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{CoreError, ValidationError, ValidationIssue};

    #[test]
    fn validation_error_accumulates_and_formats_issues() {
        let mut error = ValidationError::new();
        error.push("display.brightness", "doit être ≤ 100");
        error.push("cpu.governor", "ne doit pas être vide");

        assert_eq!(error.len(), 2);
        assert!(!error.is_empty());
        let rendered = error.to_string();
        assert!(rendered.contains("display.brightness"));
        assert!(rendered.contains("cpu.governor"));
    }

    #[test]
    fn empty_validation_error_converts_to_ok() {
        let issues = Vec::<ValidationIssue>::new();
        assert_eq!(ValidationError::from_issues(issues), Ok(()));
    }

    #[test]
    fn with_path_only_rewrites_serde_errors() {
        let source = serde_json::from_str::<u32>("{").expect_err("JSON invalide attendu");
        let error = CoreError::Serde(source).with_path("/tmp/settings.json");
        match error {
            CoreError::Json { path, .. } => {
                assert_eq!(path.to_string_lossy(), "/tmp/settings.json")
            }
            other => panic!("variante inattendue: {other:?}"),
        }

        let error = CoreError::config("test").with_path("/tmp/settings.json");
        assert!(matches!(error, CoreError::Config(_)));
    }
}
