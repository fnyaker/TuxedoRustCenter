// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Vérifie que Info, Support et le Dashboard n'affichent aucune valeur produit
// issue de `DemoData` en live, et que les capacités réelles sont utilisées.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "InfoSupport"
    when: windowShown

    Component { id: infoComponent; InfoPage { } }
    Component { id: supportComponent; SupportPage { } }
    Component { id: dashboardComponent; DashboardPage { } }
    Component { id: settingsComponent; SettingsPage { } }

    readonly property string profilesJson: JSON.stringify([
        {
            "id": "custom-a", "name": "Studio", "description": "",
            "display": { "refreshRate": 120, "useRefRate": true },
            "cpu": {}, "webcam": {}, "fan": {}
        }
    ])

    readonly property string hardwareJson: JSON.stringify({
        "device": {
            "available": true,
            "vendor": "TUXEDO Computers GmbH",
            "productName": "TUXEDO Book X",
            "sku": "IBM15A10",
            "isTuxedo": true,
            "model": "IBM15A10",
            "kernelRelease": "6.12.0-tuxedo",
            "architecture": "x86_64",
            "tuxedoDriverVersion": "0.3.9"
        },
        "cpu": { "totalCores": 16, "onlineCores": 12,
                 "availableGovernors": ["powersave", "performance"],
                 "availableEpp": ["power", "performance"] },
        "display": { "available": true, "hasInternalDisplay": true,
                     "internalConnector": "eDP-1", "connected": true },
        "capabilities": { "hasBattery": true, "hasAc": true,
                          "hasIgpu": true, "hasDedicatedGpu": false },
        "sensors": { "enabled": true, "available": true,
                     "cpu": { "temperatureC": 50.0 } },
        "charging": { "available": true },
        "prime": { "available": true, "mode": "on-demand" }
    })

    readonly property string agentJson: JSON.stringify({
        "available": true,
        "session": { "type": "wayland", "desktop": "Hyprland" },
        "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
        "internalOutput": {
            "name": "eDP-1", "connected": true, "enabled": true,
            "hasInternalDisplay": true,
            "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60 },
            "modes": [
                { "width": 2560, "height": 1600, "refreshHz": 60 },
                { "width": 2560, "height": 1600, "refreshHz": 165 }
            ]
        },
        "supported": true, "dryRun": false, "error": null
    })

    function reset() {
        AgentData.available = false;
        AgentData.version = "";
        AgentData.error = "";
        AgentData.displayInfoJson = "{}";
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        HardwareData.fanJson = "{}";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        LiveData.activeProfileId = "";
        LiveData.daemonVersion = "";
        LiveData.contractVersion = 0;
    }

    function live() {
        HardwareData.connected = true;
        HardwareData.hardwareJson = testCase.hardwareJson;
        HardwareData.hardwareError = "";
        HardwareData.fanJson = JSON.stringify({
            "available": true,
            "backend": "tuxi",
            "writable": true,
            "controlEnabled": true,
            "mode": "auto",
            "fans": [
                { "index": 0, "label": "cpu", "target": "cpu",
                  "temp": 55, "speed": 42, "applied": 45 },
                { "index": 1, "label": "gpu1", "target": "gpu",
                  "temp": 61, "speed": 63, "applied": 65 }
            ]
        });
        LiveData.connected = true;
        LiveData.profilesJson = testCase.profilesJson;
        LiveData.daemonVersion = "0.4.2";
        LiveData.contractVersion = 1;
        AgentData.available = true;
        AgentData.version = "0.4.2";
        AgentData.displayInfoJson = testCase.agentJson;
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function test_infoUsesRealValuesOnly() {
        live();
        var page = createTemporaryObject(infoComponent, testCase);
        verify(page !== null);
        compare(page.appVersion, Qt.application.version !== ""
                                  ? Qt.application.version
                                  : Strings.t("common.unavailable"));
        compare(page.daemonVersion, "0.4.2");
        compare(page.contractVersion, "v1");
        compare(page.textOr(HardwareData.device.productName), "TUXEDO Book X");
        compare(page.textOr(HardwareData.device.kernelRelease), "6.12.0-tuxedo");
        compare(page.textOr(HardwareData.device.architecture), "x86_64");
        compare(page.textOr(HardwareData.device.tuxedoDriverVersion), "0.3.9");
        compare(page.changelog.length, 2);
        // Le changelog est explicitement marqué statique.
        compare(testCase.exists(page, "infoStaticBadge"), true);
    }

    function test_infoDoesNotFallBackToDemoData() {
        // Daemon absent : aucune version/model de démonstration.
        var page = createTemporaryObject(infoComponent, testCase);
        compare(page.daemonVersion, Strings.t("common.unavailable"));
        compare(page.contractVersion, Strings.t("common.unavailable"));
        compare(page.textOr(HardwareData.device.productName),
                Strings.t("common.unavailable"));
        compare(HardwareData.device.productName, "");
        // La page ne doit pas référencer DemoData : le modèle démo est vide.
        verify(page.changelog[0].notes.length > 0);
    }

    function test_dashboardUsesRealCapabilities() {
        live();
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        compare(page.deviceEyebrow, "TUXEDO Book X");
        compare(page.primeVisible, true);
        compare(page.graphModeLabel(), Strings.t("settings.graphics.ondemand"));
        compare(page.coresHint(), "12 / 16 " + Strings.t("dash.cores").toLowerCase());
        compare(page.agentAvailable, AgentData.available);
        compare(testCase.exists(page, "dashboardDgpuCard"), true);
        compare(testCase.exists(page, "dashboardIgpuCard"), true);
        // La collecte est automatique : aucun bouton ni note de bail.
        compare(testCase.exists(page, "dashboardSensorsNote"), false);
        compare(testCase.containsAppButton(
                    testCase.findChild(page, "dashboardCpuCard")), false);
    }

    function test_dashboardShowsDedicatedIgpuCardAndCharts() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {
                "enabled": true, "available": true, "cpu": {},
                "igpu": { "vendor": "intel", "role": "integrated",
                          "name": "Intel Iris Xe",
                          "temperatureC": 48, "frequencyMhz": 1200,
                          "powerW": 3.5 },
                "dgpu": null
            },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        compare(page.igpuVisible, true);
        compare(page.dgpuVisible, false);
        compare(page.igpuTempAvailable, true);
        compare(page.igpuFreqAvailable, true);
        compare(page.igpuPowerAvailable, true);
        compare(testCase.exists(page, "dashboardIgpuCard"), true);

        var igpuCard = testCase.findChild(page, "dashboardIgpuCard");
        var tempGauge = testCase.findChild(page, "dashboardIgpuTempGauge");
        var freqGauge = testCase.findChild(page, "dashboardIgpuFreqGauge");
        var powerGauge = testCase.findChild(page, "dashboardIgpuPowerGauge");
        verify(tempGauge !== null);
        verify(freqGauge !== null);
        verify(powerGauge !== null);
        compare(tempGauge.available, true);
        compare(freqGauge.available, true);
        compare(powerGauge.available, true);

        // Les trois graphes iGPU restent dans leur carte (inchangés).
        verify(testCase.isDescendant(
                   testCase.findChild(page, "dashboardIgpuTempChart"), igpuCard));
        verify(testCase.isDescendant(
                   testCase.findChild(page, "dashboardIgpuFreqChart"), igpuCard));
        verify(testCase.isDescendant(
                   testCase.findChild(page, "dashboardIgpuPowerChart"), igpuCard));

        // Plus aucune carte « Historique » séparée.
        compare(testCase.hasCardTitle(page, Strings.t("dash.history")), false);

        // Échantillonnage local : une valeur par métrique disponible.
        compare(page.igpuTempSamples.length, 0);
        page.pushSamples();
        compare(page.igpuTempSamples.length, 1);
        compare(page.igpuFreqSamples.length, 1);
        compare(page.igpuPowerSamples.length, 1);
        compare(page.igpuTempSamples[0], 48);
        compare(page.igpuFreqSamples[0], 1.2);
        compare(page.igpuPowerSamples[0], 3.5);

        // Métrique absente : aucune valeur 0 n'est poussée.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {
                "enabled": true, "available": true, "cpu": {},
                "igpu": { "vendor": "intel", "role": "integrated",
                          "name": "Intel Iris Xe",
                          "temperatureC": null, "frequencyMhz": 1200,
                          "powerW": null },
                "dgpu": null
            },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        var second = createTemporaryObject(dashboardComponent, testCase);
        compare(second.igpuTempAvailable, false);
        compare(second.igpuFreqAvailable, true);
        compare(second.igpuPowerAvailable, false);
        compare(testCase.findChild(second, "dashboardIgpuTempGauge").available, false);
        compare(testCase.findChild(second, "dashboardIgpuFreqGauge").available, true);
        compare(testCase.findChild(second, "dashboardIgpuPowerGauge").available, false);
        second.pushSamples();
        compare(second.igpuTempSamples.length, 0);
        compare(second.igpuFreqSamples.length, 1);
        compare(second.igpuPowerSamples.length, 0);

        // iGPU retiré d'un snapshot fiable : l'historique est vidé.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        compare(second.igpuVisible, false);
        second.pushSamples();
        compare(second.igpuFreqSamples.length, 0);
    }

    function test_dashboardHasNoRedundantCards() {
        live();
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);

        // Cartes redondantes supprimées : l'état PPD/alimentation est dans la
        // carte du profil actif, les diagnostics agent dans Réglages.
        compare(testCase.hasCardTitle(page, Strings.t("dash.systemState")), false);
        compare(testCase.hasCardTitle(page, Strings.t("dash.sensors")), false);
        compare(testCase.hasCardTitle(page, Strings.t("dash.history")), false);

        // Les anciens objectNames ne doivent pas réapparaître.
        compare(testCase.exists(page, "dashboardSensorsIgpu"), false);
        compare(testCase.exists(page, "dashboardSensorsDgpu"), false);
        compare(testCase.exists(page, "dashboardGpuFanRow"), false);
        compare(testCase.exists(page, "dashboardAgentBackend"), false);
        compare(testCase.exists(page, "dashboardAgentOutput"), false);
        compare(testCase.exists(page, "dashboardAgentCurrent"), false);
    }

    function test_dashboardCpuChartLivesInCpuCard() {
        live();
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        var cpuCard = testCase.findChild(page, "dashboardCpuCard");
        var chart = testCase.findChild(page, "dashboardCpuChart");
        verify(cpuCard !== null);
        verify(chart !== null);
        verify(testCase.isDescendant(chart, cpuCard));
        compare(page.cpuChartVisible, true);

        // Légende et valeur courante dans la même carte.
        var value = testCase.findChild(page, "dashboardCpuChartValue");
        verify(value !== null);
        verify(testCase.isDescendant(value, cpuCard));
        compare(value.text, Strings.formatTemp(page.cpu.temp));

        // Une seule série CPU, pas de doublon.
        compare(testCase.countCharts(cpuCard), 1);
    }

    function test_dashboardDgpuChartLivesInDgpuCard() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": {
                "enabled": true, "available": true,
                "cpu": { "temperatureC": 52 },
                "igpu": { "vendor": "intel", "role": "integrated",
                          "name": "Intel Iris Xe",
                          "temperatureC": 48, "frequencyMhz": 1200,
                          "powerW": 3.5 },
                "dgpu": { "vendor": "nvidia", "role": "discrete",
                          "name": "NVIDIA RTX 4060",
                          "temperatureC": 61, "frequencyMhz": 1800,
                          "powerW": 42 }
            },
            "capabilities": { "hasIgpu": true, "hasDedicatedGpu": true },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        compare(page.dgpuVisible, true);
        compare(page.dgpuTempAvailable, true);

        var dgpuCard = testCase.findChild(page, "dashboardDgpuCard");
        var block = testCase.findChild(page, "dashboardDgpuChartBlock");
        var chart = testCase.findChild(page, "dashboardDgpuChart");
        verify(dgpuCard !== null);
        verify(block !== null);
        verify(chart !== null);
        verify(testCase.isDescendant(block, dgpuCard));
        verify(testCase.isDescendant(chart, dgpuCard));
        compare(page.dgpuChartVisible, true);

        var value = testCase.findChild(page, "dashboardDgpuChartValue");
        verify(value !== null);
        compare(value.text, Strings.formatTemp(61));

        // dGPU absent : le bloc de graphe reste masqué, sans graphe orphelin.
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        compare(page.dgpuVisible, false);
        compare(page.dgpuChartVisible, false);
    }

    function test_dashboardUsesRealGpuFan() {
        live();
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        verify(page.gpuFan !== null);
        compare(page.gpuFan.target, "gpu");
        compare(page.gpuFan.speed, 63);
        compare(page.gpuFanAvailable, true);
        compare(testCase.exists(page, "dashboardGpuFanGauge"), true);

        // Sans ventilateur GPU exposé : aucune jauge inventée.
        HardwareData.fanJson = JSON.stringify({
            "available": true, "backend": "tuxi", "writable": true,
            "controlEnabled": true, "mode": "auto",
            "fans": [{ "index": 0, "label": "cpu", "target": "cpu",
                       "temp": 55, "speed": 42, "applied": 45 }]
        });
        compare(page.gpuFan, null);
        compare(page.gpuFanAvailable, false);
    }

    function test_dashboardHidesUnavailableGraphMode() {
        live();
        HardwareData.hardwareJson = JSON.stringify({
            "sensors": { "enabled": true, "available": true, "cpu": {} },
            "charging": { "available": true },
            "device": { "available": true, "productName": "TUXEDO Book X" },
            "prime": { "available": false }
        });
        var page = createTemporaryObject(dashboardComponent, testCase);
        compare(page.primeVisible, false);
        compare(page.deviceEyebrow, "TUXEDO Book X");
        compare(page.igpuVisible, false);
        compare(page.dgpuVisible, false);
    }

    function test_dashboardDemoDoesNotInventDeviceOrCores() {
        var page = createTemporaryObject(dashboardComponent, testCase);
        compare(page.deviceEyebrow, "");
        compare(HardwareData.deviceLabel, "");
        compare(HardwareData.cpuCoresText, "");
        compare(HardwareData.prime.available, false);
        compare(page.primeVisible, false);
    }

    function test_dashboardShowsRealAgentState() {
        live();
        LiveData.activeProfileId = "custom-a";
        var page = createTemporaryObject(dashboardComponent, testCase);
        verify(page !== null);
        compare(page.agentAvailable, true);
        compare(page.agentUsable, true);
        compare(AgentData.currentModeLabel,
                "2560 × 1600 @ " + Strings.formatHz(60));
        compare(page.liveRate, 120);
        // Les diagnostics détaillés (backend/output/mode courant) vivent dans
        // Réglages : le Dashboard ne les duplique plus.
        compare(testCase.exists(page, "dashboardAgentBackend"), false);
        compare(testCase.exists(page, "dashboardAgentOutput"), false);
        compare(testCase.exists(page, "dashboardAgentCurrent"), false);
    }

    function test_dashboardExplainsUnsupportedOrMissingAgent() {
        live();
        AgentData.displayInfoJson = JSON.stringify({
            "available": true, "supported": false,
            "internalOutput": { "name": "eDP-1", "hasInternalDisplay": false,
                                "modes": [] }
        });
        var page = createTemporaryObject(dashboardComponent, testCase);
        compare(page.agentUsable, false);
        compare(testCase.exists(page, "dashboardAgentNotice"), true);
        compare(AgentData.errorText, "");
        compare(AgentData.displayErrorText, "");

        AgentData.available = false;
        AgentData.error = "Unavailable";
        compare(AgentData.errorText, Strings.t("display.agentUnavailable"));
        compare(page.agentAvailable, false);
    }

    function test_settingsShowsAgentBackendAndModes() {
        live();
        var page = createTemporaryObject(settingsComponent, testCase);
        verify(page !== null);
        compare(page.agentBackendLabel, "hyprland");
        compare(page.agentOutputLabel, "eDP-1");
        compare(testCase.exists(page, "settingsAgentBackendRow"), true);
        compare(testCase.exists(page, "settingsAgentOutputRow"), true);
        compare(testCase.exists(page, "settingsAgentCurrentRow"), true);
        compare(testCase.exists(page, "settingsAgentModesRow"), true);
        compare(testCase.exists(page, "settingsAgentNotice"), true);

        AgentData.available = false;
        AgentData.error = "Contract";
        compare(AgentData.errorText, Strings.t("display.agentContract"));
    }

    function test_supportKeepsOnlyRealBackends() {
        var page = createTemporaryObject(supportComponent, testCase);
        verify(page !== null);
        compare(page.supportUrl, "https://www.tuxedocomputers.com/support");
        compare(testCase.exists(page, "supportContactCard"), true);
        compare(testCase.exists(page, "supportOnlineButton"), true);
        // Aucun service sans backend, aucune note « à venir » : la page ne
        // contient que les canaux de contact réels.
        compare(testCase.exists(page, "supportUnavailableNotice"), false);
        compare(testCase.exists(page, "supportDiagnosticsCard"), false);
        compare(testCase.exists(page, "supportWebfaiCard"), false);
        compare(testCase.exists(page, "supportAnydeskCard"), false);
    }

    function exists(page, name) {
        return findChild(page, name) !== null;
    }

    /// Vrai si `item` est un descendant (parenté visuelle) de `ancestor`.
    function isDescendant(item, ancestor) {
        var node = item ? item.parent : null;
        while (node) {
            if (node === ancestor)
                return true;
            node = node.parent;
        }
        return false;
    }

    /// Compte les `Chart` d'un sous-arbre (objet avec `values` + `strokeColor`).
    function countCharts(item) {
        if (item === null || item === undefined)
            return 0;
        var count = typeof item.values !== "undefined"
                && typeof item.strokeColor !== "undefined" ? 1 : 0;
        var children = item.children;
        for (var i = 0; i < children.length; ++i)
            count += countCharts(children[i]);
        return count;
    }

    /// Vrai si une `AppCard` (title + subtitle + padding) porte ce titre.
    function hasCardTitle(item, title) {
        if (item === null || item === undefined)
            return false;
        if (typeof item.padding !== "undefined"
                && typeof item.subtitle !== "undefined"
                && item.title === title)
            return true;
        var children = item.children;
        for (var i = 0; i < children.length; ++i) {
            if (hasCardTitle(children[i], title))
                return true;
        }
        return false;
    }

    /// Détecte un `AppButton` dans un sous-arbre (variant + iconName + text).
    function containsAppButton(item) {
        if (item === null || item === undefined)
            return false;
        var children = item.children;
        for (var i = 0; i < children.length; ++i) {
            var child = children[i];
            if (typeof child.variant !== "undefined"
                    && typeof child.iconName !== "undefined"
                    && typeof child.text !== "undefined")
                return true;
            if (containsAppButton(child))
                return true;
        }
        return false;
    }
}
