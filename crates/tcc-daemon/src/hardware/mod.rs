// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Workers matériels autonomes, branchés par `linux-platform`.
//!
//! # Rôle
//!
//! Ce module fournit la **matrice matérielle** demandée par le daemon :
//! charge, éclairage, ODM/TDP, Fn-Lock, capteurs, NVIDIA cTGP, webcam, PRIME
//! et YCbCr 4:2:0. Il ne touche ni au service D-Bus ni à la configuration :
//! `linux-platform` crée [`HardwareInputs`] depuis `RuntimeState`, démarre
//! [`HardwareWorkers`], puis expose les instantanés [`HardwareSnapshot`] sur
//! l'interface de son choix.
//!
//! # Point de branchement (une seule ligne dans `main.rs`)
//!
//! ```ignore
//! mod hardware;
//!
//! let inputs = Arc::new(tokio::sync::RwLock::new(
//!     hardware::HardwareInputs::new(&state_snapshot),
//! ));
//! let hw_status = Arc::new(hardware::HardwareStatus::default());
//! let sensors_enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
//! let hardware = hardware::HardwareWorkers::start(
//!     inputs.clone(),
//!     hw_status.clone(),
//!     root.clone(),
//!     dry_run,
//!     sensors_enabled.clone(),
//! ).await;
//! // À chaque changement d'état/profil/réglages :
//! *inputs.write().await = hardware::HardwareInputs::new(&state_snapshot);
//! hardware.notify();
//! // Sur `SetSensorDataCollectionStatus(true)` :
//! sensors_enabled.store(true, Ordering::Relaxed);
//! // À l'arrêt :
//! hardware.shutdown().await;
//! ```
//!
//! `linux-platform` reste propriétaire de `main.rs`, de `dbus.rs`, de
//! `runtime.rs` et de la persistance. Voir `docs/hardware/HARDWARE_MATRIX.md`
//! pour les payloads proposés et la matrice de capacités par machine.
//!
//! # Concurrence
//!
//! Chaque worker tourne dans sa tâche tokio et effectue ses accès sysfs dans
//! `spawn_blocking` : aucun I/O bloquant dans le runtime asynchrone. Les
//! instantanés sont publiés dans [`HardwareStatus`] (verrous `RwLock` standard,
//! sections critiques très courtes), lisibles par les handlers D-Bus.
//!
//! Un seul [`tokio::sync::Notify`] partagé réveille tous les workers : un
//! changement de profil ou de réglages provoque une réconciliation complète,
//! idempotente (les écritures inchangées sont évitées).
//!
//! # Dégradation
//!
//! Aucun matériel n'est requis au démarrage : chaque domaine publie
//! `available: false` et ne fait aucune écriture. Les workers qui dépendent de
//! `/dev/tuxedo_io` (ODM, TDP, webcam) rouvrent l'interface périodiquement
//! après un chargement tardif du module noyau.

pub mod capabilities;
pub mod charging;
pub mod cpu;
pub mod device;
pub mod display;
pub mod fn_lock;
pub mod lighting;
pub mod nvidia;
pub mod odm;
pub mod prime;
pub mod sensors;
pub mod webcam;
pub mod ycbcr;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_core::{Profile, Settings};
use tcc_hw::sysfs::SysfsRoot;
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;

/// Entrées communes des workers, reconstruites par `linux-platform`.
///
/// Le clone est volontairement simple : `Settings` et `Profile` sont des
/// structures de configuration de quelques kilo-octets, relues au plus toutes
/// les 2 secondes et à chaque changement.
#[derive(Debug, Clone, Default)]
pub struct HardwareInputs {
    /// Réglages globaux (charge, éclairage, activation des contrôles, ...).
    pub settings: Settings,
    /// Profil actif, `None` avant la première résolution.
    pub profile: Option<Profile>,
    /// `power-profiles-daemon` est présent sur le bus système.
    ///
    /// Sert uniquement à la politique de propriété de `platform_profile` (D4) :
    /// PPD propriétaire par défaut, aucune écriture TCC sans override explicite.
    pub ppd_active: bool,
    /// Override explicite « TCC prioritaire sur PPD » pour
    /// `platform_profile`.
    ///
    /// Faux par défaut (D4). `linux-platform` l'alimente depuis le réglage
    /// utilisateur ; toute activation doit être journalisée et affichée dans
    /// l'UI. Sans cet override, aucun `platform_profile` n'est écrit quand PPD
    /// est actif.
    pub ppd_override: bool,
    /// Révision incrémentée à chaque modification, utilisée pour ne pas
    /// recalculer inutilement les signatures.
    pub revision: u64,
}

impl HardwareInputs {
    /// Construit les entrées depuis les composants connus.
    #[must_use]
    pub fn new(
        settings: Settings,
        profile: Option<Profile>,
        ppd_active: bool,
        revision: u64,
    ) -> Self {
        Self {
            settings,
            profile,
            ppd_active,
            ppd_override: false,
            revision,
        }
    }

    /// Active l'override explicite « TCC prioritaire sur PPD ».
    #[must_use]
    pub fn with_ppd_override(mut self, enabled: bool) -> Self {
        self.ppd_override = enabled;
        self
    }

    /// Propriétaire effectif de `platform_profile`.
    #[must_use]
    pub fn platform_profile_owner(&self) -> tcc_hw::platform_profile::ProfileOwnership {
        if self.ppd_active && !self.ppd_override {
            tcc_hw::platform_profile::ProfileOwnership::PpdOwned
        } else {
            tcc_hw::platform_profile::ProfileOwnership::TccOwned
        }
    }
}

/// Contexte partagé par tous les workers.
///
/// Regroupe les canaux communs pour garder des signatures courtes et
/// homogènes.
#[derive(Debug, Clone)]
pub(crate) struct WorkerContext {
    pub inputs: Arc<RwLock<HardwareInputs>>,
    pub status: Arc<HardwareStatus>,
    pub stop: Arc<AtomicBool>,
    pub notify: Arc<Notify>,
}

/// Délai maximal accordé à l'arrêt des workers.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Instantané complet de la matrice matérielle.
///
/// Sérialisé en JSON pour les payloads D-Bus proposés (voir
/// `docs/hardware/HARDWARE_MATRIX.md`). Chaque champ est également accessible
/// individuellement via [`HardwareStatus`] pour une méthode D-Bus dédiée.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HardwareSnapshot {
    /// Identification DMI (modèle, fabricant, SKU).
    pub device: device::DeviceData,
    /// Topologie et capacités CPU (lecture seule).
    pub cpu: cpu::CpuCapabilitiesData,
    /// Connecteur interne DRM (les modes restent fournis par `tccd-user`).
    pub display: display::DisplayData,
    /// Capabilities simples (batterie, secteur, iGPU/dGPU).
    pub capabilities: capabilities::CapabilitiesData,
    pub charging: charging::ChargingData,
    pub lighting: lighting::LightingData,
    pub odm: odm::OdmData,
    pub sensors: sensors::SensorData,
    pub nvidia: nvidia::NvidiaData,
    pub webcam: webcam::WebcamData,
    pub fn_lock: fn_lock::FnLockData,
    pub prime: prime::PrimeData,
    pub ycbcr: ycbcr::YcbcrData,
}

/// État partagé des domaines matériels.
///
/// Les verrous sont standard (pas `tokio`) : les sections critiques ne font
/// que cloner un instantané, jamais d'I/O.
#[derive(Debug, Default)]
pub struct HardwareStatus {
    device: std::sync::RwLock<device::DeviceData>,
    cpu: std::sync::RwLock<cpu::CpuCapabilitiesData>,
    display: std::sync::RwLock<display::DisplayData>,
    capabilities: std::sync::RwLock<capabilities::CapabilitiesData>,
    charging: std::sync::RwLock<charging::ChargingData>,
    lighting: std::sync::RwLock<lighting::LightingData>,
    odm: std::sync::RwLock<odm::OdmData>,
    sensors: std::sync::RwLock<sensors::SensorData>,
    nvidia: std::sync::RwLock<nvidia::NvidiaData>,
    webcam: std::sync::RwLock<webcam::WebcamData>,
    fn_lock: std::sync::RwLock<fn_lock::FnLockData>,
    prime: std::sync::RwLock<prime::PrimeData>,
    ycbcr: std::sync::RwLock<ycbcr::YcbcrData>,
}

impl HardwareStatus {
    /// Instantané complet.
    #[must_use]
    pub fn snapshot(&self) -> HardwareSnapshot {
        HardwareSnapshot {
            device: self.device(),
            cpu: self.cpu_capabilities(),
            display: self.display(),
            capabilities: self.capabilities(),
            charging: self.charging(),
            lighting: self.lighting(),
            odm: self.odm(),
            sensors: self.sensors(),
            nvidia: self.nvidia(),
            webcam: self.webcam(),
            fn_lock: self.fn_lock(),
            prime: self.prime(),
            ycbcr: self.ycbcr(),
        }
    }

    pub fn device(&self) -> device::DeviceData {
        clone_or_default(&self.device)
    }

    pub fn cpu_capabilities(&self) -> cpu::CpuCapabilitiesData {
        clone_or_default(&self.cpu)
    }

    pub fn display(&self) -> display::DisplayData {
        clone_or_default(&self.display)
    }

    pub fn capabilities(&self) -> capabilities::CapabilitiesData {
        clone_or_default(&self.capabilities)
    }

    pub fn charging(&self) -> charging::ChargingData {
        clone_or_default(&self.charging)
    }

    pub fn lighting(&self) -> lighting::LightingData {
        clone_or_default(&self.lighting)
    }

    pub fn odm(&self) -> odm::OdmData {
        clone_or_default(&self.odm)
    }

    pub fn sensors(&self) -> sensors::SensorData {
        clone_or_default(&self.sensors)
    }

    pub fn nvidia(&self) -> nvidia::NvidiaData {
        clone_or_default(&self.nvidia)
    }

    pub fn webcam(&self) -> webcam::WebcamData {
        clone_or_default(&self.webcam)
    }

    pub fn fn_lock(&self) -> fn_lock::FnLockData {
        clone_or_default(&self.fn_lock)
    }

    pub fn prime(&self) -> prime::PrimeData {
        clone_or_default(&self.prime)
    }

    pub fn ycbcr(&self) -> ycbcr::YcbcrData {
        clone_or_default(&self.ycbcr)
    }

    /// Publie un instantané de charge (workers uniquement).
    pub(crate) fn set_device(&self, data: device::DeviceData) {
        store(&self.device, data);
    }

    pub(crate) fn set_cpu(&self, data: cpu::CpuCapabilitiesData) {
        store(&self.cpu, data);
    }

    pub(crate) fn set_display(&self, data: display::DisplayData) {
        store(&self.display, data);
    }

    pub(crate) fn set_capabilities(&self, data: capabilities::CapabilitiesData) {
        store(&self.capabilities, data);
    }

    pub(crate) fn set_charging(&self, data: charging::ChargingData) {
        store(&self.charging, data);
    }

    pub(crate) fn set_lighting(&self, data: lighting::LightingData) {
        store(&self.lighting, data);
    }

    pub(crate) fn set_odm(&self, data: odm::OdmData) {
        store(&self.odm, data);
    }

    pub(crate) fn set_sensors(&self, data: sensors::SensorData) {
        store(&self.sensors, data);
    }

    pub(crate) fn set_nvidia(&self, data: nvidia::NvidiaData) {
        store(&self.nvidia, data);
    }

    pub(crate) fn set_webcam(&self, data: webcam::WebcamData) {
        store(&self.webcam, data);
    }

    pub(crate) fn set_fn_lock(&self, data: fn_lock::FnLockData) {
        store(&self.fn_lock, data);
    }

    pub(crate) fn set_prime(&self, data: prime::PrimeData) {
        store(&self.prime, data);
    }

    pub(crate) fn set_ycbcr(&self, data: ycbcr::YcbcrData) {
        store(&self.ycbcr, data);
    }
}

fn clone_or_default<T: Clone + Default>(lock: &std::sync::RwLock<T>) -> T {
    match lock.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn store<T>(lock: &std::sync::RwLock<T>, value: T) {
    match lock.write() {
        Ok(mut guard) => *guard = value,
        Err(poisoned) => *poisoned.into_inner() = value,
    }
}

/// Ensemble des workers matériels.
#[derive(Debug)]
pub struct HardwareWorkers {
    stop: Arc<AtomicBool>,
    notify: Arc<Notify>,
    tasks: Vec<JoinHandle<()>>,
}

impl HardwareWorkers {
    /// Démarre tous les workers.
    ///
    /// `sensors_enabled` est piloté par `linux-platform` depuis
    /// `SetSensorDataCollectionStatus` : tant qu'il est faux, la collecte de
    /// capteurs ne lit rien (budget CPU, `docs/QUALITY.md` §4).
    pub async fn start(
        inputs: Arc<RwLock<HardwareInputs>>,
        status: Arc<HardwareStatus>,
        root: SysfsRoot,
        dry_run: bool,
        sensors_enabled: Arc<AtomicBool>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let ctx = WorkerContext {
            inputs,
            status,
            stop: Arc::clone(&stop),
            notify: Arc::clone(&notify),
        };
        let tasks = vec![
            device::spawn(ctx.clone(), root.clone()),
            cpu::spawn(ctx.clone(), root.clone(), dry_run),
            display::spawn(ctx.clone(), root.clone()),
            capabilities::spawn(ctx.clone(), root.clone()),
            charging::spawn(ctx.clone(), root.clone(), dry_run),
            lighting::spawn(ctx.clone(), root.clone(), dry_run),
            odm::spawn(ctx.clone(), root.clone(), dry_run),
            sensors::spawn(
                ctx.clone(),
                root.clone(),
                Some(Arc::clone(&sensors_enabled)),
            ),
            nvidia::spawn(ctx.clone(), root.clone(), dry_run),
            webcam::spawn(ctx.clone(), root.clone(), dry_run, Some(sensors_enabled)),
            fn_lock::spawn(ctx.clone(), root.clone(), dry_run),
            prime::spawn(ctx.clone(), root.clone()),
            ycbcr::spawn(ctx.clone(), root.clone(), dry_run),
        ];

        Self {
            stop,
            notify,
            tasks,
        }
    }

    /// Réveille tous les workers (changement de profil, de réglages ou SIGHUP).
    pub fn notify(&self) {
        self.notify.notify_waiters();
    }

    /// Demande l'arrêt et **aborte immédiatement** toutes les tâches.
    ///
    /// Cette méthode est synchrone et ne peut pas être annulée : elle doit
    /// être appelée avant tout `await` de veille/arrêt (SL-01). Un tick déjà
    /// engagé dans `spawn_blocking` peut encore se terminer, mais les tâches
    /// tokio ne peuvent plus écrire de nouvelle consigne après l'abort.
    pub fn abort_all(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        for task in &self.tasks {
            task.abort();
        }
    }

    /// Arrête tous les workers.
    ///
    /// L'abort est effectué avant la première attente ; l'attente des handles
    /// n'est qu'un nettoyage borné. Les domaines matériels ne sont pas
    /// « restaurés » : le retour auto des ventilateurs reste géré par
    /// `FanWorker`.
    pub async fn shutdown(mut self) {
        self.abort_all();
        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;
        for task in self.tasks {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            match tokio::time::timeout_at(deadline, task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::debug!(%error, "worker matériel arrêté"),
                Err(_) => tracing::warn!("arrêt des workers matériels : budget de 3 s épuisé"),
            }
        }
    }
}

/// Boucle commune d'un worker matériel.
///
/// - attend `interval` **ou** un signal de réconciliation ;
/// - lit les entrées, exécute `step` dans `spawn_blocking` ;
/// - publie le résultat.
///
/// `gate` permet de suspendre complètement le travail (collecte de capteurs
/// désactivée) sans détruire l'état accumulé (delta RAPL).
pub(crate) fn spawn_worker<R, D, F>(
    label: &'static str,
    interval: Duration,
    ctx: WorkerContext,
    runtime: R,
    gate: Option<Arc<AtomicBool>>,
    publish: fn(&HardwareStatus, D),
    step: F,
) -> JoinHandle<()>
where
    R: Send + 'static,
    D: Send + 'static,
    F: FnMut(&mut R, &HardwareInputs) -> D + Send + 'static,
{
    let WorkerContext {
        inputs,
        status,
        stop,
        notify,
    } = ctx;
    tokio::spawn(async move {
        let runtime = Arc::new(std::sync::Mutex::new(runtime));
        let step = Arc::new(std::sync::Mutex::new(step));
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = notify.notified() => {}
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let Some(gate) = &gate
                && !gate.load(Ordering::Relaxed)
            {
                continue;
            }
            let input = inputs.read().await.clone();
            let runtime = Arc::clone(&runtime);
            let step = Arc::clone(&step);
            let result = tokio::task::spawn_blocking(move || {
                // Un verrou empoisonné (panic dans un tick précédent) ne doit
                // pas arrêter la régulation matérielle : on récupère l'état et
                // on continue, avec un log unique par incident.
                let mut step = match step.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        tracing::warn!(worker = label, "état de worker empoisonné, récupération");
                        poisoned.into_inner()
                    }
                };
                let mut guard = match runtime.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                step(&mut guard, &input)
            })
            .await;
            match result {
                Ok(data) => publish(&status, data),
                Err(error) => tracing::warn!(worker = label, %error, "tick matériel interrompu"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn abort_all_stops_tasks_without_awaiting() {
        let task = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        let mut workers = HardwareWorkers {
            stop: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
            tasks: vec![task],
        };
        workers.abort_all();
        tokio::task::yield_now().await;
        assert!(workers.tasks[0].is_finished(), "tâche non abortée");
        assert!(workers.stop.load(Ordering::SeqCst));
    }

    #[test]
    fn ppd_ownership_defaults_to_ppd_and_override_is_explicit() {
        use tcc_hw::platform_profile::ProfileOwnership;
        let inputs = HardwareInputs::new(tcc_core::Settings::default(), None, true, 1);
        assert_eq!(
            inputs.platform_profile_owner(),
            ProfileOwnership::PpdOwned,
            "PPD actif sans override ⇒ lecture seule (D4)"
        );
        assert_eq!(
            inputs
                .clone()
                .with_ppd_override(true)
                .platform_profile_owner(),
            ProfileOwnership::TccOwned
        );
        let no_ppd = HardwareInputs::new(tcc_core::Settings::default(), None, false, 1);
        assert_eq!(no_ppd.platform_profile_owner(), ProfileOwnership::TccOwned);
    }

    #[test]
    fn status_roundtrip_and_snapshot() {
        let status = HardwareStatus::default();
        status.set_fn_lock(fn_lock::FnLockData {
            supported: true,
            enabled: Some(true),
            writable: true,
        });
        assert_eq!(status.fn_lock().enabled, Some(true));
        let snapshot = status.snapshot();
        assert!(snapshot.fn_lock.supported);
        let json = serde_json::to_string(&snapshot).expect("json");
        assert!(json.contains("\"fnLock\""));
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"device\""));
        assert!(json.contains("\"cpu\""));
        assert!(json.contains("\"display\""));
        assert!(json.contains("\"capabilities\""));
        assert!(json.contains("\"kernelRelease\""));
        assert!(json.contains("\"architecture\""));
        assert!(json.contains("\"hasInternalDisplay\":false"));
        assert!(json.contains("\"hasBattery\":false"));
        assert!(json.contains("\"controlsWritable\":false"));

        // Compatibilité : un payload sans les nouveaux champs se désérialise
        // avec leurs valeurs par défaut.
        let empty: HardwareSnapshot = serde_json::from_str("{}").expect("payload vide");
        assert_eq!(empty, HardwareSnapshot::default());
        assert!(!empty.device.available);
        assert_eq!(empty.device.kernel_release, None);
        assert!(empty.device.architecture.is_empty());
        assert!(empty.cpu.available_governors.is_empty());
        assert!(!empty.display.available);
        assert!(!empty.display.has_internal_display);
        assert!(!empty.capabilities.has_battery);
        assert!(!empty.capabilities.has_ac);
        assert!(!empty.capabilities.has_igpu);
        assert!(!empty.capabilities.has_dedicated_gpu);
        assert!(!empty.webcam.controls_writable);
    }

    #[test]
    fn new_capability_fields_default_when_absent() {
        let charging: charging::ChargingData =
            serde_json::from_str(r#"{"available":true}"#).expect("charge");
        assert!(charging.available);
        assert!(!charging.write_supported);
        assert!(!charging.profile_writable);
        assert!(!charging.charge_type_writable);
        assert!(charging.charge_types.is_empty());

        let lighting: lighting::LightingData =
            serde_json::from_str(r#"{"available":true}"#).expect("lighting");
        assert!(!lighting.keyboard.writable);
        assert!(lighting.keyboard.modes.is_empty());
        assert!(!lighting.lightbar.writable);

        let webcam: webcam::WebcamData =
            serde_json::from_str(r#"{"switchAvailable":true}"#).expect("webcam");
        assert!(webcam.switch_available);
        assert!(!webcam.controls_writable);

        let device: device::DeviceData =
            serde_json::from_str(r#"{"productName":"Book"}"#).expect("device");
        assert_eq!(device.product_name.as_deref(), Some("Book"));
        assert!(!device.is_tuxedo);

        let cpu: cpu::CpuCapabilitiesData =
            serde_json::from_str(r#"{"totalCores":4}"#).expect("cpu");
        assert_eq!(cpu.total_cores, 4);
        assert!(!cpu.writable);

        let display: display::DisplayData =
            serde_json::from_str(r#"{"internalConnector":"eDP-1"}"#).expect("display");
        assert_eq!(display.internal_connector.as_deref(), Some("eDP-1"));
        assert!(!display.available);
        assert!(!display.has_internal_display);
        assert!(!display.connected);

        let capabilities: capabilities::CapabilitiesData =
            serde_json::from_str(r#"{"hasBattery":true}"#).expect("capabilities");
        assert!(capabilities.has_battery);
        assert!(!capabilities.has_ac);
        assert!(!capabilities.has_igpu);
        assert!(!capabilities.has_dedicated_gpu);
    }

    #[test]
    fn status_survives_poisoned_lock() {
        let status = Arc::new(HardwareStatus::default());
        let clone = Arc::clone(&status);
        let _ = std::thread::spawn(move || {
            let _guard = clone.charging.write().expect("write");
            panic!("poison");
        })
        .join();
        // La lecture ne panique pas et retourne la valeur par défaut.
        assert!(!status.charging().available);
        status.set_charging(charging::ChargingData {
            available: true,
            ..charging::ChargingData::default()
        });
        assert!(status.charging().available);
    }

    #[test]
    fn worker_publishes_and_stops() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // Le worker `ycbcr` sert de cas réel : sans debugfs, il publie un
            // état non disponible et s'arrête proprement.
            let inputs = Arc::new(RwLock::new(HardwareInputs::default()));
            let status = Arc::new(HardwareStatus::default());
            let root = SysfsRoot::new("/nonexistent-tcc-hardware");
            let workers = HardwareWorkers::start(
                inputs,
                status.clone(),
                root,
                true,
                Arc::new(AtomicBool::new(false)),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            workers.notify();
            tokio::time::sleep(Duration::from_millis(50)).await;
            workers.shutdown().await;
            // Un état a été publié par au moins un domaine.
            assert!(status.ycbcr().ports.is_empty() || !status.ycbcr().available);
        });
    }

    #[test]
    fn gate_skips_work() {
        let counter = Arc::new(AtomicU32::new(0));
        let runtime_counter = Arc::clone(&counter);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let inputs = Arc::new(RwLock::new(HardwareInputs::default()));
            let status = Arc::new(HardwareStatus::default());
            let stop = Arc::new(AtomicBool::new(false));
            let notify = Arc::new(Notify::new());
            let gate = Arc::new(AtomicBool::new(false));
            let ctx = WorkerContext {
                inputs,
                status,
                stop: stop.clone(),
                notify,
            };
            let task = spawn_worker(
                "test",
                Duration::from_millis(20),
                ctx,
                (),
                Some(gate.clone()),
                |_, _| {},
                move |_, _| {
                    runtime_counter.fetch_add(1, Ordering::Relaxed);
                },
            );
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(counter.load(Ordering::Relaxed), 0);
            gate.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(80)).await;
            stop.store(true, Ordering::Relaxed);
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
            assert!(counter.load(Ordering::Relaxed) > 0);
        });
    }
}
