// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Dashboard / moniteur système. Le profil actif, le PPD et l'alimentation
// viennent du daemon (`LiveData`) ; les capteurs, la charge et les capacités
// viennent de `Manager1` (`HardwareData`), avec repli `DemoData`. Aucune
// écriture matérielle : la collecte capteurs est automatique et son bail se
// renouvelle tant que le Dashboard est visible.
//
// Le Dashboard ne duplique pas les états déjà portés par la carte du profil
// actif, la sidebar ou les cartes matérielles : les températures historiques
// sont intégrées à la carte du composant concerné (CPU, iGPU, dGPU).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    readonly property var activeProfile: LiveData.activeProfile
    /// État réel du service de session `User1`.
    readonly property bool agentAvailable: AgentData.available
    readonly property bool agentUsable: AgentData.usable
    readonly property bool primeVisible: HardwareData.prime.available
    readonly property real liveRate: LiveData.useRefRate ? LiveData.refreshRate : 0

    // ── Capteurs (live ou démo) ─────────────────────────────────────────────
    readonly property var cpu: HardwareData.cpu
    readonly property var igpu: HardwareData.igpu
    readonly property var dgpu: HardwareData.dgpu
    readonly property var fan: HardwareData.fans.length > 0 ? HardwareData.fans[0] : null
    /// Ventilateur GPU réellement exposé par `GetFanDataJSON` (target/label).
    /// Aucun fallback inventé : la jauge est masquée si absent.
    readonly property var gpuFan: {
        var list = HardwareData.fans;
        for (var i = 0; i < list.length; ++i) {
            var target = String(list[i].target || "").toLowerCase();
            var label = String(list[i].label || "").toLowerCase();
            if (target.indexOf("gpu") >= 0 || label.indexOf("gpu") >= 0)
                return list[i];
        }
        return null;
    }
    readonly property bool gpuFanAvailable: page.gpuFan !== null
                                            && page.gpuFan.speed >= 0
    readonly property bool sensorsLive: HardwareData.sensorsAvailable
    /// Visibilité des blocs GPU : absents seulement sur snapshot sûr. La
    /// détection DMI/DRM (`capabilities`) évite de masquer une carte pendant
    /// la première seconde où la collecte capteurs n'est pas encore armée.
    readonly property bool dgpuVisible: HardwareData.visibleIfPresent(
        page.dgpu.present || HardwareData.capabilities.hasDedicatedGpu)
    readonly property bool igpuVisible: HardwareData.visibleIfPresent(
        page.igpu.present || HardwareData.capabilities.hasIgpu)
    /// Disponibilité par métrique : ne jamais tracer un 0 inventé.
    readonly property bool igpuTempAvailable: page.igpu.present
            && page.igpu.tempAvailable === true
    readonly property bool igpuFreqAvailable: page.igpu.present
            && page.igpu.freqAvailable === true
    readonly property bool igpuPowerAvailable: page.igpu.present
            && page.igpu.powerAvailable === true
    readonly property bool dgpuTempAvailable: page.dgpu.present
            && page.dgpu.tempAvailable === true
    readonly property bool dgpuFreqAvailable: page.dgpu.present
            && page.dgpu.freqAvailable === true
    readonly property bool dgpuPowerAvailable: page.dgpu.present
            && page.dgpu.powerAvailable === true
    /// Graphe CPU : démo, ou collecte live avec une température réelle.
    readonly property bool cpuChartVisible: !HardwareData.isLive
            || (page.sensorsLive && page.cpu.available)
    /// Graphe dGPU : uniquement si le GPU dédié est présent.
    readonly property bool dgpuChartVisible: page.dgpu.present
            && (!HardwareData.isLive || page.sensorsLive)

    // Historique local (le contrat n'expose pas de série temporelle).
    property var cpuSamples: []
    property var gpuSamples: []
    property var igpuTempSamples: []
    property var igpuFreqSamples: []
    property var igpuPowerSamples: []

    function appendSample(list, value) {
        var out = list.slice(Math.max(0, list.length - 47));
        out.push(Math.round(value * 10) / 10);
        return out;
    }

    function pushSamples() {
        var cpuList = page.cpuSamples.slice(Math.max(0, page.cpuSamples.length - 47));
        var gpuList = page.gpuSamples.slice(Math.max(0, page.gpuSamples.length - 47));
        cpuList.push(Math.round(page.cpu.temp * 10) / 10);
        page.cpuSamples = cpuList;
        if (page.dgpu.present && page.dgpuTempAvailable) {
            gpuList.push(Math.round(page.dgpu.temp * 10) / 10);
            page.gpuSamples = gpuList;
        }

        // iGPU : chaque métrique n'est échantillonnée que si elle existe.
        // L'historique est conservé hors Dashboard et pendant une simple
        // expiration de bail ; il n'est vidé que si un snapshot fiable montre
        // que le GPU a disparu.
        if (!page.igpu.present) {
            if (page.sensorsLive) {
                page.igpuTempSamples = [];
                page.igpuFreqSamples = [];
                page.igpuPowerSamples = [];
            }
            return;
        }
        if (!page.sensorsLive)
            return;
        if (page.igpuTempAvailable)
            page.igpuTempSamples = page.appendSample(page.igpuTempSamples,
                                                      page.igpu.temp);
        if (page.igpuFreqAvailable)
            page.igpuFreqSamples = page.appendSample(page.igpuFreqSamples,
                                                     page.igpu.freqGhz);
        if (page.igpuPowerAvailable)
            page.igpuPowerSamples = page.appendSample(page.igpuPowerSamples,
                                                      page.igpu.powerW);
    }

    Timer {
        // Les capteurs sont publiés toutes les 2 s : échantillonner plus vite
        // ne fait que dupliquer des points et reconstruire les courbes.
        interval: 2000
        repeat: true
        running: page.visible && AppState.windowVisible
                 && HardwareData.isLive && page.sensorsLive
        onTriggered: page.pushSamples()
    }

    function ppdLabel() {
        if (LiveData.ppd === "power-saver")
            return Strings.t("settings.ppdSaver");
        if (LiveData.ppd === "performance")
            return Strings.t("settings.ppdPerformance");
        if (LiveData.ppd === "balanced")
            return Strings.t("settings.ppdBalanced");
        return Strings.t("settings.ppdUnknown");
    }

    function graphModeLabel() {
        var mode = HardwareData.prime.mode;
        if (mode === "iGPU" || mode === "igpu")
            return Strings.t("settings.graphics.igpu");
        if (mode === "dGPU" || mode === "dgpu")
            return Strings.t("settings.graphics.dgpu");
        if (mode === "on-demand" || mode === "ondemand")
            return Strings.t("settings.graphics.ondemand");
        if (mode === "off")
            return Strings.t("settings.graphics.off");
        return Strings.t("settings.ppdUnknown");
    }

    /// « 12 / 16 cœurs » seulement si le daemon expose la topologie.
    function coresHint() {
        if (HardwareData.cpuCoresText === "")
            return "";
        return HardwareData.cpuCoresText + " " + Strings.t("dash.cores").toLowerCase();
    }

    readonly property string deviceEyebrow: {
        if (HardwareData.deviceLabel !== "")
            return HardwareData.deviceLabel;
        return LiveData.isLive ? Strings.t("common.unavailable") : "";
    }

    contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        eyebrow: page.deviceEyebrow
        title: Strings.t("dash.title")
        subtitle: Strings.t("dash.subtitle")
        iconName: "dashboard"

        StatusBadge {
            kind: LiveData.isLive ? "ok" : "warn"
            label: LiveData.isLive ? Strings.t("app.live") : Strings.t("common.demoTag")
            AppToolTip {
                visible: headerHover.hovered
                text: LiveData.isLive ? Strings.t("app.liveHint") : Strings.t("app.demoHint")
            }
            HoverHandler { id: headerHover }
        }

        AppButton {
            text: Strings.t("dash.editProfile")
            variant: "secondary"
            iconName: "settings"
            enabled: LiveData.resolvedActiveId !== ""
            onClicked: AppState.editProfile(LiveData.resolvedActiveId)
        }
    }

    // ── Profil actif ────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true
        highlighted: true

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s16

            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 58
                Layout.preferredHeight: 58
                radius: Theme.radiusMd
                color: Theme.accentSoft

                AppIcon {
                    anchors.centerIn: parent
                    name: page.activeProfile && page.activeProfile.icon
                          ? page.activeProfile.icon : "profile_custom"
                    monochrome: false
                    size: 40
                }
            }

            AbstractButton {
                id: profileLink
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                hoverEnabled: true
                focusPolicy: Qt.TabFocus
                enabled: LiveData.resolvedActiveId !== ""
                Accessible.role: Accessible.Button
                Accessible.name: Strings.tf("dash.openProfile", [LiveData.activeName])
                onClicked: AppState.editProfile(LiveData.resolvedActiveId)

                background: Rectangle {
                    radius: Theme.radiusSm
                    color: profileLink.hovered ? Theme.surfaceHover : "transparent"

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -2
                        radius: parent.radius + 2
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.focusRing
                        visible: profileLink.visualFocus
                    }
                }

                contentItem: ColumnLayout {
                    id: identity
                    spacing: 2

                    AppLabel {
                        variant: "label"
                        secondary: true
                        text: Strings.t("dash.activeProfile")
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "h2"
                        text: LiveData.activeName !== "" ? LiveData.activeName
                                                         : Strings.t("dash.noData")
                        elide: Text.ElideRight
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "small"
                        secondary: true
                        text: LiveData.activeDescription !== "" ? LiveData.activeDescription
                                                                : Strings.t("dash.noData")
                        elide: Text.ElideRight
                        maximumLineCount: 1
                    }
                }

                AppToolTip {
                    visible: profileHover.hovered && profileLink.enabled
                    text: Strings.t("dash.openProfileHint")
                }
                HoverHandler { id: profileHover }
            }

            ColumnLayout {
                spacing: Theme.s6
                Layout.alignment: Qt.AlignVCenter

                RowLayout {
                    Layout.alignment: Qt.AlignRight
                    spacing: Theme.s6

                    StatusBadge {
                        kind: "info"
                        iconName: "power"
                        label: page.ppdLabel()
                    }

                    StatusBadge {
                        visible: HardwareData.capabilities.hasAc
                                 || HardwareData.capabilities.hasBattery
                                 || !HardwareData.liveKnown
                        kind: LiveData.power === "ac" ? "ok" : "neutral"
                        iconName: LiveData.power === "ac" ? "pluggedin" : "batterymode"
                        label: LiveData.power === "ac" ? Strings.t("dash.ac")
                             : LiveData.power === "battery" ? Strings.t("dash.battery")
                             : Strings.t("state.unavailable")
                    }

                    StatusBadge {
                        visible: LiveData.manualOverride
                        kind: "accent"
                        iconName: "power"
                        label: Strings.t("profiles.manualOverride")
                    }

                    AbstractButton {
                        id: primeLink
                        visible: page.primeVisible
                        focusPolicy: Qt.TabFocus
                        Accessible.role: Accessible.Button
                        Accessible.name: Strings.tf("dash.openGraphicsSettings",
                                                    [page.graphModeLabel()])
                        onClicked: AppState.openSettings("prime")

                        background: Rectangle {
                            radius: Theme.radiusPill
                            color: "transparent"
                            border.width: 2
                            border.color: Theme.focusRing
                            visible: primeLink.visualFocus
                        }

                        contentItem: StatusBadge {
                            id: primeBadge
                            kind: "neutral"
                            iconName: "monitor"
                            label: page.primeVisible ? page.graphModeLabel()
                                                     : Strings.t("common.unavailable")
                        }

                        AppToolTip {
                            visible: primeHover.hovered
                            text: Strings.t("dash.openGraphicsSettingsHint")
                        }
                        HoverHandler { id: primeHover }
                    }
                }

                RowLayout {
                    Layout.alignment: Qt.AlignRight
                    spacing: Theme.s6

                    StatusBadge {
                        visible: page.agentAvailable
                        kind: page.agentUsable ? "ok" : "warn"
                        iconName: "speedometer"
                        label: AgentData.currentModeLabel !== ""
                               ? AgentData.currentModeLabel
                               : (page.agentUsable ? Strings.formatHz(page.liveRate)
                                                   : Strings.t("display.unsupported"))
                        AppToolTip {
                            visible: rateHover.hovered
                            text: AgentData.errorText !== "" ? AgentData.errorText
                                                             : Strings.t("display.currentHint")
                        }
                        HoverHandler { id: rateHover }
                    }

                    AppButton {
                        small: true
                        variant: "ghost"
                        iconName: "allprofiles"
                        text: Strings.t("profiles.title")
                        onClicked: AppState.navigate("profiles")
                    }
                }
            }
        }
    }

    // ── Processeur ──────────────────────────────────────────────────────────
    AppCard {
        objectName: "dashboardCpuCard"
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("dash.cpu")
            hint: {
                var parts = [];
                var cores = page.coresHint();
                if (cores !== "")
                    parts.push(cores);
                if (!page.sensorsLive)
                    parts.push(Strings.t("common.demoTag"));
                return parts.join(" · ");
            }
            iconName: "cpu"
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: 720
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.s4
            spacing: Theme.s8

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.cpu.temp
                available: page.cpu.available
                unavailableReason: Strings.t("dash.missingSensor")
                to: 100
                unit: Strings.tempUnit
                label: Strings.t("dash.temp")
                iconName: "temperature"
                warningThreshold: 90
            }

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.cpu.freqGhz
                available: page.cpu.available
                unavailableReason: Strings.t("dash.missingSensor")
                from: 0
                to: 5
                decimals: 2
                unit: "GHz"
                label: Strings.t("dash.freq")
                iconName: "speedometer"
            }

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.fan && page.fan.speed >= 0 ? page.fan.speed : 0
                available: HardwareData.fan.available && page.fan !== null
                           && page.fan.speed >= 0
                unavailableReason: Strings.t("dash.missingSensor")
                unit: "%"
                label: Strings.t("dash.fan")
                iconName: "fan"
            }

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.cpu.powerW
                available: page.cpu.available
                unavailableReason: Strings.t("dash.missingSensor")
                to: 80
                decimals: 1
                unit: "W"
                label: Strings.t("dash.powerDraw")
                iconName: "power"
            }
        }

        // ── Ventilateurs : vitesse réelle, consigne et mode (lecture seule) ──
        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            spacing: Theme.s8

            AppLabel {
                variant: "caption"
                secondary: true
                text: Strings.t("fans.mode")
            }

            StatusBadge {
                kind: "neutral"
                label: Strings.t("fans.mode." + HardwareData.fan.mode)
            }

            AppLabel {
                variant: "caption"
                secondary: true
                text: Strings.t("fans.consigne")
            }

            AppLabel {
                variant: "caption"
                text: page.fan && page.fan.applied >= 0
                      ? Strings.formatPercent(page.fan.applied) : "—"
            }

            AppLabel {
                variant: "caption"
                secondary: true
                text: Strings.t("fans.backend")
            }

            AppLabel {
                variant: "caption"
                text: HardwareData.fan.backend !== "" ? HardwareData.fan.backend : "—"
            }

            Item { Layout.fillWidth: true }

            AppLabel {
                variant: "caption"
                secondary: true
                text: Strings.t("fans.readOnlyHint")
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: HardwareData.fanStale
            variant: "caption"
            color: Theme.warning
            wrapMode: Text.WordWrap
            text: Strings.t("fans.stale") + " · " + HardwareData.fanError
        }

        // ── Température CPU : historique intégré à la carte ─────────────────
        // Déplacé depuis la carte « Historique » : une seule série, un seul
        // emplacement, aucune carte dédiée à un graphe orphelin.
        ColumnLayout {
            objectName: "dashboardCpuChartBlock"
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            spacing: Theme.s6
            visible: page.cpuChartVisible

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s6

                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: 4
                    color: Theme.gauge
                }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: Strings.t("dash.temp")
                }

                Item { Layout.fillWidth: true }

                AppLabel {
                    objectName: "dashboardCpuChartValue"
                    variant: "caption"
                    secondary: true
                    text: page.cpu.available ? Strings.formatTemp(page.cpu.temp) : "—"
                }
            }

            Chart {
                objectName: "dashboardCpuChart"
                Layout.fillWidth: true
                Layout.preferredHeight: 90
                values: HardwareData.isLive
                        ? (page.sensorsLive ? page.cpuSamples : [])
                        : DemoData.cpuHistory
                autoScale: true
                strokeColor: Theme.gauge
                showGrid: true
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: HardwareData.stale && HardwareData.hardwareError !== ""
            variant: "caption"
            color: Theme.warning
            wrapMode: Text.WordWrap
            text: Strings.t("sensors.stale") + " · " + HardwareData.hardwareError
        }
    }

    // ── iGPU ────────────────────────────────────────────────────────────────
    AppCard {
        objectName: "dashboardIgpuCard"
        Layout.fillWidth: true
        visible: page.igpuVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("dash.igpu")
            hint: page.igpu.present && page.igpu.name !== ""
                  ? page.igpu.name : Strings.t("common.unavailable")
            iconName: "gpu"

            StatusBadge {
                kind: page.igpu.present ? "ok" : "neutral"
                label: page.igpu.present
                       ? (page.igpu.vendor !== "" ? page.igpu.vendor.toUpperCase()
                                                  : Strings.t("common.supported"))
                       : Strings.t("common.unavailable")
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: 720
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.s4
            spacing: Theme.s8

            Gauge {
                objectName: "dashboardIgpuTempGauge"
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.igpu.temp
                available: page.igpuTempAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                to: 100
                unit: Strings.tempUnit
                label: Strings.t("dash.temp")
                iconName: "temperature"
                warningThreshold: 90
            }

            Gauge {
                objectName: "dashboardIgpuFreqGauge"
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.igpu.freqGhz
                available: page.igpuFreqAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                from: 0
                to: 2.5
                decimals: 2
                unit: "GHz"
                label: Strings.t("dash.freq")
                iconName: "speedometer"
            }

            Gauge {
                objectName: "dashboardIgpuPowerGauge"
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.igpu.powerW
                available: page.igpuPowerAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                to: 40
                decimals: 1
                unit: "W"
                label: Strings.t("dash.powerDraw")
                iconName: "power"
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            spacing: Theme.s24

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.s6

                RowLayout {
                    spacing: Theme.s6
                    Rectangle { Layout.preferredWidth: 8; Layout.preferredHeight: 8; radius: 4; color: Theme.gauge }
                    AppLabel { variant: "caption"; secondary: true; text: Strings.t("dash.temp") }
                    Item { Layout.fillWidth: true }
                    AppLabel {
                        variant: "caption"
                        secondary: true
                        text: page.igpuTempAvailable ? Strings.formatTemp(page.igpu.temp) : "—"
                    }
                }

                Chart {
                    objectName: "dashboardIgpuTempChart"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 72
                    values: page.igpuTempSamples
                    autoScale: true
                    strokeColor: Theme.gauge
                    showGrid: false
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.s6

                RowLayout {
                    spacing: Theme.s6
                    Rectangle { Layout.preferredWidth: 8; Layout.preferredHeight: 8; radius: 4; color: Theme.info }
                    AppLabel { variant: "caption"; secondary: true; text: Strings.t("dash.freq") }
                    Item { Layout.fillWidth: true }
                    AppLabel {
                        variant: "caption"
                        secondary: true
                        text: page.igpuFreqAvailable
                              ? Strings.formatNumber(page.igpu.freqGhz, 2) + " GHz" : "—"
                    }
                }

                Chart {
                    objectName: "dashboardIgpuFreqChart"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 72
                    values: page.igpuFreqSamples
                    autoScale: true
                    strokeColor: Theme.info
                    showGrid: false
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.s6

                RowLayout {
                    spacing: Theme.s6
                    Rectangle { Layout.preferredWidth: 8; Layout.preferredHeight: 8; radius: 4; color: Theme.accentText }
                    AppLabel { variant: "caption"; secondary: true; text: Strings.t("dash.powerDraw") }
                    Item { Layout.fillWidth: true }
                    AppLabel {
                        variant: "caption"
                        secondary: true
                        text: page.igpuPowerAvailable
                              ? Strings.formatNumber(page.igpu.powerW, 1) + " W" : "—"
                    }
                }

                Chart {
                    objectName: "dashboardIgpuPowerChart"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 72
                    values: page.igpuPowerSamples
                    autoScale: true
                    strokeColor: Theme.accentText
                    showGrid: false
                }
            }
        }
    }

    // ── GPU ─────────────────────────────────────────────────────────────────
    AppCard {
        objectName: "dashboardDgpuCard"
        Layout.fillWidth: true
        // Absence matérielle confirmée uniquement sur un snapshot live fiable.
        visible: page.dgpuVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("dash.dgpu")
            hint: page.dgpu.present && page.dgpu.name !== ""
                  ? page.dgpu.name : Strings.t("common.unavailable")
            iconName: "gpu"

            StatusBadge {
                kind: page.dgpu.present ? "ok" : "neutral"
                label: page.dgpu.present
                       ? (page.dgpu.vendor !== "" ? page.dgpu.vendor.toUpperCase()
                                                  : Strings.t("common.supported"))
                       : Strings.t("common.unavailable")
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: 720
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.s4
            spacing: Theme.s8

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.dgpu.temp
                available: page.dgpuTempAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                to: 100
                unit: Strings.tempUnit
                label: Strings.t("dash.temp")
                iconName: "temperature"
                warningThreshold: 88
            }

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.dgpu.freqGhz
                available: page.dgpuFreqAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                from: 0
                to: 2.5
                decimals: 2
                unit: "GHz"
                label: Strings.t("dash.freq")
                iconName: "speedometer"
            }

            Gauge {
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                value: page.dgpu.powerW
                available: page.dgpuPowerAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                to: 90
                decimals: 0
                unit: "W"
                label: Strings.t("dash.powerDraw")
                iconName: "power"
            }

            Gauge {
                objectName: "dashboardGpuFanGauge"
                Layout.fillWidth: true
                Layout.maximumWidth: 168
                implicitHeight: 150
                visible: page.gpuFan !== null
                value: page.gpuFanAvailable ? page.gpuFan.speed : 0
                available: page.gpuFanAvailable
                unavailableReason: Strings.t("dash.missingSensor")
                unit: "%"
                label: Strings.t("dash.fanGpu")
                iconName: "fan"
            }
        }

        // ── Température dGPU : historique intégré à la carte ────────────────
        ColumnLayout {
            objectName: "dashboardDgpuChartBlock"
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            spacing: Theme.s6
            visible: page.dgpuChartVisible

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s6

                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: 4
                    color: Theme.info
                }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: Strings.t("dash.temp")
                }

                Item { Layout.fillWidth: true }

                AppLabel {
                    objectName: "dashboardDgpuChartValue"
                    variant: "caption"
                    secondary: true
                    text: page.dgpuTempAvailable ? Strings.formatTemp(page.dgpu.temp) : "—"
                }
            }

            Chart {
                objectName: "dashboardDgpuChart"
                Layout.fillWidth: true
                Layout.preferredHeight: 90
                values: HardwareData.isLive ? page.gpuSamples : DemoData.dgpuHistory
                autoScale: true
                strokeColor: Theme.info
                showGrid: true
            }
        }
    }

    InlineNotice {
        objectName: "dashboardAgentNotice"
        Layout.fillWidth: true
        visible: !page.agentUsable
        kind: page.agentAvailable ? "warn" : "info"
        title: page.agentAvailable ? Strings.t("display.agentUnsupported")
                                   : Strings.t("display.agentUnavailable")
        body: AgentData.errorText !== "" ? AgentData.errorText
              : (AgentData.displayErrorText !== "" ? AgentData.displayErrorText
                 : Strings.t("display.agentUsableHint"))
        actionText: Strings.t("display.refreshAgent")
        onActionClicked: AppState.requestAgentRefresh()
    }
}
