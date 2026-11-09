# Attributions des ressources — `crates/tcc-gui/assets`

## Police Titillium Web

- Fichiers : `fonts/TitilliumWeb-{Light,Regular,Italic,SemiBold,Bold}.ttf`
- Copyright © 2009–2011 Accademia di Belle Arti di Urbino and students of the
  MA course of Visual design.
- Licence : **SIL Open Font License 1.1** — texte intégral dans
  `fonts/OFL-1.1.txt`.
- Fichiers copiés **sans modification** depuis
  `tuxedo-control-center-master/src/ng-app/assets/fonts/`.
- Aucun nom réservé n'est utilisé pour désigner une version modifiée.

## Icônes et logos

Source : projet **TUXEDO Control Center**
(`tuxedo-control-center-master/src/ng-app/assets/`), © TUXEDO Computers GmbH,
licence **GPL-3.0-or-later** (voir `LICENSE` et `NOTICE.md` à la racine).

- `icons/icon_*.svg` : icônes d'interface. Les variantes monochromes ont été
  **modifiées** pour le rendu Qt : les remplissages `var(--icon-main)` et
  `var(--icon-detail)` ont été remplacés par du blanc avec des opacités
  (1.0 / 0.55) afin de permettre une teinte à l'exécution via `ColorOverlay` (Qt5Compat.GraphicalEffects).
  La géométrie et les métadonnées d'origine sont conservées.
- `icons/icon_profile_*.svg` et `icons/icon_displaylight.svg` (renommée pour
  suivre le préfixe `icon_`) sont des illustrations colorées copiées telles
  quelles.
- `brand/tcc_logo.svg`, `brand/taskbar_icon_free.svg`,
  `brand/tuxedo-logo.svg` : identité visuelle du projet. Les marques et logos
  TUXEDO restent la propriété de TUXEDO Computers GmbH ; ils sont utilisés ici
  pour désigner l'application réécrite et son matériel, pas pour suggérer une
  approbation.

Aucun fichier Angular/TypeScript, aucun composant web et aucune police
d'icônes propriétaire n'est utilisé par cette interface.
