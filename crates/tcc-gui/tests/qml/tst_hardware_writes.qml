// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Écritures matérielles persistantes : type de charge, Fn-Lock et éclairage
// via `SettingsEdit` / `SaveSettingsJSON`. Aucun setter `Manager1`.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "HardwareWrites"
    when: windowShown

    Component { id: settingsComponent; SettingsPage { } }
    Component { id: toolsComponent; ToolsPage { } }

    SignalSpy { id: saveSpy; target: AppState; signalName: "settingsSaveRequested" }

    readonly property string hardwareJson: JSON.stringify({
        "sensors": { "enabled": true, "available": true, "cpu": {} },
        "charging": {
            "available": true, "writeSupported": true,
            "chargeTypeAvailable": true, "chargeTypeWritable": true,
            "chargeTypes": ["Fast", "Standard"], "chargeType": "Fast",
            "profiles": ["balanced"], "profile": "balanced",
            "priorities": ["balanced"], "priority": "balanced",
            "startThreshold": 60, "endThreshold": 80,
            "startThresholds": [40, 60], "endThresholds": [80, 90],
            "thresholdsAvailable": true
        },
        "lighting": {
            "available": true, "enabled": true, "unsupported": false,
            "keyboard": { "available": true, "writable": true,
                          "modes": ["static"], "kind": "rgb", "zones": 2,
                          "maxBrightness": 100, "maxRed": 255,
                          "maxGreen": 255, "maxBlue": 255 },
            "lightbar": { "available": true, "writable": true,
                          "modes": ["static"], "maxBrightness": 100,
                          "maxRed": 255, "maxGreen": 255, "maxBlue": 255 },
            "states": [{ "brightness": 50, "red": 10, "green": 20, "blue": 30 }],
            "lightbarState": { "brightness": 40, "red": 1, "green": 2, "blue": 3 }
        },
        "fnLock": { "supported": true, "writable": true, "enabled": false }
    })

    function reset() {
        AdminState.available = false;
        AdminState.error = "";
        AdminState.savingSettings = false;
        AdminState.saveSettingsError = "";
        AdminState.saveSettingsErrorDetail = "";
        AdminState.saveSettingsSucceeded = false;
        SettingsData.connected = false;
        SettingsData.error = "";
        SettingsData.fahrenheit = false;
        SettingsData.cpuEnabled = true;
        SettingsData.fanEnabled = true;
        SettingsData.lightingEnabled = true;
        SettingsData.chargingProfile = "";
        SettingsData.chargingType = "";
        SettingsData.keyboardMode = "";
        SettingsData.chargingPriority = "";
        SettingsData.chargingStart = -1;
        SettingsData.chargingEnd = -1;
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        saveSpy.clear();
    }

    function live() {
        AdminState.available = true;
        AdminState.savingSettings = false;
        SettingsData.connected = true;
        SettingsData.error = "";
        HardwareData.connected = true;
        HardwareData.hardwareJson = testCase.hardwareJson;
        HardwareData.hardwareError = "";
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function test_chargeTypeSavesOnlyWhenSelected() {
        live();
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        compare(page.chargeTypeSelectable, true);
        compare(page.chargeTypeDraft, "");
        compare(page.chargeTypeOptions()[0], "");
        compare(page.chargeTypeLabels()[0],
                Strings.t("settings.chargeTypeUnchanged"));
        compare(page.chargeTypeOptions().indexOf("Fast") > 0, true);
        compare(page.chargeDirty(), false);

        page.chargeTypeDraft = "Fast";
        compare(page.chargeDirty(), true);
        compare(page.chargeValid(), true);
        page.submitCharge();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.chargingType, "Fast");
        compare(dto.chargingProfile, null);
        compare(dto.fnLockStatus, null);
    }

    function test_chargeTypeHiddenWhenNotWritable() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true, "chargeTypeWritable": false,
                          "chargeTypes": [], "profiles": [], "priorities": [] },
            "fnLock": { "supported": false }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.chargeTypeSelectable, false);
        compare(page.chargeEditable(), true);
        page.chargeTypeDraft = "Fast";
        page.submitCharge();
        // L'écriture de charge reste possible (autres champs), mais le type
        // n'est jamais envoyé : il n'est pas modifiable.
        compare(saveSpy.count, 0);
    }

    function test_fnLockTogglePersistsStatus() {
        live();
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        compare(page.fnLockVisible, true);
        compare(HardwareData.fnLock.writable, true);
        compare(page.fnLockDraft, false);
        page.submitFnLock(true);
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.fnLockStatus, true);
        compare(page.fnLockDraft, true);

        // Matériel non inscriptible : le toggle n'est pas proposé et aucun
        // appel n'est émis.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": false, "enabled": false }
        });
        page.submitFnLock(true);
        compare(saveSpy.count, 1);
    }

    function test_fnLockErrorRevertsDraft() {
        live();
        var page = createTemporaryObject(settingsComponent, testCase);
        page.submitFnLock(true);
        compare(page.fnLockDraft, true);
        AdminState.saveSettingsError = "NotAuthorized";
        compare(page.fnLockSubmitting, false);
        compare(page.fnLockDraft, false);
        compare(AdminState.saveSettingsErrorText !== "", true);
    }

    function test_lightingKeyboardSavesZonesAndLightbar() {
        live();
        var page = createTemporaryObject(toolsComponent, testCase);
        verify(page !== null);
        compare(page.keyboardAvailable, true);
        compare(page.keyboardBrightness, 50);
        compare(page.keyboardRed, 10);
        compare(page.lightingDirty(), false);

        page.keyboardBrightness = 80;
        page.keyboardRed = 200;
        compare(page.lightingDirty(), true);
        page.submitLighting();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.keyboardStates.length, 2);
        compare(dto.keyboardStates[0].brightness, 80);
        compare(dto.keyboardStates[0].red, 200);
        compare(dto.keyboardStates[0].green, 20);
        compare(dto.keyboardStates[0].blue, 30);
        compare(dto.keyboardStates[1].red, 200);
        compare(dto.lightbarStates.brightness, 40);
        compare(dto.lightbarStates.red, 1);
        compare(dto.chargingType, null);
        compare(dto.fnLockStatus, null);
    }

    function test_whiteKeyboardSendsUnsupportedColors() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "lighting": {
                "available": true, "enabled": true,
                "keyboard": { "available": true, "writable": true,
                              "modes": ["static"], "kind": "white",
                              "zones": 1, "maxBrightness": 3 },
                "lightbar": { "available": false },
                "states": [{ "brightness": 2, "red": 0, "green": 0, "blue": 0 }]
            },
            "fnLock": { "supported": false }
        });
        var page = createTemporaryObject(toolsComponent, testCase);
        compare(page.keyboardRgb, false);
        compare(page.keyboardMaxRed, 0);
        page.keyboardBrightness = 3;
        page.submitLighting();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.keyboardStates.length, 1);
        compare(dto.keyboardStates[0].red, -1);
        compare(dto.keyboardStates[0].green, -1);
        compare(dto.keyboardStates[0].blue, -1);
        compare(dto.lightbarStates, undefined);
    }

    function test_breathingModeIsPersistedWhenSupported() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "lighting": {
                "available": true, "enabled": true,
                "keyboard": { "available": true, "writable": true,
                              "modes": ["static", "breathing"], "kind": "rgb",
                              "zones": 1, "maxBrightness": 100,
                              "maxRed": 255, "maxGreen": 255, "maxBlue": 255 },
                "lightbar": { "available": false },
                "states": [{ "brightness": 50, "red": 1, "green": 2, "blue": 3 }]
            },
            "fnLock": { "supported": false }
        });
        SettingsData.keyboardMode = "breathing";
        var page = createTemporaryObject(toolsComponent, testCase);
        compare(page.keyboardBreathing, true);
        compare(page.keyboardMode, "breathing");
        page.keyboardBrightness = 60;
        page.submitLighting();
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.keyboardStates[0].mode, "breathing");
    }

    function test_lightingErrorKeepsDraft() {
        live();
        var page = createTemporaryObject(toolsComponent, testCase);
        page.keyboardBrightness = 80;
        page.submitLighting();
        compare(page.keyboardBrightness, 80);
        AdminState.saveSettingsError = "Validation";
        compare(page.lightingSubmitting, false);
        compare(page.lightingError !== "", true);
        compare(page.keyboardBrightness, 80);
    }

    function test_lightingGlobalToggleDisablesWrites() {
        live();
        SettingsData.lightingEnabled = false;
        var page = createTemporaryObject(toolsComponent, testCase);
        compare(page.keyboardEditable, false);
        compare(page.lightingEditable, false);
        page.keyboardBrightness = 80;
        page.submitLighting();
        compare(saveSpy.count, 0);
    }
}
