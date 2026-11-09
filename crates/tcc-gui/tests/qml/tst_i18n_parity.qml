// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Parité des clés de traduction FR/EN/DE. Exigence de `docs/QUALITY.md` :
// toute clé ajoutée dans une langue doit exister dans les autres.
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "I18nParity"

    function keys(dictionary) {
        var result = [];
        for (var key in dictionary)
            result.push(key);
        result.sort();
        return result;
    }

    function test_languagesExposeExactlyTheSameKeys() {
        var fr = keys(Strings.fr);
        var en = keys(Strings.en);
        var de = keys(Strings.de);

        compare(en.length, fr.length, "EN et FR n'ont pas le même nombre de clés");
        compare(de.length, fr.length, "DE et FR n'ont pas le même nombre de clés");

        for (var i = 0; i < fr.length; ++i) {
            compare(en[i], fr[i], "clé EN manquante ou renommée: " + fr[i]);
            compare(de[i], fr[i], "clé DE manquante ou renommée: " + fr[i]);
        }
    }

    function test_noEmptyTranslations() {
        var languages = [["fr", Strings.fr], ["en", Strings.en], ["de", Strings.de]];
        for (var i = 0; i < languages.length; ++i) {
            var name = languages[i][0];
            var dictionary = languages[i][1];
            for (var key in dictionary) {
                var value = dictionary[key];
                verify(typeof value === "string", name + "." + key + " n'est pas une chaîne");
                verify(value.length > 0, name + "." + key + " est vide");
            }
        }
    }

    function test_languageSwitchAffectsLookup() {
        Strings.lang = "fr";
        compare(Strings.t("common.save"), "Enregistrer");
        compare(Strings.t("nav.dashboard"), "Dashboard");

        Strings.lang = "en";
        compare(Strings.t("common.save"), "Save");

        Strings.lang = "de";
        compare(Strings.t("common.save"), "Speichern");

        Strings.lang = "fr";
        // Une clé absente reste visible en développement, jamais silencieuse.
        compare(Strings.t("missing.key"), "missing.key");
    }
}
