// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Poussée initiale de l'état du pont Rust vers les singletons QML.
//
// `DaemonClient` calcule plusieurs propriétés **avant** le chargement du QML
// (autostart XDG, disponibilité des services, état du worker). Les signaux
// `*Changed` ne partent que sur modification : sans cette poussée explicite,
// les singletons gardent leurs valeurs par défaut et l'UI affiche un faux état
// (par exemple « Répertoire utilisateur introuvable » alors que l'autostart
// est disponible).
//
// Appelé une seule fois par `Main.qml` à `Component.onCompleted`. Les mises à
// jour suivantes passent exclusivement par les `Connections` de `Main.qml`.
// `daemon` est typé `var` (duck-typing) pour permettre un test QML sans
// instancier le pont C++.

pragma Singleton

import QtQuick

QtObject {
    id: root

    function pushInitial(daemon) {
        if (!daemon)
            return;

        // Autostart XDG : état résolu à la construction du pont Rust, jamais
        // republié ensuite (seul un `setAutostart` le modifie).
        if (typeof daemon.autostartEnabled !== "undefined") {
            AutostartData.enabled = daemon.autostartEnabled;
            AutostartData.available = daemon.autostartAvailable;
            AutostartData.error = daemon.autostartError;
        }

        // Inhibiteur de veille : état du worker logind.
        if (typeof daemon.powerBlockerAvailable !== "undefined") {
            PowerBlockerData.available = daemon.powerBlockerAvailable;
            PowerBlockerData.blocked = daemon.powerBlocked;
            PowerBlockerData.pending = daemon.powerBlockerPending;
            PowerBlockerData.error = daemon.powerBlockerError;
        }

        // Service d'administration `Admin1` et dernier enregistrement.
        if (typeof daemon.adminAvailable !== "undefined") {
            AdminState.available = daemon.adminAvailable;
            AdminState.version = daemon.adminVersion;
            AdminState.error = daemon.adminError;
            AdminState.savingSettings = daemon.savingSettings;
            AdminState.saveSettingsError = daemon.saveSettingsError;
            AdminState.saveSettingsErrorDetail = daemon.saveSettingsErrorDetail;
            AdminState.saveSettingsSucceeded = daemon.saveSettingsSucceeded;
        }

        // Agent de session `User1` (refresh rate, webcam).
        if (typeof daemon.agentAvailable !== "undefined") {
            AgentData.available = daemon.agentAvailable;
            AgentData.version = daemon.agentVersion;
            AgentData.error = daemon.agentError;
            AgentData.displayInfoJson = daemon.displayInfoJson;
            AgentData.uvcWriteSupported = daemon.agentUvcWriteSupported;
            AgentData.uvcPending = daemon.agentUvcWritePending;
            AgentData.uvcError = daemon.agentUvcWriteError;
            AgentData.uvcSucceeded = daemon.agentUvcWriteSucceeded;
        }
    }
}
