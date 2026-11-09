// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Daemon privilégié `tccd`.
//!
//! Jalon 1 : chargement de la configuration, watchers PPD/UPower/sysfs, moteur
//! de bascule et service D-Bus de lecture. L'application CPU/backlight est
//! branchée ; les autres domaines matériels arrivent aux jalons suivants.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod apply;
mod config;
mod dbus;
mod fan;
mod hardware;
mod rate_limit;
mod runtime;
mod sensor_lease;
mod shutdown_timer;
mod sleep;
mod watchers;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tcc_core::{Profile, Settings};
use tcc_hw::sysfs::SysfsRoot;
use tokio::sync::{Notify, RwLock, broadcast};

use crate::apply::ApplyReport;
use crate::fan::{FanData, FanWorker};
use crate::hardware::{HardwareInputs, HardwareStatus, HardwareWorkers};
use crate::runtime::{Inputs, RuntimeState};
use crate::sensor_lease::SensorLease;

const STATE_TICK: Duration = Duration::from_millis(250);
/// Budget d'une lecture PPD/UPower à la reprise (SL-04).
const RESUME_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Lecture bornée d'un service optionnel (PPD/UPower) avec repli `None`.
async fn bounded_read<T, F>(budget: Duration, event: &'static str, future: F) -> Option<T>
where
    F: std::future::Future<Output = Option<T>>,
{
    match tokio::time::timeout(budget, future).await {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(result = event, "lecture de service dépassée, repli");
            None
        }
    }
}

/// Coordination des workers matériels avec l'état du daemon.
struct HardwareRuntime {
    inputs: Arc<RwLock<HardwareInputs>>,
    /// `None` pendant la veille : les workers et leurs descripteurs sont
    /// libérés, puis recréés à la reprise.
    workers: Option<HardwareWorkers>,
    status: Arc<HardwareStatus>,
    root: SysfsRoot,
    dry_run: bool,
    sensors_enabled: Arc<AtomicBool>,
    last_state_revision: u64,
    last_ppd_active: bool,
    /// Révision monotone propre aux workers. Elle change aussi quand seul
    /// `ppd_active` change (même révision d'état), pour forcer la
    /// réconciliation de `platform_profile`.
    next_revision: u64,
    /// Forcé à la reprise : la prochaine observation doit réappliquer les
    /// entrées même si la révision d'état n'a pas changé.
    force_update: bool,
    /// Avertissement `ppd_override=true` déjà journalisé pour l'état courant.
    ppd_override_logged: bool,
}

/// Construit les entrées matérielles en propageant `Settings.ppd_override`.
///
/// Source unique : le flag persistant `Settings.ppd_override` est le
/// consentement explicite (D4). `tccd` doit le relire à chaque rechargement
/// pour rester cohérent avec `Admin.Hw.SetOdmProfileOverride`.
fn hardware_inputs(
    settings: Settings,
    profile: Option<Profile>,
    ppd_active: bool,
    revision: u64,
) -> HardwareInputs {
    let ppd_override = settings.ppd_override;
    HardwareInputs::new(settings, profile, ppd_active, revision).with_ppd_override(ppd_override)
}

impl HardwareRuntime {
    fn needs_update(&self, state_revision: u64, ppd_active: bool) -> bool {
        self.force_update
            || state_revision != self.last_state_revision
            || ppd_active != self.last_ppd_active
    }

    /// Met à jour les entrées des workers et les réveille si nécessaire.
    async fn apply_values(
        &mut self,
        state_revision: u64,
        settings: Settings,
        profile: Option<Profile>,
        ppd_active: bool,
    ) {
        if !self.needs_update(state_revision, ppd_active) {
            return;
        }
        self.write_inputs(state_revision, settings, profile, ppd_active)
            .await;
    }

    /// Écrit les entrées et réveille les workers s'ils sont actifs.
    async fn write_inputs(
        &mut self,
        state_revision: u64,
        settings: Settings,
        profile: Option<Profile>,
        ppd_active: bool,
    ) {
        self.next_revision = self.next_revision.saturating_add(1);
        let ppd_override = settings.ppd_override;
        if ppd_override && ppd_active && !self.ppd_override_logged {
            self.ppd_override_logged = true;
            tracing::warn!(
                "ppd_override=true : TCC peut écrire platform_profile malgré power-profiles-daemon"
            );
        } else if !ppd_override {
            self.ppd_override_logged = false;
        }
        let inputs = hardware_inputs(settings, profile, ppd_active, self.next_revision);
        *self.inputs.write().await = inputs;
        self.last_state_revision = state_revision;
        self.last_ppd_active = ppd_active;
        self.force_update = false;
        if let Some(workers) = &self.workers {
            workers.notify();
        }
    }

    /// Force une réconciliation complète à la prochaine observation.
    async fn refresh_after_resume(
        &mut self,
        state_revision: u64,
        settings: Settings,
        profile: Option<Profile>,
        ppd_active: bool,
    ) {
        self.write_inputs(state_revision, settings, profile, ppd_active)
            .await;
        self.force_update = true;
    }

    /// Arrête les workers matériels (veille ou arrêt).
    ///
    /// `take` + `abort_all` sont synchrones : même si le budget de veille
    /// annule ce future, aucune tâche worker ne survit pour cohabiter avec la
    /// reprise (SL-01).
    async fn pause(&mut self) {
        if let Some(mut workers) = self.workers.take() {
            workers.abort_all();
            workers.shutdown().await;
        }
    }

    /// Recrée les workers après la reprise.
    async fn resume(&mut self) {
        if self.workers.is_some() {
            return;
        }
        self.workers = Some(
            HardwareWorkers::start(
                Arc::clone(&self.inputs),
                Arc::clone(&self.status),
                self.root.clone(),
                self.dry_run,
                Arc::clone(&self.sensors_enabled),
            )
            .await,
        );
        self.force_update = true;
    }

    async fn shutdown(mut self) {
        self.pause().await;
    }
}

/// Effets de bord de la veille branchés sur les composants du daemon.
struct DaemonSleepHooks<'a> {
    hardware: &'a mut HardwareRuntime,
    fan: &'a mut Option<FanWorker>,
    state: Arc<RwLock<RuntimeState>>,
    inputs: Arc<RwLock<Inputs>>,
    fan_status: Arc<std::sync::RwLock<FanData>>,
    sensor_lease: Arc<SensorLease>,
    root: SysfsRoot,
    dry_run: bool,
}

#[async_trait::async_trait]
impl sleep::SleepHooks for DaemonSleepHooks<'_> {
    async fn suspend(&mut self) {
        let fan = self.fan.take();
        let hardware = &mut *self.hardware;
        // Budget global appliqué par `sleep::handle_event` (3 s). Les deux
        // arrêts sont concurrents ; les ventilateurs repassent en auto même si
        // l'attente de la boucle est interrompue.
        tokio::join!(
            async {
                if let Some(fan) = fan {
                    fan.suspend().await;
                }
            },
            async {
                hardware.pause().await;
            }
        );
    }

    async fn resume(&mut self) {
        // Nouvelle session de capteurs : désactivée jusqu'à une demande
        // explicite de la GUI.
        self.sensor_lease.disarm();

        // Redémarrage critique en premier : les lecteurs PPD/UPower peuvent
        // être lents ou figés, la boucle principale ne doit pas attendre plus
        // que leur budget (SL-04).
        self.hardware.resume().await;
        if self.fan.is_none() {
            *self.fan = Some(
                FanWorker::start(
                    self.state.clone(),
                    self.fan_status.clone(),
                    self.root.clone(),
                    self.dry_run,
                )
                .await,
            );
        }

        let previous = self.inputs.read().await.clone();
        let ppd =
            match tokio::time::timeout(RESUME_READ_TIMEOUT, watchers::read_ppd_initial()).await {
                Ok(value) => value,
                Err(_) => {
                    tracing::warn!(
                        result = "ppd_timeout",
                        "reprise : PPD lent, conservation de la dernière valeur"
                    );
                    previous.ppd
                }
            };
        let power =
            match tokio::time::timeout(RESUME_READ_TIMEOUT, watchers::read_power_initial()).await {
                Ok(Some(power)) => power,
                Ok(None) => Inputs::from_sysfs(&self.root).power,
                Err(_) => {
                    tracing::warn!(
                        result = "upower_timeout",
                        "reprise : UPower lent, conservation de la dernière valeur"
                    );
                    previous.power
                }
            };
        let current = {
            let mut guard = self.inputs.write().await;
            guard.ppd = ppd;
            guard.power = power;
            guard.clone()
        };

        let (profile, settings, revision) = {
            let state = self.state.read().await;
            (
                state.active_profile().cloned(),
                state.settings.clone(),
                state.revision,
            )
        };
        self.hardware
            .refresh_after_resume(
                revision,
                settings.clone(),
                profile.clone(),
                current.ppd.is_some(),
            )
            .await;

        if let Some(profile) = &profile {
            let report = apply::apply_profile(
                profile,
                &settings,
                current.ppd.is_some(),
                &self.root,
                self.dry_run,
            );
            log_report(&report);
        }
    }
}

#[derive(Debug)]
struct Args {
    check: bool,
    help: bool,
    version: bool,
    /// Mode développement : bus de session, dry-run, non root.
    dev_session: bool,
    /// Dry-run root sur le bus système : lectures et D-Bus réels, aucune
    /// écriture matérielle. Peut être combiné avec `--start`.
    dry_run: bool,
    /// Nom D-Bus alternatif pour cohabiter avec l'ancien TCC pendant le
    /// développement.
    bus_name: String,
}

impl Args {
    /// Vrai si le daemon doit s'interdire toute écriture matérielle.
    ///
    /// `--check` et `--dev-session` impliquent déjà le dry-run ; `--dry-run`
    /// s'y ajoute sans forcer le bus de session ni retirer l'exigence root.
    fn effective_dry_run(&self) -> bool {
        self.check || self.dev_session || self.dry_run
    }
}

impl Default for Args {
    fn default() -> Self {
        Self {
            check: false,
            help: false,
            version: false,
            dev_session: false,
            dry_run: false,
            bus_name: tcc_ipc::DBUS_SERVICE.to_owned(),
        }
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.help {
        println!(
            "tccd {}\n\n\
             USAGE:\n    tccd [--start|--check|--version|--help] [--bus-name <nom>] [--dev-session] [--dry-run]\n\n\
             OPTIONS:\n    --check            charge la configuration, affiche l'état et quitte\n    --bus-name <nom>   nom D-Bus alternatif (défaut : com.tuxedocomputers.tccd)\n    --dev-session      bus de session + dry-run, sans root (développement)\n    --dry-run          démarre le service sans écriture matérielle (root, bus système)\n    --version          affiche la version\n    --help             affiche cette aide\n\n\
             Sans option, démarre le daemon (root requis).",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if args.version {
        println!("tccd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("construction du runtime tokio")?;
    runtime.block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    let started = Instant::now();
    let config = config::load().context("chargement de la configuration")?;
    for warning in &config.warnings {
        tracing::warn!(warning = %warning, "configuration");
    }

    let root = SysfsRoot::from_env_or_system();
    let mut inputs = Inputs::from_sysfs(&root);
    if let Some(ppd) = bounded_read(
        RESUME_READ_TIMEOUT,
        "ppd_timeout_startup",
        watchers::read_ppd_initial(),
    )
    .await
    {
        inputs.ppd = Some(ppd);
    }

    // SL-07 : un redémarrage pendant la veille ne doit pas écrire CPU,
    // backlight ni démarrer les workers avant le signal de reprise.
    let initially_suspended = sleep::read_preparing_for_sleep().await.unwrap_or(false);
    if initially_suspended {
        tracing::warn!(
            "logind indique une préparation de veille au démarrage :              application et workers différés jusqu'à la reprise"
        );
    }

    let mut state = RuntimeState::new(&config);
    let dry_run = args.effective_dry_run();
    if args.dev_session {
        tracing::warn!("mode développement : bus de session, aucune écriture matérielle");
    } else if args.dry_run {
        tracing::warn!(
            "dry-run: aucune écriture matérielle ; bus système et nom normal (root requis)"
        );
    }
    if let Some(resolved) = state.observe(&inputs, Duration::ZERO) {
        tracing::info!(
            profile = %resolved.profile_id,
            source = ?resolved.source,
            ppd = ?inputs.ppd,
            power = %inputs.power,
            "profil initial"
        );
        if !initially_suspended {
            if let Some(profile) = state.active_profile().cloned() {
                let report = apply::apply_profile(
                    &profile,
                    &state.settings,
                    inputs.ppd.is_some(),
                    &root,
                    dry_run,
                );
                log_report(&report);
            }
        }
    }

    if args.check {
        let snapshot = state.snapshot();
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).context("sérialisation de l'état")?
        );
        return Ok(());
    }

    if !is_root() && !args.dev_session {
        anyhow::bail!(
            "tccd doit être lancé en root ; --dry-run exige aussi root. Utilisez --check ou --dev-session pour un diagnostic non privilégié"
        );
    }

    if !args.dev_session {
        config::persist_defaults_if_needed(&config)
            .context("persistance des valeurs par défaut")?;
    }

    let initial_revision = state.revision;
    let initial_ppd_active = inputs.ppd.is_some();
    let initial_ppd_override = state.settings.ppd_override;
    if initial_ppd_override && initial_ppd_active {
        tracing::warn!(
            "ppd_override=true : TCC peut écrire platform_profile malgré power-profiles-daemon"
        );
    }
    let hw_inputs = Arc::new(RwLock::new(hardware_inputs(
        state.settings.clone(),
        state.active_profile().cloned(),
        initial_ppd_active,
        initial_revision,
    )));
    let hw_status = Arc::new(HardwareStatus::default());
    let sensor_lease = Arc::new(sensor_lease::SensorLease::new());
    let sensors_enabled = sensor_lease.enabled_flag();
    let sleeping = Arc::new(AtomicBool::new(initially_suspended));
    let mut shutdown_timer = shutdown_timer::ShutdownTimer::system();
    let shutdown_status = shutdown_timer.status_handle();

    let state = Arc::new(RwLock::new(state));
    let inputs = Arc::new(RwLock::new(inputs));
    let notify = Arc::new(Notify::new());
    let (state_tx, state_rx) = broadcast::channel(16);
    // Statut des ventilateurs : écrit par le worker, lu par D-Bus.
    let fan_status = Arc::new(std::sync::RwLock::new(FanData::unavailable()));

    let workers = if initially_suspended {
        None
    } else {
        Some(
            HardwareWorkers::start(
                hw_inputs.clone(),
                hw_status.clone(),
                root.clone(),
                dry_run,
                sensors_enabled.clone(),
            )
            .await,
        )
    };
    let mut hardware = HardwareRuntime {
        inputs: hw_inputs,
        workers,
        status: Arc::clone(&hw_status),
        root: root.clone(),
        dry_run,
        sensors_enabled: sensors_enabled.clone(),
        last_state_revision: initial_revision,
        last_ppd_active: initial_ppd_active,
        next_revision: initial_revision,
        force_update: false,
        ppd_override_logged: initial_ppd_override && initial_ppd_active,
    };

    let bus = if args.dev_session {
        dbus::BusType::Session
    } else {
        dbus::BusType::System
    };
    let (connection, mut reload_requests) = dbus::serve(
        state.clone(),
        notify.clone(),
        fan_status.clone(),
        if args.dev_session {
            // Mode développement : session + dry-run, authorizer permissif.
            dbus::HardwareBridge::dev_session(
                hw_status,
                Arc::clone(&sensor_lease),
                root.clone(),
                Arc::clone(&sleeping),
                Arc::clone(&shutdown_status),
            )
        } else {
            dbus::HardwareBridge::production(
                hw_status,
                Arc::clone(&sensor_lease),
                root.clone(),
                dry_run,
                Arc::clone(&sleeping),
                Arc::clone(&shutdown_status),
            )
        },
        &args.bus_name,
        bus,
    )
    .await
    .context("enregistrement du service D-Bus")?;
    dbus::spawn_state_forwarder(&connection, state_rx);
    let mut connection_lost = dbus::watch_connection(&connection);

    tokio::spawn(watchers::watch_ppd(inputs.clone(), notify.clone()));
    tokio::spawn(watchers::watch_upower(inputs.clone(), notify.clone()));
    tokio::spawn(watchers::watch_sysfs_power(inputs.clone(), notify.clone()));

    // Worker dédié : il ne dépend pas de la boucle de bascule et ne la bloque
    // jamais (les accès matériels passent par `spawn_blocking`).
    let mut fan_worker = if initially_suspended {
        None
    } else {
        Some(FanWorker::start(state.clone(), fan_status.clone(), root.clone(), dry_run).await)
    };

    // Veille/reprise : signal logind poussé dans la boucle principale. Aucun
    // état matériel n'est conservé dans le watcher ; les événements sont
    // sérialisés avec le reload et le tick d'état. Le superviseur relance le
    // watcher s'il meurt (SL-05).
    let (sleep_tx, mut sleep_rx) = tokio::sync::mpsc::channel(8);
    sleep::spawn_logind_supervisor(sleep_tx);
    let mut sleep_machine = if initially_suspended {
        sleep::SleepStateMachine::suspended()
    } else {
        sleep::SleepStateMachine::new()
    };
    // Inhibitor pris dès le démarrage : il retarde la première veille le
    // temps du handoff, puis est relâché et ré-acquis après reprise.
    let mut sleep_inhibitor = if initially_suspended {
        None
    } else {
        sleep::acquire_sleep_inhibitor().await
    };
    let mut deferred_reload = sleep::DeferredReload::default();
    let mut sleep_watch_open = true;
    reconcile_shutdown_timer(&state, &mut shutdown_timer, sleep_machine.is_suspended()).await;

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("handler SIGTERM")?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("handler SIGINT")?;
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .context("handler SIGHUP")?;

    let mut tick = tokio::time::interval(STATE_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tracing::info!("tccd démarré");
    let mut connection_lost_flag = false;
    loop {
        tokio::select! {
            _ = notify.notified() => {
                // Pendant la veille, ne pas écrire CPU/backlight : le profil
                // sera réappliqué intégralement à la reprise.
                if !sleep_machine.is_suspended() {
                    observe_and_apply(
                        &state,
                        &inputs,
                        &root,
                        dry_run,
                        started,
                        &state_tx,
                        &mut hardware,
                    )
                    .await;
                    reconcile_shutdown_timer(
                        &state,
                        &mut shutdown_timer,
                        sleep_machine.is_suspended(),
                    )
                    .await;
                }
            }
            _ = tick.tick() => {
                if !sleep_machine.is_suspended() {
                    observe_and_apply(
                        &state,
                        &inputs,
                        &root,
                        dry_run,
                        started,
                        &state_tx,
                        &mut hardware,
                    )
                    .await;
                }
            }
            _ = sighup.recv() => {
                if sleep_machine.is_suspended() {
                    deferred_reload.defer();
                    tracing::warn!(
                        result = "deferred",
                        pending = deferred_reload.is_pending(),
                        "SIGHUP différé pendant la veille"
                    );
                } else {
                    match reload(
                        &state,
                        &inputs,
                        &root,
                        dry_run,
                        &state_tx,
                        &mut hardware,
                    )
                    .await
                    {
                        Ok(report) if report.is_ok() => notify.notify_one(),
                        Ok(report) => {
                            tracing::warn!(
                                warnings = report.warnings.len(),
                                "rechargement SIGHUP avec avertissements"
                            );
                            notify.notify_one();
                        }
                        Err(error) => tracing::error!(%error, "rechargement SIGHUP échoué"),
                    }
                    reconcile_shutdown_timer(
                        &state,
                        &mut shutdown_timer,
                        sleep_machine.is_suspended(),
                    )
                    .await;
                }
            }
            Some(request) = reload_requests.recv() => {
                if sleep_machine.is_suspended() {
                    deferred_reload.defer();
                    let _ = request.reply.send(
                        tcc_ipc::AdminReloadReport::from_warnings(vec![
                            "rechargement différé : veille en cours".to_owned(),
                        ]),
                    );
                    tracing::warn!(
                        result = "deferred",
                        pending = deferred_reload.is_pending(),
                        "Control1.ReloadConfig différé pendant la veille"
                    );
                } else {
                    // `Control1` est root-only et sérialisé côté interface ; la
                    // boucle principale applique chaque rechargement puis rend
                    // son rapport. Un échec devient un rapport honnête
                    // (`ok=false`) pour déclencher le rollback de l'admin.
                    let report = match reload(
                        &state,
                        &inputs,
                        &root,
                        dry_run,
                        &state_tx,
                        &mut hardware,
                    )
                    .await
                    {
                        Ok(report) => report,
                        Err(error) => {
                            tracing::error!(%error, "rechargement Control1 échoué");
                            tcc_ipc::AdminReloadReport::from_warnings(vec![
                                "rechargement échoué".to_owned(),
                            ])
                        }
                    };
                    let _ = request.reply.send(report);
                    notify.notify_one();
                    reconcile_shutdown_timer(
                        &state,
                        &mut shutdown_timer,
                        sleep_machine.is_suspended(),
                    )
                    .await;
                }
            }
            maybe_start = sleep_rx.recv(), if sleep_watch_open => {
                let Some(start) = maybe_start else {
                    sleep_watch_open = false;
                    tracing::warn!(
                        result = "closed",
                        "canal logind fermé : gestion de veille désactivée"
                    );
                    continue;
                };
                let was_suspended = sleep_machine.is_suspended();
                if start {
                    sleeping.store(true, std::sync::atomic::Ordering::SeqCst);
                } else {
                    sleeping.store(false, std::sync::atomic::Ordering::SeqCst);
                }
                let mut hooks = DaemonSleepHooks {
                    hardware: &mut hardware,
                    fan: &mut fan_worker,
                    state: state.clone(),
                    inputs: inputs.clone(),
                    fan_status: fan_status.clone(),
                    sensor_lease: Arc::clone(&sensor_lease),
                    root: root.clone(),
                    dry_run,
                };
                sleep::handle_event(&mut sleep_machine, &mut hooks, start).await;
                if start && sleep_machine.phase() == sleep::SleepPhase::Suspended {
                    // Handoff terminé : laisser logind suspendre.
                    if sleep_inhibitor.take().is_some() {
                        tracing::debug!("logind : inhibitor sleep/delay libéré après handoff");
                    }
                } else if !start && was_suspended && sleep_machine.phase() == sleep::SleepPhase::Awake {
                    // Reprise terminée : ré-armer l'inhibitor pour la
                    // prochaine veille.
                    sleep_inhibitor = sleep::acquire_sleep_inhibitor().await;
                }
                if !start && sleep_machine.phase() == sleep::SleepPhase::Awake {
                    reconcile_shutdown_timer(&state, &mut shutdown_timer, false).await;
                }
                if !start
                    && sleep_machine.phase() == sleep::SleepPhase::Awake
                    && deferred_reload.take()
                {
                    match reload(
                        &state,
                        &inputs,
                        &root,
                        dry_run,
                        &state_tx,
                        &mut hardware,
                    )
                    .await
                    {
                        Ok(report) if report.is_ok() => notify.notify_one(),
                        Ok(report) => {
                            tracing::warn!(
                                warnings = report.warnings.len(),
                                "rechargement différé avec avertissements"
                            );
                            notify.notify_one();
                        }
                        Err(error) => {
                            tracing::error!(%error, "rechargement différé après veille échoué");
                        }
                    }
                }
            }
            _ = &mut connection_lost => {
                tracing::error!(
                    "connexion D-Bus perdue ; arrêt pour laisser systemd redémarrer tccd"
                );
                connection_lost_flag = true;
                break;
            }
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
        }
    }

    if sleep_machine.is_suspended() {
        tracing::info!("tccd arrêté pendant la veille : aucun réveil synthétique");
    }
    // Rendre les ventilateurs au firmware avant de quitter. Si le daemon
    // s'arrête pendant la veille, les composants sont déjà arrêtés : les
    // options sont vides et l'arrêt est idempotent.
    hardware.shutdown().await;
    if let Some(fan) = fan_worker.take() {
        fan.shutdown().await;
    }
    if connection_lost_flag {
        anyhow::bail!("connexion D-Bus perdue");
    }
    tracing::info!("tccd arrêté");
    Ok(())
}

/// Réconcilie la minuterie d'extinction avec logind (hors veille).
async fn reconcile_shutdown_timer(
    state: &RwLock<RuntimeState>,
    timer: &mut shutdown_timer::ShutdownTimer,
    suspended: bool,
) {
    if suspended {
        return;
    }
    let value = state.read().await.settings.shutdown_time.clone();
    timer.reconcile(value.as_deref(), false, true).await;
}

async fn observe_and_apply(
    state: &RwLock<RuntimeState>,
    inputs: &RwLock<Inputs>,
    root: &SysfsRoot,
    dry_run: bool,
    started: Instant,
    snapshots: &broadcast::Sender<String>,
    hardware: &mut HardwareRuntime,
) {
    let current_inputs = inputs.read().await.clone();
    let ppd_active = current_inputs.ppd.is_some();
    let (resolved, profile, settings, snapshot, hw_values) = {
        let mut guard = state.write().await;
        let resolved = guard.observe(&current_inputs, started.elapsed());
        let hw_values = hardware.needs_update(guard.revision, ppd_active).then(|| {
            (
                guard.revision,
                guard.settings.clone(),
                guard.active_profile().cloned(),
            )
        });
        if resolved.is_some() {
            (
                resolved,
                guard.active_profile().cloned(),
                Some(guard.settings.clone()),
                serde_json::to_string(&guard.snapshot()).ok(),
                hw_values,
            )
        } else {
            (None, None, None, None, hw_values)
        }
    };

    // Même sans changement de profil (override manuel, réglages), les
    // workers matériels doivent voir la nouvelle révision.
    if let Some((revision, hw_settings, hw_profile)) = hw_values {
        hardware
            .apply_values(revision, hw_settings, hw_profile, ppd_active)
            .await;
    }

    let (Some(resolved), Some(profile), Some(settings)) = (resolved, profile, settings) else {
        return;
    };

    if let Some(snapshot) = snapshot {
        let _ = snapshots.send(snapshot);
    }

    tracing::info!(
        profile = %resolved.profile_id,
        name = %profile.name,
        source = ?resolved.source,
        ppd = ?current_inputs.ppd,
        power = %current_inputs.power,
        "profil actif"
    );

    let report = apply::apply_profile(
        &profile,
        &settings,
        current_inputs.ppd.is_some(),
        root,
        dry_run,
    );
    log_report(&report);
}

async fn reload(
    state: &RwLock<RuntimeState>,
    inputs: &RwLock<Inputs>,
    root: &SysfsRoot,
    dry_run: bool,
    snapshots: &broadcast::Sender<String>,
    hardware: &mut HardwareRuntime,
) -> Result<tcc_ipc::AdminReloadReport> {
    let config = config::load().context("rechargement de la configuration")?;
    // Rapport pour `Control1` : uniquement des catégories, jamais de chemin
    // ni de contenu de fichier (AD-2/AD-6).
    let report = tcc_ipc::AdminReloadReport::from_warnings(config.report_warnings.clone());
    for warning in &config.warnings {
        tracing::warn!(warning = %warning, "configuration rechargée");
    }

    let current_inputs = inputs.read().await.clone();
    let ppd_active = current_inputs.ppd.is_some();
    let (profile, settings, snapshot, hw_values) = {
        let mut guard = state.write().await;
        guard.reload(&config);
        // Garantit que les workers voient le changement même si le profil
        // résolu est identique.
        guard.revision = guard.revision.saturating_add(1);
        // Force une observation immédiate : le resolver applique le profil
        // courant avec les nouvelles règles.
        let _ = guard.observe(&current_inputs, std::time::Duration::ZERO);
        let hw_values = hardware.needs_update(guard.revision, ppd_active).then(|| {
            (
                guard.revision,
                guard.settings.clone(),
                guard.active_profile().cloned(),
            )
        });
        (
            guard.active_profile().cloned(),
            guard.settings.clone(),
            serde_json::to_string(&guard.snapshot()).ok(),
            hw_values,
        )
    };

    if let Some((revision, hw_settings, hw_profile)) = hw_values {
        hardware
            .apply_values(revision, hw_settings, hw_profile, ppd_active)
            .await;
    }

    if let Some(snapshot) = snapshot {
        let _ = snapshots.send(snapshot);
    }

    if let (Some(profile), settings) = (profile, settings) {
        let apply_report = apply::apply_profile(
            &profile,
            &settings,
            current_inputs.ppd.is_some(),
            root,
            dry_run,
        );
        log_report(&apply_report);
    }
    tracing::info!("configuration rechargée");
    Ok(report)
}

fn log_report(report: &ApplyReport) {
    if report.is_ok() {
        tracing::info!(profile = %report.profile, "profil appliqué");
    } else {
        tracing::warn!(profile = %report.profile, "profil appliqué partiellement");
    }
    for note in &report.notes {
        tracing::info!(profile = %report.profile, note = %note, "application");
    }
    for error in &report.errors {
        tracing::warn!(profile = %report.profile, error = %error, "application partielle");
    }
}

fn parse_args() -> Result<Args> {
    parse_args_from(std::env::args().skip(1))
}

/// Analyse pure des arguments, sans lecture de l'environnement process.
///
/// Les tests appellent cette fonction avec des vecteurs explicites : ils ne
/// dépendent jamais des arguments ajoutés par le harnais de test (`--quiet`,
/// `--test-threads`, etc.).
fn parse_args_from<I>(raw: I) -> Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut args = Args::default();
    let mut iter = raw.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--check" => args.check = true,
            "--dry-run" => args.dry_run = true,
            "--help" | "-h" => args.help = true,
            "--version" | "-V" => args.version = true,
            "--start" => {}
            "--dev-session" => args.dev_session = true,
            "--bus-name" => {
                let value = iter.next().context("--bus-name attend un nom D-Bus")?;
                args.bus_name = validate_bus_name(&value)?;
            }
            other if other.starts_with("--bus-name=") => {
                args.bus_name = validate_bus_name(other.trim_start_matches("--bus-name="))?;
            }
            other => anyhow::bail!("argument inconnu: {other} (essayez --help)"),
        }
    }
    Ok(args)
}

/// Validation volontairement stricte : le nom vient de la ligne de commande
/// root et ne doit contenir que la grammaire D-Bus attendue.
fn validate_bus_name(name: &str) -> Result<String> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        anyhow::bail!("nom D-Bus invalide: {name:?}");
    }
    Ok(name.to_owned())
}

/// Vrai si l'UID effectif est root, sans `unsafe`/libc.
#[must_use]
pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(|value| value.parse::<u32>().ok())
        })
        .is_some_and(|uid| uid == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::SleepHooks;

    fn parse(values: &[&str]) -> Result<Args> {
        parse_args_from(
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn empty_arguments_use_defaults() {
        let args = parse(&[]).expect("arguments vides");
        assert!(!args.check);
        assert!(!args.help);
        assert!(!args.version);
        assert!(!args.dev_session);
        assert!(!args.dry_run);
        assert_eq!(args.bus_name, tcc_ipc::DBUS_SERVICE);
        assert!(!args.effective_dry_run());
    }

    #[test]
    fn parses_check_and_dry_run_separately() {
        let check = parse(&["--check"]).expect("check");
        assert!(check.check);
        assert!(!check.dry_run);

        let dry_run = parse(&["--dry-run"]).expect("dry-run");
        assert!(dry_run.dry_run);
        assert!(
            !dry_run.check,
            "--dry-run ne doit pas quitter comme --check"
        );
        assert!(!dry_run.dev_session);
        assert_eq!(dry_run.bus_name, tcc_ipc::DBUS_SERVICE);
        assert!(dry_run.effective_dry_run());

        let combined = parse(&["--check", "--dry-run"]).expect("combinaison");
        assert!(combined.check);
        assert!(combined.dry_run);

        let dev = parse(&["--dev-session"]).expect("dev-session");
        assert!(dev.dev_session);
        assert!(!dev.dry_run);
        assert!(dev.effective_dry_run());
    }

    #[test]
    fn dry_run_start_is_not_a_check_exit() {
        let args = parse(&["--start", "--dry-run"]).expect("dry-run start");
        assert!(args.dry_run);
        assert!(!args.check);
        assert!(args.effective_dry_run());

        // `--check` conserve la sortie immédiate, même combiné.
        let check = parse(&["--start", "--dry-run", "--check"]).expect("check");
        assert!(check.check);
    }

    #[test]
    fn parses_help_and_version_without_starting() {
        assert!(parse(&["--help"]).expect("help").help);
        assert!(parse(&["-h"]).expect("h").help);
        assert!(parse(&["--version"]).expect("version").version);
        assert!(parse(&["-V"]).expect("V").version);
    }

    #[test]
    fn start_is_a_compatible_noop() {
        let args = parse(&["--start"]).expect("start");
        assert!(!args.check && !args.help && !args.version && !args.dry_run);
    }

    #[test]
    fn parses_bus_name_in_both_forms() {
        let separate = parse(&["--bus-name", "com.example.Dev"]).expect("bus-name");
        assert_eq!(separate.bus_name, "com.example.Dev");
        let joined = parse(&["--bus-name=com.example.Dev"]).expect("bus-name=");
        assert_eq!(joined.bus_name, "com.example.Dev");
    }

    #[test]
    fn rejects_unknown_and_libtest_arguments() {
        assert!(parse(&["--force"]).is_err());
        assert!(parse(&["--quiet"]).is_err());
        assert!(parse(&["not-an-option"]).is_err());
    }

    #[test]
    fn rejects_bus_name_without_value_or_invalid() {
        assert!(parse(&["--bus-name"]).is_err());
        assert!(parse(&["--bus-name", "bad name"]).is_err());
        assert!(parse(&["--bus-name", ""]).is_err());
        assert!(parse(&["--bus-name", ".."]).is_err());
    }

    #[test]
    fn dev_session_bridge_is_only_wired_for_dev_session() {
        let source = include_str!("main.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert_eq!(
            production.matches("HardwareBridge::dev_session").count(),
            1,
            "le pont permissif ne doit être construit qu'à un seul endroit"
        );
        let index = production
            .find("HardwareBridge::dev_session")
            .expect("occurrence du pont dev-session");
        let context = &production[index.saturating_sub(250)..index];
        assert!(
            context.contains("args.dev_session"),
            "l'injection doit être conditionnée par --dev-session"
        );
        assert!(
            !production.contains("HardwareBridge::test_with_authorizer"),
            "le constructeur de test ne doit pas apparaître en production"
        );
    }

    #[tokio::test]
    async fn bounded_startup_read_does_not_block() {
        let started = Instant::now();
        let value: Option<u8> = bounded_read(
            Duration::from_millis(50),
            "test_timeout",
            std::future::pending(),
        )
        .await;
        assert_eq!(value, None);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn root_probe_does_not_panic() {
        let _ = is_root();
    }

    async fn test_hardware_runtime() -> HardwareRuntime {
        let root = SysfsRoot::new("/nonexistent-tcc-test");
        let inputs = Arc::new(RwLock::new(HardwareInputs::new(
            Settings::default(),
            None,
            false,
            0,
        )));
        let status = Arc::new(HardwareStatus::default());
        let sensors = Arc::new(AtomicBool::new(false));
        let workers = HardwareWorkers::start(
            Arc::clone(&inputs),
            Arc::clone(&status),
            root.clone(),
            true,
            Arc::clone(&sensors),
        )
        .await;
        HardwareRuntime {
            inputs,
            workers: Some(workers),
            status,
            root,
            dry_run: true,
            sensors_enabled: sensors,
            last_state_revision: 0,
            last_ppd_active: false,
            next_revision: 0,
            force_update: false,
            ppd_override_logged: false,
        }
    }

    #[tokio::test]
    async fn hardware_runtime_pause_resume_ten_cycles() {
        let mut hardware = test_hardware_runtime().await;
        for cycle in 0..10 {
            hardware.pause().await;
            assert!(hardware.workers.is_none(), "cycle {cycle}: workers arrêtés");
            hardware.resume().await;
            assert!(hardware.workers.is_some(), "cycle {cycle}: workers recréés");
            assert!(
                hardware.force_update,
                "cycle {cycle}: réconciliation forcée"
            );
            hardware
                .apply_values(0, Settings::default(), None, false)
                .await;
            assert!(!hardware.force_update);
        }
        hardware.shutdown().await;
    }

    fn test_runtime_state() -> Arc<RwLock<RuntimeState>> {
        let config = crate::config::LoadedConfig {
            store: tcc_core::ConfigStore::with_admin_roots("/tmp/state", "/tmp/admin"),
            settings: Settings::default(),
            profiles: tcc_core::default_profiles(),
            fan_tables: tcc_core::FanTableSet::default(),
            warnings: Vec::new(),
            report_warnings: Vec::new(),
        };
        Arc::new(RwLock::new(RuntimeState::new(&config)))
    }

    #[test]
    fn hardware_inputs_follow_ppd_override_setting() {
        let settings = Settings {
            ppd_override: true,
            ..Settings::default()
        };
        let enabled = hardware_inputs(settings, None, true, 1);
        assert_eq!(
            enabled.platform_profile_owner(),
            tcc_hw::platform_profile::ProfileOwnership::TccOwned,
            "ppd_override=true doit donner la propriété à TCC"
        );

        let disabled = hardware_inputs(Settings::default(), None, true, 1);
        assert_eq!(
            disabled.platform_profile_owner(),
            tcc_hw::platform_profile::ProfileOwnership::PpdOwned,
            "ppd_override=false doit laisser PPD propriétaire"
        );
    }

    #[tokio::test]
    async fn hardware_runtime_reloads_ppd_override_flag() {
        let inputs = Arc::new(RwLock::new(HardwareInputs::default()));
        let mut runtime = HardwareRuntime {
            inputs: Arc::clone(&inputs),
            workers: None,
            status: Arc::new(HardwareStatus::default()),
            root: SysfsRoot::new("/nonexistent-tcc-test"),
            dry_run: true,
            sensors_enabled: Arc::new(AtomicBool::new(false)),
            last_state_revision: 0,
            last_ppd_active: false,
            next_revision: 0,
            force_update: false,
            ppd_override_logged: false,
        };

        let settings = Settings {
            ppd_override: true,
            ..Settings::default()
        };
        runtime.write_inputs(1, settings, None, true).await;
        assert!(inputs.read().await.ppd_override);
        assert!(runtime.ppd_override_logged);

        runtime
            .write_inputs(2, Settings::default(), None, true)
            .await;
        assert!(!inputs.read().await.ppd_override);
        assert!(!runtime.ppd_override_logged);
    }

    #[tokio::test]
    async fn sleep_hooks_restart_components_and_reset_sensors() {
        let root = SysfsRoot::new("/nonexistent-tcc-test");
        let state = test_runtime_state();
        let inputs = Arc::new(RwLock::new(Inputs::from_sysfs(&root)));
        let fan_status = Arc::new(std::sync::RwLock::new(FanData::unavailable()));
        let sensor_lease = Arc::new(SensorLease::new());
        sensor_lease.arm(Instant::now());
        let mut hardware = test_hardware_runtime().await;
        hardware.pause().await;
        assert!(hardware.workers.is_none());
        let mut fan: Option<FanWorker> = None;

        let mut hooks = DaemonSleepHooks {
            hardware: &mut hardware,
            fan: &mut fan,
            state,
            inputs,
            fan_status,
            sensor_lease: Arc::clone(&sensor_lease),
            root,
            dry_run: true,
        };
        hooks.resume().await;

        assert!(hardware.workers.is_some(), "workers matériels recréés");
        assert!(fan.is_some(), "worker ventilateurs recréé");
        assert!(
            !sensor_lease.is_enabled(),
            "le bail capteurs doit être réinitialisé"
        );
        assert!(hardware.force_update, "profil à réappliquer");

        if let Some(fan) = fan.take() {
            fan.shutdown().await;
        }
        hardware.shutdown().await;
    }
}
