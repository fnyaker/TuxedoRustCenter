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

## Tableau de bord gpui : jauges

Jauges en arc (270°) pour la charge CPU, la puissance CPU, la température et
chaque ventilateur. Maximum : 100 % et 100 °C ; pour la puissance, au moins
15 W, au moins le TDP connu, relevé par paliers de 5 W dès qu'une mesure le
dépasse (« max auto »). L'affichage suit le réglage °C/°F.

## Langues

`tcc-gpui` (en, fr, de), le plugin de barre (en, fr, de ; les autres langues
d'Omarchy retombent sur l'anglais) et la GUI Qt (fr, en, de) suivent la langue
du système (`LC_ALL`, `LC_MESSAGES`, `LANGUAGE`, `LANG`). Des tests vérifient
que chaque clé existe dans toutes les langues et que chaque clé utilisée dans
le code existe. Les messages de `tcc-cli` ne sont pas encore traduits.

## Tray Qt

L'icône est le logo TUXEDO monochrome (`taskbar_icon_mono.svg`), sans
pourcentage ni statut. Le panneau du tray liste les profils avec une barre de
recherche (tous les mots saisis doivent apparaître dans le nom).
