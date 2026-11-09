// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'adaptateur `AgentData` (User1 → modes réels de l'écran interne).

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "AgentData"
    when: windowShown

    readonly property string displayJson: JSON.stringify({
        "available": true,
        "session": { "type": "wayland", "desktop": "Hyprland",
                     "waylandDisplay": "wayland-1", "display": null },
        "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
        "internalOutput": {
            "name": "eDP-1", "connected": true, "enabled": true,
            "hasInternalDisplay": true,
            "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60,
                             "preferred": false, "current": true },
            "modes": [
                { "width": 2560, "height": 1600, "refreshHz": 60,
                  "preferred": false, "current": true },
                { "width": 2560, "height": 1600, "refreshHz": 165,
                  "preferred": true, "current": false },
                { "width": 1920, "height": 1200, "refreshHz": 60,
                  "preferred": false, "current": false },
                { "width": 1920, "height": 1200, "refreshHz": 60,
                  "preferred": false, "current": false },
                { "width": 99999, "height": 1600, "refreshHz": 60,
                  "preferred": false, "current": false }
            ]
        },
        "supported": true,
        "dryRun": false,
        "error": null,
        "externalOutput": { "name": "DP-1",
                            "modes": [{ "width": 3840, "height": 2160,
                                        "refreshHz": 60 }] }
    })

    function reset() {
        AgentData.available = false;
        AgentData.version = "";
        AgentData.error = "";
        AgentData.displayInfoJson = "{}";
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function test_realModesAreNormalized() {
        AgentData.available = true;
        AgentData.version = "0.4.0";
        AgentData.displayInfoJson = testCase.displayJson;

        compare(AgentData.usable, true);
        compare(AgentData.display.backend.kind, "hyprland");
        compare(AgentData.display.output.name, "eDP-1");
        compare(AgentData.display.output.hasInternalDisplay, true);
        // Mode invalide (largeur hors bornes) écarté.
        compare(AgentData.modes.length, 4);
        compare(AgentData.currentRate, 60);
        compare(AgentData.currentModeLabel,
                "2560 × 1600 @ " + Strings.formatHz(60));

        // Résolutions uniques, triées par surface.
        var resolutions = AgentData.resolutions();
        compare(resolutions.length, 2);
        compare(resolutions[0].width, 2560);
        compare(resolutions[1].width, 1920);
        compare(resolutions[0].label,
                Strings.tf("display.resolutionFormat", [2560, 1600]));

        // Taux réels, triés décroissants et dédupliqués.
        compare(AgentData.refreshRates(2560, 1600).join(","), "165,60");
        compare(AgentData.refreshRates(1920, 1200).join(","), "60");
        compare(AgentData.refreshRates(1280, 800).length, 0);
        compare(AgentData.refreshIndex(2560, 1600, 165), 0);
        compare(AgentData.refreshIndex(2560, 1600, 60), 1);
        compare(AgentData.modeExists(2560, 1600, 165.4), true);
        compare(AgentData.modeExists(2560, 1600, 120), false);
        compare(AgentData.resolutionExists(1920, 1200), true);
        compare(AgentData.resolutionExists(1280, 800), false);
        // L'écran externe n'apparaît jamais.
        compare(AgentData.resolutionExists(3840, 2160), false);
    }

    function test_unsupportedAndDormantStates() {
        AgentData.available = true;
        AgentData.displayInfoJson = JSON.stringify({
            "available": true, "supported": false,
            "internalOutput": { "name": "", "hasInternalDisplay": false,
                                "modes": [] }
        });
        compare(AgentData.usable, false);
        compare(AgentData.dormant, false);
        compare(AgentData.resolutions().length, 0);

        AgentData.available = true;
        AgentData.displayInfoJson = "{}";
        compare(AgentData.usable, false);
        compare(AgentData.dormant, true);

        AgentData.available = false;
        AgentData.displayInfoJson = testCase.displayJson;
        compare(AgentData.usable, false);
    }

    function test_backendUnavailableFixtureIsExposed() {
        AgentData.available = true;
        AgentData.displayInfoJson = JSON.stringify({
            "available": false,
            "backend": { "kind": "hyprland", "name": "hyprland",
                         "available": false },
            "internalOutput": { "name": "", "hasInternalDisplay": false,
                                "modes": [] },
            "supported": false,
            "error": "backend absent"
        });
        compare(AgentData.usable, false);
        compare(AgentData.display.available, false);
        compare(AgentData.display.supported, false);
        compare(AgentData.display.error, "backend absent");
        compare(AgentData.modes.length, 0);
        compare(AgentData.resolutions().length, 0);
    }

    function test_errorsAreMappedAndInvalidJsonIsRejected() {
        AgentData.available = false;
        AgentData.error = "Contract";
        compare(AgentData.errorKey, "display.agentContract");
        AgentData.error = "Transport";
        compare(AgentData.errorKey, "display.agentTransport");
        AgentData.error = "Timeout";
        compare(AgentData.errorKey, "display.agentTimeout");
        AgentData.error = "InvalidPayload";
        compare(AgentData.errorKey, "display.agentInvalidPayload");
        AgentData.error = "Unavailable";
        compare(AgentData.errorKey, "display.agentUnavailable");

        AgentData.displayInfoJson = "{ pas du json";
        verify(AgentData.parseError !== "");
        compare(AgentData.usable, false);
        compare(AgentData.modes.length, 0);
    }

    function test_invalidModeValuesAreDropped() {
        AgentData.available = true;
        AgentData.displayInfoJson = JSON.stringify({
            "available": true, "supported": true,
            "internalOutput": {
                "name": "eDP-1", "hasInternalDisplay": true,
                "modes": [
                    { "width": 0, "height": 1080, "refreshHz": 60 },
                    { "width": 1920, "height": 1080, "refreshHz": 0 },
                    { "width": 1920, "height": 1080, "refreshHz": 1200 },
                    { "width": 16385, "height": 1080, "refreshHz": 60 },
                    { "width": 1920, "height": 16385, "refreshHz": 60 },
                    { "width": 1920, "height": 1080, "refreshHz": 120 }
                ]
            }
        });
        compare(AgentData.modes.length, 1);
        compare(AgentData.modes[0].refreshHz, 120);
    }
}
