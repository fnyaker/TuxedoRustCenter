// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests des règles d'édition (`ProfileRules`) et du mapping d'erreurs
// d'administration (`AdminState`).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "ProfileRules"
    when: windowShown

    function form(overrides) {
        var base = {
            "name": "Studio",
            "description": "Profil de test",
            "brightness": 70,
            "useBrightness": true,
            "refreshRate": 120,
            "useRefreshRate": true,
            "cores": 6,
            "freqMin": 0.8,
            "freqMax": 3.5,
            "freqMaxReduced": false,
            "maxPerfGov": false,
            "epp": "balance_power",
            "noTurbo": true,
            "webcamOn": false,
            "useWebcam": true,
            "fanUseControl": true,
            "fanProfile": "quiet",
            "fanMin": 10,
            "fanMax": 55,
            "fanOffset": 0
        };
        if (overrides) {
            for (var key in overrides)
                base[key] = overrides[key];
        }
        return base;
    }

    function test_validFormHasNoErrors() {
        var result = ProfileRules.validate(form(), []);
        compare(result.ok, true);
        for (var key in result.errors)
            compare(result.errors[key], "", "champ " + key);
    }

    function test_nameRules() {
        compare(ProfileRules.validate(form({ "name": "" }), []).errors.name,
                "editor.error.nameEmpty");
        compare(ProfileRules.validate(form({ "name": "   " }), []).errors.name,
                "editor.error.nameEmpty");
        compare(ProfileRules.validate(form({ "name": "x".repeat(129) }), []).errors.name,
                "editor.error.nameTooLong");
        // Unicité insensible à la casse et aux espaces.
        compare(ProfileRules.validate(form({ "name": "  GAMING " }),
                                      ["gaming"]).errors.name,
                "editor.error.nameDuplicate");
        // Le nom du profil édité n'est pas dans la liste des autres noms.
        compare(ProfileRules.validate(form({ "name": "Gaming" }), []).ok, true);
    }

    function test_descriptionAndBrightnessBounds() {
        compare(ProfileRules.validate(form({ "description": "x".repeat(4097) }), [])
                .errors.description, "editor.error.descriptionTooLong");
        compare(ProfileRules.validate(form({ "brightness": 0 }), []).ok, true);
        compare(ProfileRules.validate(form({ "brightness": 100 }), []).ok, true);
        compare(ProfileRules.validate(form({ "brightness": 101 }), []).errors.brightness,
                "editor.error.brightnessRange");
        compare(ProfileRules.validate(form({ "brightness": -1 }), []).errors.brightness,
                "editor.error.brightnessRange");
    }

    function test_refreshRateRules() {
        compare(ProfileRules.validate(form({ "useRefreshRate": false, "refreshRate": 0 }), []).ok,
                true);
        compare(ProfileRules.validate(form({ "refreshRate": 0 }), []).errors.refreshRate,
                "editor.error.refreshRange");
        compare(ProfileRules.validate(form({ "refreshRate": 1000 }), []).ok, true);
        compare(ProfileRules.validate(form({ "refreshRate": 1001 }), []).errors.refreshRate,
                "editor.error.refreshRange");
    }

    function test_cpuRules() {
        compare(ProfileRules.validate(form({ "cores": 0 }), []).errors.cores,
                "editor.error.coresRange");
        compare(ProfileRules.validate(form({ "cores": 1025 }), []).errors.cores,
                "editor.error.coresRange");
        compare(ProfileRules.validate(form({ "freqMin": 0.05 }), []).errors.freqMin,
                "editor.error.freqRange");
        compare(ProfileRules.validate(form({ "freqMax": 11 }), []).errors.freqMax,
                "editor.error.freqRange");
        compare(ProfileRules.validate(form({ "freqMin": 4.0, "freqMax": 2.0 }), []).errors.freqMin,
                "editor.error.freqOrder");
        // Fréquence max « réduite » : pas de valeur à comparer.
        compare(ProfileRules.validate(form({ "freqMaxReduced": true, "freqMax": 0 }), []).ok,
                true);
    }

    function test_fanRules() {
        compare(ProfileRules.validate(form({ "fanMin": 80, "fanMax": 20 }), []).errors.fanMin,
                "editor.error.fanOrder");
        compare(ProfileRules.validate(form({ "fanMax": 101 }), []).errors.fanMax,
                "editor.error.fanRange");
        compare(ProfileRules.validate(form({ "fanOffset": -101 }), []).errors.fanOffset,
                "editor.error.fanOffsetRange");
        compare(ProfileRules.validate(form({ "fanMin": 0, "fanMax": 100, "fanOffset": -100 }), [])
                .ok, true);
    }

    function test_fanCurveRules() {
        var curve = [
            { "temp": 30, "speed": 20 },
            { "temp": 60, "speed": 50 },
            { "temp": 90, "speed": 90 }
        ];
        var valid = ProfileRules.validate(form({
            "fanProfile": "custom", "fanCurve": curve
        }), []);
        compare(valid.ok, true);
        compare(valid.errors.fanCurve, "");

        // Une courbe n'est validée qu'en mode custom.
        var quiet = ProfileRules.validate(form({ "fanProfile": "quiet", "fanCurve": [] }), []);
        compare(quiet.ok, true);
        compare(quiet.errors.fanCurve, "");

        compare(ProfileRules.validate(form({
            "fanProfile": "custom", "fanCurve": [{ "temp": 30, "speed": 20 }]
        }), []).errors.fanCurve, "fanCurve.error.min");
        compare(ProfileRules.validate(form({
            "fanProfile": "custom",
            "fanCurve": [{ "temp": 60, "speed": 20 }, { "temp": 30, "speed": 50 }]
        }), []).errors.fanCurve, "fanCurve.error.order");
        compare(ProfileRules.validate(form({
            "fanProfile": "custom",
            "fanCurve": [{ "temp": 101, "speed": 20 }, { "temp": 130, "speed": 50 }]
        }), []).errors.fanCurve, "fanCurve.error.range");
        compare(ProfileRules.validate(form({
            "fanProfile": "custom",
            "fanCurve": [{ "temp": 30, "speed": 20 },
                         { "temp": 30, "speed": 50 }]
        }), []).errors.fanCurve, "fanCurve.error.order");
    }

    function test_toEditBuildsRustDto() {
        var dto = ProfileRules.toEdit(form({ "name": "  Studio 2  " }));
        compare(dto.name, "Studio 2");
        compare(dto.onlineCores, 6);
        compare(dto.scalingMinKhz, 800000);
        compare(dto.scalingMaxKhz, 3500000);
        compare(dto.refreshRate, 120);
        compare(dto.useRefreshRate, true);
        compare(dto.useResolution, false);
        compare(dto.xResolution, null);
        compare(dto.yResolution, null);
        compare(dto.fanProfile, "quiet");
        compare(dto.fanUseControl, true);
        compare(dto.webcamStatus, false);
        compare(dto.useWebcamStatus, true);

        var resolved = ProfileRules.toEdit(form({
            "useResolution": true, "resolutionWidth": 2880,
            "resolutionHeight": 1800, "resolutionExists": true
        }));
        compare(resolved.useResolution, true);
        compare(resolved.xResolution, 2880);
        compare(resolved.yResolution, 1800);

        // « réduit » devient `-1` (marqueur `tcc-core`), pas 0.
        var reduced = ProfileRules.toEdit(form({ "freqMaxReduced": true }));
        compare(reduced.scalingMaxKhz, -1);

        // Champs `Option` inconnus : `null`, jamais une valeur inventée.
        var unknown = ProfileRules.toEdit(form({
            "coresKnown": false, "freqMinKnown": false, "freqMaxKnown": false
        }));
        compare(unknown.onlineCores, null);
        compare(unknown.scalingMinKhz, null);
        compare(unknown.scalingMaxKhz, null);

        // Taux désactivé et nul : `null`, jamais 0.
        var noRate = ProfileRules.toEdit(form({ "useRefreshRate": false, "refreshRate": 0 }));
        compare(noRate.refreshRate, null);
        compare(noRate.useRefreshRate, false);

        // Courbe : `null` hors mode custom, table CPU seule en custom.
        var quietCurve = ProfileRules.toEdit(form());
        compare(quietCurve.customFanCurve, null);
        var custom = ProfileRules.toEdit(form({
            "fanProfile": "custom",
            "fanCurve": [{ "temp": 30, "speed": 20 }, { "temp": 80, "speed": 80 }]
        }));
        compare(custom.customFanCurve.tableCPU.length, 2);
        compare(custom.customFanCurve.tableCPU[0].temp, 30);
        compare(custom.customFanCurve.tableGpu, undefined);
    }

    function test_unknownOptionalFieldsSkipValidation() {
        // Un profil sans `onlineCores`/fréquences ne doit pas être invalidé
        // par les valeurs de repli de l'interface.
        var result = ProfileRules.validate(form({
            "coresKnown": false, "cores": 1,
            "freqMinKnown": false, "freqMin": 0.8,
            "freqMaxKnown": false, "freqMax": 0.4
        }), []);
        compare(result.ok, true);
        compare(result.errors.cores, "");
        compare(result.errors.freqMin, "");
        compare(result.errors.freqMax, "");
    }

    function test_knownOptionalFieldsAreValidated() {
        var result = ProfileRules.validate(form({ "coresKnown": true, "cores": 0 }), []);
        compare(result.errors.cores, "editor.error.coresRange");
        var freqs = ProfileRules.validate(form({
            "freqMinKnown": true, "freqMin": 4.0,
            "freqMaxKnown": true, "freqMax": 2.0
        }), []);
        compare(freqs.errors.freqMin, "editor.error.freqOrder");
    }

    function test_adminConflictCodeIsMapped() {
        compare(AdminState.keyFor("Conflict"), "admin.error.conflict");
        compare(AdminState.keyFor("PolicyKitUnavailable"),
                "admin.error.policyKitUnavailable");
    }

    function test_firstErrorHelper() {
        var errors = { "a": "", "b": "editor.error.nameEmpty", "c": "editor.error.fanRange" };
        compare(ProfileRules.firstError(errors, ["a", "b", "c"]), "editor.error.nameEmpty");
        compare(ProfileRules.firstError(errors, ["a"]), "");
    }

    function test_adminErrorCodesMapToKeys() {
        var expected = {
            "NotAuthorized": "admin.error.notAuthorized",
            "Busy": "admin.error.busy",
            "PayloadTooLarge": "admin.error.payloadTooLarge",
            "InvalidPayload": "admin.error.invalidPayload",
            "InvalidArgs": "admin.error.invalidPayload",
            "Validation": "admin.error.validation",
            "InUse": "admin.error.inUse",
            "ReloadFailed": "admin.error.reloadFailed",
            "WriteFailed": "admin.error.writeFailed",
            "RateLimited": "admin.error.rateLimited",
            "NotSupported": "admin.error.notSupported",
            "AdminUnavailable": "admin.error.unavailable",
            "Timeout": "admin.error.timeout",
            "Transport": "admin.error.transport",
            "NoData": "admin.error.noData",
            "NotFound": "admin.error.notFound",
            "PresetReadOnly": "admin.error.presetReadOnly",
            "SomethingNew": "admin.error.unknown",
            "": ""
        };
        for (var code in expected)
            compare(AdminState.keyFor(code), expected[code], "code " + code);
    }

    function test_adminAvailabilityStates() {
        AdminState.available = false;
        AdminState.error = "";
        compare(AdminState.availabilityKey, "admin.unavailable.missing");

        AdminState.error = "Contract";
        compare(AdminState.availabilityKey, "admin.unavailable.contract");

        AdminState.error = "Transport";
        compare(AdminState.availabilityKey, "admin.unavailable.transport");

        AdminState.available = true;
        compare(AdminState.availabilityKey, "");
        compare(AdminState.availabilityText, "");

        AdminState.available = false;
        AdminState.error = "";
    }

    function test_adminReadyAndSavingFlags() {
        AdminState.available = false;
        AdminState.saving = false;
        compare(AdminState.ready, false);

        AdminState.available = true;
        compare(AdminState.ready, true);
        AdminState.saving = true;
        compare(AdminState.ready, false);

        AdminState.saving = false;
        AdminState.available = false;
    }

    function test_adminErrorMessageIsTranslated() {
        Strings.lang = "fr";
        AdminState.saveError = "NotAuthorized";
        compare(AdminState.saveErrorKey, "admin.error.notAuthorized");
        compare(AdminState.saveErrorText, "Autorisation refusée ou annulée ; aucune modification n'a été enregistrée.");
        AdminState.saveError = "";
        compare(AdminState.saveErrorText, "");

        Strings.lang = "en";
        AdminState.saveError = "Busy";
        compare(AdminState.saveErrorText, "Another operation is in progress; retry shortly.");
        Strings.lang = "fr";
        AdminState.saveError = "";
    }

    function cleanup() {
        AdminState.available = false;
        AdminState.error = "";
        AdminState.saving = false;
        AdminState.saveError = "";
        AdminState.saveErrorDetail = "";
        AdminState.saveSucceeded = false;
        Strings.lang = "fr";
    }
}
