// SPDX-License-Identifier: GPL-3.0-or-later
//
// Tests d'intégration de `tcc-hw` sur une racine sysfs factice complète.
// Aucun accès à `/sys`, aucun privilège, aucun laptop TUXEDO : le builder
// `tests/support/fake_sysfs.rs` crée l'arborescence dans un `TempDir`.

#[path = "../../../tests/support/fake_sysfs.rs"]
mod fake_sysfs;

use fake_sysfs::FakeSysfs;
use tcc_hw::backlight::{self, BacklightDevice};
use tcc_hw::charging::ChargingController;
use tcc_hw::cpu::{CpuController, ScalingDriver};
use tcc_hw::device::TuxedoDevice;
use tcc_hw::dmi::DmiInfo;
use tcc_hw::error::HwError;
use tcc_hw::fan::{self, FanBackendKind, FanTarget};
use tcc_hw::fn_lock::FnLock;
use tcc_hw::lighting::{KeyboardKind, LightingController, RgbColor};
use tcc_hw::nvidia::CtgpControl;
use tcc_hw::platform_profile::{PlatformProfile, ProfileOwnership, ProfileSource};
use tcc_hw::power_supply::{PowerState, PowerSupplies};
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::{webcam, ycbcr};

fn root(sysfs: &FakeSysfs) -> SysfsRoot {
    SysfsRoot::new(sysfs.path())
}

#[test]
fn complete_tuxedo_fake_root_is_readable() {
    let sysfs = FakeSysfs::laptop();
    let root = root(&sysfs);

    for path in [
        "devices/system/cpu/cpu7/cpufreq/scaling_governor",
        "class/power_supply/AC/online",
        "class/power_supply/BAT0/capacity",
        "class/backlight/intel_backlight/brightness",
        "class/hwmon/hwmon0/fan1_input",
        "bus/platform/devices/tuxedo_fan_control/fan1_pwm",
        "devices/platform/tuxedo_keyboard/charging_profile/charging_profile",
        "devices/platform/tuxedo_keyboard/fn_lock",
        "class/leds/rgb:kbd_backlight/multi_intensity",
        "class/leds/rgb:lightbar/brightness",
        "bus/platform/devices/tuxedo_platform_profile/profile",
        "firmware/acpi/platform_profile_choices",
        "kernel/debug/dri/0/card0-eDP-1/force_yuv420_output",
        "class/dmi/id/product_sku",
        "class/video4linux/video0/name",
    ] {
        assert!(root.exists(path), "chemin factice manquant: {path}");
    }

    let dmi = DmiInfo::read(&root).unwrap();
    assert!(dmi.is_tuxedo());
    assert_eq!(dmi.sku(), Some("TUXEDO-QUIRK-SKU"));

    let cpu = CpuController::new(root.clone()).unwrap();
    assert_eq!(cpu.topology().cpu_count(), 8);
    assert_eq!(cpu.scaling_driver(), ScalingDriver::IntelPstate);
    assert_eq!(cpu.policies().len(), 8);
    assert_eq!(cpu.online_cores(), (0..8).collect::<Vec<_>>());
    assert_eq!(cpu.intel_pstate().unwrap().unwrap().no_turbo, Some(false));

    let supplies = PowerSupplies::read(&root).unwrap();
    assert_eq!(supplies.power_state(), PowerState::Ac);
    assert_eq!(supplies.first_battery().unwrap().capacity_percent, Some(73));
    assert_eq!(supplies.average_capacity(), Some(73));

    let backlight = backlight::pick_primary(&root).unwrap().unwrap();
    assert_eq!(backlight.name, "intel_backlight");
    assert_eq!(backlight.brightness_percent(), 50);

    let fans = fan::detect_with(&root, None).expect("backend tuxi");
    assert_eq!(fans.backend(), FanBackendKind::Tuxi);
    assert_eq!(fans.fan_count(), 2);
    assert_eq!(fans.fan_label(0), "cpu");
    assert_eq!(fans.fan_label(1), "gpu1");
    assert_eq!(fans.fan_target(0), FanTarget::Cpu);
    assert_eq!(fans.fan_target(1), FanTarget::Gpu);
    assert_eq!(fans.fan_temperature(0).unwrap(), 45);
    assert_eq!(fans.fan_temperature(1).unwrap(), 55);
    assert_eq!(fans.fan_speed_percent(0).unwrap(), 50);
    assert_eq!(fans.fan_speed_percent(1).unwrap(), 30);

    let charging = ChargingController::new(root.clone())
        .capabilities()
        .expect("capacités de charge");
    assert_eq!(charging.profile.as_deref(), Some("high"));
    assert_eq!(charging.profiles, ["high", "medium", "low"]);
    assert_eq!(charging.priority.as_deref(), Some("charging"));
    assert_eq!(charging.battery.as_deref(), Some("BAT0"));
    assert_eq!(charging.start_threshold, Some(40));
    assert_eq!(charging.end_threshold, Some(80));
    assert_eq!(charging.charge_type.as_deref(), Some("Standard"));
    assert!(charging.thresholds_available);
    assert!(!charging.is_empty());

    let lighting = LightingController::detect(root.clone()).expect("éclairage");
    assert!(lighting.keyboard_available());
    assert_eq!(lighting.keyboard_capabilities().kind, KeyboardKind::Rgb);
    assert_eq!(lighting.keyboard_capabilities().zones, 2);
    assert_eq!(lighting.keyboard_capabilities().max_brightness, 255);
    assert!(lighting.lightbar_available());

    let platform = PlatformProfile::detect(&root, TuxedoDevice::Unknown).expect("platform_profile");
    assert_eq!(platform.source(), Some(ProfileSource::Tuxedo));
    assert_eq!(platform.current(), Some("balanced"));
    assert_eq!(platform.choices().len(), 4);
    assert_eq!(platform.default_profile(), Some("performance"));
    assert!(platform.is_available());

    let fn_lock = FnLock::new(root.clone());
    assert!(fn_lock.is_available());
    assert_eq!(fn_lock.status().unwrap(), Some(false));

    let video = webcam::enumerate(&root).expect("webcam");
    assert_eq!(video.len(), 1);
    assert_eq!(video[0].name, "video0");
    assert_eq!(video[0].device_path.as_deref(), Some("/dev/video0"));
    assert_eq!(video[0].device_name.as_deref(), Some("TUXEDO Webcam"));
    assert!(video[0].is_capture());
    assert!(!video[0].is_metadata());

    let ports = ycbcr::enumerate(&root).expect("ports YCbCr");
    assert_eq!(ports.len(), 2);
    assert!(ports.iter().any(|port| port.port == "card0-eDP-1"));
    assert!(ports.iter().any(|port| port.port == "card0-DP-1"));

    // cTGP NVIDIA : l'attribut sysfs suffit à exposer le contrôle, sans
    // dépendre de `nvidia-smi` sur la machine de test.
    let ctgp = CtgpControl::new(root.clone());
    assert!(ctgp.attribute_available());
}

#[test]
fn cpu_writes_are_validated_and_clamped() {
    let sysfs = FakeSysfs::new().with_cpu(8);
    let root = root(&sysfs);
    let cpu = CpuController::new(root).unwrap();

    cpu.set_governor("performance").unwrap();
    assert_eq!(
        sysfs
            .read("devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .trim(),
        "performance"
    );
    assert_eq!(
        sysfs
            .read("devices/system/cpu/cpu7/cpufreq/scaling_governor")
            .trim(),
        "performance"
    );

    // Governor absent de `scaling_available_governors` : refus avant toute
    // écriture partielle.
    assert!(matches!(
        cpu.set_governor("schedutil"),
        Err(HwError::InvalidValue { .. })
    ));
    assert_eq!(
        sysfs
            .read("devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .trim(),
        "performance"
    );

    cpu.set_scaling_max_khz(9_000_000).unwrap();
    assert_eq!(
        sysfs
            .read("devices/system/cpu/cpu3/cpufreq/scaling_max_freq")
            .trim(),
        "4800000"
    );

    cpu.set_scaling_min_khz(50_000).unwrap();
    assert_eq!(
        sysfs
            .read("devices/system/cpu/cpu3/cpufreq/scaling_min_freq")
            .trim(),
        "400000"
    );

    assert_eq!(cpu.set_online_cores(4).unwrap(), 4);
    assert_eq!(sysfs.read("devices/system/cpu/cpu3/online").trim(), "1");
    assert_eq!(sysfs.read("devices/system/cpu/cpu4/online").trim(), "0");
    assert!(cpu.set_online_cores(0).is_err());
    assert!(cpu.set_online_cores(9).is_err());
}

#[test]
fn fan_backend_priority_and_multifan_writes() {
    let sysfs = FakeSysfs::new().with_tuxi_fans().with_pwm_hwmon();
    let root = root(&sysfs);
    let control = fan::detect_with(&root, None).expect("backend");

    // `tuxi` est prioritaire sur `pwm`, même quand les deux existent.
    assert_eq!(control.backend(), FanBackendKind::Tuxi);
    assert_eq!(control.fan_count(), 2);
    assert!(control.has_write_support());
    assert_eq!(control.fans_min_speed(), 0);
    assert!(control.fans_off_available());

    control.set_manual_mode(true).unwrap();
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_fan_control/fan1_pwm_enable")
            .trim(),
        "1"
    );
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_fan_control/fan2_pwm_enable")
            .trim(),
        "1"
    );

    control.set_fan_speed_percent(1, 50).unwrap();
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_fan_control/fan2_pwm")
            .trim(),
        "128"
    );
    assert!(matches!(
        control.set_fan_speed_percent(2, 50),
        Err(HwError::OutOfRange { .. })
    ));
    assert!(matches!(
        control.set_fan_speed_percent(0, 101),
        Err(HwError::OutOfRange { .. })
    ));

    control.set_fans_auto().unwrap();
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_fan_control/fan1_pwm_enable")
            .trim(),
        "2"
    );
}

#[test]
fn amdgpu_backlight_quirk_is_normalized() {
    let sysfs = FakeSysfs::new().with_backlight("amdgpu_bl0", 255, 128, Some(32768));
    let root = root(&sysfs);
    let device = BacklightDevice::read(&root, "amdgpu_bl0").unwrap();

    assert!(device.amdgpu_quirk);
    assert_eq!(device.effective_raw(), 32768);
    assert_eq!(device.display_raw_8bit(), 128);
    assert_eq!(device.brightness_percent(), 50);
    assert_eq!(device.percent_to_raw(50), 128);

    device.set_percent(&root, 50).unwrap();
    assert_eq!(
        sysfs.read("class/backlight/amdgpu_bl0/brightness").trim(),
        "128"
    );
}

#[test]
fn sysfs_writes_are_bounded_and_symlinks_are_refused() {
    let sysfs = FakeSysfs::new().with_power_supply(true, 50, "Charging");
    let root = root(&sysfs);

    assert!(root.resolve("/etc/passwd").is_err());
    assert!(root.resolve("../etc/passwd").is_err());
    assert!(root.resolve("class/../etc/passwd").is_err());

    let ac_online = "class/power_supply/AC/online";
    assert!(root.write_string(ac_online, "").is_err());
    assert!(root.write_string(ac_online, "\0").is_err());
    assert!(root.write_string(ac_online, "1\n2").is_err());
    assert!(root.write_string(ac_online, &"9".repeat(4097)).is_err());
    root.write_i64(ac_online, 1).expect("écriture normale");

    // Un lien symbolique final ne doit jamais être suivi par une écriture
    // privilégiée, même vers un fichier anodin.
    std::fs::remove_file(sysfs.path().join(ac_online)).unwrap();
    sysfs.symlink("/etc/passwd", ac_online);
    assert!(matches!(
        root.write_i64(ac_online, 0),
        Err(HwError::UnsafeTarget { .. })
    ));
}

#[test]
fn charging_lighting_platform_and_ycbcr_writes_are_validated() {
    let sysfs = FakeSysfs::laptop();
    let root = root(&sysfs);

    // Charge : profil, priorité, seuils et type.
    let charging = ChargingController::new(root.clone());
    charging.set_profile("medium").unwrap();
    assert_eq!(
        sysfs
            .read("devices/platform/tuxedo_keyboard/charging_profile/charging_profile")
            .trim(),
        "medium"
    );
    assert!(matches!(
        charging.set_profile("turbo"),
        Err(HwError::InvalidValue { .. })
    ));
    charging.set_priority("performance").unwrap();
    assert_eq!(
        sysfs
            .read("devices/platform/tuxedo_keyboard/charging_priority/charging_prio")
            .trim(),
        "performance"
    );
    charging.set_thresholds(20, 80).unwrap();
    assert_eq!(
        sysfs
            .read("class/power_supply/BAT0/charge_control_start_threshold")
            .trim(),
        "20"
    );
    assert_eq!(
        sysfs.read("class/power_supply/BAT0/charge_type").trim(),
        "Custom"
    );
    assert!(matches!(
        charging.set_thresholds(80, 20),
        Err(HwError::InvalidValue { .. })
    ));
    assert!(matches!(
        charging.set_thresholds(30, 80),
        Err(HwError::InvalidValue { .. })
    ));
    assert_eq!(
        sysfs
            .read("class/power_supply/BAT0/charge_control_start_threshold")
            .trim(),
        "20",
        "une validation en échec ne doit rien écrire"
    );

    // Éclairage : deux zones RGB et lightbar.
    let lighting = LightingController::detect(root.clone()).unwrap();
    let report = lighting
        .apply_keyboard(200, &[RgbColor::new(1, 2, 3), RgbColor::new(4, 5, 6)])
        .unwrap();
    assert!(report.applied, "{report:?}");
    assert_eq!(report.zones_written, 2);
    assert_eq!(
        sysfs
            .read("devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/brightness")
            .trim(),
        "200"
    );
    assert_eq!(
        sysfs
            .read("devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight_1/multi_intensity")
            .trim(),
        "4 5 6"
    );
    assert_eq!(
        sysfs
            .read(
                "devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight/device/controls/buffer_input"
            )
            .trim(),
        "0",
        "buffer_input doit être relâché"
    );
    lighting.apply_lightbar(50, RgbColor::new(7, 8, 9)).unwrap();
    assert_eq!(
        sysfs.read("class/leds/rgb:lightbar/brightness").trim(),
        "50"
    );
    assert_eq!(
        sysfs.read("class/leds/rgb:lightbar/multi_intensity").trim(),
        "7 8 9"
    );

    // platform_profile : PPD propriétaire (D4) puis TCC.
    let platform = PlatformProfile::detect(&root, TuxedoDevice::Unknown).unwrap();
    assert!(matches!(
        platform.apply(&root, "performance", ProfileOwnership::PpdOwned),
        Err(HwError::PolicyDenied { .. })
    ));
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_platform_profile/platform_profile")
            .trim(),
        "balanced"
    );
    platform
        .apply(&root, "performance", ProfileOwnership::TccOwned)
        .unwrap();
    assert_eq!(
        sysfs
            .read("bus/platform/devices/tuxedo_platform_profile/platform_profile")
            .trim(),
        "performance"
    );
    assert!(matches!(
        platform.apply(&root, "turbo", ProfileOwnership::TccOwned),
        Err(HwError::InvalidValue { .. })
    ));

    // Fn-Lock.
    let fn_lock = FnLock::new(root.clone());
    fn_lock.set(true).unwrap();
    assert_eq!(fn_lock.status().unwrap(), Some(true));

    // YCbCr : seul le port demandé change, une seconde application est un no-op.
    assert!(ycbcr::apply(&root, 0, "card0-DP-1", true).unwrap());
    assert_eq!(
        sysfs
            .read("kernel/debug/dri/0/card0-DP-1/force_yuv420_output")
            .trim(),
        "1"
    );
    assert!(!ycbcr::apply(&root, 0, "card0-DP-1", true).unwrap());
    assert!(matches!(
        ycbcr::apply(&root, 0, "card0-DP-1/../../x", true),
        Err(HwError::InvalidValue { .. })
    ));
}

#[test]
fn missing_hardware_degrades_cleanly() {
    let sysfs = FakeSysfs::new();
    let root = root(&sysfs);

    assert_eq!(DmiInfo::read(&root).unwrap(), DmiInfo::default());
    assert_eq!(
        PowerSupplies::read(&root).unwrap(),
        PowerSupplies::default()
    );
    assert!(backlight::enumerate(&root).unwrap().is_empty());
    assert!(backlight::pick_primary(&root).unwrap().is_none());
    assert!(fan::detect_with(&root, None).is_none());
}
