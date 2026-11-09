// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Carte de profil. Deux présentations : grille (illustration + capacités) et
// liste compacte. L'interrupteur active le profil manuellement ; les icônes de
// state indiquent les assignations automatiques (secteur / batterie).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    required property var profile
    property bool listMode: false
    property bool active: false

    signal activateRequested(bool checked)
    signal editRequested()
    signal duplicateRequested()
    signal exportRequested()
    signal deleteRequested()

    implicitHeight: listMode ? 92 : gridContent.implicitHeight + Theme.s16 * 2
    radius: Theme.radiusLg
    color: Theme.surface
    border.width: 1
    border.color: active ? Theme.accent : (hoverArea.hovered ? Theme.borderStrong : Theme.border)
    // Grille : ne jamais tasser une carte sous 300 px (cf. ProfilesPage).
    readonly property int minimumCardWidth: 300
    Layout.minimumWidth: minimumCardWidth
    Behavior on border.color {
        ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
    }

    HoverHandler { id: hoverArea }

    function fanLabel(mode) {
        return Strings.t("fan.mode." + mode);
    }

    // Métriques : les valeurs absentes du profil (sentinelles `-1`, `useXxx`
    // faux) sont affichées comme « inchangé », jamais inventées.
    function coresText() {
        if (root.profile.coresKnown === false)
            return Strings.t("common.auto");
        return root.profile.cores + " " + Strings.t("cpu.coresUnit");
    }

    function freqText() {
        if (root.profile.freqMaxReduced === true)
            return Strings.t("cpu.reduced");
        if (root.profile.freqMax > 0)
            return Strings.formatNumber(root.profile.freqMin, 1) + " – "
                    + Strings.formatNumber(root.profile.freqMax, 1) + " GHz";
        return Strings.t("common.auto");
    }

    function brightnessText() {
        if (root.profile.useBrightness === false)
            return Strings.t("common.unchanged");
        return Strings.formatPercent(root.profile.brightness);
    }

    function refreshText() {
        if (root.profile.useRefRate === false)
            return Strings.t("common.unchanged");
        return Strings.formatHz(root.profile.refreshRate);
    }

    function webcamText() {
        if (root.profile.useWebcam === false)
            return Strings.t("common.unchanged");
        return root.profile.webcam ? Strings.t("common.enabled") : Strings.t("common.disabled");
    }

    // ── Grille ──────────────────────────────────────────────────────────────
    ColumnLayout {
        id: gridContent
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Theme.s16
        spacing: Theme.s8
        visible: !root.listMode

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s4

            AppSwitch {
                id: gridSwitch
                Accessible.name: root.active ? Strings.t("profiles.deactivate")
                                             : Strings.t("profiles.activate")
                // `clicked` (et non `toggled`) : le Binding ci-dessous
                // modifie `checked` depuis le daemon, sans effet de bord.
                onClicked: root.activateRequested(checked)
            }

            // Le clic casse la liaison `checked` ; ce Binding re-applique l'état
            // du daemon (et fournit la valeur initiale) pour que l'affichage ne
            // reste pas optimiste après un snapshot.
            Binding {
                target: gridSwitch
                property: "checked"
                value: root.active
                restoreMode: Binding.RestoreNone
            }

            Item { Layout.fillWidth: true }

            AppIcon {
                visible: root.profile.assignedAc === true
                name: "pluggedin"
                color: Theme.textSecondary
                size: 15
                AppToolTip {
                    visible: acHover.hovered
                    text: Strings.t("profiles.assignedAc")
                }
                HoverHandler { id: acHover }
            }

            AppIcon {
                visible: root.profile.assignedBat === true
                name: "batterymode"
                color: Theme.textSecondary
                size: 15
                Layout.leftMargin: Theme.s4
                AppToolTip {
                    visible: batHover.hovered
                    text: Strings.t("profiles.assignedBat")
                }
                HoverHandler { id: batHover }
            }

            IconButton {
                id: kebab
                iconName: ""
                implicitWidth: 28
                implicitHeight: 28
                Accessible.name: Strings.t("common.more")
                contentItem: Glyph {
                    anchors.centerIn: parent
                    name: "kebab"
                    color: Theme.textSecondary
                    size: 16
                }
                onClicked: cardMenu.showAt(kebab)
            }
        }

        AppIcon {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.s4
            name: root.profile.icon
            monochrome: false
            size: 58
        }

        AppLabel {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            variant: "h3"
            text: root.profile.name
            elide: Text.ElideRight
        }

        StatusBadge {
            Layout.alignment: Qt.AlignHCenter
            visible: root.active
            kind: "accent"
            label: Strings.t("profiles.active")
        }

        AppLabel {
            Layout.fillWidth: true
            Layout.topMargin: 2
            horizontalAlignment: Text.AlignHCenter
            variant: "small"
            secondary: true
            text: root.profile.description
            wrapMode: Text.WordWrap
            maximumLineCount: 3
            elide: Text.ElideRight
        }

        GridLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            columns: 2
            columnSpacing: Theme.s12
            rowSpacing: Theme.s8

            Repeater {
                model: [
                    { "icon": "cpu", "text": root.coresText() },
                    { "icon": "speedometer", "text": root.freqText() },
                    { "icon": "fan", "text": root.fanLabel(root.profile.fanMode) },
                    { "icon": "theme", "text": root.brightnessText() },
                    { "icon": "monitor", "text": root.refreshText() },
                    { "icon": "webcam", "text": root.webcamText() }
                ]

                delegate: RowLayout {
                    id: metricRow
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: Theme.s6

                    AppIcon {
                        name: metricRow.modelData.icon
                        color: Theme.textMuted
                        size: 14
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "caption"
                        secondary: true
                        text: metricRow.modelData.text
                        elide: Text.ElideRight
                    }
                }
            }
        }
    }

    // ── Liste compacte ──────────────────────────────────────────────────────
    RowLayout {
        anchors.fill: parent
        anchors.margins: Theme.s16
        spacing: Theme.s16
        visible: root.listMode

        AppIcon {
            name: root.profile.icon
            monochrome: false
            size: 40
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            RowLayout {
                spacing: Theme.s8

                AppLabel {
                    variant: "h3"
                    text: root.profile.name
                    elide: Text.ElideRight
                }

                StatusBadge {
                    visible: root.active
                    kind: "accent"
                    label: Strings.t("profiles.active")
                }

                AppIcon {
                    visible: root.profile.assignedAc === true
                    name: "pluggedin"
                    color: Theme.textMuted
                    size: 13
                }

                AppIcon {
                    visible: root.profile.assignedBat === true
                    name: "batterymode"
                    color: Theme.textMuted
                    size: 13
                }
            }

            AppLabel {
                Layout.fillWidth: true
                variant: "small"
                secondary: true
                text: root.profile.description
                elide: Text.ElideRight
                maximumLineCount: 1
            }
        }

        AppLabel {
            visible: root.width > 720
            variant: "caption"
            secondary: true
            text: root.profile.cores + " · " + Strings.formatNumber(root.profile.freqMax, 1) + " GHz · "
                  + root.fanLabel(root.profile.fanMode) + " · " + Strings.formatHz(root.profile.refreshRate)
        }

        AppSwitch {
            id: listSwitch
            Accessible.name: root.profile.name
            onClicked: root.activateRequested(checked)
        }

        Binding {
            target: listSwitch
            property: "checked"
            value: root.active
            restoreMode: Binding.RestoreNone
        }

        IconButton {
            id: kebabList
            iconName: ""
            implicitWidth: 28
            implicitHeight: 28
            Accessible.name: Strings.t("common.more")
            contentItem: Glyph {
                anchors.centerIn: parent
                name: "kebab"
                color: Theme.textSecondary
                size: 16
            }
            onClicked: cardMenu.showAt(kebabList)
        }
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        onClicked: root.editRequested()
        enabled: !root.listMode
        z: -1
    }

    AppContextMenu {
        id: cardMenu
        items: [
            { "label": Strings.t("profiles.edit"), "icon": "settings" },
            { "label": Strings.t("profiles.clone"), "icon": "copy",
              "enabled": root.profile.preset !== true,
              "disabledReason": Strings.t("profiles.presetActionHint") },
            { "label": Strings.t("profiles.export"), "icon": "download" },
            { "label": Strings.t("profiles.delete"), "icon": "trash", "destructive": true,
              "enabled": root.profile.preset !== true,
              "disabledReason": Strings.t("profiles.presetActionHint") }
        ]
        onTriggered: function(index) {
            if (index === 0) root.editRequested();
            else if (index === 1 && root.profile.preset !== true) root.duplicateRequested();
            else if (index === 2) root.exportRequested();
            else if (index === 3 && root.profile.preset !== true) root.deleteRequested();
        }
    }
}
