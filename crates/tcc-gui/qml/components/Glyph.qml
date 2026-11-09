// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Petits glyphes d'interface dessinés nativement (aucune police d'icônes,
// aucun asset externe pour les chevrons, croix, cases à cocher…).
// Formes vectorielles simples, nettes à toutes les échelles.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import QtQuick.Shapes

Item {
    id: glyphRoot

    // chevron-down | chevron-up | chevron-left | chevron-right |
    // check | plus | minus | close | kebab | search | dot
    property string name: "chevron-down"
    property color color: Theme.textSecondary
    property real size: 16
    property real strokeWidth: Math.max(1.5, size / 9)

    implicitWidth: size
    implicitHeight: size
    Accessible.ignored: true

    // ── Chevron ─────────────────────────────────────────────────────────────
    Shape {
        id: chevron
        anchors.fill: parent
        visible: glyphRoot.name.indexOf("chevron") === 0
        // Rotation autour du centre selon la direction.
        rotation: glyphRoot.name === "chevron-left" ? 90
                : glyphRoot.name === "chevron-up" ? 180
                : glyphRoot.name === "chevron-right" ? 270 : 0
        transformOrigin: Item.Center
        layer.enabled: false

        ShapePath {
            strokeColor: glyphRoot.color
            strokeWidth: glyphRoot.strokeWidth
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            startX: glyphRoot.size * 0.26
            startY: glyphRoot.size * 0.38
            PathLine { x: glyphRoot.size * 0.5; y: glyphRoot.size * 0.62 }
            PathLine { x: glyphRoot.size * 0.74; y: glyphRoot.size * 0.38 }
        }
    }

    // ── Coche ───────────────────────────────────────────────────────────────
    Shape {
        anchors.fill: parent
        visible: glyphRoot.name === "check"
        ShapePath {
            strokeColor: glyphRoot.color
            strokeWidth: glyphRoot.strokeWidth
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            startX: glyphRoot.size * 0.22
            startY: glyphRoot.size * 0.52
            PathLine { x: glyphRoot.size * 0.42; y: glyphRoot.size * 0.72 }
            PathLine { x: glyphRoot.size * 0.79; y: glyphRoot.size * 0.28 }
        }
    }

    // ── Plus / moins ────────────────────────────────────────────────────────
    Rectangle {
        visible: glyphRoot.name === "plus" || glyphRoot.name === "minus"
        anchors.centerIn: parent
        width: glyphRoot.size * 0.62
        height: glyphRoot.strokeWidth
        radius: height / 2
        color: glyphRoot.color
    }

    Rectangle {
        visible: glyphRoot.name === "plus"
        anchors.centerIn: parent
        width: glyphRoot.strokeWidth
        height: glyphRoot.size * 0.62
        radius: width / 2
        color: glyphRoot.color
    }

    // ── Croix ───────────────────────────────────────────────────────────────
    Item {
        visible: glyphRoot.name === "close"
        anchors.fill: parent

        Rectangle {
            anchors.centerIn: parent
            width: glyphRoot.size * 0.66
            height: glyphRoot.strokeWidth
            radius: height / 2
            color: glyphRoot.color
            rotation: 45
        }

        Rectangle {
            anchors.centerIn: parent
            width: glyphRoot.size * 0.66
            height: glyphRoot.strokeWidth
            radius: height / 2
            color: glyphRoot.color
            rotation: -45
        }
    }

    // ── Menu « kebab » ─────────────────────────────────────────────────────
    Column {
        visible: glyphRoot.name === "kebab"
        anchors.centerIn: parent
        spacing: glyphRoot.size * 0.14

        Repeater {
            model: 3
            delegate: Rectangle {
                required property int index
                width: Math.max(2.5, glyphRoot.size * 0.16)
                height: width
                radius: width / 2
                color: glyphRoot.color
            }
        }
    }

    // ── Loupe ───────────────────────────────────────────────────────────────
    Item {
        visible: glyphRoot.name === "search"
        anchors.fill: parent

        Rectangle {
            x: glyphRoot.size * 0.12
            y: glyphRoot.size * 0.12
            width: glyphRoot.size * 0.5
            height: glyphRoot.size * 0.5
            radius: width / 2
            color: "transparent"
            border.width: glyphRoot.strokeWidth
            border.color: glyphRoot.color
        }

        Rectangle {
            x: glyphRoot.size * 0.58
            y: glyphRoot.size * 0.68
            width: glyphRoot.size * 0.3
            height: glyphRoot.strokeWidth
            radius: height / 2
            color: glyphRoot.color
            rotation: 45
        }
    }

    // ── Point d'état ───────────────────────────────────────────────────────
    Rectangle {
        visible: glyphRoot.name === "dot"
        anchors.centerIn: parent
        width: glyphRoot.size * 0.4
        height: width
        radius: width / 2
        color: glyphRoot.color
    }
}
