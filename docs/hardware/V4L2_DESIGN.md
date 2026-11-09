# Contrôles UVC webcam — conception

Statut : proposition de conception (aucun code livré dans cette étape).
Périmètre : `tcc-hw/src/v4l2.rs` (futur), intégration daemon/GUI, règle udev.
Documents liés : `docs/security/REVIEW_HARDWARE.md` §4, `docs/hardware/HARDWARE_MATRIX.md`,
`docs/FEATURE_PARITY.md` §4/§9, `docs/linux/PLATFORM_DESIGN.md` §2.6.
Source originale de référence : `tuxedo-control-center-master/src/cameractrls/cameractrls.py`
(MIT, dérivé de `cameractrls`), `WebcamWorker` et `webcam-settings.component.ts`.

---

## 0. Décisions proposées (résumé)

| Sujet | Décision proposée |
|---|---|
| Implémentation | Module local `tcc-hw/src/v4l2.rs`, **pas** de dépendance V4L2 externe en première intention |
| `unsafe` | Confiné à `v4l2/real.rs` (`#[allow(unsafe_code)]`), 5 ioctl, `// SAFETY:` par appel ; le reste du crate garde `#![deny(unsafe_code)]` |
| ioctl | `VIDIOC_QUERYCAP`, `VIDIOC_QUERYCTRL`, `VIDIOC_QUERYMENU`, `VIDIOC_G_CTRL`, `VIDIOC_S_CTRL` — **5**, pas 4 (voir §4.2) |
| Transport | Trait `V4l2Transport` + `RealTransport` + `FakeTransport` ; aucune caméra en CI |
| Nœud | `/dev/videoN` dérivé de `webcam::enumerate` (sysfs) ; lecture `tccd` en `O_RDONLY`, écriture `tccd-user` en `O_RDWR` ; `O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`, `fstat` char 81 + minor sysfs |
| Contrôles | `QUERYCTRL` puis **allowlist normative fermée** `cid_allowed()` (V-01) ; `S_CTRL` uniquement sur un CID de cette table ; types INTEGER/BOOLEAN/MENU/INTEGER_MENU seulement |
| CID privés | Refusés **toujours** (`id >= 0x08000000`) ; aucune table privée en phase 1–3, extension phase 4 après revue |
| Valeurs | `min ≤ value ≤ max`, multiple de `step`, menus dans les entrées énumérées ; relecture après écriture |
| Privilèges (V-03) | `tccd` **lecture seule** (`DeviceAllow=char-video4linux r`) ; écriture UVC dans **`tccd-user`** (session, non privilégié) via bus de session ; `rw` dans `tccd` uniquement en repli avec V-01+V-02+revue |
| Cycle de vie (V-04) | `try_lock` par device, état `stalled`, jamais de `lock()` bloquant dans un worker ; cooldown + reset au rebranchement |
| Budget (V-05) | ≤ 512 ioctl par cycle et par processus, menus paresseux, lecture à la demande (bail), un device en vol à la fois |
| Préréglages | Schéma v2 dans `/var/lib/tuxedo-rust-center/webcam.json` (root:root 0644), clés texte stables, migration `/etc/tcc/webcam`, application par `tccd-user` |
| udev | Règle **tag-only** conservée (aucun `RUN+=`) ; `ACTION=="add"` ; hotplug par ré-énumération 2 s en phase 1–3, netlink optionnel plus tard |
| D-Bus | Lecture dans `tccd` ; écriture volatile via le service de session `User1` (`SetUvcControl`, `GetUvcWriteSupported`) ; repli `Manager1.SetWebcamControl` **seulement** avec `CheckAuthorization` (V-02) ; persistance uniquement `Admin1` |
| Authz (V-02) | Action dédiée `com.tuxedocomputers.tccd1.webcam` (`auth_admin`, fail-closed, sans cache) si le repli `tccd` est ouvert ; policy fermée en attendant |
| Profil | Phase 3 : `webcam.preset`/`usePreset` (à ajouter dans `tcc-core`) ; avant cela, préréglage actif par device comme l'original |

---

## 1. Contexte et objectifs

### 1.1 État actuel

- `tcc-hw/src/webcam.rs` énumère `/sys/class/video4linux/videoN`, expose
  `VideoDevice` et le kill switch Clevo via `tuxedo_io`.
- Aucun contrôle UVC n'est lu ni écrit ; `WebcamData.controls_supported` vaut
  `false` (`crates/tcc-daemon/src/hardware/webcam.rs`).
- L'original appliquait les préréglages via `cameractrls.py` (Python) appelé par
  udev (`99-webcam.rules`), et pilotait l'UI via `cameractrls.py --json-list` /
  `-c clé=valeur`.
- Python, les scripts udev privilégiés et les commandes shell sont interdits
  dans la cible. Les contrôles UVC étaient listés manquants dans
  `docs/FEATURE_PARITY.md` ; la lecture (étape 1) et le backend d'écriture
  (étape 2) sont livrés, l'écriture réelle restant à valider sur caméra.

### 1.2 Objectifs

1. Lire les contrôles UVC standards d'une webcam (image, exposition, couleur,
   avancé) avec validation stricte. L'écriture est livrée dans `tcc-hw` et
   exposée par `tccd-user` (`User1.SetUvcControl`), avec allowlist et
   validation strictes.
2. Exposer ces contrôles à l'UI en mode simple et avancé, sans logique
   privilégiée dans la GUI.
3. Restaurer les préréglages par device au démarrage et au branchement.
4. Rester testable en CI sans caméra, sans `unsafe` hors des transports
   `real`/`real_write`.
5. Éviter toute nouvelle surface privilégiée non revue : pas de script udev,
   pas de contrôle arbitraire, pas d'écriture UVC dans `tccd` root.
6. **`tccd` reste en lecture seule** : pas de `DeviceAllow=char-video4linux rw`,
   aucun appel à `S_CTRL`. L'écriture est portée par `tccd-user` (session non
   privilégiée, V-03) ; les préréglages persistés restent l'étape 3.

### 1.3 Hors périmètre

- **Flux vidéo / aperçu** : Qt Multimedia natif dans la GUI, jamais `tcc-hw`.
- **Résolution et FPS** : ce sont des formats de flux (`VIDIOC_G_FMT`,
  `TRY_FMT`, `S_FMT`, `G_PARM`), pas des contrôles UVC. Ils restent gérés par
  l'aperçu, pas par `v4l2.rs`. L'original les exposait via `cameractrls` ;
  l'écart est documenté (le mode simple ne les inclut pas).
- **Kill switch** : déjà implémenté (`WebcamSwitch`). `V4L2_CID_PRIVACY` est un
  contrôle UVC distinct, exposé seulement s'il existe (voir §2.3).
- **Contrôles privés** (Kiyo Pro, Logitech, ...) : phase 4, table statique.
- **Extension de codec/JPEG** (`V4L2_CID_CODEC_BASE`, `JPEG_CLASS`), sans usage
  webcam courant.

---

## 1.4 État d'implémentation — étape 1 (lecture seule)

L'étape 1 est implémentée dans `crates/tcc-hw/src/v4l2/` :

| Fichier | Rôle |
|---|---|
| `abi.rs` | constantes ioctl, structures `#[repr(C)]`, static asserts tailles/offsets, flags |
| `control.rs` | allowlist normative des 15 CIDs, types exposés, normalisation des noms (MIT), catégories |
| `device.rs` | validation du nœud (`fstat`), identité USB, parsing `PRODUCT` |
| `fake.rs` | `FakeReader`/`FakeControl` (aucun `/dev`) |
| `real.rs` | transport **lecture** (`unsafe`) : `QUERYCAP`, `QUERYCTRL`, `QUERYMENU`, `G_CTRL` |
| `mod.rs` | `UvcReaderController`, budget, `EINTR`, allowlist, menus |

Points vérifiés par les tests et la garde statique :

- **4 ioctl de lecture** dans `real.rs` (aucun `S_CTRL` dans ce transport),
  aucun `REQBUFS`/`QBUF`/`STREAMON`/`mmap`
  (`tests/v4l2_unsafe_guard.rs`) ;
- ouverture `O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`, `fstat` caractère
  major 81, minor sysfs, uid root, `device_caps` autoritaire. Le bit
  d'écriture monde (`0o002`) est **journalisé, pas refusé** : il est courant
  (`root:video 0666` + ACL `uaccess`, `crwxrwxrwx+`) et inoffensif pour un
  descripteur `O_RDONLY` (voir §4.1) ;
- allowlist fermée : les contrôles hors liste (dont privés et
  `TEST_PATTERN`) ne sont **jamais** exposés ;
- budget 512 ioctl/cycle, 16 devices, 256 contrôles, 64 menus ; `truncated`
  remonté ;
- menus lus uniquement lors de la demande, jamais au tick de 2 s ;
- `EINTR` rejoué 3 fois au maximum ; `EBUSY`/`ENODEV` remontés sans panic.

Intégration daemon (`crates/tcc-daemon/src/hardware/webcam.rs`) :

- `WebcamData` expose `controlsSupported`, `controlsTruncated` et, par device,
  `identity`, `busInfo`, `controls`, `controlsError` ;
- la lecture est déclenchée par le **bail de collecte** (même drapeau que les
  capteurs, armé par `GetHardwareSnapshotJSON`/`SetSensorDataCollectionStatus`)
  et mise en cache 30 s ; sans client, aucun ioctl de contrôle ;
- `dry_run` (`--check`, `--dev-session`) n'ouvre aucun device ;
- `UvcControlData` n'expose **pas** l'`id` noyau : uniquement la clé stable,
  les bornes, la valeur et les menus.

Limites de l'étape 1 :

- **Chemin de lecture sans écriture** : `real.rs` et `tccd` n'appellent
  jamais `S_CTRL`. L'écriture est isolée dans `write.rs`/`real_write.rs`
  (étape 2) et consommée par `tccd-user` via `User1.SetUvcControl`.
- **Aucun préréglage** : le schéma v2 et `Admin1` sont en étape 3.
- **Packaging** : `tccd` doit recevoir `DeviceAllow=char-video4linux r` ; sans
  cette autorisation, l'ouverture échoue en `EACCES`, le device est
  `available: false` et l'UI affiche une erreur de permission. Le fichier
  `packaging/systemd/tccd.service` n'est pas modifié par cette étape.
- Les contrôles ne sont pas relus à chaque tick : si la page webcam est
  ouverte sans que le bail capteurs soit armé, les contrôles restent vides
  jusqu'à la prochaine demande de snapshot. Un bail dédié webcam pourra être
  ajouté par `linux-platform`.
- Test réel opt-in livré (`tests/real_v4l2.rs`, `#[ignore]`) ; voir §9.4.
- **Validation matérielle** (webcam FHD `2b7e:c858`, `/dev/video0`
  `crwxrwxrwx+ root:video`, 2026-11-06) : `read_all()` expose 11 contrôles
  (luminosité, contraste, saturation, teinte, gamma, gain, fréquence de ligne,
  balance des blancs, netteté, compensation de contre-jour, exposition
  absolue), `controlsSupported: true` dans le worker, `errors: []`. Le nœud
  metadata (`video1`) et la caméra IR (`video2`, 0 contrôle de l'allowlist)
  sont correctement gérés.
- Cycle de vie V-04 : en lecture seule, un seul worker sérialise les appels
  (`spawn_blocking`). Le writer d'écriture dispose de `try_lock` par device,
  d'un budget et d'un état `stalled` (étape 2). Un ioctl bloqué immobilise le
  thread appelant, pas le runtime tokio.

Tests des étapes 1–2 : `cargo test -p tcc-hw` (**265 tests unitaires + 19
tests d'intégration**, dont `fake_v4l2`, `v4l2_unsafe_guard`, les tests
`v4l2::write` et `real_v4l2` ignoré par défaut) et
`cargo test -p tcc-daemon --test hardware_workers` (**107 tests**). Aucun
test par défaut n'ouvre de caméra.


---

## 1.5 État d'implémentation — étape 2 (écriture, `tcc-hw` uniquement)

L'écriture UVC est implémentée dans `tcc-hw` et **exposée par `tccd-user`**
sur le bus de session via l'interface `User1` (contrat dans `docs/DBUS_API.md`) :

| Fichier | Rôle |
|---|---|
| `write.rs` | `UvcWriter`, `UvcWriterController`, allowlist/bornes/menus, budget, `try_lock`, `stalled`, relecture |
| `real_write.rs` | **seul module avec `S_CTRL`** : `O_RDWR`, `QUERYCAP`, `QUERYCTRL`, `QUERYMENU`, `G_CTRL`, `S_CTRL` |
| `fake.rs` | `FakeWriter` (tests sans `/dev`) |
| `abi.rs` | constante `VIDIOC_S_CTRL` (`0xC008561C`, 8 octets) |

API livrée (utilisée par `tccd-user`) :

```rust
let writer = tcc_hw::v4l2::UvcWriterController::system(
    tcc_hw::sysfs::SysfsRoot::system(),
);
let report = writer.set_control("/dev/video0", "brightness", 128)?;
// report.changed, report.previous, report.value
```

Service de session :

```text
com.tuxedocomputers.tccd.User — interface User1
  GetUvcWriteSupported() -> b
  SetUvcControl(s devicePath, s controlKey, i value) -> b
```

`devicePath` est validé comme `/dev/videoN` canonique **et réellement énuméré**
par `tcc-hw` au moment de l'appel (aucun chemin arbitraire, aucun lien, nœud
metadata refusé) ; `controlKey` est une clé de l'allowlist ; `value` est un
entier borné (index pour un menu). Le retour booléen vaut `true` si la
`S_CTRL` a changé la valeur, `false` si elle était déjà courante
(`report.changed=false`, aucun `S_CTRL`). Les erreurs typées sont traduites par
`tccd-user` en erreurs D-Bus stables (`InvalidArgs`, `NotSupported`,
`OutOfRange`, `Busy`, `Stalled`, `Failed`, `LimitsExceeded`). `EACCES`/`EPERM`
(ACL `uaccess` ou groupe `video` absent) est traduit en `NotSupported`, jamais
en `Failed`.

Propriétés de sécurité :

- `O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`, `fstat` caractère major 81, minor
  sysfs, uid root, `device_caps` capture ;
- allowlist fermée des 15 CIDs ; un CID privé ou hors liste n'est jamais écrit ;
- identité stable après `QUERYCAP` : `driver`, `card` et, si dérivable du lien
  sysfs, `bus_info` sont comparés au device énuméré ; un mismatch ⇒ `Stalled`,
  aucune écriture (UVCW-03) ;
- `QUERYCTRL`/`G_CTRL` avant `S_CTRL`, validation type/min/max/step/menu,
  relecture `G_CTRL` après écriture ; si la valeur relue diffère (arrondi,
  clamp firmware), restauration **best-effort** de la valeur précédente puis
  `InvalidValue` avec l'état constaté (UVCW-04) ;
- budget [`MAX_IOCTLS_PER_WRITE` = 64], timeout 2 s, `try_lock` par device ;
- **une seule écriture en vol par processus** (porte globale `try_lock`) :
  les appels concurrents échouent immédiatement `Busy` sans consommer un
  second thread `spawn_blocking` ; un `S_CTRL` bloqué n'immobilise donc qu'un
  thread par processus. Un watchdog côté service reste à ajouter pour borner
  un pilote pathologique (le délai 2 s ne peut pas interrompre l'ioctl) ;
- `EBUSY` ⇒ `Busy`, `EIO`/`ENODEV`/déconnexion ⇒ `Stalled` (quarantaine 30 s) ;
- aucun `S_CTRL` dans `real.rs`, `tccd`, la GUI ou les helpers de lecture
  (garde statique `tests/v4l2_unsafe_guard.rs`).
- service de session non privilégié : bus de session, ACL `uaccess`/groupe
  `video`, **aucune action polkit**, **aucun `DeviceAllow` systemd** ;
  débit limité à 20 écritures/s (burst 20) par UID appelant.

Limites :

- l'écriture réelle n'a pas été testée sur une caméra physique dans cette
  campagne ; les tests couvrent le backend factice et les erreurs ;
- pas de préréglages persistés (étape 3) ; la GUI simple/avancée est en cours ;
- une autre application (aperçu, navigateur) peut détenir le device : `Busy` ;
- un firmware peut arrondir/ignorer une écriture : la relecture détecte
  l'écart (`OutOfRange`/`Failed`) ;
- une autre application (aperçu, navigateur) peut détenir le device : le
  noyau renvoie `EBUSY` ou l'écriture est ignorée ; le writer propage l'erreur ;
- `tccd-user` n'a **pas** besoin de `DeviceAllow` : l'accès est régi par l'ACL
  `uaccess`/le groupe `video` de la session. Aucun repli root.

---

## 2. Périmètre fonctionnel

### 2.1 Allowlist normative d'écriture (V-01)

`cid_allowed(cid: u32) -> bool` est une **constante fermée** du module
`control.rs`, revue par `security`. Elle est appliquée dans `set` **avant**
toute résolution de menu ou d'index. Un CID énuméré mais absent de cette table
est **refusé en écriture** (`NotSupported`), jamais transmis à `S_CTRL`.
L'énumération `QUERYCTRL` n'est **pas** une autorisation : elle ne sert qu'à
vérifier la présence, le type et les bornes.

Liste initiale (phase 1–3), volontairement minimale :

| CID | Type | Clé stable |
|---|---|---|
| `V4L2_CID_BRIGHTNESS` | INTEGER | `brightness` |
| `V4L2_CID_CONTRAST` | INTEGER | `contrast` |
| `V4L2_CID_SATURATION` | INTEGER | `saturation` |
| `V4L2_CID_HUE` | INTEGER | `hue` |
| `V4L2_CID_GAMMA` | INTEGER | `gamma` |
| `V4L2_CID_WHITE_BALANCE_TEMPERATURE` | INTEGER | `white_balance_temperature` |
| `V4L2_CID_EXPOSURE_ABSOLUTE` | INTEGER | `exposure_absolute` |
| `V4L2_CID_EXPOSURE` | INTEGER | `exposure` |
| `V4L2_CID_GAIN` | INTEGER | `gain` |
| `V4L2_CID_SHARPNESS` | INTEGER | `sharpness` |
| `V4L2_CID_FOCUS_ABSOLUTE` | INTEGER | `focus_absolute` |
| `V4L2_CID_FOCUS_AUTO` | BOOLEAN | `focus_auto` |
| `V4L2_CID_POWER_LINE_FREQUENCY` | MENU | `power_line_frequency` |
| `V4L2_CID_BACKLIGHT_COMPENSATION` | INTEGER/BOOLEAN | `backlight_compensation` |
| `V4L2_CID_ZOOM_ABSOLUTE` | INTEGER | `zoom_absolute` |

Conséquences assumées :

- `V4L2_CID_EXPOSURE_AUTO`, `V4L2_CID_AUTO_WHITE_BALANCE`,
  `V4L2_CID_WHITE_BALANCE_TEMPERATURE_AUTO` (si exposé), `AUTOGAIN`,
  `AUTO_EXPOSURE_PRIORITY`, etc. ne sont **pas** inscriptibles en phase 1–3 :
  ils restent visibles en lecture seule. Écrire `exposure_absolute` ou
  `white_balance_temperature` alors que l'automatisme est actif peut donc être
  ignoré par le pilote ; le design l'expose comme une limite, et l'UI affiche
  l'état « auto » sans le modifier. Leur ajout à la table est un candidat de
  phase 2, **après revue sécurité dédiée** (ils changent le couplage
  auto/manuel).
- `V4L2_CID_PRIVACY`, `AUTO_FOCUS_STATUS`, `CAMERA_ORIENTATION`,
  `CAMERA_SENSOR_ROTATION` et tous les CID privés ne sont **jamais** dans la
  table d'écriture. `PRIVACY` reste exposé en lecture si le device l'annonce ;
  le kill switch reste `tuxedo_io`.
- Toute autre valeur retournée par `QUERYCTRL` (y compris `TEST_PATTERN`,
  `DIGITAL_GAIN`, `DV_TX_*`, `MPEG_*`, ...) est refusée en écriture même si le
  pilote l'énumère.

### 2.2 Surface exposée en lecture

Tous les contrôles énumérés ci-dessous sont **lisibles** (s'ils existent), mais
seuls ceux de la table §2.1 sont inscriptibles. Le tableau sert à l'UI
(catégories, mode simple/avancé) et à la validation des types.

| Catégorie UI | CIDs exposés (noms noyau) |
|---|---|
| Basic — Crop | `ZOOM_ABSOLUTE`, `ZOOM_RELATIVE`, `ZOOM_CONTINUOUS`, `PAN_ABSOLUTE`, `PAN_RELATIVE`, `TILT_ABSOLUTE`, `TILT_RELATIVE` |
| Basic — Focus | `FOCUS_AUTO`, `FOCUS_ABSOLUTE`, `FOCUS_RELATIVE`, `AUTO_FOCUS_RANGE`, `AUTO_FOCUS_STATUS` (lecture) |
| Exposure | `EXPOSURE_AUTO`, `EXPOSURE_ABSOLUTE`, `EXPOSURE_AUTO_PRIORITY`, `AUTOGAIN`, `EXPOSURE`, `EXPOSURE_METERING`, `AUTO_EXPOSURE_BIAS`, `GAIN`, `CHROMA_AGC`, `CHROMA_GAIN`, `IRIS_ABSOLUTE`, `IRIS_RELATIVE`, `IMAGE_STABILIZATION`, `SCENE_MODE` |
| Exposure — ISO | `ISO_SENSITIVITY`, `ISO_SENSITIVITY_AUTO` |
| Exposure — Dynamic Range | `BACKLIGHT_COMPENSATION`, `WIDE_DYNAMIC_RANGE` |
| Color — Balance | `AUTO_WHITE_BALANCE`, `AUTO_N_PRESET_WHITE_BALANCE`, `WHITE_BALANCE_TEMPERATURE`, `RED_BALANCE`, `BLUE_BALANCE` |
| Color | `AUTOBRIGHTNESS`, `BRIGHTNESS`, `CONTRAST`, `SATURATION`, `SHARPNESS`, `HUE_AUTO`, `HUE`, `GAMMA`, `COLOR_KILLER`, `BAND_STOP_FILTER`, `BG_COLOR` |
| Color — Effects | `COLORFX`, `COLORFX_CBCR`, `COLORFX_RGB` |
| Advanced — Power line | `POWER_LINE_FREQUENCY` |
| Advanced — Privacy | `PRIVACY` |
| Advanced — Rotate/Flip | `ROTATE`, `HFLIP`, `VFLIP` |
| Métadonnées (lecture) | `CAMERA_ORIENTATION`, `CAMERA_SENSOR_ROTATION` |

Règles :

- `V4L2_CID_DO_WHITE_BALANCE`, `PAN_RESET`, `TILT_RESET`, `AUTO_FOCUS_START/STOP`
  sont des **boutons** (`CTRL_TYPE_BUTTON`) : jamais exposés en écriture (une
  écriture déclenche une action). L'original les ignorait à l'énumération.
- `3A_LOCK` est un `BITMASK` : ignoré en phase 1–3 (serait un booléen multiple).
- `PAN_SPEED`/`TILT_SPEED` sont des contrôles **transitoires** (remis à zéro
  après le geste) : phase 4, avec une API dédiée « impulsion » ; pas de valeur
  persistée et hors allowlist d'écriture.
- Les CID privés (`>= 0x08000000`) sont **refusés en écriture dans toutes les
  phases** ; une éventuelle table `(vid,pid)` phase 4 devra être ajoutée à
  `cid_allowed` et revue. En lecture, ils sont ignorés (non exposés).
- Un CID absent du device n'est jamais synthétisé : l'UI n'affiche que ce que
  `QUERYCTRL` retourne.

### 2.3 Types de contrôles

| Type noyau | Valeur Rust | Exposition |
|---|---|---|
| `INTEGER` | `UvcValue::Integer(i64)` | slider/stepper, borné `min/max/step` |
| `BOOLEAN` | `UvcValue::Boolean(bool)` | interrupteur |
| `MENU` | `UvcValue::Menu { index, key, label }` | liste déroulante |
| `INTEGER_MENU` | `UvcValue::Menu { index, key, label }` | idem ; `key` = valeur entière 64 bits, `index` = index d'écriture |
| `BUTTON` | — | **ignoré** |
| `CTRL_CLASS`, `STRING`, `BITMASK`, `INTEGER64`, `U8/U16/U32`, `AREA`, ... | — | ignoré en phase 1–3 |

Un INTEGER dont `min=0`, `max=1`, `step=1` est normalisé en `Boolean` (comme
`cameractrls.py`), pour l'UI.

### 2.4 Mode simple / avancé

- **Simple** : `brightness`, `contrast`, `saturation` (les trois présents,
  tous inscriptibles dans l'allowlist §2.1). Les indicateurs
  `exposure_auto`/`white_balance_automatic` peuvent être affichés en **lecture
  seule** ; ils ne sont pas inscriptibles en phase 1–3. La résolution et les
  FPS restent dans l'aperçu.
  L'original utilisait `brightness`, `contrast`, `resolution` ; « resolution »
  n'étant pas un contrôle UVC, l'écart est explicite.
- **Avancé** : toutes les catégories du tableau §2.1, contrôles inactifs grisés
  (`INACTIVE`), lecture seule affichée sans écriture (`READ_ONLY`).
- Un contrôle `WRITE_ONLY` n'est pas affiché en lecture (valeur inconnue) ; il
  reste pilotable si l'utilisateur le modifie.
- `V4L2_CTRL_FLAG_UPDATE` : après écriture, l'énumération est relue avant
  l'opération suivante (les contrôles dépendants ont pu changer).

### 2.5 Préréglages

- Un préréglage est un ensemble de `clé -> valeur` pour un device identifié par
  `(vendor, product, serial?)` (voir §6).
- Un seul préréglage actif par device (comportement original).
- Application : au démarrage du daemon, au branchement, et (phase 3) à
  l'activation d'un profil qui porte un préréglage.
- Échec partiel : les contrôles valides sont appliqués, les autres sont
  rapportés ; le préréglage n'est jamais appliqué « à moitié silencieusement ».
- Si la caméra est occupée (`EBUSY`), nouvelle tentative avec back-off borné ;
  l'état expose `pending` et la dernière erreur.

### 2.6 Application à l'activation du profil

L'original applique le **kill switch** par profil (`WebcamWorker`) et les
préréglages au branchement (udev), pas à l'activation du profil.
Proposition phase 3, à valider par `rust-core`/`qt-ui` :

```rust
// tcc-core::WebcamProfile (extension proposée, hors périmètre actuel)
pub struct WebcamProfile {
    pub status: bool,
    pub use_status: bool,
    pub preset: Option<String>,   // nom du préréglage
    pub use_preset: bool,
}
```

Ordre d'application à l'activation : kill switch d'abord (matériel), puis
préréglage si `use_preset`. Sans ce champ, le préréglage actif par device reste
appliqué au démarrage/branchement (parité avec l'original).

---

## 3. Architecture

### 3.1 Arborescence proposée

```text
crates/tcc-hw/src/
├── webcam.rs           # existant : sysfs (enumeration), kill switch — NON MODIFIÉ
└── v4l2/
    ├── mod.rs          # API publique, contrôleur, politiques (safe)
    ├── abi.rs          # constantes ioctl, structures #[repr(C)], tailles (safe)
    ├── control.rs      # allowlist CID, types, conversion nom->clé, validation (safe)
    ├── device.rs       # validation du nœud, identité, ouverture (safe + real)
    ├── preset.rs       # schéma v2, migration, application ordonnée (safe)
    ├── fake.rs         # FakeTransport + fixtures (cfg(test) ou feature test)
    └── real.rs         # SEUL module avec unsafe : 5 ioctl
```

- `v4l2/mod.rs` consomme `crate::webcam::VideoDevice` (lecture sysfs) et ne
  duplique pas l'énumération.
- `fake.rs` est compilé en test uniquement (`#[cfg(test)]`) ou derrière une
  feature `test-fakes` si `qa-integration` en a besoin hors crate.
- Aucun lien avec `tcc-core`/`tcc-ipc` : types matériels autonomes, conversions
  côté daemon.

### 3.2 Traits de transport et séparation lecture/écriture (V-06)

```rust
pub enum OpenMode { Read, Write } // O_RDONLY vs O_RDWR

pub trait V4l2Reader: Send + Sync {
    fn querycap(&self) -> Result<Capabilities>;
    fn queryctrl(&self, id: u32) -> Result<QueryCtrl>;
    fn querymenu(&self, id: u32, index: u32) -> Result<MenuEntry>;
    fn get_ctrl(&self, id: u32) -> Result<RawValue>;
}

pub trait V4l2Writer: V4l2Reader {
    fn set_ctrl(&self, id: u32, value: i32) -> Result<RawValue>;
}
```

- `RealTransport` possède un `File` (fd) ; `OpenMode::Read` fait
  `O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`, `OpenMode::Write` fait `O_RDWR`.
- Le type **lecteur** ne compile pas `set_ctrl` : une erreur de câblage ne peut
  pas transformer `tccd` en écrivain.
- `FakeTransport` implémente les deux traits et enregistre les écritures ;
  il ne touche ni `/dev` ni sysfs.
- `UvcReader` (contrôleur lecture) et `UvcWriter` (contrôleur écriture) portent
  la politique : allowlist normative, validation, ordre, relecture. Le
  transport ne sait rien des CIDs.
- **Cycle de vie (V-04)** : un `try_lock` par device, jamais `lock()`. Si le
  verrou est déjà pris (lot en cours ou thread bloqué), l'appel retourne
  immédiatement `Busy`/`Stalled` ; il ne consomme pas un thread
  `spawn_blocking` supplémentaire. L'état `stalled` est porté par un
  `AtomicBool` par device, remis à zéro au débranchement/rebranchement.
- Les fds ne sont pas conservés à vie : ouverture par lot
  (`enumerate`, `read_all`, `apply_preset`) puis fermeture (voir §4.7).
- Budget global (V-05) : un seul device en vol à la fois par processus, compteur
  d'ioctl partagé par cycle (voir §4.7).

### 3.3 Décision `unsafe`

- Le crate garde `#![deny(unsafe_code)]` (déjà dans `lib.rs`). Il **ne peut pas**
  passer à `#![forbid]` tant que `tuxedo_io::real`, `v4l2::real` et
  `v4l2::real_write` existent : `forbid` interdit toute dérogation, y compris
  dans les sous-modules.
- `v4l2/real.rs` **et** `v4l2/real_write.rs` portent `#![allow(unsafe_code)]`
  et sont les seuls fichiers avec `libc::ioctl` (un helper par transport).
  Chaque appel a un commentaire `// SAFETY:` qui cite le contrat noyau (taille
  de structure, direction `_IOR`/`_IOWR`, validité du fd).
- Pour obtenir la garantie « `forbid` ailleurs », chaque module sûr de `v4l2/`
  (`abi.rs`, `control.rs`, `device.rs`, `write.rs`, `fake.rs`) déclare
  individuellement `#![forbid(unsafe_code)]`. `v4l2/mod.rs` ne le peut pas
  (il déclare `real`/`real_write`) : il hérite du `deny` du crate.
- Un test statique (comme `no_admin_reachability.rs` côté daemon) vérifie que
  `v4l2/` ne contient `unsafe` que dans `real.rs` et `real_write.rs`, qu'il y a
  exactement un helper `libc::ioctl` par transport, et que `S_CTRL` n'apparaît
  que dans `abi.rs`/`real_write.rs`.
- Aucun `unsafe` dans `tcc-daemon`, GUI, CLI ou agent de session.

### 3.4 Injection et testabilité

- `UvcReaderController::system(root)` ou `with_factory(root, ReaderFactory)`.
- `UvcWriterController::system(root)` ou `with_factory(root, WriterFactory)` ;
  `WriterFactory = Arc<dyn Fn(&VideoDevice) -> Result<Box<dyn UvcWriter>>>`.
- Production : `UvcReaderController::system` pour `tccd`,
  `UvcWriterController::system` pour `tccd-user` (une **seule instance**
  partagée par le service `User1`). `tccd` ne construit que le lecteur.
- Tests : `FakeReader`/`FakeWriter` fournis avec des contrôles simulés, aucun
  privilège.
- La validation de nœud est extraite en fonction pure sur une structure
  `NodeMetadata { file_type, major, minor, gid, mode }`, testable sans
  `/dev` réel (voir §9).

### 3.5 Flux et schéma de privilèges (V-03)

Décision retenue : **lecture dans `tccd`, écriture dans `tccd-user`**.

```text
tccd (root, lecture seule)                         tccd-user (session, utilisateur)
  └─ hardware::webcam (2 s / notif)                  └─ webcam::session (bus de session)
       └─ UvcReaderController (spawn_blocking)             └─ UvcWriterController
            ├─ webcam::enumerate (sysfs)                      ├─ webcam::enumerate (sysfs)
            ├─ O_RDONLY /dev/videoN                           ├─ O_RDWR /dev/videoN (ACL/group video)
            ├─ QUERYCAP/QUERYCTRL/QUERYMENU/G_CTRL            ├─ QUERY… + S_CTRL (allowlist §2.1)
            └─ WebcamData.controls (cache/à la demande)       └─ set_control (presets étape 3)
       └─ GetWebcamDataJSON (système)                         └─ User1.SetUvcControl /
                                                                 GetUvcWriteSupported
                                                                 (bus de session, même UID)
GUI ── lecture : bus système (tccd) ─────────────────────────────┘
    └─ écriture : bus de session (tccd-user, même UID) ──────────┘
```

- `tccd` **n'ouvre jamais** `/dev/videoN` en écriture et n'appelle jamais
  `S_CTRL` ; il ne peut donc pas modifier la caméra d'un utilisateur.
- `tccd-user` tourne dans la session, possède le device via l'ACL `uaccess`
  (logind) ou le groupe `video`, et n'a aucun privilège root.
- Si `tccd-user` n'est pas lancé (TTY, session absente, service arrêté),
  `GetUvcWriteSupported` retourne `false` (ou le service est absent) ; **aucun
  repli root implicite**.
- Les préréglages persistés ne sont **pas implémentés** (étape 3) ; seule
  l'écriture volatile `SetUvcControl` est livrée.
- Deux sessions / bascule de siège : chaque `tccd-user` tente `try_lock` ; le
  premier écrivain gagne, les autres voient `Busy`. Aucun daemon root ne
  contourne ce comportement.
- Repli documenté (non retenu par défaut) : `tccd` garde `rw` + `S_CTRL`
  **seulement** si V-01 et V-02 sont implémentés et revus. Le basculement est
  un changement de `DeviceAllow` et de policy, pas une option runtime.

Aucun `ioctl` dans `hardware/webcam.rs` : les contrôleurs sont appelés dans
`spawn_blocking`, comme les autres domaines matériels.

---

## 4. Sécurité

### 4.1 Ouverture et validation du nœud

1. Le nom `videoN` provient **exclusivement** de
   `webcam::enumerate` (`valid_video_name`, N numérique), jamais d'un client
   D-Bus.
2. Chemin construit `format!("/dev/video{N}")`, jamais fourni par l'appelant.
3. Mode d'ouverture explicite (V-06) :
   - `tccd` (lecture) : `O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC` ;
   - `tccd-user` (écriture) : `O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`.
   `O_NOFOLLOW` refuse un lien symbolique (les chemins `/dev/v4l/by-id` ne sont
   jamais utilisés).
4. `File::metadata()` (`fstat` sur le fd ouvert, pas de TOCTOU) **pour
   `tccd`** :
   - type = périphérique caractère ;
   - `major == 81` (`VIDEO_MAJOR`) ;
   - `minor` égal au minor lu dans
     `/sys/class/video4linux/videoN/dev` (`81:minor`) ;
   - `uid == 0` ;
   - **le bit d'écriture monde (`0o002`) n'est pas un critère de refus en
     lecture seule**. `O_RDONLY` ne confère aucun droit d'écriture au
     descripteur et le mode `root:video 0666` avec ACL `uaccess`
     (`crwxrwxrwx+`) est le cas standard de nombreuses webcams FHD. Le mode
     et le `gid` sont journalisés en debug (V-09) : un contrôle strict dépendrait de
     NSS/LDAP et casserait des distributions ;
   - **chemin d'écriture** (`real_write.rs`, `tccd-user`) : même validation
     `fstat` (caractère, major 81, minor sysfs, uid root), ouverture
     `O_RDWR` ; le noyau applique l'ACL `uaccess`/le groupe `video` à
     l'ouverture. Aucune vérification d'ACL en espace utilisateur n'est
     dupliquée. Cette règle remplace l'exigence `(mode & 0o002) == 0` de V-09
     pour la lecture seule ; `security` doit mettre à jour
     `docs/security/REVIEW_V4L2.md` (finding V-09 et test n°7) ;
   - refus si le device sysfs n'existe plus entre l'énumération et l'ouverture.
5. `tccd-user` ouvre en tant qu'utilisateur de la session : les mêmes
   contrôles de nœud sont appliqués (caractère, major 81, major+minor sysfs,
   **nœud root-owned**). Ce n'est pas l'UID de l'appelant qui est exigé root :
   l'accès en écriture est régi par l'ACL `uaccess` ou le groupe `video` à
   l'ouverture `O_RDWR`. `EACCES` ⇒ `unsupported`, jamais de repli root.
6. `VIDIOC_QUERYCAP` après ouverture, avec `device_caps` **autoritaire** quand
   `V4L2_CAP_DEVICE_CAPS` est posé (V-07) :
   `caps = if capabilities & DEVICE_CAPS != 0 { device_caps } else { capabilities }`.
   Exiger `V4L2_CAP_VIDEO_CAPTURE`, refuser `V4L2_CAP_META_CAPTURE`, refuser
   `index > 0` (metadata) et un nœud sans capture (codec-only).
7. Toutes les chaînes venant du device (`card`, `driver`, `bus_info`, noms de
   contrôles/menus) sont sanitisées avant stockage/publication (V-14) :
   troncature 32/128 octets, NUL final retiré, caractères de contrôle supprimés,
   UTF-8 lossy. Jamais de chaîne device dans un chemin ou du HTML.
8. Aucun descripteur n'est transmis à un autre processus, aucun `mmap`, aucun
   `read`/`write` de flux.

### 4.2 Allowlist d'ioctl (ABI figée)

`REVIEW_V4L2.md` §1 et §4 **accepte les 5 ioctls** : `VIDIOC_QUERYMENU` est
indispensable aux menus (`exposure_auto`, balance des blancs) et ne crée aucun
flux. `REVIEW_HARDWARE.md` §4 (« 4 ioctls ») doit être amendé par une note
datée, sans réécrire l'historique (action `security`, V-15).

| Constante | Requête (x86_64/aarch64) | Structure | Taille |
|---|---|---|---|
| `VIDIOC_QUERYCAP` | `0x80685600` | `v4l2_capability` | 104 |
| `VIDIOC_G_CTRL` | `0xC008561B` | `v4l2_control` | 8 |
| `VIDIOC_S_CTRL` | `0xC008561C` | `v4l2_control` | 8 |
| `VIDIOC_QUERYCTRL` | `0xC0445624` | `v4l2_queryctrl` | 68 |
| `VIDIOC_QUERYMENU` | `0xC02C5625` | `v4l2_querymenu` | 44 |

Structures (`#[repr(C)]`, champs `u32`/`i32`/`[u8; 32]`, aucune union Rust) :

```rust
struct v4l2_capability { driver: [u8;16], card: [u8;32], bus_info: [u8;32],
                         version: u32, capabilities: u32, device_caps: u32,
                         reserved: [u32;3] }
struct v4l2_control { id: u32, value: i32 }
struct v4l2_queryctrl { id: u32, ctrl_type: u32, name: [u8;32],
                        minimum: i32, maximum: i32, step: i32,
                        default_value: i32, flags: u32, reserved: [u32;2] }
struct v4l2_querymenu { id: u32, index: u32, payload: [u8;32], reserved: u32 }
```

- `v4l2_querymenu` est représenté **sans union** : `payload` contient le nom
  (32 octets) pour `MENU`, ou la valeur `i64` en little-endian/native pour
  `INTEGER_MENU`. La lecture se fait par `from_ne_bytes`, sans `unsafe`.
- Tests d'ABI (V-16) : tailles 104/8/68/44, offsets `queryctrl` (`name` 8,
  `minimum` 40, `maximum` 44, `step` 48, `default_value` 52, `flags` 56) **et**
  `querymenu` (`payload` 8, `reserved` 40, `align_of == 4`, `reserved` mis à 0
  avant l'appel). Constantes ioctl exactes, sur toutes les architectures 64 bits
  supportées. Flags testés (valeurs du header noyau, la revue V-16 inversait
  `GRABBED`/`EXECUTE_ON_WRITE`) : `DISABLED=0x0001`, `GRABBED=0x0002`,
  `READ_ONLY=0x0004`, `UPDATE=0x0008`, `INACTIVE=0x0010`, `WRITE_ONLY=0x0040`,
  `HAS_PAYLOAD=0x0100`, `EXECUTE_ON_WRITE=0x0200`.
- `EINTR` : boucle de reprise **bornée** (3 tentatives, 10 ms entre) autour de
  chaque ioctl ; `EINTR` n'est jamais interprété comme une fin d'énumération.
- L'encodage `_IOC` est un `const fn` sûr (aucun `unsafe`), testé par rapport
  aux valeurs du header noyau.

### 4.3 Allowlist normative et résolution (V-01, V-08)

- **`cid_allowed(cid)` est la barrière d'écriture** (table §2.1, constante
  fermée revue par `security`). Elle est appelée dans `set` **avant** toute
  résolution de menu/index, avant toute lecture de valeur et avant `S_CTRL`.
  Un CID énuméré mais absent de la table ⇒ `PolicyDenied`, jamais d'ioctl.
- Énumération avec `V4L2_CTRL_FLAG_NEXT_CTRL` **seul** (pas de `NEXT_COMPOUND`,
  aucun type composé supporté), puis `id = queried_id | NEXT_CTRL` jusqu'à
  `EINVAL`. Les flags `NEXT_CTRL`/`NEXT_COMPOUND` sont **masqués** dans l'`id`
  retourné avant stockage et avant `S_CTRL` (V-08).
- Bornes : 256 contrôles maximum, nom tronqué à 32 octets, menu 64 entrées
  maximum (chargées paresseusement, §4.5), 16 devices maximum.
- Sont ignorés : boutons, classes, strings, bitmasks, integer64, u8/u16/u32,
  area, contrôles `DISABLED`/`GRABBED`, CID privés. Un contrôle
  `EXECUTE_ON_WRITE` ou `HAS_PAYLOAD` est refusé en écriture (action implicite
  ou payload non supporté).
- `S_CTRL` exige **cumulativement** : présence dans l'énumération fraîche,
  `cid_allowed(id)`, type supporté, `min/max/step` valides, absence de
  `READ_ONLY`/`INACTIVE`/`DISABLED`/`GRABBED`. Aucune valeur numérique de CID
  ne vient de D-Bus. Le daemon manipule une **clé texte** (`brightness`,
  `exposure_auto`, ...) ; le contrôleur résout la clé vers un CID après
  énumération et vérifie l'identité du device.
- Les clés sont normalisées comme l'original
  (`cameractrls.py::to_text_id`) : minuscules, espace/`-` → `_`, suppression de
  `, & ( ) . /`, `__` → `_`. Exemples : `Auto Exposure` → `auto_exposure`,
  `White Balance Temperature, Auto` → `white_balance_temperature_auto`.
  Une collision de clés est désambiguïsée par `_<id>` et journalisée.
- `QUERYCTRL` est aussi utilisé pour rafraîchir un contrôle unique
  (`INACTIVE`/`READ_ONLY` peuvent changer) ; pas d'accès direct par CID.

### 4.4 Validation des valeurs

Pour un contrôle énuméré :

- type INTEGER : valeur entière `i64`, `min ≤ v ≤ max`, `step > 0` ⇒
  `(v - min) % step == 0` (sinon refus, pas d'arrondi silencieux) ;
- type MENU : la valeur est un **index** `min ≤ v ≤ max`, validé par
  `QUERYMENU(cid, v)` ;
- type INTEGER_MENU : `min..=max` est l'intervalle des **index**, pas des
  valeurs ; le contrôleur parcourt les index et compare `QUERYMENU.value` à la
  valeur demandée, refuse si aucune entrée ne correspond (UVCW-05) ; la valeur
  doit tenir dans `i32` pour `S_CTRL` ;
- BOOLEAN : `0`/`1` uniquement ;
- refus si `READ_ONLY`, `INACTIVE`, `DISABLED`, `GRABBED` ;
- `S_CTRL` peut modifier la valeur (arrondi pilote) : la valeur retournée par
  l'ioctl est relue ; si elle diffère de la demande, l'ancienne valeur est
  restaurée en best-effort puis `InvalidValue` est retournée avec l'état ;
- après écriture d'un contrôle `UPDATE`, l'énumération est relue avant de
  poursuivre (les contrôles dépendants peuvent changer de bornes/état) ;
- aucun `VIDIOC_S_EXT_CTRLS`, aucun contrôle composé.

### 4.5 Menus

- `QUERYMENU` pour `minimum..=maximum`, avec cap 64 itérations et saut des
  index invalides (`EINVAL`). Chargement **paresseux** (V-05) : les libellés ne
  sont demandés que lorsqu'un client demande les contrôles du device ; jamais
  lors d'un tick de disponibilité, jamais pour un device sans client.
- Nom du menu : 32 octets, NUL final, UTF-8 lossy, caractères de contrôle
  supprimés ; clé stable normalisée ; libellé affiché = nom noyau (i18n plus
  tard pour les CIDs connus).
- `INTEGER_MENU` : la valeur noyau est l'`i64` du payload ; l'index de l'API
  V4L2 reste l'index de menu.

### 4.6 Contrôles privés

- `V4L2_CID_PRIVATE_BASE` (0x08000000) et au-delà : **refusés partout**
  (`cid_allowed` retourne toujours `false`), y compris en lecture publique.
- Justification : sémantique inconnue, valeurs et effets non documentés ;
  écrire un CID privé depuis un daemon root est une surface d'attaque.
- Phase 4 : une table statique `(vendor, product) -> &[u32]` pourra être
  ajoutée à `cid_allowed` (contrôles Kiyo Pro/Logitech connus), chaque entrée
  revue ; sans table, refus.
- Les préréglages legacy contenant des clés privées sont conservés mais ignorés
  à l'application, avec un log unique par device.

### 4.7 Cycle de vie, blocage et budget (V-04, V-05)

**Verrou par device**

- Chaque device possède un `Mutex<()>` de session, acquis par `try_lock`
  **uniquement**. Aucun `lock()` bloquant n'est appelé depuis un worker, un
  handler D-Bus ou `spawn_blocking`.
- Si le verrou est déjà pris (lot en cours, ou thread bloqué dans un ioctl) :
  retour immédiat `Busy` ou `Stalled`, **aucun** thread supplémentaire
  consommé.
- Un `AtomicBool stalled` par device est levé par le chien de garde quand un lot
  dépasse son budget. Tant qu'il est levé, tous les appels retournent
  `Stalled` immédiatement (pas d'ioctl, pas de thread), avec cooldown 30 s. Il
  est remis à zéro au débranchement/rebranchement (identité ou minor changé) ou
  après un lot réussi.
- Un verrou empoisonné (panic d'un thread abandonné) est traité comme
  `Stalled` ; on ne tente pas de le récupérer pour relancer un ioctl.
- Le verrou n'est **pas** tenu au-delà du lot : fd et verrou sont relâchés à
  la fin normale. Si un ioctl est bloqué définitivement, le thread reste bloqué
  avec le fd et le verrou : le device passe `stalled`, aucun nouveau thread
  n'est lancé pour lui, et le verrou est considéré comme perdu jusqu'au
  rebranchement ou à la fin du processus. C'est le compromis explicite de V-04
  (pas d'annulation d'ioctl sans `unsafe` supplémentaire).

**Budget global**

- Budget par cycle **et par processus** : **512 ioctl** au total, tous devices
  confondus, et **un seul device en vol** à la fois. `tccd` (lecture) et
  `tccd-user` (écriture) ont chacun leur budget.
- Compteur partagé (`AtomicU32` remis à zéro à chaque cycle) décrémenté avant
  chaque ioctl ; dépassement ⇒ arrêt propre, `truncated: true`, log unique.
- Les menus sont paresseux (aucun `QUERYMENU` sans demande de client) ; la
  lecture des contrôles est à la demande (bail/lease), pas à chaque tick 2 s
  (voir §7.1). À l'expiration du bail (aucun client), **aucun ioctl de contrôle
  n'est émis** ; seul le tick sysfs de 2 s subsiste.
- Le tick de 2 s ne fait qu'énumérer les nœuds sysfs (aucun ioctl) et détecter
  les apparitions/disparitions.

**Délais**

- Budget indicatif par lot : **2 s** (énumération + lectures), **1 s** par
  écriture. Le chien de garde (`Instant`) marque `stalled` au-delà et cesse de
  planifier. Cette limite est **advisory** : elle ne tue pas l'ioctl ; le
  timeout réel est celui du transfert USB du noyau (souvent 5 s), à mesurer.
- `EINTR` : reprise bornée (3 tentatives, 10 ms) ; jamais interprété comme fin
  d'énumération.

**Erreurs**

- `EBUSY`/`EAGAIN` : device occupé (aperçu, autre application). Le lot échoue
  proprement, l'état expose `busy` et la dernière erreur ; back-off 1/2/4/8 s,
  3 tentatives maximum, jamais de boucle serrée.
- `EACCES`/`EPERM` : `unsupported` (permissions), pas de retry infini ; dans
  `tccd-user`, l'UI affiche l'erreur d'ACL/groupe.
- `ENODEV` : device retiré ; état nettoyé, `stalled` remis à zéro.
- Le contrôleur ne garde pas le fd ouvert entre deux lots : moins de conflits
  avec les applications et pas de caméra « verrouillée » par le daemon.

### 4.8 Hotplug et tempêtes d'événements

- `tccd` ré-énumère les nœuds sysfs toutes les 2 s (aucun ioctl) : un nouveau
  `videoN` capture est publié (`WebcamDeviceAdded`), sans ouverture en écriture.
- `tccd-user` applique le préréglage actif **une seule fois** par
  `(node, identity, session)` après un debounce de 1 s ; coalescence des
  demandes, au plus un lot par device.
- Un appareil peut apparaître avant que l'ACL `uaccess` ne soit posée : back-off
  court (0,5/1/2 s) puis `EACCES` → état `pendingPermission`, pas de boucle.
- Un débranchement nettoie l'état, annule le suivi et remet `stalled` à zéro ;
  un rebranchement réapplique.
- Deux sessions simultanées : `try_lock` est **par processus** ; deux
  `tccd-user` distincts ne partagent pas le verrou. Le noyau arbitre l'accès
  concurrent (écritures `EBUSY`/`EAGAIN` selon pilote) : back-off borné, pas de
  coordination root, pas de file d'attente infinie.
- Pas d'application depuis udev (§8).

### 4.9 Persistance et entrées non fiables

- `/var/lib/tuxedo-rust-center/webcam.json`, root:root 0644, écriture atomique
  (tmp même répertoire, `fsync`, `rename`, `fchmod`), jamais `/etc`.
- **Seul `Admin1` écrit le fichier** (polkit, phase 3) ; `tccd` et `tccd-user`
  le lisent en lecture seule. `Manager1` ne persiste jamais.
- Lecture bornée : 256 Kio, 64 préréglages, 128 valeurs, clés ≤ 64 caractères,
  chaînes ≤ 64, entiers uniquement, JSON strict.
- Un préréglage invalide est ignoré avec un log ; il ne bloque pas le démarrage.
- Aucune valeur n'est écrite avant d'être validée contre l'énumération réelle
  du device **et** `cid_allowed` ; le fichier n'est jamais une source
  d'autorité pour un CID.
- Migration `/etc/tcc/webcam` : lecture, validation, sauvegarde de l'ancien
  fichier, conversion v2, écriture dans `/var/lib` (D2). Les clés hors
  allowlist sont conservées mais ignorées à l'application (log unique).

### 4.10 Authentification, journalisation et audit (V-02)

- **Canal livré** : écriture via `User1.SetUvcControl` dans `tccd-user` sur le
  **bus de session**, même UID que la GUI. Aucune action polkit n'est
  nécessaire : l'utilisateur écrit un périphérique auquel sa session a déjà
  accès. Validation `cid_allowed` + bornes + allowlist de clés identique à
  `tcc-hw` ; rate-limit 20/s burst 20 par UID (`LimitsExceeded`).
- **Repli `tccd`** (`Manager1.SetWebcamControl`) : **interdit sans
  `CheckAuthorization`**. Le handler doit appeler `tcc-authz` avec l'action
  dédiée `com.tuxedocomputers.tccd1.webcam` (`auth_admin`, sans `_keep`, **sans
  cache**), sujet `system-bus-name`, **fail-closed** (erreur/timeout polkit ⇒
  refus), **avant** d'ouvrir le membre et avant tout ioctl. La policy D-Bus reste
  fermée sur ce membre tant que l'authz n'est pas active et testée. Un
  rate-limit et un audit ne remplacent pas l'autorisation.
- `tccd` en lecture ne requiert pas polkit ; il reste soumis à la policy
  D-Bus existante.
- `tracing` : device (node + identité), contrôle (clé), résultat, erreur
  noyau ; jamais de données d'image, jamais de contenu de preset complet.
- `dry_run` reste propre à `tccd` ; le service de session n'écrit que sur
  appel explicite et audite `caller_uid`, device, clé et résultat. Aucun
  contenu de preset/image n'est journalisé.
- `controls_supported` ne passe à `true` que si le module est compilé **et**
  qu'au moins un contrôle est lisible sur au moins un device ;
  la capacité d'écriture est publiée séparément par `tccd-user` via
  `GetUvcWriteSupported`.

### 4.11 Interdits

- `unsafe` hors `v4l2/real.rs` et `v4l2/real_write.rs`.
- `ioctl` dans `tcc-daemon/src/hardware/webcam.rs`, dans un handler D-Bus ou sur
  le runtime async ; `S_CTRL` dans `tccd` par défaut (lecture seule).
- `VIDIOC_*` de streaming (`REQBUFS`, `QBUF`, `STREAMON`, ...).
- `S_CTRL` sur un CID absent de `cid_allowed`, même énuméré.
- CID/valeur fournis par le client sans résolution dans l'énumération ; `id`
  numérique accepté en entrée D-Bus.
- `lock()` bloquant dans un worker ; file d'attente par device.
- `RUN+=`, `IMPORT{program}`, shell, Python, `v4l2-ctl` à l'exécution.
- Application d'un préréglage depuis udev ou par `tccd`.
- Persistance d'un préréglage non validé, ou hors `Admin1`.
- Ouverture d'un chemin `/dev/v4l/by-id/*` ou d'un chemin non canonique/non
  énuméré fourni par l'UI (`User1` exige `/dev/videoN` énuméré).

### 4.12 Systemd / packaging (à transmettre à `packaging-release`)

**`tccd` (lecture seule)**

Le service a `DevicePolicy=closed` et `ProtectKernelTunables=no`. Pour la
lecture des contrôles :

```ini
DeviceAllow=char-video4linux r
```

- `char-video4linux` est le nom du groupe de périphériques caractères
  (major 81, visible dans `/proc/devices`) ; à confirmer par
  `systemd-analyze verify`/`security`.
- **Pas de `rw` par défaut** : l'écriture ne se fait pas dans `tccd` (V-03).
  Le repli `rw` est un delta explicite, conditionné à V-01+V-02 et à une revue
  sécurité.
- `PrivateDevices=yes` reste incompatible (`/dev/video*` disparaîtrait).
- Sans `DeviceAllow`, `QUERYCAP` échoue en `EACCES` : le domaine est
  `unsupported`, jamais un fallback root.
- Le mode du nœud (`0660`, `0666` + ACL `uaccess`, ...) n'est pas filtré par
  `DeviceAllow` : la règle d'accès systemd reste `r`. Un nœud world-writable
  ne change pas les droits du service (ouvert en `O_RDONLY`) ; il est
  journalisé pour diagnostic.

**`tccd-user` (écriture, session)**

- Aucun `DevicePolicy=closed` ; l'agent ouvre `/dev/videoN` en tant
  qu'utilisateur de la session.
- Accès via l'ACL `uaccess` de logind (siège actif) ou le groupe `video`.
  `tccd-user` doit gérer `EACCES` (siège inactif, ACL retirée) sans retry
  infini et sans élévation.
- Le commentaire actuel de l'unité « n'ouvre aucun périphérique » devra être
  corrigé (action `packaging-release`).
- `systemd-analyze security` : vérifier que l'ouverture du device ne nécessite
  aucune capacité supplémentaire ; ne pas ajouter `CAP_DAC_OVERRIDE`.

**Vérifications**

- `systemd-analyze verify` sur les deux unités.
- Test `tccd` avec `r` : lecture OK, `S_CTRL` impossible (le code ne l'expose
  pas) ; test `tccd-user` : écriture OK pour l'utilisateur du siège, `EACCES`
  sinon.
- `udevadm test` sur la règle (§8.1) ; aucun processus lancé.

---

## 5. API Rust proposée

```rust
/// Identité stable d'une caméra pour les préréglages.
pub struct UvcIdentity {
    pub vendor: Option<String>,   // "1bcf"
    pub product: Option<String>,  // "2c99"
    pub serial: Option<String>,
    pub fallback_name: String,    // nom sysfs si pas d'identité USB
}

pub struct UvcDevice {
    pub node: String,             // "video0"
    pub path: String,             // "/dev/video0"
    pub name: String,             // nom lisible (tronqué 128)
    pub identity: UvcIdentity,
    pub bus_info: Option<String>, // QUERYCAP, diagnostic
    pub control_count: usize,     // sans charger les détails (budget)
}

pub enum UvcValue {
    Integer(i64),
    Boolean(bool),
    Menu { index: i64, key: String, label: String },
}

pub enum UvcControlKind { Integer, Boolean, Menu, IntegerMenu }

pub struct UvcControlInfo {
    // Pas d'`id` public (V-11) : il reste interne au module `v4l2`.
    pub key: String,              // clé stable, ex. "brightness"
    pub name: String,             // nom noyau, sanitisé
    pub kind: UvcControlKind,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub default: i64,
    pub current: Option<UvcValue>, // None si WRITE_ONLY/illisible
    pub menu: Vec<UvcMenuEntry>,
    pub flags: UvcFlags,          // inactive, read_only, write_only, update, disabled
    pub category: UvcCategory,    // Basic, Exposure, Color, Advanced, Other
    pub simple: bool,             // visible en mode simple
    pub writable: bool,           // cid_allowed(id) && flags compatibles
}

pub struct UvcMenuEntry {
    pub index: i64,
    pub key: String,   // clé stable normalisée
    pub label: String, // libellé noyau
}

pub struct UvcPreset {
    pub name: String,
    pub device: UvcIdentity,
    pub values: std::collections::BTreeMap<String, UvcPresetValue>,
}

pub enum UvcPresetValue {
    Integer(i64),
    Boolean(bool),
    Menu(String), // clé de menu, jamais un index brut
}

pub struct UvcApplyReport {
    pub device: UvcIdentity,
    pub applied: Vec<String>,     // clés
    pub skipped: Vec<(String, String)>, // clé, raison
    pub errors: Vec<(String, String)>,
    pub needs_reapply: bool,
}

// API livrée (étape 1) — lecture
pub struct UvcReaderController;
impl UvcReaderController {
    pub fn system(root: SysfsRoot) -> Self;
    pub fn devices(&self) -> UvcDevicesReport;
    pub fn read_all(&self) -> UvcAllReport;
}

// API livrée (étape 2) — écriture, utilisée par tccd-user
pub struct UvcWriterController;
impl UvcWriterController {
    pub fn system(root: SysfsRoot) -> Self;
    pub fn set_control(
        &self,
        device_path: &str,
        control_key: &str,
        value: i64,
    ) -> Result<UvcWriteReport>;
}

// Étape 3 (non implémentée) : UvcPreset/UvcApplyReport restent un design.
```

- `tccd` n'instancie que `UvcReaderController` ; `tccd-user` instancie
  `UvcWriterController` (une instance partagée) et l'expose via `User1`.
  Aucun `set_control` n'est disponible dans le processus root.
- Les deux contrôleurs sont injectables (`ReaderFactory`/`WriterFactory`,
  `FakeReader`/`FakeWriter`) pour les tests.
- `set_control` prend une clé texte ; `cid_allowed` + résolution CID +
  validation se font à l'intérieur, sur une énumération fraîche. Aucun `u32`
  client.
- La lecture et l'écriture ouvrent une session courte (`try_lock`), puis la
  ferment.
- `UvcValue`/`UvcWriteReport` sont convertis côté daemon (voir §7), pas dans
  `tcc-hw`.

Erreurs `tcc-hw` livrées : `Ioctl`, `Unavailable`, `NotSupported`,
`InvalidValue`, `OutOfRange`, `PolicyDenied`, `Io`, `Busy`, `Stalled`,
`BudgetExceeded`. `tccd-user` les traduit en erreurs D-Bus stables
(`DBUS_API.md`). `NotAuthorized` ne concerne que le repli `Manager1` hypothétique
(2bis), hors périmètre actuel.

---

## 6. Préréglages et persistance

### 6.1 Schéma v2

```json
{
  "schemaVersion": 2,
  "presets": [
    {
      "name": "Bureau",
      "device": {
        "vendor": "1bcf",
        "product": "2c99",
        "serial": null,
        "fallbackName": "Integrated Camera"
      },
      "active": true,
      "values": {
        "brightness": 128,
        "contrast": 32,
        "white_balance_temperature": 4600
      }
    }
  ]
}
```

- `values` : clé texte → booléen, entier, ou clé de menu
  (`"aperture_priority_mode"`). **Toute clé hors allowlist §2.1 est conservée
  telle quelle mais ignorée à l'application** (`skipped`, log unique) : c'est
  le cas des automatismes (`white_balance_automatic`, `exposure_auto`) et des
  CIDs privés importés de l'original.
- Ordre de résolution d'identité : `(vendor, product, serial)` si serial
  présent, sinon `(vendor, product)`, sinon `fallbackName`.
- Deux caméras identiques sans serial partagent leur préréglage : documenté,
  l'UI affiche le chemin pour distinguer.

### 6.2 Migration legacy

`/etc/tcc/webcam` (original) :

```json
[{ "active": true, "presetName": "Default", "webcamId": "1bcf:2c99",
   "webcamSettings": { "brightness": 128, "exposure_auto": "aperture_priority_mode" } }]
```

- `webcamId` `vendor:product` → `device.vendor/product`.
- `presetName` → `name`, `webcamSettings` → `values`.
- Sauvegarde `.bak` root:root 0600, log des clés inconnues, valeurs non
  convertibles ignorées.
- Le fichier original n'est jamais modifié.

### 6.3 Application ordonnée

1. `focus_auto` d'abord s'il est présent et inscriptible (il conditionne
   `focus_absolute`).
2. Puis valeurs manuelles dépendantes (température de balance, exposition
   absolue, exposition, gain, ...).
3. Les clés hors allowlist (`exposure_auto`, `white_balance_automatic`, CIDs
   privés, ...) sont **ignorées** avec `skipped` + log unique ; elles ne
   produisent aucun ioctl. Conséquence : si l'automatisme est actif côté
   pilote, les valeurs manuelles correspondantes peuvent être sans effet ;
   c'est une limite assumée de la phase 1–3 (extension d'allowlist après
   revue).
4. Relecture de chaque contrôle après écriture ; si la valeur diffère, elle est
   publiée (arrondi pilote) et un avertissement est émis.
5. Après un contrôle `UPDATE`, ré-énumération avant de continuer.
6. Un échec n'interrompt pas les autres clés ; rapport final.

---

## 7. D-Bus et GUI

### 7.1 Lecture seule dans `tccd` (livrée)

- `tccd` expose `GetWebcamDataJSON()` (ou l'agrégat `GetHardwareSnapshotJSON`)
  en lecture seule, sans `DeviceAllow` en écriture.
- Lecture **à la demande** (bail/lease, comme les capteurs) : pas de
  `QUERYCTRL`/`QUERYMENU` à chaque tick de 2 s. Le tick ne fait qu'énumérer
  sysfs. Sans client, aucun ioctl de contrôle.
- Payload :

```json
{
  "switchAvailable": true,
  "enabled": true,
  "controlsSupported": true,
  "controlsTruncated": false,
  "devices": [
    {
      "name": "video0",
      "devicePath": "/dev/video0",
      "deviceName": "Integrated Camera",
      "index": 0,
      "metadata": false,
      "identity": {
        "vendor": "1bcf", "product": "2c99", "serial": null,
        "fallbackName": "Integrated Camera"
      },
      "busInfo": "usb-0000:00:14.0-1",
      "controls": [
        {
          // Pas d'`id` public (V-11) : seule la clé est exposée.
          "key": "brightness",
          "name": "Brightness",
          "kind": "integer",
          "min": 0, "max": 255, "step": 1, "default": 128,
          "current": {"kind": "integer", "value": 128},
          "menu": [],
          "flags": {"inactive": false, "readOnly": false,
                    "writeOnly": false, "update": false,
                    "disabled": false, "grabbed": false},
          "category": "color", "simple": true
        }
      ],
      "controlsError": null
    }
  ],
  "applied": null,
  "errors": []
}
```

- Aucun setter, aucun `unsafe` exposé, aucune persistance.
- `controlsSupported: true` seulement si au moins un contrôle **lisible** a
  été lu ; `controlsTruncated: true` si le budget ou un plafond a coupé la
  lecture.
- `tccd` ne publie **aucun** `writeSupported` : le payload est strictement en
  lecture (`controlsWritable: false`). La GUI interroge `tccd-user` sur le bus
  de session (`GetUvcWriteSupported`) pour connaître la capacité d'écriture
  réelle.
- `controls` n'est rempli qu'après une demande (bail) ; `devices()` seul ne
  charge ni contrôles ni menus.

### 7.2 Écriture volatile — service `User1` livré (V-03)

Le canal livré est le service **de session** `tccd-user`, même UID que la GUI.
Contrat de référence : `docs/DBUS_API.md` §« Service de session ».

```text
Service   com.tuxedocomputers.tccd.User
Objet     /com/tuxedocomputers/tccd/User
Interface com.tuxedocomputers.tccd.User1

GetUvcWriteSupported() -> b
SetUvcControl(s devicePath, s controlKey, i value) -> b
```

- `GetUvcWriteSupported` ne lit que l'énumération sysfs ; aucun device n'est
  ouvert au démarrage. Faux si le backend est absent ou si aucun device V4L2
  n'est énuméré.
- `SetUvcControl` :
  - `devicePath` = `/dev/videoN` canonique **réellement énuméré** ; metadata,
    chemin inventé, zéro de tête et `..` refusés (`InvalidArgs`) ;
  - `controlKey` = clé texte stable de l'allowlist (ASCII `[a-z0-9_]+`,
    ≤ 64 octets) ;
  - `value` entier (`i32`) : bornes, pas et index de menu validés par
    `tcc-hw` (`QUERYCTRL`/`QUERYMENU`), puis relecture après `S_CTRL` ;
  - retour `true` si la valeur a changé, `false` si elle était déjà courante
    (no-op, aucun `S_CTRL`) ;
  - budget 64 ioctl, timeout 2 s, `try_lock` par device, quarantaine 30 s ;
  - erreurs : `InvalidArgs`, `NotSupported`, `OutOfRange`, `Busy`, `Stalled`,
    `Failed`, `LimitsExceeded` (noms complets dans `DBUS_API.md`).
- Entrées bornées : corps D-Bus ≤ **64 Kio** refusé avant désérialisation
  (`InvalidArgs("message.size")`) ; `devicePath` et `controlKey` ≤ 64 octets
  avant toute énumération/ouverture.
- `GetUvcWriteSupported` : cache court de **1 s** (une énumération V4L2 par
  seconde au plus), plutôt qu'un rate limit qui ajouterait une erreur
  `LimitsExceeded` sur une méthode de capability.
- Débit : **20 écritures/s, burst 20, par UID** appelant ; si l'UID ne peut
  être résolu, le nom unique D-Bus sert de clé. Au-delà : `LimitsExceeded`.
- **Aucune action polkit, aucun `DeviceAllow`** : accès régi par l'ACL
  `uaccess` de logind ou le groupe `video`. Si l'ACL manque, l'ouverture
  échoue en `NotSupported` (jamais `Failed`) sans retry infini et **sans repli
  root**.
- `tccd` reste strictement en lecture : `S_CTRL` n'est jamais appelé par le
  daemon, la GUI ou les helpers de lecture.
- Persistance/préréglages : **non implémentés** (étape 3). `SetUvcControl` est
  volatile ; un préréglage par profil reste hors périmètre actuel.

### 7.3 Repli `tccd` (non retenu par défaut, V-02)

- `Manager1.SetWebcamControl(s device, s control, v value) -> b` n'est ouvert
  que si `CheckAuthorization` est implémenté et testé :
  - action dédiée `com.tuxedocomputers.tccd1.webcam` (`auth_admin`, **sans**
    `_keep`, **sans cache**, sujet `system-bus-name`) ;
  - `tcc-authz` en **fail-closed** (refus/timeout ⇒ `NotAuthorized`, aucun
    ioctl) ;
  - la policy D-Bus reste **fermée** sur ce membre tant que ce n'est pas fait ;
  - rate-limit/audit conservés, mais ne remplacent pas l'autorisation.
- `S_CTRL` dans `tccd` exige alors `DeviceAllow=char-video4linux rw` (delta
  explicite, revue sécurité).
- Aucune persistance dans `Manager1` : valeur perdue au redémarrage.

### 7.4 Persistance — `Admin1` uniquement (V-02, V-10)

- `Admin1.SaveWebcamSettingsJSON(s json) -> b` : polkit `system-bus-name`,
  action **existante** `com.tuxedocomputers.tccd1.settings` **ou** nouvelle
  action `com.tuxedocomputers.tccd1.webcam` (`auth_admin`, sans `_keep`) —
  l'action `configure` citée dans la première version de ce document **n'existe
  pas** dans `packaging/polkit`. À trancher par `security`/`packaging-release`
  et documenter dans `DBUS_API.md`.
- Écriture atomique dans `/var/lib` (D2), retour d'un rapport
  (`saved`, `backup`, `warnings`).
- `Admin1.SetActiveWebcamPreset(s device, s preset) -> b` ou champ `active`
  dans le JSON ; applique après sauvegarde.
- `Manager1.ApplyWebcamPreset` n'est pas nécessaire ; ne pas l'ajouter sans
  authz.

### 7.5 GUI

- `qt-ui` lit `GetWebcamDataJSON` (bus système) : mode simple (brightness,
  contrast, saturation) et avancé (catégories) ; les contrôles non
  `writable` sont en lecture seule.
- Écriture via `tccd-user` (bus de session, §7.2) ; l'UI interroge
  `GetUvcWriteSupported` et masque/désactive les contrôles si l'agent de
  session est absent, si l'ACL manque ou si la méthode retourne faux. Le
  payload de `tccd` reste toujours en lecture (`controlsWritable: false`).
- Contrôles `inactive` désactivés, `readOnly` en lecture, `WRITE_ONLY` sans
  valeur initiale.
- Rafraîchissement après écriture (valeur relue) et après `UPDATE`.
- Aperçu : Qt Multimedia natif ; résolution/FPS dans l'aperçu, pas dans
  `v4l2.rs`.
- Aucune écriture directe : la GUI passe par D-Bus, jamais par `/dev`.

---

## 8. udev et hotplug

### 8.1 Règle actuelle

`packaging/udev/99-tuxedo-rust-center-webcam.rules` :

```udev
SUBSYSTEM=="video4linux", ACTION=="add", KERNEL=="video[0-9]*", TAG+="tuxedo-rust-center-webcam"
```

Elle ne lance **aucun** processus, ce qui est correct. Trois corrections à
demander à `packaging-release` (V-12) :

1. Le commentaire affirme que « tccd écoute le netlink udev » : c'est faux
   aujourd'hui (`tccd` ré-énumère sysfs toutes les 2 s). Corriger le commentaire
   pour refléter le mécanisme réel.
2. Restreindre à `ACTION=="add"` (les `change` uaccess/ACL sont nombreux et
   n'apportent rien au préréglage) — à valider avec `qa-integration`.
3. Ne rien ajouter d'autre : pas de `RUN+=`, `IMPORT{program}`, `$env{}`, ni de
   propriété porteuse de valeur de contrôle. Le tag sert au diagnostic et à un
   futur consommateur netlink ; il n'est pas nécessaire au fonctionnement.

### 8.2 Déclenchement sans script privilégié

| Option | Latence | Complexité | Recommandation |
|---|---|---|---|
| Ré-énumération 2 s existante | ≤ 2 s | nulle (déjà là) | **phase 1–3** |
| Netlink udev dans `tccd` (crate sûre, ex. `neli`, MIT) | immédiate | dépendance + revue | phase 4 si besoin |
| `inotify` sur `/sys/class/video4linux` | immédiate | dépendance `notify`/`inotify` | alternative |
| Unité systemd par device + `ExecStart` | immédiate | processus privilégié par event | **rejeté** |
| `RUN+=` dans udev | — | script privilégié | **interdit** |

- `tccd` détecte l'apparition (sysfs, 2 s) et publie `WebcamDeviceAdded`
  (bus système) ; il **n'ouvre pas** le device en écriture.
- `tccd-user` applique le préréglage actif du device pour l'utilisateur de la
  session, après le debounce et la disponibilité de l'ACL (V-03). Avant la
  première session graphique, aucun préréglage n'est appliqué : écart assumé
  avec l'original (udev root), qui est précisément la surface supprimée.
- En cas de `EBUSY`, back-off borné ; l'état expose `pending`.
- Pas d'application sur `change` (évite les rafales uaccess) ; un rebranchement
  produit un nouvel `add`.
- Aucun événement udev ne transporte de valeur de contrôle : uniquement
  l'identité du device (déjà dans sysfs).
- Netlink (phase 4) ne remplacerait que la détection sysfs de `tccd` ; il ne
  déplacerait pas l'écriture hors de `tccd-user`.

---

## 9. Tests

### 9.1 Unitaires (`tcc-hw`)

- **ABI (V-16)** : tailles 104/8/68/44, offsets `queryctrl` et `querymenu`
  (`payload` 8, `reserved` 40, `align_of` 4), `reserved=0`, constantes ioctl
  exactes, `GRABBED=0x0002`, `EXECUTE_ON_WRITE=0x0200`,
  `HAS_PAYLOAD=0x0100`, `EINTR` borné, masquage des flags `NEXT_*` ;
  compilation 64 bits.
- **Allowlist normative (V-01)** : `V4L2_CID_TEST_PATTERN` énuméré ⇒ `set`
  refusé sans ioctl ; test exhaustif de la table (`cid_allowed` vrai uniquement
  pour les 15 CIDs §2.1) ; CID privé refusé en lecture et en écriture ;
  `AUTO_FOCUS_STATUS`/`CAMERA_*` en lecture seule.
- **Garde `unsafe`** : test statique qui scanne `crates/tcc-hw/src/v4l2/` et
  échoue si un fichier autre que `real.rs` contient `unsafe`/`libc::ioctl`, ou
  si `real.rs` contient plus de 5 appels `libc::ioctl`.
- **Validation du nœud** : prédicat pur sur `NodeMetadata` (char 81, minor
  attendu, uid 0 ; écriture monde journalisée et acceptée en lecture ;
  mauvais type et symlink refusés).
- **Énumération** : `FakeTransport` avec contrôles integer/boolean/menu/
  integer_menu/button/string/private/disabled ; skip des types non supportés ;
  cap 256/64 ; `NEXT_CTRL` jusqu'à `EINVAL`.
- **Normalisation des noms** : cas de l'original (`Auto Exposure`,
  `White Balance Temperature, Auto`, `Backlight Compensation`, collisions).
- **Menus** : sparse (`EINVAL` au milieu), cap 64, UTF-8 lossy, label vide.
- **Validation des valeurs** : min/max/step (multiple exact), booléen 0/1,
  index de menu inconnu, `READ_ONLY`/`INACTIVE`/`DISABLED` refusés, valeur
  retournée différente (avertissement), `UPDATE` → ré-énumération.
- **Device absent** : `ENODEV`/`ENOENT` → `unsupported`, pas de panic.
- **Busy** : `EBUSY` → rapport, back-off, pas de boucle.
- **Cycle de vie (V-04)** : `FakeTransport` bloquant ⇒ premier appel
  `Stalled` dans le budget, deuxième immédiat sans thread ; verrou empoisonné
  ⇒ `Stalled` ; 16 devices bloqués ⇒ 0 thread supplémentaire ; reset au
  rebranchement.
- **Budget (V-05)** : `FakeTransport` 16 devices × 256 contrôles ⇒ nombre
  d'ioctl ≤ 512 par cycle ; `truncated=true` ; aucun `QUERYMENU` sans client ;
  aucun ioctl de contrôle au tick de 2 s.
- **Ouverture (V-06/V-07/V-09)** : le contrôleur de lecture n'expose pas
  `set_control` ; `device_caps` autoritaire (capabilities capture + device_caps
  meta ⇒ refus) ;
  codec-only refusé ; gid/mode journalisés en debug ; `0666` accepté en
  lecture, `chown` non-root refusé par `tccd` ; `O_NOFOLLOW` sur symlink.
- **Chaînes (V-14)** : noms/bus_info/menus avec `\n`, `\0`, UTF-8 invalide,
  4 Kio ⇒ tronqués/sanitisés, jamais dans un chemin.
- **Authz (V-02)** : faux authorizer refus ⇒ 0 appel backend ; timeout/erreur
  polkit ⇒ refus ; policy fermée testée.
- **Identité** : `uevent` USB (`PRODUCT=`), serial, fallback nom, deux caméras
  identiques, device metadata ignoré.
- **Préréglages** (étape 3, non implémentés) : JSON valide/invalide, taille,
  doublons, clé inconnue (skippée), clé hors allowlist (skippée), migration
  legacy, ordre `focus_auto`→manuel, échec partiel.
- **Hotplug** : machine à états déterministe (`Instant` injecté) : apparition,
  debounce, une seule application, disparition, réapparition.
- **Absence de caméra** : `devices()` vide, `controls_supported: false`.

### 9.2 Intégration (`fake_v4l2.rs`, `v4l2::write`, tests `tccd-user`)

- `FakeReader`/`FakeWriter` + racine sysfs factice (`tempfile`) :
  énumération, lecture, écriture (writer seul), device absent, permissions
  refusées, budget, `stalled`, `try_lock`.
- Aucun `/dev/video*`, aucun privilège, aucun réseau.
- Test daemon `tccd` : `HardwareStatus` publie `controlsSupported`, les
  contrôles (sans `id`), n'appelle jamais `S_CTRL`, `dry_run` sans effet.
- Test `tccd-user` (session, `qa-integration`/`linux-platform`) : service
  `User1` testé (validation, rate limit, erreurs typées, fake writer) ;
  `GetUvcWriteSupported` correct ; écriture réelle non testée sur caméra
  physique.

### 9.3 Fuzzing / propriétés

- Parseur de préréglages (taille, types, bornes, clés) : propriétés + corpus.
- Noms de contrôles/menus (UTF-8, 32 octets, contrôle chars).
- Machine à états hotplug (séquences d'événements aléatoires) sans panic.

### 9.4 Smoke test matériel (hors CI, documenté)

0. Lecture seule (étape 1), sans outil externe :
   `cargo test -p tcc-hw --test real_v4l2 -- --ignored --nocapture` et
   `cargo test -p tcc-daemon --bins real_hardware_exposes_uvc_controls -- --ignored --nocapture`.
   Vérifier `errors: []` et `controlsSupported: true`. Résultat de référence :
   11 contrôles sur une webcam FHD `2b7e:c858` (2026-11-06).
1. `v4l2-ctl --list-ctrls` (outil de diagnostic, jamais utilisé au runtime)
   comparé à `GetWebcamDataJSON` sur la caméra interne et une USB.
2. Écrire `brightness`, `contrast`, `white_balance_temperature` ; relire ;
   vérifier l'absence de LED allumée à l'énumération.
3. Ouvrir l'aperçu puis changer un contrôle (`EBUSY` attendu ou succès selon
   pilote) ; vérifier le back-off.
4. Débrancher/rebrancher : préréglage appliqué une fois, pas de boucle.
5. Suspend/reprise : device ré-énuméré, préréglage réappliqué.
6. Vérifier `DeviceAllow=char-video4linux r` sur `tccd` avec
   `systemd-analyze security` : lecture OK, `S_CTRL` jamais appelé par `tccd`
   (le module d'écriture existe dans `tcc-hw` pour `tccd-user`).
7. Vérifier `tccd-user` : écriture OK avec ACL `uaccess` et groupe `video` ;
   ACL retirée/siège inactif ⇒ `EACCES` sans retry infini ; aucune capacité
   supplémentaire.
8. Vérifier les permissions : `chmod 0666` accepté en lecture (cas réel) ;
   `chown` non-root ⇒ refus ; symlink ⇒ `O_NOFOLLOW` ; major/minor faux ⇒
   refus. Pour l'écriture : `O_RDWR` réussi avec ACL `uaccess`/groupe
   `video`, échec `EACCES` sinon ; jamais de repli root.
9. `strace -f -e trace=ioctl` : 4 ioctl de contrôle en lecture
   (`QUERYCAP/QUERYCTRL/QUERYMENU/G_CTRL`) ; en écriture, jusqu'à 5
   (`+ S_CTRL`), aucun `REQBUFS/QBUF/STREAMON/read/mmap`.
10. Deux écritures simultanées dans le **même** `tccd-user` : la seconde voit
    `Busy` (`try_lock`). Deux processus de session distincts ne partagent pas
    le verrou applicatif : le noyau arbitre (`EBUSY`/`EIO`). `tccd` ne
    modifie rien.

---

## 10. Plan révisé et critères de sortie

L'écriture UVC est livrée dans `tcc-hw` et exposée par le service de session
`User1` de `tccd-user`. `tccd` reste en lecture seule, sans
`DeviceAllow=char-video4linux rw` et sans appel à `S_CTRL`. Les préréglages
persistés restent l'étape 3 ; la GUI simple/avancée est en cours.

| Étape | Contenu | Prérequis / revue |
|---|---|---|
| **0** | Intégration des findings V-01..V-16 dans ce document ; validation par `security` de la table §2.1, du schéma de privilèges V-03 et de l'action polkit | revue documentaire |
| **1 — livrable lecture seule — FAIT** | `abi.rs`, `real.rs` (QUERYCAP/QUERYCTRL/QUERYMENU/G_CTRL, **pas S_CTRL**), `control.rs` (allowlist + lecture), `device.rs` (O_RDONLY), `FakeTransport`, `UvcReaderController`, `WebcamData.controls` à la demande, `controlsSupported` ; `tccd.service` : `DeviceAllow=char-video4linux r` (packaging à suivre) | revue unsafe/ABI + test `tccd` sans écriture |
| **2 — écriture session — FAIT (backend + service)** | `UvcWriterController`/`RealWriter`/`FakeWriter` dans `tcc-hw` ; service `User1` (`SetUvcControl`, `GetUvcWriteSupported`) dans `tccd-user`, rate limit et erreurs typées testés. GUI simple/avancé en cours. Aucune écriture dans `tccd`. | revue canal de session + test réel caméra restant |
| **2bis — repli root (non planifié)** | `Manager1.SetWebcamControl` + `CheckAuthorization` `tccd1.webcam` + `DeviceAllow=… rw` dans `tccd` | **seulement** si V-01+V-02+revue du delta ; sinon rejeté |
| **3 — préréglages** | schéma v2, migration, `Admin1.SaveWebcamSettingsJSON` (polkit `settings` ou `webcam`), application `tccd-user` au démarrage/hotplug, GUI | revue persistance + polkit |
| **4 — parité avancée** | CID privés connus (table par `vid:pid`), `webcam.preset` par profil (`tcc-core`), i18n, netlink si retenu | revue par CID privé / dépendance |

### Critères de sortie

- **Étape 1** : `cargo test -p tcc-hw` vert ; `FakeTransport` 16×256 respecte
  le budget ; `tccd` ne contient ni `S_CTRL` ni `OpenMode::Write` ;
  `GetWebcamDataJSON` expose les contrôles sans `id`, `truncated` honnête,
  `controlsSupported` faux sans device ; `DeviceAllow` limité à `r` ;
  `systemd-analyze security` stable ; smoke test lecture sur 2 caméras, aucune
  LED allumée (ou documenté).
- **Étape 2** : `cid_allowed` testé exhaustivement ; `User1.SetUvcControl`
  livré, rate-limité par UID ; `try_lock`/`stalled` testés (device bloqué) ;
  erreurs `InvalidArgs`/`NotSupported`/`OutOfRange`/`Busy`/`Stalled`/
  `LimitsExceeded` ; `tccd-user` écrit avec ACL/groupe, `EACCES` sans retry
  infini ; `tccd` ne peut pas écrire ; écriture réelle caméra à valider
  manuellement.
- **Étape 2bis** (si retenue) : faux authorizer refus ⇒ 0 ioctl ; policy
  fermée tant que l'authz n'est pas active ; test `r` vs `rw`.
- **Étape 3** : migration legacy sans perte ; `Admin1` seul écrivain ; clés
  hors allowlist skippées ; application une seule fois au hotplug ; préréglage
  non appliqué avant session (documenté).
- **Étape 4** : chaque CID privé a une justification et un test ; aucune
  régression des étapes 1–3.

Après chaque étape : `cargo fmt`, `clippy -D warnings`, `cargo test -p tcc-hw
-p tcc-daemon`, `cargo deny`/`audit`, smoke test matériel, mise à jour de
`FEATURE_PARITY.md`, `HARDWARE_MATRIX.md`, `DBUS_API.md`, note de revue
sécurité.

## 11. Alternatives

| Option | Licence | Avantages | Inconvénients | Verdict |
|---|---|---|---|---|
| **Locale (`v4l2.rs`)** | GPL-3.0-or-later (projet) | ~250–350 lignes auditable, 5 ioctl, aucune dépendance, `unsafe` confiné, ABI testée | À maintenir (structs noyau) | **Recommandée** |
| `v4l` + `v4l2-sys-mit` | MIT | API stable, large couverture | FFI généré (bindgen), grande surface d'audit, dépendances, `unsafe` hors de notre contrôle | Acceptable si le local est refusé |
| `v4l2r` | MIT/Apache-2.0 | Wrapper plus sûr, types | Dépend de `v4l`/FFI, encore plus de code, API orientée streaming | Non préférée |
| `v4l2-ctl` (v4l-utils) | GPL-2.0+ | Existant | Commande externe, parsing, non garantie installée, pas d'allowlist de contrôles | Rejeté |
| `cameractrls.py` | MIT | Parité immédiate | Python interdit, udev privilégié, parsing | Rejeté |

La décision locale respecte la recommandation n°2 de
`REVIEW_HARDWARE.md` §4 et évite d'ajouter une dépendance FFI non revue.

**Attribution (V-13)** : la normalisation des noms de contrôles est dérivée de
`cameractrls.py` (MIT, copyright soyer). Si ce code est repris, `control.rs`
doit conserver un en-tête MIT et la provenance, et la compatibilité
GPL-3.0-or-later doit être confirmée (MIT est compatible). Une revue
licence/CI est requise ; `cargo deny` ne couvre pas le code recopié.

---

## 12. Risques résiduels et points à trancher

| Risque | Impact | Mitigation / décision |
|---|---|---|
| ABI noyau qui dérive | ioctl incorrect, corruption | Constantes figées + tests de taille/offset ; pas de `QUERY_EXT_CTRL` |
| Driver UVC bogué (bornes/step faux, G_CTRL en erreur) | valeurs refusées ou incohérentes | Relire après écriture, skip des contrôles illisibles, log unique |
| ioctl bloquant | thread `spawn_blocking` immobilisé, verrou de device gelé | `try_lock` + `stalled` (jamais `lock()`), budget/chien de garde, un thread max par device, cooldown ; timeout USB noyau à mesurer |
| Caméra occupée / streaming | `EBUSY`, contrôles indisponibles | back-off borné, état `busy`, ne jamais interrompre l'aperçu |
| LED/confidentialité | allumage involontaire | ne jamais streamer, tester l'énumération ; documenter le comportement par modèle |
| Périphérique USB malveillant | métadonnées piégées (menus géants, noms) | bornes 256/64, UTF-8 lossy, pas de confiance aux tailles |
| CID privés | sémantique inconnue | refus par défaut ; table revue en phase 4 |
| Deux caméras identiques | préréglage appliqué aux deux | identité serial si disponible, sinon documenté |
| `DeviceAllow` manquant | `EACCES` silencieux | `char-video4linux r` dans `tccd` (lecture), test systemd/hardware ; `rw` seulement en repli V-02 |
| ACL/siège absent pour `tccd-user` | écriture impossible | état `pendingPermission`, message UI, aucun repli root |
| Préréglage avant session | non appliqué au boot | écart assumé vs original ; documenté, application à la première session |
| Setter non authentifié (repli) | utilisateur local modifie la caméra d'un autre | pas de `Manager1` d'écriture par défaut ; `CheckAuthorization` fail-closed si repli ; policy fermée sinon |
| Écriture root | surface privilégiée inutile | écriture dans `tccd-user` ; `tccd` en `O_RDONLY`/`r` |
| Pas de test matériel en CI | régressions pilote | smoke tests documentés + `qa-integration` sur laptop |
| 5 ioctls vs 4 dans `REVIEW_HARDWARE.md` | incohérence documentaire | `REVIEW_V4L2.md` accepte les 5 ; note datée à ajouter dans `REVIEW_HARDWARE.md` §4 (action `security`) |
| `unsafe` ABI | erreur de layout | `#[repr(C)]`, tests, revue ligne à ligne |

### Points tranchés par la revue

1. **5 ioctls** : acceptés (`VIDIOC_QUERYMENU` inclus) ; note datée à ajouter
   dans `REVIEW_HARDWARE.md` §4 (action `security`).
2. **CID privés** : refus en phase 1–3, table `(vid,pid)` phase 4 après revue.
3. **Préréglage** : par device d'abord ; `webcam.preset` par profil = extension
   phase 3, non condition de la phase 1–2.
4. **Écriture** : `tccd-user` recommandé ; `Manager1` volatile seulement avec
   authz ; persistance `Admin1`.
5. **Hotplug** : polling 2 s en phase 1–3 ; netlink phase 4 si nécessaire.
6. **`DeviceAllow`** : `r` dans `tccd` ; pas de `rw` par défaut.
7. **Allowlist** : table normative §2.1, 15 CIDs inscriptibles, refus de tout
   autre CID.

### Actions hors périmètre de ce document

- `security` : note datée dans `REVIEW_HARDWARE.md` §4 acceptant
  `VIDIOC_QUERYMENU` (V-15) ; confirmer l'action polkit retenue.
- `packaging-release` : règle udev `ACTION=="add"` + commentaire corrigé
  (V-12) ; `DeviceAllow=char-video4linux r` sur `tccd` ; commentaire de
  `tccd-user` (« n'ouvre aucun périphérique ») à corriger.
- `linux-platform` : interface de session `tccd-user`, bail de lecture webcam,
  `prepareForSleep` déjà en place ; aucun chemin d'écriture dans `tccd`.
- `rust-core` : `webcam.preset`/`usePreset` si le préréglage par profil est
  retenu (phase 3).

### Points restants à trancher

1. **Action polkit** de persistance : `com.tuxedocomputers.tccd1.settings`
   (existante) ou nouvelle `tccd1.webcam` (`auth_admin`, sans `_keep`) ?
2. **Repli `tccd` rw** : est-il nécessaire un jour, ou la voie session suffit
   pour tous les cas d'usage (pas de session graphique, serveur) ?
3. **Préréglage avant session** : accepter l'écart, ou prévoir un mode
   explicitement autorisé par polkit pour appliquer au boot ?
4. **Gid strict** : journaliser seulement (proposé) ou refuser un groupe autre
   que `video` (risque NSS/distributions) ?
5. **Allowlist auto** : ajouter `exposure_auto`/`white_balance_automatic` en
   phase 2 après revue, pour rendre `exposure_absolute`/température réellement
   effectifs ?
6. **LED** : comportement exact de l'énumération sur les modèles TUXEDO
   (aucun flux ne doit être ouvert) — à documenter par modèle.

---

## Annexe A — références originales

- `tuxedo-control-center-master/src/cameractrls/cameractrls.py` : énumération
  `QUERYCTRL`, menus, normalisation des noms, catégories, presets.
- `src/common/models/TccWebcamSettings.ts` : `WebcamPreset`,
  `WebcamPresetValues`, `WebcamDeviceInformation`.
- `src/ng-app/app/webcam-settings/webcam-settings.component.ts` :
  `easyOptions = ['brightness','contrast','resolution']`, catégories, presets.
- `src/dist-data/99-webcam.rules` : ancien `RUN+=` Python (à ne pas reproduire).
- `crates/tcc-hw/src/webcam.rs` : énumération sysfs et kill switch existants.
- `docs/security/REVIEW_HARDWARE.md` §4 : contraintes initiales.
- `docs/security/REVIEW_V4L2.md` : revue du présent document, findings
  V-01..V-16 et décisions §4.

## Annexe B — encodage ioctl et ABI

```text
_IOC(dir, type, nr, size) = (dir << 30) | (size << 16) | (type << 8) | nr
_IOR  = dir 2, _IOW = dir 1, _IOWR = dir 3
```

Valeurs vérifiées par compilation du header noyau (`linux/videodev2.h`) :

| Constante | Valeur |
|---|---|
| `VIDIOC_QUERYCAP` | `0x80685600` |
| `VIDIOC_G_CTRL` | `0xC008561B` |
| `VIDIOC_S_CTRL` | `0xC008561C` |
| `VIDIOC_QUERYCTRL` | `0xC0445624` |
| `VIDIOC_QUERYMENU` | `0xC02C5625` |

Flags utilisés : `NEXT_CTRL=0x80000000` (seul flag d'énumération),
`NEXT_COMPOUND=0x40000000` (jamais utilisé, masqué s'il apparaît),
`DISABLED=0x0001`, `GRABBED=0x0002`, `READ_ONLY=0x0004`, `UPDATE=0x0008`,
`INACTIVE=0x0010`, `WRITE_ONLY=0x0040`, `HAS_PAYLOAD=0x0100`,
`EXECUTE_ON_WRITE=0x0200`.
