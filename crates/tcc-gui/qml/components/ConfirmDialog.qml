// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Boîte de confirmation modale pour les actions destructives. Le bouton sûr
// (Annuler) reçoit le focus à l'ouverture ; Échap annule.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Popup {
    id: dialog

    parent: Overlay.overlay
    modal: true
    dim: true
    width: 440
    padding: 0
    closePolicy: Popup.CloseOnEscape
    Overlay.modal: Rectangle { color: Theme.overlay }

    property string title: ""
    property string body: ""
    property string confirmText: ""
    property string cancelText: ""
    property bool destructive: true

    signal confirmed()

    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)

    function ask() {
        open();
    }

    background: Rectangle {
        radius: Theme.radiusLg
        color: Theme.surface
        border.width: 1
        border.color: Theme.borderStrong
    }

    contentItem: ColumnLayout {
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.s24
            spacing: Theme.s10

            RowLayout {
                spacing: Theme.s10

                Rectangle {
                    Layout.preferredWidth: 32
                    Layout.preferredHeight: 32
                    radius: Theme.radiusSm
                    color: dialog.destructive ? Theme.errorSoft : Theme.accentSoft

                    AppIcon {
                        anchors.centerIn: parent
                        name: dialog.destructive ? "trash" : "info"
                        color: dialog.destructive ? Theme.error : Theme.accentText
                        size: 16
                    }
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "h3"
                    text: dialog.title
                }
            }

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                secondary: true
                text: dialog.body
                wrapMode: Text.WordWrap
            }
        }

        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.divider }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.s16
            spacing: Theme.s8

            Item { Layout.fillWidth: true }

            AppButton {
                id: cancelButton
                text: dialog.cancelText !== "" ? dialog.cancelText : Strings.t("common.cancel")
                variant: "secondary"
                onClicked: dialog.close()
            }

            AppButton {
                text: dialog.confirmText !== "" ? dialog.confirmText : Strings.t("common.delete")
                variant: "danger"
                onClicked: {
                    dialog.close();
                    dialog.confirmed();
                }
            }
        }
    }

    onOpened: cancelButton.forceActiveFocus()
}
