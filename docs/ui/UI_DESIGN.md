# Design system & shell QML — TuxedoRustCenter

Document de référence de l'interface Qt 6 native (jalon 1), à lire avec
`AGENTS.md`, `docs/ARCHITECTURE.md` et `docs/FEATURE_PARITY.md`.

- Code : `crates/tcc-gui/qml/`
- Ressources : `crates/tcc-gui/assets/`
- Captures de référence : `docs/ui/captures/`
- Source fonctionnelle : `tuxedo-control-center-master/` (Angular, jamais modifié)

Aucun composant web n'est utilisé : pas de QtWebEngine/QtWebView, pas de
HTML/CSS/JS applicatif, pas d'Angular. Toute l'interface est QML + Qt Quick
Controls 2 (style Basic) avec des composants dessinés nativement (Shapes).

---

## 1. Direction visuelle

L'original est une interface claire, dense, avec des cartes bordées, une
barre latérale à deux niveaux et des jauges circulaires rouges. La réécriture
conserve ces repères et les modernise :

| Axe | Original | Réécriture |
|---|---|---|
| Structure | Sidebar 2 niveaux + zone centrale | Même hiérarchie, sidebar repliable en mode icônes |
| Cartes | Bordure grise, rayon faible | Surfaces `surface`, rayon 14 px, bordure 1 px, **aucune ombre** |
| Rouge | Présent partout (jauges, boutons, titres) | Réservé aux jauges, à l'action primaire, au profil actif et à la sélection de navigation |
| Vert | Interrupteurs d'activation | Conservé : vert = « activé », jamais confondu avec la marque |
| Typo | Titillium Web, tailles en `em` | Titillium Web embarquée, échelle 12→38 px, poids 400/600 |
| Densité | Élevée | Densité maîtrisée : 24 px de marge de page, rangées ≥ 44 px |
| Thème | Clair + sombre | Clair, sombre, **système** avec bascule à chaud |

Principes :

1. **Le rouge est un signal, pas un fond.** Une seule action primaire par vue,
   une seule bordure accentuée (profil actif), une barre de 3 px pour la
   navigation sélectionnée.
2. **Pas de couleur seule.** Tout état est doublé d'un texte ou d'une icône
   (« Actif », « Non disponible », point + libellé).
3. **Plat et rapide.** Bordures plutôt qu'ombres, pas de blur, pas de
   re-render inutile. Les seuls effets GPU sont la teinte des icônes et les
   tracés vectoriels.
4. **La donnée appartient au daemon.** Le QML affiche des modèles, il ne
   calcule ni validation ni application matérielle.

---

## 2. Jetons de design

Source unique : `qml/core/Theme.qml` (singleton `Theme`).

### 2.1 Couleurs

| Jeton | Clair | Sombre | Usage |
|---|---|---|---|
| `bg` | `#f4f4f5` | `#17171a` | Fond de fenêtre |
| `surface` | `#ffffff` | `#202023` | Cartes, champs, popups |
| `surfaceMuted` | `#fafafa` | `#26262a` | Carte « nouveau profil », fonds secondaires |
| `surfaceSunken` | `#efeff1` | `#1b1b1e` | Pistes de segments, zones creuses |
| `surfaceHover` | `#f4f4f5` | `#2a2a2f` | Survol |
| `sidebar` | `#fbfbfc` | `#1c1c1f` | Barre latérale |
| `border` | `#e4e4e7` | `#303035` | Bordures 1 px |
| `borderStrong` | `#d4d4d8` | `#3f3f46` | Champs, boutons secondaires |
| `divider` | `#ebebee` | `#2a2a2f` | Séparateurs internes |
| `textPrimary` | `#18181b` | `#f4f4f5` | Titres, valeurs |
| `textSecondary` | `#52525b` | `#b4b4bb` | Corps secondaire |
| `textMuted` | `#6b6b75` | `#94949d` | Légendes, désactivé |
| `accent` | `#e30016` | `#ff4d5e` | Bouton primaire, sélection, bordure active |
| `accentText` | `#b30013` | `#ff6b75` | Texte/icône accentués sur surface |
| `accentSoft` | `#fce7e9` | `#352026` | Fond de sélection, pastilles d'icônes |
| `gauge` | `#e22131` | `#f2545b` | Arc de valeur des jauges |
| `gaugeTrack` | `#ebebee` | `#34343a` | Piste des jauges |
| `success` | `#157f3f` | `#4ade80` | Interrupteur ON, badges OK |
| `warning` | `#8a5300` | `#fbbf24` | Seuil d'alerte, états partiels |
| `error` | `#b3261e` | `#ff7a70` | Erreur, destructif |
| `info` | `#1f5f9e` | `#7cb7f0` | Information, série GPU des graphes |
| `focusRing` | `#e30016` | `#ff4d5e` | Anneau de focus 2 px |
| `overlay` | `#0f0f0f66` | `#00000099` | Voile des dialogues |

`brandRed` (`#e30016`) est **invariant** : il ne change pas avec le thème
(logo, illustrations de profil).

Contrastes WCAG 2.1 calculés (AA texte normal ≥ 4,5:1, grand texte ≥ 3:1) :

| Paire | Ratio |
|---|---|
| clair — texte `#18181b` / `#ffffff` | **17,7:1** |
| clair — secondaire `#52525b` / `#ffffff` | **7,7:1** |
| clair — atténué `#6b6b75` / `#ffffff` | **5,3:1** |
| clair — accent texte `#b30013` / `#ffffff` | **7,2:1** |
| clair — accent `#e30016` / `#ffffff` | 4,9:1 (OK, réservé aux grands libellés/boutons) |
| clair — blanc / `#e30016` | 4,9:1 (libellé du bouton primaire) |
| clair — jauge `#e22131` / `#ffffff` | 4,7:1 (graphique, non textuel) |
| clair — succès / erreur / alerte / info | 5,1 / 6,5 / 6,3 / 6,6:1 |
| sombre — texte `#f4f4f5` / `#202023` | **14,8:1** |
| sombre — secondaire `#b4b4bb` / `#202023` | **7,9:1** |
| sombre — atténué `#94949d` / `#202023` | **5,4:1** |
| sombre — accent `#ff4d5e` / `#202023` | **5,0:1** |
| sombre — accent texte `#ff6b75` / `#202023` | **5,9:1** |
| sombre — jauge `#f2545b` / `#202023` | 4,8:1 |
| sombre — succès / erreur / alerte / info | 9,3 / 6,4 / 9,7 / 7,7:1 |

> ⚠ En thème sombre, `brandRed #e30016` sur `#202023` ne donne que **3,3:1** :
> il est interdit pour du texte. On utilise `accent` ou `accentText`.

### 2.2 Typographie

- Famille : **Titillium Web** (SIL OFL 1.1), embarquée dans
  `assets/fonts/` et chargée par `FontLoader`.
- Fallback : `Inter, Noto Sans, sans-serif` (résolution fontconfig si la
  police embarquée manque).
- Échelle : `12 / 13 / 14 / 15 / 17 / 20 / 26 / 34 / 38` px.
- Poids : 400 (corps), 600 (titres, valeurs de jauges, libellés de boutons).
- Interlignage : 1,08 pour les titres, 1,15 pour le corps.
- Chiffres : formatés via `toLocaleString` (virgule décimale en FR/DE),
  jamais concaténés à la main.

### 2.3 Espacement, rayons, gabarits

- Échelle 4 pt : `2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64`.
- Marges de page : 24 px ; largeur maximale de lecture : 1180 px.
- Rayons : 4 (petit), 6 (contrôles), 10 (popups), 14 (cartes), 18 (dialogues),
  999 (pastilles).
- Hauteurs : contrôles 36 px, petits contrôles 30 px, cible minimale 32 px,
  rangées de réglage ≥ 44 px.
- Sidebar : 256 px ; mode compact : 76 px (déclenché sous 1120 px de large).

### 2.4 Mouvement

- Durées : 120 ms (micro), 180 ms (base), 320 ms (rare).
- Courbes : `OutCubic`.
- `Theme.reduceMotion` (réglage « Réduire les animations ») met toutes les
  durées à **0** : transitions, jauges, interrupteurs, spinner.
- Aucune animation décorative en boucle hors indicateur d'activité.

---

## 3. Composants

| Composant | Rôle | États / points d'attention |
|---|---|---|
| `Sidebar` | Identité, navigation, profils utilisés, utilitaires | Repliable, tooltips en compact, badge « Démo », section « Profils utilisés » cliquable |
| `NavItem` | Entrée de navigation | Normal / survol / sélectionné (fond `accentSoft` + barre 3 px) / désactivé ; rôle `PageTab` |
| `UsedProfileRow` | Profil assigné à Secteur/Batterie | Icône d'état + libellé + nom, tooltip en compact |
| `PageHeader` | Titre, sous-titre, fil d'Ariane, actions | Eyebrow 12 px, h1 26 px, actions à droite |
| `SectionHeader` | Titre de section dans une carte | Pastille d'icône 30 px `accentSoft`, hint 13 px, action à droite |
| `AppCard` | Surface de contenu | Bordure 1 px, rayon 14, padding 20, `highlighted` = bordure accent |
| `AppButton` | Action | `primary` (une par vue), `secondary`, `ghost`, `danger`, `link` ; anneau de focus 2 px |
| `IconButton` | Action icône seule | 32×32, `Accessible.name` obligatoire, tooltip |
| `AppSwitch` | Activation | Vert = ON, gris = OFF, anneau de focus, libellé externe |
| `AppSlider` | Réglage continu | Piste 4 px, poignée 16 px, focus visible |
| `AppSelect` | Choix fermé | Popup stylé, chevron dessiné, navigation clavier |
| `AppTextField` / `AppTextArea` | Saisie | Bordure d'erreur, anneau de focus, `activeFocus` (TextField n'expose pas `visualFocus`) |
| `SegmentedControl` | Filtres, thème, unités | Rôle `RadioButton`, flèches gauche/droite |
| `ToggleRow` | Réglage booléen complet | Libellé + description + interrupteur, état « non pris en charge » + raison |
| `SliderRow` | Réglage numérique complet | Valeur localisée, boutons −/+ 26 px, état non pris en charge |
| `SelectRow` | Réglage à choix complet | Badge d'état optionnel (ex. refresh rate), icône d'aide + infobulle accessible |
| `TextFieldRow` | Saisie complète | Champ 320 px, message d'erreur, reset du modèle via `Binding` |
| `StatusBadge` | État sémantique | `ok`, `warn`, `error`, `info`, `accent`, `neutral` ; point ou icône + texte |
| `InfoRow` | Ligne clé/valeur | Icône discrète, valeur alignée à droite, badge optionnel |
| `Gauge` | Jauge circulaire 270° | Valeur, unité, icône, légende, seuil d'alerte, état indisponible + tooltip |
| `Chart` | Historique / sparkline | `Shape` + `PathSvg`, grille optionnelle, point final, aire en dégradé |
| `FanCurveChart` | Courbe température → vitesse | Axes gradués, points de consigne, tooltip d'accessibilité textuel ; édition directe optionnelle (`editable`) |
| `ProfileCard` | Carte de profil (grille ou liste) | Actif (bordure accent), assignations AC/BAT, kebab, interrupteur, métriques |
| `EmptyState` | Liste vide | Icône + titre + corps + action |
| `LoadingState` | Activité | Arc rotatif natif, désactivé si `reduceMotion` |
| `AppContextMenu` | Menu kebab | Entrées destructives en rouge, fermeture Échap |
| `TrayMenu` | Aperçu du menu tray | Profils radio, blocage veille, ouvrir, quitter |
| `SystemTrayIcon` | Tray natif Qt Labs Platform | Menu complet, tooltip, clic afficher/masquer |
| `ConfirmDialog` | Confirmation destructive | Voile, focus sur « Annuler », Échap |
| `Toast` | Notification transitoire | Barre de couleur, fermeture 24 px, durée 4,2 s |
| `AppToolTip` | Aide contextuelle | Contraste inversé, délai 400–600 ms |
| `Glyph` | Chevrons, croix, coche, +/−, kebab, loupe | Dessiné nativement, jamais une police d'icônes |
| `AppIcon` | Icône SVG 512² | Teinte à l'exécution (ColorOverlay, Qt5Compat), illustrations colorées non teintées |

### Règles d'états

- **Survol** : uniquement sur les éléments actionnables, via `surfaceHover`.
- **Focus** : anneau 2 px `focusRing` avec 3 px de décalage, visible
  uniquement au clavier (`activeFocus`/`visualFocus`), jamais au clic souris.
  La politique de focus n'est pas fixée dans le QML (`Item.focusPolicy` exige
  Qt 6.7, la cible `.deb` est Ubuntu 24.04 / Qt 6.4) : les contrôles utilisent
  les valeurs par défaut de Qt Quick Controls.
- **Désactivé** : couleur atténuée + `enabled: false`, jamais seulement
  l'opacité (les lecteurs d'écran doivent aussi voir l'état).
- **Non pris en charge** : le contrôle est désactivé **et** un badge/texte
  explique pourquoi (modèle, firmware, agent absent).

---

## 4. Maquettes textuelles

### 4.1 Dashboard

```
┌ Sidebar ────────┬───────────────────────────────────────────────────────────┐
│ [logo] TUXEDO   │ TUXEDO INFINITYBOOK PRO 14 GEN9          [Modifier le profil]│
│        Démo     │ Moniteur système                                           │
│ ─────────────── │ Capteurs et état du profil actif                           │
│ Dashboard  ●    │ ┌───────────────────────────────────────────────────────┐ │
│ Profils         │ │ [icône] PROFIL ACTIF          [Équilibré][Secteur]     │ │
│ Outils          │ │         Default               [À la demande][144 Hz]   │ │
│                 │ │         Performance maximale…                          │ │
│ ─────────────── │ └───────────────────────────────────────────────────────┘ │
│ PROFILS UTILISÉS│ ┌ Processeur ──────────────────────────────── [OK] ────┐ │
│ ⚡ Secteur      │ │  ◔ 56 °C   ◔ 3,44 GHz   ◔ 39 %   ◔ 30,0 W            │ │
│   Default       │ │  Historique (60 s)                              ● CPU  │ │
│ 🔋 Batterie     │ │  ╭─────────────────────────────────────────────────╮  │ │
│   Powersave ext.│ │  ╰─────────────────────────────────────────────────╯  │ │
│                 │ └───────────────────────────────────────────────────────┘ │
│                 │ ┌ GPU dédié ─────────────────────── NVIDIA RTX 4060 ───┐ │
│                 │ │  ◔ 61 °C  ◔ 1,40 GHz  ◔ 42 W  ◔ —  (non exposé)      │ │
│                 │ │  GPU intégré 53 °C · 12/16 cœurs · eDP-1 (interne)   │ │
│                 │ └───────────────────────────────────────────────────────┘ │
│                 │ ┌ État du système ─────┐ ┌ Disponibilité des capteurs ┐ │
│                 │ │ PPD        Équilibré │ │ CPU                56 °C   │ │
│                 │ │ Alimentation Secteur │ │ Ventilateur GPU  [N/A]    │ │
│                 │ └──────────────────────┘ └───────────────────────────┘ │
│ ─────────────── │ ┌ Historique (60 s) ────────────────────────────────────┐ │
│ Réglages        │ │ CPU ╭──╮     ╭─╮             GPU ╭──╮                  │ │
│ Support         │ │     ╰──╯─────╯ ╰──               ╰──╯                  │ │
│ À propos        │ └───────────────────────────────────────────────────────┘ │
└─────────────────┴───────────────────────────────────────────────────────────┘
```

L'en-tête, les cœurs, le mode graphique et l'état de l'agent viennent du
snapshot réel (`device`, `cpu`, `prime`, `display`) ; `DemoData` ne sert plus
qu'aux capteurs/historique en **mode démonstration global**, toujours badgé.
La collecte simulée ne tourne que si `AppState.page === "dashboard"` ; c'est
le contrat « collecte à la demande » de la cible.

Navigation directe depuis la carte « profil actif » :

- clic (ou Entrée/Espace) sur le bloc identité → éditeur du profil actif
  (`AppState.editProfile`) ; bouton accessible, anneau de focus, infobulle
  traduite ;
- clic sur le badge du mode graphique → `AppState.openSettings("prime")` :
  la page Réglages défile jusqu'à la carte PRIME (`PageScroll.scrollToItem`).

Les jauges CPU/GPU et le bloc ventilateur viennent de `HardwareData`
(`GetHardwareSnapshotJSON` + `GetFanDataJSON`) avec repli démo ; la ligne
ventilateur affiche vitesse réelle, consigne, mode (`off|auto|manual`) et
backend, en lecture seule (voir §16).

### 4.2 Profils

```
Profils                                            [Importer][Exporter][Nouveau]
Presets du constructeur et profils personnalisés

[Tous][Presets][Perso][Utilisés]              7 profils  [Rechercher…]  [grille]

┌──────────────────────┐ ┌──────────────────────┐ ┌──────────────────────┐
│ ●───          ⚡ 🔋 ⋮│ │ ○───             ⋮   │ │ ○───          ⚡  ⋮  │
│        (icône)       │ │      (icône)         │ │      (icône)         │
│        Default       │ │  Cool and breezy     │ │  Powersave extreme   │
│        (Actif)       │ │                      │ │                      │
│  Performance max…    │ │  Réduit la puissance…│ │  Réduction forte…    │
│  8 cœurs   0,8–3,5G  │ │  8 cœurs  0,8–1,8G   │ │  4 cœurs  0,8–1,2G   │
│  Performance  100 %  │ │  Discret  100 %      │ │  Silencieux 60 %     │
│  144 Hz  Activé      │ │  60 Hz  Activé       │ │  60 Hz  Désactivé    │
└──────────────────────┘ └──────────────────────┘ └──────────────────────┘
[ + Créer un profil ]
```

- Filtres par état, recherche texte, bascule grille/liste (`Ctrl+F` pour la
  recherche).
- Grille responsive **plafonnée à 4 colonnes** : 1 colonne < 700 px, 2 de
  700 à 1049 px, 3 de 1050 à 1399 px, 4 au-delà (350 px par colonne,
  gouttière incluse). Chaque carte garde au moins 300 px
  (`Layout.minimumWidth`), la zone de lecture passe à 1440 px sur cette page
  pour éviter tout débordement horizontal. La carte « nouveau profil » suit la
  même règle.
- Carte active : bordure `accent` + badge « Actif » ; l'interrupteur active le
  profil manuellement (`AppState.manualProfileId`).
- Kebab : Modifier, Dupliquer, Exporter, Supprimer (destructif → dialogue).

### 4.3 Éditeur de profil

```
PROFILS
Default                                     [Retour] [Réinitialiser] [Enregistrer]
Profil personnalisé

┌ Général ──────────────────────────────────────────────────────────────────┐
│ Nom                          [Default                                ]     │
│ Description                  [Performance maximale…                  ]     │
│ ───────────────────────────────────────────────────────────────────────── │
│ Secteur                                              (interrupteur ON)     │
│ Batterie                                             (interrupteur OFF)    │
└───────────────────────────────────────────────────────────────────────────┘
┌ Affichage ─────────────────────────────────────── [Pris en charge] ───────┐
│ Définir la luminosité                                (interrupteur ON)     │
│ Luminosité                                    100 %  [− ────────●── +]     │
│ Définir la résolution                                (interrupteur OFF)    │
│ Résolution                                    [2560 × 1600 ▾] (désactivé)  │
│ Définir le taux de rafraîchissement                  (interrupteur ON)     │
│ Taux de rafraîchissement      [Pris en charge] [165 Hz ▾]                 │
│ Backend hyprland · eDP-1 · Mode actuel 2560 × 1600 @ 60 Hz                │
└───────────────────────────────────────────────────────────────────────────┘
┌ Processeur ───────────────────────────────────────────────────────────────┐
│ Cœurs en ligne                                  8 / 8  [− ──────●── +]     │
│ Forcer la performance maximale                       (interrupteur ON)     │
│ Préférence d'énergie (EPP)                         [performance ▾]         │
│ Fréquence minimale                              0,8 GHz [− ─●────── +]     │
│ Fréquence maximale                              3,5 GHz [− ─────●─ +]     │
│ Désactiver le turbo                                  (interrupteur OFF)    │
└───────────────────────────────────────────────────────────────────────────┘
┌ Webcam … ─────────────────────────────────────────────────────────────────┐
┌ Ventilateurs ─────────────────────────────────────────────────────────────┐
│ Profil de ventilateur                              [Performance ▾]         │
│ Vitesse minimale / maximale / offset + courbe température → vitesse       │
└───────────────────────────────────────────────────────────────────────────┘
┌ ODM / TDP / cTGP ──────────────────────────────────────── [tuxedo-io] ───┐
│ Profil ODM                                      [Enthusiast ▾]            │
│ Limite PL1 [ 40 ] W   Limite PL2 [ 80 ] W                                │
│ Offset cTGP NVIDIA [ 12 ] W                                               │
│ Enregistré dans le profil ; appliqué à son activation.                    │
└───────────────────────────────────────────────────────────────────────────┘
```

Le bandeau d'actions est en haut de page (`Retour`, `Réinitialiser`, sauvegarde
secondaire « ghost »). La **voie principale** est une barre d'action flottante
en bas de la zone de contenu, hors flux de défilement : état
(`dirty`/validation), `Annuler`, `Enregistrer`, spinner pendant
l'enregistrement. Un espaceur en fin de contenu évite de masquer le dernier
champ. Les modifications sont locales ; le daemon valide à l'enregistrement.

### 4.4 Réglages

```
Réglages                                        [Données de démonstration]
Comportement global, bascule automatique et services

┌ Apparence et format ──────────────────────────────────────────────────────┐
│ Thème                     [Système][Clair][Sombre]                        │
│ Langue                    [English][Français][Deutsch]                    │
│ Unité de température      [°C][°F]                                        │
│ Réduire les animations                             (interrupteur OFF)     │
└───────────────────────────────────────────────────────────────────────────┘
┌ Bascule automatique ────────────────────────────────────────────── [OK] ──┐
│                    Secteur                    Batterie                    │
│ Économie       [Powersave extreme ▾]     [Powersave extreme ▾]            │
│ Équilibré      [Office silencieux ▾]     [Office silencieux ▾]            │
│ Performance    [Gaming ▾]                [Office silencieux ▾]            │
│ PPD reste propriétaire de platform_profile et de l'EPP…                   │
└───────────────────────────────────────────────────────────────────────────┘
┌ Charge ─────────────────────────────────────── [Pris en charge] ──────────┐
│ Profil de charge [balanced ▾] · Priorité [balanced ▾]                     │
│ Seuil de début 80 % · Seuil de fin 90 %                                   │
└───────────────────────────────────────────────────────────────────────────┘
┌ Affichage et agent de session ───────────────── [Pris en charge] ─────────┐
│ État de l'agent : 0.4.2 · Backend : hyprland · eDP-1                     │
│ Mode actuel : 2560 × 1600 @ 60 Hz · 12 modes      [Actualiser l'agent]    │
└───────────────────────────────────────────────────────────────────────────┘
┌ Mode graphique (PRIME) ───────────────────────────────────────────────────┐
│ Mode [À la demande ▾]                    [Appliquer et redémarrer]        │
└───────────────────────────────────────────────────────────────────────────┘
┌ ODM / TDP / cTGP ───────────────────────────────── [Transitoire] ────────┐
│ Profil ODM [Équilibré ▾] [PPD propriétaire]                    [Forcer]   │
│ Autoriser TCC à écrire platform_profile                    (interrupteur) │
│ ⚠ PPD reste la source de vérité par défaut ; consentement explicite.      │
│ Limite pl1 [ 35 ] W   Limite pl2 [ 45 ] W                   [Appliquer]   │
│ Offset cTGP NVIDIA [ 5 ] W                                  [Appliquer]   │
│ Appliqué immédiatement ; non persisté dans le profil.                     │
└───────────────────────────────────────────────────────────────────────────┘
┌ Services ─────────────────────────────────────────────────────────────────┐
│ Lancer dans la barre système au démarrage · Bloquer la mise en veille     │
└───────────────────────────────────────────────────────────────────────────┘
```

### 4.5 Outils / Support / Info

- **Outils** : minuterie d'extinction (activation, heure/minute, statut
  logind), **éclairage éditable** (clavier blanc/RGB, luminosité, couleurs,
  lightbar, mode statique/respiration si exposé) via `SaveSettingsJSON`,
  **webcam UVC éditable** lorsque l'agent de session expose l'écriture
  (`User1.GetUvcWriteSupported`) : nom, valeur courante, plage/pas/défaut,
  menu, catégorie. Aucun aperçu vidéo, aucun setter `Manager1`.
- LUKS hors périmètre (décision utilisateur), aucun écran/contrôle.

#### Contrôles UVC (Outils)

- Une section par caméra (`webcam.devices[]`), masquée si aucune caméra/switch.
- Si l'agent n'expose pas l'écriture (`GetUvcWriteSupported=false`,
  `UnknownMethod` ou service absent) : chaque contrôle est affiché en lecture
  seule — nom, valeur courante (entier, booléen ou libellé de menu), badge
  « Lecture seule », plage `min–max`, pas, défaut et catégorie. Aucun éditeur
  n'est instancié.
- `controlsSupported=false` : « Contrôles non disponibles » uniquement si une
  caméra est présente. `controlsError` et `controlsTruncated` sont affichés
  comme avertissements.
- Aucun aperçu ; la preview Qt Multimedia est hors périmètre de cette carte.

#### Écriture UVC en direct (Outils)

- **Activation** : `User1.GetUvcWriteSupported` est interrogé lors de la sonde
  `User1`. `false`, `UnknownMethod` (agent plus ancien) ou service absent →
  lecture seule, sans erreur bloquante.
- **Chemin** : uniquement `webcam.devices[].devicePath` du snapshot `Manager1`
  (`/dev/videoN`, nœud de capture). Le QML ne fabrique jamais de chemin.
  `DaemonClient.setUvcControl` revalide côté Rust le chemin (`/dev/videoN`,
  ≤ 64 octets, sans `..`) et la clé (`[a-z0-9_]{1,64}`) avant tout appel ; tout
  refus local produit `InvalidArgs`.
- **Allowlist** : fermée côté `tcc-user`/`tcc-hw` (15 CIDs UVC). La GUI
  n'instancie un éditeur que si le contrôle est exposé et que ses drapeaux
  autorisent l'écriture (`!readOnly && !disabled && !grabbed && !inactive &&
  !writeOnly`). Les nœuds `metadata` ne sont jamais éditables.
- **Types** : entier → curseur borné `min`/`max`, pas `step>0?step:1`, commit
  au relâchement (un seul appel par geste, pas à chaque pixel ; clavier et
  molette committent par cran) ; booléen → interrupteur 0/1 immédiat ;
  `menu`/`integerMenu` → liste déroulante sur les libellés, la valeur transmise
  est l'index UVC (`menu[].index`), jamais la position du modèle.
- **Attente** : un seul appel à la fois. Pendant `agentUvcWritePending`, tous
  les éditeurs sont désactivés et le contrôle visé affiche « Application… ».
- **Retour** : `true` = valeur écrite ; `false` = valeur déjà courante
  (idempotent, pas une erreur). Après succès, le worker relit
  `Manager1.GetHardwareSnapshotJSON` même si le bail capteurs est suspendu
  (page Outils), puis les curseurs reprennent la valeur réelle.
- **Erreurs stables**, affichées inline sur le contrôle visé : `Busy` (caméra
  occupée), `Stalled` (device perdu/bloqué), `OutOfRange`, `NotSupported`,
  `InvalidArgs`, `LimitsExceeded` (20/s), `Unavailable`, `Transport`, `Failed`.
  Aucun message D-Bus brut n'est publié vers QML. En cas d'échec, le brouillon
  revient à la valeur du snapshot (aucun état optimiste) et le message reste
  visible jusqu'au prochain essai.
- **Session non privilégiée** : l'écriture passe par `tccd-user` (`User1`, bus
  de session, non root). `tccd` conserve `controlsWritable=false` et n'est
  jamais utilisé pour écrire. Aucun secret, aucune commande shell.

#### Minuterie d'extinction (Outils)

- Interrupteur **Activer la minuterie** + sélecteurs **heure** (00–23) et
  **minute** (00–59), sans saisie libre.
- **Appliquer** écrit `Settings.shutdownTime` (`HH:MM`) via
  `Admin1.SaveSettingsJSON` ; **Annuler la planification** envoie
  `clearShutdownTime: true`. Rien n'est planifié tant que « Appliquer » n'a pas
  abouti.
- Badge et lignes d'état issus de `Manager1.GetShutdownTimerJSON` :
  planifié + échéance locale + source (`settings`/`logind`), annulé,
  indisponible, ou **erreur** (`source=error`) avec message générique — jamais
  confondu avec « annulé ». L'édition est alors désactivée tant que l'erreur
  persiste (`enabled=false` conservé).
- Brouillon local : l'heure/minute et l'interrupteur ne sont pas optimistes ;
  après refus, le brouillon est conservé ; après succès, `shutdownTime` est
  relu et le statut se rafraîchit au cycle suivant (≤ 2 s).
- Désactivé si `Admin1` manque, si les réglages ne sont pas live, si le statut
  logind est indisponible ou pendant `savingSettings`.
- L'heure est locale ; fuseau/DST et échéance sont gérés par `tccd`/logind. La
  GUI n'exécute aucune commande (`systemctl`, `shutdown`).
- **Support** : diagnostic système (progression → ticket → formulaire),
  contact réel uniquement.
- **À propos** : logo sur plaque claire, versions réelles (application via
  `Qt.application.version`, daemon via `GetVersion`, contrat IPC, noyau,
  architecture, modèle/SKU, driver TUXEDO), journal des modifications
  **statique** et licences. Aucune valeur de `DemoData`.
- **Support** : seules les informations réelles sont affichées. Le diagnostic
  intégré, WebFAI et AnyDesk n'ont pas de backend : leurs cartes sont
  absentes, aucun ticket/timer/installation n'est simulé. Le site TUXEDO
  s'ouvre par `Qt.openUrlExternally` (URL fixe, aucun shell).

### 4.6 Tray et dialogues

#### Tray natif

- `SystemTrayIcon` + `Menu`/`MenuItem`/`MenuSeparator` de
  `Qt.labs.platform`, icône `assets/brand/taskbar_icon_free.svg` (rouge
  TUXEDO), tooltip « TuxedoRustCenter — profil actif ».
- Menu : Ouvrir, profils (radio sur le profil actif), Autostart, Bloquer la
  veille, **Fn-Lock (interrupteur si `writable`/`enabledKnown`, sinon état en
  lecture seule)**, mode graphique (lecture seule), version, Quitter.
- Clic simple/double : afficher/masquer la fenêtre. Fermeture de fenêtre :
  masquer si le tray est disponible, sinon quitter. `--tray` démarre masqué
  (fenêtre visible si le tray n'est pas disponible).
- Sous Linux, Qt Labs Platform s'appuie sur `QSystemTrayIcon` (Qt Widgets) :
  la GUI est donc lancée avec une `QApplication` (glue C++ minimale, aucune
  logique métier). Tray indisponible (GNOME sans extension, Wayland sans SNI,
  module absent) : aucun crash, notice dans Réglages → Affichage/agent,
  fenêtre normale.
- Si `trayAvailable` devient faux alors que la fenêtre était masquée, elle est
  réaffichée immédiatement (sinon plus aucun moyen de la rappeler). Limite Qt :
  `SystemTrayIcon.available` est une propriété **constante** (pas de
  `availableChanged`) ; l'état est lié à la création et une disparition de
  l'hôte SNI en cours de vie n'est pas observable par cette API.
- Autostart : case « Lancer dans la barre système au démarrage » → fichier
  `$XDG_CONFIG_HOME/autostart/tuxedo-rust-center.desktop` (`Exec=… --tray`,
  `X-GNOME-Autostart-enabled=true`), écriture atomique, aucun privilège,
  aucune commande shell. Le bureau peut désactiver l'entrée.
- Sécurité autostart : `autostart` créé en **0700**, `$XDG_CONFIG_HOME` vérifié
  (propriétaire = utilisateur, `mode & 0o022 == 0`), fichiers symlinks refusés,
  fichier final **0644** vérifié, `Type=Application` + `Exec=… --tray` exigés
  pour considérer l'entrée active, temporaire nettoyé en cas d'échec, `fsync`
  du répertoire après `rename`. Un répertoire 0775 préexistant est refusé avec
  une erreur explicite, sans écriture.
- Menu profils : chaque tentative reconstruit les entrées radio, ce qui
  resynchronise la coche sur `LiveData.resolvedActiveId` même après un refus
  (l'aperçu `TrayMenu` utilise la version réelle `Qt.application.version`, pas
  la version de démonstration).

```
┌ TrayMenu (312 px) ────────────────┐   ┌ ConfirmDialog (440 px) ─────────┐
│ [logo] TUXEDO Control Center      │   │ [!] Supprimer ce profil ?       │
│        Version 0.1.0  [Déconnecté]│   │ « Gaming » sera définitivement  │
│ ───────────────────────────────── │   │ supprimé. Les profils assignés… │
│ ACTIVER UN PROFIL                 │   │ ─────────────────────────────── │
│  Default                ✓         │   │            [Annuler][Supprimer] │
│  Cool and breezy                  │   └─────────────────────────────────┘
│  …                                │
│ ───────────────────────────────── │   Toast (bas droite)
│ Bloquer la veille      (switch)   │   ┌─────────────────────────────┐
│ ───────────────────────────────── │   │ ▍ « Gaming » activé       × │
│ Ouvrir la fenêtre                 │   └─────────────────────────────┘
│ Quitter                    (rouge)│
└───────────────────────────────────┘
```

---

## 5. États de chargement, vide, erreur

| État | Composant | Règle |
|---|---|---|
| Chargement | `LoadingState` | Jamais d'écran blanc : spinner + libellé. Spinner statique (couleur + texte) si `reduceMotion`. |
| Vide | `EmptyState` | Toujours une action de sortie (créer, réessayer, changer de filtre). |
| Erreur | `StatusBadge` + `InfoRow` | Message compréhensible + cause + action. Pas de code d'erreur seul. |
| Non disponible | `Gauge` (—), `StatusBadge` | Le capteur absent est affiché explicitement, avec tooltip de compatibilité. |
| Agent absent | Bandeau/badge `warn` | Le refresh rate est marqué « non pris en charge », jamais silencieusement ignoré. |
| Déconnecté | Badge sidebar + tray | « Déconnecté » visible en permanence ; l'UI reste navigable. |
| Erreur daemon | `ConnectionBanner` en haut de la zone de contenu | Visible dès que `lastError` est non vide (daemon absent, bus inaccessible, payload refusé), avec un bouton « Réessayer ». Le badge « Démo » de la sidebar reste distinct : il décrit la source des données. |

Les pages ne bloquent jamais l'interface : les appels D-Bus seront
asynchrones (Rust) et l'UI affichera un état pendant l'attente.

### 5.1 Visibilité matérielle : absence vs indisponible/stale

- **Absence confirmée** = snapshot live fiable (`HardwareData.liveKnown`,
  c.-à-d. `isLive && !stale`) avec `available=false`/`supported=false` (ou GPU
  `present=false`). Le bloc est alors **masqué**, pas seulement grisé.
- **Chargement / démo / payload périmé** (`!isLive` ou `stale`) : aucun
  masquage, pour éviter le flicker et les faux « absent ». Les blocs affichent
  l'état « collecte inactive », « démo » ou la dernière valeur connue.
- Helpers : `HardwareData.visibleIfAvailable(value)` et
  `visibleIfPresent(present)` centralisent la règle ; les pages exposent des
  propriétés (`chargingVisible`, `primeVisible`, `webcamVisible`, …) pour les
  tests.
- Dashboard : cartes iGPU/dGPU et historique GPU masqués si le GPU est absent ;
  la carte CPU reste ; un GPU présent mais sans capteurs garde sa carte avec
  l'état « non disponible ».
- Réglages : PRIME, charge, YCbCr, éclairage (toggle si aucune zone) et
  Fn-Lock sont masqués. La carte Capacités ne montre ODM/TDP/cTGP que si
  `Admin.Hw` répond **et** qu'au moins un de ces matériels est exposé ; si la
  carte reste visible pour YCbCr, un message explique l'absence du service au
  lieu d'afficher des champs inertes.
- Outils : éclairage et webcam masqués si le matériel n'existe pas ; minuterie
  et infos daemon conservées.
- Éditeur : webcam masquée ; la carte ODM/TDP/cTGP devient un simple renvoi
  vers Réglages et disparaît sans matériel ; l'agent d'affichage et le refresh
  restent visibles avec leur état explicite.
- Tray : Fn-Lock et mode graphique masqués si non supportés ; les entrées
  Ouvrir/Profils/Autostart/Power blocker/Version/Quitter restent.

---

## 6. Accessibilité

- **Contraste** : table §2.1, AA texte normal, AAA pour le corps principal.
- **Cible** : 32 px minimum (WCAG 2.5.8), 36–44 px pour les actions
  principales et les rangées de réglage.
- **Clavier** : Tab/Shift+Tab, Entrée/Espace, flèches dans les segments et les
  listes, Échap ferme les popups/dialogues. Raccourcis globaux :
  `Ctrl+1…6` (pages), `Ctrl+F` (recherche profils),
  `Alt+←` (retour).
- **Focus visible** : anneau 2 px, contraste ≥ 3:1, jamais supprimé.
- **Lecteurs d'écran** : `Accessible.name` sur chaque action icône,
  `Accessible.checked` sur interrupteurs/cases/segments, `Accessible.role`
  explicite, jauges décrites comme « libellé + valeur + unité ».
- **Daltonisme** : aucune information portée par la couleur seule ; les badges
  associent toujours un point/icône et un texte.
- **Réduction des animations** : réglage explicite, durées à 0.
- **Courbe de ventilation** : points sélectionnables, `Suppr` retire le point
  actif, anneau de focus visible, infobulle texte « température → vitesse »,
  cible de saisie élargie à 18 px autour de chaque point ; `editable: false`
  laisse le graphe purement informatif.
- **Aides contextuelles** : les icônes d'information de `SelectRow` sont
  focusables et exposent leur texte à `Accessible.description` du champ, donc
  lisibles sans survol.
- **Zoom/HiDPI** : tailles en pixels indépendants, SVG avec `sourceSize` 2×,
  aucun bitmap d'interface.
- **Langue** : changement à chaud, sans redémarrage.

---

## 7. i18n et formats

- FR / EN / DE complets dans `qml/core/Strings.qml` ; parité des clés
  garantie par `tests/qml/tst_design_system.qml`.
- `Strings.t(clé)`, `Strings.tf(clé, [args])` pour les substitutions `%1…%9`.
- Pluriels : `Strings.plural(n, cléSingulier, cléPluriel)` (1 profil / 4 profils).
- Nombres : `toLocaleString` (`Qt.locale`) → « 3,44 » en FR/DE, « 3.44 » en EN.
- Températures : `Strings.formatTemp(celsius)` et `Strings.tempValue()`,
  unité `°C`/`°F` choisie dans Réglages → Apparence ; les seuils, historiques
  et jauges suivent immédiatement.
- Les valeurs techniques (gouverneurs `schedutil`, EPP, profils de charge)
  restent en notation noyau, avec une aide contextuelle.

> Le dictionnaire embarqué est un choix de jalon 1 pour permettre la bascule à
> chaud sans `lrelease`. Au jalon 3, il sera remplacé par des catalogues Qt
> Linguist `.ts/.qm` générés avec `lupdate` ; les clés actuelles servent de
> source d'extraction et les tests de parité restent valables.

---

## 8. Refresh rate de l'écran interne

- Le profil porte `refresh_rate_hz: Option<f32>` + `use_refresh_rate`
  (migration de `refreshRate: -1`).
- L'éditeur propose un interrupteur « Définir le taux de rafraîchissement »,
  puis une liste des modes **exposés par le daemon** (pas de valeurs
  inventées), avec badge d'état :

| État | Affichage | Source |
|---|---|---|
| Supporté | badge vert « Pris en charge », liste active | agent de session actif + backend détecté + modes disponibles |
| Non supporté | badge orange « Non pris en charge », liste désactivée + raison | pas d'agent, compositeur inconnu, aucun mode |
| Erreur | badge rouge + message (ex. mode refusé par le compositeur) | échec d'application remonté par `tccd-user` |

- Informations affichées : backend (`Hyprland IPC`, `Mutter DisplayConfig`,
  `KScreen`, `Sway`, `XRandR`), écran interne (`eDP-1`), mode actuel.
- **Aucune logique privilégiée dans la GUI** : elle écrit le taux dans le
  brouillon de profil ; le daemon valide, l'agent de session applique. La GUI
  ne touche jamais aux écrans externes et ne devine pas les modes.
- En démonstration, un interrupteur « Simuler l'absence de l'agent » permet de
  vérifier l'état « non pris en charge » sans matériel.

Besoins `tcc-ipc` correspondants : capacité d'affichage (backend, output
interne, modes `{width, height, rate, preferred}`, mode courant, statut de
l'agent, erreur éventuelle), signal de changement, et accusé d'application.

---

## 9. Performance

- **Pas de logique métier en QML** : les calculs de profils, matrices et
  courbes vivront dans `tcc-core` ; le QML ne fait que du formatage
  d'affichage.
- **Payload daemon analysé une seule fois** : `LiveData` mémoïse
  `connected + "|" + profilesJson` et ne relance `JSON.parse` que lorsque la
  chaîne change réellement, malgré le polling de 2 s du pont.
- Graphes en `Shape`/`PathSvg` (pas de QtCharts) : un seul nœud GPU par courbe,
  aucun repaint JS. QtCharts n'est pas requis par le shell.
- Jauges en `Shape` (arcs) ; pas de Canvas ni d'image animée.
- Les liaisons coûteuses sont limitées : les cartes de profil sont reconstruites
  uniquement quand le filtre, la recherche ou le profil actif changent.
- La collecte n'est active que sur le dashboard (`running` lié à la page).
- Chargement paresseux des pages : chaque page n'est instanciée qu'à sa première
  visite (7 `Loader` explicites dans le `StackLayout`), puis conservée pour ne
  pas perdre brouillons/état. Le démarrage initial ne construit que Dashboard.
- Animations coupées hors page visible ou fenêtre masquée : spinner
  `LoadingState` (ne tourne plus sur les pages cachées), jauges (350 ms,
  `reduceMotion`), historique Dashboard (échantillonnage 2 s, aligné sur les
  capteurs). `DemoData` s'arrête aussi si la fenêtre est masquée.
- Fenêtre masquée (tray) : le worker passe son polling de 2 s à 10 s (les
  commandes restent immédiates) et ne renouvelle plus le bail capteurs.
- Aucune ombre/blur ; les icônes teintées sont les seuls effets.
- Mesure 2026-11-06 (release, Xvfb 1440×900, `QT_QPA_PLATFORM=xcb`,
  `QT_QUICK_BACKEND=software`, faux daemon) : démarrage jusqu'à fenêtre mappée
  **266–323 ms** ; Dashboard **127 MiB / 0,6 % CPU** ; Profils **135 MiB /
  0,0 %** ; Réglages **148 MiB / 0,2 %** ; toutes pages visitées **157 MiB /
  0,5 %** ; fenêtre démasquée après unmap X **148 MiB / 0,0 %**, RSS stable sur
  30 s.
- La même mesure avec le backend GL par défaut (llvmpipe sous Xvfb) donne
  **~267 MiB / ~1,9 % CPU** : l'écart est le coût du rendu logiciel
  (tampons/driver), pas du modèle QML. À re-mesurer sur GPU réel ; le budget
  < 200 Mo n'est **pas** revendiqué dans ce cas.
- Aucune animation si `reduceMotion` ; objectif RSS < 200 Mo atteint avec le
  rendu software, à confirmer sur matériel.

---

## 10. Sécurité et privilèges

- La GUI est **non privilégiée** ; elle ne lit/écrit ni `/sys`, ni `/etc`, ni
  `/var/lib`.
- PRIME, Tomte, WebFAI : actions ponctuelles avec confirmation et état de
  progression ; aucune commande shell construite en QML.
- Import de profils : le QML ne lit pas de fichiers arbitraires ; le daemon
  valide taille, schéma, plages et doublons.
- Écriture UVC : uniquement via `tccd-user` (bus de session, non root) ; le
  chemin `/dev/videoN` vient du snapshot `Manager1` et la clé est revalidée en
  Rust. Aucun accès privilégié, aucune écriture sysfs depuis la GUI.
- Les données de démonstration sont signalées en permanence (badge « Démo »
  dans la sidebar et bandeau sur Outils/Support/À propos).

---

## 11. Ressources et licences

- Police **Titillium Web** — © Accademia di Belle Arti di Urbino, SIL OFL 1.1
  (`assets/fonts/OFL-1.1.txt`), fichiers non modifiés.
- Icônes et logos issus de **TUXEDO Control Center**, © TUXEDO Computers GmbH,
  GPL-3.0-or-later. Les icônes monochromes ont été normalisées (blanc + alpha)
  pour la teinte runtime ; géométrie inchangée.
- Détail complet : `crates/tcc-gui/assets/ATTRIBUTION.md`.
- Aucune police d'icônes (Material Icons) n'est utilisée : les glyphes
  d'interface sont dessinés en QML.

---

## 12. Différences assumées avec l'original

1. **Sidebar plus sobre** : profil actif en carte dédiée, profils AC/BAT
   conservés, utilitaires regroupés en pied de sidebar.
2. **Dashboard enrichi** : jauges + historique 60 s + état système + liste de
   disponibilité des capteurs (l'original n'affichait que 5 jauges).
3. **Pas de dégradés décoratifs ni d'ombres** ; la profondeur vient des
   surfaces et des bordures.
4. **Interrupteurs verts** (convention de l'original) ; le rouge reste
   l'accent de marque.
5. **Refresh rate par profil** avec état explicite (l'original ne propose que
   X11, sans état de support).
6. **Matrice PPD × AC/BAT** éditable (l'original ne connaît que AC/BAT).
7. **Tray natif** : `SystemTrayIcon` de `Qt.labs.platform` (SNI sous Linux),
   chargé de façon tolérante (`Loader`) ; repli fenêtre normale si le module
   ou l'hôte SNI manque. L'aperçu `TrayMenu` reste disponible dans la sidebar.
8. **ODM/TDP/cTGP sont portés par le profil** (comme l'original) :
   `Profile.odmProfile.name`, `odmPowerLimits.tdpValues`,
   `nvidiaPowerCTRLProfile.cTGPOffset` sont édités dans l'éditeur et appliqués
   à l'activation. La carte Réglages → `Admin.Hw` reste une **application
   immédiate transitoire** avancée ; les profils sont la voie normale.

---

## 13. Pont avec `tcc-ipc` / cxx-qt (jalon 3)

Modèles à exposer (lecture) :

- `ProfileListModel` (id, nom, description, preset, icône, assignations),
  `activeProfileId`, `manualProfileId` ;
- `SensorSnapshot` (CPU/GPU : temp, fréquence, ventilateur, puissance,
  disponibilité ; ventilateurs ; charge batterie) ;
- `SystemState` (PPD, source d'alimentation, mode graphique, profil résolu) ;
- `DisplayCapabilities` (backend, output interne, modes, mode courant, agent,
  erreur) ;
- `SettingsModel` (thème, langue, unité, réduction des animations, contrôles
  matériels, matrice, charge, services) ;
- `ToolCapabilities` (éclairage, webcam UVC, Tomte, WebFAI, AnyDesk).

Actions (écriture) : `saveProfile`, `deleteProfile`, `duplicateProfile`,
`importProfiles`, `exportProfile`, `setTempProfile`, `applySettings`,
`applyDisplayMode`, `setCharge`, `setLighting`, `setWebcamControl`,
`startDiagnostics`, `submitAdminJob` (PRIME/WebFAI/Tomte via
`tccd-admin` + polkit avec progression/annulation).

Contraintes d'intégration :

- Les modèles Rust doivent être la seule source de vérité pour les données
  qu'ils exposent. Depuis le jalon 1.5, `LiveData` couvre les profils, le profil
  actif, le PPD et l'alimentation ; `DemoData` ne sert plus que de **repli
  explicite** et de source pour les données sans contrat (capteurs, agent,
  outils). Voir §15.
- L'identité QCoreApplication (`organizationName`, `organizationDomain`,
  `applicationName`) doit être définie dans `main.rs` pour que
  `QtCore.Settings` persiste les préférences locales (sinon avertissement
  QSettings et absence de persistance).
- Les icônes et polices seront chargées depuis
  `/usr/share/tuxedo-rust-center/` (qrc ou chemin disque) ; `AppIcon` centralise
  déjà la résolution des chemins.
- L'agent de session `tccd-user` fournit l'état du refresh rate ; le daemon
  expose la capacité, la GUI ne fait qu'afficher.

---

## 14. Vérifications

```sh
# Analyse QML (Qt 6 ; /usr/bin/qmllint peut être Qt 5 sur certaines distros)
/usr/lib/qt6/bin/qmllint crates/tcc-gui/qml/Main.qml \
  crates/tcc-gui/qml/core/*.qml crates/tcc-gui/qml/components/*.qml \
  crates/tcc-gui/qml/pages/*.qml crates/tcc-gui/tests/qml/*.qml

# Tests Qt Quick (jetons, i18n, composants)
/usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

# Aperçu
/usr/lib/qt6/bin/qmlscene crates/tcc-gui/qml/Main.qml

# Sans écran (validation de chargement uniquement)
QT_QPA_PLATFORM=offscreen /usr/lib/qt6/bin/qmlscene crates/tcc-gui/qml/Main.qml
```

Le shell a été validé en Qt 6.12, rendu OpenGL sous Xvfb, thèmes clair et
sombre, FR/EN/DE. Les captures de `docs/ui/captures/` sont générées à partir
de ce shell et des données de démonstration.

Avertissement `qmllint` connu : « Type <X> not declared as singleton in
qmldir » lorsqu'on analyse un singleton isolément (`Theme.qml`,
`SettingsData.qml`, `AdminHwData.qml`, …), alors que `qml/core/qmldir` les
déclare bien et que le chargement runtime fonctionne. C'est un faux positif
d'import, de la même classe pour chaque singleton ; aucune alerte d'une autre
classe n'est émise.

---

## 15. Séparation `DemoData` / `LiveData`

> Cette section décrit le jalon 1. L'état courant des sources réelles est
> documenté au §20 : `DemoData` ne fournit plus ni identité appareil, ni
> versions, ni cœurs, ni mode graphique, ni état de l'agent.

### 15.1 Deux sources, deux rôles

| Source | Contenu | Vérité |
|---|---|---|
| `qml/core/LiveData.qml` | profils (`GetProfilesJSON`), profil actif, PPD, source d'alimentation, override manuel, erreur de connexion | **daemon** (`tcc-ipc`) |
| `qml/core/DemoData.qml` | capteurs CPU/GPU, historique, charge/éclairage/ODM/NVIDIA simulés, **repli** des profils/état — uniquement en mode démonstration global badgé | aucune (revue visuelle) |

`LiveData` est un singleton **alimenté par `Main.qml`** depuis le pont Rust
`DaemonClient` (`profilesJson`, `connected`, `activeProfileId`,
`activeProfileName`, `ppdProfile`, `powerSource`, `manualOverride`,
`lastError`). Aucune page n'importe le module cxx-qt `Tcc` : le QML ne connaît
que `LiveData` et les signaux de demande d'action d'`AppState`
(`refreshRequested`, `profileActivationRequested`, `tempProfileClearRequested`),
traduits en appels D-Bus dans `Main.qml`.

Aucun composant QML ne fait d'écriture privilégiée : la seule action d'écriture
branchée est `SetTempProfile`/`ClearTempProfile`, déjà exposée par le daemon et
sans privilège particulier.

### 15.2 Contrat de l'adaptateur

Entrées (poussées, jamais lues depuis D-Bus dans l'adaptateur) :
`connected`, `profilesJson`, `activeProfileId`, `activeProfileName`,
`powerSource`, `ppdProfile`, `manualOverride`, `lastError`.

Sorties : `profiles`, `profileById(id)` (→ `null` si absent), `activeProfile`,
`activeName`, `activeDescription`, `assignedAc`, `assignedBat`, `refreshRate`,
`useRefRate`, `power`, `ppd`, `usingDemo`, `isLive`, `parseError`.

### 15.3 Règles de repli

1. `connected === false` → **aucune** analyse du JSON ; `profiles` pointe sur
   `DemoData.profiles`, `usingDemo = true`.
2. JSON invalide, non-tableau, tableau vide, ou entrée sans `id`, `name`,
   `display`, `cpu` ou `fan` → rejet **en bloc** (jamais de liste partielle),
   repli démo, `parseError` technique affiché en infobulle.
3. `JSON.parse` n'est exécuté qu'une fois par payload : mémo
   `connected + "|" + profilesJson`, ce qui neutralise le polling de 2 s.
4. Identifiant actif absent de la liste → `activeProfile = null` (jamais un
   profil arbitraire) ; `activeProfileName` du snapshot sert de repli.
5. `assignedAc`/`assignedBat` : le daemon n'expose pas la matrice d'état. En
   live, seule la source courante est connue ; on affiche le profil réellement
   appliqué, jamais une assignation inventée.

### 15.4 Normalisation (unités et sentinelles)

| Champ daemon | Champ d'affichage | Règle |
|---|---|---|
| `cpu.scalingMinFrequency`, `scalingMaxFrequency` (kHz) | `freqMin`, `freqMax` (GHz) | ÷ 10⁶, arrondi 2 décimales |
| `scalingMaxFrequency = -1` | `freqMaxReduced = true` | affiché « Réduit » |
| `display.refreshRate = -1` | `refreshRate = 0` | affiché « Inchangé » |
| `display.useRefRate` | `useRefRate` | faux → « Inchangé » |
| `display.useBrightness` | `useBrightness` | faux → « Inchangé » |
| `webcam.useStatus` | `useWebcam` | faux → « Inchangé » |
| `fan.useControl = false` | `fanMode = "auto"` | « Automatique » |
| `fan.fanProfile` (`Silent`, `Quiet`, …) | `fanMode` (`silent`, `quiet`, …) | minuscules ; inconnu → `auto` |
| `fan.customFanCurve.tableCPU` | `fanCurve` `[{temp, speed}]` | points invalides ignorés |
| `id` ∈ presets `tcc-core` | `preset`, `icon` | miroir de `ProfileId::is_builtin_preset` |
| `odmPowerLimits.tdpValues` | `tdp1`, `tdp2` | index 0/1, `0` si absent |
| `nvidiaPowerCTRLProfile.cTGPOffset` | `ctgp` | watts |

### 15.5 Écrans branchés

- **Dashboard** : nom/description du profil actif, PPD, alimentation, badge
  « override manuel », taux visé par le profil.
- **Profils** : liste live, filtres, profil actif, activation temporaire
  (`SetTempProfile` / `ClearTempProfile`), badge de source + erreur de payload.
  Création, duplication, import/export et suppression répondent « écriture non
  branchée » quand le daemon est connecté.
- **Sidebar / tray** : badge de connexion, profils AC/BAT (source courante),
  bouton « Réessayer » quand une erreur est remontée.
- **Éditeur** : édition réelle via `Admin1`, capacités CPU/écran issues du
  snapshot (`cpu.totalCores`, `availableEpp`, `display`), aucune liste
  inventée.
- **Réglages** : carte « Daemon et agent » (connexion, source des données,
  nombre de profils, PPD, alimentation, dernière erreur, Réessayer).
- **Bandeau** `ConnectionBanner` : visible dès que `lastError` ≠ "" (daemon
  absent ou en erreur), avec « Réessayer ».

### 15.6 Ce qui reste en démonstration

| Domaine | Raison |
|---|---|
| Capteurs CPU/GPU, historique, jauges | simulés **uniquement** hors connexion (mode démo badgé) |
| Tomte, Aquaris, WebFAI, AnyDesk, diagnostic | aucun backend : cartes retirées, aucune action simulée |
| PRIME, webcam UVC, YCbCr | live en lecture ; PRIME/YCbCr sans écriture |
| ODM/TDP/cTGP | écriture transitoire via `Admin.Hw` ; voir §18.7 |
| Charge (type), Fn-Lock, éclairage | écriture persistante via `Admin1.SaveSettingsJSON` ; voir §22 |
| Agent de session, backend compositeur, modes disponibles, mode courant | **exposés** par `User1` (bus de session) ; voir §21 |
| Diagnostic intégré, WebFAI, AnyDesk, ticket support | aucun backend dans cette version : cartes retirées |
| Identité appareil, versions daemon/contrat, noyau, architecture, driver | **exposées** (`device`) et affichées dans Info (§20) |

### 15.7 Besoins à porter au contrat (linux-platform / rust-core)

1. **Matrice d'état** en lecture (et écriture) : `StateMap`/`StateMatrix` par
   `(ppd, power)`, pour les assignations AC/BAT et l'édition des Réglages.
2. **Capacités d'affichage** : backend, output interne, modes
   `{width, height, rate, preferred}`, mode courant, état de l'agent, erreur.
3. **Capteurs** : snapshot événementiel + `SetSensorDataCollectionStatus`
   (collecte à la demande), y compris disponibilité par capteur.
4. **Écriture de profil** : création, duplication, suppression, import/export
   (validation côté daemon), ou méthode dédiée par profil.
5. **Capacités matérielles** : ODM/TDP/cTGP, charge, lighting, webcam UVC.

### 15.8 Limites assumées

- Aucune écriture de profil : les actions destructives et la création
  répondent « écriture non branchée » plutôt que de simuler.
- L'activation manuelle est asynchrone : les interrupteurs sont re-liés par
  `Binding` et suivent le snapshot suivant (≤ 2 s).
- Le taux affiché est la **cible du profil**, pas le mode réel du panneau
  (non exposé).
- `AppState.demoAgentMissing` reste un interrupteur de démonstration pour
  l'état « non pris en charge ».
- En live, `assignedAc`/`assignedBat` reflètent la source courante, pas une
  assignation persistante.

### 15.9 Tests

`tests/qml/tst_live_data.qml` couvre : repli démo hors connexion, absence de
parse quand déconnecté, payload valide et normalisation (kHz, sentinelles,
fanMode, TDP, cTGP, preset/icône), `-1` sur la fréquence max, profil actif,
repli du nom via le snapshot, assignations selon la source, JSON invalide,
payload non-tableau, tableau vide, entrée incomplète, mémo de parse, reconnexion
et signaux de demande d'action — soit 15 cas, en plus des 15 tests du design
system (32 avec `initTestCase`/`cleanupTestCase`).

### 15.10 Durcissement du pont (revue jalon 1)

| Finding | Mesure |
|---|---|
| **R-16** — taille des réponses | `MAX_RESPONSE_BYTES = 4 Mio` vérifié **avant** `serde_json::from_str` ; dépassement = `ClientError::ResponseTooLarge`, jamais de panic. QML reçoit la **re-sérialisation** des modèles validés (`StateSnapshot`, `ProfileCollection`), pas la chaîne brute : les champs inconnus hors modèle ne franchissent pas la frontière. |
| **R-14** — overrides d'environnement | Feature Cargo `dev-overrides` : `TCC_QML_DIR`, `TCC_DBUS_SERVICE` (validé comme nom de bus) et `TCC_DBUS_BUS` ne sont lus qu'avec elle. Sans la feature : `/usr/share/tuxedo-rust-center/qml` et `com.tuxedocomputers.tccd` sur le bus système. |
| **R-15** — threads par appel | Worker unique (`tcc-dbus`) + `sync_channel(8)`. `refresh()`, `setTempProfile()`, `clearTempProfile()` empilent une commande ; file pleine → `busy = true` + message, aucun thread créé. Les commandes en attente sont coalescées (seule la dernière intention compte). |

Conséquences UI : le bandeau d'erreur distingue « daemon injoignable »
(déconnecté) de « erreur du daemon » (refus sémantique, file pleine) ; les
boutons « Réessayer » sont neutralisés tant qu'une commande est en vol
(`LiveData.commandBusy`).


---

## 16. Données matérielles live (`Manager1`)

### 16.1 Chaîne de données

```
Manager1.GetHardwareSnapshotJSON (JSON daemon)
        │  borne 4 Mio, désérialisation
        ▼
src/hardware_view.rs  ── vue typée (sous-ensemble d'affichage, camelCase)
        │  re-sérialisation canonique (clés inconnues supprimées)
        ▼
DaemonClient.hardwareJson  (pont QML)
        │
        ▼
qml/core/HardwareData.qml  ── validation de forme, normalisation, repli démo
        │
        ▼
Dashboard · Réglages · Éditeur · Outils
```

Le type canonique (`tcc_daemon::hardware::HardwareSnapshot`) n'est **pas encore
dans `tcc-ipc`** : la GUI possède une vue typée de lecture (`hardware_view.rs`)
qui ne conserve que les champs affichés. C'est volontairement un **DTO
d'affichage**, pas une copie du contrat : toute clé inconnue est supprimée à la
re-sérialisation, et un champ absent prend sa valeur par défaut (l'UI affiche
« non disponible »). À porter dans `tcc-ipc` par `rust-core`/`linux-platform`
pour supprimer cette duplication.

### 16.2 Propriétés du pont

| Propriété | Rôle |
|---|---|
| `hardwareJson` | dernier instantané **validé et re-sérialisé** |
| `hardwareError` | erreur du dernier cycle matériel (payload précédent conservé, marqué périmé) |
| `sensorsEnabled` | collecte **automatique** (vraie par défaut) ; le worker ne renouvelle le bail que si le Dashboard est affiché et la fenêtre visible. Aucun interrupteur UI |
| `busy` | commande en vol / file pleine |

Invokables : `refreshHardware()` (cycle complet) ; `setSensorsEnabled(bool)`
reste disponible pour le diagnostic/CLI mais n'est plus appelé par l'UI.
`refreshHardware` ne crée aucun thread : il empile une commande dans la file
bornée du worker unique (R-15).

### 16.3 Bail de collecte (10 s) et visibilité du Dashboard

- `SetSensorDataCollectionStatus(true)` arme le bail ; `GetHardwareSnapshotJSON`
  le réarme à chaque lecture (TTL 10 s côté daemon).
- Le renouvellement exige **deux** conditions : collecte automatique
  (`DaemonClient.sensorsEnabled`, vraie par défaut) **et** Dashboard affiché
  (`DaemonClient.setSensorPageVisible` + fenêtre visible, drapeaux partagés lus
  par le worker).
- Dashboard affiché + fenêtre visible → `Set(true)` puis
  `GetHardwareSnapshotJSON` à chaque cycle de 2 s.
- Sortie du Dashboard → la GUI **cesse de relire l'instantané pendant 11 s**
  (TTL 10 s + marge) pour laisser le bail expirer, et **n'envoie pas
  `false`** : l'intention est conservée. Passé ce délai, les lectures
  reprennent (elles ne réarment plus un bail éteint) pour rafraîchir
  charge/éclairage sur les autres pages.
- Retour au Dashboard → `Set(true)` immédiat (et `refreshHardware()`), le bail
  est réarmé.
- Si la GUI disparaît, le daemon coupe la collecte après 10 s : aucun client ne
  peut maintenir la lecture sysfs indéfiniment via la GUI.
- L'UI distingue trois états : **active** (payload `sensors.enabled`),
  **en attente du Dashboard** (page quittée : expiration volontaire, aucun
  `false`) et **expirée** (Dashboard affiché mais bail retombé : suspension,
  redémarrage du daemon). Aucun bouton : une note passive indique que la
  collecte reprend automatiquement.

### 16.3.1 Ventilateurs (`GetFanDataJSON`)

- Source : interface legacy `com.tuxedocomputers.tccd`, lue à chaque cycle
  (2 s) ; elle ne touche pas au bail capteurs.
- DTO `FanView` (`src/fan_view.rs`) : `available`, `backend`, `writable`,
  `controlEnabled`, `mode` (`off|auto|manual`), `fans[]`
  (`index`, `label`, `target`, `temp`, `speed`, `applied`).
- Même durcissement que l'état/profils : borne 4 Mio avant `serde_json`,
  objet JSON obligatoire, re-sérialisation de la vue validée, timeout 3 s.
- `HardwareData` normalise (`mode` inconnu → `unknown`, `null` → `-1`) et
  expose `fan`, `fans`, `fanStale`. Le Dashboard affiche vitesse réelle,
  consigne, mode et backend ; **aucune écriture** n'est proposée.

### 16.4 Écrans

| Écran | Live | Lecture seule |
|---|---|---|
| Dashboard | jauges CPU (temp/fréq/puissance), iGPU/dGPU, ventilateurs (`GetFanDataJSON` : vitesse, consigne, mode, backend), historiques locaux échantillonnés à 2 s (CPU, dGPU, iGPU temp/fréq/puissance) | — |
| Dashboard | état de la collecte + bouton activer/désactiver + états expirée/en attente | écriture matérielle |
| Dashboard | liens : profil actif → éditeur, mode graphique → Réglages §PRIME | — |
| Réglages | charge (profil, priorité, seuils, **type**), **Fn-Lock** (si writable), PRIME/YCbCr en lecture ; ODM/`platform_profile`, TDP, cTGP **en écriture transitoire** via `Admin.Hw` | charge/Fn-Lock via `Admin1.SaveSettingsJSON` ; ODM/TDP/cTGP si `Admin.Hw` répond |
| Éditeur de profil | disponibilité ODM/NVIDIA/webcam ; renvoi vers Réglages pour ODM/TDP/cTGP | tout (ces réglages ne sont pas dans le profil) |
| Outils | **éclairage** (clavier blanc/RGB, zones, lightbar, mode) et webcam (kill switch, périphériques V4L2, contrôles UVC) | éclairage via `Admin1.SaveSettingsJSON` ; contrôles UVC via `User1.SetUvcControl` (session) |

Les écritures charge, Fn-Lock et éclairage passent exclusivement par
`Admin1.SaveSettingsJSON` (polkit côté admin), jamais par les setters
`Manager1`. Les contrôles sont désactivés si le service d'administration ou le
matériel manque.

### 16.5 Repli et dégradation

1. `connected === false` → aucune analyse du JSON ; `HardwareData` expose le
   jeu de démonstration (`usingDemo = true`).
2. JSON invalide, non-objet, ou sections `sensors`/`charging` absentes → repli
   démo + `parseError` (infobulle).
3. Sections présentes mais vides (matériel absent) → `isLive = true`,
   `available = false` partout, jauges « — » : on ne mélange pas live et démo.
4. Erreur du bridge (`hardwareError`) → dernier payload conservé + badge
   « données périmées », jamais de zéros trompeurs.
5. Webcam, ventilateurs : si le contrat ne les expose pas (ou pas encore pour
   `sensors.fans`), l'UI affiche « non disponible » au lieu d'inventer.

### 16.6 Limites assumées

- **Écritures** : charge (dont type), Fn-Lock et éclairage via `Admin1` ;
  ODM/TDP/cTGP dans le profil (persistants) et via `Admin.Hw` (application
  immédiate transitoire). PRIME et YCbCr restent en lecture seule ; aucun
  setter `Manager1`.
- Les ventilateurs viennent de `GetFanDataJSON` (contrat legacy), pas de
  `HardwareSnapshot` : `sensors.fans` reste accepté par la vue Rust mais n'est
  plus utilisé par l'UI.
- Le pilotage des ventilateurs reste au daemon (profil + courbes) : le
  Dashboard affiche vitesse, consigne, mode et backend en lecture seule.
- Le bail capteurs suit la visibilité du Dashboard : tant qu'une autre page est
  ouverte, les capteurs ne sont volontairement pas rafraîchis (charge et
  éclairage le sont après la fenêtre de grâce de 11 s).
- L'historique du dashboard est **local à la GUI** (échantillons 1 s, 48
  points) : le contrat n'expose pas de série temporelle.
- `onlineCores`/`totalCores` et l'agent de session restent des données de
  démonstration (absents du snapshot matériel).
- Le contrat matériel vit dans `tcc-daemon` ; sa promotion dans `tcc-ipc`
  supprimera la vue locale.

### 16.7 Tests

`tests/qml/tst_hardware_data.qml` (13 cas) : repli démo hors connexion, absence
de parse quand déconnecté, normalisation MHz→GHz, ventilateurs
(`GetFanDataJSON` : mode, backend, vitesse, consigne, indisponible), charge,
éclairage, Fn-Lock, ODM/TDP, NVIDIA, PRIME, YCbCr, webcam, états du bail
(active/inactive/**en attente**/expirée selon la visibilité du Dashboard),
drapeau « périmé » (matériel et ventilateurs), JSON invalide/non-objet/sections
manquantes, dégradation section par section, mémo de parse et signaux de
collecte.

`tests/qml/tst_navigation.qml` (7 cas) : profil actif → éditeur, retour
contextuel, mode graphique → Réglages §PRIME (section consommée une fois),
liste des pages et présence des clés i18n de navigation.

Côté Rust : `hardware_view` (sous-ensemble, clés inconnues, types faux,
nullables), `fan_view`, `decode_hardware`/`decode_fans` (borne 4 Mio,
non-objet, re-sérialisation) et `next_lease_state` (renouvellement lié à la
visibilité, suspension de 11 s sans `false`, désactivation explicite).

---

## 17. Édition et enregistrement de profil (phase 1)

### 17.1 Disponibilité du service d'administration

L'éditeur interroge `Admin1` par le pont Rust (détection sans activation D-Bus,
voir `docs/ui/PROFILE_EDITING.md` §10.1). Trois états sont affichés :

| État | Affichage | Effet |
|---|---|---|
| disponible | badge vert (version) dans l'en-tête | formulaire éditable, Enregistrer actif |
| absent / désactivé | `InlineNotice` warn + « Réessayer » | formulaire en lecture seule |
| contrat incompatible | `InlineNotice` warn (mise à jour requise) | lecture seule |

Aucune écriture n'est tentée sans service : le bouton est désactivé avec une
description accessible expliquant pourquoi.

### 17.2 Formulaire

- Cartes éditables : Général (nom, description), Affichage (luminosité,
  refresh), CPU, Webcam, Ventilateurs (`useControl`, profil, min/max/offset,
  courbe CPU en mode `custom`).
- Cartes en lecture seule : assignation AC/BAT (réglages), résolution (hors
  phase 1).
- **ODM/TDP/cTGP ne sont plus édités par profil** : la carte affiche un renvoi
  « Ouvrir les Réglages » (réglages globaux, transitoires) et disparaît si le
  matériel est absent.
- Sections masquées si le matériel est absent (snapshot live fiable) :
  webcam (aucune caméra/switch), ODM/TDP/cTGP. Pas de champs inertes ni
  d'espace vide.
- Presets : bandeau « preset en lecture seule », Enregistrer désactivé.
- Création : bandeau « création non disponible » (hors phase 1).
- Validation locale par champ (`ProfileRules`) : nom vide/trop long/doublon,
  description, luminosité 0–100, taux 1–1000 Hz, cœurs, fréquences
  0,1–10 GHz et ordre, ventilateurs 0–100 et ordre, offset ±100, courbe
  2–256 points / 0–100 / ordre strict.
  Messages sous la carte concernée, jamais de correction silencieuse.
- Bouton « Modifier la courbe » : ouvre `FanCurveEditor` en mode `custom`,
  sinon toast « sélectionnez Custom ». Le dialogue édite les points CPU
  (champs bornés, ajout/suppression), affiche l'aperçu `FanCurveChart` en
  direct, valide avant « Appliquer » (brouillon dirty). Le graphique de la
  page reflète le brouillon ; nom et table GPU de la courbe existante sont
  préservés par Rust. Clair/Annuler, clavier, `PlainText`.

Interactions de la courbe (`FanCurveChart`, `editable: true`) :

- **Glisser** un point : température et vitesse suivent le curseur, bornées à
  la zone tracée (30–100 °C, 0–100 %). L'ordre reste strictement croissant :
  chaque point est limité à `voisin − 1 °C` / `voisin + 1 °C` ; si l'intervalle
  est impossible, le déplacement est ignoré.
- **Double-clic** sur une zone vide : ajoute un point à la position (refusé
  au-delà de 256 points, ou si l'écart avec un voisin est < 1 °C).
- **Clic droit** sur un point, ou **Suppr** quand il est actif : retire le
  point, jamais sous deux points. Le point actif est agrandi et rempli
  d'accent ; une infobulle affiche « température → vitesse ».
- `editable: false` (aperçus en lecture seule) désactive toute interaction :
  aucun déplacement, aucun ajout, aucune suppression.
- Le composant n'écrit jamais `points` : il émet `pointMoved`, `pointAdded`,
  `pointRemoved` et le parent (`FanCurveEditor`) met à jour le tableau ; les
  deux vues restent synchronisées et Rust revalide le DTO.
- Coût : une réévaluation de l'aperçu par déplacement, bornée à 256 points,
  uniquement pendant le dialogue. Aucun polling, aucun impact sur le démarrage
  ni sur le Dashboard en lecture seule.

### 17.3 Enregistrement

- Un seul enregistrement à la fois : libellé « Enregistrement… », bouton
  désactivé, bandeau d'attente.
- Succès : toast, rechargement depuis les données relues par le daemon.
- Échec : bandeau rouge traduit (`NotAuthorized`, `PolicyKitUnavailable`,
  `Conflict`, `Busy`, `Validation`, `InUse`, `ReloadFailed`, `WriteFailed`,
  `RateLimited`, `PayloadTooLarge`, `Timeout`, `Transport`, …), formulaire
  **conservé**.
- Conflit : si les profils live changent pendant un brouillon ou si le profil
  actif change, bandeau « le profil a changé » + « Recharger » ; **Enregistrer
  est désactivé** tant que le conflit n'est pas résolu. Le worker relit la
  collection juste avant l'écriture et renvoie `Conflict` si un autre client a
  enregistré entre-temps (course résiduelle documentée : pas de CAS côté admin).
- Champs `Option` inconnus (cœurs/fréquences absents du profil) : affichés
  « Auto », désactivés, envoyés `null` — jamais une valeur inventée.
- Résolution et taux : les listes proviennent de `User1` (modes de l'écran
  interne). Un mode enregistré disparu rend le formulaire invalide ; l'agent
  reste seul juge à l'application.
- Aucun chemin, contenu de fichier ou secret n'est affiché : seul un nom de
  champ ou un identifiant peut apparaître en détail.

### 17.4 Limites phase 1

Création, duplication, suppression, import/export sont couverts par la phase
courante. PRIME reste en lecture seule ; les contrôles UVC sont éditables en
direct dans Outils via `User1` (allowlist `tcc-hw`) ; ODM/TDP/cTGP sont édités
**hors du profil**, dans Réglages (§18.7). La courbe GPU n'est pas éditable
(seule la table CPU l'est, nom/GPU préservés).

---

## 18. Réglages globaux persistants (phase 1)

Quatre réglages, la charge (dont type), la matrice PPD × alimentation, le
consentement `ppdOverride`, Fn-Lock et l'éclairage sont enregistrés via
`Admin1.SaveSettingsJSON` ; les écritures ODM/TDP/cTGP passent par `Admin.Hw`
et sont **transitoires**. PRIME et YCbCr restent en lecture seule.

### 18.1 Apparence — unités

- Le segmented °C/°F envoie immédiatement un DTO ; il est désactivé sans
  service d'administration ou pendant l'enregistrement.
- L'unité affichée dans toute l'UI est appliquée **après relecture** du daemon
  (`Settings.fahrenheit`), pas seulement au niveau local.

### 18.2 Contrôle matériel

- Trois interrupteurs : contrôle CPU, ventilateurs, éclairage.
- Désactivés si `Admin1` est absent (badge « service requis ») ou pendant un
  enregistrement ; sinon l'état vient de `GetSettingsJSON`.
- Après un refus, les interrupteurs reviennent aux valeurs du daemon et le
  message traduit est affiché ; après succès, toast et resynchronisation.
- Réglages invalides (schéma, référence de profil absente) : bandeau
  « Réglages invalides ou illisibles » et édition désactivée.
- Si le worker s'arrête pendant un enregistrement, les indicateurs
  « enregistrement » sont réinitialisés par le surveillant (≤ 250 ms) au lieu
  d'attendre le garde-fou de 300 s.
- Le type de clavier détecté reste affiché en description (lecture seule), le
  Fn-Lock devient un interrupteur si `fnLock.writable` et que l'état est
  lisible ; sinon ligne d'information en lecture seule.

### 18.3 Charge

- Profil et priorité : listes déroulantes alimentées par
  `hardware.charging.profiles` / `priorities` (dernier instantané matériel).
- Seuils début/fin : listes `startThresholds` / `endThresholds` si
  `thresholdsAvailable` ; libellés en pourcentage, valeurs entières.
- Sélection initiale = valeur matérielle courante, repli sur la valeur
  persistée (`GetSettingsJSON`).
- Seuls les champs **modifiés** sont envoyés (`null` sinon) ; bouton
  « Appliquer » actif uniquement si le brouillon est modifié et valide.
- Validation par champ : valeur hors liste, seuil obligatoire manquant, et
  `début < fin` strict ; message rouge sous le champ concerné, aucun envoi
  tant que l'erreur existe. Rust revalide contre les capacités matérielles
  (défense en profondeur, listes jamais fournies par QML).
- « Réinitialiser » revient aux valeurs lues. Après refus, le brouillon est
  conservé (bandeau d'erreur) ; après succès, relecture du daemon et brouillon
  remis à zéro.
- Édition désactivée si `Admin1` manque, si l'instantané est périmé/absent
  (`HardwareData.isLive`, `stale`) ou pendant un enregistrement.
- Le **type de charge** est éditable si `chargeTypeWritable` : liste
  `chargeTypes` si non vide, sinon valeurs ABI standard, avec « Ne pas
  changer » pour `None`. Les setters `Manager1` ne sont pas utilisés.

### 18.4 Matrice PPD × alimentation

- Tableau 3×2 : `power-saver`, `balanced`, `performance` × AC/batterie. Chaque
  cellule propose « Fallback AC/BAT » + les profils live (presets et
  personnalisés), avec le nom affiché.
- Une cellule vide signifie « pas d'entrée de matrice » : le daemon retombe sur
  `stateMap` (AC/BAT), puis sur le profil par défaut.
- Valeur initiale = `stateMatrix` de `GetSettingsJSON`. Le brouillon compare
  chaque cellule à sa valeur initiale ; « Appliquer » n'est actif que si la
  matrice a changé et qu'aucune cellule n'est invalide.
- Cellule invalide (profil supprimé par un autre client) : message rouge dans
  la cellule, application bloquée tant que la cellule n'est pas corrigée ou
  réinitialisée. Rust revalide clés, doublons et existence des profils.
- « Réinitialiser » revient à la matrice relue. Après refus : brouillon
  conservé + bandeau d'erreur ; après succès : relecture des réglages.
- Édition désactivée si `Admin1` manque, si les réglages sont invalides/absents
  ou si aucun profil live n'est disponible ; boutons désactivés pendant
  `savingSettings`.

### 18.5 Bandeaux

« Service d'administration indisponible » (avec Réessayer), « Enregistrement
des réglages… », erreur traduite (`NotAuthorized`, `Busy`, `Conflict`,
`ReloadFailed`, `Timeout`, …), « Réglages modifiés ailleurs » + Recharger.

### 18.6 Inhibiteur de veille (non persistant)

- Interrupteur « Bloquer la mise en veille » dans la carte **Affichage /
  agent** (et dans le menu tray).
- Mécanisme : `org.freedesktop.login1.Manager.Inhibit("idle",
  "TuxedoRustCenter", …, "block")` sur le bus système, sans root, sans polkit,
  sans commande shell. `block` ne concerne que l'inactivité : une veille
  demandée explicitement par l'utilisateur n'est pas bloquée (`delay` n'est pas
  utilisé).
- Le fd retourné par logind est conservé par le **worker unique** tant que
  l'interrupteur est actif ; il est fermé à la désactivation, à l'arrêt du
  worker ou à la fermeture de la GUI, ce qui libère l'inhibiteur côté logind.
- **Aucune persistance** : l'état n'est pas écrit dans `Settings` ; il disparaît
  à la fermeture. L'UI reflète le fd réel (`PowerBlockerData.blocked`), pas
  l'intention optimiste.
- Désactivé si `XDG_SESSION_TYPE` n'est pas `x11`/`wayland`/`mir`, pendant une
  commande, ou si le worker est arrêté. Erreur générique traduite
  (`NotGraphical`, `Unavailable`, `Refused`, `Timeout`) sans détail de bus.
- Accessible clavier via l'interrupteur ; pas de dialogue ni de confirmation
  (action réversible).

### 18.7 ODM / TDP / cTGP (`Admin.Hw`, application immédiate)

La **voie normale** est le profil : `odmProfile.name`,
`odmPowerLimits.tdpValues` et `nvidiaPowerCTRLProfile.cTGPOffset` sont édités
dans l'éditeur et persistés via `Admin1.SaveProfilesJSON`. La carte ci-dessous
est une application **immédiate et transitoire** avancée.

- **Service** : `com.tuxedocomputers.tccd.Admin.Hw`, objet
  `/com/tuxedocomputers/tccd/Admin/Hw`, interface
  `com.tuxedocomputers.tccd.AdminHw1`, contrat 1. La GUI le **sonde sans
  l'activer** (`NameHasOwner` puis `ListActivatableNames`) toutes les 15 s ;
  `Ping`/`GetContractVersion` ne sont appelés que si le nom est déjà possédé.
  Aucun setter n'est envoyé au démarrage.
- **Disponibilité** : `AdminHwData.available` + `version` ; absence →
  « Service d'administration matérielle indisponible », contrat incompatible →
  message dédié, transport → message dédié. Les contrôles sont alors masqués
  (pas de champs `enabled:false`), **mais la carte reste visible avec la
  notice et « Réessayer »** tant que le matériel ODM/TDP/NVIDIA est présent
  (UIW-03). Elle n'est masquée que si le matériel lui-même est absent (hors
  YCbCr). « Réessayer » ressonde `Admin1` et `Admin.Hw`.
- **ODM** : liste des choix réels — `platform.choices` si disponible, sinon
  **fallback `odm.io.choices`** (`tuxedo-io`). Sélection initiale :
  `platform.current`, sinon `io.defaultProfile`, sinon le premier choix.
  Badge de source `platform_profile` / `tuxedo-io`. `odm.available` est agrégé
  par le daemon (`platform` ou `io`). Bouton **Appliquer** actif seulement si
  le drapeau `odmWritable` est vrai et que le choix a changé. Si `owner=ppd` :
  badge « PPD propriétaire », Appliquer désactivé, bouton **Forcer** visible
  mais actif seulement si le consentement `ppdOverride` est enregistré. La GUI
  **n'envoie jamais** `SetOdmProfileOverride` sans ce consentement (garde
  locale, en plus du refus `PolicyDenied` côté service).
- **Consentement PPD** : interrupteur « Autoriser TCC à écrire
  platform_profile », persisté via `Admin1.SaveSettingsJSON`
  (`SettingsEdit.ppdOverride`). Texte explicite : PPD reste la source de
  vérité par défaut, l'override est un choix conscient. Après refus, le
  brouillon et l'interrupteur reviennent aux valeurs relues.
- **ODM (bornes)** : nom **≤ 64 caractères**, ASCII graphique, sans
  antislash — mêmes règles que `tccd-admin::validate_odm_name`. La valeur
  vient de la liste matérielle ; une saisie hors allowlist est refusée
  localement avec un message sûr, sans appel.
- **TDP (bornes)** : affiché seulement si `hardware.odm.tdpAvailable` et
  qu'au moins une limite `hardware.odm.tdp[]` existe. Sur les modèles où le
  « TDP CPU » officiel passe par le profil ODM (`io.tdpAvailable=false`), les
  curseurs PL sont masqués. Chaque champ (`descriptor`, `current`, `min`,
  `max`), **1 à 3 valeurs** (PL1/PL2/PL4),
  1–1000 W, descripteurs dans l'ordre du firmware et **sans doublon**.
  Une liste de 4 entrées, un ordre inattendu ou une valeur hors bornes bloque
  **Appliquer** et affiche un message rouge. **Appliquer** envoie
  `SetTdpValues` ; le serveur revalide count/valeurs et refuse au-delà de 3.
- **cTGP (bornes)** : champ unique en watts, **0…`ctgpMax`** ;
  `ctgpMax` vaut la borne dynamique si `nvidia.maxCtgpOffset` est exposée,
  sinon **200 W** statiques. Les valeurs négatives sont refusées (plus de
  −200). Masqué si NVIDIA absent ; **Appliquer** envoie
  `SetNvidiaCtgpOffset`.
- **Transitoire assumé** : ces écritures ne sont **pas** enregistrées dans le
  profil ni dans `Settings` ; un badge « Appliqué immédiatement ; non
  persisté » et un texte de carte le rappellent. Après succès : toast,
  relecture matérielle (`requestHardwareRefresh`) et brouillons resynchronisés.
- **Une écriture à la fois** : le worker refuse une seconde demande
  (`Busy`) tant que la précédente n'est pas terminée ; timeouts 10 s en
  lecture, 180 s en écriture. Pendant l'attente, la carte affiche
  « Application en cours… » et les boutons sont désactivés.
- **Erreurs traduites** : `PolicyDenied` (PPD/consentement), `PartialApply`
  (état intermédiaire non restauré), `InvalidArgs`/`Validation`,
  `NotSupported`, `WriteFailed`, `RateLimited`, `Busy`, `NotAuthorized`,
  `Timeout`, `Transport`, service arrêté. Le **formulaire est conservé** pour
  permettre de corriger ou réessayer ; aucun chemin, contenu de fichier ou
  secret n'est affiché (détail borné et filtré).
- **Éditeur de profil** : la carte ODM/TDP/cTGP est remplacée par un renvoi
  « Ouvrir les Réglages » ; aucun champ n'est dupliqué, aucun toggle LED ODM
  informatif n'est conservé.
- **Sécurité** : aucune écriture privilégiée dans la GUI ; elle appelle
  `Admin.Hw`, qui applique polkit avec l'identité de l'appelant. Le service
  n'est pas installé par défaut : l'UI reste fonctionnelle et explique
  l'absence.

### 18.8 Limites

PRIME et YCbCr restent en lecture seule ; Fn-Lock, charge (dont type) et
éclairage sont persistés via `Admin1` ; ODM, TDP et cTGP sont des écritures
transitoires via `Admin.Hw`, jamais persistées dans un profil. Les setters
`Manager1` restent hors UI. La matrice est limitée aux
3 PPD et 2 sources connus, remplacée en bloc, et ne peut référencer que des
profils existants. Aucune révision côté admin : la relecture avant écriture
réduit le risque d'écrasement inter-clients sans le supprimer. Le service
`Admin.Hw` n'étant pas installé par défaut, les contrôles ODM/TDP/cTGP sont
absents tant qu'il ne répond pas.

---

## 19. Gestion de la collection de profils

### 19.1 Création

- Le bouton « Nouveau profil », la carte dédiée et l'état vide ouvrent
  l'éditeur en mode brouillon (nom « Nouveau profil », valeurs par défaut,
  cœurs/fréquences « Auto »).
- Bandeau informatif « Ce profil sera créé à l'enregistrement » ; l'ID est
  généré côté Rust, jamais en QML.
- Après succès : toast avec le nom, éditeur basculé sur le profil créé.

### 19.2 Duplication

- Menu contextuel de la carte → « Dupliquer » (désactivé pour les presets,
  raison dans `Accessible.description`).
- La copie reçoit un ID généré et un nom unique `X (copie)`, `X (copie 2)`.
- Succès : toast « X dupliqué » ; la liste est relue depuis le daemon.

### 19.3 Suppression

- Menu contextuel → « Supprimer » (désactivé pour les presets) → dialogue de
  confirmation explicite.
- Refus `InUse` : bandeau d'erreur traduit, profil conservé, aucune écriture.
- Succès : toast « X supprimé », liste relue.

### 19.4 Import / export de profils

- Bouton **Exporter** dans l'en-tête de la page (et menu de carte) :
  `FileDialog.SaveFile`, filtre `*.json`, nom par défaut
  `TCC_Profiles.json` dans Documents. Exporte les profils personnalisés dans
  une enveloppe versionnée `{ "exportVersion": 1, "profiles": […] }`.
- Bouton **Importer** : `FileDialog.OpenFile`, même filtre. Désactivé sans
  `Admin1` (l'import écrit via polkit) ou pendant un enregistrement.
- L'export reste disponible sans `Admin1` : c'est une écriture locale non
  privilégiée, bornée à 1 Mio, refusant symlinks/dossiers.
- Conflits : ID existant → nouvel ID ; nom existant → `(importé)`,
  `(importé 2)` ; presets ignorés. Jamais d'écrasement silencieux.
- Retour utilisateur : toast de succès avec compteurs (importés, renommés,
  presets ignorés), toast d'annulation, bandeau d'erreur traduit
  (`InvalidImport`, `PathInvalid`, `ReadFailed`, `FileWriteFailed`,
  `NothingToExport`, `Conflict`, `TooManyProfiles`).
- Aucun `JSON.parse` en QML : lecture/validation/sérialisation Rust ; le
  contenu du fichier ne détermine jamais un chemin.

### 19.5 États transverses

- Sans `Admin1` : bandeau « service d'administration indisponible » + bouton
  Réessayer, actions désactivées.
- Pendant une mutation : boutons désactivés (`AdminState.saving`), badge
  « opération en cours » via l'info-bulle.
- Conflit (collection modifiée entre-temps) : erreur `Conflict`, aucune
  écriture, bandeau avec « Recharger » côté éditeur ; la liste se resynchronise
  au prochain cycle.

---

## 20. Capacités réelles (`device`, `cpu`, `display`, `capabilities`)

Le snapshot `Manager1` expose désormais quatre sections consommées par la
GUI. `hardware_view.rs` les désérialise de façon tolérante (champs absents
par défaut, types faux rejetés, entiers `u32` bornés) et `HardwareData.qml`
les normalise (chaînes ≤ 128 caractères sans contrôle, cœurs bornés à 4096,
`maxCtgpOffset` 0–200).

| Section | Champs utilisés | Écran |
|---|---|---|
| `device` | `productName`, `vendor`, `model`, `sku`, `kernelRelease`, `architecture`, `tuxedoDriverVersion`, `isTuxedo` | Dashboard (eyebrow), Info |
| `cpu` | `totalCores`, `onlineCores`, `availableGovernors`, `availableEpp`, `hasCpuControl`, `writable` | Dashboard, Info, éditeur |
| `display` | `available`, `hasInternalDisplay`, `internalConnector`, `connected`, `source` | Dashboard, Réglages, éditeur |
| `capabilities` | `hasBattery`, `hasAc`, `hasIgpu`, `hasDedicatedGpu` | Dashboard (badge alimentation), Réglages |

Règles :

- **Aucune valeur produit de `DemoData` en live** : le modèle d'appareil, les
  versions, les cœurs, le mode graphique et l'agent viennent du snapshot.
  En démonstration globale (daemon absent), ces champs sont vides et l'UI
  affiche « non disponible » ; seuls les capteurs/historique simulés restent,
  avec le badge « Démo ».
- **Mode graphique** : affiché seulement si `prime.available` ; libellé dérivé
  de `prime.mode` (`iGPU`, `dGPU`, `on-demand`, `off`). Masqué sinon, y
  compris dans la carte « État du système ».
- **Cœurs** : `onlineCores / totalCores` ; si `totalCores == 0`, la mention
  est omise au lieu d'afficher une constante.
- **Agent de session** : l'état réel vient de `User1` (backend, connecteur,
  mode courant, modes, support/erreur). Le Dashboard et les Réglages
  affichent ces valeurs ; si l'agent est absent ou non supporté, un bandeau
  explicite propose « Actualiser l'agent » et aucun mode n'est inventé.
- **Réglages → Affichage** : backend, écran interne, mode courant et nombre
  de modes réels ; repli DRM (connecteur + source) uniquement si l'agent ne
  fournit pas de sortie. Aucun toggle « simuler l'agent absent ».
- **Éditeur CPU** : maximum de cœurs = `cpu.totalCores` (repli sur la valeur
  du profil), EPP = `cpu.availableEpp` (valeur du profil préservée), une
  entrée « Auto » si la liste est vide. `availableGovernors` conditionne le
  toggle « gouverneur performance » ; aucune liste inventée.
- **Support** : les cartes sans backend sont retirées ; contact/site restent,
  le site s'ouvre via `Qt.openUrlExternally`.

---

## 21. Service de session `User1` : résolution, refresh et écriture UVC

`tccd-user` expose `com.tuxedocomputers.tccd.User1` sur le **bus de session**
(`/com/tuxedocomputers/tccd/User`, contrat `SESSION_CONTRACT_VERSION = 1`).
L'affichage et le refresh passent par les méthodes de lecture ; la seule
écriture est `SetUvcControl` (contrôles webcam, session non privilégiée).

### 21.1 Client

- Sonde **sans activation abusive** : `NameHasOwner`, puis
  `ListActivatableNames` ; `Ping`/`GetContractVersion`/`GetVersion` seulement
  si le nom est déjà possédé (timeouts 2 s).
- `GetDisplayInfoJSON` (timeout 3 s) est lu si le service est possédé, ou sur
  action explicite `refreshAgent` (qui peut l'activer).
- `GetUvcWriteSupported` est lu à la même sonde ; `UnknownMethod` (agent plus
  ancien), service absent ou échec → `agentUvcWriteSupported=false`, sans
  erreur bloquante.
- `SetUvcControl(devicePath, controlKey, value)` (timeout 4 s) : chemin et clé
  revalidés en Rust, file bornée, un seul appel à la fois. `true` = écrit ;
  `false` = valeur déjà courante (idempotent). Les erreurs sont mappées en
  codes stables (`Busy`, `Stalled`, `OutOfRange`, `NotSupported`, `InvalidArgs`,
  `LimitsExceeded`, `Unavailable`, `Transport`, `Failed`), jamais en message
  serveur.
- Le JSON est décodé par `display_view.rs` (DTO typé, mode externe impossible)
  et re-sérialisé avant publication ; la vue est bornée à 256 modes.
- Propriétés : `agentAvailable`, `agentVersion`, `agentError`,
  `displayInfoJson`, `agentUvcWriteSupported`, `agentUvcWritePending`,
  `agentUvcWriteError`, `agentUvcWriteDetail`, `agentUvcWriteSucceeded`.
  `AgentData.qml` normalise et expose backend, sortie interne, mode courant,
  résolutions uniques et taux par résolution, ainsi que `uvcWriteSupported`,
  `uvcPending`, `uvcError`, `uvcErrorText` et `uvcSucceeded`.

### 21.2 Écrans

- **Dashboard** : badge du mode courant, backend, sortie interne, mode
  courant ; bandeau + action si l'agent est absent/non supporté.
- **Réglages** : état de l'agent, backend, sortie, mode courant, nombre de
  modes ; repli DRM si l'agent ne fournit pas de sortie.
- **Éditeur** :
  - Résolution : liste des `WxH` **uniques** de `internalOutput.modes` ; le
    toggle `useResolution` n'est actif que si l'agent est utilisable.
  - Refresh : liste des taux **de la résolution choisie** ; aucune
    combinaison absente n'est proposée.
  - Mode enregistré disparu : le formulaire est invalide et invite à choisir
    un mode réel ; l'enregistrement est bloqué.
  - Agent indisponible : contrôles désactivés, valeurs du profil **préservées**
    (aucun effacement), bandeau + « Actualiser l'agent ».
  - `ProfileEdit` transporte `useResolution`, `xResolution`, `yResolution` ;
    ils sont envoyés uniquement si la résolution est demandée. Rust applique
    au `profile.display` et conserve les valeurs existantes sinon.

### 21.3 Limites

- Les modes dépendent du compositeur et de la session : XRandR, Hyprland,
  GNOME/Mutter, KDE/KScreen ou Sway. Un compositeur non supporté donne
  `available=false`/`supported=false` et un bandeau, jamais une liste
  inventée.
- L'agent peut être endormi (nom activable) : l'UI le signale et propose une
  actualisation explicite.
- La résolution n'est appliquée que par l'agent ; la GUI ne fait aucune
  écriture d'affichage et ne modifie jamais un écran externe.

---

## 22. Écritures matérielles persistantes (charge, Fn-Lock, éclairage)

Toutes passent par `SettingsEdit` → `Admin1.SaveSettingsJSON` (polkit côté
admin), puis sont appliquées par le daemon au rechargement. Aucun setter
`Manager1` n'est appelé ; la GUI ne fait aucune écriture privilégiée.

### 22.1 Champs

| Champ `Settings` | Contrôle | Règles |
|---|---|---|
| `chargingType` | Réglages → Charge | liste `chargeTypes` si non vide, sinon Trickle/Fast/Standard/Adaptive/Custom/LongLife/Bypass ; « Ne pas changer » = `null` ; ≤ 64 caractères, `[A-Za-z0-9._:-]`, valeur dans la liste |
| `fnLockStatus` | Réglages → Fn-Lock | booléen, uniquement si `fnLock.supported && writable && enabledKnown` |
| `keyboardStates` | Outils → Éclairage | 0–64 zones ; luminosité 0–`maxBrightness` (≤255) ; couleurs -1 (non supporté) ou 0–`max*` (≤255) ; mode `static`/`breathing` si exposé |
| `lightbarStates` | Outils → Éclairage | luminosité 0–`maxBrightness`, couleurs 0–`max*` ; `null` explicite efface, absence préserve |

### 22.2 Comportement

- **Pas de changement** : champ absent/`null` (sauf `lightbarStates` où
  `null` explicite efface). Aucun écrasement silencieux.
- **Charge** : le sélecteur affiche le type persisté ; le type matériel
  courant reste visible en lecture. Le bouton Appliquer partage le flux
  conflit/relecture de la carte Charge.
- **Fn-Lock** : l'interrupteur reflète l'état matériel ; après enregistrement,
  la relecture du worker fait foi. Un refus rétablit l'état réel.
- **Éclairage** : le clavier blanc n'a que luminosité/on-off ; le RGB expose
  R/G/B, luminosité et mode. Une couleur uniforme est appliquée aux zones
  (limite assumée). La lightbar suit le même modèle. On/off = luminosité 0.
- **Erreurs** : admin absent/busy/conflit/refus → contrôles désactivés ou
  bandeau traduit, brouillon conservé (éclairage), état réel rétabli
  (Fn-Lock). Matériel absent → carte masquée.
- **Limites** : les modes/couleurs dépendent du firmware ; un type de charge
  inconnu du backend est refusé par le daemon. La synchronisation
  lid/UPower de l'éclairage reste hors périmètre.

---

## 23. Fermeture des écarts d'audit (2026-11-07)

- **Aquaris** : entrée de navigation retirée tant qu'aucun backend BLE
  n'existe ; aucune promesse « bientôt », aucun toast.
- **Fn-Lock tray** : `MenuItem` cochable si `supported && writable &&
  enabledKnown` ; sauvegarde `fnLockStatus` via le même DTO `SettingsEdit`
  que Réglages ; relecture/refus resynchronisés, jamais de `Manager1`.
- **Icônes AC/BAT** : `LiveData` lit la matrice persistée (`SettingsData`
  ← `GetSettingsJSON.stateMatrix`) ; `assignedAc`/`assignedBat` par profil
  indiquent au moins une cellule AC/BAT, et les propriétés singulières suivent
  le PPD courant. Aucune assignation déduite du profil actif.
- **cTGP** : `nvidia.maxCtgpOffset` est exposé par `HardwareView` quand le
  snapshot le fournit ; repli 200 W documenté sinon.
- **TDP** : monotonie `PL1 ≤ PL2 ≤ PL4` validée en QML et dans le helper Rust ;
  message sûr `settings.tdpMonotonicError`.
- **DisplayInfoView** : dimensions `1..=16384`, taux `1..=1000`, modes
  invalides filtrés en Rust (`sanitize`) avant publication ; QML reste
  défensif.
- **Ventilateur GPU** : la jauge dGPU utilise le fan `target`/`label` GPU de
  `GetFanDataJSON` ; masquée si absent, aucun `available:false` inventé.

---

## 24. Éditeur de profil — barre d'action flottante

- Overlay `Rectangle` ancré en bas de la page, **hors** du `PageScroll` ;
  toujours visible quand `page.editable` (masqué pour un preset ou admin
  absent).
- Contenu : état (`Aucune modification`, `Modifications non enregistrées`,
  `Corrigez les champs…` en rouge), spinner pendant `AdminState.saving`,
  `Annuler` (relit le profil) et `Enregistrer`.
- `Enregistrer` est activé uniquement si `dirty && valid && AdminState.ready
  && !conflict && !saving`. Le bouton du haut devient secondaire (`ghost`).
- Un espaceur de hauteur `floatingBarHeight + s16` termine le contenu : le
  dernier champ reste accessible au clavier et à la souris.
- Accessibilité : boutons atteignables au Tab ; aucun `Keys.onReturnPressed`
  global, donc Entrée dans un champ n'enregistre pas par accident. Le libellé
  d'état est aussi exposé comme texte, pas seulement par la couleur.
- Tests : `tst_profile_editor` vérifie visibilité, activation, save, annulation,
  espaceur et masquage pour un preset.

## 25. Tray — panneau compact et menu de secours

- Clic gauche (`SystemTrayIcon.Trigger`) → `TrayPanel`, fenêtre `Qt.Popup`
  sans décoration, positionnée au coin bas-droit de l'écran principal
  (`Screen.width/height`). Clic droit → menu natif existant (secours).
- Panneau : profil actif + description, badges PPD et AC/BAT, liste radio des
  profils (activation temporaire, sans ouvrir la fenêtre), Fn-Lock si
  `writable && enabledKnown`, power blocker, autostart, `Ouvrir la fenêtre`,
  `Quitter`, version discrète.
- Aucune logique métier dupliquée : `LiveData`, `HardwareData`,
  `PowerBlockerData`, `AutostartData` et `AppState` restent les sources.
- Limites SNI : certains compositeurs (Wayland/SNI) ouvrent le menu sur clic
  gauche ou replacent la fenêtre ; le menu reste le repli documenté. Le
  panneau ne dépend pas de la visibilité de la fenêtre principale.
- Tests : `tst_tray` vérifie contenu, profils, open/quit, profil demandé,
  Fn-Lock/blocker/autostart et non-régression du menu.

## 26. Launcher / dock / taskbar — app_id et icône

- `qapplication_shim.cpp` définit
  `QGuiApplication::setDesktopFileName("tuxedo-rust-center")` (app_id Wayland,
  association au `.desktop`) et `QApplication::setWindowIcon` :
  `QIcon::fromTheme("tuxedo-rust-center")`, repli sur
  `/usr/share/icons/hicolor/256x256/apps/tuxedo-rust-center.png`.
- `packaging/desktop/tuxedo-rust-center.desktop` : `Icon=tuxedo-rust-center`,
  `StartupWMClass=tuxedo-rust-center`, `Exec=tuxedo-rust-center`,
  `X-GNOME-UsesNotifications=true`.
- Icônes installées par `scripts/release/install-tree.sh` : `scalable` + PNG
  256/128/64/48/32 (générées depuis le SVG avec `rsvg-convert`).
- Validation : `desktop-file-validate` passe ; vérifier
  `ls /usr/share/icons/hicolor/*/apps/tuxedo-rust-center.*` après installation.
  Sur Wayland, l'app_id effectif dépend du compositeur ; sur X11,
  `StartupWMClass` et l'icône de fenêtre couvrent le dock/taskbar.
