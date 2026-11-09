// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Curseur natif : piste fine, remplissage accent, poignée contrastée.
// Le libellé et la valeur sont fournis par SliderRow.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

Slider {
    id: control

    implicitHeight: 24
    implicitWidth: 160
    hoverEnabled: true

    background: Rectangle {
        x: control.leftPadding
        y: control.topPadding + control.availableHeight / 2 - height / 2
        width: control.availableWidth
        height: 4
        radius: 2
        color: Theme.gaugeTrack

        Rectangle {
            width: control.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: control.enabled ? Theme.accent : Theme.textMuted
        }
    }

    handle: Rectangle {
        x: control.leftPadding + control.visualPosition * (control.availableWidth - width)
        y: control.topPadding + control.availableHeight / 2 - height / 2
        width: 16
        height: 16
        radius: 8
        color: Theme.surface
        border.width: 2
        border.color: control.enabled ? Theme.accent : Theme.textMuted
        opacity: control.enabled ? 1 : 0.7

        Rectangle {
            anchors.centerIn: parent
            width: parent.width + 10
            height: parent.height + 10
            radius: width / 2
            color: "transparent"
            border.width: 2
            border.color: Theme.focusRing
            visible: control.visualFocus
        }
    }

}
