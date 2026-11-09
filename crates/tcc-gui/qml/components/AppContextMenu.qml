// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Menu contextuel léger (kebab des cartes de profil). Style natif, navigation
// clavier haut/bas, fermeture Échap. Les entrées destructives sont marquées
// visuellement et jamais pré-sélectionnées.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Popup {
    id: menu

    parent: Overlay.overlay
    property var items: []          // [{ label, icon?, destructive? }]
    signal triggered(int index)

    width: 216
    padding: Theme.s4
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    modal: false

    function showAt(sourceItem) {
        var p = sourceItem.mapToItem(null, sourceItem.width, sourceItem.height + Theme.s4);
        menu.x = Math.min(p.x - menu.width, menu.parent.width - menu.width - Theme.s8);
        menu.y = p.y;
        menu.open();
    }

    background: Rectangle {
        radius: Theme.radiusMd
        color: Theme.surface
        border.width: 1
        border.color: Theme.borderStrong
    }

    contentItem: ColumnLayout {
        id: column
        spacing: 0

        Repeater {
            model: menu.items

            delegate: AbstractButton {
                id: row
                required property var modelData
                required property int index

                Layout.fillWidth: true
                implicitHeight: 36
                enabled: modelData.enabled !== false
                hoverEnabled: true
                focusPolicy: enabled ? Qt.TabFocus : Qt.NoFocus
                Accessible.role: Accessible.MenuItem
                Accessible.name: modelData.label
                Accessible.description: modelData.disabledReason === undefined
                                        ? "" : modelData.disabledReason

                onClicked: {
                    menu.close();
                    menu.triggered(index);
                }

                background: Rectangle {
                    radius: Theme.radiusXs
                    color: row.pressed ? Theme.surfaceSunken
                         : row.hovered ? Theme.surfaceHover : "transparent"
                }

                contentItem: RowLayout {
                    spacing: Theme.s8

                    AppIcon {
                        Layout.leftMargin: Theme.s8
                        visible: row.modelData.icon !== undefined
                        name: row.modelData.icon === undefined ? "" : row.modelData.icon
                        color: !row.enabled ? Theme.textMuted
                             : row.modelData.destructive === true ? Theme.error
                             : Theme.textSecondary
                        size: 15
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        variant: "small"
                        text: row.modelData.label
                        color: !row.enabled ? Theme.textMuted
                             : row.modelData.destructive === true ? Theme.error
                             : Theme.textPrimary
                    }

                    Glyph {
                        Layout.rightMargin: Theme.s8
                        visible: row.modelData.checked === true
                        name: "check"
                        color: Theme.accentText
                        size: 12
                        strokeWidth: 2
                    }
                }
            }
        }
    }

    onOpened: {
        if (column.children.length > 0)
            column.children[0].forceActiveFocus();
    }
}
