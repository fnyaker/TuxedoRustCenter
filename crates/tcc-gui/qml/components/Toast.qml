// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Notification transitoire non bloquante. Le texte est annoncé aux lecteurs
// d'écran ; la fermeture manuelle reste possible (cible 24 px).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import "../core"

Rectangle {
    id: root

    property int toastId: 0
    property string message: ""
    property string kind: "info"     // info | ok | warn | error
    property int duration: 4200

    readonly property color tone: kind === "ok" ? Theme.success
                                 : kind === "warn" ? Theme.warning
                                 : kind === "error" ? Theme.error
                                 : Theme.info

    implicitWidth: Math.min(420, row.implicitWidth + Theme.s16 * 2)
    implicitHeight: row.implicitHeight + Theme.s16 * 2
    radius: Theme.radiusMd
    color: Theme.surface
    border.width: 1
    border.color: Theme.borderStrong
    opacity: 0

    Accessible.role: Accessible.StaticText
    Accessible.name: message

    Component.onCompleted: opacity = 1
    Behavior on opacity {
        NumberAnimation { duration: Theme.durationBase; easing.type: Theme.easeOut }
    }

    Timer {
        interval: root.duration
        running: true
        onTriggered: AppState.dismissToast(root.toastId)
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: Theme.s16
        spacing: Theme.s12

        Rectangle {
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: 3
            Layout.preferredHeight: 26
            radius: 1.5
            color: root.tone
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            text: root.message
            wrapMode: Text.WordWrap
        }

        AbstractButton {
            id: closeButton
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: 24
            implicitHeight: 24
            hoverEnabled: true
            Accessible.role: Accessible.Button
            Accessible.name: Strings.t("common.close")
            onClicked: AppState.dismissToast(root.toastId)

            background: Rectangle {
                radius: Theme.radiusXs
                color: closeButton.hovered ? Theme.surfaceHover : "transparent"
            }

            contentItem: Glyph {
                anchors.centerIn: parent
                name: "close"
                color: Theme.textMuted
                size: 12
            }
        }
    }
}
