// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Adaptateur du service de session `User1` (`GetDisplayInfoJSON`,
// `GetUvcWriteSupported`). Le JSON est déjà validé et re-sérialisé par Rust
// (`display_view.rs`) ; ce fichier ne fait que normaliser pour l'affichage et
// proposer des modes **réellement exposés** par le compositeur.
//
// L'écriture UVC est pilotée par `ToolsPage` (signal `AppState`) : cet adaptateur
// ne fait que refléter la capacité, l'attente et le code d'erreur publiés par
// `DaemonClient`. Aucune écriture, aucune commande shell.
//
// `internalOutput.modes` ne contient que l'écran interne : un mode externe ne
// peut jamais être proposé ici.

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées poussées par Main.qml ───────────────────────────────────────
    property bool available: false
    property string version: ""
    /// `""`, `Unavailable`, `Contract`, `Transport`, `Timeout`,
    /// `InvalidPayload`, `PayloadTooLarge`.
    property string error: ""
    property string displayInfoJson: "{}"
    /// Écriture UVC exposée par l'agent (`GetUvcWriteSupported`).
    property bool uvcWriteSupported: false
    /// Une écriture UVC est en cours (un seul appel à la fois).
    property bool uvcPending: false
    /// Code stable de la dernière écriture (`""`, `Busy`, `Stalled`, …).
    property string uvcError: ""
    /// Dernière écriture acceptée (valeur écrite ou déjà courante).
    property bool uvcSucceeded: false

    // ── Sorties normalisées ─────────────────────────────────────────────────
    property string parseError: ""
    property var display: root._emptyDisplay()
    property string _lastInput: ""

    readonly property bool usable: root.available && root.display.available
                                   && root.display.supported
                                   && root.display.output.hasInternalDisplay
    readonly property bool hasModes: root.modes.length > 0
    readonly property var modes: root.display.output.modes
    readonly property var currentMode: root.display.output.currentMode
    readonly property real currentRate: root.currentMode
                                        ? root.currentMode.refreshHz : 0
    readonly property string currentModeLabel: root.currentMode
            ? root.currentMode.width + " × " + root.currentMode.height
              + " @ " + Strings.formatHz(root.currentMode.refreshHz)
            : ""

    /// L'agent est installé/activé mais endormi (nom activable seulement).
    readonly property bool dormant: root.available && !root.display.available
    readonly property string errorKey: root.keyFor(root.error)
    readonly property string errorText: root.errorKey === ""
                                        ? "" : Strings.t(root.errorKey)
    readonly property string displayErrorText: {
        var raw = root.display.error;
        if (raw === "")
            return "";
        return Strings.t("display.agentErrorDetail");
    }

    /// Message localisé de la dernière écriture UVC (jamais le texte serveur).
    readonly property string uvcErrorText: root.uvcError === ""
                                           ? "" : Strings.t(root.uvcErrorKey())

    Component.onCompleted: root.rebuild()
    onDisplayInfoJsonChanged: root.rebuild()

    function _emptyDisplay() {
        return {
            "available": false,
            "supported": false,
            "dryRun": false,
            "error": "",
            "session": { "type": "", "desktop": "", "waylandDisplay": "" },
            "backend": { "kind": "", "name": "", "available": false },
            "output": {
                "name": "",
                "connected": false,
                "enabled": false,
                "hasInternalDisplay": false,
                "currentMode": null,
                "modes": []
            }
        };
    }

    function rebuild() {
        if (root.displayInfoJson === root._lastInput)
            return;
        root._lastInput = root.displayInfoJson;

        var parsed;
        try {
            parsed = JSON.parse(root.displayInfoJson);
        } catch (error) {
            root._fail("displayInfoJson: JSON invalide");
            return;
        }
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
            root._fail("displayInfoJson: forme inattendue");
            return;
        }
        root.display = root._normalize(parsed);
        root.parseError = "";
    }

    function _fail(reason) {
        root.display = root._emptyDisplay();
        root.parseError = reason;
    }

    function _normalize(data) {
        var session = data.session || {};
        var backend = data.backend || {};
        var output = data.internalOutput || {};
        var modes = root._modes(output.modes);
        var current = root._mode(output.currentMode);
        if (current && !root._containsMode(modes, current.width, current.height,
                                           current.refreshHz)) {
            modes.push(current);
        }
        modes.sort(function(a, b) {
            if (a.width !== b.width)
                return b.width - a.width;
            if (a.height !== b.height)
                return b.height - a.height;
            return b.refreshHz - a.refreshHz;
        });
        return {
            "available": data.available === true,
            "supported": data.supported === true,
            "dryRun": data.dryRun === true,
            "error": root._text(data.error, 256),
            "session": {
                "type": root._text(session.type, 32),
                "desktop": root._text(session.desktop, 64),
                "waylandDisplay": root._text(session.waylandDisplay, 128)
            },
            "backend": {
                "kind": root._text(backend.kind, 32),
                "name": root._text(backend.name, 64),
                "available": backend.available === true
            },
            "output": {
                "name": root._text(output.name, 64),
                "connected": output.connected === true,
                "enabled": output.enabled === true,
                "hasInternalDisplay": output.hasInternalDisplay === true,
                "currentMode": current,
                "modes": modes
            }
        };
    }

    function _modes(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && out.length < 256; ++i) {
            var mode = root._mode(list[i]);
            if (mode)
                out.push(mode);
        }
        return out;
    }

    function _mode(value) {
        if (!value || typeof value !== "object")
            return null;
        var width = root._int(value.width, 0);
        var height = root._int(value.height, 0);
        var rate = typeof value.refreshHz === "number" && isFinite(value.refreshHz)
                   ? value.refreshHz : 0;
        if (width < 1 || width > 16384 || height < 1 || height > 16384
                || rate < 1 || rate > 1000)
            return null;
        return {
            "width": width,
            "height": height,
            "refreshHz": Math.round(rate * 10) / 10,
            "preferred": value.preferred === true,
            "current": value.current === true
        };
    }

    function _containsMode(list, width, height, rate) {
        for (var i = 0; i < list.length; ++i) {
            if (list[i].width === width && list[i].height === height
                    && Math.abs(list[i].refreshHz - rate) <= 0.5)
                return true;
        }
        return false;
    }

    // ── Modes proposables ───────────────────────────────────────────────────
    /// Résolutions uniques `{width, height, label}`, triées par surface.
    function resolutions() {
        var seen = {};
        var out = [];
        for (var i = 0; i < root.modes.length; ++i) {
            var mode = root.modes[i];
            var key = mode.width + "x" + mode.height;
            if (seen[key])
                continue;
            seen[key] = true;
            out.push({
                "width": mode.width,
                "height": mode.height,
                "label": Strings.tf("display.resolutionFormat",
                                    [mode.width, mode.height])
            });
        }
        return out;
    }

    function resolutionIndex(width, height) {
        var list = root.resolutions();
        for (var i = 0; i < list.length; ++i) {
            if (list[i].width === width && list[i].height === height)
                return i;
        }
        return -1;
    }

    function resolutionExists(width, height) {
        return root.resolutionIndex(width, height) >= 0;
    }

    /// Taux uniques d'une résolution, triés décroissants.
    function refreshRates(width, height) {
        var seen = {};
        var out = [];
        for (var i = 0; i < root.modes.length; ++i) {
            var mode = root.modes[i];
            if (mode.width !== width || mode.height !== height)
                continue;
            var key = String(mode.refreshHz);
            if (seen[key])
                continue;
            seen[key] = true;
            out.push(mode.refreshHz);
        }
        out.sort(function(a, b) { return b - a; });
        return out;
    }

    function refreshIndex(width, height, rate) {
        var list = root.refreshRates(width, height);
        for (var i = 0; i < list.length; ++i) {
            if (Math.abs(list[i] - rate) <= 0.5)
                return i;
        }
        return -1;
    }

    function modeExists(width, height, rate) {
        return root.refreshIndex(width, height, rate) >= 0;
    }

    /// Résolution par défaut : mode courant, sinon la plus grande.
    function defaultResolution() {
        var list = root.resolutions();
        if (list.length === 0)
            return null;
        if (root.currentMode) {
            var index = root.resolutionIndex(root.currentMode.width,
                                             root.currentMode.height);
            if (index >= 0)
                return list[index];
        }
        return list[0];
    }

    // ── Erreurs ─────────────────────────────────────────────────────────────
    function uvcErrorKey() {
        switch (root.uvcError) {
        case "":
            return "";
        case "Busy":
            return "tools.webcam.uvcBusy";
        case "Stalled":
            return "tools.webcam.uvcStalled";
        case "OutOfRange":
            return "tools.webcam.uvcOutOfRange";
        case "NotSupported":
            return "tools.webcam.uvcNotSupported";
        case "InvalidArgs":
            return "tools.webcam.uvcInvalidArgs";
        case "LimitsExceeded":
            return "tools.webcam.uvcLimits";
        case "Unavailable":
            return "tools.webcam.uvcUnavailable";
        case "Transport":
            return "tools.webcam.uvcTransport";
        default:
            return "tools.webcam.uvcFailed";
        }
    }

    function keyFor(code) {
        switch (code) {
        case "":
            return "";
        case "Unavailable":
            return "display.agentUnavailable";
        case "Contract":
            return "display.agentContract";
        case "Transport":
            return "display.agentTransport";
        case "Timeout":
            return "display.agentTimeout";
        case "InvalidPayload":
        case "PayloadTooLarge":
            return "display.agentInvalidPayload";
        default:
            return "display.agentUnavailable";
        }
    }

    function _text(value, max) {
        if (typeof value !== "string")
            return "";
        var out = "";
        for (var i = 0; i < value.length && out.length < max; ++i) {
            var code = value.charCodeAt(i);
            if (code >= 32 && code !== 127)
                out += value.charAt(i);
        }
        return out.trim();
    }

    function _int(value, fallback) {
        return typeof value === "number" && isFinite(value)
                ? Math.round(value) : fallback;
    }
}
