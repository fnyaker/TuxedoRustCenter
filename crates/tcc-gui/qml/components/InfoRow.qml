// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne d'information compacte : libellé discret à gauche, valeur ou badge à
// droite. Utilisée par les panneaux d'état, les diagnostics et les réglages.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string label: ""
    property string value: ""
    property string iconName: ""
    property string badgeLabel: ""
    property string badgeKind: "neutral"
    property bool valueStrong: false

    implicitHeight: Math.max(row.implicitHeight, 28)

    RowLayout {
        id: row
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.s8

        AppIcon {
            visible: root.iconName !== ""
            name: root.iconName
            color: Theme.textMuted
            size: 14
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            secondary: true
            text: root.label
            elide: Text.ElideRight
            wrapMode: Text.NoWrap
        }

        StatusBadge {
            visible: root.badgeLabel !== ""
            kind: root.badgeKind
            label: root.badgeLabel
        }

        AppLabel {
            visible: root.value !== ""
            variant: "small"
            strong: root.valueStrong
            text: root.value
            horizontalAlignment: Text.AlignRight
            elide: Text.ElideRight
            wrapMode: Text.NoWrap
        }
    }
}
