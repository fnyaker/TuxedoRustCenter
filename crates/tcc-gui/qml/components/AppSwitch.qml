// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Interrupteur accessible. Vert = activé (convention de l'original) ; le rouge
// reste réservé à l'action primaire et au profil actif.
// Le libellé est fourni par ToggleRow : ce composant ne porte que le contrôle.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

Switch {
    id: control

    implicitWidth: 40
    implicitHeight: 24
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    Accessible.checked: checked

    indicator: Rectangle {
        implicitWidth: 40
        implicitHeight: 24
        radius: 12
        color: !control.enabled ? Theme.track
             : control.checked ? (control.hovered ? Qt.darker(Theme.success, 1.1) : Theme.success)
             : (control.hovered ? Theme.borderStrong : Theme.track)
        Behavior on color {
            ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
        }

        Rectangle {
            width: 18
            height: 18
            radius: 9
            y: 3
            x: control.checked ? parent.width - width - 3 : 3
            color: control.checked ? "#ffffff" : (Theme.dark ? "#c9c9cf" : "#ffffff")
            border.width: control.checked ? 0 : 1
            border.color: Theme.borderStrong
            Behavior on x {
                NumberAnimation { duration: Theme.durationBase; easing.type: Theme.easeOut }
            }
        }

        Rectangle {
            anchors.fill: parent
            anchors.margins: -4
            radius: height / 2
            color: "transparent"
            border.width: 2
            border.color: Theme.focusRing
            visible: control.visualFocus
        }
    }

    contentItem: Item { implicitWidth: 0; implicitHeight: 0 }
}
