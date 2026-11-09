// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne « profil utilisé » de la barre latérale : icône d'état (secteur ou
// batterie), libellé de l'état et nom du profil assigné.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

AbstractButton {
    id: control

    property string stateIcon: "pluggedin"
    property string stateLabel: ""
    property string profileName: ""
    property bool compact: false

    implicitHeight: 44
    implicitWidth: compact ? 52 : Theme.sidebarWidth - Theme.s24
    hoverEnabled: true
    Accessible.role: Accessible.Button
    Accessible.name: stateLabel + " : " + profileName

    background: Rectangle {
        radius: Theme.radiusSm
        color: control.pressed ? Theme.surfaceSunken
             : control.hovered ? Theme.surfaceHover : "transparent"

        Rectangle {
            anchors.fill: parent
            anchors.margins: -2
            radius: parent.radius + 2
            color: "transparent"
            border.width: 2
            border.color: Theme.focusRing
            visible: control.visualFocus
        }
    }

    contentItem: RowLayout {
        spacing: Theme.s12

        Item {
            visible: control.compact
            Layout.fillWidth: true
        }

        AppIcon {
            Layout.leftMargin: control.compact ? 0 : Theme.s4
            name: control.stateIcon
            color: Theme.textSecondary
            size: 16
        }

        ColumnLayout {
            Layout.fillWidth: true
            visible: !control.compact
            spacing: 0

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                text: control.stateLabel
            }

            AppLabel {
                Layout.fillWidth: true
                variant: "small"
                text: control.profileName
                elide: Text.ElideRight
                wrapMode: Text.NoWrap
            }
        }

        Item {
            visible: control.compact
            Layout.fillWidth: true
        }
    }

    AppToolTip {
        visible: control.hovered && control.compact
        text: control.stateLabel + " : " + control.profileName
        delay: 400
    }
}
