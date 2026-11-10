// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Thread de fond : toutes les E/S D-Bus (le rendu gpui ne bloque jamais).

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tcc_client::bus::Target;
use tcc_client::client::Client;
use tcc_client::stats;

/// Intervalle de rafraîchissement.
const POLL: Duration = Duration::from_millis(1500);

/// Dernier état connu du daemon.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Le daemon répond.
    pub connected: bool,
    /// Dernière erreur de connexion.
    pub error: Option<String>,
    /// `GetStateJSON`.
    pub state: Value,
    /// `GetProfilesJSON` (tableau).
    pub profiles: Vec<Value>,
    /// `GetSettingsJSON`.
    pub settings: Value,
    /// `GetFanDataJSON`.
    pub fans: Value,
    /// `GetHardwareSnapshotJSON`.
    pub hardware: Value,
    /// Charge CPU en %.
    pub cpu_usage: Option<f64>,
    /// Batterie (voir `tcc_client::stats::battery`).
    pub battery: Value,
    /// Dernier résultat d'action : (message, succès).
    pub notice: Option<(String, bool)>,
    /// Incrémenté à chaque mise à jour.
    pub revision: u64,
}

/// Actions demandées par l'interface.
#[derive(Debug)]
pub enum Command {
    /// Activer un profil temporairement.
    UseProfile(String),
    /// Supprimer l'override.
    ClearTemp,
    /// Créer un profil.
    CreateProfile(String),
    /// Copier un profil.
    CopyProfile {
        /// Source (id).
        from: String,
        /// Nom de la copie.
        name: String,
    },
    /// Supprimer un profil.
    DeleteProfile(String),
    /// Modifier un champ de profil.
    SetProfileField {
        /// Id.
        profile: String,
        /// Chemin pointé.
        key: String,
        /// Valeur JSON ou texte.
        raw: String,
    },
    /// Modifier un réglage global.
    SetSetting {
        /// Chemin pointé.
        key: String,
        /// Valeur JSON ou texte.
        raw: String,
    },
    /// Appel `Manager1` à un argument chaîne (charge).
    ManagerString {
        /// Méthode.
        method: &'static str,
        /// Argument.
        value: String,
    },
    /// Appel `Manager1` à un entier (seuils de charge).
    ManagerInt {
        /// Méthode.
        method: &'static str,
        /// Argument.
        value: i32,
    },
    /// Verrou Fn.
    FnLock(bool),
    /// Limites TDP (vide = maxima firmware).
    SetTdp(Vec<i32>),
}

/// Poignée vers le thread de fond.
#[derive(Clone)]
pub struct Handle {
    shared: Arc<Mutex<Snapshot>>,
    tx: Sender<Command>,
}

impl Handle {
    /// Copie du dernier état.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Envoie une action (ignorée si le thread est arrêté).
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

/// Démarre le thread de fond.
#[must_use]
pub fn spawn(dev_session: bool) -> Handle {
    let shared = Arc::new(Mutex::new(Snapshot::default()));
    let (tx, rx) = channel();
    let worker_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("tcc-gpui-worker".to_owned())
        .spawn(move || run(dev_session, &worker_shared, &rx))
        .ok();
    Handle { shared, tx }
}

fn run(dev_session: bool, shared: &Arc<Mutex<Snapshot>>, rx: &Receiver<Command>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut client = Client::new(dev_session);
    let mut full = true;
    loop {
        runtime.block_on(refresh(&mut client, shared, full));
        full = false;
        match rx.recv_timeout(POLL) {
            Ok(command) => {
                let notice = runtime.block_on(execute(&mut client, command));
                if let Ok(mut guard) = shared.lock() {
                    guard.notice = Some(notice);
                }
                full = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

async fn refresh(client: &mut Client, shared: &Arc<Mutex<Snapshot>>, full: bool) {
    let state = client.json(Target::Daemon, "GetStateJSON").await;
    let mut next = shared.lock().map(|g| g.clone()).unwrap_or_default();
    match state {
        Ok(raw) => {
            next.connected = true;
            next.error = None;
            next.state = parse(&raw);
            if full || next.profiles.is_empty() {
                if let Ok(raw) = client.json(Target::Daemon, "GetProfilesJSON").await {
                    next.profiles = parse(&raw).as_array().cloned().unwrap_or_default();
                }
                if let Ok(raw) = client.json(Target::Daemon, "GetSettingsJSON").await {
                    next.settings = parse(&raw);
                }
            }
            if let Ok(raw) = client.json(Target::Daemon, "GetFanDataJSON").await {
                next.fans = parse(&raw);
            }
            if let Ok(raw) = client
                .json(Target::Manager, "GetHardwareSnapshotJSON")
                .await
            {
                next.hardware = parse(&raw);
                if next.hardware["sensors"]["enabled"].as_bool() != Some(true) {
                    // Bail de 10 s, renouvelé par chaque lecture.
                    let _: Result<bool, _> = client
                        .bus
                        .call(Target::Manager, "SetSensorDataCollectionStatus", &true)
                        .await;
                }
            }
        }
        Err(error) => {
            next.connected = false;
            next.error = Some(format!("{error:#}"));
        }
    }
    let usage = tokio::task::spawn_blocking(|| stats::cpu_usage(Duration::from_millis(150)))
        .await
        .ok()
        .flatten();
    next.cpu_usage = usage;
    next.battery = stats::battery(std::path::Path::new("/sys/class/power_supply"));
    next.revision += 1;
    if let Ok(mut guard) = shared.lock() {
        // Ne pas écraser un message posté pendant le rafraîchissement.
        next.notice = guard.notice.take().or(next.notice);
        *guard = next;
    }
}

async fn execute(client: &mut Client, command: Command) -> (String, bool) {
    let result: anyhow::Result<String> = async {
        Ok(match command {
            Command::UseProfile(id) => format!("Profil actif : {}", client.use_profile(&id).await?),
            Command::ClearTemp => {
                client.clear_temp().await?;
                "Retour à la bascule automatique".to_owned()
            }
            Command::CreateProfile(name) => {
                client.create_profile(name).await?;
                "Profil créé".to_owned()
            }
            Command::CopyProfile { from, name } => {
                client.copy_profile(&from, name).await?;
                "Profil copié".to_owned()
            }
            Command::DeleteProfile(id) => {
                client.delete_profile(&id).await?;
                "Profil supprimé".to_owned()
            }
            Command::SetProfileField { profile, key, raw } => {
                client.set_profile_field(&profile, &key, &raw).await?;
                format!("{key} enregistré")
            }
            Command::SetSetting { key, raw } => {
                client.set_setting(&key, &raw).await?;
                format!("{key} enregistré")
            }
            Command::ManagerString { method, value } => {
                let _: bool = client
                    .bus
                    .call(Target::Manager, method, &value.as_str())
                    .await?;
                format!("{method} appliqué")
            }
            Command::ManagerInt { method, value } => {
                let _: bool = client.bus.call(Target::Manager, method, &value).await?;
                format!("{method} appliqué")
            }
            Command::FnLock(enabled) => {
                let _: bool = client
                    .bus
                    .call(Target::Manager, "SetFnLockStatus", &enabled)
                    .await?;
                "Fn Lock appliqué".to_owned()
            }
            Command::SetTdp(watts) => {
                let _: bool = client
                    .bus
                    .call(Target::AdminHw, "SetTdpValues", &watts)
                    .await?;
                "Limites TDP appliquées".to_owned()
            }
        })
    }
    .await;
    match result {
        Ok(message) => (message, true),
        Err(error) => (format!("{error:#}"), false),
    }
}
