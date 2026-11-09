// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// ⚠ DONNÉES FACTICES DE DÉMONSTRATION ⚠
//
// Ce singleton ne contient AUCUNE vérité métier : il simule les payloads de
// tcc-ipc pour permettre la revue visuelle du shell QML au jalon 1. Il sera
// supprimé et remplacé par les modèles Rust exposés via cxx-qt au jalon 3.
// La collecte simulée ne tourne que lorsque le dashboard est visible (parité
// avec la contrainte « collecte à la demande »).

pragma Singleton
import QtQuick

QtObject {
    id: root

    // ── État global simulé ─────────────────────────────────────────────────
    property string ppd: "balanced"            // power-saver | balanced | performance
    property string powerSource: "ac"           // ac | battery
    property string activeProfileId: "default"
    // Capacités simulées pour le mode démonstration global (badgé).
    property bool killswitchAvailable: true
    property bool odmAvailable: true
    property bool nvidiaAvailable: true
    property bool chargingAvailable: true

    // ── Profils simulés ────────────────────────────────────────────────────
    // Champs alignés sur tcc-core::Profile (camelCase de l'original conservé).
    readonly property var profiles: [
        {
            "id": "default", "name": "Default", "preset": true,
            "description": "Performance maximale, au prix du bruit et des températures.",
            "icon": "profile_default", "assignedAc": true, "assignedBat": false,
            "cores": 8, "freqMin": 0.8, "freqMax": 3.5, "fanMode": "performance",
            "brightness": 100, "refreshRate": 144, "webcam": true,
            "governor": "performance", "epp": "performance", "noTurbo": false,
            "fanMin": 0, "fanMax": 100, "fanOffset": 0,
            "odm": "performance", "tdp1": 45, "tdp2": 65, "ctgp": 15
        },
        {
            "id": "breezy", "name": "Cool and breezy", "preset": true,
            "description": "Réduit la puissance au profit de basses températures et d'un ventilateur discret.",
            "icon": "profile_breezy", "assignedAc": false, "assignedBat": false,
            "cores": 8, "freqMin": 0.8, "freqMax": 1.8, "fanMode": "quiet",
            "brightness": 100, "refreshRate": 60, "webcam": true,
            "governor": "powersave", "epp": "balance_power", "noTurbo": true,
            "fanMin": 0, "fanMax": 70, "fanOffset": 0,
            "odm": "quiet", "tdp1": 25, "tdp2": 35, "ctgp": 0
        },
        {
            "id": "powersave", "name": "Powersave extreme", "preset": true,
            "description": "Réduction forte de la puissance pour une consommation minimale et un refroidissement silencieux.",
            "icon": "profile_energysaver", "assignedAc": false, "assignedBat": true,
            "cores": 4, "freqMin": 0.8, "freqMax": 1.2, "fanMode": "silent",
            "brightness": 60, "refreshRate": 60, "webcam": false,
            "governor": "powersave", "epp": "power", "noTurbo": true,
            "fanMin": 0, "fanMax": 50, "fanOffset": 0,
            "odm": "power_save", "tdp1": 15, "tdp2": 20, "ctgp": 0
        },
        {
            "id": "studio", "name": "Studio", "preset": false,
            "description": "Ventilateurs modérés et fréquence stable pour l'enregistrement audio.",
            "icon": "profile_custom", "assignedAc": false, "assignedBat": false,
            "cores": 8, "freqMin": 1.2, "freqMax": 3.2, "fanMode": "balanced",
            "brightness": 80, "refreshRate": 120, "webcam": true,
            "governor": "schedutil", "epp": "balance_performance", "noTurbo": false,
            "fanMin": 20, "fanMax": 60, "fanOffset": 5,
            "odm": "balanced", "tdp1": 35, "tdp2": 50, "ctgp": 5
        },
        {
            "id": "gaming", "name": "Gaming", "preset": false,
            "description": "Puissance maximale, ventilateurs agressifs, écran 144 Hz.",
            "icon": "profile_performance", "assignedAc": false, "assignedBat": false,
            "cores": 8, "freqMin": 2.2, "freqMax": 4.4, "fanMode": "performance",
            "brightness": 100, "refreshRate": 144, "webcam": false,
            "governor": "performance", "epp": "performance", "noTurbo": false,
            "fanMin": 40, "fanMax": 100, "fanOffset": 10,
            "odm": "overboost", "tdp1": 55, "tdp2": 75, "ctgp": 20
        },
        {
            "id": "office", "name": "Office silencieux", "preset": false,
            "description": "Bureautique silencieuse sur batterie, sans compromis sur la réactivité.",
            "icon": "profile_quiet3", "assignedAc": false, "assignedBat": false,
            "cores": 6, "freqMin": 0.8, "freqMax": 2.4, "fanMode": "quiet",
            "brightness": 70, "refreshRate": 60, "webcam": false,
            "governor": "schedutil", "epp": "balance_power", "noTurbo": true,
            "fanMin": 10, "fanMax": 55, "fanOffset": 0,
            "odm": "quiet", "tdp1": 22, "tdp2": 30, "ctgp": 0
        },
        {
            "id": "presentation", "name": "Présentation", "preset": false,
            "description": "Écran lumineux, webcam active, ventilateurs discrets.",
            "icon": "profile_custom", "assignedAc": false, "assignedBat": false,
            "cores": 8, "freqMin": 1.0, "freqMax": 2.8, "fanMode": "quiet",
            "brightness": 100, "refreshRate": 60, "webcam": true,
            "governor": "schedutil", "epp": "balance_power", "noTurbo": false,
            "fanMin": 0, "fanMax": 45, "fanOffset": 0,
            "odm": "balanced", "tdp1": 25, "tdp2": 35, "ctgp": 0
        }
    ]

    function profileById(id) {
        for (var i = 0; i < profiles.length; ++i)
            if (profiles[i].id === id)
                return profiles[i];
        return profiles[0];
    }

    readonly property var fanCurve: [
        { "temp": 30, "speed": 0 },
        { "temp": 45, "speed": 25 },
        { "temp": 55, "speed": 40 },
        { "temp": 70, "speed": 65 },
        { "temp": 80, "speed": 85 },
        { "temp": 92, "speed": 100 }
    ]

    // ── Capteurs simulés ───────────────────────────────────────────────────
    property real cpuTemp: 56
    property real cpuFreq: 3.42
    property real cpuFan: 42
    property real cpuPower: 28.5
    property real igpuTemp: 54
    property real igpuFreq: 1.1
    property real dgpuTemp: 61
    property real dgpuFreq: 1.35
    property real dgpuPower: 42
    property real dgpuFan: -1        // -1 = capteur non exposé
    property var cpuHistory: []
    property var dgpuHistory: []

    readonly property bool dgpuPresent: true
    readonly property bool dgpuFanAvailable: false

    Component.onCompleted: {
        cpuHistory = series(56, 4, 48);
        dgpuHistory = series(61, 6, 48);
    }

    function series(base, amplitude, count) {
        var out = [];
        var v = base;
        for (var i = 0; i < count; ++i) {
            v += (Math.random() - 0.5) * amplitude;
            v = Math.max(base - amplitude * 2, Math.min(base + amplitude * 2, v));
            out.push(Math.round(v * 10) / 10);
        }
        return out;
    }

    function pushHistory(list, value) {
        var out = list.slice(Math.max(0, list.length - 47));
        out.push(Math.round(value * 10) / 10);
        return out;
    }

    // Rafraîchissement factice, uniquement quand le dashboard est visible.
    property Timer ticker: Timer {
        interval: 1000
        repeat: true
        running: AppState.page === "dashboard" && AppState.windowVisible
        onTriggered: root._step()
    }

    function _step() {
        cpuTemp = clamp(cpuTemp + (Math.random() - 0.5) * 1.6, 38, 88);
        cpuFreq = clamp(cpuFreq + (Math.random() - 0.5) * 0.22, 0.8, 4.4);
        cpuFan = clamp(cpuFan + (Math.random() - 0.5) * 4, 0, 100);
        cpuPower = clamp(cpuPower + (Math.random() - 0.5) * 2.4, 8, 62);
        igpuTemp = clamp(igpuTemp + (Math.random() - 0.5) * 1.2, 38, 80);
        dgpuTemp = clamp(dgpuTemp + (Math.random() - 0.5) * 1.8, 40, 90);
        dgpuFreq = clamp(dgpuFreq + (Math.random() - 0.5) * 0.1, 0.4, 2.1);
        dgpuPower = clamp(dgpuPower + (Math.random() - 0.5) * 3, 5, 80);
        cpuHistory = pushHistory(cpuHistory, cpuTemp);
        dgpuHistory = pushHistory(dgpuHistory, dgpuTemp);
    }

    function clamp(v, lo, hi) {
        return Math.max(lo, Math.min(hi, v));
    }

}
