// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Thread de fond : toutes les E/S D-Bus et fichiers (le rendu gpui ne bloque
//! jamais).
//!
//! Chemins utilisés, identiques à ceux de l'interface Qt :
//! - lecture : `tccd` (`GetStateJSON`, `GetProfilesJSON`, `GetSettingsJSON`,
//!   `GetFanDataJSON`, `Manager1.GetHardwareSnapshotJSON`,
//!   `Manager1.GetShutdownTimerJSON`) et l'agent de session
//!   (`User1.GetDisplayInfoJSON`) ;
//! - profil temporaire : `SetTempProfile` / `ClearTempProfile` ;
//! - écritures persistantes : `Admin1.SaveProfilesJSON` /
//!   `Admin1.SaveSettingsJSON` (polkit tranche côté service) ;
//! - écritures matérielles transitoires : `AdminHw1` (ODM, TDP, cTGP) ;
//! - contrôles UVC : `User1.SetUvcControl` ;
//! - blocage de la veille : inhibiteur `idle` de logind, sans privilège.
//!
//! Les setters `Manager1` sont refusés par la politique D-Bus : ils ne sont
//! jamais appelés.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use tcc_client::bus::Target;
use tcc_client::client::Client;
use tcc_client::{ops, stats};
use tcc_core::{Profile, ProfileCollection, ProfileId, Settings};

use crate::i18n::{t, tf};
use crate::model;

/// Intervalle de rafraîchissement.
const POLL: Duration = Duration::from_millis(1500);
/// Les données lentes (profils, réglages, agent, minuterie) sont relues tous
/// les `SLOW_EVERY` cycles.
const SLOW_EVERY: u64 = 4;

/// Dernier état connu du daemon et des services associés.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Le daemon répond.
    pub connected: bool,
    /// Dernière erreur de connexion.
    pub error: Option<String>,
    /// `GetStateJSON`.
    pub state: Value,
    /// `GetProfilesJSON`, typé (presets inclus).
    pub profiles: Vec<Profile>,
    /// `GetSettingsJSON`, typé ; `None` tant qu'il n'a pas été lu.
    pub settings: Option<Settings>,
    /// `GetFanDataJSON`.
    pub fans: Value,
    /// `GetHardwareSnapshotJSON`.
    pub hardware: Value,
    /// `User1.GetDisplayInfoJSON` (`Null` si l'agent est absent).
    pub display: Value,
    /// Version de l'agent de session (vide s'il est absent).
    pub agent_version: String,
    /// L'agent accepte les écritures UVC.
    pub uvc_write: bool,
    /// `GetShutdownTimerJSON` (`Null` si indisponible).
    pub timer: Value,
    /// `tccd-admin` répond.
    pub admin: bool,
    /// `tccd-admin-hw` répond.
    pub admin_hw: bool,
    /// Version du daemon.
    pub daemon_version: String,
    /// Version du contrat IPC (0 = inconnue).
    pub contract: u32,
    /// Charge CPU en %.
    pub cpu_usage: Option<f64>,
    /// Batterie (voir `tcc_client::stats::battery`).
    pub battery: Value,
    /// Un inhibiteur de veille est tenu.
    pub power_blocked: bool,
    /// Profils proposés par power-profiles-daemon (vide s'il est absent).
    pub ppd_profiles: Vec<String>,
    /// Une commande est en cours.
    pub busy: bool,
    /// Incrémenté à chaque mise à jour.
    pub revision: u64,
}

/// Partie des réglages concernée par une écriture, pour que la page
/// resynchronise le bon brouillon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTag {
    /// Interrupteurs et unités.
    General,
    /// Matrice PPD × alimentation.
    Matrix,
    /// Profil, priorité, seuils et type de charge.
    Charging,
    /// Minuterie d'extinction (`true` = annulation).
    Timer(bool),
    /// Éclairage du clavier et de la barre lumineuse.
    Lighting,
    /// Verrou Fn.
    FnLock,
}

/// Opération terminée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Activate,
    ClearTemp,
    SaveProfile,
    /// Identifiant du profil créé.
    CreateProfile(String),
    Duplicate,
    Delete,
    Export,
    Import,
    Settings(SettingsTag),
    /// ODM, TDP ou cTGP immédiat.
    Hardware,
    Uvc,
    PowerBlocker,
    /// Profil power-profiles-daemon.
    Ppd,
    Refresh,
}

/// Résultat d'une commande, à afficher.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub op: Op,
    pub ok: bool,
    pub message: String,
}

/// Modification appliquée aux réglages relus juste avant l'écriture.
pub type SettingsPatch = Box<dyn FnOnce(&mut Settings) + Send>;

/// Actions demandées par l'interface.
pub enum Command {
    /// Relire tout, y compris la disponibilité des services.
    Refresh,
    /// Activer un profil jusqu'au prochain changement d'état.
    UseProfile { id: String, name: String },
    /// Revenir à la bascule automatique.
    ClearTemp,
    /// Enregistrer un profil personnalisé existant.
    SaveProfile(Box<Profile>),
    /// Créer un profil (l'identifiant est régénéré).
    CreateProfile(Box<Profile>),
    /// Dupliquer un profil.
    Duplicate { id: String, name: String },
    /// Supprimer un profil personnalisé.
    Delete { id: String, name: String },
    /// Exporter les profils personnalisés.
    Export(PathBuf),
    /// Importer des profils.
    Import(PathBuf),
    /// Modifier les réglages persistants.
    Settings {
        tag: SettingsTag,
        patch: SettingsPatch,
    },
    /// Appliquer un profil ODM / `platform_profile` (`force` : dérogation PPD).
    Odm { name: String, force: bool },
    /// Limites TDP en watts.
    Tdp(Vec<i32>),
    /// Offset cTGP NVIDIA en watts.
    Ctgp(i32),
    /// Contrôle UVC d'une webcam.
    Uvc {
        device: String,
        key: String,
        value: i32,
    },
    /// Bloquer ou libérer la veille d'inactivité.
    PowerBlocker(bool),
    /// Choisir le profil de power-profiles-daemon (`ActiveProfile`). PPD reste
    /// propriétaire : `tccd` suit ensuite sa matrice d'états.
    Ppd(String),
}

struct Shared {
    snapshot: Snapshot,
    outcomes: VecDeque<Outcome>,
}

/// Poignée vers le thread de fond.
#[derive(Clone)]
pub struct Handle {
    shared: Arc<Mutex<Shared>>,
    sensors: Arc<AtomicBool>,
    tx: Sender<Command>,
}

impl Handle {
    /// Copie du dernier état.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .lock()
            .map(|guard| guard.snapshot.clone())
            .unwrap_or_default()
    }

    /// Numéro de révision courant, sans copier l'état.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.shared
            .lock()
            .map_or(0, |guard| guard.snapshot.revision)
    }

    /// Résultats de commandes pas encore affichés.
    #[must_use]
    pub fn take_outcomes(&self) -> Vec<Outcome> {
        self.shared
            .lock()
            .map(|mut guard| guard.outcomes.drain(..).collect())
            .unwrap_or_default()
    }

    /// Envoie une action (ignorée si le thread est arrêté).
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    /// Demande (ou relâche) la collecte des capteurs : elle n'est active que
    /// lorsque le tableau de bord est affiché.
    pub fn set_sensors_wanted(&self, wanted: bool) {
        self.sensors.store(wanted, Ordering::Relaxed);
    }
}

/// Démarre le thread de fond.
#[must_use]
pub fn spawn(dev_session: bool) -> Handle {
    let shared = Arc::new(Mutex::new(Shared {
        snapshot: Snapshot::default(),
        outcomes: VecDeque::new(),
    }));
    let sensors = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let handle = Handle {
        shared,
        sensors,
        tx,
    };
    let worker = handle.clone();
    std::thread::Builder::new()
        .name("tcc-gpui-worker".to_owned())
        .spawn(move || run(dev_session, &worker, &rx))
        .ok();
    handle
}

struct Worker {
    client: Client,
    /// Dernier échantillon `/proc/stat`.
    cpu_sample: Option<(u64, u64)>,
    /// Descripteur de l'inhibiteur logind ; le fermer libère la veille.
    inhibitor: Option<zbus::zvariant::OwnedFd>,
    tick: u64,
}

fn run(dev_session: bool, handle: &Handle, rx: &Receiver<Command>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut worker = Worker {
        client: Client::new(dev_session),
        cpu_sample: None,
        inhibitor: None,
        tick: 0,
    };
    let mut full = true;
    loop {
        runtime.block_on(worker.refresh(handle, full));
        full = false;
        match rx.recv_timeout(POLL) {
            Ok(command) => {
                set_busy(handle, true);
                let outcome = runtime.block_on(worker.execute(command));
                if let Ok(mut guard) = handle.shared.lock() {
                    guard.snapshot.busy = false;
                    guard.snapshot.power_blocked = worker.inhibitor.is_some();
                    guard.snapshot.revision += 1;
                    guard.outcomes.push_back(outcome);
                }
                full = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn set_busy(handle: &Handle, busy: bool) {
    if let Ok(mut guard) = handle.shared.lock() {
        guard.snapshot.busy = busy;
        guard.snapshot.revision += 1;
    }
}

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

impl Worker {
    async fn refresh(&mut self, handle: &Handle, full: bool) {
        let sensors = handle.sensors.load(Ordering::Relaxed);
        let slow = full || self.tick.is_multiple_of(SLOW_EVERY);
        self.tick += 1;
        let mut next = handle.snapshot();
        let client = &mut self.client;
        match client.json(Target::Daemon, "GetStateJSON").await {
            Ok(raw) => {
                next.connected = true;
                next.error = None;
                next.state = parse(&raw);
                if slow || next.profiles.is_empty() {
                    if let Ok(collection) = client.load_profiles().await {
                        next.profiles = collection.as_slice().to_vec();
                    }
                    if let Ok(settings) = client.load_settings().await {
                        next.settings = Some(settings);
                    }
                    next.timer = client
                        .json(Target::Manager, "GetShutdownTimerJSON")
                        .await
                        .map_or(Value::Null, |raw| parse(&raw));
                }
                if let Ok(raw) = client.json(Target::Daemon, "GetFanDataJSON").await {
                    next.fans = parse(&raw);
                }
                if (sensors || slow || next.hardware.is_null())
                    && let Ok(raw) = client
                        .json(Target::Manager, "GetHardwareSnapshotJSON")
                        .await
                {
                    next.hardware = parse(&raw);
                    if sensors && next.hardware["sensors"]["enabled"] != true {
                        // Bail de courte durée, renouvelé tant que le tableau
                        // de bord est affiché.
                        let _: Result<bool> = client
                            .bus
                            .call(Target::Manager, "SetSensorDataCollectionStatus", &true)
                            .await;
                    }
                }
                if full {
                    next.daemon_version = client
                        .bus
                        .call(Target::Daemon, "GetVersion", &())
                        .await
                        .unwrap_or_default();
                    next.contract = client
                        .bus
                        .call(Target::Manager, "GetContractVersion", &())
                        .await
                        .unwrap_or_default();
                }
            }
            Err(error) => {
                next.connected = false;
                next.error = Some(format!("{error:#}"));
            }
        }
        if full {
            next.admin = ping(client, Target::Admin).await;
            next.admin_hw = ping(client, Target::AdminHw).await;
        }
        if slow {
            next.ppd_profiles = ppd_profiles().await.unwrap_or_default();
            match client.json(Target::Session, "GetDisplayInfoJSON").await {
                Ok(raw) => {
                    next.display = parse(&raw);
                    if full || next.agent_version.is_empty() {
                        next.agent_version = client
                            .bus
                            .call(Target::Session, "GetVersion", &())
                            .await
                            .unwrap_or_default();
                        next.uvc_write = client
                            .bus
                            .call(Target::Session, "GetUvcWriteSupported", &())
                            .await
                            .unwrap_or(false);
                    }
                }
                Err(_) => {
                    next.display = Value::Null;
                    next.agent_version.clear();
                    next.uvc_write = false;
                }
            }
        }
        let sample = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|content| stats::parse_proc_stat(&content));
        next.cpu_usage = match (self.cpu_sample, sample) {
            (Some(before), Some(after)) => stats::usage_percent(before, after),
            _ => None,
        };
        self.cpu_sample = sample;
        next.battery = stats::battery(std::path::Path::new("/sys/class/power_supply"));
        next.power_blocked = self.inhibitor.is_some();
        if let Ok(mut guard) = handle.shared.lock() {
            next.busy = guard.snapshot.busy;
            next.revision = guard.snapshot.revision + 1;
            guard.snapshot = next;
        }
    }

    async fn execute(&mut self, command: Command) -> Outcome {
        let (op, result) = self.dispatch(command).await;
        match result {
            Ok((op, message)) => Outcome {
                op,
                ok: true,
                message,
            },
            Err(error) => Outcome {
                op,
                ok: false,
                message: explain(&error),
            },
        }
    }

    /// Exécute une commande. Le premier `Op` sert en cas d'échec ; celui du
    /// succès peut porter une donnée (identifiant créé).
    async fn dispatch(&mut self, command: Command) -> (Op, Result<(Op, String)>) {
        let client = &mut self.client;
        match command {
            Command::Refresh => (Op::Refresh, Ok((Op::Refresh, String::new()))),
            Command::UseProfile { id, name } => {
                let result = client.use_profile(&id).await.map(|_| {
                    (
                        Op::Activate,
                        tf("toast.profileActivationRequested", &[&name]),
                    )
                });
                (Op::Activate, result)
            }
            Command::ClearTemp => {
                let result = client
                    .clear_temp()
                    .await
                    .map(|()| (Op::ClearTemp, t("toast.tempProfileCleared")));
                (Op::ClearTemp, result)
            }
            Command::SaveProfile(profile) => {
                let result = save_profile(client, *profile)
                    .await
                    .map(|()| (Op::SaveProfile, t("toast.profileSaved")));
                (Op::SaveProfile, result)
            }
            Command::CreateProfile(profile) => {
                let name = profile.name.clone();
                let result = create_profile(client, *profile)
                    .await
                    .map(|id| (Op::CreateProfile(id), tf("profiles.created", &[&name])));
                (Op::CreateProfile(String::new()), result)
            }
            Command::Duplicate { id, name } => {
                let result = duplicate_profile(client, &id)
                    .await
                    .map(|()| (Op::Duplicate, tf("profiles.duplicated", &[&name])));
                (Op::Duplicate, result)
            }
            Command::Delete { id, name } => {
                let result = client
                    .delete_profile(&id)
                    .await
                    .map(|_| (Op::Delete, tf("profiles.deleted", &[&name])));
                (Op::Delete, result)
            }
            Command::Export(path) => {
                let result = export_profiles(client, &path)
                    .await
                    .map(|count| (Op::Export, model::export_message(count)));
                (Op::Export, result)
            }
            Command::Import(path) => {
                let result = import_profiles(client, &path)
                    .await
                    .map(|summary| (Op::Import, model::import_message(summary)));
                (Op::Import, result)
            }
            Command::Settings { tag, patch } => {
                let op = Op::Settings(tag);
                let result = save_settings(client, patch).await.map(|()| {
                    let key = match tag {
                        SettingsTag::Timer(true) => "toast.timerCancelled",
                        SettingsTag::Timer(false) => "toast.timerSet",
                        SettingsTag::Lighting => "tools.lighting.saved",
                        SettingsTag::FnLock => "tray.fnLockSaved",
                        _ => "toast.settingsSaved",
                    };
                    (op.clone(), t(key))
                });
                (op, result)
            }
            Command::Odm { name, force } => {
                let method = if force {
                    "SetOdmProfileOverride"
                } else {
                    "SetOdmProfile"
                };
                (Op::Hardware, hw_call(client, method, &name.as_str()).await)
            }
            Command::Tdp(watts) => (Op::Hardware, hw_call(client, "SetTdpValues", &watts).await),
            Command::Ctgp(watts) => (
                Op::Hardware,
                hw_call(client, "SetNvidiaCtgpOffset", &watts).await,
            ),
            Command::Uvc { device, key, value } => {
                let result = async {
                    let ok: bool = client
                        .bus
                        .call(
                            Target::Session,
                            "SetUvcControl",
                            &(device.as_str(), key.as_str(), value),
                        )
                        .await?;
                    if !ok {
                        bail!(t("tools.webcam.uvcFailed"));
                    }
                    Ok((Op::Uvc, String::new()))
                }
                .await;
                (Op::Uvc, result)
            }
            Command::PowerBlocker(blocked) => {
                let result = if blocked {
                    inhibit().await.map(|fd| {
                        self.inhibitor = Some(fd);
                    })
                } else {
                    self.inhibitor = None;
                    Ok(())
                };
                (
                    Op::PowerBlocker,
                    result.map(|()| (Op::PowerBlocker, String::new())),
                )
            }
            Command::Ppd(name) => {
                let result = set_ppd_profile(&name)
                    .await
                    .map(|()| (Op::Ppd, String::new()));
                (Op::Ppd, result)
            }
        }
    }
}

async fn ping(client: &mut Client, target: Target) -> bool {
    client
        .bus
        .call::<_, bool>(target, "Ping", &())
        .await
        .unwrap_or(false)
}

async fn hw_call<B>(client: &mut Client, method: &str, body: &B) -> Result<(Op, String)>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let ok: bool = client.bus.call(Target::AdminHw, method, body).await?;
    if !ok {
        bail!(t("adminHw.error.unknown"));
    }
    Ok((Op::Hardware, t("toast.hwApplied")))
}

fn other_names(collection: &ProfileCollection, id: &ProfileId) -> impl Iterator<Item = String> {
    collection
        .iter()
        .filter(move |profile| profile.id != *id)
        .map(|profile| model::normalized_name(&profile.name))
}

async fn save_profile(client: &mut Client, mut profile: Profile) -> Result<()> {
    profile.name = profile.name.trim().to_owned();
    profile.validate().context(t("admin.error.validation"))?;
    let mut collection = client.load_profiles().await?;
    if other_names(&collection, &profile.id)
        .any(|name| name == model::normalized_name(&profile.name))
    {
        bail!(t("editor.error.nameDuplicate"));
    }
    let id = profile.id.as_str().to_owned();
    *ops::editable(&mut collection, &id)? = profile;
    client.save_profiles(&collection).await
}

async fn create_profile(client: &mut Client, mut profile: Profile) -> Result<String> {
    profile.name = profile.name.trim().to_owned();
    let mut collection = client.load_profiles().await?;
    profile.id = ProfileId::generate();
    while collection.contains(&profile.id) {
        profile.id = ProfileId::generate();
    }
    profile.validate().context(t("admin.error.validation"))?;
    if other_names(&collection, &profile.id)
        .any(|name| name == model::normalized_name(&profile.name))
    {
        bail!(t("editor.error.nameDuplicate"));
    }
    let id = profile.id.as_str().to_owned();
    collection.upsert(profile);
    client.save_profiles(&collection).await?;
    Ok(id)
}

async fn duplicate_profile(client: &mut Client, id: &str) -> Result<()> {
    let collection = client.load_profiles().await?;
    let source = ops::resolve_profile(&collection, id)?;
    let name = model::unique_copy_name(&collection, &source.name);
    client.copy_profile(id, name).await.map(|_| ())
}

async fn export_profiles(client: &mut Client, path: &std::path::Path) -> Result<usize> {
    let collection = client.load_profiles().await?;
    let (json, count) = model::build_export(&collection).map_err(|e| anyhow::anyhow!(t(e.0)))?;
    std::fs::write(path, json).context(t("admin.error.fileWriteFailed"))?;
    Ok(count)
}

async fn import_profiles(
    client: &mut Client,
    path: &std::path::Path,
) -> Result<model::ImportSummary> {
    let meta = std::fs::metadata(path).context(t("admin.error.readFailed"))?;
    if meta.len() > model::MAX_TRANSFER_BYTES as u64 {
        bail!(t("admin.error.payloadTooLarge"));
    }
    let text = std::fs::read_to_string(path).context(t("admin.error.readFailed"))?;
    let imported = model::parse_import(&text).map_err(|e| anyhow::anyhow!(t(e.0)))?;
    let mut collection = client.load_profiles().await?;
    let summary =
        model::merge_import(&mut collection, imported).map_err(|e| anyhow::anyhow!(t(e.0)))?;
    if summary.imported > 0 {
        client.save_profiles(&collection).await?;
    }
    Ok(summary)
}

async fn save_settings(client: &mut Client, patch: SettingsPatch) -> Result<()> {
    let mut settings = client.load_settings().await?;
    patch(&mut settings);
    client.save_settings(&settings).await
}

/// Inhibiteur `idle` en mode `block` : la veille demandée explicitement par
/// l'utilisateur n'est pas bloquée, seule la veille d'inactivité l'est.
async fn inhibit() -> Result<zbus::zvariant::OwnedFd> {
    let connection = zbus::Connection::system()
        .await
        .context("connexion au bus système")?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .context("proxy logind")?;
    proxy
        .call(
            "Inhibit",
            &(
                "idle",
                "TUXEDO Control Center",
                "Blocage de la veille demandé par l'utilisateur",
                "block",
            ),
        )
        .await
        .context("logind a refusé l'inhibiteur de veille")
}

async fn ppd_proxy() -> Result<zbus::Proxy<'static>> {
    let connection = zbus::Connection::system()
        .await
        .context("connexion au bus système")?;
    zbus::Proxy::new(
        &connection,
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    )
    .await
    .context("proxy power-profiles-daemon")
}

/// Noms des profils exposés par power-profiles-daemon (`Profiles`).
async fn ppd_profiles() -> Result<Vec<String>> {
    type Entry = std::collections::HashMap<String, zbus::zvariant::OwnedValue>;
    let entries: Vec<Entry> = ppd_proxy().await?.get_property("Profiles").await?;
    Ok(entries
        .iter()
        .filter_map(|entry| entry.get("Profile"))
        .filter_map(|value| <&str>::try_from(value).ok().map(str::to_owned))
        .collect())
}

/// Écrit `ActiveProfile` : c'est l'API publique de PPD, soumise à sa propre
/// autorisation polkit. Aucun fichier `platform_profile` n'est touché ici.
async fn set_ppd_profile(name: &str) -> Result<()> {
    if !matches!(name, "power-saver" | "balanced" | "performance") {
        bail!("profil PPD inconnu : {name}");
    }
    ppd_proxy()
        .await?
        .set_property("ActiveProfile", name)
        .await
        .context("power-profiles-daemon a refusé le profil")
}

/// Message lisible pour une erreur : les codes des services
/// d'administration sont traduits, le reste est affiché tel quel.
fn explain(error: &anyhow::Error) -> String {
    let raw = format!("{error:#}");
    let hw = raw.contains("tccd-admin-hw");
    let table: [(&str, &str); 13] = [
        ("NotAuthorized", "admin.error.notAuthorized"),
        ("PolicyKitUnavailable", "admin.error.policyKitUnavailable"),
        ("PolicyDenied", "adminHw.error.policyDenied"),
        ("PartialApply", "adminHw.error.partialApply"),
        ("PayloadTooLarge", "admin.error.payloadTooLarge"),
        ("InvalidArgs", "adminHw.error.invalidArgs"),
        ("Validation", "admin.error.validation"),
        ("InUse", "admin.error.inUse"),
        ("ReloadFailed", "admin.error.reloadFailed"),
        ("WriteFailed", "admin.error.writeFailed"),
        ("RateLimited", "admin.error.rateLimited"),
        ("NotSupported", "admin.error.notSupported"),
        ("Busy", "admin.error.busy"),
    ];
    if let Some((_, key)) = table.iter().find(|(code, _)| raw.contains(code)) {
        return t(key);
    }
    if raw.contains("ServiceUnknown") || raw.contains("NameHasNoOwner") {
        return t(if hw {
            "adminHw.error.unavailable"
        } else {
            "admin.error.unavailable"
        });
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_service_error_codes() {
        let error = anyhow::anyhow!("com.tuxedocomputers.tccd.Admin.Error.NotAuthorized: refusé")
            .context("SaveProfilesJSON : service admin (tccd-admin) indisponible");
        assert_eq!(explain(&error), t("admin.error.notAuthorized"));
        let error = anyhow::anyhow!("org.freedesktop.DBus.Error.ServiceUnknown")
            .context("SetOdmProfile : service matériel (tccd-admin-hw) indisponible");
        assert_eq!(explain(&error), t("adminHw.error.unavailable"));
        let error = anyhow::anyhow!("autre chose");
        assert_eq!(explain(&error), "autre chose");
    }
}
