# Sécurité du packaging — revue, remédiations et risques résiduels

Périmètre : units systemd, politique D-Bus, polkit, udev, tmpfiles, scripts de
maintenance, manifests. Le code runtime n'est pas revu ici.

Références : `docs/security/DECISIONS.md` (D1–D7),
`docs/security/THREAT_MODEL.md` §8, `docs/security/CHECKLIST.md` §6,
`docs/security/REVIEW_JALON1.md` (findings R-01 à R-23).

Cette version intègre les remédiations **R-01, R-02, R-05, R-06, R-07, R-08,
R-09, R-21, R-22**.

---

## 1. Correspondance D1–D7 → preuves de packaging

| Décision | Preuve dans le paquet |
|---|---|
| **D1** séparation `tccd` / `tccd-admin` | binaires distincts ; `tccd-admin.service` phase 1 sans `[Install]`, `Restart=no`, sans `StateDirectory`, activation D-Bus uniquement ; **non installé par défaut** (binaire présent, service non validé fonctionnellement, gating `TCC_WITH_ADMIN=1`) |
| **D2** état dans `/var/lib`, `/etc` RO | `ReadWritePaths=/var/lib/tuxedo-rust-center /run/tuxedo-rust-center` (pas `/etc`) ; `ConfigurationDirectory=` en lecture seule ; tmpfiles 0755 root:root |
| **D3** JSON legacy gelé | politique D-Bus fail-closed : chaque méthode legacy est autorisée **membre par membre** ; toute nouvelle méthode est injoignable par défaut |
| **D4** PPD propriétaire | `Wants=power-profiles-daemon.service` sans `Requires=` ni `After=` (cycle multi-user) ; aucune unité n'écrit `platform_profile` à l'installation |
| **D5** (règle historique) | **LUKS hors périmètre** : aucun code, service, action polkit, méthode ou secret LUKS dans le paquet ; `LimitCORE=0` reste appliqué |
| **D6** pas de `/proc/*/environ` | `tccd-user.service` fournit l'environnement de session ; aucune unité ne transmet `DISPLAY` depuis `/proc` |
| **D7** overrides dev verrouillés | `install-tree.sh` compile sans feature ; `tccd.service` ne contient ni `EnvironmentFile=` ni `TCC_SYSFS_ROOT`/`TCC_CONFIG_DIR` ; `validate.sh` échoue si ces motifs apparaissent en directive active |

---

## 2. Durcissement systemd

### `tccd.service`

```ini
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/tuxedo-rust-center /run/tuxedo-rust-center
ProtectHome=yes
PrivateTmp=yes
ProtectProc=invisible
ProcSubset=pid
PrivateDevices=no
DevicePolicy=closed
DeviceAllow=/dev/tuxedo_io rw
ProtectKernelTunables=no
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
KeyringMode=private
RestrictAddressFamilies=AF_UNIX AF_NETLINK
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RemoveIPC=yes
SystemCallArchitectures=native
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=256M
TasksMax=64
CPUQuota=25%
CapabilityBoundingSet=
AmbientCapabilities=
```

Score mesuré : **3.3 OK** (systemd 262).

### `tccd-user.service`

`UMask=0077`, `LimitCORE=0`, `ProtectKernelLogs=yes`, `MemoryMax=128M`,
`TasksMax=32`, `NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_UNIX`,
`MemoryDenyWriteExecute=yes`. Score : **3.6 OK**.

Accès UVC : l'agent ouvre **à la demande** un nœud `/dev/videoN` en
lecture/écriture pour `User1.SetUvcControl`, sans `DeviceAllow` ni
`PrivateDevices` (voir V-17). L'autorisation repose sur l'ACL `uaccess` de
logind (ou le groupe `video`), pas sur polkit. `ProtectSystem=strict` et
`ProtectHome=read-only` restent en place ; `PrivateTmp` est absent pour le
socket X11. Une caméra déjà ouverte par une autre application donne une
erreur typée `Busy`, sans contournement de l'exclusion mutuelle du noyau.

R-21 : `ProtectSystem=strict` et `ProtectHome=read-only` d'une unité
**utilisateur** dépendent des namespaces utilisateur non privilégiés. Si la
distribution les restreint (`kernel.apparmor_restrict_unprivileged_userns=1`,
`user.max_user_namespaces=0`), l'unité échoue. Repli documenté dans
`MIGRATION.md` §6 ; il ne réduit pas le privilège (agent déjà non privilégié).

### `tccd-admin.service` (non installé)

`Restart=no`, pas de `[Install]`, pas de `StateDirectory`, `DevicePolicy=closed`,
`MemoryMax=512M`, `KeyringMode=private`, `ProtectKernelLogs=yes`.
**Aucune capacité n'est publiée** : `CapabilityBoundingSet=` est vide. Les
capacités nécessaires estimées sont documentées en commentaire
(`CAP_SYS_ADMIN`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`) et devront être activées
**une par une**, après mesure sur matériel réel (R-07).

### `tccd-sleep.service` — retiré (R-06)

Le fichier est conservé sous `packaging/systemd/tccd-sleep.service.example`
avec la mention « NON INSTALLÉ ». Raisons :

- `systemctl` dans une unité attendant la fin d'un job de la même transaction :
  risque de blocage de `systemctl suspend` ;
- `ExecStop` relance `tccd` même après un arrêt administrateur ;
- score `systemd-analyze security` : **9.2 UNSAFE**.

Remplacement à implémenter par `linux-platform` dans `tccd` : abonnement à
`org.freedesktop.login1.Manager.PrepareForSleep(b)`, relâchement de
`/dev/tuxedo_io` avant veille, réapplication du profil au réveil. Fallback
acceptable : hook binaire Rust dans `/usr/lib/systemd/system-sleep/` (jamais
un script shell avec des arguments non validés).

### Exceptions justifiées

| Exception | Raison | Risque résiduel |
|---|---|---|
| `ProtectKernelTunables=no` | écritures sysfs (cpufreq, backlight, LED, charge, `platform_profile`) ; `ProtectSystem=strict` laisse `/sys` inscriptible, `=yes` le monterait en RO | le daemon peut écrire tout `/sys` inscriptible par root ; atténué par la validation de chemins dans `tcc-hw` et l'absence de chemin fourni par le client |
| `PrivateDevices=no` + `DevicePolicy=closed` (`tccd`) | `/dev/tuxedo_io` n'est pas recréé de façon fiable dans un `/dev` privé sur tous les noyaux | seuls `/dev/tuxedo_io` (rw), `char-video4linux` (r) et les nœuds implicites sont accessibles ; **non validé sur matériel réel** |
| `SystemCallFilter` absent | `ioctl` doit être validé dans `@system-service` ; un filtre trop strict casserait le pilotage matériel | surface de syscalls plus large ; à activer après test |
| `CapabilityBoundingSet=` vide | root est propriétaire des attributs sysfs utilisés | si un pilote expose un nœud non lisible par root sans `CAP_DAC_OVERRIDE`, l'ajouter explicitement et documenter |
| `MemoryMax=256M` | garde-fou anti-OOM (budget réel < 30 Mo) | un pic légitime > 256M serait tué ; marge ×8, à ajuster après mesure |

### V-17 — V4L2 : lecture dans `tccd`, écriture UVC dans `tccd-user`

`tccd` énumère/ouvre les webcams V4L2 (`/dev/video*`) en **lecture seule**
pour les capacités et l'énumération. L'unité root conserve :

```ini
DevicePolicy=closed
DeviceAllow=/dev/tuxedo_io rw
DeviceAllow=char-video4linux r
```

- **`r` uniquement dans `tccd`** : aucune écriture UVC (brightness/contrast/
  exposition) depuis le daemon root. `validate.sh` échoue si
  `char-video4linux rw` apparaît dans `tccd.service`.
- **Écriture UVC : `tccd-user`**, service de session non privilégié, via
  `User1.SetUvcControl` (allowlist de contrôles, valeurs bornées, rate limit,
  erreurs typées). L'agent ouvre **à la demande** le nœud `/dev/videoN` en
  lecture/écriture, uniquement pendant l'appel.
- **Permissions** : `tccd-user` n'a **aucun `DeviceAllow`/`DevicePolicy`** et
  pas de `PrivateDevices`, aucune capacité. L'accès dépend de l'ACL `uaccess`
  posée par logind pour l'utilisateur de la session (ou de l'appartenance au
  groupe `video`). `PrivateDevices=yes` recréerait un `/dev` sans caméra ;
  `DeviceAllow` figerait des chemins dynamiques (`video0`, `video2`, ...) et
  ne remplacerait pas les permissions POSIX/ACL. Si la distribution n'applique
  pas `uaccess` aux nœuds `video4linux`, l'utilisateur doit être membre du
  groupe `video` (paquet/administrateur, hors TCC).
- **Pas de polkit** : l'appel reste sur le bus de session de l'utilisateur ;
  la barrière est la permission du nœud et l'allowlist. Aucun nouveau service
  D-Bus système, aucune action polkit, aucune règle udev supplémentaire.
- **Limite** : une autre application peut détenir la caméra ; l'agent renvoie
  alors une erreur typée `Busy` sans insister ni contourner l'exclusion
  mutuelle du noyau.
- `systemd-analyze security` : `tccd` **3.3**, `tccd-user` **3.6**
  (inchangés ; l'absence de `DeviceAllow` dans l'unité utilisateur ne change
  pas le score).
- `char-video4linux` couvre **tous** les nœuds V4L2 (webcam interne et
  périphériques USB), pas seulement la webcam interne. Un confinement par
  nœud exigerait un tag udev + chemins dynamiques, non retenu ici.
- La validation réelle (caméra présente, ACL `uaccess`, lecture `tccd`,
  écriture `User1.SetUvcControl`, comportement `Busy` si la caméra est
  occupée) reste à faire sur matériel TUXEDO.

### R-22 — `DeviceAllow` et ordre de chargement du module

`DeviceAllow=/dev/tuxedo_io rw` n'est résolu qu'au démarrage de l'unité. Si
`tuxedo_io` n'est pas chargé, systemd peut ne pas ajouter le nœud à la liste
autorisée, et `DevicePolicy=closed` le refuse ensuite. L'unité déclare :

```ini
After=systemd-modules-load.service modprobe@tuxedo_io.service
Wants=modprobe@tuxedo_io.service
```

`Wants=` est non bloquant : un module intégré ou un matériel sans `tuxedo_io`
laisse `tccd` démarrer en mode dégradé. **Cette dépendance n'a pas été validée
sur matériel réel** ; à tester sur un modèle où le module est chargé à la
demande par udev. Si `modprobe@.service` n'existe pas (systemd ancien), le
`Wants=` est simplement inactif.

---

## 2.1 Dépendances QML de la GUI

La GUI est Qt 6 native (aucun QtWebEngine/HTML). Les imports QML
`Qt.labs.platform` (tray), `QtQuick.Dialogs` (`FileDialog`),
`QtQuick.Effects`, `QtQml.Models`, etc. sont couverts par des paquets runtime
déclarés (Debian : `qml6-module-*` ; RPM/Arch : `qt6-qtdeclarative` /
`qt6-declarative`). Le tray natif utilise `QApplication` via une glue C++ et
**Qt6Widgets** : `libqt6widgets6` (Debian), `qt6-qtbase-gui` (RPM),
`qt6-base` (Arch). `validate.sh` échoue si un import direct n'a pas de
dépendance déclarée ; `qmlimportscanner` est rejoué en CI sur l'arborescence
installée. Aucun module web n'est utilisé.

## 3. Politique D-Bus — fail-closed (R-02/R-08)

Le fichier `packaging/dbus/com.tuxedocomputers.tccd.conf` applique :

1. `deny own` pour tous les noms (`tccd`, `.Admin`, `.Admin.Hw` ; seul root les possède) ;
2. `deny send_destination` **global** pour tous les noms ;
3. puis des `allow` **par interface ou par membre** uniquement.

Autorisations actuelles pour `com.tuxedocomputers.tccd` :

| Interface / membre | Raison |
|---|---|
| `org.freedesktop.DBus.Introspectable` | `busctl introspect`, clients |
| `org.freedesktop.DBus.Peer` | `Ping` D-Bus |
| `com.tuxedocomputers.tccd.Ping` / `GetVersion` | diagnostic |
| `...GetStateJSON`, `GetActiveProfileJSON`, `GetProfilesJSON`, `GetSettingsJSON`, `GetFanDataJSON` | lecture (D3) |
| `...SetTempProfile`, `ClearTempProfile` | profil temporaire (risque ci-dessous) |
| `Manager1.GetContractVersion` | version du contrat (P-12) |
| `Manager1.GetHardwareSnapshotJSON` | lecture capteurs |
| `Manager1.GetSensorDataCollectionStatus` | lecture du bail capteurs |
| `Manager1.GetShutdownTimerJSON` | lecture de la minuterie d'extinction (réglage via `Admin1.SaveSettingsJSON`) |
| `Manager1.SetSensorDataCollectionStatus` | bail de collecte (pas une écriture matérielle) |
| `Manager1.Set*` (charge/clavier/lightbar/Fn-Lock) | **refusés au bus** en attente M1-01/M1-02 (§3.2) |

**Aucun `Properties`** (`Get`/`GetAll`/`Set`) n'est autorisé, ni sur `tccd`
ni sur `.Admin` (P-12). La version du contrat passe par
`Manager1.GetContractVersion` et `Admin1.GetContractVersion`. Aucun `allow
send_destination` générique n'existe. Toute méthode ajoutée sans mise à jour
de ce fichier est injoignable depuis le bus.

Admin (phase 1) : `Admin1.SaveProfilesJSON`, `SaveSettingsJSON`,
`ReloadConfig`, `Ping`, `GetVersion`, `GetContractVersion`, `Introspectable`
et `Peer` sont autorisés. **Aucun `Properties.Get`/`GetAll` sur `.Admin`**
(A1-11) : une propriété future ne doit pas être exposée automatiquement ; la
version passe par `Admin1.GetContractVersion`. Les setters phase 2/3 sont
refusés ; `Properties.Set` est absent. **Aucun `Properties`** sur `.Admin` ni
sur `tccd` (P-12). Le nom `.Admin` n'a **aucune activation installée** par
défaut : le binaire
existe, mais le service n'est pas validé fonctionnellement.

`Control1.ReloadConfig` sur `com.tuxedocomputers.tccd` est autorisé **root
uniquement** (`<policy user="root">`), explicitement refusé dans le bloc
`context="default"`. `tccd` re-vérifie l'UID appelant (AD-6).

Admin.Hw (phase 2, préparée) : `AdminHw1.Ping`, `GetVersion`,
`GetContractVersion`, `GetTdpValues`, `SetTdpValues`, `SetOdmProfile`,
`SetOdmProfileOverride`, `SetNvidiaCtgpOffset`, `Introspectable` et `Peer`
sont autorisés **membre par membre**. Un `deny` d'interface `AdminHw1` est
placé **avant** ces allow : toute méthode future non listée reste
injoignable. `Properties.Get/GetAll/Set` est explicitement refusé sur
`.Admin.Hw` (A1-11) ; la version passe par `AdminHw1.GetContractVersion`.
Les méthodes Admin.Hw ne sont pas joignables via `.Admin`, et les setters
phase 1 ne le sont pas via `.Admin.Hw` (test fonctionnel §3.2). Le fichier de
policy est toujours installé ; l'activation D-Bus ne l'est que si
`TCC_WITH_ADMIN_HW=1`.

### 3.3 `Admin1` phase 1 et `Control1` (AD-2/AD-6)

`Admin1` est limité à trois membres d'écriture et à des lectures sans effet
de bord :

| Membre | Rôle | Action polkit prévue |
|---|---|---|
| `SaveProfilesJSON` | persistance des profils | `com.tuxedocomputers.tccd1.profiles` |
| `SaveSettingsJSON` | persistance des réglages | `com.tuxedocomputers.tccd1.settings` |
| `ReloadConfig` | demande de rechargement à `tccd` | `com.tuxedocomputers.tccd1.settings` |
| `Ping` | diagnostic de présence | aucune (lecture) |
| `GetVersion` | version du binaire | aucune (lecture) |
| `GetContractVersion` | version du contrat Admin (A1-11) | aucune (lecture) |

Le bus **n'autorise pas** les setters : phase 2 (`SetTdpValues`,
`SetOdmProfile`, `SetNvidiaCtgpOffset`, `SetPrimeMode`) et phase 3 (`TomteRun`,
`PackagesApply`) sont explicitement refusés pour rendre leur
activation future consciente. Aucun `allow Properties` n'existe sur `.Admin` ;
`Properties.Set` est absent partout. L'admin n'est pas validé
fonctionnellement : l'unité et l'activation D-Bus ne sont **pas installées**
par défaut.

> **A1-06/A1-11 fermés côté policy** ; **A1-03 reste ouvert côté runtime** :
> le service touche son activité avant l'autorisation, donc un client refusé
> peut maintenir le processus root jusqu'à `RuntimeMaxSec=5min`. Les lectures
> `Ping`/`GetVersion`/`GetContractVersion` sont sans polkit ; la durée de vie
> reste bornée par systemd. Correction owner `linux-platform`.
>
> **P-12 fermé** : `Properties.Get`/`GetAll` ont été retirés de `tccd` et
> `.Admin` ; la version passe par `GetContractVersion`. `Properties.Set`
> reste absent. Conséquence assumée : `busctl` ne peut plus lire les
> propriétés (`busctl introspect` peut afficher moins de valeurs) ; les
> clients livrés (GUI, `tcc-cli`, agent) utilisent les méthodes JSON/
> `Manager1`/`Admin1` et ont été vérifiés sans `Properties.Get`.

`Control1.ReloadConfig` est root-only : allow dans le bloc root, deny explicite
pour tous les autres. Le service admin n'est pas actif ; cette policy prépare
son activation, elle ne la déclenche pas.

### Risque résiduel : `SetTempProfile` / `ClearTempProfile`

La GUI et le tray en ont besoin sans authentification. Tant que ces méthodes
restent sur l'interface legacy :

- tout utilisateur local peut changer le profil matériel (CPU, luminosité,
  à terme ventilation) jusqu'au prochain changement d'état PPD/AC ;
- aucune persistance, aucun accès fichier, aucun secret ;
- impact : perturbation thermique/énergétique, pas d'élévation de privilège.

État au 2026-11-06 : le daemon implémente déjà le token bucket par nom unique
(`WriteRateLimiter`, 5 écritures/s) et journalise `caller`/`caller_uid` sans
payload brut. Il reste à appliquer polkit.

Remédiation **bloquante jalon 2** (R-03, owner `linux-platform`) :

1. conserver le token bucket et l'audit (faits) ;
2. déplacer `SetTempProfile` sur `Manager1` ; la couche legacy reste
   strictement en lecture ;
3. option multi-utilisateur strict : `CheckAuthorization` sur
   `com.tuxedocomputers.tccd1.temp-profile` — **action non déclarée à ce
   jour**, car non implémentée.

### 3.2 Setters `Manager1` — deny temporaire (M1-01/M1-02)

Les 8 setters matériels de `Manager1` sont **refusés au bus** par des `deny`
explicites, tant que `security` n'a pas confirmé la levée de :

- **M1-01** : limiteur par **nom unique** seulement, pas de bucket par UID ni
  de `DialogGate` global autour de `CheckAuthorization` ; un utilisateur
  pourrait ouvrir plusieurs connexions et multiplier les dialogues polkit.
- **M1-02** : `SetKeyboardStatesJSON` / `SetLightbarStatesJSON` ne respectent
  pas `keyboard_backlight_control_enabled` (régression H-06) et écriraient
  malgré le contrôle désactivé.

Membres refusés : `SetChargingProfile`, `SetChargingPriority`,
`SetChargeStartThreshold`, `SetChargeEndThreshold`, `SetChargeType`,
`SetKeyboardStatesJSON`, `SetLightbarStatesJSON`, `SetFnLockStatus`.

Restent autorisés : `GetContractVersion`, `GetHardwareSnapshotJSON`,
`GetSensorDataCollectionStatus`, `SetSensorDataCollectionStatus` (bail
capteurs sans polkit, borné H-02). Aucun `Properties`, aucun allow
d'interface.

Le code d'authz polkit (`tccd1.charging` / `.settings`, sujet
`system-bus-name`, fail-closed) est présent et les actions polkit restent
**installées** pour l'ouverture future. Le déblocage exige :

1. correction M1-01 (bucket UID + `DialogGate`) et M1-02 (respect du contrôle
   lighting) côté `linux-platform` ;
2. revue `security` des deux correctifs ;
3. réouverture **membre par membre** de la policy, sans allow d'interface ;
4. `dbus-policy-test.sh` repassé en `check_ok` pour les 8 setters et test réel
   root/polkit (utilisateur non admin ⇒ `AccessDenied`).

`SetTempProfile` legacy reste ouvert et rate-limité (voir ci-dessus).

---

## 4. Polkit — actions déclarées, application phase 1 en validation (R-01/R-04/R-09)

`packaging/polkit/com.tuxedocomputers.tccd.policy` contient des IDs versionnés
`com.tuxedocomputers.tccd1.*` :

| Action | Défaut actif | Cache |
|---|---|---|
| `profiles` (remplace `assign-profile`) | `auth_admin` | **aucun** |
| `settings` | `auth_admin` | aucun |
| `power-limits` (TDP, cTGP) | `auth_admin` | aucun |
| `charging` | `auth_admin` | aucun |
| `display` (YCbCr, résolution) | `auth_admin` | aucun |
| `prime-select` | `auth_admin` | aucun |
| `tomte-run` | `auth_admin` | aucun |
| `packages` (WebFAI) | `auth_admin` | aucun |

`allow_any=no` et `allow_inactive=no` partout. **Aucune action n'utilise
`auth_admin_keep`** : `assign-profile` a été supprimée (ADMIN_DESIGN §3.2) ;
`profiles` ne bénéficie d'aucun cache de 5 min.

Les actions sont déclarées ; leur **application réelle** par
`tcc-admin`/`tccd` est en cours d'implémentation/validation (phase 1). Le
fichier ne doit pas être présenté comme une protection active tant que les
tests d'autorisation ne sont pas passés. Quand `CheckAuthorization` est
appelé, le sujet doit être `system-bus-name(caller_unique_name)`, avec cache
TTL ≤ 60 s invalidé sur `NameOwnerChanged` (CHECKLIST §3.2).

**R-01 fermé** : l'ancienne action `com.tuxedocomputers.tccd.tomte` et toutes
les annotations `org.freedesktop.policykit.exec.path`, `exec.argv1` et
`allow_gui` ont été supprimées. Plus aucune action n'est une primitive
`pkexec`. Tomte passe par `tccd-admin` (`Command::new` chemin absolu,
arguments séparés, `env_clear`, timeout) quand il sera implémenté.

---

## 5. Udev, tmpfiles, fichiers

- udev : `TAG+=` uniquement, **aucun `RUN+=`**, `IMPORT{program}`,
  `PROGRAM=`, ni interpréteur (S-19/F-11) ;
- tmpfiles : `/run`, `/var/lib`, `/etc` en 0755 root:root ;
- aucun setuid/setgid, aucune capability de fichier (vérifié par
  `validate.sh` et la CI) ;
- binaires 0755 root:root, units/policies 0644, répertoires d'état 0755 ;
- `/etc/tuxedo-rust-center/README` est le seul conffile ;
- l'état `/var/lib` est écrit par le daemon en 0644 root:root (D2), jamais par
  un script de paquet ;
- aucun téléchargement, aucune commande `curl | sh`, aucun secret dans les
  scripts ;
- les scripts de maintenance utilisent des chemins littéraux et des commandes
  directes, sans `sh -c` interpolé.

---

## 6. `tccd-admin` : préparé, non installé, non actif (R-07)

Le paquet n'expose ni l'unité, ni l'activation D-Bus, ni aucune capacité par
défaut. Un binaire `tccd-admin` existe, mais le service n'est **pas validé
fonctionnellement**. `install-tree.sh` exige `TCC_WITH_ADMIN=1` **et** un binaire
(`tccd-admin` ou `tcc-admin`) ; l'existence du binaire ne suffit pas à
considérer le service comme prêt. Par défaut, rien n'est installé et
`validate.sh --tree` le vérifie. Le service n'est **pas actif** et ne doit pas
être présenté comme tel.

Unité phase 1 (prête, non installée) :

- `Type=dbus`, `BusName=com.tuxedocomputers.tccd.Admin`, `Restart=no`,
  `RuntimeMaxSec=5min`, pas de `[Install]`, pas de `StateDirectory` ;
- `PrivateDevices=yes`, `ProtectKernelTunables=yes`, `DevicePolicy=closed`,
  `ProtectSystem=strict`, `ReadWritePaths=/var/lib/tuxedo-rust-center`,
  `SystemCallFilter=@system-service`, `CapabilityBoundingSet=` vide,
  `MemoryMax=192M`, `TasksMax=16`, `LimitCORE=0` ;
- phase 2 (TDP/ODM/cTGP/PRIME) : delta séparé, `PrivateDevices=no`,
  `ProtectKernelTunables=no`, `DeviceAllow=/dev/tuxedo_io rw`, capacités
  mesurées ; phase 3 Tomte/WebFAI ; **LUKS hors périmètre** (décision
  utilisateur) : aucun service ni action à prévoir.

Avant `TCC_WITH_ADMIN=1` :

1. binaire présent, testé, sans `unwrap` en production ;
2. `Admin1` vérifie polkit (`profiles`/`settings`) avec le sujet
   `system-bus-name`, **même pour root** ;
3. `Control1.ReloadConfig` re-vérifie l'UID 0 et audite ;
4. persistance uniquement via `ConfigStore::system()` (`/var/lib`, atomique) ;
5. rollback si `ReloadConfig` échoue ;
6. phase 2/3 : revue `security` dédiée, capacités mesurées ;
7. audit `security` de l'unité (`systemd-analyze security` cible ≤ 2.5).

---

## 6.1 `tccd-admin-hw` : phase 2 préparée, non installée, service non validé

Décision AD-12 option B2 : les limites matérielles (TDP PL1/PL2/PL4, profil
ODM, offset cTGP NVIDIA) vivent dans un service **distinct** de la phase 1 et
de PRIME. Le binaire `tccd-admin-hw` est **livré** (0.1.0) : l'unité et la
policy sont vérifiées, et le binaire a été exercé sur un **bus système isolé**
(Ping/GetVersion/GetContractVersion, écriture refusée fail-closed), mais il
n'est pas validé fonctionnellement sur un système réel ni sur matériel. Rien n'est installé par défaut et aucune écriture matérielle n'est
revendiquée validée.

### Gating

- `TCC_WITH_ADMIN_HW=1` ou `install-tree.sh --with-admin-hw` (défaut `0`) ;
  `build.sh --with-admin-hw`, `make-rpm.sh` (`--with admin_hw`),
  `make-arch.sh` (`_with_admin_hw=1`).
- **Indépendant de `TCC_WITH_ADMIN`** : `--with-admin` seul n'installe ni le
  binaire, ni l'unité, ni l'activation, ni aucune capability Admin.Hw.
- Binaire accepté : `tccd-admin-hw` ou `tcc-admin-hw`, installé sous le nom
  stable `tccd-admin-hw`. Absent ⇒ **échec d'installation explicite**, jamais
  d'activation D-Bus morte.
- `validate.sh --tree` : le binaire, l'unité et l'activation vont ensemble ;
  l'absence de binaire implique l'absence des deux fichiers d'intégration.
- `payload` : binaire `0755`, unité `0644`, activation `0644` ; aucun
  `[Install]`, aucun `StateDirectory`, aucun `ReadWritePaths=`.

### Unité (`packaging/systemd/tccd-admin-hw.service`)

| Réglage | Valeur | Justification |
|---|---|---|
| `Type` / `BusName` | `dbus` / `com.tuxedocomputers.tccd.Admin.Hw` | activation à la demande, pas de boot |
| `Restart` / `RuntimeMaxSec` | `no` / `5min` | processus neuf par action, polkit 120 s couvert |
| `PrivateDevices` / `DevicePolicy` | `no` / `closed` | un seul nœud : `/dev/tuxedo_io` |
| `DeviceAllow` | `/dev/tuxedo_io rw` | TDP via ioctl ; ne restreint **pas** sysfs |
| `ProtectKernelTunables` | **`no`** | écritures `platform_profile`/`ctgp_offset` ; décision bloquante D4/D-règles |
| `ReadOnlyPaths` | `/sys/kernel`, `/sys/module`, `/sys/devices/system/cpu`, `-/sys/class/{leds,backlight,power_supply,hwmon,thermal,drm}`, `/etc/tuxedo-rust-center`, `/var/lib/tuxedo-rust-center` | compensation minimale ; le reste du noyau et l'état restent RO. **`/proc/sys` n'est pas listé** : `ProcSubset=pid` le masque entièrement (plus fort qu'un montage read-only). Le préfixe `-` sur `/sys/class/*` tolère une classe absente (desktop sans rétroéclairage/LED) sans faire échouer l'unité ; si le chemin existe, il est bien remonté en lecture seule |
| `ReadWritePaths` | **aucun** | design §4.3 (« Hw aucun ») : le service lit `Settings.ppd_override`, ne persiste rien |
| `ProtectKernelModules` / `Logs` / `ControlGroups` | `yes` | pas de module, pas de log noyau, pas de cgroup |
| `CapabilityBoundingSet` / `AmbientCapabilities` | **vides** | root possède les attributs sysfs et le nœud ; hypothèse à mesurer (design §4.6) |
| `SystemCallFilter` | `@system-service` + `EPERM` | `ioctl` vérifié présent (`systemd-analyze syscall-filter`) ; `strace` matériel à faire |
| `MemoryMax` / `TasksMax` / `LimitCORE` | `256M` / `16` / `0` | AH-01 : zbus 5 peut recevoir une trame proche de 128 Mio malgré le garde 64 Kio ; 256M évite l'OOM avant rejet. Aucun core. Ce n'est pas un budget de fonctionnement (RSS visé < 30 Mo) |
| `UMask` | `0077` | aucun fichier créé |
| `Wants`/`After` | `modprobe@tuxedo_io.service` | garantit `/dev/tuxedo_io` pour `DeviceAllow` ; échec non bloquant |

Écart assumé par rapport au bloc §4.4 du design : `Wants=`/`After=modprobe@`
a été ajouté (le design §4.6 exige le module chargé avant l'activation) et
`ReadOnlyPaths` inclut `/etc` et `/var/lib` (D2), sans `ReadWritePaths`.
**`/proc/sys` et `ProcSubset=pid`** : une première version listait
`ReadOnlyPaths=/proc/sys` en compensation de `ProtectKernelTunables=no`. La
validation réelle a échoué au démarrage :
`status=226/NAMESPACE`, `Failed to set up mount namespacing: /proc/sys: No
such file or directory`, car `ProcSubset=pid` (avec `ProtectProc=invisible`)
masque `/proc/sys` dans le namespace. L'entrée a été retirée : le masquage
est **plus fort** qu'un montage en lecture seule et ne nécessite aucune
compensation. `validate.sh` interdit désormais tout
`ReadOnlyPaths=…/proc/sys` sur cette unité. Les autres chemins ne sont pas
affectés par `ProcSubset` (uniquement `/proc`), `PrivateDevices`
(`/dev`), `ProtectHome` (`/home`, `/root`, `/run/user`) ou
`ProtectSystem=strict` (déjà RO, sans conflit de cible).

`MemoryMax=256M` s'écarte du bloc §4.4 (128M) sur recommandation **AH-01**
(`docs/security/REVIEW_ADMIN_HW.md`) : zbus 5 peut encore recevoir une trame
proche de 128 Mio malgré le garde applicatif de 64 Kio, et 128M tuerait le
service avant qu'il ne la rejette. C'est une mesure de disponibilité, pas un
budget : le RSS réel visé reste < 30 Mo, et la barrière principale reste le
garde 64 Kio côté runtime. La valeur 192M de la phase 1 est propre à la
persistance. Les chemins `/sys/class/*` sont préfixés `-` pour tolérer leur
absence sur un matériel sans rétroéclairage/LED ; cela ne réduit pas la
protection lorsque les chemins existent. Aucun `ConditionPathExists=` : l'activation D-Bus doit répondre
`NotSupported`, pas échouer sans erreur applicative.

### Score et capacités mesurées

- `systemd-analyze security --offline=true` : **1.9** (systemd 262 hôte,
  systemd 257 `debian:trixie`), cible design **≤ 3.0** ; phase 1 : 1.6.
- Aucun capability, aucun `DeviceAllow` supplémentaire, aucun `execve`.
- `DeviceAllow` sur un chemin inexistant est ignoré par systemd et
  `DevicePolicy=closed` refuse l'ouverture : la méthode renvoie
  `NotSupported`, et l'unité doit être relancée après chargement du module
  (documenté, pas de `ConditionPathExists`).
- `ProtectKernelTunables=no` laisse le processus root écrire d'autres
  fichiers `/sys` non listés (P2-R4, risque faible) : la barrière réelle est
  l'absence de chemin contrôlé par le client et une surface de trois scalaires.

### Polkit et policy

- Action `com.tuxedocomputers.tccd1.power-limits` : `auth_admin`, **sans**
  `_keep`, `allow_any=no`, `allow_inactive=no`, aucune annotation `exec.*`.
- `AdminHw1.GetTdpValues` est sans polkit (lecture, rate limit + sortie 30 s) ;
  `SetTdpValues`, `SetOdmProfile`, `SetOdmProfileOverride`,
  `SetNvidiaCtgpOffset` vérifient polkit dans le service avec le sujet
  `system-bus-name` (à implémenter par `linux-platform`).
- Policy D-Bus : membres un par un, deny d'interface, `Properties.*` refusé.
  Test fonctionnel : **61/61** cas, dont 20 pour Admin.Hw.

### Avant `TCC_WITH_ADMIN_HW=1`

1. binaire `tccd-admin-hw` livré et testé (fait : `--version`, démarrage bus
   isolé, lecture/écriture refusée fail-closed) ; reste la validation
   fonctionnelle réelle et matérielle ;
2. polkit réel (`power-limits`, deux appels ⇒ deux dialogues), sujet
   `system-bus-name`, `is_challenge` refusé ;
3. validation matérielle Clevo/Uniwill (ioctl, `platform_profile`,
   `ctgp_offset`), `strace` de `@system-service` (ioctl) ;
4. `Settings.ppd_override` implémenté (sinon `SetOdmProfileOverride` refusé) ;
5. `tccd` : bucket UID (H-05) et bornes TDP/cTGP (H-07/H-08/H-09) ;
6. revue `security` de l'unité et de la policy ; score consigné ;
7. installation/désinstallation réelles du binaire dans un conteneur propre.

---

## 7. Risques résiduels

| # | Risque | Sévérité | Propriétaire | Statut |
|---|---|---|---|---|
| P-01 | `SetTempProfile` ouvert aux utilisateurs locaux | Moyen | `linux-platform` + `security` | **rate limit + audit faits** ; polkit `.temp-profile` à ajouter |
| P-02 | Setters `Manager1` : M1-01/M1-02 avant ouverture | Moyen | `linux-platform` + `security` | **deny bus temporaire** ; actions polkit installées ; réouverture après correctifs + revue + test réel |
| P-10 | Aucune action polkit n'est encore appliquée par le code | Élevé (régression) | `linux-platform` | **déclaré, non implémenté** ; blocage jalon 2 (R-04) |
| P-03 | Ancienne politique D-Bus permissive si TCC installé hors paquet | Faible | admin système | documenté, détection au postinst, suppression manuelle |
| P-04 | `ProtectSystem=strict` d'une unité utilisateur échoue si userns restreints | Moyen | `linux-platform` | repli documenté (`MIGRATION.md` §6) |
| P-05 | `DevicePolicy=closed` + `PrivateDevices=no` non validé sur tous les modèles | Moyen | `linux-platform` + `security` | à tester sur laptop TUXEDO (Clevo/Uniwill) |
| P-13 | V4L2 : `char-video4linux` couvre tous les nœuds vidéo, et l'écriture UVC de session n'est pas validée sur caméra réelle (V-17) | Faible | `linux-platform` + `security` | `tccd` en lecture seule ; écriture via `tccd-user` (ACL `uaccess`/groupe `video`, allowlist, rate limit, `Busy` si occupée) ; test caméra à faire |
| P-14 | Écriture UVC depuis la session : l'utilisateur peut modifier les contrôles de sa propre caméra ; une autre application peut la détenir | Faible | `linux-platform` | service `User1.SetUvcControl` livré et testé, pas de polkit (bus de session), allowlist ; écriture réelle non testée |
| P-06 | `DeviceAllow=/dev/tuxedo_io` dépend du chargement du module | Faible (disponibilité) | `linux-platform` | `Wants=`/`After=modprobe@` ajouté, **non validé** |
| P-07 | `SystemCallFilter` absent | Faible | `linux-platform` | à activer après validation `ioctl` |
| P-08 | Aucun profil AppArmor/SELinux | Faible | `security` | backlog |
| P-09 | Pas de veille/reprise gérée par `tccd` | Moyen (disponibilité) | `linux-platform` | `PrepareForSleep` à implémenter ; ancienne unité retirée |
| P-11 | A1-03 : `Ping`/`GetVersion`/`GetContractVersion` sans polkit peuvent maintenir l'admin root jusqu'à `RuntimeMaxSec=5min` (activité touchée avant autorisation) | Faible | `linux-platform` | **A1-06/A1-11 fermés côté policy** ; correction runtime à faire, durée bornée par systemd |
| P2-R4 | `ProtectKernelTunables=no` sur Admin.Hw : le processus root peut écrire d'autres fichiers `/sys` | Faible | `linux-platform` + `security` | `ReadOnlyPaths` étendus ; `/proc/sys` masqué par `ProcSubset=pid` ; code sans chemin, surface scalaire ; revue avant activation |
| P2-R11 | `AdminHw1.GetTdpValues` sans polkit : un utilisateur local peut activer le service root en boucle | Faible (dispo) | `linux-platform` | rate limit 2/s, `StartLimitBurst=20/60`, sortie 30 s ; aucune donnée sensible |
| P2-R12 | `DeviceAllow=/dev/tuxedo_io` non résolu si le module n'est pas chargé | Faible (dispo) | `linux-platform` | `Wants=`/`After=modprobe@`, `NotSupported` explicite, redémarrage après chargement ; **non validé matériellement** |
| P2-R13 | Service `tccd-admin-hw` non validé sur système réel/matériel | Moyen | `linux-platform` + `security` | **aucune installation par défaut** ; unité/activation/policy testées sur bus isolé ; validation §8.5 du design requise |
| AH-01 | Trame D-Bus géante (`ai`/`s`) : désérialisation avant contrôle, service tuable par OOM | Moyen (dispo locale) | `linux-platform` + `security` | garde 64 Kio côté runtime ; `MemoryMax=256M` pour laisser zbus recevoir/rejeter la trame ; `StartLimitBurst=20/60` ; aucune écriture, pas d'escalade ; test payload géant à faire |

---

## 8. Commandes de vérification

```sh
scripts/release/validate.sh --repo
scripts/release/validate.sh --tree build/pkgroot

# Aucun /opt, aucun setuid, aucun world-writable, aucune capability
test ! -e build/pkgroot/opt
find build/pkgroot -perm /6000 -print
find build/pkgroot -perm -0002 ! -type l -print
getcap -r build/pkgroot

# Units réellement installées (après dpkg/rpm/pacman)
systemd-analyze verify /usr/lib/systemd/system/tccd.service
systemd-analyze security --offline=true /usr/lib/systemd/system/tccd.service
systemd-analyze security --offline=true /usr/lib/systemd/user/tccd-user.service
systemd-analyze security --offline=true packaging/systemd/tccd-admin-hw.service

# Admin.Hw : absent par défaut, gating strict
test ! -e build/pkgroot/usr/lib/systemd/system/tccd-admin-hw.service
TCC_WITH_ADMIN_HW=1 scripts/release/install-tree.sh /tmp/hw-root   # doit échouer sans binaire

# ProcSubset=pid masque /proc/sys : ne jamais le remettre en ReadOnlyPaths
! grep -qE '^ReadOnlyPaths=.*/proc/sys' packaging/systemd/tccd-admin-hw.service
# Validation réelle (root) : doit passer sans 226/NAMESPACE
systemctl start tccd-admin-hw.service && systemctl stop tccd-admin-hw.service

# Politique D-Bus : chargement réel dans un bus de test isolé
tmp=$(mktemp -d)
printf '<busconfig><type>system</type><listen>unix:tmpdir=%s</listen><pidfile>%s/pid</pidfile><include>%s</include></busconfig>\n' \
    "$tmp" "$tmp" /usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf > "$tmp/bus.conf"
timeout 2 dbus-daemon --config-file="$tmp/bus.conf" --nofork --print-address
rm -rf "$tmp"

# Polkit : aucun exec.*, allow_any/inactive = no, aucun _keep, aucune action LUKS
grep -nE 'policykit\.exec\.' /usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy
grep -nE '<allow_(any|inactive)>' /usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy
! grep -n 'auth_admin_keep' /usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy
! grep -n 'luks' /usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy

# udev
udevadm verify /usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules
```
