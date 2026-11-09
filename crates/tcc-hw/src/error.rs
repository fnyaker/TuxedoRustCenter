// SPDX-License-Identifier: GPL-3.0-or-later
//! Erreurs typées de la couche matérielle.
//!
//! Aucune erreur de ce module ne contient de secret. Les chemins sont ceux du
//! noyau (jamais des données utilisateur), les valeurs invalides sont tronquées
//! à une taille raisonnable par les constructeurs.

use std::io;
use std::path::PathBuf;

/// Erreur de la couche `tcc-hw`.
#[derive(Debug, thiserror::Error)]
pub enum HwError {
    /// Le fichier ou le répertoire sysfs attendu n'existe pas.
    #[error("chemin sysfs absent : {path}")]
    NotFound { path: PathBuf },

    /// Le chemin demandé sort de la racine sysfs ou n'est pas un chemin relatif.
    #[error("chemin sysfs invalide : {path}")]
    InvalidPath { path: PathBuf },

    /// Le fichier cible d'une écriture est un lien symbolique ou n'est pas un
    /// fichier régulier. Refusé pour éviter une écriture hors sysfs.
    #[error("écriture refusée sur {path} : lien symbolique ou fichier non régulier")]
    UnsafeTarget { path: PathBuf },

    /// Le fichier cible d'une écriture n'est pas possédé par root ou est
    /// accessible en écriture au monde.
    #[error("écriture refusée sur {path} : propriétaire ou permissions incorrects")]
    UnsafePermissions { path: PathBuf },

    /// Une valeur lue ou à écrire n'a pas le format attendu.
    #[error("valeur invalide pour {context} : {value}")]
    InvalidValue {
        context: &'static str,
        value: String,
    },

    /// Une valeur numérique sort de la plage acceptée par le matériel.
    #[error("valeur hors plage pour {context} : {value} (attendu {min}..={max})")]
    OutOfRange {
        context: &'static str,
        value: i64,
        min: i64,
        max: i64,
    },

    /// Le périphérique n'est pas présent ou ne peut pas être ouvert.
    #[error("périphérique indisponible : {device}")]
    Unavailable {
        device: &'static str,
        #[source]
        source: io::Error,
    },

    /// Un capteur existe mais ne fournit pas de valeur exploitable (ventilateur
    /// absent, température à zéro, ...).
    #[error("capteur indisponible : {sensor}")]
    SensorUnavailable { sensor: &'static str },

    /// Un appel `ioctl` a échoué.
    #[error("ioctl {name} (0x{request:08x}) a échoué")]
    Ioctl {
        name: &'static str,
        request: u64,
        #[source]
        source: io::Error,
    },

    /// Ni l'interface Clevo ni l'interface Uniwill n'a répondu.
    #[error("interface tuxedo-io non reconnue (ni Clevo ni Uniwill)")]
    UnsupportedDevice,

    /// La fonction n'existe pas sur l'interface détectée.
    #[error("fonction non supportée par cette interface : {0}")]
    NotSupported(&'static str),

    /// La version du module noyau est plus ancienne que l'API minimale.
    #[error("version d'API tuxedo-io incompatible : {found} < {minimum}")]
    IncompatibleApiVersion { found: String, minimum: String },

    /// Erreur d'entrée/sortie avec le chemin concerné.
    #[error("erreur d'E/S sur {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Un fichier sysfs contient des octets non UTF-8.
    #[error("contenu non UTF-8 dans {path}")]
    NonUtf8 { path: PathBuf },

    /// Aucun backend d'affichage n'est disponible dans la session.
    #[error("aucun backend d'affichage disponible")]
    NoDisplayBackend,

    /// Le mode demandé n'existe pas pour l'écran interne.
    #[error("mode d'affichage introuvable : {0}")]
    DisplayModeNotFound(String),

    /// L'appelant a désigné un écran externe ; refusé par contrat.
    #[error("écran non interne refusé : {0}")]
    NotInternalOutput(String),

    /// Le backend d'affichage a signalé une erreur.
    #[error("backend d'affichage : {0}")]
    Display(String),

    /// Une ressource est pilotée par un autre composant (power-profiles-daemon
    /// par défaut) et l'écriture n'est pas autorisée dans ce mode.
    #[error("écriture refusée sur {resource} : {reason}")]
    PolicyDenied {
        /// Ressource concernée (`platform_profile`, `ctgp_offset`, ...).
        resource: &'static str,
        /// Raison stable, sans donnée utilisateur.
        reason: &'static str,
    },

    /// Une commande externe allowlistée a échoué, expiré ou produit une sortie
    /// inexploitable.
    #[error("commande externe {program} : {reason}")]
    External {
        /// Chemin absolu du programme (constante de compilation).
        program: &'static str,
        /// Description bornée et nettoyée de l'erreur.
        reason: String,
    },

    /// Le périphérique est occupé (aperçu actif, autre application, `EBUSY`).
    ///
    /// Ce n'est pas une absence matérielle : l'appelant peut réessayer avec un
    /// back-off borné.
    #[error("périphérique occupé : {device}")]
    Busy {
        /// Nom du périphérique (`video0`, `tuxedo_io`, ...).
        device: String,
    },

    /// Une opération matérielle est bloquée au-delà du budget et le
    /// périphérique est marqué `stalled` jusqu'à un rebranchement.
    #[error("périphérique bloqué : {device}")]
    Stalled {
        /// Nom du périphérique.
        device: String,
    },

    /// Le budget d'ioctl d'une opération est épuisé avant la fin.
    #[error("budget d'ioctl épuisé pour {operation} (limite {limit})")]
    BudgetExceeded {
        /// Opération concernée.
        operation: &'static str,
        /// Nombre maximal d'ioctl autorisés.
        limit: u32,
    },
}

impl HwError {
    /// Construit une erreur d'E/S contextualisée.
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Construit une erreur d'E/S contextualisée ou `NotFound` si le noyau
    /// indique que le fichier n'existe pas.
    pub fn io_or_not_found(path: impl Into<PathBuf>, source: io::Error) -> Self {
        let path = path.into();
        if source.kind() == io::ErrorKind::NotFound {
            Self::NotFound { path }
        } else {
            Self::Io { path, source }
        }
    }

    /// Indique si l'erreur signifie « fonctionnalité absente du matériel ou du
    /// noyau », cas normal sur une machine non-TUXEDO.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::NotFound { .. }
                | Self::Unavailable { .. }
                | Self::UnsupportedDevice
                | Self::NotSupported(_)
        )
    }

    /// Indique si l'erreur signifie « présente mais pilotée par un autre
    /// composant » ; l'appelant doit sauter l'écriture sans la considérer comme
    /// une panne.
    pub fn is_policy_denied(&self) -> bool {
        matches!(self, Self::PolicyDenied { .. })
    }
}

/// Résultat de la couche matérielle.
pub type Result<T> = std::result::Result<T, HwError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_is_unavailable() {
        let err = HwError::NotFound {
            path: PathBuf::from("/sys/class/power_supply/AC"),
        };
        assert!(err.is_unavailable());
    }

    #[test]
    fn out_of_range_message_contains_bounds() {
        let err = HwError::OutOfRange {
            context: "fan",
            value: 250,
            min: 0,
            max: 100,
        };
        let msg = err.to_string();
        assert!(msg.contains("0..=100"));
    }
}
