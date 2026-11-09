// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Icône tray native : `SystemTrayIcon` de Qt.labs.platform (SNI sous Linux).
// Ce fichier est chargé par un `Loader` tolérant : si le module Qt Labs
// Platform n'est pas installé, Main.qml conserve une fenêtre normale.
//
// Aucune logique métier ici : les actions émettent des signaux vers Main.qml
// ou passent par `AppState`/`LiveData`/`PowerBlockerData`/`AutostartData`.

pragma ComponentBehavior: Bound

import QtQuick
import Qt.labs.platform 1.1
import QtQml.Models
import "../core"

Item {
    id: tray

    visible: false
    width: 0
    height: 0

    signal showWindowRequested()
    signal toggleWindowRequested()
    signal quitRequested()

    /// Disponibilité SNI/plateforme (`SystemTrayIcon.available`).
    readonly property alias available: icon.available
    /// Panneau compact ancré (fenêtre Qt.Popup sans décoration).
    readonly property alias panelVisible: trayPanel.visible
    /// Toute tentative d'activation de profil force la reconstruction du menu
    /// afin de resynchroniser l'état coché même si la demande est refusée.
    property int profileAttempt: 0
    readonly property bool fnLockVisible: HardwareData.visibleIfAvailable(
        HardwareData.fnLock.supported)
    readonly property bool graphicsVisible: HardwareData.visibleIfAvailable(
        HardwareData.prime.supported)

    // `SystemTrayIcon.available` est une propriété constante (pas de signal
    // `availableChanged` dans Qt Labs Platform) : un Binding maintient l'état
    // QML synchronisé à la création. Une disparition de l'hôte SNI en cours de
    // vie n'est pas observable par cette API.
    Binding {
        target: AppState
        property: "trayAvailable"
        value: tray.available
    }

    /// Intention locale du blocage de veille, révertée sur refus.
    property bool blockerDraft: PowerBlockerData.blocked
    /// Intention locale de l'autostart, révertée si l'écriture échoue.
    property bool autostartChecked: AutostartData.enabled
    /// Fn-Lock : vrai seulement si le matériel accepte l'écriture et que son
    /// état est lisible. Sinon l'entrée reste informative.
    readonly property bool fnLockWritable: HardwareData.fnLock.supported
            && HardwareData.fnLock.writable
            && HardwareData.fnLock.enabledKnown
    property bool fnLockDraft: HardwareData.fnLock.enabled
    property bool fnLockSubmitting: false

    function activateProfile(profileId, showWindow) {
        // Le menu natif bascule `checked` avant le retour du daemon : une
        // reconstruction restaure l'état réel en cas de refus (T2-06).
        tray.profileAttempt += 1;
        if (LiveData.usingDemo) {
            DemoData.activeProfileId = profileId;
            AppState.manualProfileId = profileId;
        } else {
            AppState.requestProfileActivation(profileId);
        }
        if (showWindow !== false)
            tray.showWindowRequested();
    }

    function togglePanel() {
        if (trayPanel.visible) {
            trayPanel.close();
            return;
        }
        trayPanel.show();
        trayPanel.requestActivate();
    }

    function syncFnLock() {
        if (HardwareData.fnLock.enabledKnown) {
            tray.fnLockDraft = HardwareData.fnLock.enabled;
        }
    }

    /// Même DTO borné que Réglages ; seul `fnLockStatus` change.
    function fnLockEdit(value) {
        return {
            "fahrenheit": SettingsData.fahrenheit,
            "cpuSettingsEnabled": SettingsData.cpuEnabled,
            "fanControlEnabled": SettingsData.fanEnabled,
            "keyboardBacklightControlEnabled": SettingsData.lightingEnabled,
            "chargingProfile": null,
            "chargingPriority": null,
            "chargingStartThreshold": null,
            "chargingEndThreshold": null,
            "chargingType": null,
            "fnLockStatus": value,
            "stateMatrix": null,
            "shutdownTime": null,
            "clearShutdownTime": false,
            "ppdOverride": null
        };
    }

    function submitFnLock(value) {
        if (!tray.fnLockWritable || !AdminState.available
                || AdminState.savingSettings)
            return;
        tray.fnLockDraft = value;
        tray.fnLockSubmitting = true;
        AppState.requestSettingsSave(JSON.stringify(tray.fnLockEdit(value)));
    }

    function primeLabel(mode) {
        if (mode === "iGPU" || mode === "igpu")
            return Strings.t("settings.graphics.igpu");
        if (mode === "dGPU" || mode === "dgpu")
            return Strings.t("settings.graphics.dgpu");
        if (mode === "on-demand" || mode === "ondemand")
            return Strings.t("settings.graphics.ondemand");
        if (mode === "off")
            return Strings.t("settings.graphics.off");
        return Strings.t("settings.ppdUnknown");
    }

    TrayPanel {
        id: trayPanel

        profiles: LiveData.profiles
        activeProfileId: LiveData.resolvedActiveId
        activeProfileName: LiveData.activeName
        activeProfileDescription: LiveData.activeDescription
        ppd: LiveData.ppd
        power: LiveData.power
        fnLockVisible: tray.fnLockVisible
        fnLockChecked: tray.fnLockDraft
        fnLockEnabled: tray.fnLockWritable && HardwareData.fnLock.enabledKnown
        blockerChecked: tray.blockerDraft
        blockerSupported: PowerBlockerData.editable
        autostartChecked: tray.autostartChecked
        autostartSupported: AutostartData.editable

        onOpenWindowRequested: {
            trayPanel.close();
            tray.showWindowRequested();
        }

        onQuitRequested: {
            trayPanel.close();
            tray.quitRequested();
        }

        onProfileRequested: function(profileId) {
            tray.activateProfile(profileId, false);
        }

        onFnLockRequested: function(value) {
            tray.submitFnLock(value);
        }

        onBlockerRequested: function(value) {
            tray.blockerDraft = value;
            AppState.requestPowerBlocker(value);
        }

        onAutostartRequested: function(value) {
            tray.autostartChecked = value;
            AppState.requestAutostart(value);
        }
    }

    SystemTrayIcon {
        id: icon
        objectName: "trayIcon"

        icon.source: Qt.resolvedUrl("../../assets/brand/taskbar_icon_free.svg")
        tooltip: LiveData.activeName !== ""
                 ? Strings.tf("tray.tooltip", [LiveData.activeName])
                 : Strings.t("app.name")
        visible: available

        onActivated: function(reason) {
            // Clic gauche : panneau compact. Le menu natif reste attaché pour
            // le clic droit (selon le support SNI du compositeur).
            if (reason === SystemTrayIcon.Trigger
                    || reason === SystemTrayIcon.DoubleClick)
                tray.togglePanel();
        }

        menu: Menu {
            id: trayMenu

            MenuItem {
                objectName: "openMenuItem"
                text: Strings.t("tray.open")
                onTriggered: {
                    AppState.navigate("dashboard");
                    tray.showWindowRequested();
                }
            }

            MenuSeparator { }

            Menu {
                id: profileMenu
                objectName: "profileMenu"
                title: Strings.t("tray.profiles")

                MenuItemGroup {
                    id: profileGroup
                    exclusive: true
                }

                Instantiator {
                    // Reconstruit les entrées quand le profil actif change :
                    // cela réapplique l'état coché après activation.
                    model: {
                        // `profileAttempt` force la reconstruction à chaque
                        // clic, y compris si l'activation est refusée. La clé
                        // est incluse dans le modèle pour que la dépendance
                        // soit réellement observée.
                        var attempt = tray.profileAttempt;
                        return LiveData.profiles.map(function(profile) {
                            return {
                                "id": profile.id,
                                "name": profile.name,
                                "attempt": attempt,
                                "active": LiveData.resolvedActiveId === profile.id
                            };
                        });
                    }

                    delegate: MenuItem {
                        required property var modelData

                        text: modelData.name
                        checkable: true
                        checked: modelData.active
                        group: profileGroup
                        onTriggered: tray.activateProfile(modelData.id)
                    }

                    onObjectAdded: function(index, object) {
                        profileMenu.insertItem(index, object);
                    }
                    onObjectRemoved: function(index, object) {
                        profileMenu.removeItem(object);
                    }
                }
            }

            MenuSeparator { }

            MenuItem {
                id: autostartItem
                objectName: "autostartMenuItem"

                text: Strings.t("tray.autostart")
                checkable: true
                checked: tray.autostartChecked
                enabled: AutostartData.editable
                onTriggered: {
                    AppState.requestAutostart(!AutostartData.enabled);
                    // Écriture synchrone : en cas d'échec, l'état réel fait foi.
                    tray.autostartChecked = AutostartData.enabled;
                    autostartItem.checked = AutostartData.enabled;
                }
            }

            MenuItem {
                id: blockerItem
                objectName: "blockerMenuItem"

                text: Strings.t("tray.blocker")
                checkable: true
                checked: tray.blockerDraft
                enabled: PowerBlockerData.editable
                onTriggered: {
                    tray.blockerDraft = !tray.blockerDraft;
                    AppState.requestPowerBlocker(tray.blockerDraft);
                }
            }

            MenuSeparator {
                visible: tray.fnLockVisible || tray.graphicsVisible
            }

            MenuItem {
                id: fnLockItem
                objectName: "fnLockMenuItem"
                visible: tray.fnLockVisible
                checkable: tray.fnLockWritable
                checked: tray.fnLockDraft
                text: HardwareData.fnLock.enabledKnown
                      ? Strings.tf("tray.fnLockState",
                                   [HardwareData.fnLock.enabled
                                    ? Strings.t("common.yes")
                                    : Strings.t("common.no")])
                      : Strings.t("tray.fnLockUnsupported")
                enabled: tray.fnLockWritable && AdminState.available
                         && !AdminState.savingSettings
                onTriggered: tray.submitFnLock(checked)
            }

            MenuItem {
                objectName: "graphicsMenuItem"
                visible: tray.graphicsVisible
                text: Strings.tf("tray.graphicsMode",
                                 [tray.primeLabel(HardwareData.prime.mode)])
                enabled: false
            }

            MenuSeparator { }

            MenuItem {
                text: "TuxedoRustCenter " + Qt.application.version
                enabled: false
            }

            MenuItem {
                objectName: "quitMenuItem"
                text: Strings.t("tray.quit")
                onTriggered: tray.quitRequested()
            }
        }
    }

    Connections {
        target: PowerBlockerData

        function onBlockedChanged() {
            if (!PowerBlockerData.pending) {
                tray.blockerDraft = PowerBlockerData.blocked;
                blockerItem.checked = PowerBlockerData.blocked;
            }
        }

        function onPendingChanged() {
            if (!PowerBlockerData.pending) {
                tray.blockerDraft = PowerBlockerData.blocked;
                blockerItem.checked = PowerBlockerData.blocked;
            }
        }
    }

    Connections {
        target: AutostartData

        function onEnabledChanged() {
            tray.autostartChecked = AutostartData.enabled;
            autostartItem.checked = AutostartData.enabled;
        }

        function onErrorChanged() {
            // Un refus d'écriture ne change pas `enabled` : on réapplique.
            autostartItem.checked = AutostartData.enabled;
        }
    }

    Connections {
        target: HardwareData

        function onFnLockChanged() {
            if (!tray.fnLockSubmitting) {
                tray.syncFnLock();
                fnLockItem.checked = tray.fnLockDraft;
            }
        }
    }

    Connections {
        target: AdminState

        function onSaveSettingsErrorChanged() {
            if (AdminState.saveSettingsError === "" || !tray.fnLockSubmitting)
                return;
            tray.fnLockSubmitting = false;
            tray.syncFnLock();
            fnLockItem.checked = tray.fnLockDraft;
            AppState.notify(Strings.t("tray.fnLockError"), "error");
        }

        function onSaveSettingsSucceededChanged() {
            if (!AdminState.saveSettingsSucceeded || !tray.fnLockSubmitting)
                return;
            tray.fnLockSubmitting = false;
            tray.syncFnLock();
            fnLockItem.checked = tray.fnLockDraft;
            AppState.notify(Strings.t("tray.fnLockSaved"), "ok");
        }
    }
}
