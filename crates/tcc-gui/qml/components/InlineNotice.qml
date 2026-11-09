// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Bandeau d'information contextuel (info / succès / avertissement / erreur)
// avec action optionnelle. Sert aux états de l'éditeur de profil : service
// d'administration absent, preset en lecture seule, conflit de profil actif,
// erreur d'enregistrement.
//
// Le fond est toujours accompagné d'un titre et, si présent, d'un détail : la
// couleur n'est jamais le seul signal (accessibilité daltonisme).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    /// `info`, `ok`, `warn` ou `error`.
    property string kind: "info"
    property string title: ""
    property string body: ""
    property string actionText: ""
    /// Bouton secondaire optionnel ("Recharger", "Réessayer"…).
    signal actionClicked()

    readonly property color tone: root.kind === "error" ? Theme.error
                              : root.kind === "warn" ? Theme.warning
                              : root.kind === "ok" ? Theme.success : Theme.info
    readonly property color surfaceTone: root.kind === "error" ? Theme.errorSoft
                                       : root.kind === "warn" ? Theme.warningSoft
                                       : root.kind === "ok" ? Theme.successSoft
                                       : Theme.infoSoft

    implicitHeight: content.implicitHeight + Theme.s20
    radius: Theme.radiusMd
    color: root.surfaceTone
    border.width: 1
    border.color: root.tone
    Accessible.name: root.title
    Accessible.description: root.body

    RowLayout {
        id: content
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.margins: Theme.s12
        spacing: Theme.s12

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                strong: true
                color: root.tone
                text: root.title
                wrapMode: Text.WordWrap
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.body !== ""
                variant: "small"
                secondary: true
                wrapMode: Text.WordWrap
                text: root.body
            }
        }

        AppButton {
            visible: root.actionText !== ""
            variant: "secondary"
            small: true
            text: root.actionText
            Accessible.name: root.actionText
            onClicked: root.actionClicked()
        }
    }
}
