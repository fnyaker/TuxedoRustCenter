// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'autostart XDG (Réglages) : poussée initiale du pont, état du
// toggle, requête, erreurs, indisponibilité et notice tray.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "Autostart"
    when: windowShown

    Component {
        id: settingsComponent
        SettingsPage { }
    }

    SignalSpy {
        id: autostartSpy
        target: AppState
        signalName: "autostartRequested"
    }

    function reset() {
        AutostartData.enabled = false;
        AutostartData.available = false;
        AutostartData.error = "";
        PowerBlockerData.available = false;
        PowerBlockerData.blocked = false;
        PowerBlockerData.pending = false;
        PowerBlockerData.error = "";
        AdminState.available = false;
        AdminState.version = "";
        AdminState.error = "";
        AdminState.savingSettings = false;
        AdminState.saveSettingsError = "";
        AdminState.saveSettingsErrorDetail = "";
        AdminState.saveSettingsSucceeded = false;
        AgentData.available = false;
        AgentData.version = "";
        AgentData.error = "";
        AgentData.displayInfoJson = "{}";
        AgentData.uvcWriteSupported = false;
        AgentData.uvcPending = false;
        AgentData.uvcError = "";
        AgentData.uvcSucceeded = false;
        SettingsData.connected = false;
        SettingsData.error = "";
        AdminState.available = false;
        AppState.trayLoaded = false;
        AppState.trayAvailable = false;
        autostartSpy.clear();
    }

    /// Objet aux mêmes propriétés que `DaemonClient` (sans le type C++).
    function daemonFixture(overrides) {
        var fixture = {
            "autostartEnabled": false,
            "autostartAvailable": false,
            "autostartError": "",
            "powerBlockerAvailable": false,
            "powerBlocked": false,
            "powerBlockerPending": false,
            "powerBlockerError": "",
            "adminAvailable": false,
            "adminVersion": "",
            "adminError": "",
            "savingSettings": false,
            "saveSettingsError": "",
            "saveSettingsErrorDetail": "",
            "saveSettingsSucceeded": false,
            "agentAvailable": false,
            "agentVersion": "",
            "agentError": "",
            "displayInfoJson": "{}",
            "agentUvcWriteSupported": false,
            "agentUvcWritePending": false,
            "agentUvcWriteError": "",
            "agentUvcWriteSucceeded": false
        };
        if (overrides) {
            for (var key in overrides)
                fixture[key] = overrides[key];
        }
        return fixture;
    }

    function init() {
        reset();
        SettingsData.connected = true;
    }

    function cleanup() {
        reset();
    }

    function autostartRow(page) {
        var row = findChild(page, "autostartRow");
        verify(row !== null);
        return row;
    }

    function test_initialBridgeStateIsPushedWithoutSignal() {
        // Valeurs par défaut des singletons : aucun signal `changed` ne peut
        // avoir été émis par le pont Rust avant le chargement du QML.
        compare(AutostartData.enabled, false);
        compare(AutostartData.available, false);
        compare(AutostartData.error, "");

        BridgeState.pushInitial(daemonFixture({
            "autostartEnabled": true,
            "autostartAvailable": true,
            "autostartError": ""
        }));

        compare(AutostartData.enabled, true);
        compare(AutostartData.available, true);
        compare(AutostartData.error, "");
        compare(AutostartData.editable, true);
        compare(AutostartData.hasError, false);
    }

    function test_initialBridgePushesServiceStates() {
        BridgeState.pushInitial(daemonFixture({
            "powerBlockerAvailable": true,
            "powerBlocked": true,
            "adminAvailable": true,
            "adminVersion": "1.2.3",
            "agentAvailable": true,
            "agentVersion": "0.9.0"
        }));

        compare(PowerBlockerData.available, true);
        compare(PowerBlockerData.blocked, true);
        compare(AdminState.available, true);
        compare(AdminState.version, "1.2.3");
        compare(AgentData.available, true);
        compare(AgentData.version, "0.9.0");
    }

    function test_toggleReflectsStateAndSendsRequest() {
        AutostartData.enabled = true;
        AutostartData.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        var row = autostartRow(page);
        compare(row.checked, true);
        compare(row.supported, true);

        row.toggled(false);
        compare(autostartSpy.count, 1);
        compare(autostartSpy.signalArguments[0][0], false);

        // Réponse réelle du pont (succès) : le toggle suit.
        AutostartData.enabled = false;
        compare(row.checked, false);
    }

    function test_noHomeAloneShowsDedicatedMessage() {
        // Poussée initiale : `NoHome` est le seul code qui doit afficher
        // « Répertoire utilisateur introuvable ».
        BridgeState.pushInitial(daemonFixture({
            "autostartAvailable": false,
            "autostartError": "NoHome"
        }));
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = autostartRow(page);

        compare(row.supported, false);
        compare(row.unsupportedReason, Strings.t("settings.autostartNoHome"));
        compare(AutostartData.hasError, true);
        compare(AutostartData.errorKey, "settings.autostartNoHome");
    }

    function test_unavailableWithoutCodeDoesNotClaimNoHome() {
        // `available=false` + `error=""` : état non résolu, pas « NoHome ».
        AutostartData.available = false;
        AutostartData.error = "";
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = autostartRow(page);

        compare(row.supported, false);
        compare(row.unsupportedReason, Strings.t("common.unavailable"));
        verify(row.unsupportedReason !== Strings.t("settings.autostartNoHome"));
        compare(AutostartData.hasError, false);
    }

    function test_availableWithoutErrorShowsNoMessage() {
        AutostartData.available = true;
        AutostartData.enabled = true;
        AutostartData.error = "";
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = autostartRow(page);

        compare(row.supported, true);
        compare(row.checked, true);
        // Aucune erreur : le bandeau n'a pas de contenu et le toggle reste
        // actionnable, sans message « répertoire introuvable ».
        compare(AutostartData.hasError, false);
        compare(AutostartData.errorText, "");
        compare(findChild(page, "autostartError").body, "");
    }

    function test_errorAndUnavailableAreExposed() {
        AutostartData.available = false;
        AutostartData.error = "NoHome";
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = autostartRow(page);
        compare(row.supported, false);
        compare(row.unsupportedReason, Strings.t("settings.autostartNoHome"));

        AutostartData.available = true;
        AutostartData.error = "PathInvalid";
        compare(AutostartData.errorKey, "settings.autostartPathInvalid");
        compare(AutostartData.errorText, Strings.t("settings.autostartPathInvalid"));
        compare(AutostartData.hasError, true);

        AutostartData.error = "WriteFailed";
        compare(AutostartData.errorKey, "settings.autostartWriteFailed");
        AutostartData.error = "InsecurePermissions";
        compare(AutostartData.errorKey, "settings.autostartInsecure");
        AutostartData.error = "Bogus";
        compare(AutostartData.errorKey, "settings.autostartError");
        AutostartData.error = "";
        compare(AutostartData.hasError, false);
    }

    function test_trayUnavailableNoticeState() {
        AppState.trayLoaded = true;
        AppState.trayAvailable = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.trayUnavailable, false);

        // Tray perdu : la notice s'affiche, l'état QML reste cohérent.
        AppState.trayAvailable = false;
        compare(page.trayUnavailable, true);
        AppState.trayAvailable = true;
        compare(page.trayUnavailable, false);
        AppState.trayLoaded = false;
        compare(page.trayUnavailable, false);
    }
}
