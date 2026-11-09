// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de l'adaptateur `LiveData` (pont daemon → affichage).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml
//
// Les payloads ci-dessous reproduisent la sérialisation de `tcc-ipc` /
// `tcc-core` (camelCase, sentinelles `-1`, fréquences en kHz). Ils ne
// dépendent pas de D-Bus : `LiveData` est alimenté directement.

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "LiveData"
    when: windowShown

    // Payload valide : un preset (`__office__`) et un profil personnalisé.
    readonly property string liveJson: JSON.stringify([
        {
            "id": "__office__",
            "name": "Office",
            "description": "Bureau",
            "display": {
                "brightness": 60,
                "useBrightness": true,
                "refreshRate": 120,
                "useRefRate": true,
                "xResolution": -1,
                "yResolution": -1,
                "useResolution": false
            },
            "cpu": {
                "onlineCores": 6,
                "useMaxPerfGov": false,
                "scalingMinFrequency": 800000,
                "scalingMaxFrequency": 3500000,
                "governor": "schedutil",
                "energyPerformancePreference": "balance_power",
                "noTurbo": true
            },
            "webcam": { "status": true, "useStatus": false },
            "fan": {
                "useControl": true,
                "fanProfile": "Quiet",
                "minimumFanspeed": 10,
                "maximumFanspeed": 55,
                "offsetFanspeed": 0,
                "customFanCurve": {
                    "tableCPU": [{ "temp": 40, "speed": 20 }, { "temp": 70, "speed": 60 }],
                    "tableGPU": []
                }
            },
            "odmProfile": { "name": "quiet" },
            "odmPowerLimits": { "tdpValues": [22, 30] },
            "nvidiaPowerCTRLProfile": { "cTGPOffset": 5 }
        },
        {
            "id": "custom-abc",
            "name": "Studio",
            "description": "",
            "display": {
                "brightness": 100,
                "useBrightness": false,
                "refreshRate": -1,
                "useRefRate": false
            },
            "cpu": {},
            "webcam": { "status": true, "useStatus": true },
            "fan": {
                "useControl": false,
                "fanProfile": "Balanced",
                "customFanCurve": { "tableCPU": [], "tableGPU": [] }
            },
            "odmProfile": {},
            "odmPowerLimits": { "tdpValues": [] },
            "nvidiaPowerCTRLProfile": { "cTGPOffset": 0 }
        }
    ])

    function reset() {
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        LiveData.activeProfileId = "";
        LiveData.activeProfileName = "";
        LiveData.powerSource = "";
        LiveData.ppdProfile = "";
        LiveData.manualOverride = false;
        LiveData.lastError = "";
        SettingsData.connected = false;
        SettingsData.error = "";
        SettingsData.matrixPowerSaverAc = "";
        SettingsData.matrixPowerSaverBat = "";
        SettingsData.matrixBalancedAc = "";
        SettingsData.matrixBalancedBat = "";
        SettingsData.matrixPerformanceAc = "";
        SettingsData.matrixPerformanceBat = "";
    }

    function cleanup() {
        reset();
    }

    function test_demoFallbackWhenDisconnected() {
        reset();
        compare(LiveData.usingDemo, true);
        compare(LiveData.isLive, false);
        compare(LiveData.profiles.length, DemoData.profiles.length);
        verify(LiveData.profileById("gaming") !== null);
        compare(LiveData.parseError, "");
        // La démo prime aussi pour l'état.
        compare(LiveData.ppd, DemoData.ppd);
        compare(LiveData.power, DemoData.powerSource);
        compare(LiveData.resolvedActiveId, DemoData.activeProfileId);
    }

    function test_disconnectedJsonIsNotParsed() {
        reset();
        LiveData.profilesJson = liveJson;
        compare(LiveData.usingDemo, true);
        compare(LiveData.isLive, false);
        compare(LiveData.profiles.length, DemoData.profiles.length);
    }

    function test_livePayloadIsParsedAndNormalized() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;

        compare(LiveData.usingDemo, false);
        compare(LiveData.isLive, true);
        compare(LiveData.parseError, "");
        compare(LiveData.profiles.length, 2);

        var office = LiveData.profileById("__office__");
        verify(office !== null);
        compare(office.name, "Office");
        compare(office.preset, true);
        compare(office.icon, "profile_default");
        compare(office.live, true);
        // kHz → GHz, arrondi 2 décimales.
        compare(office.freqMin, 0.8);
        compare(office.freqMax, 3.5);
        compare(office.freqMaxReduced, false);
        compare(office.cores, 6);
        compare(office.coresKnown, true);
        compare(office.fanMode, "quiet");
        compare(office.useBrightness, true);
        compare(office.brightness, 60);
        compare(office.useRefRate, true);
        compare(office.refreshRate, 120);
        compare(office.webcam, true);
        compare(office.useWebcam, false);
        compare(office.odm, "quiet");
        compare(office.tdp1, 22);
        compare(office.tdp2, 30);
        compare(office.ctgp, 5);
        compare(office.fanCurve.length, 2);
        compare(office.fanCurve[1].speed, 60);
        // Assignations AC/BAT non exposées : jamais devinées.
        compare(office.assignedAc, false);
        compare(office.assignedBat, false);

        var studio = LiveData.profileById("custom-abc");
        verify(studio !== null);
        compare(studio.preset, false);
        compare(studio.icon, "profile_custom");
        compare(studio.coresKnown, false);
        compare(studio.freqMax, 0);
        compare(studio.fanMode, "auto");
        compare(studio.refreshRate, 0);
        compare(studio.useRefRate, false);
        compare(studio.odm, "");
        compare(studio.tdp1, 0);
    }

    function test_reducedMaxFrequency() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = JSON.stringify([
            {
                "id": "custom-reduced",
                "name": "Reduced",
                "display": {},
                "cpu": { "scalingMinFrequency": 400000, "scalingMaxFrequency": -1 },
                "fan": {}
            }
        ]);
        var profile = LiveData.profileById("custom-reduced");
        verify(profile !== null);
        compare(profile.freqMaxReduced, true);
        compare(profile.freqMax, 0);
        compare(profile.freqMin, 0.4);
    }

    function test_activeProfileAndRefreshTarget() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;
        LiveData.activeProfileId = "__office__";

        compare(LiveData.resolvedActiveId, "__office__");
        compare(LiveData.activeProfile.id, "__office__");
        compare(LiveData.activeName, "Office");
        compare(LiveData.activeDescription, "Bureau");
        compare(LiveData.useRefRate, true);
        compare(LiveData.refreshRate, 120);

        LiveData.activeProfileId = "custom-abc";
        compare(LiveData.activeProfile.id, "custom-abc");
        compare(LiveData.useRefRate, false);
        compare(LiveData.refreshRate, 0);

        // Identifiant inconnu : profil nul, jamais un fallback silencieux.
        LiveData.activeProfileId = "does-not-exist";
        verify(LiveData.activeProfile === null);
        compare(LiveData.activeName, "");
    }

    function test_stateMatrixAssignmentsAreReal() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = testCase.liveJson;
        LiveData.ppdProfile = "balanced";
        SettingsData.connected = true;
        SettingsData.error = "";
        SettingsData.matrixBalancedAc = "__office__";
        SettingsData.matrixPowerSaverBat = "custom-abc";

        compare(LiveData.assignedAc.id, "__office__");
        compare(LiveData.assignedBat, null);
        compare(LiveData.profileById("__office__").assignedAc, true);
        compare(LiveData.profileById("__office__").assignedBat, false);
        compare(LiveData.profileById("custom-abc").assignedBat, true);
        compare(LiveData.profileById("custom-abc").assignedAc, false);

        SettingsData.matrixBalancedBat = "custom-abc";
        compare(LiveData.assignedBat.id, "custom-abc");
        compare(LiveData.profileById("custom-abc").assignedBat, true);

        SettingsData.matrixBalancedAc = "";
        compare(LiveData.assignedAc, null);
        compare(LiveData.profileById("__office__").assignedAc, false);
    }

    function test_activeNameFallsBackToSnapshot() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;
        LiveData.activeProfileId = "not-in-list";
        LiveData.activeProfileName = "Snapshot name";
        compare(LiveData.activeName, "Snapshot name");
    }

    function test_assignedProfilesWithoutMatrixAreNull() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;
        LiveData.activeProfileId = "__office__";

        LiveData.powerSource = "ac";
        compare(LiveData.assignedAc, null);
        LiveData.powerSource = "battery";
        compare(LiveData.assignedBat, null);
        // Aucune assignation inventée depuis le profil actif.
        compare(LiveData.profileById("__office__").assignedAc, false);
        compare(LiveData.profileById("__office__").assignedBat, false);
    }

    function test_invalidJsonFallsBackToDemo() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = "{ pas du json";
        compare(LiveData.usingDemo, true);
        compare(LiveData.isLive, false);
        verify(LiveData.parseError !== "");
        compare(LiveData.profiles.length, DemoData.profiles.length);
        verify(LiveData.profileById("gaming") !== null);
    }

    function test_nonArrayPayloadFallsBackToDemo() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = "{\"profiles\": []}";
        compare(LiveData.usingDemo, true);
        verify(LiveData.parseError !== "");
    }

    function test_emptyArrayFallsBackToDemo() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = "[]";
        compare(LiveData.usingDemo, true);
        verify(LiveData.parseError !== "");
    }

    function test_missingFieldRejectsWholePayload() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = JSON.stringify([
            { "id": "a", "name": "A", "display": {}, "cpu": {}, "fan": {} },
            { "id": "b", "display": {}, "cpu": {}, "fan": {} }
        ]);
        compare(LiveData.usingDemo, true);
        verify(LiveData.parseError.indexOf("profil 1") >= 0);
        // Payload rejeté en bloc : aucune liste partielle.
        verify(LiveData.profileById("a") === null);
    }

    function test_missingSectionsRejectsPayload() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = JSON.stringify([{ "id": "a", "name": "A" }]);
        compare(LiveData.usingDemo, true);
        verify(LiveData.parseError !== "");
    }

    function test_unchangedPayloadIsNotReparsed() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;
        var first = LiveData.profiles;

        LiveData.profilesJson = liveJson;
        verify(LiveData.profiles === first);
        compare(LiveData.usingDemo, false);
    }

    function test_reconnectRestoresLiveData() {
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = liveJson;
        compare(LiveData.isLive, true);

        LiveData.connected = false;
        compare(LiveData.usingDemo, true);

        LiveData.connected = true;
        compare(LiveData.isLive, true);
        compare(LiveData.profiles.length, 2);
    }

    function test_actionRequestsAreSignalled() {
        refreshSpy.clear();
        activateSpy.clear();
        clearSpy.clear();

        AppState.requestRefresh();
        compare(refreshSpy.count, 1);

        AppState.requestProfileActivation("__office__");
        compare(activateSpy.count, 1);
        compare(activateSpy.signalArguments[0][0], "__office__");

        AppState.requestTempProfileClear();
        compare(clearSpy.count, 1);
    }

    SignalSpy { id: refreshSpy; target: AppState; signalName: "refreshRequested" }
    SignalSpy { id: activateSpy; target: AppState; signalName: "profileActivationRequested" }
    SignalSpy { id: clearSpy; target: AppState; signalName: "tempProfileClearRequested" }
}
