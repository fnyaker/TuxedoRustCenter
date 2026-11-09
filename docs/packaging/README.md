# Packaging et release — TuxedoRustCenter

Guide opérationnel du packaging : arborescence FHS, dépendances, builds
deb/rpm/Arch, reproductibilité, migration depuis TUXEDO Control Center.

Statut : **jalon 1/2**. Les binaires livrés sont `tccd` (daemon), `tccd-user`
(agent de session), `tuxedo-rust-center` (GUI Qt 6) et `tcc-cli` (diagnostic).
`tccd-admin` est **conçu** (D1) ; un binaire existe, mais le service n'est pas validé
fonctionnellement : il n'est **pas installé par défaut**.

---

## 1. Principes

1. **FHS strict** : aucun fichier dans `/opt`, aucun runtime Node.js/Python.
2. **Une seule source de vérité** pour les chemins :
   `scripts/release/install-tree.sh`. Les paquets deb, rpm et Arch l'appellent
   tous ; aucun manifest ne duplique la liste des fichiers.
3. **Moindre privilège** : daemon root minimal, agent et GUI non privilégiés,
   aucun setuid, aucune capability de fichier.
4. **D2** : `/etc/tuxedo-rust-center` est en lecture seule pour le daemon ;
   l'état mutable vit dans `/var/lib/tuxedo-rust-center`.
5. **Reproductibilité** : `Cargo.lock` verrouillé, `SOURCE_DATE_EPOCH`,
   mtimes normalisés, sommes SHA-256.

---

## 2. Arborescence installée

| Contenu | Chemin | Mode |
|---|---|---|
| GUI | `/usr/bin/tuxedo-rust-center` | 0755 |
| CLI de diagnostic | `/usr/bin/tcc-cli` | 0755 |
| Daemon | `/usr/libexec/tuxedo-rust-center/tccd` | 0755 |
| Agent de session | `/usr/libexec/tuxedo-rust-center/tccd-user` | 0755 |
| Admin à la demande | `/usr/libexec/tuxedo-rust-center/tccd-admin` | **non installé** |
| Admin matériel à la demande | `/usr/libexec/tuxedo-rust-center/tccd-admin-hw` | **non installé** |
| Unit daemon | `/usr/lib/systemd/system/tccd.service` | 0644 |
| Unit admin | `/usr/lib/systemd/system/tccd-admin.service` | **non installée** |
| Unit admin matériel | `/usr/lib/systemd/system/tccd-admin-hw.service` | **non installée** |
| Unit veille | `tccd-sleep.service.example` | **référence seule, non installée** |
| Unit agent | `/usr/lib/systemd/user/tccd-user.service` | 0644 |
| Activation agent | `/usr/lib/systemd/user/graphical-session.target.wants/tccd-user.service` | lien relatif |
| Politique D-Bus | `/usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf` | 0644 |
| Activation admin | `/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service` | **non installée** |
| Activation Admin.Hw | `/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service` | **non installée** |
| Polkit | `/usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy` | 0644 |
| Udev | `/usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules` | 0644 |
| tmpfiles | `/usr/lib/tmpfiles.d/tuxedo-rust-center.conf` | 0644 |
| QML | `/usr/share/tuxedo-rust-center/qml/` | 0644 |
| Assets | `/usr/share/tuxedo-rust-center/assets/` | 0644 |
| Bureau | `/usr/share/applications/tuxedo-rust-center.desktop` | 0644 |
| Icône scalable | `/usr/share/icons/hicolor/scalable/apps/tuxedo-rust-center.svg` | 0644 |
| Icônes PNG | `/usr/share/icons/hicolor/{32x32,48x48,64x64,128x128,256x256}/apps/tuxedo-rust-center.png` | 0644 |
| AppStream | `/usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml` | 0644 |
| Licence | `/usr/share/licenses/tuxedo-rust-center/LICENSE` | 0644 |
| Doc | `/usr/share/doc/tuxedo-rust-center/` | 0644 |
| Configuration admin | `/etc/tuxedo-rust-center/README` | 0644 (conffile) |
| État mutable | `/var/lib/tuxedo-rust-center/` | 0755 (contenu 0644 root:root) |
| Runtime | `/run/tuxedo-rust-center/` | créé par tmpfiles/systemd, **pas dans le paquet** |

La règle udev webcam ne fait que poser un tag (`TAG+=`) : aucun `RUN+=`,
aucun processus, aucune ACL. L'ACL `uaccess` des nœuds `/dev/video*` vient des
règles udev système par défaut (logind) ; le groupe `video` reste le repli.
Aucune règle udev ni action polkit supplémentaire n'est nécessaire pour
`User1.SetUvcControl`.

`/etc/tuxedo-rust-center` ne contient qu'une note de configuration : le paquet
n'installe aucun réglage par défaut (les valeurs par défaut sont compilées
dans `tcc-core`). Les surcharges administrateur sont lues en priorité.

### Correspondance binaire Cargo → nom installé

| Crate | Binaire Cargo | Nom installé |
|---|---|---|
| `tcc-daemon` | `target/release/tcc-daemon` | `tccd` |
| `tcc-user` | `target/release/tcc-user` | `tccd-user` |
| `tcc-gui` | `target/release/tcc-gui` | `tuxedo-rust-center` |
| `tcc-cli` | `target/release/tcc-cli` | `tcc-cli` |
| `tcc-admin` | `target/release/tccd-admin` | `tccd-admin` (**non installé par défaut**) |
| `tcc-admin` | `target/release/tccd-admin-hw` | `tccd-admin-hw` (**non installé par défaut**) |

---

## 3. `tccd-admin` : gating explicite (phase 1 prête, non installée)

Le binaire `tccd-admin` existe, mais le service n'est pas validé fonctionnellement
(polkit/`Control1`/idle-timeout à implémenter). Le paquet **doit rester
installable et démarrer** sans lui :

- `install-tree.sh` n'installe ni le binaire, ni `tccd-admin.service`, ni
  l'activation D-Bus `com.tuxedocomputers.tccd.Admin.service` par défaut ;
- `TCC_WITH_ADMIN=1` (ou `--with-admin`) exige un binaire `tccd-admin` ou
  `tcc-admin` et échoue s'il est absent — jamais de demi-installation ;
- l'unité phase 1 est prête et durcie : `PrivateDevices=yes`,
  `ProtectKernelTunables=yes`, `DevicePolicy=closed`,
  `CapabilityBoundingSet=` vide, `RuntimeMaxSec=5min`, `MemoryMax=192M`,
  `TasksMax=16`, `SystemCallFilter=@system-service`, `Restart=no`, pas de
  `[Install]`, pas de `StateDirectory` ;
- la politique D-Bus autorise root à posséder le nom `.Admin`, mais l'unité
  n'est pas installée, donc aucune activation n'aboutit ;
- `Admin1` phase 1 : seuls `SaveProfilesJSON`, `SaveSettingsJSON`,
  `ReloadConfig` + `Introspectable`/`Peer` sont autorisés ; le reste est refusé
  au bus. `Control1.ReloadConfig` est root-only ;
- `validate.sh --tree` vérifie l'invariant : pas de binaire ⇒ pas d'unité ni
  d'activation admin.

Checklist avant d'activer `TCC_WITH_ADMIN=1` dans un paquet :

1. binaire `tccd-admin` construit et testé (D1) ;
2. chaque méthode vérifie polkit avec le sujet `system-bus-name` de l'appelant
   (`com.tuxedocomputers.tccd1.profiles`/`.settings`, sans `_keep`) ;
3. `Control1.ReloadConfig` re-vérifie l'UID 0 et audite ;
4. persistance uniquement via `ConfigStore::system()` (`/var/lib`, atomique) ;
5. `tccd-admin.service` sans `[Install]`, `Restart=no`, `LimitCORE=0`,
   sans `StateDirectory=` (D1 : pas d'état persistant) ;
6. phase 2 (TDP/ODM/cTGP/PRIME) : unité/ensemble séparé, `PrivateDevices=no`,
   `ProtectKernelTunables=no`, `DeviceAllow=/dev/tuxedo_io`, capacités
   mesurées ; phase 3 Tomte/WebFAI ; **LUKS hors périmètre** ;
7. relire `docs/packaging/SECURITY.md` §6.

---

## 3.1 `tccd-admin-hw` : phase 2 TDP/ODM/cTGP (préparée, non installée)

Décision AD-12 option B2 : service **distinct** de `tccd-admin` (phase 1) et
du futur `tccd-admin-prime`. Le binaire `tccd-admin-hw` est livré (0.1.0) et
le packaging est validé, mais le service n'est **pas validé
fonctionnellement** sur bus système réel ni sur matériel ; rien n'est installé
par défaut.

- Gating : `TCC_WITH_ADMIN_HW=1` ou `--with-admin-hw` (défaut `0`),
  **indépendant** de `TCC_WITH_ADMIN` : `--with-admin` seul n'installe aucun
  fichier Admin.Hw.
- `build.sh --with-admin-hw`, `make-rpm.sh` (`rpmbuild --with admin_hw`),
  `make-arch.sh` (`_with_admin_hw=1`), `.deb` via le pkgroot.
- Binaire requis (`tccd-admin-hw` ou `tcc-admin-hw`) : absent ⇒ échec
  explicite, jamais une activation D-Bus morte.
- Unité `tccd-admin-hw.service` : `Type=dbus`,
  `BusName=com.tuxedocomputers.tccd.Admin.Hw`, `Restart=no`,
  `RuntimeMaxSec=5min`, pas de `[Install]`, pas de `StateDirectory`.
  Durcissement : `PrivateDevices=no` + `DevicePolicy=closed` +
  `DeviceAllow=/dev/tuxedo_io rw`, `ProtectKernelTunables=no` compensé par
  `ReadOnlyPaths` (`/sys/kernel`, `/sys/module`, cpufreq, LEDs, backlight,
  power supply, hwmon, thermal, DRM ; classes `/sys/class/*` préfixées `-`
  pour tolérer une classe absente), `/etc` et `/var/lib` en lecture seule ;
  **pas de `/proc/sys`** : `ProcSubset=pid` le masque (un `ReadOnlyPaths`
  dessus provoque `226/NAMESPACE`), **aucun `ReadWritePaths=`**, `CapabilityBoundingSet=` vide,
  `MemoryMax=256M` (AH-01 : marge pour une trame zbus proche de 128 Mio ;
  pas un budget de fonctionnement), `TasksMax=16`,
  `SystemCallFilter=@system-service`,
  `LimitCORE=0`. Score `systemd-analyze security` : **1.9** (cible ≤ 3.0).
- D-Bus : `com.tuxedocomputers.tccd.AdminHw1`, objet
  `/com/tuxedocomputers/tccd/Admin/Hw`, membres autorisés un par un
  (`Ping`, `GetVersion`, `GetContractVersion`, `GetTdpValues`,
  `SetTdpValues`, `SetOdmProfile`, `SetOdmProfileOverride`,
  `SetNvidiaCtgpOffset`) ; deny d'interface pour le reste ; `Properties.*`
  refusé. Polkit : `com.tuxedocomputers.tccd1.power-limits` (`auth_admin`,
  sans `_keep`).
- `validate.sh --tree` : binaire, unité et activation vont ensemble ; sans
  binaire, les deux fichiers d'intégration sont absents. Voir
  `docs/packaging/SECURITY.md` §6.1 et `VALIDATION.md` §2.2.

---

## 4. Prérequis de build

### Debian / Ubuntu 24.04 LTS (`noble`)

Cible officielle du `.deb`. Les noms `t64` et `qml6-module-*` ci-dessous sont
ceux de `noble` ; vérifier à chaque changement de distribution cible.

```sh
sudo apt-get install -y build-essential pkg-config cargo rustc \
    qt6-base-dev qt6-declarative-dev qt6-tools-dev qt6-l10n-tools \
    qt6-svg-dev libgl1-mesa-dev \
    dpkg-dev fakeroot lintian desktop-file-utils appstream libxml2-utils
```

`qmlimportscanner` provient de `qt6-declarative-dev-tools` sur `noble` (et non
de `qt6-declarative-dev`) ; la CI l'installe pour inspecter l'arborescence
installée.

### Fedora

Cible du `.rpm`. Image CI : `fedora:latest` (variable `RPM_IMAGE`).

```sh
sudo dnf install -y cargo rust gcc gcc-c++ make \
    qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtsvg-devel \
    rpm-build rpmlint systemd-rpm-macros desktop-file-utils appstream libxml2
```

### Arch Linux

```sh
sudo pacman -S --needed base-devel rust qt6-base qt6-declarative qt6-svg \
    namcap desktop-file-utils appstream libxml2
```

---

## 5. Build et paquets

```sh
# Build complet : Cargo verrouillé + arborescence FHS + validation + .deb
scripts/release/build.sh

# Variantes
scripts/release/build.sh --source            # + archive source reproductible
scripts/release/build.sh --rpm               # + RPM (rpmbuild requis)
scripts/release/build.sh --arch              # + paquet Arch (makepkg, non root)
scripts/release/build.sh --no-deb --source   # archive source seulement
scripts/release/build.sh --with-admin        # exige tccd-admin/tcc-admin
scripts/release/build.sh --with-admin-hw     # exige tccd-admin-hw
```

Les services admin ne sont pas validés fonctionnellement sur système réel :
**les paquets par défaut ne les installent pas**. Variante phase 1 :

```sh
scripts/release/build.sh --with-admin          # .deb (pkgroot avec admin)
scripts/release/build.sh --with-admin --rpm    # rpmbuild --with admin
scripts/release/build.sh --with-admin --arch   # PKGBUILD _with_admin=1
```

Variante phase 2 (TDP/ODM/cTGP), **indépendante** de la phase 1 :

```sh
scripts/release/build.sh --with-admin-hw          # .deb (pkgroot avec Admin.Hw)
scripts/release/build.sh --with-admin-hw --rpm    # rpmbuild --with admin_hw
scripts/release/build.sh --with-admin-hw --arch   # PKGBUILD _with_admin_hw=1
```

Équivalents manuels :

```sh
rpmbuild --with admin -bb distro/rpm/tuxedo-rust-center.spec
rpmbuild --with admin_hw -bb distro/rpm/tuxedo-rust-center.spec
# Arch : _with_admin=1 / _with_admin_hw=1 dans distro/arch/PKGBUILD
```

`install-tree.sh` refuse `TCC_WITH_ADMIN=1` sans binaire `tccd-admin` ou
`tcc-admin` ; le nom installé reste `tccd-admin`.

Sorties :

```
build/pkgroot/                    arborescence installée (inspection)
build/packages/*.deb              paquets
build/packages/SHA256SUMS         sommes de contrôle
build/tuxedo-rust-center-<v>.tar.gz  archive source (--source)
```

Scripts individuels :

| Script | Rôle |
|---|---|
| `scripts/release/lib.sh` | version, `SOURCE_DATE_EPOCH`, helpers |
| `scripts/release/install-tree.sh` | arborescence FHS (source de vérité) |
| `scripts/release/validate.sh` | contrôles statiques + arborescence |
| `scripts/release/make-deb.sh` | `.deb` via `dpkg-deb` (sans `dh`, sans `cargo-deb`) |
| `scripts/release/make-rpm.sh` | RPM via `rpmbuild` |
| `scripts/release/make-arch.sh` | paquet Arch via `makepkg` |
| `scripts/release/make-source-tarball.sh` | archive source reproductible |

Le build Cargo se fait **sans** la feature `dev-overrides` (D7) :

```sh
cargo build --release --locked
```

`install-tree.sh` refuse un binaire absent et n'ajoute jamais de feature.

### 5.1 CI de release (GitHub Actions)

`.github/workflows/release.yml` construit et publie les paquets à chaque tag
`v<version>`. Un `workflow_dispatch` rejoue les builds sans publier (le job
`release` exige un tag).

| Cible | Runner / image | Artefact |
|---|---|---|
| **Ubuntu 24.04 LTS** (`noble`) | runner `ubuntu-24.04`, installation dans le conteneur `ubuntu:24.04` | `tuxedo-rust-center_<v>-1_amd64.deb` |
| **Fedora** | conteneur `fedora:latest` (`RPM_IMAGE`) | `tuxedo-rust-center-<v>-1.fc<NN>.x86_64.rpm` |
| **Arch Linux** | conteneur `archlinux:latest` (`ARCH_IMAGE`), `makepkg` non-root | `tuxedo-rust-center-<v>-1-x86_64.pkg.tar.zst` |

- `version-check` échoue si le tag n'est pas exactement `v<version>` avec
  `[workspace.package].version` de `Cargo.toml` ; les builds en dépendent.
- Les images Fedora et Arch sont paramétrables sans modifier les étapes :
  définir la variable de dépôt `RPM_IMAGE` / `ARCH_IMAGE` (Settings → Actions
  → Variables). `container:` n'accepte pas le contexte `env` ; les étapes
  reçoivent malgré tout `RPM_IMAGE`/`ARCH_IMAGE` pour les diagnostics.
- Le job Fedora vérifie la présence de `rpmbuild`, `rpmspec`, `rpmlint` et
  des paquets `qt6-*` sur l'image visée avant de construire ; le job Arch
  vérifie `makepkg`, `namcap` et `qt6-*`. Une image sans ces paquets échoue
  explicitement.
- Le `.deb` est produit sur Ubuntu 24.04 et son changelog est normalisé sur
  `noble` (`TCC_DEB_DISTRIBUTION`, défaut `noble`). Lintian, installation,
  `systemd-analyze verify/security`, `desktop-file-validate`,
  `appstreamcli validate`, `qmlimportscanner`, smoke test GUI offscreen et
  purge sont exécutés dans `ubuntu:24.04`.
- `SOURCE_DATE_EPOCH` est dérivé du commit (`git log -1 --format=%ct`) dans
  chaque job, avant le build.
- La release ne publie que sur tag et attache le `.deb`, le `.rpm`, le
  `.pkg.tar.zst`, l'archive source et `SHA256SUMS` (plus `SHA256SUMS.asc` si
  les secrets `GPG_PRIVATE_KEY`/`GPG_KEY_ID` sont définis). Aucun secret n'est
  requis sinon.
- Aucun `set -x`, aucune commande shell construite par interpolation, actions
  officielles uniquement, `permissions: contents: read` par défaut et
  `contents: write` réservé au job `release`.

Procédure de tag :

```sh
# 1. Aligner les quatre versions (validate.sh vérifie l'égalité) :
#    Cargo.toml [workspace.package].version, spec RPM, PKGBUILD, changelog.
# 2. Taguer et pousser.
git tag -a v0.1.0 -m "TuxedoRustCenter 0.1.0"
git push origin v0.1.0
# 3. La CI construit les trois paquets, puis crée la release GitHub.
```

Un tag déjà publié ne peut pas être republié tel quel (`gh release create
--verify-tag` échoue si la release existe) : publier une nouvelle version
plutôt que réécrire un tag.

### 5.2 Cible Qt 6.4 (Ubuntu 24.04) et teinte des icônes

Ubuntu 24.04 LTS embarque **Qt 6.4.2** ; `QtQuick.Effects` (`MultiEffect`)
n'existe qu'à partir de **Qt 6.5**. `crates/tcc-gui/qml/components/AppIcon.qml`
teinte donc les icônes avec `ColorOverlay` de `Qt5Compat.GraphicalEffects`,
disponible dans Qt 6.4 et fourni par les trois cibles
(`qml6-module-qt5compat-graphicaleffects` sur Debian/Ubuntu,
`qt6-qt5compat` sur Fedora, `qt6-5compat` sur Arch).

Conséquences maintenues :

- `distro/deb/control.in` ne dépend plus de `qml6-module-qtquick-effects`
  (inexistant sur `noble`) mais de `qml6-module-qt5compat-graphicaleffects` ;
  le `.deb` est installable sur un 24.04 stock, et le job `deb-install`
  vérifie l'installation réelle.
- `qt6-svg-plugins` n'existe pas sur `noble` : le plugin SVG (`libqsvg.so`)
  est fourni par `libqt6svg6`. La dépendance est donc
  `qt6-svg-plugins | libqt6svg6`, satisfaite sur Debian 13 **et** Ubuntu 24.04.

**Ne pas réintroduire `QtQuick.Effects`/`MultiEffect`** sans relever la cible
`.deb` vers Qt ≥ 6.5 (Debian 13, Ubuntu ≥ 25.04) : le paquet deviendrait
ininstallable sur Ubuntu 24.04 et la CI de release échouerait volontairement.

Autres limites Qt 6.4 appliquées au QML (vérifiées en chargeant chaque
composant avec Qt 6.4.2 dans `ubuntu:24.04`) :

- préférences persistées via `Qt.labs.settings`, pas `QtCore.Settings`
  (`Settings` n'existe qu'à partir de Qt 6.5) ;
- pas de `focusPolicy` sur les items (`Item.focusPolicy` date de Qt 6.7) :
  les contrôles utilisent la politique de focus par défaut ;
- `StandardPaths` (QtCore) est disponible mais reste utilisé pour le seul
  chemin par défaut des dialogues d'import/export ;
- éviter les identifiants réservés en JS (`boolean`, …) : le moteur QML de
  Qt 6.4 les refuse à la compilation.

---

## 6. Dépendances

### Debian (`distro/deb/control.in`)

- `Depends` : bibliothèques Qt 6 réellement liées (`libQt6Core`, `Gui`,
  `Widgets` — glue `QApplication` du tray, `Qml`, `Quick`, `QuickControls2`,
  `QuickTemplates2`, `Svg`), plugins de plateforme
  (`qt6-qpa-plugins`), plugin image SVG (`qt6-svg-plugins | libqt6svg6` :
  séparé sur Debian 13, fourni par `libqt6svg6` sur Ubuntu 24.04) et modules
  QML importés. Les noms Debian sont préfixés
  `qml6-module-` : `qtcore`, `qt-labs-platform` (`Qt.labs.platform`, tray),
  `qtquick`, `qtquick-controls`, `qtquick-dialogs` (`FileDialog`),
  `qt5compat-graphicaleffects` (`ColorOverlay`), `qtquick-layouts`,
  `qtquick-shapes`, `qtquick-window`,
  `qtqml-models`, `qtqml-workerscript`.
- `Recommends` : `tuxedo-drivers | tuxedo-keyboard`, `power-profiles-daemon`,
  `upower`, `polkitd`, `hicolor-icon-theme`.
- `Suggests` : `tuxedo-tomte`, `tuxedo-systeminfos`, `prime-select`,
  `switcheroo-control`, `nvidia-driver`, `kscreen`.
- `Conflicts`/`Replaces` : `tuxedo-control-center`, `tuxedofancontrol`.
- `Provides` : `tuxedo-control-center`.

Les noms `qml6-module-*` et `libqt6*6t64` sont ceux de Debian 13 / Ubuntu
24.04. Les alternatives `libqt6core6t64 | libqt6core6` couvrent la transition
`t64`. Le paquet de release cible **Ubuntu 24.04 LTS (`noble`)** : le
changelog embarqué est normalisé sur cette distribution
(`TCC_DEB_DISTRIBUTION`, défaut `noble`). Vérifier ces noms à chaque nouvelle
distribution cible.

### RPM (`distro/rpm/tuxedo-rust-center.spec`)

- `Requires` : `qt6-qtbase`, `qt6-qtbase-gui` (Qt6Widgets/QApplication),
  `qt6-qtdeclarative`, `qt6-qt5compat` (`Qt5Compat.GraphicalEffects`,
  `ColorOverlay`), `qt6-qtsvg`, `hicolor-icon-theme`, `systemd`.
  `qt6-qtdeclarative` fournit `Qt.labs.platform`, `QtQuick.Dialogs` et
  `QtQml.Models` (vérifié sur Fedora).
- `Recommends` : `tuxedo-drivers`, `power-profiles-daemon`, `upower`, `polkit`.
- `Suggests` : `tuxedo-tomte`, `tuxedo-systeminfos`, `kscreen`,
  `switcheroo-control`.
- `Conflicts`/`Obsoletes`/`Provides` : `tuxedo-control-center`,
  `tuxedofancontrol`.

### Arch (`distro/arch/PKGBUILD`)

- `depends` : `glibc`, `gcc-libs`, `qt6-base` (Qt6Widgets/QApplication),
  `qt6-declarative`, `qt6-5compat` (`Qt5Compat.GraphicalEffects`,
  `ColorOverlay`), `qt6-svg`, `hicolor-icon-theme`, `dbus`, `systemd`.
  `qt6-declarative` fournit `Qt.labs.platform`, `QtQuick.Dialogs` et
  `QtQml.Models` (vérifié sur Arch).
- `optdepends` : drivers TUXEDO, PPD, UPower, polkit, tomte, kscreen,
  `nvidia-utils`, `switcheroo-control`.

Aucune dépendance dure inutile : sans `tuxedo-drivers`, le daemon démarre et
expose les fonctions CPU/rétroéclairage/affichage disponibles ; les fonctions
ventilateurs/TDP/charge sont marquées « non supportées ».

---

## 7. Reproductibilité et versioning

- Version unique : `[workspace.package].version` dans `Cargo.toml` ;
  `validate.sh` vérifie la cohérence avec le spec RPM, le PKGBUILD et le
  changelog.
- `SOURCE_DATE_EPOCH` : fourni par l'environnement, sinon dernier commit git,
  sinon constante `1793923200` (2026-11-06). `install-tree.sh` ramène tous les
  mtimes à cette valeur.
- Archive source : `tar --sort=name --mtime=@epoch --owner=0 --group=0`,
  `gzip -9n`.
- `.deb` : `dpkg-deb --root-owner-group -Zxz -z9`, `md5sums` triés,
  `DEBIAN/*` datés avant archivage.
- RPM : `use_source_date_epoch_as_buildtime` et
  `clamp_mtime_to_source_date_epoch`.
- Arch : archive source datée, `makepkg` reproductible ; le PKGBUILD du dépôt
  utilise `sha256sums=('SKIP')`, remplacé par la somme réelle dans
  `make-arch.sh`.
- `build/packages/SHA256SUMS` ; signature détachée si `TCC_GPG_KEY` est
  défini :

```sh
TCC_GPG_KEY=ABCD1234 scripts/release/build.sh
gpg --verify build/packages/SHA256SUMS.asc build/packages/SHA256SUMS
```

---

## 8. Validation

```sh
# Fichiers d'intégration du dépôt
scripts/release/validate.sh --repo

# Arborescence installée (modes, setuid, /opt, liens, services admin)
scripts/release/validate.sh --tree build/pkgroot

# Politique D-Bus : test fonctionnel allow/deny (root + client non privilégié)
sudo scripts/release/dbus-policy-test.sh packaging/dbus/com.tuxedocomputers.tccd.conf

# Outils externes
systemd-analyze verify packaging/systemd/tccd.service \
    packaging/systemd/tccd-user.service packaging/systemd/tccd-admin.service \
    packaging/systemd/tccd-admin-hw.service
systemd-analyze security --offline=true packaging/systemd/tccd.service
systemd-analyze security --offline=true packaging/systemd/tccd-admin-hw.service
desktop-file-validate packaging/desktop/tuxedo-rust-center.desktop
appstreamcli validate --no-net packaging/metainfo/*.metainfo.xml
xmllint --noout packaging/polkit/*.policy packaging/dbus/*.conf
udevadm verify packaging/udev/*.rules

# Politique D-Bus chargée par un bus de test isolé (fail-closed + parse réel)
scripts/release/validate.sh --repo
```

`validate.sh` vérifie notamment : `deny send_destination` avant tout `allow`,
aucun `allow send_destination` sans interface/membre, aucun
`Properties.Set`, aucune annotation polkit `exec.*`, `allow_any`/
`allow_inactive` = `no`, et l'absence des unités non livrées (`tccd-sleep`,
`tccd-admin`, `tccd-admin-hw`). Le test D-Bus couvre 61 cas, dont 20 pour
`Admin.Hw` (allow par membre, deny d'interface, `Properties.*` refusé,
revendication du nom réservée à root). LUKS est explicitement hors périmètre : aucune action, méthode
ou procédure de changement de phrase n'est fournie.

Les linters de paquet (`lintian`, `rpmlint`, `namcap`) et les tests
d'installation/désinstallation en conteneur sont exécutés par
`.github/workflows/release.yml`. Résultats locaux dans
`docs/packaging/VALIDATION.md`.

---

## 9. Ce qui n'est pas installé (assumé)

- `tccd-admin` et son activation D-Bus **par défaut** (service non validé fonctionnellement, D1).
  Le packaging phase 1 est prêt ; l'installation se fait explicitement avec
  `TCC_WITH_ADMIN=1` / `--with-admin` / `rpmbuild --with admin` /
  `_with_admin=1` (Arch).
- `tccd-admin-hw`, son unité et son activation D-Bus **par défaut** (binaire
  encore absent, phase 2). Installation explicite uniquement :
  `TCC_WITH_ADMIN_HW=1` / `--with-admin-hw` / `rpmbuild --with admin_hw` /
  `_with_admin_hw=1` (Arch). `--with-admin` seul ne l'installe pas.
- `tccd-sleep.service` : retiré (REVIEW_JALON1 R-06, `systemctl` dans une
  unité). La veille sera gérée par `PrepareForSleep` dans `tccd` ; le fichier
  `packaging/systemd/tccd-sleep.service.example` documente l'ancien
  mécanisme et n'est **pas** un livrable.
- Page de manuel (la documentation est dans
  `/usr/share/doc/tuxedo-rust-center/`).
- Profil AppArmor/SELinux (amélioration, pas un prérequis de release).
- Autostart de tray séparé : le tray sera intégré à la GUI, pas un second
  `.desktop` (contrairement à l'original).
- Runtime Node.js/Python : aucun.
