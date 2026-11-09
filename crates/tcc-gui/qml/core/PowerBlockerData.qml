// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Inhibiteur de veille « idle » (logind, non privilégié). Poussé par
// `Main.qml` depuis le worker Rust.
//
// Portée : actif uniquement tant que la GUI tourne. Aucune persistance, aucun
// réglage écrit : fermer TuxedoRustCenter libère l'inhibiteur.
// `idle`/`block` ne bloque pas une veille demandée explicitement.

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées, poussées par Main.qml ──────────────────────────────────────
    /// Session graphique détectée (`XDG_SESSION_TYPE`).
    property bool available: false
    /// Un fd d'inhibiteur est détenu par le worker.
    property bool blocked: false
    /// Commande en cours vers le worker.
    property bool pending: false
    /// Code stable : `""`, `NotGraphical`, `Unavailable`, `Refused`, `Timeout`.
    property string error: ""

    // ── Sorties ─────────────────────────────────────────────────────────────
    readonly property bool editable: root.available && !root.pending
    readonly property bool hasError: root.error !== ""

    readonly property string errorKey: {
        switch (root.error) {
        case "":
            return "";
        case "NotGraphical":
            return "settings.powerBlockerNotGraphical";
        case "Refused":
            return "settings.powerBlockerRefused";
        case "Timeout":
            return "settings.powerBlockerTimeout";
        case "Unavailable":
            return "settings.powerBlockerUnavailable";
        case "Busy":
            return "settings.powerBlockerQueueFull";
        default:
            return "settings.powerBlockerError";
        }
    }

    readonly property string errorText: root.errorKey === ""
                                        ? "" : Strings.t(root.errorKey)
}
