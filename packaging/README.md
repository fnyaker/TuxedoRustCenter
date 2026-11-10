# Packaging — intégration système TuxedoRustCenter

Squelettes d'intégration système installés par les paquets. **Aucun `/opt`,
aucun runtime Node/Python, aucun setuid.** Les chemins respectent le FHS.

Statut : jalon 1/2. Binaires livrés : `tccd`, `tccd-user`, `tuxedo-rust-center`
(GUI), `tcc-cli`. `tccd-admin` **n'est pas livré par défaut** : le binaire existe, le service n'est pas validé fonctionnellement (D1). `tccd-admin-hw` (phase 2, TDP/ODM/cTGP) est livré, mais le service n'est pas validé fonctionnellement et rien n'est installé par défaut.

La source de vérité des chemins installés est
`scripts/release/install-tree.sh` ; les manifests deb/rpm/Arch l'appellent.

## Fichiers livrés

| Fichier du dépôt | Destination installée | Statut |
|---|---|---|
| `packaging/systemd/tccd.service` | `/usr/lib/systemd/system/tccd.service` | **installé** |
| `packaging/systemd/tccd-user.service` | `/usr/lib/systemd/user/tccd-user.service` + `graphical-session.target.wants/` | **installé** |
| `packaging/systemd/tccd-admin.service` | `/usr/lib/systemd/system/tccd-admin.service` | **installé par défaut** (phase 1, activation D-Bus ; `TCC_WITH_ADMIN=0` pour l'exclure) |
| `packaging/systemd/tccd-admin-hw.service` | `/usr/lib/systemd/system/tccd-admin-hw.service` | **non installé** (phase 2 préparée, service non validé) |
| `packaging/systemd/tccd-sleep.service.example` | — | **référence seule** (R-06) |
| `packaging/dbus/com.tuxedocomputers.tccd.conf` | `/usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf` | **installé** |
| `packaging/dbus/com.tuxedocomputers.tccd.Admin.service` | `/usr/share/dbus-1/system-services/...Admin.service` | **non installé** (phase 1 préparée) |
| `packaging/dbus/com.tuxedocomputers.tccd.Admin.Hw.service` | `/usr/share/dbus-1/system-services/...Admin.Hw.service` | **non installé** (phase 2 préparée) |
| `packaging/polkit/com.tuxedocomputers.tccd.policy` | `/usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy` | **installé** (actions déclarées, non encore appelées) |
| `packaging/tmpfiles/tuxedo-rust-center.conf` | `/usr/lib/tmpfiles.d/tuxedo-rust-center.conf` | **installé** |
| `packaging/udev/99-tuxedo-rust-center-webcam.rules` | `/usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules` | **installé** |
| `packaging/desktop/tuxedo-rust-center.desktop` | `/usr/share/applications/tuxedo-rust-center.desktop` | **installé** |
| `packaging/icons/hicolor/...` | `/usr/share/icons/hicolor/{scalable,256x256,128x128,64x64,48x48,32x32}/apps/tuxedo-rust-center.{svg,png}` | **installé** |
| `packaging/metainfo/*.metainfo.xml` | `/usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml` | **installé** |
| `packaging/etc/README` | `/etc/tuxedo-rust-center/README` (conffile) | **installé** |
| `packaging/changelog` | `/usr/share/doc/tuxedo-rust-center/changelog.gz` | **installé** |

Binaires :

| Binaire Cargo | Chemin installé |
|---|---|
| GUI | `/usr/bin/tuxedo-rust-center` |
| CLI | `/usr/bin/tcc-cli` |
| Daemon | `/usr/libexec/tuxedo-rust-center/tccd` |
| Agent de session | `/usr/libexec/tuxedo-rust-center/tccd-user` |
| Admin à la demande | `/usr/libexec/tuxedo-rust-center/tccd-admin` (**non installé**) |
| Admin matériel | `/usr/libexec/tuxedo-rust-center/tccd-admin-hw` (installé si `TCC_WITH_ADMIN_HW=1`) |

Données (D2) :

| Donnée | Chemin | Droits |
|---|---|---|
| Configuration administrateur (lecture seule daemon) | `/etc/tuxedo-rust-center/` | root:root 0755 |
| État mutable (profils/settings/fantables/webcam) | `/var/lib/tuxedo-rust-center/` | root:root 0755 (fichiers 0644) |
| Runtime | `/run/tuxedo-rust-center/` | root:root 0755, créé par tmpfiles/systemd |
| Assets/QML | `/usr/share/tuxedo-rust-center/{qml,assets}/` | root:root 0644 |
| Documentation | `/usr/share/doc/tuxedo-rust-center/` | root:root 0644 |

## Accès périphériques (V-17)

`tccd.service` utilise `PrivateDevices=no`, `DevicePolicy=closed` et :

```ini
DeviceAllow=/dev/tuxedo_io rw
DeviceAllow=char-video4linux r
```

`char-video4linux r` couvre l'énumération/lecture V4L2 des webcams **dans
`tccd`** ; `validate.sh` refuse `char-video4linux rw` et toute règle
`DeviceAllow` dans `tccd-user.service`.

L'écriture UVC passe par **`tccd-user`** (`User1.SetUvcControl`, allowlist,
rate limit, erreurs typées). L'agent ouvre **à la demande** `/dev/videoN` en
lecture/écriture, sans `DeviceAllow` ni `PrivateDevices` : l'accès repose sur
l'ACL `uaccess` de logind (ou le groupe `video`). Aucun polkit, aucune
capacité, aucun nouveau service D-Bus système. Une autre application peut
détenir la caméra ; l'agent renvoie `Busy` sans contourner l'exclusion du
noyau. Voir `docs/packaging/SECURITY.md` V-17.

La règle udev `99-tuxedo-rust-center-webcam.rules` ne fait que poser un tag
(`TAG+=`) : elle ne lance rien et ne pose pas d'ACL. L'ACL `uaccess` est celle
des règles udev système (logind) ; aucune règle udev ni action polkit
supplémentaire n'est requise.

## Dépendances QML (GUI)

La GUI importe `Qt.labs.platform` (tray), `QtQuick.Dialogs` (`FileDialog`),
`Qt5Compat.GraphicalEffects`, `QtQml.Models`, `QtQuick.Shapes`, etc. Le tray natif
utilise en plus `QApplication` (glue C++ `qapplication_shim.cpp`) et
**Qt6Widgets**. Les modules runtime sont déclarés :

- Debian : `libqt6widgets6` (QApplication/tray), `qml6-module-qt-labs-platform`,
  `qml6-module-qtquick-dialogs`, `qml6-module-qtqml-models`, plus les modules
  QtQuick déjà listés dans `distro/deb/control.in` ;
- RPM : `qt6-qtbase-gui` (Qt6Widgets) et `qt6-qtdeclarative` (Qt.labs.platform,
  QtQuick.Dialogs, Qt5Compat.GraphicalEffects) ;
- Arch : `qt6-base` (Qt6Widgets) et `qt6-declarative` (Labs Platform,
  Dialogs, Effects).

`validate.sh` mappe chaque import QML vers le paquet Debian attendu et vérifie
la présence de `qt6-qtdeclarative`/`qt6-declarative` dans les manifests
RPM/Arch. La CI `deb-install` exécute `qmlimportscanner` sur l'arborescence
installée et la GUI offscreen.

## Activation

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now tccd.service

# Agent de session : déjà activé globalement par le paquet pour les nouvelles
# sessions graphiques. Pour la session courante :
systemctl --user start tccd-user.service
```

La veille n'utilise plus `tccd-sleep.service` (retiré, REVIEW_JALON1 R-06).
Elle sera gérée dans `tccd` par le signal
`org.freedesktop.login1.Manager.PrepareForSleep` : réapplication du profil au
réveil. **À implémenter par `linux-platform`** ; d'ici là, `tccd` reste actif
pendant la veille et le noyau réinitialise le matériel au réveil (les limites
sont réappliquées au prochain changement d'état ou `SIGHUP`).

`tccd-admin.service` n'a **pas** de `[Install]` et **n'est pas installé par
défaut** ; le binaire `tccd-admin` existe et le service est activé uniquement
par défaut (`TCC_WITH_ADMIN=0` pour l'exclure). Voir §tccd-admin.

## tccd-admin (D1) — préparé, NON installé par défaut

- **Binaire admin livré, service non installé par défaut** : `tccd-admin`
  existe ; ni unité, ni activation D-Bus, ni capacité ne sont installées sans
  `TCC_WITH_ADMIN=1`.
- `TCC_WITH_ADMIN=1` (ou `--with-admin`) exige un binaire `tccd-admin` ou
  `tcc-admin` et échoue sinon. Le nom installé est toujours `tccd-admin`.
  Variantes : `build.sh --with-admin --rpm` (`rpmbuild --with admin`),
  `--arch` (`_with_admin=1`), `.deb` via le pkgroot. **Par défaut : rien.**
- **Phase 1** (unité prête) : persistance des profils/réglages +
  `Control1.ReloadConfig`. Unité durcie :
  `PrivateDevices=yes`, `ProtectKernelTunables=yes`,
  `CapabilityBoundingSet=` vide, `DevicePolicy=closed`, `RuntimeMaxSec=5min`,
  `MemoryMax=192M`, `TasksMax=16`, `SystemCallFilter=@system-service`,
  `Restart=no`, pas de `[Install]`, pas de `StateDirectory`.
- **Phase 2** (TDP/ODM/cTGP/PRIME) : delta séparé, à revoir par `security` :
  `PrivateDevices=no`, `ProtectKernelTunables=no`, `DeviceAllow=/dev/tuxedo_io`,
  capacités mesurées. **Phase 3** : Tomte/WebFAI dans un service plus
  restreint. **LUKS est hors périmètre** (décision utilisateur) : aucun
  service, aucune action polkit, aucune méthode, aucun secret.
- Chaque méthode devra vérifier polkit avec le sujet `system-bus-name`
  (`com.tuxedocomputers.tccd1.*`), même pour root (AD-3).
- Le service n'est **jamais** activé au boot ; il est activé à la demande par
  D-Bus et sort après inactivité (30 s) ou `RuntimeMaxSec`.

## tccd-admin-hw (phase 2) — préparé, NON installé par défaut

Limites matérielles à la demande (TDP PL1/PL2/PL4, profil ODM, offset cTGP
NVIDIA), décision **AD-12 option B2** : service distinct de la phase 1.
Binaire livré (`tccd-admin-hw 0.1.0`), mais le service n'est **pas validé
fonctionnellement** sur bus système réel ni sur matériel : il n'est jamais
installé par défaut.

- Gating **indépendant** : `TCC_WITH_ADMIN_HW=1` / `--with-admin-hw`
  (`build.sh`, `install-tree.sh`), `rpmbuild --with admin_hw`,
  `_with_admin_hw=1` (Arch). **`--with-admin` seul n'installe AUCUN fichier
  Admin.Hw** (l'unité relâche `PrivateDevices` et `ProtectKernelTunables`).
- Binaire requis (`tccd-admin-hw` ou `tcc-admin-hw`) : absent ⇒ échec
  explicite, jamais une activation D-Bus morte.
- Unité : `Type=dbus` (`com.tuxedocomputers.tccd.Admin.Hw`), `Restart=no`,
  `RuntimeMaxSec=5min`, pas de `[Install]`, pas de `StateDirectory`.
  `PrivateDevices=no` + `DevicePolicy=closed` +
  `DeviceAllow=/dev/tuxedo_io rw` ; `ProtectKernelTunables=no` compensé par
  `ReadOnlyPaths` (`/sys/kernel`, `/sys/module`, cpufreq, LEDs, backlight,
  power supply, hwmon, thermal, DRM ; classes `/sys/class/*` préfixées `-`
  pour tolérer une classe absente) ; `/etc` et `/var/lib` en lecture seule ;
  **pas de `/proc/sys`** (masqué par `ProcSubset=pid` ; sinon
  `226/NAMESPACE`), **aucun `ReadWritePaths=`** (le service lit
  `Settings.ppd_override`, ne persiste rien) ; `CapabilityBoundingSet=` vide,
  `MemoryMax=256M` (AH-01 : trame zbus proche de 128 Mio malgré le garde
  64 Kio ; pas un budget de fonctionnement), `TasksMax=16`, `LimitCORE=0`,
  `SystemCallFilter=@system-service` (`ioctl` présent), `Wants=`/`After=`
  `modprobe@tuxedo_io.service`.
- Score `systemd-analyze security` : **1.9** (cible design ≤ 3.0 ; phase 1
  1.6).
- D-Bus `AdminHw1` : `Ping`, `GetVersion`, `GetContractVersion`,
  `GetTdpValues`, `SetTdpValues`, `SetOdmProfile`, `SetOdmProfileOverride`,
  `SetNvidiaCtgpOffset` autorisés **un par un** ; deny d'interface pour le
  reste ; `Properties.*` refusé (A1-11). Polkit
  `com.tuxedocomputers.tccd1.power-limits` (`auth_admin`, sans `_keep`).
- `validate.sh --tree` : binaire, unité et activation vont ensemble ; sans
  binaire, aucun des deux fichiers d'intégration. Détails :
  `docs/packaging/SECURITY.md` §6.1 et `VALIDATION.md` §2.2.

## Overrides de développement (D7)

`TCC_SYSFS_ROOT` et `TCC_CONFIG_DIR` ne sont lus que si la feature Cargo
`dev-overrides` est compilée, jamais en root, et **jamais** transmis par les
unités systemd. Les paquets de release ne l'activent pas :

```sh
cargo build --release --locked          # sans dev-overrides
cargo test -p tcc-core --features dev-overrides
```

## D-Bus et polkit

- Politique **fail-closed** : `deny send_destination` global, puis `allow` par
  interface/membre (Introspectable, Peer, méthodes JSON listées, **membres**
  `Manager1` listés, **membres** `Admin1` listés). Aucun `Properties`
  (`Get`/`GetAll`/`Set`) n'est autorisé (P-12).
- `SetTempProfile` / `ClearTempProfile` restent ouverts (interface legacy) :
  rate limit + audit déjà implémentés côté daemon, polkit non appliqué. Risque
  documenté dans `docs/packaging/SECURITY.md` §3.
- `Manager1` : `GetContractVersion`, `GetHardwareSnapshotJSON`,
  `GetSensorDataCollectionStatus`, `GetShutdownTimerJSON` (lecture seule,
  minuterie réglée via `Admin1.SaveSettingsJSON`) et
  `SetSensorDataCollectionStatus` (bail capteurs) sont autorisés. Les 8 setters matériels (charge, clavier,
  lightbar, Fn-Lock) sont **en deny explicite** en attente de M1-01 (bucket
  par UID + `DialogGate`) et M1-02 (lighting respectant
  `keyboard_backlight_control_enabled`). Les actions polkit
  `com.tuxedocomputers.tccd1.charging` / `.settings` restent installées pour
  l'ouverture future. Ne pas ouvrir avant revue `security` ; voir
  `docs/packaging/SECURITY.md` §3.2.
- `Admin1` phase 1 : `SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig`,
  `Ping`, `GetVersion`, `GetContractVersion` + `Introspectable`/`Peer` sont
  autorisés ; les setters phase 2/3 sont refusés au bus et **aucun
  `Properties`** (Get/GetAll/Set) n'est autorisé sur `tccd` ni `.Admin`
  (A1-11/P-12). `Control1.ReloadConfig` est réservé à root
  (`<policy user="root">`), refusé aux utilisateurs. Voir
  `docs/packaging/SECURITY.md` §3.2/§3.3.
- `Admin.Hw` phase 2 : `AdminHw1` membres listés un par un, deny d'interface
  avant les allow (le reste est injoignable), `Properties.*` refusé, méthodes
  Admin.Hw non joignables via `.Admin` et inversement. Le fichier de policy
  est toujours installé ; l'activation ne l'est que si
  `TCC_WITH_ADMIN_HW=1`. Test fonctionnel : 61/61.
- Polkit : IDs `com.tuxedocomputers.tccd1.*`, `allow_any=no`,
  `allow_inactive=no`, **`auth_admin` sans `_keep` partout**.
  `com.tuxedocomputers.tccd1.profiles` remplace `assign-profile` (supprimée).
  Les actions sont déclarées ; leur **application réelle** par
  `tcc-admin`/`tccd` doit être validée avant d'être considérée comme active
  (phase 1 en cours). Ne pas présenter polkit comme une protection déjà
  effective.
- Aucun `org.freedesktop.policykit.exec.path|argv1|allow_gui` : pas de
  primitive `pkexec`.

## Migration depuis TCC original

Le paquet ne supprime jamais `/etc/tcc` ni `/opt`. Procédure complète :
`docs/packaging/MIGRATION.md`. Résumé :

1. sauvegarder `/etc/tcc` **avant** de désinstaller l'original (son script
   `after_remove` le supprime) ;
2. installer TuxedoRustCenter (Conflits/Replaces avec `tuxedo-control-center`) ;
3. au premier démarrage, `tccd` migre vers `/var/lib/tuxedo-rust-center`
   (validation + écriture atomique), sans supprimer ni modifier `/etc/tcc` ;
   aucune copie `.bak` automatique de `/etc/tcc` n'est créée (R-18) : le
   sauvegarder avant ;
4. si `/opt/tuxedo-control-center` subsiste, le `postinst`/`%post` avertit et
   **n'active ni ne démarre** `tccd` : arrêter l'ancien service, puis
   `systemctl enable --now tccd` après nettoyage.

## Vérifications avant publication

```sh
scripts/release/validate.sh --repo
scripts/release/validate.sh --tree build/pkgroot
systemd-analyze verify packaging/systemd/tccd.service \
    packaging/systemd/tccd-user.service packaging/systemd/tccd-admin.service \
    packaging/systemd/tccd-admin-hw.service
systemd-analyze security --offline=true packaging/systemd/tccd-admin-hw.service

# Admin.Hw : absent par défaut, et refus si le binaire manque
test ! -e build/pkgroot/usr/lib/systemd/system/tccd-admin-hw.service
TCC_WITH_ADMIN_HW=1 scripts/release/install-tree.sh /tmp/hw-root   # échoue
systemd-analyze security --offline=true packaging/systemd/tccd.service
xmllint --noout packaging/dbus/com.tuxedocomputers.tccd.conf \
    packaging/polkit/com.tuxedocomputers.tccd.policy
udevadm verify packaging/udev/99-tuxedo-rust-center-webcam.rules
desktop-file-validate packaging/desktop/tuxedo-rust-center.desktop
appstreamcli validate --no-net packaging/metainfo/*.metainfo.xml
```

`systemd-analyze verify` signale les exécutables absents tant que les binaires
ne sont pas installés ; c'est attendu avant le premier packaging. La CI
rejoue la vérification après installation réelle dans un conteneur.
