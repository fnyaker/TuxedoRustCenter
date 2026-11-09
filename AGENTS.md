# TuxedoRustCenter — instructions projet

## Mission

Réécrire **TUXEDO Control Center (TCC)** en **Rust**, avec :

- une **GUI Qt 6 native** (aucun composant web) ;
- un **daemon Rust privilégié** ;
- un **agent de session Rust non privilégié** pour les opérations liées à la session graphique (notamment le refresh rate) ;
- une **parité fonctionnelle au minimum** avec l'original ;
- une installation **FHS standard, sans `/opt`** ;
- une bascule automatique de profil pilotée par **power-profiles-daemon** et par l'**état AC/batterie** ;
- le **changement du refresh rate de l'écran interne par profil**, sous **X11 et Wayland**.

Le projet original de référence est dans `tuxedo-control-center-master/`. Il ne doit pas être modifié : c'est la source de vérité fonctionnelle et comportementale.

## Licence

L'original est sous **GPL-3.0**. La réécriture doit rester compatible **GPL-3.0-or-later** et conserver les notices de licence pour toute partie dérivée. Ne pas copier de code sans son en-tête de licence et sans vérifier la compatibilité.

## Documents à lire avant toute implémentation

1. `docs/ARCHITECTURE.md` — analyse de l'existant, architecture Rust cible, contrats, décisions.
2. `docs/FEATURE_PARITY.md` — checklist de parité fonctionnelle.
3. `docs/QUALITY.md` — portes de qualité, revues croisées, preuves attendues.
4. `docs/security/DECISIONS.md` — décisions de sécurité bloquantes.
5. `docs/HANDOFF.md` — état courant, agents en cours et reprise après compaction.
6. Le code original pertinent dans `tuxedo-control-center-master/src/`.

Ne jamais inventer un comportement matériel. Vérifier dans l'original, dans la documentation du noyau, ou dans les sources du driver concerné.

## Contraintes non négociables

1. **Runtime 100 % Rust** pour la GUI et le daemon. Pas de Node.js, Electron, Angular, Python, ni script embarqué. Le C/C++ n'est toléré que comme glue de build pour les bindings Qt, jamais pour la logique métier.
2. **UI 100 % Qt 6 native** : QML/Qt Quick Controls 2 ou Qt Widgets. **Interdiction** de QtWebEngine, QtWebView, HTML/CSS/JS, Electron ou tout composant web.
2bis. **Décisions sécurité bloquantes** : lire `docs/security/DECISIONS.md` et `docs/security/THREAT_MODEL.md`. En résumé : `tccd-admin` séparé pour LUKS/Prime/Tomte/WebFAI ; état mutable dans `/var/lib/tuxedo-rust-center`, `/etc` en lecture seule pour le daemon ; secrets LUKS uniquement par fd + `zeroize` ; jamais de `pkexec /bin/sh -c` ni de téléchargement exécuté en root ; `ProtectKernelTunables=no` ; overrides dev derrière une feature non release.
3. **Aucun fichier installé dans `/opt`.** Utiliser les répertoires standard Linux.
4. **Design moderne** avec le rouge TUXEDO `#E30016` en accent (jauges `#E22131`), utilisé avec parcimonie. L'original est clair avec des accents rouges ; le mode sombre doit aussi exister.
5. **Parité fonctionnelle au minimum** : tout ce que fait l'original doit avoir un équivalent, sauf décision explicite documentée.
6. **Moindre privilège** : GUI non privilégiée, daemon root minimal, agent de session utilisateur non privilégié, pas de setuid, pas de `pkexec /bin/sh -c`.
7. **Refresh rate de l'écran interne par profil** : X11 via XRandR natif ; Wayland via le compositeur (Hyprland, GNOME, KDE, wlroots/Sway). Ne jamais toucher aux écrans externes.
8. **Bascule automatique** : écouter `net.hadess.PowerProfiles` et l'état AC/batterie. Les deux signaux forment l'état. Debounce, fallback si PPD absent, override manuel temporaire, pas de conflit avec PPD.
9. **Sécurité** : validation stricte de toutes les entrées, chemins et valeurs ; écritures sysfs contrôlées ; pas de commande shell interpolée ; fichiers de config atomiques et permissions correctes.
10. **Qualité** : `cargo fmt`, `cargo clippy -- -D warnings`, tests unitaires et d'intégration, `cargo deny`/`cargo audit`.

## Chemins cibles

À confirmer et faire évoluer par l'agent plateforme, mais la base est :

| Élément | Chemin standard |
|---|---|
| GUI | `/usr/bin/tuxedo-rust-center` |
| Daemon | `/usr/libexec/tuxedo-rust-center/tccd` |
| Agent de session | `/usr/libexec/tuxedo-rust-center/tccd-user` |
| Unit systemd système | `/usr/lib/systemd/system/tccd.service` |
| Unit systemd utilisateur | `/usr/lib/systemd/user/tccd-user.service` |
| Configuration admin (lecture seule daemon) | `/etc/tuxedo-rust-center/` |
| État mutable (profils/settings) | `/var/lib/tuxedo-rust-center/` |
| Runtime | `/run/tuxedo-rust-center/` |
| Ressources QML/assets | `/usr/share/tuxedo-rust-center/` |
| Desktop entry | `/usr/share/applications/` |
| Icônes | `/usr/share/icons/hicolor/` |
| AppStream | `/usr/share/metainfo/` |
| Politique D-Bus | `/usr/share/dbus-1/system.d/` |
| Actions polkit | `/usr/share/polkit-1/actions/` |
| Logs | journald (pas de fichier `/var/log`) |

Migration souhaitée depuis `/etc/tcc/{settings,profiles,fantables,webcam}` vers le nouveau format, avec sauvegarde et validation.

## Architecture cible (résumé)

- Workspace Cargo : `tcc-core`, `tcc-ipc`, `tcc-hw`, `tcc-daemon`, `tcc-user`, `tcc-gui`, éventuellement `tcc-cli`.
- `tcc-core` : modèles, profils, settings, moteur de bascule, courbes de ventilateurs, migration, logique pure testable.
- `tcc-hw` : sysfs, hwmon, ioctl `tuxedo-io`, DMI, DRM, backlight, charging, lighting, webcam V4L2, backends d'affichage.
- `tcc-daemon` : service D-Bus système, workers, orchestration, persistance, application des profils.
- `tcc-user` : agent de session utilisateur pour le refresh rate et les APIs du compositeur.
- `tcc-gui` : Qt 6 natif via cxx-qt (ou équivalent Rust/Qt), modèles Rust, aucune logique privilégiée.
- D-Bus système : daemon ↔ GUI ; D-Bus/session : agent ↔ compositeur. Le daemon ne manipule pas directement la session graphique.

## Bascule automatique — contrat minimal

Entrées :

- `ppd`: `power-saver`, `balanced`, `performance`, absent/inconnu ;
- `power`: `AC`, `Battery`, inconnu ;
- override manuel temporaire éventuel.

Résolution :

1. Construire une clé `(ppd, power)`.
2. Chercher le profil dans une matrice configurable.
3. Fallback : matrice absente → `stateMap.power_ac` / `stateMap.power_bat` (compatibilité original), puis profil par défaut.
4. Debounce (≈ 500–1000 ms) pour éviter les oscillations.
5. Un changement de `ppd` ou de `power` invalide l'override manuel temporaire.
6. Si PPD est absent, utiliser uniquement AC/batterie.
7. Ne pas se battre avec PPD : par défaut, PPD reste propriétaire de `platform_profile` et de l'EPP ; TCC possède les limites CPU, les ventilateurs, l'affichage, la webcam et la charge. Toute dérogation doit être explicite, documentée et loguée.

## Refresh rate — contrat minimal

- Le profil contient un taux optionnel (`Option<f32>`, `use_refresh_rate`), avec migration de `refreshRate: -1` de l'original.
- Cible exclusive : le connecteur interne (`eDP-*`, `LVDS-*`, `DSI-*`), jamais les écrans externes.
- X11 : XRandR natif via `x11rb`.
- Wayland : agent de session, backend selon le compositeur :
  - Hyprland : socket IPC natif ;
  - GNOME : `org.gnome.Mutter.DisplayConfig` ;
  - KDE : D-Bus KScreen / `kscreen-doctor` en fallback documenté ;
  - Sway/wlroots : `zwlr_output_manager_v1` ou IPC Sway.
- Ne changer le mode que si le taux diffère réellement. Tolérance raisonnable (≈ 0,5 Hz), sinon échec propre et log.
- Ne pas changer la résolution sauf si le profil le demande explicitement.
- Si aucun backend n'est disponible, ne rien casser : exposer l'état « non supporté » dans l'UI.

## Règles d'ingénierie

- Lire le code existant avant de modifier. Ne pas supprimer ou écraser un travail utilisateur sans enquête.
- Préférer les APIs natives (D-Bus, sysfs, ioctl, XRandR, Wayland) aux commandes externes.
- Si une commande externe est indispensable (`tuxedo-tomte`, `prime-select`, `systeminfos`, `apt`), l'encapsuler dans une allowlist, sans shell, avec arguments séparés et validation.
- Aucun `unwrap()`/`expect()` dans les chemins de production du daemon. Erreurs typées et contexte.
- `unsafe` uniquement dans des wrappers isolés, avec commentaire de sûreté et tests.
- Pas de blocage dans les contextes async. Canaux bornés, annulation, arrêt propre.
- Logs structurés via `tracing`, jamais de secret ni de contenu de fichier sensible.
- Tests avec racines sysfs factices et D-Bus mocké ; la CI ne doit pas dépendre d'un laptop TUXEDO.
- Performance : event-driven par défaut. Le polling capteurs ne doit être actif que lorsqu'un client le demande. Budgets indicatifs à mesurer : daemon inactif < 1 % CPU et < 30 Mo RSS ; GUI < 200 Mo RSS et démarrage < 1,5 s.
- i18n dès le départ (au moins FR/EN/DE), formats °C/°F, accessibilité clavier et contraste.
- Documenter chaque chemin sysfs, ioctl, méthode D-Bus et décision matérielle.

## Répartition des agents

| Agent | Domaine | Ne doit pas |
|---|---|---|
| `linux-platform` | Daemon, service D-Bus, orchestration, PPD/UPower, agent de session, intégration systemd | Choisir le design UI, dupliquer la matrice matérielle |
| `tuxedo-hardware` | `tcc-hw`, `tcc-daemon/src/hardware/**`, ioctl, hwmon, ventilateurs, TDP/ODM, NVIDIA, PRIME, charge, lighting, webcam V4L2, quirks | Modifier `main.rs`/`dbus.rs`/`runtime.rs`/`config.rs`/`watchers.rs` (linux-platform), ni la GUI |
| `rust-core` | Workspace, `tcc-core`, `tcc-ipc`, performance, fiabilité, CI de base | Décider du comportement matériel ou du design UI |
| `qt-ui` | `tcc-gui`, design system, écrans, accessibilité, i18n, pont QML | Écrire du code privilégié ou de la logique driver |
| `security` | Menaces, privilèges, validation, D-Bus/polkit, hardening, audit | Modifier le code hors `docs/security/**` |
| `qa-integration` | Tests d'intégration, fixtures fake sysfs/D-Bus, tests QML, fuzzing, couverture, CI de test | Modifier le code de production |
| `packaging-release` | Packaging deb/rpm/Arch, FHS, systemd/D-Bus/polkit/udev, migration TCC, CI release | Modifier le code runtime ou affaiblir les décisions de sécurité |

### Protocole de coordination

- Un fichier n'a qu'un propriétaire à la fois. En cas de besoin croisé, créer un module séparé ou demander l'arbitrage au agent principal.
- `tuxedo-hardware` fournit les APIs matérielles ; `linux-platform` les branche dans le daemon et le service D-Bus.
- `qa-integration` possède les tests d'intégration et les fixtures ; les propriétaires ajoutent leurs tests unitaires dans leurs modules.
- `packaging-release` possède le packaging et les manifests ; les autres agents proposent les changements de chemins/dépendances.
- `security` est en lecture seule sur le code et produit des findings dans `docs/security/**`.
- Toute nouvelle méthode D-Bus ou tout nouveau chemin sysfs doit être documenté et revu par `security` avant packaging.

En cas de désaccord d'interface, documenter la proposition et demander l'arbitrage avant de dupliquer la logique.

### Parallélisation

Le agent principal doit lancer **en parallèle** toutes les tâches indépendantes qui ne partagent pas de fichier, afin de maximiser l'avancement sans dégrader la qualité.

- **Parallélisable sans risque** : documentation (`docs/qa`, `docs/security`, `docs/ui` distincts), manifests/packaging, modèles `tcc-core`/`tcc-ipc`, fixtures et tests d'intégration, audits, design reviews, tâches sur des crates ou modules disjoints.
- **À sérialiser** : deux writers sur le même fichier/module, changement de contrat D-Bus + policy + GUI, migration de schéma, service privilégié + unité + polkit + packaging, installation/désinstallation réelle, tests sur matériel réel, rebuild du paquet et redémarrage des services.
- **Toujours lancer les sous-agents en arrière-plan (`background=true`)**, y compris les follow-ups : ne jamais bloquer la conversation sur leur exécution. Les résultats sont intégrés à réception des notifications.
- Avant de lancer, vérifier les agents actifs et leurs périmètres ; ne jamais modifier un fichier détenu par un autre agent.
- Si un besoin croisé apparaît, créer un module séparé ou demander l'arbitrage ; ne pas dupliquer la logique.
- Après chaque lot, relancer la validation complète (`fmt`, clippy, tests, QML, packaging) et mettre à jour `docs/STATUS.md`/`docs/HANDOFF.md`.
- Les preuves matérielles/root restent **séquentielles** : une seule opération privilégiée à la fois, avec sauvegarde et rollback.

## Definition of done

Une fonctionnalité est terminée lorsque :

- le comportement est conforme à l'original ou la différence est documentée ;
- les erreurs et cas matériel absent sont gérés ;
- les tests couvrent la logique et les entrées invalides ;
- `cargo fmt`, `clippy -D warnings`, tests et audit passent ;
- les chemins d'installation respectent le FHS ;
- la sécurité et la performance ont été relues ;
- l'UI expose l'état réel (disponible, indisponible, en erreur).
