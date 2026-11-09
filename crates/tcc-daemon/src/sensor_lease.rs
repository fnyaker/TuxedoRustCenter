// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Bail de collecte des capteurs.
//!
//! `SetSensorDataCollectionStatus(true)` ouvre une session de collecte :
//!
//! - bail d'inactivité de 10 s, réarmé par `GetHardwareSnapshotJSON` ;
//! - **plafond absolu de 5 min** par session, non repoussé par les lectures
//!   ni par un nouveau `Set(true)` tant que la session est active ;
//! - une tâche de supervision (1 s) coupe la collecte à l'échéance.
//!
//! Après expiration, un nouveau `SetSensorDataCollectionStatus(true)` ouvre une
//! nouvelle session de 5 min. La GUI peut donc collecter durablement en
//! renouvelant explicitement le bail, sans qu'une simple boucle de lectures
//! puisse le maintenir indéfiniment.
//!
//! Les méthodes prennent un `Instant` explicite pour que les tests simulent le
//! temps sans `sleep`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Durée de vie d'une activation de collecte sans lecture.
pub(crate) const SENSOR_COLLECTION_TTL: Duration = Duration::from_secs(10);
/// Durée maximale d'une session de collecte, renouvelable explicitement.
pub(crate) const MAX_SENSOR_SESSION: Duration = Duration::from_secs(5 * 60);
/// Période de la tâche de supervision.
pub(crate) const SENSOR_SUPERVISOR_INTERVAL: Duration = Duration::from_secs(1);

/// Raison de l'expiration d'un bail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryReason {
    /// Plus aucune lecture pendant le TTL (10 s).
    Idle,
    /// Plafond absolu de session atteint (5 min).
    SessionMax,
}

#[derive(Debug, Default)]
struct LeaseState {
    idle_deadline: Option<Instant>,
    session_deadline: Option<Instant>,
}

/// Bail partagé entre les handlers D-Bus, la supervision et le worker capteurs.
#[derive(Debug)]
pub struct SensorLease {
    enabled: Arc<AtomicBool>,
    state: Mutex<LeaseState>,
    ttl: Duration,
    max_session: Duration,
}

impl SensorLease {
    /// Crée un bail désactivé avec les limites de production.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(SENSOR_COLLECTION_TTL, MAX_SENSOR_SESSION)
    }

    /// Crée un bail avec un TTL d'inactivité explicite (tests).
    #[cfg(test)]
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self::with_limits(ttl, MAX_SENSOR_SESSION)
    }

    /// Crée un bail avec des limites explicites (tests).
    #[must_use]
    pub fn with_limits(ttl: Duration, max_session: Duration) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(LeaseState::default()),
            ttl,
            max_session,
        }
    }

    /// Drapeau partagé avec le worker de capteurs.
    #[must_use]
    pub fn enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }

    /// Vrai si la collecte est actuellement autorisée.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Active la collecte et arme les deadlines.
    ///
    /// Si une session est déjà active, seul le bail d'inactivité est repoussé :
    /// le plafond de 5 min n'est pas réinitialisé. La session suivante démarre
    /// après l'expiration.
    pub fn arm(&self, now: Instant) {
        let mut state = self.lock_state();
        let session_expired = state
            .session_deadline
            .is_some_and(|deadline| now >= deadline);
        if !self.enabled.load(Ordering::Relaxed) || session_expired {
            // Nouvelle session explicite. Un `true` pendant une session
            // encore valide ne repousse pas le plafond absolu.
            state.session_deadline = Some(now + self.max_session);
        }
        state.idle_deadline = Some(now + self.ttl);
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Désactive immédiatement la collecte et efface les deadlines.
    pub fn disarm(&self) {
        let mut state = self.lock_state();
        *state = LeaseState::default();
        self.enabled.store(false, Ordering::Relaxed);
    }

    /// Réarme le bail d'inactivité si la collecte est active.
    ///
    /// Ne prolonge jamais le plafond absolu de session et ne réactive jamais
    /// une collecte désactivée.
    pub fn touch(&self, now: Instant) {
        let mut state = self.lock_state();
        if self.enabled.load(Ordering::Relaxed) {
            state.idle_deadline = Some(now + self.ttl);
        }
    }

    /// Coupe la collecte si un deadline est dépassé.
    ///
    /// Le plafond de session est prioritaire sur l'inactivité dans la raison
    /// retournée. Retourne `None` si la collecte reste active.
    pub fn expire_if_due(&self, now: Instant) -> Option<ExpiryReason> {
        let mut state = self.lock_state();
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        let reason = if state
            .session_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            Some(ExpiryReason::SessionMax)
        } else if state.idle_deadline.is_some_and(|deadline| now >= deadline) {
            Some(ExpiryReason::Idle)
        } else {
            None
        };
        if reason.is_some() {
            *state = LeaseState::default();
            self.enabled.store(false, Ordering::Relaxed);
        }
        reason
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, LeaseState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Bail d'inactivité courant (tests).
    #[cfg(test)]
    fn idle_deadline(&self) -> Option<Instant> {
        self.lock_state().idle_deadline
    }

    /// Bail de session courant (tests).
    #[cfg(test)]
    fn session_deadline(&self) -> Option<Instant> {
        self.lock_state().session_deadline
    }

    /// Bail d'inactivité exposé aux tests du module D-Bus.
    #[cfg(test)]
    pub(crate) fn deadline_for_test(&self) -> Option<Instant> {
        self.idle_deadline()
    }
}

impl Default for SensorLease {
    fn default() -> Self {
        Self::new()
    }
}

/// Lance la tâche de supervision (une seule tâche, pas de thread dédié).
///
/// Le handle est détaché : la tâche s'arrête avec le runtime. Elle ne fait
/// qu'une lecture d'`Instant` et un éventuel `store` atomique par seconde.
pub(crate) fn spawn_supervisor(lease: Arc<SensorLease>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SENSOR_SUPERVISOR_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match lease.expire_if_due(Instant::now()) {
                Some(ExpiryReason::Idle) => {
                    tracing::debug!("collecte des capteurs expirée après 10 s sans lecture");
                }
                Some(ExpiryReason::SessionMax) => {
                    tracing::warn!(
                        "collecte des capteurs arrêtée après 5 min ; \
                         renouveler explicitement SetSensorDataCollectionStatus(true)"
                    );
                }
                None => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let lease = SensorLease::new();
        assert!(!lease.is_enabled());
        assert_eq!(lease.idle_deadline(), None);
        assert_eq!(lease.session_deadline(), None);
    }

    #[test]
    fn arm_then_disarm_is_immediate() {
        let lease = SensorLease::new();
        let start = Instant::now();
        lease.arm(start);
        assert!(lease.is_enabled());
        assert_eq!(lease.idle_deadline(), Some(start + SENSOR_COLLECTION_TTL));
        assert_eq!(lease.session_deadline(), Some(start + MAX_SENSOR_SESSION));

        lease.disarm();
        assert!(!lease.is_enabled());
        assert_eq!(lease.idle_deadline(), None);
        assert_eq!(lease.session_deadline(), None);
        // Un touch après désactivation ne réactive pas.
        lease.touch(start + Duration::from_secs(1));
        assert!(!lease.is_enabled());
    }

    #[test]
    fn read_rearms_idle_deadline_but_not_session_cap() {
        let lease = SensorLease::new();
        let start = Instant::now();
        lease.arm(start);
        let session_before = lease.session_deadline();

        // Lecture à mi-parcours : seul le bail d'inactivité est repoussé.
        let read = start + Duration::from_secs(8);
        lease.touch(read);
        assert_eq!(lease.idle_deadline(), Some(read + SENSOR_COLLECTION_TTL));
        assert_eq!(lease.session_deadline(), session_before);

        // Juste avant le nouveau deadline d'inactivité : toujours actif.
        assert_eq!(
            lease.expire_if_due(read + SENSOR_COLLECTION_TTL - Duration::from_millis(1)),
            None
        );
        assert!(lease.is_enabled());

        // À l'échéance d'inactivité : expiration.
        assert_eq!(
            lease.expire_if_due(read + SENSOR_COLLECTION_TTL),
            Some(ExpiryReason::Idle)
        );
        assert!(!lease.is_enabled());
        assert_eq!(lease.idle_deadline(), None);
    }

    #[test]
    fn session_cap_wins_over_repeated_rearms() {
        let lease = SensorLease::new();
        let start = Instant::now();
        lease.arm(start);

        // Un Set(true) et des lectures répétées ne repoussent pas le plafond.
        lease.arm(start + Duration::from_secs(60));
        lease.touch(start + MAX_SENSOR_SESSION - Duration::from_secs(1));
        assert_eq!(
            lease.expire_if_due(start + MAX_SENSOR_SESSION + Duration::from_millis(1)),
            Some(ExpiryReason::SessionMax)
        );
        assert!(!lease.is_enabled());

        // Un nouveau Set(true) explicite ouvre une nouvelle session.
        let renewed = start + MAX_SENSOR_SESSION + Duration::from_secs(2);
        lease.arm(renewed);
        assert!(lease.is_enabled());
        assert_eq!(lease.session_deadline(), Some(renewed + MAX_SENSOR_SESSION));
    }

    #[test]
    fn expiry_before_deadline_does_nothing() {
        let lease = SensorLease::new();
        let start = Instant::now();
        lease.arm(start);
        assert_eq!(lease.expire_if_due(start + Duration::from_secs(9)), None);
        assert!(lease.is_enabled());
    }

    #[test]
    fn supervisor_is_not_required_for_expiry_logic() {
        // La décision d'expiration est pure et testable sans tâche Tokio.
        let lease = SensorLease::with_ttl(Duration::from_millis(50));
        let start = Instant::now();
        lease.arm(start);
        assert_eq!(
            lease.expire_if_due(start + Duration::from_millis(50)),
            Some(ExpiryReason::Idle)
        );
        assert!(!lease.is_enabled());
    }

    #[test]
    fn explicit_set_after_session_expiry_starts_new_session() {
        let lease = SensorLease::new();
        let start = Instant::now();
        lease.arm(start);
        // Le superviseur n'a pas encore tourné, mais le plafond est atteint :
        // un Set(true) explicite ouvre une nouvelle session.
        let renewed = start + MAX_SENSOR_SESSION + Duration::from_secs(1);
        lease.arm(renewed);
        assert!(lease.is_enabled());
        assert_eq!(lease.session_deadline(), Some(renewed + MAX_SENSOR_SESSION));
    }

    #[test]
    fn session_cap_is_independent_of_idle_ttl() {
        let lease = SensorLease::with_limits(Duration::from_secs(10), Duration::from_secs(30));
        let start = Instant::now();
        lease.arm(start);
        // Lectures toutes les 5 s : l'inactivité ne coupe pas, mais le plafond
        // de 30 s s'applique.
        for step in 1..=6 {
            lease.touch(start + Duration::from_secs(step * 5));
        }
        assert_eq!(
            lease.expire_if_due(start + Duration::from_secs(31)),
            Some(ExpiryReason::SessionMax)
        );
        assert!(!lease.is_enabled());
    }
}
