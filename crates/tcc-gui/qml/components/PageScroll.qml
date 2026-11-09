// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Conteneur de page défilant : largeur maximale de lecture, marges cohérentes,
// barre de défilement verticale native. Le contenu est un ColumnLayout.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

ScrollView {
    id: root

    default property alias content: column.data

    property int pagePadding: Theme.s24
    property int contentMaxWidth: Theme.contentMaxWidth
    property alias contentSpacing: column.spacing

    contentWidth: availableWidth
    clip: true
    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
    ScrollBar.vertical.policy: ScrollBar.AsNeeded

    /// Flickable interne (typé `var` pour rester lisible par `qmllint`).
    readonly property var flickable: contentItem

    /// Défile jusqu'à un élément du contenu (navigation ciblée).
    function scrollToItem(item) {
        if (!item)
            return;
        var target = item.mapToItem(contentRoot, 0, 0).y - pagePadding;
        var max = Math.max(0, flickable.contentHeight - flickable.height);
        flickable.contentY = Math.max(0, Math.min(target, max));
    }

    Item {
        id: contentRoot
        width: root.availableWidth
        implicitHeight: column.implicitHeight + root.pagePadding * 2

        ColumnLayout {
            id: column
            x: Math.max(root.pagePadding, (parent.width - width) / 2)
            y: root.pagePadding
            width: Math.min(parent.width - root.pagePadding * 2, root.contentMaxWidth)
            spacing: Theme.s16
        }
    }
}
