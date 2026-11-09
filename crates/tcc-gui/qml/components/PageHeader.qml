// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// En-tête de page : titre, sous-titre, fil d'ariane optionnel et actions.
// Les actions sont fournies via le contenu par défaut (RowLayout).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    default property alias actions: actionRow.data

    property string title: ""
    property string subtitle: ""
    property string eyebrow: ""
    property string iconName: ""

    implicitHeight: headerLayout.implicitHeight

    RowLayout {
        id: headerLayout
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Theme.s16

        Rectangle {
            visible: root.iconName !== ""
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: 44
            Layout.preferredHeight: 44
            radius: Theme.radiusMd
            color: Theme.accentSoft

            AppIcon {
                anchors.centerIn: parent
                name: root.iconName
                color: Theme.accentText
                size: 22
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Theme.s2

            AppLabel {
                visible: root.eyebrow !== ""
                variant: "label"
                secondary: true
                text: root.eyebrow
            }

            AppLabel {
                Layout.fillWidth: true
                variant: "h1"
                text: root.title
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.subtitle !== ""
                variant: "body"
                secondary: true
                text: root.subtitle
            }
        }

        RowLayout {
            id: actionRow
            Layout.alignment: Qt.AlignVCenter
            spacing: Theme.s8
        }
    }
}
