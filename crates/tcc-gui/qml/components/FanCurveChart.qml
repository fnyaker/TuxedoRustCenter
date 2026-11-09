// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Courbe de ventilateur température → vitesse. Tracé natif (Shapes/PathSvg),
// axes gradués et points de consigne.
//
// Lecture seule par défaut (`editable: false`) : c'est l'aperçu du Dashboard
// et de l'éditeur. Quand `editable` est vrai (`FanCurveEditor`), les points se
// déplacent à la souris, un double-clic ajoute un point et le clic droit ou la
// touche Suppr retire le point actif. Le composant ne modifie jamais `points` :
// il émet un signal et le parent décide de la mise à jour (le tableau et
// l'aperçu restent ainsi la seule source de vérité).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Shapes
import "../core"

Item {
    id: root

    property var points: []          // [{ temp, speed }]
    property real tempFrom: 30
    property real tempTo: 100
    property real speedFrom: 0
    property real speedTo: 100
    property color curveColor: Theme.gauge

    /// Autorise le déplacement, l'ajout et la suppression de points.
    property bool editable: false
    /// Nombre maximal de points (même borne que `tcc-core`).
    property int maxPoints: 256
    /// Point sélectionné au clavier (Suppr) et mis en évidence.
    property int activeIndex: -1
    /// Point survolé (infobulle + curseur main).
    property int hoverIndex: -1

    readonly property int gutterLeft: 36
    readonly property int gutterBottom: 24
    readonly property int gutterTop: 6
    readonly property int gutterRight: 10
    /// Rayon de saisie généreux : les marqueurs de 10 px restent faciles à viser.
    readonly property real pointHitRadius: 18

    implicitHeight: 220

    signal pointMoved(int index, real temp, real speed)
    signal pointAdded(real temp, real speed)
    signal pointRemoved(int index)

    function mapX(temp) {
        var span = tempTo - tempFrom;
        if (span <= 0)
            return gutterLeft;
        return gutterLeft + ((temp - tempFrom) / span) * (width - gutterLeft - gutterRight);
    }

    function mapY(speed) {
        var span = speedTo - speedFrom;
        if (span <= 0)
            return height - gutterBottom;
        return height - gutterBottom - ((speed - speedFrom) / span) * (height - gutterBottom - gutterTop);
    }

    function clamp(value, low, high) {
        return Math.max(low, Math.min(high, value));
    }

    function tempAt(x) {
        var plotWidth = width - gutterLeft - gutterRight;
        if (plotWidth <= 0)
            return tempFrom;
        return clamp(tempFrom + ((x - gutterLeft) / plotWidth) * (tempTo - tempFrom),
                     tempFrom, tempTo);
    }

    function speedAt(y) {
        var plotHeight = height - gutterBottom - gutterTop;
        if (plotHeight <= 0)
            return speedFrom;
        return clamp(speedTo - ((y - gutterTop) / plotHeight) * (speedTo - speedFrom),
                     speedFrom, speedTo);
    }

    /// Index du point le plus proche dans le rayon de saisie, sinon -1.
    function pointAt(x, y) {
        var best = -1;
        var bestDistance = pointHitRadius * pointHitRadius;
        for (var i = 0; i < points.length; ++i) {
            var dx = mapX(points[i].temp) - x;
            var dy = mapY(points[i].speed) - y;
            var distance = dx * dx + dy * dy;
            if (distance <= bestDistance) {
                bestDistance = distance;
                best = i;
            }
        }
        return best;
    }

    /// Un point peut-il être inséré à cette température (écart ≥ 1 °C) ?
    function canInsert(temp) {
        if (points.length >= maxPoints)
            return false;
        if (temp < tempFrom || temp > tempTo)
            return false;
        for (var i = 0; i < points.length; ++i) {
            var existing = Number(points[i].temp);
            if (!isFinite(existing))
                continue;
            if (Math.abs(existing - temp) < 1)
                return false;
        }
        return true;
    }

    /// Déplace un point en respectant les bornes et l'ordre strict.
    function dragTo(index, x, y) {
        if (index < 0 || index >= points.length)
            return;
        var previous = index > 0 ? Number(points[index - 1].temp) : tempFrom;
        var next = index < points.length - 1 ? Number(points[index + 1].temp) : tempTo;
        if (!isFinite(previous))
            previous = tempFrom;
        if (!isFinite(next))
            next = tempTo;
        var low = Math.max(tempFrom, previous + 1);
        var high = Math.min(tempTo, next - 1);
        if (low > high)
            return;
        var temp = Math.round(clamp(tempAt(x), low, high) * 10) / 10;
        var speed = Math.round(clamp(speedAt(y), speedFrom, speedTo) * 10) / 10;
        if (temp === Number(points[index].temp) && speed === Number(points[index].speed))
            return;
        pointMoved(index, temp, speed);
    }

    function removePointAt(index) {
        if (!editable || points.length <= 2 || index < 0 || index >= points.length)
            return;
        pointRemoved(index);
        activeIndex = -1;
        hoverIndex = -1;
    }

    function removeActivePoint() {
        if (activeIndex >= 0)
            removePointAt(activeIndex);
    }

    /// Ajout à une position du graphe (double-clic). Refusé au-delà du
    /// plafond ou si l'écart avec un voisin est inférieur à 1 °C.
    function addPointAt(x, y) {
        if (!editable || points.length >= maxPoints)
            return;
        var temp = tempAt(x);
        var speed = speedAt(y);
        if (!canInsert(temp))
            return;
        pointAdded(Math.round(temp * 10) / 10, Math.round(speed * 10) / 10);
    }

    readonly property string curvePath: {
        if (points.length < 2)
            return "";
        var d = "";
        for (var i = 0; i < points.length; ++i) {
            d += (i === 0 ? "M" : " L") + mapX(points[i].temp).toFixed(2)
                 + " " + mapY(points[i].speed).toFixed(2);
        }
        return d;
    }

    readonly property string areaPath: curvePath === "" ? ""
        : curvePath + " L " + mapX(points[points.length - 1].temp).toFixed(2) + " " + (height - gutterBottom).toFixed(2)
                    + " L " + mapX(points[0].temp).toFixed(2) + " " + (height - gutterBottom).toFixed(2) + " Z";

    onEditableChanged: {
        if (!editable) {
            activeIndex = -1;
            hoverIndex = -1;
        }
    }

    Accessible.role: Accessible.StaticText
    Accessible.name: Strings.t("fan.curve")
    Accessible.description: {
        var parts = [];
        for (var i = 0; i < points.length; ++i)
            parts.push(points[i].temp + " °C → " + points[i].speed + " %");
        return parts.join(", ");
    }

    // Grille horizontale + libellés de vitesse
    Repeater {
        model: [0, 25, 50, 75, 100]

        delegate: Item {
            id: speedRow
            required property var modelData
            width: root.width
            height: 1
            y: root.mapY(modelData)

            Rectangle {
                x: root.gutterLeft
                width: root.width - root.gutterLeft - root.gutterRight
                height: 1
                color: Theme.divider
            }

            Text {
                x: 0
                width: root.gutterLeft - 6
                y: -8
                text: speedRow.modelData + " %"
                horizontalAlignment: Text.AlignRight
                color: Theme.textMuted
                font.family: Theme.fontFamily
                font.pixelSize: 10
                textFormat: Text.PlainText
            }
        }
    }

    // Grille verticale + libellés de température
    Repeater {
        model: [30, 50, 70, 90]

        delegate: Item {
            id: tempRow
            required property var modelData
            width: 1
            height: root.height - root.gutterBottom - root.gutterTop
            x: root.mapX(modelData)
            y: root.gutterTop

            Rectangle {
                width: 1
                height: parent.height
                color: Theme.divider
            }

            Text {
                y: parent.height + 4
                x: -12
                width: 24
                text: tempRow.modelData + "°"
                horizontalAlignment: Text.AlignHCenter
                color: Theme.textMuted
                font.family: Theme.fontFamily
                font.pixelSize: 10
                textFormat: Text.PlainText
            }
        }
    }

    Shape {
        anchors.fill: parent
        antialiasing: true
        visible: root.curvePath !== ""

        ShapePath {
            strokeColor: "transparent"
            fillColor: "transparent"
            fillGradient: LinearGradient {
                x1: 0; y1: 0; x2: 0; y2: root.height
                GradientStop { position: 0.0; color: Qt.rgba(root.curveColor.r, root.curveColor.g, root.curveColor.b, 0.18) }
                GradientStop { position: 1.0; color: Qt.rgba(root.curveColor.r, root.curveColor.g, root.curveColor.b, 0.0) }
            }
            PathSvg { path: root.areaPath }
        }

        ShapePath {
            strokeColor: root.curveColor
            strokeWidth: 2.5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: root.curvePath }
        }
    }

    // Anneau de focus : le graphe éditable se pilote aussi au clavier (Suppr).
    Rectangle {
        anchors.fill: parent
        color: "transparent"
        radius: Theme.radiusSm
        border.width: 2
        border.color: Theme.focusRing
        visible: root.editable && (root.activeFocus || interaction.activeFocus)
    }

    // Points de consigne
    Repeater {
        model: root.points.length

        delegate: Rectangle {
            id: pointDot
            required property int index
            readonly property var point: root.points[index]
            readonly property bool active: root.editable && root.activeIndex === index
            readonly property bool hovered: root.editable && root.hoverIndex === index
            width: active ? 15 : (hovered ? 13 : 10)
            height: width
            radius: width / 2
            x: root.mapX(point.temp) - width / 2
            y: root.mapY(point.speed) - height / 2
            color: active ? root.curveColor : Theme.surface
            border.width: active ? 3 : 2
            border.color: active ? Theme.surface : root.curveColor
            z: 2
        }
    }

    // Infobulle du point survolé/déplacé.
    AppToolTip {
        id: pointTip
        objectName: "fanCurvePointTooltip"
        parent: root
        delay: 0
        timeout: -1
        visible: root.editable && root.hoverIndex >= 0
                 && root.hoverIndex < root.points.length
        text: pointTip.visible
              ? Strings.tf("fanCurve.pointTooltip", [
                    Strings.formatNumber(root.points[root.hoverIndex].temp, 1),
                    Strings.formatNumber(root.points[root.hoverIndex].speed, 0)])
              : ""
        x: pointTip.visible
           ? Math.round(Math.min(root.width - pointTip.width,
                                 Math.max(0, root.mapX(root.points[root.hoverIndex].temp)
                                             - pointTip.width / 2)))
           : 0
        y: pointTip.visible
           ? Math.max(0, Math.round(root.mapY(root.points[root.hoverIndex].speed)
                                    - pointTip.height - 12))
           : 0
    }

    Keys.onPressed: function(event) {
        if (!root.editable)
            return;
        if (event.key === Qt.Key_Delete) {
            root.removeActivePoint();
            event.accepted = true;
        }
    }

    MouseArea {
        id: interaction
        anchors.fill: parent
        enabled: root.editable
        hoverEnabled: root.editable
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        cursorShape: root.hoverIndex >= 0 ? Qt.PointingHandCursor : Qt.ArrowCursor
        activeFocusOnTab: root.editable
        focus: false

        function select(index) {
            root.activeIndex = index;
            root.hoverIndex = index;
            if (index >= 0)
                interaction.forceActiveFocus();
        }

        onPressed: function(mouse) {
            if (!root.editable)
                return;
            var index = root.pointAt(mouse.x, mouse.y);
            if (mouse.button === Qt.RightButton) {
                root.removePointAt(index);
                return;
            }
            interaction.select(index);
        }

        onPositionChanged: function(mouse) {
            if (!root.editable)
                return;
            var index = root.pointAt(mouse.x, mouse.y);
            root.hoverIndex = root.activeIndex >= 0 && interaction.pressed
                    ? root.activeIndex : index;
            if (!interaction.pressed || root.activeIndex < 0)
                return;
            root.dragTo(root.activeIndex, mouse.x, mouse.y);
        }

        onReleased: function(mouse) {
            root.hoverIndex = root.pointAt(mouse.x, mouse.y);
        }

        onExited: {
            if (!interaction.pressed)
                root.hoverIndex = -1;
        }

        onDoubleClicked: function(mouse) {
            if (!root.editable || mouse.button !== Qt.LeftButton)
                return;
            // Double-clic sur un point existant : pas d'ajout (le drag a la main).
            if (root.pointAt(mouse.x, mouse.y) >= 0)
                return;
            root.addPointAt(mouse.x, mouse.y);
        }

        onCanceled: {
            root.activeIndex = -1;
            root.hoverIndex = -1;
        }
    }
}
