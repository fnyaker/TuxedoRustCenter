// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// État du service d'administration (`Admin1`, phase 1) et de l'enregistrement
// de profils. Poussé par `Main.qml` depuis le pont Rust ; aucune logique
// d'écriture ici : seuls les codes d'erreur sont traduits.
//
// Codes :
//   - `error` : `""` | `Unavailable` | `Contract` | `Transport` ;
//   - `saveError` : code stable renvoyé par le pont (`NotAuthorized`, `Busy`,
//     `Validation`, `InUse`, `ReloadFailed`, `WriteFailed`, `RateLimited`,
//     `PayloadTooLarge`, `AdminUnavailable`, `Timeout`, `Transport`, …).

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées ─────────────────────────────────────────────────────────────
    property bool available: false
    property string version: ""
    property string error: ""
    property bool saving: false
    property string saveError: ""
    property string saveErrorDetail: ""
    property bool saveSucceeded: false
    /// Dernière action de collection : `save`, `create`, `duplicate`, `delete`,
    /// `export`, `import`.
    property string saveAction: ""
    /// ID du profil créé/dupliqué (sélection de l'éditeur).
    property string createdProfileId: ""
    /// Résultats du dernier export/import (compteurs remis à zéro à chaque
    /// nouvelle demande par le pont).
    property int exportedCount: 0
    property int importedCount: 0
    property int renamedCount: 0
    property int skippedCount: 0
    property bool savingSettings: false
    property string saveSettingsError: ""
    property string saveSettingsErrorDetail: ""
    property bool saveSettingsSucceeded: false

    // ── Sorties ─────────────────────────────────────────────────────────────
    /// Vrai si un enregistrement peut être demandé maintenant.
    readonly property bool ready: root.available && !root.saving
    readonly property bool readySettings: root.available && !root.savingSettings
    readonly property bool hasSaveError: root.saveError !== ""

    /// Clé i18n de l'erreur d'enregistrement courante (`""` si aucune).
    readonly property string saveErrorKey: root.keyFor(root.saveError)
    /// Message traduit (chaîne vide si aucune erreur).
    readonly property string saveErrorText: root.saveErrorKey === ""
                                              ? "" : Strings.t(root.saveErrorKey)
    /// Détail sûr (champ ou identifiant), jamais un chemin.
    readonly property string saveErrorDetailText: root.saveErrorDetail

    /// Erreur d'enregistrement des réglages (même mapping de codes).
    readonly property string saveSettingsErrorKey: root.keyFor(root.saveSettingsError)
    readonly property string saveSettingsErrorText: root.saveSettingsErrorKey === ""
                                                      ? "" : Strings.t(root.saveSettingsErrorKey)
    readonly property string saveSettingsErrorDetailText: root.saveSettingsErrorDetail

    /// Clé i18n expliquant l'indisponibilité du service.
    readonly property string availabilityKey: {
        if (root.available)
            return "";
        if (root.error === "Contract")
            return "admin.unavailable.contract";
        if (root.error === "Transport")
            return "admin.unavailable.transport";
        return "admin.unavailable.missing";
    }

    readonly property string availabilityText: root.availabilityKey === ""
                                                 ? "" : Strings.t(root.availabilityKey)

    function keyFor(code) {
        switch (code) {
        case "":
            return "";
        case "NotAuthorized":
            return "admin.error.notAuthorized";
        case "PolicyKitUnavailable":
            return "admin.error.policyKitUnavailable";
        case "Busy":
            return "admin.error.busy";
        case "PayloadTooLarge":
            return "admin.error.payloadTooLarge";
        case "InvalidPayload":
        case "InvalidArgs":
            return "admin.error.invalidPayload";
        case "Validation":
            return "admin.error.validation";
        case "InUse":
            return "admin.error.inUse";
        case "ReloadFailed":
            return "admin.error.reloadFailed";
        case "WriteFailed":
            return "admin.error.writeFailed";
        case "RateLimited":
            return "admin.error.rateLimited";
        case "NotSupported":
            return "admin.error.notSupported";
        case "AdminUnavailable":
            return "admin.error.unavailable";
        case "Timeout":
            return "admin.error.timeout";
        case "Transport":
            return "admin.error.transport";
        case "NoData":
            return "admin.error.noData";
        case "NotFound":
            return "admin.error.notFound";
        case "PresetReadOnly":
            return "admin.error.presetReadOnly";
        case "TooManyProfiles":
            return "admin.error.tooManyProfiles";
        case "InvalidImport":
            return "admin.error.invalidImport";
        case "PathInvalid":
            return "admin.error.pathInvalid";
        case "ReadFailed":
            return "admin.error.readFailed";
        case "FileWriteFailed":
            return "admin.error.fileWriteFailed";
        case "NothingToExport":
            return "admin.error.nothingToExport";
        case "Conflict":
            return "admin.error.conflict";
        default:
            return "admin.error.unknown";
        }
    }
}
