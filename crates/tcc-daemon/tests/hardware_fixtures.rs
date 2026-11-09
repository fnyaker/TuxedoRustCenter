// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Non-régression du payload `Manager1.GetHardwareSnapshotJSON`.
//!
//! Le type canonique vit dans `crates/tcc-daemon/src/hardware/mod.rs` ; ce test
//! l'inclut via `#[path]` pour valider les fixtures versionnées sans démarrer
//! le daemon ni toucher au matériel. Il couvre :
//!
//! - le snapshot complet (device, CPU, display, capabilities, charge avec
//!   `chargeTypes`/drapeaux d'écriture, éclairage, Fn-Lock, ODM/TDP, NVIDIA,
//!   PRIME, YCbCr, webcam avec contrôles UVC réels) ;
//! - les sections absentes (`#[serde(default)]` au premier niveau) ;
//! - les types faux, les sections imbriquées non tolérantes et les valeurs
//!   hors bornes (conservées : aucune validation côté client) ;
//! - un appareil sans GPU et une webcam sans contrôle ;
//! - les bornes d'affichage de la GUI (16 devices, 64 contrôles, 128 menus) et
//!   le budget de taille des fixtures (4 Mio).
//!
//! Les bornes 16/64/128 sont celles de `crates/tcc-gui/src/daemon_client.rs`
//! (`MAX_WEBCAM_DEVICES`, `MAX_UVC_CONTROLS`, `MAX_UVC_MENU`) : le pont GUI
//! tronque avant QML. Le daemon, lui, tronque à la production (16/256/64 via
//! `tcc_hw::v4l2`). Ce test vérifie que les fixtures valides restent dans le
//! budget d'affichage et que la fixture « oversized » le dépasse volontairement.

#[path = "../src/hardware/mod.rs"]
mod hardware;

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use std::fs;
use std::path::PathBuf;

use hardware::HardwareSnapshot;

/// Bornes d'affichage de la GUI (`crates/tcc-gui/src/daemon_client.rs`).
const GUI_MAX_WEBCAM_DEVICES: usize = 16;
const GUI_MAX_UVC_CONTROLS: usize = 64;
const GUI_MAX_UVC_MENU: usize = 128;
/// Budget de réponse du pont GUI (4 Mio) et taille maximale d'une fixture.
const MAX_FIXTURE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_README_FIXTURE_BYTES: u64 = 64 * 1024;

fn snapshot(name: &str) -> HardwareSnapshot {
    let raw = fixtures::read_fixture(name);
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{name}: snapshot invalide: {error}"))
}

#[test]
fn full_snapshot_parses_and_roundtrips() {
    let raw = fixtures::read_fixture("dbus/hardware_snapshot_full.json");
    let snapshot: HardwareSnapshot = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("snapshot complet invalide: {error}"));

    // Les nouvelles sections de premier niveau sont présentes et typées.
    assert!(snapshot.device.available);
    assert_eq!(
        snapshot.device.vendor.as_deref(),
        Some("TUXEDO Computers GmbH")
    );
    assert_eq!(snapshot.device.model.as_deref(), Some("IBP14G9"));
    assert!(snapshot.device.is_tuxedo);
    assert_eq!(snapshot.device.architecture, "x86_64");

    assert_eq!(snapshot.cpu.total_cores, 16);
    assert_eq!(snapshot.cpu.online_cores, 12);
    assert_eq!(snapshot.cpu.scaling_driver.as_deref(), Some("intel_pstate"));
    assert!(snapshot.cpu.has_cpu_control);
    assert!(snapshot.cpu.writable);
    assert_eq!(snapshot.cpu.available_epp.len(), 5);

    assert!(snapshot.display.available);
    assert!(snapshot.display.has_internal_display);
    assert_eq!(
        snapshot.display.internal_connector.as_deref(),
        Some("eDP-1")
    );
    assert!(snapshot.display.connected);

    assert!(snapshot.capabilities.has_battery);
    assert!(snapshot.capabilities.has_ac);
    assert!(snapshot.capabilities.has_igpu);
    assert!(snapshot.capabilities.has_dedicated_gpu);

    assert!(snapshot.charging.available);
    assert!(snapshot.charging.write_supported);
    assert!(snapshot.charging.profile_writable);
    assert!(snapshot.charging.priority_writable);
    assert!(snapshot.charging.thresholds_writable);
    assert!(snapshot.charging.charge_type_available);
    assert!(snapshot.charging.charge_type_writable);
    // L'ABI noyau actuelle n'annonce pas de liste : le daemon publie `[]`.
    assert!(snapshot.charging.charge_types.is_empty());
    assert_eq!(snapshot.charging.battery.as_deref(), Some("BAT0"));
    assert_eq!(snapshot.charging.start_threshold, Some(40));
    assert_eq!(snapshot.charging.end_threshold, Some(80));
    assert_eq!(snapshot.charging.charge_type.as_deref(), Some("Custom"));

    assert!(snapshot.lighting.available);
    assert!(snapshot.lighting.enabled);
    assert!(snapshot.lighting.keyboard.available);
    assert!(snapshot.lighting.keyboard.writable);
    assert_eq!(snapshot.lighting.keyboard.zones, 3);
    assert_eq!(snapshot.lighting.states.len(), 3);
    assert!(snapshot.lighting.lightbar.available);
    assert!(snapshot.lighting.lightbar.writable);

    assert!(snapshot.odm.available);
    assert!(snapshot.odm.odm_writable);
    assert!(snapshot.odm.tdp_writable);
    assert!(snapshot.odm.platform_profile.available);
    assert_eq!(snapshot.odm.platform_profile.owner, "tcc");
    assert_eq!(snapshot.odm.tdp.len(), 3);
    assert_eq!(snapshot.odm.tdp[0].descriptor, "pl1");
    assert_eq!(snapshot.odm.tdp[0].current, 35);
    assert_eq!(snapshot.odm.interface.as_deref(), Some("uniwill"));

    assert!(snapshot.sensors.enabled);
    assert!(snapshot.sensors.available);
    assert!(snapshot.sensors.igpu.is_some());
    assert!(snapshot.sensors.dgpu.is_some());
    assert_eq!(snapshot.sensors.gpus.len(), 2);

    assert!(snapshot.nvidia.available);
    assert!(snapshot.nvidia.writable);
    assert_eq!(snapshot.nvidia.ctgp_offset, Some(15));

    assert!(snapshot.webcam.switch_available);
    assert!(!snapshot.webcam.controls_writable);
    assert!(snapshot.webcam.controls_supported);
    assert!(!snapshot.webcam.controls_truncated);
    assert_eq!(snapshot.webcam.devices.len(), 2);
    let camera = &snapshot.webcam.devices[0];
    assert_eq!(camera.device_path.as_deref(), Some("/dev/video0"));
    assert_eq!(camera.controls.len(), 5);
    assert_eq!(camera.controls[0].key, "brightness");
    assert_eq!(camera.controls[0].category, "basic");
    assert!(camera.controls[0].flags.read_only);
    assert_eq!(camera.controls[3].key, "power_line_frequency");
    assert_eq!(camera.controls[3].menu.len(), 3);
    assert!(snapshot.webcam.devices[1].metadata);

    assert!(snapshot.fn_lock.supported);
    assert_eq!(snapshot.fn_lock.enabled, Some(false));
    assert!(snapshot.fn_lock.writable);

    assert!(snapshot.prime.available);
    assert!(!snapshot.prime.writable);
    assert_eq!(snapshot.prime.mode.as_deref(), Some("on-demand"));
    assert!(snapshot.prime.requires_offloading);

    assert!(snapshot.ycbcr.available);
    assert_eq!(snapshot.ycbcr.ports.len(), 2);
    assert_eq!(snapshot.ycbcr.ports[0].port, "DP-1");
    assert_eq!(snapshot.ycbcr.ports[0].forced, Some(false));

    // Les clés inconnues sont tolérées puis supprimées à la re-sérialisation.
    assert!(raw.contains("x-extension"));
    let encoded = serde_json::to_string(&snapshot).expect("sérialisation du snapshot");
    assert!(!encoded.contains("x-extension"));

    // Aller-retour fidèle : les noms camelCase et les valeurs sont stables.
    let reparsed: HardwareSnapshot =
        serde_json::from_str(&encoded).expect("snapshot re-sérialisé invalide");
    assert_eq!(reparsed, snapshot);

    let json: serde_json::Value = serde_json::from_str(&raw).expect("JSON de fixture");
    for key in [
        "device",
        "cpu",
        "display",
        "capabilities",
        "charging",
        "lighting",
        "odm",
        "sensors",
        "nvidia",
        "webcam",
        "fnLock",
        "prime",
        "ycbcr",
    ] {
        assert!(
            json.get(key).is_some(),
            "section absente de la fixture: {key}"
        );
    }
    assert!(json["charging"]["writeSupported"].is_boolean());
    assert!(json["capabilities"]["hasDedicatedGpu"].is_boolean());
    assert!(json["device"]["isTuxedo"].is_boolean());
    assert!(json["webcam"]["devices"][0]["controls"][0]["flags"]["readOnly"].is_boolean());
}

#[test]
fn empty_and_partial_snapshots_are_tolerated() {
    // `{}` : toutes les sections de premier niveau ont un `default`.
    let empty: HardwareSnapshot =
        serde_json::from_str(&fixtures::read_fixture("dbus/hardware_snapshot_empty.json"))
            .expect("snapshot vide");
    assert_eq!(empty, HardwareSnapshot::default());

    // Sections partielles : chaque section `#[serde(default)]` accepte
    // l'absence de ses champs. `ycbcr` est fourni complet car il ne l'est pas.
    let partial = snapshot("dbus/hardware_snapshot_partial.json");
    assert_eq!(
        partial.device.vendor.as_deref(),
        Some("TUXEDO Computers GmbH")
    );
    assert!(partial.charging.available);
    assert!(!partial.charging.write_supported);
    assert!(partial.charging.charge_types.is_empty());
    assert!(!partial.lighting.available);
    assert!(!partial.odm.available);
    assert!(!partial.odm.platform_profile.available);
    assert!(!partial.nvidia.available);
    assert!(!partial.webcam.switch_available);
    assert!(!partial.fn_lock.supported);
    assert!(!partial.prime.available);
    assert!(!partial.ycbcr.available);
    assert!(partial.ycbcr.applied);
    // `sensors` absent -> valeur par défaut.
    assert_eq!(partial.sensors, hardware::sensors::SensorData::default());

    // Les mêmes sections en objet vide sont tolérées.
    let sections: HardwareSnapshot = serde_json::from_str(
        r#"{
            "charging": {},
            "lighting": {},
            "odm": {},
            "nvidia": {},
            "webcam": {},
            "fnLock": {},
            "prime": {}
        }"#,
    )
    .expect("sections vides tolérées");
    assert_eq!(sections, HardwareSnapshot::default());
}

#[test]
fn missing_required_nested_sections_are_rejected() {
    // Incohérence de tolérance : `ycbcr` et `sensors` n'ont pas
    // `#[serde(default)]` sur leur struct, contrairement aux autres sections.
    // Un payload partiel produit par une version antérieure échoue donc côté
    // daemon. La GUI, elle, utilise `HardwareView` (`default` partout).
    for (name, field) in [
        ("dbus/invalid/hardware_snapshot_partial_ycbcr.json", "ycbcr"),
        (
            "dbus/invalid/hardware_snapshot_partial_sensors.json",
            "sensors",
        ),
    ] {
        let raw = fixtures::read_fixture(name);
        let error = serde_json::from_str::<HardwareSnapshot>(&raw)
            .expect_err("une section imbriquée partielle doit être refusée");
        assert!(
            error.to_string().contains("missing field"),
            "{name}: erreur inattendue pour {field}: {error}"
        );
    }
}

#[test]
fn wrong_types_are_rejected() {
    for name in [
        "dbus/invalid/hardware_snapshot_wrong_type_bool.json",
        "dbus/invalid/hardware_snapshot_wrong_type_array.json",
    ] {
        let raw = fixtures::read_fixture(name);
        let error = serde_json::from_str::<HardwareSnapshot>(&raw)
            .expect_err("un type faux doit être refusé");
        assert!(
            error.to_string().contains("invalid type"),
            "{name}: erreur inattendue: {error}"
        );
    }
}

#[test]
fn out_of_range_values_are_preserved_for_display() {
    // `HardwareSnapshot` est un payload de **sortie** : il n'a pas de
    // `validate()`. Les bornes sont appliquées par les producteurs sysfs/ioctl
    // et par les méthodes D-Bus d'écriture. Ce test documente qu'un client ne
    // doit jamais déduire une autorisation d'écriture de ces valeurs.
    let snapshot = snapshot("dbus/invalid/hardware_snapshot_out_of_range.json");
    assert_eq!(snapshot.charging.start_threshold, Some(150));
    assert_eq!(snapshot.charging.end_threshold, Some(10));
    assert_eq!(snapshot.charging.charge_type.as_deref(), Some("Bogus"));
    assert_eq!(snapshot.odm.tdp[0].current, -5);
    assert!(snapshot.odm.tdp[0].min > snapshot.odm.tdp[0].max);
    assert_eq!(snapshot.nvidia.ctgp_offset, Some(9999));
    assert_eq!(snapshot.nvidia.default_power_limit, Some(-1.0));
    assert_eq!(snapshot.lighting.keyboard.zones, 255);
    assert_eq!(snapshot.lighting.keyboard.max_brightness, u32::MAX);

    let control = &snapshot.webcam.devices[0].controls[0];
    assert!(control.min > control.max);
    assert_eq!(control.step, 0);
    assert_eq!(
        control.current,
        Some(hardware::webcam::UvcValueData::Integer { value: 10_000 })
    );
}

#[test]
fn device_without_gpu_has_no_nvidia_or_prime() {
    let snapshot = snapshot("dbus/hardware_snapshot_no_gpu.json");
    assert!(snapshot.capabilities.has_igpu);
    assert!(!snapshot.capabilities.has_dedicated_gpu);
    assert!(snapshot.sensors.dgpu.is_none());
    assert_eq!(snapshot.sensors.gpus.len(), 1);
    assert!(!snapshot.nvidia.available);
    assert!(!snapshot.nvidia.attribute_available);
    assert_eq!(snapshot.nvidia.ctgp_offset, None);
    assert!(!snapshot.prime.available);
    assert!(!snapshot.prime.supported);
    assert_eq!(snapshot.prime.mode.as_deref(), Some("igpu"));
    assert_eq!(snapshot.prime.internal_display_on_nvidia, None);
}

#[test]
fn webcam_without_controls_is_a_valid_snapshot() {
    let snapshot = snapshot("dbus/hardware_snapshot_webcam_without_controls.json");
    assert!(!snapshot.webcam.switch_available);
    assert!(!snapshot.webcam.controls_supported);
    assert!(!snapshot.webcam.controls_truncated);
    assert_eq!(snapshot.webcam.devices.len(), 1);
    let device = &snapshot.webcam.devices[0];
    assert!(device.controls.is_empty());
    assert_eq!(device.controls_error.as_deref(), Some("EACCES"));
    assert_eq!(snapshot.webcam.errors, vec!["contrôles illisibles"]);
}

#[test]
fn charge_types_are_forward_compatible() {
    let snapshot = snapshot("dbus/hardware_snapshot_charge_types.json");
    assert!(snapshot.charging.available);
    assert!(snapshot.charging.write_supported);
    assert!(snapshot.charging.profile_writable);
    assert!(!snapshot.charging.priority_writable);
    assert!(snapshot.charging.thresholds_writable);
    assert!(snapshot.charging.charge_type_available);
    assert!(snapshot.charging.charge_type_writable);
    assert_eq!(
        snapshot.charging.charge_types,
        vec!["Standard", "Adaptive", "Custom"]
    );
    assert_eq!(snapshot.charging.charge_type.as_deref(), Some("Adaptive"));
}

#[test]
fn webcam_fixtures_match_the_display_bounds() {
    let full = snapshot("dbus/hardware_snapshot_full.json");
    assert!(full.webcam.devices.len() <= GUI_MAX_WEBCAM_DEVICES);
    for device in &full.webcam.devices {
        assert!(
            device.controls.len() <= GUI_MAX_UVC_CONTROLS,
            "contrôles au-delà du budget d'affichage: {}",
            device.controls.len()
        );
        for control in &device.controls {
            assert!(
                control.menu.len() <= GUI_MAX_UVC_MENU,
                "menu au-delà du budget d'affichage: {}",
                control.menu.len()
            );
        }
    }

    // Fixture volontairement au-dessus des bornes d'affichage : elle sert à
    // vérifier que la future GUI/troncature est exercée. La troncature réelle
    // est testée dans `crates/tcc-gui/src/daemon_client.rs`
    // (`webcam_controls_are_bounded_in_the_display_view`), que la QA ne peut
    // pas déplacer ici (le test Rust de la GUI lie cxx-qt).
    let oversized = snapshot("dbus/invalid/hardware_snapshot_webcam_oversized.json");
    assert!(oversized.webcam.controls_truncated);
    assert_eq!(oversized.webcam.devices.len(), 1);
    assert_eq!(
        oversized.webcam.devices[0].controls.len(),
        GUI_MAX_UVC_CONTROLS + 1
    );
    assert_eq!(
        oversized.webcam.devices[0].controls[0].menu.len(),
        GUI_MAX_UVC_MENU + 1
    );
}

#[test]
fn fixture_files_stay_within_the_repository_budget() {
    let mut paths = Vec::new();
    for directory in ["dbus", "dbus/invalid"] {
        let directory = fixtures::fixture_path(directory);
        let entries = fs::read_dir(&directory).unwrap_or_else(|error| {
            panic!("lecture de {} impossible: {error}", directory.display())
        });
        for entry in entries {
            let path: PathBuf = entry.expect("entrée de répertoire").path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(".json")
                && (name.starts_with("hardware_") || name.starts_with("admin_hw_"))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    assert!(!paths.is_empty(), "aucune fixture matérielle trouvée");

    for path in paths {
        let size = fs::metadata(&path)
            .unwrap_or_else(|error| panic!("métadonnées de {}: {error}", path.display()))
            .len();
        assert!(
            size <= MAX_FIXTURE_BYTES,
            "{} dépasse le budget D-Bus de 4 Mio: {size} octets",
            path.display()
        );
        // Règle `tests/fixtures/README.md` : une fixture lisible reste
        // sous 64 Kio. La fixture « oversized » est proche de la limite mais
        // reste en dessous.
        assert!(
            size <= MAX_README_FIXTURE_BYTES,
            "{} dépasse la règle des 64 Kio: {size} octets",
            path.display()
        );
    }
}

/// Générateur xorshift déterministe (même approche que
/// `crates/tcc-ipc/tests/payload_robustness.rs`, sans dépendance).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next_u64() as usize) % len
    }
}

#[test]
fn mutated_hardware_snapshots_never_panic() {
    let base = fixtures::read_fixture("dbus/hardware_snapshot_full.json");
    let mut rng = Rng::new(0x4857_0001);
    for _ in 0..512 {
        let mut bytes = base.as_bytes().to_vec();
        let index = rng.index(bytes.len());
        bytes[index] = (rng.next_u64() & 0xff) as u8;
        let text = String::from_utf8_lossy(&bytes);
        if let Ok(snapshot) = serde_json::from_str::<HardwareSnapshot>(&text) {
            let _ = serde_json::to_string(&snapshot);
        }
    }

    // Troncatures : un payload coupé ne doit jamais faire paniquer serde.
    for cut in (0..=base.len()).step_by(41) {
        let mut end = cut;
        while !base.is_char_boundary(end) {
            end -= 1;
        }
        let _ = serde_json::from_str::<HardwareSnapshot>(&base[..end]);
    }
}
