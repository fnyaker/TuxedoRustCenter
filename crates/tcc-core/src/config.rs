// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. L'implémentation du stockage
// de configuration est originale. Elle applique les décisions D2 (état
// mutable dans /var/lib, /etc en lecture seule) et S-05 (Openat/O_NOFOLLOW,
// vérification propriétaire/permissions, écriture atomique) du modèle de
// menace du projet.

//! Stockage JSON de la configuration et de l'état persistant.
//!
//! # Emplacements
//!
//! Deux magasins distincts :
//!
//! - **état mutable** : `/var/lib/tuxedo-rust-center/` (`settings.json`,
//!   `profiles.json`, `fantables.json`), lisible et inscriptible par `tccd`
//!   root. C'est le magasin par défaut ([`ConfigStore::state_default`]) ;
//! - **configuration administrateur** : `/etc/tuxedo-rust-center/`, en
//!   lecture seule pour le daemon ([`ConfigStore::admin_default`]). Les
//!   `save_*` y échouent structurellement.
//!
//! # Écritures atomiques
//!
//! Le fichier est sérialisé en mémoire, écrit dans un fichier temporaire du
//! même répertoire (`create_new`), passé en `0644` par `fchmod`, synchronisé
//! (`fsync`), puis renommé sur la cible. Le répertoire est ensuite
//! synchronisé pour garantir la persistance de l'entrée. En cas d'échec, le
//! temporaire est supprimé ; l'ancien fichier reste intact.
//!
//! # Écriture administrateur avec sauvegarde
//!
//! [`ConfigStore::save_settings_with_backup`] et
//! [`ConfigStore::save_profiles_with_backup`] ajoutent une génération `.bak`
//! (`0600`) avant remplacement, pour `tccd-admin` (AD-5). Elles exigent une
//! racine d'écriture déclarée (`/var/lib/tuxedo-rust-center` en production),
//! refusent les liens symboliques et les fichiers non réguliers, et ne créent
//! jamais le répertoire d'état. Un contenu identique est une opération
//! idempotente sans écriture.
//!
//! **Le `.bak` n'est remplacé que par un contenu exploitable.** Avant de
//! l'écrire, le contenu courant est relu, borné, puis désérialisé et validé
//! avec les mêmes règles que la lecture. S'il est illisible, tronqué, non
//! UTF-8, d'un autre type ou refusé par la validation, le `.bak` existant est
//! **conservé intact** : la dernière version saine n'est jamais écrasée par
//! un contenu corrompu (`docs/security/REVIEW_ADMIN_PHASE1.md`, A1-07). La
//! cible est tout de même remplacée par la nouvelle valeur, qui vient d'être
//! validée ; [`Saved::backup`] reste `None` et [`Saved::preserved_backup`]
//! porte le chemin de la sauvegarde conservée.
//!
//! # Lecture
//!
//! - ouverture avec `O_NOFOLLOW` (aucun lien symbolique final) ;
//! - vérification que le descripteur désigne un fichier régulier, possédé par
//!   le propriétaire attendu, sans écriture groupe/autres ;
//! - taille plafonnée (1 Mio par défaut) ;
//! - JSON invalide, UTF-8 invalide ou validation en échec : le fichier est
//!   renommé en `.bak` (mode `0600`), un avertissement typé est renvoyé et
//!   les valeurs par défaut sont utilisées. Si un `.bak` régulier existe
//!   déjà, il n'est jamais écrasé par le contenu corrompu : celui-ci va dans
//!   `.bak.corrupt` (nom fixe, une génération) ;
//! - fichier absent : valeurs par défaut, sans avertissement.
//!
//! Les erreurs d'entrée/sortie réelles et les violations de sécurité ne sont
//! jamais transformées en valeurs par défaut : elles remontent en
//! [`crate::CoreError`] pour être journalisées par le daemon.
//!
//! # Lecture stricte sans récupération
//!
//! [`ConfigStore::load_settings_no_recovery`] et
//! [`ConfigStore::load_profiles_no_recovery`] offrent la même lecture durcie
//! (`O_NOFOLLOW|O_CLOEXEC`, propriétaire, permissions, borne) et la même
//! validation, mais **sans aucun effet de bord** : un fichier invalide est
//! signalé par [`StrictLoadError::Invalid`] et n'est jamais renommé en
//! `.bak`/`.bak.corrupt`, aucun temporaire n'est créé. Absence ⇒
//! [`StrictLoadError::NotFound`] après consultation de la racine admin.
//! [`ConfigStore::load_profiles_state_only_no_recovery`] est la variante sans
//! repli `/etc`, réservée au diff de tombstones de `tccd-admin` (AD-R13).
//! C'est l'API à utiliser pour une validation croisée qui ne doit pas
//! modifier le disque (A1-07 de `REVIEW_ADMIN_PHASE1`).
//!
//! # Limite connue
//!
//! `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` dépend du noyau et d'une
//! dépendance supplémentaire. Cette version utilise `O_NOFOLLOW` sur le
//! dernier composant, vérifie le répertoire parent (type, propriétaire,
//! permissions) et n'utilise aucun composant de chemin fourni par
//! l'utilisateur. Les racines sont des constantes de compilation ou des
//! chemins injectés par le daemon, jamais un champ de profil.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{CoreError, StrictLoadError, ValidationError};
use crate::fan::FanTableSet;
use crate::profile::ProfileCollection;
use crate::settings::Settings;

/// Répertoire d'état mutable du daemon (D2).
pub const STATE_DIR: &str = "/var/lib/tuxedo-rust-center";
/// Répertoire de configuration administrateur, lecture seule pour le daemon.
pub const ADMIN_CONFIG_DIR: &str = "/etc/tuxedo-rust-center";
/// Nom du fichier de réglages.
pub const SETTINGS_FILE: &str = "settings.json";
/// Nom du fichier de profils.
pub const PROFILES_FILE: &str = "profiles.json";
/// Nom du fichier de courbes de ventilation.
pub const FAN_TABLES_FILE: &str = "fantables.json";
/// Nom du fichier de préréglages webcam (possédé par le module V4L2).
pub const WEBCAM_FILE: &str = "webcam.json";
/// Taille maximale par défaut d'un fichier de configuration (1 Mio).
pub const DEFAULT_MAX_CONFIG_BYTES: u64 = 1024 * 1024;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Chemins des fichiers de configuration.
///
/// Construits à partir d'une racine injectable. Les noms de fichiers sont des
/// constantes : aucun champ utilisateur ne participe à un chemin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    /// Répertoire racine.
    pub root: PathBuf,
    /// Fichier des réglages.
    pub settings: PathBuf,
    /// Fichier des profils.
    pub profiles: PathBuf,
    /// Fichier des courbes de ventilation.
    pub fan_tables: PathBuf,
    /// Fichier des préréglages webcam.
    pub webcam: PathBuf,
}

impl ConfigPaths {
    /// Construit les chemins à partir d'une racine.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            settings: root.join(SETTINGS_FILE),
            profiles: root.join(PROFILES_FILE),
            fan_tables: root.join(FAN_TABLES_FILE),
            webcam: root.join(WEBCAM_FILE),
            root,
        }
    }

    /// Chemins de l'état mutable par défaut (`/var/lib/...`).
    #[must_use]
    pub fn state_default() -> Self {
        Self::new(STATE_DIR)
    }

    /// Chemins de la configuration administrateur (`/etc/...`).
    #[must_use]
    pub fn admin_default() -> Self {
        Self::new(ADMIN_CONFIG_DIR)
    }
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self::state_default()
    }
}

/// Politique de lecture/écriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPolicy {
    /// Taille maximale d'un fichier lu, en octets.
    pub max_file_bytes: u64,
    /// Renomme un fichier invalide en `.bak` avant de repartir des défauts.
    pub backup_corrupt: bool,
    /// Propriétaire attendu des fichiers et de leur répertoire.
    ///
    /// `Some(0)` en production : le daemon root refuse un fichier ou un
    /// répertoire appartenant à un autre utilisateur. `None` désactive ce
    /// contrôle (tests, outils de diagnostic).
    pub expected_owner: Option<u32>,
    /// Vérifie que le répertoire et le fichier ne sont pas accessibles en
    /// écriture au groupe ou aux autres.
    pub enforce_permissions: bool,
}

impl Default for LoadPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_CONFIG_BYTES,
            backup_corrupt: true,
            expected_owner: None,
            enforce_permissions: true,
        }
    }
}

impl LoadPolicy {
    /// Politique de production : propriétaire root, 1 Mio, sauvegarde `.bak`.
    #[must_use]
    pub fn for_root_daemon() -> Self {
        Self {
            expected_owner: Some(0),
            ..Self::default()
        }
    }
}

/// Type d'avertissement de lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadWarningKind {
    /// Contenu non UTF-8.
    InvalidUtf8,
    /// JSON syntaxiquement invalide.
    InvalidJson,
    /// Fichier plus volumineux que la limite.
    TooLarge,
    /// JSON valide mais données rejetées par la validation.
    Validation,
}

impl LoadWarningKind {
    /// Libellé court pour les logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "UTF-8 invalide",
            Self::InvalidJson => "JSON invalide",
            Self::TooLarge => "fichier trop volumineux",
            Self::Validation => "validation échouée",
        }
    }
}

/// Avertissement de récupération.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadWarning {
    /// Fichier concerné.
    pub path: PathBuf,
    /// Nature du problème.
    pub kind: LoadWarningKind,
    /// Message détaillé (erreur de parsing ou de validation).
    pub message: String,
    /// Chemin de la sauvegarde créée, si `backup_corrupt` est actif.
    pub backup_path: Option<PathBuf>,
}

/// Valeur chargée, éventuellement accompagnée d'un avertissement.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded<T> {
    /// Valeur utilisable (fichier valide ou valeurs par défaut).
    pub value: T,
    /// Avertissement si le fichier a été ignoré.
    pub warning: Option<LoadWarning>,
}

impl<T> Loaded<T> {
    /// Construit une valeur chargée sans avertissement.
    #[must_use]
    pub fn clean(value: T) -> Self {
        Self {
            value,
            warning: None,
        }
    }

    /// Consomme le résultat et renvoie la valeur.
    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }

    /// Vrai si le fichier a été ignoré.
    #[must_use]
    pub fn has_warning(&self) -> bool {
        self.warning.is_some()
    }

    /// Avertissement éventuel.
    #[must_use]
    pub fn warning(&self) -> Option<&LoadWarning> {
        self.warning.as_ref()
    }
}

/// Résultat d'une écriture atomique avec sauvegarde.
///
/// La sauvegarde n'existe que si un fichier précédent a réellement été
/// remplacé et que son contenu était exploitable. Une écriture idempotente
/// (contenu identique) ne crée ni ne réécrit de `.bak` : elle renvoie
/// `backup: None` et `preserved_backup: None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Chemin du `.bak` créé, si une version précédente a été sauvegardée.
    pub backup: Option<PathBuf>,
    /// `.bak` existant laissé intact parce que le contenu remplacé n'était
    /// pas exploitable (corrompu, d'un autre type ou refusé par la
    /// validation).
    ///
    /// Ce n'est ni une erreur ni un no-op : la cible a bien été écrite avec
    /// la nouvelle valeur. Le champ existe pour que l'appelant puisse
    /// journaliser le fait qu'une sauvegarde plus ancienne a été préservée.
    pub preserved_backup: Option<PathBuf>,
}

/// Magasin de configuration.
///
/// Le magasin porte toujours une racine d'état mutable. Il peut en plus
/// consulter une racine de configuration administrateur (`/etc`) en lecture
/// seule, utilisée comme source de valeurs par défaut packagées lorsqu'un
/// fichier d'état est absent. Les écritures ne vont **jamais** dans la racine
/// admin.
///
/// # Écriture avec sauvegarde
///
/// [`ConfigStore::save_settings_with_backup`] et
/// [`ConfigStore::save_profiles_with_backup`] sont réservées à
/// `tccd-admin` (AD-5). Elles exigent une racine d'écriture explicite
/// ([`ConfigStore::with_write_root`]) : en production,
/// [`ConfigStore::system`] la fixe à `/var/lib/tuxedo-rust-center` (D2), et
/// aucun chemin fourni par un client ne participe à l'écriture.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    state_paths: ConfigPaths,
    admin_paths: Option<ConfigPaths>,
    policy: LoadPolicy,
    read_only: bool,
    write_root: Option<PathBuf>,
}

impl ConfigStore {
    /// Crée un magasin à partir de chemins d'état explicites.
    #[must_use]
    pub fn new(paths: ConfigPaths) -> Self {
        Self {
            state_paths: paths,
            admin_paths: None,
            policy: LoadPolicy::default(),
            read_only: false,
            write_root: None,
        }
    }

    /// Crée un magasin à partir d'une racine d'état.
    #[must_use]
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self::new(ConfigPaths::new(root))
    }

    /// Crée un magasin avec deux racines injectées : état mutable et admin.
    ///
    /// La racine admin est **toujours** en lecture seule, même si le magasin
    /// d'état ne l'est pas. C'est la configuration attendue en production
    /// (D2).
    #[must_use]
    pub fn with_admin(state: ConfigPaths, admin: ConfigPaths) -> Self {
        Self {
            state_paths: state,
            admin_paths: Some(admin),
            policy: LoadPolicy::default(),
            read_only: false,
            write_root: None,
        }
    }

    /// Variante de [`ConfigStore::with_admin`] à partir de deux racines.
    #[must_use]
    pub fn with_admin_roots(
        state_root: impl Into<PathBuf>,
        admin_root: impl Into<PathBuf>,
    ) -> Self {
        Self::with_admin(ConfigPaths::new(state_root), ConfigPaths::new(admin_root))
    }

    /// État mutable de production (`/var/lib/tuxedo-rust-center`).
    ///
    /// La racine d'écriture avec sauvegarde est fixée à
    /// [`STATE_DIR`] : cette méthode est utilisable par un outil
    /// d'administration, contrairement à [`ConfigStore::from_root`] qui
    /// n'autorise aucune écriture avec sauvegarde tant qu'une racine n'a pas
    /// été déclarée explicitement.
    #[must_use]
    pub fn state_default() -> Self {
        Self::new(ConfigPaths::state_default())
            .with_policy(LoadPolicy::for_root_daemon())
            .with_write_root(STATE_DIR)
    }

    /// Magasin de production complet : état `/var/lib` + admin `/etc`.
    ///
    /// Les fichiers absents de `/var/lib` sont cherchés dans `/etc` ; les
    /// écritures restent cantonnées à `/var/lib` et sont refusées si la racine
    /// d'état est sous `/etc`. C'est le magasin que doit utiliser
    /// `tccd-admin` (AD-5).
    #[must_use]
    pub fn system() -> Self {
        Self::with_admin(ConfigPaths::state_default(), ConfigPaths::admin_default())
            .with_policy(LoadPolicy::for_root_daemon())
            .with_write_root(STATE_DIR)
    }

    /// Variante de [`ConfigStore::system`] avec deux racines injectées et la
    /// politique root (propriétaire attendu, permissions strictes).
    ///
    /// La racine d'écriture est fixée à `state_root` : les sauvegardes ne
    /// peuvent pas sortir du magasin injecté.
    #[must_use]
    pub fn system_with_roots(
        state_root: impl Into<PathBuf>,
        admin_root: impl Into<PathBuf>,
    ) -> Self {
        let state_paths = ConfigPaths::new(state_root);
        let write_root = state_paths.root.clone();
        Self::with_admin(state_paths, ConfigPaths::new(admin_root))
            .with_policy(LoadPolicy::for_root_daemon())
            .with_write_root(write_root)
    }

    /// Configuration administrateur (`/etc/tuxedo-rust-center`), lecture seule.
    #[must_use]
    pub fn admin_default() -> Self {
        Self::new(ConfigPaths::admin_default())
            .with_policy(LoadPolicy::for_root_daemon())
            .read_only()
    }

    /// Remplace la politique de lecture/écriture.
    #[must_use]
    pub fn with_policy(mut self, policy: LoadPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Marque le magasin comme lecture seule.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Vrai si les écritures sont refusées.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Déclare la seule racine sous laquelle les écritures avec sauvegarde
    /// sont autorisées.
    ///
    /// En production, [`ConfigStore::system`] la fixe à `/var/lib/tuxedo-rust-center`
    /// (D2/AD-5). Ce constructeur explicite existe pour les tests et les
    /// outils de diagnostic : un magasin construit par
    /// [`ConfigStore::from_root`] refuse par défaut toute écriture avec
    /// sauvegarde. Aucune valeur provenant d'un client D-Bus ne doit
    /// alimenter ce paramètre.
    #[must_use]
    pub fn with_write_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.write_root = Some(root.into());
        self
    }

    /// Racine d'écriture autorisée pour les sauvegardes, si déclarée.
    #[must_use]
    pub fn write_root(&self) -> Option<&Path> {
        self.write_root.as_deref()
    }

    /// Chemins de la racine d'état (celle qui reçoit les écritures).
    #[must_use]
    pub fn paths(&self) -> &ConfigPaths {
        &self.state_paths
    }

    /// Chemins de la racine admin, si elle est configurée.
    #[must_use]
    pub fn admin_paths(&self) -> Option<&ConfigPaths> {
        self.admin_paths.as_ref()
    }

    /// Politique utilisée.
    #[must_use]
    pub fn policy(&self) -> &LoadPolicy {
        &self.policy
    }

    /// Charge les réglages.
    ///
    /// Cherche d'abord `/var/lib`, puis la racine admin si elle est
    /// configurée, puis les valeurs par défaut du crate.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur pour un problème d'entrée/sortie réel ou une
    /// violation de sécurité (lien symbolique, mauvais propriétaire,
    /// permissions trop larges).
    pub fn load_settings(&self) -> Result<Loaded<Settings>, CoreError> {
        self.load_typed(FileKind::Settings, Settings::default, |settings| {
            settings.validate()
        })
    }

    /// Sauvegarde les réglages de façon atomique dans la racine d'état.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si les réglages sont invalides, si le magasin est en
    /// lecture seule, si la racine est sous `/etc` ou si l'écriture échoue.
    pub fn save_settings(&self, settings: &Settings) -> Result<(), CoreError> {
        settings.validate()?;
        self.save_typed(FileKind::Settings, settings)
    }

    /// Charge les profils.
    ///
    /// # Errors
    ///
    /// Voir [`ConfigStore::load_settings`].
    pub fn load_profiles(&self) -> Result<Loaded<ProfileCollection>, CoreError> {
        self.load_typed(FileKind::Profiles, ProfileCollection::default, |profiles| {
            profiles.validate()
        })
    }

    /// Charge les réglages **sans récupération ni quarantaine**.
    ///
    /// Lecture stricte destinée à la validation croisée de `tccd-admin`
    /// (A1-07 de `REVIEW_ADMIN_PHASE1`) :
    ///
    /// 1. l'état (`/var/lib`) est lu avec les mêmes garanties que
    ///    [`ConfigStore::load_settings`] : `O_NOFOLLOW|O_CLOEXEC`, fichier
    ///    régulier, propriétaire et permissions vérifiés, taille bornée par
    ///    [`LoadPolicy::max_file_bytes`] ;
    /// 2. si le fichier d'état est absent, la racine admin (`/etc`) est
    ///    consultée en lecture seule, comme pour `load_settings` ;
    /// 3. le contenu est désérialisé puis validé par [`Settings::validate`].
    ///
    /// **Aucun effet de bord disque** : un fichier absent, invalide ou trop
    /// volumineux n'est jamais renommé en `.bak` ou `.bak.corrupt`, et aucun
    /// temporaire n'est créé. Un fichier d'état corrompu **n'est pas** masqué
    /// par un repli silencieux vers la racine admin : l'erreur est renvoyée.
    ///
    /// # Errors
    ///
    /// - [`StrictLoadError::NotFound`] si ni l'état ni la racine admin ne
    ///   contient le fichier ;
    /// - [`StrictLoadError::TooLarge`] au-delà de la limite ;
    /// - [`StrictLoadError::Io`] pour une erreur d'entrée/sortie ou un refus
    ///   de sécurité (lien symbolique, type, propriétaire, permissions) ;
    /// - [`StrictLoadError::Invalid`] pour un contenu non UTF-8, un JSON
    ///   invalide ou une validation en échec.
    pub fn load_settings_no_recovery(&self) -> Result<Settings, StrictLoadError> {
        self.load_typed_no_recovery(FileKind::Settings, Settings::validate)
    }

    /// Charge les profils **sans récupération ni quarantaine**.
    ///
    /// Voir [`ConfigStore::load_settings_no_recovery`] pour les garanties.
    /// Le contenu est validé par [`ProfileCollection::validate`] ; les
    /// références croisées avec les réglages restent à la charge de
    /// l'appelant (`validate_settings_references`).
    ///
    /// Cette variante conserve le repli de lecture vers `/etc` lorsque le
    /// fichier d'état est absent. Pour le diff de tombstones de
    /// `tccd-admin`, utiliser
    /// [`ConfigStore::load_profiles_state_only_no_recovery`].
    ///
    /// # Errors
    ///
    /// Voir [`ConfigStore::load_settings_no_recovery`].
    pub fn load_profiles_no_recovery(&self) -> Result<ProfileCollection, StrictLoadError> {
        self.load_typed_no_recovery(FileKind::Profiles, ProfileCollection::validate)
    }

    /// Charge **uniquement** `profiles.json` de la racine d'état, sans repli
    /// `/etc`, sans récupération ni quarantaine.
    ///
    /// Lecture stricte destinée au diff de tombstones de `tccd-admin`
    /// (AD-R13) : la collection « précédente » doit être celle réellement
    /// présente dans `/var/lib/tuxedo-rust-center/profiles.json`, jamais les
    /// profils packagés dans `/etc` qui ne sont qu'un fallback de lecture.
    /// Comparer la nouvelle collection à ce repli ferait tombstoner des
    /// profils `/etc` qu'un client n'a jamais vus.
    ///
    /// Mêmes garanties que [`ConfigStore::load_profiles_no_recovery`] :
    /// `O_NOFOLLOW|O_CLOEXEC`, fichier régulier, propriétaire et permissions
    /// vérifiés, taille bornée par [`LoadPolicy::max_file_bytes`],
    /// désérialisation puis [`ProfileCollection::validate`]. **Aucun effet de
    /// bord disque** : aucun `.bak`, `.bak.corrupt` ou temporaire.
    ///
    /// Un fichier d'état absent renvoie explicitement
    /// [`StrictLoadError::NotFound`] — jamais une collection vide implicite.
    /// L'appelant décide alors de traiter la collection précédente comme vide
    /// (première écriture) ou de refuser selon son contexte.
    ///
    /// # Errors
    ///
    /// - [`StrictLoadError::NotFound`] si le fichier d'état des profils
    ///   n'existe pas (la racine admin n'est **pas** consultée) ;
    /// - [`StrictLoadError::TooLarge`] au-delà de la limite ;
    /// - [`StrictLoadError::Io`] pour une erreur d'entrée/sortie ou un refus
    ///   de sécurité (lien symbolique, type, propriétaire, permissions) ;
    /// - [`StrictLoadError::Invalid`] pour un contenu non UTF-8, un JSON
    ///   invalide ou une validation en échec.
    pub fn load_profiles_state_only_no_recovery(
        &self,
    ) -> Result<ProfileCollection, StrictLoadError> {
        let state_path = FileKind::Profiles.path(&self.state_paths);
        self.load_one_no_recovery(state_path, &ProfileCollection::validate)?
            .ok_or_else(|| StrictLoadError::NotFound {
                path: state_path.to_owned(),
            })
    }

    /// Sauvegarde les profils de façon atomique dans la racine d'état.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si les profils sont invalides ou si l'écriture
    /// échoue.
    pub fn save_profiles(&self, profiles: &ProfileCollection) -> Result<(), CoreError> {
        profiles.validate()?;
        self.save_typed(FileKind::Profiles, profiles)
    }

    /// Sauvegarde les réglages avec une copie `.bak` de la version précédente.
    ///
    /// Variante administrateur de [`ConfigStore::save_settings`] (AD-5) :
    ///
    /// 1. les réglages sont validés avant toute I/O ;
    /// 2. l'ancien contenu est lu avec `O_NOFOLLOW` et borné ;
    /// 3. s'il est exploitable (JSON du type attendu, validation OK), il est
    ///    écrit atomiquement dans `settings.json.bak` en `0600` ;
    /// 4. le nouveau contenu est écrit atomiquement en `0644`.
    ///
    /// Un ancien contenu corrompu ne remplace **jamais** le `.bak` existant :
    /// la dernière version saine est conservée et le fait est signalé par
    /// [`Saved::preserved_backup`] (A1-07 de `REVIEW_ADMIN_PHASE1`). La cible
    /// est remplacée dans tous les cas : la nouvelle valeur vient d'être
    /// validée.
    ///
    /// Aucun répertoire n'est créé : la racine d'état doit exister
    /// (`StateDirectory` systemd). Un contenu identique à l'existant renvoie
    /// `Saved { backup: None, preserved_backup: None }` sans réécrire les
    /// fichiers (idempotence). Une politique `expected_owner = Some(0)`
    /// (celle de [`ConfigStore::system`]) exige un uid effectif 0, afin de
    /// garantir un `.bak` root:root sans `chown`.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si les réglages sont invalides, si le magasin est en
    /// lecture seule, si aucune racine d'écriture n'est déclarée, si la cible
    /// est un lien symbolique ou un fichier non régulier, si le répertoire est
    /// absent ou si l'écriture échoue. En cas d'échec, l'ancien fichier reste
    /// intact ; un `.bak` déjà écrit peut subsister.
    pub fn save_settings_with_backup(&self, settings: &Settings) -> Result<Saved, CoreError> {
        settings.validate()?;
        self.save_typed_with_backup(FileKind::Settings, settings, Settings::validate)
    }

    /// Sauvegarde les profils avec une copie `.bak` de la version précédente.
    ///
    /// Voir [`ConfigStore::save_settings_with_backup`] pour les garanties et
    /// les cas d'erreur. Cette méthode valide la collection mais **ne vérifie
    /// pas** les références croisées avec les réglages : c'est le rôle de
    /// `tccd-admin` (`validate_settings_references`) avant de commencer la
    /// transaction.
    ///
    /// # Errors
    ///
    /// Voir [`ConfigStore::save_settings_with_backup`].
    pub fn save_profiles_with_backup(
        &self,
        profiles: &ProfileCollection,
    ) -> Result<Saved, CoreError> {
        profiles.validate()?;
        self.save_typed_with_backup(FileKind::Profiles, profiles, ProfileCollection::validate)
    }

    /// Charge les courbes de ventilation.
    ///
    /// # Errors
    ///
    /// Voir [`ConfigStore::load_settings`].
    pub fn load_fan_tables(&self) -> Result<Loaded<FanTableSet>, CoreError> {
        self.load_typed(FileKind::FanTables, FanTableSet::default, |tables| {
            tables.validate()
        })
    }

    /// Sauvegarde les courbes de ventilation de façon atomique dans la racine
    /// d'état.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si les courbes sont invalides ou si l'écriture
    /// échoue.
    pub fn save_fan_tables(&self, tables: &FanTableSet) -> Result<(), CoreError> {
        tables.validate()?;
        self.save_typed(FileKind::FanTables, tables)
    }

    fn load_typed<T, F, V>(
        &self,
        kind: FileKind,
        fallback: F,
        validate: V,
    ) -> Result<Loaded<T>, CoreError>
    where
        T: DeserializeOwned,
        F: Fn() -> T,
        V: Fn(&T) -> Result<(), ValidationError>,
    {
        let state_path = kind.path(&self.state_paths);
        let allow_state_backup = self.state_backup_allowed();
        if let Some(loaded) = self.load_one(state_path, allow_state_backup, &fallback, &validate)? {
            return Ok(loaded);
        }

        if let Some(admin) = &self.admin_paths {
            let admin_path = kind.path(admin);
            // Un fichier admin invalide n'est jamais renommé : `/etc` est en
            // lecture seule pour le daemon (D2).
            if let Some(loaded) = self.load_one(admin_path, false, &fallback, &validate)? {
                return Ok(loaded);
            }
        }

        Ok(Loaded::clean(fallback()))
    }

    /// Variante stricte de [`ConfigStore::load_typed`] : aucune valeur par
    /// défaut, aucun avertissement, aucune quarantaine.
    fn load_typed_no_recovery<T, V>(
        &self,
        kind: FileKind,
        validate: V,
    ) -> Result<T, StrictLoadError>
    where
        T: DeserializeOwned,
        V: Fn(&T) -> Result<(), ValidationError>,
    {
        let state_path = kind.path(&self.state_paths);
        if let Some(value) = self.load_one_no_recovery(state_path, &validate)? {
            return Ok(value);
        }

        if let Some(admin) = &self.admin_paths {
            let admin_path = kind.path(admin);
            // Un fichier admin corrompu n'est jamais renommé (D2) et, en
            // lecture stricte, il n'est pas non plus ignoré au profit de
            // valeurs par défaut.
            if let Some(value) = self.load_one_no_recovery(admin_path, &validate)? {
                return Ok(value);
            }
        }

        Err(StrictLoadError::NotFound {
            path: state_path.to_owned(),
        })
    }

    /// Vrai si un fichier d'état corrompu peut être déplacé en `.bak`.
    ///
    /// Refusé pour un magasin en lecture seule et pour toute racine réelle
    /// sous `/etc`, y compris via un lien symbolique : `load_settings` ne doit
    /// jamais renommer un fichier admin (D2).
    fn state_backup_allowed(&self) -> bool {
        if self.read_only || is_admin_root(&self.state_paths.root) {
            return false;
        }
        resolve_future_path(&self.state_paths.root)
            .map(|resolved| !is_admin_root(&resolved))
            .unwrap_or(false)
    }

    fn load_one<T, F, V>(
        &self,
        path: &Path,
        allow_backup: bool,
        fallback: &F,
        validate: &V,
    ) -> Result<Option<Loaded<T>>, CoreError>
    where
        T: DeserializeOwned,
        F: Fn() -> T,
        V: Fn(&T) -> Result<(), ValidationError>,
    {
        let file = match open_read_no_follow(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CoreError::io(path, error)),
        };

        let metadata = file
            .metadata()
            .map_err(|error| CoreError::io(path, error))?;
        self.verify_file(path, &metadata)?;
        if let Some(parent) = path.parent() {
            self.verify_parent_directory(parent)?;
        }

        if metadata.len() > self.policy.max_file_bytes {
            drop(file);
            return self
                .recover(
                    path,
                    allow_backup,
                    LoadWarningKind::TooLarge,
                    format!(
                        "fichier de {} octets, limite {} octets",
                        metadata.len(),
                        self.policy.max_file_bytes
                    ),
                    fallback,
                )
                .map(Some);
        }

        let mut bytes = Vec::with_capacity(metadata.len().min(self.policy.max_file_bytes) as usize);
        let read_limit = self.policy.max_file_bytes.saturating_add(1);
        if let Err(error) = file.take(read_limit).read_to_end(&mut bytes) {
            return Err(CoreError::io(path, error));
        }
        if bytes.len() as u64 > self.policy.max_file_bytes {
            return self
                .recover(
                    path,
                    allow_backup,
                    LoadWarningKind::TooLarge,
                    format!("fichier dépassant {} octets", self.policy.max_file_bytes),
                    fallback,
                )
                .map(Some);
        }

        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                let message = format!("contenu non UTF-8: {error}");
                return self
                    .recover(
                        path,
                        allow_backup,
                        LoadWarningKind::InvalidUtf8,
                        message,
                        fallback,
                    )
                    .map(Some);
            }
        };

        let parsed: T = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                return self
                    .recover(
                        path,
                        allow_backup,
                        LoadWarningKind::InvalidJson,
                        format!(
                            "ligne {}, colonne {}: {error}",
                            error.line(),
                            error.column()
                        ),
                        fallback,
                    )
                    .map(Some);
            }
        };

        if let Err(error) = validate(&parsed) {
            return self
                .recover(
                    path,
                    allow_backup,
                    LoadWarningKind::Validation,
                    error.to_string(),
                    fallback,
                )
                .map(Some);
        }

        Ok(Some(Loaded::clean(parsed)))
    }

    /// Lit, désérialise et valide un fichier sans aucune récupération.
    ///
    /// Le fichier absent renvoie `Ok(None)` pour permettre le repli vers la
    /// racine admin ; l'absence des deux racines devient
    /// [`StrictLoadError::NotFound`] dans
    /// [`ConfigStore::load_typed_no_recovery`]. Aucun chemin de cette
    /// fonction n'écrit, ne renomme ou ne supprime un fichier.
    fn load_one_no_recovery<T, V>(
        &self,
        path: &Path,
        validate: &V,
    ) -> Result<Option<T>, StrictLoadError>
    where
        T: DeserializeOwned,
        V: Fn(&T) -> Result<(), ValidationError>,
    {
        // Contrôle explicite pour produire un refus lisible ; `O_NOFOLLOW`
        // reste la barrière effective contre une course avec un renommage.
        let exists = refuse_symlink_or_non_regular(path, "fichier de configuration")
            .map_err(|error| strict_error_from_core(error, path))?;
        if !exists {
            return Ok(None);
        }

        let file = match open_read_no_follow(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(StrictLoadError::Io {
                    path: path.to_owned(),
                    source: error,
                });
            }
        };

        let metadata = file.metadata().map_err(|error| StrictLoadError::Io {
            path: path.to_owned(),
            source: error,
        })?;
        self.verify_file(path, &metadata)
            .map_err(|error| strict_error_from_core(error, path))?;
        if let Some(parent) = path.parent() {
            self.verify_parent_directory(parent)
                .map_err(|error| strict_error_from_core(error, parent))?;
        }

        if metadata.len() > self.policy.max_file_bytes {
            return Err(StrictLoadError::TooLarge {
                path: path.to_owned(),
                size: metadata.len(),
                max: self.policy.max_file_bytes,
            });
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(self.policy.max_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| StrictLoadError::Io {
                path: path.to_owned(),
                source: error,
            })?;
        if bytes.len() as u64 > self.policy.max_file_bytes {
            return Err(StrictLoadError::TooLarge {
                path: path.to_owned(),
                size: bytes.len() as u64,
                max: self.policy.max_file_bytes,
            });
        }

        let text = String::from_utf8(bytes).map_err(|error| StrictLoadError::Invalid {
            path: path.to_owned(),
            reason: format!("contenu non UTF-8: {error}"),
        })?;

        let parsed: T = serde_json::from_str(&text).map_err(|error| StrictLoadError::Invalid {
            path: path.to_owned(),
            reason: format!(
                "ligne {}, colonne {}: {error}",
                error.line(),
                error.column()
            ),
        })?;

        validate(&parsed).map_err(|error| StrictLoadError::Invalid {
            path: path.to_owned(),
            reason: error.to_string(),
        })?;

        Ok(Some(parsed))
    }

    fn save_typed<T: Serialize>(&self, kind: FileKind, value: &T) -> Result<(), CoreError> {
        if self.read_only {
            return Err(CoreError::config(format!(
                "écriture refusée: {} est en lecture seule",
                self.state_paths.root.display()
            )));
        }
        validate_state_root(&self.state_paths.root)?;

        let path = kind.path(&self.state_paths);
        let bytes = serialize_pretty(value)?;

        let parent = path.parent().ok_or_else(|| {
            CoreError::config(format!("chemin sans répertoire: {}", path.display()))
        })?;
        // Résout les composants existants avant toute création : un lien
        // symbolique intermédiaire ne doit pas faire créer `/etc` par
        // `create_dir_all`.
        let resolved_parent =
            resolve_future_path(parent).map_err(|error| CoreError::io(parent, error))?;
        if is_admin_root(&resolved_parent) {
            return Err(CoreError::config(format!(
                "écriture refusée sous /etc: {}",
                resolved_parent.display()
            )));
        }
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(CoreError::io(parent, error));
        }
        self.verify_parent_directory(parent)?;

        // Vérification finale sur le chemin réel après création.
        let canonical_parent =
            fs::canonicalize(parent).map_err(|error| CoreError::io(parent, error))?;
        if is_admin_root(&canonical_parent) {
            return Err(CoreError::config(format!(
                "écriture refusée sous /etc: {}",
                canonical_parent.display()
            )));
        }

        let temp_path = unique_temp_path(path);
        let result = write_atomic(path, parent, &temp_path, &bytes, 0o644);
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result.map_err(|error| CoreError::io(path, error))
    }

    /// Écriture atomique avec sauvegarde `.bak` (AD-5).
    ///
    /// Contrairement à [`ConfigStore::save_typed`], cette méthode :
    ///
    /// - exige une racine d'écriture déclarée et vérifie que le répertoire
    ///   cible y est contenu (en production, `/var/lib/tuxedo-rust-center`) ;
    /// - ne crée **jamais** le répertoire d'état : il appartient à
    ///   l'administration système (`StateDirectory`) ;
    /// - refuse un lien symbolique ou un fichier non régulier sur la cible,
    ///   et sur la sauvegarde dès qu'elle existe, qu'elle soit réécrite ou
    ///   volontairement conservée ;
    /// - lit l'ancienne version avec `O_NOFOLLOW` et la borne à
    ///   `policy.max_file_bytes` ;
    /// - ne remplace le `.bak` que si l'ancienne version est exploitable au
    ///   sens de `validate` (même type, mêmes règles que la lecture) ; sinon
    ///   la sauvegarde existante est conservée et signalée dans
    ///   [`Saved::preserved_backup`].
    fn save_typed_with_backup<T, V>(
        &self,
        kind: FileKind,
        value: &T,
        validate: V,
    ) -> Result<Saved, CoreError>
    where
        T: Serialize + DeserializeOwned,
        V: Fn(&T) -> Result<(), ValidationError>,
    {
        if self.read_only {
            return Err(CoreError::config(format!(
                "écriture refusée: {} est en lecture seule",
                self.state_paths.root.display()
            )));
        }
        self.verify_writer_identity()?;
        validate_state_root(&self.state_paths.root)?;

        let path = kind.path(&self.state_paths);
        let parent = path.parent().ok_or_else(|| {
            CoreError::config(format!("chemin sans répertoire: {}", path.display()))
        })?;
        self.verify_write_scope(parent)?;

        // AD-5 : le répertoire est créé par systemd (`StateDirectory`), jamais
        // par l'admin. On refuse explicitement plutôt que de créer un chemin
        // privilégié.
        match fs::symlink_metadata(parent) {
            Ok(_) => self.verify_parent_directory(parent)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(CoreError::config(format!(
                    "répertoire d'état absent: {} (créé par l'administration système)",
                    parent.display()
                )));
            }
            Err(error) => return Err(CoreError::io(parent, error)),
        }

        let bytes = serialize_pretty(value)?;

        let target_exists = refuse_symlink_or_non_regular(path, "fichier cible")?;
        let previous = if target_exists {
            self.read_current_bounded(path)?
        } else {
            None
        };

        // Idempotence : réécrire un contenu identique ne doit ni modifier le
        // fichier ni remplacer une sauvegarde existante.
        if previous.as_deref() == Some(bytes.as_slice()) {
            return Ok(Saved {
                backup: None,
                preserved_backup: None,
            });
        }

        let backup_path = backup_path(path);
        let mut backup = None;
        let mut preserved_backup = None;
        if let Some(old_bytes) = previous {
            // La sauvegarde ne doit pas écraser un lien symbolique ou un
            // répertoire : on la refuse avant toute écriture, qu'elle soit
            // remplacée ou volontairement conservée.
            let backup_exists = refuse_symlink_or_non_regular(&backup_path, "sauvegarde")?;
            if is_usable_backup(&old_bytes, &validate) {
                let temp_path = unique_temp_path(&backup_path);
                let result = write_atomic(&backup_path, parent, &temp_path, &old_bytes, 0o600);
                if result.is_err() {
                    let _ = fs::remove_file(&temp_path);
                }
                result.map_err(|error| CoreError::io(&backup_path, error))?;
                backup = Some(backup_path);
            } else if backup_exists {
                // A1-07 : ne jamais écraser la dernière version saine par un
                // contenu corrompu. La cible est quand même remplacée par la
                // nouvelle valeur validée.
                preserved_backup = Some(backup_path);
            }
        }

        let temp_path = unique_temp_path(path);
        let result = write_atomic(path, parent, &temp_path, &bytes, 0o644);
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result.map_err(|error| CoreError::io(path, error))?;
        Ok(Saved {
            backup,
            preserved_backup,
        })
    }

    /// Vérifie que le répertoire cible est bien sous la racine d'écriture
    /// déclarée et jamais sous `/etc`.
    fn verify_write_scope(&self, parent: &Path) -> Result<(), CoreError> {
        let guard = self.write_root.as_deref().ok_or_else(|| {
            CoreError::config("écriture avec sauvegarde refusée: aucune racine d'écriture déclarée")
        })?;
        let resolved_parent =
            resolve_future_path(parent).map_err(|error| CoreError::io(parent, error))?;
        let resolved_guard =
            resolve_future_path(guard).map_err(|error| CoreError::io(guard, error))?;

        if is_admin_root(&resolved_parent) || is_admin_root(&resolved_guard) {
            return Err(CoreError::config(format!(
                "écriture refusée sous /etc: {}",
                resolved_parent.display()
            )));
        }
        if !resolved_parent.starts_with(&resolved_guard) {
            return Err(CoreError::config(format!(
                "écriture refusée hors de la racine d'état {}: {}",
                resolved_guard.display(),
                resolved_parent.display()
            )));
        }
        Ok(())
    }

    /// Vérifie que le processus peut garantir le propriétaire root des
    /// fichiers créés.
    ///
    /// Une politique `expected_owner = Some(0)` (celle de
    /// [`ConfigStore::system`]) impose un uid effectif 0 : sans `chown`, un
    /// processus non root ne peut pas produire un `.bak` root:root, et le
    /// crate n'utilise pas `unsafe`/`libc` pour élever ses privilèges.
    #[cfg(target_os = "linux")]
    fn verify_writer_identity(&self) -> Result<(), CoreError> {
        if self.policy.expected_owner != Some(0) {
            return Ok(());
        }
        let status = fs::read_to_string("/proc/self/status")
            .map_err(|error| CoreError::io("/proc/self/status", error))?;
        let uid = parse_effective_uid(&status).ok_or_else(|| {
            CoreError::config("impossible de lire l'uid effectif depuis /proc/self/status")
        })?;
        if uid != 0 {
            return Err(CoreError::config(
                "écriture avec sauvegarde refusée: propriétaire attendu 0, processus non root",
            ));
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn verify_writer_identity(&self) -> Result<(), CoreError> {
        Ok(())
    }

    /// Lit le fichier existant avec les contrôles de sécurité de lecture.
    ///
    /// Un fichier absent renvoie `None` ; un lien symbolique est refusé par
    /// `O_NOFOLLOW` et remonte en erreur d'entrée/sortie.
    fn read_current_bounded(&self, path: &Path) -> Result<Option<Vec<u8>>, CoreError> {
        let file = match open_read_no_follow(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CoreError::io(path, error)),
        };
        let metadata = file
            .metadata()
            .map_err(|error| CoreError::io(path, error))?;
        self.verify_file(path, &metadata)?;
        if metadata.len() > self.policy.max_file_bytes {
            return Err(CoreError::config(format!(
                "{}: fichier existant de {} octets, limite {} octets",
                path.display(),
                metadata.len(),
                self.policy.max_file_bytes
            )));
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(self.policy.max_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| CoreError::io(path, error))?;
        if bytes.len() as u64 > self.policy.max_file_bytes {
            return Err(CoreError::config(format!(
                "{}: fichier dépassant {} octets",
                path.display(),
                self.policy.max_file_bytes
            )));
        }
        Ok(Some(bytes))
    }

    fn recover<T, F>(
        &self,
        path: &Path,
        allow_backup: bool,
        kind: LoadWarningKind,
        message: String,
        fallback: &F,
    ) -> Result<Loaded<T>, CoreError>
    where
        F: Fn() -> T,
    {
        let backup_path = if allow_backup && self.policy.backup_corrupt {
            Some(self.backup(path)?)
        } else {
            None
        };
        Ok(Loaded {
            value: fallback(),
            warning: Some(LoadWarning {
                path: path.to_owned(),
                kind,
                message,
                backup_path,
            }),
        })
    }

    /// Déplace un fichier illisible pour repartir des valeurs par défaut.
    ///
    /// Le renommage ne suit jamais un lien symbolique. Un `.bak` **régulier**
    /// existant n'est jamais écrasé : la dernière version saine serait
    /// perdue (A1-07). Le contenu corrompu va alors dans `.bak.corrupt`
    /// (nom fixe, une seule génération : l'espace disque reste borné et
    /// aucune horloge n'est nécessaire). Un `.bak` symbolique ou non régulier
    /// n'est pas une sauvegarde : il est remplacé par le contenu illisible,
    /// sans jamais suivre le lien. La cible absente n'est jamais recréée.
    fn backup(&self, path: &Path) -> Result<PathBuf, CoreError> {
        let intended = backup_path(path);
        let target = match fs::symlink_metadata(&intended) {
            Ok(metadata) if metadata.is_file() => quarantine_path(path),
            Ok(_) => intended,
            Err(error) if error.kind() == io::ErrorKind::NotFound => intended,
            Err(error) => return Err(CoreError::io(&intended, error)),
        };
        fs::rename(path, &target).map_err(|error| CoreError::io(path, error))?;
        // D2 : la sauvegarde peut contenir des données sensibles (profils,
        // chemins), elle est root-only.
        set_mode(&target, 0o600).map_err(|error| CoreError::io(&target, error))?;
        Ok(target)
    }

    fn verify_file(&self, path: &Path, metadata: &fs::Metadata) -> Result<(), CoreError> {
        if !metadata.is_file() {
            return Err(CoreError::config(format!(
                "{} n'est pas un fichier régulier",
                path.display()
            )));
        }
        verify_metadata_security(path, metadata, &self.policy)
    }

    fn verify_parent_directory(&self, parent: &Path) -> Result<(), CoreError> {
        let metadata =
            fs::symlink_metadata(parent).map_err(|error| CoreError::io(parent, error))?;
        if metadata.file_type().is_symlink() {
            return Err(CoreError::config(format!(
                "répertoire de configuration symbolique refusé: {}",
                parent.display()
            )));
        }
        if !metadata.is_dir() {
            return Err(CoreError::config(format!(
                "{} n'est pas un répertoire",
                parent.display()
            )));
        }
        verify_metadata_security(parent, &metadata, &self.policy)
    }
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::state_default()
    }
}

fn verify_metadata_security(
    path: &Path,
    metadata: &fs::Metadata,
    policy: &LoadPolicy,
) -> Result<(), CoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if let Some(expected) = policy.expected_owner
            && metadata.uid() != expected
        {
            return Err(CoreError::config(format!(
                "{} appartient à l'uid {}, attendu {}",
                path.display(),
                metadata.uid(),
                expected
            )));
        }
        if policy.enforce_permissions && metadata.mode() & 0o022 != 0 {
            return Err(CoreError::config(format!(
                "{} est accessible en écriture au groupe ou aux autres (mode {:o})",
                path.display(),
                metadata.mode() & 0o7777
            )));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, metadata, policy);
    }
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".bak");
    PathBuf::from(value)
}

/// Chemin de quarantaine d'un fichier corrompu lorsqu'un `.bak` existe déjà.
///
/// Nom fixe, une seule génération : une corruption répétée remplace la
/// précédente preuve au lieu d'accumuler des fichiers dans `/var/lib`.
fn quarantine_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".bak.corrupt");
    PathBuf::from(value)
}

/// Vrai si un contenu remplacé peut servir de sauvegarde exploitable.
///
/// Les octets proviennent de [`ConfigStore::read_current_bounded`] : ils sont
/// donc déjà plafonnés par `policy.max_file_bytes` et contrôlés (fichier
/// régulier, propriétaire attendu, permissions sans écriture
/// groupe/autres). Le `.bak` n'est remplacé que par un contenu qu'un futur
/// chargement accepterait — même type, même validation — afin de ne jamais
/// écraser la dernière version saine par un fichier corrompu ou incompatible.
fn is_usable_backup<T, V>(bytes: &[u8], validate: &V) -> bool
where
    T: DeserializeOwned,
    V: Fn(&T) -> Result<(), ValidationError>,
{
    serde_json::from_slice::<T>(bytes).is_ok_and(|value| validate(&value).is_ok())
}

/// Sérialise une valeur comme les fichiers de configuration : JSON indenté
/// suivi d'un saut de ligne final.
fn serialize_pretty<T: Serialize>(value: &T) -> Result<Vec<u8>, CoreError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Refuse un lien symbolique ou un type de fichier non régulier.
///
/// Retourne `true` si le chemin existe et est un fichier régulier, `false` si
/// le chemin est absent, et une erreur de configuration sinon. Le contrôle
/// utilise `symlink_metadata` (aucun suivi de lien) pour donner un message
/// explicite ; `open_read_no_follow` reste la barrière effective contre une
/// course entre ce contrôle et l'ouverture.
fn refuse_symlink_or_non_regular(path: &Path, label: &str) -> Result<bool, CoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(CoreError::config(format!(
                    "{label} symbolique refusé: {}",
                    path.display()
                )));
            }
            if !metadata.is_file() {
                return Err(CoreError::config(format!(
                    "{label} non régulier refusé: {}",
                    path.display()
                )));
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CoreError::io(path, error)),
    }
}

/// Convertit une erreur de contrôle de [`ConfigStore`] en erreur de lecture
/// stricte, sans jamais exposer de contenu de fichier.
fn strict_error_from_core(error: CoreError, path: &Path) -> StrictLoadError {
    match error {
        CoreError::Io { path, source } => StrictLoadError::Io { path, source },
        // Les refus de propriétaire/permissions/type sont des erreurs de
        // permission du point de vue de la lecture stricte.
        CoreError::Config(reason) => StrictLoadError::Io {
            path: path.to_owned(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, reason),
        },
        other => StrictLoadError::Invalid {
            path: path.to_owned(),
            reason: other.to_string(),
        },
    }
}

/// Lit un fichier sans suivre un lien symbolique final et borne sa taille.
///
/// Helper interne partagé avec la migration des fichiers legacy. Il ne vérifie
/// pas le propriétaire : `/etc/tcc` appartient à root et la lecture est non
/// privilégiée côté données. L'écriture, elle, reste protégée par
/// [`ConfigStore`].
pub(crate) fn read_file_bounded_no_follow(
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, CoreError> {
    let file = open_read_no_follow(path).map_err(|error| CoreError::io(path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| CoreError::io(path, error))?;
    if !metadata.is_file() {
        return Err(CoreError::config(format!(
            "{} n'est pas un fichier régulier",
            path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(CoreError::config(format!(
            "{}: fichier de {} octets, limite {max_bytes} octets",
            path.display(),
            metadata.len()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| CoreError::io(path, error))?;
    if bytes.len() as u64 > max_bytes {
        return Err(CoreError::config(format!(
            "{}: fichier dépassant {max_bytes} octets",
            path.display()
        )));
    }
    Ok(bytes)
}

/// Type de fichier géré par le magasin.
#[derive(Debug, Clone, Copy)]
enum FileKind {
    Settings,
    Profiles,
    FanTables,
}

impl FileKind {
    fn path(self, paths: &ConfigPaths) -> &Path {
        match self {
            Self::Settings => &paths.settings,
            Self::Profiles => &paths.profiles,
            Self::FanTables => &paths.fan_tables,
        }
    }
}

/// Vrai si le chemin est (ou est contenu dans) la configuration admin.
fn is_admin_root(path: &Path) -> bool {
    path == Path::new(ADMIN_CONFIG_DIR)
        || path.starts_with(ADMIN_CONFIG_DIR)
        || path.starts_with("/etc")
}

/// Vérifie qu'une racine d'état peut être écrite sans jamais toucher `/etc`.
///
/// Rejette les chemins relatifs et les composants `..` : ils pourraient
/// désigner `/etc` après normalisation, avant même que le répertoire n'existe.
fn validate_state_root(root: &Path) -> Result<(), CoreError> {
    if !root.is_absolute() {
        return Err(CoreError::config(format!(
            "racine d'état relative refusée: {}",
            root.display()
        )));
    }
    if root.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err(CoreError::config(format!(
            "racine d'état avec composant relatif refusée: {}",
            root.display()
        )));
    }
    if is_admin_root(root) {
        return Err(CoreError::config(format!(
            "écriture refusée sous /etc: {} (D2 : /etc est en lecture seule pour le daemon)",
            root.display()
        )));
    }
    Ok(())
}

/// Résout le plus proche ancêtre existant puis réapplique les composants
/// manquants, sans créer de fichier.
///
/// Permet de vérifier qu'un chemin qui n'existe pas encore (par exemple
/// `/var/lib/tuxedo-rust-center` au premier démarrage) ne traverse pas un lien
/// symbolique vers `/etc`.
fn resolve_future_path(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match fs::canonicalize(&current) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = current.file_name().ok_or(error)?.to_owned();
                missing.push(name);
                if !current.pop() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "aucun ancêtre existant",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().map_or_else(
        || "config".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        counter
    ))
}

#[cfg(unix)]
fn open_read_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(not(unix))]
fn open_read_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

fn write_atomic(
    path: &Path,
    parent: &Path,
    temp_path: &Path,
    bytes: &[u8],
    mode: u32,
) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(temp_path)?;
    file.write_all(bytes)?;
    // fchmod explicite : le mode de création ne corrige pas un fichier
    // existant et ne doit pas dépendre de l'umask pour la cible finale.
    // `0644` pour un fichier d'état, `0600` pour une sauvegarde.
    set_file_mode(&file, mode)?;
    file.sync_all()?;
    drop(file);

    fs::rename(temp_path, path)?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn set_file_mode(file: &File, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_file_mode(_file: &File, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Racine d'état fournie par `TCC_CONFIG_DIR`, si la feature `dev-overrides`
/// est active.
///
/// Refusée si le processus tourne en root (D7) : une variable d'environnement
/// ne doit jamais rediriger les écritures d'un daemon privilégié. Le daemon
/// reste seul responsable de l'appel et de la journalisation.
///
/// # Errors
///
/// Renvoie une erreur si la feature est active, que la variable est définie et
/// que le chemin est relatif, sous `/etc`, que le processus est root ou que la
/// détection d'identité échoue.
#[cfg(feature = "dev-overrides")]
pub fn dev_state_root() -> Result<Option<PathBuf>, CoreError> {
    let Some(value) = std::env::var_os("TCC_CONFIG_DIR") else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(CoreError::config("TCC_CONFIG_DIR est vide"));
    }

    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(CoreError::config(
            "TCC_CONFIG_DIR doit être un chemin absolu",
        ));
    }
    if is_admin_root(&path) {
        return Err(CoreError::config(
            "TCC_CONFIG_DIR ne peut pas désigner /etc (D2)",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        // `Uid:` de `/proc/self/status` contient l'uid réel, effectif, sauvé
        // et fs. On lit l'effectif sans recourir à `unsafe`/geteuid.
        let status = fs::read_to_string("/proc/self/status")
            .map_err(|error| CoreError::io("/proc/self/status", error))?;
        let uid = parse_effective_uid(&status).ok_or_else(|| {
            CoreError::config("impossible de lire l'uid effectif depuis /proc/self/status")
        })?;
        if uid == 0 {
            return Err(CoreError::config(
                "TCC_CONFIG_DIR est interdit lorsque le processus tourne en root",
            ));
        }
        Ok(Some(path))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(CoreError::config(
            "TCC_CONFIG_DIR n'est pas supporté sur cette plateforme",
        ))
    }
}

/// Extrait l'uid effectif d'un contenu de `/proc/self/status`.
///
/// Ligne attendue : `Uid:\t<real>\t<effective>\t<saved>\t<fs>`.
#[cfg(any(test, feature = "dev-overrides", target_os = "linux"))]
fn parse_effective_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    let mut fields = line.split_whitespace();
    let _label = fields.next()?;
    let _real = fields.next()?;
    fields.next()?.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{ConfigPaths, ConfigStore, DEFAULT_MAX_CONFIG_BYTES, LoadPolicy, LoadWarningKind};
    use crate::error::StrictLoadError;
    use crate::fan::{FanCurve, FanTableEntry, FanTableSet};
    use crate::profile::{Profile, ProfileCollection};
    use crate::settings::Settings;

    fn store(temp: &TempDir) -> ConfigStore {
        ConfigStore::from_root(temp.path())
    }

    fn write(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
    }

    /// Photographie complète d'un répertoire de test : noms, taille, mode et
    /// contenu. Sert à prouver qu'une lecture n'a rien créé ni modifié.
    fn snapshot_directory(root: &Path) -> Vec<(String, u64, u32, Vec<u8>)> {
        let mut entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                let bytes = if metadata.is_file() {
                    fs::read(entry.path()).unwrap()
                } else {
                    Vec::new()
                };
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    metadata.len(),
                    metadata.mode() & 0o777,
                    bytes,
                )
            })
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn missing_files_return_defaults_without_warning() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);

        let settings = store.load_settings().unwrap();
        assert_eq!(settings.value, Settings::default());
        assert!(settings.warning().is_none());

        let profiles = store.load_profiles().unwrap();
        assert!(profiles.value.is_empty());
        assert!(profiles.warning().is_none());

        let tables = store.load_fan_tables().unwrap();
        assert!(tables.value.is_empty());
        assert!(tables.warning().is_none());
    }

    #[test]
    fn settings_roundtrip_and_permissions() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let settings = Settings {
            fahrenheit: true,
            shutdown_time: Some("22:30".to_owned()),
            ..Settings::default()
        };

        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.value, settings);
        assert!(loaded.warning().is_none());

        let mode = fs::metadata(&store.paths().settings).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o644);

        // Aucun fichier temporaire ne doit rester.
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temporaires restants: {leftovers:?}");
    }

    #[test]
    fn profiles_and_fan_tables_roundtrip() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);

        let profiles = ProfileCollection::new(vec![
            Profile::new("office", "Office"),
            Profile::new("quiet", "Quiet"),
        ]);
        store.save_profiles(&profiles).unwrap();
        assert_eq!(store.load_profiles().unwrap().value, profiles);

        let mut tables = FanTableSet::default();
        tables.upsert(FanCurve {
            name: Some("Silent".into()),
            table_cpu: vec![FanTableEntry::new(20.0, 10.0)],
            table_gpu: vec![FanTableEntry::new(20.0, 10.0)],
        });
        store.save_fan_tables(&tables).unwrap();
        assert_eq!(store.load_fan_tables().unwrap().value, tables);
    }

    #[test]
    fn corrupt_json_is_backed_up_and_defaults_are_returned() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        write(&store.paths().settings, b"{ not json");

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.value, Settings::default());
        let warning = loaded.warning().unwrap();
        assert_eq!(warning.kind, LoadWarningKind::InvalidJson);
        let backup = warning.backup_path.as_ref().unwrap();
        assert_eq!(fs::read(backup).unwrap(), b"{ not json");
        assert!(!store.paths().settings.exists());
        assert_eq!(fs::metadata(backup).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn invalid_utf8_is_backed_up() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        write(&store.paths().profiles, &[0xff, 0xfe, 0xfd]);

        let loaded = store.load_profiles().unwrap();
        assert!(loaded.value.is_empty());
        assert_eq!(loaded.warning().unwrap().kind, LoadWarningKind::InvalidUtf8);
    }

    #[test]
    fn validation_failure_is_backed_up() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let settings = Settings {
            schema_version: 0,
            ..Settings::default()
        };
        let json = serde_json::to_vec(&settings).unwrap();
        write(&store.paths().settings, &json);

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.value, Settings::default());
        let warning = loaded.warning().unwrap();
        assert_eq!(warning.kind, LoadWarningKind::Validation);
        assert!(warning.message.contains("schemaVersion"));
    }

    #[test]
    fn oversized_file_is_rejected_without_reading_it_whole() {
        let temp = TempDir::new().unwrap();
        let mut store = store(&temp);
        store = store.with_policy(LoadPolicy {
            max_file_bytes: 16,
            ..LoadPolicy::default()
        });
        write(&store.paths().settings, br#"{"fahrenheit":true}"#);
        assert!(fs::metadata(&store.paths().settings).unwrap().len() > 16);

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.value, Settings::default());
        assert_eq!(loaded.warning().unwrap().kind, LoadWarningKind::TooLarge);
    }

    #[test]
    fn save_refuses_invalid_data_and_keeps_previous_file() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let before = fs::read(&store.paths().settings).unwrap();

        let invalid =
            ProfileCollection::new(vec![Profile::new("a", "Same"), Profile::new("b", "Same")]);
        assert!(store.save_profiles(&invalid).is_err());

        assert_eq!(fs::read(&store.paths().settings).unwrap(), before);
        assert!(!store.paths().profiles.exists());
    }

    #[test]
    fn read_only_store_refuses_writes_but_allows_reads() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.save_settings(&Settings::default()).unwrap();

        let admin = ConfigStore::new(ConfigPaths::new(temp.path())).read_only();
        assert!(!admin.load_settings().unwrap().has_warning());
        assert!(admin.save_settings(&Settings::default()).is_err());
    }

    #[test]
    fn world_writable_directory_is_refused() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("insecure");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        let store = ConfigStore::from_root(&root);

        let error = store.save_settings(&Settings::default()).unwrap_err();
        assert!(error.to_string().contains("accessible en écriture"));
    }

    #[test]
    fn unexpected_owner_is_refused() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.save_settings(&Settings::default()).unwrap();
        let uid = fs::metadata(&store.paths().settings).unwrap().uid();

        let strict = store.with_policy(LoadPolicy {
            expected_owner: Some(uid.wrapping_add(1)),
            ..LoadPolicy::default()
        });
        let error = strict.load_settings().unwrap_err();
        assert!(error.to_string().contains("appartient à l'uid"));
    }

    #[test]
    fn symlink_target_is_never_followed_on_read() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let victim = temp.path().join("victim");
        write(&victim, b"do not touch");
        std::os::unix::fs::symlink(&victim, &store.paths().settings).unwrap();

        let error = store.load_settings().unwrap_err();
        assert!(matches!(error, crate::CoreError::Io { .. }));
        assert_eq!(fs::read(&victim).unwrap(), b"do not touch");
    }

    #[test]
    fn save_replaces_symlink_without_touching_target() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let victim = temp.path().join("victim");
        write(&victim, b"do not touch");
        std::os::unix::fs::symlink(&victim, &store.paths().settings).unwrap();

        store.save_settings(&Settings::default()).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"do not touch");
        assert!(
            !fs::symlink_metadata(&store.paths().settings)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn corrupt_file_can_be_replaced_after_recovery() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        write(&store.paths().settings, b"{");
        let warning = store.load_settings().unwrap().warning.unwrap();
        assert!(warning.backup_path.unwrap().exists());

        store.save_settings(&Settings::default()).unwrap();
        assert!(!store.load_settings().unwrap().has_warning());
    }

    #[test]
    fn default_policy_is_bounded() {
        let policy = LoadPolicy::default();
        assert_eq!(policy.max_file_bytes, DEFAULT_MAX_CONFIG_BYTES);
        assert!(policy.backup_corrupt);
        assert!(policy.enforce_permissions);
        assert_eq!(policy.expected_owner, None);
    }

    #[test]
    fn read_only_store_never_renames_corrupt_files() {
        let temp = TempDir::new().unwrap();
        let store = ConfigStore::new(ConfigPaths::new(temp.path())).read_only();
        write(&store.paths().settings, b"{ broken");

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.warning().unwrap().kind, LoadWarningKind::InvalidJson);
        assert!(loaded.warning().unwrap().backup_path.is_none());
        assert!(store.paths().settings.exists());
        assert!(!super::backup_path(&store.paths().settings).exists());
    }

    #[test]
    fn paths_are_constant_and_rooted() {
        let paths = ConfigPaths::new("/tmp/tcc");
        assert_eq!(paths.settings, Path::new("/tmp/tcc/settings.json"));
        assert_eq!(paths.profiles, Path::new("/tmp/tcc/profiles.json"));
        assert_eq!(paths.fan_tables, Path::new("/tmp/tcc/fantables.json"));
    }

    #[test]
    fn admin_root_is_read_as_fallback_and_never_written() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let packaged = Settings {
            fahrenheit: true,
            ..Settings::default()
        };
        let admin_file = admin.path().join(super::SETTINGS_FILE);
        write(&admin_file, &serde_json::to_vec(&packaged).unwrap());

        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        let loaded = store.load_settings().unwrap();
        assert!(loaded.value.fahrenheit);
        assert_eq!(loaded.value, packaged);

        store.save_settings(&Settings::default()).unwrap();
        assert!(store.paths().settings.exists());
        let reloaded: Settings = serde_json::from_slice(&fs::read(&admin_file).unwrap()).unwrap();
        assert!(reloaded.fahrenheit, "le fichier admin a été écrasé");
    }

    #[test]
    fn corrupt_admin_file_is_not_renamed() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let admin_file = admin.path().join(super::SETTINGS_FILE);
        write(&admin_file, b"{ broken");

        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        let loaded = store.load_settings().unwrap();
        let warning = loaded.warning().unwrap();
        assert_eq!(warning.kind, LoadWarningKind::InvalidJson);
        assert!(warning.backup_path.is_none());
        assert!(admin_file.exists(), "le fichier admin a été déplacé");
    }

    #[test]
    fn writes_under_etc_are_refused_before_any_io() {
        let target = Path::new("/etc/tuxedo-rust-center-test-does-not-exist");
        let store = ConfigStore::from_root(target);
        let error = store.save_settings(&Settings::default()).unwrap_err();
        assert!(error.to_string().contains("/etc"));
        assert!(!target.exists());

        let traversal = ConfigStore::from_root("/tmp/../etc/tuxedo-rust-center-test");
        assert!(traversal.save_settings(&Settings::default()).is_err());
    }

    #[test]
    fn production_roots_follow_d2() {
        let store = ConfigStore::system();
        assert_eq!(store.paths().root, Path::new(super::STATE_DIR));
        assert_eq!(
            store.admin_paths().unwrap().root,
            Path::new(super::ADMIN_CONFIG_DIR)
        );
        assert_eq!(store.policy().expected_owner, Some(0));
    }

    #[test]
    fn parses_effective_uid_from_proc_status() {
        let status = "Name:\ttccd\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(super::parse_effective_uid(status), Some(1000));

        let root = "Uid:\t0\t0\t0\t0\n";
        assert_eq!(super::parse_effective_uid(root), Some(0));

        assert_eq!(super::parse_effective_uid("Name:\tx\n"), None);
        assert_eq!(super::parse_effective_uid("Uid:\t1\tx\n"), None);
    }

    #[test]
    fn backup_save_requires_declared_write_root() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("racine d'écriture"));
        assert!(!store.paths().settings.exists());
    }

    #[test]
    fn backup_save_refuses_state_root_outside_write_root() {
        let temp = TempDir::new().unwrap();
        // Racine d'écriture de production : le magasin injecté sous /tmp n'y
        // est pas contenu et doit être refusé.
        let store = ConfigStore::from_root(temp.path()).with_write_root(super::STATE_DIR);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains(super::STATE_DIR));
        assert!(!store.paths().settings.exists());
    }

    #[test]
    fn backup_save_writes_0644_and_previous_content_0600() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());

        let mut settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let previous = fs::read(&store.paths().settings).unwrap();
        let backup = super::backup_path(&store.paths().settings);
        assert!(!backup.exists());

        settings.fahrenheit = true;
        let saved = store.save_settings_with_backup(&settings).unwrap();
        assert_eq!(saved.backup.as_deref(), Some(backup.as_path()));
        assert!(saved.preserved_backup.is_none());
        assert_eq!(fs::read(&backup).unwrap(), previous);
        assert_eq!(fs::metadata(&backup).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&store.paths().settings).unwrap().mode() & 0o777,
            0o644
        );
        assert_eq!(store.load_settings().unwrap().value, settings);

        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temporaires restants: {leftovers:?}");
    }

    #[test]
    fn backup_save_replaces_existing_backup_with_valid_previous() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);

        // `.bak` d'une génération antérieure, volontairement différent.
        let stale = Settings {
            fahrenheit: true,
            ..Settings::default()
        };
        write(&backup, &serde_json::to_vec_pretty(&stale).unwrap());

        let previous = Settings {
            shutdown_time: Some("21:00".to_owned()),
            ..Settings::default()
        };
        store.save_settings(&previous).unwrap();
        let previous_bytes = fs::read(&store.paths().settings).unwrap();

        let next = Settings {
            shutdown_time: Some("22:00".to_owned()),
            ..Settings::default()
        };
        let saved = store.save_settings_with_backup(&next).unwrap();

        assert_eq!(saved.backup.as_deref(), Some(backup.as_path()));
        assert!(saved.preserved_backup.is_none());
        assert_eq!(fs::read(&backup).unwrap(), previous_bytes);
        assert_eq!(store.load_settings().unwrap().value, next);
    }

    #[test]
    fn backup_save_profiles_preserves_previous_collection() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());

        let first = ProfileCollection::new(vec![Profile::new("office", "Office")]);
        store.save_profiles(&first).unwrap();
        let previous = fs::read(&store.paths().profiles).unwrap();

        let second = ProfileCollection::new(vec![
            Profile::new("office", "Office"),
            Profile::new("quiet", "Quiet"),
        ]);
        let saved = store.save_profiles_with_backup(&second).unwrap();
        let backup = saved.backup.unwrap();
        assert_eq!(fs::read(&backup).unwrap(), previous);
        assert_eq!(store.load_profiles().unwrap().value, second);
        assert_eq!(fs::metadata(&backup).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn backup_save_is_idempotent_for_identical_content() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());

        let profiles = ProfileCollection::new(vec![Profile::new("office", "Office")]);
        let first = store.save_profiles_with_backup(&profiles).unwrap();
        assert!(first.backup.is_none());
        assert!(first.preserved_backup.is_none());
        let bytes = fs::read(&store.paths().profiles).unwrap();
        let backup = super::backup_path(&store.paths().profiles);
        assert!(!backup.exists());

        let second = store.save_profiles_with_backup(&profiles).unwrap();
        assert!(second.backup.is_none());
        assert!(second.preserved_backup.is_none());
        assert_eq!(fs::read(&store.paths().profiles).unwrap(), bytes);
        assert!(!backup.exists());
    }

    #[test]
    fn backup_save_refuses_symlink_target() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let victim = temp.path().join("victim");
        write(&victim, b"do not touch");
        std::os::unix::fs::symlink(&victim, &store.paths().settings).unwrap();

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("symbolique"));
        assert_eq!(fs::read(&victim).unwrap(), b"do not touch");
        assert!(!super::backup_path(&store.paths().settings).exists());
    }

    #[test]
    fn backup_save_refuses_symlinked_state_root_to_etc() {
        let temp = TempDir::new().unwrap();
        let link = temp.path().join("state");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        let store = ConfigStore::from_root(&link).with_write_root(&link);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("/etc"));
    }

    #[test]
    fn backup_save_refuses_non_regular_target() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        fs::create_dir(&store.paths().settings).unwrap();

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("non régulier"));
    }

    #[test]
    fn backup_save_failure_keeps_previous_file() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        store.save_settings(&Settings::default()).unwrap();
        let before = fs::read(&store.paths().settings).unwrap();

        // Une sauvegarde non régulière fait échouer la transaction avant
        // l'écriture de la cible.
        fs::create_dir(super::backup_path(&store.paths().settings)).unwrap();
        let error = store
            .save_settings_with_backup(&Settings {
                fahrenheit: true,
                ..Settings::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("non régulier"));
        assert_eq!(fs::read(&store.paths().settings).unwrap(), before);
    }

    #[test]
    fn backup_save_io_failure_keeps_previous_file() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        store.save_settings(&Settings::default()).unwrap();
        let before = fs::read(&store.paths().settings).unwrap();

        // Création du temporaire impossible : la transaction échoue avant le
        // `rename`, donc l'ancien fichier reste intact. En root, le mode du
        // répertoire ne bloque pas l'écriture : on ne peut pas simuler
        // l'échec, le test se contente alors d'un résultat cohérent.
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = store.save_settings_with_backup(&Settings {
            fahrenheit: true,
            ..Settings::default()
        });
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();

        if result.is_err() {
            assert_eq!(fs::read(&store.paths().settings).unwrap(), before);
        } else {
            // Root : l'écriture a réussi, la cible est nécessairement valide.
            assert!(store.load_settings().unwrap().value.fahrenheit);
        }
    }

    #[test]
    fn backup_save_does_not_create_missing_directory() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("missing");
        let store = ConfigStore::from_root(&root).with_write_root(&root);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("répertoire d'état absent"));
        assert!(!root.exists());
    }

    #[test]
    fn backup_save_never_writes_under_etc() {
        let target = Path::new("/etc/tuxedo-rust-center-test-backup");
        let store = ConfigStore::from_root(target).with_write_root(target);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("/etc"));
        assert!(!target.exists());
    }

    #[test]
    fn backup_save_refuses_read_only_store() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path()).read_only();

        let error = store
            .save_profiles_with_backup(&ProfileCollection::default())
            .unwrap_err();
        assert!(error.to_string().contains("lecture seule"));
    }

    #[test]
    fn backup_save_requires_root_writer_for_root_policy() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp)
            .with_write_root(temp.path())
            .with_policy(LoadPolicy {
                expected_owner: Some(0),
                ..LoadPolicy::default()
            });

        let status = fs::read_to_string("/proc/self/status").unwrap();
        let uid = super::parse_effective_uid(&status).unwrap();
        let result = store.save_settings_with_backup(&Settings::default());
        if uid == 0 {
            assert!(result.is_ok());
        } else {
            let error = result.unwrap_err();
            assert!(error.to_string().contains("non root"));
            assert!(!store.paths().settings.exists());
        }
    }

    /// Réglages valides sérialisés comme le fait `ConfigStore`.
    fn healthy_settings_bytes() -> Vec<u8> {
        serde_json::to_vec_pretty(&Settings {
            fahrenheit: true,
            ..Settings::default()
        })
        .unwrap()
    }

    #[test]
    fn backup_save_keeps_healthy_backup_when_current_is_corrupt() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        // Fichier courant tronqué, comme après une coupure ou une
        // intervention externe.
        write(&store.paths().settings, br#"{"schemaVersion": 1, "fah"#);

        let next = Settings {
            shutdown_time: Some("23:00".to_owned()),
            ..Settings::default()
        };
        let saved = store.save_settings_with_backup(&next).unwrap();

        assert!(saved.backup.is_none());
        assert_eq!(saved.preserved_backup.as_deref(), Some(backup.as_path()));
        assert_eq!(
            fs::read(&backup).unwrap(),
            healthy,
            "la dernière version saine a été écrasée par le contenu corrompu"
        );
        assert_eq!(store.load_settings().unwrap().value, next);
    }

    #[test]
    fn backup_save_does_not_create_backup_from_corrupt_current() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);
        write(&store.paths().settings, b"{ not json");

        let saved = store
            .save_settings_with_backup(&Settings::default())
            .unwrap();

        assert!(saved.backup.is_none());
        assert!(saved.preserved_backup.is_none());
        assert!(!backup.exists(), "aucun .bak corrompu ne doit être créé");
        assert!(!store.load_settings().unwrap().has_warning());
    }

    #[test]
    fn backup_save_keeps_backup_when_current_has_wrong_type() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        // JSON valide, mais une collection de profils, pas des réglages.
        write(&store.paths().settings, b"[]");

        let saved = store
            .save_settings_with_backup(&Settings::default())
            .unwrap();

        assert!(saved.backup.is_none());
        assert_eq!(saved.preserved_backup.as_deref(), Some(backup.as_path()));
        assert_eq!(fs::read(&backup).unwrap(), healthy);
    }

    #[test]
    fn backup_save_keeps_backup_when_current_fails_validation() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        // JSON désérialisable mais refusé par `validate` (schéma 0).
        let invalid = Settings {
            schema_version: 0,
            ..Settings::default()
        };
        write(
            &store.paths().settings,
            &serde_json::to_vec(&invalid).unwrap(),
        );

        let saved = store
            .save_settings_with_backup(&Settings::default())
            .unwrap();

        assert!(saved.backup.is_none());
        assert_eq!(saved.preserved_backup.as_deref(), Some(backup.as_path()));
        assert_eq!(fs::read(&backup).unwrap(), healthy);
    }

    #[test]
    fn backup_save_refuses_oversized_previous_and_keeps_backup() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp)
            .with_write_root(temp.path())
            .with_policy(LoadPolicy {
                max_file_bytes: 64,
                ..LoadPolicy::default()
            });
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        let oversized = vec![b' '; 65];
        write(&store.paths().settings, &oversized);

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("limite 64 octets"));
        assert_eq!(fs::read(&backup).unwrap(), healthy);
        assert_eq!(fs::read(&store.paths().settings).unwrap(), oversized);
    }

    #[test]
    fn backup_save_refuses_insecure_previous_mode_and_keeps_backup() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_write_root(temp.path());
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        write(
            &store.paths().settings,
            &serde_json::to_vec(&Settings::default()).unwrap(),
        );
        fs::set_permissions(&store.paths().settings, fs::Permissions::from_mode(0o666)).unwrap();

        let error = store
            .save_settings_with_backup(&Settings::default())
            .unwrap_err();
        assert!(error.to_string().contains("accessible en écriture"));
        assert_eq!(fs::read(&backup).unwrap(), healthy);
    }

    #[test]
    fn corrupt_current_never_overwrites_an_existing_backup() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let backup = super::backup_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        write(&store.paths().settings, b"{ broken");

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.value, Settings::default());
        let warning = loaded.warning().unwrap();
        assert_eq!(warning.kind, LoadWarningKind::InvalidJson);
        let quarantine = warning.backup_path.as_ref().unwrap();
        assert_eq!(quarantine, &super::quarantine_path(&store.paths().settings));
        assert_eq!(
            fs::read(&backup).unwrap(),
            healthy,
            "la récupération a écrasé la dernière version saine"
        );
        assert_eq!(fs::read(quarantine).unwrap(), b"{ broken");
        assert_eq!(fs::metadata(quarantine).unwrap().mode() & 0o777, 0o600);
        assert!(!store.paths().settings.exists());
    }

    #[test]
    fn strict_read_missing_file_returns_not_found_without_side_effects() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let before = snapshot_directory(temp.path());

        let settings = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(
            settings,
            StrictLoadError::NotFound { ref path } if path == &store.paths().settings
        ));
        let profiles = store.load_profiles_no_recovery().unwrap_err();
        assert!(matches!(profiles, StrictLoadError::NotFound { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn strict_read_valid_settings_and_profiles() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let settings = Settings {
            fahrenheit: true,
            shutdown_time: Some("22:30".to_owned()),
            ..Settings::default()
        };
        let profiles = ProfileCollection::new(vec![
            Profile::new("office", "Office"),
            Profile::new("quiet", "Quiet"),
        ]);
        store.save_settings(&settings).unwrap();
        store.save_profiles(&profiles).unwrap();
        let before = snapshot_directory(temp.path());

        assert_eq!(store.load_settings_no_recovery().unwrap(), settings);
        assert_eq!(store.load_profiles_no_recovery().unwrap(), profiles);
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn strict_read_corrupt_settings_does_not_quarantine() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let backup = super::backup_path(&store.paths().settings);
        let quarantine = super::quarantine_path(&store.paths().settings);
        let healthy = healthy_settings_bytes();
        write(&backup, &healthy);
        write(&quarantine, b"previous quarantine");
        write(&store.paths().settings, b"{ broken");
        let before = snapshot_directory(temp.path());

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Invalid { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
        assert_eq!(fs::read(&backup).unwrap(), healthy);
        assert_eq!(fs::read(&quarantine).unwrap(), b"previous quarantine");
        assert!(store.paths().settings.exists());
    }

    #[test]
    fn strict_read_corrupt_profiles_does_not_quarantine() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        write(&store.paths().profiles, b"{ not json");
        let before = snapshot_directory(temp.path());

        let error = store.load_profiles_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Invalid { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
        assert!(!super::backup_path(&store.paths().profiles).exists());
        assert!(!super::quarantine_path(&store.paths().profiles).exists());
    }

    #[test]
    fn strict_read_invalid_settings_does_not_quarantine() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let invalid = Settings {
            schema_version: 0,
            ..Settings::default()
        };
        write(
            &store.paths().settings,
            &serde_json::to_vec_pretty(&invalid).unwrap(),
        );
        let before = snapshot_directory(temp.path());

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Invalid { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
        assert!(!super::backup_path(&store.paths().settings).exists());
        assert!(!super::quarantine_path(&store.paths().settings).exists());
    }

    #[test]
    fn strict_read_oversized_file_does_not_quarantine() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_policy(LoadPolicy {
            max_file_bytes: 16,
            ..LoadPolicy::default()
        });
        write(&store.paths().settings, br#"{"fahrenheit":true}"#);
        let before = snapshot_directory(temp.path());

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::TooLarge { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
        assert!(!super::backup_path(&store.paths().settings).exists());
        assert!(!super::quarantine_path(&store.paths().settings).exists());
    }

    #[test]
    fn strict_read_refuses_symlink_without_touching_target() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let victim = temp.path().join("victim");
        write(&victim, b"do not touch");
        std::os::unix::fs::symlink(&victim, &store.paths().settings).unwrap();

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(
            error,
            StrictLoadError::Io { ref source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read(&victim).unwrap(), b"do not touch");
        assert!(!super::backup_path(&store.paths().settings).exists());
        assert!(!super::quarantine_path(&store.paths().settings).exists());
    }

    #[test]
    fn strict_read_refuses_unexpected_owner_without_modifying_files() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.save_settings(&Settings::default()).unwrap();
        let uid = fs::metadata(&store.paths().settings).unwrap().uid();
        let strict = store.with_policy(LoadPolicy {
            expected_owner: Some(uid.wrapping_add(1)),
            ..LoadPolicy::default()
        });
        let before = snapshot_directory(temp.path());

        let error = strict.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Io { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn strict_read_refuses_group_writable_file_without_modifying_it() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.save_settings(&Settings::default()).unwrap();
        fs::set_permissions(&store.paths().settings, fs::Permissions::from_mode(0o666)).unwrap();
        let before = snapshot_directory(temp.path());

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Io { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn strict_read_falls_back_to_admin_without_modifying_it() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let admin_file = admin.path().join(super::SETTINGS_FILE);
        write(&admin_file, &healthy_settings_bytes());
        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        let state_before = snapshot_directory(state.path());
        let admin_before = snapshot_directory(admin.path());

        let settings = store.load_settings_no_recovery().unwrap();
        assert!(settings.fahrenheit);
        assert_eq!(snapshot_directory(state.path()), state_before);
        assert_eq!(snapshot_directory(admin.path()), admin_before);
    }

    #[test]
    fn strict_read_corrupt_state_does_not_fall_back_to_admin() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let admin_file = admin.path().join(super::SETTINGS_FILE);
        write(&admin_file, &healthy_settings_bytes());
        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        write(&store.paths().settings, b"{ broken");
        let state_before = snapshot_directory(state.path());
        let admin_before = snapshot_directory(admin.path());

        let error = store.load_settings_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Invalid { .. }));
        assert_eq!(snapshot_directory(state.path()), state_before);
        assert_eq!(snapshot_directory(admin.path()), admin_before);
    }

    #[test]
    fn strict_read_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let settings = Settings {
            fahrenheit: true,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        let before = snapshot_directory(temp.path());

        assert_eq!(store.load_settings_no_recovery().unwrap(), settings);
        assert_eq!(store.load_settings_no_recovery().unwrap(), settings);
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn state_only_profiles_ignore_admin_fallback() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let admin_profiles = ProfileCollection::new(vec![Profile::new("custom-admin", "Admin")]);
        write(
            &admin.path().join(super::PROFILES_FILE),
            &serde_json::to_vec_pretty(&admin_profiles).unwrap(),
        );
        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        let state_before = snapshot_directory(state.path());
        let admin_before = snapshot_directory(admin.path());

        // Le repli explicite lit bien `/etc`...
        assert_eq!(store.load_profiles_no_recovery().unwrap(), admin_profiles);
        // ...mais l'API state-only ne le consulte jamais.
        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(
            error,
            StrictLoadError::NotFound { ref path } if path == &store.paths().profiles
        ));

        assert_eq!(snapshot_directory(state.path()), state_before);
        assert_eq!(snapshot_directory(admin.path()), admin_before);
    }

    #[test]
    fn state_only_profiles_read_state_and_ignore_admin() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let state_profiles = ProfileCollection::new(vec![Profile::new("custom-state", "State")]);
        let admin_profiles = ProfileCollection::new(vec![Profile::new("custom-admin", "Admin")]);
        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        store.save_profiles(&state_profiles).unwrap();
        write(
            &admin.path().join(super::PROFILES_FILE),
            &serde_json::to_vec_pretty(&admin_profiles).unwrap(),
        );
        let state_before = snapshot_directory(state.path());
        let admin_before = snapshot_directory(admin.path());

        assert_eq!(
            store.load_profiles_state_only_no_recovery().unwrap(),
            state_profiles
        );
        assert_eq!(snapshot_directory(state.path()), state_before);
        assert_eq!(snapshot_directory(admin.path()), admin_before);
    }

    #[test]
    fn state_only_profiles_absent_directory_is_not_created() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("missing-state");
        let store = ConfigStore::from_root(&state_root);

        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(
            error,
            StrictLoadError::NotFound { ref path } if path == &store.paths().profiles
        ));
        assert!(!state_root.exists());
    }

    #[test]
    fn state_only_profiles_corrupt_state_does_not_fall_back_or_quarantine() {
        let state = TempDir::new().unwrap();
        let admin = TempDir::new().unwrap();
        let admin_profiles = ProfileCollection::new(vec![Profile::new("custom-admin", "Admin")]);
        write(
            &admin.path().join(super::PROFILES_FILE),
            &serde_json::to_vec_pretty(&admin_profiles).unwrap(),
        );
        let store = ConfigStore::with_admin_roots(state.path(), admin.path());
        write(&store.paths().profiles, b"{ broken");
        let state_before = snapshot_directory(state.path());
        let admin_before = snapshot_directory(admin.path());

        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Invalid { .. }));
        assert!(store.paths().profiles.exists());
        assert!(!super::backup_path(&store.paths().profiles).exists());
        assert!(!super::quarantine_path(&store.paths().profiles).exists());
        assert_eq!(snapshot_directory(state.path()), state_before);
        assert_eq!(snapshot_directory(admin.path()), admin_before);
    }

    #[test]
    fn state_only_profiles_refuse_symlink_without_touching_target() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let victim = temp.path().join("victim");
        let profiles = ProfileCollection::new(vec![Profile::new("custom-state", "State")]);
        write(&victim, &serde_json::to_vec_pretty(&profiles).unwrap());
        std::os::unix::fs::symlink(&victim, &store.paths().profiles).unwrap();

        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(
            error,
            StrictLoadError::Io { ref source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert_eq!(
            fs::read(&victim).unwrap(),
            serde_json::to_vec_pretty(&profiles).unwrap()
        );
        assert!(!super::backup_path(&store.paths().profiles).exists());
        assert!(!super::quarantine_path(&store.paths().profiles).exists());
    }

    #[test]
    fn state_only_profiles_refuse_group_writable_file_without_modifying_it() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let profiles = ProfileCollection::new(vec![Profile::new("custom-state", "State")]);
        store.save_profiles(&profiles).unwrap();
        fs::set_permissions(&store.paths().profiles, fs::Permissions::from_mode(0o666)).unwrap();
        let before = snapshot_directory(temp.path());

        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Io { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn state_only_profiles_refuse_unexpected_owner_without_modifying_files() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let profiles = ProfileCollection::new(vec![Profile::new("custom-state", "State")]);
        store.save_profiles(&profiles).unwrap();
        let uid = fs::metadata(&store.paths().profiles).unwrap().uid();
        let strict = store.with_policy(LoadPolicy {
            expected_owner: Some(uid.wrapping_add(1)),
            ..LoadPolicy::default()
        });
        let before = snapshot_directory(temp.path());

        let error = strict.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::Io { .. }));
        assert_eq!(snapshot_directory(temp.path()), before);
    }

    #[test]
    fn state_only_profiles_refuse_oversized_file_without_quarantine() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp).with_policy(LoadPolicy {
            max_file_bytes: 16,
            ..LoadPolicy::default()
        });
        let profiles = ProfileCollection::new(vec![Profile::new("custom-state", "State")]);
        write(
            &store.paths().profiles,
            &serde_json::to_vec_pretty(&profiles).unwrap(),
        );
        let before = snapshot_directory(temp.path());

        let error = store.load_profiles_state_only_no_recovery().unwrap_err();
        assert!(matches!(error, StrictLoadError::TooLarge { .. }));
        assert!(!super::backup_path(&store.paths().profiles).exists());
        assert!(!super::quarantine_path(&store.paths().profiles).exists());
        assert_eq!(snapshot_directory(temp.path()), before);
    }
}
