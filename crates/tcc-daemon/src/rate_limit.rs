// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Limiteurs de débit D-Bus (écritures et lectures coûteuses).
//!
//! Chaque limiteur utilise un token bucket par **nom unique D-Bus** (pas par
//! UID : un même utilisateur peut ouvrir plusieurs connexions, chacune a son
//! propre nom) :
//!
//! - écritures : 5/s, burst 10 (profil temporaire, setters matériels) ;
//! - lectures du snapshot matériel : 2/s, burst 5.
//!
//! La map est nettoyée périodiquement (entrées inactives) et sa taille est
//! bornée : un client qui ouvre des milliers de connexions ne peut pas faire
//! croître la mémoire du daemon indéfiniment.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Débit soutenu par défaut (écritures), en opérations par seconde.
pub(crate) const WRITE_RATE_PER_SECOND: f64 = 5.0;
/// Capacité du burst initial par défaut (écritures).
pub(crate) const WRITE_BURST: f64 = 10.0;
/// Débit de lecture du snapshot matériel (par nom unique).
pub(crate) const READ_RATE_PER_SECOND: f64 = 2.0;
/// Burst de lecture du snapshot matériel.
pub(crate) const READ_BURST: f64 = 5.0;
/// Nombre maximal d'appelants suivis simultanément.
pub(crate) const MAX_TRACKED_CALLERS: usize = 1024;
/// Durée après laquelle une entrée inactive est oubliée.
pub(crate) const CALLER_IDLE_TTL: Duration = Duration::from_secs(300);
/// Intervalle minimal entre deux nettoyages complets.
pub(crate) const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

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

    /// Consomme un jeton si possible.
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

/// Token bucket par nom unique D-Bus, avec débit/burst configurables.
#[derive(Debug)]
pub(crate) struct RateLimiter {
    rate_per_second: f64,
    burst: f64,
    buckets: HashMap<String, TokenBucket>,
    last_cleanup: Option<Instant>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(WRITE_RATE_PER_SECOND, WRITE_BURST)
    }
}

impl RateLimiter {
    /// Crée un limiteur. Les valeurs non finies ou non positives retombent sur
    /// les constantes d'écriture (fail-safe déterministe).
    pub(crate) fn new(rate_per_second: f64, burst: f64) -> Self {
        let rate_per_second = if rate_per_second.is_finite() && rate_per_second > 0.0 {
            rate_per_second
        } else {
            WRITE_RATE_PER_SECOND
        };
        let burst = if burst.is_finite() && burst >= 1.0 {
            burst
        } else {
            WRITE_BURST
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

        // Même après le TTL, un afflux de noms uniques peut saturer la map :
        // évincer les plus anciens pour rester strictement borné.
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

/// Limiteur des écritures (charge, lighting, Fn-Lock, toggle capteurs).
pub(crate) type WriteRateLimiter = RateLimiter;
/// Limiteur des lectures coûteuses (snapshot matériel).
pub(crate) type ReadRateLimiter = RateLimiter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_rate_limit_then_refill() {
        let mut limiter = WriteRateLimiter::default();
        let start = Instant::now();

        for _ in 0..10 {
            assert!(limiter.allow(":1.42", start));
        }
        assert!(!limiter.allow(":1.42", start));

        // 200 ms à 5/s = 1 jeton.
        assert!(limiter.allow(":1.42", start + Duration::from_millis(200)));
        assert!(!limiter.allow(":1.42", start + Duration::from_millis(200)));

        // Les autres appelants ont leur propre bucket.
        assert!(limiter.allow(":1.43", start));
    }

    #[test]
    fn read_limits_are_stricter() {
        let mut limiter = ReadRateLimiter::new(READ_RATE_PER_SECOND, READ_BURST);
        let start = Instant::now();
        for _ in 0..5 {
            assert!(limiter.allow(":1.42", start));
        }
        assert!(!limiter.allow(":1.42", start));
        // 2/s : 500 ms = 1 jeton.
        assert!(limiter.allow(":1.42", start + Duration::from_millis(500)));
        assert!(!limiter.allow(":1.42", start + Duration::from_millis(500)));
    }

    #[test]
    fn saturated_bucket_refills_to_burst_cap() {
        let mut limiter = WriteRateLimiter::default();
        let start = Instant::now();
        for _ in 0..10 {
            assert!(limiter.allow(":1.42", start));
        }
        // Longue attente : le bucket ne dépasse pas le burst.
        let later = start + Duration::from_secs(60);
        for _ in 0..10 {
            assert!(limiter.allow(":1.42", later));
        }
        assert!(!limiter.allow(":1.42", later));
    }

    #[test]
    fn map_is_bounded_under_unique_name_flood() {
        let mut limiter = WriteRateLimiter::default();
        let start = Instant::now();
        for index in 0..(MAX_TRACKED_CALLERS + 64) {
            let caller = format!(":1.{index}");
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
        let mut limiter = WriteRateLimiter::default();
        let start = Instant::now();
        assert!(limiter.allow(":1.old", start));
        assert_eq!(limiter.len(), 1);

        let later = start + CALLER_IDLE_TTL + Duration::from_secs(1);
        assert!(limiter.allow(":1.new", later));
        assert_eq!(limiter.len(), 1);
        assert!(!limiter.buckets.contains_key(":1.old"));
    }
}
