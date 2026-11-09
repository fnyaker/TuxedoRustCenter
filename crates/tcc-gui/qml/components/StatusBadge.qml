// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Pastille d'état : allure, couleur et libellé. `kind` porte la sémantique,
// jamais la couleur seule (accessibilité daltonisme) : un point/icône ET un
// texte explicite sont toujours affichés.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    // neutral | accent | ok | warn | error | info
    property string kind: "neutral"
    property string label: ""
    property string iconName: ""
    property bool showDot: true
    property bool compact: true

    readonly property color toneColor: kind === "ok" ? Theme.success
                                   : kind === "warn" ? Theme.warning
                                   : kind === "error" ? Theme.error
                                   : kind === "info" ? Theme.info
                                   : kind === "accent" ? Theme.accentText
                                   : Theme.textSecondary
    readonly property color toneBackground: kind === "ok" ? Theme.successSoft
                                   : kind === "warn" ? Theme.warningSoft
                                   : kind === "error" ? Theme.errorSoft
                                   : kind === "info" ? Theme.infoSoft
                                   : kind === "accent" ? Theme.accentSoft
                                   : Theme.surfaceSunken

    implicitHeight: compact ? 22 : 26
    implicitWidth: badgeRow.implicitWidth + (compact ? Theme.s8 * 2 : Theme.s12 * 2)
    radius: Theme.radiusPill
    color: toneBackground
    border.width: 1
    border.color: kind === "neutral" ? Theme.border : "transparent"

    RowLayout {
        id: badgeRow
        anchors.centerIn: parent
        spacing: Theme.s6

        Rectangle {
            visible: root.iconName === "" && root.showDot && root.label !== ""
            Layout.preferredWidth: 6
            Layout.preferredHeight: 6
            radius: 3
            color: root.toneColor
        }

        AppIcon {
            visible: root.iconName !== ""
            name: root.iconName
            color: root.toneColor
            size: root.compact ? 11 : 13
        }

        Text {
            text: root.label
            color: root.toneColor
            font.family: Theme.fontFamily
            font.pixelSize: root.compact ? Theme.fsCaption : Theme.fsSmall
            font.weight: Theme.fwSemiBold
            visible: text !== ""
        }
    }
}
