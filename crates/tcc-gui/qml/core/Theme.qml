// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Design tokens — source unique de vérité pour la couleur, la typographie,
// l'espacement et le mouvement. Voir docs/ui/UI_DESIGN.md.
//
// Aucune logique métier : uniquement des valeurs de présentation.
// Les couleurs par thème sont des bindings sur `dark`, donc un changement de
// `mode` met à jour toute l'interface sans redémarrage.

// Note : l'analyse qmllint de ce fichier seul signale un faux positif
// « not declared as singleton in qmldir ». Le singleton est bien déclaré dans
// core/qmldir et le chargement runtime est validé.
pragma Singleton
import QtQuick
import QtQuick.Controls

QtObject {
    id: theme

    // ── Mode ────────────────────────────────────────────────────────────────
    // "system" | "light" | "dark"
    property string mode: "light"
    readonly property bool systemDark: Application.styleHints.colorScheme === Qt.Dark
    readonly property bool dark: mode === "dark" || (mode === "system" && systemDark)

    // ── Marque (invariants) ─────────────────────────────────────────────────
    readonly property color brandRed: "#e30016" // TUXEDO — logo, illustrations
    readonly property color gaugeRedLight: "#e22131"
    readonly property color gaugeRedDark: "#f2545b"

    // ── Surfaces ────────────────────────────────────────────────────────────
    readonly property color bg: dark ? "#17171a" : "#f4f4f5"
    readonly property color surface: dark ? "#202023" : "#ffffff"
    readonly property color surfaceMuted: dark ? "#26262a" : "#fafafa"
    readonly property color surfaceSunken: dark ? "#1b1b1e" : "#efeff1"
    readonly property color surfaceHover: dark ? "#2a2a2f" : "#f4f4f5"
    readonly property color sidebar: dark ? "#1c1c1f" : "#fbfbfc"
    readonly property color overlay: dark ? "#00000099" : "#0f0f0f66"

    // ── Bordures ────────────────────────────────────────────────────────────
    readonly property color border: dark ? "#303035" : "#e4e4e7"
    readonly property color borderStrong: dark ? "#3f3f46" : "#d4d4d8"
    readonly property color divider: dark ? "#2a2a2f" : "#ebebee"

    // ── Texte ───────────────────────────────────────────────────────────────
    readonly property color textPrimary: dark ? "#f4f4f5" : "#18181b"
    readonly property color textSecondary: dark ? "#b4b4bb" : "#52525b"
    readonly property color textMuted: dark ? "#94949d" : "#6b6b75"
    readonly property color textOnAccent: "#ffffff"
    readonly property color textOnLight: "#18181b"

    // ── Accent TUXEDO ───────────────────────────────────────────────────────
    // `accent` sert aux surfaces de marque et aux gros éléments.
    // `accentText` est la variante texte/icône garantissant le contraste AA.
    readonly property color accent: dark ? "#ff4d5e" : "#e30016"
    readonly property color accentText: dark ? "#ff6b75" : "#b30013"
    readonly property color accentHover: dark ? "#ff6b79" : "#c50013"
    readonly property color accentPressed: dark ? "#e63a4b" : "#a80010"
    readonly property color accentSoft: dark ? "#352026" : "#fce7e9"
    readonly property color accentSoftHover: dark ? "#412830" : "#fadadd"
    readonly property color gauge: dark ? gaugeRedDark : gaugeRedLight
    readonly property color gaugeTrack: dark ? "#34343a" : "#ebebee"

    // ── États sémantiques ───────────────────────────────────────────────────
    readonly property color success: dark ? "#4ade80" : "#157f3f"
    readonly property color successSoft: dark ? "#1b2e24" : "#e7f4ec"
    readonly property color warning: dark ? "#fbbf24" : "#8a5300"
    readonly property color warningSoft: dark ? "#33291a" : "#fbf0df"
    readonly property color error: dark ? "#ff7a70" : "#b3261e"
    readonly property color errorSoft: dark ? "#3a1f1e" : "#fbe9e7"
    readonly property color info: dark ? "#7cb7f0" : "#1f5f9e"
    readonly property color infoSoft: dark ? "#1d2a3a" : "#e8f0f8"
    readonly property color focusRing: dark ? "#ff4d5e" : "#e30016"
    readonly property color track: dark ? "#3f3f46" : "#d4d4d8"

    // ── Typographie ─────────────────────────────────────────────────────────
    readonly property string fontFamily: "Titillium Web"
    readonly property string fontFallback: "Inter, Noto Sans, sans-serif"
    readonly property int fsCaption: 12
    readonly property int fsSmall: 13
    readonly property int fsBody: 14
    readonly property int fsBodyLarge: 15
    readonly property int fsTitle: 17
    readonly property int fsH2: 20
    readonly property int fsH1: 26
    readonly property int fsDisplay: 34
    readonly property int fsMetric: 38
    readonly property int fwRegular: Font.Normal
    readonly property int fwMedium: Font.Medium
    readonly property int fwSemiBold: Font.DemiBold
    readonly property int fwBold: Font.Bold

    // ── Espacement (échelle 4 pt) ───────────────────────────────────────────
    readonly property int s2: 2
    readonly property int s4: 4
    readonly property int s6: 6
    readonly property int s8: 8
    readonly property int s10: 10
    readonly property int s12: 12
    readonly property int s16: 16
    readonly property int s20: 20
    readonly property int s24: 24
    readonly property int s32: 32
    readonly property int s40: 40
    readonly property int s48: 48
    readonly property int s64: 64

    // ── Rayons ──────────────────────────────────────────────────────────────
    readonly property int radiusXs: 4
    readonly property int radiusSm: 6
    readonly property int radiusMd: 10
    readonly property int radiusLg: 14
    readonly property int radiusXl: 18
    readonly property int radiusPill: 999

    // ── Gabarits ────────────────────────────────────────────────────────────
    readonly property int sidebarWidth: 256
    readonly property int sidebarCompactWidth: 76
    readonly property int contentMaxWidth: 1180
    readonly property int controlHeight: 36
    readonly property int controlHeightSmall: 30
    readonly property int rowMinHeight: 40
    readonly property int hitTarget: 32 // cible minimale WCAG 2.5.8

    // ── Mouvement ───────────────────────────────────────────────────────────
    property bool reduceMotion: false
    readonly property int durationFast: reduceMotion ? 0 : 120
    readonly property int durationBase: reduceMotion ? 0 : 180
    readonly property int durationSlow: reduceMotion ? 0 : 320
    readonly property int easeOut: Easing.OutCubic
    readonly property int easeInOut: Easing.InOutCubic

    // ── Divers ──────────────────────────────────────────────────────────────
    readonly property real disabledOpacity: 0.45
    readonly property real mutedOpacity: 0.7
}
