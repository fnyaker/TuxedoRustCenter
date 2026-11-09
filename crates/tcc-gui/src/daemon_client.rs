// SPDX-License-Identifier: GPL-3.0-or-later
//
// Pont Rust ↔ QML vers le daemon.
//
// Un **worker unique** (thread « tcc-dbus ») possède le runtime tokio et dépile
// des commandes bornées (`Refresh`, `SetTempProfile`, `SetUvcControl`, …) ; à
// défaut de commande, il interroge l'état toutes les 2 s. Les invokables QML ne
// font qu'empiler une commande : aucun thread n'est créé par appel et une QML
// qui boucle ne peut pas faire croître le nombre de threads (R-15).
//
// Les réponses D-Bus sont traitées comme non fiables : taille bornée à
// [`MAX_RESPONSE_BYTES`] **avant** `serde_json::from_str`, validation par
// `tcc-ipc` / `tcc-core` (ou par la vue typée `hardware_view`), puis
// re-sérialisation du modèle validé avant publication vers QML (R-16). QML ne
// parse donc que du JSON canonique issu des modèles, jamais la chaîne brute.
//
// Les données matérielles proviennent de `Manager1` (`GetHardwareSnapshotJSON`)
// et sont en lecture seule côté daemon : aucun setter charge/éclairage/Fn-Lock
// n'est appelé. La seule écriture de session est `User1.SetUvcControl`, avec un
// chemin recopié du snapshot puis revalidé en Rust ; aucun secret, aucune
// commande shell et aucun chemin arbitraire ne viennent du QML.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use tcc_core::{Profile, ProfileCollection, Settings, StateKey};
use tcc_ipc::{
    ADMIN_ERROR_NAMESPACE, ADMIN_HW_CONTRACT_VERSION, ADMIN_HW_ERROR_NAMESPACE, DBUS_ADMIN_HW_PATH,
    DBUS_ADMIN_HW_SERVICE, DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE, DBUS_SESSION_AGENT_PATH,
    DBUS_SESSION_AGENT_SERVICE, MAX_CTGP_STATIC_OFFSET_WATTS, MAX_HW_STRING_CHARS, MAX_TDP_VALUES,
    SESSION_CONTRACT_VERSION, StateSnapshot, ensure_admin_contract_compatible,
};
use tokio::time::timeout;

use crate::display_view::DisplayInfoView;
use crate::fan_view::FanView;
use crate::hardware_view::HardwareView;
use crate::power_blocker::{
    BlockerError, BlockerHandle, LogindInhibitor, PowerBlocker, recover_from_queue_failure,
    session_is_graphical,
};
use crate::profile_edit::{
    SaveError, prepare_create, prepare_delete, prepare_duplicate, prepare_save,
};
use crate::profile_transfer::ImportSummary;
use crate::settings_edit::{ChargeCapabilities, prepare_settings_save};

/// Timeout d'un appel D-Bus unitaire.
const DBUS_TIMEOUT: Duration = Duration::from_secs(3);
/// Intervalle de rafraîchissement quand aucune commande n'est en attente.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Polling ralenti quand la fenêtre est masquée (tray) : aucune animation ni
/// rendu, les commandes restent traitées immédiatement.
const HIDDEN_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Taille maximale acceptée pour une réponse JSON du daemon (4 Mio).
///
/// Alignement sur `tcc-user` : au-delà, la réponse est rejetée avant toute
/// désérialisation, ce qui borne la mémoire et le travail de parsing.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Capacité de la file de commandes QML → worker.
///
/// Volontairement petite : au-delà, la commande est refusée avec l'état
/// « occupé » plutôt que d'être mise en attente indéfiniment.
const COMMAND_QUEUE_CAPACITY: usize = 8;
/// Marge appliquée après la sortie du Dashboard avant de relire l'instantané
/// matériel : le bail capteurs du daemon (10 s) doit pouvoir expirer sans être
/// réarmé par une lecture.
const SENSOR_LEASE_GRACE: Duration = Duration::from_secs(11);
/// Cadence de détection du service `Admin1`.
///
/// Volontairement lente : le service est activé à la demande et s'arrête après
/// 30 s d'inactivité. `NameHasOwner`/`ListActivatableNames` sont interrogés
/// d'abord (sans activation) ; `Ping`/`GetContractVersion` seulement si le nom
/// est déjà possédé, pour ne pas lancer `tccd-admin` toutes les 2 s.
const ADMIN_PROBE_INTERVAL: Duration = Duration::from_secs(15);
/// Timeout court de la détection (`Ping`, `GetContractVersion`).
const ADMIN_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeout d'un enregistrement.
///
/// Pire cas côté `tccd-admin` : dialogue polkit 120 s + `ReloadConfig` 30 s,
/// plus un second rechargement possible après rollback (30 s) — soit ~180 s.
/// La GUI attend 240 s pour ne pas afficher un faux `Timeout` pendant que
/// l'admin écrit encore.
///
/// Un `Timeout` client **n'annule pas** l'opération côté admin : la GUI relit
/// ensuite l'état et n'envoie jamais de nouvelle tentative automatique.
const ADMIN_SAVE_TIMEOUT: Duration = Duration::from_secs(240);
/// Garde-fou : au-delà, `saving` est considéré comme bloqué et remis à faux
/// (worker disparu, bug). Une nouvelle demande reste possible.
const ADMIN_SAVE_STALL: Duration = Duration::from_secs(300);
/// Timeout d'une écriture `Admin.Hw` (polkit + dialogue + matériel).
const ADMIN_HW_WRITE_TIMEOUT: Duration = Duration::from_secs(180);
/// Timeout d'une lecture `Admin.Hw` (`GetTdpValues`).
#[allow(dead_code)] // voie de lecture contractuelle (tests/CLI), l'UI lit le snapshot
const ADMIN_HW_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Sonde du service de session (`User1`), sans activation abusive.
const AGENT_PROBE_INTERVAL: Duration = Duration::from_secs(15);
/// Timeout court de `Ping`/`GetContractVersion`/`GetVersion`.
const AGENT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeout de `GetDisplayInfoJSON` (lecture seule, cache agent 1 s).
const AGENT_READ_TIMEOUT: Duration = Duration::from_secs(3);
/// Timeout d'une écriture UVC (`User1.SetUvcControl`).
///
/// Le backend borne lui-même l'ioctl à 2 s et le service applique un verrou par
/// device : 4 s laissent la marge nécessaire sans bloquer le worker.
const AGENT_UVC_WRITE_TIMEOUT: Duration = Duration::from_secs(4);
/// Longueur maximale d'un chemin de device UVC (contrat `User1`).
const MAX_UVC_DEVICE_PATH_BYTES: usize = 64;
/// Longueur maximale d'une clé de contrôle UVC (contrat `User1`).
const MAX_UVC_CONTROL_KEY_BYTES: usize = 64;
/// Namespace des erreurs applicatives de `User1`.
const AGENT_UVC_ERROR_NAMESPACE: &str = "org.tuxedocomputers.tccd.User1.Error";
/// Bornes TDP du contrat `Admin.Hw` (le serveur utilise les mêmes).
const ADMIN_HW_TDP_MIN_WATTS: i32 = 1;
const ADMIN_HW_TDP_MAX_WATTS: i32 = 1000;

/// Nom ODM : mêmes règles que `tccd-admin::validate_odm_name` (≤ 64 caractères,
/// ASCII graphique, sans antislash). La valeur ne doit jamais venir d'un champ
/// libre QML.
fn admin_hw_odm_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_HW_STRING_CHARS
        && name
            .chars()
            .all(|character| character.is_ascii_graphic() && character != '\\')
}

/// TDP : 1 à 3 valeurs (PL1/PL2/PL4), 1–1000 W. L'ordre et l'unicité des
/// descripteurs sont vérifiés côté QML contre `HardwareData.odm.tdp` (le
/// contrat ne transporte que les valeurs) puis par le serveur.
fn admin_hw_tdp_values_valid(values: &[i32]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_TDP_VALUES
        && values
            .iter()
            .all(|value| (ADMIN_HW_TDP_MIN_WATTS..=ADMIN_HW_TDP_MAX_WATTS).contains(value))
        // Les valeurs sont dans l'ordre des descripteurs PL1/PL2/PL4 :
        // la monotonie croissante évite une configuration incohérente.
        && values.windows(2).all(|pair| pair[0] <= pair[1])
}

/// cTGP : 0–200 W (borne statique du contrat). Une borne dynamique exposée par
/// le snapshot est appliquée par la GUI avant l'appel.
fn admin_hw_ctgp_valid(watts: i32) -> bool {
    (0..=MAX_CTGP_STATIC_OFFSET_WATTS).contains(&watts)
}

/// Chemin `/dev/videoN` canonique, identique aux règles de `tccd-user`.
///
/// Le chemin vient toujours du snapshot `Manager1` : aucune valeur inventée en
/// QML n'est acceptée, même si elle ressemble à un chemin valide.
fn agent_uvc_device_path_valid(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_UVC_DEVICE_PATH_BYTES || !path.is_ascii() {
        return false;
    }
    let Some(digits) = path.strip_prefix("/dev/video") else {
        return false;
    };
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    // `video00` n'est pas un nom canonique produit par udev.
    if digits.len() > 1 && digits.starts_with('0') {
        return false;
    }
    matches!(digits.parse::<u32>(), Ok(number) if number <= 4096)
}

/// Clé de contrôle UVC stable (`[a-z0-9_]{1,64}`), comme le service `User1`.
fn agent_uvc_control_key_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_UVC_CONTROL_KEY_BYTES
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Échec d'une écriture UVC, réduit à un code stable et un détail **statique**.
///
/// Le message D-Bus n'est jamais publié vers QML : il peut contenir un chemin ou
/// une donnée du périphérique. Seul le code est traduit par l'UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UvcWriteFailure {
    code: &'static str,
    detail: &'static str,
}

impl UvcWriteFailure {
    const fn new(code: &'static str) -> Self {
        Self { code, detail: "" }
    }

    const fn with_detail(code: &'static str, detail: &'static str) -> Self {
        Self { code, detail }
    }
}

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, state_json, cxx_name = "stateJson")]
        #[qproperty(QString, profiles_json, cxx_name = "profilesJson")]
        #[qproperty(bool, connected)]
        #[qproperty(QString, last_error, cxx_name = "lastError")]
        #[qproperty(QString, active_profile_id, cxx_name = "activeProfileId")]
        #[qproperty(QString, active_profile_name, cxx_name = "activeProfileName")]
        #[qproperty(QString, ppd_profile, cxx_name = "ppdProfile")]
        #[qproperty(QString, power_source, cxx_name = "powerSource")]
        #[qproperty(bool, manual_override, cxx_name = "manualOverride")]
        #[qproperty(bool, busy)]
        #[qproperty(i32, revision)]
        #[qproperty(QString, daemon_version, cxx_name = "daemonVersion")]
        #[qproperty(i32, contract_version, cxx_name = "contractVersion")]
        #[qproperty(QString, hardware_json, cxx_name = "hardwareJson")]
        #[qproperty(QString, hardware_error, cxx_name = "hardwareError")]
        #[qproperty(bool, sensors_enabled, cxx_name = "sensorsEnabled")]
        #[qproperty(QString, fan_json, cxx_name = "fanJson")]
        #[qproperty(QString, fan_error, cxx_name = "fanError")]
        #[qproperty(bool, admin_available, cxx_name = "adminAvailable")]
        #[qproperty(QString, admin_version, cxx_name = "adminVersion")]
        #[qproperty(QString, admin_error, cxx_name = "adminError")]
        #[qproperty(bool, admin_hw_available, cxx_name = "adminHwAvailable")]
        #[qproperty(QString, admin_hw_version, cxx_name = "adminHwVersion")]
        #[qproperty(QString, admin_hw_error, cxx_name = "adminHwError")]
        #[qproperty(bool, admin_hw_pending, cxx_name = "adminHwPending")]
        #[qproperty(QString, admin_hw_action, cxx_name = "adminHwAction")]
        #[qproperty(QString, admin_hw_save_error, cxx_name = "adminHwSaveError")]
        #[qproperty(
            QString,
            admin_hw_save_error_detail,
            cxx_name = "adminHwSaveErrorDetail"
        )]
        #[qproperty(bool, admin_hw_succeeded, cxx_name = "adminHwSucceeded")]
        #[qproperty(bool, agent_available, cxx_name = "agentAvailable")]
        #[qproperty(QString, agent_version, cxx_name = "agentVersion")]
        #[qproperty(QString, agent_error, cxx_name = "agentError")]
        #[qproperty(bool, agent_uvc_write_supported, cxx_name = "agentUvcWriteSupported")]
        #[qproperty(bool, agent_uvc_write_pending, cxx_name = "agentUvcWritePending")]
        #[qproperty(QString, agent_uvc_write_error, cxx_name = "agentUvcWriteError")]
        #[qproperty(QString, agent_uvc_write_detail, cxx_name = "agentUvcWriteDetail")]
        #[qproperty(bool, agent_uvc_write_succeeded, cxx_name = "agentUvcWriteSucceeded")]
        #[qproperty(QString, display_info_json, cxx_name = "displayInfoJson")]
        #[qproperty(bool, saving)]
        #[qproperty(QString, save_error, cxx_name = "saveError")]
        #[qproperty(QString, save_error_detail, cxx_name = "saveErrorDetail")]
        #[qproperty(bool, save_succeeded, cxx_name = "saveSucceeded")]
        #[qproperty(QString, save_action, cxx_name = "saveAction")]
        #[qproperty(QString, created_profile_id, cxx_name = "createdProfileId")]
        #[qproperty(i32, exported_count, cxx_name = "exportedCount")]
        #[qproperty(i32, imported_count, cxx_name = "importedCount")]
        #[qproperty(i32, renamed_count, cxx_name = "renamedCount")]
        #[qproperty(i32, skipped_count, cxx_name = "skippedCount")]
        #[qproperty(QString, settings_json, cxx_name = "settingsJson")]
        #[qproperty(QString, settings_error, cxx_name = "settingsError")]
        #[qproperty(bool, settings_fahrenheit, cxx_name = "settingsFahrenheit")]
        #[qproperty(bool, settings_cpu_enabled, cxx_name = "settingsCpuEnabled")]
        #[qproperty(bool, settings_fan_enabled, cxx_name = "settingsFanEnabled")]
        #[qproperty(bool, settings_lighting_enabled, cxx_name = "settingsLightingEnabled")]
        #[qproperty(bool, settings_ppd_override, cxx_name = "settingsPpdOverride")]
        #[qproperty(
            QString,
            settings_charging_profile,
            cxx_name = "settingsChargingProfile"
        )]
        #[qproperty(QString, settings_charging_type, cxx_name = "settingsChargingType")]
        #[qproperty(QString, settings_keyboard_mode, cxx_name = "settingsKeyboardMode")]
        #[qproperty(
            QString,
            settings_charging_priority,
            cxx_name = "settingsChargingPriority"
        )]
        #[qproperty(i32, settings_charging_start, cxx_name = "settingsChargingStart")]
        #[qproperty(i32, settings_charging_end, cxx_name = "settingsChargingEnd")]
        #[qproperty(
            QString,
            settings_matrix_power_saver_ac,
            cxx_name = "settingsMatrixPowerSaverAc"
        )]
        #[qproperty(
            QString,
            settings_matrix_power_saver_bat,
            cxx_name = "settingsMatrixPowerSaverBat"
        )]
        #[qproperty(
            QString,
            settings_matrix_balanced_ac,
            cxx_name = "settingsMatrixBalancedAc"
        )]
        #[qproperty(
            QString,
            settings_matrix_balanced_bat,
            cxx_name = "settingsMatrixBalancedBat"
        )]
        #[qproperty(
            QString,
            settings_matrix_performance_ac,
            cxx_name = "settingsMatrixPerformanceAc"
        )]
        #[qproperty(
            QString,
            settings_matrix_performance_bat,
            cxx_name = "settingsMatrixPerformanceBat"
        )]
        #[qproperty(QString, settings_shutdown_time, cxx_name = "settingsShutdownTime")]
        #[qproperty(bool, shutdown_timer_available, cxx_name = "shutdownTimerAvailable")]
        #[qproperty(bool, shutdown_timer_enabled, cxx_name = "shutdownTimerEnabled")]
        #[qproperty(
            QString,
            shutdown_timer_scheduled_at,
            cxx_name = "shutdownTimerScheduledAt"
        )]
        #[qproperty(QString, shutdown_timer_source, cxx_name = "shutdownTimerSource")]
        #[qproperty(QString, shutdown_timer_error, cxx_name = "shutdownTimerError")]
        #[qproperty(bool, power_blocker_available, cxx_name = "powerBlockerAvailable")]
        #[qproperty(bool, power_blocked, cxx_name = "powerBlocked")]
        #[qproperty(bool, power_blocker_pending, cxx_name = "powerBlockerPending")]
        #[qproperty(QString, power_blocker_error, cxx_name = "powerBlockerError")]
        #[qproperty(bool, autostart_enabled, cxx_name = "autostartEnabled")]
        #[qproperty(bool, autostart_available, cxx_name = "autostartAvailable")]
        #[qproperty(QString, autostart_error, cxx_name = "autostartError")]
        #[qproperty(bool, saving_settings, cxx_name = "savingSettings")]
        #[qproperty(QString, save_settings_error, cxx_name = "saveSettingsError")]
        #[qproperty(
            QString,
            save_settings_error_detail,
            cxx_name = "saveSettingsErrorDetail"
        )]
        #[qproperty(bool, save_settings_succeeded, cxx_name = "saveSettingsSucceeded")]
        type DaemonClient = super::DaemonClientRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut DaemonClient>);

        #[qinvokable]
        #[cxx_name = "setTempProfile"]
        fn set_temp_profile(self: Pin<&mut DaemonClient>, id: &QString);

        #[qinvokable]
        #[cxx_name = "clearTempProfile"]
        fn clear_temp_profile(self: Pin<&mut DaemonClient>);

        /// Force un cycle complet (état, profils, matériel).
        #[qinvokable]
        #[cxx_name = "refreshHardware"]
        fn refresh_hardware(self: Pin<&mut DaemonClient>);

        /// Signale si la page Dashboard est visible : le bail capteurs n'est
        /// renouvelé que dans ce cas.
        #[qinvokable]
        #[cxx_name = "setSensorPageVisible"]
        fn set_sensor_page_visible(self: Pin<&mut DaemonClient>, visible: bool);

        /// Fenêtre visible (masquée dans le tray) : ralentit le polling.
        #[qinvokable]
        #[cxx_name = "setWindowVisible"]
        fn set_window_visible(self: Pin<&mut DaemonClient>, visible: bool);

        /// Enregistre un profil modifié via `Admin1.SaveProfilesJSON`.
        ///
        /// `edit_json` est un DTO borné (champs phase 1), jamais la collection
        /// complète : Rust applique la modification sur la dernière collection
        /// validée avant de sérialiser.
        #[qinvokable]
        #[cxx_name = "saveProfile"]
        fn save_profile(self: Pin<&mut DaemonClient>, id: &QString, edit_json: &QString);

        /// Force une nouvelle détection du service d'administration.
        #[qinvokable]
        #[cxx_name = "refreshAdmin"]
        fn refresh_admin(self: Pin<&mut DaemonClient>);

        /// Enregistre les réglages globaux modifiables (DTO borné).
        #[qinvokable]
        #[cxx_name = "saveSettings"]
        fn save_settings(self: Pin<&mut DaemonClient>, edit_json: &QString);

        /// Crée un profil (ID généré côté Rust).
        #[qinvokable]
        #[cxx_name = "createProfile"]
        fn create_profile(self: Pin<&mut DaemonClient>, edit_json: &QString);

        /// Duplique un profil personnalisé (ID/nom générés côté Rust).
        #[qinvokable]
        #[cxx_name = "duplicateProfile"]
        fn duplicate_profile(self: Pin<&mut DaemonClient>, source_id: &QString);

        /// Supprime un profil personnalisé.
        #[qinvokable]
        #[cxx_name = "deleteProfile"]
        fn delete_profile(self: Pin<&mut DaemonClient>, id: &QString);

        /// Exporte les profils personnalisés vers un fichier choisi par
        /// l'utilisateur. Aucune écriture privilégiée : fichier local.
        #[qinvokable]
        #[cxx_name = "exportProfiles"]
        fn export_profiles(self: Pin<&mut DaemonClient>, path: &QString);

        /// Importe des profils depuis un fichier choisi par l'utilisateur.
        /// L'écriture passe par `Admin1.SaveProfilesJSON` (polkit).
        #[qinvokable]
        #[cxx_name = "importProfiles"]
        fn import_profiles(self: Pin<&mut DaemonClient>, path: &QString);

        /// Active/désactive l'inhibiteur de veille `idle` (logind, non
        /// privilégié). Le fd est détenu par le worker.
        #[qinvokable]
        #[cxx_name = "setPowerBlocker"]
        fn set_power_blocker(self: Pin<&mut DaemonClient>, blocked: bool);

        /// Active/désactive l'autostart XDG (fichier utilisateur).
        #[qinvokable]
        #[cxx_name = "setAutostart"]
        fn set_autostart(self: Pin<&mut DaemonClient>, enabled: bool);

        /// Applique un profil ODM via `Admin.Hw` (`force` = override PPD).
        #[qinvokable]
        #[cxx_name = "applyOdm"]
        fn apply_odm(self: Pin<&mut DaemonClient>, name: &QString, force: bool);

        /// Applique des limites TDP (JSON array d'entiers, borné).
        #[qinvokable]
        #[cxx_name = "applyTdp"]
        fn apply_tdp(self: Pin<&mut DaemonClient>, tdp_json: &QString);

        /// Applique l'offset cTGP NVIDIA en watts.
        #[qinvokable]
        #[cxx_name = "applyCtgp"]
        fn apply_ctgp(self: Pin<&mut DaemonClient>, watts: i32);

        /// Force une relecture du service de session (peut l'activer).
        #[qinvokable]
        #[cxx_name = "refreshAgent"]
        fn refresh_agent(self: Pin<&mut DaemonClient>);

        /// Écrit un contrôle UVC via `User1.SetUvcControl` (session non
        /// privilégiée). Le chemin vient du snapshot `Manager1` ; la clé et la
        /// valeur sont revalidées en Rust avant tout appel D-Bus.
        #[qinvokable]
        #[cxx_name = "setUvcControl"]
        fn set_uvc_control(
            self: Pin<&mut DaemonClient>,
            device_path: &QString,
            control_key: &QString,
            value: i32,
        );
    }

    impl cxx_qt::Threading for DaemonClient {}
    impl cxx_qt::Initialize for DaemonClient {}
}

/// Vue typée de `Manager1.GetShutdownTimerJSON` (lecture seule).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ShutdownTimerView {
    enabled: bool,
    scheduled_at: Option<String>,
    /// `settings`, `logind` ou `none`.
    source: String,
    error: Option<String>,
}

/// Commande déposée par QML dans la file du worker.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// Relire l'état sans action d'écriture.
    Refresh,
    /// Demander un override temporaire.
    SetTempProfile(String),
    /// Revenir à la bascule automatique.
    ClearTempProfile,
    /// Activer/désactiver le bail de collecte des capteurs.
    SetSensors(bool),
    /// Enregistrer un profil (DTO d'édition JSON borné).
    SaveProfile { id: String, edit_json: String },
    /// Créer un profil (ID généré côté Rust).
    CreateProfile { edit_json: String },
    /// Dupliquer un profil personnalisé.
    DuplicateProfile { source_id: String },
    /// Supprimer un profil personnalisé.
    DeleteProfile { id: String },
    /// Exporter les profils personnalisés vers un fichier choisi par
    /// l'utilisateur.
    ExportProfiles { path: String },
    /// Importer des profils depuis un fichier choisi par l'utilisateur.
    ImportProfiles { path: String },
    /// Enregistrer les réglages globaux (DTO d'édition JSON borné).
    SaveSettings { edit_json: String },
    /// Activer/désactiver l'inhibiteur de veille `idle`.
    SetPowerBlocker(bool),
    /// Appliquer un profil ODM (`force` = override PPD).
    ApplyOdm { name: String, force: bool },
    /// Appliquer des limites TDP (ordre firmware).
    ApplyTdp { values: Vec<i32> },
    /// Appliquer l'offset cTGP NVIDIA.
    ApplyCtgp { watts: i32 },
    /// Forcer une détection `Admin1` au prochain cycle.
    ProbeAdmin,
    /// Forcer une détection/relecture du service de session `User1`.
    RefreshAgent,
    /// Écrire un contrôle UVC via `User1.SetUvcControl` (session).
    SetUvcControl {
        device_path: String,
        control_key: String,
        value: i32,
    },
}

/// Erreurs du pont, typées pour distinguer une perte de connexion d'un refus.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientError {
    /// File de commandes pleine : la commande n'a pas été acceptée.
    Busy,
    /// Le worker D-Bus n'existe plus (fenêtre détruite, thread arrêté).
    WorkerStopped,
    /// Réponse dépassant [`MAX_RESPONSE_BYTES`] : rejetée avant parsing.
    ResponseTooLarge {
        call: &'static str,
        size: usize,
        limit: usize,
    },
    /// Aucune réponse dans [`DBUS_TIMEOUT`].
    Timeout { call: &'static str },
    /// Erreur de transport D-Bus (bus absent, service absent, refus).
    Transport { call: &'static str, detail: String },
    /// Réponse illisible (JSON invalide).
    Decode { model: &'static str, detail: String },
    /// Réponse lisible mais invalide (contrat, champ, refus du daemon).
    Invalid { model: &'static str, detail: String },
    /// Échec de re-sérialisation du modèle validé.
    Encode { model: &'static str, detail: String },
    /// Runtime tokio indisponible.
    Runtime { detail: String },
}

impl ClientError {
    /// Vrai si l'erreur signifie que le daemon n'est plus joignable.
    ///
    /// Les erreurs de décodage ou de refus sémantique laissent la connexion
    /// considérée comme établie : afficher « déconnecté » serait faux.
    fn loses_connection(&self) -> bool {
        matches!(
            self,
            Self::WorkerStopped
                | Self::Timeout { .. }
                | Self::Transport { .. }
                | Self::Runtime { .. }
        )
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("trop de commandes en attente (file pleine)"),
            Self::WorkerStopped => f.write_str("worker D-Bus arrêté"),
            Self::ResponseTooLarge { call, size, limit } => {
                write!(
                    f,
                    "{call}: réponse trop volumineuse ({size} octets, max {limit})"
                )
            }
            Self::Timeout { call } => write!(f, "{call}: timeout"),
            Self::Transport { call, detail } => {
                // Ne jamais publier une adresse de bus / un socket dans l'UI.
                let safe = sanitize_client_detail(detail);
                if safe.is_empty() {
                    write!(f, "{call}: erreur de transport")
                } else {
                    write!(f, "{call}: {safe}")
                }
            }
            Self::Decode { model, detail } => write!(f, "{model} JSON: {detail}"),
            Self::Invalid { model, detail } => write!(f, "{model}: {detail}"),
            Self::Encode { model, detail } => write!(f, "{model} JSON (sérialisation): {detail}"),
            Self::Runtime { detail } => write!(f, "runtime tokio: {detail}"),
        }
    }
}

/// Nettoie un détail d'erreur de transport avant publication vers QML.
///
/// Les messages `zbus` peuvent contenir une adresse de bus
/// (`unix:path=/run/user/…`) ou un socket : on ne garde qu'un texte ASCII
/// court, sans séparateur de chemin ni adresse.
fn sanitize_client_detail(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 80
        || !trimmed.is_ascii()
        || trimmed
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '@' | '='))
    {
        return String::new();
    }
    trimmed.to_owned()
}

/// Données validées à pousser vers QML.
#[derive(Debug)]
struct SnapshotData {
    /// `StateSnapshot` re-sérialisé depuis le modèle validé.
    state_json: String,
    /// `ProfileCollection` re-sérialisée depuis le modèle validé.
    profiles_json: String,
    active_id: String,
    active_name: String,
    ppd: String,
    power: String,
    manual_override: bool,
    revision: i32,
    daemon_version: String,
    contract_version: i32,
    /// Collection validée, conservée par le worker pour appliquer les éditions.
    profiles: Option<ProfileCollection>,
}

/// Résultat d'une tentative d'enregistrement, publié vers QML.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SaveOutcome {
    /// Aucun enregistrement dans ce cycle.
    None,
    /// Enregistrement accepté par `tccd-admin` puis relu.
    Ok,
    /// Refus ou erreur ; le formulaire QML est conservé.
    Err(SaveError),
}

/// État du service d'administration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdminStatus {
    available: bool,
    /// Version d'implémentation, vide si le service est endormi.
    version: String,
    /// Code stable : `""`, `Unavailable`, `Contract`, `Transport`.
    error: &'static str,
}

/// État du service de session `User1`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionStatus {
    available: bool,
    /// Nom déjà possédé : la lecture peut être faite sans activer le service.
    owned: bool,
    version: String,
    /// Écriture UVC exposée par l'agent (`GetUvcWriteSupported`).
    uvc_write_supported: bool,
    /// `""`, `Unavailable`, `Contract`, `Transport`.
    error: &'static str,
}

impl SessionStatus {
    fn unavailable(error: &'static str) -> Self {
        Self {
            available: false,
            owned: false,
            version: String::new(),
            uvc_write_supported: false,
            error,
        }
    }
}

impl AdminStatus {
    fn unavailable(error: &'static str) -> Self {
        Self {
            available: false,
            version: String::new(),
            error,
        }
    }
}

/// Extrémité d'envoi de la file de commandes, clonable et testable.
#[derive(Clone)]
struct CommandSender {
    tx: SyncSender<Command>,
}

impl CommandSender {
    /// Empile sans bloquer. `Busy` si la file est pleine, sans créer de thread.
    fn try_send(&self, command: Command) -> Result<(), ClientError> {
        match self.tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(ClientError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(ClientError::WorkerStopped),
        }
    }
}

/// État Rust du QObject.
pub struct DaemonClientRust {
    state_json: QString,
    profiles_json: QString,
    connected: bool,
    last_error: QString,
    active_profile_id: QString,
    active_profile_name: QString,
    ppd_profile: QString,
    power_source: QString,
    manual_override: bool,
    busy: bool,
    revision: i32,
    daemon_version: QString,
    contract_version: i32,
    hardware_json: QString,
    hardware_error: QString,
    sensors_enabled: bool,
    fan_json: QString,
    fan_error: QString,
    admin_available: bool,
    admin_version: QString,
    admin_error: QString,
    admin_hw_available: bool,
    admin_hw_version: QString,
    admin_hw_error: QString,
    admin_hw_pending: bool,
    admin_hw_action: QString,
    admin_hw_save_error: QString,
    admin_hw_save_error_detail: QString,
    admin_hw_succeeded: bool,
    agent_available: bool,
    agent_version: QString,
    agent_error: QString,
    agent_uvc_write_supported: bool,
    agent_uvc_write_pending: bool,
    agent_uvc_write_error: QString,
    agent_uvc_write_detail: QString,
    agent_uvc_write_succeeded: bool,
    display_info_json: QString,
    saving: bool,
    /// Horodatage du début d'enregistrement (garde-fou anti-blocage).
    saving_since: Option<Instant>,
    save_error: QString,
    save_error_detail: QString,
    save_succeeded: bool,
    save_action: QString,
    created_profile_id: QString,
    exported_count: i32,
    imported_count: i32,
    renamed_count: i32,
    skipped_count: i32,
    settings_json: QString,
    settings_error: QString,
    settings_fahrenheit: bool,
    settings_cpu_enabled: bool,
    settings_fan_enabled: bool,
    settings_lighting_enabled: bool,
    settings_ppd_override: bool,
    /// Profil de charge persisté (`""` = non défini).
    settings_charging_profile: QString,
    /// Type de charge persisté (`""` = non défini).
    settings_charging_type: QString,
    /// Mode de la première zone clavier (`""` = aucune zone).
    settings_keyboard_mode: QString,
    /// Priorité de charge persistée (`""` = non définie).
    settings_charging_priority: QString,
    /// Seuil de début persisté (`-1` = non défini).
    settings_charging_start: i32,
    /// Seuil de fin persisté (`-1` = non défini).
    settings_charging_end: i32,
    /// Matrice PPD × alimentation (`""` = clé absente, fallback daemon).
    settings_matrix_power_saver_ac: QString,
    settings_matrix_power_saver_bat: QString,
    settings_matrix_balanced_ac: QString,
    settings_matrix_balanced_bat: QString,
    settings_matrix_performance_ac: QString,
    settings_matrix_performance_bat: QString,
    /// Heure d'extinction persistée (`""` = aucune).
    settings_shutdown_time: QString,
    /// Statut `Manager1.GetShutdownTimerJSON`.
    shutdown_timer_available: bool,
    shutdown_timer_enabled: bool,
    shutdown_timer_scheduled_at: QString,
    shutdown_timer_source: QString,
    shutdown_timer_error: QString,
    /// Inhibiteur de veille (`idle`/`block`, logind).
    power_blocker_available: bool,
    power_blocked: bool,
    power_blocker_pending: bool,
    power_blocker_error: QString,
    /// Handle partagé avec le worker pour libérer le fd même si la file est
    /// pleine (SF-03).
    power_blocker_handle: Option<BlockerHandle>,
    /// Autostart XDG (`~/.config/autostart/…desktop`).
    autostart_enabled: bool,
    autostart_available: bool,
    autostart_error: QString,
    saving_settings: bool,
    saving_settings_since: Option<Instant>,
    save_settings_error: QString,
    save_settings_error_detail: QString,
    save_settings_succeeded: bool,
    /// Visibilité du Dashboard, lue par le worker (partagée, sans commande).
    page_visible: Arc<AtomicBool>,
    /// Fenêtre visible (partagée avec le worker, sans commande).
    window_visible: Arc<AtomicBool>,
    /// File vers le worker unique. `None` tant que `initialize` n'a pas tourné.
    commands: Option<CommandSender>,
    /// Connexion au signal `sensorsEnabledChanged` (intent utilisateur → worker).
    sensors_guard: Option<cxx_qt::QMetaObjectConnectionGuard>,
}

impl Default for DaemonClientRust {
    fn default() -> Self {
        let autostart = crate::autostart::current_state();
        Self {
            state_json: QString::from("{}"),
            profiles_json: QString::from("[]"),
            connected: false,
            last_error: QString::from(""),
            active_profile_id: QString::from(""),
            active_profile_name: QString::from(""),
            ppd_profile: QString::from(""),
            power_source: QString::from(""),
            manual_override: false,
            busy: false,
            revision: 0,
            daemon_version: QString::from(""),
            contract_version: 0,
            hardware_json: QString::from("{}"),
            hardware_error: QString::from(""),
            // Collecte automatique : le worker ne renouvelle le bail que
            // lorsque le Dashboard est visible (`page_visible`). Aucun
            // interrupteur utilisateur.
            sensors_enabled: true,
            fan_json: QString::from("{}"),
            fan_error: QString::from(""),
            admin_available: false,
            admin_version: QString::from(""),
            admin_error: QString::from(""),
            admin_hw_available: false,
            admin_hw_version: QString::from(""),
            admin_hw_error: QString::from(""),
            admin_hw_pending: false,
            admin_hw_action: QString::from(""),
            admin_hw_save_error: QString::from(""),
            admin_hw_save_error_detail: QString::from(""),
            admin_hw_succeeded: false,
            agent_available: false,
            agent_version: QString::from(""),
            agent_error: QString::from(""),
            agent_uvc_write_supported: false,
            agent_uvc_write_pending: false,
            agent_uvc_write_error: QString::from(""),
            agent_uvc_write_detail: QString::from(""),
            agent_uvc_write_succeeded: false,
            display_info_json: QString::from("{}"),
            saving: false,
            saving_since: None,
            save_error: QString::from(""),
            save_error_detail: QString::from(""),
            save_succeeded: false,
            save_action: QString::from(""),
            created_profile_id: QString::from(""),
            exported_count: 0,
            imported_count: 0,
            renamed_count: 0,
            skipped_count: 0,
            settings_json: QString::from("{}"),
            settings_error: QString::from(""),
            settings_fahrenheit: false,
            settings_cpu_enabled: true,
            settings_fan_enabled: true,
            settings_lighting_enabled: true,
            settings_ppd_override: false,
            settings_charging_profile: QString::from(""),
            settings_charging_type: QString::from(""),
            settings_keyboard_mode: QString::from(""),
            settings_charging_priority: QString::from(""),
            settings_charging_start: -1,
            settings_charging_end: -1,
            settings_matrix_power_saver_ac: QString::from(""),
            settings_matrix_power_saver_bat: QString::from(""),
            settings_matrix_balanced_ac: QString::from(""),
            settings_matrix_balanced_bat: QString::from(""),
            settings_matrix_performance_ac: QString::from(""),
            settings_matrix_performance_bat: QString::from(""),
            settings_shutdown_time: QString::from(""),
            shutdown_timer_available: false,
            shutdown_timer_enabled: false,
            shutdown_timer_scheduled_at: QString::from(""),
            shutdown_timer_source: QString::from(""),
            shutdown_timer_error: QString::from(""),
            power_blocker_available: false,
            power_blocked: false,
            power_blocker_pending: false,
            power_blocker_error: QString::from(""),
            power_blocker_handle: None,
            autostart_enabled: autostart.enabled,
            autostart_available: autostart.available,
            autostart_error: QString::from(autostart.error),
            saving_settings: false,
            saving_settings_since: None,
            save_settings_error: QString::from(""),
            save_settings_error_detail: QString::from(""),
            save_settings_succeeded: false,
            page_visible: Arc::new(AtomicBool::new(false)),
            window_visible: Arc::new(AtomicBool::new(true)),
            commands: None,
            sensors_guard: None,
        }
    }
}

impl cxx_qt::Initialize for ffi::DaemonClient {
    fn initialize(mut self: Pin<&mut Self>) {
        let qt_thread = self.qt_thread();
        let page_visible = Arc::clone(&self.page_visible);
        let window_visible = Arc::clone(&self.window_visible);
        let (tx, rx) = sync_channel(COMMAND_QUEUE_CAPACITY);
        let sender = CommandSender { tx };
        self.as_mut().rust_mut().get_mut().commands = Some(sender.clone());

        // `sensorsEnabled` est l'intention utilisateur. Le setter généré
        // (`setSensorsEnabled`, slot Qt) et l'affectation QML passent tous deux
        // par ce signal : la commande part vers le worker, jamais de
        // désynchronisation entre l'UI et le bail.
        let guard = self
            .as_mut()
            .on_sensors_enabled_changed(move |mut qobject| {
                let enabled = qobject.sensors_enabled();
                match sender.try_send(Command::SetSensors(*enabled)) {
                    Ok(()) => qobject.as_mut().set_busy(true),
                    Err(error) => {
                        if error.loses_connection() {
                            qobject.as_mut().set_connected(false);
                        }
                        qobject
                            .as_mut()
                            .set_last_error(QString::from(error.to_string().as_str()));
                    }
                }
            });
        self.as_mut().rust_mut().get_mut().sensors_guard = Some(guard);

        // L'état du fd est partagé entre le worker (possession) et le QObject
        // (libération de secours si la file refuse une désactivation).
        let power_blocker = PowerBlocker::new(Arc::new(LogindInhibitor::new()));
        self.as_mut().rust_mut().get_mut().power_blocker_handle = Some(power_blocker.handle());

        let monitor_thread = qt_thread.clone();
        let spawn = std::thread::Builder::new()
            .name("tcc-dbus".to_owned())
            .spawn(move || worker_loop(qt_thread, rx, page_visible, window_visible, power_blocker));
        match spawn {
            Ok(handle) => {
                // Surveillant unique (pas un thread par appel) : si le worker
                // s'arrête alors qu'un enregistrement a été accepté, l'UI est
                // débloquée sans attendre `ADMIN_SAVE_STALL` (S1-04).
                let monitor = std::thread::Builder::new()
                    .name("tcc-worker-monitor".to_owned())
                    .spawn(move || {
                        monitor_worker(handle, || {
                            let _ = monitor_thread.queue(apply_worker_stopped);
                        });
                    });
                if monitor.is_err() {
                    eprintln!("tcc-gui: impossible de démarrer le surveillant D-Bus");
                }
            }
            Err(_) => {
                // Le QML affichera l'état déconnecté ; pas de panic dans la GUI.
                eprintln!("tcc-gui: impossible de démarrer le worker D-Bus");
            }
        }
    }
}

/// Attend la fin du worker puis exécute `on_stop`.
///
/// `is_finished` évite de bloquer un `join` sur un worker encore actif ; le
/// thread surveillant se termine avec le worker (une seule instance).
fn monitor_worker(handle: std::thread::JoinHandle<()>, on_stop: impl FnOnce()) {
    while !handle.is_finished() {
        std::thread::sleep(Duration::from_millis(250));
    }
    on_stop();
}

/// Réinitialise l'état de connexion et des enregistrements après la mort du
/// worker : sans cela, `saving`/`savingSettings` resteraient vrais et les
/// formulaires bloqués.
fn apply_worker_stopped(mut qobject: Pin<&mut ffi::DaemonClient>) {
    let was_saving = qobject.rust().saving;
    let was_saving_settings = qobject.rust().saving_settings;
    // Le worker peut avoir disparu sans exécuter son Drop (panic) : on ferme
    // le fd via le handle partagé avant de publier l'état.
    if let Some(handle) = qobject.rust().power_blocker_handle.as_ref() {
        handle.release();
    }

    let mut this = qobject.as_mut();
    this.as_mut().set_connected(false);
    this.as_mut().set_busy(false);
    this.as_mut().set_saving(false);
    this.as_mut().set_saving_settings(false);
    // Le worker est mort : le fd de l'inhibiteur est fermé avec lui.
    this.as_mut().set_power_blocked(false);
    this.as_mut().set_power_blocker_pending(false);
    this.as_mut()
        .set_power_blocker_error(QString::from("Unavailable"));
    this.as_mut().set_admin_hw_pending(false);
    this.as_mut()
        .set_admin_hw_save_error(QString::from("Unavailable"));
    this.as_mut().set_agent_available(false);
    this.as_mut().set_agent_version(QString::from(""));
    this.as_mut().set_agent_error(QString::from("Unavailable"));
    this.as_mut().set_agent_uvc_write_supported(false);
    this.as_mut().set_agent_uvc_write_pending(false);
    this.as_mut()
        .set_agent_uvc_write_error(QString::from("Unavailable"));
    this.as_mut().set_agent_uvc_write_detail(QString::from(""));
    this.as_mut().set_agent_uvc_write_succeeded(false);
    this.as_mut().set_display_info_json(QString::from("{}"));
    this.as_mut().rust_mut().get_mut().saving_since = None;
    this.as_mut().rust_mut().get_mut().saving_settings_since = None;
    this.as_mut()
        .set_last_error(QString::from("worker D-Bus arrêté"));

    if was_saving {
        this.as_mut().set_save_succeeded(false);
        this.as_mut().set_save_error(QString::from("Transport"));
        this.as_mut().set_save_error_detail(QString::from(""));
    }
    if was_saving_settings {
        this.as_mut().set_save_settings_succeeded(false);
        this.as_mut()
            .set_save_settings_error(QString::from("Transport"));
        this.as_mut()
            .set_save_settings_error_detail(QString::from(""));
    }
}

impl ffi::DaemonClient {
    fn refresh(self: Pin<&mut Self>) {
        self.queue(Command::Refresh);
    }

    fn set_temp_profile(self: Pin<&mut Self>, id: &QString) {
        let id = String::from(id);
        // Garde-fou local : un identifiant invalide n'atteint jamais le daemon.
        if let Err(error) = tcc_core::validate_profile_id(&tcc_core::ProfileId::new(id.clone())) {
            let error = ClientError::Invalid {
                model: "SetTempProfile",
                detail: format!("identifiant invalide ({error})"),
            };
            self.report_command_error(&error);
            return;
        }
        self.queue(Command::SetTempProfile(id));
    }

    fn clear_temp_profile(self: Pin<&mut Self>) {
        self.queue(Command::ClearTempProfile);
    }

    fn refresh_hardware(self: Pin<&mut Self>) {
        // Un cycle complet rafraîchit le matériel, l'état et les profils.
        self.queue(Command::Refresh);
    }

    fn set_window_visible(self: Pin<&mut Self>, visible: bool) {
        self.rust().window_visible.store(visible, Ordering::Relaxed);
    }

    fn set_sensor_page_visible(self: Pin<&mut Self>, visible: bool) {
        // Drapeau partagé : pas de commande, donc pas de perte si la file est
        // pleine. Le worker le lit à chaque cycle.
        self.rust().page_visible.store(visible, Ordering::Relaxed);
    }

    fn refresh_admin(self: Pin<&mut Self>) {
        self.queue(Command::ProbeAdmin);
    }

    fn save_settings(mut self: Pin<&mut Self>, edit_json: &QString) {
        let edit_json = String::from(edit_json);

        if !self.rust().admin_available {
            self.as_mut().set_saving_settings(false);
            self.as_mut().set_save_settings_succeeded(false);
            self.as_mut()
                .set_save_settings_error(QString::from(SaveError::AdminUnavailable.code()));
            self.as_mut()
                .set_save_settings_error_detail(QString::from(""));
            self.as_mut().set_busy(false);
            return;
        }
        if edit_json.len() > crate::settings_edit::MAX_SETTINGS_EDIT_BYTES {
            self.as_mut().set_saving_settings(false);
            self.as_mut().set_save_settings_succeeded(false);
            self.as_mut()
                .set_save_settings_error(QString::from("PayloadTooLarge"));
            self.as_mut()
                .set_save_settings_error_detail(QString::from(""));
            self.as_mut().set_busy(false);
            return;
        }
        if self.rust().saving_settings {
            let stalled = self
                .rust()
                .saving_settings_since
                .is_some_and(|since| since.elapsed() > ADMIN_SAVE_STALL);
            if stalled {
                self.as_mut().set_saving_settings(false);
                self.as_mut().set_save_settings_succeeded(false);
                self.as_mut()
                    .set_save_settings_error(QString::from("Timeout"));
                self.as_mut()
                    .set_save_settings_error_detail(QString::from(""));
                return;
            }
            self.as_mut().set_save_settings_error(QString::from("Busy"));
            self.as_mut()
                .set_save_settings_error_detail(QString::from(""));
            return;
        }

        self.as_mut().set_saving_settings(true);
        self.as_mut().rust_mut().get_mut().saving_settings_since = Some(Instant::now());
        self.as_mut().set_save_settings_succeeded(false);
        self.as_mut().set_save_settings_error(QString::from(""));
        self.as_mut()
            .set_save_settings_error_detail(QString::from(""));

        if let Err(error) = self.as_mut().try_queue(Command::SaveSettings { edit_json }) {
            let code = if error == ClientError::Busy {
                "Busy"
            } else {
                "Transport"
            };
            self.as_mut().set_saving_settings(false);
            self.as_mut().rust_mut().get_mut().saving_settings_since = None;
            self.as_mut().set_save_settings_error(QString::from(code));
            self.as_mut()
                .set_save_settings_error_detail(QString::from(""));
        }
    }

    fn save_profile(self: Pin<&mut Self>, id: &QString, edit_json: &QString) {
        let id = String::from(id);
        let edit_json = String::from(edit_json);
        let valid = if tcc_core::validate_profile_id(&tcc_core::ProfileId::new(id.clone())).is_err()
        {
            Err(SaveError::Validation {
                field: "id".to_owned(),
            })
        } else if edit_json.len() > crate::profile_edit::MAX_EDIT_BYTES {
            Err(SaveError::PayloadTooLarge)
        } else {
            Ok(Command::SaveProfile { id, edit_json })
        };
        self.queue_profile_mutation("save", valid, true);
    }

    fn create_profile(self: Pin<&mut Self>, edit_json: &QString) {
        let edit_json = String::from(edit_json);
        let valid = if edit_json.len() > crate::profile_edit::MAX_EDIT_BYTES {
            Err(SaveError::PayloadTooLarge)
        } else {
            Ok(Command::CreateProfile { edit_json })
        };
        self.queue_profile_mutation("create", valid, true);
    }

    fn duplicate_profile(self: Pin<&mut Self>, source_id: &QString) {
        let source_id = String::from(source_id);
        let valid = if tcc_core::validate_profile_id(&tcc_core::ProfileId::new(source_id.clone()))
            .is_err()
        {
            Err(SaveError::NotFound)
        } else {
            Ok(Command::DuplicateProfile { source_id })
        };
        self.queue_profile_mutation("duplicate", valid, true);
    }

    fn delete_profile(self: Pin<&mut Self>, id: &QString) {
        let id = String::from(id);
        let valid = if tcc_core::validate_profile_id(&tcc_core::ProfileId::new(id.clone())).is_err()
        {
            Err(SaveError::NotFound)
        } else {
            Ok(Command::DeleteProfile { id })
        };
        self.queue_profile_mutation("delete", valid, true);
    }

    fn export_profiles(self: Pin<&mut Self>, path: &QString) {
        let path = String::from(path);
        // Le chemin vient exclusivement du `FileDialog` ; il est revalidé ici
        // (absolu, borné, sans `..`) avant tout accès disque.
        let valid = match crate::profile_transfer::validate_transfer_path(&path) {
            Ok(_) => Ok(Command::ExportProfiles { path }),
            Err(error) => Err(error),
        };
        // L'export est une écriture locale non privilégiée : il reste possible
        // même si `tccd-admin` est indisponible.
        self.queue_profile_mutation("export", valid, false);
    }

    fn import_profiles(self: Pin<&mut Self>, path: &QString) {
        let path = String::from(path);
        let valid = match crate::profile_transfer::validate_transfer_path(&path) {
            Ok(_) => Ok(Command::ImportProfiles { path }),
            Err(error) => Err(error),
        };
        // L'import écrit la collection via `Admin1` : admin requis.
        self.queue_profile_mutation("import", valid, true);
    }

    fn set_autostart(mut self: Pin<&mut Self>, enabled: bool) {
        match crate::autostart::set_enabled_current(enabled) {
            Ok(()) => {
                self.as_mut().set_autostart_enabled(enabled);
                self.as_mut().set_autostart_error(QString::from(""));
            }
            Err(error) => {
                // L'état réel (fichier) n'a pas changé : on publie seulement
                // le code d'erreur, le Binding QML réapplique la valeur.
                self.as_mut()
                    .set_autostart_error(QString::from(error.code()));
            }
        }
    }

    /// Refus local d'une écriture `Admin.Hw` : code sûr, aucun succès
    /// optimiste, aucun appel D-Bus.
    fn fail_admin_hw_local(mut self: Pin<&mut Self>, code: &'static str) {
        self.as_mut().set_admin_hw_pending(false);
        self.as_mut().set_admin_hw_succeeded(false);
        self.as_mut().set_admin_hw_save_error(QString::from(code));
        self.as_mut()
            .set_admin_hw_save_error_detail(QString::from(""));
    }

    fn apply_odm(mut self: Pin<&mut Self>, name: &QString, force: bool) {
        let name = String::from(name).trim().to_owned();
        if !admin_hw_odm_name_valid(&name) {
            self.as_mut().fail_admin_hw_local("InvalidArgs");
            return;
        }
        self.as_mut()
            .queue_admin_hw("odm", Command::ApplyOdm { name, force });
    }

    fn apply_tdp(mut self: Pin<&mut Self>, tdp_json: &QString) {
        let raw = String::from(tdp_json);
        let values = if raw.len() > 1024 {
            None
        } else {
            serde_json::from_str::<Vec<i32>>(&raw).ok()
        };
        let Some(values) = values else {
            self.as_mut().fail_admin_hw_local("InvalidArgs");
            return;
        };
        if values.is_empty() || values.len() > MAX_TDP_VALUES {
            self.as_mut().fail_admin_hw_local("InvalidArgs");
            return;
        }
        if !admin_hw_tdp_values_valid(&values) {
            self.as_mut().fail_admin_hw_local("Validation");
            return;
        }
        self.as_mut()
            .queue_admin_hw("tdp", Command::ApplyTdp { values });
    }

    fn apply_ctgp(mut self: Pin<&mut Self>, watts: i32) {
        if !admin_hw_ctgp_valid(watts) {
            self.as_mut().fail_admin_hw_local("Validation");
            return;
        }
        self.as_mut()
            .queue_admin_hw("ctgp", Command::ApplyCtgp { watts });
    }

    fn refresh_agent(self: Pin<&mut Self>) {
        self.queue(Command::RefreshAgent);
    }

    /// Écriture UVC : validation locale stricte puis file bornée.
    ///
    /// Refus immédiat sans appel D-Bus si le chemin/clé ne respecte pas le
    /// contrat, si l'agent n'expose pas l'écriture, ou si une écriture est déjà
    /// en cours (le service sérialise par device et la GUI n'empile pas).
    fn set_uvc_control(
        mut self: Pin<&mut Self>,
        device_path: &QString,
        control_key: &QString,
        value: i32,
    ) {
        let device_path = String::from(device_path);
        let control_key = String::from(control_key);
        if !agent_uvc_device_path_valid(&device_path) {
            self.as_mut()
                .fail_agent_uvc_local(UvcWriteFailure::with_detail("InvalidArgs", "devicePath"));
            return;
        }
        if !agent_uvc_control_key_valid(&control_key) {
            self.as_mut()
                .fail_agent_uvc_local(UvcWriteFailure::with_detail("InvalidArgs", "controlKey"));
            return;
        }
        if !self.rust().agent_uvc_write_supported {
            self.as_mut()
                .fail_agent_uvc_local(UvcWriteFailure::new("NotSupported"));
            return;
        }
        if self.rust().agent_uvc_write_pending {
            self.as_mut()
                .fail_agent_uvc_local(UvcWriteFailure::new("Busy"));
            return;
        }

        self.as_mut().set_agent_uvc_write_succeeded(false);
        self.as_mut().set_agent_uvc_write_error(QString::from(""));
        self.as_mut().set_agent_uvc_write_detail(QString::from(""));
        self.as_mut().set_agent_uvc_write_pending(true);
        if let Err(error) = self.as_mut().try_queue(Command::SetUvcControl {
            device_path,
            control_key,
            value,
        }) {
            let code = if error == ClientError::Busy {
                "Busy"
            } else {
                "Unavailable"
            };
            self.as_mut().set_agent_uvc_write_pending(false);
            self.as_mut().set_agent_uvc_write_error(QString::from(code));
            if error.loses_connection() {
                self.as_mut().set_connected(false);
            }
        }
    }

    /// Refus local d'une écriture UVC : aucun succès optimiste, aucun appel.
    fn fail_agent_uvc_local(mut self: Pin<&mut Self>, failure: UvcWriteFailure) {
        self.as_mut().set_agent_uvc_write_pending(false);
        self.as_mut().set_agent_uvc_write_succeeded(false);
        self.as_mut()
            .set_agent_uvc_write_error(QString::from(failure.code));
        self.as_mut()
            .set_agent_uvc_write_detail(QString::from(failure.detail));
    }

    /// Garde commune des écritures `Admin.Hw` : une seule à la fois, refus
    /// immédiat sans file, publication de l'action.
    fn queue_admin_hw(mut self: Pin<&mut Self>, action: &'static str, command: Command) {
        if !self.rust().admin_hw_available {
            self.as_mut().set_admin_hw_pending(false);
            self.as_mut()
                .set_admin_hw_save_error(QString::from(SaveError::AdminUnavailable.code()));
            return;
        }
        if self.rust().admin_hw_pending {
            self.as_mut().set_admin_hw_pending(false);
            self.as_mut().set_admin_hw_save_error(QString::from("Busy"));
            return;
        }
        self.as_mut().set_admin_hw_action(QString::from(action));
        self.as_mut().set_admin_hw_pending(true);
        self.as_mut().set_admin_hw_succeeded(false);
        self.as_mut().set_admin_hw_save_error(QString::from(""));
        self.as_mut()
            .set_admin_hw_save_error_detail(QString::from(""));
        if let Err(error) = self.as_mut().try_queue(command) {
            let code = if error == ClientError::Busy {
                "Busy"
            } else {
                SaveError::AdminUnavailable.code()
            };
            self.as_mut().set_admin_hw_pending(false);
            self.as_mut().set_admin_hw_save_error(QString::from(code));
            if error.loses_connection() {
                self.as_mut().set_connected(false);
            }
        }
    }

    fn set_power_blocker(mut self: Pin<&mut Self>, blocked: bool) {
        // Refus immédiat si la session n'est pas graphique : aucun appel
        // logind, aucune file occupée.
        if blocked && !self.rust().power_blocker_available {
            self.as_mut().set_power_blocker_pending(false);
            self.as_mut()
                .set_power_blocker_error(QString::from(BlockerError::NotGraphical.code()));
            return;
        }
        self.as_mut().set_power_blocker_error(QString::from(""));
        self.as_mut().set_power_blocker_pending(true);
        if let Err(error) = self.as_mut().try_queue(Command::SetPowerBlocker(blocked)) {
            let code = if error == ClientError::Busy {
                "Busy"
            } else {
                BlockerError::Unavailable.code()
            };
            // SF-03 : sur refus de file pour une désactivation, le fd est
            // fermé immédiatement via le handle partagé. Une activation ratée
            // ne doit pas modifier l'état courant.
            let published = self
                .rust()
                .power_blocker_handle
                .as_ref()
                .map(|handle| recover_from_queue_failure(blocked, handle));
            if let Some(blocked_state) = published {
                self.as_mut().set_power_blocked(blocked_state);
            }
            self.as_mut().set_power_blocker_pending(false);
            self.as_mut().set_power_blocker_error(QString::from(code));
            if error.loses_connection() {
                self.as_mut().set_connected(false);
            }
        }
    }

    /// Garde commune aux mutations de collection : un seul enregistrement,
    /// refus immédiat sans file, `saveAction` publiée pour l'UI.
    fn queue_profile_mutation(
        mut self: Pin<&mut Self>,
        action: &'static str,
        command: Result<Command, SaveError>,
        requires_admin: bool,
    ) {
        let action = QString::from(action);
        self.as_mut().set_save_action(action.clone());
        self.as_mut().set_created_profile_id(QString::from(""));
        // Tout nouveau transfert repart de compteurs nuls : un résumé
        // précédent ne doit jamais être affiché pour l'opération en cours.
        self.as_mut().set_exported_count(0);
        self.as_mut().set_imported_count(0);
        self.as_mut().set_renamed_count(0);
        self.as_mut().set_skipped_count(0);

        let command = match command {
            Ok(command) => command,
            Err(error) => {
                self.as_mut().set_saving(false);
                self.as_mut().set_save_succeeded(false);
                self.as_mut().set_save_error(QString::from(error.code()));
                self.as_mut()
                    .set_save_error_detail(QString::from(error.detail().as_str()));
                self.as_mut().set_busy(false);
                return;
            }
        };

        if requires_admin && !self.rust().admin_available {
            self.as_mut().set_saving(false);
            self.as_mut().set_save_succeeded(false);
            self.as_mut()
                .set_save_error(QString::from(SaveError::AdminUnavailable.code()));
            self.as_mut().set_save_error_detail(QString::from(""));
            self.as_mut().set_busy(false);
            return;
        }

        // Un seul enregistrement à la fois (toutes mutations confondues).
        if self.rust().saving {
            let stalled = self
                .rust()
                .saving_since
                .is_some_and(|since| since.elapsed() > ADMIN_SAVE_STALL);
            if stalled {
                self.as_mut().set_saving(false);
                self.as_mut().set_save_succeeded(false);
                self.as_mut().set_save_error(QString::from("Timeout"));
                self.as_mut().set_save_error_detail(QString::from(""));
                return;
            }
            self.as_mut().set_save_error(QString::from("Busy"));
            self.as_mut().set_save_error_detail(QString::from(""));
            return;
        }

        self.as_mut().set_saving(true);
        self.as_mut().rust_mut().get_mut().saving_since = Some(Instant::now());
        self.as_mut().set_save_succeeded(false);
        self.as_mut().set_save_error(QString::from(""));
        self.as_mut().set_save_error_detail(QString::from(""));

        if let Err(error) = self.as_mut().try_queue(command) {
            let code = if error == ClientError::Busy {
                "Busy"
            } else {
                "Transport"
            };
            self.as_mut().set_saving(false);
            self.as_mut().rust_mut().get_mut().saving_since = None;
            self.as_mut().set_save_error(QString::from(code));
            self.as_mut().set_save_error_detail(QString::from(""));
        }
    }
}

impl ffi::DaemonClient {
    /// Empile une commande ; ne crée aucun thread et ne bloque jamais.
    fn queue(mut self: Pin<&mut Self>, command: Command) {
        match self.as_mut().try_queue(command) {
            Ok(()) => self.as_mut().set_busy(true),
            Err(error) => self.report_command_error(&error),
        }
    }

    /// Empile une commande et renvoie l'erreur sans toucher aux propriétés.
    fn try_queue(&self, command: Command) -> Result<(), ClientError> {
        let sender = self.rust().commands.clone();
        let Some(sender) = sender else {
            return Err(ClientError::WorkerStopped);
        };
        sender.try_send(command)
    }

    /// Signale un refus côté QML.
    ///
    /// Seules les erreurs qui impliquent la disparition du worker ou du daemon
    /// repassent la GUI en « déconnecté » ; une file pleine laisse l'état de
    /// connexion intact.
    fn report_command_error(self: Pin<&mut Self>, error: &ClientError) {
        let mut this = self;
        if error.loses_connection() {
            this.as_mut().set_connected(false);
        }
        this.as_mut().set_busy(error == &ClientError::Busy);
        this.as_mut()
            .set_last_error(QString::from(error.to_string().as_str()));
    }
}

/// Boucle du worker unique.
///
/// Attente bornée sur la file (`POLL_INTERVAL`) : une commande est traitée dès
/// qu'elle arrive, sinon l'état est relu périodiquement. Les commandes en
/// attente sont coalescées pour ne pas rejouer des intentions obsolètes.
fn worker_loop(
    qt_thread: cxx_qt::CxxQtThread<ffi::DaemonClient>,
    commands: Receiver<Command>,
    page_visible: Arc<AtomicBool>,
    window_visible: Arc<AtomicBool>,
    power_blocker: PowerBlocker<LogindInhibitor>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            // Le QML reste utilisable en mode dégradé et affiche la cause.
            let detail = error.to_string();
            let hardware_detail = detail.clone();
            let _ = qt_thread.queue(move |qobject| {
                apply_snapshot(
                    qobject,
                    Err(ClientError::Runtime { detail }),
                    None,
                    Err(ClientError::Runtime {
                        detail: hardware_detail,
                    }),
                    session_is_graphical(),
                );
            });
            return;
        }
    };

    // Collecte capteurs automatique : l'intention est toujours vraie, mais le
    // renouvellement exige **aussi** que le Dashboard soit visible ; sinon on
    // laisse le bail expirer. Aucun bouton utilisateur.
    let mut sensors_requested = true;
    let mut was_renewing = false;
    let mut disarm_requested = false;
    let mut paused_until: Option<Instant> = None;
    // Dernière collection validée : base des éditions (jamais le JSON brut).
    let mut last_profiles: Option<ProfileCollection> = None;
    // Derniers réglages validés : base des éditions de réglages.
    let mut last_settings: Option<Settings> = None;
    // Dernier instantané matériel décodé : capacités de charge (listes).
    let mut last_hardware: Option<HardwareView> = None;
    // Prochaine détection `Admin1` (0 = immédiate).
    let mut next_admin_probe: Option<Instant> = None;
    let mut next_hw_probe: Option<Instant> = None;
    let mut next_agent_probe: Option<Instant> = None;
    let mut agent_status: Option<SessionStatus> = None;
    let admin_service = DBUS_ADMIN_SERVICE;
    let admin_hw_service = DBUS_ADMIN_HW_SERVICE;
    let agent_service = DBUS_SESSION_AGENT_SERVICE;

    // Inhibiteur de veille : état possédé par le worker. Le fd est fermé à la
    // sortie de la boucle (arrêt de la GUI), ce qui libère l'inhibiteur.
    let blocker_available = session_is_graphical();
    let qt = qt_thread.clone();
    if qt
        .queue(move |qobject| apply_power_blocker_ready(qobject, blocker_available))
        .is_err()
    {
        return;
    }

    loop {
        // Aucune fusion : les commandes sont traitées **dans l'ordre** de la
        // file. Une coalescence par « dernière commande » pouvait jeter un
        // `SaveProfile` présent derrière un `Refresh` (G1-01), laissant
        // `saving` vrai et l'éditeur bloqué. La file est bornée (8), le coût
        // d'un traitement séquentiel est négligeable.
        let poll_interval = if window_visible.load(Ordering::Relaxed) {
            POLL_INTERVAL
        } else {
            HIDDEN_POLL_INTERVAL
        };
        let command = match commands.recv_timeout(poll_interval) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => Command::Refresh,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if matches!(command, Command::ProbeAdmin) {
            // « Réessayer » depuis Réglages ressonde aussi `Admin.Hw` : le
            // service peut avoir été installé/activé entre-temps.
            next_admin_probe = None;
            next_hw_probe = None;
        }
        let agent_refresh = matches!(command, Command::RefreshAgent);
        if agent_refresh {
            next_agent_probe = None;
        }
        if let Command::SetSensors(enabled) = command {
            sensors_requested = enabled;
            // Désactivation explicite : on envoie `false` immédiatement,
            // contrairement à la sortie du Dashboard.
            disarm_requested = !enabled;
        }

        // Inhibiteur de veille : une commande = un appel logind borné.
        let mut power_blocker_update = None;
        if let Command::SetPowerBlocker(blocked) = command {
            let result = if blocked && !blocker_available {
                Err(BlockerError::NotGraphical)
            } else {
                runtime.block_on(power_blocker.set_blocked(blocked))
            };
            power_blocker_update = Some((result, power_blocker.is_blocked()));
        }

        // Écritures Admin.Hw : transitoires, un appel borné à 180 s.
        let mut admin_hw_outcome: Option<(&'static str, Result<(), SaveError>)> = None;
        match &command {
            Command::ApplyOdm { name, force } => {
                admin_hw_outcome = Some((
                    "odm",
                    runtime.block_on(set_odm_on(bus_kind(), admin_hw_service, name, *force)),
                ));
                next_hw_probe = None;
            }
            Command::ApplyTdp { values } => {
                admin_hw_outcome = Some((
                    "tdp",
                    runtime.block_on(set_tdp_on(bus_kind(), admin_hw_service, values.clone())),
                ));
                next_hw_probe = None;
            }
            Command::ApplyCtgp { watts } => {
                admin_hw_outcome = Some((
                    "ctgp",
                    runtime.block_on(set_ctgp_on(bus_kind(), admin_hw_service, *watts)),
                ));
                next_hw_probe = None;
            }
            _ => {}
        }

        // Service de session `User1` : lecture seule, bus de session. La
        // sonde n'active pas le service ; seule une demande explicite
        // (`refreshAgent`) ou un service déjà possédé déclenche la lecture.
        let mut agent_update: Option<(SessionStatus, Option<Result<String, ClientError>>)> = None;
        let probe_due = next_agent_probe.is_none_or(|deadline| Instant::now() >= deadline);
        if probe_due {
            let status = runtime.block_on(probe_agent_on(BusKind::Session, agent_service));
            let display = if status.available && (status.owned || agent_refresh) {
                Some(runtime.block_on(fetch_display_info_on(BusKind::Session, agent_service)))
            } else {
                None
            };
            next_agent_probe = Some(Instant::now() + AGENT_PROBE_INTERVAL);
            agent_status = Some(status.clone());
            agent_update = Some((status, display));
        } else if let Some(status) = agent_status.as_ref() {
            if status.available && status.owned {
                let display =
                    Some(runtime.block_on(fetch_display_info_on(BusKind::Session, agent_service)));
                agent_update = Some((status.clone(), display));
            }
        }

        // Écriture UVC : un seul appel borné par commande. Après succès (valeur
        // écrite **ou** déjà courante), l'instantané matériel est relu pour
        // réafficher la valeur réelle — même si le bail capteurs est suspendu
        // (la page Outils n'est pas le Dashboard).
        let mut force_hardware = false;
        let mut uvc_outcome: Option<Result<bool, UvcWriteFailure>> = None;
        if let Command::SetUvcControl {
            device_path,
            control_key,
            value,
        } = &command
        {
            let result = runtime.block_on(set_uvc_control_on(
                BusKind::Session,
                agent_service,
                device_path,
                control_key,
                *value,
            ));
            if result.is_ok() {
                force_hardware = true;
            }
            // Le service a pu démarrer/expirer : la capacité d'écriture est
            // resondée au prochain cycle.
            next_agent_probe = None;
            uvc_outcome = Some(result);
        }

        // Enregistrement éventuel : applique le DTO sur la collection validée,
        // sérialise borné, appelle l'admin, puis relit l'état.
        let mut save_outcome = SaveOutcome::None;
        let mut created_profile_id: Option<String> = None;
        let mutation = match &command {
            Command::SaveProfile { id, edit_json } => Some(ProfileMutation::Save { id, edit_json }),
            Command::CreateProfile { edit_json } => Some(ProfileMutation::Create { edit_json }),
            Command::DuplicateProfile { source_id } => {
                Some(ProfileMutation::Duplicate { source_id })
            }
            Command::DeleteProfile { id } => Some(ProfileMutation::Delete { id }),
            _ => None,
        };
        if let Some(mutation) = mutation {
            let (outcome, created) = match last_profiles.as_ref() {
                None => (SaveOutcome::Err(SaveError::NoData), None),
                Some(base) => {
                    // Relecture juste avant l'envoi : refuse d'écraser un autre
                    // client (G1-03). La course résiduelle entre cette lecture
                    // et l'écriture admin reste ouverte (pas de CAS au contrat).
                    let current = runtime.block_on(fetch_profile_collection());
                    run_profile_mutation(base, current, &mutation, |json| {
                        runtime.block_on(save_profiles_remote(admin_service, json))
                    })
                }
            };
            save_outcome = outcome;
            created_profile_id = created;
            // Le statut admin a pu changer (activation, sortie après inactivité).
            next_admin_probe = None;
        }

        // Export : écriture locale non privilégiée, bornée à 1 Mio et validée
        // dans `profile_transfer`. Un seul transfert à la fois (`saving`).
        let mut exported_count: Option<usize> = None;
        if let Command::ExportProfiles { path } = &command {
            save_outcome = match last_profiles.as_ref() {
                None => SaveOutcome::Err(SaveError::NoData),
                Some(base) => {
                    match crate::profile_transfer::build_export(base).and_then(|(json, count)| {
                        crate::profile_transfer::write_export_file(path, &json).map(|()| count)
                    }) {
                        Ok(count) => {
                            exported_count = Some(count);
                            SaveOutcome::Ok
                        }
                        Err(error) => SaveOutcome::Err(error),
                    }
                }
            };
            next_admin_probe = None;
        }

        // Import : lecture et parse stricts dans le worker (jamais en QML),
        // puis même relecture/`Conflict` que le CRUD avant l'écriture admin.
        let mut import_summary: Option<ImportSummary> = None;
        if let Command::ImportProfiles { path } = &command {
            let imported = crate::profile_transfer::read_import_file(path);
            save_outcome = match (last_profiles.as_ref(), imported) {
                (None, _) => SaveOutcome::Err(SaveError::NoData),
                (Some(_), Err(error)) => SaveOutcome::Err(error),
                (Some(base), Ok(imported)) => {
                    let current = runtime.block_on(fetch_profile_collection());
                    let (outcome, summary) =
                        run_import_mutation(base, current, &imported, |json| {
                            runtime.block_on(save_profiles_remote(admin_service, json))
                        });
                    import_summary = summary;
                    outcome
                }
            };
            next_admin_probe = None;
        }

        // Enregistrement des réglages : même pipeline que les profils.
        let mut save_settings_outcome = SaveOutcome::None;
        if let Command::SaveSettings { edit_json } = &command {
            save_settings_outcome = match last_settings.as_ref() {
                None => SaveOutcome::Err(SaveError::NoData),
                Some(base) => {
                    let current = runtime.block_on(fetch_settings());
                    // Les listes autorisées viennent du **dernier** instantané
                    // matériel lu par le worker, jamais du DTO QML.
                    let capabilities = last_hardware
                        .as_ref()
                        .map(|view| ChargeCapabilities::from_charging(&view.charging));
                    run_save_settings(
                        base,
                        current,
                        edit_json,
                        capabilities.as_ref(),
                        last_profiles.as_ref(),
                        |json| runtime.block_on(save_settings_remote(admin_service, json)),
                    )
                }
            };
            next_admin_probe = None;
        }

        // Écriture éventuelle (override temporaire), puis lecture de l'état.
        let write = match &command {
            Command::SetTempProfile(id) => runtime.block_on(apply_temp_profile(id)).map(|_| ()),
            Command::ClearTempProfile => runtime.block_on(clear_temp_profile()).map(|_| ()),
            Command::Refresh
            | Command::SetSensors(_)
            | Command::SaveProfile { .. }
            | Command::CreateProfile { .. }
            | Command::DuplicateProfile { .. }
            | Command::DeleteProfile { .. }
            | Command::ExportProfiles { .. }
            | Command::ImportProfiles { .. }
            | Command::SetPowerBlocker(_)
            | Command::ApplyOdm { .. }
            | Command::ApplyTdp { .. }
            | Command::ApplyCtgp { .. }
            | Command::SaveSettings { .. }
            | Command::ProbeAdmin
            | Command::RefreshAgent
            | Command::SetUvcControl { .. } => Ok(()),
        };
        let mut core = match write {
            Ok(()) => runtime.block_on(fetch_snapshot()),
            Err(error) => Err(error),
        };
        // La collection relue devient la base des prochaines éditions.
        if let Ok(snapshot) = &mut core {
            if let Some(profiles) = snapshot.profiles.take() {
                last_profiles = Some(profiles);
            }
        }

        let (renewing, next_pause) = next_lease_state(
            Instant::now(),
            sensors_requested,
            page_visible.load(Ordering::Relaxed) && window_visible.load(Ordering::Relaxed),
            was_renewing,
            paused_until,
            disarm_requested,
        );
        paused_until = next_pause;
        was_renewing = renewing;

        // Pendant la fenêtre de grâce on ne lit pas l'instantané : c'est la
        // lecture qui réarmerait le bail. Aucune commande `false` n'est émise.
        // Une écriture UVC réussie force une relecture unique (la valeur réelle
        // doit être affichée même hors Dashboard), sans réarmer le bail.
        let hardware = if paused_until.is_some() && !force_hardware {
            None
        } else {
            let result = runtime.block_on(fetch_hardware(renewing, disarm_requested));
            disarm_requested = false;
            Some(match result {
                Ok((view, json)) => {
                    // Conservé pour valider les éditions de charge même si le
                    // bail capteurs est ensuite suspendu.
                    last_hardware = Some(view);
                    Ok(json)
                }
                Err(error) => Err(error),
            })
        };

        // Les ventilateurs ne touchent pas au bail : lecture à chaque cycle.
        let fan = runtime.block_on(fetch_fans());

        // Réglages globaux : relus à chaque cycle (l'UI reflète les
        // changements d'un autre client) et conservés comme base d'édition.
        let mut settings = runtime.block_on(fetch_settings());
        if let (Ok((value, _json)), Some(profiles)) = (&settings, last_profiles.as_ref()) {
            // Un `Settings` valide peut référencer un profil supprimé par un
            // autre client : l'UI doit alors repasser en lecture seule.
            if let Err(error) = validate_settings_against_profiles(value, profiles) {
                settings = Err(error);
            }
        }
        if let Ok((value, _json)) = &settings {
            last_settings = Some(value.clone());
        }

        // Minuterie d'extinction : lecture seule Manager1, à chaque cycle.
        // Un échec n'interrompt pas les autres lectures.
        let shutdown_timer = runtime.block_on(fetch_shutdown_timer());

        // Détection `Admin1` : sans activation D-Bus à chaque cycle.
        let mut admin_status = None;
        if next_admin_probe.is_none_or(|deadline| Instant::now() >= deadline) {
            admin_status = Some(runtime.block_on(probe_admin(admin_service)));
            next_admin_probe = Some(Instant::now() + ADMIN_PROBE_INTERVAL);
        }
        let mut admin_hw_status = None;
        if next_hw_probe.is_none_or(|deadline| Instant::now() >= deadline) {
            admin_hw_status =
                Some(runtime.block_on(probe_admin_hw_on(bus_kind(), admin_hw_service)));
            next_hw_probe = Some(Instant::now() + ADMIN_PROBE_INTERVAL);
        }

        let published = qt_thread
            .clone()
            .queue(move |qobject| apply_snapshot(qobject, core, hardware, fan, blocker_available))
            .is_ok();
        if !published {
            break;
        }
        if let Some(status) = admin_status {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_admin(qobject, status))
                .is_err()
            {
                break;
            }
        }
        if let Some(status) = admin_hw_status {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_admin_hw(qobject, status))
                .is_err()
            {
                break;
            }
        }
        let qt = qt_thread.clone();
        if qt
            .queue(move |qobject| apply_settings(qobject, settings))
            .is_err()
        {
            break;
        }
        let qt = qt_thread.clone();
        if qt
            .queue(move |qobject| apply_shutdown_timer(qobject, shutdown_timer))
            .is_err()
        {
            break;
        }
        if let Some((result, blocked)) = power_blocker_update {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_power_blocker(qobject, result, blocked))
                .is_err()
            {
                break;
            }
        }
        if let Some((action, result)) = admin_hw_outcome {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_admin_hw_save(qobject, action, result))
                .is_err()
            {
                break;
            }
        }
        if let Some((status, display)) = agent_update {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_agent(qobject, status, display))
                .is_err()
            {
                break;
            }
        }
        // Publié après l'instantané : l'UI réactive les contrôles avec la
        // valeur réelle déjà relue, sans clignotement de l'ancienne valeur.
        if let Some(result) = uvc_outcome {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_agent_uvc_write(qobject, result))
                .is_err()
            {
                break;
            }
        }
        if !matches!(save_outcome, SaveOutcome::None) {
            let qt = qt_thread.clone();
            if qt
                .queue(move |mut qobject| {
                    apply_save(qobject.as_mut(), save_outcome, created_profile_id);
                    if let Some(count) = exported_count {
                        qobject.as_mut().set_exported_count(count_as_i32(count));
                    }
                    if let Some(summary) = import_summary {
                        qobject
                            .as_mut()
                            .set_imported_count(count_as_i32(summary.imported));
                        qobject
                            .as_mut()
                            .set_renamed_count(count_as_i32(summary.renamed));
                        qobject
                            .as_mut()
                            .set_skipped_count(count_as_i32(summary.skipped));
                    }
                })
                .is_err()
            {
                break;
            }
        }
        if !matches!(save_settings_outcome, SaveOutcome::None) {
            let qt = qt_thread.clone();
            if qt
                .queue(move |qobject| apply_save_settings(qobject, save_settings_outcome))
                .is_err()
            {
                break;
            }
        }
    }
}

/// Décide du renouvellement du bail et de la fenêtre de suspension.
///
/// - `renewing` = collecte automatique **et** Dashboard affiché (page active
///   + fenêtre visible) ;
/// - à la sortie du renouvellement (hors désactivation explicite), on suspend
///   les lectures matérielles pendant [`SENSOR_LEASE_GRACE`] pour laisser le
///   bail expirer côté daemon (10 s) sans envoyer `false` ;
/// - une désactivation explicite annule la suspension et envoie `false`.
fn next_lease_state(
    now: Instant,
    sensors_requested: bool,
    page_visible: bool,
    was_renewing: bool,
    paused_until: Option<Instant>,
    disarm_requested: bool,
) -> (bool, Option<Instant>) {
    let renewing = sensors_requested && page_visible;

    if disarm_requested || renewing {
        return (renewing, None);
    }
    if was_renewing {
        return (renewing, Some(now + SENSOR_LEASE_GRACE));
    }
    (renewing, paused_until.filter(|deadline| now < *deadline))
}

/// Mutation de collection demandée par QML.
enum ProfileMutation<'a> {
    Save { id: &'a str, edit_json: &'a str },
    Create { edit_json: &'a str },
    Duplicate { source_id: &'a str },
    Delete { id: &'a str },
}

/// Exécute une mutation et **renvoie toujours un résultat**.
///
/// `current` est la relecture de la collection juste avant l'envoi : toute
/// différence avec `base` annule l'écriture (`Conflict`). Toutes les branches
/// produisent un [`SaveOutcome`], ce qui garantit que `saving` est remis à faux
/// après chaque demande acceptée (G1-01). Le second membre est l'ID du profil
/// créé/dupliqué, pour sélectionner l'éditeur.
fn run_profile_mutation<F>(
    base: &ProfileCollection,
    current: Result<ProfileCollection, ClientError>,
    mutation: &ProfileMutation<'_>,
    save: F,
) -> (SaveOutcome, Option<String>)
where
    F: FnOnce(&str) -> Result<(), SaveError>,
{
    let current = match current {
        Ok(current) => current,
        Err(error) => return (SaveOutcome::Err(client_error_to_save(error)), None),
    };
    let prepared = match mutation {
        ProfileMutation::Save { id, edit_json } => prepare_save(base, &current, id, edit_json),
        ProfileMutation::Create { edit_json } => prepare_create(base, &current, edit_json),
        ProfileMutation::Duplicate { source_id } => prepare_duplicate(base, &current, source_id),
        ProfileMutation::Delete { id } => prepare_delete(base, &current, id),
    };
    match prepared {
        Err(error) => (SaveOutcome::Err(error), None),
        Ok(prepared) => match save(&prepared.json) {
            Ok(()) => (
                SaveOutcome::Ok,
                prepared.new_profile_id.map(|id| id.as_str().to_owned()),
            ),
            Err(error) => (SaveOutcome::Err(error), None),
        },
    }
}

/// Exécute un import et **renvoie toujours un résultat** (même garantie que
/// [`run_profile_mutation`] pour `saving`).
///
/// `current` est la relecture juste avant l'envoi ; toute différence avec
/// `base` annule l'écriture (`Conflict`). `prepare_import` filtre les presets,
/// résout les conflits d'ID/nom et valide la collection complète.
fn run_import_mutation<F>(
    base: &ProfileCollection,
    current: Result<ProfileCollection, ClientError>,
    imported: &[Profile],
    save: F,
) -> (SaveOutcome, Option<ImportSummary>)
where
    F: FnOnce(&str) -> Result<(), SaveError>,
{
    let current = match current {
        Ok(current) => current,
        Err(error) => return (SaveOutcome::Err(client_error_to_save(error)), None),
    };
    match crate::profile_transfer::prepare_import(base, &current, imported) {
        Err(error) => (SaveOutcome::Err(error), None),
        Ok((prepared, summary)) => match save(&prepared.json) {
            Ok(()) => (SaveOutcome::Ok, Some(summary)),
            Err(error) => (SaveOutcome::Err(error), None),
        },
    }
}

/// Compteur d'import/export borné à `i32` (propriétés QML).
fn count_as_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Convertit une erreur de lecture (relecture pré-enregistrement) en erreur
/// d'enregistrement.
fn client_error_to_save(error: ClientError) -> SaveError {
    match error {
        ClientError::Timeout { .. } => SaveError::Timeout,
        _ => SaveError::Transport,
    }
}

/// Pipeline d'enregistrement des réglages : renvoie **toujours** un résultat.
fn run_save_settings<F>(
    base: &Settings,
    current: Result<(Settings, String), ClientError>,
    edit_json: &str,
    capabilities: Option<&ChargeCapabilities>,
    profiles: Option<&ProfileCollection>,
    save: F,
) -> SaveOutcome
where
    F: FnOnce(&str) -> Result<(), SaveError>,
{
    let current = match current {
        Ok((settings, _json)) => settings,
        Err(error) => return SaveOutcome::Err(client_error_to_save(error)),
    };
    match prepare_settings_save(base, &current, edit_json, capabilities, profiles) {
        Err(error) => SaveOutcome::Err(error),
        Ok(prepared) => match save(&prepared.json) {
            Ok(()) => SaveOutcome::Ok,
            Err(error) => SaveOutcome::Err(error),
        },
    }
}

/// Valeur de matrice `ppd/power` (`""` si la clé est absente).
fn matrix_value(settings: &Settings, key: &str) -> QString {
    StateKey::from_matrix_key(key)
        .and_then(|state_key| settings.state_matrix.get(&state_key))
        .map_or_else(|| QString::from(""), |id| QString::from(id.as_str()))
}

/// Publie les réglages relus (JSON canonique + champs affichés).
fn apply_settings(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    result: Result<(Settings, String), ClientError>,
) {
    match result {
        Ok((settings, json)) => {
            qobject
                .as_mut()
                .set_settings_json(QString::from(json.as_str()));
            qobject
                .as_mut()
                .set_settings_fahrenheit(settings.fahrenheit);
            qobject
                .as_mut()
                .set_settings_cpu_enabled(settings.cpu_settings_enabled);
            qobject
                .as_mut()
                .set_settings_fan_enabled(settings.fan_control_enabled);
            qobject
                .as_mut()
                .set_settings_lighting_enabled(settings.keyboard_backlight_control_enabled);
            qobject
                .as_mut()
                .set_settings_ppd_override(settings.ppd_override);
            qobject
                .as_mut()
                .set_settings_charging_profile(QString::from(
                    settings.charging_profile.as_deref().unwrap_or(""),
                ));
            qobject.as_mut().set_settings_charging_type(QString::from(
                settings.charging_type.as_deref().unwrap_or(""),
            ));
            qobject.as_mut().set_settings_keyboard_mode(QString::from(
                settings
                    .keyboard_states
                    .first()
                    .map(|state| state.mode.as_str())
                    .unwrap_or(""),
            ));
            qobject
                .as_mut()
                .set_settings_charging_priority(QString::from(
                    settings.charging_priority.as_deref().unwrap_or(""),
                ));
            qobject.as_mut().set_settings_charging_start(
                settings.charging_start_threshold.map_or(-1, i32::from),
            );
            qobject
                .as_mut()
                .set_settings_charging_end(settings.charging_end_threshold.map_or(-1, i32::from));
            qobject
                .as_mut()
                .set_settings_matrix_power_saver_ac(matrix_value(&settings, "power-saver/ac"));
            qobject
                .as_mut()
                .set_settings_matrix_power_saver_bat(matrix_value(
                    &settings,
                    "power-saver/battery",
                ));
            qobject
                .as_mut()
                .set_settings_matrix_balanced_ac(matrix_value(&settings, "balanced/ac"));
            qobject
                .as_mut()
                .set_settings_matrix_balanced_bat(matrix_value(&settings, "balanced/battery"));
            qobject
                .as_mut()
                .set_settings_matrix_performance_ac(matrix_value(&settings, "performance/ac"));
            qobject
                .as_mut()
                .set_settings_matrix_performance_bat(matrix_value(
                    &settings,
                    "performance/battery",
                ));
            qobject.as_mut().set_settings_error(QString::from(""));
        }
        Err(error) => {
            // Dernier payload conservé : l'UI affiche un bandeau, pas des
            // interrupteurs qui changent tout seuls.
            qobject
                .as_mut()
                .set_settings_error(QString::from(error.to_string().as_str()));
        }
    }
}

/// Publie le statut réel de la minuterie.
fn apply_shutdown_timer(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    result: Result<ShutdownTimerView, ClientError>,
) {
    match result {
        Ok(view) => {
            qobject.as_mut().set_shutdown_timer_available(true);
            qobject.as_mut().set_shutdown_timer_enabled(view.enabled);
            qobject
                .as_mut()
                .set_shutdown_timer_scheduled_at(QString::from(
                    view.scheduled_at.as_deref().unwrap_or(""),
                ));
            qobject
                .as_mut()
                .set_shutdown_timer_source(QString::from(view.source.as_str()));
            qobject
                .as_mut()
                .set_shutdown_timer_error(QString::from(view.error.as_deref().unwrap_or("")));
        }
        Err(error) => {
            // Statut indisponible : l'UI désactive l'édition et affiche une
            // erreur générique, sans conserver un état planifié trompeur.
            qobject.as_mut().set_shutdown_timer_available(false);
            qobject.as_mut().set_shutdown_timer_enabled(false);
            qobject
                .as_mut()
                .set_shutdown_timer_scheduled_at(QString::from(""));
            qobject
                .as_mut()
                .set_shutdown_timer_source(QString::from(""));
            qobject
                .as_mut()
                .set_shutdown_timer_error(QString::from(error.to_string().as_str()));
        }
    }
}

/// Publie l'état initial de l'inhibiteur de veille.
fn apply_power_blocker_ready(mut qobject: Pin<&mut ffi::DaemonClient>, available: bool) {
    qobject.as_mut().set_power_blocker_available(available);
    qobject.as_mut().set_power_blocked(false);
    qobject.as_mut().set_power_blocker_pending(false);
    qobject.as_mut().set_power_blocker_error(QString::from(""));
}

/// Publie le résultat d'une activation/désactivation de l'inhibiteur.
fn apply_power_blocker(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    result: Result<(), BlockerError>,
    blocked: bool,
) {
    match result {
        Ok(()) => {
            qobject.as_mut().set_power_blocked(blocked);
            qobject.as_mut().set_power_blocker_error(QString::from(""));
        }
        Err(error) => {
            // L'état réel (fd) n'a pas changé : le Binding QML réapplique
            // `powerBlocked` et l'erreur est affichée.
            qobject
                .as_mut()
                .set_power_blocker_error(QString::from(error.code()));
        }
    }
    qobject.as_mut().set_power_blocker_pending(false);
}

/// Publie l'état du service `Admin.Hw`.
fn apply_admin_hw(mut qobject: Pin<&mut ffi::DaemonClient>, status: AdminStatus) {
    qobject.as_mut().set_admin_hw_available(status.available);
    qobject
        .as_mut()
        .set_admin_hw_version(QString::from(status.version.as_str()));
    qobject
        .as_mut()
        .set_admin_hw_error(QString::from(status.error));
}

/// Publie le résultat d'une écriture `Admin.Hw`.
fn apply_admin_hw_save(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    action: &'static str,
    result: Result<(), SaveError>,
) {
    qobject.as_mut().set_admin_hw_action(QString::from(action));
    qobject.as_mut().set_admin_hw_pending(false);
    match result {
        Ok(()) => {
            qobject.as_mut().set_admin_hw_save_error(QString::from(""));
            qobject
                .as_mut()
                .set_admin_hw_save_error_detail(QString::from(""));
            qobject.as_mut().set_admin_hw_succeeded(true);
        }
        Err(error) => {
            qobject
                .as_mut()
                .set_admin_hw_save_error(QString::from(error.code()));
            qobject
                .as_mut()
                .set_admin_hw_save_error_detail(QString::from(error.detail().as_str()));
            qobject.as_mut().set_admin_hw_succeeded(false);
        }
    }
}

/// Publie l'état du service de session et le rapport d'affichage.
fn apply_agent(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    status: SessionStatus,
    display: Option<Result<String, ClientError>>,
) {
    qobject.as_mut().set_agent_available(status.available);
    qobject
        .as_mut()
        .set_agent_version(QString::from(status.version.as_str()));
    qobject
        .as_mut()
        .set_agent_uvc_write_supported(status.uvc_write_supported);
    let mut error = status.error;
    match display {
        Some(Ok(json)) => {
            qobject
                .as_mut()
                .set_display_info_json(QString::from(json.as_str()));
        }
        Some(Err(fetch_error)) => {
            qobject.as_mut().set_display_info_json(QString::from("{}"));
            if error.is_empty() {
                error = agent_error_code(&fetch_error);
            }
        }
        None => {
            // Service endormi/absent : aucun mode réel n'est publié.
            if !status.available || !status.owned {
                qobject.as_mut().set_display_info_json(QString::from("{}"));
            }
        }
    }
    qobject.as_mut().set_agent_error(QString::from(error));
}

/// Publie le résultat d'une écriture UVC.
///
/// `pending` est remis à faux **après** le code d'erreur : les éditeurs QML se
/// réactivent avec l'erreur déjà visible et la valeur relue.
fn apply_agent_uvc_write(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    result: Result<bool, UvcWriteFailure>,
) {
    match result {
        Ok(_changed) => {
            qobject
                .as_mut()
                .set_agent_uvc_write_error(QString::from(""));
            qobject
                .as_mut()
                .set_agent_uvc_write_detail(QString::from(""));
            qobject.as_mut().set_agent_uvc_write_succeeded(true);
        }
        Err(failure) => {
            qobject
                .as_mut()
                .set_agent_uvc_write_error(QString::from(failure.code));
            qobject
                .as_mut()
                .set_agent_uvc_write_detail(QString::from(failure.detail));
            qobject.as_mut().set_agent_uvc_write_succeeded(false);
        }
    }
    qobject.as_mut().set_agent_uvc_write_pending(false);
}

/// Publie le résultat d'un enregistrement de réglages.
fn apply_save_settings(mut qobject: Pin<&mut ffi::DaemonClient>, outcome: SaveOutcome) {
    match outcome {
        SaveOutcome::None => {}
        SaveOutcome::Ok => {
            qobject.as_mut().set_saving_settings(false);
            qobject.as_mut().rust_mut().get_mut().saving_settings_since = None;
            qobject.as_mut().set_save_settings_error(QString::from(""));
            qobject
                .as_mut()
                .set_save_settings_error_detail(QString::from(""));
            qobject.as_mut().set_save_settings_succeeded(true);
        }
        SaveOutcome::Err(error) => {
            qobject.as_mut().set_saving_settings(false);
            qobject.as_mut().rust_mut().get_mut().saving_settings_since = None;
            qobject
                .as_mut()
                .set_save_settings_error(QString::from(error.code()));
            qobject
                .as_mut()
                .set_save_settings_error_detail(QString::from(error.detail().as_str()));
            qobject.as_mut().set_save_settings_succeeded(false);
        }
    }
}

fn apply_snapshot(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    result: Result<SnapshotData, ClientError>,
    hardware: Option<Result<String, ClientError>>,
    fan: Result<String, ClientError>,
    blocker_available: bool,
) {
    // Reprogrammé à chaque cycle : le handler QML peut ne pas encore exister
    // lors de la première publication.
    qobject
        .as_mut()
        .set_power_blocker_available(blocker_available);
    match result {
        Ok(snapshot) => {
            // Ordre important : `stateJson` est modifié en dernier pour que le
            // handler QML voie toutes les propriétés dérivées à jour.
            qobject
                .as_mut()
                .set_active_profile_id(QString::from(snapshot.active_id.as_str()));
            qobject
                .as_mut()
                .set_active_profile_name(QString::from(snapshot.active_name.as_str()));
            qobject
                .as_mut()
                .set_ppd_profile(QString::from(snapshot.ppd.as_str()));
            qobject
                .as_mut()
                .set_power_source(QString::from(snapshot.power.as_str()));
            qobject
                .as_mut()
                .set_manual_override(snapshot.manual_override);
            qobject.as_mut().set_revision(snapshot.revision);
            qobject
                .as_mut()
                .set_daemon_version(QString::from(snapshot.daemon_version.as_str()));
            qobject
                .as_mut()
                .set_contract_version(snapshot.contract_version);
            qobject.as_mut().set_connected(true);
            qobject.as_mut().set_last_error(QString::from(""));
            qobject
                .as_mut()
                .set_profiles_json(QString::from(snapshot.profiles_json.as_str()));
            qobject
                .as_mut()
                .set_state_json(QString::from(snapshot.state_json.as_str()));
        }
        Err(error) => {
            // Un refus sémantique ne signifie pas que le daemon est absent.
            if error.loses_connection() {
                qobject.as_mut().set_connected(false);
            }
            qobject.as_mut().set_daemon_version(QString::from(""));
            qobject.as_mut().set_contract_version(0);
            qobject
                .as_mut()
                .set_last_error(QString::from(error.to_string().as_str()));
        }
    }

    // Matériel : erreur séparée pour ne pas faire croire que tout le daemon
    // est tombé quand seul `Manager1` échoue (daemon ancien, par exemple).
    // `None` = lecture suspendue volontairement (bail laissé expirer), on ne
    // touche à aucune propriété.
    match hardware {
        Some(Ok(json)) => {
            qobject
                .as_mut()
                .set_hardware_json(QString::from(json.as_str()));
            qobject.as_mut().set_hardware_error(QString::from(""));
        }
        Some(Err(error)) => {
            // On conserve le dernier payload valide : l'UI affiche les valeurs
            // précédentes avec le badge d'erreur, jamais des zéros trompeurs.
            qobject
                .as_mut()
                .set_hardware_error(QString::from(error.to_string().as_str()));
        }
        None => {}
    }

    // Ventilateurs : lecture seule, erreur isolée.
    match fan {
        Ok(json) => {
            qobject.as_mut().set_fan_json(QString::from(json.as_str()));
            qobject.as_mut().set_fan_error(QString::from(""));
        }
        Err(error) => {
            qobject
                .as_mut()
                .set_fan_error(QString::from(error.to_string().as_str()));
        }
    }

    qobject.as_mut().set_busy(false);
}

/// Publie l'état du service d'administration.
fn apply_admin(mut qobject: Pin<&mut ffi::DaemonClient>, status: AdminStatus) {
    qobject.as_mut().set_admin_available(status.available);
    qobject
        .as_mut()
        .set_admin_version(QString::from(status.version.as_str()));
    qobject
        .as_mut()
        .set_admin_error(QString::from(status.error));
}

/// Publie le résultat d'un enregistrement.
fn apply_save(
    mut qobject: Pin<&mut ffi::DaemonClient>,
    outcome: SaveOutcome,
    created_profile_id: Option<String>,
) {
    match outcome {
        SaveOutcome::None => {}
        SaveOutcome::Ok => {
            qobject.as_mut().set_saving(false);
            qobject.as_mut().rust_mut().get_mut().saving_since = None;
            qobject.as_mut().set_save_error(QString::from(""));
            qobject.as_mut().set_save_error_detail(QString::from(""));
            qobject
                .as_mut()
                .set_created_profile_id(QString::from(created_profile_id.as_deref().unwrap_or("")));
            qobject.as_mut().set_save_succeeded(true);
        }
        SaveOutcome::Err(error) => {
            qobject.as_mut().set_saving(false);
            qobject.as_mut().rust_mut().get_mut().saving_since = None;
            qobject.as_mut().set_save_error(QString::from(error.code()));
            qobject
                .as_mut()
                .set_save_error_detail(QString::from(error.detail().as_str()));
            qobject.as_mut().set_save_succeeded(false);
        }
    }
}

#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd",
    default_service = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Tccd {
    #[zbus(name = "GetVersion")]
    fn get_version(&self) -> zbus::Result<String>;
    #[zbus(property, name = "ContractVersion")]
    fn contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "GetStateJSON")]
    fn get_state_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetProfilesJSON")]
    fn get_profiles_json(&self) -> zbus::Result<String>;
    #[zbus(name = "SetTempProfile")]
    fn set_temp_profile(&self, id: &str) -> zbus::Result<bool>;
    #[zbus(name = "ClearTempProfile")]
    fn clear_temp_profile(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetFanDataJSON")]
    fn get_fan_data_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetSettingsJSON")]
    fn get_settings_json(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.Manager1",
    default_service = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Manager1 {
    #[zbus(name = "GetHardwareSnapshotJSON")]
    fn get_hardware_snapshot_json(&self) -> zbus::Result<String>;
    #[zbus(name = "SetSensorDataCollectionStatus")]
    fn set_sensor_data_collection_status(&self, status: bool) -> zbus::Result<bool>;
    #[zbus(name = "GetShutdownTimerJSON")]
    fn get_shutdown_timer_json(&self) -> zbus::Result<String>;
}

/// Interface `Admin1` (phase 1), sur un service donné.
///
/// Le service est paramétrable pour les tests (nom unique sur le bus de
/// session) ; la production passe [`tcc_ipc::DBUS_ADMIN_SERVICE`].
#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.Admin1",
    default_service = "com.tuxedocomputers.tccd.Admin",
    default_path = "/com/tuxedocomputers/tccd/Admin"
)]
trait Admin1 {
    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetVersion")]
    fn get_version(&self) -> zbus::Result<String>;
    #[zbus(name = "GetContractVersion")]
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "SaveProfilesJSON")]
    fn save_profiles_json(&self, json: &str) -> zbus::Result<bool>;
    #[zbus(name = "SaveSettingsJSON")]
    fn save_settings_json(&self, json: &str) -> zbus::Result<bool>;
}

/// Interface de session `User1` (lecture seule, bus de session uniquement).
#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.User1",
    default_service = "com.tuxedocomputers.tccd.User",
    default_path = "/com/tuxedocomputers/tccd/User"
)]
trait SessionAgent1 {
    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetVersion")]
    fn get_version(&self) -> zbus::Result<String>;
    #[zbus(name = "GetContractVersion")]
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "GetDisplayInfoJSON")]
    fn get_display_info_json(&self) -> zbus::Result<String>;
    #[zbus(name = "GetUvcWriteSupported")]
    fn get_uvc_write_supported(&self) -> zbus::Result<bool>;
    #[zbus(name = "SetUvcControl")]
    fn set_uvc_control(
        &self,
        device_path: &str,
        control_key: &str,
        value: i32,
    ) -> zbus::Result<bool>;
}

async fn make_session_proxy_on<'a>(
    connection: &'a zbus::Connection,
    service: &str,
) -> Result<SessionAgent1Proxy<'a>, ClientError> {
    SessionAgent1Proxy::builder(connection)
        .destination(service.to_owned())
        .map_err(|error| ClientError::Transport {
            call: "destination User1",
            detail: error.to_string(),
        })?
        .path(DBUS_SESSION_AGENT_PATH)
        .map_err(|error| ClientError::Transport {
            call: "objet User1",
            detail: error.to_string(),
        })?
        .build()
        .await
        .map_err(|error| ClientError::Transport {
            call: "proxy User1",
            detail: error.to_string(),
        })
}

async fn make_admin_proxy<'a>(
    connection: &'a zbus::Connection,
    service: &str,
) -> Result<Admin1Proxy<'a>, ClientError> {
    Admin1Proxy::builder(connection)
        .destination(service.to_owned())
        .map_err(|error| ClientError::Transport {
            call: "destination Admin1",
            detail: error.to_string(),
        })?
        .path(DBUS_ADMIN_PATH)
        .map_err(|error| ClientError::Transport {
            call: "objet Admin1",
            detail: error.to_string(),
        })?
        .build()
        .await
        .map_err(|error| ClientError::Transport {
            call: "proxy Admin1",
            detail: error.to_string(),
        })
}

/// Interface `Admin.Hw` phase 2 (ODM/TDP/cTGP).
#[zbus::proxy(
    interface = "com.tuxedocomputers.tccd.AdminHw1",
    default_service = "com.tuxedocomputers.tccd.Admin.Hw",
    default_path = "/com/tuxedocomputers/tccd/Admin/Hw"
)]
trait AdminHw1 {
    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetVersion")]
    fn get_version(&self) -> zbus::Result<String>;
    #[zbus(name = "GetContractVersion")]
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "GetTdpValues")]
    fn get_tdp_values(&self) -> zbus::Result<Vec<i32>>;
    #[zbus(name = "SetTdpValues")]
    fn set_tdp_values(&self, values: Vec<i32>) -> zbus::Result<bool>;
    #[zbus(name = "SetOdmProfile")]
    fn set_odm_profile(&self, name: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetOdmProfileOverride")]
    fn set_odm_profile_override(&self, name: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetNvidiaCtgpOffset")]
    fn set_nvidia_ctgp_offset(&self, watts: i32) -> zbus::Result<bool>;
}

async fn make_admin_hw_proxy_on<'a>(
    connection: &'a zbus::Connection,
    service: &str,
) -> Result<AdminHw1Proxy<'a>, ClientError> {
    AdminHw1Proxy::builder(connection)
        .destination(service.to_owned())
        .map_err(|error| ClientError::Transport {
            call: "destination Admin.Hw",
            detail: error.to_string(),
        })?
        .path(DBUS_ADMIN_HW_PATH)
        .map_err(|error| ClientError::Transport {
            call: "objet Admin.Hw",
            detail: error.to_string(),
        })?
        .build()
        .await
        .map_err(|error| ClientError::Transport {
            call: "proxy Admin.Hw",
            detail: error.to_string(),
        })
}

/// Détecte `Admin.Hw` sans l'activer (sinon il démarrerait en root).
async fn probe_admin_hw_on(kind: BusKind, service: &str) -> AdminStatus {
    let connection = match connection_for(kind).await {
        Ok(connection) => connection,
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    let dbus = match zbus::fdo::DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    let Ok(name) = zbus::names::BusName::try_from(service.to_owned()) else {
        return AdminStatus::unavailable("Unavailable");
    };
    let owned = match timeout(ADMIN_PROBE_TIMEOUT, dbus.name_has_owner(name)).await {
        Ok(Ok(owned)) => owned,
        _ => return AdminStatus::unavailable("Transport"),
    };
    if owned {
        return probe_admin_hw_owned(&connection, service).await;
    }
    match timeout(ADMIN_PROBE_TIMEOUT, dbus.list_activatable_names()).await {
        Ok(Ok(names)) if names.iter().any(|candidate| candidate.as_str() == service) => {
            // Installé mais endormi : la première écriture l'activera.
            AdminStatus {
                available: true,
                version: String::new(),
                error: "",
            }
        }
        Ok(_) => AdminStatus::unavailable("Unavailable"),
        Err(_) => AdminStatus::unavailable("Transport"),
    }
}

async fn probe_admin_hw_owned(connection: &zbus::Connection, service: &str) -> AdminStatus {
    let proxy = match timeout(
        ADMIN_PROBE_TIMEOUT,
        make_admin_hw_proxy_on(connection, service),
    )
    .await
    {
        Ok(Ok(proxy)) => proxy,
        Ok(Err(_)) => return AdminStatus::unavailable("Unavailable"),
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    let contract = match timeout(ADMIN_PROBE_TIMEOUT, proxy.get_contract_version()).await {
        Ok(Ok(version)) => version,
        Ok(Err(_)) => return AdminStatus::unavailable("Unavailable"),
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    if contract != ADMIN_HW_CONTRACT_VERSION {
        return AdminStatus {
            available: false,
            version: String::new(),
            error: "Contract",
        };
    }
    let version = match timeout(ADMIN_PROBE_TIMEOUT, proxy.get_version()).await {
        Ok(Ok(version)) => version,
        Ok(Err(_)) => String::new(),
        Err(_) => String::new(),
    };
    AdminStatus {
        available: true,
        version,
        error: "",
    }
}

async fn set_odm_on(
    kind: BusKind,
    service: &str,
    name: &str,
    force: bool,
) -> Result<(), SaveError> {
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_hw_proxy_on(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    let call = async {
        if force {
            proxy.set_odm_profile_override(name).await
        } else {
            proxy.set_odm_profile(name).await
        }
    };
    admin_hw_write(timeout(ADMIN_HW_WRITE_TIMEOUT, call).await)
}

async fn set_tdp_on(kind: BusKind, service: &str, values: Vec<i32>) -> Result<(), SaveError> {
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_hw_proxy_on(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    admin_hw_write(timeout(ADMIN_HW_WRITE_TIMEOUT, proxy.set_tdp_values(values)).await)
}

async fn set_ctgp_on(kind: BusKind, service: &str, watts: i32) -> Result<(), SaveError> {
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_hw_proxy_on(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    admin_hw_write(timeout(ADMIN_HW_WRITE_TIMEOUT, proxy.set_nvidia_ctgp_offset(watts)).await)
}

fn admin_hw_write(
    result: Result<zbus::Result<bool>, tokio::time::error::Elapsed>,
) -> Result<(), SaveError> {
    match result {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err(SaveError::Admin {
            code: "WriteFailed",
            detail: String::new(),
        }),
        Ok(Err(error)) => Err(map_admin_hw_error(&error)),
        Err(_) => Err(SaveError::Timeout),
    }
}

/// Lit les limites TDP (10 s, lecture seule).
#[allow(dead_code)] // contrat exposé ; l'UI utilise `HardwareData.odm.tdp`
async fn get_tdp_values_on(kind: BusKind, service: &str) -> Result<Vec<i32>, SaveError> {
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_hw_proxy_on(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    match timeout(ADMIN_HW_READ_TIMEOUT, proxy.get_tdp_values()).await {
        Ok(Ok(values)) if values.len() <= 8 => Ok(values),
        Ok(Ok(_)) => Err(SaveError::PayloadTooLarge),
        Ok(Err(error)) => Err(map_admin_hw_error(&error)),
        Err(_) => Err(SaveError::Timeout),
    }
}

/// Convertit une erreur `AdminHw1.Error.*`.
fn map_admin_hw_error(error: &zbus::Error) -> SaveError {
    let zbus::Error::MethodError(name, message, _) = error else {
        return SaveError::Transport;
    };
    if matches!(
        name.as_str(),
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited"
    ) {
        return SaveError::AdminUnavailable;
    }
    // Le service n'a pas répondu à temps : c'est un timeout, pas une erreur
    // applicative inconnue.
    if matches!(
        name.as_str(),
        "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout"
    ) {
        return SaveError::Timeout;
    }
    SaveError::Admin {
        code: admin_hw_error_code(name.as_str()),
        detail: message
            .as_deref()
            .map(|value| value.chars().take(256).collect())
            .unwrap_or_default(),
    }
}

fn admin_hw_error_code(name: &str) -> &'static str {
    let suffix = name
        .strip_prefix(ADMIN_HW_ERROR_NAMESPACE)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or("");
    match suffix {
        "NotAuthorized" => "NotAuthorized",
        "InvalidArgs" => "InvalidArgs",
        "Validation" => "Validation",
        "PolicyDenied" => "PolicyDenied",
        "NotSupported" => "NotSupported",
        "WriteFailed" => "WriteFailed",
        "PartialApply" => "PartialApply",
        "RateLimited" => "RateLimited",
        "Busy" => "Busy",
        _ => "Unknown",
    }
}

/// Nombre maximal de modes conservés par la vue d'affichage.
const MAX_DISPLAY_MODES: usize = 256;

/// Décode, valide et re-sérialise le rapport `GetDisplayInfoJSON`.
fn decode_display_info(raw: &str) -> Result<(DisplayInfoView, String), ClientError> {
    ensure_response_size("GetDisplayInfoJSON", raw)?;
    // Un rapport est toujours un objet : `serde` accepterait sinon une
    // séquence vide en appliquant les valeurs par défaut.
    if !raw.trim_start().starts_with('{') {
        return Err(ClientError::Decode {
            model: "DisplayInfoView",
            detail: "objet JSON attendu".to_owned(),
        });
    }
    let mut view: DisplayInfoView =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "DisplayInfoView",
            detail: error.to_string(),
        })?;
    // Un compositeur ne devrait jamais exposer des centaines de modes ni des
    // dimensions/taux hors bornes : on filtre côté Rust avant publication.
    view.sanitize();
    view.internal_output.modes.truncate(MAX_DISPLAY_MODES);
    let canonical = serde_json::to_string(&view).map_err(|error| ClientError::Encode {
        model: "DisplayInfoView",
        detail: error.to_string(),
    })?;
    Ok((view, canonical))
}

/// Lit le rapport d'affichage du service de session (3 s).
async fn fetch_display_info_on(kind: BusKind, service: &str) -> Result<String, ClientError> {
    let connection = connection_for(kind).await?;
    let proxy = make_session_proxy_on(&connection, service).await?;
    // Vérification stricte même si le service vient d'être activé par cette
    // lecture : un rapport d'un contrat inconnu n'est jamais publié.
    let contract = timeout(AGENT_PROBE_TIMEOUT, proxy.get_contract_version())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "User1.GetContractVersion",
        })?
        .map_err(|error| ClientError::Transport {
            call: "User1.GetContractVersion",
            detail: error.to_string(),
        })?;
    if contract != SESSION_CONTRACT_VERSION {
        return Err(ClientError::Invalid {
            model: "User1",
            detail: "contract".to_owned(),
        });
    }
    let raw = timeout(AGENT_READ_TIMEOUT, proxy.get_display_info_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetDisplayInfoJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetDisplayInfoJSON",
            detail: error.to_string(),
        })?;
    let (_view, canonical) = decode_display_info(&raw)?;
    Ok(canonical)
}

/// Vrai si l'erreur D-Bus signifie « méthode absente » (agent plus ancien).
fn is_unknown_method(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
    )
}

/// Écrit un contrôle UVC via `User1.SetUvcControl` (bus de session, 4 s).
async fn set_uvc_control_on(
    kind: BusKind,
    service: &str,
    device_path: &str,
    control_key: &str,
    value: i32,
) -> Result<bool, UvcWriteFailure> {
    let connection = connection_for(kind)
        .await
        .map_err(|_| UvcWriteFailure::new("Transport"))?;
    let proxy = make_session_proxy_on(&connection, service)
        .await
        .map_err(|_| UvcWriteFailure::new("Transport"))?;
    match timeout(
        AGENT_UVC_WRITE_TIMEOUT,
        proxy.set_uvc_control(device_path, control_key, value),
    )
    .await
    {
        // `false` = valeur déjà courante (idempotence), pas une erreur.
        Ok(Ok(changed)) => Ok(changed),
        Ok(Err(error)) => Err(map_agent_uvc_error(&error)),
        Err(_) => Err(UvcWriteFailure::new("Failed")),
    }
}

/// Convertit une erreur `User1.Error.*` en code stable, sans message serveur.
fn map_agent_uvc_error(error: &zbus::Error) -> UvcWriteFailure {
    let zbus::Error::MethodError(name, _message, _) = error else {
        return UvcWriteFailure::new("Transport");
    };
    uvc_error_code_for_name(name.as_str())
}

/// Code stable d'un nom d'erreur D-Bus (séparé pour les tests de mapping).
fn uvc_error_code_for_name(name: &str) -> UvcWriteFailure {
    if matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited"
    ) {
        return UvcWriteFailure::new("Unavailable");
    }
    if matches!(
        name,
        "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout"
    ) {
        return UvcWriteFailure::new("Failed");
    }
    let suffix = name
        .strip_prefix(AGENT_UVC_ERROR_NAMESPACE)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or("");
    match suffix {
        "OutOfRange" => UvcWriteFailure::new("OutOfRange"),
        "Busy" => UvcWriteFailure::new("Busy"),
        "Stalled" => UvcWriteFailure::new("Stalled"),
        _ => match name {
            "org.freedesktop.DBus.Error.InvalidArgs" => UvcWriteFailure::new("InvalidArgs"),
            // `UnknownMethod` : l'agent ne connaît pas encore l'écriture.
            "org.freedesktop.DBus.Error.NotSupported"
            | "org.freedesktop.DBus.Error.UnknownMethod" => UvcWriteFailure::new("NotSupported"),
            "org.freedesktop.DBus.Error.LimitsExceeded" => UvcWriteFailure::new("LimitsExceeded"),
            "org.freedesktop.DBus.Error.Failed" => UvcWriteFailure::new("Failed"),
            _ => UvcWriteFailure::new("Failed"),
        },
    }
}

/// Sonde `User1` sans activer le service : `NameHasOwner` puis
/// `ListActivatableNames`. Le contrat n'est vérifié que si le nom est possédé.
async fn probe_agent_on(kind: BusKind, service: &str) -> SessionStatus {
    let connection = match connection_for(kind).await {
        Ok(connection) => connection,
        Err(_) => return SessionStatus::unavailable("Transport"),
    };
    let dbus = match zbus::fdo::DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(_) => return SessionStatus::unavailable("Transport"),
    };
    let Ok(name) = zbus::names::BusName::try_from(service.to_owned()) else {
        return SessionStatus::unavailable("Unavailable");
    };
    let owned = match timeout(AGENT_PROBE_TIMEOUT, dbus.name_has_owner(name)).await {
        Ok(Ok(owned)) => owned,
        _ => return SessionStatus::unavailable("Transport"),
    };
    if owned {
        return probe_agent_owned(&connection, service).await;
    }
    match timeout(AGENT_PROBE_TIMEOUT, dbus.list_activatable_names()).await {
        // Installé mais endormi : la première lecture explicite l'activera.
        Ok(Ok(names)) if names.iter().any(|candidate| candidate.as_str() == service) => {
            SessionStatus {
                available: true,
                owned: false,
                version: String::new(),
                // Capacité inconnue tant que le service n'est pas démarré.
                uvc_write_supported: false,
                error: "",
            }
        }
        Ok(Ok(_)) => SessionStatus::unavailable("Unavailable"),
        _ => SessionStatus::unavailable("Transport"),
    }
}

async fn probe_agent_owned(connection: &zbus::Connection, service: &str) -> SessionStatus {
    let proxy = match make_session_proxy_on(connection, service).await {
        Ok(proxy) => proxy,
        Err(_) => return SessionStatus::unavailable("Transport"),
    };
    match timeout(AGENT_PROBE_TIMEOUT, proxy.ping()).await {
        Ok(Ok(true)) => {}
        _ => return SessionStatus::unavailable("Transport"),
    }
    let contract = match timeout(AGENT_PROBE_TIMEOUT, proxy.get_contract_version()).await {
        Ok(Ok(version)) => version,
        _ => return SessionStatus::unavailable("Transport"),
    };
    if contract != SESSION_CONTRACT_VERSION {
        return SessionStatus {
            available: false,
            owned: false,
            version: String::new(),
            uvc_write_supported: false,
            error: "Contract",
        };
    }
    // Méthode additive au contrat : un agent plus ancien renvoie
    // `UnknownMethod`. L'absence d'écriture n'est pas une erreur de service.
    let uvc_write_supported =
        match timeout(AGENT_PROBE_TIMEOUT, proxy.get_uvc_write_supported()).await {
            Ok(Ok(supported)) => supported,
            Ok(Err(error)) if is_unknown_method(&error) => false,
            Ok(Err(_)) | Err(_) => false,
        };
    let version = timeout(AGENT_PROBE_TIMEOUT, proxy.get_version())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|value| sanitize_version(&value))
        .unwrap_or_default();
    SessionStatus {
        available: true,
        owned: true,
        version,
        uvc_write_supported,
        error: "",
    }
}

/// Code stable d'une erreur de lecture du rapport d'affichage.
fn agent_error_code(error: &ClientError) -> &'static str {
    match error {
        ClientError::Timeout { .. } => "Timeout",
        ClientError::ResponseTooLarge { .. } => "PayloadTooLarge",
        ClientError::Invalid { model, detail } if *model == "User1" && detail == "contract" => {
            "Contract"
        }
        ClientError::Decode { .. } | ClientError::Invalid { .. } | ClientError::Encode { .. } => {
            "InvalidPayload"
        }
        _ => "Transport",
    }
}

/// Détecte le service d'administration sans le lancer inutilement.
///
/// `NameHasOwner` et `ListActivatableNames` ne déclenchent pas l'activation
/// D-Bus : un `Ping` toutes les 2 s lancerait `tccd-admin` (root) en
/// permanence, contrairement à l'objectif « à la demande » (AD-1). Le contrat
/// n'est vérifié (`Ping` + `GetContractVersion`) que si le nom est déjà
/// possédé.
async fn probe_admin(service: &str) -> AdminStatus {
    probe_admin_on(bus_kind(), service).await
}

async fn probe_admin_on(kind: BusKind, service: &str) -> AdminStatus {
    let connection = match connection_for(kind).await {
        Ok(connection) => connection,
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    let dbus = match zbus::fdo::DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    let Ok(name) = zbus::names::BusName::try_from(service.to_owned()) else {
        return AdminStatus::unavailable("Unavailable");
    };

    // G1-05 : ces appels bus sont bornés eux aussi (un bus lent ne doit pas
    // bloquer le worker).
    let owned = match timeout(ADMIN_PROBE_TIMEOUT, dbus.name_has_owner(name)).await {
        Ok(Ok(owned)) => owned,
        _ => return AdminStatus::unavailable("Transport"),
    };
    if owned {
        return probe_admin_owned(&connection, service).await;
    }
    match timeout(ADMIN_PROBE_TIMEOUT, dbus.list_activatable_names()).await {
        Ok(Ok(names)) if names.iter().any(|candidate| candidate.as_str() == service) => {
            // Installé mais endormi : l'enregistrement l'activera.
            AdminStatus {
                available: true,
                version: String::new(),
                error: "",
            }
        }
        // Service non installé (ou activation impossible) : indisponible, pas
        // une erreur de transport.
        Ok(Ok(_)) => AdminStatus::unavailable("Unavailable"),
        _ => AdminStatus::unavailable("Transport"),
    }
}

async fn probe_admin_owned(connection: &zbus::Connection, service: &str) -> AdminStatus {
    let proxy = match make_admin_proxy(connection, service).await {
        Ok(proxy) => proxy,
        Err(_) => return AdminStatus::unavailable("Transport"),
    };
    match timeout(ADMIN_PROBE_TIMEOUT, proxy.ping()).await {
        Ok(Ok(true)) => {}
        _ => return AdminStatus::unavailable("Transport"),
    }
    let contract = match timeout(ADMIN_PROBE_TIMEOUT, proxy.get_contract_version()).await {
        Ok(Ok(version)) => version,
        _ => return AdminStatus::unavailable("Transport"),
    };
    if ensure_admin_contract_compatible(contract).is_err() {
        return AdminStatus::unavailable("Contract");
    }
    let version = timeout(ADMIN_PROBE_TIMEOUT, proxy.get_version())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|value| sanitize_version(&value))
        .unwrap_or_default();
    AdminStatus {
        available: true,
        version,
        error: "",
    }
}

/// Version d'implémentation bornée et sûre pour l'affichage.
fn sanitize_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '/' | '\\'))
    {
        return String::new();
    }
    trimmed.to_owned()
}

/// Enregistre la collection via `Admin1.SaveProfilesJSON`.
async fn save_profiles_remote(service: &str, json: &str) -> Result<(), SaveError> {
    save_profiles_on(bus_kind(), service, json).await
}

async fn save_profiles_on(kind: BusKind, service: &str, json: &str) -> Result<(), SaveError> {
    save_profiles_with_timeout(kind, service, json, ADMIN_SAVE_TIMEOUT).await
}

async fn save_profiles_with_timeout(
    kind: BusKind,
    service: &str,
    json: &str,
    limit: Duration,
) -> Result<(), SaveError> {
    if json.len() > crate::profile_edit::MAX_PROFILES_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_proxy(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    let result = timeout(limit, proxy.save_profiles_json(json))
        .await
        .map_err(|_| SaveError::Timeout)?;
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(SaveError::Admin {
            code: "WriteFailed",
            detail: String::new(),
        }),
        Err(error) => Err(map_admin_error(&error)),
    }
}

/// Enregistre les réglages via `Admin1.SaveSettingsJSON`.
async fn save_settings_remote(service: &str, json: &str) -> Result<(), SaveError> {
    save_settings_on(bus_kind(), service, json).await
}

async fn save_settings_on(kind: BusKind, service: &str, json: &str) -> Result<(), SaveError> {
    save_settings_with_timeout(kind, service, json, ADMIN_SAVE_TIMEOUT).await
}

async fn save_settings_with_timeout(
    kind: BusKind,
    service: &str,
    json: &str,
    limit: Duration,
) -> Result<(), SaveError> {
    if json.len() > crate::settings_edit::MAX_SETTINGS_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    let connection = connection_for(kind)
        .await
        .map_err(|_| SaveError::Transport)?;
    let proxy = make_admin_proxy(&connection, service)
        .await
        .map_err(|_| SaveError::Transport)?;
    let result = timeout(limit, proxy.save_settings_json(json))
        .await
        .map_err(|_| SaveError::Timeout)?;
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(SaveError::Admin {
            code: "WriteFailed",
            detail: String::new(),
        }),
        Err(error) => Err(map_admin_error(&error)),
    }
}

/// Convertit une erreur D-Bus `Admin1.Error.*` en erreur typée.
///
/// Le message renvoyé par le service ne contient qu'un nom de champ ou un
/// identifiant ; il est borné avant stockage et re-filtré à l'affichage.
fn map_admin_error(error: &zbus::Error) -> SaveError {
    let zbus::Error::MethodError(name, message, _) = error else {
        return SaveError::Transport;
    };
    // Service absent ou activation D-Bus impossible : ce n'est pas une erreur
    // de transport générique (G1-05).
    if matches!(
        name.as_str(),
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited"
    ) {
        return SaveError::AdminUnavailable;
    }
    SaveError::Admin {
        code: admin_error_code(name.as_str()),
        detail: message
            .as_deref()
            .map(|value| value.chars().take(256).collect())
            .unwrap_or_default(),
    }
}

/// Code stable dérivé d'un nom d'erreur D-Bus complet.
fn admin_error_code(name: &str) -> &'static str {
    let suffix = name
        .strip_prefix(ADMIN_ERROR_NAMESPACE)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or("");
    match suffix {
        "NotAuthorized" => "NotAuthorized",
        // Émis par une version future de l'admin : code distinct, message
        // dédié (le repli actuel côté admin est `NotAuthorized`).
        "PolicyKitUnavailable" => "PolicyKitUnavailable",
        "InvalidArgs" => "InvalidArgs",
        "PayloadTooLarge" => "PayloadTooLarge",
        "Validation" => "Validation",
        "InUse" => "InUse",
        "ReloadFailed" => "ReloadFailed",
        "WriteFailed" => "WriteFailed",
        "RateLimited" => "RateLimited",
        "Busy" => "Busy",
        "NotSupported" => "NotSupported",
        _ => "Unknown",
    }
}

async fn connection() -> Result<zbus::Connection, ClientError> {
    connection_for(bus_kind()).await
}

async fn connection_for(kind: BusKind) -> Result<zbus::Connection, ClientError> {
    match kind {
        BusKind::System => zbus::Connection::system().await,
        BusKind::Session => zbus::Connection::session().await,
    }
    .map_err(|error| ClientError::Transport {
        call: "connexion D-Bus",
        detail: error.to_string(),
    })
}

async fn make_proxy<'a>(connection: &'a zbus::Connection) -> Result<TccdProxy<'a>, ClientError> {
    TccdProxy::builder(connection)
        .destination(service_name())
        .map_err(|error| ClientError::Transport {
            call: "destination D-Bus",
            detail: error.to_string(),
        })?
        .path(tcc_ipc::DBUS_PATH)
        .map_err(|error| ClientError::Transport {
            call: "objet D-Bus",
            detail: error.to_string(),
        })?
        .build()
        .await
        .map_err(|error| ClientError::Transport {
            call: "proxy D-Bus",
            detail: error.to_string(),
        })
}

async fn fetch_snapshot() -> Result<SnapshotData, ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;

    let state_raw = timeout(DBUS_TIMEOUT, proxy.get_state_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetStateJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetStateJSON",
            detail: error.to_string(),
        })?;
    // Borne appliquée avant toute désérialisation.
    let (snapshot, state_json) = decode_snapshot(&state_raw)?;

    let profiles_raw = timeout(DBUS_TIMEOUT, proxy.get_profiles_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetProfilesJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetProfilesJSON",
            detail: error.to_string(),
        })?;
    let (profiles, profiles_json) = decode_profiles(&profiles_raw)?;

    // Métadonnées de version : une erreur ne doit pas invalider l'instantané
    // principal (daemon ancien) ; l'Info affiche alors « non disponible ».
    let daemon_version = timeout(DBUS_TIMEOUT, proxy.get_version())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|value| sanitize_version(&value))
        .unwrap_or_default();
    let contract_version = timeout(DBUS_TIMEOUT, proxy.contract_version())
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(0);

    let (active_id, active_name) = snapshot
        .active_profile
        .as_ref()
        .map(|profile| (profile.id.as_str().to_owned(), profile.name.clone()))
        .unwrap_or_default();

    Ok(SnapshotData {
        state_json,
        profiles_json,
        active_id,
        active_name,
        ppd: snapshot
            .ppd_profile
            .as_ref()
            .map(|profile| profile.as_str().to_owned())
            .unwrap_or_default(),
        power: snapshot.power_source.as_str().to_owned(),
        manual_override: snapshot.is_manual_override(),
        revision: i32::try_from(snapshot.revision).unwrap_or(i32::MAX),
        daemon_version,
        contract_version,
        profiles: Some(profiles),
    })
}

async fn apply_temp_profile(id: &str) -> Result<(), ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;
    let changed = timeout(DBUS_TIMEOUT, proxy.set_temp_profile(id))
        .await
        .map_err(|_| ClientError::Timeout {
            call: "SetTempProfile",
        })?
        .map_err(|error| ClientError::Transport {
            call: "SetTempProfile",
            detail: error.to_string(),
        })?;
    if !changed {
        return Err(ClientError::Invalid {
            model: "SetTempProfile",
            detail: "profil inconnu ou refusé par le daemon".to_owned(),
        });
    }
    Ok(())
}

async fn clear_temp_profile() -> Result<(), ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;
    timeout(DBUS_TIMEOUT, proxy.clear_temp_profile())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "ClearTempProfile",
        })?
        .map_err(|error| ClientError::Transport {
            call: "ClearTempProfile",
            detail: error.to_string(),
        })?;
    Ok(())
}

async fn make_manager_proxy_on<'a>(
    connection: &'a zbus::Connection,
    service: &str,
) -> Result<Manager1Proxy<'a>, ClientError> {
    Manager1Proxy::builder(connection)
        .destination(service.to_owned())
        .map_err(|error| ClientError::Transport {
            call: "destination D-Bus",
            detail: error.to_string(),
        })?
        .path(tcc_ipc::DBUS_PATH)
        .map_err(|error| ClientError::Transport {
            call: "objet D-Bus",
            detail: error.to_string(),
        })?
        .build()
        .await
        .map_err(|error| ClientError::Transport {
            call: "proxy Manager1",
            detail: error.to_string(),
        })
}

async fn make_manager_proxy(
    connection: &zbus::Connection,
) -> Result<Manager1Proxy<'_>, ClientError> {
    make_manager_proxy_on(connection, &service_name()).await
}

/// Lit l'instantané matériel.
///
/// `renew` arme explicitement le bail (`SetSensorDataCollectionStatus(true)`)
/// puis la lecture elle-même le réarme (TTL 10 s côté daemon). `disarm` n'est
/// vrai que sur désactivation explicite : la sortie du Dashboard laisse le bail
/// expirer sans envoyer `false`.
async fn fetch_hardware(renew: bool, disarm: bool) -> Result<(HardwareView, String), ClientError> {
    let connection = connection().await?;
    let proxy = make_manager_proxy(&connection).await?;

    if renew {
        timeout(DBUS_TIMEOUT, proxy.set_sensor_data_collection_status(true))
            .await
            .map_err(|_| ClientError::Timeout {
                call: "SetSensorDataCollectionStatus",
            })?
            .map_err(|error| ClientError::Transport {
                call: "SetSensorDataCollectionStatus",
                detail: error.to_string(),
            })?;
    } else if disarm {
        // Désactivation explicite seulement : une sortie du Dashboard laisse
        // le bail expirer sans envoyer `false`.
        let _ = timeout(DBUS_TIMEOUT, proxy.set_sensor_data_collection_status(false)).await;
    }

    let raw = timeout(DBUS_TIMEOUT, proxy.get_hardware_snapshot_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetHardwareSnapshotJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetHardwareSnapshotJSON",
            detail: error.to_string(),
        })?;

    // Seule la forme canonique de la vue typée est publiée vers QML ; la vue
    // typée reste côté worker pour les validations (charge).
    decode_hardware(&raw)
}

/// Relit le statut de la minuterie d'extinction (Manager1, sans polkit).
async fn fetch_shutdown_timer() -> Result<ShutdownTimerView, ClientError> {
    fetch_shutdown_timer_on(bus_kind(), &service_name()).await
}

async fn fetch_shutdown_timer_on(
    kind: BusKind,
    service: &str,
) -> Result<ShutdownTimerView, ClientError> {
    let connection = connection_for(kind).await?;
    let proxy = make_manager_proxy_on(&connection, service).await?;
    let raw = timeout(DBUS_TIMEOUT, proxy.get_shutdown_timer_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetShutdownTimerJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetShutdownTimerJSON",
            detail: error.to_string(),
        })?;
    decode_shutdown_timer(&raw)
}

/// Relit uniquement la collection de profils (pré-enregistrement, G1-03).
async fn fetch_profile_collection() -> Result<ProfileCollection, ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;
    let raw = timeout(DBUS_TIMEOUT, proxy.get_profiles_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetProfilesJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetProfilesJSON",
            detail: error.to_string(),
        })?;
    let (profiles, _json) = decode_profiles(&raw)?;
    Ok(profiles)
}

/// Lit `GetFanDataJSON` (interface legacy) et renvoie la vue canonique.
async fn fetch_fans() -> Result<String, ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;
    let raw = timeout(DBUS_TIMEOUT, proxy.get_fan_data_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetFanDataJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetFanDataJSON",
            detail: error.to_string(),
        })?;
    let (_view, json) = decode_fans(&raw)?;
    Ok(json)
}

/// Relit les réglages globaux (base d'édition et affichage).
async fn fetch_settings() -> Result<(Settings, String), ClientError> {
    let connection = connection().await?;
    let proxy = make_proxy(&connection).await?;
    let raw = timeout(DBUS_TIMEOUT, proxy.get_settings_json())
        .await
        .map_err(|_| ClientError::Timeout {
            call: "GetSettingsJSON",
        })?
        .map_err(|error| ClientError::Transport {
            call: "GetSettingsJSON",
            detail: error.to_string(),
        })?;
    decode_settings(&raw)
}

/// Vérifie la borne de taille **avant** toute désérialisation.
fn ensure_response_size(call: &'static str, raw: &str) -> Result<(), ClientError> {
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(ClientError::ResponseTooLarge {
            call,
            size: raw.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    Ok(())
}

/// Décode, valide et re-sérialise un `StateSnapshot`.
fn decode_snapshot(raw: &str) -> Result<(StateSnapshot, String), ClientError> {
    ensure_response_size("GetStateJSON", raw)?;
    let snapshot: StateSnapshot =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "StateSnapshot",
            detail: error.to_string(),
        })?;
    snapshot.validate().map_err(|error| ClientError::Invalid {
        model: "StateSnapshot",
        detail: error.to_string(),
    })?;
    // QML ne reçoit que le modèle validé, jamais la chaîne brute.
    let canonical = serde_json::to_string(&snapshot).map_err(|error| ClientError::Encode {
        model: "StateSnapshot",
        detail: error.to_string(),
    })?;
    Ok((snapshot, canonical))
}

/// Décode, valide et re-sérialise une `ProfileCollection`.
fn decode_profiles(raw: &str) -> Result<(ProfileCollection, String), ClientError> {
    ensure_response_size("GetProfilesJSON", raw)?;
    let profiles: ProfileCollection =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "ProfileCollection",
            detail: error.to_string(),
        })?;
    profiles.validate().map_err(|error| ClientError::Invalid {
        model: "ProfileCollection",
        detail: error.to_string(),
    })?;
    let canonical = serde_json::to_string(&profiles).map_err(|error| ClientError::Encode {
        model: "ProfileCollection",
        detail: error.to_string(),
    })?;
    Ok((profiles, canonical))
}

/// Décode, valide et re-sérialise l'instantané matériel.
fn decode_hardware(raw: &str) -> Result<(HardwareView, String), ClientError> {
    ensure_response_size("GetHardwareSnapshotJSON", raw)?;
    let mut view: HardwareView =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "HardwareSnapshot",
            detail: error.to_string(),
        })?;
    sanitize_hardware_view(&mut view);
    // QML ne reçoit que la vue typée, jamais la chaîne brute.
    let canonical = serde_json::to_string(&view).map_err(|error| ClientError::Encode {
        model: "HardwareSnapshot",
        detail: error.to_string(),
    })?;
    Ok((view, canonical))
}

/// Bornes d'affichage des contrôles UVC (le daemon a ses propres plafonds).
const MAX_WEBCAM_DEVICES: usize = 16;
const MAX_UVC_CONTROLS: usize = 64;
const MAX_UVC_MENU: usize = 128;
const MAX_DISPLAY_STRING: usize = 128;

/// Réduit la vue webcam à des tailles d'affichage sûres.
fn sanitize_hardware_view(view: &mut HardwareView) {
    view.webcam.devices.truncate(MAX_WEBCAM_DEVICES);
    for device in &mut view.webcam.devices {
        clamp_string(&mut device.name);
        if let Some(path) = &mut device.device_path {
            clamp_string(path);
        }
        if let Some(name) = &mut device.device_name {
            clamp_string(name);
        }
        device.controls.truncate(MAX_UVC_CONTROLS);
        for control in &mut device.controls {
            clamp_string(&mut control.key);
            clamp_string(&mut control.name);
            clamp_string(&mut control.kind);
            clamp_string(&mut control.category);
            control.menu.truncate(MAX_UVC_MENU);
            for entry in &mut control.menu {
                clamp_string(&mut entry.key);
                clamp_string(&mut entry.label);
            }
            if let Some(current) = &mut control.current {
                clamp_string(&mut current.kind);
                if let Some(key) = &mut current.key {
                    clamp_string(key);
                }
                if let Some(label) = &mut current.label {
                    clamp_string(label);
                }
            }
        }
    }
}

fn clamp_string(value: &mut String) {
    if value.chars().count() > MAX_DISPLAY_STRING {
        *value = value.chars().take(MAX_DISPLAY_STRING).collect();
    }
}

/// Décode, valide et re-sérialise l'instantané des ventilateurs.
fn decode_fans(raw: &str) -> Result<(FanView, String), ClientError> {
    ensure_response_size("GetFanDataJSON", raw)?;
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "FanData",
            detail: error.to_string(),
        })?;
    if !value.is_object() {
        return Err(ClientError::Decode {
            model: "FanData",
            detail: "objet JSON attendu".to_owned(),
        });
    }
    let view: FanView = serde_json::from_value(value).map_err(|error| ClientError::Decode {
        model: "FanData",
        detail: error.to_string(),
    })?;
    let canonical = serde_json::to_string(&view).map_err(|error| ClientError::Encode {
        model: "FanData",
        detail: error.to_string(),
    })?;
    Ok((view, canonical))
}

/// Décode et re-sérialise les réglages globaux.
///
/// Le modèle `tcc_core::Settings` conserve les champs inconnus (`extra`) et
/// tous les champs non modifiables ; QML ne reçoit que la forme canonique.
fn decode_settings(raw: &str) -> Result<(Settings, String), ClientError> {
    ensure_response_size("GetSettingsJSON", raw)?;
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "Settings",
            detail: error.to_string(),
        })?;
    if !value.is_object() {
        return Err(ClientError::Decode {
            model: "Settings",
            detail: "objet JSON attendu".to_owned(),
        });
    }
    let settings: Settings =
        serde_json::from_value(value).map_err(|error| ClientError::Decode {
            model: "Settings",
            detail: error.to_string(),
        })?;
    // Contrairement aux profils, `Settings` a des champs `default` : un schéma
    // faux ou une valeur hors bornes ne se voit pas à la désérialisation.
    settings
        .validate()
        .map_err(|error| settings_invalid(&error))?;
    let canonical = serde_json::to_string(&settings).map_err(|error| ClientError::Encode {
        model: "Settings",
        detail: error.to_string(),
    })?;
    Ok((settings, canonical))
}

/// Décode le statut de la minuterie d'extinction.
fn decode_shutdown_timer(raw: &str) -> Result<ShutdownTimerView, ClientError> {
    ensure_response_size("GetShutdownTimerJSON", raw)?;
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| ClientError::Decode {
            model: "ShutdownTimer",
            detail: error.to_string(),
        })?;
    if !value.is_object() {
        return Err(ClientError::Decode {
            model: "ShutdownTimer",
            detail: "objet JSON attendu".to_owned(),
        });
    }
    serde_json::from_value(value).map_err(|error| ClientError::Decode {
        model: "ShutdownTimer",
        detail: error.to_string(),
    })
}

/// Erreur `Settings` invalide avec un détail sûr (nom de champ uniquement).
fn settings_invalid(error: &tcc_core::ValidationError) -> ClientError {
    let field = error
        .issues()
        .first()
        .map(|issue| issue.field.as_str())
        .unwrap_or("champs");
    ClientError::Invalid {
        model: "Settings",
        detail: format!("champ invalide ({})", sanitize_validation_field(field)),
    }
}

/// Vérifie les références de profils d'un `Settings` (stateMap/stateMatrix).
fn validate_settings_against_profiles(
    settings: &Settings,
    profiles: &ProfileCollection,
) -> Result<(), ClientError> {
    tcc_core::validate_settings_references(settings, profiles).map_err(|error| {
        let field = error
            .issues()
            .first()
            .map(|issue| issue.field.as_str())
            .unwrap_or("champs");
        ClientError::Invalid {
            model: "Settings",
            detail: format!("référence invalide ({})", sanitize_validation_field(field)),
        }
    })
}

/// Nom de champ borné et sûr pour l'affichage (jamais une valeur).
fn sanitize_validation_field(field: &str) -> String {
    let trimmed = field.trim();
    if trimmed.is_empty()
        || trimmed.len() > 80
        || !trimmed.is_ascii()
        || trimmed
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '@' | '='))
    {
        return "champs".to_owned();
    }
    trimmed.to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BusKind {
    #[default]
    System,
    /// Construit uniquement avec `dev-overrides`.
    #[cfg_attr(not(feature = "dev-overrides"), allow(dead_code))]
    Session,
}

/// Bus D-Bus utilisé.
///
/// `TCC_DBUS_BUS` n'est honoré qu'avec la feature `dev-overrides` ; sinon le
/// bus système est imposé (comportement release).
fn bus_kind() -> BusKind {
    #[cfg(feature = "dev-overrides")]
    if std::env::var("TCC_DBUS_BUS").is_ok_and(|value| value.eq_ignore_ascii_case("session")) {
        return BusKind::Session;
    }
    BusKind::System
}

/// Nom D-Bus du service.
///
/// `TCC_DBUS_SERVICE` n'est lu qu'avec la feature `dev-overrides`, et seulement
/// s'il s'agit d'un nom de bus valide. Sinon [`tcc_ipc::DBUS_SERVICE`] est
/// utilisé.
fn service_name() -> String {
    #[cfg(feature = "dev-overrides")]
    if let Ok(value) = std::env::var("TCC_DBUS_SERVICE") {
        let value = value.trim();
        if is_valid_bus_name(value) {
            return value.to_owned();
        }
        eprintln!("tcc-gui: TCC_DBUS_SERVICE invalide, service par défaut utilisé");
    }
    tcc_ipc::DBUS_SERVICE.to_owned()
}

/// Validation minimale d'un nom de bus D-Bus bien connu.
///
/// Refuse un nom vide, trop long, commençant par un chiffre ou contenant un
/// caractère hors `[A-Za-z0-9_.-]`, ou sans `.` (D-Bus l'exige pour un nom bien
/// connu).
#[cfg(feature = "dev-overrides")]
fn is_valid_bus_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.contains('.')
        && !value.starts_with(|c: char| c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::{
        ADMIN_ERROR_NAMESPACE, ADMIN_SAVE_TIMEOUT, AdminStatus, BusKind, COMMAND_QUEUE_CAPACITY,
        ClientError, Command, CommandSender, DBUS_ADMIN_PATH, MAX_RESPONSE_BYTES,
        MAX_UVC_DEVICE_PATH_BYTES, ProfileMutation, SENSOR_LEASE_GRACE, admin_error_code,
        admin_hw_ctgp_valid, admin_hw_error_code, admin_hw_odm_name_valid,
        admin_hw_tdp_values_valid, agent_error_code, agent_uvc_control_key_valid,
        agent_uvc_device_path_valid, count_as_i32, decode_display_info, decode_fans,
        decode_hardware, decode_profiles, decode_settings, decode_shutdown_timer, decode_snapshot,
        ensure_response_size, fetch_display_info_on, fetch_shutdown_timer_on, get_tdp_values_on,
        monitor_worker, next_lease_state, probe_admin_hw_on, probe_admin_on, probe_agent_on,
        run_import_mutation, run_profile_mutation, run_save_settings, sanitize_client_detail,
        sanitize_version, save_profiles_on, save_profiles_with_timeout, save_settings_on,
        save_settings_with_timeout, set_ctgp_on, set_odm_on, set_tdp_on, set_uvc_control_on,
        uvc_error_code_for_name, validate_settings_against_profiles,
    };
    use crate::hardware_view::Charging;
    use crate::profile_edit::{SaveError, apply_edit, decode_edit};
    use crate::settings_edit::ChargeCapabilities;
    use tcc_ipc::{
        ADMIN_HW_CONTRACT_VERSION, ADMIN_HW_ERROR_NAMESPACE, DBUS_ADMIN_HW_PATH,
        DBUS_SESSION_AGENT_PATH, SESSION_CONTRACT_VERSION,
    };

    const SNAPSHOT_JSON: &str = r#"{
        "contractVersion": 1,
        "revision": 7,
        "ppdProfile": "balanced",
        "powerSource": "ac",
        "manualOverride": false,
        "activeProfile": {
            "id": "__office__",
            "name": "Office",
            "description": "Bureau",
            "isPreset": true,
            "source": "stateMatrix"
        },
        "ignoredField": true
    }"#;

    const PROFILES_JSON: &str = r#"[
        { "id": "__office__", "name": "Office", "description": "Bureau" }
    ]"#;

    #[test]
    fn oversized_response_is_rejected_before_parsing() {
        let raw = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let error = ensure_response_size("GetStateJSON", &raw).unwrap_err();
        assert!(matches!(
            error,
            ClientError::ResponseTooLarge {
                call: "GetStateJSON",
                ..
            }
        ));
        // Une réponse trop grosse n'est pas une perte de connexion.
        assert!(!error.loses_connection());
        assert!(error.to_string().contains("max"));
    }

    #[test]
    fn response_at_limit_is_accepted() {
        let raw = "x".repeat(MAX_RESPONSE_BYTES);
        assert!(ensure_response_size("GetProfilesJSON", &raw).is_ok());
    }

    #[test]
    fn snapshot_is_decoded_then_reencoded_from_validated_model() {
        let (snapshot, canonical) = decode_snapshot(SNAPSHOT_JSON).unwrap();
        assert_eq!(snapshot.revision, 7);
        // Le champ inconnu de la réponse brute ne franchit pas la frontière.
        assert!(!canonical.contains("ignoredField"));
        assert!(canonical.contains("\"contractVersion\":1"));
        assert!(canonical.contains("\"ppdProfile\":\"balanced\""));
        assert!(canonical.contains("\"manualOverride\":false"));
    }

    #[test]
    fn profiles_are_decoded_then_reencoded() {
        let (profiles, canonical) = decode_profiles(PROFILES_JSON).unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(canonical.starts_with('['));
        // Le modèle complet est sérialisé, pas la chaîne minimale reçue.
        assert!(canonical.contains("\"useRefRate\":false"));
        assert!(canonical.contains("\"fanProfile\":\"Balanced\""));
    }

    #[test]
    fn invalid_json_is_a_decode_error() {
        let error = decode_snapshot("{ pas du json").unwrap_err();
        assert!(matches!(
            error,
            ClientError::Decode {
                model: "StateSnapshot",
                ..
            }
        ));
        assert!(!error.loses_connection());
    }

    #[test]
    fn incompatible_contract_is_rejected() {
        let raw =
            r#"{"contractVersion":99,"revision":1,"powerSource":"unknown","manualOverride":false}"#;
        let error = decode_snapshot(raw).unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "StateSnapshot",
                ..
            }
        ));
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        // Nom vide : la validation `tcc-core` refuse la collection.
        let raw = r#"[{ "id": "__office__", "name": "" }]"#;
        let error = decode_profiles(raw).unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "ProfileCollection",
                ..
            }
        ));
    }

    const HARDWARE_JSON: &str = r#"{
        "charging": { "available": true, "profile": "balanced", "startThreshold": 60 },
        "lighting": { "available": true, "keyboard": { "kind": "rgb", "zones": 3 } },
        "odm": { "platformProfile": { "current": "balanced", "owner": "ppd" } },
        "sensors": {
            "enabled": true,
            "available": true,
            "cpu": { "temperatureC": 55.5, "frequencyMhz": 3200, "powerW": 28.0 },
            "dgpu": { "vendor": "nvidia", "name": "RTX 4060", "temperatureC": 61.0 },
            "fans": [{ "name": "CPU", "temp": 55.5, "speed": 42.0, "applied": true }]
        },
        "nvidia": { "available": true, "ctgpOffset": 10 },
        "fnLock": { "supported": true, "enabled": false },
        "prime": { "available": true, "mode": "on-demand" },
        "ycbcr": { "available": true, "ports": [{ "card": 0, "port": "DP-1", "forced": false }] },
        "futureField": { "dropped": true }
    }"#;

    #[test]
    fn hardware_snapshot_is_reencoded_from_validated_view() {
        let (_view, canonical) = decode_hardware(HARDWARE_JSON).unwrap();
        // Les clés inconnues ne franchissent pas la frontière.
        assert!(!canonical.contains("futureField"));
        // Toutes les sections attendues par l'adaptateur QML sont présentes.
        for key in [
            "\"charging\"",
            "\"lighting\"",
            "\"odm\"",
            "\"sensors\"",
            "\"nvidia\"",
            "\"webcam\"",
            "\"fnLock\"",
            "\"prime\"",
            "\"ycbcr\"",
        ] {
            assert!(canonical.contains(key), "clé manquante: {key}");
        }
        assert!(canonical.contains("\"ctgpOffset\":10"));
        assert!(canonical.contains("\"fans\""));
    }

    #[test]
    fn webcam_controls_are_bounded_in_the_display_view() {
        let control = serde_json::json!({
            "key": "brightness",
            "name": "Brightness",
            "kind": "integer",
            "min": 0,
            "max": 255,
            "step": 1,
            "default": 128,
            "current": { "kind": "integer", "value": 100 },
            "menu": [],
            "flags": { "readOnly": true },
            "category": "basic",
            "simple": true
        });
        let device = serde_json::json!({
            "name": "Cam",
            "devicePath": "/dev/video0",
            "controls": vec![control; 100]
        });
        let payload = serde_json::json!({
            "webcam": { "controlsSupported": true, "devices": vec![device; 20] },
            "sensors": {},
            "charging": {}
        })
        .to_string();
        let (view, canonical) = decode_hardware(&payload).expect("vue");
        assert_eq!(view.webcam.devices.len(), 16);
        assert_eq!(view.webcam.devices[0].controls.len(), 64);
        assert!(view.webcam.devices[0].controls[0].flags.read_only);
        assert!(canonical.contains("\"controls\""));
        assert!(canonical.len() <= MAX_RESPONSE_BYTES);
    }

    #[test]
    fn webcam_control_menu_is_bounded_in_the_display_view() {
        let menu: Vec<serde_json::Value> = (0..200)
            .map(|index| serde_json::json!({ "index": index, "key": "k", "label": "Label" }))
            .collect();
        let payload = serde_json::json!({
            "webcam": {
                "controlsSupported": true,
                "devices": [{
                    "name": "Cam",
                    "controls": [{
                        "key": "exposure_auto",
                        "kind": "menu",
                        "menu": menu
                    }]
                }]
            },
            "sensors": {},
            "charging": {}
        })
        .to_string();
        let (view, _canonical) = decode_hardware(&payload).expect("vue");
        assert_eq!(view.webcam.devices[0].controls[0].menu.len(), 128);
    }

    #[test]
    fn oversized_hardware_response_is_rejected() {
        let raw = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let error = decode_hardware(&raw).unwrap_err();
        assert!(matches!(
            error,
            ClientError::ResponseTooLarge {
                call: "GetHardwareSnapshotJSON",
                ..
            }
        ));
    }

    #[test]
    fn invalid_hardware_shape_is_a_decode_error() {
        let error = decode_hardware("[1, 2, 3]").unwrap_err();
        assert!(matches!(
            error,
            ClientError::Decode {
                model: "HardwareSnapshot",
                ..
            }
        ));
    }

    #[test]
    fn fan_snapshot_is_reencoded() {
        let raw = r#"{
            "available": true, "backend": "tuxi", "writable": true,
            "controlEnabled": true, "mode": "manual",
            "fans": [{ "index": 0, "label": "cpu", "target": "cpu",
                       "temp": 57, "speed": 42, "applied": 45 }],
            "futureField": true
        }"#;
        let (_view, canonical) = decode_fans(raw).unwrap();
        assert!(!canonical.contains("futureField"));
        assert!(canonical.contains("\"backend\":\"tuxi\""));
        assert!(canonical.contains("\"speed\":42"));
    }

    #[test]
    fn oversized_fan_response_is_rejected() {
        let raw = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let error = decode_fans(&raw).unwrap_err();
        assert!(matches!(
            error,
            ClientError::ResponseTooLarge {
                call: "GetFanDataJSON",
                ..
            }
        ));
    }

    #[test]
    fn non_object_fan_response_is_rejected() {
        let error = decode_fans("[1, 2]").unwrap_err();
        assert!(matches!(
            error,
            ClientError::Decode {
                model: "FanData",
                ..
            }
        ));
    }

    #[test]
    fn lease_renews_only_when_requested_and_visible() {
        let now = std::time::Instant::now();
        assert_eq!(
            next_lease_state(now, true, true, false, None, false),
            (true, None)
        );
        assert_eq!(
            next_lease_state(now, false, true, false, None, false),
            (false, None)
        );
        assert_eq!(
            next_lease_state(now, true, false, false, None, false),
            (false, None)
        );
    }

    #[test]
    fn leaving_dashboard_pauses_without_disarming() {
        let now = std::time::Instant::now();
        // On quitte le Dashboard alors que le bail était renouvelé.
        let (renewing, pause) = next_lease_state(now, true, false, true, None, false);
        assert!(!renewing);
        assert_eq!(pause, Some(now + SENSOR_LEASE_GRACE));
        // Pendant la fenêtre, on reste en pause et aucune commande `false`
        // n'est émise (le worker ne lit pas l'instantané).
        let (renewing, pause) = next_lease_state(now, true, false, false, pause, false);
        assert!(!renewing);
        assert!(pause.is_some());
        // Après la grâce, on reprend les lectures (le bail est expiré).
        let later = now + SENSOR_LEASE_GRACE + std::time::Duration::from_secs(1);
        assert_eq!(
            next_lease_state(later, true, false, false, pause, false),
            (false, None)
        );
    }

    #[test]
    fn explicit_disable_clears_pause() {
        let now = std::time::Instant::now();
        let pause = Some(now + SENSOR_LEASE_GRACE);
        assert_eq!(
            next_lease_state(now, false, false, true, pause, true),
            (false, None)
        );
    }

    #[test]
    fn returning_to_dashboard_resumes_renewal() {
        let now = std::time::Instant::now();
        let pause = Some(now + SENSOR_LEASE_GRACE);
        assert_eq!(
            next_lease_state(now, true, true, false, pause, false),
            (true, None)
        );
    }

    #[test]
    fn admin_errors_are_mapped_to_stable_codes() {
        for (suffix, code) in [
            ("NotAuthorized", "NotAuthorized"),
            ("PolicyKitUnavailable", "PolicyKitUnavailable"),
            ("InvalidArgs", "InvalidArgs"),
            ("PayloadTooLarge", "PayloadTooLarge"),
            ("Validation", "Validation"),
            ("InUse", "InUse"),
            ("ReloadFailed", "ReloadFailed"),
            ("WriteFailed", "WriteFailed"),
            ("RateLimited", "RateLimited"),
            ("Busy", "Busy"),
            ("NotSupported", "NotSupported"),
            ("FutureError", "Unknown"),
        ] {
            let full = format!("{ADMIN_ERROR_NAMESPACE}.{suffix}");
            assert_eq!(admin_error_code(&full), code, "suffixe {suffix}");
        }
        // Nom sans préfixe applicatif : jamais deviné.
        assert_eq!(
            admin_error_code("org.freedesktop.DBus.Error.AccessDenied"),
            "Unknown"
        );
        assert_eq!(admin_error_code(""), "Unknown");
    }

    #[test]
    fn admin_error_details_are_sanitized() {
        let validation = SaveError::Admin {
            code: "Validation",
            detail: "profiles[2].display.brightness".to_owned(),
        };
        assert_eq!(validation.detail(), "profiles[2].display.brightness");

        let path = SaveError::Admin {
            code: "WriteFailed",
            detail: "/var/lib/tuxedo-rust-center/profiles.json".to_owned(),
        };
        assert_eq!(path.detail(), "");

        let oversized = SaveError::Admin {
            code: "Validation",
            detail: "a".repeat(500),
        };
        assert_eq!(oversized.detail().chars().count(), 120);
    }

    #[test]
    fn version_strings_are_bounded_and_safe() {
        assert_eq!(sanitize_version("0.1.0"), "0.1.0");
        assert_eq!(sanitize_version("  0.1.0  "), "0.1.0");
        assert_eq!(sanitize_version(""), "");
        assert_eq!(sanitize_version("../../etc/passwd"), "");
        assert_eq!(sanitize_version("v1\n2"), "");
        assert_eq!(sanitize_version(&"9".repeat(65)), "");
    }

    #[test]
    fn admin_status_defaults_to_unavailable() {
        let status = AdminStatus::unavailable("Unavailable");
        assert!(!status.available);
        assert_eq!(status.error, "Unavailable");
        assert!(status.version.is_empty());
    }

    #[test]
    fn oversized_profiles_payload_is_rejected_before_dbus() {
        let huge = "x".repeat(crate::profile_edit::MAX_PROFILES_PAYLOAD_BYTES + 1);
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(save_profiles_on(
                BusKind::Session,
                "com.example.unused",
                &huge,
            ));
        assert_eq!(result, Err(SaveError::PayloadTooLarge));
    }

    /// Erreur factice au format `Admin1.Error.*`.
    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "org.tuxedocomputers.tccd.Admin1.Error")]
    enum FakeAdminError {
        NotAuthorized,
        Busy,
        ReloadFailed,
        InUse(String),
    }

    /// Comportement du faux service.
    #[derive(Debug, Clone)]
    enum FakeMode {
        Ok,
        Deny,
        Busy,
        ReloadFailed,
        InUse,
        ContractMismatch,
        Slow(std::time::Duration),
    }

    /// Service `Admin1` factice pour les tests sur bus de session.
    #[derive(Clone)]
    struct FakeStore {
        profiles: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        settings: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    struct FakeAdmin {
        store: FakeStore,
        mode: FakeMode,
    }

    impl FakeAdmin {
        fn check_mode(&self) -> Result<(), FakeAdminError> {
            match self.mode {
                FakeMode::Deny => Err(FakeAdminError::NotAuthorized),
                FakeMode::Busy => Err(FakeAdminError::Busy),
                FakeMode::ReloadFailed => Err(FakeAdminError::ReloadFailed),
                FakeMode::InUse => Err(FakeAdminError::InUse("custom-a".to_owned())),
                FakeMode::Slow(delay) => {
                    std::thread::sleep(delay);
                    Ok(())
                }
                FakeMode::Ok | FakeMode::ContractMismatch => Ok(()),
            }
        }
    }

    #[zbus::interface(name = "com.tuxedocomputers.tccd.Admin1")]
    impl FakeAdmin {
        #[zbus(name = "Ping")]
        fn ping(&self) -> bool {
            true
        }

        #[zbus(name = "GetVersion")]
        fn get_version(&self) -> String {
            "test-0.1".to_owned()
        }

        #[zbus(name = "GetContractVersion")]
        fn get_contract_version(&self) -> u32 {
            match self.mode {
                FakeMode::ContractMismatch => tcc_ipc::ADMIN_CONTRACT_VERSION + 1,
                _ => tcc_ipc::ADMIN_CONTRACT_VERSION,
            }
        }

        #[zbus(name = "SaveProfilesJSON")]
        fn save_profiles_json(&self, json: String) -> Result<bool, FakeAdminError> {
            self.check_mode()?;
            let mut saved = self
                .store
                .profiles
                .lock()
                .map_err(|_| FakeAdminError::Busy)?;
            saved.push(json);
            Ok(true)
        }

        #[zbus(name = "SaveSettingsJSON")]
        fn save_settings_json(&self, json: String) -> Result<bool, FakeAdminError> {
            self.check_mode()?;
            let mut saved = self
                .store
                .settings
                .lock()
                .map_err(|_| FakeAdminError::Busy)?;
            saved.push(json);
            Ok(true)
        }
    }

    /// Faux `Manager1` minimal pour `GetShutdownTimerJSON`.
    struct FakeManager {
        payload: std::sync::Mutex<String>,
    }

    #[zbus::interface(name = "com.tuxedocomputers.tccd.Manager1")]
    impl FakeManager {
        #[zbus(name = "GetShutdownTimerJSON")]
        fn get_shutdown_timer_json(&self) -> String {
            self.payload
                .lock()
                .map(|payload| payload.clone())
                .unwrap_or_default()
        }
    }

    /// Démarre un faux `Manager1` sur un nom unique du bus de session.
    async fn serve_fake_manager(suffix: &str, payload: &str) -> Option<(zbus::Connection, String)> {
        let connection = zbus::Connection::session().await.ok()?;
        let service = format!("com.tuxedocomputers.tccd.Manager.{suffix}");
        let name = zbus::names::WellKnownName::try_from(service.clone()).ok()?;
        connection.request_name(name).await.ok()?;
        connection
            .object_server()
            .at(
                tcc_ipc::DBUS_PATH,
                FakeManager {
                    payload: std::sync::Mutex::new(payload.to_owned()),
                },
            )
            .await
            .ok()?;
        Some((connection, service))
    }

    /// Démarre un faux `Admin1` sur un nom unique du bus de session.
    ///
    /// Renvoie `None` si aucun bus de session n'est disponible (CI sans
    /// `dbus-run-session`), comme les tests du daemon.
    async fn serve_fake_admin(
        suffix: &str,
        mode: FakeMode,
    ) -> Option<(zbus::Connection, String, FakeStore)> {
        let connection = zbus::Connection::session().await.ok()?;
        let service = format!("com.tuxedocomputers.tccd.Admin.{suffix}");
        let name = zbus::names::WellKnownName::try_from(service.clone()).ok()?;
        connection.request_name(name).await.ok()?;
        let store = FakeStore {
            profiles: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            settings: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        connection
            .object_server()
            .at(
                DBUS_ADMIN_PATH,
                FakeAdmin {
                    store: store.clone(),
                    mode,
                },
            )
            .await
            .ok()?;
        Some((connection, service, store))
    }

    /// Erreurs `AdminHw1` factices.
    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "org.tuxedocomputers.tccd.AdminHw1.Error")]
    enum FakeHwError {
        PolicyDenied(String),
        PartialApply(String),
        Busy,
    }

    const AGENT_DISPLAY_JSON: &str = r#"{
        "available": true,
        "session": { "type": "wayland", "desktop": "Hyprland",
                     "waylandDisplay": "wayland-1", "display": null },
        "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
        "internalOutput": {
            "name": "eDP-1",
            "connected": true,
            "enabled": true,
            "hasInternalDisplay": true,
            "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60.0,
                             "preferred": false, "current": true },
            "modes": [
                { "width": 2560, "height": 1600, "refreshHz": 60.0,
                  "preferred": false, "current": true },
                { "width": 2560, "height": 1600, "refreshHz": 165.0,
                  "preferred": true, "current": false }
            ]
        },
        "supported": true,
        "dryRun": false,
        "error": null
    }"#;

    /// Service `User1` factice sur le bus de session.
    struct FakeAgent {
        contract: u32,
        display_json: String,
    }

    #[zbus::interface(name = "com.tuxedocomputers.tccd.User1")]
    impl FakeAgent {
        #[zbus(name = "Ping")]
        fn ping(&self) -> bool {
            true
        }

        #[zbus(name = "GetVersion")]
        fn get_version(&self) -> String {
            "agent-test-0.3".to_owned()
        }

        #[zbus(name = "GetContractVersion")]
        fn get_contract_version(&self) -> u32 {
            self.contract
        }

        #[zbus(name = "GetDisplayInfoJSON")]
        fn get_display_info_json(&self) -> String {
            self.display_json.clone()
        }
    }

    async fn serve_fake_agent(
        suffix: &str,
        contract: u32,
        display_json: &str,
    ) -> Option<(zbus::Connection, String)> {
        let connection = zbus::Connection::session().await.ok()?;
        let service = format!("com.tuxedocomputers.tccd.User.{suffix}");
        let name = zbus::names::WellKnownName::try_from(service.clone()).ok()?;
        connection.request_name(name).await.ok()?;
        connection
            .object_server()
            .at(
                DBUS_SESSION_AGENT_PATH,
                FakeAgent {
                    contract,
                    display_json: display_json.to_owned(),
                },
            )
            .await
            .ok()?;
        Some((connection, service))
    }

    /// Erreur factice au format `User1.Error.*`.
    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "org.tuxedocomputers.tccd.User1.Error")]
    enum FakeUvcError {
        Busy,
        Stalled,
        OutOfRange,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeUvcMode {
        Changed,
        Unchanged,
        Busy,
        Stalled,
        OutOfRange,
    }

    type FakeUvcCalls = std::sync::Arc<std::sync::Mutex<Vec<(String, String, i32)>>>;

    /// Service `User1` factice avec écriture UVC.
    struct FakeUvcAgent {
        supported: bool,
        mode: FakeUvcMode,
        calls: FakeUvcCalls,
    }

    #[zbus::interface(name = "com.tuxedocomputers.tccd.User1")]
    impl FakeUvcAgent {
        #[zbus(name = "Ping")]
        fn ping(&self) -> bool {
            true
        }

        #[zbus(name = "GetVersion")]
        fn get_version(&self) -> String {
            "agent-uvc-0.4".to_owned()
        }

        #[zbus(name = "GetContractVersion")]
        fn get_contract_version(&self) -> u32 {
            SESSION_CONTRACT_VERSION
        }

        #[zbus(name = "GetDisplayInfoJSON")]
        fn get_display_info_json(&self) -> String {
            AGENT_DISPLAY_JSON.to_owned()
        }

        #[zbus(name = "GetUvcWriteSupported")]
        fn get_uvc_write_supported(&self) -> bool {
            self.supported
        }

        #[zbus(name = "SetUvcControl")]
        fn set_uvc_control(
            &self,
            device_path: String,
            control_key: String,
            value: i32,
        ) -> Result<bool, FakeUvcError> {
            match self.mode {
                FakeUvcMode::Busy => return Err(FakeUvcError::Busy),
                FakeUvcMode::Stalled => return Err(FakeUvcError::Stalled),
                FakeUvcMode::OutOfRange => return Err(FakeUvcError::OutOfRange),
                FakeUvcMode::Changed | FakeUvcMode::Unchanged => {}
            }
            self.calls.lock().map_err(|_| FakeUvcError::Busy)?.push((
                device_path,
                control_key,
                value,
            ));
            Ok(self.mode == FakeUvcMode::Changed)
        }
    }

    async fn serve_fake_uvc(
        suffix: &str,
        supported: bool,
        mode: FakeUvcMode,
    ) -> Option<(zbus::Connection, String, FakeUvcCalls)> {
        let connection = zbus::Connection::session().await.ok()?;
        let service = format!("com.tuxedocomputers.tccd.User.{suffix}");
        let name = zbus::names::WellKnownName::try_from(service.clone()).ok()?;
        connection.request_name(name).await.ok()?;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        connection
            .object_server()
            .at(
                DBUS_SESSION_AGENT_PATH,
                FakeUvcAgent {
                    supported,
                    mode,
                    calls: calls.clone(),
                },
            )
            .await
            .ok()?;
        Some((connection, service, calls))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeHwMode {
        Ok,
        PpdOwned,
        Partial,
        Busy,
    }

    /// Service `Admin.Hw` factice sur bus de session.
    struct FakeHw {
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        mode: FakeHwMode,
    }

    #[zbus::interface(name = "com.tuxedocomputers.tccd.AdminHw1")]
    impl FakeHw {
        #[zbus(name = "Ping")]
        fn ping(&self) -> bool {
            true
        }

        #[zbus(name = "GetVersion")]
        fn get_version(&self) -> String {
            "hw-test-0.1".to_owned()
        }

        #[zbus(name = "GetContractVersion")]
        fn get_contract_version(&self) -> u32 {
            ADMIN_HW_CONTRACT_VERSION
        }

        #[zbus(name = "GetTdpValues")]
        fn get_tdp_values(&self) -> Result<Vec<i32>, FakeHwError> {
            if self.mode == FakeHwMode::Busy {
                return Err(FakeHwError::Busy);
            }
            Ok(vec![35, 45])
        }

        #[zbus(name = "SetTdpValues")]
        fn set_tdp_values(&self, values: Vec<i32>) -> Result<bool, FakeHwError> {
            if self.mode == FakeHwMode::Busy {
                return Err(FakeHwError::Busy);
            }
            if self.mode == FakeHwMode::Partial {
                let descriptor = values.first().copied().unwrap_or_default();
                return Err(FakeHwError::PartialApply(format!("pl{descriptor}")));
            }
            self.calls
                .lock()
                .map_err(|_| FakeHwError::Busy)?
                .push(format!("tdp:{values:?}"));
            Ok(true)
        }

        #[zbus(name = "SetOdmProfile")]
        fn set_odm_profile(&self, name: String) -> Result<bool, FakeHwError> {
            self.set_odm(name, false)
        }

        #[zbus(name = "SetOdmProfileOverride")]
        fn set_odm_profile_override(&self, name: String) -> Result<bool, FakeHwError> {
            self.set_odm(name, true)
        }

        #[zbus(name = "SetNvidiaCtgpOffset")]
        fn set_nvidia_ctgp_offset(&self, watts: i32) -> Result<bool, FakeHwError> {
            if self.mode == FakeHwMode::Busy {
                return Err(FakeHwError::Busy);
            }
            self.calls
                .lock()
                .map_err(|_| FakeHwError::Busy)?
                .push(format!("ctgp:{watts}"));
            Ok(true)
        }
    }

    impl FakeHw {
        fn set_odm(&self, name: String, force: bool) -> Result<bool, FakeHwError> {
            if self.mode == FakeHwMode::Busy {
                return Err(FakeHwError::Busy);
            }
            if self.mode == FakeHwMode::PpdOwned && !force {
                return Err(FakeHwError::PolicyDenied("ppd.platform_profile".to_owned()));
            }
            self.calls
                .lock()
                .map_err(|_| FakeHwError::Busy)?
                .push(format!("odm:{name}:force={force}"));
            Ok(true)
        }
    }

    async fn serve_fake_hw(
        suffix: &str,
        mode: FakeHwMode,
    ) -> Option<(
        zbus::Connection,
        String,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    )> {
        let connection = zbus::Connection::session().await.ok()?;
        let service = format!("com.tuxedocomputers.tccd.Admin.Hw.{suffix}");
        let name = zbus::names::WellKnownName::try_from(service.clone()).ok()?;
        connection.request_name(name).await.ok()?;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        connection
            .object_server()
            .at(
                DBUS_ADMIN_HW_PATH,
                FakeHw {
                    calls: calls.clone(),
                    mode,
                },
            )
            .await
            .ok()?;
        Some((connection, service, calls))
    }

    #[test]
    fn admin_hw_local_bounds_match_the_server() {
        // TDP : 1–3 valeurs, 1–1000 W.
        assert!(admin_hw_tdp_values_valid(&[35]));
        assert!(admin_hw_tdp_values_valid(&[35, 45, 65]));
        assert!(!admin_hw_tdp_values_valid(&[65, 45]));
        assert!(!admin_hw_tdp_values_valid(&[35, 45, 65, 80]));
        assert!(!admin_hw_tdp_values_valid(&[]));
        assert!(!admin_hw_tdp_values_valid(&[0, 45]));
        assert!(!admin_hw_tdp_values_valid(&[35, 1001]));

        // ODM : ≤ 64 caractères ASCII graphiques, sans antislash.
        assert!(admin_hw_odm_name_valid("balanced"));
        assert!(admin_hw_odm_name_valid(&"a".repeat(64)));
        assert!(!admin_hw_odm_name_valid(&"a".repeat(65)));
        assert!(!admin_hw_odm_name_valid(""));
        assert!(!admin_hw_odm_name_valid("caf\u{e9}"));
        assert!(!admin_hw_odm_name_valid("power\\save"));

        // cTGP : 0–200 W, pas de négatif.
        assert!(admin_hw_ctgp_valid(0));
        assert!(admin_hw_ctgp_valid(200));
        assert!(!admin_hw_ctgp_valid(-1));
        assert!(!admin_hw_ctgp_valid(201));
    }

    #[test]
    fn admin_hw_error_codes_are_stable() {
        for (suffix, expected) in [
            ("NotAuthorized", "NotAuthorized"),
            ("InvalidArgs", "InvalidArgs"),
            ("Validation", "Validation"),
            ("PolicyDenied", "PolicyDenied"),
            ("NotSupported", "NotSupported"),
            ("WriteFailed", "WriteFailed"),
            ("PartialApply", "PartialApply"),
            ("RateLimited", "RateLimited"),
            ("Busy", "Busy"),
            ("UnknownThing", "Unknown"),
        ] {
            let name = format!("{ADMIN_HW_ERROR_NAMESPACE}.{suffix}");
            assert_eq!(admin_hw_error_code(&name), expected);
        }
        assert_eq!(admin_hw_error_code("org.example.Other"), "Unknown");
    }

    #[test]
    fn fake_agent_probes_and_reads_internal_display() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service)) =
                serve_fake_agent("testok", SESSION_CONTRACT_VERSION, AGENT_DISPLAY_JSON).await
            else {
                eprintln!("bus de session absent : test User1 ignoré");
                return;
            };
            let status = probe_agent_on(BusKind::Session, &service).await;
            assert!(status.available);
            assert!(status.owned);
            assert_eq!(status.error, "");
            assert_eq!(status.version, "agent-test-0.3");
            // Agent sans `GetUvcWriteSupported` : écriture non supportée, sans
            // erreur de service (`UnknownMethod` est toléré).
            assert!(!status.uvc_write_supported);

            let json = fetch_display_info_on(BusKind::Session, &service)
                .await
                .expect("rapport");
            let view: crate::display_view::DisplayInfoView =
                serde_json::from_str(&json).expect("vue");
            assert!(view.supported);
            assert_eq!(view.internal_output.name, "eDP-1");
            assert_eq!(view.internal_output.modes.len(), 2);
            // Seuls les modes internes sont publiés (`eDP-1` contient « DP-1 »).
            assert!(json.contains("\"eDP-1\""));
            assert!(!json.contains("\"DP-1\""));
            drop(connection);
        });
    }

    #[test]
    fn fake_agent_rejects_incompatible_contract() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service)) =
                serve_fake_agent("testbad", SESSION_CONTRACT_VERSION + 1, AGENT_DISPLAY_JSON).await
            else {
                return;
            };
            let status = probe_agent_on(BusKind::Session, &service).await;
            assert!(!status.available);
            assert!(!status.owned);
            assert_eq!(status.error, "Contract");
            let error = fetch_display_info_on(BusKind::Session, &service)
                .await
                .expect_err("contrat refusé");
            assert_eq!(agent_error_code(&error), "Contract");
            drop(connection);
        });
    }

    #[test]
    fn agent_uvc_local_validation_matches_the_service() {
        // Chemins canoniques produits par udev.
        for path in ["/dev/video0", "/dev/video7", "/dev/video4096"] {
            assert!(agent_uvc_device_path_valid(path), "{path}");
        }
        for path in [
            "",
            "/dev/video",
            "/dev/video00",
            "/dev/video-1",
            "/dev/video12345",
            "/dev/video0/../video1",
            "/tmp/video0",
            "/dev/video0 ",
            "/dev/videoé",
        ] {
            assert!(!agent_uvc_device_path_valid(path), "{path}");
        }
        assert!(!agent_uvc_device_path_valid(&format!(
            "/dev/{}",
            "v".repeat(MAX_UVC_DEVICE_PATH_BYTES)
        )));

        for key in ["brightness", "focus_auto", "a1_", "_", &"a".repeat(64)] {
            assert!(agent_uvc_control_key_valid(key), "{key}");
        }
        for key in [
            "",
            "Brightness",
            "focus-auto",
            "focus auto",
            "clé",
            "A",
            &"a".repeat(65),
        ] {
            assert!(!agent_uvc_control_key_valid(key), "{key}");
        }
    }

    #[test]
    fn agent_uvc_error_codes_are_stable() {
        for (name, expected) in [
            (
                "org.tuxedocomputers.tccd.User1.Error.OutOfRange",
                "OutOfRange",
            ),
            ("org.tuxedocomputers.tccd.User1.Error.Busy", "Busy"),
            ("org.tuxedocomputers.tccd.User1.Error.Stalled", "Stalled"),
            ("org.freedesktop.DBus.Error.InvalidArgs", "InvalidArgs"),
            ("org.freedesktop.DBus.Error.NotSupported", "NotSupported"),
            ("org.freedesktop.DBus.Error.UnknownMethod", "NotSupported"),
            (
                "org.freedesktop.DBus.Error.LimitsExceeded",
                "LimitsExceeded",
            ),
            ("org.freedesktop.DBus.Error.Failed", "Failed"),
            ("org.freedesktop.DBus.Error.ServiceUnknown", "Unavailable"),
            ("org.freedesktop.DBus.Error.NameHasNoOwner", "Unavailable"),
            ("org.freedesktop.DBus.Error.NoReply", "Failed"),
            ("org.example.Other", "Failed"),
        ] {
            assert_eq!(uvc_error_code_for_name(name).code, expected, "{name}");
            // Aucun message serveur n'est propagé.
            assert_eq!(uvc_error_code_for_name(name).detail, "");
        }
    }

    #[test]
    fn fake_uvc_agent_writes_and_reports_support() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, calls)) =
                serve_fake_uvc("uvcok", true, FakeUvcMode::Changed).await
            else {
                eprintln!("bus de session absent : test User1.SetUvcControl ignoré");
                return;
            };
            let status = probe_agent_on(BusKind::Session, &service).await;
            assert!(status.available);
            assert!(status.owned);
            assert!(status.uvc_write_supported);

            // `true` = valeur écrite.
            assert!(
                set_uvc_control_on(BusKind::Session, &service, "/dev/video0", "brightness", 42)
                    .await
                    .expect("écriture")
            );
            let stored = calls.lock().expect("verrou").clone();
            assert_eq!(
                stored,
                vec![("/dev/video0".to_owned(), "brightness".to_owned(), 42)]
            );
            drop(connection);

            // `false` = valeur déjà courante : idempotent, pas une erreur.
            let Some((connection, service, calls)) =
                serve_fake_uvc("uvcsame", true, FakeUvcMode::Unchanged).await
            else {
                return;
            };
            assert!(
                !set_uvc_control_on(BusKind::Session, &service, "/dev/video0", "brightness", 42)
                    .await
                    .expect("idempotent")
            );
            assert_eq!(calls.lock().expect("verrou").len(), 1);
            drop(connection);

            // Capacité `false` sans erreur de service.
            let Some((connection, service, _calls)) =
                serve_fake_uvc("uvcoff", false, FakeUvcMode::Changed).await
            else {
                return;
            };
            let status = probe_agent_on(BusKind::Session, &service).await;
            assert!(status.available);
            assert!(!status.uvc_write_supported);
            drop(connection);
        });
    }

    #[test]
    fn fake_uvc_agent_maps_typed_errors() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            for (suffix, mode, expected) in [
                ("uvcbusy", FakeUvcMode::Busy, "Busy"),
                ("uvcstall", FakeUvcMode::Stalled, "Stalled"),
                ("uvcrange", FakeUvcMode::OutOfRange, "OutOfRange"),
            ] {
                let Some((connection, service, _calls)) = serve_fake_uvc(suffix, true, mode).await
                else {
                    eprintln!("bus de session absent : test erreurs UVC ignoré");
                    return;
                };
                let failure =
                    set_uvc_control_on(BusKind::Session, &service, "/dev/video0", "brightness", 1)
                        .await
                        .expect_err("erreur attendue");
                assert_eq!(failure.code, expected);
                assert_eq!(failure.detail, "");
                drop(connection);
            }
        });
    }

    #[test]
    fn uvc_service_absent_is_reported_unavailable() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let failure = set_uvc_control_on(
                BusKind::Session,
                "com.example.tcc.uvc.absent",
                "/dev/video0",
                "brightness",
                1,
            )
            .await
            .expect_err("service absent");
            // Pas de bus de session en CI headless : le transport est également
            // une indisponibilité côté UI.
            assert!(matches!(failure.code, "Unavailable" | "Transport"));
        });
    }

    #[test]
    fn queue_accepts_and_bounds_uvc_writes() {
        let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let sender = CommandSender { tx };
        sender
            .try_send(Command::SetUvcControl {
                device_path: "/dev/video0".to_owned(),
                control_key: "brightness".to_owned(),
                value: 42,
            })
            .expect("uvc");
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .unwrap(),
            Command::SetUvcControl {
                device_path: "/dev/video0".to_owned(),
                control_key: "brightness".to_owned(),
                value: 42,
            }
        );
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            assert!(sender.try_send(Command::Refresh).is_ok());
        }
        assert_eq!(
            sender.try_send(Command::Refresh).unwrap_err(),
            ClientError::Busy
        );
    }

    #[test]
    fn display_info_decode_rejects_oversized_or_invalid_payloads() {
        // Le JSON externe n'est jamais conservé : seule la sous-section
        // `internalOutput` est désérialisée.
        let raw = r#"{
            "available": true,
            "internalOutput": { "name": "eDP-1", "modes": [] },
            "externalOutput": { "name": "HDMI-1", "modes": [] }
        }"#;
        let (_view, canonical) = decode_display_info(raw).expect("payload");
        assert!(canonical.contains("\"internalOutput\""));
        assert!(!canonical.contains("\"externalOutput\""));
        assert!(!canonical.contains("\"HDMI-1\""));

        // Modes invalides filtrés côté Rust avant publication.
        let raw = r#"{
            "available": true,
            "internalOutput": {
                "name": "eDP-1",
                "currentMode": { "width": 2560, "height": 1600, "refreshHz": 0 },
                "modes": [
                    { "width": 0, "height": 1600, "refreshHz": 60 },
                    { "width": 2560, "height": 1600, "refreshHz": 2000 },
                    { "width": 2560, "height": 1600, "refreshHz": 120 }
                ]
            }
        }"#;
        let (_view, canonical) = decode_display_info(raw).expect("modes");
        assert!(!canonical.contains("\"width\":0"));
        assert!(!canonical.contains("\"refreshHz\":2000"));
        assert!(canonical.contains("\"refreshHz\":120"));
        assert!(!canonical.contains("\"currentMode\":{"));
        assert!(decode_display_info("[]").is_err());
        assert!(decode_display_info(r#"{ "internalOutput": { "modes": 3 } }"#).is_err());
    }

    #[test]
    fn fake_admin_hw_probes_and_applies_transient_settings() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, calls)) = serve_fake_hw("testok", FakeHwMode::Ok).await
            else {
                eprintln!("bus de session absent : test Admin.Hw ignoré");
                return;
            };
            let status = probe_admin_hw_on(BusKind::Session, &service).await;
            assert!(status.available);
            assert_eq!(status.error, "");
            assert_eq!(status.version, "hw-test-0.1");

            assert_eq!(
                get_tdp_values_on(BusKind::Session, &service)
                    .await
                    .expect("lecture TDP"),
                vec![35, 45]
            );
            set_odm_on(BusKind::Session, &service, "quiet", false)
                .await
                .expect("ODM");
            set_odm_on(BusKind::Session, &service, "performance", true)
                .await
                .expect("ODM override");
            set_tdp_on(BusKind::Session, &service, vec![40, 55])
                .await
                .expect("TDP");
            set_ctgp_on(BusKind::Session, &service, 12)
                .await
                .expect("cTGP");
            let stored = calls.lock().expect("verrou").clone();
            assert!(stored.iter().any(|call| call == "odm:quiet:force=false"));
            assert!(
                stored
                    .iter()
                    .any(|call| call == "odm:performance:force=true")
            );
            assert!(stored.iter().any(|call| call == "tdp:[40, 55]"));
            assert!(stored.iter().any(|call| call == "ctgp:12"));
            drop(connection);
        });
    }

    #[test]
    fn fake_admin_hw_maps_ppd_and_partial_errors() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, _calls)) =
                serve_fake_hw("testppd", FakeHwMode::PpdOwned).await
            else {
                eprintln!("bus de session absent : test Admin.Hw ignoré");
                return;
            };
            let error = set_odm_on(BusKind::Session, &service, "quiet", false)
                .await
                .expect_err("policy");
            assert_eq!(error.code(), "PolicyDenied");
            // L'override avec consentement passe.
            set_odm_on(BusKind::Session, &service, "quiet", true)
                .await
                .expect("override");
            drop(connection);

            let Some((connection, service, _calls)) =
                serve_fake_hw("testpartial", FakeHwMode::Partial).await
            else {
                return;
            };
            let error = set_tdp_on(BusKind::Session, &service, vec![35, 45])
                .await
                .expect_err("partial");
            assert_eq!(error.code(), "PartialApply");
            assert!(!error.detail().is_empty());
            drop(connection);
        });
    }

    #[test]
    fn fake_admin_service_success_roundtrip() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, store)) =
                serve_fake_admin("testprobe", FakeMode::Ok).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };

            let status = probe_admin_on(BusKind::Session, &service).await;
            assert!(status.available, "service détecté");
            assert_eq!(status.error, "");
            assert_eq!(status.version, "test-0.1");

            let prepared = prepare_edit();
            save_profiles_on(BusKind::Session, &service, &prepared.json)
                .await
                .expect("enregistrement accepté");

            let stored = store.profiles.lock().expect("verrou").clone();
            assert_eq!(stored.len(), 1);
            let decoded: serde_json::Value = serde_json::from_str(&stored[0]).expect("JSON");
            assert_eq!(decoded[1]["name"], "Studio 2");
            // Le preset n'est pas transformé en profil personnalisé.
            assert_eq!(decoded[0]["id"], "__office__");

            drop(connection);
        });
    }

    #[test]
    fn fake_admin_service_maps_not_authorized() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, store)) =
                serve_fake_admin("testdeny", FakeMode::Deny).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };

            let prepared = prepare_edit();
            let error = save_profiles_on(BusKind::Session, &service, &prepared.json)
                .await
                .expect_err("refus attendu");
            assert_eq!(error.code(), "NotAuthorized");
            assert!(store.profiles.lock().expect("verrou").is_empty());

            // Un nom absent est signalé sans panic.
            let missing = probe_admin_on(BusKind::Session, "com.example.absent").await;
            assert!(!missing.available);

            drop(connection);
        });
    }

    /// Collection + édition valides pour les tests D-Bus.
    fn prepare_edit() -> crate::profile_edit::PreparedProfiles {
        use tcc_core::{Profile, ProfileCollection};

        let collection = ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Studio"),
        ]);
        let edit = decode_edit(
            r#"{
                "name": "Studio 2",
                "description": "Description",
                "brightness": 70,
                "useBrightness": true,
                "refreshRate": 120,
                "useRefreshRate": true,
                "onlineCores": 6,
                "useMaxPerfGov": false,
                "scalingMinKhz": 800000,
                "scalingMaxKhz": 3500000,
                "energyPerformancePreference": "balance_power",
                "noTurbo": true,
                "webcamStatus": false,
                "useWebcamStatus": true,
                "fanUseControl": true,
                "fanProfile": "quiet",
                "fanMin": 10,
                "fanMax": 55,
                "fanOffset": 0
            }"#,
        )
        .expect("DTO");
        apply_edit(&collection, "custom-a", &edit).expect("édition valide")
    }

    fn save_base() -> tcc_core::ProfileCollection {
        use tcc_core::{Profile, ProfileCollection};
        ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Studio"),
        ])
    }

    fn save_edit() -> String {
        r#"{
            "name": "Studio 2",
            "description": "",
            "brightness": 70,
            "useBrightness": true,
            "refreshRate": null,
            "useRefreshRate": false,
            "useMaxPerfGov": false,
            "energyPerformancePreference": "balance_power",
            "noTurbo": false,
            "webcamStatus": true,
            "useWebcamStatus": true,
            "fanUseControl": true,
            "fanProfile": "balanced",
            "fanMin": 0,
            "fanMax": 100,
            "fanOffset": 0
        }"#
        .to_owned()
    }

    #[test]
    fn fake_admin_maps_busy_and_reload_failed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            for (suffix, mode, code) in [
                ("testbusy", FakeMode::Busy, "Busy"),
                ("testreload", FakeMode::ReloadFailed, "ReloadFailed"),
            ] {
                let Some((connection, service, store)) = serve_fake_admin(suffix, mode).await
                else {
                    eprintln!("bus de session absent : test Admin1 ignoré");
                    return;
                };
                let error = save_profiles_on(BusKind::Session, &service, &prepare_edit().json)
                    .await
                    .expect_err("erreur attendue");
                assert_eq!(error.code(), code, "mode {suffix}");
                assert!(store.profiles.lock().expect("verrou").is_empty());
                drop(connection);
            }
        });
    }

    fn settings_base() -> tcc_core::Settings {
        let mut settings = tcc_core::Settings {
            charging_profile: Some("balanced".to_owned()),
            ..tcc_core::Settings::default()
        };
        settings.extra.insert(
            "futureField".to_owned(),
            serde_json::json!({ "kept": true }),
        );
        settings
    }

    fn settings_edit() -> String {
        r#"{
            "fahrenheit": true,
            "cpuSettingsEnabled": false,
            "fanControlEnabled": true,
            "keyboardBacklightControlEnabled": false
        }"#
        .to_owned()
    }

    #[test]
    fn settings_pipeline_always_returns_an_outcome() {
        use std::cell::Cell;

        let base = settings_base();
        let called = Cell::new(false);

        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &settings_edit(),
            None,
            None,
            |_| {
                called.set(true);
                Ok(())
            },
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        assert!(called.get());

        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &settings_edit(),
            None,
            None,
            |_| {
                Err(SaveError::Admin {
                    code: "Busy",
                    detail: String::new(),
                })
            },
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Admin { code: "Busy", .. })
        ));

        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            "{bad",
            None,
            None,
            |_| {
                panic!("ne doit pas être appelé");
            },
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::InvalidPayload)
        ));

        let outcome = run_save_settings(
            &base,
            Err(ClientError::Timeout {
                call: "GetSettingsJSON",
            }),
            &settings_edit(),
            None,
            None,
            |_| panic!("ne doit pas être appelé"),
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Timeout)
        ));
    }

    #[test]
    fn settings_pipeline_rejects_concurrent_change() {
        use std::cell::Cell;

        let base = settings_base();
        let mut current = base.clone();
        current.charging_start_threshold = Some(70);

        let called = Cell::new(false);
        let outcome = run_save_settings(
            &base,
            Ok((current, "{}".to_owned())),
            &settings_edit(),
            None,
            None,
            |_| {
                called.set(true);
                Ok(())
            },
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Conflict)
        ));
        assert!(!called.get());
    }

    #[test]
    fn settings_pipeline_validates_charge_against_hardware() {
        let base = settings_base();
        let capabilities = ChargeCapabilities::from_charging(&Charging {
            profiles: vec!["balanced".to_owned(), "high-capacity".to_owned()],
            priorities: vec!["balanced".to_owned()],
            start_thresholds: vec![40, 80],
            end_thresholds: vec![50, 90],
            thresholds_available: true,
            ..Charging::default()
        });
        let edit = |fields: &str| {
            format!(
                r#"{{"fahrenheit": false, "cpuSettingsEnabled": true,
                    "fanControlEnabled": true, "keyboardBacklightControlEnabled": true,
                    {fields}}}"#
            )
        };

        // Valeur hors liste : refusée avant tout appel admin.
        let invalid = edit(r#""chargingProfile": "turbo""#);
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &invalid,
            Some(&capabilities),
            None,
            |_| panic!("ne doit pas être appelé"),
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Validation { .. })
        ));

        // Valeur valide : sérialisée et envoyée.
        let valid = edit(
            r#""chargingProfile": "high-capacity",
               "chargingStartThreshold": 80,
               "chargingEndThreshold": 90"#,
        );
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &valid,
            Some(&capabilities),
            None,
            |json| {
                let value: serde_json::Value = serde_json::from_str(json).expect("JSON");
                assert_eq!(value["chargingProfile"], "high-capacity");
                assert_eq!(value["chargingStartThreshold"], 80);
                assert_eq!(value["chargingEndThreshold"], 90);
                // Les autres champs restent intacts.
                assert!(json.contains("futureField"));
                Ok(())
            },
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));
    }

    #[test]
    fn settings_pipeline_validates_state_matrix_against_live_profiles() {
        let base = settings_base();
        let profiles = tcc_core::ProfileCollection::new(vec![
            tcc_core::Profile::new("__office__", "Office"),
            tcc_core::Profile::new("custom-studio", "Studio"),
        ]);
        let edit = |matrix: &str| {
            format!(
                r#"{{"fahrenheit": false, "cpuSettingsEnabled": true,
                    "fanControlEnabled": true, "keyboardBacklightControlEnabled": true,
                    "stateMatrix": {matrix}}}"#
            )
        };

        // Matrice partielle valide : remplacée et sérialisée.
        let valid = edit(r#"[{"ppd": "balanced", "power": "ac", "profileId": "custom-studio"}]"#);
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &valid,
            None,
            Some(&profiles),
            |json| {
                let value: serde_json::Value = serde_json::from_str(json).expect("JSON");
                assert_eq!(value["stateMatrix"]["balanced/ac"], "custom-studio");
                assert_eq!(value["stateMatrix"].as_object().expect("objet").len(), 1);
                assert!(json.contains("futureField"));
                Ok(())
            },
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));

        // Profil supprimé par un autre client : refusé avant l'écriture.
        let invalid = edit(r#"[{"ppd": "balanced", "power": "ac", "profileId": "custom-gone"}]"#);
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &invalid,
            None,
            Some(&profiles),
            |_| panic!("ne doit pas être appelé"),
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Validation { .. })
        ));
    }

    #[test]
    fn settings_pipeline_accepts_boolean_only_edits_without_hardware() {
        let base = settings_base();
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            &settings_edit(),
            None,
            None,
            |_| Ok(()),
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));
    }

    #[test]
    fn shutdown_timer_status_is_decoded_and_bounded() {
        let view = decode_shutdown_timer(
            r#"{"enabled":true,"scheduledAt":"2026-11-06 22:30",
                "source":"settings","error":null}"#,
        )
        .expect("statut");
        assert!(view.enabled);
        assert_eq!(view.scheduled_at.as_deref(), Some("2026-11-06 22:30"));
        assert_eq!(view.source, "settings");
        assert!(view.error.is_none());

        let view =
            decode_shutdown_timer(r#"{"enabled":false,"source":"none"}"#).expect("statut minimal");
        assert!(!view.enabled);
        assert_eq!(view.scheduled_at, None);

        for payload in ["[1, 2]", r#"{"enabled":"yes"}"#, "{ pas du json"] {
            assert!(matches!(
                decode_shutdown_timer(payload),
                Err(ClientError::Decode {
                    model: "ShutdownTimer",
                    ..
                })
            ));
        }
        let raw = "x".repeat(MAX_RESPONSE_BYTES + 1);
        assert!(matches!(
            decode_shutdown_timer(&raw),
            Err(ClientError::ResponseTooLarge {
                call: "GetShutdownTimerJSON",
                ..
            })
        ));
    }

    #[test]
    fn shutdown_time_is_applied_through_the_settings_pipeline() {
        let base = settings_base();
        let edit = r#"{"fahrenheit": false, "cpuSettingsEnabled": true,
                       "fanControlEnabled": true, "keyboardBacklightControlEnabled": true,
                       "shutdownTime": "22:30", "clearShutdownTime": false}"#;
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            edit,
            None,
            None,
            |json| {
                let value: serde_json::Value = serde_json::from_str(json).expect("JSON");
                assert_eq!(value["shutdownTime"], "22:30");
                assert!(json.contains("futureField"));
                Ok(())
            },
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));

        let clear = r#"{"fahrenheit": false, "cpuSettingsEnabled": true,
                        "fanControlEnabled": true, "keyboardBacklightControlEnabled": true,
                        "shutdownTime": null, "clearShutdownTime": true}"#;
        let outcome = run_save_settings(
            &base,
            Ok((base.clone(), "{}".to_owned())),
            clear,
            None,
            None,
            |_| Ok(()),
        );
        assert!(matches!(outcome, super::SaveOutcome::Ok));
    }

    #[test]
    fn settings_json_is_bounded_and_must_be_an_object() {
        let error = decode_settings("[1, 2]").unwrap_err();
        assert!(matches!(
            error,
            ClientError::Decode {
                model: "Settings",
                ..
            }
        ));
        let raw = "x".repeat(MAX_RESPONSE_BYTES + 1);
        assert!(matches!(
            decode_settings(&raw).unwrap_err(),
            ClientError::ResponseTooLarge {
                call: "GetSettingsJSON",
                ..
            }
        ));
        // Un schéma courant reste valide.
        let (_settings, json) =
            decode_settings(r#"{ "schemaVersion": 1, "fahrenheit": true }"#).expect("objet");
        assert!(json.contains("\"fahrenheit\":true"));
    }

    #[test]
    fn settings_validation_rejects_inconsistent_payloads() {
        // Version de schéma inconnue (legacy non migré).
        let error = decode_settings(r#"{ "schemaVersion": 99 }"#).unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "Settings",
                ..
            }
        ));
        assert!(error.to_string().contains("schemaVersion"));

        // Champ obligatoire absent → schéma 0 par défaut → rejet.
        let error = decode_settings(r#"{ "fahrenheit": true }"#).unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "Settings",
                ..
            }
        ));

        // Référence de profil syntaxiquement invalide (clés legacy snake_case).
        let error = decode_settings(
            r#"{ "schemaVersion": 1, "stateMap": { "power_ac": "../etc", "power_bat": "ok" } }"#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "Settings",
                ..
            }
        ));
        assert!(!error.to_string().contains('/'));

        // Seuil de charge hors bornes.
        let error = decode_settings(r#"{ "schemaVersion": 1, "chargingStartThreshold": 120 }"#)
            .unwrap_err();
        assert!(error.to_string().contains("chargingStartThreshold"));
    }

    #[test]
    fn settings_references_must_exist_in_the_collection() {
        use tcc_core::{Profile, ProfileCollection, Settings};

        let profiles = ProfileCollection::new(vec![Profile::new("custom-a", "Studio")]);
        let mut settings = Settings::default();
        settings.state_map.power_ac = tcc_core::ProfileId::new("custom-a");
        settings.state_map.power_bat = tcc_core::ProfileId::new("custom-a");
        assert!(validate_settings_against_profiles(&settings, &profiles).is_ok());

        // Un autre client a supprimé le profil référencé.
        settings.state_map.power_bat = tcc_core::ProfileId::new("custom-gone");
        let error = validate_settings_against_profiles(&settings, &profiles).unwrap_err();
        assert!(matches!(
            error,
            ClientError::Invalid {
                model: "Settings",
                ..
            }
        ));
        assert!(error.to_string().contains("stateMap.power_bat"));
    }

    #[test]
    fn worker_monitor_runs_after_exit() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let handle =
            std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_millis(50)));
        monitor_worker(handle, move || flag.store(true, Ordering::SeqCst));
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn shutdown_timer_is_read_from_a_fake_manager() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service)) = serve_fake_manager(
                "testtimer",
                r#"{"enabled":true,"scheduledAt":"2026-11-06 22:30",
                    "source":"settings","error":null}"#,
            )
            .await
            else {
                eprintln!("bus de session absent : test Manager1 ignoré");
                return;
            };
            let view = fetch_shutdown_timer_on(BusKind::Session, &service)
                .await
                .expect("statut lu");
            assert!(view.enabled);
            assert_eq!(view.scheduled_at.as_deref(), Some("2026-11-06 22:30"));
            assert_eq!(view.source, "settings");
            drop(connection);
        });
    }

    #[test]
    fn fake_admin_saves_settings_and_maps_busy() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // Succès : le payload settings est stocké.
            let Some((connection, service, store)) =
                serve_fake_admin("testsetok", FakeMode::Ok).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            save_settings_on(BusKind::Session, &service, &settings_edit())
                .await
                .expect("enregistrement accepté");
            let stored = store.settings.lock().expect("verrou").clone();
            assert_eq!(stored.len(), 1);
            assert!(stored[0].contains("fahrenheit"));
            assert!(store.profiles.lock().expect("verrou").is_empty());
            drop(connection);

            // Refus admin.
            let Some((connection, service, _store)) =
                serve_fake_admin("testsetbusy", FakeMode::Busy).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            let error = save_settings_on(BusKind::Session, &service, &settings_edit())
                .await
                .expect_err("refus attendu");
            assert_eq!(error.code(), "Busy");
            drop(connection);
        });
    }

    #[test]
    fn slow_settings_admin_times_out() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, _store)) = serve_fake_admin(
                "testsetslow",
                FakeMode::Slow(std::time::Duration::from_millis(400)),
            )
            .await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            let error = save_settings_with_timeout(
                BusKind::Session,
                &service,
                &settings_edit(),
                std::time::Duration::from_millis(80),
            )
            .await
            .expect_err("timeout attendu");
            assert_eq!(error, SaveError::Timeout);
            drop(connection);
        });
    }

    #[test]
    fn fake_admin_maps_in_use_with_profile_id() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, _store)) =
                serve_fake_admin("testinuse", FakeMode::InUse).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            let error = save_profiles_on(BusKind::Session, &service, &prepare_edit().json)
                .await
                .expect_err("refus attendu");
            assert_eq!(error.code(), "InUse");
            // Seul l'identifiant est relayé (pas de contenu de fichier).
            assert_eq!(error.detail(), "custom-a");
            drop(connection);
        });
    }

    #[test]
    fn fake_admin_reports_contract_mismatch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, _store)) =
                serve_fake_admin("testcontract", FakeMode::ContractMismatch).await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            let status = probe_admin_on(BusKind::Session, &service).await;
            assert!(!status.available);
            assert_eq!(status.error, "Contract");
            drop(connection);
        });
    }

    #[test]
    fn slow_admin_times_out_without_hanging_the_worker() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let Some((connection, service, _store)) = serve_fake_admin(
                "testslow",
                FakeMode::Slow(std::time::Duration::from_millis(400)),
            )
            .await
            else {
                eprintln!("bus de session absent : test Admin1 ignoré");
                return;
            };
            let error = save_profiles_with_timeout(
                BusKind::Session,
                &service,
                &prepare_edit().json,
                std::time::Duration::from_millis(80),
            )
            .await
            .expect_err("timeout attendu");
            assert_eq!(error, SaveError::Timeout);
            drop(connection);
        });
    }

    fn run_save_for_test(
        base: &tcc_core::ProfileCollection,
        current: tcc_core::ProfileCollection,
        id: &str,
        edit: &str,
        save: impl FnOnce(&str) -> Result<(), SaveError>,
    ) -> super::SaveOutcome {
        let mutation = ProfileMutation::Save {
            id,
            edit_json: edit,
        };
        run_profile_mutation(base, Ok(current), &mutation, save).0
    }

    #[test]
    fn save_pipeline_always_returns_an_outcome() {
        use std::cell::Cell;

        let base = save_base();
        let called = Cell::new(false);

        // Succès.
        let outcome = run_save_for_test(&base, base.clone(), "custom-a", &save_edit(), |_| {
            called.set(true);
            Ok(())
        });
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        assert!(called.get());

        // Refus admin.
        let outcome = run_save_for_test(&base, base.clone(), "custom-a", &save_edit(), |_| {
            Err(SaveError::Admin {
                code: "Busy",
                detail: String::new(),
            })
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Admin { code: "Busy", .. })
        ));

        // DTO invalide : le save distant n'est pas appelé.
        let outcome = run_save_for_test(&base, base.clone(), "custom-a", "{bad", |_| {
            panic!("ne doit pas être appelé");
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::InvalidPayload)
        ));

        // Relecture en échec.
        let mutation = ProfileMutation::Save {
            id: "custom-a",
            edit_json: &save_edit(),
        };
        let (outcome, created) = run_profile_mutation(
            &base,
            Err(ClientError::Timeout {
                call: "GetProfilesJSON",
            }),
            &mutation,
            |_| panic!("ne doit pas être appelé"),
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Timeout)
        ));
        assert!(created.is_none());

        // Aucune branche ne renvoie `None` : `saving` sera toujours remis à faux.
        assert_ne!(format!("{outcome:?}"), "None");
    }

    #[test]
    fn save_pipeline_rejects_concurrent_change() {
        use std::cell::Cell;

        let base = save_base();
        let mut current = base.clone();
        // Un autre client a ajouté un profil depuis notre chargement.
        current.upsert(tcc_core::Profile::new("custom-new", "Nouveau"));

        let called = Cell::new(false);
        let outcome = run_save_for_test(&base, current, "custom-a", &save_edit(), |_| {
            called.set(true);
            Ok(())
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Conflict)
        ));
        assert!(!called.get());
    }

    #[test]
    fn commands_are_received_in_order_without_coalescing() {
        let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        tx.try_send(Command::Refresh).expect("refresh");
        tx.try_send(Command::SaveProfile {
            id: "custom-a".to_owned(),
            edit_json: save_edit(),
        })
        .expect("save");
        tx.try_send(Command::Refresh).expect("refresh 2");

        // Le worker ne fusionne plus : chaque commande est reçue, le save inclus.
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_millis(50)),
            Ok(Command::Refresh)
        ));
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_millis(50)),
            Ok(Command::SaveProfile { .. })
        ));
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_millis(50)),
            Ok(Command::Refresh)
        ));
    }

    #[test]
    fn two_saves_are_queued_in_order() {
        let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        for name in ["a", "b"] {
            tx.try_send(Command::SaveProfile {
                id: format!("custom-{name}"),
                edit_json: String::new(),
            })
            .expect("save");
        }
        for expected in ["custom-a", "custom-b"] {
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(Command::SaveProfile { id, .. }) => assert_eq!(id, expected),
                other => panic!("commande inattendue: {other:?}"),
            }
        }
    }

    #[test]
    fn closed_worker_reports_stopped() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        drop(rx);
        let sender = CommandSender { tx };
        assert_eq!(
            sender.try_send(Command::Refresh).unwrap_err(),
            ClientError::WorkerStopped
        );
    }

    #[test]
    fn save_timeout_covers_admin_worst_case() {
        // Polkit 120 s + reload 30 s + rollback/reload 30 s + marge.
        assert!(ADMIN_SAVE_TIMEOUT >= std::time::Duration::from_secs(210));
    }

    #[test]
    fn transport_details_are_sanitized() {
        assert_eq!(
            sanitize_client_detail("connection refused"),
            "connection refused"
        );
        assert_eq!(sanitize_client_detail("unix:path=/run/user/1000/bus"), "");
        assert_eq!(sanitize_client_detail("tcp:host=127.0.0.1,port=1234"), "");
        assert_eq!(sanitize_client_detail("a\u{7}b"), "");
        assert_eq!(sanitize_client_detail(&"x".repeat(81)), "");

        let error = ClientError::Transport {
            call: "connexion D-Bus",
            detail: "unix:path=/run/user/1000/bus".to_owned(),
        };
        assert!(!error.to_string().contains("/run"));
        assert!(error.to_string().contains("erreur de transport"));
    }

    #[test]
    fn import_pipeline_reports_summary_and_never_overwrites() {
        use tcc_core::Profile;

        let base = save_base();
        let imported = vec![
            // Preset : filtré, jamais importé.
            Profile::new("__office__", "Office importé"),
            // Même ID qu'un profil existant → nouvel ID.
            Profile::new("custom-a", "Studio"),
            // Nom déjà présent (deux fois) → suffixes.
            Profile::new("custom-b", "Studio"),
        ];
        let (outcome, summary) = run_import_mutation(&base, Ok(base.clone()), &imported, |json| {
            let collection: tcc_core::ProfileCollection =
                serde_json::from_str(json).expect("collection");
            // L'original « Studio » n'est pas écrasé.
            let original = collection.get_by_id("custom-a").expect("source");
            assert_eq!(original.name, "Studio");
            // Les deux imports ont un ID distinct et unique.
            let mut ids: Vec<&str> = collection
                .iter()
                .map(|profile| profile.id.as_str())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), collection.len());
            assert!(
                collection
                    .iter()
                    .any(|profile| profile.name == "Studio (importé)")
            );
            assert!(
                collection
                    .iter()
                    .any(|profile| profile.name == "Studio (importé 2)")
            );
            Ok(())
        });
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        let summary = summary.expect("résumé");
        assert_eq!(summary.imported, 2);
        assert_eq!(summary.renamed, 2);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn import_pipeline_returns_an_outcome_on_every_branch() {
        use tcc_core::Profile;

        let base = save_base();
        let imported = vec![Profile::new("custom-new", "Nouveau")];

        // Relecture en échec.
        let (outcome, summary) = run_import_mutation(
            &base,
            Err(ClientError::Timeout {
                call: "GetProfilesJSON",
            }),
            &imported,
            |_| panic!("ne doit pas être appelé"),
        );
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Timeout)
        ));
        assert!(summary.is_none());

        // Collection modifiée par un autre client : conflit, pas d'écriture.
        let mut current = base.clone();
        current.upsert(Profile::new("custom-other", "Autre"));
        let (outcome, summary) = run_import_mutation(&base, Ok(current), &imported, |_| {
            panic!("ne doit pas être appelé")
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Conflict)
        ));
        assert!(summary.is_none());

        // Refus admin : l'erreur remonte, aucun résumé.
        let (outcome, summary) = run_import_mutation(&base, Ok(base.clone()), &imported, |_| {
            Err(SaveError::Admin {
                code: "NotAuthorized",
                detail: String::new(),
            })
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Admin {
                code: "NotAuthorized",
                ..
            })
        ));
        assert!(summary.is_none());
    }

    #[test]
    fn import_pipeline_never_partially_imports() {
        use tcc_core::Profile;

        let base = save_base();
        // Un profil invalide (nom vide) invalide tout l'import.
        let mut invalid = Profile::new("custom-bad", "Invalide");
        invalid.name.clear();
        let imported = vec![Profile::new("custom-ok", "Bon"), invalid];
        let (outcome, summary) = run_import_mutation(&base, Ok(base.clone()), &imported, |_| {
            panic!("ne doit pas être appelé")
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Validation { .. })
        ));
        assert!(summary.is_none());
        // La base d'édition n'est pas modifiée.
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn transfer_counters_saturate() {
        assert_eq!(count_as_i32(0), 0);
        assert_eq!(count_as_i32(42), 42);
        assert_eq!(count_as_i32(usize::MAX), i32::MAX);
    }

    #[test]
    fn create_profile_uses_generated_id_and_defaults() {
        let base = save_base();
        let mutation = ProfileMutation::Create {
            edit_json: &save_edit(),
        };
        let (outcome, created) = run_profile_mutation(&base, Ok(base.clone()), &mutation, |json| {
            let collection: tcc_core::ProfileCollection =
                serde_json::from_str(json).expect("collection");
            assert_eq!(collection.len(), base.len() + 1);
            let created = collection
                .iter()
                .find(|profile| profile.name == "Studio 2")
                .expect("profil créé");
            assert!(!created.id.is_empty());
            assert_ne!(created.id.as_str(), "custom-a");
            tcc_core::validate_profile_id(&created.id).expect("ID valide");
            // Les sections non éditées gardent les valeurs par défaut.
            assert_eq!(created.odm_profile, tcc_core::OdmProfile::default());
            assert_eq!(
                created.odm_power_limits,
                tcc_core::OdmPowerLimits::default()
            );
            Ok(())
        });
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        let id = created.expect("ID créé");
        assert!(tcc_core::validate_profile_id(&tcc_core::ProfileId::new(id)).is_ok());
    }

    #[test]
    fn duplicate_profile_is_deep_and_gets_unique_name() {
        let base = save_base();
        let mutation = ProfileMutation::Duplicate {
            source_id: "custom-a",
        };
        let (outcome, created) = run_profile_mutation(&base, Ok(base.clone()), &mutation, |json| {
            let collection: tcc_core::ProfileCollection =
                serde_json::from_str(json).expect("collection");
            let source = collection.get_by_id("custom-a").expect("source");
            let copy = collection
                .iter()
                .find(|profile| profile.name.contains("(copie)"))
                .expect("copie");
            assert_ne!(copy.id.as_str(), source.id.as_str());
            // Clone profond : toutes les sections sont dupliquées.
            assert_eq!(copy.display, source.display);
            assert_eq!(copy.cpu, source.cpu);
            assert_eq!(copy.fan, source.fan);
            assert_eq!(copy.webcam, source.webcam);
            assert_eq!(copy.fan.custom_fan_curve, source.fan.custom_fan_curve);
            Ok(())
        });
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        assert!(created.is_some());
    }

    #[test]
    fn duplicate_names_are_incremented() {
        use tcc_core::{Profile, ProfileCollection};
        let mut collection = ProfileCollection::new(vec![Profile::new("custom-a", "Studio")]);
        assert_eq!(
            crate::profile_edit::unique_copy_name(&collection, "Studio"),
            "Studio (copie)"
        );
        collection.upsert(Profile::new("custom-b", "Studio (copie)"));
        assert_eq!(
            crate::profile_edit::unique_copy_name(&collection, "Studio"),
            "Studio (copie 2)"
        );
        collection.upsert(Profile::new("custom-c", "Studio (copie 2)"));
        assert_eq!(
            crate::profile_edit::unique_copy_name(&collection, "Studio"),
            "Studio (copie 3)"
        );
        // Un nom très long reste borné à `MAX_PROFILE_NAME_LEN`.
        let long = "x".repeat(tcc_core::MAX_PROFILE_NAME_LEN);
        let name = crate::profile_edit::unique_copy_name(&collection, &long);
        assert!(name.chars().count() <= tcc_core::MAX_PROFILE_NAME_LEN);
    }

    #[test]
    fn delete_profile_removes_only_target() {
        let base = save_base();
        let mutation = ProfileMutation::Delete { id: "custom-a" };
        let (outcome, created) = run_profile_mutation(&base, Ok(base.clone()), &mutation, |json| {
            let collection: tcc_core::ProfileCollection =
                serde_json::from_str(json).expect("collection");
            assert!(collection.get_by_id("custom-a").is_none());
            assert!(collection.get_by_id("__office__").is_some());
            Ok(())
        });
        assert!(matches!(outcome, super::SaveOutcome::Ok));
        assert!(created.is_none());
    }

    #[test]
    fn preset_mutations_are_refused() {
        let base = save_base();
        for mutation in [
            ProfileMutation::Duplicate {
                source_id: "__office__",
            },
            ProfileMutation::Delete { id: "__office__" },
        ] {
            let (outcome, created) =
                run_profile_mutation(&base, Ok(base.clone()), &mutation, |_| {
                    panic!("ne doit pas être appelé")
                });
            assert!(matches!(
                outcome,
                super::SaveOutcome::Err(SaveError::PresetReadOnly)
            ));
            assert!(created.is_none());
        }
    }

    #[test]
    fn collection_size_limit_is_enforced() {
        use tcc_core::{Profile, ProfileCollection};
        let mut base = ProfileCollection::new(Vec::new());
        for index in 0..tcc_core::MAX_PROFILES {
            base.upsert(Profile::new(
                format!("custom-{index}"),
                format!("Profil {index}"),
            ));
        }
        let mutation = ProfileMutation::Create {
            edit_json: &save_edit(),
        };
        let (outcome, created) = run_profile_mutation(&base, Ok(base.clone()), &mutation, |_| {
            panic!("ne doit pas être appelé")
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::TooManyProfiles)
        ));
        assert!(created.is_none());
    }

    #[test]
    fn mutations_are_blocked_by_a_concurrent_change() {
        let base = save_base();
        let mut current = base.clone();
        current.upsert(tcc_core::Profile::new("custom-other", "Autre"));
        for mutation in [
            ProfileMutation::Create {
                edit_json: &save_edit(),
            },
            ProfileMutation::Duplicate {
                source_id: "custom-a",
            },
            ProfileMutation::Delete { id: "custom-a" },
        ] {
            let (outcome, created) =
                run_profile_mutation(&base, Ok(current.clone()), &mutation, |_| {
                    panic!("ne doit pas être appelé")
                });
            assert!(matches!(
                outcome,
                super::SaveOutcome::Err(SaveError::Conflict)
            ));
            assert!(created.is_none());
        }
    }

    #[test]
    fn duplicate_name_must_be_unique_on_create() {
        let base = save_base();
        // « Office » existe déjà dans la collection de test (casse ignorée).
        let duplicate = save_edit().replace("Studio 2", "  OFFICE ");
        let mutation = ProfileMutation::Create {
            edit_json: &duplicate,
        };
        let (outcome, _) = run_profile_mutation(&base, Ok(base.clone()), &mutation, |_| {
            panic!("ne doit pas être appelé")
        });
        assert!(matches!(
            outcome,
            super::SaveOutcome::Err(SaveError::Validation { .. })
        ));
    }

    #[test]
    fn queue_accepts_sensor_commands() {
        let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let sender = CommandSender { tx };
        sender.try_send(Command::SetSensors(true)).unwrap();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .unwrap(),
            Command::SetSensors(true)
        );
    }

    #[test]
    fn queue_is_bounded_and_reports_busy() {
        let (tx, rx) = std::sync::mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let sender = CommandSender { tx };
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            assert!(sender.try_send(Command::Refresh).is_ok());
        }
        // File pleine : refus immédiat, aucun thread ni blocage.
        let error = sender.try_send(Command::Refresh).unwrap_err();
        assert_eq!(error, ClientError::Busy);
        assert_eq!(rx.try_iter().count(), COMMAND_QUEUE_CAPACITY);
    }

    #[test]
    fn closed_queue_reports_worker_stopped() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        drop(rx);
        let sender = CommandSender { tx };
        assert_eq!(
            sender.try_send(Command::Refresh).unwrap_err(),
            ClientError::WorkerStopped
        );
    }

    #[test]
    fn connection_errors_are_classified() {
        assert!(
            ClientError::Timeout {
                call: "GetStateJSON"
            }
            .loses_connection()
        );
        assert!(
            ClientError::Transport {
                call: "connexion D-Bus",
                detail: "absent".to_owned()
            }
            .loses_connection()
        );
        assert!(!ClientError::Busy.loses_connection());
        // Worker disparu : la GUI est effectivement déconnectée.
        assert!(ClientError::WorkerStopped.loses_connection());
        assert!(
            !ClientError::ResponseTooLarge {
                call: "GetProfilesJSON",
                size: MAX_RESPONSE_BYTES + 1,
                limit: MAX_RESPONSE_BYTES
            }
            .loses_connection()
        );
    }

    #[cfg(feature = "dev-overrides")]
    #[test]
    fn bus_name_validation_rejects_unsafe_values() {
        assert!(super::is_valid_bus_name("com.tuxedocomputers.tccd.Dev"));
        assert!(!super::is_valid_bus_name(""));
        assert!(!super::is_valid_bus_name("tccd"));
        assert!(!super::is_valid_bus_name("com.tuxedocomputers tccd"));
        assert!(!super::is_valid_bus_name("1com.tccd"));
        assert!(!super::is_valid_bus_name("com.tccd./../../etc"));
    }
}
