// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests du dialogue de courbe de ventilation : chargement, ajout/suppression,
// validation, aperçu et application.

import QtQuick
import QtTest
import "../../qml/components"

TestCase {
    id: testCase
    name: "FanCurveEditor"
    when: windowShown

    Component {
        id: editorComponent
        FanCurveEditor { }
    }

    function points() {
        return [
            { "temp": 30, "speed": 20 },
            { "temp": 60, "speed": 50 },
            { "temp": 90, "speed": 90 }
        ];
    }

    function test_openAddRemoveApply() {
        var editor = createTemporaryObject(editorComponent, testCase);
        verify(editor !== null);
        editor.openWith(points());
        compare(editor.previewPoints.length, 3);
        compare(editor.errorKey, "");
        compare(editor.previewPoints[1].temp, 60);

        editor.addPoint();
        compare(editor.previewPoints.length, 4);
        editor.removePoint(3);
        compare(editor.previewPoints.length, 3);
        // Le minimum de deux points est respecté.
        editor.removePoint(2);
        editor.removePoint(1);
        compare(editor.previewPoints.length, 2);

        var applied = [];
        editor.applied.connect(function(value) { applied = value; });
        editor.apply();
        compare(applied.length, 2);
        compare(applied[0].temp, 30);
        compare(applied[1].speed, 50);
        compare(editor.opened, false);
    }

    function test_validationRejectsRangeAndOrder() {
        var editor = createTemporaryObject(editorComponent, testCase);

        editor.openWith([{ "temp": 101, "speed": 20 },
                         { "temp": 60, "speed": 50 }]);
        compare(editor.errorKey, "fanCurve.error.range");

        editor.openWith([{ "temp": 80, "speed": 20 },
                         { "temp": 60, "speed": 50 }]);
        compare(editor.errorKey, "fanCurve.error.order");

        editor.openWith([{ "temp": 30, "speed": 101 },
                         { "temp": 60, "speed": 50 }]);
        compare(editor.errorKey, "fanCurve.error.range");

        // Une courbe valide efface l'erreur.
        editor.openWith(points());
        compare(editor.errorKey, "");
    }

    function test_cancelDoesNotApply() {
        var editor = createTemporaryObject(editorComponent, testCase);
        editor.openWith(points());
        var applied = false;
        editor.applied.connect(function() { applied = true; });
        editor.close();
        compare(applied, false);
        compare(editor.opened, false);
    }

    function test_previewIsEditableAndSynchronized() {
        var editor = createTemporaryObject(editorComponent, testCase);
        editor.openWith(points());
        var chart = findChild(editor, "fanCurvePreview");
        verify(chart !== null);
        compare(chart.editable, true);
        compare(chart.maxPoints, 256);
        compare(chart.points.length, 3);

        // Drag simulé par le signal du composant : la ligne et l'aperçu suivent.
        editor.movePreviewPoint(1, 70, 60);
        compare(editor.previewPoints.length, 3);
        compare(editor.previewPoints[1].temp, 70);
        compare(editor.previewPoints[1].speed, 60);
        compare(editor.errorKey, "");

        // Ajout au double-clic : inséré à la bonne place (ordre strict).
        editor.addPreviewPoint(45, 35);
        compare(editor.previewPoints.length, 4);
        compare(editor.previewPoints[1].temp, 45);
        compare(editor.previewPoints[2].temp, 70);

        // Suppression : le point retiré disparaît du tableau et de l'aperçu.
        editor.removePreviewPoint(1);
        compare(editor.previewPoints.length, 3);
        compare(editor.previewPoints[1].temp, 70);

        // Le tableau reste la référence : « Appliquer » renvoie ses valeurs.
        var applied = [];
        editor.applied.connect(function(value) { applied = value; });
        editor.apply();
        compare(applied.length, 3);
        compare(applied[1].temp, 70);
        compare(applied[1].speed, 60);
    }

    function test_previewAddRespectsGapAndMinimum() {
        var editor = createTemporaryObject(editorComponent, testCase);
        editor.openWith(points());

        // Écart < 1 °C avec un voisin : refusé.
        editor.addPreviewPoint(60.5, 40);
        compare(editor.previewPoints.length, 3);
        editor.addPreviewPoint(59.5, 40);
        compare(editor.previewPoints.length, 3);

        // Minimum de deux points : la suppression directe est refusée.
        editor.removePoint(2);
        compare(editor.previewPoints.length, 2);
        editor.removePreviewPoint(1);
        compare(editor.previewPoints.length, 2);

        // Hors bornes : refusé sans casser l'aperçu.
        editor.addPreviewPoint(101, 40);
        compare(editor.previewPoints.length, 2);
        compare(editor.errorKey, "");
    }
}
