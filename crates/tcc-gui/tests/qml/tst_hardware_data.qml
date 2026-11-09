// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'adaptateur `HardwareData` (Manager1 → affichage).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml
//
// Les payloads reproduisent la forme canonique publiée par le pont Rust
// (`hardware_view`), qui a déjà rejeté les clés inconnues.

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "HardwareData"
    when: windowShown

    readonly property string liveJson: JSON.stringify({
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
            "availableEpp": ["power", "balance_power", "performance"],
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
            "profiles": ["balanced", "high_capacity"],
            "profile": "balanced",
            "priorities": ["charging", "performance"],
            "priority": "performance",
            "battery": "BAT0",
            "startThreshold": 60,
            "endThreshold": 90,
            "startThresholds": [40, 50, 60],
            "endThresholds": [80, 90, 100],
            "chargeType": "Custom",
            "chargeTypeAvailable": true,
            "chargeTypeWritable": true,
            "chargeTypes": ["Fast", "Standard"],
            "writeSupported": true,
            "profileWritable": true,
            "priorityWritable": true,
            "thresholdsWritable": true,
            "thresholdsAvailable": true,
            "applied": true,
            "errors": []
        },
        "lighting": {
            "available": true,
            "enabled": true,
            "unsupported": false,
            "keyboard": { "available": true, "writable": true,
                          "modes": ["static", "breathing"], "kind": "rgb",
                          "zones": 3, "maxBrightness": 100, "maxRed": 255,
                          "maxGreen": 255, "maxBlue": 255, "bufferInput": true },
            "lightbar": { "available": true, "writable": true,
                          "modes": ["static"], "maxBrightness": 80,
                          "maxRed": 255, "maxGreen": 255, "maxBlue": 255 },
            "states": [{ "brightness": 50, "red": 1, "green": 2, "blue": 3 }],
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
                "current": "performance",
                "choices": ["power_save", "balanced", "performance"],
                "defaultProfile": "balanced",
                "acpiQuirkBlocked": false,
                "owner": "ppd",
                "applied": "balanced"
            },
            "io": {
                "available": true,
                "interface": "uniwill",
                "moduleVersion": "0.2.6",
                "odmAvailable": true,
                "choices": ["power_save", "enthusiast", "overboost"],
                "defaultProfile": "enthusiast",
                "tdpAvailable": false
            },
            "tdpAvailable": true,
            "tdp": [{ "descriptor": "pl1", "current": 35, "min": 10, "max": 80 }],
            "interface": "uniwill",
            "moduleVersion": "0.2.6",
            "applied": true,
            "errors": []
        },
        "sensors": {
            "enabled": true,
            "available": true,
            "cpu": { "temperatureC": 55.5, "frequencyMhz": 3200, "powerW": 28.5 },
            "igpu": { "vendor": "intel", "role": "integrated", "name": "Intel iGPU",
                      "card": "card1", "temperatureC": 54.0, "frequencyMhz": 1100,
                      "maxFrequencyMhz": 1650, "powerW": 4.5 },
            "dgpu": { "vendor": "nvidia", "role": "discrete", "name": "RTX 4060",
                      "card": "card2", "temperatureC": 61.0, "frequencyMhz": 1350,
                      "maxFrequencyMhz": 2400, "powerW": 42.0 },
            "gpus": [],
            "fans": [{ "name": "CPU", "temp": 55.5, "speed": 42.0, "applied": true }]
        },
        "nvidia": {
            "available": true,
            "attributeAvailable": true,
            "nvidiaSmiAvailable": true,
            "ctgpOffset": 10,
            "defaultPowerLimit": 60,
            "maxPowerLimit": 90,
            "appliedOffset": 10,
            "errors": []
        },
        "webcam": {
            "switchAvailable": true,
            "enabled": false,
            "controlsSupported": false,
            "devices": [{ "name": "Integrated Camera", "devicePath": "/dev/video0",
                          "deviceName": "Integrated Camera", "index": 0,
                          "metadata": false }],
            "applied": false,
            "errors": []
        },
        "fnLock": { "supported": true, "writable": true, "enabled": true },
        "prime": {
            "available": true,
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
            "ports": [{ "card": 0, "port": "DP-1", "forced": false }],
            "pendingModeReapply": false,
            "applied": true,
            "errors": []
        }
    })

    /// Instantané `GetFanDataJSON` (lecture seule).
    readonly property string fanJson: JSON.stringify({
        "available": true,
        "backend": "tuxi",
        "writable": true,
        "controlEnabled": true,
        "mode": "manual",
        "fans": [
            { "index": 0, "label": "cpu", "target": "cpu",
              "temp": 57, "speed": 42, "applied": 45 },
            { "index": 1, "label": "gpu1", "target": "gpu",
              "temp": null, "speed": null, "applied": null }
        ]
    })

    /// Même payload avec la collecte expirée côté daemon.
    readonly property string expiredJson: JSON.stringify({
        "charging": { "available": false },
        "lighting": { "available": false, "keyboard": { "kind": "none" } },
        "odm": { "available": false, "platformProfile": {} },
        "sensors": { "enabled": false, "available": false, "cpu": {}, "fans": [] },
        "nvidia": { "available": false },
        "webcam": { "switchAvailable": false, "devices": [] },
        "fnLock": { "supported": false },
        "prime": { "available": false },
        "ycbcr": { "available": false, "ports": [] }
    })

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        HardwareData.fanError = "";
        HardwareData.sensorsRequested = false;
        HardwareData.dashboardVisible = false;
    }

    function cleanup() {
        reset();
    }

    function test_demoFallbackWhenDisconnected() {
        reset();
        compare(HardwareData.usingDemo, true);
        compare(HardwareData.isLive, false);
        compare(HardwareData.parseError, "");
        // La démo est disponible et cohérente.
        compare(HardwareData.cpu.available, true);
        verify(HardwareData.fans.length > 0);
        compare(HardwareData.fan.mode, "auto");
        compare(HardwareData.charging.available, true);
        compare(HardwareData.sensorsActive, false);
        compare(HardwareData.leaseExpired, false);
        compare(HardwareData.sensorsPending, false);
        // Aucune valeur produit inventée en démonstration.
        compare(HardwareData.device.available, false);
        compare(HardwareData.device.productName, "");
        compare(HardwareData.deviceLabel, "");
        compare(HardwareData.cpuCaps.totalCores, 0);
        compare(HardwareData.cpuCoresText, "");
        compare(HardwareData.displayInfo.available, false);
        compare(HardwareData.internalDisplayLabel, "");
        compare(HardwareData.capabilities.hasBattery, false);
        compare(HardwareData.prime.available, false);
    }

    function test_deviceTextAndCountsAreBounded() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "device": { "productName": "Book\u0000X", "vendor": null },
            "cpu": { "totalCores": 999999, "onlineCores": -4 },
            "display": { "internalConnector": "eDP-1", "connected": false },
            "capabilities": {}
        });
        compare(HardwareData.isLive, true);
        compare(HardwareData.device.productName, "BookX");
        compare(HardwareData.device.vendor, "");
        compare(HardwareData.cpuCaps.totalCores, 4096);
        compare(HardwareData.cpuCaps.onlineCores, 0);
        compare(HardwareData.internalDisplayLabel, "eDP-1 (déconnecté)");
        compare(HardwareData.capabilities.hasBattery, false);
    }

    function test_disconnectedPayloadIsNotParsed() {
        reset();
        HardwareData.hardwareJson = liveJson;
        compare(HardwareData.usingDemo, true);
        compare(HardwareData.isLive, false);
    }

    function test_livePayloadIsNormalized() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        HardwareData.fanJson = fanJson;

        compare(HardwareData.usingDemo, false);
        compare(HardwareData.isLive, true);
        compare(HardwareData.parseError, "");

        // Appareil et capacités : valeurs réelles, sans DemoData.
        compare(HardwareData.device.available, true);
        compare(HardwareData.device.vendor, "TUXEDO Computers GmbH");
        compare(HardwareData.device.productName, "TUXEDO Book X");
        compare(HardwareData.device.sku, "IBM15A10");
        compare(HardwareData.device.kernelRelease, "6.12.0-tuxedo");
        compare(HardwareData.device.architecture, "x86_64");
        compare(HardwareData.device.tuxedoDriverVersion, "0.3.9");
        compare(HardwareData.deviceLabel, "TUXEDO Book X");
        compare(HardwareData.deviceVendor, "TUXEDO Computers GmbH");
        compare(AgentData.available, false);

        compare(HardwareData.cpuCaps.totalCores, 16);
        compare(HardwareData.cpuCaps.onlineCores, 12);
        compare(HardwareData.cpuCoresText, "12 / 16");
        compare(HardwareData.cpuCaps.availableGovernors.length, 2);
        compare(HardwareData.cpuCaps.availableEpp[2], "performance");
        compare(HardwareData.cpuCaps.scalingDriver, "intel_pstate");

        compare(HardwareData.displayInfo.available, true);
        compare(HardwareData.displayInfo.hasInternalDisplay, true);
        compare(HardwareData.internalDisplayLabel, "eDP-1");
        compare(HardwareData.capabilities.hasBattery, true);
        compare(HardwareData.capabilities.hasAc, true);
        compare(HardwareData.capabilities.hasDedicatedGpu, true);

        // CPU : MHz → GHz, unités conservées.
        compare(HardwareData.cpu.temp, 55.5);
        compare(HardwareData.cpu.freqGhz, 3.2);
        compare(HardwareData.cpu.powerW, 28.5);
        compare(HardwareData.cpu.available, true);

        // GPU.
        compare(HardwareData.igpu.present, true);
        compare(HardwareData.igpu.temp, 54.0);
        compare(HardwareData.igpu.freqGhz, 1.1);
        compare(HardwareData.igpu.tempAvailable, true);
        compare(HardwareData.igpu.freqAvailable, true);
        compare(HardwareData.igpu.powerAvailable, true);
        compare(HardwareData.dgpu.present, true);
        compare(HardwareData.dgpu.name, "RTX 4060");
        compare(HardwareData.dgpu.freqGhz, 1.35);
        compare(HardwareData.dgpu.powerW, 42.0);
        compare(HardwareData.dgpu.tempAvailable, true);
        compare(HardwareData.dgpu.freqAvailable, true);
        compare(HardwareData.dgpu.powerAvailable, true);

        // Ventilateurs : instantané dédié `GetFanDataJSON`.
        compare(HardwareData.fans.length, 2);
        compare(HardwareData.fan.mode, "manual");
        compare(HardwareData.fan.backend, "tuxi");
        compare(HardwareData.fan.writable, true);
        compare(HardwareData.fans[0].speed, 42);
        compare(HardwareData.fans[0].applied, 45);
        compare(HardwareData.fans[1].speed, -1);
        compare(HardwareData.fanStale, false);

        // Charge.
        compare(HardwareData.charging.profile, "balanced");
        compare(HardwareData.charging.priority, "performance");
        compare(HardwareData.charging.start, 60);
        compare(HardwareData.charging.end, 90);
        compare(HardwareData.charging.chargeType, "Custom");
        compare(HardwareData.charging.chargeTypeWritable, true);
        compare(HardwareData.charging.chargeTypes.length, 2);
        compare(HardwareData.charging.chargeTypes[0], "Fast");
        compare(HardwareData.charging.writeSupported, true);
        compare(HardwareData.charging.thresholdsAvailable, true);
        compare(HardwareData.charging.profiles.length, 2);
        compare(HardwareData.charging.profiles[1], "high_capacity");
        compare(HardwareData.charging.priorities[0], "charging");
        compare(HardwareData.charging.startThresholds.length, 3);
        compare(HardwareData.charging.startThresholds[2], 60);
        compare(HardwareData.charging.endThresholds[0], 80);

        // Éclairage.
        compare(HardwareData.lighting.keyboard.kind, "rgb");
        compare(HardwareData.lighting.keyboard.zones, 3);
        compare(HardwareData.lighting.keyboard.writable, true);
        compare(HardwareData.lighting.keyboard.modes.length, 2);
        compare(HardwareData.lighting.keyboard.maxRed, 255);
        compare(HardwareData.lighting.lightbar.writable, true);
        compare(HardwareData.lighting.lightbar.modes[0], "static");
        compare(HardwareData.lighting.states.length, 1);
        compare(HardwareData.lighting.states[0].blue, 3);
        compare(HardwareData.lighting.lightbar.available, true);

        // Fn-Lock, ODM, NVIDIA, PRIME, YCbCr.
        compare(HardwareData.fnLock.supported, true);
        compare(HardwareData.fnLock.enabled, true);
        compare(HardwareData.fnLock.writable, true);
        compare(HardwareData.fnLock.enabledKnown, true);
        compare(HardwareData.odm.platform.current, "performance");
        compare(HardwareData.odm.platform.writable, true);
        compare(HardwareData.odm.platform.owner, "ppd");
        compare(HardwareData.odm.odmWritable, true);
        compare(HardwareData.odm.io.odmAvailable, true);
        compare(HardwareData.odm.io.choices[1], "enthusiast");
        compare(HardwareData.odmSourceKind, "platform_profile");
        compare(HardwareData.odmChoices[0], "power_save");
        compare(HardwareData.odmDefaultProfile, "balanced");
        compare(HardwareData.odmWritable, true);
        compare(HardwareData.odm.tdp.length, 1);
        compare(HardwareData.odm.tdp[0].descriptor, "pl1");
        compare(HardwareData.odm.tdp[0].current, 35);
        compare(HardwareData.nvidia.ctgpOffset, 10);
        compare(HardwareData.nvidia.maxPowerLimit, 90);
        compare(HardwareData.prime.mode, "on-demand");
        compare(HardwareData.prime.requiresOffloading, true);
        compare(HardwareData.ycbcr.ports.length, 1);
        compare(HardwareData.ycbcr.ports[0].port, "DP-1");

        // Webcam.
        compare(HardwareData.webcam.switchAvailable, true);
        compare(HardwareData.webcam.enabled, false);
        compare(HardwareData.webcam.devices.length, 1);
        compare(HardwareData.webcam.devices[0].path, "/dev/video0");

        // Collecte active.
        compare(HardwareData.sensorsActive, true);
        compare(HardwareData.leaseExpired, false);
    }

    function test_gpuMetricAvailabilityFollowsNulls() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {
                "enabled": true, "available": true, "cpu": {},
                "igpu": { "vendor": "intel", "role": "integrated",
                          "temperatureC": 50, "frequencyMhz": null,
                          "powerW": null },
                "dgpu": { "vendor": "nvidia", "role": "discrete",
                          "temperatureC": null, "frequencyMhz": null,
                          "powerW": null }
            },
            "charging": {}
        });
        compare(HardwareData.igpu.present, true);
        compare(HardwareData.igpu.tempAvailable, true);
        compare(HardwareData.igpu.freqAvailable, false);
        compare(HardwareData.igpu.powerAvailable, false);
        compare(HardwareData.dgpu.present, true);
        compare(HardwareData.dgpu.tempAvailable, false);
        compare(HardwareData.dgpu.freqAvailable, false);
        compare(HardwareData.dgpu.powerAvailable, false);
        compare(HardwareData.dgpu.available, false);
    }

    function test_odmFallsBackToTuxedoIo() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true,
                "odmWritable": true,
                "tdpWritable": false,
                "platformProfile": { "available": false, "owner": "" },
                "io": {
                    "available": true,
                    "interface": "uniwill",
                    "moduleVersion": "0.2.6",
                    "odmAvailable": true,
                    "choices": ["power_save", "enthusiast", "overboost"],
                    "defaultProfile": "enthusiast",
                    "tdpAvailable": false
                },
                "tdpAvailable": false,
                "tdp": []
            },
            "nvidia": { "available": false }
        });
        compare(HardwareData.isLive, true);
        compare(HardwareData.odm.available, true);
        compare(HardwareData.odm.platform.available, false);
        compare(HardwareData.odmSourceKind, "tuxedo-io");
        compare(HardwareData.odmChoices.length, 3);
        compare(HardwareData.odmChoices[1], "enthusiast");
        compare(HardwareData.odmCurrentProfile, "");
        compare(HardwareData.odmDefaultProfile, "enthusiast");
        compare(HardwareData.odmWritable, true);
        compare(HardwareData.tdpWritable, false);
        compare(HardwareData.odm.tdpAvailable, false);
    }

    function test_sensorLeaseStates() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        HardwareData.fanJson = fanJson;
        HardwareData.sensorsRequested = true;
        HardwareData.dashboardVisible = true;
        compare(HardwareData.sensorsActive, true);
        compare(HardwareData.leaseExpired, false);

        // Bail retombé côté daemon alors que l'utilisateur l'a demandé et que
        // le Dashboard est visible : expiration réelle.
        HardwareData.hardwareJson = expiredJson;
        compare(HardwareData.sensorsActive, false);
        compare(HardwareData.leaseExpired, true);
        compare(HardwareData.sensorsPending, false);

        // Hors Dashboard, l'expiration est volontaire : état « en attente ».
        HardwareData.hardwareJson = liveJson;
        HardwareData.hardwareJson = expiredJson;
        HardwareData.dashboardVisible = false;
        compare(HardwareData.leaseExpired, false);
        compare(HardwareData.sensorsPending, true);

        // L'utilisateur n'a rien demandé : aucun état particulier.
        HardwareData.sensorsRequested = false;
        compare(HardwareData.leaseExpired, false);
        compare(HardwareData.sensorsPending, false);
    }

    function test_staleDataFlag() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        compare(HardwareData.stale, false);
        HardwareData.hardwareError = "GetHardwareSnapshotJSON: timeout";
        HardwareData.fanError = "GetFanDataJSON: timeout";
        compare(HardwareData.stale, true);
        compare(HardwareData.fanStale, true);
        // Le dernier payload valide reste affiché.
        compare(HardwareData.isLive, true);
        compare(HardwareData.cpu.temp, 55.5);
    }

    function test_invalidJsonFallsBackToDemo() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = "{ pas du json";
        compare(HardwareData.usingDemo, true);
        compare(HardwareData.isLive, false);
        verify(HardwareData.parseError !== "");
        compare(HardwareData.charging.available, true); // demo
    }

    function test_invalidFanJsonFallsBackToDemo() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        HardwareData.fanJson = "{ pas du json";
        compare(HardwareData.usingDemo, true);
        verify(HardwareData.parseError !== "");
    }

    function test_fanUnavailableSnapshot() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        HardwareData.fanJson = JSON.stringify({
            "available": false, "backend": null, "writable": false,
            "controlEnabled": false, "mode": "off", "fans": []
        });
        compare(HardwareData.isLive, true);
        compare(HardwareData.fan.available, false);
        compare(HardwareData.fan.mode, "off");
        compare(HardwareData.fans.length, 0);
    }

    function test_uvcControlsAreNormalized() {
        var payload = JSON.stringify({
            "sensors": {},
            "charging": {},
            "webcam": {
                "controlsSupported": true,
                "controlsTruncated": false,
                "errors": [],
                "devices": [{
                    "name": "Integrated Camera",
                    "devicePath": "/dev/video0",
                    "controlsError": "",
                    "controls": [{
                        "key": "brightness",
                        "name": "Brightness",
                        "kind": "integer",
                        "min": 0, "max": 255, "step": 1, "default": 128,
                        "current": { "kind": "integer", "value": 100 },
                        "menu": [],
                        "flags": { "readOnly": true },
                        "category": "basic",
                        "simple": true
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
        });
        HardwareData.connected = true;
        HardwareData.hardwareJson = payload;
        compare(HardwareData.webcam.controlsSupported, true);
        compare(HardwareData.webcam.controlsWritable, false);
        compare(HardwareData.webcam.controlsError, "");
        var controls = HardwareData.webcam.devices[0].controls;
        compare(controls.length, 2);
        compare(controls[0].name, "Brightness");
        compare(controls[0].current.value, 100);
        compare(controls[0].flags.readOnly, true);
        compare(controls[1].kind, "menu");
        compare(controls[1].current.label, "Manual Mode");
        compare(controls[1].menu.length, 2);
        compare(controls[1].menu[1].label, "Manual Mode");
    }

    function test_webcamControlsErrorIsExposed() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": {},
            "webcam": {
                "controlsSupported": false,
                "errors": [],
                "devices": [{ "name": "Cam", "controlsError": "EACCES" }]
            }
        });
        compare(HardwareData.webcam.controlsError, "EACCES");
    }

    function test_chargeListsAreSanitized() {
        compare(HardwareData._thresholdList([90, "x", 40, 40, 120, 60]).join(","),
                "40,60,90");
        compare(HardwareData._chargingList(["a", "", 3, "b"]).join(","), "a,b");
    }

    function test_arrayPayloadFallsBackToDemo() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = "[]";
        compare(HardwareData.usingDemo, true);
        verify(HardwareData.parseError !== "");
    }

    function test_missingSectionsFallBackToDemo() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({ "sensors": {}, "lighting": {} });
        compare(HardwareData.usingDemo, true);
        verify(HardwareData.parseError !== "");
    }

    function test_unchangedPayloadIsNotReparsed() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = liveJson;
        var first = HardwareData.livePayload;
        HardwareData.hardwareJson = liveJson;
        verify(HardwareData.livePayload === first);
    }

    function test_emptySectionsDegradeGracefully() {
        reset();
        HardwareData.connected = true;
        HardwareData.hardwareJson = expiredJson;
        compare(HardwareData.isLive, true);
        compare(HardwareData.usingDemo, false);
        compare(HardwareData.cpu.available, false);
        compare(HardwareData.cpu.temp, 0);
        compare(HardwareData.fans.length, 0);
        compare(HardwareData.charging.available, false);
        compare(HardwareData.fan.available, false);
        compare(HardwareData.fans.length, 0);
        compare(HardwareData.lighting.keyboard.kind, "none");
        compare(HardwareData.fnLock.supported, false);
        compare(HardwareData.nvidia.available, false);
        compare(HardwareData.prime.mode, "unknown");
        compare(HardwareData.ycbcr.available, false);
    }

    function test_sensorCollectionRequestsAreSignalled() {
        sensorSpy.clear();
        AppState.requestSensorCollection(true);
        compare(sensorSpy.count, 1);
        compare(sensorSpy.signalArguments[0][0], true);

        hardwareSpy.clear();
        AppState.requestHardwareRefresh();
        compare(hardwareSpy.count, 1);
    }

    SignalSpy { id: sensorSpy; target: AppState; signalName: "sensorCollectionRequested" }
    SignalSpy { id: hardwareSpy; target: AppState; signalName: "hardwareRefreshRequested" }
}
