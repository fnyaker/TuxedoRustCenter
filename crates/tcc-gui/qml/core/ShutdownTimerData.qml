// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Statut réel de la minuterie d'extinction, poussé par `Main.qml` depuis
// `Manager1.GetShutdownTimerJSON` (lecture seule, sans polkit).
//
// Aucune logique d'écriture ici : la planification passe par
// `Settings.shutdownTime` et `Admin1.SaveSettingsJSON`.

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées, poussées par Main.qml ──────────────────────────────────────
    /// Vrai si `GetShutdownTimerJSON` a répondu.
    property bool available: false
    property bool enabled: false
    /// Échéance locale `YYYY-MM-DD HH:MM`, vide si inconnue.
    property string scheduledAt: ""
    /// `settings`, `logind`, `none` ou vide.
    property string source: ""
    /// Erreur générique du daemon ou du transport (jamais un chemin).
    property string error: ""

    // ── Sorties ─────────────────────────────────────────────────────────────
    readonly property bool scheduled: root.available && root.enabled
    /// `source=error` ou erreur logind : l'état n'est pas « annulé ».
    readonly property bool errorState: root.available
                                       && (root.source === "error"
                                           || root.error !== "")

    readonly property string sourceKey: {
        if (root.source === "settings")
            return "tools.timer.sourceSettings";
        if (root.source === "logind")
            return "tools.timer.sourceLogind";
        if (root.source === "none")
            return "tools.timer.sourceNone";
        if (root.source === "error")
            return "tools.timer.sourceError";
        return "tools.timer.sourceUnknown";
    }

    readonly property string sourceText: Strings.t(root.sourceKey)

    readonly property string statusKind: !root.available ? "warn"
                                          : root.errorState ? "error"
                                          : root.enabled ? "ok" : "neutral"

    readonly property string statusText: {
        if (!root.available)
            return Strings.t("tools.timer.unavailable");
        if (root.errorState)
            return Strings.t("tools.timer.statusError");
        if (root.enabled)
            return Strings.tf("tools.timer.statusScheduled", [root.scheduledAt]);
        return Strings.t("tools.timer.statusNone");
    }

    /// Message générique si `source=error` sans détail.
    readonly property string errorText: root.error !== ""
                                        ? root.error
                                        : (root.source === "error"
                                           ? Strings.t("tools.timer.errorGeneric")
                                           : "")
}
