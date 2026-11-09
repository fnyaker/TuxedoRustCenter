// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne de sélection : libellé + description, liste déroulante, état optionnel.
// Utilisée pour les taux de rafraîchissement (avec état supporté/non supporté)
// et les choix matériels de l'éditeur de profil.
//
// `tooltip` ajoute une icône d'information discrète : elle est accessible au
// clavier (focus) et son texte est aussi exposé à `Accessible.description` du
// champ, pour ne pas dépendre du survol.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string label: ""
    property string description: ""
    property string tooltip: ""
    property var options: []           // chaînes ou objets { name, label, value }
    property int currentIndex: 0
    property int selectWidth: 220
    property bool supported: true
    property string unsupportedReason: ""
    property string statusLabel: ""
    property string statusKind: "neutral"

    signal activated(int index)
    signal changed(int index)

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
                spacing: Theme.s6

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: root.label
                    color: root.supported ? Theme.textPrimary : Theme.textMuted
                }

                Item {
                    id: infoTarget
                    objectName: "selectRowInfo"
                    visible: root.tooltip !== ""
                    Layout.preferredWidth: 18
                    Layout.preferredHeight: 18
                    activeFocusOnTab: visible
                    Accessible.role: Accessible.StaticText
                    Accessible.name: root.tooltip

                    AppIcon {
                        anchors.centerIn: parent
                        name: "help"
                        color: infoTarget.activeFocus || infoHover.hovered
                               ? Theme.accentText : Theme.textMuted
                        size: 14
                    }

                    HoverHandler { id: infoHover }

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -2
                        radius: 4
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.focusRing
                        visible: infoTarget.activeFocus
                    }

                    AppToolTip {
                        id: infoTip
                        objectName: "selectRowTooltip"
                        parent: infoTarget
                        delay: 300
                        text: root.tooltip
                        visible: infoHover.hovered || infoTarget.activeFocus
                    }
                }
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.description !== ""
                variant: "small"
                secondary: true
                wrapMode: Text.WordWrap
                text: root.description
            }
        }

        StatusBadge {
            Layout.alignment: Qt.AlignVCenter
            visible: root.statusLabel !== ""
            kind: root.statusKind
            label: root.statusLabel
        }

        AppSelect {
            id: select
            objectName: "selectRowSelect"
            Layout.alignment: Qt.AlignVCenter
            Layout.preferredWidth: root.selectWidth
            model: root.options
            enabled: root.supported
            Accessible.name: root.label
            Accessible.description: root.tooltip
            onActivated: function(index) {
                // `currentIndex` reste piloté par le parent via le Binding
                // ci-dessous : un ComboBox casse sa liaison en interne quand
                // l'utilisateur choisit une entrée.
                root.changed(index);
                root.activated(index);
            }
        }

        // Réapplique la sélection du modèle après chaque choix utilisateur et
        // à chaque changement de profil, sans laisser un index périmé.
        Binding {
            target: select
            property: "currentIndex"
            value: root.currentIndex
            restoreMode: Binding.RestoreNone
        }
    }
}
