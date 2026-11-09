// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Capacités réelles dans Réglages et l'éditeur : écran interne, cœurs CPU,
// listes EPP/gouverneurs. Aucune liste inventée quand le daemon n'expose rien.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "RealCapabilities"
    when: windowShown

    Component { id: settingsComponent; SettingsPage { } }
    Component { id: editorComponent; ProfileEditorPage { } }

    readonly property string profilesJson: JSON.stringify([
        {
            "id": "custom-a", "name": "Studio", "description": "",
            "display": { "refreshRate": 120, "useRefRate": true },
            "cpu": { "onlineCores": 12, "energyPerformancePreference": "power" },
            "webcam": {}, "fan": {}
        },
        {
            "id": "custom-b", "name": "Display", "description": "",
            "display": { "refreshRate": 165, "useRefRate": true,
                         "useResolution": true, "xResolution": 2560,
                         "yResolution": 1600 },
            "cpu": {}, "webcam": {}, "fan": {}
        }
    ])

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        AppState.editingProfileId = "";
        AdminState.available = false;
        AdminState.saving = false;
        AgentData.available = false;
        AgentData.error = "";
        AgentData.displayInfoJson = "{}";
    }

    function live(hardware) {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify(hardware);
        HardwareData.hardwareError = "";
        LiveData.connected = true;
        LiveData.profilesJson = testCase.profilesJson;
    }

    function baseHardware() {
        return {
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "display": { "available": true, "hasInternalDisplay": true,
                         "internalConnector": "eDP-1", "connected": true,
                         "source": "drm" },
            "cpu": { "totalCores": 16, "onlineCores": 12,
                     "availableGovernors": ["powersave", "performance"],
                     "availableEpp": ["power", "balance_power", "performance"] }
        };
    }

    readonly property string agentJson: JSON.stringify({
        "available": true,
        "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
        "internalOutput": {
            "name": "eDP-1", "connected": true, "enabled": true,
            "hasInternalDisplay": true,
            "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60 },
            "modes": [
                { "width": 2560, "height": 1600, "refreshHz": 60 },
                { "width": 2560, "height": 1600, "refreshHz": 165 },
                { "width": 1920, "height": 1200, "refreshHz": 60 }
            ]
        },
        "supported": true, "dryRun": false, "error": null
    })

    function agentLive(json) {
        AgentData.available = true;
        AgentData.version = "0.4.2";
        AgentData.error = "";
        AgentData.displayInfoJson = json === undefined ? testCase.agentJson : json;
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function test_settingsShowsOnlyRealDisplayInfo() {
        live(baseHardware());
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        compare(HardwareData.internalDisplayLabel, "eDP-1");
        compare(testCase.exists(page, "settingsInternalDisplayRow"), true);
        compare(testCase.exists(page, "settingsAgentNotice"), true);

        // Écran absent sur snapshot fiable : la ligne est masquée.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "display": { "available": false, "hasInternalDisplay": false }
        });
        compare(HardwareData.internalDisplayLabel, "");
        compare(testCase.exists(page, "settingsInternalDisplayRow"), true);
    }

    function test_editorUsesRealCpuBoundsAndEpp() {
        live(baseHardware());
        AppState.editingProfileId = "custom-a";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.cpuMaxCores, 16);
        compare(page.cores, 12);
        compare(page.eppKnown, true);
        compare(page.eppValues.length, 3);
        compare(page.eppValues[page.eppIndex], "power");
        compare(page.performanceGovernorKnown, true);
        compare(page.agentUsable, false);
        compare(page.effectiveRefreshRate, 120);
        compare(page.effectiveUseRefresh, true);
    }

    function test_editorUsesRealResolutionAndRates() {
        live(baseHardware());
        agentLive();
        AppState.editingProfileId = "custom-b";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.agentUsable, true);
        compare(page.resolutionKnown, true);
        compare(page.resolutionIndex, 0);
        compare(page.selectedResolution.width, 2560);
        compare(page.selectedResolution.height, 1600);
        compare(page.refreshIndex, 0);
        compare(page.selectedRefreshRate, 165);
        compare(page.resolutionExists, true);
        compare(page.refreshExists, true);
        compare(page.valid, true);

        var dto = ProfileRules.toEdit(page.form);
        compare(dto.useResolution, true);
        compare(dto.xResolution, 2560);
        compare(dto.yResolution, 1600);
        compare(dto.useRefreshRate, true);
        compare(dto.refreshRate, 165);

        // Résolution 1920 × 1200 : seul 60 Hz est réellement disponible.
        page.selectResolution(1);
        compare(page.selectedResolution.width, 1920);
        compare(page.selectedResolution.height, 1200);
        compare(page.activeRates.length, 1);
        compare(page.form.refreshRate, 60);
        compare(page.refreshExists, true);
        compare(page.valid, true);
    }

    function test_editorRefusesStoredModeAbsentFromAgent() {
        live(baseHardware());
        agentLive(JSON.stringify({
            "available": true,
            "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
            "internalOutput": {
                "name": "eDP-1", "connected": true, "hasInternalDisplay": true,
                "currentMode": { "width": 1920, "height": 1200, "refreshHz": 60 },
                "modes": [{ "width": 1920, "height": 1200, "refreshHz": 60 }]
            },
            "supported": true, "error": null
        }));
        AppState.editingProfileId = "custom-b";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.agentUsable, true);
        compare(page.resolutionMissing, true);
        compare(page.form.resolutionExists, false);
        compare(page.valid, false);
        compare(page.displayError, "editor.error.resolutionUnavailable");
        compare(page.savePossible(), false);
    }

    function test_editorPreservesDisplayWhenAgentUnavailable() {
        live(baseHardware());
        AppState.editingProfileId = "custom-b";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.agentUsable, false);
        compare(page.effectiveUseResolution, true);
        compare(page.effectiveWidth, 2560);
        compare(page.effectiveHeight, 1600);
        compare(page.effectiveUseRefresh, true);
        compare(page.effectiveRefreshRate, 165);
        compare(page.valid, true);

        var dto = ProfileRules.toEdit(page.form);
        compare(dto.useResolution, true);
        compare(dto.xResolution, 2560);
        compare(dto.yResolution, 1600);
        compare(dto.refreshRate, 165);
    }

    function test_editorKeepsAutoWithoutInventedLists() {
        var hardware = baseHardware();
        hardware.cpu.availableGovernors = [];
        hardware.cpu.availableEpp = [];
        live(hardware);
        AppState.editingProfileId = "custom-a";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.cpuMaxCores, 16);
        compare(page.eppKnown, false);
        compare(page.eppValues.length, 1);
        compare(page.eppOptionLabels()[0], Strings.t("common.auto"));
        compare(page.performanceGovernorKnown, true);
        // La valeur EPP du profil reste préservée, jamais remplacée par une
        // liste matérielle vide.
        compare(page.eppValues[page.eppIndex], "power");
    }

    function test_editorWithoutHardwareKeepsProfileValues() {
        LiveData.connected = true;
        LiveData.profilesJson = testCase.profilesJson;
        AppState.editingProfileId = "custom-a";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.cpuMaxCores, 12);
        compare(page.eppValues.length, 1);
        compare(page.eppValues[0], "power");
    }

    function exists(page, name) {
        return findChild(page, name) !== null;
    }
}
