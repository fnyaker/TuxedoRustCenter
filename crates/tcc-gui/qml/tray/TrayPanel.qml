// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Panneau compact du tray : profil actif, bascule rapide, état PPD × alimentation,
// Fn-Lock, inhibiteur de veille, autostart et actions. Aucune logique métier :
// les singletons `LiveData`/`HardwareData`/`PowerBlockerData`/`AutostartData`
// restent la source de vérité ; les actions remontent à `SystemTray`.
//
// Fenêtre `Qt.Popup` sans décoration, positionnée près du coin bas-droit de
// l'écran. Sur les compositeurs qui ne permettent pas le positionnement des
// popups (Wayland), le compositeur peut replacer la fenêtre ; le menu tray
// reste disponible en secours.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import "../components"
import "../core"

Window {
    id: panel

    objectName: "trayPanel"
    flags: Qt.Popup | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
    modality: Qt.NonModal
    color: "transparent"
    width: 360
    height: Math.min(Screen.height - 24,
                     content.implicitHeight + Theme.s24 + 8)
    visible: false

    // ── Entrées, poussées par SystemTray ────────────────────────────────────
    property var profiles: []
    property string activeProfileId: ""
    property string activeProfileName: ""
    property string activeProfileDescription: ""
    property string ppd: ""
    property string power: ""
    property bool fnLockVisible: false
    property bool fnLockChecked: false
    property bool fnLockEnabled: true
    property bool blockerChecked: false
    property bool blockerSupported: false
    property bool autostartChecked: false
    property bool autostartSupported: false

    // ── Actions ─────────────────────────────────────────────────────────────
    signal openWindowRequested()
    signal quitRequested()
    signal profileRequested(string profileId)
    signal fnLockRequested(bool value)
    signal blockerRequested(bool value)
    signal autostartRequested(bool value)

    // Coin bas-droit de l'écran principal ; les compositeurs Wayland peuvent
    // replacer la fenêtre (limite documentée).
    x: Screen.width - width - 12
    y: Screen.height - height - 12

    /// Texte saisi dans la recherche de profils (vidé à chaque ouverture).
    property string profileFilter: ""

    /// Profils dont le nom contient tous les mots saisis (insensible à la casse).
    readonly property var filteredProfiles: {
        var words = panel.profileFilter.toLowerCase().split(/\s+/).filter(function(w) {
            return w.length > 0;
        });
        if (words.length === 0)
            return panel.profiles;
        return panel.profiles.filter(function(profile) {
            var name = String(profile.name).toLowerCase();
            return words.every(function(w) { return name.indexOf(w) >= 0; });
        });
    }

    onVisibleChanged: if (visible) panel.profileFilter = ""

    function ppdLabel() {
        if (panel.ppd === "power-saver")
            return Strings.t("settings.ppdSaver");
        if (panel.ppd === "performance")
            return Strings.t("settings.ppdPerformance");
        if (panel.ppd === "balanced")
            return Strings.t("settings.ppdBalanced");
        return Strings.t("settings.ppdUnknown");
    }

    function powerLabel() {
        if (panel.power === "ac")
            return Strings.t("dash.ac");
        if (panel.power === "battery")
            return Strings.t("dash.battery");
        return Strings.t("state.unavailable");
    }

    Rectangle {
        id: card
        anchors.fill: parent
        radius: Theme.radiusLg
        color: Theme.surface
        border.width: 1
        border.color: Theme.borderStrong

        ColumnLayout {
            id: content
            anchors.fill: parent
            anchors.margins: Theme.s12
            spacing: Theme.s8

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s8

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "title"
                        text: panel.activeProfileName !== ""
                              ? panel.activeProfileName
                              : Strings.t("dash.noData")
                        elide: Text.ElideRight
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        visible: panel.activeProfileDescription !== ""
                        variant: "caption"
                        secondary: true
                        text: panel.activeProfileDescription
                        elide: Text.ElideRight
                        maximumLineCount: 1
                    }
                }

                ColumnLayout {
                    spacing: 2

                    StatusBadge {
                        kind: "info"
                        label: panel.ppdLabel()
                    }

                    StatusBadge {
                        kind: panel.power === "ac" ? "ok" : "neutral"
                        label: panel.powerLabel()
                    }
                }
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

            AppLabel {
                Layout.fillWidth: true
                variant: "label"
                secondary: true
                text: Strings.t("profiles.title")
            }

            AppTextField {
                id: profileSearch
                objectName: "panelProfileSearch"
                Layout.fillWidth: true
                visible: panel.profiles.length > 0
                placeholderText: Strings.t("tray.search")
                Accessible.name: Strings.t("tray.search")
                text: panel.profileFilter
                onTextChanged: panel.profileFilter = text
            }

            ListView {
                id: profileList
                objectName: "panelProfileList"
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(contentHeight, 176)
                visible: panel.profiles.length > 0
                clip: true
                spacing: 0
                model: panel.filteredProfiles
                boundsBehavior: Flickable.StopAtBounds

                delegate: AbstractButton {
                    id: profileChoice
                    required property var modelData
                    width: ListView.view ? ListView.view.width : 0
                    height: 32
                    hoverEnabled: true
                    Accessible.role: Accessible.RadioButton
                    Accessible.name: modelData.name
                    Accessible.checked: panel.activeProfileId === modelData.id
                    onClicked: panel.profileRequested(modelData.id)

                    background: Rectangle {
                        radius: Theme.radiusSm
                        color: profileChoice.hovered ? Theme.surfaceHover : "transparent"
                    }

                    contentItem: RowLayout {
                        spacing: Theme.s8

                        Rectangle {
                            Layout.preferredWidth: 14
                            Layout.preferredHeight: 14
                            radius: 7
                            color: "transparent"
                            border.width: 1
                            border.color: panel.activeProfileId === profileChoice.modelData.id
                                          ? Theme.accent : Theme.borderStrong

                            Rectangle {
                                anchors.centerIn: parent
                                width: 6
                                height: 6
                                radius: 3
                                color: Theme.accent
                                visible: panel.activeProfileId === profileChoice.modelData.id
                            }
                        }

                        AppLabel {
                            Layout.fillWidth: true
                            variant: "body"
                            text: profileChoice.modelData.name
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            AppLabel {
                Layout.fillWidth: true
                visible: panel.profiles.length > 0 && panel.filteredProfiles.length === 0
                variant: "caption"
                secondary: true
                text: Strings.t("tray.noMatch")
            }

            AppLabel {
                Layout.fillWidth: true
                visible: panel.profiles.length === 0
                variant: "caption"
                secondary: true
                text: Strings.t("dash.noData")
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

            ToggleRow {
                objectName: "panelFnLockToggle"
                Layout.fillWidth: true
                visible: panel.fnLockVisible
                label: Strings.t("settings.fnLock")
                checked: panel.fnLockChecked
                supported: panel.fnLockEnabled
                unsupportedReason: Strings.t("settings.fnLockUnknown")
                onToggled: function(checked) { panel.fnLockRequested(checked); }
            }

            ToggleRow {
                objectName: "panelBlockerToggle"
                Layout.fillWidth: true
                label: Strings.t("settings.powerBlocker")
                checked: panel.blockerChecked
                supported: panel.blockerSupported
                unsupportedReason: Strings.t("settings.powerBlockerNotGraphical")
                onToggled: function(checked) { panel.blockerRequested(checked); }
            }

            ToggleRow {
                objectName: "panelAutostartToggle"
                Layout.fillWidth: true
                label: Strings.t("settings.trayAutostart")
                checked: panel.autostartChecked
                supported: panel.autostartSupported
                unsupportedReason: Strings.t("settings.autostartNoHome")
                onToggled: function(checked) { panel.autostartRequested(checked); }
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s8

                AppButton {
                    objectName: "panelOpenButton"
                    text: Strings.t("tray.open")
                    iconName: "dashboard"
                    onClicked: panel.openWindowRequested()
                }

                Item { Layout.fillWidth: true }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: "v" + Qt.application.version
                }

                AppButton {
                    objectName: "panelQuitButton"
                    variant: "ghost"
                    text: Strings.t("tray.quit")
                    onClicked: panel.quitRequested()
                }
            }
        }
    }
}
