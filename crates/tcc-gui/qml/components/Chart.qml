// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Courbe d'historique native (Qt Quick Shapes + PathSvg). Aucun QtCharts,
// aucune vue web. Le tracé est une fonction pure de `values` et de la taille :
// il n'est recalculé que lorsque ces entrées changent. Utilisable en sparkline
// (showGrid: false) ou en graphe avec grille.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Shapes
import "../core"

Item {
    id: root

    property var values: []
    property real minValue: 0
    property real maxValue: 100
    property bool autoScale: true
    property color strokeColor: Theme.accentText
    property bool showArea: true
    property bool showGrid: true
    property bool showLastPoint: true
    property bool animate: false

    implicitHeight: 90

    // Bornes : échelle fixe par défaut, ou ajustée avec 15 % de marge.
    readonly property var bounds: {
        var lo = minValue;
        var hi = maxValue;
        if (autoScale && values.length > 1) {
            var dataLo = Math.min.apply(null, values);
            var dataHi = Math.max.apply(null, values);
            var pad = Math.max((dataHi - dataLo) * 0.18, 0.8);
            lo = dataLo - pad;
            hi = dataHi + pad;
        }
        if (hi <= lo)
            hi = lo + 1;
        return { "lo": lo, "hi": hi };
    }

    readonly property string linePath: {
        if (values.length < 2)
            return "";
        var w = width;
        var h = height;
        var lo = bounds.lo;
        var hi = bounds.hi;
        var n = values.length;
        var d = "";
        for (var i = 0; i < n; ++i) {
            var x = (i / (n - 1)) * w;
            var y = h - ((values[i] - lo) / (hi - lo)) * h;
            y = Math.max(1, Math.min(h - 1, y));
            d += (i === 0 ? "M" : " L") + x.toFixed(2) + " " + y.toFixed(2);
        }
        return d;
    }

    readonly property string areaPath: {
        if (linePath === "")
            return "";
        return linePath + " L " + width.toFixed(2) + " " + height.toFixed(2)
                       + " L 0 " + height.toFixed(2) + " Z";
    }

    readonly property real lastY: {
        if (values.length === 0)
            return height / 2;
        var y = height - ((values[values.length - 1] - bounds.lo) / (bounds.hi - bounds.lo)) * height;
        return Math.max(1, Math.min(height - 1, y));
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusSm
        color: "transparent"

        Repeater {
            model: root.showGrid ? 3 : 0
            delegate: Rectangle {
                required property int index
                x: 0
                y: (root.height - 1) * index / 2
                width: root.width
                height: 1
                color: Theme.divider
            }
        }
    }

    Shape {
        anchors.fill: parent
        antialiasing: true
        visible: root.linePath !== ""

        ShapePath {
            strokeColor: "transparent"
            fillColor: "transparent"
            fillGradient: LinearGradient {
                x1: 0; y1: 0; x2: 0; y2: root.height
                GradientStop { position: 0.0; color: Qt.rgba(root.strokeColor.r, root.strokeColor.g, root.strokeColor.b, root.showArea ? 0.22 : 0.0) }
                GradientStop { position: 1.0; color: Qt.rgba(root.strokeColor.r, root.strokeColor.g, root.strokeColor.b, root.showArea ? 0.01 : 0.0) }
            }
            PathSvg { path: root.areaPath }
        }

        ShapePath {
            strokeColor: root.strokeColor
            strokeWidth: 2
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: root.linePath }
        }
    }

    Rectangle {
        visible: root.showLastPoint && root.linePath !== ""
        width: 6
        height: 6
        radius: 3
        x: root.width - 3
        y: root.lastY - 3
        color: root.strokeColor
    }
}
