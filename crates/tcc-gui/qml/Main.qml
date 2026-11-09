// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Fenêtre principale du shell TuxedoRustCenter.
//
// Navigation complète, thème clair / sombre / système, i18n à chaud FR/EN/DE,
// préférences persistées localement. Les pages consomment les modèles Rust
// exposés par cxx-qt ; aucune logique métier ne doit être ajoutée ici.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Tcc 1.0
import "core"
import "components"
import "pages"

ApplicationWindow {
    id: window

    /// Renseigné par `--tray` (propriétés initiales du moteur QML).
    property bool startInTray: false

    // Chargement paresseux : chaque page n'est instanciée qu'à sa première
    // visite, puis conservée (brouillons/état). Réduit RSS et CPU au démarrage.
    property var pageVisited: [true, false, false, false, false, false, false]
    function markPageVisited(index) {
        if (index < 0 || index >= pageVisited.length || pageVisited[index])
            return;
        var next = pageVisited.slice();
        next[index] = true;
        pageVisited = next;
    }

    width: 1280
    height: 860
    minimumWidth: 980
    minimumHeight: 620
    visible: !startInTray
    title: Strings.t("app.name")
    color: Theme.bg
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fsBody

    // ── Polices embarquées (SIL OFL 1.1) ────────────────────────────────────
    FontLoader { source: "../assets/fonts/TitilliumWeb-Light.ttf" }
    FontLoader { source: "../assets/fonts/TitilliumWeb-Regular.ttf" }
    FontLoader { source: "../assets/fonts/TitilliumWeb-Italic.ttf" }
    FontLoader { source: "../assets/fonts/TitilliumWeb-SemiBold.ttf" }
    FontLoader { source: "../assets/fonts/TitilliumWeb-Bold.ttf" }

    // ── Pont D-Bus Rust → QML ───────────────────────────────────────────────
    // `DaemonClient` reste local à la fenêtre ; les pages lisent `LiveData`.
    // Aucune propriété du daemon n'est écrite ici : ce sont des copies.
    DaemonClient {
        id: daemon
    }

    // ── Tray natif (Qt.labs.platform) ───────────────────────────────────────
    // Chargement tolérant : si le module ou l'hôte SNI manque, la fenêtre
    // garde son comportement normal et une notice est affichée dans Réglages.
    Loader {
        id: trayLoader
        source: "tray/SystemTray.qml"

        onLoaded: {
            AppState.trayLoaded = true;
            if (!AppState.trayAvailable)
                console.warn("tcc-gui: icône tray indisponible (SNI/extension GNOME ?)");
            window.ensureStartupVisible();
        }

        onStatusChanged: {
            if (status === Loader.Error) {
                AppState.trayLoaded = true;
                AppState.trayAvailable = false;
                console.warn("tcc-gui: Qt.labs.platform indisponible; fenêtre normale");
                window.ensureStartupVisible();
            }
        }
    }

    Connections {
        target: trayLoader.item

        function onShowWindowRequested() {
            window.showWindow();
        }

        function onToggleWindowRequested() {
            window.toggleWindow();
        }

        function onQuitRequested() {
            window.quitApp();
        }
    }

    function showWindow() {
        window.show();
        window.raise();
        window.requestActivate();
    }

    function toggleWindow() {
        if (window.visible)
            window.hide();
        else
            window.showWindow();
    }

    function quitApp() {
        AppState.quitting = true;
        Qt.quit();
    }

    function ensureStartupVisible() {
        if (window.startInTray && !AppState.trayAvailable && !window.visible)
            window.show();
    }

    function ensureWindowReachable() {
        // Tray perdu pendant que la fenêtre était masquée : sans icône, il
        // n'existe plus aucun moyen de la réafficher.
        if (!AppState.trayAvailable && !window.visible && !AppState.quitting)
            window.showWindow();
    }

    onVisibleChanged: {
        AppState.windowVisible = window.visible;
        daemon.setWindowVisible(window.visible);
        if (window.visible)
            daemon.refresh();
    }

    onActiveChanged: AppState.windowActive = window.active

    onClosing: function(close) {
        if (AppState.trayAvailable && !AppState.quitting) {
            close.accepted = false;
            window.hide();
        }
    }

    // Filet de sécurité si le Loader n'a pas encore conclu (module lent).
    Timer {
        interval: 750
        repeat: false
        running: window.startInTray && !window.visible && !AppState.trayAvailable
        onTriggered: window.ensureStartupVisible()
    }

    Connections {
        target: daemon

        function onConnectedChanged() {
            LiveData.connected = daemon.connected;
            LiveData.lastError = daemon.lastError;
            HardwareData.connected = daemon.connected;
            SettingsData.connected = daemon.connected;
        }

        function onStateJsonChanged() {
            LiveData.activeProfileId = daemon.activeProfileId;
            LiveData.activeProfileName = daemon.activeProfileName;
            LiveData.ppdProfile = daemon.ppdProfile;
            LiveData.powerSource = daemon.powerSource;
            LiveData.manualOverride = daemon.manualOverride;
        }

        function onProfilesJsonChanged() {
            LiveData.profilesJson = daemon.profilesJson;
        }

        function onLastErrorChanged() {
            LiveData.lastError = daemon.lastError;
        }

        function onBusyChanged() {
            LiveData.commandBusy = daemon.busy;
        }

        function onDaemonVersionChanged() {
            LiveData.daemonVersion = daemon.daemonVersion;
        }

        function onContractVersionChanged() {
            LiveData.contractVersion = daemon.contractVersion;
        }

        function onHardwareJsonChanged() {
            HardwareData.hardwareJson = daemon.hardwareJson;
        }

        function onHardwareErrorChanged() {
            HardwareData.hardwareError = daemon.hardwareError;
        }

        function onSensorsEnabledChanged() {
            HardwareData.sensorsRequested = daemon.sensorsEnabled;
        }

        function onFanJsonChanged() {
            HardwareData.fanJson = daemon.fanJson;
        }

        function onFanErrorChanged() {
            HardwareData.fanError = daemon.fanError;
        }

        function onAdminAvailableChanged() {
            AdminState.available = daemon.adminAvailable;
        }

        function onAdminVersionChanged() {
            AdminState.version = daemon.adminVersion;
        }

        function onAdminErrorChanged() {
            AdminState.error = daemon.adminError;
        }

        function onAdminHwAvailableChanged() {
            AdminHwData.available = daemon.adminHwAvailable;
        }

        function onAdminHwVersionChanged() {
            AdminHwData.version = daemon.adminHwVersion;
        }

        function onAdminHwErrorChanged() {
            AdminHwData.error = daemon.adminHwError;
        }

        function onAdminHwPendingChanged() {
            AdminHwData.pending = daemon.adminHwPending;
        }

        function onAdminHwActionChanged() {
            AdminHwData.action = daemon.adminHwAction;
        }

        function onAdminHwSaveErrorChanged() {
            AdminHwData.saveError = daemon.adminHwSaveError;
        }

        function onAdminHwSaveErrorDetailChanged() {
            AdminHwData.saveErrorDetail = daemon.adminHwSaveErrorDetail;
        }

        function onAdminHwSucceededChanged() {
            AdminHwData.succeeded = daemon.adminHwSucceeded;
        }

        function onAgentAvailableChanged() {
            AgentData.available = daemon.agentAvailable;
        }

        function onAgentVersionChanged() {
            AgentData.version = daemon.agentVersion;
        }

        function onAgentErrorChanged() {
            AgentData.error = daemon.agentError;
        }

        function onDisplayInfoJsonChanged() {
            AgentData.displayInfoJson = daemon.displayInfoJson;
        }

        function onAgentUvcWriteSupportedChanged() {
            AgentData.uvcWriteSupported = daemon.agentUvcWriteSupported;
        }

        function onAgentUvcWritePendingChanged() {
            AgentData.uvcPending = daemon.agentUvcWritePending;
        }

        function onAgentUvcWriteErrorChanged() {
            AgentData.uvcError = daemon.agentUvcWriteError;
        }

        function onAgentUvcWriteSucceededChanged() {
            AgentData.uvcSucceeded = daemon.agentUvcWriteSucceeded;
        }

        function onSavingChanged() {
            AdminState.saving = daemon.saving;
        }

        function onSaveErrorChanged() {
            AdminState.saveError = daemon.saveError;
        }

        function onSaveErrorDetailChanged() {
            AdminState.saveErrorDetail = daemon.saveErrorDetail;
        }

        function onSaveSucceededChanged() {
            AdminState.saveSucceeded = daemon.saveSucceeded;
        }

        function onSaveActionChanged() {
            AdminState.saveAction = daemon.saveAction;
        }

        function onCreatedProfileIdChanged() {
            AdminState.createdProfileId = daemon.createdProfileId;
        }

        function onExportedCountChanged() {
            AdminState.exportedCount = daemon.exportedCount;
        }

        function onImportedCountChanged() {
            AdminState.importedCount = daemon.importedCount;
        }

        function onRenamedCountChanged() {
            AdminState.renamedCount = daemon.renamedCount;
        }

        function onSkippedCountChanged() {
            AdminState.skippedCount = daemon.skippedCount;
        }

        function onSettingsErrorChanged() {
            SettingsData.error = daemon.settingsError;
        }

        function onSettingsFahrenheitChanged() {
            SettingsData.fahrenheit = daemon.settingsFahrenheit;
        }

        function onSettingsCpuEnabledChanged() {
            SettingsData.cpuEnabled = daemon.settingsCpuEnabled;
        }

        function onSettingsFanEnabledChanged() {
            SettingsData.fanEnabled = daemon.settingsFanEnabled;
        }

        function onSettingsLightingEnabledChanged() {
            SettingsData.lightingEnabled = daemon.settingsLightingEnabled;
        }

        function onSettingsPpdOverrideChanged() {
            SettingsData.ppdOverride = daemon.settingsPpdOverride;
        }

        function onSettingsChargingProfileChanged() {
            SettingsData.chargingProfile = daemon.settingsChargingProfile;
        }

        function onSettingsChargingTypeChanged() {
            SettingsData.chargingType = daemon.settingsChargingType;
        }

        function onSettingsKeyboardModeChanged() {
            SettingsData.keyboardMode = daemon.settingsKeyboardMode;
        }

        function onSettingsChargingPriorityChanged() {
            SettingsData.chargingPriority = daemon.settingsChargingPriority;
        }

        function onSettingsChargingStartChanged() {
            SettingsData.chargingStart = daemon.settingsChargingStart;
        }

        function onSettingsChargingEndChanged() {
            SettingsData.chargingEnd = daemon.settingsChargingEnd;
        }

        function onSettingsMatrixPowerSaverAcChanged() {
            SettingsData.matrixPowerSaverAc = daemon.settingsMatrixPowerSaverAc;
        }

        function onSettingsMatrixPowerSaverBatChanged() {
            SettingsData.matrixPowerSaverBat = daemon.settingsMatrixPowerSaverBat;
        }

        function onSettingsMatrixBalancedAcChanged() {
            SettingsData.matrixBalancedAc = daemon.settingsMatrixBalancedAc;
        }

        function onSettingsMatrixBalancedBatChanged() {
            SettingsData.matrixBalancedBat = daemon.settingsMatrixBalancedBat;
        }

        function onSettingsMatrixPerformanceAcChanged() {
            SettingsData.matrixPerformanceAc = daemon.settingsMatrixPerformanceAc;
        }

        function onSettingsMatrixPerformanceBatChanged() {
            SettingsData.matrixPerformanceBat = daemon.settingsMatrixPerformanceBat;
        }

        function onSettingsShutdownTimeChanged() {
            SettingsData.shutdownTime = daemon.settingsShutdownTime;
        }

        function onShutdownTimerAvailableChanged() {
            ShutdownTimerData.available = daemon.shutdownTimerAvailable;
        }

        function onShutdownTimerEnabledChanged() {
            ShutdownTimerData.enabled = daemon.shutdownTimerEnabled;
        }

        function onShutdownTimerScheduledAtChanged() {
            ShutdownTimerData.scheduledAt = daemon.shutdownTimerScheduledAt;
        }

        function onShutdownTimerSourceChanged() {
            ShutdownTimerData.source = daemon.shutdownTimerSource;
        }

        function onShutdownTimerErrorChanged() {
            ShutdownTimerData.error = daemon.shutdownTimerError;
        }

        function onPowerBlockerAvailableChanged() {
            PowerBlockerData.available = daemon.powerBlockerAvailable;
        }

        function onPowerBlockedChanged() {
            PowerBlockerData.blocked = daemon.powerBlocked;
        }

        function onPowerBlockerPendingChanged() {
            PowerBlockerData.pending = daemon.powerBlockerPending;
        }

        function onPowerBlockerErrorChanged() {
            PowerBlockerData.error = daemon.powerBlockerError;
        }

        function onAutostartEnabledChanged() {
            AutostartData.enabled = daemon.autostartEnabled;
        }

        function onAutostartAvailableChanged() {
            AutostartData.available = daemon.autostartAvailable;
        }

        function onAutostartErrorChanged() {
            AutostartData.error = daemon.autostartError;
        }

        function onSavingSettingsChanged() {
            AdminState.savingSettings = daemon.savingSettings;
        }

        function onSaveSettingsErrorChanged() {
            AdminState.saveSettingsError = daemon.saveSettingsError;
        }

        function onSaveSettingsErrorDetailChanged() {
            AdminState.saveSettingsErrorDetail = daemon.saveSettingsErrorDetail;
        }

        function onSaveSettingsSucceededChanged() {
            AdminState.saveSettingsSucceeded = daemon.saveSettingsSucceeded;
        }
    }

    // Traduction des demandes QML en appels D-Bus (jamais l'inverse).
    Connections {
        target: AppState

        function onRefreshRequested() {
            daemon.refresh();
        }

        function onProfileActivationRequested(profileId) {
            if (profileId !== "")
                daemon.setTempProfile(profileId);
        }

        function onTempProfileClearRequested() {
            daemon.clearTempProfile();
        }

        function onSensorCollectionRequested(enabled) {
            daemon.setSensorsEnabled(enabled);
        }

        function onHardwareRefreshRequested() {
            daemon.refreshHardware();
        }

        function onProfileSaveRequested(profileId, editJson) {
            daemon.saveProfile(profileId, editJson);
        }

        function onProfileCreateRequested(editJson) {
            daemon.createProfile(editJson);
        }

        function onProfileDuplicateRequested(profileId) {
            daemon.duplicateProfile(profileId);
        }

        function onProfileDeleteRequested(profileId) {
            daemon.deleteProfile(profileId);
        }

        function onProfileExportRequested(path) {
            daemon.exportProfiles(path);
        }

        function onProfileImportRequested(path) {
            daemon.importProfiles(path);
        }

        function onSettingsSaveRequested(editJson) {
            daemon.saveSettings(editJson);
        }

        function onAdminRefreshRequested() {
            daemon.refreshAdmin();
        }

        function onAgentRefreshRequested() {
            daemon.refreshAgent();
        }

        function onUvcControlRequested(devicePath, controlKey, value) {
            daemon.setUvcControl(devicePath, controlKey, value);
        }

        function onPowerBlockerRequested(blocked) {
            daemon.setPowerBlocker(blocked);
        }

        function onAutostartRequested(enabled) {
            daemon.setAutostart(enabled);
        }

        function onOdmApplyRequested(name, force) {
            daemon.applyOdm(name, force);
        }

        function onTdpApplyRequested(valuesJson) {
            daemon.applyTdp(valuesJson);
        }

        function onCtgpApplyRequested(watts) {
            daemon.applyCtgp(watts);
        }

        function onTrayAvailableChanged() {
            window.ensureWindowReachable();
        }

        function onPageChanged() {
            window.markPageVisited(AppState.pageIndex);
            var dashboard = AppState.page === "dashboard";
            // Le bail capteurs n'est renouvelé que sur le Dashboard ; ailleurs
            // on laisse expirer (aucun `false` envoyé).
            daemon.setSensorPageVisible(dashboard);
            HardwareData.dashboardVisible = dashboard;
            if (dashboard)
                daemon.refreshHardware();
        }
    }

    Component.onCompleted: {
        // Les propriétés calculées par `DaemonClient::default()` (autostart,
        // disponibilité des services) existent avant le QML et n'émettent
        // aucun signal `changed` : pousser l'état initial explicitement.
        BridgeState.pushInitial(daemon);
        var dashboard = AppState.page === "dashboard";
        daemon.setSensorPageVisible(dashboard);
        HardwareData.dashboardVisible = dashboard;
        AppState.windowVisible = window.visible;
        AppState.windowActive = window.active;
        daemon.setWindowVisible(window.visible);
        window.markPageVisited(AppState.pageIndex);
        if (window.startInTray)
            Qt.callLater(window.ensureStartupVisible);
    }

    // ── Liaison préférences → design system ─────────────────────────────────
    Binding {
        target: Theme
        property: "mode"
        value: AppState.themeMode
    }

    Binding {
        target: Theme
        property: "reduceMotion"
        value: AppState.reducedMotion
    }

    Binding {
        target: Strings
        property: "lang"
        value: AppState.language
    }

    Binding {
        target: Strings
        property: "fahrenheit"
        value: AppState.temperatureUnit === "f"
    }

    // Les réglages du daemon font foi : °C/°F est appliqué après relecture.
    Connections {
        target: SettingsData

        function onFahrenheitChanged() {
            if (SettingsData.live)
                AppState.setTemperatureUnit(SettingsData.fahrenheit ? "f" : "c");
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Sidebar {
            Layout.fillHeight: true
            compact: window.width < 1120
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            // Bandeau d'erreur : visible dès que le pont Rust remonte une
            // erreur (daemon absent, bus inaccessible, payload refusé). Le
            // badge de la sidebar indique, lui, la source des données.
            ConnectionBanner {
                Layout.fillWidth: true
                visible: LiveData.lastError !== ""
                title: LiveData.connected
                       ? Strings.t("connection.commandError")
                       : Strings.t("connection.title")
                detail: LiveData.lastError
                busy: LiveData.commandBusy
                onRetryRequested: AppState.requestRefresh()
            }

            StackLayout {
                id: stack
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: AppState.pageIndex

                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[0] || AppState.pageIndex === 0
                    sourceComponent: dashboardPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[1] || AppState.pageIndex === 1
                    sourceComponent: profilesPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[2] || AppState.pageIndex === 2
                    sourceComponent: editorPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[3] || AppState.pageIndex === 3
                    sourceComponent: toolsPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[4] || AppState.pageIndex === 4
                    sourceComponent: settingsPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[5] || AppState.pageIndex === 5
                    sourceComponent: supportPageComponent
                }
                Loader {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    active: window.pageVisited[6] || AppState.pageIndex === 6
                    sourceComponent: infoPageComponent
                }
            }
        }
    }

    // Fondu très court au changement de page (désactivable via reduceMotion).
    NumberAnimation {
        id: pageFade
        target: stack
        property: "opacity"
        from: 0.55
        to: 1
        duration: Theme.durationFast
        easing.type: Theme.easeOut
    }

    Connections {
        target: AppState

        function onPageChanged() {
            pageFade.restart();
        }
    }

    // ── Toasts ──────────────────────────────────────────────────────────────
    Item {
        anchors.fill: parent
        z: 50

        Column {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: Theme.s24
            spacing: Theme.s8
            width: Math.min(400, parent.width - Theme.s48)

            Repeater {
                model: AppState.toasts

                delegate: Toast {
                    required property var modelData
                    width: parent.width
                    toastId: modelData.toastId
                    message: modelData.message
                    kind: modelData.kind
                }
            }
        }
    }

    // ── Raccourcis globaux ──────────────────────────────────────────────────
    Shortcut { sequence: "Ctrl+1"; onActivated: AppState.navigate("dashboard") }
    Shortcut { sequence: "Ctrl+2"; onActivated: AppState.navigate("profiles") }
    Shortcut { sequence: "Ctrl+3"; onActivated: AppState.navigate("tools") }
    Shortcut { sequence: "Ctrl+4"; onActivated: AppState.navigate("settings") }
    Shortcut { sequence: "Ctrl+5"; onActivated: AppState.navigate("support") }
    Shortcut { sequence: "Ctrl+6"; onActivated: AppState.navigate("info") }
    Shortcut { sequence: "Alt+Left"; enabled: AppState.page === "editor"; onActivated: AppState.navigate("profiles") }

    // Composants de pages instanciés à la demande par le StackLayout.
    Component { id: dashboardPageComponent; DashboardPage { } }
    Component { id: profilesPageComponent; ProfilesPage { } }
    Component { id: editorPageComponent; ProfileEditorPage { } }
    Component { id: toolsPageComponent; ToolsPage { } }
    Component { id: settingsPageComponent; SettingsPage { } }
    Component { id: supportPageComponent; SupportPage { } }
    Component { id: infoPageComponent; InfoPage { } }
}
