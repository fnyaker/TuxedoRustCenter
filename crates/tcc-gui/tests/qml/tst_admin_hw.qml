// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de la carte Capacités branchée sur Admin.Hw : ODM/TDP/cTGP
// transitoires, consentement PPD, visibilité et mapping d'erreurs.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "AdminHw"
    when: windowShown

    Component { id: settingsComponent; SettingsPage { } }

    SignalSpy { id: odmSpy; target: AppState; signalName: "odmApplyRequested" }
    SignalSpy { id: tdpSpy; target: AppState; signalName: "tdpApplyRequested" }
    SignalSpy { id: ctgpSpy; target: AppState; signalName: "ctgpApplyRequested" }
    SignalSpy { id: settingsSpy; target: AppState; signalName: "settingsSaveRequested" }

    function hardware(owner) {
        return JSON.stringify({
            "sensors": {},
            "charging": {},
            "odm": {
                "available": true,
                "odmWritable": true,
                "tdpWritable": true,
                "platformProfile": {
                    "available": true,
                    "writable": true,
                    "source": "tuxedo_platform_profile",
                    "current": "balanced",
                    "choices": ["power_save", "quiet", "balanced", "performance"],
                    "owner": owner === undefined ? "tcc" : owner
                },
                "tdpAvailable": true,
                "tdp": [
                    { "descriptor": "pl1", "current": 35, "min": 10, "max": 80 },
                    { "descriptor": "pl2", "current": 45, "min": 10, "max": 100 }
                ]
            },
            "nvidia": { "available": true, "ctgpOffset": 5 },
            "prime": { "available": true, "supported": true },
            "ycbcr": { "available": true, "ports": [] }
        });
    }

    function enable(owner) {
        HardwareData.connected = true;
        HardwareData.hardwareError = "";
        HardwareData.hardwareJson = hardware(owner);
        AdminHwData.available = true;
        AdminHwData.pending = false;
        AdminHwData.saveError = "";
        AdminHwData.saveErrorDetail = "";
        AdminHwData.succeeded = false;
        AdminState.available = true;
        AdminState.savingSettings = false;
        SettingsData.connected = true;
        SettingsData.error = "";
        SettingsData.ppdOverride = false;
    }

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        AdminHwData.available = false;
        AdminHwData.pending = false;
        AdminHwData.saveError = "";
        AdminHwData.saveErrorDetail = "";
        AdminHwData.succeeded = false;
        AdminState.available = false;
        AdminState.savingSettings = false;
        SettingsData.connected = false;
        SettingsData.error = "";
        SettingsData.ppdOverride = false;
        odmSpy.clear();
        tdpSpy.clear();
        ctgpSpy.clear();
        settingsSpy.clear();
    }

    function init() { reset(); }

    function exists(page, name) {
        return findChild(page, name) !== null;
    }
    function cleanup() { reset(); }

    function test_blocksHiddenWithoutService() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = hardware("tcc");
        AdminHwData.available = false;
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.odmEditing, false);
        compare(page.tdpEditing, false);
        compare(page.ctgpEditing, false);
        // La carte et la notice restent visibles : le matériel existe mais le
        // service manque (UIW-03).
        compare(page.capabilitiesVisible, true);
        compare(page.hwServiceMissing, true);
        compare(page.hwPresent, true);
        compare(testCase.exists(page, "capabilitiesCard"), true);
        compare(testCase.exists(page, "hwServiceMissingNotice"), true);
    }

    function test_blocksHiddenWithoutHardware() {
        enable("tcc");
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {},
            "charging": {},
            "odm": {
                "available": false,
                "platformProfile": {
                    "available": false,
                    "source": "",
                    "current": "",
                    "choices": [],
                    "owner": ""
                },
                "tdpAvailable": false,
                "tdp": []
            },
            "nvidia": { "available": false, "ctgpOffset": 0 },
            "prime": { "available": false, "supported": false },
            "ycbcr": { "available": false, "ports": [] }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.odmEditing, false);
        compare(page.tdpEditing, false);
        compare(page.ctgpEditing, false);
        compare(page.capabilitiesVisible, false);
        compare(page.hwServiceMissing, false);
    }

    function test_ycbcrKeepsCardAndExplainsMissingService() {
        HardwareData.connected = true;
        HardwareData.hardwareError = "";
        HardwareData.hardwareJson = hardware("tcc");
        AdminHwData.available = false;
        AdminHwData.pending = false;
        var page = createTemporaryObject(settingsComponent, testCase);
        // YCbCr reste lisible, mais les écritures ODM/TDP/cTGP sont masquées.
        compare(page.capabilitiesVisible, true);
        compare(page.hwServiceMissing, true);
        compare(page.odmEditing, false);
        compare(page.tdpEditing, false);
        compare(page.ctgpEditing, false);
        compare(AdminHwData.availabilityKey, "adminHw.unavailable.missing");
    }

    function test_odmApplyAndPpdConsent() {
        enable("tcc");
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.odmEditing, true);
        compare(page.tdpEditing, true);
        compare(page.ctgpEditing, true);
        compare(page.capabilitiesVisible, true);
        compare(page.hwServiceMissing, false);

        // ODM non possédé par PPD : Apply actif après changement.
        compare(page.odmDraftIndex, 2);
        page.odmDraftIndex = 3;
        page.submitOdm(false);
        compare(odmSpy.count, 1);
        compare(odmSpy.signalArguments[0][0], "performance");
        compare(odmSpy.signalArguments[0][1], false);

        // PPD propriétaire : Forcer n'est actif qu'après consentement.
        HardwareData.hardwareJson = hardware("ppd");
        compare(page.odmPpdOwned, true);
        page.odmDraftIndex = 3;
        compare(page.ppdOverrideDraft, false);
        page.submitOdm(true);
        // Le service refuserait ; la GUI n'envoie pas sans consentement.
        compare(odmSpy.count, 1);
        page.submitPpdOverride(true);
        compare(settingsSpy.count, 1);
        var dto = JSON.parse(settingsSpy.signalArguments[0][0]);
        compare(dto.ppdOverride, true);
        page.ppdOverrideDraft = true;
        page.submitOdm(true);
        compare(odmSpy.count, 2);
        compare(odmSpy.signalArguments[1][1], true);
    }

    function test_tdpRejectsFourValues() {
        enable("tcc");
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true,
                "platformProfile": { "available": true, "current": "balanced",
                                     "choices": ["balanced"], "owner": "tcc" },
                "tdpAvailable": true,
                "tdp": [
                    { "descriptor": "pl1", "current": 35, "min": 10, "max": 80 },
                    { "descriptor": "pl2", "current": 45, "min": 10, "max": 100 },
                    { "descriptor": "pl4", "current": 65, "min": 10, "max": 120 },
                    { "descriptor": "pl9", "current": 75, "min": 10, "max": 130 }
                ]
            },
            "nvidia": { "available": false, "ctgpOffset": 0 }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.tdpCountValid(), false);
        compare(page.tdpValid(), false);
        page.submitTdp();
        compare(tdpSpy.count, 0);
        compare(page.tdpError, "settings.tdpDescriptorError");
    }

    function test_tdpRejectsDuplicateOrMisorderedDescriptors() {
        enable("tcc");
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": { "available": true, "platformProfile": { "available": true },
                     "tdpAvailable": true,
                     "tdp": [
                         { "descriptor": "pl1", "current": 35, "min": 10, "max": 80 },
                         { "descriptor": "pl2", "current": 45, "min": 10, "max": 100 }
                     ] },
            "nvidia": { "available": false, "ctgpOffset": 0 }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.tdpDescriptorsValid(), true);
        // Doublon : deux PL2.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": { "available": true, "platformProfile": { "available": true },
                     "tdpAvailable": true,
                     "tdp": [
                         { "descriptor": "pl2", "current": 35, "min": 10, "max": 80 },
                         { "descriptor": "pl2", "current": 45, "min": 10, "max": 100 }
                     ] },
            "nvidia": { "available": false, "ctgpOffset": 0 }
        });
        compare(page.tdpDescriptorsValid(), false);
        page.submitTdp();
        compare(tdpSpy.count, 0);
    }

    function test_odmRejectsInvalidName() {
        enable("tcc");
        var longName = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        compare(longName.length, 65);
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true,
                "odmWritable": true,
                "platformProfile": { "available": true, "writable": true,
                                     "current": "balanced",
                                     "choices": ["balanced", longName], "owner": "tcc" },
                "tdpAvailable": false, "tdp": []
            },
            "nvidia": { "available": false, "ctgpOffset": 0 }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.odmNameValid("balanced"), true);
        compare(page.odmNameValid(longName), false);
        compare(page.odmNameValid("power\\save"), false);
        page.odmDraftIndex = 1;
        page.submitOdm(false);
        compare(odmSpy.count, 0);
        compare(page.odmError, "adminHw.error.invalidArgs");
        verify(Strings.t(page.odmError).length > 0);
    }

    function test_ctgpRejectsNegativeAndHonoursDynamicBound() {
        enable("tcc");
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": { "available": false, "platformProfile": {}, "tdpAvailable": false },
            "nvidia": { "available": true, "ctgpOffset": 120, "maxCtgpOffset": 150 }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(HardwareData.ctgpMax, 150);
        page.ctgpDraft = "-5";
        compare(page.ctgpValid(), false);
        page.submitCtgp();
        compare(ctgpSpy.count, 0);

        page.ctgpDraft = "150";
        compare(page.ctgpValid(), true);
        page.submitCtgp();
        compare(ctgpSpy.count, 1);
        compare(ctgpSpy.signalArguments[0][0], 150);

        page.ctgpDraft = "151";
        compare(page.ctgpValid(), false);
        page.submitCtgp();
        compare(ctgpSpy.count, 1);
    }

    function test_odmUsesTuxedoIoFallback() {
        enable("tcc");
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {}, "charging": {},
            "odm": {
                "available": true,
                "odmWritable": true,
                "tdpWritable": false,
                "platformProfile": { "available": false, "owner": "" },
                "io": {
                    "available": true,
                    "interface": "uniwill",
                    "moduleVersion": "0.2.6",
                    "odmAvailable": true,
                    "choices": ["power_save", "enthusiast", "overboost"],
                    "defaultProfile": "enthusiast",
                    "tdpAvailable": false
                },
                "tdpAvailable": false,
                "tdp": []
            },
            "nvidia": { "available": false }
        });
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.odmEditing, true);
        compare(page.capabilitiesVisible, true);
        compare(page.hwServiceMissing, false);
        compare(HardwareData.odmSourceKind, "tuxedo-io");
        compare(page.odmChoices().length, 3);
        compare(page.odmDraftIndex, 1);
        compare(page.odmPpdOwned, false);
        // TDP officiel via le profil ODM : pas de curseurs PL sur ce laptop.
        compare(page.tdpEditing, false);

        page.odmDraftIndex = 2;
        page.submitOdm(false);
        compare(odmSpy.count, 1);
        compare(odmSpy.signalArguments[0][0], "overboost");
        compare(odmSpy.signalArguments[0][1], false);
    }

    function test_tdpAndCtgpApply() {
        enable("tcc");
        var page = createTemporaryObject(settingsComponent, testCase);
        compare(page.tdpDraft.length, 2);
        compare(page.tdpDraft[0], "35");

        page.setTdpDraft(0, "40");
        compare(page.tdpValid(), true);
        page.submitTdp();
        compare(tdpSpy.count, 1);
        compare(tdpSpy.signalArguments[0][0], "[40,45]");

        // Monotonie PL1 ≤ PL2 : 70 > 45 est refusé localement.
        page.setTdpDraft(0, "70");
        compare(page.tdpMonotonicInvalid, true);
        compare(page.tdpValid(), false);
        page.submitTdp();
        compare(tdpSpy.count, 1);
        compare(page.tdpError, "settings.tdpMonotonicError");

        // Hors bornes : aucun envoi.
        page.setTdpDraft(1, "101");
        compare(page.tdpValid(), false);
        page.submitTdp();
        compare(tdpSpy.count, 1);

        page.ctgpDraft = "12";
        compare(page.ctgpValid(), true);
        page.submitCtgp();
        compare(ctgpSpy.count, 1);
        compare(ctgpSpy.signalArguments[0][0], 12);
    }

    function test_errorsAreMappedAndFormKept() {
        enable("tcc");
        var page = createTemporaryObject(settingsComponent, testCase);
        page.setTdpDraft(0, "70");

        AdminHwData.saveError = "PolicyDenied";
        compare(AdminHwData.saveErrorKey, "adminHw.error.policyDenied");
        verify(AdminHwData.saveErrorText.length > 0);
        compare(page.tdpDraft[0], "70");

        AdminHwData.saveError = "PartialApply";
        compare(AdminHwData.saveErrorKey, "adminHw.error.partialApply");

        AdminHwData.saveError = "Busy";
        compare(AdminHwData.saveErrorKey, "admin.error.busy");
        AdminHwData.saveError = "NotAuthorized";
        compare(AdminHwData.saveErrorKey, "admin.error.notAuthorized");

        AdminHwData.available = false;
        AdminHwData.error = "Contract";
        compare(AdminHwData.availabilityKey, "adminHw.unavailable.contract");
        AdminHwData.error = "Transport";
        compare(AdminHwData.availabilityKey, "adminHw.unavailable.transport");
        AdminHwData.error = "Unavailable";
        compare(AdminHwData.availabilityKey, "adminHw.unavailable.missing");
    }
}
