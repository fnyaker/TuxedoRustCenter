// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Tests de la page Profils : création, duplication et suppression via
// `Admin1.SaveProfilesJSON` (signaux, garde admin, presets).
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "ProfilesPage"
    when: windowShown

    Component {
        id: pageComponent
        ProfilesPage { }
    }

    SignalSpy {
        id: duplicateSpy
        target: AppState
        signalName: "profileDuplicateRequested"
    }

    SignalSpy {
        id: deleteSpy
        target: AppState
        signalName: "profileDeleteRequested"
    }

    SignalSpy {
        id: exportSpy
        target: AppState
        signalName: "profileExportRequested"
    }

    SignalSpy {
        id: importSpy
        target: AppState
        signalName: "profileImportRequested"
    }

    function fixture() {
        return JSON.stringify([
            {
                "id": "__office__",
                "name": "Office",
                "description": "Preset",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": {},
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Balanced",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            },
            {
                "id": "custom-a",
                "name": "Studio",
                "description": "Custom",
                "display": { "refreshRate": -1, "useRefRate": false },
                "cpu": {},
                "webcam": { "status": true, "useStatus": true },
                "fan": { "useControl": true, "fanProfile": "Balanced",
                         "minimumFanspeed": 0, "maximumFanspeed": 100,
                         "offsetFanspeed": 0 }
            }
        ]);
    }

    function reset() {
        AdminState.available = false;
        AdminState.saving = false;
        AdminState.saveAction = "";
        AdminState.saveError = "";
        AdminState.saveErrorDetail = "";
        AdminState.saveSucceeded = false;
        AdminState.createdProfileId = "";
        AdminState.exportedCount = 0;
        AdminState.importedCount = 0;
        AdminState.renamedCount = 0;
        AdminState.skippedCount = 0;
        AppState.editingProfileId = "";
        AppState.previousPage = "dashboard";
        AppState.page = "dashboard";
        LiveData.connected = false;
        LiveData.profilesJson = "[]";
        duplicateSpy.clear();
        deleteSpy.clear();
        exportSpy.clear();
        importSpy.clear();
    }

    function init() {
        // Les libellés comparés dans les tests sont les chaînes françaises
        // de référence : la langue par défaut de l'application est l'anglais.
        Strings.lang = "fr";
        reset();
        LiveData.connected = true;
        LiveData.profilesJson = fixture();
    }

    function cleanup() {
        reset();
    }

    function profileById(id) {
        return LiveData.profileById(id);
    }

    function test_newProfileOpensEditorWhenAdminAvailable() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page !== null);
        page.requestNewProfile();
        compare(AppState.page, "editor");
        compare(AppState.editingProfileId, "new");
    }

    function test_newProfileBlockedWithoutAdmin() {
        AdminState.available = false;
        var page = createTemporaryObject(pageComponent, testCase);
        page.requestNewProfile();
        compare(AppState.page, "dashboard");
        compare(AppState.editingProfileId, "");
    }

    function test_duplicateCustomProfileSignals() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.duplicateProfile(profileById("custom-a"));
        compare(duplicateSpy.count, 1);
        compare(duplicateSpy.signalArguments[0][0], "custom-a");
    }

    function test_duplicatePresetIsRefused() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.duplicateProfile(profileById("__office__"));
        compare(duplicateSpy.count, 0);
    }

    function test_duplicateBlockedWithoutAdmin() {
        AdminState.available = false;
        var page = createTemporaryObject(pageComponent, testCase);
        page.duplicateProfile(profileById("custom-a"));
        compare(duplicateSpy.count, 0);
    }

    function test_deleteRequiresConfirmation() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.requestDelete(profileById("custom-a"));
        // La demande ouvre le dialogue, aucun signal avant confirmation.
        compare(deleteSpy.count, 0);
        compare(page.pendingDeleteId, "custom-a");

        page.confirmDelete();
        compare(deleteSpy.count, 1);
        compare(deleteSpy.signalArguments[0][0], "custom-a");
    }

    function test_deletePresetIsRefused() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.requestDelete(profileById("__office__"));
        compare(page.pendingDeleteId, "");
        page.confirmDelete();
        compare(deleteSpy.count, 0);
    }

    function test_actionsBusyWhileSaving() {
        AdminState.available = true;
        AdminState.saving = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.duplicateProfile(profileById("custom-a"));
        page.requestNewProfile();
        compare(duplicateSpy.count, 0);
        compare(AppState.page, "dashboard");
    }

    function test_successToastForDuplicate() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        var before = AppState.toasts.count;

        page.duplicateProfile(profileById("custom-a"));
        AdminState.saveAction = "duplicate";
        AdminState.saveSucceeded = true;
        compare(AppState.toasts.count, before + 1);
        compare(AppState.toasts.get(AppState.toasts.count - 1).kind, "ok");
    }


    function test_localPathConvertsFileUrlsOnly() {
        var page = createTemporaryObject(pageComponent, testCase);
        // Espaces encodés décodés, autorité vide.
        compare(page.localPath("file:///home/user/TCC%20Profils.json"),
                "/home/user/TCC Profils.json");
        // Tout ce qui n'est pas un chemin absolu local est refusé.
        compare(page.localPath("https://example.com/a.json"), "");
        compare(page.localPath("relative.json"), "");
        compare(page.localPath("file://"), "");
        compare(page.localPath("file://relative.json"), "");
    }

    function test_exportRequestUsesSelectedPath() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page.acceptExport("file:///tmp/tcc-export.json"));
        compare(exportSpy.count, 1);
        compare(exportSpy.signalArguments[0][0], "/tmp/tcc-export.json");

        // URL non locale : aucun signal, erreur explicite.
        verify(!page.acceptExport("https://example.com/x.json"));
        compare(exportSpy.count, 1);
        compare(AppState.toasts.get(AppState.toasts.count - 1).kind, "error");
    }

    function test_importRequestUsesSelectedPath() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page.acceptImport("file:///tmp/tcc-import.json"));
        compare(importSpy.count, 1);
        compare(importSpy.signalArguments[0][0], "/tmp/tcc-import.json");
    }

    function test_transferButtonsFollowAvailability() {
        AdminState.available = false;
        var page = createTemporaryObject(pageComponent, testCase);
        var importButton = findChild(page, "importButton");
        var exportButton = findChild(page, "exportButton");
        verify(importButton !== null);
        verify(exportButton !== null);
        // L'import écrit via Admin1 : bloqué sans admin.
        compare(importButton.enabled, false);
        // L'export est une écriture locale non privilégiée : autorisé.
        compare(exportButton.enabled, true);

        AdminState.available = true;
        compare(importButton.enabled, true);
        AdminState.saving = true;
        compare(importButton.enabled, false);
        compare(exportButton.enabled, false);
    }

    function test_importRefusedWithoutAdmin() {
        AdminState.available = false;
        var page = createTemporaryObject(pageComponent, testCase);
        page.startImport();
        compare(importSpy.count, 0);
        compare(page.canMutate(), false);
    }

    function test_transferCancelFeedback() {
        var page = createTemporaryObject(pageComponent, testCase);
        page.cancelExport();
        var toast = AppState.toasts.get(AppState.toasts.count - 1);
        compare(toast.message, Strings.t("profiles.exportCancelled"));
        compare(toast.kind, "info");

        page.cancelImport();
        toast = AppState.toasts.get(AppState.toasts.count - 1);
        compare(toast.message, Strings.t("profiles.importCancelled"));
    }

    function test_exportSummaryToast() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.acceptExport("file:///tmp/tcc-export.json");

        AdminState.saveAction = "export";
        AdminState.exportedCount = 3;
        AdminState.saveSucceeded = true;
        var toast = AppState.toasts.get(AppState.toasts.count - 1);
        compare(toast.kind, "ok");
        compare(toast.message, Strings.exportResult(3));
    }

    function test_importSummaryToast() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.acceptImport("file:///tmp/tcc-import.json");

        AdminState.saveAction = "import";
        AdminState.importedCount = 2;
        AdminState.renamedCount = 1;
        AdminState.skippedCount = 1;
        AdminState.saveSucceeded = true;
        var toast = AppState.toasts.get(AppState.toasts.count - 1);
        compare(toast.kind, "ok");
        verify(toast.message.indexOf("2") >= 0);
        verify(toast.message.indexOf("1") >= 0);
    }

    function test_importWithoutProfilesIsInformative() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.acceptImport("file:///tmp/tcc-import.json");

        AdminState.saveAction = "import";
        AdminState.importedCount = 0;
        AdminState.skippedCount = 2;
        AdminState.saveSucceeded = true;
        var toast = AppState.toasts.get(AppState.toasts.count - 1);
        compare(toast.kind, "info");
        compare(toast.message, Strings.t("profiles.importEmpty"));
    }

    function test_importErrorBannerIsShown() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.acceptImport("file:///tmp/tcc-bad.json");

        AdminState.saveAction = "import";
        AdminState.saveError = "InvalidImport";
        AdminState.saving = false;
        compare(AdminState.saveErrorKey, "admin.error.invalidImport");
        compare(page.saveErrorVisible, true);
        var notice = findChild(page, "saveErrorNotice");
        verify(notice !== null);

        // Un code inconnu reste traduit génériquement.
        AdminState.saveError = "Bogus";
        compare(AdminState.saveErrorKey, "admin.error.unknown");
    }

    function test_inUseErrorKeepsProfile() {
        AdminState.available = true;
        var page = createTemporaryObject(pageComponent, testCase);
        page.requestDelete(profileById("custom-a"));
        page.confirmDelete();

        AdminState.saveAction = "delete";
        AdminState.saveError = "InUse";
        AdminState.saveErrorDetail = "custom-a";
        // Le profil reste dans la liste et le message est traduit.
        verify(LiveData.profileById("custom-a") !== null);
        compare(AdminState.saveErrorKey, "admin.error.inUse");
        verify(AdminState.saveErrorText.indexOf("état") >= 0);
    }

    function test_gridColumnCountIsResponsiveAndCapped() {
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page !== null);
        page.height = 900;

        var widths = [500, 900, 1200, 1600];
        var expected = [1, 2, 3, 4];
        for (var i = 0; i < widths.length; ++i) {
            page.width = widths[i];
            compare(page.columnCount, expected[i],
                    "largeur " + widths[i] + " → " + expected[i] + " colonne(s)");
        }

        // Bornes documentées.
        page.width = 699; compare(page.columnCount, 1);
        page.width = 700; compare(page.columnCount, 2);
        page.width = 1049; compare(page.columnCount, 2);
        page.width = 1050; compare(page.columnCount, 3);
        page.width = 1399; compare(page.columnCount, 3);
        page.width = 1400; compare(page.columnCount, 4);

        // Plafond dur : jamais 5 colonnes, même sur un écran très large.
        page.width = 2560;
        compare(page.columnCount, 4);
    }

    function test_gridCardsKeepMinimumWidthWithoutOverflow() {
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page !== null);
        page.width = 1600;
        page.height = 900;
        wait(50);

        var grid = findChild(page, "profileGrid");
        verify(grid !== null);
        compare(grid.columns, 4);
        verify(grid.implicitWidth <= grid.width + 1,
               "la grille ne doit pas déborder horizontalement");
        verify(grid.width <= page.width, "la grille reste dans la page");

        var card = findChild(page, "profileCard_custom-a");
        verify(card !== null);
        compare(card.minimumCardWidth, 300);
        verify(card.width >= 300, "carte de profil ≥ 300 px, obtenu " + card.width);

        var newCard = findChild(page, "newProfileCard");
        verify(newCard !== null);
        compare(newCard.minimumCardWidth, 300);
        verify(newCard.width >= 300, "carte nouveau profil ≥ 300 px");
    }

    function test_filtersAndListModeStillWork() {
        var page = createTemporaryObject(pageComponent, testCase);
        verify(page !== null);
        page.width = 1200;
        page.height = 900;
        wait(20);

        // Filtres : presets, personnalisés, recherche.
        page.filterIndex = 1;
        compare(page.filteredProfiles().length, 1);
        compare(page.filteredProfiles()[0].id, "__office__");
        page.filterIndex = 2;
        compare(page.filteredProfiles().length, 1);
        compare(page.filteredProfiles()[0].id, "custom-a");
        page.filterIndex = 0;
        page.searchText = "stud";
        compare(page.filteredProfiles().length, 1);
        compare(page.filteredProfiles()[0].id, "custom-a");

        // Bascule liste : la grille disparaît, les lignes restent dans la page.
        page.searchText = "";
        page.listMode = true;
        wait(20);
        var grid = findChild(page, "profileGrid");
        verify(grid !== null);
        compare(grid.visible, false);
        var row = findChild(page, "profileRow_custom-a");
        verify(row !== null);
        verify(row.width <= page.width, "la ligne ne déborde pas de la page");
    }
}
