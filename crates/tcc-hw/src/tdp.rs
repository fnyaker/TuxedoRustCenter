// SPDX-License-Identifier: GPL-3.0-or-later
//! Limites de puissance TDP (PL1/PL2/PL4) via `/dev/tuxedo_io` (Uniwill).
//!
//! L'original (`ODMPowerLimitWorker` + `TDPInfo` de l'addon natif) expose 0 à
//! 3 limites selon le firmware. Chaque limite possède une valeur courante, un
//! minimum et un maximum lus par ioctl :
//!
//! ```text
//! R_UW_TDP{0,1,2}       valeur courante (W)
//! R_UW_TDP{0,1,2}_MIN   borne basse (W)
//! R_UW_TDP{0,1,2}_MAX   borne haute (W)
//! W_UW_TDP{0,1,2}       écriture (W)
//! ```
//!
//! Les descripteurs sont `pl1`, `pl2`, `pl4` (ordre du firmware). Clevo ne
//! expose pas de TDP : [`TdpControl::limits`] retourne une liste vide.
//!
//! # Validation
//!
//! - chaque valeur doit être **entière** (le profil stocke des `f64`) et tenir
//!   dans un `i32` ;
//! - chaque valeur doit appartenir à `[min, max]` du firmware, bornes incluses ;
//! - la séquence fournie doit être **croissante ou égale** (`pl1 <= pl2 <=
//!   pl4`). L'original n'effectuait pas ce contrôle ; un PL1 supérieur au PL4
//!   est presque toujours une erreur de configuration et peut provoquer des
//!   oscillations de l'EC. Les limites du firmware restent la première barrière.
//! - une liste vide applique les **maxima** du firmware. C'est le comportement
//!   de `Admin.Hw.SetTdpValues([])` explicite ; le worker de profil, lui,
//!   n'appelle pas `set_values` quand `tdpValues` est vide (« ne pas changer »).
//!
//! Une écriture échouée n'est pas rollbackée : les limites TDP ne sont pas
//! transactionnelles côté EC. L'erreur est retournée avec l'index concerné et
//! le daemon peut réappliquer le profil. Documenté dans
//! `docs/hardware/HARDWARE_MATRIX.md`.

use std::sync::Arc;

use crate::error::{HwError, Result};
use crate::tuxedo_io::{Interface, TuxedoIo};

/// Nombre maximal de limites TDP exposées.
pub const MAX_TDP_LIMITS: usize = 3;
/// Descripteurs, dans l'ordre du firmware.
pub const TDP_DESCRIPTORS: [&str; MAX_TDP_LIMITS] = ["pl1", "pl2", "pl4"];

/// Limite TDP exposée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdpLimit {
    /// Descripteur (`pl1`, `pl2`, `pl4`).
    pub descriptor: &'static str,
    /// Valeur courante en watts.
    pub current: i32,
    /// Borne basse du firmware en watts.
    pub min: i32,
    /// Borne haute du firmware en watts.
    pub max: i32,
}

impl TdpLimit {
    /// Vrai si une valeur est acceptable pour cette limite.
    #[must_use]
    pub fn accepts(&self, watts: i32) -> bool {
        watts >= self.min && watts <= self.max
    }
}

/// Contrôleur TDP adossé à une interface `tuxedo-io`.
#[derive(Debug, Clone)]
pub struct TdpControl {
    io: Arc<TuxedoIo>,
}

impl TdpControl {
    /// Construit le contrôleur sur une interface déjà ouverte.
    #[must_use]
    pub fn new(io: Arc<TuxedoIo>) -> Self {
        Self { io }
    }

    /// Interface matérielle sous-jacente.
    #[must_use]
    pub fn interface(&self) -> Interface {
        self.io.interface()
    }

    /// Vrai si le firmware expose au moins une limite.
    ///
    /// # Errors
    ///
    /// Erreur ioctl lors de la détection du nombre de limites.
    pub fn is_available(&self) -> Result<bool> {
        Ok(self.io.tdp_count()? > 0)
    }

    /// Lit toutes les limites exposées (0 à 3).
    ///
    /// # Errors
    ///
    /// Erreur ioctl si une lecture échoue.
    pub fn limits(&self) -> Result<Vec<TdpLimit>> {
        let count = usize::from(self.io.tdp_count()?).min(MAX_TDP_LIMITS);
        let mut limits = Vec::with_capacity(count);
        for index in 0..count {
            let index = u8::try_from(index).map_err(|_| HwError::OutOfRange {
                context: "index TDP",
                value: index as i64,
                min: 0,
                max: (MAX_TDP_LIMITS - 1) as i64,
            })?;
            limits.push(TdpLimit {
                descriptor: TDP_DESCRIPTORS[usize::from(index)],
                current: self.io.tdp(index)?,
                min: self.io.tdp_min(index)?,
                max: self.io.tdp_max(index)?,
            });
        }
        Ok(limits)
    }

    /// Applique les valeurs fournies.
    ///
    /// Une liste vide applique les maxima du firmware (action explicite
    /// `Admin.Hw.SetTdpValues([])`). Le worker de profil ne l'utilise pas pour
    /// une liste vide : il conserve l'état firmware. Une liste plus courte que
    /// le nombre de limites n'écrit que les premières, comme l'original.
    /// Toutes les valeurs sont validées **avant** la première écriture.
    ///
    /// # Errors
    ///
    /// - [`HwError::NotSupported`] si aucune limite n'est exposée ;
    /// - [`HwError::InvalidValue`] si une valeur n'est pas entière, sort de la
    ///   plage firmware ou casse la monotonie ;
    /// - [`HwError::Ioctl`] en cas d'échec d'écriture.
    pub fn set_values(&self, values: &[f64]) -> Result<Vec<TdpLimit>> {
        let limits = self.limits()?;
        if limits.is_empty() {
            return Err(HwError::NotSupported("TDP"));
        }
        if values.is_empty() {
            return self.apply_watts(
                &limits,
                &limits.iter().map(|limit| limit.max).collect::<Vec<_>>(),
            );
        }
        if values.len() > limits.len() {
            return Err(HwError::InvalidValue {
                context: "TDP (trop de valeurs)",
                value: format!("{} > {}", values.len(), limits.len()),
            });
        }
        let mut watts = Vec::with_capacity(values.len());
        for value in values.iter().copied() {
            watts.push(to_watts(value)?);
        }
        for (index, value) in watts.iter().copied().enumerate() {
            if !limits[index].accepts(value) {
                return Err(HwError::OutOfRange {
                    context: "TDP",
                    value: i64::from(value),
                    min: i64::from(limits[index].min),
                    max: i64::from(limits[index].max),
                });
            }
            if let Some(previous) = index
                .checked_sub(1)
                .and_then(|previous| watts.get(previous))
                && value < *previous
            {
                return Err(HwError::InvalidValue {
                    context: "TDP (séquence non croissante)",
                    value: format!("{:?}", watts),
                });
            }
        }
        self.apply_watts(&limits, &watts)
    }

    /// Applique les maxima du firmware.
    ///
    /// # Errors
    ///
    /// Voir [`TdpControl::set_values`].
    pub fn apply_defaults(&self) -> Result<Vec<TdpLimit>> {
        let limits = self.limits()?;
        if limits.is_empty() {
            return Err(HwError::NotSupported("TDP"));
        }
        let maxima: Vec<i32> = limits.iter().map(|limit| limit.max).collect();
        self.apply_watts(&limits, &maxima)
    }

    fn apply_watts(&self, limits: &[TdpLimit], watts: &[i32]) -> Result<Vec<TdpLimit>> {
        let mut applied = limits.to_vec();
        for (index, value) in watts.iter().copied().enumerate() {
            let Some(limit) = applied.get_mut(index) else {
                break;
            };
            if limit.current != value {
                self.io.set_tdp(index as u8, value)?;
                limit.current = value;
            }
        }
        Ok(applied)
    }
}

/// Convertit une valeur de profil en watts entiers.
fn to_watts(value: f64) -> Result<i32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(i32::MAX) {
        return Err(HwError::InvalidValue {
            context: "TDP",
            value: format!("{value}"),
        });
    }
    let rounded = value.round();
    if (value - rounded).abs() > 0.001 {
        return Err(HwError::InvalidValue {
            context: "TDP (valeur non entière)",
            value: format!("{value}"),
        });
    }
    i32::try_from(rounded as i64).map_err(|_| HwError::InvalidValue {
        context: "TDP (hors i32)",
        value: format!("{value}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuxedo_io::fake::FakeTransport;

    fn uniwill() -> Arc<TuxedoIo> {
        Arc::new(TuxedoIo::with_transport(Box::new(FakeTransport::uniwill())).unwrap())
    }

    fn clevo() -> Arc<TuxedoIo> {
        Arc::new(TuxedoIo::with_transport(Box::new(FakeTransport::clevo())).unwrap())
    }

    #[test]
    fn reads_limits_in_firmware_order() {
        let control = TdpControl::new(uniwill());
        let limits = control.limits().unwrap();
        assert_eq!(limits.len(), 3);
        assert_eq!(limits[0].descriptor, "pl1");
        assert_eq!(limits[1].descriptor, "pl2");
        assert_eq!(limits[2].descriptor, "pl4");
        assert_eq!(limits[0].current, 35);
        assert_eq!(limits[0].min, 10);
        assert_eq!(limits[0].max, 80);
        assert!(control.is_available().unwrap());
    }

    /// Chemin explicite `Admin.Hw.SetTdpValues([])` : maxima firmware.
    #[test]
    fn empty_values_apply_maxima() {
        let control = TdpControl::new(uniwill());
        let applied = control.set_values(&[]).unwrap();
        assert_eq!(
            applied
                .iter()
                .map(|limit| limit.current)
                .collect::<Vec<_>>(),
            vec![80, 100, 120]
        );
        assert_eq!(control.limits().unwrap()[0].current, 80);
    }

    #[test]
    fn applies_partial_values() {
        let control = TdpControl::new(uniwill());
        let applied = control.set_values(&[45.0]).unwrap();
        assert_eq!(applied[0].current, 45);
        assert_eq!(
            applied[1].current, 45,
            "les autres limites restent inchangées"
        );
        assert_eq!(applied[2].current, 60);
    }

    #[test]
    fn rejects_out_of_range_non_integer_and_decreasing_values() {
        let control = TdpControl::new(uniwill());
        assert!(matches!(
            control.set_values(&[5.0]),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            control.set_values(&[90.0]),
            Err(HwError::OutOfRange { .. })
        ));
        assert!(matches!(
            control.set_values(&[40.5]),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            control.set_values(&[60.0, 45.0]),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            control.set_values(&[45.0, 46.0, 47.0, 48.0]),
            Err(HwError::InvalidValue { .. })
        ));
        // Aucune écriture n'a eu lieu.
        assert_eq!(control.limits().unwrap()[0].current, 35);
    }

    #[test]
    fn equal_values_are_allowed_and_not_rewritten() {
        let control = TdpControl::new(uniwill());
        let applied = control.set_values(&[35.0, 35.0]).unwrap();
        assert_eq!(applied[0].current, 35);
        assert_eq!(applied[1].current, 35);
    }

    #[test]
    fn clevo_has_no_tdp() {
        let control = TdpControl::new(clevo());
        assert!(!control.is_available().unwrap());
        assert!(control.limits().unwrap().is_empty());
        assert!(matches!(
            control.set_values(&[30.0]),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn firmware_without_limits_is_unsupported() {
        let transport = FakeTransport::uniwill().with_tdp_limits([-1, -1, -1], [0, 0, 0]);
        let io = Arc::new(TuxedoIo::with_transport(Box::new(transport)).unwrap());
        let control = TdpControl::new(io);
        assert!(!control.is_available().unwrap());
        assert!(matches!(
            control.apply_defaults(),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn limit_accepts_checks_bounds() {
        let limit = TdpLimit {
            descriptor: "pl1",
            current: 35,
            min: 10,
            max: 80,
        };
        assert!(limit.accepts(10));
        assert!(limit.accepts(80));
        assert!(!limit.accepts(9));
        assert!(!limit.accepts(81));
    }

    #[test]
    fn watts_conversion_is_strict() {
        assert_eq!(to_watts(45.0).unwrap(), 45);
        assert_eq!(
            to_watts(-1.0).unwrap_err().to_string(),
            "valeur invalide pour TDP : -1"
        );
        assert!(to_watts(f64::NAN).is_err());
        assert!(to_watts(1e20).is_err());
    }
}
