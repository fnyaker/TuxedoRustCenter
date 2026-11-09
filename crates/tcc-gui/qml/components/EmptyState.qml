// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// État vide : message explicite + action de sortie. Jamais un écran blanc.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string iconName: "info"
    property string title: ""
    property string body: ""
    property string actionText: ""

    signal actionClicked()

    implicitHeight: content.implicitHeight + Theme.s32

    ColumnLayout {
        id: content
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.s32, 380)
        spacing: Theme.s10

        AppIcon {
            Layout.alignment: Qt.AlignHCenter
            name: root.iconName
            color: Theme.textMuted
            size: 34
        }

        AppLabel {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            variant: "h3"
            text: root.title
        }

        AppLabel {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            variant: "small"
            secondary: true
            text: root.body
            wrapMode: Text.WordWrap
        }

        AppButton {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.s6
            visible: root.actionText !== ""
            variant: "secondary"
            text: root.actionText
            onClicked: root.actionClicked()
        }
    }
}
