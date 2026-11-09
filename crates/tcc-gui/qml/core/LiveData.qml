// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Adaptateur d'affichage entre le daemon (JSON de `tcc-ipc`, non fiable) et
// les vues QML. Il ne fait QUE de l'adaptation :
//   - `JSON.parse` de `profilesJson` une seule fois par payload, et seulement
//     si le daemon est connecté ;
//   - validation minimale de la forme (tableau, `id`/`name`/`display`/`cpu`/`fan`) ;
//   - normalisation des unités noyau vers le modèle d'affichage (kHz → GHz,
//     sentinelles `-1`, noms de courbes) ;
//   - repli intégral sur `DemoData` si le daemon est absent ou la réponse
//     invalide, pour que le shell reste navigable.
//
// Ce fichier ne contient aucune règle métier : pas de validation de plage, pas
// de calcul de profil, pas d'écriture. Le daemon reste seul juge (cf.
// `docs/ui/UI_DESIGN.md` §15).
//
// Singleton : `Main.qml` y pousse les propriétés du pont Rust `DaemonClient`.
// `AppState` ne le référence pas, ce qui évite un cycle de singletons avec
// `DemoData` (DemoData → AppState pour la page courante).

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées, poussées par Main.qml (jamais lues depuis D-Bus ici) ───────
    property bool connected: false
    property string profilesJson: "[]"
    property string activeProfileId: ""
    property string activeProfileName: ""
    property string powerSource: ""      // ac | battery | unknown | ""
    property string ppdProfile: ""       // power-saver | balanced | performance | ""
    property bool manualOverride: false
    /// Vrai quand le worker D-Bus traite une commande ou a refusé la dernière
    /// (file bornée pleine). Sert à désactiver les boutons « Réessayer ».
    property bool commandBusy: false
    property string lastError: ""
    /// Version du daemon (`GetVersion`) et version du contrat IPC, lues via
    /// le pont Rust. Chaîne vide / 0 = non disponible.
    property string daemonVersion: ""
    property int contractVersion: 0

    // ── Sorties ─────────────────────────────────────────────────────────────
    /// Profils normalisés prêts à afficher (live, ou `DemoData` en repli).
    property var profiles: []
    /// Vrai tant que la liste affichée provient de `DemoData`.
    property bool usingDemo: true
    /// Détail technique du dernier rejet de payload (affiché en infobulle).
    property string parseError: ""

    // Mémo du dernier payload traité : évite de re-parser toutes les 2 s.
    property string _lastInput: ""

    readonly property bool isLive: connected && !usingDemo

    /// Source d'alimentation effective : live si connue, sinon démo.
    readonly property string power: {
        if (root.powerSource !== "")
            return root.powerSource;
        return root.usingDemo ? DemoData.powerSource : "unknown";
    }

    /// Profil PPD effectif : live si connu, sinon démo.
    readonly property string ppd: {
        if (root.ppdProfile !== "")
            return root.ppdProfile;
        return root.usingDemo ? DemoData.ppd : "";
    }

    /// Identifiant actif : le daemon prime, `DemoData` sert de repli.
    readonly property string resolvedActiveId: {
        if (root.activeProfileId !== "")
            return root.activeProfileId;
        return root.usingDemo ? DemoData.activeProfileId : "";
    }

    readonly property var activeProfile: root.profileById(root.resolvedActiveId)

    /// Nom/description du profil actif : la liste prime, le snapshot sert de
    /// repli si l'identifiant n'est pas (encore) dans la liste.
    readonly property string activeName: {
        if (root.activeProfile)
            return root.activeProfile.name;
        return root.activeProfileName;
    }

    readonly property string activeDescription: root.activeProfile
            ? root.activeProfile.description : ""

    /// Profil utilisé en AC pour le PPD courant, d'après la matrice
    /// persistée (`GetSettingsJSON.stateMatrix`). Aucune assignation inventée
    /// quand la matrice est absente ou le daemon déconnecté.
    readonly property var assignedAc: {
        if (root.usingDemo) {
            for (var i = 0; i < root.profiles.length; ++i)
                if (root.profiles[i].assignedAc === true)
                    return root.profiles[i];
            return null;
        }
        if (!SettingsData.live)
            return null;
        var id = root._matrixId(root.ppd, "ac");
        return id === "" ? null : root.profileById(id);
    }

    readonly property var assignedBat: {
        if (root.usingDemo) {
            for (var i = 0; i < root.profiles.length; ++i)
                if (root.profiles[i].assignedBat === true)
                    return root.profiles[i];
            return null;
        }
        if (!SettingsData.live)
            return null;
        var id = root._matrixId(root.ppd, "battery");
        return id === "" ? null : root.profileById(id);
    }

    /// Profils normalisés avant annotation des assignations AC/BAT.
    property var _baseProfiles: []
    readonly property var matrixPpdKeys: ["power-saver", "balanced", "performance"]

    /// Taux visé par le profil actif (0 = aucun changement demandé).
    readonly property real refreshRate: {
        if (root.usingDemo)
            return 0;
        return root.activeProfile && root.activeProfile.refreshRate > 0
                ? root.activeProfile.refreshRate : 0;
    }

    /// Vrai si le profil actif demande réellement un changement de taux.
    readonly property bool useRefRate: {
        if (root.usingDemo)
            return false;
        return root.activeProfile ? root.activeProfile.useRefRate === true : false;
    }

    Component.onCompleted: root.rebuild()
    onConnectedChanged: root.rebuild()
    onProfilesJsonChanged: root.rebuild()

    property Connections settingsConnections: Connections {
        target: SettingsData

        function onLiveChanged() { root._annotateAssignments(); }
        function onMatrixPowerSaverAcChanged() { root._annotateAssignments(); }
        function onMatrixPowerSaverBatChanged() { root._annotateAssignments(); }
        function onMatrixBalancedAcChanged() { root._annotateAssignments(); }
        function onMatrixBalancedBatChanged() { root._annotateAssignments(); }
        function onMatrixPerformanceAcChanged() { root._annotateAssignments(); }
        function onMatrixPerformanceBatChanged() { root._annotateAssignments(); }
    }

    // ── Construction de la liste affichée ───────────────────────────────────
    function rebuild() {
        var key = (root.connected ? "1" : "0") + "|" + root.profilesJson;
        if (key === root._lastInput)
            return;
        root._lastInput = key;

        if (!root.connected) {
            root._applyDemo();
            return;
        }

        var parsed;
        try {
            parsed = JSON.parse(root.profilesJson);
        } catch (error) {
            root._fail("profilesJson: JSON invalide");
            return;
        }
        if (!Array.isArray(parsed) || parsed.length === 0) {
            root._fail("profilesJson: tableau vide ou invalide");
            return;
        }

        var out = [];
        for (var i = 0; i < parsed.length; ++i) {
            var normalized = root._normalize(parsed[i]);
            if (normalized === null) {
                // Payload rejeté en bloc : une liste partielle serait trompeuse.
                root._fail("profilesJson: profil " + i + " invalide");
                return;
            }
            out.push(normalized);
        }

        root._baseProfiles = out;
        root.usingDemo = false;
        root.parseError = "";
        root._annotateAssignments();
    }

    function profileById(id) {
        if (id === undefined || id === null || id === "")
            return null;
        var list = root.profiles;
        for (var i = 0; i < list.length; ++i) {
            if (list[i].id === id)
                return list[i];
        }
        return null;
    }

    function _applyDemo() {
        root._baseProfiles = DemoData.profiles;
        root.profiles = DemoData.profiles;
        root.usingDemo = true;
        root.parseError = "";
    }

    /// Identifiant de profil d'une cellule de la matrice persistée.
    function _matrixId(ppd, power) {
        switch (ppd + "/" + power) {
        case "power-saver/ac":
            return SettingsData.matrixPowerSaverAc;
        case "power-saver/battery":
            return SettingsData.matrixPowerSaverBat;
        case "balanced/ac":
            return SettingsData.matrixBalancedAc;
        case "balanced/battery":
            return SettingsData.matrixBalancedBat;
        case "performance/ac":
            return SettingsData.matrixPerformanceAc;
        case "performance/battery":
            return SettingsData.matrixPerformanceBat;
        default:
            return "";
        }
    }

    function _assignedTo(id, power) {
        for (var i = 0; i < root.matrixPpdKeys.length; ++i) {
            if (root._matrixId(root.matrixPpdKeys[i], power) === id)
                return true;
        }
        return false;
    }

    /// Annote chaque profil avec ses icônes AC/BAT réelles. La matrice peut
    /// assigner un profil à plusieurs PPD ; l'icône indique au moins une
    /// assignation pour la source.
    function _annotateAssignments() {
        if (root.usingDemo || !SettingsData.live) {
            root.profiles = root._baseProfiles;
            return;
        }
        var out = [];
        for (var i = 0; i < root._baseProfiles.length; ++i) {
            var profile = root._baseProfiles[i];
            var copy = {};
            for (var key in profile)
                copy[key] = profile[key];
            copy.assignedAc = root._assignedTo(profile.id, "ac");
            copy.assignedBat = root._assignedTo(profile.id, "battery");
            out.push(copy);
        }
        root.profiles = out;
    }

    function _fail(reason) {
        root._applyDemo();
        root.parseError = reason;
    }

    // ── Normalisation d'un profil `tcc-core::Profile` ───────────────────────
    function _normalize(raw) {
        if (!raw || typeof raw !== "object")
            return null;
        if (typeof raw.id !== "string" || raw.id === "")
            return null;
        if (typeof raw.name !== "string" || raw.name.trim() === "")
            return null;
        if (!root._isObject(raw.display) || !root._isObject(raw.cpu)
                || !root._isObject(raw.fan))
            return null;

        var display = raw.display;
        var cpu = raw.cpu;
        var fan = raw.fan;

        var preset = root.presetIcons[raw.id] !== undefined;
        var maxRaw = cpu.scalingMaxFrequency;
        var freqMaxReduced = maxRaw === -1;

        var tdp = root._isObject(raw.odmPowerLimits)
                && Array.isArray(raw.odmPowerLimits.tdpValues)
                ? raw.odmPowerLimits.tdpValues : [];

        var ctgp = root._isObject(raw.nvidiaPowerCTRLProfile)
                ? root._number(raw.nvidiaPowerCTRLProfile.cTGPOffset, 0) : 0;

        return {
            "id": raw.id,
            "name": raw.name,
            "description": typeof raw.description === "string" ? raw.description : "",
            "preset": preset,
            "icon": preset ? root.presetIcons[raw.id] : "profile_custom",
            // Assignations AC/BAT non exposées par `GetProfilesJSON` : jamais
            // devinées, l'adaptateur les calcule depuis l'état courant.
            "assignedAc": false,
            "assignedBat": false,

            "cores": root._number(cpu.onlineCores, 0),
            "coresKnown": typeof cpu.onlineCores === "number" && cpu.onlineCores > 0,
            "freqMin": root._ghz(cpu.scalingMinFrequency),
            "freqMax": freqMaxReduced ? 0 : root._ghz(maxRaw),
            "freqMaxReduced": freqMaxReduced,
            "governor": typeof cpu.governor === "string" ? cpu.governor : "",
            "epp": typeof cpu.energyPerformancePreference === "string"
                   ? cpu.energyPerformancePreference : "",
            "maxPerfGov": cpu.useMaxPerfGov === true,
            "noTurbo": cpu.noTurbo === true,

            "useBrightness": display.useBrightness === true,
            "brightness": root._number(display.brightness, 0),
            "useRefRate": display.useRefRate === true,
            "refreshRate": root._positive(display.refreshRate),
            "useResolution": display.useResolution === true,
            "width": root._positive(display.xResolution),
            "height": root._positive(display.yResolution),

            "webcam": raw.webcam ? raw.webcam.status === true : false,
            "useWebcam": raw.webcam ? raw.webcam.useStatus === true : false,

            "useFanControl": fan.useControl !== false,
            "fanMode": root._fanMode(fan.fanProfile, fan.useControl),
            "fanMin": root._number(fan.minimumFanspeed, 0),
            "fanMax": root._number(fan.maximumFanspeed, 100),
            "fanOffset": root._number(fan.offsetFanspeed, 0),
            "fanCurve": root._fanCurve(fan.customFanCurve),

            "odm": root._isObject(raw.odmProfile) && typeof raw.odmProfile.name === "string"
                   ? raw.odmProfile.name : "",
            "tdp1": root._number(tdp[0], 0),
            "tdp2": root._number(tdp[1], 0),
            "tdpValues": tdp.map(function(value) {
                return root._number(value, 0);
            }),
            "ctgp": ctgp,

            "live": true
        };
    }

    // Miroir de `ProfileId::is_builtin_preset` (tcc-core). À maintenir aligné :
    // c'est la seule information de « preset » disponible côté affichage.
    readonly property var presetIcons: ({
        "__profile_max_energy_save__": "profile_energysaver",
        "__legacy_powersave_extreme__": "profile_energysaver",
        "__profile_silent__": "profile_quiet3",
        "__legacy_cool_and_breezy__": "profile_breezy",
        "__office__": "profile_default",
        "__legacy_default__": "profile_default",
        "__high_performance__": "profile_performance"
    })

    readonly property var fanModes: ({
        "silent": "silent",
        "quiet": "quiet",
        "balanced": "balanced",
        "performance": "performance",
        "custom": "custom"
    })

    function _fanMode(name, useControl) {
        if (useControl === false)
            return "auto";
        var key = typeof name === "string" ? name.toLowerCase() : "";
        return root.fanModes[key] !== undefined ? root.fanModes[key] : "auto";
    }

    function _fanCurve(curve) {
        var out = [];
        if (!root._isObject(curve) || !Array.isArray(curve.tableCPU))
            return out;
        for (var i = 0; i < curve.tableCPU.length; ++i) {
            var point = curve.tableCPU[i];
            if (!root._isObject(point))
                continue;
            out.push({ "temp": root._number(point.temp, 0),
                       "speed": root._number(point.speed, 0) });
        }
        return out;
    }

    function _isObject(value) {
        return value !== null && typeof value === "object" && !Array.isArray(value);
    }

    function _number(value, fallback) {
        return typeof value === "number" && isFinite(value) ? value : fallback;
    }

    function _positive(value) {
        return typeof value === "number" && isFinite(value) && value > 0 ? value : 0;
    }

    // kHz → GHz, arrondi à 2 décimales (0,8 GHz, 3,5 GHz).
    function _ghz(khz) {
        if (typeof khz !== "number" || !isFinite(khz) || khz <= 0)
            return 0;
        return Math.round(khz / 1e4) / 100;
    }
}
