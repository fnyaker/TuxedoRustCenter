// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Aperçu du menu de la zone de notification (tray). En production, ce contenu
// est hébergé par l'icône tray native (QSystemTrayIcon + QQuickWindow) pilotée
// par tcc-gui ; le QML ci-dessous décrit uniquement la présentation.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Popup {
    id: tray

    /// Intention locale, révertée sur refus via `PowerBlockerData.blocked`.
    property bool blockerDraft: PowerBlockerData.blocked

    parent: Overlay.overlay
    width: 312
    padding: 0
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    Connections {
        target: AutostartData

        function onEnabledChanged() {
            autostartSwitch.checked = AutostartData.enabled;
        }
    }

    Connections {
        target: PowerBlockerData

        function onBlockedChanged() {
            if (!PowerBlockerData.pending)
                tray.blockerDraft = PowerBlockerData.blocked;
        }

        function onPendingChanged() {
            if (!PowerBlockerData.pending)
                tray.blockerDraft = PowerBlockerData.blocked;
        }
    }

    background: Rectangle {
        radius: Theme.radiusLg
        color: Theme.surface
        border.width: 1
        border.color: Theme.borderStrong
    }

    function activateProfile(profileId, profileName) {
        if (LiveData.usingDemo) {
            DemoData.activeProfileId = profileId;
            AppState.manualProfileId = profileId;
            AppState.notify(Strings.tf("toast.profileActivated", [profileName]), "ok");
        } else {
            AppState.requestProfileActivation(profileId);
            AppState.notify(Strings.tf("toast.profileActivationRequested", [profileName]), "info");
        }
        tray.close();
    }

    contentItem: ColumnLayout {
        spacing: 0

        // En-tête
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.s12
            spacing: Theme.s10

            Rectangle {
                Layout.preferredWidth: 30
                Layout.preferredHeight: 30
                radius: Theme.radiusSm
                color: "#ffffff"
                border.width: 1
                border.color: Theme.border

                Image {
                    anchors.centerIn: parent
                    source: "../../assets/brand/taskbar_icon_free.svg"
                    sourceSize.width: 44
                    width: 22
                    height: 22
                    fillMode: Image.PreserveAspectFit
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                AppLabel {
                    variant: "small"
                    strong: true
                    text: Strings.t("tray.title")
                }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: Strings.tf("tray.version", [Qt.application.version])
                }
            }

            StatusBadge {
                kind: LiveData.connected ? "ok" : "warn"
                label: LiveData.connected ? Strings.t("state.connected")
                                          : Strings.t("state.disconnected")
            }
        }

        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

        AppLabel {
            Layout.leftMargin: Theme.s12
            Layout.topMargin: Theme.s10
            variant: "label"
            secondary: true
            text: Strings.t("tray.profiles")
        }

        Repeater {
            model: LiveData.profiles

            delegate: AbstractButton {
                id: profileRow
                required property var modelData

                Layout.fillWidth: true
                Layout.leftMargin: Theme.s6
                Layout.rightMargin: Theme.s6
                implicitHeight: 34
                hoverEnabled: true
                Accessible.role: Accessible.RadioButton
                Accessible.name: modelData.name
                Accessible.checked: LiveData.resolvedActiveId === modelData.id

                onClicked: tray.activateProfile(modelData.id, modelData.name)

                background: Rectangle {
                    radius: Theme.radiusXs
                    color: profileRow.hovered ? Theme.surfaceHover : "transparent"
                }

                contentItem: RowLayout {
                    spacing: Theme.s8

                    AppIcon {
                        Layout.leftMargin: Theme.s8
                        name: profileRow.modelData.icon
                        monochrome: false
                        size: 18
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "small"
                        text: profileRow.modelData.name
                        elide: Text.ElideRight
                    }

                    Glyph {
                        Layout.rightMargin: Theme.s8
                        visible: LiveData.resolvedActiveId === profileRow.modelData.id
                        name: "check"
                        color: Theme.accentText
                        size: 12
                    }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; Layout.topMargin: Theme.s6; Layout.preferredHeight: 1; color: Theme.divider }

        // Interrupteurs
        Item {
            Layout.fillWidth: true
            Layout.margins: Theme.s8
            implicitHeight: 34

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.s4
                anchors.rightMargin: Theme.s4
                spacing: Theme.s8

                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    text: Strings.t("tray.blocker")
                }

                AppSwitch {
                    checked: tray.blockerDraft
                    enabled: PowerBlockerData.editable
                    Accessible.name: Strings.t("tray.blocker")
                    onToggled: {
                        tray.blockerDraft = checked;
                        AppState.requestPowerBlocker(checked);
                    }
                }
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.margins: Theme.s8
            implicitHeight: 34

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.s4
                anchors.rightMargin: Theme.s4
                spacing: Theme.s8

                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    text: Strings.t("tray.autostart")
                }

                AppSwitch {
                    id: autostartSwitch
                    checked: AutostartData.enabled
                    enabled: AutostartData.editable
                    Accessible.name: Strings.t("tray.autostart")
                    onToggled: {
                        AppState.requestAutostart(checked);
                        // Écriture synchrone : réapplique l'état réel.
                        autostartSwitch.checked = AutostartData.enabled;
                    }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

        AbstractButton {
            id: openRow
            Layout.fillWidth: true
            Layout.margins: Theme.s6
            implicitHeight: 34
            hoverEnabled: true
            Accessible.role: Accessible.Button
            Accessible.name: Strings.t("tray.open")

            onClicked: {
                tray.close();
                AppState.navigate("dashboard");
            }

            background: Rectangle {
                radius: Theme.radiusXs
                color: openRow.hovered ? Theme.surfaceHover : "transparent"
            }

            contentItem: RowLayout {
                spacing: Theme.s8
                AppIcon {
                    Layout.leftMargin: Theme.s8
                    name: "monitor"
                    color: Theme.textSecondary
                    size: 15
                }
                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    text: Strings.t("tray.open")
                }
            }
        }

        AbstractButton {
            id: quitRow
            Layout.fillWidth: true
            Layout.leftMargin: Theme.s6
            Layout.rightMargin: Theme.s6
            Layout.bottomMargin: Theme.s6
            implicitHeight: 34
            hoverEnabled: true
            Accessible.role: Accessible.Button
            Accessible.name: Strings.t("tray.quit")

            onClicked: {
                tray.close();
                AppState.quitting = true;
                Qt.quit();
            }

            background: Rectangle {
                radius: Theme.radiusXs
                color: quitRow.hovered ? Theme.surfaceHover : "transparent"
            }

            contentItem: RowLayout {
                spacing: Theme.s8
                AppIcon {
                    Layout.leftMargin: Theme.s8
                    name: "power"
                    color: Theme.error
                    size: 15
                }
                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    text: Strings.t("tray.quit")
                    color: Theme.error
                }
            }
        }
    }
}
