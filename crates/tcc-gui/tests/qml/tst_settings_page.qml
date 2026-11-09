// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de la page Réglages : interrupteurs persistants, état
// d'enregistrement, refus, conflit et synchronisation après relecture.
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "SettingsPage"
    when: windowShown

    Component {
        id: settingsComponent
        SettingsPage { }
    }

    SignalSpy {
        id: saveSpy
        target: AppState
        signalName: "settingsSaveRequested"
    }

    function reset() {
        AdminState.available = false;
        AdminState.error = "";
        AdminState.savingSettings = false;
        AdminState.saveSettingsError = "";
        AdminState.saveSettingsErrorDetail = "";
        AdminState.saveSettingsSucceeded = false;
        SettingsData.connected = false;
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
        SettingsData.matrixPowerSaverAc = "";
        SettingsData.matrixPowerSaverBat = "";
        SettingsData.matrixBalancedAc = "";
        SettingsData.matrixBalancedBat = "";
        SettingsData.matrixPerformanceAc = "";
        SettingsData.matrixPerformanceBat = "";
        SettingsData.error = "";
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        saveSpy.clear();
    }

    readonly property string profileFixture: JSON.stringify([
        {
            "id": "__office__",
            "name": "Office",
            "description": "Preset",
            "display": { "refreshRate": -1, "useRefRate": false },
            "cpu": {},
            "webcam": { "status": true, "useStatus": true },
            "fan": { "useControl": true, "fanProfile": "Balanced",
                     "minimumFanspeed": 0, "maximumFanspeed": 100,
                     "offsetFanspeed": 0 }
        },
        {
            "id": "custom-studio",
            "name": "Studio",
            "description": "Custom",
            "display": { "refreshRate": -1, "useRefRate": false },
            "cpu": {},
            "webcam": { "status": true, "useStatus": true },
            "fan": { "useControl": true, "fanProfile": "Balanced",
                     "minimumFanspeed": 0, "maximumFanspeed": 100,
                     "offsetFanspeed": 0 }
        }
    ])

    function init() {
        reset();
        SettingsData.connected = true;
        LiveData.connected = true;
        LiveData.profilesJson = profileFixture;
    }

    function cleanup() {
        reset();
    }

    function test_editableOnlyWithAdmin() {
        AdminState.available = false;
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        compare(page.settingsEditable(), false);
        page.fahrenheit = true;
        page.submitSettings();
        compare(saveSpy.count, 0);

        AdminState.available = true;
        compare(page.settingsEditable(), true);
        AdminState.savingSettings = true;
        compare(page.settingsEditable(), false);
    }

    function test_toggleSendsFullDto() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        page.cpuEnabled = false;
        page.fanEnabled = false;
        page.lightingEnabled = false;
        page.submitSettings();

        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.fahrenheit, false);
        compare(dto.cpuSettingsEnabled, false);
        compare(dto.fanControlEnabled, false);
        compare(dto.keyboardBacklightControlEnabled, false);
        // Aucun champ non concerné ne fuit dans le DTO ; les champs de charge
        // non modifiés sont explicitement `null` (aucun écrasement).
        compare(dto.chargingProfile, null);
        compare(dto.chargingPriority, null);
        compare(dto.chargingStartThreshold, null);
        compare(dto.chargingEndThreshold, null);
        compare(dto.stateMatrix, null);
        compare(dto.shutdownTime, null);
        compare(dto.clearShutdownTime, false);
        compare(dto.ppdOverride, null);
        compare(dto.chargingType, null);
        compare(dto.fnLockStatus, null);
        compare(Object.keys(dto).length, 14);
    }


    readonly property string chargeHardwareJson: JSON.stringify({
        "sensors": {},
        "charging": {
            "available": true,
            "profiles": ["balanced", "high_capacity"],
            "profile": "balanced",
            "priorities": ["charging", "performance"],
            "priority": "performance",
            "startThreshold": 60,
            "endThreshold": 90,
            "startThresholds": [40, 50, 60, 90],
            "endThresholds": [80, 90, 100],
            "chargeType": "Custom",
            "thresholdsAvailable": true,
            "applied": true,
            "errors": []
        }
    })

    function useLiveHardware() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = chargeHardwareJson;
    }

    function chargePage(adminAvailable) {
        AdminState.available = adminAvailable === undefined ? true : adminAvailable;
        useLiveHardware();
        return createTemporaryObject(settingsComponent, testCase);
    }

    function test_chargeListsAndValuesComeFromHardware() {
        var page = chargePage();
        verify(page !== null);
        compare(page.chargeEditable(), true);
        compare(page.chargeProfile, "balanced");
        compare(page.chargePriority, "performance");
        compare(page.chargeStart, 60);
        compare(page.chargeEnd, 90);
        compare(page.chargeProfileIndex(), 0);
        compare(page.chargePriorityIndex(), 1);
        compare(page.chargeStartIndex(), 2);
        compare(page.chargeEndIndex(), 1);
        compare(page.thresholdLabels([40, 100]).length, 2);
        verify(page.thresholdLabels([40, 100])[0].indexOf("40") >= 0);
        compare(page.chargeDirty(), false);
    }

    function test_chargeEditSendsOnlyModifiedFields() {
        var page = chargePage();
        page.chargeProfile = "high_capacity";
        page.chargeStart = 40;
        compare(page.chargeDirty(), true);
        compare(page.chargeValid(), true);
        page.submitCharge();

        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.chargingProfile, "high_capacity");
        compare(dto.chargingStartThreshold, 40);
        // Non modifiés : null, jamais une valeur d'écrasement.
        compare(dto.chargingPriority, null);
        compare(dto.chargingEndThreshold, null);
        compare(dto.fahrenheit, page.fahrenheit);
        compare(dto.cpuSettingsEnabled, page.cpuEnabled);
    }

    function test_chargeValidationRejectsRangeAndUnknownValues() {
        var page = chargePage();
        // Début = fin = 90 : les deux valeurs existent, mais l'ordre est refusé.
        page.chargeStart = 90;
        page.chargeEnd = 90;
        compare(page.chargeRangeInvalid, true);
        compare(page.chargeValid(), false);
        page.submitCharge();
        compare(saveSpy.count, 0);

        // Valeur absente des listes matérielles.
        page.chargeStart = 55;
        compare(page.chargeStartInvalid, true);
        compare(page.chargeValid(), false);
        page.submitCharge();
        compare(saveSpy.count, 0);

        // Couple valide : la demande part.
        page.chargeStart = 40;
        page.chargeEnd = 100;
        compare(page.chargeValid(), true);
        page.submitCharge();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.chargingStartThreshold, 40);
        compare(dto.chargingEndThreshold, 100);
    }

    function test_chargeEditingRequiresAdminHardwareAndIdle() {
        var page = chargePage(false);
        compare(page.chargeEditable(), false);

        AdminState.available = true;
        compare(page.chargeEditable(), true);
        AdminState.savingSettings = true;
        compare(page.chargeEditable(), false);
        AdminState.savingSettings = false;

        // Collecte/instantané matériel indisponible.
        HardwareData.connected = false;
        compare(page.chargeEditable(), false);
        HardwareData.connected = true;
        compare(page.chargeEditable(), true);
    }

    function test_chargeButtonsFollowDraftValidity() {
        var page = chargePage();
        var apply = findChild(page, "chargeApplyButton");
        var reset = findChild(page, "chargeResetButton");
        verify(apply !== null);
        verify(reset !== null);
        compare(apply.enabled, false);
        compare(reset.enabled, false);

        page.chargeProfile = "high_capacity";
        compare(apply.enabled, true);
        compare(reset.enabled, true);

        page.chargeStart = 90;
        page.chargeEnd = 90;
        compare(apply.enabled, false);

        page.resetCharge();
        compare(page.chargeDirty(), false);
        compare(apply.enabled, false);
        compare(page.chargeProfile, "balanced");
    }

    function test_chargeDraftKeptOnErrorAndSyncedOnSuccess() {
        var page = chargePage();
        page.chargeProfile = "high_capacity";
        page.submitCharge();
        compare(page.chargeSubmitting, true);

        // Refus : le brouillon reste pour correction, le conflit est signalé.
        AdminState.saveSettingsError = "Conflict";
        compare(page.chargeSubmitting, false);
        compare(page.chargeProfile, "high_capacity");
        compare(page.chargeDirty(), true);
        compare(page.settingsConflict, true);

        // Nouvel essai puis succès : le daemon relit les réglages persistés
        // avant de signaler le succès ; le brouillon repart de cette vérité.
        page.submitCharge();
        SettingsData.chargingProfile = "high_capacity";
        AdminState.saveSettingsSucceeded = true;
        compare(page.chargeSubmitting, false);
        compare(page.chargeProfile, "high_capacity");
        compare(page.chargeDirty(), false);
        compare(page.settingsConflict, false);
    }

    function test_chargeSaveErrorIsTranslated() {
        var page = chargePage();
        page.chargeProfile = "high_capacity";
        page.submitCharge();
        AdminState.saveSettingsError = "NotAuthorized";
        compare(AdminState.saveSettingsErrorKey, "admin.error.notAuthorized");
        verify(AdminState.saveSettingsErrorText.length > 0);
    }


    function matrixPage() {
        AdminState.available = true;
        return createTemporaryObject(settingsComponent, testCase);
    }

    function test_matrixCellsComeFromSettingsAndProfiles() {
        SettingsData.matrixBalancedAc = "custom-studio";
        SettingsData.matrixPerformanceBat = "__office__";
        var page = matrixPage();

        compare(page.matrixEditable(), true);
        compare(page.matrixCell("balanced", "ac"), "custom-studio");
        compare(page.matrixCell("performance", "battery"), "__office__");
        // Cellule absente : fallback AC/BAT.
        compare(page.matrixCell("power-saver", "ac"), "");
        compare(page.matrixDirty(), false);

        // Options : fallback en tête, puis les profils live.
        var labels = page.matrixOptionLabels();
        compare(labels[0], Strings.t("settings.matrixFallback"));
        compare(labels.length, 3);
        compare(page.matrixOptionIndex("balanced", "ac"), 2);
        compare(page.matrixOptionIndex("power-saver", "ac"), 0);
    }

    function test_matrixEditSendsTheWholePartialMatrix() {
        SettingsData.matrixBalancedAc = "custom-studio";
        SettingsData.matrixPerformanceBat = "__office__";
        var page = matrixPage();

        page.setMatrixCell("balanced", "ac", "__office__");
        compare(page.matrixDirty(), true);
        compare(page.matrixValid(), true);
        page.submitMatrix();

        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        var matrix = dto.stateMatrix;
        compare(matrix.length, 2);
        var byKey = {};
        for (var i = 0; i < matrix.length; ++i)
            byKey[matrix[i].ppd + "/" + matrix[i].power] = matrix[i].profileId;
        compare(byKey["balanced/ac"], "__office__");
        compare(byKey["performance/battery"], "__office__");
        // Les autres champs restent neutres.
        compare(dto.chargingProfile, null);
        compare(dto.fahrenheit, page.fahrenheit);
    }

    function test_matrixClearingACellUsesFallback() {
        SettingsData.matrixBalancedBat = "custom-studio";
        var page = matrixPage();
        compare(page.matrixDirty(), false);

        page.setMatrixCell("balanced", "battery", "");
        compare(page.matrixDirty(), true);
        page.submitMatrix();
        var dto = JSON.parse(saveSpy.signalArguments[0][0]);
        compare(dto.stateMatrix.length, 0);
    }

    function test_matrixRejectsUnknownProfiles() {
        var page = matrixPage();
        page.setMatrixCell("balanced", "ac", "custom-gone");
        compare(page.matrixCellInvalid("balanced", "ac"), true);
        compare(page.matrixValid(), false);
        page.submitMatrix();
        compare(saveSpy.count, 0);
        compare(page.matrixDirty(), true);

        // Réinitialiser efface le brouillon invalide.
        page.resetMatrix();
        compare(page.matrixDirty(), false);
        compare(page.matrixValid(), true);
    }

    function test_matrixDisabledWithoutAdminOrProfiles() {
        var page = matrixPage();
        compare(page.matrixEditable(), true);
        AdminState.savingSettings = true;
        compare(page.matrixEditable(), false);
        AdminState.savingSettings = false;

        AdminState.available = false;
        compare(page.matrixEditable(), false);

        // Profils live absents : aucune option éditable.
        LiveData.connected = false;
        compare(page.matrixEditable(), false);
    }

    function test_matrixButtonsFollowDraft() {
        var page = matrixPage();
        var apply = findChild(page, "matrixApplyButton");
        var reset = findChild(page, "matrixResetButton");
        verify(apply !== null);
        verify(reset !== null);
        compare(apply.enabled, false);
        compare(reset.enabled, false);

        page.setMatrixCell("balanced", "ac", "custom-studio");
        compare(apply.enabled, true);
        compare(reset.enabled, true);

        page.setMatrixCell("balanced", "ac", "custom-gone");
        compare(apply.enabled, false);

        page.resetMatrix();
        compare(page.matrixDirty(), false);
        compare(apply.enabled, false);
    }

    function test_matrixDraftKeptOnErrorAndSyncedOnSuccess() {
        var page = matrixPage();
        page.setMatrixCell("balanced", "ac", "custom-studio");
        page.submitMatrix();
        compare(page.matrixSubmitting, true);

        AdminState.saveSettingsError = "Conflict";
        compare(page.matrixSubmitting, false);
        compare(page.matrixCell("balanced", "ac"), "custom-studio");
        compare(page.matrixDirty(), true);
        compare(page.settingsConflict, true);

        // Succès : la matrice relue par le daemon fait foi.
        page.submitMatrix();
        SettingsData.matrixBalancedAc = "custom-studio";
        AdminState.saveSettingsSucceeded = true;
        compare(page.matrixSubmitting, false);
        compare(page.matrixCell("balanced", "ac"), "custom-studio");
        compare(page.matrixDirty(), false);
        compare(page.settingsConflict, false);
    }

    function test_fahrenheitIsSavedImmediately() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.fahrenheit, false);

        page.setFahrenheit(true);
        compare(page.fahrenheit, true);
        compare(saveSpy.count, 1);
        compare(JSON.parse(saveSpy.signalArguments[0][0]).fahrenheit, true);

        // Valeur déjà sélectionnée : aucune requête supplémentaire.
        page.setFahrenheit(true);
        compare(saveSpy.count, 1);
    }

    function test_liveValuesSyncToggles() {
        SettingsData.fahrenheit = true;
        SettingsData.cpuEnabled = false;
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.fahrenheit, true);
        compare(page.cpuEnabled, false);

        // Changement par un autre client : reflété hors enregistrement.
        SettingsData.fanEnabled = false;
        compare(page.fanEnabled, false);

        // Pendant un enregistrement, la valeur locale n'est pas écrasée.
        AdminState.savingSettings = true;
        SettingsData.lightingEnabled = false;
        compare(page.lightingEnabled, true);
    }

    function test_errorRevertsToLiveValues() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        page.fanEnabled = false;
        compare(page.fanEnabled, false);

        // Refus admin : retour à la valeur réellement lue.
        AdminState.saveSettingsError = "Busy";
        compare(page.fanEnabled, SettingsData.fanEnabled);
        compare(page.settingsConflict, false);
    }

    function test_conflictShowsBannerAndReload() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        AdminState.saveSettingsError = "Conflict";
        compare(page.settingsConflict, true);

        // Une nouvelle valeur live est prise en compte par le rechargement.
        SettingsData.fahrenheit = true;
        page.reloadSettings();
        compare(page.settingsConflict, false);
        compare(page.fahrenheit, true);
    }

    function test_successSyncsAfterReload() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        page.fahrenheit = true;

        // Le worker publie les nouvelles valeurs puis le succès.
        AdminState.savingSettings = true;
        SettingsData.fahrenheit = true;
        SettingsData.cpuEnabled = false;
        compare(page.cpuEnabled, true);

        AdminState.savingSettings = false;
        AdminState.saveSettingsSucceeded = true;
        compare(page.cpuEnabled, false);
        compare(page.settingsConflict, false);
    }

    function test_settingsErrorTextIsTranslated() {
        Strings.lang = "fr";
        AdminState.saveSettingsError = "NotAuthorized";
        compare(AdminState.saveSettingsErrorKey, "admin.error.notAuthorized");
        compare(AdminState.saveSettingsErrorText,
                "Autorisation refusée ou annulée ; aucune modification n'a été enregistrée.");
        AdminState.saveSettingsError = "";
        compare(AdminState.saveSettingsErrorText, "");
        Strings.lang = "fr";
    }

    function test_invalidSettingsDisableEditing() {
        AdminState.available = true;
        SettingsData.connected = true;
        SettingsData.error = "Settings: champ invalide (schemaVersion)";
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        // Réglages invalides : lecture seule, aucune requête.
        compare(page.settingsEditable(), false);
        page.cpuEnabled = false;
        page.submitSettings();
        compare(saveSpy.count, 0);
        page.setFahrenheit(true);
        compare(saveSpy.count, 0);

        // Retour à un payload valide : édition de nouveau possible.
        SettingsData.error = "";
        compare(page.settingsEditable(), true);
    }

    function test_settingsDataLiveFlag() {
        SettingsData.connected = false;
        compare(SettingsData.live, false);
        SettingsData.connected = true;
        compare(SettingsData.live, true);
        SettingsData.error = "GetSettingsJSON: timeout";
        compare(SettingsData.live, false);
        SettingsData.error = "";
        compare(SettingsData.live, true);
    }

    function test_sensorsCardIsRemoved() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        // La collecte est automatique et visible sur le Dashboard : la carte
        // passive n'apportait aucun contrôle ni état actionnable.
        compare(findChild(page, "settingsSensorsCard"), null);
        compare(findChild(page, "settingsSensorsBadge"), null);
    }

    function test_technicalNotesAreNotPermanent() {
        AdminState.available = true;
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);

        // Notes d'implémentation supprimées (ou déplacées en infobulle).
        var removed = [
            "settings.autostartNote",
            "settings.graphicsNote",
            "settings.capabilitiesNote",
            "settings.switchingNote",
            "settings.powerBlockerNote"
        ];
        for (var i = 0; i < removed.length; ++i) {
            compare(containsAppLabel(page, Strings.t(removed[i])), false,
                    "note permanente encore affichée : " + removed[i]);
        }

        // L'explication utile du contrôle matériel reste visible, courte.
        compare(containsAppLabel(page, Strings.t("settings.hardwareNote")),
                true);
    }

    /// Vrai si un `AppLabel` porte exactement ce texte. Les infobulles
    /// (`AppToolTip`) utilisent un `Text` natif et ne sont donc pas comptées.
    function containsAppLabel(item, text) {
        if (item === null || item === undefined)
            return false;
        if (typeof item.variant !== "undefined" && item.text === text)
            return true;
        var children = item.children;
        for (var i = 0; i < children.length; ++i) {
            if (containsAppLabel(children[i], text))
                return true;
        }
        return false;
    }
}
