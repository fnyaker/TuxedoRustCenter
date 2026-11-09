// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Bandeau d'erreur de connexion au daemon. Visible dès que le pont Rust
// remonte une erreur (`lastError` : daemon absent, bus inaccessible, payload
// refusé). Le badge de la barre latérale indique séparément la source des
// données (daemon ou démonstration).
//
// Le bandeau ne bloque jamais la navigation : il informe et propose de
// relancer une lecture (`DaemonClient::refresh`).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    property string title: Strings.t("connection.title")
    property string detail: ""
    /// Commande déjà en vol : le bouton « Réessayer » est neutralisé.
    property bool busy: false
    signal retryRequested()

    implicitHeight: Theme.s48
    color: Theme.errorSoft
    Accessible.name: root.title
    Accessible.description: root.detail

    // Barre d'accent : l'erreur n'est jamais portée par la couleur seule.
    Rectangle {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 3
        color: Theme.error
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.s16
        anchors.rightMargin: Theme.s12
        spacing: Theme.s12

        AppLabel {
            variant: "small"
            strong: true
            color: Theme.error
            text: root.title
        }

        AppLabel {
            id: detailLabel
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            elide: Text.ElideRight
            text: root.detail
        }

        HoverHandler {
            id: detailHover
            enabled: detailLabel.truncated
        }

        AppToolTip {
            visible: detailHover.hovered
            text: root.detail
        }

        AppButton {
            variant: "secondary"
            small: true
            text: Strings.t("common.retry")
            enabled: !root.busy
            Accessible.name: Strings.t("common.retry")
            onClicked: root.retryRequested()
        }
    }
}
