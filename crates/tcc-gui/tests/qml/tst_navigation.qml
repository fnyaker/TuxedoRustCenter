// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Navigation ciblée depuis le Dashboard : profil actif → éditeur,
// mode graphique → Réglages (section PRIME).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"

TestCase {
    id: testCase
    name: "Navigation"
    when: windowShown

    function reset() {
        AppState.settingsSection = "";
        AppState.editingProfileId = "";
        AppState.previousPage = "dashboard";
        AppState.page = "dashboard";
    }

    function init() {
        reset();
    }

    function cleanup() {
        reset();
    }

    function test_openProfileOpensEditor() {
        AppState.editProfile("__office__");
        compare(AppState.page, "editor");
        compare(AppState.editingProfileId, "__office__");
        // La sidebar met en évidence la section Profils.
        compare(AppState.section, "profiles");
    }

    function test_backFromEditorReturnsToPreviousPage() {
        // Ouvert depuis le Dashboard : Alt+← revient au Dashboard.
        AppState.editProfile("__office__");
        AppState.goBack();
        compare(AppState.page, "dashboard");

        // Ouvert depuis la liste des profils : retour aux profils.
        AppState.navigate("profiles");
        AppState.editProfile("__office__");
        compare(AppState.page, "editor");
        AppState.goBack();
        compare(AppState.page, "profiles");
    }

    function test_openGraphicsSettingsTargetsPrimeSection() {
        AppState.openSettings("prime");
        compare(AppState.page, "settings");
        compare(AppState.settingsSection, "prime");
    }

    function test_openSettingsWithoutSectionClearsRequest() {
        AppState.openSettings("prime");
        AppState.openSettings();
        compare(AppState.page, "settings");
        compare(AppState.settingsSection, "");
    }

    function test_openSettingsWhileOnSettingsKeepsSection() {
        AppState.navigate("settings");
        AppState.openSettings("prime");
        compare(AppState.page, "settings");
        // Pas de changement de page : la section reste disponible pour le
        // défilement (`SettingsPage` consomme puis remet à vide).
        compare(AppState.settingsSection, "prime");
    }

    function test_settingsSectionIsConsumableOnce() {
        AppState.openSettings("prime");
        // Contrat de consommation : la page vide la demande après défilement.
        AppState.settingsSection = "";
        compare(AppState.settingsSection, "");

        // Une nouvelle demande reste possible.
        AppState.openSettings("prime");
        compare(AppState.settingsSection, "prime");
    }

    function test_pagesListIsStable() {
        compare(AppState.pages.length, 7);
        compare(AppState.pages[0], "dashboard");
        compare(AppState.pages.indexOf("settings") >= 0, true);
        compare(AppState.pageIndex, 0);
        AppState.navigate("profiles");
        compare(AppState.pageIndex, 1);
    }

    function test_i18nHintsExist() {
        Strings.lang = "fr";
        verify(!Strings.t("dash.openProfileHint").startsWith("dash."));
        verify(!Strings.t("dash.openGraphicsSettingsHint").startsWith("dash."));
        verify(!Strings.t("sensors.stale").startsWith("sensors."));
        verify(!Strings.t("fans.mode.manual").startsWith("fans."));
        // Légendes des graphes intégrés aux cartes CPU/GPU.
        verify(!Strings.t("dash.temp").startsWith("dash."));
        verify(!Strings.t("dash.freq").startsWith("dash."));
        // Ciblesses de navigation encore résolues (profil, PRIME, profils).
        verify(!Strings.t("profiles.title").startsWith("profiles."));
        verify(!Strings.t("settings.graphics.ondemand").startsWith("settings."));
        Strings.lang = "en";
        compare(Strings.t("fans.mode.off"), "Off");
        Strings.lang = "fr";
    }

    function test_dashboardCrossPageTargetsAreStable() {
        // Les liens du Dashboard (profil actif, mode graphique, profils) ne
        // dépendent pas des cartes de diagnostic supprimées.
        AppState.navigate("dashboard");
        AppState.openSettings("prime");
        compare(AppState.page, "settings");
        compare(AppState.settingsSection, "prime");

        AppState.editProfile("custom-a");
        compare(AppState.page, "editor");
        compare(AppState.editingProfileId, "custom-a");
    }
}
