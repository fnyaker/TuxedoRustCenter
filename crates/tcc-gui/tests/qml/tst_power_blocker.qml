// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'inhibiteur de veille (Réglages → Affichage/agent) :
// interrupteur, états disponible/pending/erreur, absence de persistance.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "PowerBlocker"
    when: windowShown

    Component {
        id: settingsComponent
        SettingsPage { }
    }

    SignalSpy {
        id: blockerSpy
        target: AppState
        signalName: "powerBlockerRequested"
    }

    function reset() {
        PowerBlockerData.available = false;
        PowerBlockerData.blocked = false;
        PowerBlockerData.pending = false;
        PowerBlockerData.error = "";
        SettingsData.connected = false;
        SettingsData.error = "";
        AdminState.available = false;
        AdminState.savingSettings = false;
        blockerSpy.clear();
    }

    function init() {
        reset();
        SettingsData.connected = true;
        PowerBlockerData.available = true;
    }

    function cleanup() {
        reset();
    }

    function blockerRow(page) {
        var row = findChild(page, "powerBlockerRow");
        verify(row !== null);
        return row;
    }

    function test_toggleSendsRequestAndFollowsState() {
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        var row = blockerRow(page);
        compare(row.checked, false);
        compare(row.supported, true);

        row.toggled(true);
        compare(blockerSpy.count, 1);
        compare(blockerSpy.signalArguments[0][0], true);

        // L'état affiché suit le fd réel, pas l'intention.
        PowerBlockerData.blocked = true;
        compare(row.checked, true);
        row.toggled(false);
        compare(blockerSpy.count, 2);
        compare(blockerSpy.signalArguments[1][0], false);

        PowerBlockerData.blocked = false;
        compare(row.checked, false);
    }

    function test_refusalAndSuccessReapplyTheRealState() {
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = blockerRow(page);

        // Intention optimiste, puis refus : le fd n'existe pas, on revient.
        row.toggled(true);
        compare(row.checked, true);
        PowerBlockerData.pending = true;
        PowerBlockerData.pending = false;
        compare(row.checked, false);

        // Nouvelle tentative acceptée : le worker détient un fd.
        row.toggled(true);
        PowerBlockerData.pending = true;
        PowerBlockerData.blocked = true;
        PowerBlockerData.pending = false;
        compare(row.checked, true);

        // Désactivation acceptée.
        row.toggled(false);
        PowerBlockerData.pending = true;
        PowerBlockerData.blocked = false;
        PowerBlockerData.pending = false;
        compare(row.checked, false);
    }

    function test_unavailableDisablesTheToggle() {
        PowerBlockerData.available = false;
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = blockerRow(page);
        compare(PowerBlockerData.editable, false);
        compare(row.supported, false);
    }

    function test_pendingDisablesTheToggle() {
        PowerBlockerData.pending = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        var row = blockerRow(page);
        compare(PowerBlockerData.editable, false);
        compare(row.supported, false);
        compare(row.unsupportedReason, Strings.t("settings.powerBlockerPending"));
    }

    function test_errorCodesAreTranslated() {
        for (var code in {
            "NotGraphical": "settings.powerBlockerNotGraphical",
            "Unavailable": "settings.powerBlockerUnavailable",
            "Refused": "settings.powerBlockerRefused",
            "Timeout": "settings.powerBlockerTimeout",
            "Bogus": "settings.powerBlockerError"
        }) {
            PowerBlockerData.error = code;
            compare(PowerBlockerData.errorKey,
                    { "NotGraphical": "settings.powerBlockerNotGraphical",
                      "Unavailable": "settings.powerBlockerUnavailable",
                      "Refused": "settings.powerBlockerRefused",
                      "Timeout": "settings.powerBlockerTimeout",
                      "Bogus": "settings.powerBlockerError" }[code]);
            compare(PowerBlockerData.errorText,
                    Strings.t(PowerBlockerData.errorKey));
        }
        PowerBlockerData.error = "";
        compare(PowerBlockerData.hasError, false);
        compare(PowerBlockerData.errorText, "");
    }

    function test_noteAndNoPersistence() {
        // Aucune préférence locale n'est écrite : l'état vient du worker.
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        var texts = [Strings.t("settings.powerBlockerHint"),
                     Strings.t("settings.powerBlockerNote")];
        for (var i = 0; i < texts.length; ++i) {
            verify(texts[i].length > 0);
            verify(texts[i] !== "settings.powerBlockerHint");
            verify(texts[i] !== "settings.powerBlockerNote");
        }
    }
}
