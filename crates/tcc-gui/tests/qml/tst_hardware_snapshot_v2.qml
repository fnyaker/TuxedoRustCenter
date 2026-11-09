// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Non-régression de `HardwareData` face au `HardwareSnapshot` étendu :
// device, cpu, display, capabilities, charge avec `chargeTypes`/drapeaux
// d'écriture, éclairage, Fn-Lock, ODM/TDP, NVIDIA, PRIME, YCbCr et webcam.
//
// Le pont Rust (`src/hardware_view.rs`) désérialise déjà ces sections et
// re-sérialise une vue bornée ; ce test vérifie que l'adaptateur QML les
// normalise sans erreur et applique les mêmes bornes d'affichage (16 devices,
// 64 contrôles, 128 menus). Aucune fixture n'est lue ici : `qmltestrunner`
// n'a pas d'accès fichier portable ; les fixtures sont consommées par les
// tests Rust (`crates/tcc-daemon/tests/hardware_fixtures.rs`).

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "HardwareSnapshotV2"
    when: windowShown

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        HardwareData.fanError = "";
        HardwareData.sensorsRequested = false;
        HardwareData.dashboardVisible = false;
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function setLive(payload) {
        HardwareData.connected = true;
        HardwareData.hardwareError = "";
        HardwareData.hardwareJson = JSON.stringify(payload);
    }

    function fullPayload() {
        return {
            "device": {
                "available": true,
                "vendor": "TUXEDO Computers GmbH",
                "productName": "TUXEDO InfinityBook Pro 14 Gen9",
                "boardName": "PH4PRX1_REN",
                "boardVendor": "TUXEDO",
                "chassisVendor": "TUXEDO",
                "sku": "IBP14G9",
                "isTuxedo": true,
                "model": "IBP14G9",
                "kernelRelease": "6.12.0-tuxedo",
                "architecture": "x86_64",
                "tuxedoDriverVersion": "0.3.9",
                "errors": []
            },
            "cpu": {
                "totalCores": 16,
                "onlineCores": 12,
                "availableGovernors": ["performance", "powersave"],
                "availableEpp": ["default", "performance"],
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
                "chargeTypes": [],
                "profiles": ["high_capacity", "balanced", "stationary"],
                "profile": "balanced",
                "priorities": ["charging", "performance"],
                "priority": "charging",
                "battery": "BAT0",
                "startThreshold": 40,
                "endThreshold": 80,
                "startThresholds": [40, 50, 60],
                "endThresholds": [80, 90, 100],
                "chargeType": "Custom",
                "thresholdsAvailable": true,
                "applied": true,
                "errors": []
            },
            "lighting": {
                "available": true,
                "enabled": true,
                "unsupported": false,
                "keyboard": {
                    "available": true,
                    "writable": true,
                    "modes": ["static"],
                    "kind": "rgb",
                    "zones": 3,
                    "maxBrightness": 100,
                    "bufferInput": true
                },
                "lightbar": { "available": true, "writable": true, "maxBrightness": 80 },
                "states": [{ "brightness": 50, "red": 255, "green": 32, "blue": 16 }],
                "lightbarState": { "brightness": 20, "red": 4, "green": 5, "blue": 6 },
                "applied": true,
                "errors": []
            },
            "odm": {
                "available": true,
                "odmWritable": true,
                "tdpWritable": true,
                "platformProfile": {
                    "available": true,
                    "writable": true,
                    "source": "tuxedo_platform_profile",
                    "current": "balanced",
                    "choices": ["power_save", "balanced", "performance"],
                    "defaultProfile": "balanced",
                    "owner": "tcc",
                    "applied": "balanced"
                },
                "tdpAvailable": true,
                "tdp": [
                    { "descriptor": "pl1", "current": 35, "min": 10, "max": 80 },
                    { "descriptor": "pl2", "current": 45, "min": 10, "max": 90 }
                ],
                "interface": "uniwill",
                "moduleVersion": "0.3.9",
                "applied": true,
                "errors": []
            },
            "sensors": {
                "enabled": true,
                "available": true,
                "cpu": { "temperatureC": 55.5, "frequencyMhz": 3200, "powerW": 28.5 },
                "igpu": {
                    "vendor": "intel", "role": "integrated", "name": "Intel Iris Xe",
                    "card": "card0", "temperatureC": 54.0, "frequencyMhz": 1100,
                    "maxFrequencyMhz": 1650, "powerW": 4.5
                },
                "dgpu": {
                    "vendor": "nvidia", "role": "discrete",
                    "name": "NVIDIA GeForce RTX 4060", "card": "card1",
                    "temperatureC": 61.0, "frequencyMhz": 1350,
                    "maxFrequencyMhz": 2400, "powerW": 42.0
                },
                "gpus": []
            },
            "nvidia": {
                "available": true,
                "writable": true,
                "attributeAvailable": true,
                "nvidiaSmiAvailable": true,
                "ctgpOffset": 15,
                "defaultPowerLimit": 60.0,
                "maxPowerLimit": 90.0,
                "appliedOffset": 15,
                "errors": []
            },
            "webcam": {
                "switchAvailable": true,
                "controlsWritable": false,
                "enabled": true,
                "controlsSupported": true,
                "controlsTruncated": false,
                "devices": [{
                    "name": "Integrated Camera",
                    "devicePath": "/dev/video0",
                    "deviceName": "Integrated Camera",
                    "index": 0,
                    "metadata": false,
                    "controls": [{
                        "key": "brightness",
                        "name": "Brightness",
                        "kind": "integer",
                        "min": 0, "max": 255, "step": 1, "default": 128,
                        "current": { "kind": "integer", "value": 128 },
                        "menu": [],
                        "flags": { "readOnly": true },
                        "category": "basic",
                        "simple": true
                    }, {
                        "key": "power_line_frequency",
                        "name": "Power Line Frequency",
                        "kind": "menu",
                        "min": 0, "max": 2, "step": 1, "default": 1,
                        "current": { "kind": "menu", "index": 1,
                                     "key": "50_hz", "label": "50 Hz" },
                        "menu": [
                            { "index": 0, "key": "disabled", "label": "Disabled" },
                            { "index": 1, "key": "50_hz", "label": "50 Hz" },
                            { "index": 2, "key": "60_hz", "label": "60 Hz" }
                        ],
                        "flags": { "readOnly": true },
                        "category": "advanced",
                        "simple": false
                    }],
                    "controlsError": null
                }],
                "applied": true,
                "errors": []
            },
            "fnLock": { "supported": true, "enabled": false, "writable": true },
            "prime": {
                "available": true,
                "writable": false,
                "platformSupported": true,
                "supported": true,
                "tuxPrime": true,
                "mode": "on-demand",
                "internalDisplayOnNvidia": false,
                "requiresOffloading": true,
                "primeSelect": "/usr/bin/prime-select"
            },
            "ycbcr": {
                "available": true,
                "ports": [
                    { "card": 0, "port": "DP-1", "forced": false },
                    { "card": 0, "port": "HDMI-A-1", "forced": false }
                ],
                "pendingModeReapply": false,
                "applied": true,
                "errors": []
            }
        };
    }

    function test_newSectionsAreNormalized() {
        setLive(fullPayload());

        compare(HardwareData.isLive, true);
        compare(HardwareData.parseError, "");

        compare(HardwareData.device.available, true);
        compare(HardwareData.device.productName, "TUXEDO InfinityBook Pro 14 Gen9");
        compare(HardwareData.deviceLabel, "TUXEDO InfinityBook Pro 14 Gen9");
        compare(HardwareData.device.isTuxedo, true);

        compare(HardwareData.cpuCaps.totalCores, 16);
        compare(HardwareData.cpuCaps.onlineCores, 12);
        compare(HardwareData.cpuCaps.hasControl, true);
        compare(HardwareData.cpuCaps.writable, true);
        compare(HardwareData.cpuCoresText, "12 / 16");

        compare(HardwareData.displayInfo.internalConnector, "eDP-1");
        compare(HardwareData.hasInternalDisplay, true);
        compare(HardwareData.internalDisplayLabel, "eDP-1");

        compare(HardwareData.capabilities.hasBattery, true);
        compare(HardwareData.capabilities.hasDedicatedGpu, true);

        compare(HardwareData.charging.available, true);
        compare(HardwareData.charging.profiles.length, 3);
        compare(HardwareData.charging.chargeType, "Custom");
        compare(HardwareData.charging.start, 40);
        compare(HardwareData.charging.end, 80);
        compare(HardwareData.charging.startThresholds.length, 3);
        compare(HardwareData.charging.thresholdsAvailable, true);

        compare(HardwareData.lighting.available, true);
        compare(HardwareData.lighting.keyboard.zones, 3);
        compare(HardwareData.lighting.lightbar.available, true);

        compare(HardwareData.odm.available, true);
        compare(HardwareData.odm.tdp.length, 2);
        compare(HardwareData.odm.platform.current, "balanced");
        compare(HardwareData.odm.platform.owner, "tcc");

        compare(HardwareData.nvidia.available, true);
        compare(HardwareData.nvidia.ctgpOffset, 15);
        compare(HardwareData.prime.mode, "on-demand");
        compare(HardwareData.ycbcr.ports.length, 2);
        compare(HardwareData.fnLock.supported, true);

        compare(HardwareData.webcam.devices.length, 1);
        compare(HardwareData.webcam.devices[0].controls.length, 2);
        compare(HardwareData.webcam.devices[0].controls[1].menu.length, 3);
        compare(HardwareData.webcam.devices[0].controls[1].current.label, "50 Hz");
    }

    function test_partialPayloadKeepsDefaults() {
        setLive({ "sensors": {}, "charging": { "available": true, "chargeTypes": [] } });

        compare(HardwareData.isLive, true);
        compare(HardwareData.parseError, "");
        compare(HardwareData.charging.available, true);
        compare(HardwareData.device.available, false);
        compare(HardwareData.cpuCaps.totalCores, 0);
        compare(HardwareData.displayInfo.available, false);
        compare(HardwareData.capabilities.hasDedicatedGpu, false);
        compare(HardwareData.webcam.devices.length, 0);
    }

    function test_webcamDeviceBoundIsEnforced() {
        var devices = [];
        for (var i = 0; i < 17; ++i) {
            devices.push({
                "name": "Camera " + i,
                "devicePath": "/dev/video" + i,
                "metadata": false,
                "controls": []
            });
        }
        setLive({ "sensors": {}, "charging": {}, "webcam": { "switchAvailable": true, "devices": devices } });
        compare(HardwareData.webcam.devices.length, 16);
    }

    function test_webcamControlAndMenuBoundsAreEnforced() {
        var menu = [];
        for (var i = 0; i < 129; ++i)
            menu.push({ "index": i, "key": "entry_" + i, "label": "Entry " + i });

        var controls = [];
        for (var j = 0; j < 65; ++j) {
            controls.push({
                "key": "brightness_" + j,
                "name": "Brightness " + j,
                "kind": "integer",
                "min": 0, "max": 255, "step": 1, "default": 128,
                "current": { "kind": "integer", "value": 128 },
                "menu": j === 0 ? menu : [],
                "flags": { "readOnly": true },
                "category": "basic",
                "simple": true
            });
        }
        setLive({
            "sensors": {},
            "charging": {},
            "webcam": {
                "switchAvailable": true,
                "controlsSupported": true,
                "devices": [{
                    "name": "Oversized Camera",
                    "devicePath": "/dev/video0",
                    "controls": controls
                }]
            }
        });

        compare(HardwareData.webcam.devices.length, 1);
        compare(HardwareData.webcam.devices[0].controls.length, 64);
        compare(HardwareData.webcam.devices[0].controls[0].menu.length, 128);
    }
}
