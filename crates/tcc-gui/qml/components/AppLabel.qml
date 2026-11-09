// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Échelle typographique unique. Toute étiquette de l'interface passe par ce
// composant pour garantir la hiérarchie et les couleurs de texte.

pragma ComponentBehavior: Bound

import QtQuick
import "../core"
import QtQuick.Controls

Label {
    id: root

    // display | h1 | h2 | h3 | title | body | bodyLarge | small | caption |
    // label | metric
    property string variant: "body"
    property bool secondary: false
    property bool muted: false
    property bool strong: false

    font.family: Theme.fontFamily
    font.pixelSize: variant === "display" ? Theme.fsDisplay
                  : variant === "h1" ? Theme.fsH1
                  : variant === "h2" ? Theme.fsH2
                  : variant === "h3" ? Theme.fsTitle
                  : variant === "title" ? Theme.fsBodyLarge
                  : variant === "bodyLarge" ? Theme.fsBodyLarge
                  : variant === "small" ? Theme.fsSmall
                  : variant === "caption" ? Theme.fsCaption
                  : variant === "label" ? Theme.fsCaption
                  : variant === "metric" ? Theme.fsMetric
                  : Theme.fsBody
    font.weight: variant === "display" || variant === "h1" || variant === "h2"
                 || variant === "h3" || variant === "title" || variant === "label"
                 ? Theme.fwSemiBold
                 : (strong ? Theme.fwSemiBold : Theme.fwRegular)
    font.letterSpacing: variant === "label" ? 0.8 : 0
    font.capitalization: variant === "label" ? Font.AllUppercase : Font.MixedCase
    color: muted ? Theme.textMuted : (secondary ? Theme.textSecondary : Theme.textPrimary)
    wrapMode: Text.NoWrap
    textFormat: Text.PlainText
    elide: Text.ElideNone
    lineHeight: variant === "display" || variant === "h1" ? 1.08 : 1.15
}
