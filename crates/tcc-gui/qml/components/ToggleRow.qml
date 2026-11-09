// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne de réglage : libellé + description à gauche, contrôle à droite.
// Hauteur minimale 48 px pour rester une cible confortable, y compris tactile.
// États : normal, désactivé, non pris en charge (raison affichée).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string label: ""
    property string description: ""
    property bool checked: false
    property bool supported: true
    property string unsupportedReason: ""
    signal toggled(bool checked)

    implicitHeight: Math.max(row.implicitHeight, 44)

    RowLayout {
        id: row
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.s16

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s8

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: root.label
                    color: root.supported ? Theme.textPrimary : Theme.textMuted
                }

                StatusBadge {
                    visible: !root.supported
                    kind: "neutral"
                    label: root.unsupportedReason !== "" ? root.unsupportedReason
                                                         : Strings.t("common.unavailable")
                }
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.description !== ""
                variant: "small"
                secondary: true
                text: root.description
            }
        }

        AppSwitch {
            id: control
            Layout.alignment: Qt.AlignVCenter
            enabled: root.supported
            Accessible.name: root.label
            Accessible.description: root.description
            onToggled: root.toggled(checked)
        }

        // Le clic casse la liaison interne de l'interrupteur ; ce Binding
        // re-applique l'état porté par `root.checked` après un refus ou une
        // relecture du daemon.
        Binding {
            target: control
            property: "checked"
            value: root.checked
            restoreMode: Binding.RestoreNone
        }
    }
}
