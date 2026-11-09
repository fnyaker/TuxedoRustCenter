// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Règles de visibilité matérielle : absence confirmée par un snapshot live
// fiable = bloc masqué ; chargement/démo/payload périmé = bloc conservé.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"
import "../../qml/tray"

TestCase {
    id: testCase
    name: "HardwareVisibility"
    when: windowShown

    Component { id: dashboardComponent; DashboardPage { } }
    Component { id: settingsComponent; SettingsPage { } }
    Component { id: toolsComponent; ToolsPage { } }
    Component { id: editorComponent; ProfileEditorPage { } }
    Component { id: trayComponent; SystemTray { } }

    readonly property string profilesJson: JSON.stringify([
        {
            "id": "custom-a",
            "name": "Studio",
            "description": "",
            "display": { "refreshRate": -1, "useRefRate": false },
            "cpu": {},
            "webcam": { "status": true, "useStatus": true },
            "fan": { "useControl": true, "fanProfile": "Balanced",
                     "minimumFanspeed": 0, "maximumFanspeed": 100,
                     "offsetFanspeed": 0 }
        }
    ])

    function basePayload() {
        return {
            "sensors": { "enabled": true, "available": true,
                         "cpu": { "temperatureC": 50.0 } },
            "charging": { "available": true },
            "lighting": { "available": true,
                          "keyboard": { "available": true, "kind": "white" },
                          "lightbar": { "available": true } },
            "fnLock": { "supported": true, "enabled": false },
            "odm": { "available": true, "tdpAvailable": true,
                     "tdp": [{ "descriptor": "pl1", "current": 45,
                               "min": 15, "max": 75 }] },
            "nvidia": { "available": true },
            "prime": { "available": true, "supported": true },
            "ycbcr": { "available": true },
            "webcam": { "switchAvailable": true,
                        "devices": [{ "name": "Integrated Camera" }] }
        };
    }

    function live(overrides) {
        var payload = basePayload();
        for (var key in overrides)
            payload[key] = overrides[key];
        HardwareData.connected = true;
        HardwareData.hardwareError = "";
        HardwareData.hardwareJson = JSON.stringify(payload);
    }

    function absent(overrides) {
        live(overrides);
    }

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        AppState.editingProfileId = "";
    }

    function init() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = profilesJson;
    }

    function cleanup() {
        reset();
    }

    function exists(page, name) {
        return findChild(page, name) !== null;
    }

    function test_loadingAndDemoKeepBlocksVisible() {
        // Aucun snapshot live : aucune absence ne doit être déduite.
        var dashboard = createTemporaryObject(dashboardComponent, testCase);
        compare(dashboard.dgpuVisible, true);
        compare(dashboard.igpuVisible, true);

        var settings = createTemporaryObject(settingsComponent, testCase);
        compare(settings.chargingVisible, true);
        compare(settings.capabilitiesVisible, true);

        var tools = createTemporaryObject(toolsComponent, testCase);
        compare(tools.lightingVisible, true);
        compare(tools.webcamVisible, true);

        // Payload périmé : même si le dernier snapshot dit « absent », les
        // blocs restent visibles (valeur potentiellement obsolète).
        live({ "charging": { "available": false } });
        HardwareData.hardwareError = "transport";
        compare(settings.chargingVisible, true);
        HardwareData.hardwareError = "";
    }

    function test_dashboardHidesAbsentGpusButKeepsInactive() {
        live({});
        var page = createTemporaryObject(dashboardComponent, testCase);
        compare(page.igpuVisible, false);
        compare(page.dgpuVisible, false);
        // Le CPU reste affiché (carte Processeur indépendante) ; les cartes
        // masquées ne consomment pas de place.
        compare(exists(page, "dashboardDgpuCard"), true);

        // Le graphe CPU reste dans la carte Processeur, même sans GPU.
        var cpuChart = findChild(page, "dashboardCpuChart");
        verify(cpuChart !== null);
        compare(page.cpuChartVisible, true);

        // Le graphe dGPU ne s'affiche pas sans dGPU (aucune carte orpheline).
        var dgpuBlock = findChild(page, "dashboardDgpuChartBlock");
        verify(dgpuBlock !== null);
        compare(page.dgpuChartVisible, false);

        // GPU présents mais capteurs inactifs : carte conservée avec état.
        live({
            "sensors": { "enabled": false, "available": true,
                         "cpu": { "temperatureC": null },
                         "igpu": { "vendor": "intel", "role": "integrated",
                                   "temperatureC": null, "frequencyMhz": null },
                         "dgpu": { "vendor": "nvidia", "role": "discrete",
                                   "temperatureC": null, "frequencyMhz": null }
            }
        });
        compare(page.dgpuVisible, true);
        compare(page.igpuVisible, true);
    }

    function test_settingsHideAbsentHardware() {
        live({
            "charging": { "available": false },
            "lighting": { "available": false,
                          "keyboard": { "available": false },
                          "lightbar": { "available": false } },
            "fnLock": { "supported": false },
            "odm": { "available": false, "tdpAvailable": false, "tdp": [] },
            "nvidia": { "available": false },
            "prime": { "available": false, "supported": false },
            "ycbcr": { "available": false }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.chargingVisible, false);
        compare(page.primeVisible, false);
        compare(page.capabilitiesVisible, false);
        compare(page.lightingVisible, false);
        compare(page.fnLockVisible, false);
        compare(page.ycbcrVisible, false);
        compare(page.hwPresent, false);

        // Snapshot complet : les blocs reviennent.
        live({});
        compare(page.chargingVisible, true);
        compare(page.primeVisible, true);
        compare(page.capabilitiesVisible, true);
        compare(page.hwPresent, true);
        compare(page.lightingVisible, true);
        compare(page.fnLockVisible, true);
    }

    function test_toolsHideAbsentCards() {
        live({
            "lighting": { "available": false,
                          "keyboard": { "available": false },
                          "lightbar": { "available": false } },
            "webcam": { "switchAvailable": false, "devices": [] }
        });
        var page = createTemporaryObject(toolsComponent, testCase);
        compare(page.lightingVisible, false);
        compare(page.webcamVisible, false);
        // La minuterie logicielle reste présente et visible.
        verify(exists(page, "toolsTimerCard"));

        live({});
        compare(page.lightingVisible, true);
        compare(page.webcamVisible, true);
    }

    function test_editorHidesAbsentSections() {
        AppState.editingProfileId = "custom-a";
        live({
            "nvidia": { "available": false },
            "odm": { "available": false, "tdpAvailable": false, "tdp": [] },
            "webcam": { "switchAvailable": false, "devices": [] }
        });
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.webcamVisible, false);
        compare(page.odmCardVisible, false);

        live({});
        compare(page.webcamVisible, true);
        compare(page.odmCardVisible, true);
    }

    function test_editorShowsOdmReferralWithIoFallback() {
        AppState.editingProfileId = "custom-a";
        live({
            "odm": {
                "available": true,
                "platformProfile": { "available": false, "owner": "" },
                "io": {
                    "available": true, "odmAvailable": true,
                    "choices": ["power_save", "enthusiast", "overboost"],
                    "defaultProfile": "enthusiast",
                    "tdpAvailable": false
                },
                "tdpAvailable": false,
                "tdp": []
            }
        });
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.odmCardVisible, true);
        compare(HardwareData.odm.available, true);
        compare(HardwareData.odm.platform.available, false);
        compare(HardwareData.odmSourceKind, "tuxedo-io");
    }

    function test_trayHidesUnsupportedEntries() {
        live({
            "fnLock": { "supported": false },
            "prime": { "available": false, "supported": false }
        });
        var tray = createTemporaryObject(trayComponent, testCase);
        verify(tray !== null);
        compare(tray.fnLockVisible, false);
        compare(tray.graphicsVisible, false);
        verify(exists(tray, "fnLockMenuItem"));

        live({});
        compare(tray.fnLockVisible, true);
        compare(tray.graphicsVisible, true);
    }
}
