// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne de réglage numérique : libellé, valeur formatée localement, curseur et
// boutons -/+ (cibles 28 px) pour un ajustement fin au clavier ou à la souris.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string label: ""
    property string description: ""
    property real from: 0
    property real to: 100
    property real value: 0
    property real stepSize: 1
    property int decimals: 0
    property string unit: ""
    property string valueText: ""
    property bool supported: true
    property string unsupportedReason: ""

    signal moved(real value)
    signal committed(real value)

    implicitHeight: column.implicitHeight

    function _display() {
        if (valueText !== "")
            return valueText;
        var s = Strings.formatNumber(value, decimals);
        return unit !== "" ? s + " " + unit : s;
    }

    function _step(direction) {
        var next = Math.max(root.from, Math.min(root.to, root.value + direction * root.stepSize));
        next = Math.round(next / root.stepSize) * root.stepSize;
        root.value = next;
        root.moved(next);
        root.committed(next);
    }

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.s6

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s12

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 1

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: root.label
                    color: root.supported ? Theme.textPrimary : Theme.textMuted
                }

                AppLabel {
                    Layout.fillWidth: true
                    visible: root.description !== ""
                    variant: "small"
                    secondary: true
                    text: root.description
                }
            }

            AppLabel {
                Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
                variant: "body"
                strong: true
                text: root.supported ? root._display() : "—"
                color: root.supported ? Theme.textPrimary : Theme.textMuted
            }

            StatusBadge {
                visible: !root.supported
                kind: "neutral"
                label: root.unsupportedReason !== "" ? root.unsupportedReason
                                                     : Strings.t("common.unavailable")
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: root.supported

            IconButton {
                id: minusButton
                iconName: ""
                implicitWidth: 26
                implicitHeight: 26
                enabled: root.value > root.from
                Accessible.name: Strings.t("common.reset")
                background: Rectangle {
                    radius: Theme.radiusXs
                    color: minusButton.hovered ? Theme.surfaceHover : Theme.surfaceSunken
                }
                contentItem: Glyph {
                    anchors.centerIn: parent
                    name: "minus"
                    color: parent.enabled ? Theme.textSecondary : Theme.textMuted
                    size: 12
                }
                onClicked: root._step(-1)
            }

            AppSlider {
                Layout.fillWidth: true
                from: root.from
                to: root.to
                stepSize: root.stepSize
                value: root.value
                Accessible.name: root.label
                onMoved: root.moved(value)
                onPressedChanged: if (!pressed) root.committed(value)
            }

            IconButton {
                id: plusButton
                iconName: ""
                implicitWidth: 26
                implicitHeight: 26
                enabled: root.value < root.to
                Accessible.name: root.label
                background: Rectangle {
                    radius: Theme.radiusXs
                    color: plusButton.hovered ? Theme.surfaceHover : Theme.surfaceSunken
                }
                contentItem: Glyph {
                    anchors.centerIn: parent
                    name: "plus"
                    color: parent.enabled ? Theme.textSecondary : Theme.textMuted
                    size: 12
                }
                onClicked: root._step(1)
            }
        }
    }
}
