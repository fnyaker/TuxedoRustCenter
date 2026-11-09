// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Veille/reprise via `org.freedesktop.login1.Manager.PrepareForSleep`.
//!
//! # Flux
//!
//! - `true` : la machine va se mettre en veille. Le daemon arrête les
//!   workers et rend les ventilateurs au firmware, dans un budget borné
//!   ([`SUSPEND_BUDGET`]).
//! - `false` : reprise. Les workers sont recréés (les descripteurs peuvent
//!   avoir été perdus), le profil actif est réappliqué et le bail capteurs est
//!   réinitialisé.
//! - un `false` pendant `Preparing` signifie que la veille a été annulée : le
//!   daemon redémarre ce qu'il a arrêté.
//!
//! Les signaux dupliqués sont ignorés par la machine à états. Si logind est
//! absent, le watcher réessaie périodiquement et ne rend jamais le daemon
//! fatal : `tccd` continue sans gestion de veille.
//!
//! Les tests injectent [`SleepHooks`] et ne nécessitent ni logind ni bus
//! système.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;

/// Budget maximal accordé à l'arrêt avant veille.
pub const SUSPEND_BUDGET: Duration = Duration::from_secs(3);
/// Budget global de la reprise (workers + lectures PPD/UPower + profil).
pub const RESUME_BUDGET: Duration = Duration::from_secs(5);
/// Délai avant de retenter la connexion à logind.
const LOGIND_RECONNECT: Duration = Duration::from_secs(5);

/// Phase de la machine à états.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepPhase {
    /// Aucune veille en cours.
    Awake,
    /// Arrêt des composants en cours.
    Preparing,
    /// Composants arrêtés, machine en veille.
    Suspended,
    /// Redémarrage des composants en cours.
    Resuming,
}

/// Action décidée par la machine à états.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepAction {
    /// Arrêter les composants avant la veille.
    Suspend,
    /// Redémarrer les composants et réappliquer le profil.
    Resume,
    /// Signal dupliqué, spurious ou déjà en cours : ne rien faire.
    Ignore,
}

/// Machine à états pure, testable sans logind.
#[derive(Debug)]
pub struct SleepStateMachine {
    phase: SleepPhase,
}

impl SleepStateMachine {
    /// Crée une machine à l'état éveillé.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: SleepPhase::Awake,
        }
    }

    /// Crée une machine déjà suspendue (redémarrage de `tccd` pendant la
    /// veille, SL-07).
    #[must_use]
    pub const fn suspended() -> Self {
        Self {
            phase: SleepPhase::Suspended,
        }
    }

    /// Phase courante.
    #[must_use]
    pub const fn phase(&self) -> SleepPhase {
        self.phase
    }

    /// Vrai si la machine est en veille ou en train de s'endormir.
    #[must_use]
    pub const fn is_suspended(&self) -> bool {
        matches!(self.phase, SleepPhase::Preparing | SleepPhase::Suspended)
    }

    /// Traite `PrepareForSleep(start)` et décide de l'action.
    ///
    /// Règles :
    ///
    /// - `true` en `Awake` ⇒ `Suspend` ; dupliqué en `Preparing`/`Suspended`
    ///   ⇒ `Ignore` ;
    /// - `false` en `Suspended` ⇒ `Resume` ;
    /// - `false` en `Preparing` (veille annulée) ⇒ `Resume` ;
    /// - `false` en `Awake`/`Resuming`, ou `true` en `Resuming` ⇒ `Ignore`.
    pub fn on_prepare(&mut self, start: bool) -> SleepAction {
        match (self.phase, start) {
            (SleepPhase::Awake, true) => {
                self.phase = SleepPhase::Preparing;
                SleepAction::Suspend
            }
            (SleepPhase::Preparing | SleepPhase::Suspended, true) => SleepAction::Ignore,
            (SleepPhase::Suspended | SleepPhase::Preparing, false) => {
                self.phase = SleepPhase::Resuming;
                SleepAction::Resume
            }
            (SleepPhase::Awake | SleepPhase::Resuming, false) | (SleepPhase::Resuming, true) => {
                SleepAction::Ignore
            }
        }
    }

    /// Marque la fin de l'arrêt (succès ou budget dépassé).
    pub fn suspend_finished(&mut self) {
        if self.phase == SleepPhase::Preparing {
            self.phase = SleepPhase::Suspended;
        }
    }

    /// Marque la fin de la reprise.
    pub fn resume_finished(&mut self) {
        if self.phase == SleepPhase::Resuming {
            self.phase = SleepPhase::Awake;
        }
    }
}

impl Default for SleepStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// Demande de rechargement différée pendant la veille (SL-02).
#[derive(Debug, Default)]
pub struct DeferredReload {
    pending: bool,
}

impl DeferredReload {
    /// Mémorise une demande ; les demandes suivantes sont coalescées.
    pub fn defer(&mut self) {
        self.pending = true;
    }

    /// Vrai si une demande est en attente.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending
    }

    /// Consomme la demande ; une seule exécution, même après N demandes.
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

/// Effets de bord de la veille, injectables pour les tests.
#[async_trait]
pub trait SleepHooks: Send {
    /// Arrête les composants et rend le matériel au firmware.
    async fn suspend(&mut self);
    /// Recrée les composants et réapplique le profil actif.
    async fn resume(&mut self);
}

/// Traite un signal `PrepareForSleep` avec un budget d'arrêt borné.
pub async fn handle_event<H>(machine: &mut SleepStateMachine, hooks: &mut H, start: bool)
where
    H: SleepHooks + ?Sized,
{
    handle_event_with_budget(machine, hooks, start, SUSPEND_BUDGET, RESUME_BUDGET).await;
}

async fn handle_event_with_budget<H>(
    machine: &mut SleepStateMachine,
    hooks: &mut H,
    start: bool,
    suspend_budget: Duration,
    resume_budget: Duration,
) where
    H: SleepHooks + ?Sized,
{
    match machine.on_prepare(start) {
        SleepAction::Suspend => {
            tracing::info!("veille : arrêt des workers et retour ventilateurs en auto");
            if tokio::time::timeout(suspend_budget, hooks.suspend())
                .await
                .is_err()
            {
                tracing::warn!(
                    budget_ms = suspend_budget.as_millis() as u64,
                    "veille : budget d'arrêt dépassé"
                );
            }
            machine.suspend_finished();
            tracing::info!("veille : composants arrêtés");
        }
        SleepAction::Resume => {
            tracing::info!("reprise : redémarrage des workers et réapplication du profil");
            if tokio::time::timeout(resume_budget, hooks.resume())
                .await
                .is_err()
            {
                tracing::warn!(
                    budget_ms = resume_budget.as_millis() as u64,
                    result = "resume_timeout",
                    "reprise : budget dépassé, réconciliation au prochain tick"
                );
            }
            machine.resume_finished();
            tracing::info!("reprise : daemon réconcilié");
        }
        SleepAction::Ignore => {
            tracing::debug!(
                phase = ?machine.phase(),
                start,
                "PrepareForSleep ignoré (doublon ou état incohérent)"
            );
        }
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

    #[zbus(name = "Inhibit")]
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;
}

/// Logind est-il déjà en préparation de veille au démarrage (SL-07) ?
///
/// Retourne `None` si logind est absent ou ne répond pas dans le budget.
pub async fn read_preparing_for_sleep() -> Option<bool> {
    let connection = tokio::time::timeout(Duration::from_secs(1), zbus::Connection::system())
        .await
        .ok()?
        .ok()?;
    let proxy = tokio::time::timeout(Duration::from_secs(1), Login1ManagerProxy::new(&connection))
        .await
        .ok()?
        .ok()?;
    tokio::time::timeout(Duration::from_secs(1), proxy.preparing_for_sleep())
        .await
        .ok()?
        .ok()
}

/// Prend un inhibitor `sleep` en mode `delay` pour la durée du handoff.
///
/// Retourne `None` si logind/Inhibit est indisponible : le daemon continue en
/// repli signal-only (SL-03).
pub async fn acquire_sleep_inhibitor() -> Option<zbus::zvariant::OwnedFd> {
    let connection = tokio::time::timeout(Duration::from_secs(2), zbus::Connection::system())
        .await
        .ok()?
        .ok()?;
    acquire_inhibitor_on(&connection, "org.freedesktop.login1").await
}

async fn acquire_inhibitor_on(
    connection: &zbus::Connection,
    service: &str,
) -> Option<zbus::zvariant::OwnedFd> {
    let proxy = match tokio::time::timeout(Duration::from_secs(2), async {
        let builder = Login1ManagerProxy::builder(connection).destination(service.to_owned())?;
        builder.build().await
    })
    .await
    {
        Ok(Ok(proxy)) => proxy,
        Ok(Err(error)) => {
            tracing::warn!(%error, "logind : proxy Inhibit indisponible");
            return None;
        }
        Err(_) => {
            tracing::warn!("logind : proxy Inhibit en timeout");
            return None;
        }
    };
    match tokio::time::timeout(
        Duration::from_secs(2),
        proxy.inhibit("sleep", "tccd", "hardware handoff", "delay"),
    )
    .await
    {
        Ok(Ok(fd)) => {
            tracing::info!("logind : inhibitor sleep/delay pris pour le handoff");
            Some(fd)
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "logind : Inhibit refusé, repli signal-only");
            None
        }
        Err(_) => {
            tracing::warn!("logind : Inhibit en timeout, repli signal-only");
            None
        }
    }
}

/// Supervise le watcher logind : relance avec back-off si la tâche meurt.
pub fn spawn_logind_supervisor(events: mpsc::Sender<bool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            if events.is_closed() {
                return;
            }
            let handle = tokio::spawn(watch_logind(events.clone()));
            match handle.await {
                Ok(()) => {
                    if events.is_closed() {
                        return;
                    }
                    tracing::warn!("logind : watcher terminé, relance");
                }
                Err(error) => {
                    tracing::warn!(%error, "logind : watcher interrompu, relance");
                }
            }
            if events.is_closed() {
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    })
}

/// Observe `PrepareForSleep` sur le bus système.
///
/// - logind absent : un `warn` la première fois, puis `debug`, et nouvelle
///   tentative toutes les 5 s ;
/// - récepteur fermé (arrêt du daemon) : sortie immédiate.
pub async fn watch_logind(events: mpsc::Sender<bool>) {
    let mut unavailable_logged = false;
    loop {
        if events.is_closed() {
            return;
        }
        match run_logind(&events).await {
            Ok(()) => {
                if events.is_closed() {
                    return;
                }
                tracing::warn!("logind : flux PrepareForSleep terminé, reconnexion");
                unavailable_logged = false;
            }
            Err(error) => {
                if unavailable_logged {
                    tracing::debug!(%error, "logind toujours indisponible");
                } else {
                    tracing::warn!(
                        %error,
                        "logind indisponible : veille non gérée (nouvelle tentative dans 5 s)"
                    );
                    unavailable_logged = true;
                }
            }
        }
        if events.is_closed() {
            return;
        }
        tokio::time::sleep(LOGIND_RECONNECT).await;
    }
}

async fn run_logind(events: &mpsc::Sender<bool>) -> Result<(), zbus::Error> {
    let connection = zbus::Connection::system().await?;
    let proxy = Login1ManagerProxy::new(&connection).await?;
    let mut signals = proxy.receive_prepare_for_sleep().await?;
    tracing::info!("logind : PrepareForSleep surveillé");
    while let Some(signal) = signals.next().await {
        let args = signal.args()?;
        if events.send(args.start).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SleepAction, SleepHooks, SleepPhase, SleepStateMachine, handle_event};

    #[derive(Debug, Default)]
    struct RecordingHooks {
        events: Vec<&'static str>,
    }

    #[async_trait::async_trait]
    impl SleepHooks for RecordingHooks {
        async fn suspend(&mut self) {
            self.events.push("suspend");
        }

        async fn resume(&mut self) {
            self.events.push("resume");
        }
    }

    #[tokio::test]
    async fn suspend_then_resume_runs_hooks_once() {
        let mut machine = SleepStateMachine::new();
        let mut hooks = RecordingHooks::default();
        handle_event(&mut machine, &mut hooks, true).await;
        assert_eq!(machine.phase(), SleepPhase::Suspended);
        handle_event(&mut machine, &mut hooks, false).await;
        assert_eq!(machine.phase(), SleepPhase::Awake);
        assert_eq!(hooks.events, vec!["suspend", "resume"]);
    }

    #[tokio::test]
    async fn duplicate_suspend_is_ignored() {
        let mut machine = SleepStateMachine::new();
        let mut hooks = RecordingHooks::default();
        handle_event(&mut machine, &mut hooks, true).await;
        handle_event(&mut machine, &mut hooks, true).await;
        assert_eq!(hooks.events, vec!["suspend"]);
        assert_eq!(machine.phase(), SleepPhase::Suspended);
    }

    #[tokio::test]
    async fn cancelled_sleep_resumes() {
        let mut machine = SleepStateMachine::new();
        // `true` puis `false` avant la fin de l'arrêt : veille annulée.
        assert_eq!(machine.on_prepare(true), SleepAction::Suspend);
        assert_eq!(machine.on_prepare(false), SleepAction::Resume);
        assert_eq!(machine.phase(), SleepPhase::Resuming);
        machine.suspend_finished(); // ignoré : on est déjà en reprise
        machine.resume_finished();
        assert_eq!(machine.phase(), SleepPhase::Awake);
    }

    #[tokio::test]
    async fn spurious_and_duplicate_resume_are_ignored() {
        let mut machine = SleepStateMachine::new();
        let mut hooks = RecordingHooks::default();
        handle_event(&mut machine, &mut hooks, false).await;
        assert!(hooks.events.is_empty());
        handle_event(&mut machine, &mut hooks, true).await;
        handle_event(&mut machine, &mut hooks, false).await;
        handle_event(&mut machine, &mut hooks, false).await;
        assert_eq!(hooks.events, vec!["suspend", "resume"]);
    }

    #[tokio::test]
    async fn slow_resume_is_bounded() {
        struct SlowResume;
        #[async_trait::async_trait]
        impl SleepHooks for SlowResume {
            async fn suspend(&mut self) {}
            async fn resume(&mut self) {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
        let mut machine = SleepStateMachine::new();
        let mut hooks = SlowResume;
        super::handle_event_with_budget(
            &mut machine,
            &mut hooks,
            true,
            Duration::from_millis(50),
            Duration::from_millis(100),
        )
        .await;
        assert_eq!(machine.phase(), SleepPhase::Suspended);
        let started = std::time::Instant::now();
        super::handle_event_with_budget(
            &mut machine,
            &mut hooks,
            false,
            Duration::from_millis(50),
            Duration::from_millis(100),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(machine.phase(), SleepPhase::Awake);
    }

    #[derive(Debug)]
    struct FakeLogind {
        fail: bool,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl FakeLogind {
        fn inhibit(
            &self,
            _what: &str,
            _who: &str,
            _why: &str,
            _mode: &str,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                return Err(zbus::fdo::Error::Failed(
                    "inhibitor refusé (test)".to_owned(),
                ));
            }
            let file = std::fs::File::open("/dev/null")
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
            let fd: std::os::fd::OwnedFd = file.into();
            Ok(zbus::zvariant::OwnedFd::from(fd))
        }
    }

    async fn start_fake_logind(
        fail: bool,
    ) -> Option<(zbus::Connection, String, std::sync::Arc<FakeLogind>)> {
        let connection = zbus::Connection::session().await.ok()?;
        let name = format!(
            "com.example.LogindTest{}_{}",
            std::process::id(),
            NEXT_LOGIND.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        connection.request_name(name.clone()).await.ok()?;
        let fake = std::sync::Arc::new(FakeLogind {
            fail,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        // zbus sert une valeur ; un clone léger n'est pas possible, on sert
        // l'Arc directement (l'interface est implémentée pour FakeLogind).
        connection
            .object_server()
            .at("/org/freedesktop/login1", FakeLogindHandle(fake.clone()))
            .await
            .ok()?;
        Some((connection, name, fake))
    }

    static NEXT_LOGIND: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    #[derive(Debug)]
    struct FakeLogindHandle(std::sync::Arc<FakeLogind>);

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl FakeLogindHandle {
        fn inhibit(
            &self,
            what: &str,
            who: &str,
            why: &str,
            mode: &str,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
            self.0.inhibit(what, who, why, mode)
        }
    }

    #[tokio::test]
    async fn inhibitor_fd_is_acquired_and_failure_falls_back() {
        let Some((_service, name, fake)) = start_fake_logind(false).await else {
            eprintln!("bus de session absent : test inhibitor ignoré");
            return;
        };
        let connection = zbus::Connection::session().await.expect("session");
        let fd = super::acquire_inhibitor_on(&connection, &name).await;
        assert!(fd.is_some(), "inhibitor attendu");
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(fd);

        let Some((_service2, name2, fake2)) = start_fake_logind(true).await else {
            return;
        };
        assert!(
            super::acquire_inhibitor_on(&connection, &name2)
                .await
                .is_none()
        );
        assert_eq!(fake2.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn logind_supervisor_stops_when_receiver_is_closed() {
        let (events, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        let supervisor = super::spawn_logind_supervisor(events);
        tokio::time::timeout(Duration::from_secs(2), supervisor)
            .await
            .expect("le superviseur doit sortir")
            .expect("superviseur");
    }

    #[test]
    fn startup_suspended_resumes_on_false() {
        let mut machine = SleepStateMachine::suspended();
        assert!(machine.is_suspended());
        assert_eq!(machine.on_prepare(true), SleepAction::Ignore);
        assert_eq!(machine.on_prepare(false), SleepAction::Resume);
        machine.resume_finished();
        assert_eq!(machine.phase(), SleepPhase::Awake);
    }

    #[test]
    fn deferred_reload_coalesces_requests() {
        let mut deferred = super::DeferredReload::default();
        assert!(!deferred.is_pending());
        deferred.defer();
        deferred.defer();
        assert!(deferred.is_pending());
        assert!(deferred.take());
        assert!(!deferred.take());
    }

    #[tokio::test]
    async fn ten_cycles_leave_machine_awake_without_leak() {
        let mut machine = SleepStateMachine::new();
        let mut hooks = RecordingHooks::default();
        for _ in 0..10 {
            handle_event(&mut machine, &mut hooks, true).await;
            handle_event(&mut machine, &mut hooks, false).await;
        }
        assert_eq!(machine.phase(), SleepPhase::Awake);
        assert_eq!(
            hooks
                .events
                .iter()
                .filter(|event| **event == "suspend")
                .count(),
            10
        );
        assert_eq!(
            hooks
                .events
                .iter()
                .filter(|event| **event == "resume")
                .count(),
            10
        );
    }

    #[tokio::test]
    async fn shutdown_while_suspended_never_resumes() {
        let mut machine = SleepStateMachine::new();
        let mut hooks = RecordingHooks::default();
        handle_event(&mut machine, &mut hooks, true).await;
        // Arrêt du daemon pendant la veille : aucun `false` ne doit être
        // synthétisé par la machine.
        assert!(machine.is_suspended());
        assert_eq!(hooks.events, vec!["suspend"]);
    }

    #[tokio::test]
    async fn logind_absence_does_not_terminate_the_watcher() {
        let (events, _receiver) = tokio::sync::mpsc::channel(1);
        // Sans logind, le watcher réessaie au lieu de rendre la main ou de
        // faire échouer le daemon. Le timeout annule la tâche de test.
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                super::watch_logind(events),
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn logind_watcher_stops_when_receiver_is_closed() {
        let (events, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::watch_logind(events),
        )
        .await
        .expect("le watcher doit sortir immédiatement");
    }
}
