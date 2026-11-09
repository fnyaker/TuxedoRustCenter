// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Icône SVG native. Les icônes mono sont normalisées en blanc + alpha puis
// teintées via ColorOverlay (Qt5Compat.GraphicalEffects) : une seule ressource
// sert les deux thèmes et tous les états (normal, actif, désactivé). Les
// illustrations colorées (profils, marque) sont rendues telles quelles.
//
// Choix de ColorOverlay plutôt que MultiEffect : QtQuick.Effects n'existe
// qu'à partir de Qt 6.5, alors que la cible .deb est Ubuntu 24.04 (Qt 6.4).
// Qt5Compat.GraphicalEffects est fourni par Qt 6.4 et par toutes les cibles
// (Arch, Fedora, Ubuntu).
//
// Résolution des chemins centralisée ici : un déploiement en qrc:/ ne change
// que ce fichier.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import Qt5Compat.GraphicalEffects

Item {
    id: root

    property string name: ""
    property bool monochrome: true
    property color color: Theme.textSecondary
    property real size: 18
    // 2x pour rester net en HiDPI sans surcoût mémoire notable sur des SVG 512.
    readonly property int rasterSize: Math.ceil(size * 2)

    readonly property url imageSource: name === "" ? ""
        : name.indexOf("/") >= 0 ? Qt.resolvedUrl("../../assets/" + name + ".svg")
        : Qt.resolvedUrl("../../assets/icons/icon_" + name + ".svg")

    implicitWidth: size
    implicitHeight: size
    Accessible.ignored: true

    Image {
        id: img
        anchors.fill: parent
        source: root.imageSource
        sourceSize.width: root.rasterSize
        sourceSize.height: root.rasterSize
        fillMode: Image.PreserveAspectFit
        smooth: true
        visible: !root.monochrome
    }

    ColorOverlay {
        anchors.fill: parent
        source: img
        visible: root.monochrome
        color: root.color
    }
}
