// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// En-tête de section interne à une carte : icône dans une pastille teintée,
// titre, aide courte et action à droite.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    default property alias actions: actionRow.data

    property string title: ""
    property string hint: ""
    property string iconName: ""

    implicitHeight: Math.max(iconChip.height, textColumn.implicitHeight)

    RowLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.s12

        Rectangle {
            id: iconChip
            visible: root.iconName !== ""
            Layout.preferredWidth: 30
            Layout.preferredHeight: 30
            radius: Theme.radiusSm
            color: Theme.accentSoft

            AppIcon {
                anchors.centerIn: parent
                name: root.iconName
                color: Theme.accentText
                size: 16
            }
        }

        ColumnLayout {
            id: textColumn
            Layout.fillWidth: true
            spacing: 0

            AppLabel {
                Layout.fillWidth: true
                variant: "h3"
                text: root.title
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.hint !== ""
                variant: "small"
                secondary: true
                text: root.hint
            }
        }

        RowLayout {
            id: actionRow
            spacing: Theme.s8
        }
    }
}
