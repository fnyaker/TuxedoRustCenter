// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// État de présentation du shell (page courante, préférences, toasts).
// Aucune logique métier : la vérité vient du daemon via tcc-ipc.
// Les préférences locales sont persistées avec QtCore.Settings (QSettings).

pragma Singleton
import QtQuick
import QtCore

Item {
    id: root
    visible: false
    width: 0
    height: 0

    // ── Navigation ──────────────────────────────────────────────────────────
    readonly property var pages: ["dashboard", "profiles", "editor", "tools",
                                  "settings", "support", "info"]
    property string page: "dashboard"
    property string previousPage: "dashboard"
    property string editingProfileId: ""
    /// Section demandée à l'ouverture des Réglages (ex. `prime`). Consommée
    /// par `SettingsPage`, remise à vide après défilement.
    property string settingsSection: ""
    readonly property string section: page === "editor" ? "profiles" : page

    function navigate(target) {
        if (target === page)
            return;
        previousPage = page;
        page = target;
    }

    /// Ouvre les Réglages, éventuellement sur une section précise.
    function openSettings(sectionName) {
        settingsSection = sectionName === undefined ? "" : sectionName;
        navigate("settings");
    }

    function editProfile(profileId) {
        editingProfileId = profileId;
        navigate("editor");
    }

    function goBack() {
        navigate(previousPage === "editor" ? "profiles" : previousPage);
    }

    readonly property int pageIndex: {
        var i = pages.indexOf(page);
        return i < 0 ? 0 : i;
    }

    // ── Préférences persistées ──────────────────────────────────────────────
    Settings {
        id: prefs
        category: "ui-jalon1"
        property string themeMode: "system"
        property string language: "en"
        property string temperatureUnit: "c"
        property bool reducedMotion: false
    }

    readonly property string themeMode: prefs.themeMode
    readonly property string language: prefs.language
    readonly property string temperatureUnit: prefs.temperatureUnit
    readonly property bool reducedMotion: prefs.reducedMotion

    function setThemeMode(value) { prefs.themeMode = value; }
    function setLanguage(value) { prefs.language = value; }
    function setTemperatureUnit(value) { prefs.temperatureUnit = value; }
    function setReducedMotion(value) { prefs.reducedMotion = value; }

    // ── Demande d'action au daemon ──────────────────────────────────────────
    // Les pages n'importent jamais le module cxx-qt `Tcc` : elles émettent un
    // signal, `Main.qml` le traduit en appel D-Bus. Aucune écriture privilégiée
    // ne transite par QML.
    signal refreshRequested()
    signal profileActivationRequested(string profileId)
    signal tempProfileClearRequested()
    signal sensorCollectionRequested(bool enabled)
    signal hardwareRefreshRequested()
    signal profileSaveRequested(string profileId, string editJson)
    signal profileCreateRequested(string editJson)
    signal profileDuplicateRequested(string profileId)
    signal profileDeleteRequested(string profileId)
    /// Export/import : seuls des chemins choisis par `FileDialog` transitent.
    signal profileExportRequested(string path)
    signal profileImportRequested(string path)
    signal settingsSaveRequested(string editJson)
    signal powerBlockerRequested(bool blocked)
    signal autostartRequested(bool enabled)
    signal odmApplyRequested(string name, bool force)
    signal tdpApplyRequested(string valuesJson)
    signal ctgpApplyRequested(int watts)
    signal adminRefreshRequested()
    signal agentRefreshRequested()
    /// Écriture UVC d'un contrôle (chemin issu du snapshot `Manager1`).
    signal uvcControlRequested(string devicePath, string controlKey, int value)

    function requestRefresh() { refreshRequested(); }
    function requestProfileActivation(profileId) { profileActivationRequested(profileId); }
    function requestTempProfileClear() { tempProfileClearRequested(); }
    function requestSensorCollection(enabled) { sensorCollectionRequested(enabled); }
    function requestHardwareRefresh() { hardwareRefreshRequested(); }
    function requestProfileSave(profileId, editJson) {
        profileSaveRequested(profileId, editJson);
    }
    function requestProfileCreate(editJson) { profileCreateRequested(editJson); }
    function requestProfileDuplicate(profileId) { profileDuplicateRequested(profileId); }
    function requestProfileDelete(profileId) { profileDeleteRequested(profileId); }
    function requestProfileExport(path) { profileExportRequested(path); }
    function requestProfileImport(path) { profileImportRequested(path); }
    function requestSettingsSave(editJson) { settingsSaveRequested(editJson); }
    function requestPowerBlocker(blocked) { powerBlockerRequested(blocked); }
    function requestAutostart(enabled) { autostartRequested(enabled); }
    function requestOdmApply(name, force) { odmApplyRequested(name, force); }
    function requestTdpApply(valuesJson) { tdpApplyRequested(valuesJson); }
    function requestCtgpApply(watts) { ctgpApplyRequested(watts); }
    function requestAdminRefresh() { adminRefreshRequested(); }
    function requestAgentRefresh() { agentRefreshRequested(); }
    function requestUvcControl(devicePath, controlKey, value) {
        uvcControlRequested(devicePath, controlKey, value);
    }

    // ── États de présentation restants ─────────────────────────────────────
    // Profils, profil actif, PPD et alimentation sont live dans `LiveData`
    // (repli `DemoData` si le daemon est absent).
    /// Icône tray native disponible (SNI/plateforme).
    property bool trayAvailable: false
    /// Le composant tray a fini de charger (succès ou module absent).
    property bool trayLoaded: false
    /// Quitter demandé explicitement (ne pas masquer la fenêtre).
    property bool quitting: false
    /// Fenêtre visible (mise à jour par Main.qml) : coupe animations/historique
    /// et ralentit le polling du worker quand la fenêtre est masquée.
    property bool windowVisible: true
    property bool windowActive: true
    property string manualProfileId: ""

    // ── Toasts ──────────────────────────────────────────────────────────────
    property ListModel toasts: ListModel { }
    property int nextToastId: 1

    function notify(message, kind) {
        toasts.append({ "toastId": nextToastId++, "message": message,
                          "kind": kind === undefined ? "info" : kind });
    }

    function dismissToast(toastId) {
        for (var i = 0; i < toasts.count; ++i) {
            if (toasts.get(i).toastId === toastId) {
                toasts.remove(i);
                return;
            }
        }
    }
}
