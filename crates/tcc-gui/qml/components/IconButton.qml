// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Bouton icône 32×32 minimum (cible WCAG 2.5.8), avec infobulle et nom
// accessible obligatoire lorsque l'icône est seule.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import QtQuick.Controls

AbstractButton {
    id: control

    property string iconName: ""
    property bool monochromeIcon: true
    property color iconColor: Theme.textSecondary
    property string tooltip: ""

    implicitWidth: Theme.hitTarget
    implicitHeight: Theme.hitTarget
    hoverEnabled: true
    Accessible.name: text !== "" ? text : tooltip
    Accessible.role: Accessible.Button

    background: Rectangle {
        radius: Theme.radiusSm
        color: !control.enabled ? "transparent"
             : control.pressed ? Theme.surfaceSunken
             : control.hovered ? Theme.surfaceHover : "transparent"
        Behavior on color {
            ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
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

    contentItem: AppIcon {
        anchors.centerIn: parent
        name: control.iconName
        monochrome: control.monochromeIcon
        color: control.enabled ? control.iconColor : Theme.textMuted
        size: 18
    }

    AppToolTip {
        visible: control.hovered && control.tooltip !== "" && !AppState.toasts.count
        text: control.tooltip
        delay: 600
    }
}
