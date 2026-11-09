// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Page Profils : presets et profils personnalisés, filtres, recherche, vues
// grille/liste, activation manuelle temporaire et actions de gestion.
//
// Les profils viennent de `LiveData` (daemon via `GetProfilesJSON`, repli
// `DemoData`). L'activation manuelle passe par `SetTempProfile`. La création,
// la duplication et la suppression passent par `Admin1.SaveProfilesJSON`
// (worker unique, conflit relu juste avant écriture). Presets non modifiables.

pragma ComponentBehavior: Bound

import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    property int filterIndex: 0
    property bool listMode: false
    property string searchText: ""
    property string pendingDeleteId: ""
    /// Nom du profil concerné par la dernière action (toast).
    property string pendingActionName: ""
    /// Nombre de colonnes de la grille. Plafonné à 4 : au-delà, les cartes
    /// deviennent trop étroites sur un écran large. Seuils : 1 colonne
    /// < 700 px, 2 de 700 à 1049, 3 de 1050 à 1399, 4 au-delà (350 px par
    /// colonne, gouttière incluse). `contentMaxWidth` (ci-dessous) garantit
    /// 4 × 300 px + gouttières.
    readonly property int columnCount: Math.max(1, Math.min(4,
            Math.floor(width / 350)))
    /// Bandeau d'erreur des actions de collection (y compris export/import).
    readonly property bool saveErrorVisible: !AdminState.saving
            && AdminState.saveError !== ""
            && (AdminState.saveAction === "duplicate"
                || AdminState.saveAction === "delete"
                || AdminState.saveAction === "export"
                || AdminState.saveAction === "import")

    function isActiveId(id) {
        return LiveData.resolvedActiveId === id;
    }

    function filteredProfiles() {
        var q = searchText.toLowerCase();
        var list = LiveData.profiles;
        var out = [];
        for (var i = 0; i < list.length; ++i) {
            var p = list[i];
            if (filterIndex === 1 && !p.preset)
                continue;
            if (filterIndex === 2 && p.preset)
                continue;
            if (filterIndex === 3 && !(p.assignedAc || p.assignedBat || isActiveId(p.id)))
                continue;
            if (q !== "" && p.name.toLowerCase().indexOf(q) < 0
                    && p.description.toLowerCase().indexOf(q) < 0)
                continue;
            out.push(p);
        }
        return out;
    }

    function automaticProfile() {
        if (LiveData.power === "ac")
            return LiveData.assignedAc;
        if (LiveData.power === "battery")
            return LiveData.assignedBat;
        return null;
    }

    function activateProfile(profile, checked) {
        if (LiveData.usingDemo) {
            if (checked) {
                DemoData.activeProfileId = profile.id;
                AppState.manualProfileId = profile.id;
                AppState.notify(Strings.tf("toast.profileActivated", [profile.name]), "ok");
            } else {
                // Désactiver l'override manuel rend la main à la bascule automatique.
                var automatic = automaticProfile();
                DemoData.activeProfileId = automatic ? automatic.id : "default";
                AppState.manualProfileId = "";
                AppState.notify(Strings.t("tray.auto"), "info");
            }
            return;
        }

        if (checked) {
            AppState.requestProfileActivation(profile.id);
            AppState.notify(Strings.tf("toast.profileActivationRequested", [profile.name]), "info");
        } else {
            AppState.requestTempProfileClear();
            AppState.notify(Strings.t("toast.tempProfileCleared"), "info");
        }
    }

    function canMutate() {
        return !LiveData.usingDemo && AdminState.available && !AdminState.saving;
    }

    function customProfileCount() {
        var list = LiveData.profiles;
        var count = 0;
        for (var i = 0; i < list.length; ++i) {
            if (!list[i].preset)
                count++;
        }
        return count;
    }

    /// Convertit l'URL du `FileDialog` en chemin local absolu.
    ///
    /// Seul `file://` est accepté ; le chemin repart ensuite en Rust, qui le
    /// revalide (absolu, borné, sans `..`) avant tout accès disque. Aucun
    /// chemin n'est jamais déduit du contenu du fichier.
    function localPath(url) {
        var text = String(url);
        if (text.indexOf("file://") !== 0)
            return "";
        text = text.substring(7);
        if (text === "" || text.charAt(0) !== "/")
            return "";
        try {
            return decodeURIComponent(text);
        } catch (error) {
            return "";
        }
    }

    function startExport() {
        if (LiveData.usingDemo) {
            AppState.notify(Strings.t("toast.demoOnly"), "info");
            return;
        }
        if (AdminState.saving) {
            AppState.notify(Strings.t("profiles.actionBusy"), "info");
            return;
        }
        if (page.customProfileCount() === 0) {
            AppState.notify(Strings.t("admin.error.nothingToExport"), "info");
            return;
        }
        exportDialog.open();
    }

    function startImport() {
        if (LiveData.usingDemo) {
            AppState.notify(Strings.t("toast.demoOnly"), "info");
            return;
        }
        if (!AdminState.available) {
            AppState.notify(Strings.t("admin.unavailable.title"), "info");
            return;
        }
        if (AdminState.saving) {
            AppState.notify(Strings.t("profiles.actionBusy"), "info");
            return;
        }
        importDialog.open();
    }

    function cancelExport() {
        AppState.notify(Strings.t("profiles.exportCancelled"), "info");
    }

    function cancelImport() {
        AppState.notify(Strings.t("profiles.importCancelled"), "info");
    }

    /// Valide la sélection du dialogue d'export (chemin → Rust).
    function acceptExport(url) {
        var path = page.localPath(url);
        if (path === "") {
            AppState.notify(Strings.t("admin.error.pathInvalid"), "error");
            return false;
        }
        AppState.requestProfileExport(path);
        return true;
    }

    /// Valide la sélection du dialogue d'import (chemin → Rust).
    function acceptImport(url) {
        var path = page.localPath(url);
        if (path === "") {
            AppState.notify(Strings.t("admin.error.pathInvalid"), "error");
            return false;
        }
        AppState.requestProfileImport(path);
        return true;
    }

    function requestDelete(profile) {
        if (LiveData.usingDemo) {
            // Mode démo : simulation locale (aucune écriture).
            page.pendingDeleteId = profile.id;
            page.pendingActionName = profile.name;
            deleteDialog.title = Strings.t("profiles.deleteTitle");
            deleteDialog.body = Strings.tf("profiles.deleteBody", [profile.name]);
            deleteDialog.ask();
            return;
        }
        if (profile.preset) {
            AppState.notify(Strings.t("profiles.presetActionHint"), "info");
            return;
        }
        if (!page.canMutate()) {
            AppState.notify(Strings.t(AdminState.available ? "profiles.actionBusy"
                                                           : "admin.unavailable.title"), "info");
            return;
        }
        page.pendingDeleteId = profile.id;
        page.pendingActionName = profile.name;
        deleteDialog.title = Strings.t("profiles.deleteTitle");
        deleteDialog.body = Strings.tf("profiles.deleteBody", [profile.name]);
        deleteDialog.ask();
    }

    function confirmDelete() {
        if (page.pendingDeleteId === "")
            return;
        if (LiveData.usingDemo) {
            page.removeProfile(page.pendingDeleteId);
            return;
        }
        AppState.requestProfileDelete(page.pendingDeleteId);
    }

    function requestNewProfile() {
        if (LiveData.usingDemo) {
            AppState.notify(Strings.t("toast.demoOnly"), "info");
            return;
        }
        if (!page.canMutate()) {
            AppState.notify(Strings.t(AdminState.available ? "profiles.actionBusy"
                                                           : "admin.unavailable.title"), "info");
            return;
        }
        AppState.editProfile("new");
    }

    function duplicateProfile(profile) {
        if (LiveData.usingDemo) {
            AppState.notify(Strings.t("toast.demoOnly"), "info");
            return;
        }
        if (profile.preset) {
            AppState.notify(Strings.t("profiles.presetActionHint"), "info");
            return;
        }
        if (!page.canMutate()) {
            AppState.notify(Strings.t(AdminState.available ? "profiles.actionBusy"
                                                           : "admin.unavailable.title"), "info");
            return;
        }
        page.pendingActionName = profile.name;
        AppState.requestProfileDuplicate(profile.id);
    }

    function removeProfile(id) {
        AppState.notify(Strings.t("toast.profileDeleted"), "ok");
        if (DemoData.activeProfileId === id)
            DemoData.activeProfileId = "default";
        page.pendingDeleteId = "";
    }

    // Retour des mutations (duplication/suppression) : toast ou bandeau.
    Connections {
        target: AdminState

        function onSaveSucceededChanged() {
            if (!AdminState.saveSucceeded)
                return;
            if (AdminState.saveAction === "duplicate") {
                AppState.notify(Strings.tf("profiles.duplicated",
                                           [page.pendingActionName]), "ok");
                page.pendingActionName = "";
            } else if (AdminState.saveAction === "delete") {
                AppState.notify(Strings.tf("profiles.deleted",
                                           [page.pendingActionName]), "ok");
                page.pendingActionName = "";
                page.pendingDeleteId = "";
            } else if (AdminState.saveAction === "export") {
                AppState.notify(Strings.exportResult(AdminState.exportedCount), "ok");
            } else if (AdminState.saveAction === "import") {
                if (AdminState.importedCount === 0) {
                    AppState.notify(Strings.t("profiles.importEmpty"), "info");
                } else {
                    AppState.notify(Strings.importResult(AdminState.importedCount,
                                                         AdminState.renamedCount,
                                                         AdminState.skippedCount), "ok");
                }
            }
        }
    }

    Shortcut {
        sequences: ["Ctrl+F"]
        onActivated: searchField.forceActiveFocus()
    }

    contentSpacing: Theme.s16
    // Une grille de 4 cartes doit rester confortable : 4 × 300 + 3 × 16 = 1248
    // px, plus les marges de page. Les autres pages gardent 1180 px de lecture.
    contentMaxWidth: 1440

    PageHeader {
        Layout.fillWidth: true
        title: Strings.t("profiles.title")
        subtitle: Strings.t("profiles.subtitle")
        iconName: "allprofiles"

        StatusBadge {
            id: sourceBadge
            kind: LiveData.isLive ? "ok" : "warn"
            label: LiveData.isLive ? Strings.t("app.live") : Strings.t("common.demoTag")
            AppToolTip {
                visible: sourceHover.hovered
                text: LiveData.isLive ? Strings.t("app.liveHint") : Strings.t("app.demoHint")
            }
            HoverHandler { id: sourceHover }
        }

        StatusBadge {
            visible: LiveData.parseError !== ""
            kind: "error"
            label: Strings.t("profiles.sourceError")
            AppToolTip {
                visible: errorHover.hovered
                text: LiveData.parseError
            }
            HoverHandler { id: errorHover }
        }

        AppButton {
            id: importButton
            objectName: "importButton"
            text: Strings.t("profiles.import")
            variant: "secondary"
            iconName: "download"
            enabled: !AdminState.saving && (LiveData.usingDemo || AdminState.available)
            Accessible.description: !LiveData.usingDemo && !AdminState.available
                                    ? Strings.t("admin.unavailable.missing") : ""
            onClicked: page.startImport()
        }

        AppButton {
            id: exportButton
            objectName: "exportButton"
            text: Strings.t("profiles.export")
            variant: "secondary"
            iconName: "save"
            enabled: !AdminState.saving
                     && (LiveData.usingDemo || page.customProfileCount() > 0)
            Accessible.description: !LiveData.usingDemo
                                    && page.customProfileCount() === 0
                                    ? Strings.t("admin.error.nothingToExport") : ""
            onClicked: page.startExport()
        }

        AppButton {
            text: Strings.t("profiles.new")
            enabled: !AdminState.saving
            Accessible.description: !AdminState.available
                                    ? Strings.t("admin.unavailable.missing") : ""
            onClicked: page.requestNewProfile()
        }
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: !LiveData.usingDemo && !AdminState.available
        kind: "warn"
        title: Strings.t("admin.unavailable.title")
        body: AdminState.availabilityText
        actionText: Strings.t("common.retry")
        onActionClicked: AppState.requestAdminRefresh()
    }

    InlineNotice {
        id: saveErrorNotice
        objectName: "saveErrorNotice"
        Layout.fillWidth: true
        visible: page.saveErrorVisible
        kind: "error"
        title: AdminState.saveErrorText
        body: AdminState.saveErrorDetailText !== ""
              ? Strings.t("editor.errorField") + " " + AdminState.saveErrorDetailText
              : ""
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.s8

        SegmentedControl {
            id: filters
            options: [
                { "label": Strings.t("profiles.filter.all") },
                { "label": Strings.t("profiles.filter.preset") },
                { "label": Strings.t("profiles.filter.custom") },
                { "label": Strings.t("profiles.filter.inUse") }
            ]
            onActivated: function(index) { page.filterIndex = index; }
        }

        Item { Layout.fillWidth: true }

        AppLabel {
            variant: "caption"
            secondary: true
            text: Strings.profileCount(page.filteredProfiles().length)
        }

        AppTextField {
            id: searchField
            Layout.preferredWidth: 220
            placeholderText: Strings.t("profiles.searchPlaceholder")
            Accessible.name: Strings.t("profiles.searchPlaceholder")
            onTextChanged: page.searchText = text
        }

        IconButton {
            iconName: page.listMode ? "allprofiles" : "monitor"
            tooltip: page.listMode ? Strings.t("profiles.view.grid") : Strings.t("profiles.view.list")
            Accessible.name: tooltip
            onClicked: page.listMode = !page.listMode
        }
    }

    // ── Grille ──────────────────────────────────────────────────────────────
    GridLayout {
        objectName: "profileGrid"
        Layout.fillWidth: true
        Layout.topMargin: Theme.s4
        columns: page.columnCount
        columnSpacing: Theme.s16
        rowSpacing: Theme.s16
        visible: !page.listMode && page.filteredProfiles().length > 0

        Repeater {
            model: page.filteredProfiles()

            delegate: ProfileCard {
                required property var modelData

                objectName: "profileCard_" + modelData.id
                Layout.fillWidth: true
                Layout.preferredHeight: 348
                profile: modelData
                active: page.isActiveId(modelData.id)
                onActivateRequested: function(checked) { page.activateProfile(modelData, checked); }
                onEditRequested: AppState.editProfile(modelData.id)
                onDuplicateRequested: page.duplicateProfile(modelData)
                onExportRequested: page.startExport()
                onDeleteRequested: page.requestDelete(modelData)
            }
        }

        // Carte « nouveau profil »
        Rectangle {
            objectName: "newProfileCard"
            readonly property int minimumCardWidth: 300
            Layout.fillWidth: true
            Layout.minimumWidth: minimumCardWidth
            Layout.preferredHeight: 348
            radius: Theme.radiusLg
            color: Theme.surfaceMuted
            border.width: 1
            border.color: newHover.hovered ? Theme.accent : Theme.border

            HoverHandler {
                id: newHover
                cursorShape: Qt.PointingHandCursor
            }

            ColumnLayout {
                anchors.centerIn: parent
                width: parent.width - Theme.s32
                spacing: Theme.s10

                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    width: 52
                    height: 52
                    radius: 26
                    color: Theme.accentSoft

                    Glyph {
                        anchors.centerIn: parent
                        name: "plus"
                        color: Theme.accentText
                        size: 20
                        strokeWidth: 2
                    }
                }

                AppLabel {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    variant: "h3"
                    text: Strings.t("profiles.newCardTitle")
                }

                AppLabel {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    variant: "small"
                    secondary: true
                    text: Strings.t("profiles.newCardBody")
                    wrapMode: Text.WordWrap
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: page.requestNewProfile()
            }
        }
    }

    // ── Liste ───────────────────────────────────────────────────────────────
    ColumnLayout {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s4
        spacing: Theme.s8
        visible: page.listMode && page.filteredProfiles().length > 0

        Repeater {
            model: page.filteredProfiles()

            delegate: ProfileCard {
                required property var modelData

                objectName: "profileRow_" + modelData.id
                Layout.fillWidth: true
                listMode: true
                profile: modelData
                active: page.isActiveId(modelData.id)
                onActivateRequested: function(checked) { page.activateProfile(modelData, checked); }
                onEditRequested: AppState.editProfile(modelData.id)
                onDuplicateRequested: page.duplicateProfile(modelData)
                onExportRequested: page.startExport()
                onDeleteRequested: page.requestDelete(modelData)
            }
        }
    }

    EmptyState {
        Layout.fillWidth: true
        Layout.topMargin: Theme.s32
        visible: page.filteredProfiles().length === 0
        iconName: "allprofiles"
        title: Strings.t("profiles.empty.title")
        body: Strings.t("profiles.empty.body")
        actionText: Strings.t("profiles.empty.cta")
        onActionClicked: page.requestNewProfile()
    }

    ConfirmDialog {
        id: deleteDialog
        confirmText: Strings.t("common.delete")
        onConfirmed: page.confirmDelete()
    }

    // Export : chemin choisi par l'utilisateur, écriture locale bornée en Rust.
    FileDialog {
        id: exportDialog
        title: Strings.t("profiles.exportTitle")
        fileMode: FileDialog.SaveFile
        nameFilters: [Strings.t("profiles.exportNameFilter")]
        defaultSuffix: "json"
        onAccepted: page.acceptExport(selectedFile)
        onRejected: page.cancelExport()

        Component.onCompleted: {
            var documents = StandardPaths.writableLocation(StandardPaths.DocumentsLocation);
            var home = StandardPaths.writableLocation(StandardPaths.HomeLocation);
            var base = documents !== "" ? documents : home;
            if (base !== "")
                currentFile = "file://" + base + "/TCC_Profiles.json";
        }
    }

    // Import : lecture bornée (1 Mio) et validation stricte côté Rust ; aucun
    // `JSON.parse` en QML, aucune écriture privilégiée hors `Admin1`.
    FileDialog {
        id: importDialog
        title: Strings.t("profiles.importTitle")
        fileMode: FileDialog.OpenFile
        nameFilters: [Strings.t("profiles.importNameFilter")]
        onAccepted: page.acceptImport(selectedFile)
        onRejected: page.cancelImport()
    }
}
