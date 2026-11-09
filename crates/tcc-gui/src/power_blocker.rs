// SPDX-License-Identifier: GPL-3.0-or-later
//
// Inhibiteur de veille « idle » via logind, sans privilège.
//
// La GUI demande à logind un inhibitor `idle` en mode `block` :
// - `idle` uniquement : une demande de veille explicite de l'utilisateur
//   (`systemctl suspend`, bouton du menu) n'est **pas** bloquée ;
// - `block` (et non `delay`) : l'inhibiteur empêche la veille automatique
//   d'inactivité tant que la GUI le maintient ;
// - le fd retourné par `Inhibit` est conservé tant que le blocage est actif ;
//   sa fermeture (désactivation, arrêt du worker, fermeture de la GUI) libère
//   l'inhibiteur côté logind.
//
// Aucun `systemctl`, aucun shell, aucune activation de service root, aucun
// polkit : `Inhibit` est autorisé aux sessions actives par logind.
//
// Le backend est injectable (`Inhibitor`) pour les tests ; le worker unique de
// `daemon_client.rs` possède l'état et exécute les appels, jamais QML.

use std::future::Future;
use std::os::fd::OwnedFd;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Budget maximal d'une demande d'inhibiteur (connexion + appel D-Bus).
pub const INHIBIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Erreur typée du blocage de veille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerError {
    /// La session n'est pas graphique (TTY, SSH, environnement absent).
    NotGraphical,
    /// Bus système ou service logind injoignable.
    Unavailable,
    /// logind a refusé la demande (session non active, policy, …).
    Refused,
    /// Pas de réponse dans [`INHIBIT_TIMEOUT`].
    Timeout,
}

impl BlockerError {
    /// Code stable pour l'UI (`NotGraphical`, `Unavailable`, …).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NotGraphical => "NotGraphical",
            Self::Unavailable => "Unavailable",
            Self::Refused => "Refused",
            Self::Timeout => "Timeout",
        }
    }
}

/// Backend d'inhibition (logind en production, faux en test).
pub trait Inhibitor: Send + Sync {
    /// Prend un inhibitor `idle`/`block` et retourne le fd qui le maintient.
    ///
    /// Fermer le fd libère l'inhibiteur. L'implémentation ne doit pas bloquer
    /// le thread courant au-delà de [`INHIBIT_TIMEOUT`].
    fn inhibit<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<OwnedFd, BlockerError>> + Send + 'a>>;
}

/// État partagé du fd d'inhibition.
#[derive(Debug, Default)]
struct BlockerState {
    fd: Mutex<Option<OwnedFd>>,
}

impl BlockerState {
    fn is_blocked(&self) -> bool {
        self.lock().is_some()
    }

    fn take(&self) -> Option<OwnedFd> {
        self.lock().take()
    }

    fn set(&self, fd: OwnedFd) {
        // Remplace un éventuel fd concurrent : l'ancien est fermé, donc un
        // seul inhibitor reste actif.
        let mut guard = self.lock();
        let _ = guard.replace(fd);
    }

    fn release(&self) {
        let _ = self.take();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<OwnedFd>> {
        // Un panic d'un autre thread ne doit pas empêcher la libération d'un
        // descripteur : on récupère l'intérieur empoisonné.
        self.fd
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Handle de secours partagé avec le QObject : permet de fermer le fd même si
/// la file de commandes du worker est pleine (SF-03).
#[derive(Debug, Clone)]
pub struct BlockerHandle {
    state: Arc<BlockerState>,
}

impl BlockerHandle {
    /// Vrai si le fd est actuellement détenu.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.state.is_blocked()
    }

    /// Ferme le fd détenu (idempotent, thread-safe).
    pub fn release(&self) {
        self.state.release();
    }
}

/// Bloqueur : backend + état partagé avec un éventuel [`BlockerHandle`].
pub struct PowerBlocker<I: Inhibitor + ?Sized> {
    backend: Arc<I>,
    state: Arc<BlockerState>,
}

impl<I: Inhibitor + ?Sized> PowerBlocker<I> {
    /// Construit un bloqueur inactif.
    #[must_use]
    pub fn new(backend: Arc<I>) -> Self {
        Self {
            backend,
            state: Arc::new(BlockerState::default()),
        }
    }

    /// Handle de secours pour libérer le fd depuis un autre thread.
    #[must_use]
    pub fn handle(&self) -> BlockerHandle {
        BlockerHandle {
            state: Arc::clone(&self.state),
        }
    }

    /// Vrai si un inhibitor est actuellement détenu.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.state.is_blocked()
    }

    /// Active ou libère l'inhibiteur.
    ///
    /// Activations et désactivations répétées sont idempotentes : une seconde
    /// activation ne demande pas un deuxième inhibitor, une désactivation sans
    /// inhibiteur réussit sans appel D-Bus.
    ///
    /// # Errors
    ///
    /// [`BlockerError`] du backend ; l'état reste alors inchangé.
    pub async fn set_blocked(&self, blocked: bool) -> Result<(), BlockerError> {
        if blocked {
            if self.state.is_blocked() {
                return Ok(());
            }
            let fd = self.backend.inhibit().await?;
            self.state.set(fd);
        } else {
            self.state.release();
        }
        Ok(())
    }

    /// Ferme le fd et libère l'inhibiteur côté logind.
    pub fn release(&self) {
        self.state.release();
    }
}

impl<I: Inhibitor + ?Sized> Drop for PowerBlocker<I> {
    fn drop(&mut self) {
        self.release();
    }
}

/// Après un refus d'envoi dans la file du worker (SF-03) :
/// - désactivation : le fd est fermé immédiatement, l'état publié est `false` ;
/// - activation : rien n'a été acquis, l'état courant est conservé.
///
/// Retourne l'état `powerBlocked` à publier.
#[must_use]
pub fn recover_from_queue_failure(blocked_command: bool, handle: &BlockerHandle) -> bool {
    if blocked_command {
        handle.is_blocked()
    } else {
        handle.release();
        false
    }
}

/// Backend de production : `org.freedesktop.login1.Manager.Inhibit`.
#[derive(Debug, Default)]
pub struct LogindInhibitor;

impl LogindInhibitor {
    /// Backend logind réel (bus système).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(name = "Inhibit")]
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
}

impl Inhibitor for LogindInhibitor {
    fn inhibit<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<OwnedFd, BlockerError>> + Send + 'a>> {
        Box::pin(async move {
            let call = async {
                let connection = zbus::Connection::system()
                    .await
                    .map_err(|_| BlockerError::Unavailable)?;
                let proxy = Login1ManagerProxy::new(&connection)
                    .await
                    .map_err(|_| BlockerError::Unavailable)?;
                let fd = proxy
                    .inhibit(
                        "idle",
                        "TuxedoRustCenter",
                        "mode démo/veille bloquée",
                        "block",
                    )
                    .await
                    .map_err(|error| match error {
                        zbus::Error::MethodError(..) => BlockerError::Refused,
                        _ => BlockerError::Unavailable,
                    })?;
                Ok::<_, BlockerError>(fd)
            };
            match tokio::time::timeout(INHIBIT_TIMEOUT, call).await {
                Ok(Ok(fd)) => Ok(fd.into()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(BlockerError::Timeout),
            }
        })
    }
}

/// Détecte une session graphique à partir de `XDG_SESSION_TYPE`.
///
/// `tty`, une valeur inconnue ou une variable absente désactivent le réglage :
/// logind reste de toute façon l'autorité finale (il peut refuser une session
/// non active). La GUI ne tente pas de lire `/proc` ni de deviner la session.
#[must_use]
pub fn session_type_is_graphical(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("x11" | "wayland" | "mir")
    )
}

/// Session courante : voir [`session_type_is_graphical`].
#[must_use]
pub fn session_is_graphical() -> bool {
    session_type_is_graphical(std::env::var("XDG_SESSION_TYPE").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::{
        BlockerError, Inhibitor, LogindInhibitor, PowerBlocker, session_type_is_graphical,
    };
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Faux backend : compte les appels et rend un fd fourni (ou échoue).
    struct FakeInhibitor {
        calls: AtomicUsize,
        fd: Mutex<Option<OwnedFd>>,
        fail: bool,
    }

    impl FakeInhibitor {
        fn ok(fd: OwnedFd) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fd: Mutex::new(Some(fd)),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fd: Mutex::new(None),
                fail: true,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Inhibitor for FakeInhibitor {
        fn inhibit<'a>(
            &'a self,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<OwnedFd, BlockerError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if self.fail {
                    return Err(BlockerError::Refused);
                }
                self.fd
                    .lock()
                    .map_err(|_| BlockerError::Unavailable)?
                    .take()
                    .ok_or(BlockerError::Unavailable)
            })
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    /// Paire socket : le fd retourné par le faux backend. Quand il est fermé,
    /// `probe` lit EOF (fin de flux), ce qui prouve la libération.
    fn probe_pair() -> (UnixStream, OwnedFd) {
        let (probe, held) = UnixStream::pair().expect("socketpair");
        probe
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout de lecture");
        (probe, held.into())
    }

    fn assert_released(probe: &mut UnixStream) {
        let mut buffer = [0u8; 1];
        assert_eq!(
            probe.read(&mut buffer).expect("lecture EOF"),
            0,
            "le fd de l'inhibiteur doit être fermé"
        );
    }

    #[test]
    fn activation_and_release_close_the_fd() {
        let runtime = runtime();
        let (mut probe, fd) = probe_pair();
        let fake = Arc::new(FakeInhibitor::ok(fd));
        let blocker = PowerBlocker::new(fake.clone());

        runtime
            .block_on(blocker.set_blocked(true))
            .expect("activation");
        assert!(blocker.is_blocked());
        assert_eq!(fake.calls(), 1);

        runtime
            .block_on(blocker.set_blocked(false))
            .expect("désactivation");
        assert!(!blocker.is_blocked());
        assert_released(&mut probe);
        // Désactivation répétée : idempotente, sans nouvel appel.
        runtime
            .block_on(blocker.set_blocked(false))
            .expect("désactivation idempotente");
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn double_activation_is_idempotent() {
        let runtime = runtime();
        let (_probe, fd) = probe_pair();
        let fake = Arc::new(FakeInhibitor::ok(fd));
        let blocker = PowerBlocker::new(fake.clone());

        runtime.block_on(blocker.set_blocked(true)).expect("1");
        runtime.block_on(blocker.set_blocked(true)).expect("2");
        assert!(blocker.is_blocked());
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn drop_releases_the_inhibitor() {
        let runtime = runtime();
        let (mut probe, fd) = probe_pair();
        let fake = Arc::new(FakeInhibitor::ok(fd));
        let blocker = PowerBlocker::new(fake);
        runtime
            .block_on(blocker.set_blocked(true))
            .expect("activation");
        drop(blocker);
        assert_released(&mut probe);
    }

    #[test]
    fn external_handle_releases_the_fd_while_blocker_lives() {
        let runtime = runtime();
        let (mut probe, fd) = probe_pair();
        let fake = Arc::new(FakeInhibitor::ok(fd));
        let blocker = PowerBlocker::new(fake);
        let handle = blocker.handle();
        runtime
            .block_on(blocker.set_blocked(true))
            .expect("activation");
        assert!(handle.is_blocked());

        // Cas SF-03 : la file refuse la désactivation, le QObject libère.
        assert!(!super::recover_from_queue_failure(false, &handle));
        assert!(!handle.is_blocked());
        assert!(!blocker.is_blocked());
        assert_released(&mut probe);

        // Une activation ratée ne ferme rien par elle-même.
        assert!(!super::recover_from_queue_failure(true, &handle));
    }

    #[test]
    fn logind_error_is_typed_and_state_is_unchanged() {
        let runtime = runtime();
        let fake = Arc::new(FakeInhibitor::failing());
        let blocker = PowerBlocker::new(fake);
        let error = runtime
            .block_on(blocker.set_blocked(true))
            .expect_err("refus attendu");
        assert_eq!(error, BlockerError::Refused);
        assert_eq!(error.code(), "Refused");
        assert!(!blocker.is_blocked());
        // Une désactivation après échec reste sûre.
        runtime
            .block_on(blocker.set_blocked(false))
            .expect("release");
    }

    #[test]
    fn session_type_detection_is_strict() {
        for value in ["x11", "wayland", " Wayland ", "MIR"] {
            assert!(session_type_is_graphical(Some(value)), "valeur: {value}");
        }
        for value in [None, Some("tty"), Some(""), Some("ssh"), Some("dumb")] {
            assert!(!session_type_is_graphical(value), "valeur: {value:?}");
        }
    }

    #[test]
    fn production_backend_is_constructible_without_side_effect() {
        // Aucune connexion n'est ouverte à la construction.
        let backend = LogindInhibitor::new();
        let _ = backend;
        assert!(session_type_is_graphical(Some("wayland")));
    }
}
