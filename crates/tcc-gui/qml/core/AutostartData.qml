// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Autostart XDG de la GUI (`$XDG_CONFIG_HOME/autostart/…desktop`).
// Poussé par `Main.qml` depuis le pont Rust ; le fichier vit uniquement dans
// le répertoire utilisateur, aucune persistance QSettings.

pragma Singleton

import QtQuick

QtObject {
    id: root

    property bool enabled: false
    /// Chemin autostart résolvable (`$HOME`/`XDG_CONFIG_HOME` valides).
    property bool available: false
    /// Code stable : `""`, `NoHome`, `PathInvalid`, `ReadFailed`, `WriteFailed`.
    property string error: ""

    readonly property bool editable: root.available
    readonly property bool hasError: root.error !== ""

    readonly property string errorKey: {
        switch (root.error) {
        case "":
            return "";
        case "NoHome":
            return "settings.autostartNoHome";
        case "PathInvalid":
            return "settings.autostartPathInvalid";
        case "ReadFailed":
            return "settings.autostartReadFailed";
        case "InsecurePermissions":
            return "settings.autostartInsecure";
        case "WriteFailed":
            return "settings.autostartWriteFailed";
        default:
            return "settings.autostartError";
        }
    }

    readonly property string errorText: root.errorKey === ""
                                        ? "" : Strings.t(root.errorKey)
}
