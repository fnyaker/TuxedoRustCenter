# CLI, modes d'interface et plugin de barre

## `tcc-cli`

Non privilégié ; tout ce que fait la GUI y est disponible. `--json` donne une
sortie compacte, `completions <shell>` génère l'autocomplétion.

| Domaine | Commandes |
|---|---|
| État | `status`, `ping`, `daemon-version`, `stats [--no-sensors]`, `fans`, `hardware`, `timer`, `sensors [on\|off\|status]`, `display` |
| Profils | `profile list\|show\|active\|use\|clear\|create\|copy\|rename\|delete\|get\|set\|export\|import` |
| Réglages | `settings show\|get\|set` |
| Matériel | `charging show\|profile\|priority\|start\|end\|type`, `keyboard`, `lightbar`, `fn-lock`, `tdp get\|set\|reset`, `odm`, `ctgp`, `uvc` |
| Services | `admin`, `reload`, `omarchy-plugin install\|uninstall` |

`profile set <profil> <chemin.pointé> <valeur>` modifie n'importe quel champ
d'un profil personnalisé (valeur JSON ou texte) ; la clé doit exister, le
profil est validé par `tcc-core` puis écrit via `tccd-admin` (polkit). Les
presets se copient d'abord (`profile copy`). `settings set` fonctionne de la
même façon. Les anciennes commandes (`set-temp`, `clear-temp`, `fan`, `hw`…)
restent acceptées.

## Modes d'interface

`/usr/bin/tuxedo-rust-center` est un lanceur (`tcc-launcher`) :

1. `--ui auto|qt6|gpui` ;
2. variable `TCC_UI` ;
3. fichier `~/.config/tuxedo-rust-center/ui` (un mot) ;
4. sinon `auto` : **gpui sous Hyprland** (si installée), Qt 6 sinon.

Un mode explicite dont le binaire est absent est une erreur ; `auto` retombe
sur l'autre interface. Binaires : `/usr/libexec/tuxedo-rust-center/tcc-gui-qt`
et `tcc-gui-gpui` (optionnelle, `--with-gpui`).

`tcc-gpui` (interface style Omarchy, `gpui-omarchy`) vit dans un **workspace
Cargo séparé** : gpui embarque zbus avec le backend `async-io`, incompatible
avec le backend `tokio` du reste du projet (unification des features), et exige
rustc ≥ 1.92. `tcc-client` expose donc les features `tokio` (défaut) et
`async-io`. Build : `cargo build --release --locked --manifest-path
crates/tcc-gpui/Cargo.toml`.

## Plugin de barre Omarchy

`packaging/omarchy-plugin/tuxedo.tcc-stats` : charge CPU, puissance de la
batterie, profil actif et sélecteur de profil. Le shell Omarchy n'accepte que
du QML : le plugin ne contient que de la présentation et appelle `tcc-cli`
(`stats`, `profile list|use|clear`). Hors panneau ouvert il appelle
`stats --no-sensors` (pas de collecte de capteurs) ; panneau ouvert, la
puissance CPU active la collecte (bail de 10 s).

```
tcc-cli omarchy-plugin install
omarchy plugin enable tuxedo.tcc-stats
```

## Interface gpui

`tcc-gpui` couvre le même périmètre que l'interface Qt, avec les mêmes textes
(`src/strings.rs` est la copie de `Strings.qml` ; un test signale toute
dérive) et les mêmes chemins d'écriture :

| Page | Contenu |
|---|---|
| Dashboard | Profil actif, changement de profil (liste), profil power-profiles-daemon, jauges CPU (charge, température, fréquence, puissance), ventilateurs, iGPU et dGPU (jauges + historiques), batterie |
| Profils | Filtres, recherche, grille/liste, activation, duplication, suppression confirmée, import/export |
| Éditeur | Général, affichage (résolution et taux réellement exposés par l'agent), CPU, webcam, ventilateurs avec **courbe éditable à la souris**, ODM/TDP/cTGP par profil |
| Outils | Minuterie d'extinction, éclairage clavier/barre, contrôles UVC |
| Réglages | Langue, unités, domaines pilotés, Fn-Lock, matrice PPD × alimentation, charge, agent d'affichage, blocage de veille, ODM/TDP/cTGP immédiats (`AdminHw1`), autostart |
| À propos | Versions, appareil, support, licences |

Écritures : profil temporaire via `SetTempProfile`/`ClearTempProfile` ;
profils et réglages via `Admin1.SaveProfilesJSON`/`SaveSettingsJSON` ; ODM, TDP
et cTGP immédiats via `AdminHw1` ; UVC via `User1.SetUvcControl` ; veille via
un inhibiteur `idle` de logind. Les setters `Manager1` (refusés par la
politique D-Bus) ne sont pas appelés. Le profil PPD se change par
`net.hadess.PowerProfiles.ActiveProfile` : PPD reste propriétaire et `tccd`
suit sa matrice.

Courbe de ventilation : glisser un point, double-clic pour en ajouter, clic
droit ou Suppr pour en enlever, flèches pour ajuster le point sélectionné.
Éditable seulement avec le profil de ventilation « Personnalisé ».

Jauges en arc (270°). Puissance CPU : au moins 15 W, au moins le TDP connu,
relevée par paliers de 5 W (« max auto »). L'affichage suit °C/°F. La collecte
des capteurs n'est demandée que lorsque le Dashboard est affiché.

Options : `--page dashboard|profiles|tools|settings|info`, `--edit <profil>`
(identifiant ou nom), `--zoom <30..300>`. Préférences locales (langue, vue en
liste) : `~/.config/tuxedo-rust-center/gpui.json`.

Écarts assumés avec Qt : le thème suit Omarchy (pas de sélecteur
clair/sombre), pas de données de démonstration hors connexion, pas de
changelog statique, description de profil sur une ligne, pas de détection de
conflit à l'enregistrement (la dernière écriture gagne, l'éditeur signale un
profil modifié ailleurs).

## Langues

`tcc-gpui` (en, fr, de), le plugin de barre (en, fr, de ; les autres langues
d'Omarchy retombent sur l'anglais) et la GUI Qt (fr, en, de) suivent la langue
du système (`LC_ALL`, `LC_MESSAGES`, `LANGUAGE`, `LANG`) ; `tcc-gpui` permet
aussi de la choisir dans Réglages. Des tests vérifient
que chaque clé existe dans toutes les langues et que chaque clé utilisée dans
le code existe. Les messages de `tcc-cli` ne sont pas encore traduits.

## Tray Qt

L'icône est le logo TUXEDO monochrome (`taskbar_icon_mono.svg`), sans
pourcentage ni statut. Le panneau du tray liste les profils avec une barre de
recherche (tous les mots saisis doivent apparaître dans le nom).
