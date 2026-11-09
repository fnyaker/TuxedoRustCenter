// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Le code est une
// réimplémentation originale de la logique de courbes de ventilation de
// TUXEDO Control Center (GPL-3.0-or-later). Aucun fichier source de
// l'implémentation Electron/Node.js n'a été copié.

//! Courbes de ventilation pures.
//!
//! Ce module ne touche à aucun matériel : il transforme une température en
//! pourcentage de vitesse, puis applique les limites configurées par le
//! profil. L'écriture sysfs/hwmon/ioctl appartient à `tcc-hw`.
//!
//! # Sémantique
//!
//! - une table associe des points `(température °C, vitesse %)` ;
//! - la vitesse est interpolée linéairement entre deux points, puis bornée à
//!   `0..=100` ;
//! - sous le premier point (resp. au-dessus du dernier), la vitesse du point
//!   extrême est utilisée ;
//! - l'offset est ajouté **avant** le clamp min/max du profil, comme dans
//!   l'original ;
//! - une table vide, une température non finie ou une table dont aucune
//!   entrée n'est exploitable donne `None` plutôt qu'une valeur arbitraire.
//!
//! L'hystérésis optionnelle de [`FanCurveEvaluator`] est un simple seuil en
//! pourcentage : tant que la nouvelle consigne ne s'écarte pas de la
//! précédente d'au moins ce seuil, la vitesse précédente est conservée. Elle
//! évite les oscillations d'un ventilateur autour d'un point de consigne sans
//! introduire de temporisation (pas d'horloge, donc testable).

use serde::{Deserialize, Serialize};

/// Vitesse minimale d'un ventilateur, en pourcentage.
pub const FAN_SPEED_MIN_PERCENT: f32 = 0.0;
/// Vitesse maximale d'un ventilateur, en pourcentage.
pub const FAN_SPEED_MAX_PERCENT: f32 = 100.0;
/// Offset minimal accepté par les profils, en pourcentage.
pub const FAN_OFFSET_MIN_PERCENT: f32 = -100.0;
/// Offset maximal accepté par les profils, en pourcentage.
pub const FAN_OFFSET_MAX_PERCENT: f32 = 100.0;
/// Nombre maximal de points dans une table de ventilation.
pub const MAX_FAN_TABLE_ENTRIES: usize = 256;

/// Cible d'une courbe de ventilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FanTarget {
    /// Ventilateur associé au CPU.
    Cpu,
    /// Ventilateur associé au GPU.
    Gpu,
}

/// Point d'une table de ventilation.
///
/// La sérialisation reprend les clés de l'original : `temp` et `speed`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanTableEntry {
    /// Température du point, en degrés Celsius.
    pub temp: f32,
    /// Vitesse du point, en pourcentage (`0..=100`).
    pub speed: f32,
}

impl Default for FanTableEntry {
    fn default() -> Self {
        Self {
            temp: 0.0,
            speed: 0.0,
        }
    }
}

impl FanTableEntry {
    /// Crée un point de courbe.
    #[must_use]
    pub fn new(temp: f32, speed: f32) -> Self {
        Self { temp, speed }
    }
}

/// Courbe de ventilation CPU/GPU nommée.
///
/// Les clés JSON sont `tableCPU` et `tableGPU`, identiques à l'original. Le
/// nom est optionnel car les courbes importées peuvent ne pas en avoir ; la
/// migration en génère un avant écriture.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanCurve {
    /// Nom de la courbe, utilisé par `Profile::fan.fanProfile`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Table du ventilateur CPU.
    #[serde(rename = "tableCPU")]
    pub table_cpu: Vec<FanTableEntry>,
    /// Table du ventilateur GPU.
    #[serde(rename = "tableGPU")]
    pub table_gpu: Vec<FanTableEntry>,
}

impl FanCurve {
    /// Crée une courbe nommée.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        table_cpu: Vec<FanTableEntry>,
        table_gpu: Vec<FanTableEntry>,
    ) -> Self {
        Self {
            name: Some(name.into()),
            table_cpu,
            table_gpu,
        }
    }

    /// Courbe personnalisée par défaut de TCC.
    ///
    /// Ces valeurs reproduisent la courbe `customFanPreset` de l'original
    /// (GPL-3.0-or-later) pour conserver un comportement de départ identique.
    #[must_use]
    pub fn default_custom() -> Self {
        let points = [
            (20.0, 12.0),
            (30.0, 14.0),
            (40.0, 22.0),
            (50.0, 35.0),
            (60.0, 44.0),
            (70.0, 56.0),
            (80.0, 79.0),
            (90.0, 85.0),
            (100.0, 90.0),
        ];
        let table: Vec<FanTableEntry> = points
            .iter()
            .map(|&(temp, speed)| FanTableEntry::new(temp, speed))
            .collect();
        Self {
            name: None,
            table_cpu: table.clone(),
            table_gpu: table,
        }
    }

    /// Table correspondant à la cible, ou une tranche vide.
    #[must_use]
    pub fn table(&self, target: FanTarget) -> &[FanTableEntry] {
        match target {
            FanTarget::Cpu => &self.table_cpu,
            FanTarget::Gpu => &self.table_gpu,
        }
    }

    /// Vrai si les deux tables sont vides.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.table_cpu.is_empty() && self.table_gpu.is_empty()
    }

    /// Vrai si la table de la cible contient au moins un point.
    #[must_use]
    pub fn has_table(&self, target: FanTarget) -> bool {
        !self.table(target).is_empty()
    }

    /// Trie les tables par température croissante.
    ///
    /// Renvoie `true` si une table a effectivement été modifiée. Les points
    /// dont la température ou la vitesse n'est pas finie ne sont pas
    /// supprimés (la validation les rejette explicitement) mais sont placés
    /// en fin de table par `total_cmp`.
    pub fn normalize(&mut self) -> bool {
        let mut changed = false;
        for table in [&mut self.table_cpu, &mut self.table_gpu] {
            let before: Vec<(f32, f32)> = table
                .iter()
                .map(|entry| (entry.temp, entry.speed))
                .collect();
            table.sort_by(|a, b| a.temp.total_cmp(&b.temp));
            let after: Vec<(f32, f32)> = table
                .iter()
                .map(|entry| (entry.temp, entry.speed))
                .collect();
            changed |= before != after;
        }
        changed
    }

    /// Interpole la vitesse pour une température et une cible.
    ///
    /// Renvoie `None` si la table est vide, si la température n'est pas finie
    /// ou si aucune entrée exploitable n'existe. Le résultat est toujours
    /// borné à `0..=100`.
    #[must_use]
    pub fn speed_at(&self, target: FanTarget, temp_c: f32) -> Option<f32> {
        if !temp_c.is_finite() {
            return None;
        }

        let mut lower: Option<&FanTableEntry> = None;
        let mut upper: Option<&FanTableEntry> = None;
        for entry in self.table(target) {
            if !entry.temp.is_finite() || !entry.speed.is_finite() {
                continue;
            }
            if entry.temp <= temp_c && lower.is_none_or(|current| entry.temp > current.temp) {
                lower = Some(entry);
            }
            if entry.temp >= temp_c && upper.is_none_or(|current| entry.temp < current.temp) {
                upper = Some(entry);
            }
        }

        let clamp_speed = |entry: &FanTableEntry| {
            entry
                .speed
                .clamp(FAN_SPEED_MIN_PERCENT, FAN_SPEED_MAX_PERCENT)
        };

        let speed = match (lower, upper) {
            (Some(low), Some(high)) => {
                let span = high.temp - low.temp;
                if span.abs() <= f32::EPSILON {
                    clamp_speed(high)
                } else {
                    let low_speed = clamp_speed(low);
                    let high_speed = clamp_speed(high);
                    low_speed + (high_speed - low_speed) * (temp_c - low.temp) / span
                }
            }
            (Some(low), None) => clamp_speed(low),
            (None, Some(high)) => clamp_speed(high),
            (None, None) => return None,
        };

        Some(speed.clamp(FAN_SPEED_MIN_PERCENT, FAN_SPEED_MAX_PERCENT))
    }

    /// Bornes de température effectivement couvertes par la table.
    #[must_use]
    pub fn temperature_range(&self, target: FanTarget) -> Option<(f32, f32)> {
        let mut range: Option<(f32, f32)> = None;
        for entry in self.table(target) {
            if !entry.temp.is_finite() {
                continue;
            }
            range = Some(match range {
                None => (entry.temp, entry.temp),
                Some((min, max)) => (min.min(entry.temp), max.max(entry.temp)),
            });
        }
        range
    }
}

/// Limites de vitesse appliquées après interpolation.
///
/// Les champs reprennent les clés `minimumFanspeed`, `maximumFanspeed` et
/// `offsetFanspeed` des profils originaux.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanLimits {
    /// Vitesse minimale imposée, en pourcentage.
    pub minimum_fanspeed: f32,
    /// Vitesse maximale imposée, en pourcentage.
    pub maximum_fanspeed: f32,
    /// Offset ajouté à la vitesse interpolée, en pourcentage.
    pub offset_fanspeed: f32,
}

impl Default for FanLimits {
    fn default() -> Self {
        Self {
            minimum_fanspeed: FAN_SPEED_MIN_PERCENT,
            maximum_fanspeed: FAN_SPEED_MAX_PERCENT,
            offset_fanspeed: 0.0,
        }
    }
}

impl FanLimits {
    /// Crée des limites explicites.
    #[must_use]
    pub fn new(minimum_fanspeed: f32, maximum_fanspeed: f32, offset_fanspeed: f32) -> Self {
        Self {
            minimum_fanspeed,
            maximum_fanspeed,
            offset_fanspeed,
        }
    }

    /// Applique l'offset puis les bornes min/max et le clamp `0..=100`.
    ///
    /// La fonction ne panique jamais, même avec des valeurs non finies ou des
    /// bornes inversées : une valeur non finie est traitée comme la borne
    /// minimale, et des bornes inversées sont remises dans l'ordre. La
    /// validation des profils reste la source de vérité pour refuser ces
    /// configurations en amont.
    #[must_use]
    pub fn apply(&self, speed_percent: f32) -> f32 {
        let base = if speed_percent.is_finite() {
            speed_percent
        } else {
            FAN_SPEED_MIN_PERCENT
        };

        let offset = if self.offset_fanspeed.is_finite() {
            self.offset_fanspeed
                .clamp(FAN_OFFSET_MIN_PERCENT, FAN_OFFSET_MAX_PERCENT)
        } else {
            0.0
        };

        let minimum = if self.minimum_fanspeed.is_finite() {
            self.minimum_fanspeed
                .clamp(FAN_SPEED_MIN_PERCENT, FAN_SPEED_MAX_PERCENT)
        } else {
            FAN_SPEED_MIN_PERCENT
        };
        let maximum = if self.maximum_fanspeed.is_finite() {
            self.maximum_fanspeed
                .clamp(FAN_SPEED_MIN_PERCENT, FAN_SPEED_MAX_PERCENT)
        } else {
            FAN_SPEED_MAX_PERCENT
        };
        let (low, high) = if minimum <= maximum {
            (minimum, maximum)
        } else {
            (maximum, minimum)
        };

        (base + offset)
            .clamp(FAN_SPEED_MIN_PERCENT, FAN_SPEED_MAX_PERCENT)
            .clamp(low, high)
    }
}

/// Évaluateur avec état, chargé d'appliquer une courbe et une hystérésis.
///
/// L'état se limite à la dernière vitesse appliquée. Aucun accès matériel,
/// aucune horloge : l'évaluateur est déterministe et testable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanCurveEvaluator {
    limits: FanLimits,
    min_change_percent: f32,
    last_speed: Option<f32>,
}

impl Default for FanCurveEvaluator {
    fn default() -> Self {
        Self::new(FanLimits::default())
    }
}

impl FanCurveEvaluator {
    /// Crée un évaluateur sans hystérésis.
    #[must_use]
    pub fn new(limits: FanLimits) -> Self {
        Self {
            limits,
            min_change_percent: 0.0,
            last_speed: None,
        }
    }

    /// Crée un évaluateur avec un seuil d'hystérésis en pourcentage.
    ///
    /// Une valeur non finie ou négative désactive l'hystérésis.
    #[must_use]
    pub fn with_hysteresis(limits: FanLimits, min_change_percent: f32) -> Self {
        let mut evaluator = Self::new(limits);
        evaluator.set_hysteresis(min_change_percent);
        evaluator
    }

    /// Limites actuellement appliquées.
    #[must_use]
    pub fn limits(&self) -> FanLimits {
        self.limits
    }

    /// Remplace les limites sans réinitialiser la vitesse courante.
    pub fn set_limits(&mut self, limits: FanLimits) {
        self.limits = limits;
    }

    /// Seuil d'hystérésis courant, en pourcentage.
    #[must_use]
    pub fn hysteresis(&self) -> f32 {
        self.min_change_percent
    }

    /// Définit le seuil d'hystérésis (valeur négative ou non finie : zéro).
    pub fn set_hysteresis(&mut self, min_change_percent: f32) {
        self.min_change_percent = if min_change_percent.is_finite() {
            min_change_percent.max(0.0)
        } else {
            0.0
        };
    }

    /// Dernière vitesse appliquée, si une évaluation a déjà eu lieu.
    #[must_use]
    pub fn last_speed(&self) -> Option<f32> {
        self.last_speed
    }

    /// Oublie la dernière vitesse (par exemple au changement de profil).
    pub fn reset(&mut self) {
        self.last_speed = None;
    }

    /// Évalue la courbe pour une cible et une température.
    ///
    /// Renvoie `None` si la courbe ne permet pas de calculer une vitesse. Une
    /// valeur `Some` est toujours finie et dans `0..=100`.
    pub fn evaluate(&mut self, curve: &FanCurve, target: FanTarget, temp_c: f32) -> Option<f32> {
        let interpolated = curve.speed_at(target, temp_c)?;
        let target_speed = self.limits.apply(interpolated);

        let applied = match self.last_speed {
            Some(last)
                if last.is_finite() && (target_speed - last).abs() < self.min_change_percent =>
            {
                last
            }
            _ => target_speed,
        };

        self.last_speed = Some(applied);
        Some(applied)
    }
}

/// Ensemble de courbes nommées, tel que stocké dans `/etc/tcc/fantables`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FanTableSet {
    curves: Vec<FanCurve>,
}

impl FanTableSet {
    /// Construit un ensemble à partir de courbes existantes.
    #[must_use]
    pub fn new(curves: Vec<FanCurve>) -> Self {
        Self { curves }
    }

    /// Courbes contenues dans l'ensemble.
    #[must_use]
    pub fn as_slice(&self) -> &[FanCurve] {
        &self.curves
    }

    /// Itère sur les courbes.
    pub fn iter(&self) -> std::slice::Iter<'_, FanCurve> {
        self.curves.iter()
    }

    /// Nombre de courbes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.curves.len()
    }

    /// Vrai si l'ensemble est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }

    /// Cherche une courbe par son nom exact.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FanCurve> {
        self.curves
            .iter()
            .find(|curve| curve.name.as_deref() == Some(name))
    }

    /// Noms des courbes, dans l'ordre de stockage.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.curves.iter().filter_map(|curve| curve.name.as_deref())
    }

    /// Ajoute une courbe, ou remplace celle portant déjà le même nom.
    ///
    /// Une courbe sans nom est toujours ajoutée (elle ne peut pas être
    /// sélectionnée par nom, mais la migration attribue un nom avant
    /// écriture).
    pub fn upsert(&mut self, curve: FanCurve) {
        if let Some(name) = curve.name.as_deref()
            && let Some(existing) = self
                .curves
                .iter_mut()
                .find(|existing| existing.name.as_deref() == Some(name))
        {
            *existing = curve;
            return;
        }
        self.curves.push(curve);
    }

    /// Retire et renvoie la courbe portant ce nom.
    pub fn remove(&mut self, name: &str) -> Option<FanCurve> {
        let index = self
            .curves
            .iter()
            .position(|curve| curve.name.as_deref() == Some(name))?;
        Some(self.curves.remove(index))
    }

    /// Valide toutes les courbes et l'unicité des noms.
    ///
    /// # Errors
    ///
    /// Renvoie une [`crate::ValidationError`] si une courbe est invalide ou si
    /// deux courbes portent le même nom.
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        crate::validate::validate_fan_tables(self)
    }
}

impl From<Vec<FanCurve>> for FanTableSet {
    fn from(curves: Vec<FanCurve>) -> Self {
        Self::new(curves)
    }
}

impl FromIterator<FanCurve> for FanTableSet {
    fn from_iter<T: IntoIterator<Item = FanCurve>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{FanCurve, FanCurveEvaluator, FanLimits, FanTableEntry, FanTableSet, FanTarget};

    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "attendu {expected}, obtenu {actual}"
        );
    }

    fn curve() -> FanCurve {
        FanCurve {
            name: Some("test".into()),
            table_cpu: vec![
                FanTableEntry::new(20.0, 10.0),
                FanTableEntry::new(40.0, 30.0),
                FanTableEntry::new(60.0, 50.0),
            ],
            table_gpu: vec![FanTableEntry::new(30.0, 20.0)],
        }
    }

    #[test]
    fn interpolates_between_points() {
        let curve = curve();
        approx(curve.speed_at(FanTarget::Cpu, 30.0).unwrap(), 20.0);
        approx(curve.speed_at(FanTarget::Cpu, 50.0).unwrap(), 40.0);
    }

    #[test]
    fn exact_points_are_returned_unchanged() {
        let curve = curve();
        approx(curve.speed_at(FanTarget::Cpu, 20.0).unwrap(), 10.0);
        approx(curve.speed_at(FanTarget::Cpu, 40.0).unwrap(), 30.0);
    }

    #[test]
    fn clamps_outside_table_bounds() {
        let curve = curve();
        approx(curve.speed_at(FanTarget::Cpu, -20.0).unwrap(), 10.0);
        approx(curve.speed_at(FanTarget::Cpu, 120.0).unwrap(), 50.0);
    }

    #[test]
    fn single_point_table_is_constant() {
        let curve = curve();
        approx(curve.speed_at(FanTarget::Gpu, 0.0).unwrap(), 20.0);
        approx(curve.speed_at(FanTarget::Gpu, 100.0).unwrap(), 20.0);
    }

    #[test]
    fn empty_table_returns_none() {
        let curve = FanCurve::default();
        assert_eq!(curve.speed_at(FanTarget::Cpu, 50.0), None);
        assert!(curve.is_empty());
    }

    #[test]
    fn non_finite_temperature_returns_none() {
        let curve = curve();
        assert_eq!(curve.speed_at(FanTarget::Cpu, f32::NAN), None);
        assert_eq!(curve.speed_at(FanTarget::Cpu, f32::INFINITY), None);
    }

    #[test]
    fn unsorted_tables_are_interpolated_correctly() {
        let curve = FanCurve {
            name: None,
            table_cpu: vec![
                FanTableEntry::new(60.0, 50.0),
                FanTableEntry::new(20.0, 10.0),
                FanTableEntry::new(40.0, 30.0),
            ],
            table_gpu: Vec::new(),
        };
        approx(curve.speed_at(FanTarget::Cpu, 30.0).unwrap(), 20.0);
        approx(curve.speed_at(FanTarget::Cpu, 70.0).unwrap(), 50.0);
    }

    #[test]
    fn normalize_sorts_tables_and_reports_change() {
        let mut curve = FanCurve {
            name: None,
            table_cpu: vec![
                FanTableEntry::new(60.0, 50.0),
                FanTableEntry::new(20.0, 10.0),
            ],
            table_gpu: Vec::new(),
        };
        assert!(curve.normalize());
        assert_eq!(curve.table_cpu[0].temp, 20.0);
        assert!(!curve.normalize());
    }

    #[test]
    fn speed_is_clamped_to_percent_range() {
        let curve = FanCurve {
            name: None,
            table_cpu: vec![
                FanTableEntry::new(20.0, -10.0),
                FanTableEntry::new(40.0, 140.0),
            ],
            table_gpu: Vec::new(),
        };
        approx(curve.speed_at(FanTarget::Cpu, 20.0).unwrap(), 0.0);
        approx(curve.speed_at(FanTarget::Cpu, 40.0).unwrap(), 100.0);
        approx(curve.speed_at(FanTarget::Cpu, 30.0).unwrap(), 50.0);
    }

    #[test]
    fn limits_apply_offset_then_bounds() {
        let limits = FanLimits::new(20.0, 80.0, 10.0);
        approx(limits.apply(0.0), 20.0);
        approx(limits.apply(50.0), 60.0);
        approx(limits.apply(90.0), 80.0);
    }

    #[test]
    fn reversed_limits_are_tolerated() {
        let limits = FanLimits::new(80.0, 20.0, 0.0);
        approx(limits.apply(50.0), 50.0);
        approx(limits.apply(100.0), 80.0);
    }

    #[test]
    fn non_finite_limit_inputs_do_not_panic() {
        let limits = FanLimits::new(f32::NAN, f32::INFINITY, f32::NAN);
        let value = limits.apply(f32::NAN);
        assert!(value.is_finite());
        assert!((0.0..=100.0).contains(&value));
    }

    #[test]
    fn evaluator_applies_limits() {
        let mut evaluator = FanCurveEvaluator::new(FanLimits::new(25.0, 75.0, 5.0));
        let speed = evaluator.evaluate(&curve(), FanTarget::Cpu, 20.0).unwrap();
        approx(speed, 25.0);
        assert_eq!(evaluator.last_speed(), Some(speed));
    }

    #[test]
    fn evaluator_hysteresis_keeps_previous_speed() {
        let mut evaluator = FanCurveEvaluator::with_hysteresis(FanLimits::default(), 10.0);
        let first = evaluator.evaluate(&curve(), FanTarget::Cpu, 30.0).unwrap();
        approx(first, 20.0);

        // 21 % ne s'écarte que de 1 point : la vitesse précédente est gardée.
        let second = evaluator.evaluate(&curve(), FanTarget::Cpu, 31.0).unwrap();
        approx(second, 20.0);

        // 40 % s'écarte de 20 points : la nouvelle valeur est appliquée.
        let third = evaluator.evaluate(&curve(), FanTarget::Cpu, 50.0).unwrap();
        approx(third, 40.0);
    }

    #[test]
    fn evaluator_reset_forgets_last_speed() {
        let mut evaluator = FanCurveEvaluator::new(FanLimits::default());
        assert!(evaluator.evaluate(&curve(), FanTarget::Cpu, 30.0).is_some());
        assert!(evaluator.last_speed().is_some());
        evaluator.reset();
        assert_eq!(evaluator.last_speed(), None);
    }

    #[test]
    fn default_custom_curve_matches_original_preset() {
        let curve = FanCurve::default_custom();
        assert_eq!(curve.table_cpu.len(), 9);
        approx(curve.speed_at(FanTarget::Cpu, 65.0).unwrap(), 50.0);
        approx(curve.speed_at(FanTarget::Gpu, 100.0).unwrap(), 90.0);
    }

    #[test]
    fn fan_table_set_upserts_and_removes_by_name() {
        let mut set = FanTableSet::default();
        set.upsert(curve());
        assert_eq!(set.len(), 1);
        assert!(set.get("test").is_some());

        let mut replacement = curve();
        replacement.table_cpu.clear();
        set.upsert(replacement);
        assert_eq!(set.len(), 1);
        assert!(set.get("test").unwrap().table_cpu.is_empty());

        assert!(set.remove("test").is_some());
        assert!(set.remove("test").is_none());
        assert!(set.is_empty());
    }

    #[test]
    fn fan_tables_serde_uses_original_keys() {
        let json = serde_json::to_string(&curve()).unwrap();
        assert!(json.contains("\"tableCPU\""));
        assert!(json.contains("\"tableGPU\""));
        let parsed: FanCurve = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, curve());
    }
}
