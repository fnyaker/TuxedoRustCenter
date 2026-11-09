# Environnement de développement

## Prérequis

- Rust stable récent (edition 2024, MSRV 1.85).
- Cargo.
- Qt 6 : `Qt6Core`, `Qt6Gui`, `Qt6Widgets` (requis par Qt Labs Platform sous
  Linux), `Qt6Quick`, `Qt6QuickControls2`, `Qt6Charts`, `Qt6Multimedia`.
- Module QML optionnel **`Qt.labs.platform`** (tray natif / menus) : la GUI le
  charge de façon tolérante et retombe sur une fenêtre normale s'il manque.
  Dépendance à déclarer par `packaging-release` (Debian/Ubuntu
  `qml6-module-qt-labs-platform`, Arch : fourni par `qt6-declarative`).
- CMake ≥ 3.21 et Ninja (pour l'intégration cxx-qt à venir).
- `pkg-config`.
- Outils optionnels : `cargo-deny`, `cargo-audit`, `cargo-nextest`, `qmllint`.

Arch Linux :

```sh
sudo pacman -S rust cargo qt6-base qt6-declarative qt6-charts qt6-multimedia \
    cmake ninja pkgconf
```

Debian/Ubuntu :

```sh
sudo apt install rustc cargo qt6-base-dev qt6-declarative-dev qt6-charts-dev \
    qt6-multimedia-dev cmake ninja-build pkg-config
```

## Commandes du workspace

```sh
cargo check --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## GUI Qt/QML

La GUI est un binaire Rust Qt 6 (cxx-qt) qui charge `crates/tcc-gui/qml/Main.qml`.

Les overrides d'environnement de la GUI (`TCC_QML_DIR`, `TCC_DBUS_SERVICE`,
`TCC_DBUS_BUS`) sont derrière la feature Cargo `dev-overrides` (D7/R-14),
**absente des paquets release**. Sans elle, le binaire utilise
`/usr/share/tuxedo-rust-center/qml` et `com.tuxedocomputers.tccd` sur le bus
système, quelles que soient les variables d'environnement.

```sh
# contre le daemon système (production/dev avec le daemon installé)
cargo run -p tcc-gui

# contre le daemon de développement sur le bus de session
TCC_DBUS_BUS=session \
TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
cargo run -p tcc-gui --features dev-overrides
```

En dev, `TCC_QML_DIR` permet de pointer vers un autre arbre QML :

```sh
TCC_QML_DIR="$PWD/crates/tcc-gui/qml" cargo run -p tcc-gui --features dev-overrides
```

Le binaire sans la feature cherche `Main.qml` dans
`/usr/share/tuxedo-rust-center/qml` ; en build `debug` uniquement, l'arbre
source sert de repli pour `cargo run` (chemin fixé à la compilation, jamais lu
depuis l'environnement). En release, un paquet incomplet est une erreur
explicite.

Le pont D-Bus utilise un **worker unique** avec une file bornée (8 commandes) :
les appels QML `refresh()`, `setTempProfile()` et `clearTempProfile()`
n'empilent qu'une commande, sans créer de thread. Une file pleine renvoie
immédiatement l'état « occupé » (`busy`).

Le shell QML peut aussi être prévisualisé isolément avec les outils Qt :

```sh
qml crates/tcc-gui/qml/Main.qml
```

Validation syntaxique :

```sh
qmllint crates/tcc-gui/qml/Main.qml
```

En environnement sans écran :

```sh
QT_QPA_PLATFORM=offscreen qml crates/tcc-gui/qml/Main.qml
```

## Tests matériels

Les tests unitaires utilisent des racines sysfs factices. Aucun laptop TUXEDO
n'est requis pour la CI. Les tests réels sont documentés au cas par cas et restent
optionnels.

## Configuration locale

Les binaires liront par défaut :

- `/etc/tuxedo-rust-center/` — configuration administrateur, **lecture seule** pour le daemon ;
- `/var/lib/tuxedo-rust-center/` — état mutable (profils/settings), root:root 0644 ;
- `/run/tuxedo-rust-center/` — runtime.

Pour le développement, les overrides `TCC_SYSFS_ROOT` et `TCC_CONFIG_DIR` sont
derrière la feature Cargo `dev-overrides`, **absente des paquets release**. Ils sont
refusés si le processus tourne en root. Ne jamais écrire dans `/etc` ou `/sys` réels
pendant les tests.

### Stack de développement sur le bus de session

Script tout-en-un :

```sh
./scripts/dev-session.sh
```

Il démarre le daemon en dry-run, l'agent de session et la GUI, puis nettoie à
la fermeture.

Pour tester manuellement chaque brique, sans root, sans toucher au matériel et
sans entrer en conflit avec le TCC original :

```sh
# terminal 1 : daemon en dry-run sur le bus de session
cargo run -p tcc-daemon -- --dev-session --bus-name com.tuxedocomputers.tccd.Dev

# terminal 2 : agent de session
TCC_DBUS_BUS=session \
TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
cargo run -p tcc-user

# terminal 3 : diagnostic
TCC_DBUS_BUS=session \
TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
cargo run -p tcc-cli -- status
```

Le mode `--dev-session` force le dry-run : aucune écriture CPU/backlight. Le
refresh rate n'est appliqué que par `tccd-user` si le profil actif le demande.

## Licence

Le projet est sous GPL-3.0-or-later. Voir `LICENSE` et `NOTICE.md`.
