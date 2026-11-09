// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Entrée de navigation de la barre latérale. Sélection = fond accent très
// clair + barre de 3 px + texte semi-gras : l'état ne repose jamais sur la
// seule couleur.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

AbstractButton {
    id: control

    property string iconName: ""
    property string label: ""
    property bool selected: false
    property bool compact: false
    property string badgeText: ""
    property string badgeKind: "neutral"

    implicitHeight: 40
    implicitWidth: compact ? 52 : Theme.sidebarWidth - Theme.s24
    hoverEnabled: true
    Accessible.role: Accessible.PageTab
    Accessible.name: label
    Accessible.selected: selected
    Accessible.description: badgeText

    background: Rectangle {
        radius: Theme.radiusSm
        color: !control.enabled ? "transparent"
             : control.selected ? Theme.accentSoft
             : control.pressed ? Theme.surfaceSunken
             : control.hovered ? Theme.surfaceHover : "transparent"
        Behavior on color {
            ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
        }

        Rectangle {
            visible: control.selected
            width: 3
            height: parent.height - 10
            radius: 1.5
            anchors.left: parent.left
            anchors.leftMargin: -Theme.s8
            anchors.verticalCenter: parent.verticalCenter
            color: Theme.accent
        }

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

        // Deux ressorts égaux en mode compact pour centrer l'icône sans
        // combinaison d'ancres conditionnelles.
        Item {
            visible: control.compact
            Layout.fillWidth: true
        }

        AppIcon {
            Layout.leftMargin: control.compact ? 0 : Theme.s4
            Layout.alignment: Qt.AlignVCenter
            name: control.iconName
            color: !control.enabled ? Theme.textMuted
                 : control.selected ? Theme.accentText
                 : Theme.textSecondary
            size: 18
        }

        AppLabel {
            Layout.fillWidth: true
            visible: !control.compact
            variant: "body"
            strong: control.selected
            text: control.label
            color: !control.enabled ? Theme.textMuted
                 : control.selected ? Theme.textPrimary
                 : Theme.textSecondary
            elide: Text.ElideRight
            wrapMode: Text.NoWrap
        }

        StatusBadge {
            Layout.alignment: Qt.AlignVCenter
            Layout.rightMargin: control.compact ? 0 : Theme.s4
            visible: !control.compact && control.badgeText !== ""
            kind: control.badgeKind
            label: control.badgeText
        }

        Item {
            visible: control.compact
            Layout.fillWidth: true
        }
    }

    AppToolTip {
        visible: control.hovered && control.compact && control.label !== ""
        text: control.label
        delay: 400
    }
}
