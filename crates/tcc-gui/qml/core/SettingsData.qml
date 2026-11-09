// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Réglages globaux lus depuis le daemon (`GetSettingsJSON` → `tcc_core::Settings`).
//
// Le pont Rust désérialise et re-sérialise le modèle : QML ne reçoit que des
// champs typés (aucun `JSON.parse`). Sont modifiables : quatre booléens, la
// charge (profil, priorité, seuils, type), la matrice PPD × alimentation,
// Fn-Lock et l'éclairage (via des DTO dédiés) ; le reste est conservé côté
// Rust et affiché en lecture seule.

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées, poussées par Main.qml ──────────────────────────────────────
    property bool connected: false
    property bool fahrenheit: false
    property bool cpuEnabled: true
    property bool fanEnabled: true
    property bool lightingEnabled: true
    /// Consentement persistant pour que TCC écrive `platform_profile`.
    property bool ppdOverride: false
    /// Charge persistée : `""` / `-1` = non défini (le matériel fait foi).
    property string chargingProfile: ""
    /// Type de charge persisté (`""` = non défini / ne pas changer).
    property string chargingType: ""
    /// Mode de la première zone clavier (`""` = aucune zone).
    property string keyboardMode: ""
    property string chargingPriority: ""
    property int chargingStart: -1
    property int chargingEnd: -1
    /// Matrice PPD × alimentation (`""` = clé absente, fallback AC/BAT).
    property string matrixPowerSaverAc: ""
    property string matrixPowerSaverBat: ""
    property string matrixBalancedAc: ""
    property string matrixBalancedBat: ""
    property string matrixPerformanceAc: ""
    property string matrixPerformanceBat: ""
    /// Heure d'extinction persistée (`""` = aucune, format `HH:MM`).
    property string shutdownTime: ""
    /// Erreur de la dernière lecture (dernier payload conservé).
    property string error: ""

    // ── Sorties ─────────────────────────────────────────────────────────────
    /// Vrai si les valeurs viennent réellement du daemon.
    readonly property bool live: root.connected && root.error === ""
}
