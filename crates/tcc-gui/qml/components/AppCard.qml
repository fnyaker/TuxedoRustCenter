// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Surface de base : carte bordée, sans ombre (choix de performance et de
// lisibilité). Le contenu est un ColumnLayout : utiliser Layout.fillWidth.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    default property alias content: contentLayout.data

    property real padding: Theme.s20
    property string title: ""
    property string subtitle: ""
    property string headerIcon: ""
    property bool highlighted: false
    property bool interactive: false

    implicitWidth: 260
    implicitHeight: outer.implicitHeight + padding * 2
    radius: Theme.radiusLg
    color: Theme.surface
    border.width: 1
    border.color: highlighted ? Theme.accent
                : (interactive && hoverArea.hovered ? Theme.borderStrong : Theme.border)
    Behavior on border.color {
        ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
    }

    HoverHandler {
        id: hoverArea
        enabled: root.interactive
        cursorShape: root.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
    }

    ColumnLayout {
        id: outer
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: root.padding
        spacing: Theme.s16

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s10
            visible: root.title !== ""

            AppIcon {
                visible: root.headerIcon !== ""
                name: root.headerIcon
                color: root.highlighted ? Theme.accent : Theme.textSecondary
                size: 18
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                AppLabel {
                    Layout.fillWidth: true
                    variant: "h3"
                    text: root.title
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    secondary: true
                    visible: root.subtitle !== ""
                    text: root.subtitle
                }
            }
        }

        ColumnLayout {
            id: contentLayout
            Layout.fillWidth: true
            spacing: Theme.s12
        }
    }
}
