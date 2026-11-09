// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Limiteur de débit des écritures UVC de l'agent de session.
//!
//! Le service `User1` est non privilégié, mais une écriture `VIDIOC_S_CTRL`
//! ouvre un périphérique et peut bloquer brièvement. Un client local peut
//! boucler : le limiteur borne le débit par **UID appelant** (et non par nom
//! unique D-Bus), conformément au contrat de `SetUvcControl`.
//!
//! - 20 écritures/s soutenues, burst initial de 20 ;
//! - map bornée (256 appelants) et entrées inactives oubliées après 60 s ;
//! - un échec de résolution de l'UID retombe sur le nom unique D-Bus, ce qui
//!   garde une limite par connexion au lieu d'un bucket partagé global.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Débit soutenu des écritures UVC, par UID appelant.
pub(crate) const UVC_WRITE_RATE_PER_SECOND: f64 = 20.0;
/// Capacité du burst initial des écritures UVC.
pub(crate) const UVC_WRITE_BURST: f64 = 20.0;
/// Nombre maximal d'appelants suivis simultanément.
const MAX_TRACKED_CALLERS: usize = 256;
/// Durée après laquelle un appelant inactif est oublié.
const CALLER_IDLE_TTL: Duration = Duration::from_secs(60);
/// Intervalle minimal entre deux nettoyages complets.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    last_seen: Instant,
}

impl TokenBucket {
    fn new(now: Instant, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: now,
            last_seen: now,
        }
    }

    fn refill(&mut self, now: Instant, rate_per_second: f64, burst: f64) {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate_per_second).min(burst);
        self.last_refill = now;
    }

    fn consume(&mut self, now: Instant, rate_per_second: f64, burst: f64) -> bool {
        self.refill(now, rate_per_second, burst);
        self.last_seen = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Token bucket des écritures UVC, par clé d'appelant (`uid:1000`).
#[derive(Debug)]
pub(crate) struct UvcWriteRateLimiter {
    rate_per_second: f64,
    burst: f64,
    buckets: HashMap<String, TokenBucket>,
    last_cleanup: Option<Instant>,
}

impl Default for UvcWriteRateLimiter {
    fn default() -> Self {
        Self::new(UVC_WRITE_RATE_PER_SECOND, UVC_WRITE_BURST)
    }
}

impl UvcWriteRateLimiter {
    /// Crée un limiteur. Les valeurs non finies ou non positives retombent sur
    /// les constantes par défaut (fail-safe déterministe).
    pub(crate) fn new(rate_per_second: f64, burst: f64) -> Self {
        let rate_per_second = if rate_per_second.is_finite() && rate_per_second > 0.0 {
            rate_per_second
        } else {
            UVC_WRITE_RATE_PER_SECOND
        };
        let burst = if burst.is_finite() && burst >= 1.0 {
            burst
        } else {
            UVC_WRITE_BURST
        };
        Self {
            rate_per_second,
            burst,
            buckets: HashMap::new(),
            last_cleanup: None,
        }
    }

    /// Vérifie et consomme un jeton pour `caller`.
    pub(crate) fn allow(&mut self, caller: &str, now: Instant) -> bool {
        self.cleanup(now);
        let rate = self.rate_per_second;
        let burst = self.burst;
        let bucket = self
            .buckets
            .entry(caller.to_owned())
            .or_insert_with(|| TokenBucket::new(now, burst));
        bucket.consume(now, rate, burst)
    }

    /// Nombre d'appelants actuellement suivis (tests/diagnostic).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.buckets.len()
    }

    fn cleanup(&mut self, now: Instant) {
        let due = self
            .last_cleanup
            .is_none_or(|last| now.saturating_duration_since(last) >= CLEANUP_INTERVAL)
            || self.buckets.len() >= MAX_TRACKED_CALLERS;
        if !due {
            return;
        }

        self.buckets
            .retain(|_, bucket| now.saturating_duration_since(bucket.last_seen) < CALLER_IDLE_TTL);

        while self.buckets.len() >= MAX_TRACKED_CALLERS {
            let oldest = self
                .buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.last_seen)
                .map(|(caller, _)| caller.clone());
            match oldest {
                Some(caller) => {
                    self.buckets.remove(&caller);
                }
                None => break,
            }
        }
        self.last_cleanup = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_rate_limit_then_refill() {
        let mut limiter = UvcWriteRateLimiter::default();
        let start = Instant::now();

        for _ in 0..20 {
            assert!(limiter.allow("uid:1000", start));
        }
        assert!(!limiter.allow("uid:1000", start));

        // 100 ms à 20/s = 2 jetons.
        assert!(limiter.allow("uid:1000", start + Duration::from_millis(100)));
        assert!(limiter.allow("uid:1000", start + Duration::from_millis(100)));
        assert!(!limiter.allow("uid:1000", start + Duration::from_millis(100)));

        // Un autre UID a son propre bucket.
        assert!(limiter.allow("uid:1001", start));
    }

    #[test]
    fn saturated_bucket_refills_to_burst_cap() {
        let mut limiter = UvcWriteRateLimiter::default();
        let start = Instant::now();
        for _ in 0..20 {
            assert!(limiter.allow("uid:1000", start));
        }
        let later = start + Duration::from_secs(60);
        for _ in 0..20 {
            assert!(limiter.allow("uid:1000", later));
        }
        assert!(!limiter.allow("uid:1000", later));
    }

    #[test]
    fn map_is_bounded_under_unique_caller_flood() {
        let mut limiter = UvcWriteRateLimiter::default();
        let start = Instant::now();
        for index in 0..(MAX_TRACKED_CALLERS + 64) {
            let caller = format!("name::1.{index}");
            let _ = limiter.allow(&caller, start);
        }
        assert!(
            limiter.len() <= MAX_TRACKED_CALLERS,
            "map non bornée : {} entrées",
            limiter.len()
        );
    }

    #[test]
    fn idle_callers_are_evicted() {
        let mut limiter = UvcWriteRateLimiter::default();
        let start = Instant::now();
        assert!(limiter.allow("uid:1000", start));
        assert_eq!(limiter.len(), 1);

        let later = start + CALLER_IDLE_TTL + Duration::from_secs(1);
        assert!(limiter.allow("uid:1001", later));
        assert_eq!(limiter.len(), 1);
        assert!(!limiter.buckets.contains_key("uid:1000"));
    }

    #[test]
    fn invalid_configuration_falls_back_to_defaults() {
        let mut limiter = UvcWriteRateLimiter::new(f64::NAN, -1.0);
        let start = Instant::now();
        for _ in 0..20 {
            assert!(limiter.allow("uid:1000", start));
        }
        assert!(!limiter.allow("uid:1000", start));
    }
}
