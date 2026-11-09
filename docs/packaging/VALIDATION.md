# Rapport de validation — packaging jalon 1/2

Date : 2026-11-06. Environnement principal : CachyOS/Arch, systemd 262,
Rust stable, Qt 6.12. Les tests Debian sont exécutés dans des conteneurs
`debian:trixie` (Docker), les tests Fedora/Arch dans `fedora:latest` et
`archlinux:latest`.

> Depuis le 2026-11-09, la CI de release (`.github/workflows/release.yml`)
> cible **Ubuntu 24.04 LTS (`noble`)** pour le `.deb` (runner `ubuntu-24.04`,
> installation dans `ubuntu:24.04`), Fedora via `RPM_IMAGE` (défaut
> `fedora:latest`) et Arch via `ARCH_IMAGE` (défaut `archlinux:latest`). Les
> validations `debian:trixie` ci-dessous restent des preuves historiques
> (jalon 1/2) ; les jobs `deb-install` et le changelog `noble` font foi pour
> la publication.

Ce rapport liste ce qui a été **réellement exécuté**, pas seulement ce qui est
prévu par la CI. Les points non testables localement sont explicitement
marqués.

---

## 1. Validation statique du dépôt

```sh
scripts/release/validate.sh --repo
```

Résultat : **351 vérifications réussies, 0 échec**.

```sh
scripts/release/validate.sh --tree build/pkgroot
```

Résultat : **392 vérifications réussies, 0 échec** (arborescence issue du
dernier build Cargo connu bon).

Couvre notamment :

- `systemd-analyze verify` des quatre units livrées (seule erreur tolérée :
  `ExecStart` absent avant installation) ;
- `systemd-analyze security` : `tccd` (avertissement au-delà de 4.0) et
  `tccd-admin-hw` (échec au-delà de 3.0, cible design §4.4) ;
- hardening : `ProtectKernelTunables=no`, `ProtectSystem=strict`,
  `ReadWritePaths` sans `/etc`, `LimitCORE=0`, `UMask=0077`,
  `CapabilityBoundingSet=` vide, `MemoryMax=256M`, `TasksMax=64`,
  `CPUQuota=25%`, `ProtectKernelLogs=yes`, `DeviceAllow=/dev/tuxedo_io rw`,
  `DeviceAllow=char-video4linux r` (V4L2 lecture seule dans `tccd`), absence de
  `EnvironmentFile`/`TCC_*` ;
- `tccd-admin` non installable sans binaire (`TCC_WITH_ADMIN=1`), non installé
  par défaut ; unité phase 1 vérifiée (`PrivateDevices=yes`,
  `ProtectKernelTunables=yes`, capacités vides, `RuntimeMaxSec=5min`) ;
- `tccd-sleep.service` absent ;
- politique D-Bus fail-closed : `deny send_destination` avant tout `allow`,
  aucun `allow send_destination` sans `send_interface`/`send_member`, aucun
  `Properties.Set` ;
- `Manager1` : aucun allow d'interface global (H-10) ; `GetContractVersion`,
  `GetShutdownTimerJSON` (lecture minuterie), 2 lectures/bail sont autorisés ;
  les 8 setters matériels sont **en deny explicite** en attente M1-01/M1-02
  (marqueur vérifié par `validate.sh`) ;
- **aucun `Properties`** (Get/GetAll/Set) sur `tccd` ni `.Admin` (P-12) ; la
  version passe par `Manager1.GetContractVersion` / `Admin1.GetContractVersion` ;
- `Admin1` phase 1 : aucun allow d'interface global ; 3 écritures
  (`SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig`) + lectures
  `Ping`, `GetVersion`, `GetContractVersion` ; **aucun `Properties` sur
  `.Admin`** (A1-11) ; les setters phase 2/3 sont explicitement refusés ;
  `Properties.Set` absent ;
- `Control1.ReloadConfig` : allow dans le bloc root, deny explicite dans le
  bloc par défaut ;
- polkit : IDs `tccd1.*`, `allow_any=no`, `allow_inactive=no`,
  **aucun `auth_admin_keep`**, `profiles` remplace `assign-profile`, aucune
  annotation `exec.path`/`exec.argv1`/`allow_gui` ;
- **LUKS hors périmètre** : `validate.sh` vérifie l'absence de l'action
  polkit LUKS, des méthodes `Luks*` et de toute référence à un outil de
  chiffrement dans le packaging ;
- udev : aucun `RUN+=`, `IMPORT{program}`, `PROGRAM=`, interpréteur ;
- tmpfiles : trois répertoires 0755 root:root, acceptés par
  `systemd-tmpfiles --create --dry-run` ;
- dépendances QML : chaque import direct (`Qt.labs.platform`, `QtQuick.Dialogs`,
  `QtQuick.Effects`, `QtQml.Models`, etc.) est mappé vers un paquet Debian
  déclaré ; RPM/Arch vérifient `qt6-qtdeclarative`/`qt6-declarative` ;
- desktop/AppStream : `desktop-file-validate` et `appstreamcli validate
  --no-net` passent ;
- absence de `/opt`, de setuid/setgid et de runtime Node/Python ;
- cohérence de version Cargo/spec/PKGBUILD/changelog.

### Outils

| Outil | Version / disponibilité |
|---|---|
| `systemd-analyze` | 262 (Arch) |
| `xmllint` | présent |
| `desktop-file-validate` | présent |
| `appstreamcli` | présent |
| `udevadm verify` | présent (1 règle, 0 échec) |
| `systemd-tmpfiles` | présent |
| `dbus-daemon` | présent |
| `lintian` | exécuté dans `debian:trixie` |
| `rpmspec` / `rpmlint` | `rpmspec` OK dans `fedora:latest` ; `rpmlint` en CI |
| `namcap` | en CI `archlinux:latest` |
| `nfpm` | non utilisé (pas de duplication de manifest) |

---

## 2. Durcissement systemd

```sh
systemd-analyze security --offline=true packaging/systemd/tccd.service
systemd-analyze security --offline=true packaging/systemd/tccd-user.service
systemd-analyze security --offline=true packaging/systemd/tccd-admin.service
systemd-analyze security --offline=true packaging/systemd/tccd-admin-hw.service
```

| Unit | Score | Statut |
|---|---|---|
| `tccd.service` | **3.3** | OK |
| `tccd-user.service` | **3.6** | OK (unité utilisateur) |
| `tccd-admin.service` | **1.6** | OK (phase 1, non installé) |
| `tccd-admin-hw.service` | **1.9** | OK (phase 2 préparée, non installée ; cible ≤ 3.0) |
| `tccd-sleep.service` (référence) | 9.2 | **retiré du paquet** (R-06) |

Les exceptions (`ProtectKernelTunables=no`, `PrivateDevices=no`,
`SystemCallFilter` absent) sont justifiées dans
`docs/packaging/SECURITY.md` §2.

### 1.1 Imports QML

`qmlimportscanner` (Debian trixie, `qt6-declarative-dev` + modules QML
installés) sur `crates/tcc-gui/qml` :

```sh
/usr/lib/qt6/libexec/qmlimportscanner -rootPath crates/tcc-gui/qml \
  -importPath /usr/lib/x86_64-linux-gnu/qt6/qml
```

Modules résolus : `QtCore`, `Qt.labs.platform`, `QtQml.Models`, `QtQuick`,
`QtQuick.Controls`, `QtQuick.Dialogs`, `QtQuick.Effects`, `QtQuick.Layouts`,
`QtQuick.Shapes`, `QtQuick.Window`. `Tcc` est compilé dans le binaire.
La CI `deb-install` rejoue ce contrôle sur l'arborescence installée.

Tray : `ldd /usr/bin/tuxedo-rust-center` contient `libQt6Widgets.so.6`
(glue `QApplication`). Test offscreen `--tray` : la GUI reste active
(`rc=124`), journalise « icône tray indisponible (SNI/extension GNOME ?) » et
poursuit sans module QML manquant.

### 2.1 `tccd-admin` phase 1 (préparé, non installé)

- `systemd-analyze verify packaging/systemd/tccd-admin.service` : seul
  diagnostic `ExecStart ... is not executable` dans l'arborescence par défaut (binaire non installé, attendu).
- `systemd-analyze security --offline=true` : **1.6 OK** (cible ≤ 2.5).
- Unité : `PrivateDevices=yes`, `ProtectKernelTunables=yes`,
  `DevicePolicy=closed`, `CapabilityBoundingSet=` vide,
  `SystemCallFilter=@system-service`, `RuntimeMaxSec=5min`,
  `MemoryMax=192M`, `TasksMax=16`, `ReadWritePaths=/var/lib/tuxedo-rust-center`,
  `Restart=no`, pas de `[Install]`, pas de `StateDirectory`.
- Gating : `TCC_WITH_ADMIN=1 scripts/release/install-tree.sh …` sans binaire
  admin → **échec explicite** (code 1), aucune demi-installation.
- Arborescence par défaut : **aucun** fichier admin (`tccd-admin`,
  `tccd-admin.service`, `…Admin.service`) ; le `.deb` par défaut n'en contient
  aucun. Le service n'est **pas actif**.
- Variante admin testée avec le **binaire réel** `tccd-admin` produit par
  `cargo build --release -p tcc-admin` (pas un faux binaire) :
  - `TCC_WITH_ADMIN=1 install-tree.sh` → binaire 0755, unité et activation
    D-Bus installées ; `validate.sh --tree` : **393 OK / 0 échec** ;
  - `.deb` jetable dans `/tmp` : `lintian` 0 erreur ;
  - installation dans `debian:trixie`, `systemd-analyze verify` de l'unité
    admin : **aucun diagnostic** ; `systemd-analyze security` : **1.6** ;
  - `tccd-admin --version` répond.
  Ce paquet n'est **pas publié** et le service n'est **pas validé
  fonctionnellement** (phase 1 en cours) : l'implémentation polkit/`Control1`/idle-timeout doit
  être validée par `linux-platform`.

Après installation réelle du `.deb` dans Debian trixie :

```sh
systemd-analyze verify /usr/lib/systemd/system/tccd.service   # aucun diagnostic
```

### 2.2 `tccd-admin-hw` phase 2 (préparé, non installé, service non validé)

Le binaire `tccd-admin-hw 0.1.0` a été livré pendant la tâche. Il est validé
au niveau **packaging + bus isolé**, pas au niveau matériel.

- `systemd-analyze verify packaging/systemd/tccd-admin-hw.service` : seul
  diagnostic `ExecStart ... is not executable` dans une arborescence où le
  binaire n'est pas installé (attendu) ; aucun diagnostic dans
  `/tmp/opencode/realhw`.
- **Correction après validation réelle** : le premier démarrage sur la
  machine de l'utilisateur a échoué avec `status=226/NAMESPACE` et
  `Failed to set up mount namespacing: /proc/sys: No such file or directory`.
  `ProcSubset=pid` masque `/proc/sys` ; l'entrée `ReadOnlyPaths=/proc/sys`
  a été retirée (le masquage est plus fort). `validate.sh` refuse désormais
  ce motif. Les autres `ReadOnlyPaths` ne croisent ni `ProcSubset` (limité à
  `/proc`), ni `PrivateDevices` (`/dev`), ni `ProtectHome`.
- `systemd-analyze security --offline=true` : **1.9 OK** (systemd 262 hôte et
  systemd 257 dans `debian:trixie`), cible design §4.4 **≤ 3.0**.
- Unité : `PrivateDevices=no`, `DevicePolicy=closed`,
  `DeviceAllow=/dev/tuxedo_io rw`, `ProtectKernelTunables=no` compensé par
  `ReadOnlyPaths` (`/sys/kernel`, `/sys/module`, `/sys/devices/system/cpu`,
  classes `/sys/class/*` préfixées `-` pour tolérer une classe absente) ;
  **`/proc/sys` volontairement absent** : `ProcSubset=pid` le masque ; un
  `ReadOnlyPaths=/proc/sys` provoque `226/NAMESPACE` (voir la correction ci-dessus) ;
  **aucun `ReadWritePaths=`** (design §4.3 : « Hw aucun »), `/etc` et
  `/var/lib` explicitement en lecture seule ; `CapabilityBoundingSet=` vide,
  `MemoryMax=256M` (AH-01 : trame zbus proche de 128 Mio malgré le garde
  64 Kio ; ce n'est pas un budget de fonctionnement), `TasksMax=16`,
  `SystemCallFilter=@system-service`
  (`ioctl` vérifié présent dans le filtre), `RuntimeMaxSec=5min`,
  `Restart=no`, pas de `[Install]`, pas de `StateDirectory`.
- Gating (install-tree) :
  - arborescence par défaut → **aucun** fichier Admin.Hw ;
  - `--with-admin` seul → phase 1 installée, **aucun** fichier Admin.Hw ;
  - `TCC_WITH_ADMIN_HW=1` avec le binaire déplacé temporairement → **échec
    explicite** (code 1), aucune unité ni activation installée ;
  - avec le **binaire réel** (`target/release/tccd-admin-hw`, ELF 4,3 Mio,
    lié à `libc`/`libgcc_s`/`libm` uniquement) :
    `install-tree.sh --with-admin --with-admin-hw` → binaire 0755, unité
    0644, activation 0644 ; `validate.sh --tree` : **395 OK / 0 échec**.
- Binaire installé :
  - `--version` → `tccd-admin-hw 0.1.0 (com.tuxedocomputers.tccd.AdminHw1
    contrat v1)` ;
  - sans argument en non-root → code **77** et message « root requis ».
- Service exercé dans `debian:trixie` sur un **bus système isolé** (policy
  livrée incluse, client non privilégié) :
  - `AdminHw1.Ping` → `true` ; `GetVersion` → `0.1.0` ;
    `GetContractVersion` → `1` ;
  - `GetTdpValues` sans matériel → erreur typée `NotSupported: tdp`
    (pas de crash, pas de faux succès) ;
  - `SetTdpValues` sans polkit → `NotAuthorized`, journal
    `PolicyKitUnavailable` (fail-closed) ;
  - `Properties.Get` → `AccessDenied` par le bus (2 règles : deny global +
    deny Properties) ;
  - arrêt propre sur SIGTERM.
  Ce test ne valide **pas** les écritures TDP/ODM/cTGP, polkit réel, ni le
  matériel : voir §8.
- Paquet Arch reconstruit avec `--with-admin --with-admin-hw`
  (`_with_admin=1`, `_with_admin_hw=1`) : binaire, unité et activation
  présents ; `validate.sh --tree` sur l'archive extraite : **395 OK / 0
  échec** ; `tccd-admin-hw --version` OK, non-root → **77**. L'unité
  embarquée ne contient **aucun** `ReadOnlyPaths=…/proc/sys`, et
  `tccd-user.service` décrit l'accès UVC à la demande (ACL, pas de
  `DeviceAllow`/`PrivateDevices`).
- `.deb` conditionnel (`TCC_WITH_ADMIN_HW=1`, **sans** phase 1) : contenu
  exact (binaire `0755 root:root`, unité et activation `0644`), `lintian`
  **0 erreur**, aucune trace de `tccd-admin`. Installé puis purgé dans
  `debian:trixie` : `--version` OK, `systemd-analyze verify` sans diagnostic,
  score **1.9**, exécution non-root → code **77**, purge propre (aucun des
  trois fichiers ne subsiste). Le `.deb` par défaut, lui, n'en contient aucun.
- Manifests : `rpmspec -P` (défaut, `--with admin`, `--with admin_hw`, les
  deux) OK ; `--with admin` seul n'ajoute pas les fichiers Admin.Hw et
  `--with admin_hw` seul n'ajoute pas la phase 1 ; `makepkg
  --printsrcinfo` OK.
- D-Bus : 20 cas Admin.Hw ajoutés au test fonctionnel (voir §3.2).

### 2.3 Webcam UVC : `tccd` lecture, `tccd-user` écriture

- `tccd.service` conserve `DeviceAllow=char-video4linux r` et
  `PrivateDevices=no` + `DevicePolicy=closed` : énumération/lecture V4L2
  uniquement, score **3.3** inchangé. `validate.sh` refuse
  `char-video4linux rw`.
- `tccd-user.service` n'a **aucun `DeviceAllow`/`DevicePolicy`/`PrivateDevices`**
  et aucune capacité : il ouvre à la demande `/dev/videoN` en lecture/écriture
  pour `User1.SetUvcControl`. L'accès dépend de l'ACL `uaccess` de logind (ou
  du groupe `video`), pas de polkit. `validate.sh` échoue si un `DeviceAllow`
  ou `PrivateDevices=yes` est ajouté à cette unité.
- Conservation : `ProtectSystem=strict`, `ProtectHome=read-only`,
  `PrivateTmp` absent (socket X11), `RestrictAddressFamilies=AF_UNIX`,
  `CapabilityBoundingSet=` vide. Score **3.6** inchangé.
- `systemd-analyze verify packaging/systemd/tccd-user.service` : aucun
  diagnostic (hors `ExecStart` non installé).
- La règle udev `99-tuxedo-rust-center-webcam.rules` ne fait que `TAG+=` :
  aucune ACL, aucun processus. Aucune règle udev ni action polkit
  supplémentaire n'est nécessaire.
- Non testé : écriture réelle sur `/dev/video0` (allowlist, ACL, `Busy` si une
  autre application détient la caméra) ; validation matérielle à faire.

---

## 3. Politique D-Bus

### 3.1 Chargement réel par `dbus-daemon`

`validate.sh --repo` charge le fichier dans un bus de test isolé
(`<listen>unix:tmpdir=…</listen>` + `<include>`). Résultat : le bus démarre,
la politique est acceptée. Un parse invalide est bloquant.

### 3.2 Test fonctionnel allow/deny

```sh
scripts/release/dbus-policy-test.sh packaging/dbus/com.tuxedocomputers.tccd.conf
```

Exécuté dans `debian:trixie` avec `dbus-daemon`, `dbus-test-tool`, un service
root et un client non privilégié :

| Cas | Résultat |
|---|---|
| `GetStateJSON` | **OK** (délivré) |
| `Introspect` | **OK** (délivré) |
| `SetTempProfile` | **OK** (délivré, risque documenté) |
| méthode non listée `Evil` | **OK** (refusée par le bus, `AccessDenied`) |
| `Properties.Set` | **OK** (refusé par le bus) |
| revendication du nom par un non-root | **OK** (refusée) |
| `Manager1.GetContractVersion` | **OK** (délivré, P-12) |
| `Manager1.GetHardwareSnapshotJSON` | **OK** (délivré) |
| `Manager1.GetSensorDataCollectionStatus` | **OK** (délivré) |
| `Manager1.GetShutdownTimerJSON` | **OK** (délivré, lecture) |
| `Manager1.SetSensorDataCollectionStatus` | **OK** (délivré, bail capteurs) |
| `Manager1.SetChargingProfile` / `SetChargingPriority` | **OK** (refusés par le bus, M1-01/M1-02) |
| `Manager1.SetChargeStartThreshold` / `SetChargeEndThreshold` / `SetChargeType` | **OK** (refusés par le bus, M1-01/M1-02) |
| `Manager1.SetKeyboardStatesJSON` / `SetLightbarStatesJSON` / `SetFnLockStatus` | **OK** (refusés par le bus, M1-01/M1-02) |
| `Admin1.SaveProfilesJSON` / `SaveSettingsJSON` / `ReloadConfig` | **OK** (délivrés ; polkit dans le service) |
| `Admin1.Ping` / `GetVersion` / `GetContractVersion` | **OK** (délivrés, lecture seule) |
| `tccd Properties.Get` / `GetAll` | **OK** (refusés par le bus, P-12) |
| `Admin Properties.Get` / `GetAll` | **OK** (refusés par le bus, A1-11) |
| `Admin1.SetTdpValues` / `SetOdmProfile` / `SetNvidiaCtgpOffset` / `SetPrimeMode` | **OK** (refusés par le bus) |
| `Admin1.TomteRun` / `PackagesApply` | **OK** (refusés par le bus) |
| `Admin Properties.Set` | **OK** (refusé par le bus) |
| revendication du nom `.Admin` par un non-root | **OK** (refusée) |
| `Control1.ReloadConfig` non-root | **OK** (refusé par le bus) |
| `Control1.ReloadConfig` root | **OK** (délivré) |

Tests runtime complémentaires (faux authorizer, pas de vrai polkit) :

```sh
cargo test -p tcc-daemon --bin tcc-daemon setter_denied_by_polkit_does_not_write
cargo test -p tcc-daemon --bin tcc-daemon unavailable_polkit_fails_closed
cargo test -p tcc-daemon --bin tcc-daemon lighting_setters_use_settings_action
```

→ **3/3 OK** : refus polkit ⇒ aucune écriture ; polkit indisponible ⇒
fail-closed ; clavier/lightbar/Fn-Lock utilisent bien `.settings`. Ces tests
prouvent l'authz interne, mais la policy reste **fermée** jusqu'à la levée de
M1-01/M1-02. Le **vrai** polkit sur le bus système et un utilisateur non admin
restent à tester à la réouverture.

Conclusion : la politique est bien **fail-closed** (61/61). Toute méthode
ajoutée sans mise à jour du `.conf` sera injoignable. Les setters `Manager1`
existent dans le binaire mais **ne sont pas joignables par le bus** ; ils ne
seront rouverts qu'après `CheckAuthorization` + test dédié.

Note : `GetHardwareSnapshotJSON` reste autorisé (lecture) ; le coût CPU et le
bail capteurs sont bornés côté daemon (REVIEW_HARDWARE H-02), pas par le bus.

---

## 4. Paquet Debian

### 4.1 Construction

```sh
scripts/release/build.sh --source
# → build/packages/tuxedo-rust-center_0.1.0-1_amd64.deb (~2,5 Mo)
# → build/tuxedo-rust-center-0.1.0.tar.gz
```

Inspection :

```sh
dpkg-deb --info  build/packages/*.deb
dpkg-deb --contents build/packages/*.deb
```

- **aucun** chemin `./opt/` ;
- **aucun** setuid/setgid ;
- `tccd-admin` absent ;
- `tccd-sleep.service` absent ;
- `tccd.service`, `tccd-user.service`, D-Bus, polkit, udev, tmpfiles, desktop,
  icônes, metainfo, QML/assets, licences et docs présents ;
- `Installed-Size` = 17 544 KiB (paquet ≈ 3,2 Mo) ;
- conffile unique : `/etc/tuxedo-rust-center/README`.

### 4.2 lintian

```sh
docker run --rm -v "$PWD/build/packages:/pkg:ro" debian:trixie \
  lintian --fail-on error /pkg/*.deb
```

**0 erreur.** Avertissements restants :

- `maintainer-script-calls-systemctl` (le `systemctl daemon-reload` standard, `postinst`) ;
- `no-manual-page` (`tuxedo-rust-center`, `tcc-cli`) — pages de manuel non
  livrées à ce jalon, documenté ;
- `initial-upload-closes-no-bugs` (premier envoi, sans objet hors archive).

### 4.3 Installation / désinstallation réelles

Dans `debian:trixie`, avec résolution des dépendances Qt 6 :

```sh
apt-get install -y ./tuxedo-rust-center_0.1.0-1_amd64.deb
```

Résultat : **succès**, toutes les dépendances `Depends` sont satisfaites
(Qt 6 core/gui/qml/quick/controls/templates/svg, `qt6-svg-plugins`,
`qt6-qpa-plugins`, modules QML). Vérifications exécutées après installation :

- binaires aux chemins FHS exacts, `tccd-admin` absent ;
- `tccd.service` et `tccd-user.service` installés ; `tccd-sleep` et
  `tccd-admin` absents ;
- `systemd-analyze verify` : aucun diagnostic ;
- `systemd-analyze security` : **3.3 OK** ;
- `desktop-file-validate` : OK ;
- `appstreamcli validate --no-net` : succès (1 information « pedantic ») ;
- **smoke test GUI** : `QT_QPA_PLATFORM=offscreen timeout 3
  /usr/bin/tuxedo-rust-center` reste actif (code 124) et ne journalise
  **aucune** erreur `Unsupported image format` — le plugin SVG
  `qt6-svg-plugins` est bien installé par les dépendances ;
- aucun setuid, aucune capability ;
- `tccd --check` (dry-run) : code 0, JSON d'état valide ;
- **politique D-Bus installée** :
  `dbus-policy-test.sh /usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf`
  → **61/61** (`Manager1.GetContractVersion`, `Admin1.GetContractVersion` et
  `AdminHw1.GetContractVersion` autorisés ; `Properties.Get/GetAll` refusés
  sur tccd, Admin et Admin.Hw ; `Properties.Set` refusé ; setters
  `Manager1`/`Admin1` refusés ; méthodes Admin.Hw non listées refusées ;
  `Control1` root-only).

> **Finding corrigé pendant la validation** : `libqt6svg6` ne contient pas le
> plugin image Qt (`libqsvg.so`). Sans `qt6-svg-plugins`, toutes les icônes SVG
> échouent (`Unsupported image format`) et la GUI s'affiche sans icônes.
> `qt6-svg-plugins` a été ajouté à `Depends`. Sur Fedora et Arch, le plugin est
> inclus dans `qt6-qtsvg` / `qt6-svg`.

Contrôle d'intégrité après installation :

```sh
dpkg -V tuxedo-rust-center   # aucune sortie
```

Puis :

```sh
apt-get purge -y tuxedo-rust-center
```

Vérifications : binaires, `/usr/share/tuxedo-rust-center`,
`/var/lib/tuxedo-rust-center` et `/etc/tuxedo-rust-center/README` supprimés.
`/etc/tcc` n'est jamais touché.

---

## 5. Paquet RPM

- `rpmspec -P distro/rpm/tuxedo-rust-center.spec` : **parse OK** dans
  `fedora:latest` ;
- dépendances générées : Qt 6 Core/Gui/Qml/QuickControls2, glibc, libstdc++,
  `systemd`, scriptlets `/bin/sh` ;
- `Recommends`/`Suggests`/`Conflicts`/`Obsoletes`/`Provides` corrects ;
- build complet dans `fedora:latest` (avant l'ajout du conditionnel admin) : **succès** ;
- `rpmspec -P` et `rpmspec -P --with admin` : **parse OK** après les changements admin ;
  le build complet et `rpmlint` doivent être rejoués par la CI ;
- `rpmlint` final : **0 erreur**, 13 avertissements, 0 badness. Un
  `spelling-error ('js', ...)` dû à « Node.js » a été corrigé en
  « Node/Python ». Avertissements restants :
  binaires non strippés (RUSTFLAGS Fedora), `tmpfile-not-in-filelist /run/...`
  (volontaire : tmpfiles crée `/run`), `dbus-policy-allow-receive`
  (nécessaire pour `StateChanged`), `dangerous-command-in-%post rm`
  (nettoyage du lien `tccd-sleep` de l'ancien TCC), pages de manuel absentes,
  `obsolete-not-provided tuxedofancontrol` ;
- installation/désinstallation dans `fedora:latest` : **succès**, GUI
  offscreen sans erreur d'image (`gui_rc=124`, 0 `Unsupported image format`) ;
  le job `build-rpm` de la CI rejoue le build complet.

---

## 6. Paquet Arch

- `makepkg --printsrcinfo` : **parse OK** ;
- `options=('!debug' '!strip' '!lto')` : le `!lto` évite un échec de link
  cxx-qt/Qt observé avec le LTO par défaut de `makepkg` (symboles C++ non
  résolus) ;
- build complet dans `archlinux:latest` (avant `_with_admin`) : **succès** ;
- `makepkg --printsrcinfo` : **parse OK** après ajout de `_with_admin` ; le
  build complet doit être rejoué par la CI.
- `namcap` : **0 erreur** (`NAMCAP_EXIT=0`), avertissements documentés :
  `usr/libexec` non standard pour Arch (chemin imposé par `AGENTS.md` et les
  units), modules QML « uninstalled dependency » (faux positifs : `qt6-base`,
  `qt6-declarative`, `qt6-svg` sont bien dans `depends`), répertoire
  `system-services` vide (corrigé depuis : il n'est plus installé sans
  `tccd-admin`), `ld-linux` inutilisé (binaire Rust standard) ;
- installation, `systemd-analyze verify` et désinstallation : **succès** ;
- GUI offscreen installée : **succès**, aucune erreur d'image.

---

## 7. Reproductibilité

Commandes disponibles et vérifiées :

```sh
SOURCE_DATE_EPOCH=1793923200 scripts/release/build.sh
sha256sum build/packages/*
```

- `Cargo.lock` verrouillé, `cargo build --release --locked` ;
- mtimes de l'arborescence ramenés à `SOURCE_DATE_EPOCH` avant archivage ;
- archive source `tar --sort=name --mtime=@epoch --owner=0 --group=0` +
  `gzip -9n` ;
- `dpkg-deb --root-owner-group -Zxz -z9`, `DEBIAN/*` datés avant archivage ;
- RPM : `use_source_date_epoch_as_buildtime`,
  `clamp_mtime_to_source_date_epoch` ;
- `build/packages/SHA256SUMS` généré par `build.sh`, signature détachée si
  `TCC_GPG_KEY` est défini.

Non vérifié localement : comparaison bit-à-bit de deux builds successifs
(nécessite deux environnements identiques). À faire dans la CI de release
(amélioration, CHECKLIST §7).

---

## 8. Ce qui n'a pas pu être validé localement

| Point | Raison | Où ce sera validé |
|---|---|---|
| Rebuild après les derniers ajustements de scriptlets (`%post` conditionnel, `.install`) | rebuild lourd | jobs `build-rpm` / `build-arch` |
| `PrivateDevices=no` + `DeviceAllow=/dev/tuxedo_io` | pas de laptop TUXEDO | test matériel (Clevo/Uniwill) |
| V4L2 `char-video4linux r` (V-17) | pas de caméra dans le conteneur | test caméra réelle : lecture `tccd`, écriture `tccd-user` (`User1.SetUvcControl`, ACL), `Busy` si occupée |
| Ordre de chargement `tuxedo_io` (R-22) | idem | test matériel |
| Autorisation polkit réelle de `AdminHw1.Set*` | pas de polkit sur le bus isolé ; bus système réel non testé | root + utilisateur non admin sur bus système |
| Écritures TDP/ODM/cTGP et `ioctl` sous `@system-service` | pas de laptop TUXEDO | `strace` + validation matérielle (design §4.6/§8.5) |
| `DeviceAllow=/dev/tuxedo_io` réel | pas de module TUXEDO dans le conteneur | test matériel Clevo/Uniwill |
| Vrai polkit (`CheckAuthorization`) | tests avec faux authorizer uniquement | root + utilisateur non admin sur bus système |
| `PrepareForSleep` | implémenté, non validé sur matériel | `systemctl suspend` sur laptop TUXEDO |
| Refresh rate Wayland réel | dépend du compositeur | tests KDE/Hyprland/Sway |
| `systemd-analyze security` sur Debian/Fedora | versions systemd différentes | CI (scores archivés) |

---

## 9. Synthèse

| Élément | État |
|---|---|
| Arborescence FHS exacte | **validée** (staging + `.deb` installé) |
| Aucun `/opt` | **vérifié** (dépôt, staging, `.deb`) |
| Aucun setuid/capability | **vérifié** |
| Units réellement installées | `tccd`, `tccd-user` ; `tccd-admin`, `tccd-admin-hw` et `tccd-sleep` **non installés par défaut** |
| Hardening | `tccd` 3.3, `tccd-user` 3.6, `tccd-admin` (phase 1) **1.6**, `tccd-admin-hw` (phase 2 préparée) **1.9** |
| `tccd-admin-hw` | binaire livré, install/validate OK, service démarré sur bus isolé (Ping/GetVersion/GetContractVersion), écritures fail-closed ; **polkit réel et matériel non validés** |
| Politique D-Bus | **fail-closed, test fonctionnel 61/61** ; `Manager1` setters **fermés** (M1-01/M1-02), `Admin1` setters phase 2/3 fermés, `AdminHw1` membres listés un par un, `Control1` root-only |
| Polkit | IDs versionnés, défauts durcis ; **application phase 1 en cours de validation** (non revendiquée active) |
| Debian | **installable, GUI chargée, purgeable dans un conteneur propre** |
| RPM | build, `rpmlint` 0 erreur, install/désinstall, GUI validés localement |
| Arch | build, `namcap` 0 erreur, install/désinstall, GUI validés localement |
| Reproductibilité | mécanismes en place, comparaison bit-à-bit à faire en CI |

Findings de `REVIEW_JALON1.md` fermés côté packaging : **R-01, R-02, R-05,
R-06, R-07, R-08, R-09, R-21, R-22**.
Findings de `REVIEW_HARDWARE.md` : **H-10** fermé (allow d'interface
`Manager1` supprimé, membres énumérés) ; **H-05** est **temporairement
neutralisé** par le deny bus, en attente de M1-01/M1-02. Le **vrai polkit** et
le refus d'un utilisateur non admin restent à tester à la réouverture.

`REVIEW_ADMIN_PHASE1.md` : **A1-06/A1-11 fermés côté policy**
(`Admin1.Ping`, `GetVersion`, `GetContractVersion` en lecture seule ;
`Properties.Get`/`GetAll` retirés de `.Admin` ; `Properties.Set` absent).
**P-12 fermé** : `Manager1.GetContractVersion` autorisé ; `Properties.Get`/
`GetAll` retirés de `tccd` et `.Admin`. **LUKS est hors périmètre** (décision
utilisateur) : l'action polkit LUKS, les méthodes `Luks*` et toute référence
à un outil de chiffrement ont été retirées ; `validate.sh` échoue si elles
réapparaissent. **A1-03** (activité touchée avant autorisation, durée root
jusqu'à 5 min) reste côté runtime `linux-platform`.
A1-01/A1-02/A1-04/A1-05/A1-07 sont hors packaging.

`REVIEW_MANAGER1_AUTHZ.md` : **M1-01** (bucket UID + `DialogGate`) et
**M1-02** (lighting respectant `keyboard_backlight_control_enabled`) sont
**bloquants** avant l'ouverture des 8 setters. La policy les maintient en
`deny` explicite ; les actions polkit `.charging`/`.settings` restent
installées. Réouverture après correctifs + revue `security` + test réel.

`ADMIN_DESIGN.md` (AD-1..AD-12) : le **packaging phase 1 est prêt** (policy
`Admin1` membre par membre, lectures `Ping`/`GetVersion`/`GetContractVersion`,
`Control1` root-only, unité durcie, polkit `profiles` sans `_keep`, gating
`TCC_WITH_ADMIN=1`). **A1-06/A1-11/P-12 fermés côté policy** ; A1-03 (activité
touchée avant autorisation) reste côté runtime. Le binaire admin existe, mais
le service n'est **pas validé fonctionnellement** : **non installé par défaut,
non actif**. Le vrai polkit et le vrai `Control1` restent à valider par
`linux-platform`.

R-03/R-04 (polkit appliqué, rate limit par UID), H-02 (plafond du bail
capteurs) et `PrepareForSleep` restent à implémenter par `linux-platform`.
