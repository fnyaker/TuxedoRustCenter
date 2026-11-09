// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests du composant SelectRow : infobulle au survol et au focus clavier,
// sélection pilotée par le modèle (sans index périmé).

import QtQuick
import QtQuick.Window
import QtTest
import "../../qml/components"

TestCase {
    id: testCase
    name: "SelectRow"
    when: windowShown

    Window {
        id: testWindow
        width: 640
        height: 320
        visible: true
    }

    Component {
        id: rowComponent
        SelectRow {
            width: 560
            label: "Energy preference"
            description: "Stored per profile"
            tooltip: "CPU driver EPP; power-profiles-daemon owns the effective value."
            options: ["power", "balance_power", "performance"]
            currentIndex: 1
        }
    }

    function test_tooltipAppearsOnHoverAndKeyboardFocus() {
        var row = createTemporaryObject(rowComponent, testWindow);
        verify(row !== null);

        var info = findChild(row, "selectRowInfo");
        var tip = findChild(row, "selectRowTooltip");
        verify(info !== null);
        verify(tip !== null);
        compare(tip.text, row.tooltip);
        verify(tip.text.length > 0);
        compare(tip.visible, false);

        // Survol.
        mouseMove(info, info.width / 2, info.height / 2);
        tryCompare(tip, "visible", true);

        // Le même texte est accessible au clavier (focus de l'icône d'aide).
        testWindow.requestActivate();
        wait(50);
        info.forceActiveFocus();
        compare(info.activeFocus, true);
        compare(tip.visible, true);

        // Le champ expose le libellé et l'aide aux lecteurs d'écran.
        var select = findChild(row, "selectRowSelect");
        verify(select !== null);
        compare(select.Accessible.name, row.label);
        compare(select.Accessible.description, row.tooltip);
    }

    function test_selectionFollowsModelWithoutStaleIndex() {
        var row = createTemporaryObject(rowComponent, testWindow);
        verify(row !== null);
        var select = findChild(row, "selectRowSelect");
        verify(select !== null);
        compare(select.currentIndex, 1);

        // Changement piloté par le parent (changement de profil, de mode…).
        row.currentIndex = 2;
        tryCompare(select, "currentIndex", 2);
        row.currentIndex = 0;
        tryCompare(select, "currentIndex", 0);

        // Choix utilisateur : le signal part, le parent met à jour le modèle.
        var changed = -1;
        row.changed.connect(function(index) { changed = index; });
        select.activated(2);
        compare(changed, 2);
        row.currentIndex = 2;
        tryCompare(select, "currentIndex", 2);
    }
}
