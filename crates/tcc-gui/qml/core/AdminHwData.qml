// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// État du service `Admin.Hw` (ODM/TDP/cTGP, phase 2) et de la dernière
// écriture transitoire. Aucune logique d'écriture ici.

pragma Singleton

import QtQuick

QtObject {
    id: root

    property bool available: false
    property string version: ""
    /// `""`, `Unavailable`, `Contract`, `Transport`.
    property string error: ""
    property bool pending: false
    /// `odm`, `tdp`, `ctgp`.
    property string action: ""
    property string saveError: ""
    property string saveErrorDetail: ""
    property bool succeeded: false

    readonly property bool ready: root.available && !root.pending
    readonly property bool hasSaveError: root.saveError !== ""

    readonly property string saveErrorKey: root.keyFor(root.saveError)
    readonly property string saveErrorText: root.saveErrorKey === ""
                                            ? "" : Strings.t(root.saveErrorKey)
    readonly property string saveErrorDetailText: root.saveErrorDetail

    readonly property string availabilityKey: {
        if (root.available)
            return "";
        if (root.error === "Contract")
            return "adminHw.unavailable.contract";
        if (root.error === "Transport")
            return "adminHw.unavailable.transport";
        return "adminHw.unavailable.missing";
    }

    readonly property string availabilityText: root.availabilityKey === ""
                                                 ? "" : Strings.t(root.availabilityKey)

    function keyFor(code) {
        switch (code) {
        case "":
            return "";
        case "NotAuthorized":
            return "admin.error.notAuthorized";
        case "PolicyDenied":
            return "adminHw.error.policyDenied";
        case "PartialApply":
            return "adminHw.error.partialApply";
        case "InvalidArgs":
        case "Validation":
            return "adminHw.error.invalidArgs";
        case "NotSupported":
            return "admin.error.notSupported";
        case "WriteFailed":
            return "admin.error.writeFailed";
        case "RateLimited":
            return "admin.error.rateLimited";
        case "Busy":
            return "admin.error.busy";
        case "Timeout":
            return "admin.error.timeout";
        case "Transport":
            return "admin.error.transport";
        case "AdminUnavailable":
            return "adminHw.error.unavailable";
        default:
            return "adminHw.error.unknown";
        }
    }
}
