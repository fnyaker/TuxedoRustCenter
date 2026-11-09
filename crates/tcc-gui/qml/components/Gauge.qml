// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Jauge circulaire native (Qt Quick Shapes, pas de QtCharts) : arc 270°,
// valeur, unité, icône et légende. Trois états : disponible, indisponible
// (capteur non exposé) et seuil d'alerte. Animations désactivables via
// Theme.reduceMotion.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Shapes
import QtQuick.Window
import "../core"

Item {
    id: root

    property real value: 0
    property real from: 0
    property real to: 100
    property int decimals: 0
    property string unit: ""
    property string label: ""
    property string iconName: ""
    property bool available: true
    property string unavailableReason: ""
    property real warningThreshold: NaN
    property bool animate: true

    implicitWidth: 168
    implicitHeight: 168

    readonly property real targetRatio: {
        if (!available || to === from)
            return 0;
        var r = (value - from) / (to - from);
        return Math.max(0, Math.min(1, r));
    }
    readonly property bool alarming: available && !isNaN(warningThreshold) && value >= warningThreshold
    readonly property color arcColor: !available ? Theme.textMuted
                                    : alarming ? Theme.warning
                                    : Theme.gauge
    readonly property string displayValue: available
        ? Strings.formatNumber(value, decimals)
        : "—"

    property real drawnRatio: 0

    /// Animations coupées hors page visible, fenêtre masquée ou reduceMotion.
    readonly property bool animationActive: root.animate && root.visible
        && !Theme.reduceMotion
        && (!root.Window.window || root.Window.window.visible)

    onTargetRatioChanged: drawnRatio = targetRatio
    Component.onCompleted: drawnRatio = targetRatio

    Behavior on drawnRatio {
        NumberAnimation {
            duration: root.animationActive ? 350 : 0
            easing.type: Easing.OutCubic
        }
    }

    Accessible.role: Accessible.StaticText
    Accessible.name: root.label
    Accessible.description: root.available
        ? root.displayValue + " " + root.unit
        : (root.unavailableReason !== "" ? root.unavailableReason : Strings.t("common.unavailable"))

    HoverHandler { id: hover }

    Shape {
        id: arc
        anchors.fill: parent
        antialiasing: true

        // Piste
        ShapePath {
            strokeColor: Theme.gaugeTrack
            strokeWidth: 7
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap

            PathAngleArc {
                centerX: arc.width / 2
                centerY: arc.height / 2
                radiusX: Math.min(arc.width, arc.height) / 2 - 10
                radiusY: Math.min(arc.width, arc.height) / 2 - 10
                startAngle: 135
                sweepAngle: 270
            }
        }

        // Valeur
        ShapePath {
            strokeColor: root.arcColor
            strokeWidth: 7
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap

            PathAngleArc {
                centerX: arc.width / 2
                centerY: arc.height / 2
                radiusX: Math.min(arc.width, arc.height) / 2 - 10
                radiusY: Math.min(arc.width, arc.height) / 2 - 10
                startAngle: 135
                sweepAngle: 270 * root.drawnRatio
            }
        }
    }

    Column {
        id: centerColumn
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: Math.round(root.height * 0.15)
        spacing: 0

        AppIcon {
            anchors.horizontalCenter: parent.horizontalCenter
            name: root.iconName
            color: root.alarming ? Theme.warning : Theme.textMuted
            size: 17
            visible: root.iconName !== ""
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            topPadding: 4
            text: root.displayValue
            color: root.available ? Theme.textPrimary : Theme.textMuted
            font.family: Theme.fontFamily
            font.pixelSize: 30
            font.weight: Theme.fwSemiBold
            textFormat: Text.PlainText
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.available ? root.unit : ""
            color: Theme.textMuted
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fsCaption
            font.weight: Theme.fwSemiBold
            visible: text !== ""
        }
    }

    Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        width: Math.min(parent.width - 8, legendText.implicitWidth + Theme.s24)
        height: 26
        radius: 13
        color: Theme.surfaceSunken
        border.width: 1
        border.color: Theme.border

        Text {
            id: legendText
            anchors.centerIn: parent
            width: parent.width - Theme.s12
            text: root.label
            color: Theme.textSecondary
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fsCaption
            font.weight: Theme.fwSemiBold
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }
    }

    AppToolTip {
        visible: hover.hovered && !root.available && root.unavailableReason !== ""
        text: root.unavailableReason
        delay: 400
    }
}
