// SPDX-License-Identifier: GPL-3.0-or-later
//
// Vue typée du `HardwareSnapshot` exposé par `Manager1`.
//
// Ce module n'est **pas** le contrat D-Bus : le type canonique vit dans
// `tcc-daemon` (`hardware::HardwareSnapshot`) et n'est pas encore partagé par
// `tcc-ipc`. La GUI ne désérialise donc qu'un **sous-ensemble d'affichage**,
// volontairement tolérant (`default` partout) :
//
// - toute clé inconnue du daemon est ignorée puis disparaît à la
//   re-sérialisation (la QML ne voit jamais de passthrough brut) ;
// - un champ absent ou renommé prend sa valeur par défaut et l'UI affiche
//   « non disponible » au lieu d'une valeur inventée ;
// - `sensors.fans` est optionnel : le contrat actuel n'expose pas encore les
//   ventilateurs, la vue accepte le champ dès qu'il sera ajouté.
//
// Les noms JSON sont ceux du daemon (`camelCase`). Toute évolution du contrat
// doit être répercutée ici **et** dans `qml/core/HardwareData.qml`.

#![allow(dead_code)] // Sous-ensemble : tous les champs ne sont pas encore affichés.

use serde::{Deserialize, Serialize};

/// Instantané matériel réduit aux besoins d'affichage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HardwareView {
    /// Identification DMI et versions noyau/driver.
    pub device: Device,
    /// Topologie et capacités CPU (lecture seule).
    pub cpu: CpuCapabilities,
    /// Connecteur interne DRM (les modes restent fournis par `tccd-user`).
    pub display: Display,
    /// Capabilities simples (batterie, secteur, iGPU/dGPU).
    pub capabilities: Capabilities,
    pub charging: Charging,
    pub lighting: Lighting,
    pub odm: Odm,
    pub sensors: Sensors,
    pub nvidia: Nvidia,
    pub webcam: Webcam,
    pub fn_lock: FnLock,
    pub prime: Prime,
    pub ycbcr: Ycbcr,
}

/// Identification de l'appareil et versions système.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Device {
    /// Au moins un champ DMI lisible.
    pub available: bool,
    pub vendor: Option<String>,
    pub product_name: Option<String>,
    pub board_name: Option<String>,
    pub board_vendor: Option<String>,
    pub chassis_vendor: Option<String>,
    pub sku: Option<String>,
    pub is_tuxedo: bool,
    /// Modèle TUXEDO stable (`IBM15A10`, …), absent si inconnu.
    pub model: Option<String>,
    pub kernel_release: Option<String>,
    pub architecture: String,
    pub tuxedo_driver_version: Option<String>,
    pub errors: Vec<String>,
}

/// Capacités CPU exposées par le daemon.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CpuCapabilities {
    /// Nombre de CPU présents, borné par le daemon.
    pub total_cores: u32,
    /// Nombre de CPU en ligne, borné par le daemon.
    pub online_cores: u32,
    pub available_governors: Vec<String>,
    pub available_epp: Vec<String>,
    pub has_cpu_control: bool,
    pub writable: bool,
    pub scaling_driver: Option<String>,
    pub errors: Vec<String>,
}

/// Connecteur interne détecté dans `/sys/class/drm`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Display {
    pub available: bool,
    pub has_internal_display: bool,
    pub internal_connector: Option<String>,
    pub connected: bool,
    /// Source de l'information (`drm`), `None` si aucun connecteur.
    pub source: Option<String>,
    pub errors: Vec<String>,
}

/// Capacités matérielles simples.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub has_battery: bool,
    pub has_ac: bool,
    pub has_igpu: bool,
    pub has_dedicated_gpu: bool,
    pub errors: Vec<String>,
}

/// Charge de la batterie.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Charging {
    pub available: bool,
    pub write_supported: bool,
    pub profile_writable: bool,
    pub priority_writable: bool,
    pub thresholds_writable: bool,
    pub charge_type_available: bool,
    pub charge_type_writable: bool,
    pub charge_types: Vec<String>,
    pub profiles: Vec<String>,
    pub profile: Option<String>,
    pub priorities: Vec<String>,
    pub priority: Option<String>,
    pub battery: Option<String>,
    pub start_threshold: Option<u8>,
    pub end_threshold: Option<u8>,
    pub start_thresholds: Vec<u8>,
    pub end_thresholds: Vec<u8>,
    pub charge_type: Option<String>,
    pub thresholds_available: bool,
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Éclairage clavier / lightbar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lighting {
    pub available: bool,
    pub enabled: bool,
    pub unsupported: bool,
    pub keyboard: Keyboard,
    pub lightbar: Lightbar,
    pub states: Vec<Zone>,
    pub lightbar_state: Option<Zone>,
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Capacités du clavier rétroéclairé.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Keyboard {
    pub available: bool,
    /// TCC peut appliquer un état (matériel présent, contrôle activé).
    pub writable: bool,
    /// Modes réellement appliqués (`static`, éventuellement `breathing`).
    pub modes: Vec<String>,
    pub kind: String,
    pub zones: u8,
    pub max_brightness: u32,
    pub max_red: Option<u16>,
    pub max_green: Option<u16>,
    pub max_blue: Option<u16>,
    pub buffer_input: bool,
}

/// Capacités de la barre lumineuse.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lightbar {
    pub available: bool,
    pub writable: bool,
    pub modes: Vec<String>,
    pub max_brightness: u32,
    pub max_red: u16,
    pub max_green: u16,
    pub max_blue: u16,
}

/// État d'une zone RVB.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Zone {
    pub brightness: u8,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// ODM (`platform_profile`) et limites TDP.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Odm {
    /// Un réglage ODM/TDP est exposé (platform_profile ou `tuxedo-io`).
    pub available: bool,
    /// Un profil ODM est pilotable (platform TCC ou fallback io).
    pub odm_writable: bool,
    /// Au moins une limite TDP est pilotable.
    pub tdp_writable: bool,
    pub platform_profile: PlatformProfile,
    /// Capacités du chemin `tuxedo-io` (fallback ODM/TDP).
    pub io: Io,
    pub tdp_available: bool,
    pub tdp: Vec<Tdp>,
    pub interface: Option<String>,
    pub module_version: Option<String>,
    pub applied: bool,
    pub errors: Vec<String>,
}

/// `platform_profile` courant et propriétaire (PPD ou TCC).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlatformProfile {
    pub available: bool,
    pub writable: bool,
    pub source: Option<String>,
    pub current: Option<String>,
    pub choices: Vec<String>,
    pub default_profile: Option<String>,
    pub acpi_quirk_blocked: bool,
    pub owner: String,
    pub applied: Option<String>,
}

/// Capacités du chemin `tuxedo-io` (fallback quand `platform_profile` est
/// absent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Io {
    pub available: bool,
    pub interface: Option<String>,
    pub module_version: Option<String>,
    pub odm_available: bool,
    pub choices: Vec<String>,
    pub default_profile: Option<String>,
    pub tdp_available: bool,
}

/// Une limite de puissance (`pl1`, `pl2`, `pl4`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tdp {
    pub descriptor: String,
    pub current: i32,
    pub min: i32,
    pub max: i32,
}

/// Capteurs CPU/GPU et ventilateurs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sensors {
    pub enabled: bool,
    pub available: bool,
    pub cpu: Cpu,
    pub igpu: Option<Gpu>,
    pub dgpu: Option<Gpu>,
    pub gpus: Vec<Gpu>,
    /// Optionnel : absent du contrat actuel, accepté dès qu'il sera exposé.
    pub fans: Vec<Fan>,
}

/// Capteurs CPU.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cpu {
    pub temperature_c: Option<f64>,
    pub frequency_mhz: Option<u64>,
    pub power_w: Option<f64>,
}

/// Capteurs GPU.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Gpu {
    pub vendor: String,
    pub role: String,
    pub name: String,
    pub card: String,
    pub temperature_c: Option<f64>,
    pub frequency_mhz: Option<u64>,
    pub max_frequency_mhz: Option<u64>,
    pub power_w: Option<f64>,
}

/// Ventilateur (champ optionnel, voir [`Sensors::fans`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fan {
    pub name: Option<String>,
    pub temp: Option<f64>,
    pub speed: Option<f64>,
    pub applied: Option<bool>,
}

/// NVIDIA cTGP.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Nvidia {
    pub available: bool,
    pub attribute_available: bool,
    pub nvidia_smi_available: bool,
    pub ctgp_offset: Option<i32>,
    pub default_power_limit: Option<f64>,
    pub max_power_limit: Option<f64>,
    /// Borne dynamique de l'offset cTGP, si le backend l'expose (AH-11).
    pub max_ctgp_offset: Option<i32>,
    pub applied_offset: Option<i32>,
    pub errors: Vec<String>,
}

/// Webcam et kill switch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Webcam {
    pub switch_available: bool,
    pub enabled: Option<bool>,
    pub controls_supported: bool,
    pub controls_truncated: bool,
    pub devices: Vec<VideoDevice>,
    pub applied: Option<bool>,
    pub errors: Vec<String>,
}

/// Nœud V4L2 détecté.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoDevice {
    pub name: String,
    pub device_path: Option<String>,
    pub device_name: Option<String>,
    pub index: Option<u32>,
    pub metadata: bool,
    /// Contrôles UVC en lecture seule (allowlist daemon).
    pub controls: Vec<UvcControl>,
    /// Erreur de lecture des contrôles pour ce device.
    pub controls_error: Option<String>,
}

/// Contrôle UVC exposé en lecture seule.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UvcControl {
    /// Clé stable (`brightness`, `focus_auto`, …).
    pub key: String,
    /// Nom noyau sanitisé.
    pub name: String,
    /// `integer`, `boolean`, `menu`, `integerMenu`.
    pub kind: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    #[serde(rename = "default")]
    pub default_value: i64,
    /// Valeur courante taguée par `kind` (tolérante aux formes inconnues).
    pub current: Option<UvcValue>,
    pub menu: Vec<UvcMenu>,
    pub flags: UvcFlags,
    /// `basic`, `exposure`, `color`, `advanced`, `other`.
    pub category: String,
    /// Visible en mode simple.
    pub simple: bool,
}

/// Valeur courante d'un contrôle UVC.
///
/// `value` reste un JSON brut : entier pour `integer`, booléen pour
/// `boolean` ; les menus portent `index`/`key`/`label`. Une valeur d'un type
/// inconnu est conservée sans faire échouer tout l'instantané.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UvcValue {
    pub kind: String,
    pub value: Option<serde_json::Value>,
    pub index: Option<i64>,
    pub key: Option<String>,
    pub label: Option<String>,
}

/// Entrée de menu UVC.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UvcMenu {
    pub index: i64,
    pub key: String,
    pub label: String,
}

/// Drapeaux UVC simples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UvcFlags {
    pub inactive: bool,
    pub read_only: bool,
    pub write_only: bool,
    pub update: bool,
    pub disabled: bool,
    pub grabbed: bool,
}

/// Verrouillage Fn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FnLock {
    pub supported: bool,
    pub enabled: Option<bool>,
    /// Le matériel accepte une écriture (policy D-Bus séparée).
    pub writable: bool,
}

/// Mode PRIME.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prime {
    pub available: bool,
    pub platform_supported: bool,
    pub supported: bool,
    pub tux_prime: bool,
    pub mode: Option<String>,
    pub internal_display_on_nvidia: Option<bool>,
    pub requires_offloading: bool,
    pub prime_select: Option<String>,
}

/// YCbCr 4:2:0 par port.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ycbcr {
    pub available: bool,
    pub ports: Vec<YcbcrPort>,
    pub pending_mode_reapply: bool,
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Port forcé en YCbCr 4:2:0.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct YcbcrPort {
    pub card: u32,
    pub port: String,
    pub forced: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::HardwareView;

    const MINIMAL: &str = r#"{
        "device": {
            "available": true,
            "vendor": "TUXEDO Computers GmbH",
            "productName": "TUXEDO Book X",
            "boardName": "Board",
            "boardVendor": "TUXEDO Computers",
            "chassisVendor": "TUXEDO",
            "sku": "IBM15A10",
            "isTuxedo": true,
            "model": "IBM15A10",
            "kernelRelease": "6.12.0-tuxedo",
            "architecture": "x86_64",
            "tuxedoDriverVersion": "0.3.9",
            "errors": []
        },
        "cpu": {
            "totalCores": 16,
            "onlineCores": 12,
            "availableGovernors": ["powersave", "performance"],
            "availableEpp": ["power", "performance"],
            "hasCpuControl": true,
            "writable": true,
            "scalingDriver": "intel_pstate",
            "errors": []
        },
        "display": {
            "available": true,
            "hasInternalDisplay": true,
            "internalConnector": "eDP-1",
            "connected": true,
            "source": "drm",
            "errors": []
        },
        "capabilities": {
            "hasBattery": true,
            "hasAc": true,
            "hasIgpu": true,
            "hasDedicatedGpu": true,
            "errors": []
        },
        "charging": {
            "available": true,
            "writeSupported": true,
            "profileWritable": true,
            "priorityWritable": true,
            "thresholdsWritable": true,
            "chargeTypeAvailable": true,
            "chargeTypeWritable": true,
            "chargeTypes": ["Fast", "Standard"],
            "profile": "balanced"
        },
        "lighting": {
            "available": true,
            "keyboard": { "writable": true, "modes": ["static"], "kind": "rgb", "zones": 3 },
            "lightbar": { "writable": true, "modes": ["static"] }
        },
        "odm": {
            "available": true,
            "odmWritable": true,
            "tdpWritable": false,
            "platformProfile": { "available": false, "current": null, "owner": "ppd" },
            "io": {
                "available": true,
                "interface": "uniwill",
                "moduleVersion": "0.2.6",
                "odmAvailable": true,
                "choices": ["power_save", "enthusiast", "overboost"],
                "defaultProfile": "enthusiast",
                "tdpAvailable": false
            }
        },
        "sensors": {
            "enabled": true,
            "available": true,
            "cpu": { "temperatureC": 55.5, "frequencyMhz": 3200, "powerW": 28.0 },
            "dgpu": { "vendor": "nvidia", "name": "RTX 4060", "temperatureC": 61.0 },
            "fans": [{ "name": "CPU", "temp": 55.5, "speed": 42.0, "applied": true }]
        },
        "nvidia": { "available": true, "ctgpOffset": 10, "maxCtgpOffset": 150 },
        "fnLock": { "supported": true, "enabled": false, "writable": true },
        "prime": { "available": true, "mode": "on-demand" },
        "ycbcr": { "available": true, "ports": [{ "card": 0, "port": "DP-1", "forced": false }] },
        "unknownTopLevel": { "ignored": true }
    }"#;

    #[test]
    fn view_deserializes_subset_and_drops_unknown_keys() {
        let view: HardwareView = serde_json::from_str(MINIMAL).unwrap();
        assert!(view.device.available);
        assert_eq!(view.device.product_name.as_deref(), Some("TUXEDO Book X"));
        assert!(view.device.is_tuxedo);
        assert_eq!(view.device.kernel_release.as_deref(), Some("6.12.0-tuxedo"));
        assert_eq!(view.device.architecture, "x86_64");
        assert_eq!(view.cpu.total_cores, 16);
        assert_eq!(view.cpu.online_cores, 12);
        assert_eq!(view.cpu.available_governors.len(), 2);
        assert_eq!(view.cpu.available_epp[1], "performance");
        assert_eq!(view.cpu.scaling_driver.as_deref(), Some("intel_pstate"));
        assert!(view.display.available);
        assert!(view.display.has_internal_display);
        assert_eq!(view.display.internal_connector.as_deref(), Some("eDP-1"));
        assert!(view.display.connected);
        assert!(view.capabilities.has_battery);
        assert!(view.capabilities.has_ac);
        assert!(view.capabilities.has_igpu);
        assert!(view.capabilities.has_dedicated_gpu);
        assert!(view.charging.available);
        assert!(view.charging.write_supported);
        assert!(view.charging.profile_writable);
        assert!(view.charging.priority_writable);
        assert!(view.charging.thresholds_writable);
        assert!(view.charging.charge_type_available);
        assert!(view.charging.charge_type_writable);
        assert_eq!(view.charging.charge_types, ["Fast", "Standard"]);
        assert_eq!(view.charging.profile.as_deref(), Some("balanced"));
        assert_eq!(view.lighting.keyboard.kind, "rgb");
        assert_eq!(view.lighting.keyboard.zones, 3);
        assert!(view.lighting.keyboard.writable);
        assert_eq!(view.lighting.keyboard.modes, ["static"]);
        assert!(view.lighting.lightbar.writable);
        assert_eq!(view.lighting.lightbar.modes, ["static"]);
        assert!(view.odm.available);
        assert!(view.odm.odm_writable);
        assert!(!view.odm.tdp_writable);
        assert!(view.odm.io.available);
        assert!(view.odm.io.odm_available);
        assert_eq!(view.odm.io.choices[1], "enthusiast");
        assert_eq!(view.odm.io.default_profile.as_deref(), Some("enthusiast"));
        assert!(!view.odm.io.tdp_available);
        assert!(view.sensors.enabled);
        assert_eq!(view.sensors.cpu.temperature_c, Some(55.5));
        assert_eq!(view.sensors.fans.len(), 1);
        assert_eq!(view.sensors.fans[0].speed, Some(42.0));
        assert_eq!(view.nvidia.ctgp_offset, Some(10));
        assert_eq!(view.nvidia.max_ctgp_offset, Some(150));
        assert_eq!(view.fn_lock.enabled, Some(false));
        assert!(view.fn_lock.writable);
        assert_eq!(view.prime.mode.as_deref(), Some("on-demand"));
        assert_eq!(view.ycbcr.ports[0].port, "DP-1");

        let canonical = serde_json::to_string(&view).unwrap();
        assert!(!canonical.contains("unknownTopLevel"));
        // Toutes les sections sont présentes, même vides.
        for key in [
            "\"device\"",
            "\"cpu\"",
            "\"display\"",
            "\"capabilities\"",
            "\"charging\"",
            "\"lighting\"",
            "\"odm\"",
            "\"sensors\"",
            "\"nvidia\"",
            "\"webcam\"",
            "\"fnLock\"",
            "\"prime\"",
            "\"ycbcr\"",
        ] {
            assert!(canonical.contains(key), "clé manquante: {key}");
        }
    }

    #[test]
    fn webcam_controls_are_parsed_and_unknown_fields_dropped() {
        let payload = r#"{
            "charging": { "available": true },
            "sensors": {},
            "webcam": {
                "switchAvailable": true,
                "controlsSupported": true,
                "controlsTruncated": false,
                "devices": [{
                    "name": "Integrated Camera",
                    "devicePath": "/dev/video0",
                    "controlsError": null,
                    "controls": [{
                        "key": "brightness",
                        "name": "Brightness",
                        "kind": "integer",
                        "min": 0, "max": 255, "step": 1, "default": 128,
                        "current": { "kind": "integer", "value": 100 },
                        "menu": [],
                        "flags": { "readOnly": true, "inactive": false },
                        "category": "basic",
                        "simple": true,
                        "futureField": "ignored"
                    }, {
                        "key": "exposure_auto",
                        "name": "Exposure, Auto",
                        "kind": "menu",
                        "min": 0, "max": 3, "step": 1, "default": 0,
                        "current": { "kind": "menu", "index": 1,
                                     "key": "manual", "label": "Manual Mode" },
                        "menu": [
                            { "index": 0, "key": "auto", "label": "Auto" },
                            { "index": 1, "key": "manual", "label": "Manual Mode" }
                        ],
                        "flags": { "readOnly": false },
                        "category": "exposure",
                        "simple": false
                    }]
                }]
            }
        }"#;
        let view: HardwareView = serde_json::from_str(payload).expect("payload");
        assert!(view.webcam.controls_supported);
        let device = &view.webcam.devices[0];
        assert_eq!(device.device_path.as_deref(), Some("/dev/video0"));
        assert_eq!(device.controls.len(), 2);
        let brightness = &device.controls[0];
        assert_eq!(brightness.key, "brightness");
        assert_eq!(brightness.kind, "integer");
        assert_eq!(brightness.min, 0);
        assert_eq!(brightness.max, 255);
        assert_eq!(brightness.default_value, 128);
        assert!(brightness.flags.read_only);
        assert_eq!(brightness.category, "basic");
        assert!(brightness.simple);
        assert_eq!(
            brightness
                .current
                .as_ref()
                .and_then(|value| value.value.as_ref()),
            Some(&serde_json::json!(100))
        );

        let exposure = &device.controls[1];
        assert_eq!(exposure.kind, "menu");
        assert_eq!(exposure.menu.len(), 2);
        assert_eq!(exposure.menu[1].label, "Manual Mode");
        let current = exposure.current.as_ref().expect("current");
        assert_eq!(current.kind, "menu");
        assert_eq!(current.label.as_deref(), Some("Manual Mode"));

        let canonical = serde_json::to_string(&view).unwrap();
        assert!(!canonical.contains("futureField"));
        assert!(canonical.contains("\"controls\""));
        assert!(!canonical.contains("\"tableCPU\""));
    }

    #[test]
    fn missing_webcam_control_fields_are_tolerated() {
        let view: HardwareView = serde_json::from_str(
            r#"{ "webcam": { "devices": [{ "name": "Cam", "controls": [{}] }] } }"#,
        )
        .expect("tolérant");
        let control = &view.webcam.devices[0].controls[0];
        assert_eq!(control.key, "");
        assert_eq!(control.kind, "");
        assert_eq!(control.min, 0);
        assert_eq!(control.current, None);
        assert!(control.menu.is_empty());
        assert!(!control.simple);
    }

    #[test]
    fn device_cpu_display_and_capabilities_are_tolerant() {
        let view: HardwareView = serde_json::from_str(
            r#"{
                "device": { "productName": null, "kernelRelease": null, "architecture": "" },
                "cpu": { "totalCores": 0, "availableEpp": [] },
                "display": { "internalConnector": null, "connected": false },
                "capabilities": { "hasBattery": false }
            }"#,
        )
        .expect("tolérant");
        assert!(!view.device.available);
        assert_eq!(view.device.product_name, None);
        assert_eq!(view.device.kernel_release, None);
        assert!(view.device.architecture.is_empty());
        assert_eq!(view.cpu.total_cores, 0);
        assert!(view.cpu.available_epp.is_empty());
        assert!(!view.display.available);
        assert_eq!(view.display.internal_connector, None);
        assert!(!view.capabilities.has_battery);
        assert!(!view.capabilities.has_ac);
    }

    #[test]
    fn device_cpu_display_bounds_are_rejected() {
        // Les entiers négatifs ou hors `u32` sont rejetés : la vue ne fabrique
        // jamais un nombre de cœurs incohérent.
        assert!(
            serde_json::from_str::<HardwareView>(r#"{ "cpu": { "totalCores": -1 } }"#).is_err()
        );
        assert!(
            serde_json::from_str::<HardwareView>(r#"{ "cpu": { "onlineCores": 4294967296 } }"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<HardwareView>(r#"{ "device": { "available": "yes" } }"#)
                .is_err()
        );
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let view: HardwareView = serde_json::from_str("{}").unwrap();
        assert!(!view.sensors.available);
        assert!(view.sensors.fans.is_empty());
        assert_eq!(view.charging.start_threshold, None);
        assert!(view.odm.tdp.is_empty());
    }

    #[test]
    fn wrong_field_type_is_rejected() {
        // La vue est tolérante aux champs absents, pas aux types faux.
        // (Le rejet d'un JSON non-objet est fait par `decode_hardware`.)
        assert!(serde_json::from_str::<HardwareView>(r#"{ "sensors": 3 }"#).is_err());
        assert!(
            serde_json::from_str::<HardwareView>(r#"{ "charging": { "available": "yes" } }"#)
                .is_err()
        );
    }

    #[test]
    fn null_optionals_are_accepted() {
        let raw = r#"{
            "sensors": {
                "cpu": { "temperatureC": null, "frequencyMhz": null, "powerW": null },
                "igpu": null,
                "dgpu": null
            },
            "charging": { "startThreshold": null, "endThreshold": null, "profile": null }
        }"#;
        let view: HardwareView = serde_json::from_str(raw).unwrap();
        assert_eq!(view.sensors.cpu.temperature_c, None);
        assert!(view.sensors.igpu.is_none());
        assert_eq!(view.charging.start_threshold, None);
    }
}
