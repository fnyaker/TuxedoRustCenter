// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'icône tray native : chargement tolérant, menu, actions,
// état des toggles. Le runner est hors bureau : `SystemTrayIcon.available`
// peut être faux, ce qui doit rester sans crash.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/tray"

TestCase {
    id: testCase
    name: "Tray"
    when: windowShown

    Component {
        id: trayComponent
        SystemTray { }
    }

    SignalSpy {
        id: activationSpy
        target: AppState
        signalName: "profileActivationRequested"
    }

    SignalSpy {
        id: autostartSpy
        target: AppState
        signalName: "autostartRequested"
    }

    SignalSpy {
        id: blockerSpy
        target: AppState
        signalName: "powerBlockerRequested"
    }

    SignalSpy {
        id: settingsSpy
        target: AppState
        signalName: "settingsSaveRequested"
    }

    readonly property string profileFixture: JSON.stringify([
        { "id": "__office__", "name": "Office", "description": "",
          "display": { "refreshRate": -1, "useRefRate": false }, "cpu": {},
          "webcam": { "status": true, "useStatus": true },
          "fan": { "useControl": true, "fanProfile": "Balanced",
                   "minimumFanspeed": 0, "maximumFanspeed": 100,
                   "offsetFanspeed": 0 } },
        { "id": "custom-studio", "name": "Studio", "description": "",
          "display": { "refreshRate": -1, "useRefRate": false }, "cpu": {},
          "webcam": { "status": true, "useStatus": true },
          "fan": { "useControl": true, "fanProfile": "Balanced",
                   "minimumFanspeed": 0, "maximumFanspeed": 100,
                   "offsetFanspeed": 0 } }
    ])

    function reset() {
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        AutostartData.enabled = false;
        AutostartData.available = true;
        AutostartData.error = "";
        PowerBlockerData.available = true;
        PowerBlockerData.blocked = false;
        PowerBlockerData.pending = false;
        PowerBlockerData.error = "";
        SettingsData.connected = false;
        SettingsData.error = "";
        SettingsData.fahrenheit = false;
        SettingsData.cpuEnabled = true;
        SettingsData.fanEnabled = true;
        SettingsData.lightingEnabled = true;
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        AdminState.savingSettings = false;
        AdminState.saveSettingsError = "";
        AdminState.saveSettingsErrorDetail = "";
        AdminState.saveSettingsSucceeded = false;
        activationSpy.clear();
        autostartSpy.clear();
        blockerSpy.clear();
        settingsSpy.clear();
    }

    function init() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = profileFixture;
        AdminState.available = true;
    }

    function cleanup() {
        reset();
    }

    function test_menuModelAndActions() {
        var tray = createTemporaryObject(trayComponent, testCase);
        verify(tray !== null);
        compare(typeof tray.available, "boolean");
        compare(AppState.trayAvailable, tray.available);

        var icon = findChild(tray, "trayIcon");
        verify(icon !== null);
        verify(icon.menu !== null);

        var openItem = findChild(tray, "openMenuItem");
        var autostartItem = findChild(tray, "autostartMenuItem");
        var blockerItem = findChild(tray, "blockerMenuItem");
        var quitItem = findChild(tray, "quitMenuItem");
        verify(openItem !== null && autostartItem !== null);
        verify(blockerItem !== null && quitItem !== null);
        compare(autostartItem.checked, AutostartData.enabled);
        compare(blockerItem.checked, PowerBlockerData.blocked);

        var profileMenu = findChild(tray, "profileMenu");
        verify(profileMenu !== null);
        compare(profileMenu.items.length, 2);

        // Actions.
        var quitCount = 0;
        tray.quitRequested.connect(function() { quitCount += 1; });
        quitItem.triggered();
        compare(quitCount, 1);

        autostartItem.triggered();
        compare(autostartSpy.count, 1);
        compare(autostartSpy.signalArguments[0][0], true);

        blockerItem.triggered();
        compare(blockerSpy.count, 1);
        compare(blockerSpy.signalArguments[0][0], true);

        var firstProfile = profileMenu.items[0];
        // Simule la bascule native avant le retour du daemon.
        firstProfile.checked = true;
        firstProfile.triggered();
        compare(activationSpy.count, 1);
        compare(activationSpy.signalArguments[0][0], "__office__");
        // Le menu est reconstruit : l'état coché revient à la vérité de
        // `LiveData` même si l'activation est refusée (T2-06).
        compare(profileMenu.items.length, 2);
        compare(profileMenu.items[0].checked,
                LiveData.resolvedActiveId === "__office__");
    }

    function test_togglesFollowLiveState() {
        var tray = createTemporaryObject(trayComponent, testCase);
        var autostartItem = findChild(tray, "autostartMenuItem");
        var blockerItem = findChild(tray, "blockerMenuItem");

        AutostartData.enabled = true;
        compare(autostartItem.checked, true);
        AutostartData.enabled = false;
        compare(autostartItem.checked, false);

        PowerBlockerData.blocked = true;
        compare(blockerItem.checked, true);
        PowerBlockerData.blocked = false;
        compare(blockerItem.checked, false);
    }

    function test_autostartFailureRevertsToggle() {
        var tray = createTemporaryObject(trayComponent, testCase);
        var autostartItem = findChild(tray, "autostartMenuItem");
        verify(autostartItem !== null);

        AutostartData.enabled = false;
        compare(autostartItem.checked, false);
        autostartItem.triggered();
        compare(autostartSpy.count, 1);
        // L'intention n'est pas persistée : l'écriture réelle fait foi.
        compare(autostartItem.checked, false);

        // Échec d'écriture : l'état reste désactivé, pas de toggle fantôme.
        AutostartData.error = "WriteFailed";
        compare(autostartItem.checked, false);
        AutostartData.error = "";

        // Écriture réussie par un autre canal : l'état live est repris.
        AutostartData.enabled = true;
        compare(autostartItem.checked, true);
        AutostartData.enabled = false;
        compare(autostartItem.checked, false);
    }

    function test_fnLockTogglePersistsViaSettings() {
        SettingsData.connected = true;
        SettingsData.error = "";
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": true, "enabled": false }
        });
        var tray = createTemporaryObject(trayComponent, testCase);
        var item = findChild(tray, "fnLockMenuItem");
        verify(item !== null);
        compare(item.checkable, true);
        compare(item.checked, false);
        compare(item.enabled, true);

        item.checked = true;
        item.triggered();
        compare(settingsSpy.count, 1);
        var dto = JSON.parse(settingsSpy.signalArguments[0][0]);
        compare(dto.fnLockStatus, true);
        compare(dto.chargingType, null);
        compare(tray.fnLockDraft, true);

        // Le worker a appliqué l'état : la relecture fait foi.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": true, "enabled": true }
        });
        AdminState.saveSettingsSucceeded = true;
        compare(tray.fnLockSubmitting, false);
        compare(item.checked, true);

        // Refus : retour à l'état matériel, aucun toggle fantôme.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": true, "enabled": false }
        });
        item.checked = true;
        item.triggered();
        compare(settingsSpy.count, 2);
        AdminState.saveSettingsError = "NotAuthorized";
        compare(tray.fnLockSubmitting, false);
        compare(item.checked, false);
    }

    function test_fnLockReadOnlyWithoutWritable() {
        SettingsData.connected = true;
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": false, "enabled": false }
        });
        var tray = createTemporaryObject(trayComponent, testCase);
        var item = findChild(tray, "fnLockMenuItem");
        compare(tray.fnLockWritable, false);
        compare(item.checkable, false);
        compare(item.enabled, false);
        item.triggered();
        compare(settingsSpy.count, 0);
    }

    function test_compactPanelContentAndActions() {
        LiveData.activeProfileId = "__office__";
        var tray = createTemporaryObject(trayComponent, testCase);
        verify(tray !== null);
        var panel = findChild(tray, "trayPanel");
        verify(panel !== null);
        compare(panel.profiles.length, 2);
        compare(panel.activeProfileId, "__office__");
        compare(panel.activeProfileName, "Office");
        compare(tray.panelVisible, false);

        tray.togglePanel();
        compare(tray.panelVisible, true);
        tray.togglePanel();
        compare(tray.panelVisible, false);

        var openCount = 0;
        tray.showWindowRequested.connect(function() { openCount += 1; });
        panel.openWindowRequested();
        compare(openCount, 1);
        compare(tray.panelVisible, false);

        var quitCount = 0;
        tray.quitRequested.connect(function() { quitCount += 1; });
        panel.quitRequested();
        compare(quitCount, 1);

        activationSpy.clear();
        panel.profileRequested("custom-studio");
        compare(activationSpy.count, 1);
        compare(activationSpy.signalArguments[0][0], "custom-studio");

        verify(findChild(panel, "panelOpenButton") !== null);
        verify(findChild(panel, "panelQuitButton") !== null);
        verify(findChild(panel, "panelFnLockToggle") !== null);
        verify(findChild(panel, "panelBlockerToggle") !== null);
        verify(findChild(panel, "panelAutostartToggle") !== null);
    }

    function test_compactPanelTogglesAreWired() {
        SettingsData.connected = true;
        SettingsData.error = "";
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": { "available": true },
            "fnLock": { "supported": true, "writable": true, "enabled": false }
        });
        AutostartData.enabled = false;
        PowerBlockerData.available = true;
        PowerBlockerData.blocked = false;
        PowerBlockerData.pending = false;
        var tray = createTemporaryObject(trayComponent, testCase);
        var panel = findChild(tray, "trayPanel");
        verify(panel !== null);

        settingsSpy.clear();
        panel.fnLockRequested(true);
        compare(settingsSpy.count, 1);
        var dto = JSON.parse(settingsSpy.signalArguments[0][0]);
        compare(dto.fnLockStatus, true);

        blockerSpy.clear();
        panel.blockerRequested(true);
        compare(blockerSpy.count, 1);
        compare(blockerSpy.signalArguments[0][0], true);

        autostartSpy.clear();
        panel.autostartRequested(true);
        compare(autostartSpy.count, 1);
        compare(autostartSpy.signalArguments[0][0], true);

        verify(findChild(panel, "panelFnLockToggle") !== null);
    }

    function test_unavailableTrayDoesNotCrash() {
        var tray = createTemporaryObject(trayComponent, testCase);
        verify(tray !== null);
        // Hors bureau, available peut être faux : aucune exception et le menu
        // reste construit.
        var quitItem = findChild(tray, "quitMenuItem");
        verify(quitItem !== null);
        if (!tray.available)
            compare(tray.visible, false);
    }
}
