// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Zone de texte multiligne native (description de profil, notes). Même langage
// visuel que AppTextField ; focus clavier géré via activeFocus (TextArea
// n'expose pas visualFocus, comme TextField).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

TextArea {
    id: control

    property bool invalid: false
    property string errorText: ""

    implicitHeight: 84
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsBody
    color: Theme.textPrimary
    placeholderTextColor: Theme.textMuted
    selectionColor: Theme.accent
    selectedTextColor: Theme.textOnAccent
    leftPadding: Theme.s12
    rightPadding: Theme.s12
    topPadding: Theme.s8
    bottomPadding: Theme.s8
    wrapMode: TextEdit.Wrap
    selectByMouse: true
    Accessible.description: errorText

    background: Rectangle {
        radius: Theme.radiusSm
        color: control.enabled ? Theme.surface : Theme.surfaceSunken
        border.width: 1
        border.color: control.invalid ? Theme.error
                    : control.activeFocus ? Theme.accent
                    : Theme.borderStrong

        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: parent.radius + 3
            color: "transparent"
            border.width: 2
            border.color: control.invalid ? Theme.error : Theme.focusRing
            visible: control.activeFocus
        }
    }
}
