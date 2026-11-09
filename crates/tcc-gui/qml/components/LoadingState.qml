// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Indicateur d'activité natif (arc rotatif, pas de GIF ni de vue web).
// Respecte la préférence de réduction des animations : dans ce cas, seule la
// couleur et le texte informent.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Shapes
import QtQuick.Window
import "../core"

Item {
    id: root

    property string label: ""
    property int size: 28

    implicitHeight: content.implicitHeight
    implicitWidth: content.implicitWidth

    ColumnLayout {
        id: content
        anchors.centerIn: parent
        spacing: Theme.s8

        Item {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: root.size
            Layout.preferredHeight: root.size

            Shape {
                anchors.fill: parent
                antialiasing: true

                ShapePath {
                    strokeColor: Theme.gaugeTrack
                    strokeWidth: 3
                    fillColor: "transparent"
                    capStyle: ShapePath.RoundCap
                    PathAngleArc {
                        centerX: root.size / 2
                        centerY: root.size / 2
                        radiusX: root.size / 2 - 3
                        radiusY: root.size / 2 - 3
                        startAngle: 0
                        sweepAngle: 360
                    }
                }

                ShapePath {
                    strokeColor: Theme.accent
                    strokeWidth: 3
                    fillColor: "transparent"
                    capStyle: ShapePath.RoundCap
                    PathAngleArc {
                        centerX: root.size / 2
                        centerY: root.size / 2
                        radiusX: root.size / 2 - 3
                        radiusY: root.size / 2 - 3
                        startAngle: 0
                        sweepAngle: 90
                    }
                }
            }

            RotationAnimator on rotation {
                // Ne pas animer un spinner caché (toutes les pages sont
                // instanciées dans le StackLayout).
                running: root.visible && !Theme.reduceMotion
                         && (!root.Window.window || root.Window.window.visible)
                loops: Animation.Infinite
                from: 0
                to: 360
                duration: 900
            }
        }

        AppLabel {
            Layout.alignment: Qt.AlignHCenter
            visible: root.label !== ""
            variant: "small"
            secondary: true
            text: root.label
        }
    }

    Accessible.role: Accessible.ProgressBar
    Accessible.name: root.label !== "" ? root.label : Strings.t("common.loading")
}
