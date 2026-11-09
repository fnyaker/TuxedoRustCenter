// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Infobulle native, contraste inversé par rapport à la surface.
// Sert aux capteurs indisponibles, aux icônes seules et aux aides courtes.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

ToolTip {
    id: control

    delay: 500
    timeout: 6000
    padding: Theme.s8
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsCaption
    closePolicy: Popup.CloseOnEscape

    contentItem: Text {
        text: control.text
        color: Theme.dark ? Theme.textOnLight : Theme.textPrimary
        font: control.font
        wrapMode: Text.Wrap
        width: Math.min(implicitWidth, 320)
        textFormat: Text.PlainText
    }

    background: Rectangle {
        radius: Theme.radiusSm
        color: Theme.dark ? "#e9e7e7" : "#1f1f23"
        border.width: 1
        border.color: Theme.dark ? "#ffffff26" : "#ffffff14"
    }
}
