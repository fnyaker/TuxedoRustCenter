// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Liste déroulante native. Popup stylé, navigation clavier complète,
// indicateur chevron dessiné. Fournir `textRole` si le modèle est un tableau
// d'objets.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import "../core"

ComboBox {
    id: control

    implicitHeight: Theme.controlHeight
    implicitWidth: 200
    hoverEnabled: true
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsSmall
    leftPadding: Theme.s12
    rightPadding: 30
    spacing: Theme.s8

    contentItem: Text {
        leftPadding: control.leftPadding
        rightPadding: control.rightPadding
        verticalAlignment: Text.AlignVCenter
        text: control.displayText
        font: control.font
        color: control.enabled ? Theme.textPrimary : Theme.textMuted
        elide: Text.ElideRight
        textFormat: Text.PlainText
    }

    indicator: Glyph {
        x: control.width - width - Theme.s10
        y: control.topPadding + (control.availableHeight - height) / 2
        name: "chevron-down"
        color: control.enabled ? Theme.textSecondary : Theme.textMuted
        size: 14
        strokeWidth: 1.6
    }

    background: Rectangle {
        radius: Theme.radiusSm
        color: control.enabled ? (control.hovered ? Theme.surfaceHover : Theme.surface)
                               : Theme.surfaceSunken
        border.width: 1
        border.color: control.visualFocus ? Theme.accent : Theme.borderStrong

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

    delegate: ItemDelegate {
        id: option
        required property var model
        required property int index

        width: ListView.view ? ListView.view.width : control.width
        implicitHeight: 34
        highlighted: control.highlightedIndex === index
        hoverEnabled: true
        padding: 0

        contentItem: Text {
            leftPadding: Theme.s12
            rightPadding: Theme.s12
            verticalAlignment: Text.AlignVCenter
            text: option.model[control.textRole] !== undefined
                  ? option.model[control.textRole]
                  : (option.model.name !== undefined ? option.model.name : option.model)
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fsSmall
            color: option.highlighted ? Theme.textPrimary : Theme.textSecondary
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        background: Rectangle {
            radius: Theme.radiusXs
            color: option.highlighted ? Theme.surfaceHover : "transparent"
        }
    }

    popup: Popup {
        y: control.height + Theme.s4
        width: control.width
        implicitHeight: Math.min(contentItem.implicitHeight + Theme.s8, 320)
        padding: Theme.s4
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: control.popup.visible ? control.delegateModel : null
            currentIndex: control.highlightedIndex
            boundsBehavior: Flickable.StopAtBounds
            ScrollIndicator.vertical: ScrollIndicator { }
        }

        background: Rectangle {
            radius: Theme.radiusMd
            color: Theme.surface
            border.width: 1
            border.color: Theme.borderStrong
        }
    }
}
