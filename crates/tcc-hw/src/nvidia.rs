// SPDX-License-Identifier: GPL-3.0-or-later
//! NVIDIA : offset cTGP (sysfs) et lectures `nvidia-smi` (allowlist).
//!
//! ```text
//! /sys/devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset   (R/W, watts)
//! nvidia-smi --format=csv,noheader,nounits --query-gpu=power.default_limit
//! nvidia-smi --format=csv,noheader,nounits --query-gpu=power.max_limit
//! nvidia-smi --query-gpu=power.draw,power.max_limit,enforced.power.limit,clocks.gr,clocks.max.gr --format=csv,noheader
//! ```
//!
//! Sources : `NVIDIAPowerCTRLListener` et `GpuInfoWorker` de l'original.
//!
//! # Règles
//!
//! - `ctgp_offset` n'est appliqué que si le fichier existe **et** que
//!   `nvidia-smi` est présent (comportement original). L'absence de l'outil
//!   n'empêche pas de lire/écrire l'offset seul, mais le daemon n'expose pas
//!   la fonctionnalité à l'UI dans ce cas (voir
//!   [`CtgpControl::is_available`]).
//! - `nvidia-smi` est exécuté par chemin absolu (jamais via `PATH`), avec des
//!   arguments séparés et un délai maximal de [`NVIDIA_SMI_TIMEOUT`]
//!   (`command::run`). Aucun shell.
//! - **Cache** : une même instance [`NvidiaSmi`] ne relance pas l'outil plus
//!   d'une fois toutes les [`NVIDIA_MIN_QUERY_INTERVAL`] (10 s par défaut),
//!   y compris après un échec (repli borné). Le worker de capteurs, qui tick
//!   toutes les 2 s, ne peut donc pas forker `nvidia-smi` en root à cette
//!   fréquence. Le coût est **un processus toutes les 10 s au pire**, et
//!   seulement si une GPU NVIDIA est détectée.
//! - Toutes les lectures sont **en lecture seule**. Aucune commande
//!   d'écriture de limite n'est exposée : la limite est pilotée par
//!   `ctgp_offset`, comme dans l'original.
//! - Une sortie inexploitable retourne `Ok(None)` (capteur indisponible), pas
//!   une erreur : la collecte de capteurs ne doit pas échouer à cause d'un
//!   driver NVIDIA en veille.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::command::{self, CommandOutput};
use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Chemin de l'offset cTGP.
pub const CTGP_OFFSET_PATH: &str = "devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset";
/// Délai maximal accordé à `nvidia-smi`.
pub const NVIDIA_SMI_TIMEOUT: Duration = Duration::from_secs(2);
/// Intervalle minimal entre deux exécutions de `nvidia-smi` pour une instance.
///
/// Le worker de capteurs tourne toutes les 2 s ; sans ce cache, il lancerait un
/// processus root 43 000 fois par jour (finding H-01/H-02 de la revue
/// sécurité). 10 s borne le coût sans dégrader la réactivité de l'UI.
pub const NVIDIA_MIN_QUERY_INTERVAL: Duration = Duration::from_secs(10);
/// Borne de sécurité de l'offset cTGP, en watts.
///
/// Reprend la borne de validation des profils (`tcc-core`) comme seconde
/// barrière : une valeur aberrante ne doit jamais atteindre le firmware, même
/// si l'appelant a contourné la validation de configuration.
pub const MAX_CTGP_OFFSET_WATTS: f64 = 200.0;

/// Taille maximale d'un message d'échec mis en cache.
const MAX_CACHED_ERROR: usize = 512;

const QUERY_DEFAULT_LIMIT: [&str; 2] = [
    "--format=csv,noheader,nounits",
    "--query-gpu=power.default_limit",
];
const QUERY_MAX_LIMIT: [&str; 2] = [
    "--format=csv,noheader,nounits",
    "--query-gpu=power.max_limit",
];
const QUERY_SENSORS: [&str; 2] = [
    "--query-gpu=power.draw,power.max_limit,enforced.power.limit,clocks.gr,clocks.max.gr",
    "--format=csv,noheader,nounits",
];

/// Limites de puissance annoncées par le driver NVIDIA (watts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvidiaPowerLimits {
    /// Limite par défaut.
    pub default_watts: f64,
    /// Limite maximale autorisée.
    pub max_watts: f64,
}

/// Capteurs NVIDIA (watts, MHz).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NvidiaSensors {
    /// Puissance consommée.
    pub power_draw_watts: Option<f64>,
    /// Limite de puissance effective.
    pub enforced_limit_watts: Option<f64>,
    /// Limite maximale.
    pub max_limit_watts: Option<f64>,
    /// Fréquence graphique courante.
    pub core_clock_mhz: Option<f64>,
    /// Fréquence graphique maximale.
    pub max_core_clock_mhz: Option<f64>,
}

/// Entrée de cache réussie.
#[derive(Debug, Clone, Copy)]
struct CacheEntry<T> {
    value: Option<T>,
    fetched_at: Instant,
}

/// Échec mis en cache pour éviter de relancer un outil défaillant à chaque tick.
#[derive(Debug, Clone)]
struct CachedFailure {
    program: &'static str,
    reason: String,
    until: Instant,
}

impl CachedFailure {
    fn new(program: &'static str, error: &HwError, until: Instant) -> Self {
        // Pour `External`, le message contient déjà le programme et le détail :
        // on ne réemboîte pas le préfixe lors de la relecture du cache.
        let raw = match error {
            HwError::External { reason, .. } => reason.clone(),
            other => other.to_string(),
        };
        let reason: String = raw
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_CACHED_ERROR)
            .collect();
        Self {
            program,
            reason,
            until,
        }
    }

    fn to_error(&self) -> HwError {
        HwError::External {
            program: self.program,
            reason: self.reason.clone(),
        }
    }
}

/// Cache par instance de [`NvidiaSmi`].
#[derive(Debug, Default)]
struct NvidiaCache {
    sensors: Option<CacheEntry<NvidiaSensors>>,
    sensors_failure: Option<CachedFailure>,
    limits: Option<CacheEntry<NvidiaPowerLimits>>,
    limits_failure: Option<CachedFailure>,
}

/// Lanceur de commande injectable.
///
/// En production il s'agit de [`crate::command::run`]. Les tests unitaires
/// fournissent une closure pure (compteur atomique + sortie fixe) afin de ne
/// dépendre ni d'un processus externe, ni d'un fichier exécutable temporaire :
/// c'est ce qui élimine le `ETXTBSY` observé en exécution parallèle.
type CommandRunner =
    Arc<dyn Fn(&'static str, &[&str], Duration) -> Result<CommandOutput> + Send + Sync>;

/// Lecteur `nvidia-smi` allowlisté, avec cache borné dans le temps.
pub struct NvidiaSmi {
    program: Option<&'static str>,
    min_interval: Duration,
    cache: Arc<Mutex<NvidiaCache>>,
    runner: CommandRunner,
}

impl fmt::Debug for NvidiaSmi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NvidiaSmi")
            .field("program", &self.program)
            .field("min_interval", &self.min_interval)
            .finish_non_exhaustive()
    }
}

impl Clone for NvidiaSmi {
    fn clone(&self) -> Self {
        Self {
            program: self.program,
            min_interval: self.min_interval,
            cache: Arc::clone(&self.cache),
            runner: Arc::clone(&self.runner),
        }
    }
}

impl Default for NvidiaSmi {
    fn default() -> Self {
        Self::with_program(None, NVIDIA_MIN_QUERY_INTERVAL)
    }
}

impl NvidiaSmi {
    /// Résout `nvidia-smi` dans l'allowlist de chemins absolus.
    #[must_use]
    pub fn detect() -> Self {
        Self::with_program(
            command::resolve(&command::NVIDIA_SMI),
            NVIDIA_MIN_QUERY_INTERVAL,
        )
    }

    /// Construit un lecteur avec un programme et un intervalle explicites.
    ///
    /// Le programme reste privé : seules les constantes allowlistées de
    /// [`crate::command`] peuvent être injectées en production. Les tests du
    /// module utilisent un script temporaire.
    fn with_program(program: Option<&'static str>, min_interval: Duration) -> Self {
        Self::with_runner(program, min_interval, Arc::new(command::run))
    }

    /// Construit un lecteur avec un lanceur injecté.
    ///
    /// Privé : seul `with_program` (production) et les tests du module
    /// l'utilisent. Aucun programme arbitraire n'est exposé publiquement.
    fn with_runner(
        program: Option<&'static str>,
        min_interval: Duration,
        runner: CommandRunner,
    ) -> Self {
        Self {
            program,
            min_interval,
            cache: Arc::new(Mutex::new(NvidiaCache::default())),
            runner,
        }
    }

    /// Ajuste l'intervalle minimal entre deux exécutions.
    ///
    /// Une valeur nulle désactive le cache (utile au diagnostic explicite).
    #[must_use]
    pub fn with_min_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    /// Intervalle minimal configuré.
    #[must_use]
    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }

    /// Vrai si `nvidia-smi` est installé à un chemin allowlisté.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.program.is_some()
    }

    /// Chemin résolu, pour le diagnostic.
    #[must_use]
    pub fn program(&self) -> Option<&'static str> {
        self.program
    }

    /// Lit les limites de puissance par défaut et maximale.
    ///
    /// Le résultat est mis en cache pendant [`NvidiaSmi::min_interval`].
    ///
    /// # Errors
    ///
    /// Erreur typée si la commande échoue **et** qu'un résultat était attendu.
    /// Une erreur est également mise en cache pour la même durée, afin de ne
    /// pas relancer un outil défaillant à chaque tick. Une sortie vide
    /// retourne `Ok(None)`.
    pub fn power_limits(&self) -> Result<Option<NvidiaPowerLimits>> {
        self.power_limits_at(Instant::now())
    }

    fn power_limits_at(&self, now: Instant) -> Result<Option<NvidiaPowerLimits>> {
        let Some(program) = self.program else {
            return Ok(None);
        };
        {
            let cache = self.lock_cache();
            if let Some(entry) = cache.limits
                && now.saturating_duration_since(entry.fetched_at) < self.min_interval
            {
                return Ok(entry.value);
            }
            if let Some(failure) = &cache.limits_failure
                && now < failure.until
            {
                return Err(failure.to_error());
            }
        }

        let runner = Arc::clone(&self.runner);
        let result = (|| {
            let default = runner(program, &QUERY_DEFAULT_LIMIT, NVIDIA_SMI_TIMEOUT)?;
            let max = runner(program, &QUERY_MAX_LIMIT, NVIDIA_SMI_TIMEOUT)?;
            let (Some(default), Some(max)) =
                (parse_first_number(&default), parse_first_number(&max))
            else {
                return Ok(None);
            };
            if !default.is_finite() || !max.is_finite() || default <= 0.0 || max <= 0.0 {
                return Ok(None);
            }
            Ok(Some(NvidiaPowerLimits {
                default_watts: default,
                max_watts: max,
            }))
        })();

        let mut cache = self.lock_cache();
        match &result {
            Ok(value) => {
                cache.limits = Some(CacheEntry {
                    value: *value,
                    fetched_at: now,
                });
                cache.limits_failure = None;
            }
            Err(error) => {
                cache.limits_failure = Some(CachedFailure::new(
                    program,
                    error,
                    now.checked_add(self.min_interval).unwrap_or(now),
                ));
            }
        }
        result
    }

    /// Lit les capteurs de la première GPU.
    ///
    /// Le résultat est mis en cache pendant [`NvidiaSmi::min_interval`] : c'est
    /// ce cache qui évite un `nvidia-smi` root toutes les 2 s.
    ///
    /// # Errors
    ///
    /// Erreur typée si la commande échoue ; l'erreur est mise en cache pour la
    /// même durée.
    pub fn sensors(&self) -> Result<Option<NvidiaSensors>> {
        self.sensors_at(Instant::now())
    }

    fn sensors_at(&self, now: Instant) -> Result<Option<NvidiaSensors>> {
        let Some(program) = self.program else {
            return Ok(None);
        };
        {
            let cache = self.lock_cache();
            if let Some(entry) = cache.sensors
                && now.saturating_duration_since(entry.fetched_at) < self.min_interval
            {
                return Ok(entry.value);
            }
            if let Some(failure) = &cache.sensors_failure
                && now < failure.until
            {
                return Err(failure.to_error());
            }
        }

        let result = (self.runner)(program, &QUERY_SENSORS, NVIDIA_SMI_TIMEOUT)
            .map(|output| parse_sensors(&output));

        let mut cache = self.lock_cache();
        match &result {
            Ok(value) => {
                cache.sensors = Some(CacheEntry {
                    value: *value,
                    fetched_at: now,
                });
                cache.sensors_failure = None;
            }
            Err(error) => {
                cache.sensors_failure = Some(CachedFailure::new(
                    program,
                    error,
                    now.checked_add(self.min_interval).unwrap_or(now),
                ));
            }
        }
        result
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, NvidiaCache> {
        match self.cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn parse_first_number(output: &CommandOutput) -> Option<f64> {
    let line = output.first_line()?;
    parse_number(line.split(',').next()?)
}

fn parse_sensors(output: &CommandOutput) -> Option<NvidiaSensors> {
    let line = output.first_line()?;
    let fields: Vec<&str> = line.split(',').collect();
    if fields.len() < 5 {
        return None;
    }
    Some(NvidiaSensors {
        power_draw_watts: parse_number(fields[0]),
        max_limit_watts: parse_number(fields[1]),
        enforced_limit_watts: parse_number(fields[2]),
        core_clock_mhz: parse_number(fields[3]),
        max_core_clock_mhz: parse_number(fields[4]),
    })
}

/// Extrait le premier nombre d'un champ texte (`"42.50 W"` → `42.5`).
///
/// Les champs `[N/A]` de `nvidia-smi` ne contiennent aucun chiffre et
/// retournent `None`.
fn parse_number(value: &str) -> Option<f64> {
    let value = value.trim();
    let mut start = None;
    let mut end = None;
    for (index, character) in value.char_indices() {
        let is_number = character.is_ascii_digit() || character == '.' || character == '-';
        match (is_number, start) {
            (true, None) => start = Some(index),
            (true, Some(_)) => end = Some(index + character.len_utf8()),
            (false, Some(_)) => break,
            (false, None) => {}
        }
    }
    let start = start?;
    let end = match end {
        Some(end) => end,
        None => {
            let character = value[start..].chars().next()?;
            start + character.len_utf8()
        }
    };
    value[start..end]
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite())
}

/// Contrôle de l'offset cTGP.
#[derive(Debug, Clone)]
pub struct CtgpControl {
    root: SysfsRoot,
    smi: NvidiaSmi,
}

impl CtgpControl {
    /// Crée le contrôleur et résout `nvidia-smi`.
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self {
            root,
            smi: NvidiaSmi::detect(),
        }
    }

    /// Construit le contrôleur avec un lecteur `nvidia-smi` fourni (tests).
    #[must_use]
    pub fn with_smi(root: SysfsRoot, smi: NvidiaSmi) -> Self {
        Self { root, smi }
    }

    /// Attribut `ctgp_offset` exposé par le firmware.
    #[must_use]
    pub fn attribute_available(&self) -> bool {
        self.root.is_file(CTGP_OFFSET_PATH)
    }

    /// Vrai si la fonctionnalité complète est disponible : attribut **et**
    /// `nvidia-smi`, comme `NVIDIAPowerCTRLListener.ctgpAvailable`.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.attribute_available() && self.smi.is_available()
    }

    /// Lecteur `nvidia-smi` associé.
    #[must_use]
    pub fn smi(&self) -> &NvidiaSmi {
        &self.smi
    }

    /// Offset cTGP courant, en watts.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si l'attribut est absent ;
    /// - erreur d'E/S ou valeur illisible.
    pub fn offset(&self) -> Result<Option<i32>> {
        if !self.attribute_available() {
            return Ok(None);
        }
        let raw = self.root.read_i64(CTGP_OFFSET_PATH)?;
        i32::try_from(raw)
            .map(Some)
            .map_err(|_| HwError::InvalidValue {
                context: "ctgp_offset",
                value: raw.to_string(),
            })
    }

    /// Applique un offset cTGP en watts.
    ///
    /// La valeur doit être entière et comprise dans
    /// `[0, MAX_CTGP_OFFSET_WATTS]`. Une valeur identique à la valeur courante
    /// n'est pas réécrite.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si l'attribut est absent ;
    /// - [`HwError::InvalidValue`] si la valeur n'est pas entière ;
    /// - [`HwError::OutOfRange`] si elle sort de la plage de sécurité ;
    /// - erreur d'E/S en cas d'échec d'écriture.
    pub fn set_offset_watts(&self, watts: f64) -> Result<()> {
        if !self.attribute_available() {
            return Err(HwError::NotSupported("ctgp_offset"));
        }
        if !watts.is_finite() {
            return Err(HwError::InvalidValue {
                context: "ctgp_offset",
                value: format!("{watts}"),
            });
        }
        let rounded = watts.round();
        if (watts - rounded).abs() > 0.001 {
            return Err(HwError::InvalidValue {
                context: "ctgp_offset (valeur non entière)",
                value: format!("{watts}"),
            });
        }
        if !(0.0..=MAX_CTGP_OFFSET_WATTS).contains(&rounded) {
            return Err(HwError::OutOfRange {
                context: "ctgp_offset",
                value: rounded as i64,
                min: 0,
                max: MAX_CTGP_OFFSET_WATTS as i64,
            });
        }
        let current = self.offset()?;
        let value = rounded as i32;
        if current == Some(value) {
            return Ok(());
        }
        self.root.write_i64(CTGP_OFFSET_PATH, i64::from(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn output(stdout: &str) -> CommandOutput {
        CommandOutput {
            stdout: stdout.to_owned(),
            stderr: String::new(),
            code: Some(0),
        }
    }

    #[test]
    fn ctgp_reads_and_writes_with_validation() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), CTGP_OFFSET_PATH, "15\n");
        let control = CtgpControl::with_smi(SysfsRoot::new(dir.path()), NvidiaSmi::default());
        assert!(control.attribute_available());
        assert_eq!(control.offset().unwrap(), Some(15));

        control.set_offset_watts(25.0).unwrap();
        assert_eq!(control.offset().unwrap(), Some(25));
        // Idempotent.
        control.set_offset_watts(25.0).unwrap();
        assert!(matches!(
            control.set_offset_watts(25.5),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            control.set_offset_watts(500.0),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            control.set_offset_watts(-5.0),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            control.set_offset_watts(f64::NAN),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn absent_attribute_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let control = CtgpControl::with_smi(SysfsRoot::new(dir.path()), NvidiaSmi::default());
        assert!(!control.attribute_available());
        assert_eq!(control.offset().unwrap(), None);
        assert!(matches!(
            control.set_offset_watts(10.0),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn number_parsing_handles_units_and_na() {
        assert_eq!(parse_number("42.50 W"), Some(42.5));
        assert_eq!(parse_number(" 1500 MHz"), Some(1500.0));
        assert_eq!(parse_number("[N/A]"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("2.5"), Some(2.5));
        assert_eq!(parse_number("0"), Some(0.0));
    }

    #[test]
    fn sensors_parsing_uses_first_line() {
        let parsed =
            parse_sensors(&output("45.32 W, 115.00 W, 100.00 W, 2100 MHz, 2610 MHz\n")).unwrap();
        assert_eq!(parsed.power_draw_watts, Some(45.32));
        assert_eq!(parsed.max_limit_watts, Some(115.0));
        assert_eq!(parsed.enforced_limit_watts, Some(100.0));
        assert_eq!(parsed.core_clock_mhz, Some(2100.0));
        assert_eq!(parsed.max_core_clock_mhz, Some(2610.0));

        assert!(parse_sensors(&output("too,few,fields\n")).is_none());
        assert!(parse_sensors(&output("")).is_none());
    }

    #[test]
    fn power_limits_parse_first_number() {
        assert_eq!(parse_first_number(&output("80\n")), Some(80.0));
        assert_eq!(parse_first_number(&output("")), None);
        assert_eq!(parse_first_number(&output("[N/A]\n")), None);
    }

    #[test]
    fn absent_nvidia_smi_is_not_an_error() {
        let smi = NvidiaSmi::default();
        if smi.is_available() {
            // Machine de développement avec NVIDIA : on ne teste pas l'absence.
            return;
        }
        assert!(smi.power_limits().unwrap().is_none());
        assert!(smi.sensors().unwrap().is_none());
    }

    /// Chemin factice : le lanceur injecté ignore le programme.
    const FAKE_PROGRAM: &str = "/usr/bin/nvidia-smi";

    /// Construit un `NvidiaSmi` dont le lanceur renvoie `stdout` et compte ses
    /// appels. Aucun processus, fichier ou attente : le cache est testé de
    /// façon purement déterministe (le `ETXTBSY` des scripts temporaires en
    /// exécution parallèle ne peut plus se produire).
    fn fake_smi(stdout: &str, interval: Duration) -> (NvidiaSmi, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let output = output(stdout);
        let runner: CommandRunner = Arc::new(move |_program, _args, _timeout| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(output.clone())
        });
        (
            NvidiaSmi::with_runner(Some(FAKE_PROGRAM), interval, runner),
            calls,
        )
    }

    /// Variante en échec : le lanceur retourne une erreur typée.
    fn failing_smi(interval: Duration) -> (NvidiaSmi, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let runner: CommandRunner = Arc::new(move |program, _args, _timeout| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(HwError::External {
                program,
                reason: "boom".to_owned(),
            })
        });
        (
            NvidiaSmi::with_runner(Some(FAKE_PROGRAM), interval, runner),
            calls,
        )
    }

    #[test]
    fn nvidia_smi_sensors_are_cached_between_ticks() {
        let (smi, calls) = fake_smi(
            "45.00 W, 115.00 W, 100.00 W, 2100 MHz, 2610 MHz\n",
            Duration::from_secs(10),
        );
        let start = Instant::now();

        let first = smi.sensors_at(start).unwrap().unwrap();
        // Un tick de 2 s plus tard : le cache répond, aucun nouvel appel.
        let second = smi
            .sensors_at(start + Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Au-delà de l'intervalle, un seul nouvel appel.
        let third = smi.sensors_at(start + Duration::from_secs(11)).unwrap();
        assert!(third.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn nvidia_smi_failures_are_cached_too() {
        let (smi, calls) = failing_smi(Duration::from_secs(10));
        let start = Instant::now();

        assert!(smi.sensors_at(start).is_err());
        // Deuxième appel dans l'intervalle : erreur rejouée sans appel.
        assert!(smi.sensors_at(start + Duration::from_secs(1)).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        assert!(smi.sensors_at(start + Duration::from_secs(11)).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn nvidia_smi_empty_result_is_cached() {
        let (smi, calls) = fake_smi("\n", Duration::from_secs(10));
        let start = Instant::now();
        assert!(smi.sensors_at(start).unwrap().is_none());
        assert!(
            smi.sensors_at(start + Duration::from_secs(2))
                .unwrap()
                .is_none()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn nvidia_smi_limits_are_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let runner: CommandRunner = Arc::new(move |_program, _args, _timeout| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(output("80\n"))
        });
        let smi = NvidiaSmi::with_runner(Some(FAKE_PROGRAM), Duration::from_secs(10), runner);
        let start = Instant::now();

        let limits = smi.power_limits_at(start).unwrap().unwrap();
        assert_eq!(limits.default_watts, 80.0);
        assert_eq!(limits.max_watts, 80.0);
        // Deux requêtes par rafraîchissement (défaut + maximum).
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        assert!(
            smi.power_limits_at(start + Duration::from_secs(5))
                .unwrap()
                .is_some()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2, "valeur encore fraîche");

        assert!(
            smi.power_limits_at(start + Duration::from_secs(11))
                .unwrap()
                .is_some()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn zero_interval_disables_cache() {
        let smi = NvidiaSmi::default().with_min_interval(Duration::ZERO);
        assert_eq!(smi.min_interval(), Duration::ZERO);
        assert_eq!(
            NvidiaSmi::default().min_interval(),
            NVIDIA_MIN_QUERY_INTERVAL
        );
    }
}
