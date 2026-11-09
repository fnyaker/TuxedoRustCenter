// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests Qt Quick du design system (qmltestrunner).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml
//
// Ces tests couvrent les jetons, l'i18n et le chargement des composants.
// Les tests d'intégration D-Bus/cxx-qt sont couverts séparément.

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/components"

TestCase {
    id: testCase
    name: "DesignSystem"
    when: windowShown

    Component {
        id: buttonComponent
        AppButton { }
    }

    Component {
        id: gaugeComponent
        Gauge { value: 42; to: 100; unit: "%"; label: "CPU" }
    }

    Component {
        id: badgeComponent
        StatusBadge { kind: "ok"; label: "Actif" }
    }

    function test_themeBrandTokens() {
        Theme.mode = "light";
        compare(Theme.dark, false);
        compare(String(Theme.brandRed), "#e30016");
        compare(String(Theme.gauge), "#e22131");
        compare(String(Theme.accent), "#e30016");
        compare(String(Theme.bg), "#f4f4f5");
        compare(String(Theme.surface), "#ffffff");
    }

    function test_themeDarkTokens() {
        Theme.mode = "dark";
        compare(Theme.dark, true);
        compare(String(Theme.bg), "#17171a");
        compare(String(Theme.surface), "#202023");
        compare(String(Theme.gauge), "#f2545b");
        compare(String(Theme.accent), "#ff4d5e");
        Theme.mode = "light";
    }

    function test_themeMotionRespectsReducedMotion() {
        Theme.reduceMotion = false;
        verify(Theme.durationBase > 0);
        Theme.reduceMotion = true;
        compare(Theme.durationBase, 0);
        compare(Theme.durationSlow, 0);
        Theme.reduceMotion = false;
    }

    function test_stringsLookupAndFallback() {
        Strings.lang = "fr";
        compare(Strings.t("nav.dashboard"), "Dashboard");
        compare(Strings.t("common.save"), "Enregistrer");
        // Une clé absente est renvoyée telle quelle (visible en développement).
        compare(Strings.t("missing.key"), "missing.key");
    }

    function test_stringsSubstitution() {
        Strings.lang = "fr";
        compare(Strings.tf("profiles.deleteBody", ["Gaming"]).indexOf("Gaming") >= 0, true);
        compare(Strings.tf("tray.version", ["1.2.3"]), "Version 1.2.3");
    }

    function test_stringsPlural() {
        Strings.lang = "fr";
        compare(Strings.plural(1, "profiles.unit", "profiles.unitPlural"), "profil");
        compare(Strings.plural(3, "profiles.unit", "profiles.unitPlural"), "profils");
        compare(Strings.profileCount(1), "1 profil");
        compare(Strings.profileCount(4), "4 profils");
    }

    function test_temperatureUnits() {
        Strings.fahrenheit = false;
        compare(Strings.tempUnit, "°C");
        compare(Strings.formatTemp(56), "56 °C");

        Strings.fahrenheit = true;
        compare(Strings.tempUnit, "°F");
        // 56 °C = 132,8 °F → arrondi à 133.
        compare(Strings.formatTemp(56), "133 °F");

        Strings.fahrenheit = false;
    }

    function test_numberFormattingUsesLocale() {
        Strings.lang = "fr";
        compare(Strings.formatNumber(3.5, 1), "3,5");
        Strings.lang = "en";
        compare(Strings.formatNumber(3.5, 1), "3.5");
        Strings.lang = "fr";
    }

    function test_i18nCatalogsHaveSameKeys() {
        var reference = Object.keys(Strings.fr).sort();
        var en = Object.keys(Strings.en).sort();
        var de = Object.keys(Strings.de).sort();

        compare(en.length, reference.length);
        compare(de.length, reference.length);

        for (var i = 0; i < reference.length; ++i) {
            compare(en[i], reference[i]);
            compare(de[i], reference[i]);
        }
    }

    function test_demoDataIntegrity() {
        verify(DemoData.profiles.length >= 4);
        var seen = {};
        for (var i = 0; i < DemoData.profiles.length; ++i) {
            var profile = DemoData.profiles[i];
            verify(profile.id !== "");
            verify(profile.name !== "");
            verify(!seen[profile.id]);
            seen[profile.id] = true;
            verify(profile.freqMin <= profile.freqMax);
            verify(profile.cores >= 1 && profile.cores <= 256);
        }
        compare(DemoData.profileById("gaming").name, "Gaming");
        // Identifiant inconnu → profil par défaut, jamais null.
        compare(DemoData.profileById("nope").id, "default");
    }

    function test_fanCurveIsMonotonic() {
        var curve = DemoData.fanCurve;
        verify(curve.length >= 2);
        for (var i = 1; i < curve.length; ++i) {
            verify(curve[i].temp > curve[i - 1].temp);
            verify(curve[i].speed >= curve[i - 1].speed);
        }
    }

    function test_componentsInstantiate() {
        var button = createTemporaryObject(buttonComponent, testCase, { "text": "Enregistrer" });
        verify(button !== null);
        compare(button.variant, "primary");

        var gauge = createTemporaryObject(gaugeComponent, testCase);
        verify(gauge !== null);
        verify(gauge.targetRatio > 0.4 && gauge.targetRatio < 0.5);

        var badge = createTemporaryObject(badgeComponent, testCase);
        verify(badge !== null);
        compare(badge.kind, "ok");
    }

    function test_gaugeUnavailableState() {
        var gauge = createTemporaryObject(gaugeComponent, testCase, {
            "value": 0,
            "available": false
        });
        verify(gauge !== null);
        compare(gauge.targetRatio, 0);
        compare(gauge.displayValue, "—");
    }
}
