// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Application matérielle d'un profil.
//!
//! Le jalon 1 applique le CPU et la luminosité. Les ventilateurs sont régulés
//! par le worker dédié `FanWorker` (tick 1 s, voir [`crate::fan`]) qui relit le
//! profil actif ; `apply_profile` ne fait que tracer la délégation pour éviter
//! deux writers sur la même consigne. La charge, le lighting, la webcam,
//! l'ODM/TDP et NVIDIA sont branchés aux jalons suivants ; le rapport les
//! mentionne explicitement pour ne pas laisser croire qu'ils sont actifs.

use tcc_core::{CpuProfile, FrequencyLimit, Profile, Settings};
use tcc_hw::backlight;
use tcc_hw::cpu::CpuController;
use tcc_hw::error::HwError;
use tcc_hw::sysfs::SysfsRoot;

/// Résultat d'une application de profil.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Profil appliqué.
    pub profile: String,
    /// Actions effectuées ou planifiées.
    pub notes: Vec<String>,
    /// Erreurs non bloquantes.
    pub errors: Vec<String>,
}

impl ApplyReport {
    /// Vrai si aucune erreur n'est survenue.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Applique un profil.
///
/// `ppd_active` indique que power-profiles-daemon est présent : dans ce cas,
/// l'EPP et `platform_profile` restent sous son contrôle (D4). `dry_run`
/// n'écrit rien et ne fait que produire un plan lisible.
#[must_use]
pub fn apply_profile(
    profile: &Profile,
    settings: &Settings,
    ppd_active: bool,
    root: &SysfsRoot,
    dry_run: bool,
) -> ApplyReport {
    let mut report = ApplyReport {
        profile: profile.name.clone(),
        ..ApplyReport::default()
    };

    if settings.cpu_settings_enabled {
        if dry_run {
            report.notes.push(format!(
                "dry-run: CPU {} (maxPerfGov={}, min={:?} kHz, max={:?}, cores={:?}, noTurbo={})",
                profile.cpu.governor,
                profile.cpu.use_max_perf_gov,
                profile.cpu.scaling_min_frequency,
                profile.cpu.scaling_max_frequency,
                profile.cpu.online_cores,
                profile.cpu.no_turbo
            ));
        } else {
            apply_cpu(profile, ppd_active, root, &mut report);
        }
    } else {
        report.notes.push("contrôle CPU désactivé".to_owned());
    }

    if profile.display.use_brightness {
        if dry_run {
            report.notes.push(format!(
                "dry-run: luminosité {} %",
                profile.display.brightness
            ));
        } else {
            match backlight::set_all_percent(root, profile.display.brightness) {
                Ok(applied) => {
                    report.notes.push(format!(
                        "luminosité {} % appliquée à {} périphérique(s)",
                        profile.display.brightness,
                        applied.applied.len()
                    ));
                    for (name, error) in applied.failures {
                        report.errors.push(format!("backlight {name}: {error}"));
                    }
                }
                Err(error) => report
                    .errors
                    .push(format!("énumération backlight: {error}")),
            }
        }
    }

    if settings.fan_control_enabled && profile.fan.use_control {
        // La régulation réelle est faite par `FanWorker` (tick 1 s), qui lit
        // le profil actif dans `RuntimeState`. On ne duplique pas l'écriture
        // ici : deux writers se battraient pour la même consigne.
        report.notes.push(format!(
            "ventilateurs: profil '{}' délégué au worker de régulation",
            profile.fan.fan_profile
        ));
    }

    report
}

fn apply_cpu(profile: &Profile, ppd_active: bool, root: &SysfsRoot, report: &mut ApplyReport) {
    let cpu = match CpuController::new(root.clone()) {
        Ok(cpu) => cpu,
        Err(error) => {
            report.errors.push(format!("CPU indisponible: {error}"));
            return;
        }
    };

    let governor = select_governor(&cpu, &profile.cpu);
    match cpu.set_governor(&governor) {
        Ok(()) => report.notes.push(format!("governor CPU: {governor}")),
        Err(error) => report.errors.push(format!("governor {governor}: {error}")),
    }

    if ppd_active {
        report
            .notes
            .push("EPP laissé à power-profiles-daemon (D4)".to_owned());
    } else if !profile.cpu.energy_performance_preference.is_empty() {
        match cpu.set_energy_performance_preference(&profile.cpu.energy_performance_preference) {
            Ok(()) => report.notes.push(format!(
                "EPP CPU: {}",
                profile.cpu.energy_performance_preference
            )),
            Err(error) => report.errors.push(format!("EPP: {error}")),
        }
    }

    if let Some(min) = profile.cpu.scaling_min_frequency {
        match cpu.set_scaling_min_khz(min) {
            Ok(()) => report.notes.push(format!("fréquence min CPU: {min} kHz")),
            Err(error) => report.errors.push(format!("fréquence min: {error}")),
        }
    }

    match profile.cpu.scaling_max_frequency {
        Some(FrequencyLimit::Hz(max)) => match cpu.set_scaling_max_khz(max) {
            Ok(()) => report.notes.push(format!("fréquence max CPU: {max} kHz")),
            Err(error) => report.errors.push(format!("fréquence max: {error}")),
        },
        Some(FrequencyLimit::Reduced) => report
            .notes
            .push("fréquence max 'Reduced' non résolue au jalon 1".to_owned()),
        None => {}
    }

    if let Some(cores) = profile.cpu.online_cores {
        match cpu.set_online_cores(cores as usize) {
            Ok(count) => report.notes.push(format!("cœurs en ligne: {count}")),
            Err(error) => report.errors.push(format!("cœurs en ligne: {error}")),
        }
    }

    if profile.cpu.no_turbo {
        match cpu.set_no_turbo(true) {
            Ok(()) => report.notes.push("turbo désactivé".to_owned()),
            Err(HwError::NotSupported(_)) => match cpu.set_boost(false) {
                Ok(()) => report.notes.push("boost désactivé".to_owned()),
                Err(HwError::NotSupported(_)) => {
                    report.notes.push("turbo/boost non exposé".to_owned());
                }
                Err(error) => report.errors.push(format!("boost: {error}")),
            },
            Err(error) => report.errors.push(format!("no_turbo: {error}")),
        }
    }
}

fn select_governor(cpu: &CpuController, profile: &CpuProfile) -> String {
    let driver = cpu.scaling_driver();
    if profile.use_max_perf_gov {
        return "performance".to_owned();
    }
    if driver.is_pstate() {
        return "powersave".to_owned();
    }

    let available: Vec<&str> = cpu
        .policies()
        .iter()
        .flat_map(|policy| policy.available_governors.iter().map(String::as_str))
        .collect();
    for candidate in [
        profile.governor.as_str(),
        "schedutil",
        "ondemand",
        "conservative",
        "powersave",
    ] {
        if !candidate.is_empty() && available.contains(&candidate) {
            return candidate.to_owned();
        }
    }
    "powersave".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_core::presets::office_profile;

    #[test]
    fn dry_run_does_not_touch_hardware() {
        let root = SysfsRoot::new("/nonexistent-tcc-sysfs");
        let profile = office_profile();
        let settings = Settings::default();
        let report = apply_profile(&profile, &settings, true, &root, true);
        assert!(report.is_ok());
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("dry-run: CPU"))
        );
        assert!(!report.notes.iter().any(|note| note.contains("EPP laissé")));
    }

    #[test]
    fn dry_run_keeps_backlight_file_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = SysfsRoot::new(dir.path());
        let backlight = dir.path().join("class/backlight/test");
        std::fs::create_dir_all(&backlight).expect("répertoire backlight");
        std::fs::write(backlight.join("max_brightness"), "100\n").expect("max");
        std::fs::write(backlight.join("brightness"), "20\n").expect("brightness");

        let mut profile = office_profile();
        profile.display.use_brightness = true;
        profile.display.brightness = 80;
        let settings = Settings {
            cpu_settings_enabled: false,
            ..Settings::default()
        };
        let report = apply_profile(&profile, &settings, false, &root, true);
        assert!(report.is_ok());
        assert_eq!(
            std::fs::read_to_string(backlight.join("brightness")).expect("lecture"),
            "20\n",
            "dry-run ne doit pas écrire le rétroéclairage"
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("dry-run: luminosité 80"))
        );
    }

    #[test]
    fn disabled_cpu_is_reported() {
        let root = SysfsRoot::new("/nonexistent-tcc-sysfs");
        let profile = office_profile();
        let settings = Settings {
            cpu_settings_enabled: false,
            ..Settings::default()
        };
        let report = apply_profile(&profile, &settings, false, &root, true);
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("contrôle CPU désactivé"))
        );
    }
}
