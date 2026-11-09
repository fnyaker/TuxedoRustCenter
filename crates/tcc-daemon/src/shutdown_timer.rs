// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Minuterie d'extinction native via `org.freedesktop.login1`.
//!
//! `Settings.shutdown_time` contient une heure locale `"HH:MM"`. Le module la
//! convertit en échéance absolue (`usec` depuis l'epoch) et utilise
//! `ScheduleShutdown("poweroff", usec)` / `CancelScheduledShutdown` de logind.
//! Aucun `systemctl`, `shutdown` ou shell n'est utilisé.
//!
//! # Idempotence
//!
//! Une même échéance n'est programmée qu'une fois. Une erreur logind n'est pas
//! réessayée à chaque tick : elle ne l'est qu'après un changement de réglage,
//! un reload ou une reprise de veille (`force`).
//!
//! # Fuseau et DST
//!
//! L'heure est locale. `chrono::Local` résout le fuseau et les transitions
//! DST ; une heure locale inexistante est signalée comme erreur plutôt que
//! décalée silencieusement.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use zbus::proxy;

/// Délai maximal d'un appel logind.
const LOGIND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Origine d'une échéance exposée à l'UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TimerSource {
    /// Planification demandée par `settings.shutdownTime`.
    Settings,
    /// Échéance déjà présente côté logind (réservé à une future lecture).
    Logind,
    /// Aucune échéance active.
    #[default]
    None,
    /// Le réglage est invalide ou logind a échoué : état d'erreur visible.
    Error,
}

/// État exposé par `Manager1.GetShutdownTimerJSON`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownTimerStatus {
    /// Vrai si une extinction est planifiée et acceptée par logind.
    pub enabled: bool,
    /// Échéance locale `YYYY-MM-DD HH:MM`, si connue.
    pub scheduled_at: Option<String>,
    /// Origine de l'échéance.
    pub source: TimerSource,
    /// Erreur générique, sans chemin ni secret.
    pub error: Option<String>,
}

/// Horloge injectable (tests sans sleep réel).
pub trait ShutdownClock: Send + Sync {
    /// Date, heure locale et instant epoch courant.
    fn now(&self) -> LocalNow;
    /// Convertit une date/heure locale en microsecondes epoch.
    fn to_usec(&self, date: NaiveDate, time: NaiveTime) -> Option<i64>;
}

/// Instant local courant.
#[derive(Debug, Clone, Copy)]
pub struct LocalNow {
    /// Date locale.
    pub date: NaiveDate,
    /// Instant epoch en microsecondes.
    pub now_usec: i64,
}

/// Horloge système réelle.
#[derive(Debug, Default)]
pub struct SystemClock;

impl ShutdownClock for SystemClock {
    fn now(&self) -> LocalNow {
        let now = Local::now();
        LocalNow {
            date: now.date_naive(),
            now_usec: now.timestamp_micros(),
        }
    }

    fn to_usec(&self, date: NaiveDate, time: NaiveTime) -> Option<i64> {
        let naive = NaiveDateTime::new(date, time);
        match Local.from_local_datetime(&naive) {
            LocalResult::Single(value) => Some(value.timestamp_micros()),
            LocalResult::Ambiguous(earliest, _latest) => Some(earliest.timestamp_micros()),
            LocalResult::None => None,
        }
    }
}

/// Client logind injectable.
#[async_trait]
pub trait LogindShutdown: Send + Sync {
    /// Planifie une extinction.
    async fn schedule(&self, usec: u64) -> Result<(), String>;
    /// Annule l'extinction planifiée.
    async fn cancel(&self) -> Result<(), String>;
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Shutdown {
    #[zbus(name = "ScheduleShutdown")]
    fn schedule_shutdown(&self, kind: &str, usec: u64) -> zbus::Result<()>;

    #[zbus(name = "CancelScheduledShutdown")]
    fn cancel_scheduled_shutdown(&self) -> zbus::Result<()>;
}

/// Client logind réel sur le bus système.
#[derive(Debug, Default)]
pub struct ZbusLogind;

impl ZbusLogind {
    async fn proxy() -> Result<Login1ShutdownProxy<'static>, String> {
        let connection = tokio::time::timeout(LOGIND_TIMEOUT, zbus::Connection::system())
            .await
            .map_err(|_| "logind : délai de connexion dépassé".to_owned())?
            .map_err(|_| "logind indisponible".to_owned())?;
        tokio::time::timeout(LOGIND_TIMEOUT, Login1ShutdownProxy::new(&connection))
            .await
            .map_err(|_| "logind : délai de proxy dépassé".to_owned())?
            .map_err(|_| "logind indisponible".to_owned())
    }
}

#[async_trait]
impl LogindShutdown for ZbusLogind {
    async fn schedule(&self, usec: u64) -> Result<(), String> {
        let proxy = Self::proxy().await?;
        tokio::time::timeout(LOGIND_TIMEOUT, proxy.schedule_shutdown("poweroff", usec))
            .await
            .map_err(|_| "logind : planification en timeout".to_owned())?
            .map_err(|_| "logind a refusé la planification".to_owned())
    }

    async fn cancel(&self) -> Result<(), String> {
        let proxy = Self::proxy().await?;
        tokio::time::timeout(LOGIND_TIMEOUT, proxy.cancel_scheduled_shutdown())
            .await
            .map_err(|_| "logind : annulation en timeout".to_owned())?
            .map_err(|_| "logind a refusé l'annulation".to_owned())
    }
}

/// Échéance calculée.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Deadline {
    usec: u64,
    label: String,
}

/// Minuterie d'extinction.
pub struct ShutdownTimer {
    logind: Arc<dyn LogindShutdown>,
    clock: Arc<dyn ShutdownClock>,
    desired: Option<u64>,
    applied: Option<u64>,
    status: Arc<Mutex<ShutdownTimerStatus>>,
}

impl std::fmt::Debug for ShutdownTimer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownTimer")
            .field("desired", &self.desired)
            .field("applied", &self.applied)
            .finish_non_exhaustive()
    }
}

impl ShutdownTimer {
    /// Construit la minuterie avec un client et une horloge injectables.
    #[must_use]
    pub fn new(logind: Arc<dyn LogindShutdown>, clock: Arc<dyn ShutdownClock>) -> Self {
        Self {
            logind,
            clock,
            desired: None,
            applied: None,
            status: Arc::new(Mutex::new(ShutdownTimerStatus::default())),
        }
    }

    /// Minuterie de production (logind + horloge système).
    #[must_use]
    pub fn system() -> Self {
        Self::new(Arc::new(ZbusLogind), Arc::new(SystemClock))
    }

    /// Handle partagé avec D-Bus pour `GetShutdownTimerJSON`.
    #[must_use]
    pub fn status_handle(&self) -> Arc<Mutex<ShutdownTimerStatus>> {
        Arc::clone(&self.status)
    }

    /// Réconcilie le réglage avec logind.
    ///
    /// `force` réessaie après une erreur ou une reprise de veille ; sans lui,
    /// une échéance identique n'est jamais reprogrammée. `suspended` empêche
    /// toute action pendant la veille.
    pub async fn reconcile(&mut self, value: Option<&str>, suspended: bool, force: bool) {
        if suspended {
            return;
        }

        let (deadline, parse_error) = match next_deadline(self.clock.as_ref(), value) {
            Ok(deadline) => (deadline, None),
            Err(error) => (None, Some(error)),
        };
        let desired = deadline.as_ref().map(|deadline| deadline.usec);

        if !force && desired == self.desired && parse_error.is_none() {
            return;
        }
        self.desired = desired;

        if parse_error.is_some() {
            // L'erreur de réglage reste visible même si l'annulation de
            // l'ancienne échéance réussit (SF-02).
            self.set_error("heure d'extinction invalide");
            if self.applied.is_some() {
                let _ = self.cancel().await;
            }
            return;
        }

        match deadline {
            Some(deadline) => {
                if self.applied == Some(deadline.usec) {
                    self.set_scheduled(&deadline);
                    return;
                }
                match self.logind.schedule(deadline.usec).await {
                    Ok(()) => {
                        self.applied = Some(deadline.usec);
                        self.set_scheduled(&deadline);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "minuterie d'extinction : logind a échoué");
                        self.applied = None;
                        self.set_error("logind indisponible");
                    }
                }
            }
            None => {
                if self.applied.is_some() {
                    match self.cancel().await {
                        Ok(()) => self.set_disabled(),
                        Err(()) => self.set_error("logind indisponible"),
                    }
                } else {
                    self.set_disabled();
                }
            }
        }
    }

    async fn cancel(&mut self) -> Result<(), ()> {
        match self.logind.cancel().await {
            Ok(()) => {
                self.applied = None;
                Ok(())
            }
            Err(error) => {
                tracing::warn!(%error, "minuterie d'extinction : annulation logind échouée");
                // Conserver `applied` pour qu'un `force` ultérieur retente
                // l'annulation : logind peut encore avoir l'échéance.
                Err(())
            }
        }
    }

    fn set_scheduled(&self, deadline: &Deadline) {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *status = ShutdownTimerStatus {
            enabled: true,
            scheduled_at: Some(deadline.label.clone()),
            source: TimerSource::Settings,
            error: None,
        };
    }

    fn set_disabled(&self) {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *status = ShutdownTimerStatus::default();
    }

    fn set_error(&self, message: &'static str) {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *status = ShutdownTimerStatus {
            enabled: false,
            scheduled_at: None,
            source: TimerSource::Error,
            error: Some(message.to_owned()),
        };
    }
}

fn next_deadline(clock: &dyn ShutdownClock, value: Option<&str>) -> Result<Option<Deadline>, ()> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let time = parse_hhmm(value).ok_or(())?;
    let now = clock.now();
    let mut date = now.date;
    let mut target = clock.to_usec(date, time).ok_or(())?;
    if target <= now.now_usec {
        date = date.succ_opt().ok_or(())?;
        target = clock.to_usec(date, time).ok_or(())?;
    }
    let usec = u64::try_from(target).map_err(|_| ())?;
    Ok(Some(Deadline {
        usec,
        label: format!("{} {}", date.format("%Y-%m-%d"), time.format("%H:%M")),
    }))
}

fn parse_hhmm(value: &str) -> Option<NaiveTime> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    if !bytes[0..2].iter().all(u8::is_ascii_digit) || !bytes[3..5].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let hour = value[0..2].parse::<u32>().ok()?;
    let minute = value[3..5].parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{DateTime, FixedOffset};

    use super::{
        LocalNow, LogindShutdown, ShutdownClock, ShutdownTimer, ShutdownTimerStatus, TimerSource,
    };

    #[derive(Debug, Default)]
    struct FakeLogind {
        calls: Mutex<Vec<Call>>,
        fail_schedule: bool,
        fail_cancel: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Schedule(u64),
        Cancel,
    }

    impl FakeLogind {
        fn calls(&self) -> Vec<Call> {
            self.calls
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl LogindShutdown for FakeLogind {
        async fn schedule(&self, usec: u64) -> Result<(), String> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(Call::Schedule(usec));
            }
            if self.fail_schedule {
                Err("fake schedule".to_owned())
            } else {
                Ok(())
            }
        }

        async fn cancel(&self) -> Result<(), String> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(Call::Cancel);
            }
            if self.fail_cancel {
                Err("fake cancel".to_owned())
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug)]
    struct FixedClock {
        now: DateTime<FixedOffset>,
    }

    impl ShutdownClock for FixedClock {
        fn now(&self) -> LocalNow {
            LocalNow {
                date: self.now.date_naive(),
                now_usec: self.now.timestamp_micros(),
            }
        }

        fn to_usec(&self, date: chrono::NaiveDate, time: chrono::NaiveTime) -> Option<i64> {
            let naive = chrono::NaiveDateTime::new(date, time);
            let offset = i64::from(self.now.offset().local_minus_utc());
            Some(naive.and_utc().timestamp_micros() - offset * 1_000_000)
        }
    }

    fn clock(value: &str) -> FixedClock {
        FixedClock {
            now: DateTime::parse_from_rfc3339(value).expect("date de test"),
        }
    }

    fn timer(clock: FixedClock, logind: FakeLogind) -> (ShutdownTimer, std::sync::Arc<FakeLogind>) {
        let logind = std::sync::Arc::new(logind);
        (
            ShutdownTimer::new(logind.clone(), std::sync::Arc::new(clock)),
            logind,
        )
    }

    fn status(timer: &ShutdownTimer) -> ShutdownTimerStatus {
        timer.status.lock().expect("status").clone()
    }

    #[tokio::test]
    async fn schedules_future_time_today() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), false, false).await;
        let calls = logind.calls();
        assert_eq!(calls.len(), 1);
        let Call::Schedule(usec) = calls[0] else {
            panic!("planification attendue");
        };
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 11, 6)
            .and_then(|date| date.and_hms_opt(20, 0, 0))
            .map(|naive| naive.and_utc().timestamp_micros() - 3_600_000_000)
            .expect("date test");
        assert_eq!(usec, expected as u64);
        let status = status(&timer);
        assert!(status.enabled);
        assert_eq!(status.scheduled_at.as_deref(), Some("2026-11-06 20:00"));
        assert_eq!(status.error, None);
    }

    #[tokio::test]
    async fn past_time_is_scheduled_tomorrow() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("08:00"), false, false).await;
        let status = status(&timer);
        assert_eq!(status.scheduled_at.as_deref(), Some("2026-11-07 08:00"));
        assert_eq!(logind.calls().len(), 1);
    }

    #[tokio::test]
    async fn cancellation_is_idempotent() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(None, false, false).await;
        timer.reconcile(None, false, false).await;
        let calls = logind.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], Call::Schedule(_)));
        assert_eq!(calls[1], Call::Cancel);
        assert!(!status(&timer).enabled);
    }

    #[tokio::test]
    async fn same_deadline_is_not_rescheduled() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(Some("20:00"), false, true).await;
        assert_eq!(logind.calls().len(), 1);
    }

    #[tokio::test]
    async fn changed_time_is_rescheduled() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(Some("21:00"), false, false).await;
        assert_eq!(logind.calls().len(), 2);
        assert_eq!(
            status(&timer).scheduled_at.as_deref(),
            Some("2026-11-06 21:00")
        );
    }

    #[tokio::test]
    async fn logind_error_is_reported_and_not_retried_without_force() {
        let (mut timer, logind) = timer(
            clock("2026-11-06T18:30:00+01:00"),
            FakeLogind {
                fail_schedule: true,
                ..FakeLogind::default()
            },
        );
        timer.reconcile(Some("20:00"), false, false).await;
        assert_eq!(status(&timer).error.as_deref(), Some("logind indisponible"));
        assert_eq!(status(&timer).source, TimerSource::Error);
        assert!(!status(&timer).enabled);
        timer.reconcile(Some("20:00"), false, false).await;
        assert_eq!(logind.calls().len(), 1);
        timer.reconcile(Some("20:00"), false, true).await;
        assert_eq!(logind.calls().len(), 2);
    }

    #[tokio::test]
    async fn suspended_and_resumed() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), true, true).await;
        assert!(logind.calls().is_empty());
        timer.reconcile(Some("20:00"), false, true).await;
        assert_eq!(logind.calls().len(), 1);
    }

    #[tokio::test]
    async fn invalid_time_is_an_error_without_call() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("25:00"), false, false).await;
        assert_eq!(
            status(&timer).error.as_deref(),
            Some("heure d'extinction invalide")
        );
        assert_eq!(status(&timer).source, TimerSource::Error);
        assert!(logind.calls().is_empty());
    }

    #[tokio::test]
    async fn invalid_time_cancels_existing_schedule_but_keeps_error() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(Some("25:99"), false, true).await;
        let calls = logind.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], Call::Schedule(_)));
        assert_eq!(calls[1], Call::Cancel);
        let invalid = status(&timer);
        assert!(!invalid.enabled);
        assert_eq!(invalid.source, TimerSource::Error);
        assert_eq!(
            invalid.error.as_deref(),
            Some("heure d'extinction invalide")
        );

        // Toujours invalide : aucune reprogrammation, aucune nouvelle annulation.
        timer.reconcile(Some("25:99"), false, true).await;
        assert_eq!(logind.calls().len(), 2);
        assert_eq!(
            status(&timer).error.as_deref(),
            Some("heure d'extinction invalide")
        );

        // Réglage redevenu valide : une nouvelle planification efface l'erreur.
        timer.reconcile(Some("21:00"), false, true).await;
        let valid = status(&timer);
        assert!(valid.enabled);
        assert_eq!(valid.source, TimerSource::Settings);
        assert_eq!(valid.error, None);
        assert_eq!(logind.calls().len(), 3);
    }

    #[tokio::test]
    async fn empty_value_disables_and_clears_error() {
        let (mut timer, logind) = timer(clock("2026-11-06T18:30:00+01:00"), FakeLogind::default());
        timer.reconcile(Some("25:99"), false, false).await;
        assert_eq!(status(&timer).source, TimerSource::Error);
        timer.reconcile(Some("   "), false, true).await;
        let disabled = status(&timer);
        assert!(!disabled.enabled);
        assert_eq!(disabled.source, TimerSource::None);
        assert_eq!(disabled.error, None);
        assert!(logind.calls().is_empty());
    }

    #[tokio::test]
    async fn invalid_time_keeps_parse_error_when_cancel_fails() {
        let (mut timer, logind) = timer(
            clock("2026-11-06T18:30:00+01:00"),
            FakeLogind {
                fail_cancel: true,
                ..FakeLogind::default()
            },
        );
        timer.reconcile(Some("20:00"), false, false).await;
        timer.reconcile(Some("25:99"), false, true).await;
        assert_eq!(
            status(&timer).error.as_deref(),
            Some("heure d'extinction invalide")
        );
        assert_eq!(status(&timer).source, TimerSource::Error);
        // L'annulation sera retentée sur un prochain `force`.
        assert_eq!(logind.calls().len(), 2);
    }
}
