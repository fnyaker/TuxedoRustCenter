// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests d'intégration de l'éditeur de profil : disponibilité admin, validation
// locale, envoi du DTO, presets en lecture seule et conflit de profil.
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "ProfileEditor"
    when: windowShown

    Component {
        id: editorComponent
        ProfileEditorPage { }
    }

    SignalSpy {
        id: saveSpy
        target: AppState
        signalName: "profileSaveRequested"
    }

    SignalSpy {
        id: createSpy
        target: AppState
        signalName: "profileCreateRequested"
    }

    function fixture(extraDescription) {
        return JSON.stringify([
            {
                "id": "__office__",
                "name": "Office",
                "description": "Preset TUXEDO",
                "display": { "brightness": 60, "useBrightness": true,
                             "refreshRate": 60, "useRefRate": true },
                "cpu": { "onlineCores": 4, "scalingMinFrequency": 800000,
                         "scalingMaxFrequency": 2000000,
                         "energyPerformancePreference": "balance_power" },
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Silent",
                         "minimumFanspeed": 0, "maximumFanspeed": 50,
                         "offsetFanspeed": 0 }
            },
            {
                "id": "custom-a",
                "name": "Studio",
                "description": extraDescription ? extraDescription : "Profil de test",
                "display": { "brightness": 70, "useBrightness": true,
                             "refreshRate": 120, "useRefRate": true },
                "cpu": { "onlineCores": 6, "scalingMinFrequency": 800000,
                         "scalingMaxFrequency": 3500000,
                         "energyPerformancePreference": "balance_power",
                         "noTurbo": true },
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Quiet",
                         "minimumFanspeed": 10, "maximumFanspeed": 55,
                         "offsetFanspeed": 0 },
                "odmProfile": { "name": "balanced" },
                "odmPowerLimits": { "tdpValues": [40, 50] },
                "nvidiaPowerCTRLProfile": { "cTGPOffset": 5 }
            },
            {
                "id": "custom-bare",
                "name": "Minimal",
                "description": "Profil sans champs CPU optionnels",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": {},
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Balanced",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            },
            {
                "id": "custom-b",
                "name": "Gaming",
                "description": "Autre profil",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": { "onlineCores": 8 },
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": false, "fanProfile": "Balanced",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            }
        ]);
    }

    /// Deux profils aux EPP distincts, pour vérifier qu'aucun index/valeur
    /// n'est partagé entre eux.
    function eppFixture(first, second) {
        return JSON.stringify([
            {
                "id": "epp-a",
                "name": "Batterie",
                "description": "Profil A",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": { "onlineCores": 4,
                         "energyPerformancePreference": first },
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Balanced",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            },
            {
                "id": "epp-b",
                "name": "Performance",
                "description": "Profil B",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": { "onlineCores": 8,
                         "energyPerformancePreference": second },
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Performance",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            }
        ]);
    }

    function setEppHardware() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": {},
            "cpu": {
                "availableGovernors": ["powersave", "performance"],
                "availableEpp": ["power", "balance_power", "performance"]
            }
        });
    }

    function reset() {
        AdminState.saveAction = "";
        AdminState.createdProfileId = "";
        createSpy.clear();
        AdminState.available = false;
        AdminState.version = "";
        AdminState.error = "";
        AdminState.saving = false;
        AdminState.saveError = "";
        AdminState.saveErrorDetail = "";
        AdminState.saveSucceeded = false;
        AppState.editingProfileId = "";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        SettingsData.connected = false;
        SettingsData.error = "";
        SettingsData.matrixBalancedAc = "";
        SettingsData.matrixBalancedBat = "";
        SettingsData.matrixPowerSaverAc = "";
        SettingsData.matrixPowerSaverBat = "";
        SettingsData.matrixPerformanceAc = "";
        SettingsData.matrixPerformanceBat = "";
    }

    function init() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = fixture();
        AppState.editingProfileId = "custom-a";
        saveSpy.clear();
    }

    function cleanup() {
        reset();
        saveSpy.clear();
    }

    function test_readOnlyWithoutAdmin() {
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.isNew, false);
        compare(page.isPreset, false);
        compare(page.readOnly, true);
        compare(page.editable, false);
        compare(page.savePossible(), false);
        // Raison affichée : service absent.
        compare(page.saveBlockedReason(), Strings.t("admin.unavailable.missing"));
    }

    function test_editableAndSaveSendsBoundedDto() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.readOnly, false);
        compare(page.editable, true);
        compare(page.valid, true);

        // Sans modification, rien à envoyer.
        compare(page.savePossible(), false);
        page.dirty = true;
        compare(page.savePossible(), true);

        page.save();
        compare(saveSpy.count, 1);
        var args = saveSpy.signalArguments[0];
        compare(args[0], "custom-a");

        var dto = JSON.parse(args[1]);
        compare(dto.name, "Studio");
        compare(dto.brightness, 70);
        compare(dto.useBrightness, true);
        compare(dto.refreshRate, 120);
        compare(dto.useRefreshRate, true);
        compare(dto.onlineCores, 6);
        compare(dto.scalingMinKhz, 800000);
        compare(dto.scalingMaxKhz, 3500000);
        compare(dto.energyPerformancePreference, "balance_power");
        compare(dto.noTurbo, true);
        compare(dto.webcamStatus, true);
        compare(dto.useWebcamStatus, true);
        compare(dto.fanUseControl, true);
        compare(dto.fanProfile, "quiet");
        compare(dto.fanMin, 10);
        compare(dto.fanMax, 55);
        compare(dto.fanOffset, 0);
        // Champs ODM/TDP/cTGP : `null` tant qu'ils ne sont pas modifiés.
        compare(dto.tdp, undefined);
        compare(dto.odmProfile, null);
        compare(dto.tdpValues, null);
        compare(dto.ctgpOffset, null);
    }

    function test_profileOdmTdpCtgpAreEdited() {
        AdminState.available = true;
        AdminState.saving = false;
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true, "odmWritable": true, "tdpWritable": true,
                "platformProfile": { "available": false, "owner": "" },
                "io": {
                    "available": true, "odmAvailable": true,
                    "choices": ["power_save", "enthusiast", "overboost"],
                    "defaultProfile": "enthusiast", "tdpAvailable": true
                },
                "tdpAvailable": true,
                "tdp": [
                    { "descriptor": "pl1", "current": 40, "min": 5, "max": 80 },
                    { "descriptor": "pl2", "current": 50, "min": 5, "max": 100 }
                ]
            },
            "nvidia": { "available": true, "ctgpOffset": 5,
                        "maxCtgpOffset": 150 }
        });
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.odmCardVisible, true);
        compare(page.tdpEditing, true);
        compare(page.ctgpEditing, true);
        compare(page.odmProfileName(), "balanced");
        compare(page.tdpDraft[0], "40");
        compare(page.ctgpDraft, "5");

        page.odmProfileIndex = page.odmEditChoices.indexOf("overboost") + 1;
        page.setTdpDraft(1, "80");
        page.ctgpDraft = "12";
        page.touch();
        compare(page.valid, true);
        compare(page.odmError, "");

        saveSpy.clear();
        page.save();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(dto.odmProfile, "overboost");
        compare(dto.tdpValues[0], 40);
        compare(dto.tdpValues[1], 80);
        compare(dto.ctgpOffset, 12);
    }

    function test_profileTdpAndCtgpHiddenWithoutHardware() {
        AdminState.available = true;
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true, "odmWritable": true,
                "platformProfile": { "available": false, "owner": "" },
                "io": { "available": true, "odmAvailable": true,
                        "choices": ["power_save", "enthusiast", "overboost"],
                        "defaultProfile": "enthusiast", "tdpAvailable": false },
                "tdpAvailable": false, "tdp": []
            },
            "nvidia": { "available": false }
        });
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.odmCardVisible, true);
        compare(page.tdpEditing, false);
        compare(page.ctgpEditing, false);
        page.odmProfileIndex = 1;
        page.touch();
        compare(page.valid, true);
        var dto = ProfileRules.toEdit(page.form);
        compare(dto.tdpValues, null);
        compare(dto.ctgpOffset, null);
        compare(dto.odmProfile, "power_save");
    }

    function test_profileOdmTdpCtgpInvalidAreBlocked() {
        AdminState.available = true;
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true, "odmWritable": true,
                "platformProfile": { "available": false, "owner": "" },
                "io": { "available": true, "odmAvailable": true,
                        "choices": ["power_save", "enthusiast", "overboost"],
                        "defaultProfile": "enthusiast", "tdpAvailable": true },
                "tdpAvailable": true,
                "tdp": [
                    { "descriptor": "pl1", "current": 40, "min": 5, "max": 80 },
                    { "descriptor": "pl2", "current": 50, "min": 5, "max": 100 }
                ]
            },
            "nvidia": { "available": true, "ctgpOffset": 5,
                        "maxCtgpOffset": 150 }
        });
        var page = createTemporaryObject(editorComponent, testCase);
        page.setTdpDraft(0, "70");
        page.setTdpDraft(1, "50");
        compare(page.valid, false);
        compare(page.odmError, "editor.error.tdpValues");

        page.setTdpDraft(0, "40");
        page.ctgpDraft = "151";
        page.touch();
        compare(page.valid, false);
        compare(page.odmError, "editor.error.ctgpOffset");
    }

    function test_floatingActionBar() {
        AdminState.available = true;
        AdminState.saving = false;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        var bar = findChild(page, "editorFloatingBar");
        var save = findChild(page, "floatingSaveButton");
        var cancel = findChild(page, "floatingCancelButton");
        var status = findChild(page, "floatingStatus");
        var spacer = findChild(page, "editorBottomSpacer");
        verify(bar !== null && save !== null && cancel !== null);
        verify(status !== null && spacer !== null);
        compare(page.editable, true);
        compare(page.floatingBarVisible, true);
        compare(save.enabled, false);
        compare(cancel.enabled, false);
        verify(spacer.implicitHeight > 0);

        page.profileName = "Brouillon";
        page.touch();
        compare(save.enabled, true);
        compare(cancel.enabled, true);
        compare(status.text, Strings.t("editor.unsaved"));

        saveSpy.clear();
        save.clicked();
        compare(saveSpy.count, 1);

        cancel.clicked();
        compare(page.dirty, false);
        compare(save.enabled, false);
        compare(page.profileName, "Studio");
    }

    function test_floatingBarHiddenForPreset() {
        AdminState.available = true;
        AppState.editingProfileId = "__office__";
        var page = createTemporaryObject(editorComponent, testCase);
        var bar = findChild(page, "editorFloatingBar");
        verify(bar !== null);
        compare(page.editable, false);
        compare(page.floatingBarVisible, false);
    }

    function test_fanCurveEditorOnlyInCustomMode() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        var dialog = findChild(page, "fanCurveEditor");
        verify(dialog !== null);

        // Mode non custom : message clair, pas d'ouverture.
        page.fanProfileIndex = page.fanProfileKeys.indexOf("quiet");
        page.openFanCurveEditor();
        compare(dialog.opened, false);
        compare(AppState.toasts.get(AppState.toasts.count - 1).message,
                Strings.t("fan.customOnly"));

        // Mode custom : le dialogue s'ouvre et l'application met à jour le
        // brouillon + dirty.
        page.fanProfileIndex = page.fanProfileKeys.indexOf("custom");
        page.openFanCurveEditor();
        compare(dialog.opened, true);
        dialog.apply();
        compare(page.dirty, true);
        compare(page.fanCurve.length > 1, true);
        compare(page.valid, true);
    }

    function test_fanCurveCurveIsSentOnlyInCustomMode() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.fanCurve.length > 1, true);

        // Non custom : `customFanCurve` nul, la courbe existante est préservée.
        page.dirty = true;
        page.save();
        var quiet = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(quiet.customFanCurve, null);

        // Custom : la table CPU est envoyée.
        saveSpy.clear();
        page.fanProfileIndex = page.fanProfileKeys.indexOf("custom");
        page.fanCurve = [
            { "temp": 30, "speed": 20 },
            { "temp": 80, "speed": 80 }
        ];
        page.touch();
        page.save();
        compare(saveSpy.count, 1);
        var custom = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(custom.customFanCurve.tableCPU.length, 2);
        compare(custom.customFanCurve.tableCPU[0].temp, 30);
        compare(custom.customFanCurve.tableGpu, undefined);
    }

    function test_duplicateNameBlocksSave() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.profileName = "  GAMING ";
        page.dirty = true;
        compare(page.valid, false);
        compare(page.errors.name, "editor.error.nameDuplicate");
        compare(page.savePossible(), false);
        page.save();
        compare(saveSpy.count, 0);
    }

    function test_presetIsReadOnly() {
        AdminState.available = true;
        AppState.editingProfileId = "__office__";
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.isPreset, true);
        compare(page.readOnly, true);
        page.dirty = true;
        compare(page.savePossible(), false);
        page.save();
        compare(saveSpy.count, 0);
    }

    function test_savingDisablesSave() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        AdminState.saving = true;
        compare(page.savePossible(), false);
        compare(page.saveBlockedReason(), Strings.t("editor.saving"));
        AdminState.saving = false;
        compare(page.savePossible(), true);
    }

    function test_matrixAssignmentDoesNotConflict() {
        AdminState.available = true;
        SettingsData.connected = true;
        SettingsData.error = "";
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        page.profileName = "Brouillon";
        page.touch();
        compare(page.dirty, true);

        // La matrice change les icônes AC/BAT, pas le contenu du profil.
        SettingsData.matrixBalancedAc = "custom-a";
        compare(page.conflict, false);
        compare(page.profileName, "Brouillon");
    }

    function test_conflictWhenLiveProfilesChange() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        compare(page.conflict, false);

        LiveData.profilesJson = fixture("Description modifiée par un autre client");
        compare(page.conflict, true);
        // Le formulaire n'est pas écrasé, et l'enregistrement est interdit.
        compare(page.profileDescription, "Profil de test");
        compare(page.savePossible(), false);
        compare(page.saveBlockedReason(), Strings.t("editor.conflictBlocked"));

        // Recharger repart des valeurs live et efface le conflit.
        page.reloadFromLive();
        compare(page.conflict, false);
        compare(page.profileDescription, "Description modifiée par un autre client");
        compare(page.dirty, false);
        page.dirty = true;
        compare(page.savePossible(), true);
    }

    function test_saveConflictErrorForcesReload() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        compare(page.savePossible(), true);

        AdminState.saveError = "Conflict";
        compare(page.conflict, true);
        compare(page.savePossible(), false);
        compare(page.saveBlockedReason(), Strings.t("editor.conflictBlocked"));
    }

    function test_unknownOptionalFieldsAreNotSent() {
        AdminState.available = true;
        AppState.editingProfileId = "custom-bare";
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.coresKnown, false);
        compare(page.freqMinKnown, false);
        compare(page.freqMaxKnown, false);
        compare(page.valid, true);

        page.dirty = true;
        page.save();
        compare(saveSpy.count, 1);
        var dto = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(dto.onlineCores, null);
        compare(dto.scalingMinKhz, null);
        compare(dto.scalingMaxKhz, null);
        // Les autres champs restent envoyés normalement.
        compare(dto.name, "Minimal");
        compare(dto.fanUseControl, true);
    }

    function test_saveErrorKeepsFormAndDirty() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        page.profileName = "Studio renommé";

        AdminState.saveError = "NotAuthorized";
        AdminState.saveErrorDetail = "";
        compare(AdminState.hasSaveError, true);
        // Le formulaire reste intact et modifiable après un refus.
        compare(page.profileName, "Studio renommé");
        compare(page.dirty, true);
        compare(page.savePossible(), true);
    }

    function test_successReloadsAndClearsDirty() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        compare(page.dirty, true);

        // Le worker publie les nouvelles données puis `saveSucceeded`.
        AdminState.saveAction = "save";
        LiveData.profilesJson = fixture("Enregistré");
        AdminState.saveSucceeded = true;
        compare(page.dirty, false);
        // `loadProfile` a repris la description relue.
        compare(page.profileDescription, "Enregistré");
    }

    function test_newProfileIsEditableAndCreates() {
        AdminState.available = true;
        AppState.editingProfileId = "new";
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.isNew, true);
        compare(page.readOnly, false);
        compare(page.editable, true);
        // Nom localisé par défaut, brouillon valide.
        compare(page.profileName, Strings.t("editor.newProfileName"));
        compare(page.valid, true);

        page.dirty = true;
        compare(page.savePossible(), true);
        page.save();

        // La création passe par le signal dédié, pas par `saveProfile`.
        compare(createSpy.count, 1);
        compare(saveSpy.count, 0);
        var dto = JSON.parse(createSpy.signalArguments[0][0]);
        compare(dto.name, Strings.t("editor.newProfileName"));
        compare(dto.fanUseControl, true);
        // Valeurs par défaut : cœurs/fréquences inconnus restent `null`.
        compare(dto.onlineCores, null);
        compare(dto.scalingMaxKhz, null);
    }

    function test_createSuccessNavigatesToCreatedProfile() {
        AdminState.available = true;
        AppState.editingProfileId = "new";
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;

        AdminState.saveAction = "create";
        AdminState.createdProfileId = "custom-new";
        AdminState.saveSucceeded = true;

        compare(AppState.editingProfileId, "custom-new");
        compare(page.isNew, false);
    }

    function test_otherActionsDoNotReloadEditor() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        page.profileName = "En cours";

        // Une duplication réussie ne doit pas recharger le brouillon.
        AdminState.saveAction = "duplicate";
        AdminState.saveSucceeded = true;
        compare(page.dirty, true);
        compare(page.profileName, "En cours");
    }

    function test_newProfileBlockedWithoutAdmin() {
        AdminState.available = false;
        AppState.editingProfileId = "new";
        var page = createTemporaryObject(editorComponent, testCase);
        compare(page.isNew, true);
        compare(page.readOnly, true);
        page.dirty = true;
        compare(page.savePossible(), false);
        page.save();
        compare(createSpy.count, 0);
    }

    function test_invalidFieldShowsReason() {
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        page.dirty = true;
        page.fanMin = 80;
        page.fanMax = 20;
        compare(page.valid, false);
        compare(page.fanError, "editor.error.fanOrder");
        compare(page.saveBlockedReason(), Strings.t("editor.invalidHint"));
    }

    function test_eppIsStoredPerProfileWithoutLeak() {
        setEppHardware();
        LiveData.profilesJson = eppFixture("balance_power", "performance");
        AdminState.available = true;
        AppState.editingProfileId = "epp-a";

        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.source.epp, "balance_power");
        compare(page.eppValue, "balance_power");
        compare(page.eppValues[page.eppIndex], "balance_power");

        // Changement d'EPP du profil A uniquement.
        saveSpy.clear();
        compare(page.eppValues.indexOf("power") >= 0, true);
        page.selectEpp(page.eppValues.indexOf("power"));
        compare(page.eppValue, "power");
        compare(page.dirty, true);
        page.save();

        compare(saveSpy.count, 1);
        compare(saveSpy.signalArguments[0][0], "epp-a");
        var dtoA = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(dtoA.energyPerformancePreference, "power");
        compare(dtoA.name, "Batterie");
        // DTO plat : aucun autre profil ne voyage avec celui édité.
        compare(Object.keys(dtoA).indexOf("profiles"), -1);

        // Persistance simulée : A devient « power », B reste « performance ».
        LiveData.profilesJson = eppFixture("power", "performance");
        AppState.editingProfileId = "epp-b";
        compare(page.eppValue, "performance");
        compare(page.eppValues[page.eppIndex], "performance");
        compare(page.source.epp, "performance");
        compare(page.dirty, false);

        // B conserve son EPP : le DTO ne contient que la valeur de B.
        saveSpy.clear();
        page.dirty = true;
        page.save();
        compare(saveSpy.count, 1);
        compare(saveSpy.signalArguments[0][0], "epp-b");
        var dtoB = JSON.parse(saveSpy.signalArguments[0][1]);
        compare(dtoB.energyPerformancePreference, "performance");
        compare(dtoB.name, "Performance");

        // Aucun profil source n'a été muté par l'édition de l'autre.
        compare(LiveData.profileById("epp-a").epp, "power");
        compare(LiveData.profileById("epp-b").epp, "performance");

        // Retour à A : sa valeur persistée est toujours là.
        AppState.editingProfileId = "epp-a";
        compare(page.eppValue, "power");
        compare(page.eppValues[page.eppIndex], "power");
    }

    /// Le vrai symptôme perçu : après un choix EPP, la liste déroulante restait
    /// figée sur l'index précédent quand on ouvrait un autre profil (binding
    /// cassé par ComboBox). Le modèle reste la seule source de vérité.
    function test_eppIndexFollowsLateHardwareList() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "cpu": { "availableEpp": [] }
        });
        LiveData.profilesJson = eppFixture("balance_power", "performance");
        AdminState.available = true;
        AppState.editingProfileId = "epp-a";

        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.eppValue, "balance_power");
        // Liste matérielle vide : une seule entrée, celle du profil.
        compare(page.eppValues.length, 1);
        compare(page.eppValues[page.eppIndex], "balance_power");

        // Le snapshot matériel arrive après le chargement : l'index se
        // recalcule sur la valeur, il ne désigne pas une autre préférence.
        setEppHardware();
        compare(page.eppValues.length, 3);
        compare(page.eppValues[page.eppIndex], "balance_power");
        compare(page.eppIndex, page.eppValues.indexOf("balance_power"));
        compare(page.eppValue, "balance_power");
    }

    function test_eppSelectFollowsProfileSwitchAfterUserChoice() {
        setEppHardware();
        LiveData.profilesJson = eppFixture("balance_power", "performance");
        AdminState.available = true;
        AppState.editingProfileId = "epp-a";

        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        var row = findChild(page, "editorEppRow");
        verify(row !== null);
        var select = findChild(row, "selectRowSelect");
        verify(select !== null);
        compare(select.currentIndex, page.eppIndex);
        compare(select.currentIndex, page.eppValues.indexOf("balance_power"));

        // Choix utilisateur (chemin `SelectRow.changed`).
        select.activated(page.eppValues.indexOf("power"));
        compare(page.eppValue, "power");
        compare(select.currentIndex, page.eppIndex);
        compare(select.currentIndex, page.eppValues.indexOf("power"));

        // Changement de profil : la liaison doit se réappliquer.
        AppState.editingProfileId = "epp-b";
        compare(page.eppValue, "performance");
        compare(select.currentIndex, page.eppIndex);
        compare(select.currentIndex, page.eppValues.indexOf("performance"));

        // Et retour au profil A, sans valeur résiduelle de B.
        AppState.editingProfileId = "epp-a";
        compare(page.eppValue, "balance_power");
        compare(select.currentIndex, page.eppIndex);
    }

    function test_newProfileKeepsDefaultEppInsteadOfUndefined() {
        setEppHardware();
        AdminState.available = true;
        AppState.editingProfileId = "new";

        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);
        compare(page.eppValue, "balance_performance");
        compare(page.eppValues[page.eppIndex], "balance_performance");
        verify(page.eppValue !== "undefined");

        page.dirty = true;
        page.save();
        compare(createSpy.count, 1);
        var dto = JSON.parse(createSpy.signalArguments[0][0]);
        compare(dto.energyPerformancePreference, "balance_performance");
        verify(dto.energyPerformancePreference !== "undefined");
    }

    function test_eppFieldExplainsPpdOwnership() {
        Strings.lang = "fr";
        AdminState.available = true;
        var page = createTemporaryObject(editorComponent, testCase);
        verify(page !== null);

        var row = findChild(page, "editorEppRow");
        verify(row !== null);
        compare(row.tooltip, Strings.t("cpu.eppTooltip"));
        verify(row.tooltip.length > 0);
        verify(row.tooltip.indexOf("power-profiles-daemon") >= 0);
        // La valeur est bien annoncée comme enregistrée par profil.
        verify(row.tooltip.toLowerCase().indexOf("profil") >= 0);

        var tip = findChild(row, "selectRowTooltip");
        if (tip === null)
            tip = findChild(page, "selectRowTooltip");
        verify(tip !== null);
        compare(tip.text, row.tooltip);
        verify(tip.text.length > 0);

        // Le champ expose le même texte aux lecteurs d'écran.
        var select = findChild(row, "selectRowSelect");
        verify(select !== null);
        compare(select.Accessible.name, row.label);
        compare(select.Accessible.description, row.tooltip);

        // L'icône d'aide est focusable : le tooltip apparaît aussi au clavier.
        var info = findChild(row, "selectRowInfo");
        verify(info !== null);
        info.forceActiveFocus();
        tryCompare(tip, "visible", true);
        verify(tip.text.length > 0);
    }
}
