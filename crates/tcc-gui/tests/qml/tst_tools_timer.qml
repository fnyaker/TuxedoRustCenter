// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de la minuterie d'extinction (page Outils) : synchronisation depuis
// `Settings.shutdownTime`, brouillon heure/minute, application/annulation via
// `Admin1.SaveSettingsJSON`, statut réel `GetShutdownTimerJSON`.
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "ToolsTimer"
    when: windowShown

    Component {
        id: toolsComponent
        ToolsPage { }
    }

    SignalSpy {
        id: saveSpy
        target: AppState
        signalName: "settingsSaveRequested"
    }

    function reset() {
        AdminState.available = false;
        AdminState.savingSettings = false;
        AdminState.saveSettingsError = "";
        AdminState.saveSettingsErrorDetail = "";
        AdminState.saveSettingsSucceeded = false;
        SettingsData.connected = false;
        SettingsData.fahrenheit = false;
        SettingsData.cpuEnabled = true;
        SettingsData.fanEnabled = true;
        SettingsData.lightingEnabled = true;
        SettingsData.shutdownTime = "";
        SettingsData.error = "";
        ShutdownTimerData.available = false;
        ShutdownTimerData.enabled = false;
        ShutdownTimerData.scheduledAt = "";
        ShutdownTimerData.source = "none";
        ShutdownTimerData.error = "";
        saveSpy.clear();
    }

    function init() {
        reset();
        SettingsData.connected = true;
        ShutdownTimerData.available = true;
    }

    function cleanup() {
        reset();
    }

    function timerPage() {
        AdminState.available = true;
        return createTemporaryObject(toolsComponent, testCase);
    }

    function test_timerSyncsFromSettings() {
        SettingsData.shutdownTime = "22:30";
        var page = timerPage();
        verify(page !== null);
        compare(page.timerEnabled, true);
        compare(page.timerHour, 22);
        compare(page.timerMinute, 30);
        compare(page.timerDirty(), false);

        // Relecture (aucun brouillon) : la valeur du daemon fait foi.
        SettingsData.shutdownTime = "07:05";
        compare(page.timerHour, 7);
        compare(page.timerMinute, 5);
        compare(page.timerDirty(), false);

        SettingsData.shutdownTime = "";
        compare(page.timerEnabled, false);
        compare(page.timerDirty(), false);
    }

    function test_timerEditSendsSelectedTime() {
        SettingsData.shutdownTime = "22:30";
        var page = timerPage();
        page.timerHour = 7;
        page.timerMinute = 5;
        compare(page.timerDirty(), true);
        page.submitTimer();

        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.shutdownTime, "07:05");
        compare(dto.clearShutdownTime, false);
        // Champs hors minuterie : neutres.
        compare(dto.chargingProfile, null);
        compare(dto.stateMatrix, null);
        compare(dto.fahrenheit, SettingsData.fahrenheit);
        compare(dto.cpuSettingsEnabled, SettingsData.cpuEnabled);
    }

    function test_timerToggleOffClearsSchedule() {
        SettingsData.shutdownTime = "22:30";
        var page = timerPage();
        page.timerEnabled = false;
        compare(page.timerDirty(), true);
        page.submitTimer();

        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.shutdownTime, null);
        compare(dto.clearShutdownTime, true);
        compare(page.timerClearing, true);
    }

    function test_timerCancelSendsClear() {
        SettingsData.shutdownTime = "22:30";
        ShutdownTimerData.enabled = true;
        ShutdownTimerData.scheduledAt = "2026-11-06 22:30";
        ShutdownTimerData.source = "settings";
        var page = timerPage();
        compare(page.timerEditable(), true);
        page.cancelTimerSchedule();

        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.clearShutdownTime, true);
        compare(dto.shutdownTime, null);
    }

    function test_timerDisabledWithoutAdminOrStatus() {
        var page = timerPage();
        compare(page.timerEditable(), true);
        AdminState.savingSettings = true;
        compare(page.timerEditable(), false);
        AdminState.savingSettings = false;

        AdminState.available = false;
        compare(page.timerEditable(), false);
        AdminState.available = true;

        // Statut réel indisponible : pas d'édition (état logind inconnu).
        ShutdownTimerData.available = false;
        compare(page.timerEditable(), false);
        var toggle = findChild(page, "timerEnableRow");
        verify(toggle !== null);
        compare(toggle.supported, false);
    }

    function test_timerStatusExposed() {
        ShutdownTimerData.enabled = true;
        ShutdownTimerData.scheduledAt = "2026-11-06 22:30";
        ShutdownTimerData.source = "settings";
        compare(ShutdownTimerData.scheduled, true);
        compare(ShutdownTimerData.sourceKey, "tools.timer.sourceSettings");
        compare(ShutdownTimerData.sourceText,
                Strings.t("tools.timer.sourceSettings"));

        ShutdownTimerData.available = false;
        compare(ShutdownTimerData.scheduled, false);
    }

    function test_timerButtonsFollowDraft() {
        SettingsData.shutdownTime = "";
        var page = timerPage();
        var apply = findChild(page, "timerApplyButton");
        var cancel = findChild(page, "timerCancelButton");
        verify(apply !== null);
        verify(cancel !== null);
        compare(apply.enabled, false);
        // Aucune planification à annuler.
        compare(cancel.enabled, false);

        SettingsData.shutdownTime = "22:00";
        compare(cancel.enabled, true);
        page.timerHour = 23;
        compare(apply.enabled, true);
        compare(page.timerDirty(), true);
    }

    function test_timerDraftKeptOnErrorAndSyncedOnSuccess() {
        SettingsData.shutdownTime = "22:30";
        var page = timerPage();
        page.timerHour = 6;
        page.submitTimer();
        compare(page.timerSubmitting, true);

        AdminState.saveSettingsError = "Conflict";
        compare(page.timerSubmitting, false);
        compare(page.timerHour, 6);
        compare(page.timerDirty(), true);

        page.submitTimer();
        SettingsData.shutdownTime = "06:00";
        AdminState.saveSettingsSucceeded = true;
        compare(page.timerSubmitting, false);
        compare(page.timerEnabled, true);
        compare(page.timerHour, 6);
        compare(page.timerMinute, 0);
        compare(page.timerDirty(), false);
        compare(AppState.toasts.get(AppState.toasts.count - 1).message,
                Strings.t("toast.timerSet"));
    }

    function test_timerErrorSourceIsNotCancelled() {
        ShutdownTimerData.available = true;
        ShutdownTimerData.enabled = false;
        ShutdownTimerData.source = "error";
        ShutdownTimerData.error = "logind indisponible";
        var page = timerPage();

        compare(ShutdownTimerData.errorState, true);
        compare(ShutdownTimerData.statusKind, "error");
        compare(ShutdownTimerData.statusText, Strings.t("tools.timer.statusError"));
        compare(ShutdownTimerData.sourceKey, "tools.timer.sourceError");
        compare(ShutdownTimerData.errorText, "logind indisponible");
        compare(page.timerStatusError, true);
        compare(page.timerEditable(), false);

        // `source=error` sans détail : message générique, pas « annulé ».
        ShutdownTimerData.error = "";
        compare(ShutdownTimerData.errorText, Strings.t("tools.timer.errorGeneric"));

        // `none` reste distinct et réactive l'édition.
        ShutdownTimerData.source = "none";
        compare(ShutdownTimerData.errorState, false);
        compare(ShutdownTimerData.statusText, Strings.t("tools.timer.statusNone"));
        compare(page.timerStatusError, false);
        compare(page.timerEditable(), true);
    }

    function test_timerTimeHelpers() {
        var page = timerPage();
        compare(page.hourOptions().length, 24);
        compare(page.minuteOptions().length, 60);
        compare(page.hourOptions()[0], "00");
        compare(page.minuteOptions()[59], "59");
        compare(page.parseShutdownTime("23:59").hour, 23);
        compare(page.parseShutdownTime("00:00").minute, 0);
        compare(page.parseShutdownTime("24:00"), null);
        compare(page.parseShutdownTime("9:00"), null);
        compare(page.parseShutdownTime(""), null);
        page.timerHour = 8;
        page.timerMinute = 5;
        compare(page.timerTime(), "08:05");
    }
}
