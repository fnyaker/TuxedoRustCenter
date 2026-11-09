// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Icône SVG native. Les icônes mono sont normalisées en blanc + alpha puis
// teintées via MultiEffect : une seule ressource sert les deux thèmes et tous
// les états (normal, actif, désactivé). Les illustrations colorées (profils,
// marque) sont rendues telles quelles.
//
// Résolution des chemins centralisée ici : un déploiement en qrc:/ ne change
// que ce fichier.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import QtQuick.Effects

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

    MultiEffect {
        anchors.fill: parent
        source: img
        visible: root.monochrome
        colorizationColor: root.color
        colorization: 1.0
        // Pas d'ombre ni de flou : uniquement la teinte, coût minimal.
    }
}
