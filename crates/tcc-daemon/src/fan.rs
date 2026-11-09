// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker de régulation des ventilateurs.
//!
//! Un tick par seconde, indépendant de la boucle de bascule :
//!
//! 1. lecture du profil actif et de `settings.fan_control_enabled` ;
//! 2. si le contrôle est désactivé (ou `useControl == false`), retour en mode
//!    automatique du firmware ;
//! 3. sinon lecture des températures, évaluation des courbes CPU/GPU avec
//!    [`FanCurveEvaluator`] (min/max/offset, hystérésis de 2 %), application
//!    des vitesses avec les garde-fous matériels ;
//! 4. publication d'un instantané pour `GetFanDataJSON`.
//!
//! La logique de courbe reste dans `tcc-core`. Ce module ne contient que
//! l'orchestration et trois garde-fous repris de l'original :
//! [`apply_hw_limits`] (`FanControlLogic.applyHwFanLimitations`),
//! [`limit_falling_speed`] (`FanControlLogic.limitFallingFanSpeed`) et
//! [`critical_temperature_floor`] (`FanUtils.manageCriticalTemperature`).
//!
//! # Choix documentés
//!
//! - Le filtre médian sur 13 échantillons de l'original n'est pas reproduit :
//!   l'hystérésis de 2 % joue le même rôle anti-oscillation avec un temps de
//!   réaction d'une seconde au lieu de treize. Voir
//!   `docs/linux/PLATFORM_DESIGN.md` §2.2.
//! - Les écritures sont dédupliquées (sauf quirk Uniwill) : un tick sans
//!   changement de consigne ne touche pas sysfs.
//! - En `dry-run` (`--dev-session`, `--check`), aucune écriture n'est faite ;
//!   le worker se contente de lire et de publier.
//! - Arrêt propre : les ventilateurs sont rendus au firmware avant de quitter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_core::fan::{
    FanCurve, FanCurveEvaluator, FanLimits, FanTableEntry, FanTarget as CoreFanTarget,
};
use tcc_core::{FanProfile, FanTableSet, Profile};
use tcc_hw::fan::{self, FanControl, FanTarget};
use tcc_hw::sysfs::SysfsRoot;
use tokio::sync::RwLock;

use crate::runtime::RuntimeState;

/// Période de régulation.
pub const FAN_TICK: Duration = Duration::from_secs(1);
/// Seuil d'hystérésis appliqué à chaque ventilateur, en pourcentage.
pub const HYSTERESIS_PERCENT: f32 = 2.0;
/// En mode automatique, rafraîchit la télémétrie un tick sur N (5 s).
const IDLE_REFRESH_EVERY: u64 = 5;
/// Au-dessus de ce seuil, la vitesse ne peut pas chuter de plus de 2 %/tick.
const SPEED_JUMP_THRESHOLD: u8 = 20;
/// Chute maximale par tick au-dessus du seuil, en pourcentage.
const MAX_SPEED_JUMP: u8 = 2;
/// Délai maximal accordé au worker pour s'arrêter.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
/// Attente maximale de la boucle avant la veille, pour laisser le temps au
/// retour en mode auto dans le budget global de 3 s.
const SUSPEND_WAIT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Mode courant des ventilateurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanMode {
    /// Aucun backend disponible : rien n'est piloté.
    Off,
    /// Firmware en mode automatique (contrôle daemon inactif).
    Auto,
    /// Le daemon impose les vitesses.
    Manual,
}

/// État d'un ventilateur exposé à l'UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanReading {
    /// Index 0-based.
    pub index: u8,
    /// Libellé matériel (`cpu`, `gpu1`, `fan1`, ...).
    pub label: String,
    /// Rôle utilisé pour la courbe (`cpu` ou `gpu`).
    pub target: String,
    /// Température en °C, si le capteur répond.
    pub temp: Option<i32>,
    /// Vitesse mesurée en pourcentage, si elle est lisible.
    pub speed: Option<u8>,
    /// Dernière consigne appliquée par le daemon, en pourcentage.
    pub applied: Option<u8>,
}

/// Instantané des ventilateurs, sérialisé par `GetFanDataJSON`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanData {
    /// Vrai si un backend a été détecté.
    pub available: bool,
    /// Backend actif (`tuxi`, `pwm`, `tuxedo-io`).
    pub backend: Option<String>,
    /// Vrai si le backend accepte les écritures.
    pub writable: bool,
    /// Vrai si le profil actif et les réglages demandent le contrôle.
    pub control_enabled: bool,
    /// Mode effectif.
    pub mode: FanMode,
    /// Détail par ventilateur.
    pub fans: Vec<FanReading>,
}

impl FanData {
    /// État initial, avant la première détection.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            available: false,
            backend: None,
            writable: false,
            control_enabled: false,
            mode: FanMode::Off,
            fans: Vec::new(),
        }
    }
}

impl Default for FanData {
    fn default() -> Self {
        Self::unavailable()
    }
}

/// Entrées d'un tick, extraites de [`RuntimeState`].
#[derive(Debug, Clone)]
struct TickInput {
    enabled: bool,
    profile: Option<Profile>,
    fan_tables: FanTableSet,
    revision: u64,
}

/// État persistant d'un ventilateur entre deux ticks.
#[derive(Debug)]
struct Slot {
    index: u8,
    label: String,
    target: FanTarget,
    evaluator: FanCurveEvaluator,
    last_commanded: Option<u8>,
    last_temp: Option<i32>,
    temperature_error_logged: bool,
    /// Un repli de température a déjà été journalisé pour ce slot. Remis à
    /// faux dès que le capteur propre répond de nouveau.
    fallback_logged: bool,
    write_error_logged: bool,
}

/// Courbes résolues pour un tick.
#[derive(Debug)]
struct Curves {
    cpu: FanCurve,
    gpu: FanCurve,
}

impl Curves {
    fn resolve(fan: &FanProfile, tables: &FanTableSet) -> Self {
        Self {
            cpu: resolve_curve(fan, tables, CoreFanTarget::Cpu),
            gpu: resolve_curve(fan, tables, CoreFanTarget::Gpu),
        }
    }

    fn for_target(&self, target: FanTarget) -> &FanCurve {
        match target {
            FanTarget::Cpu => &self.cpu,
            FanTarget::Gpu => &self.gpu,
        }
    }
}

/// Sélectionne la courbe d'une cible.
///
/// Ordre : courbe personnalisée du profil (`Custom`), courbe utilisateur du
/// `FanTableSet`, puis courbe intégrée du même nom, puis `Balanced`.
fn resolve_curve(fan: &FanProfile, tables: &FanTableSet, target: CoreFanTarget) -> FanCurve {
    if fan.uses_custom_curve() {
        return fan.custom_fan_curve.clone();
    }
    if let Some(curve) = tables.get(&fan.fan_profile) {
        return curve.clone();
    }
    builtin_curve(&fan.fan_profile).unwrap_or_else(|| {
        tracing::debug!(
            profile = %fan.fan_profile,
            target = ?target,
            "courbe de ventilation inconnue, utilisation de Balanced"
        );
        builtin_curve("Balanced").unwrap_or_else(FanCurve::default_custom)
    })
}

/// Construit une courbe intégrée par son nom (`Silent`, `Quiet`, `Balanced`,
/// `Cool`, `Freezy`).
fn builtin_curve(name: &str) -> Option<FanCurve> {
    let (name, cpu, gpu) = BUILTIN_CURVES
        .iter()
        .find(|(candidate, _, _)| *candidate == name)?;
    let to_entries = |points: &[(f32, f32)]| -> Vec<FanTableEntry> {
        points
            .iter()
            .map(|&(temp, speed)| FanTableEntry::new(temp, speed))
            .collect()
    };
    Some(FanCurve::new(*name, to_entries(cpu), to_entries(gpu)))
}

/// Limites appliquées par l'évaluateur.
///
/// L'original ignore min/max/offset lorsqu'une courbe personnalisée est
/// utilisée (`FanControlBaseClass.setFanProfileValues`) : les bornes du profil
/// décrivent la courbe nommée, pas la courbe dessinée par l'utilisateur.
fn limits_for(fan: &FanProfile) -> FanLimits {
    if fan.uses_custom_curve() {
        FanLimits::default()
    } else {
        FanLimits::new(
            fan.minimum_fanspeed,
            fan.maximum_fanspeed,
            fan.offset_fanspeed,
        )
    }
}

/// Arrondit une consigne en pourcentage, sans panic.
fn round_percent(value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    value.clamp(0.0, 100.0).round() as u8
}

/// Applique la vitesse minimale matérielle (`applyHwFanLimitations`).
///
/// Si `fans_off_available` est faux, la consigne ne descend jamais sous le
/// minimum : l'original pouvait renvoyer une valeur inférieure au minimum dans
/// ce cas précis, ce qui n'a pas de sens pour un EC qui ne sait pas s'arrêter.
fn apply_hw_limits(speed: u8, min_speed: u8, fans_off_available: bool) -> u8 {
    let min = min_speed.min(100);
    if speed >= min {
        return speed;
    }
    if fans_off_available && speed < min / 2 {
        0
    } else {
        min
    }
}

/// Limite la chute de vitesse à 2 %/tick au-dessus de 20 %.
fn limit_falling_speed(speed: u8, last: Option<u8>) -> u8 {
    let Some(last) = last else {
        return speed;
    };
    if last > SPEED_JUMP_THRESHOLD && speed.saturating_add(MAX_SPEED_JUMP) <= last {
        last - MAX_SPEED_JUMP
    } else {
        speed
    }
}

/// Plancher de sécurité au-dessus de 80 °C / 90 °C
/// (`FanUtils.manageCriticalTemperature`).
fn critical_temperature_floor(temp_c: i32, speed: u8) -> u8 {
    if temp_c >= 90 {
        speed.max(40)
    } else if temp_c >= 80 {
        speed.max(30)
    } else {
        speed
    }
}

fn core_target(target: FanTarget) -> CoreFanTarget {
    match target {
        FanTarget::Cpu => CoreFanTarget::Cpu,
        FanTarget::Gpu => CoreFanTarget::Gpu,
    }
}

/// Exécution d'un tick, hors contexte asynchrone.
#[derive(Debug)]
struct FanRuntime {
    control: Arc<dyn FanControl>,
    dry_run: bool,
    mode: FanMode,
    signature: Option<String>,
    slots: Vec<Slot>,
    readings: Vec<FanReading>,
    idle_ticks: u64,
    no_temperature_logged: bool,
}

impl FanRuntime {
    fn new(control: Arc<dyn FanControl>, dry_run: bool) -> Self {
        Self {
            control,
            dry_run,
            // `Off` force un premier passage par `set_fans_auto` : si un
            // daemon précédent a laissé les ventilateurs en manuel, ils sont
            // rendus au firmware avant toute régulation.
            mode: FanMode::Off,
            signature: None,
            slots: Vec::new(),
            readings: Vec::new(),
            idle_ticks: 0,
            no_temperature_logged: false,
        }
    }

    fn tick(&mut self, input: &TickInput) -> FanData {
        let control = Arc::clone(&self.control);
        let writable = control.has_write_support();
        let requested = input.enabled
            && input
                .profile
                .as_ref()
                .is_some_and(|profile| profile.fan.use_control);

        let mut data = FanData {
            available: true,
            backend: Some(control.backend().as_str().to_owned()),
            writable,
            control_enabled: requested,
            mode: self.mode,
            fans: Vec::new(),
        };

        if !requested || !writable || self.dry_run {
            self.ensure_auto(&control);
            self.idle_ticks = self.idle_ticks.saturating_add(1);
            if self.idle_ticks % IDLE_REFRESH_EVERY == 0 || self.readings.is_empty() {
                self.ensure_slots(&control);
                let _ = self.read_temperatures(&control);
                self.readings = self.collect_readings(&control);
            }
            data.mode = self.mode;
            data.fans = self.readings.clone();
            return data;
        }

        let Some(profile) = input.profile.clone() else {
            self.ensure_auto(&control);
            data.mode = self.mode;
            return data;
        };

        self.sync_profile(&control, &profile, input);
        self.apply(&control, &profile, input);
        self.readings = self.collect_readings(&control);
        data.mode = self.mode;
        data.fans = self.readings.clone();
        data
    }

    /// Reconstruit les évaluateurs quand le profil ou les limites changent.
    fn sync_profile(
        &mut self,
        control: &Arc<dyn FanControl>,
        profile: &Profile,
        input: &TickInput,
    ) {
        let signature = format!(
            "{}|{}|{}|{}|{}|{}",
            input.revision,
            profile.fan.fan_profile,
            profile.fan.use_control,
            profile.fan.minimum_fanspeed,
            profile.fan.maximum_fanspeed,
            profile.fan.offset_fanspeed,
        );
        if self.signature.as_deref() == Some(signature.as_str()) && !self.slots.is_empty() {
            return;
        }
        self.signature = Some(signature);
        self.slots = build_slots(control, limits_for(&profile.fan));
        tracing::debug!(
            fans = self.slots.len(),
            profile = %profile.fan.fan_profile,
            "courbes de ventilation rechargées"
        );
    }

    /// Construit les slots une première fois en lecture seule.
    fn ensure_slots(&mut self, control: &Arc<dyn FanControl>) {
        if self.slots.is_empty() {
            self.slots = build_slots(control, FanLimits::default());
        }
    }

    /// Repasse les ventilateurs en automatique si nécessaire.
    fn ensure_auto(&mut self, control: &Arc<dyn FanControl>) {
        if self.mode == FanMode::Auto {
            return;
        }
        if !self.dry_run {
            match control.set_fans_auto() {
                Ok(()) => {}
                Err(error) if error.is_unavailable() => {
                    tracing::debug!(%error, "retour en mode auto non supporté");
                }
                Err(error) => tracing::warn!(%error, "retour en mode auto impossible"),
            }
            if let Err(error) = control.set_manual_mode(false) {
                tracing::debug!(%error, "désactivation du mode manuel impossible");
            }
        }
        self.mode = FanMode::Auto;
        for slot in &mut self.slots {
            slot.last_commanded = None;
            slot.fallback_logged = false;
        }
    }

    /// Lit les températures de tous les ventilateurs.
    ///
    /// Retourne `true` si au moins un capteur répond. Les erreurs ne sont
    /// journalisées qu'une fois par ventilateur pour ne pas spammer journald
    /// à chaque tick.
    fn read_temperatures(&mut self, control: &Arc<dyn FanControl>) -> bool {
        let mut any = false;
        for slot in &mut self.slots {
            match control.fan_temperature(slot.index) {
                Ok(value) => {
                    slot.last_temp = Some(value);
                    slot.temperature_error_logged = false;
                    any = true;
                }
                Err(error) => {
                    slot.last_temp = None;
                    if !slot.temperature_error_logged {
                        slot.temperature_error_logged = true;
                        tracing::debug!(
                            fan = slot.index,
                            %error,
                            "température de ventilateur indisponible, consigne conservée"
                        );
                    }
                }
            }
        }
        any
    }

    /// Régulation complète pour le profil courant.
    fn apply(&mut self, control: &Arc<dyn FanControl>, profile: &Profile, input: &TickInput) {
        if !self.read_temperatures(control) {
            // Aucune température exploitable : ne pas verrouiller l'EC en
            // manuel sans consigne. Le prochain tick retentera, sans répéter
            // le log à chaque seconde.
            if !self.no_temperature_logged {
                self.no_temperature_logged = true;
                tracing::debug!("aucune température de ventilateur : maintien en mode auto");
            }
            self.ensure_auto(control);
            return;
        }
        self.no_temperature_logged = false;

        if self.mode != FanMode::Manual {
            if !self.dry_run {
                match control.set_manual_mode(true) {
                    Ok(()) => {}
                    Err(error) => {
                        tracing::warn!(%error, "passage en mode manuel impossible");
                    }
                }
            }
            self.mode = FanMode::Manual;
            for slot in &mut self.slots {
                slot.evaluator.reset();
                slot.last_commanded = None;
            }
        }

        let curves = Curves::resolve(&profile.fan, &input.fan_tables);
        let min_speed = control.fans_min_speed();
        let off_available = control.fans_off_available();
        let every_tick = control.writes_every_tick();
        let dry_run = self.dry_run;
        // Repli commun au backend : première température valide, typiquement
        // le CPU. Un ventilateur sans capteur propre doit quand même recevoir
        // une consigne en mode manuel, sinon l'EC le laisse à 100 %.
        let fallback_temperature = self.slots.iter().find_map(|slot| slot.last_temp);

        for slot in &mut self.slots {
            let (temperature, used_fallback) = match slot.last_temp {
                Some(temperature) => (temperature, false),
                None => match fallback_temperature {
                    Some(temperature) => (temperature, true),
                    None => continue,
                },
            };
            if used_fallback {
                if !slot.fallback_logged {
                    slot.fallback_logged = true;
                    tracing::info!(
                        fan = slot.index,
                        fallback_temp = temperature,
                        "température propre indisponible : température de repli utilisée"
                    );
                }
            } else {
                slot.fallback_logged = false;
            }

            let curve = curves.for_target(slot.target);
            let Some(raw) =
                slot.evaluator
                    .evaluate(curve, core_target(slot.target), temperature as f32)
            else {
                continue;
            };

            let mut speed = round_percent(raw);
            speed = apply_hw_limits(speed, min_speed, off_available);
            speed = limit_falling_speed(speed, slot.last_commanded);
            speed = critical_temperature_floor(temperature, speed);
            speed = speed.min(100);

            let changed = slot.last_commanded != Some(speed);
            if !changed && !every_tick {
                continue;
            }

            if dry_run {
                slot.last_commanded = Some(speed);
                continue;
            }
            match control.set_fan_speed_percent(slot.index, speed) {
                Ok(()) => {
                    slot.last_commanded = Some(speed);
                    slot.write_error_logged = false;
                }
                Err(error) => {
                    if !slot.write_error_logged {
                        slot.write_error_logged = true;
                        tracing::warn!(
                            fan = slot.index,
                            speed,
                            %error,
                            "écriture de la vitesse de ventilateur échouée"
                        );
                    }
                }
            }
        }
    }

    /// Lit températures et vitesses sans écrire.
    fn collect_readings(&self, control: &Arc<dyn FanControl>) -> Vec<FanReading> {
        self.slots
            .iter()
            .map(|slot| FanReading {
                index: slot.index,
                label: slot.label.clone(),
                target: slot.target.as_str().to_owned(),
                temp: slot.last_temp,
                speed: control.fan_speed_percent(slot.index).ok(),
                applied: slot.last_commanded,
            })
            .collect()
    }
}

/// Construit les slots de régulation pour tous les ventilateurs du backend.
fn build_slots(control: &Arc<dyn FanControl>, limits: FanLimits) -> Vec<Slot> {
    (0..control.fan_count())
        .map(|index| Slot {
            index,
            label: control.fan_label(index),
            target: control.fan_target(index),
            evaluator: FanCurveEvaluator::with_hysteresis(limits, HYSTERESIS_PERCENT),
            last_commanded: None,
            last_temp: None,
            temperature_error_logged: false,
            fallback_logged: false,
            write_error_logged: false,
        })
        .collect()
}

/// Worker tokio des ventilateurs.
#[derive(Debug)]
pub struct FanWorker {
    handle: Option<tokio::task::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
    control: Option<Arc<dyn FanControl>>,
    /// Runtime partagé avec la boucle, utilisé pour attendre la fin d'un tick
    /// `spawn_blocking` avant le retour auto final.
    runtime: Option<Arc<Mutex<FanRuntime>>>,
    dry_run: bool,
}

impl FanWorker {
    /// Détecte le backend et démarre le worker.
    ///
    /// La détection est faite dans `spawn_blocking` : elle ouvre
    /// `/dev/tuxedo_io` et parcourt `/sys/class/hwmon`. Un backend absent
    /// laisse simplement le worker inactif ; le statut D-Bus reste
    /// `available: false`.
    pub async fn start(
        state: Arc<RwLock<RuntimeState>>,
        status: Arc<std::sync::RwLock<FanData>>,
        root: SysfsRoot,
        dry_run: bool,
    ) -> Self {
        let control = match tokio::task::spawn_blocking(move || fan::detect(&root)).await {
            Ok(control) => control,
            Err(error) => {
                tracing::warn!(%error, "détection des ventilateurs interrompue");
                None
            }
        };

        let Some(control) = control else {
            tracing::info!("aucun backend de ventilation disponible");
            if let Ok(mut guard) = status.write() {
                *guard = FanData::unavailable();
            }
            return Self {
                handle: None,
                stop: Arc::new(AtomicBool::new(false)),
                notify: Arc::new(tokio::sync::Notify::new()),
                control: None,
                runtime: None,
                dry_run,
            };
        };

        tracing::info!(
            backend = control.backend().as_str(),
            fans = control.fan_count(),
            writable = control.has_write_support(),
            "régulation des ventilateurs démarrée"
        );

        let runtime = Arc::new(Mutex::new(FanRuntime::new(Arc::clone(&control), dry_run)));
        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(run_loop(
            state,
            Arc::clone(&runtime),
            Arc::clone(&status),
            Arc::clone(&stop),
            Arc::clone(&notify),
        ));
        Self {
            handle: Some(handle),
            stop,
            notify,
            control: Some(control),
            runtime: Some(runtime),
            dry_run,
        }
    }

    /// Arrête le worker et rend les ventilateurs au firmware.
    pub async fn shutdown(self) {
        self.stop_worker(SHUTDOWN_TIMEOUT, "arrêt").await;
    }

    /// Arrête le worker pour la veille, en priorisant le retour en mode auto.
    ///
    /// Le mode auto est appliqué **avant** l'attente de la boucle, puis une
    /// seconde fois après : même si la boucle est lente ou si le budget global
    /// de veille interrompt l'attente, l'EC n'est pas laissé en manuel. Les
    /// deux appels sont idempotents.
    pub async fn suspend(self) {
        self.stop_worker(SUSPEND_WAIT_TIMEOUT, "veille").await;
    }

    async fn stop_worker(self, wait: Duration, reason: &'static str) {
        self.stop.store(true, Ordering::SeqCst);
        self.notify.notify_one();

        // Abort **avant** toute attente : l'outer timeout de veille ne peut
        // plus interrompre un arrêt déjà engagé (SL-01).
        if let Some(handle) = self.handle {
            handle.abort();
        }

        let Some(control) = self.control else {
            return;
        };
        if self.dry_run {
            return;
        }

        // `set_auto` est exécuté dans une tâche détachée : même si le budget
        // de veille annule ce future, le retour auto va à son terme. Le
        // verrou runtime garantit qu'un tick `spawn_blocking` en cours est
        // terminé avant le dernier écrivain.
        let runtime = self.runtime;
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(runtime) = runtime {
                let _ = tokio::task::spawn_blocking(move || {
                    // Attendre la fin d'un éventuel tick en cours.
                    let _guard = runtime.lock();
                })
                .await;
            }
            set_auto(control).await;
            let _ = done_tx.send(());
        });
        if tokio::time::timeout(wait, done_rx).await.is_err() {
            tracing::warn!(
                reason,
                "retour auto des ventilateurs non confirmé dans le budget"
            );
        }
    }

    /// Constructeur de test sans détection ni boucle de régulation.
    #[cfg(test)]
    fn for_test(control: Arc<dyn FanControl>, dry_run: bool) -> Self {
        Self {
            handle: None,
            stop: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
            control: Some(control),
            runtime: None,
            dry_run,
        }
    }
}

/// Applique le retour auto + libération du mode manuel.
async fn set_auto(control: Arc<dyn FanControl>) {
    let result = tokio::task::spawn_blocking(move || {
        if let Err(error) = control.set_fans_auto() {
            tracing::warn!(%error, "veille/arrêt : retour en mode auto impossible");
        }
        if let Err(error) = control.set_manual_mode(false) {
            tracing::debug!(%error, "veille/arrêt : mode manuel Uniwill non libéré");
        }
    })
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "veille/arrêt des ventilateurs interrompu");
    }
}

async fn run_loop(
    state: Arc<RwLock<RuntimeState>>,
    runtime: Arc<Mutex<FanRuntime>>,
    status: Arc<std::sync::RwLock<FanData>>,
    stop: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
) {
    let mut interval = tokio::time::interval(FAN_TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = notify.notified() => {}
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let input = {
            let guard = state.read().await;
            TickInput {
                enabled: guard.settings.fan_control_enabled,
                profile: guard.active_profile().cloned(),
                fan_tables: guard.fan_tables.clone(),
                revision: guard.revision,
            }
        };

        let runtime_handle = Arc::clone(&runtime);
        let result = tokio::task::spawn_blocking(move || match runtime_handle.lock() {
            Ok(mut guard) => Some(guard.tick(&input)),
            Err(_) => None,
        })
        .await;

        match result {
            Ok(Some(data)) => {
                if let Ok(mut guard) = status.write() {
                    *guard = data;
                }
            }
            Ok(None) => tracing::warn!("état interne des ventilateurs empoisonné"),
            Err(error) => tracing::warn!(%error, "tick de régulation interrompu"),
        }
    }
}

/// Courbes de ventilation intégrées, identiques aux `defaultFanProfiles`
/// de l'original (`src/common/models/TccFanTable.ts`).
///
/// Les tables d'origine définissent une valeur par degré (fonction en
/// escalier). Elles sont encodées ici par leurs points de rupture, la rampe
/// entre deux paliers étant limitée à 1 °C : la valeur interpolée reste
/// toujours comprise entre le palier précédent et le suivant, donc jamais
/// inférieure à la consigne d'origine pour une même température entière.
type CurvePoints = &'static [(f32, f32)];
/// Nom, table CPU et table GPU d'une courbe intégrée.
type BuiltinCurve = (&'static str, CurvePoints, CurvePoints);
const BUILTIN_CURVES: [BuiltinCurve; 5] = [
    // Silent CPU : 32 points
    (
        "Silent",
        &[
            (0.0, 0.0),
            (60.0, 0.0),
            (61.0, 20.0),
            (65.0, 20.0),
            (66.0, 25.0),
            (69.0, 25.0),
            (70.0, 30.0),
            (71.0, 30.0),
            (72.0, 35.0),
            (73.0, 35.0),
            (74.0, 40.0),
            (75.0, 40.0),
            (76.0, 45.0),
            (77.0, 45.0),
            (78.0, 50.0),
            (79.0, 50.0),
            (80.0, 55.0),
            (81.0, 55.0),
            (82.0, 60.0),
            (83.0, 60.0),
            (84.0, 65.0),
            (85.0, 65.0),
            (86.0, 70.0),
            (87.0, 70.0),
            (88.0, 75.0),
            (89.0, 80.0),
            (90.0, 85.0),
            (91.0, 90.0),
            (92.0, 90.0),
            (93.0, 95.0),
            (94.0, 95.0),
            (95.0, 100.0),
        ],
        // Silent GPU : 34 points
        &[
            (0.0, 0.0),
            (59.0, 0.0),
            (60.0, 20.0),
            (61.0, 20.0),
            (62.0, 22.0),
            (63.0, 22.0),
            (64.0, 23.0),
            (65.0, 24.0),
            (66.0, 25.0),
            (67.0, 25.0),
            (68.0, 28.0),
            (69.0, 30.0),
            (70.0, 33.0),
            (71.0, 37.0),
            (72.0, 40.0),
            (73.0, 43.0),
            (74.0, 44.0),
            (75.0, 46.0),
            (76.0, 48.0),
            (77.0, 52.0),
            (78.0, 52.0),
            (79.0, 55.0),
            (80.0, 55.0),
            (81.0, 60.0),
            (82.0, 60.0),
            (83.0, 65.0),
            (84.0, 65.0),
            (85.0, 70.0),
            (86.0, 70.0),
            (87.0, 80.0),
            (88.0, 80.0),
            (89.0, 90.0),
            (90.0, 90.0),
            (91.0, 100.0),
        ],
    ),
    // Quiet CPU : 38 points
    (
        "Quiet",
        &[
            (0.0, 0.0),
            (50.0, 0.0),
            (51.0, 20.0),
            (60.0, 20.0),
            (61.0, 22.0),
            (63.0, 22.0),
            (64.0, 23.0),
            (65.0, 24.0),
            (66.0, 25.0),
            (67.0, 25.0),
            (68.0, 28.0),
            (69.0, 30.0),
            (70.0, 33.0),
            (71.0, 37.0),
            (72.0, 40.0),
            (73.0, 43.0),
            (74.0, 44.0),
            (75.0, 46.0),
            (76.0, 48.0),
            (77.0, 52.0),
            (78.0, 52.0),
            (79.0, 55.0),
            (80.0, 55.0),
            (81.0, 60.0),
            (82.0, 60.0),
            (83.0, 65.0),
            (84.0, 65.0),
            (85.0, 70.0),
            (86.0, 70.0),
            (87.0, 80.0),
            (88.0, 80.0),
            (89.0, 85.0),
            (90.0, 85.0),
            (91.0, 90.0),
            (92.0, 90.0),
            (93.0, 95.0),
            (94.0, 95.0),
            (95.0, 100.0),
        ],
        // Quiet GPU : 30 points
        &[
            (0.0, 0.0),
            (50.0, 0.0),
            (51.0, 20.0),
            (60.0, 20.0),
            (61.0, 25.0),
            (64.0, 25.0),
            (65.0, 30.0),
            (68.0, 30.0),
            (69.0, 35.0),
            (71.0, 35.0),
            (72.0, 40.0),
            (73.0, 43.0),
            (74.0, 44.0),
            (75.0, 46.0),
            (76.0, 48.0),
            (77.0, 52.0),
            (78.0, 52.0),
            (79.0, 55.0),
            (80.0, 55.0),
            (81.0, 60.0),
            (82.0, 60.0),
            (83.0, 65.0),
            (84.0, 65.0),
            (85.0, 70.0),
            (86.0, 70.0),
            (87.0, 80.0),
            (88.0, 80.0),
            (89.0, 90.0),
            (90.0, 90.0),
            (91.0, 100.0),
        ],
    ),
    // Balanced CPU : 43 points
    (
        "Balanced",
        &[
            (0.0, 0.0),
            (45.0, 0.0),
            (46.0, 20.0),
            (51.0, 20.0),
            (52.0, 23.0),
            (53.0, 23.0),
            (54.0, 26.0),
            (56.0, 26.0),
            (57.0, 30.0),
            (59.0, 30.0),
            (60.0, 33.0),
            (62.0, 33.0),
            (63.0, 35.0),
            (64.0, 35.0),
            (65.0, 38.0),
            (66.0, 40.0),
            (67.0, 42.0),
            (68.0, 45.0),
            (69.0, 47.0),
            (70.0, 50.0),
            (71.0, 50.0),
            (72.0, 52.0),
            (73.0, 53.0),
            (74.0, 53.0),
            (75.0, 57.0),
            (76.0, 57.0),
            (77.0, 60.0),
            (78.0, 60.0),
            (79.0, 63.0),
            (80.0, 65.0),
            (81.0, 65.0),
            (82.0, 70.0),
            (83.0, 70.0),
            (84.0, 75.0),
            (85.0, 75.0),
            (86.0, 80.0),
            (87.0, 80.0),
            (88.0, 85.0),
            (89.0, 90.0),
            (91.0, 90.0),
            (92.0, 95.0),
            (94.0, 95.0),
            (95.0, 100.0),
        ],
        // Balanced GPU : 41 points
        &[
            (0.0, 0.0),
            (45.0, 0.0),
            (46.0, 20.0),
            (51.0, 20.0),
            (52.0, 23.0),
            (53.0, 23.0),
            (54.0, 26.0),
            (56.0, 26.0),
            (57.0, 30.0),
            (59.0, 30.0),
            (60.0, 33.0),
            (62.0, 33.0),
            (63.0, 35.0),
            (64.0, 35.0),
            (65.0, 38.0),
            (66.0, 40.0),
            (67.0, 42.0),
            (68.0, 45.0),
            (69.0, 47.0),
            (70.0, 50.0),
            (71.0, 50.0),
            (72.0, 52.0),
            (73.0, 53.0),
            (74.0, 53.0),
            (75.0, 57.0),
            (76.0, 57.0),
            (77.0, 60.0),
            (78.0, 60.0),
            (79.0, 63.0),
            (80.0, 65.0),
            (81.0, 65.0),
            (82.0, 70.0),
            (83.0, 70.0),
            (84.0, 75.0),
            (85.0, 75.0),
            (86.0, 80.0),
            (87.0, 80.0),
            (88.0, 85.0),
            (89.0, 90.0),
            (90.0, 90.0),
            (91.0, 100.0),
        ],
    ),
    // Cool CPU : 43 points
    (
        "Cool",
        &[
            (0.0, 0.0),
            (39.0, 0.0),
            (40.0, 20.0),
            (45.0, 20.0),
            (46.0, 25.0),
            (50.0, 25.0),
            (51.0, 30.0),
            (55.0, 30.0),
            (56.0, 32.0),
            (57.0, 33.0),
            (58.0, 34.0),
            (59.0, 35.0),
            (60.0, 35.0),
            (61.0, 37.0),
            (62.0, 40.0),
            (63.0, 40.0),
            (64.0, 42.0),
            (65.0, 45.0),
            (67.0, 45.0),
            (68.0, 47.0),
            (69.0, 50.0),
            (70.0, 50.0),
            (71.0, 52.0),
            (72.0, 55.0),
            (73.0, 55.0),
            (74.0, 57.0),
            (75.0, 60.0),
            (76.0, 60.0),
            (77.0, 65.0),
            (78.0, 65.0),
            (79.0, 70.0),
            (80.0, 70.0),
            (81.0, 75.0),
            (82.0, 75.0),
            (83.0, 80.0),
            (84.0, 80.0),
            (85.0, 85.0),
            (86.0, 85.0),
            (87.0, 90.0),
            (89.0, 90.0),
            (90.0, 95.0),
            (94.0, 95.0),
            (95.0, 100.0),
        ],
        // Cool GPU : 27 points
        &[
            (0.0, 0.0),
            (39.0, 0.0),
            (40.0, 25.0),
            (44.0, 25.0),
            (45.0, 30.0),
            (49.0, 30.0),
            (50.0, 35.0),
            (54.0, 35.0),
            (55.0, 40.0),
            (59.0, 40.0),
            (60.0, 45.0),
            (64.0, 45.0),
            (65.0, 50.0),
            (69.0, 50.0),
            (70.0, 60.0),
            (74.0, 60.0),
            (75.0, 70.0),
            (79.0, 70.0),
            (80.0, 75.0),
            (84.0, 75.0),
            (85.0, 85.0),
            (86.0, 85.0),
            (87.0, 90.0),
            (88.0, 90.0),
            (89.0, 95.0),
            (90.0, 95.0),
            (91.0, 100.0),
        ],
    ),
    // Freezy CPU : 33 points
    (
        "Freezy",
        &[
            (0.0, 20.0),
            (29.0, 20.0),
            (30.0, 25.0),
            (39.0, 25.0),
            (40.0, 30.0),
            (45.0, 30.0),
            (46.0, 35.0),
            (49.0, 35.0),
            (50.0, 40.0),
            (55.0, 40.0),
            (56.0, 45.0),
            (60.0, 45.0),
            (61.0, 50.0),
            (65.0, 50.0),
            (66.0, 55.0),
            (70.0, 55.0),
            (71.0, 60.0),
            (75.0, 60.0),
            (76.0, 65.0),
            (77.0, 65.0),
            (78.0, 70.0),
            (79.0, 70.0),
            (80.0, 75.0),
            (81.0, 75.0),
            (82.0, 80.0),
            (83.0, 80.0),
            (84.0, 85.0),
            (85.0, 85.0),
            (86.0, 90.0),
            (89.0, 90.0),
            (90.0, 95.0),
            (94.0, 95.0),
            (95.0, 100.0),
        ],
        // Freezy GPU : 25 points
        &[
            (0.0, 25.0),
            (35.0, 25.0),
            (36.0, 30.0),
            (40.0, 30.0),
            (41.0, 35.0),
            (45.0, 35.0),
            (46.0, 40.0),
            (50.0, 40.0),
            (51.0, 45.0),
            (55.0, 45.0),
            (56.0, 50.0),
            (60.0, 50.0),
            (61.0, 60.0),
            (65.0, 60.0),
            (66.0, 65.0),
            (70.0, 65.0),
            (71.0, 70.0),
            (75.0, 70.0),
            (76.0, 75.0),
            (80.0, 75.0),
            (81.0, 85.0),
            (85.0, 85.0),
            (86.0, 95.0),
            (90.0, 95.0),
            (91.0, 100.0),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_core::presets::office_profile;

    /// Backend factice qui enregistre les écritures.
    #[derive(Debug)]
    struct FakeFan {
        temps: Mutex<Vec<i32>>,
        writes: Mutex<Vec<(u8, u8)>>,
        manual: Mutex<Option<bool>>,
        auto_calls: Mutex<u32>,
        min_speed: u8,
        off_available: bool,
        writable: bool,
        every_tick: bool,
        /// Cibles explicites ; par défaut fan0=CPU, les suivants=GPU.
        targets: Option<Vec<FanTarget>>,
    }

    impl FakeFan {
        fn new(temps: Vec<i32>) -> Self {
            Self {
                temps: Mutex::new(temps),
                writes: Mutex::new(Vec::new()),
                manual: Mutex::new(None),
                auto_calls: Mutex::new(0),
                min_speed: 0,
                off_available: true,
                writable: true,
                every_tick: false,
                targets: None,
            }
        }

        fn with_targets(mut self, targets: Vec<FanTarget>) -> Self {
            self.targets = Some(targets);
            self
        }

        fn with_min_speed(mut self, min_speed: u8, off_available: bool) -> Self {
            self.min_speed = min_speed;
            self.off_available = off_available;
            self
        }

        fn writes(&self) -> Vec<(u8, u8)> {
            self.writes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }

        fn auto_calls(&self) -> u32 {
            self.auto_calls.lock().map(|guard| *guard).unwrap_or(0)
        }
    }

    impl FanControl for FakeFan {
        fn backend(&self) -> fan::FanBackendKind {
            fan::FanBackendKind::Tuxi
        }

        fn fan_count(&self) -> u8 {
            self.temps
                .lock()
                .map(|guard| guard.len() as u8)
                .unwrap_or(0)
        }

        fn fan_label(&self, fan: u8) -> String {
            match fan {
                0 => "cpu".to_owned(),
                _ => format!("gpu{fan}"),
            }
        }

        fn fan_target(&self, fan: u8) -> FanTarget {
            self.targets
                .as_ref()
                .and_then(|targets| targets.get(usize::from(fan)).copied())
                .unwrap_or(if fan == 0 {
                    FanTarget::Cpu
                } else {
                    FanTarget::Gpu
                })
        }

        fn fan_temperature(&self, fan: u8) -> tcc_hw::Result<i32> {
            let value = self
                .temps
                .lock()
                .ok()
                .and_then(|guard| guard.get(usize::from(fan)).copied())
                .ok_or(tcc_hw::HwError::SensorUnavailable { sensor: "test" })?;
            if value <= 0 {
                return Err(tcc_hw::HwError::SensorUnavailable { sensor: "test" });
            }
            Ok(value)
        }

        fn fan_speed_percent(&self, fan: u8) -> tcc_hw::Result<u8> {
            if usize::from(fan) >= usize::from(self.fan_count()) {
                return Err(tcc_hw::HwError::OutOfRange {
                    context: "index ventilateur",
                    value: i64::from(fan),
                    min: 0,
                    max: i64::from(self.fan_count()) - 1,
                });
            }
            Ok(42)
        }

        fn set_fan_speed_percent(&self, fan: u8, percent: u8) -> tcc_hw::Result<()> {
            if let Ok(mut guard) = self.writes.lock() {
                guard.push((fan, percent));
            }
            Ok(())
        }

        fn set_manual_mode(&self, manual: bool) -> tcc_hw::Result<()> {
            if let Ok(mut guard) = self.manual.lock() {
                *guard = Some(manual);
            }
            Ok(())
        }

        fn set_fans_auto(&self) -> tcc_hw::Result<()> {
            if let Ok(mut guard) = self.auto_calls.lock() {
                *guard += 1;
            }
            Ok(())
        }

        fn fans_min_speed(&self) -> u8 {
            self.min_speed
        }

        fn fans_off_available(&self) -> bool {
            self.off_available
        }

        fn has_write_support(&self) -> bool {
            self.writable
        }

        fn writes_every_tick(&self) -> bool {
            self.every_tick
        }
    }

    fn profile_with(name: &str, use_control: bool) -> Profile {
        let mut profile = office_profile();
        profile.fan.fan_profile = name.to_owned();
        profile.fan.use_control = use_control;
        profile
    }

    fn input(profile: Option<Profile>, enabled: bool) -> TickInput {
        TickInput {
            enabled,
            profile,
            fan_tables: FanTableSet::default(),
            revision: 1,
        }
    }

    fn make_runtime(fake: Arc<FakeFan>, dry_run: bool) -> FanRuntime {
        FanRuntime::new(fake, dry_run)
    }

    #[test]
    fn balanced_curve_applies_expected_speed() {
        let fake = Arc::new(FakeFan::new(vec![70, 70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));

        assert_eq!(data.mode, FanMode::Manual);
        assert!(data.available);
        assert!(data.control_enabled);
        assert_eq!(fake.writes(), vec![(0, 50), (1, 50)]);
        assert_eq!(data.fans[0].applied, Some(50));
        assert_eq!(data.fans[0].temp, Some(70));
        assert_eq!(data.fans[0].speed, Some(42));
        assert_eq!(data.fans[0].target, "cpu");
        assert_eq!(data.fans[1].target, "gpu");
    }

    #[test]
    fn disabled_control_restores_auto() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), false));

        assert_eq!(data.mode, FanMode::Auto);
        assert!(fake.writes().is_empty());
        assert_eq!(fake.auto_calls(), 1);
        // Deuxième tick : pas de nouvelle écriture auto inutile.
        let _ = runtime.tick(&input(Some(profile_with("Balanced", true)), false));
        assert_eq!(fake.auto_calls(), 1);
    }

    #[test]
    fn profile_without_use_control_restores_auto() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let data = runtime.tick(&input(Some(profile_with("Balanced", false)), true));
        assert_eq!(data.mode, FanMode::Auto);
        assert!(fake.writes().is_empty());
    }

    #[test]
    fn profile_change_resets_evaluator() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let mut first = input(Some(profile_with("Balanced", true)), true);
        runtime.tick(&first);
        assert!(runtime.slots[0].evaluator.last_speed().is_some());

        first.revision = 2;
        first.profile = Some(profile_with("Silent", true));
        runtime.tick(&first);
        // Silent à 70 °C vaut 30 %, consigne appliquée en une fois malgré
        // l'hystérésis, car l'évaluateur a été réinitialisé.
        assert_eq!(fake.writes().last().copied(), Some((0, 30)));
    }

    #[test]
    fn dry_run_never_writes() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), true);
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(data.mode, FanMode::Auto);
        assert!(fake.writes().is_empty());
        assert_eq!(fake.auto_calls(), 0);
        assert!(fake.manual.lock().map(|g| *g).unwrap_or(None).is_none());
    }

    #[test]
    fn missing_temperature_keeps_last_commanded() {
        let fake = Arc::new(FakeFan::new(vec![70, 70]));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(runtime.slots[0].last_commanded, Some(50));
        assert_eq!(runtime.slots[1].last_commanded, Some(50));

        // Un seul capteur tombe : sa consigne est conservée, l'autre continue.
        if let Ok(mut temps) = fake.temps.lock() {
            temps[0] = -1;
        }
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(runtime.slots[0].last_commanded, Some(50));
        assert_eq!(runtime.slots[1].last_commanded, Some(50));
    }

    #[test]
    fn no_usable_temperature_returns_to_auto() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));

        if let Ok(mut temps) = fake.temps.lock() {
            temps[0] = -1;
        }
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(data.mode, FanMode::Auto);
        assert_eq!(runtime.slots[0].last_commanded, None);
        assert_eq!(fake.auto_calls(), 1);
    }

    #[test]
    fn missing_fan_temperature_uses_cpu_fallback() {
        // fan0 (CPU) à 70 °C, fan1 sans capteur propre, les deux ciblent CPU.
        let fake =
            Arc::new(FakeFan::new(vec![70, -1]).with_targets(vec![FanTarget::Cpu, FanTarget::Cpu]));
        let mut runtime = make_runtime(fake.clone(), false);
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));

        assert_eq!(data.mode, FanMode::Manual);
        let writes = fake.writes();
        assert!(
            writes.contains(&(0, 50)),
            "fan0 doit recevoir sa consigne: {writes:?}"
        );
        assert!(
            writes.contains(&(1, 50)),
            "fan1 doit recevoir une consigne de repli, pas rester à 100 %: {writes:?}"
        );
        assert_eq!(data.fans[0].applied, Some(50));
        assert_eq!(
            data.fans[1].applied,
            Some(50),
            "FanData.applied doit être renseigné pour fan1"
        );
        assert_eq!(
            data.fans[1].temp, None,
            "la température propre de fan1 reste indisponible"
        );
        assert!(runtime.slots[1].fallback_logged);
    }

    #[test]
    fn no_sensor_at_all_keeps_auto_with_two_fans() {
        let fake = Arc::new(FakeFan::new(vec![-1, -1]));
        let mut runtime = make_runtime(fake.clone(), false);
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));

        assert_eq!(data.mode, FanMode::Auto);
        assert!(fake.writes().is_empty());
        assert_eq!(fake.auto_calls(), 1);
        assert!(data.fans.iter().all(|fan| fan.applied.is_none()));
    }

    #[test]
    fn own_temperature_replaces_fallback() {
        let fake =
            Arc::new(FakeFan::new(vec![70, -1]).with_targets(vec![FanTarget::Cpu, FanTarget::Cpu]));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(runtime.slots[1].last_commanded, Some(50));

        // La température propre de fan1 revient à 90 °C : la courbe CPU donne
        // 90 %, valeur différente du repli à 70 °C (50 %).
        if let Ok(mut temps) = fake.temps.lock() {
            temps[1] = 90;
        }
        let data = runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(data.fans[1].temp, Some(90));
        assert_eq!(data.fans[1].applied, Some(90));
        assert!(!runtime.slots[1].fallback_logged);
    }

    #[test]
    fn min_speed_and_fans_off_are_applied() {
        // Ventilateurs non arrêtables : plancher au minimum matériel.
        let fake = Arc::new(FakeFan::new(vec![10]).with_min_speed(20, false));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(fake.writes(), vec![(0, 20)]);

        // Arrêt possible et consigne très basse : 0 %.
        let fake = Arc::new(FakeFan::new(vec![5]).with_min_speed(20, true));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile_with("Balanced", true)), true));
        assert_eq!(fake.writes(), vec![(0, 0)]);
    }

    #[test]
    fn repeated_speed_is_not_rewritten() {
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let tick = input(Some(profile_with("Balanced", true)), true);
        runtime.tick(&tick);
        runtime.tick(&tick);
        assert_eq!(fake.writes(), vec![(0, 50)]);
    }

    #[test]
    fn uniwill_quirk_rewrites_every_tick() {
        let mut fake = FakeFan::new(vec![70]);
        fake.every_tick = true;
        let fake = Arc::new(fake);
        let mut runtime = make_runtime(fake.clone(), false);
        let tick = input(Some(profile_with("Balanced", true)), true);
        runtime.tick(&tick);
        runtime.tick(&tick);
        assert_eq!(fake.writes(), vec![(0, 50), (0, 50)]);
    }

    #[test]
    fn custom_curve_ignores_named_limits() {
        let mut profile = profile_with("Custom", true);
        profile.fan.minimum_fanspeed = 80.0;
        profile.fan.maximum_fanspeed = 90.0;
        // Courbe droite : 0 % à 0 °C, 100 % à 100 °C.
        profile.fan.custom_fan_curve = FanCurve::new(
            "custom",
            vec![
                FanTableEntry::new(0.0, 0.0),
                FanTableEntry::new(100.0, 100.0),
            ],
            vec![
                FanTableEntry::new(0.0, 0.0),
                FanTableEntry::new(100.0, 100.0),
            ],
        );
        let fake = Arc::new(FakeFan::new(vec![50]));
        let mut runtime = make_runtime(fake.clone(), false);
        runtime.tick(&input(Some(profile), true));
        // Les bornes du profil sont ignorées pour une courbe personnalisée.
        assert_eq!(fake.writes(), vec![(0, 50)]);
    }

    #[test]
    fn user_fan_table_overrides_builtin() {
        let mut tables = FanTableSet::default();
        tables.upsert(FanCurve::new(
            "Quiet",
            vec![
                FanTableEntry::new(0.0, 10.0),
                FanTableEntry::new(100.0, 10.0),
            ],
            vec![
                FanTableEntry::new(0.0, 10.0),
                FanTableEntry::new(100.0, 10.0),
            ],
        ));
        let fake = Arc::new(FakeFan::new(vec![70]));
        let mut runtime = make_runtime(fake.clone(), false);
        let mut tick = input(Some(profile_with("Quiet", true)), true);
        tick.fan_tables = tables;
        runtime.tick(&tick);
        assert_eq!(fake.writes(), vec![(0, 10)]);
    }

    #[test]
    fn unknown_curve_falls_back_to_balanced() {
        let curve = resolve_curve(
            &profile_with("DoesNotExist", true).fan,
            &FanTableSet::default(),
            CoreFanTarget::Cpu,
        );
        let balanced = builtin_curve("Balanced").expect("balanced");
        assert_eq!(curve.table_cpu, balanced.table_cpu);
    }

    #[test]
    fn builtin_curves_never_lower_than_original_steps() {
        // Aux points de rupture, l'interpolation doit rendre exactement la
        // valeur d'origine.
        let silent = builtin_curve("Silent").expect("silent");
        assert_eq!(silent.speed_at(CoreFanTarget::Cpu, 61.0), Some(20.0));
        assert_eq!(silent.speed_at(CoreFanTarget::Cpu, 60.0), Some(0.0));
        assert_eq!(silent.speed_at(CoreFanTarget::Cpu, 40.0), Some(0.0));
        assert_eq!(silent.speed_at(CoreFanTarget::Cpu, 100.0), Some(100.0));
        let balanced = builtin_curve("Balanced").expect("balanced");
        assert_eq!(balanced.speed_at(CoreFanTarget::Cpu, 70.0), Some(50.0));
        assert_eq!(balanced.speed_at(CoreFanTarget::Cpu, 45.0), Some(0.0));
    }

    #[test]
    fn hw_limits_follow_original() {
        assert_eq!(apply_hw_limits(9, 20, true), 0);
        assert_eq!(apply_hw_limits(10, 20, true), 20);
        assert_eq!(apply_hw_limits(19, 20, false), 20);
        assert_eq!(apply_hw_limits(30, 20, true), 30);
        assert_eq!(apply_hw_limits(30, 0, true), 30);
    }

    #[test]
    fn falling_speed_is_limited_above_threshold() {
        assert_eq!(limit_falling_speed(30, Some(50)), 48);
        assert_eq!(limit_falling_speed(48, Some(50)), 48);
        assert_eq!(limit_falling_speed(49, Some(50)), 49);
        // Sous le seuil, la chute est libre (arrêt possible).
        assert_eq!(limit_falling_speed(0, Some(20)), 0);
        assert_eq!(limit_falling_speed(30, None), 30);
    }

    #[test]
    fn critical_temperature_floor_is_applied() {
        assert_eq!(critical_temperature_floor(95, 10), 40);
        assert_eq!(critical_temperature_floor(85, 10), 30);
        assert_eq!(critical_temperature_floor(70, 10), 10);
        assert_eq!(critical_temperature_floor(95, 60), 60);
    }

    #[test]
    fn fan_data_serializes_camel_case() {
        let data = FanData {
            available: true,
            backend: Some("tuxi".to_owned()),
            writable: true,
            control_enabled: true,
            mode: FanMode::Manual,
            fans: vec![FanReading {
                index: 0,
                label: "cpu".to_owned(),
                target: "cpu".to_owned(),
                temp: Some(45),
                speed: Some(30),
                applied: Some(32),
            }],
        };
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"controlEnabled\":true"));
        assert!(json.contains("\"mode\":\"manual\""));
        assert!(json.contains("\"applied\":32"));
        assert!(json.contains("\"fans\":["));
    }

    #[test]
    fn unavailable_data_is_stable() {
        let data = FanData::unavailable();
        assert!(!data.available);
        assert_eq!(data.mode, FanMode::Off);
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"available\":false"));
    }

    #[tokio::test]
    async fn suspend_returns_fans_to_auto_and_releases_manual() {
        let fake = Arc::new(FakeFan::new(vec![50]));
        let worker = FanWorker::for_test(fake.clone(), false);
        worker.suspend().await;
        assert_eq!(fake.auto_calls(), 1, "un seul retour auto final");
        assert_eq!(*fake.manual.lock().expect("manual"), Some(false));
    }

    #[tokio::test]
    async fn suspend_aborts_loop_before_auto() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&dropped);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _guard = DropFlag(flag);
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("tâche démarrée");
        let fake = Arc::new(FakeFan::new(vec![50]));
        let worker = FanWorker {
            handle: Some(handle),
            stop: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
            control: Some(fake.clone()),
            runtime: None,
            dry_run: false,
        };
        worker.suspend().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(dropped.load(Ordering::SeqCst), "boucle non abortée");
        assert_eq!(fake.auto_calls(), 1);
    }
}
