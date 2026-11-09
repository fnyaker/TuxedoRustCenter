# TuxedoRustCenter

Réécriture 100 % Rust de **TUXEDO Control Center** : GUI Qt 6 native, daemon Rust, agent de session Rust.

## Statut

Jalon 2 : daemon et GUI Qt 6 fonctionnels — profils, bascule
power-profiles-daemon × alimentation, CPU, luminosité, ventilateurs, charge,
éclairage, ODM/TDP, capteurs, NVIDIA/PRIME/YCbCr, refresh rate X11 et Wayland
(Hyprland, Sway, KDE). L'éditeur de profils, les réglages (charge, matrice PPD), la minuterie
d'extinction et l'import/export enregistrent via `tccd-admin` + polkit ; le
tray Qt et le power save blocker sont disponibles. Les setters matériels
sensibles restent refusés au niveau du bus tant que la policy n'est pas
validée en root. Paquets deb/rpm/Arch
validés en conteneurs.

`tccd-admin` n'est pas installé par défaut (`TCC_WITH_ADMIN=1` requis) et doit
encore être validé sur bus système avec le vrai polkit. Voir
[`docs/STATUS.md`](docs/STATUS.md) et
[`docs/FEATURE_PARITY.md`](docs/FEATURE_PARITY.md) pour les écarts restants.

## Documentation

- [`AGENTS.md`](AGENTS.md) — règles projet et contraintes non négociables.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — analyse de l'original et architecture cible.
- [`docs/FEATURE_PARITY.md`](docs/FEATURE_PARITY.md) — checklist de parité fonctionnelle.
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — jalons de réalisation.
- [`docs/STATUS.md`](docs/STATUS.md) — état vérifié, commandes et limites connues.
- [`docs/QUALITY.md`](docs/QUALITY.md) — portes de qualité et revues croisées.
- [`docs/HANDOFF.md`](docs/HANDOFF.md) — état courant et reprise après compaction.
- [`docs/RELEASE_CHECKLIST.md`](docs/RELEASE_CHECKLIST.md) — checklist de release (qualité, sécurité, matériel, packaging).
- [`docs/validation/MANUAL_VALIDATION.md`](docs/validation/MANUAL_VALIDATION.md) — runbook root/polkit/matériel/tray, remplacement du TCC officiel avec rollback.
- [`docs/user/GUIDE.md`](docs/user/GUIDE.md) — guide utilisateur (profils, refresh rate, admin, diagnostic, limites).
- [`docs/DBUS_API.md`](docs/DBUS_API.md) — API D-Bus du daemon et payloads JSON.
- [`docs/DEV_SETUP.md`](docs/DEV_SETUP.md) — prérequis et commandes de développement.
- [`docs/security/DECISIONS.md`](docs/security/DECISIONS.md) — décisions de sécurité bloquantes.
- [`docs/security/THREAT_MODEL.md`](docs/security/THREAT_MODEL.md) — modèle de menace complet.
- [`docs/security/ADMIN_DESIGN.md`](docs/security/ADMIN_DESIGN.md) — design `tccd-admin`/polkit.
- [`docs/security/REVIEW_ADMIN_PHASE1.md`](docs/security/REVIEW_ADMIN_PHASE1.md) — revue sécurité de la phase 1 admin.
- [`docs/ui/PROFILE_EDITING.md`](docs/ui/PROFILE_EDITING.md) — design de l'édition/sauvegarde des profils.
- [`docs/linux/PLATFORM_DESIGN.md`](docs/linux/PLATFORM_DESIGN.md) — chemins sysfs, ioctl et intégration Linux.
- [`docs/hardware/V4L2_DESIGN.md`](docs/hardware/V4L2_DESIGN.md) — design des contrôles UVC webcam.

## Contraintes clés

- Runtime **100 % Rust** pour la GUI et le daemon.
- UI **Qt 6 native**, aucun composant web.
- Installation **FHS**, aucun fichier dans `/opt`.
- Accent rouge TUXEDO `#E30016`, jauges `#E22131`.
- Bascule automatique sur **power-profiles-daemon** + état **AC/batterie**.
- Refresh rate de l'écran interne par profil, **X11 et Wayland**.
- Licence **GPL-3.0-or-later**.

## Code de référence amont

La réécriture suit le comportement du TUXEDO Control Center original (GPL-3.0).
La copie de référence n'est pas versionnée ici : récupérer
`tuxedocomputers/tuxedo-control-center` dans `tuxedo-control-center-master/`
(chemin ignoré par git) pour disposer des sources de comparaison citées par
`AGENTS.md` et `docs/ARCHITECTURE.md`.

## Construction

```sh
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Lancer la GUI Qt 6 native :

```sh
cargo run -p tcc-gui
```

En développement, les variables `TCC_QML_DIR`, `TCC_DBUS_BUS` et
`TCC_DBUS_SERVICE` ne sont lues qu'avec la feature `dev-overrides` :

```sh
cargo run -p tcc-gui --features dev-overrides
```

Les paquets de release ne compilent pas cette feature et utilisent
exclusivement les chemins installés.

Stack de développement sans root, sur le bus de session :

```sh
./scripts/dev-session.sh
```

ou manuellement :

```sh
cargo run -p tcc-daemon -- --dev-session --bus-name com.tuxedocomputers.tccd.Dev
TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev cargo run -p tcc-gui
```

Voir [`docs/DEV_SETUP.md`](docs/DEV_SETUP.md) pour les dépendances Qt 6.

## Installation (paquets)

Les manifests deb, rpm et Arch sont dans `distro/`. Le contenu installé, les
dépendances, la désinstallation propre et la cohabitation avec le TCC original
sont documentés dans [`docs/packaging/README.md`](docs/packaging/README.md) et
[`docs/packaging/MIGRATION.md`](docs/packaging/MIGRATION.md).

`tccd-admin` n'est installé que si le paquet est construit avec
`TCC_WITH_ADMIN=1` ; les paquets par défaut ne l'activent pas tant que
l'autorisation polkit et la transaction de persistance ne sont pas validées.
