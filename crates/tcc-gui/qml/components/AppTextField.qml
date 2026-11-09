// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Champ de saisie natif avec état d'erreur explicite et anneau de focus.
// Les champs de mot de passe ne doivent jamais être journalisés ni conservés
// au-delà de la soumission (voir docs/ui/UI_DESIGN.md, section sécurité).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

TextField {
    id: control

    property bool invalid: false
    property string errorText: ""

    implicitHeight: Theme.controlHeight
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsBody
    color: Theme.textPrimary
    placeholderTextColor: Theme.textMuted
    selectionColor: Theme.accent
    selectedTextColor: Theme.textOnAccent
    leftPadding: Theme.s12
    rightPadding: Theme.s12
    topPadding: 0
    bottomPadding: 0
    verticalAlignment: TextInput.AlignVCenter
    selectByMouse: true
    focusPolicy: Qt.StrongFocus
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
