// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Contrôle segmenté (filtres, thème, unités). Navigation clavier : flèches
// gauche/droite, origine sur l'élément actif. Rôle accessible RadioButton.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property var options: []            // [{ value, label, icon? }]
    property int currentIndex: 0
    property bool compact: false
    signal activated(int index)

    implicitHeight: compact ? 30 : 34
    implicitWidth: trackLayout.implicitWidth + 6

    function select(index) {
        if (index < 0 || index >= options.length || index === currentIndex)
            return;
        currentIndex = index;
        activated(index);
    }

    function move(delta) {
        var next = currentIndex + delta;
        if (next >= 0 && next < options.length)
            select(next);
    }

    Rectangle {
        id: track
        anchors.fill: parent
        radius: Theme.radiusSm
        color: Theme.surfaceSunken
        border.width: 1
        border.color: Theme.border

        RowLayout {
            id: trackLayout
            anchors.fill: parent
            anchors.margins: 3
            spacing: 2

            Repeater {
                model: root.options

                delegate: AbstractButton {
                    id: segment
                    required property var modelData
                    required property int index

                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(56, label.implicitWidth + Theme.s16)
                    hoverEnabled: true
                    Accessible.role: Accessible.RadioButton
                    Accessible.name: modelData.label
                    Accessible.checked: root.currentIndex === index
                    Accessible.onPressAction: root.select(index)

                    onClicked: {
                        root.currentIndex = index;
                        root.activated(index);
                    }

                    background: Rectangle {
                        radius: Theme.radiusSm - 2
                        color: root.currentIndex === segment.index ? Theme.surface
                             : segment.hovered ? Theme.surfaceHover : "transparent"
                        border.width: root.currentIndex === segment.index ? 1 : 0
                        border.color: Theme.border
                        Behavior on color {
                            ColorAnimation { duration: Theme.durationFast; easing.type: Theme.easeOut }
                        }
                    }

                    contentItem: RowLayout {
                        spacing: Theme.s6
                        anchors.centerIn: parent

                        AppIcon {
                            visible: segment.modelData.icon !== undefined
                            name: segment.modelData.icon === undefined ? "" : segment.modelData.icon
                            color: root.currentIndex === segment.index ? Theme.accentText : Theme.textSecondary
                            size: 14
                        }

                        Text {
                            id: label
                            text: segment.modelData.label
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fsSmall
                            font.weight: root.currentIndex === segment.index ? Theme.fwSemiBold : Theme.fwRegular
                            color: root.currentIndex === segment.index ? Theme.textPrimary : Theme.textSecondary
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -1
                        radius: Theme.radiusSm
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.focusRing
                        visible: segment.visualFocus
                    }

                    Keys.onLeftPressed: root.move(-1)
                    Keys.onRightPressed: root.move(1)
                }
            }
        }
    }
}
