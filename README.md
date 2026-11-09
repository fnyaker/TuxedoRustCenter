# TuxedoRustCenter

[![CI](https://github.com/fnyaker/TuxedoRustCenter/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/fnyaker/TuxedoRustCenter/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fnyaker/TuxedoRustCenter?sort=semver)](https://github.com/fnyaker/TuxedoRustCenter/releases)
[![Licence : GPL-3.0-or-later](https://img.shields.io/badge/licence-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](rust-toolchain.toml)
[![Qt 6](https://img.shields.io/badge/Qt-6-41CD52.svg)](https://www.qt.io/)

**Réécriture 100 % Rust de [TUXEDO Control Center](https://github.com/tuxedocomputers/tuxedo-control-center).**
Une GUI Qt 6 native, un daemon Rust privilégié et un agent de session non
privilégié : aucun Electron, aucun Angular, aucun composant web, aucun runtime
Node ou Python.

| Dashboard | Profils |
|---|---|
| ![Dashboard](docs/ui/captures/dashboard-light.png) | ![Profils](docs/ui/captures/profiles-dark.png) |

## Pourquoi une réécriture

L'original cumule une GUI Electron/Angular, un daemon Node.js packagé et un
addon C++, installés dans `/opt`. TuxedoRustCenter reprend les mêmes fonctions
matérielles avec une pile entièrement native :

- **démarrage et empreinte réduits** — GUI ~300 ms et 127 Mio en rendu logiciel,
  daemon ~0,3 % CPU et 15 Mio RSS au repos (mesurés, voir `docs/STATUS.md`) ;
- **surface d'attaque plus faible** — trois processus séparés, pas de setuid,
  polkit pour les opérations sensibles, aucune commande shell construite ;
- **installation FHS** — `/usr`, `/etc`, `/var/lib`, `/run` ; aucun `/opt` ;
- **intégration moderne** — power-profiles-daemon, UPower, systemd, Wayland,
  journald ;
- **comportement vérifiable** — logique matérielle testable sur sysfs factice,
  CI sans laptop TUXEDO.

## Fonctionnalités

### Profils et bascule automatique

- Profils TUXEDO (Max Energy Save, Quiet, Office, High Performance) et profils
  personnalisés : création, duplication, import/export validé.
- Bascule automatique sur la matrice **power-profiles-daemon × alimentation**
  (3 × 2), fallback AC/batterie, debounce, override manuel temporaire invalidé
  au changement d'état.
- PPD reste propriétaire de `platform_profile` et de l'EPP ; TCC possède les
  limites CPU, les ventilateurs, l'affichage, la webcam et la charge.
- Application d'un profil complet : CPU, ventilateurs, écran, webcam, charge,
  ODM, TDP, cTGP, Fn-Lock.

### Matériel

- **CPU** : cœurs en ligne, governor, fréquences min/max, EPP, `no_turbo`,
  quirks `intel_pstate` / `amd-pstate-epp` / `acpi-cpufreq`.
- **Ventilateurs** : backends `tuxi`, `pwm` et `tuxedo-io`, courbes CPU/GPU
  éditables au glisser-déposer, min/max/offset, hystérésis, retour en mode auto.
- **Charge** : profil, priorité, seuils start/end, type de charge.
- **Éclairage** : clavier blanc, zones RGB, per-key, lightbar, modes statique et
  respiration.
- **Webcam** : kill switch matériel, contrôles UVC via V4L2.
- **ODM / TDP / NVIDIA** : `tuxedo_platform_profile`, PL1/PL2/PL4 via
  `tuxedo-io`, offset cTGP, état PRIME en lecture.
- **Capteurs** : températures, fréquences, puissances (RAPL, hwmon, NVIDIA),
  ventilateurs, collecte uniquement à la demande.
- **Divers** : Fn-Lock, YCbCr 4:2:0, luminosité, veille/reprise, minuterie
  d'extinction via logind.

### Affichage

- **Refresh rate de l'écran interne par profil**, sur X11 (XRandR natif via
  `x11rb`) et Wayland : Hyprland, Sway, KDE (`kscreen-doctor`), GNOME/Mutter.
- Cible exclusivement `eDP-*`, `LVDS-*` ou `DSI-*` : les écrans externes ne
  sont jamais modifiés.
- Résolution optionnelle par profil, tolérance de 0,5 Hz, échec propre et état
  « non supporté » exposé à l'interface.

### Interface

- Qt 6 natif (QML + cxx-qt) : Dashboard avec jauges et historiques, Profils,
  éditeur de profil, Outils, Réglages, Support, À propos.
- Thèmes clair/sombre/système, accent rouge TUXEDO `#E30016`, police Titillium
  Web embarquée.
- Tray natif (`Qt.labs.platform`) : profil actif, bascule rapide, Fn-Lock,
  inhibition de veille, autostart.
- i18n **FR/EN/DE**, °C/°F, réduction des animations, navigation clavier.
- Mode dégradé propre si le daemon est absent.

### En ligne de commande

`tcc-cli` expose `status`, `profiles`, `settings`, `fans`, `hardware`, `sensors`,
`timer`, `admin`, `ping`, `set-temp`, `clear-temp`.

## Plateformes

| Paquet | Cible | Statut |
|---|---|---|
| `.pkg.tar.zst` | Arch Linux et dérivées (EndeavourOS, CachyOS, Manjaro…) | construit et testé |
| `.deb` | Ubuntu 24.04 LTS | construit et testé |
| `.rpm` | Fedora | construit et testé |

Les paquets sont produits automatiquement par GitHub Actions à chaque tag
`v*` et joints à la release, avec les sommes SHA-256.

Sur un portable TUXEDO, `tuxedo-drivers` (ou `tuxedo-keyboard`) est recommandé
pour les ventilateurs, la charge, l'éclairage et `tuxedo-io`. Sur d'autres
machines, l'application reste utilisable : les fonctions absentes sont
détectées et affichées comme indisponibles, jamais simulées.

## Installation

Télécharger le paquet de la [dernière release](https://github.com/fnyaker/TuxedoRustCenter/releases) :

```sh
# Arch et dérivées
sudo pacman -U tuxedo-rust-center-*.pkg.tar.zst

# Ubuntu 24.04
sudo apt install ./tuxedo-rust-center_*_amd64.deb

# Fedora
sudo dnf install ./tuxedo-rust-center-*.rpm
```

Puis activer les services :

```sh
sudo systemctl enable --now tccd          # daemon privilégié
systemctl --user enable --now tccd-user   # agent de session (refresh rate)
```

Le paquet est en `Conflicts`/`Provides` avec `tuxedo-control-center` : il
remplace l'original, mais la désinstallation de celui-ci et la migration des
profils sont détaillées dans [`docs/packaging/MIGRATION.md`](docs/packaging/MIGRATION.md).

### Compilation depuis les sources

Prérequis : Rust 1.85+, Qt 6 (Core, Gui, Widgets, Quick, QuickControls2),
CMake, Ninja, pkg-config. Détails par distribution dans
[`docs/DEV_SETUP.md`](docs/DEV_SETUP.md).

```sh
cargo build --release --workspace
cargo test --workspace
```

Stack de développement sans root, sur le bus de session :

```sh
./scripts/dev-session.sh
```

Packages locaux (`deb`, `rpm`, Arch) :

```sh
scripts/release/build.sh            # .deb
scripts/release/build.sh --rpm      # + RPM
scripts/release/build.sh --arch     # + paquet Arch
```

## Architecture

```
┌───────────────────────────────────────────────┐
│ tcc-gui — GUI Qt 6 native (utilisateur)       │
└─────────────┬─────────────────────┬───────────┘
              │ D-Bus système       │ D-Bus système (polkit)
┌─────────────▼──────────────┐  ┌───▼─────────────────────────┐
│ tccd — daemon root         │  │ tccd-admin — root à la      │
│ profils, workers, matériel │  │ demande : écritures         │
│                            │  │ persistantes, ODM/TDP/cTGP  │
└─────────────┬──────────────┘  └─────────────────────────────┘
              │ signal de profil actif
┌─────────────▼──────────────┐
│ tccd-user — agent session  │
│ refresh rate, compositeur  │
└────────────────────────────┘
```

| Crate | Rôle |
|---|---|
| `tcc-core` | modèles, profils, settings, moteur de bascule, migration, validation |
| `tcc-ipc` | contrats D-Bus typés et versionnés (`Manager1`, `Admin1`, `Admin.Hw`, `User1`) |
| `tcc-hw` | sysfs, hwmon, ioctl `tuxedo-io`, DMI, DRM, V4L2, backends d'affichage |
| `tcc-daemon` | binaire `tccd` : orchestration, workers, persistance, D-Bus système |
| `tcc-admin` | binaire `tccd-admin` : opérations sensibles, polkit, à la demande |
| `tcc-user` | binaire `tccd-user` : agent de session, refresh rate et résolution |
| `tcc-gui` | binaire Qt 6 (`cxx-qt` + QML) |
| `tcc-cli` | outil de diagnostic non privilégié |
| `tcc-authz` | client polkit partagé, fail-closed |

`tccd-admin` n'est **pas** installé par les paquets par défaut : il n'est
embarqué que si le paquet est construit avec `TCC_WITH_ADMIN=1`, le temps de
valider l'autorisation polkit et la transaction de persistance sur un bus
système réel.

## Sécurité

- Aucun setuid ; GUI et agent de session sans privilège ; daemon root minimal.
- Écritures persistantes et opérations matérielles sensibles via `tccd-admin`
  + polkit (`system-bus-name`, fail-closed) et transactions avec sauvegarde.
- État mutable dans `/var/lib/tuxedo-rust-center`, `/etc` en lecture seule pour
  le daemon ; écritures atomiques, permissions explicites.
- Aucune commande shell interpolée, allowlists d'arguments, validation stricte
  des entrées D-Bus et des profils importés.
- systemd durci (`ProtectSystem=strict`, `PrivateDevices` + `DeviceAllow`,
  `SystemCallFilter`, `CapabilityBoundingSet=`, `LimitCORE=0`,
  `ProtectKernelTunables=no` pour conserver les écritures `/sys`).
- Changement de mot de passe LUKS : **hors périmètre**, non implémenté.

Voir [`SECURITY.md`](SECURITY.md) et [`docs/security/THREAT_MODEL.md`](docs/security/THREAT_MODEL.md).

## État du projet

Jalon 2 : daemon, agent de session et GUI fonctionnels ; profils, bascule
PPD × alimentation, CPU, luminosité, ventilateurs, charge, éclairage,
ODM/TDP/cTGP, capteurs, PRIME/YCbCr en lecture, refresh rate X11 et Wayland.
1 500+ tests Rust et 300+ tests QML passent ; paquets deb/rpm/Arch validés en
conteneurs.

Limites connues, tenues à jour dans [`docs/STATUS.md`](docs/STATUS.md) :

- écriture UVC testée en lecture réelle, pas encore en écriture sur caméra ;
- TDP et cTGP dépendants du firmware et du GPU, non testables sur tous les
  modèles ;
- `Admin.Prime`, Tomte, WebFAI et Aquaris ne sont pas encore implémentés.

## Documentation

- [`AGENTS.md`](AGENTS.md) — règles du projet et contraintes.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — analyse de l'original et architecture cible.
- [`docs/FEATURE_PARITY.md`](docs/FEATURE_PARITY.md) — parité fonctionnelle, item par item.
- [`docs/STATUS.md`](docs/STATUS.md) — état vérifié, mesures et limites connues.
- [`docs/DBUS_API.md`](docs/DBUS_API.md) — API D-Bus et payloads.
- [`docs/user/GUIDE.md`](docs/user/GUIDE.md) — guide utilisateur.
- [`docs/packaging/README.md`](docs/packaging/README.md) — paquets et intégration système.
- [`docs/security/`](docs/security/) — modèle de menace et décisions.

## Contribuer

Les contributions sont bienvenues : bugs, tests matériels, traductions,
support de nouveaux compositeurs Wayland. Merci de lire
[`AGENTS.md`](AGENTS.md) et [`docs/QUALITY.md`](docs/QUALITY.md) (portes de
qualité : `cargo fmt`, `clippy -D warnings`, tests, revue sécurité).

Le code de référence du TCC original n'est pas versionné ici : le récupérer
dans `tuxedo-control-center-master/` (ignoré par git) pour comparer un
comportement.

## Licence

[GPL-3.0-or-later](LICENSE). TuxedoRustCenter est une réécriture indépendante ;
voir [`NOTICE.md`](NOTICE.md) pour l'attribution et la provenance des
ressources.
