// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Bouton du design system. Variantes : primary (accent, action unique),
// secondary (bordure), ghost (neutre), danger (destructif), link.
// L'accent rouge reste rare : une seule action primaire par vue.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import QtQuick.Controls

Button {
    id: control

    property string variant: "primary"
    property string iconName: ""
    property bool monochromeIcon: true
    property bool small: false

    readonly property color foreground: !enabled ? Theme.textMuted
                                     : variant === "primary" ? Theme.textOnAccent
                                     : variant === "danger" ? Theme.error
                                     : variant === "link" ? Theme.accentText
                                     : Theme.textPrimary
    readonly property color surfaceColor: {
        if (!enabled)
            return variant === "primary" ? Theme.textMuted : "transparent";
        if (variant === "primary")
            return pressed ? Theme.accentPressed : (hovered ? Theme.accentHover : Theme.accent);
        if (variant === "secondary")
            return pressed ? Theme.surfaceSunken : (hovered ? Theme.surfaceHover : Theme.surface);
        if (variant === "danger")
            return pressed ? Theme.error : (hovered ? Theme.errorSoft : "transparent");
        if (variant === "ghost" || variant === "link")
            return pressed ? Theme.surfaceSunken : (hovered ? Theme.surfaceHover : "transparent");
        return Theme.surface;
    }

    implicitHeight: small ? Theme.controlHeightSmall : Theme.controlHeight
    implicitWidth: Math.max(implicitContentWidth + leftPadding + rightPadding, implicitHeight)
    leftPadding: small ? Theme.s10 : 14
    rightPadding: small ? Theme.s10 : 14
    topPadding: 0
    bottomPadding: 0
    hoverEnabled: true
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsSmall
    font.weight: Theme.fwSemiBold
    opacity: enabled ? 1 : 0.8

    background: Rectangle {
        radius: Theme.radiusSm
        color: control.surfaceColor
        border.width: control.variant === "secondary" || (control.variant === "danger" && !control.hovered) ? 1 : 0
        border.color: control.variant === "danger" ? Theme.error : Theme.borderStrong
        Behavior on color {
            ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
        }

        // Anneau de focus visible au clavier uniquement.
        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: parent.radius + 3
            color: "transparent"
            border.width: 2
            border.color: Theme.focusRing
            visible: control.visualFocus
        }
    }

    contentItem: Row {
        spacing: Theme.s8
        anchors.centerIn: parent

        AppIcon {
            anchors.verticalCenter: parent.verticalCenter
            visible: control.iconName !== ""
            name: control.iconName
            monochrome: control.monochromeIcon
            color: control.foreground
            size: control.small ? 14 : 16
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: control.text
            color: control.foreground
            font: control.font
            visible: control.text !== ""
            elide: Text.ElideRight
        }
    }
}
