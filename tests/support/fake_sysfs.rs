// SPDX-License-Identifier: GPL-3.0-or-later
//
// Builder de racine sysfs factice, partagé par les tests d'intégration de
// `tcc-hw`. Aucun chemin absolu n'est codé en dur : la racine est un
// `tempfile::TempDir` détruit à la fin du test.
//
// Ce fichier est inclus via `#[path = "../../../tests/support/fake_sysfs.rs"]`.
// Il n'utilise que `std` et `tempfile`, dev-dépendance de `tcc-hw`.
//
// Les méthodes `with_*` consomment puis retournent `Self`, ce qui permet de
// construire une arborescence en une expression sans borrowed temporary.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// Arborescence sysfs minimale et contrôlable.
#[derive(Debug)]
pub struct FakeSysfs {
    dir: TempDir,
}

impl Default for FakeSysfs {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSysfs {
    /// Crée une racine vide.
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir sysfs"),
        }
    }

    /// Racine physique du répertoire temporaire.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Résout un chemin relatif en refusant les évasions accidentelles.
    fn resolve(&self, relative: &str) -> PathBuf {
        assert!(
            !relative.starts_with('/'),
            "un chemin de fixture doit être relatif: {relative}"
        );
        assert!(
            !relative.split('/').any(|component| component == ".."),
            "un chemin de fixture ne doit pas contenir `..`: {relative}"
        );
        self.path().join(relative)
    }

    /// Écrit un attribut (crée les répertoires parents).
    pub fn write(&self, relative: &str, contents: &str) -> &Self {
        self.write_bytes(relative, contents.as_bytes())
    }

    /// Écrit des octets bruts (UTF-8 invalide, NUL, ...).
    pub fn write_bytes(&self, relative: &str, contents: &[u8]) -> &Self {
        let path = self.resolve(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("création du répertoire sysfs");
        }
        fs::write(&path, contents).expect("écriture de l'attribut sysfs");
        self
    }

    /// Crée un répertoire.
    pub fn mkdir(&self, relative: &str) -> &Self {
        fs::create_dir_all(self.resolve(relative)).expect("création du répertoire sysfs");
        self
    }

    /// Lit un attribut brut.
    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.resolve(relative)).expect("lecture de l'attribut sysfs")
    }

    /// Vrai si le chemin existe.
    pub fn exists(&self, relative: &str) -> bool {
        self.resolve(relative).exists()
    }

    /// Crée un lien symbolique (POSIX uniquement).
    #[cfg(unix)]
    pub fn symlink(&self, target: impl AsRef<Path>, link: &str) -> &Self {
        use std::os::unix::fs::symlink;

        let path = self.resolve(link);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("création du répertoire du lien");
        }
        symlink(target, &path).expect("création du lien symbolique");
        self
    }

    // -----------------------------------------------------------------
    // Arborescences réalistes
    // -----------------------------------------------------------------

    /// Arborescence complète d'un portable TUXEDO de test : CPU, alimentation,
    /// rétroéclairage, hwmon `tuxi`, clavier/lightbar, platform_profile,
    /// debugfs DRM, DMI et webcam.
    pub fn laptop() -> Self {
        Self::new()
            .with_cpu(8)
            .with_power_supply(true, 73, "Charging")
            .with_backlight("intel_backlight", 255, 128, Some(128))
            .with_tuxi_fans()
            .with_tuxedo_keyboard()
            .with_platform_profile()
            .with_drm_debugfs()
            .with_dmi()
            .with_webcam()
            .with_nvidia_ctgp()
    }

    /// Topologie CPU et cpufreq `intel_pstate` pour `cores` processeurs.
    pub fn with_cpu(self, cores: u32) -> Self {
        assert!(cores > 0, "au moins un CPU");
        let last = cores - 1;
        let range = if cores == 1 {
            "0".to_owned()
        } else {
            format!("0-{last}")
        };
        self.write("devices/system/cpu/possible", &format!("{range}\n"))
            .write("devices/system/cpu/present", &format!("{range}\n"))
            .write("devices/system/cpu/online", &format!("{range}\n"))
            .write("devices/system/cpu/intel_pstate/no_turbo", "0\n")
            .write("devices/system/cpu/intel_pstate/status", "active\n")
            .write("devices/system/cpu/intel_pstate/max_perf_pct", "100\n")
            .write("devices/system/cpu/intel_pstate/min_perf_pct", "1\n")
            .write("devices/system/cpu/intel_pstate/num_pstates", "32\n")
            .write("devices/system/cpu/intel_pstate/turbo_pct", "50\n")
            .write("devices/system/cpu/cpufreq/boost", "0\n");

        for cpu in 0..cores {
            let base = format!("devices/system/cpu/cpu{cpu}/cpufreq");
            self.write(&format!("{base}/scaling_driver"), "intel_pstate\n")
                .write(&format!("{base}/scaling_governor"), "powersave\n")
                .write(
                    &format!("{base}/scaling_available_governors"),
                    "performance powersave\n",
                )
                .write(
                    &format!("{base}/energy_performance_preference"),
                    "balance_performance\n",
                )
                .write(
                    &format!("{base}/energy_performance_available_preferences"),
                    "default performance balance_performance balance_power power\n",
                )
                .write(&format!("{base}/cpuinfo_min_freq"), "400000\n")
                .write(&format!("{base}/cpuinfo_max_freq"), "4800000\n")
                .write(&format!("{base}/scaling_min_freq"), "400000\n")
                .write(&format!("{base}/scaling_max_freq"), "4800000\n")
                .write(&format!("{base}/scaling_cur_freq"), "2200000\n")
                .write(
                    &format!("{base}/scaling_available_frequencies"),
                    "400000 800000 1200000 1600000 2000000 2400000 \
                     2800000 3200000 3600000 4000000 4400000 4800000\n",
                );
            if cpu != 0 {
                self.write(&format!("devices/system/cpu/cpu{cpu}/online"), "1\n");
            }
        }
        self
    }

    /// Adaptateur secteur et batterie.
    pub fn with_power_supply(self, ac_online: bool, capacity: u8, status: &str) -> Self {
        self.write("class/power_supply/AC/type", "Mains\n")
            .write(
                "class/power_supply/AC/online",
                if ac_online { "1\n" } else { "0\n" },
            )
            .write("class/power_supply/BAT0/type", "Battery\n")
            .write("class/power_supply/BAT0/capacity", &format!("{capacity}\n"))
            .write("class/power_supply/BAT0/status", &format!("{status}\n"))
            .write("class/power_supply/BAT0/present", "1\n")
            .write("class/power_supply/BAT0/charge_type", "Standard\n")
            .write(
                "class/power_supply/BAT0/charge_control_start_threshold",
                "40\n",
            )
            .write(
                "class/power_supply/BAT0/charge_control_end_threshold",
                "80\n",
            )
            .write(
                "class/power_supply/BAT0/charge_control_start_available_thresholds",
                "0 20 40 60 80 100\n",
            )
            .write(
                "class/power_supply/BAT0/charge_control_end_available_thresholds",
                "0 20 40 60 80 100\n",
            );
        self
    }

    /// Périphérique de rétroéclairage avec `actual_brightness` optionnel.
    pub fn with_backlight(
        self,
        name: &str,
        max_brightness: u32,
        brightness: u32,
        actual_brightness: Option<u32>,
    ) -> Self {
        let base = format!("class/backlight/{name}");
        self.write(&format!("{base}/type"), "raw\n")
            .write(
                &format!("{base}/max_brightness"),
                &format!("{max_brightness}\n"),
            )
            .write(&format!("{base}/brightness"), &format!("{brightness}\n"));
        if let Some(actual) = actual_brightness {
            self.write(&format!("{base}/actual_brightness"), &format!("{actual}\n"));
        }
        self
    }

    /// hwmon `tuxedo_tuxi_sensors` avec deux ventilateurs et deux capteurs.
    pub fn with_tuxi_fans(self) -> Self {
        self.write("class/hwmon/hwmon0/name", "tuxedo_tuxi_sensors\n")
            .write("class/hwmon/hwmon0/fan1_input", "1500\n")
            .write("class/hwmon/hwmon0/fan1_max", "3000\n")
            .write("class/hwmon/hwmon0/fan1_label", "cpu\n")
            .write("class/hwmon/hwmon0/fan2_input", "900\n")
            .write("class/hwmon/hwmon0/fan2_max", "3000\n")
            .write("class/hwmon/hwmon0/fan2_label", "gpu1\n")
            .write("class/hwmon/hwmon0/temp1_input", "45000\n")
            .write("class/hwmon/hwmon0/temp1_label", "cpu\n")
            .write("class/hwmon/hwmon0/temp2_input", "55000\n")
            .write("class/hwmon/hwmon0/temp2_label", "gpu1\n")
            .write("bus/platform/devices/tuxedo_fan_control/fan1_pwm", "0\n")
            .write(
                "bus/platform/devices/tuxedo_fan_control/fan1_pwm_enable",
                "2\n",
            )
            .write("bus/platform/devices/tuxedo_fan_control/fan2_pwm", "0\n")
            .write(
                "bus/platform/devices/tuxedo_fan_control/fan2_pwm_enable",
                "2\n",
            );
        self
    }

    /// hwmon `tuxedo` (backend `pwm`), pour tester la priorité des backends.
    pub fn with_pwm_hwmon(self) -> Self {
        self.write("class/hwmon/hwmon1/name", "tuxedo\n")
            .write("class/hwmon/hwmon1/fan1_input", "255\n")
            .write("class/hwmon/hwmon1/fan1_max", "255\n")
            .write("class/hwmon/hwmon1/fan1_label", "cpu\n");
        self
    }

    /// Module `tuxedo_keyboard` : charge, Fn-Lock et LEDs clavier/lightbar.
    pub fn with_tuxedo_keyboard(self) -> Self {
        self.write(
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profile",
            "high\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/charging_profile/charging_profiles_available",
            "high medium low\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prio",
            "charging\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/charging_priority/charging_prios_available",
            "charging performance\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/charge_control_start_threshold",
            "40\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/charge_control_end_threshold",
            "80\n",
        )
        .write("devices/platform/tuxedo_keyboard/fn_lock", "0\n")
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/brightness",
            "120\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/max_brightness",
            "255\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/multi_intensity",
            "227 0 22\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input",
            "0\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_1/brightness",
            "120\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_1/max_brightness",
            "255\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_1/multi_intensity",
            "227 0 22\n",
        )
        .write("class/leds/rgb:kbd_backlight/brightness", "120\n")
        .write("class/leds/rgb:kbd_backlight/max_brightness", "255\n")
        .write("class/leds/rgb:kbd_backlight/multi_intensity", "227 0 22\n")
        .write("class/leds/rgb:kbd_backlight_1/brightness", "120\n")
        .write("class/leds/rgb:kbd_backlight_1/max_brightness", "255\n")
        .write(
            "class/leds/rgb:kbd_backlight_1/multi_intensity",
            "227 0 22\n",
        )
        .write("class/leds/rgb:lightbar/brightness", "50\n")
        .write("class/leds/rgb:lightbar/max_brightness", "255\n")
        .write("class/leds/rgb:lightbar/multi_intensity", "227 0 22\n");
        self
    }

    /// Variante clavier blanc (une seule intensité, sans couleur).
    pub fn with_white_keyboard(self) -> Self {
        self.write(
            "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/brightness",
            "200\n",
        )
        .write(
            "devices/platform/tuxedo_keyboard/leds/white:kbd_backlight/max_brightness",
            "255\n",
        )
        .write("class/leds/white:kbd_backlight/brightness", "200\n")
        .write("class/leds/white:kbd_backlight/max_brightness", "255\n");
        self
    }

    /// Profils ODM exposés par `tuxedo_platform_profile` et l'ACPI générique.
    pub fn with_platform_profile(self) -> Self {
        self.write(
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        )
        .write(
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save quiet balanced performance\n",
        )
        .write(
            "bus/platform/devices/tuxedo_platform_profile/profile",
            "balanced\n",
        )
        .write(
            "bus/platform/devices/tuxedo_platform_profile/available_profiles",
            "power_save quiet balanced performance\n",
        )
        .write("firmware/acpi/platform_profile", "balanced\n")
        .write(
            "firmware/acpi/platform_profile_choices",
            "low-power balanced performance\n",
        );
        self
    }

    /// Workaround YCbCr 4:2:0 exposé dans debugfs.
    pub fn with_drm_debugfs(self) -> Self {
        self.write("kernel/debug/dri/0/card0-eDP-1/force_yuv420_output", "0\n")
            .write("kernel/debug/dri/0/card0-DP-1/force_yuv420_output", "0\n");
        self
    }

    /// Identité DMI d'un portable TUXEDO.
    pub fn with_dmi(self) -> Self {
        self.write("class/dmi/id/product_sku", "TUXEDO-QUIRK-SKU\n")
            .write("class/dmi/id/product_name", "TUXEDO Book Test\n")
            .write("class/dmi/id/board_name", "TRC-TEST\n")
            .write("class/dmi/id/board_vendor", "TUXEDO Computers\n")
            .write("class/dmi/id/chassis_vendor", "TUXEDO\n")
            .write("class/dmi/id/sys_vendor", "TUXEDO Computers GmbH\n");
        self
    }

    /// Webcam V4L2 minimale.
    pub fn with_webcam(self) -> Self {
        self.write("class/video4linux/video0/name", "TUXEDO Webcam\n")
            .write("class/video4linux/video0/index", "0\n")
            .write("class/video4linux/video0/dev", "81:0\n");
        self
    }

    /// Contrôle cTGP NVIDIA (attribut sysfs uniquement, pas de `nvidia-smi`).
    pub fn with_nvidia_ctgp(self) -> Self {
        self.write(
            "devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset",
            "0\n",
        );
        self
    }
}
