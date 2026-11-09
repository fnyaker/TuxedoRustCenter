// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de la courbe de ventilation éditable : drag avec clamp voisin,
// double-clic d'ajout, clic droit/Suppr, minimum deux points, plafond de
// points et mode lecture seule.

import QtQuick
import QtQuick.Window
import QtTest
import "../../qml/components"

TestCase {
    id: testCase
    name: "FanCurveChart"
    when: windowShown

    Window {
        id: testWindow
        width: 800
        height: 600
        visible: true
    }

    Component {
        id: chartComponent
        FanCurveChart {
            width: 520
            height: 260
        }
    }

    SignalSpy { id: movedSpy; signalName: "pointMoved" }
    SignalSpy { id: addedSpy; signalName: "pointAdded" }
    SignalSpy { id: removedSpy; signalName: "pointRemoved" }

    function points() {
        return [
            { "temp": 30, "speed": 20 },
            { "temp": 60, "speed": 50 },
            { "temp": 90, "speed": 90 }
        ];
    }

    function makeChart(source, editable) {
        var chart = createTemporaryObject(chartComponent, testWindow);
        verify(chart !== null);
        chart.points = source;
        chart.editable = editable;
        movedSpy.target = chart;
        addedSpy.target = chart;
        removedSpy.target = chart;
        movedSpy.clear();
        addedSpy.clear();
        removedSpy.clear();
        wait(0);
        return chart;
    }

    function lastMoved() {
        return movedSpy.signalArguments[movedSpy.count - 1];
    }

    /// Qt 6.12 n'expose pas `mouseDoubleClick` dans Qt Quick Test. On exerce
    /// donc `addPointAt`, le chemin exact appelé par `MouseArea.onDoubleClicked`
    /// (la liaison événementielle reste une ligne de QML).
    function doubleClick(chart, x, y) {
        chart.addPointAt(x, y);
    }

    function test_dragMovesPointWithinGraphBounds() {
        var chart = makeChart(points(), true);
        mousePress(chart, chart.mapX(60), chart.mapY(50));
        mouseMove(chart, chart.mapX(70), chart.mapY(65));
        mouseRelease(chart, chart.mapX(70), chart.mapY(65));

        verify(movedSpy.count > 0, "le drag doit émettre pointMoved");
        var moved = lastMoved();
        compare(moved[0], 1);
        compare(Math.round(moved[1]), 70);
        compare(Math.round(moved[2]), 65);

        // La température reste dans la zone tracée et la vitesse dans 0–100.
        verify(moved[1] >= chart.tempFrom && moved[1] <= chart.tempTo);
        verify(moved[2] >= chart.speedFrom && moved[2] <= chart.speedTo);
    }

    function test_dragClampsToNeighbourAndSpeedBounds() {
        var chart = makeChart(points(), true);

        // Voisin suivant à 90 °C : le point médian ne peut pas dépasser 89 °C.
        mousePress(chart, chart.mapX(60), chart.mapY(50));
        mouseMove(chart, chart.mapX(95), chart.mapY(50));
        mouseRelease(chart, chart.mapX(95), chart.mapY(50));
        compare(lastMoved()[1], 89);

        // Vitesse au-delà du haut du graphe : clamp à 100 %.
        movedSpy.clear();
        mousePress(chart, chart.mapX(60), chart.mapY(50));
        mouseMove(chart, chart.mapX(60), chart.mapY(130));
        mouseRelease(chart, chart.mapX(60), chart.mapY(130));
        compare(lastMoved()[2], 100);

        // Vitesse sous le bas du graphe : clamp à 0 %.
        movedSpy.clear();
        mousePress(chart, chart.mapX(60), chart.mapY(50));
        mouseMove(chart, chart.mapX(60), chart.mapY(-40));
        mouseRelease(chart, chart.mapX(60), chart.mapY(-40));
        compare(lastMoved()[2], 0);
    }

    function test_doubleClickAddsPointAndHonoursMaximum() {
        var chart = makeChart(points(), true);
        doubleClick(chart, chart.mapX(45), chart.mapY(30));
        compare(addedSpy.count, 1);
        var added = addedSpy.signalArguments[0];
        compare(Math.round(added[0]), 45);
        compare(Math.round(added[1]), 30);

        // Plafond : aucun ajout quand maxPoints est atteint.
        addedSpy.clear();
        chart.maxPoints = 3;
        doubleClick(chart, chart.mapX(75), chart.mapY(70));
        compare(addedSpy.count, 0);

        // Un double-clic sur un point existant ne crée pas de doublon.
        addedSpy.clear();
        chart.maxPoints = 256;
        doubleClick(chart, chart.mapX(60), chart.mapY(50));
        compare(addedSpy.count, 0);

        // Plafond réel de 256 points : plus aucun ajout.
        var full = [];
        for (var i = 0; i < 256; ++i)
            full.push({ "temp": 30 + i * (70 / 255), "speed": 20 });
        chart.points = full;
        compare(chart.points.length, 256);
        addedSpy.clear();
        doubleClick(chart, chart.mapX(50), chart.mapY(50));
        compare(addedSpy.count, 0);
    }

    function test_rightClickAndDeleteRemovePointWithMinimumTwo() {
        var chart = makeChart(points(), true);
        mouseClick(chart, chart.mapX(60), chart.mapY(50), Qt.RightButton);
        compare(removedSpy.count, 1);
        compare(removedSpy.signalArguments[0][0], 1);

        // Touche Suppr sur le point actif.
        removedSpy.clear();
        chart.activeIndex = 0;
        testWindow.requestActivate();
        wait(50);
        chart.forceActiveFocus();
        compare(chart.activeFocus, true);
        keyClick(Qt.Key_Delete);
        compare(removedSpy.count, 1);
        compare(removedSpy.signalArguments[0][0], 0);

        // Minimum de deux points : plus aucune suppression.
        removedSpy.clear();
        chart.points = [{ "temp": 30, "speed": 20 },
                        { "temp": 80, "speed": 80 }];
        mouseClick(chart, chart.mapX(30), chart.mapY(20), Qt.RightButton);
        compare(removedSpy.count, 0);
        chart.activeIndex = 0;
        chart.forceActiveFocus();
        keyClick(Qt.Key_Delete);
        compare(removedSpy.count, 0);
    }

    function test_readOnlyChartNeverMoves() {
        var chart = makeChart(points(), false);
        mousePress(chart, chart.mapX(60), chart.mapY(50));
        mouseMove(chart, chart.mapX(75), chart.mapY(70));
        mouseRelease(chart, chart.mapX(75), chart.mapY(70));
        compare(movedSpy.count, 0);
        compare(chart.points[1].temp, 60);

        doubleClick(chart, chart.mapX(45), chart.mapY(30));
        compare(addedSpy.count, 0);
        mouseClick(chart, chart.mapX(60), chart.mapY(50), Qt.RightButton);
        compare(removedSpy.count, 0);
    }

    function test_hoverTooltipAndAccessibility() {
        var chart = makeChart(points(), true);
        compare(chart.maxPoints, 256);
        verify(chart.Accessible.name.length > 0);
        verify(chart.Accessible.description.indexOf("60") >= 0);

        var tip = findChild(chart, "fanCurvePointTooltip");
        verify(tip !== null);
        mouseMove(chart, chart.mapX(60), chart.mapY(50));
        tryCompare(tip, "visible", true);
        verify(tip.text.length > 0);
        verify(tip.text.indexOf("60") >= 0);
    }
}
