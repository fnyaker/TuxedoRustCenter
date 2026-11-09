# Revue de sécurité — contrôles UVC webcam (V4L2)

| Champ | Valeur |
|---|---|
| Version | 1.1 |
| Date | 2026-11-06 (mise à jour 2026-11-07) |
| Auteur | agent `security` |
| Objet | revue de sécurité du design `docs/hardware/V4L2_DESIGN.md` (contrôles UVC, 5 ioctl, allowlist, D-Bus, udev, privilèges) |
| Périmètre | `docs/security/**` uniquement — aucun code modifié, design non modifié |
| Documents lus | `docs/hardware/V4L2_DESIGN.md` (892 l.), `docs/security/REVIEW_HARDWARE.md` §4, `docs/security/DECISIONS.md` (D1/D2/D4/D5/D7), `docs/security/ADMIN_DESIGN.md` (AD-8), `docs/FEATURE_PARITY.md`, `docs/hardware/HARDWARE_MATRIX.md`, `packaging/{systemd,dbus,polkit,udev}` |
| Code lu | `crates/tcc-hw/src/{lib.rs,webcam.rs,tuxedo_io.rs,error.rs,command.rs}`, `crates/tcc-daemon/src/{hardware/webcam.rs,dbus.rs}`, `packaging/systemd/{tccd,tccd-user}.service`, `packaging/dbus/com.tuxedocomputers.tccd.conf`, `packaging/polkit/com.tuxedocomputers.tccd.policy`, `packaging/udev/99-tuxedo-rust-center-webcam.rules` |
| Statut | **Design revu + lecture seule validée sur matériel réel + écriture UVC de session livrée** (`write.rs`/`real_write.rs` dans `tcc-hw`, `User1.SetUvcControl` dans `tccd-user`) ; `tccd` reste `O_RDONLY`/`DeviceAllow=char-video4linux r` ; tests fake/CI, écriture réelle non testée ; **UVCW-02…07 fermés**, **UVCW-08 fonctionnellement fermé** (commentaire résiduel), **UVCW-01 accepté comme dette documentée** ; voir **§2ter**. |

> **Mise à jour 2026-11-06** : l'étape 1 **lecture seule** est désormais implémentée
> (`crates/tcc-hw/src/v4l2/**`, 4 ioctl, allowlist 15 CIDs, `unsafe` confiné, `O_RDONLY`).
> Les constats §1/§2 restent l'état du design ; le statut réel et les nouveaux findings sont en
> **§2bis**. L'écriture (`S_CTRL`) était alors hors jalon 2 (voir la note 2026-11-07 et **§2ter**).
>
> **Mise à jour 2026-11-07** : l'écriture UVC est implémentée dans `tcc-hw`
> (`write.rs`/`real_write.rs`) et exposée par `tccd-user` (`User1.SetUvcControl`, bus de session,
> non privilégié). `tccd` reste `O_RDONLY` et `DeviceAllow=char-video4linux r`. Les findings
> d'écriture et le verdict sont en **§2ter**.

---

## 1. Verdict

Le design est **globalement solide et implémentable**. L'ABI est correcte (les 5 ioctl, tailles, offsets et
constantes ont été recoupés avec le header noyau), l'`unsafe` est correctement confiné, l'ouverture du
nœud est défensive, et les menus/valeurs sont bornés. **Aucun finding Critique ou Élevé.**

Trois écarts **Moyens** doivent être corrigés avant d'écrire du code de contrôle :
1. l'allowlist de CIDs est **descriptive** (§2.1) mais pas **normative** dans la règle d'écriture (§4.3) ;
2. le setter volatil `Manager1.SetWebcamControl` n'a **aucune authentification** dans le design (rate-limit
   et audit ne sont pas une autorisation) ;
3. le design met l'écriture UVC dans `tccd` **root** (`DeviceAllow=char-video4linux rw`), alors que
   l'écriture est faisable sans privilège dans la session ; le choix doit être tranché et documenté.

Deux **Moyens** supplémentaires portent sur la **disponibilité** : un ioctl bloqué qui détient le
`Mutex` du device gèle toutes les opérations, et l'énumération complète toutes les 2 s n'a pas de budget
global (un device piégé peut multiplier les ioctls). Les autres findings sont Faibles/Info.

**Décision demandée** : `security` **accepte les 5 ioctl** — `VIDIOC_QUERYMENU` est indispensable pour
les menus (`exposure_auto`, balance des blancs) et ne crée pas de flux. `REVIEW_HARDWARE.md` §4 qui
parle de « 4 ioctls » doit être amendé (citation, pas réécriture de l'historique).

---

## 2. Findings

| ID | Sév. | Réf. design | Constat | Remédiation | Test |
|---|---|---|---|---|---|
| **V-01** | **Moyen / I** | `V4L2_DESIGN.md:76-91` (§2.1) vs `:337-340` (§4.3) | **L'allowlist de CIDs n'est pas une barrière d'écriture.** §2.1 liste les CIDs « exposés », mais §4.3 n'exige que l'appartenance à l'**énumération fraîche** : tout contrôle non privé, non listé et non interdit par type (`TEST_PATTERN`, `DIGITAL_GAIN`, `DV_TX_*`, `MPEG_*`, etc.) peut être écrit par clé. La liste est descriptive, pas normative. | Ajouter `fn cid_allowed(id: u32) -> bool` (match statique sur la table §2.1, `AUTO_FOCUS_STATUS`/`CAMERA_ORIENTATION`/`CAMERA_SENSOR_ROTATION` explicitement **lecture seule**) et l'appeler dans `set` **avant** toute résolution de menu. Un CID énuméré mais hors allowlist ⇒ `NotSupported`/`PolicyDenied`, jamais `S_CTRL`. | Test avec `FakeTransport` : `V4L2_CID_TEST_PATTERN` énuméré ⇒ `set` refusé ; test de propriété sur la table ; revue de la table par `security`. |
| **V-02** | **Moyen / I** | `V4L2_DESIGN.md:658-669` (§7.2) ; `packaging/dbus/…conf:111-138` | **Le setter volatil n'est pas authentifié.** Le design s'appuie sur « rate-limit existant + audit `caller_uid` + `dry_run` ». Un rate-limit n'authentifie pas : sur un poste multi-utilisateur, tout utilisateur local peut modifier la caméra d'un autre, y compris `V4L2_CID_PRIVACY` (`:89`) et l'exposition automatique. Aujourd'hui la policy refuse **tous** les setters `Manager1` au niveau bus ; l'ouverture de `SetWebcamControl` serait un changement de policy non couvert par une autorisation. | Deux options, dans l'ordre : (a) `tccd` appelle `CheckAuthorization` (action dédiée `com.tuxedocomputers.tccd1.webcam`, `auth_admin`, sans cache, sujet `system-bus-name`) **avant** d'ouvrir le membre ; (b) à défaut, garder le membre **refusé au bus** et ne livrer que la lecture. Ne jamais ouvrir la policy sur le seul rate-limit. | Faux authorizer : refus ⇒ 0 appel backend ; policy test : membre absent/refusé tant que l'authz n'est pas active ; test multi-utilisateur documenté. |
| **V-03** | **Moyen / I** | `V4L2_DESIGN.md:246-262` (§3.5), `:448-460` (§4.12) ; `packaging/systemd/tccd.service:64-66` | **Écriture UVC dans `tccd` root avec `DeviceAllow=char-video4linux rw`.** `tccd` parse des messages D-Bus en continu ; lui ajouter un second périphérique à ioctl augmente la surface root, alors que l'écriture UVC est une opération **utilisateur** : l'utilisateur de la session possède déjà `/dev/videoN` via l'ACL uaccess (ou le groupe `video` selon la distribution). Le design ne discute pas cette alternative et suppose `rw` dans `tccd`. | **Recommandé** : `tccd` **lecture seule** (`DeviceAllow=char-video4linux r`, ouverture `O_RDONLY`) pour `WebcamData` ; `S_CTRL` et l'application des préréglages dans `tccd-user` (session, même utilisateur, petite interface D-Bus de session **à ajouter**). **Acceptable en repli** : `tccd` conserve `rw`, mais **seulement** avec V-01 + V-02 + revue du delta. **Rejeté** : `rw` sans authz, ou déport dans `tccd-admin` (qui n'a aucun besoin de `/dev/video*`). | Test : `tccd` ne peut pas `S_CTRL` (device en `r`) ; `tccd-user` peut ; test de bascule de siège (ACL perdue ⇒ `EACCES`) ; `systemd-analyze security`. |
| **V-04** | **Moyen / I (dispo)** | `V4L2_DESIGN.md:386-397` (§4.7), `:214-217` (§3.2) | **Un ioctl bloqué qui détient le `Mutex` du device gèle le device.** Le design dit « le thread bloqué est abandonné : au pire un thread par device », mais si le `Mutex` est détenu pendant l'ioctl, les appels suivants bloquent sur `lock()` (threads `spawn_blocking` consommés) ou le mutex reste verrouillé (et empoisonné si le thread panique). Le watchdog est **advisory** et ne libère pas le verrou. | Utiliser `try_lock` (jamais `lock()` bloquant sur le chemin worker) ; si le verrou est pris, marquer `stalled` et refuser immédiatement (`Busy`/`Stalled`), avec cooldown et réinitialisation au rebranchement. Ne pas tenir le verrou pendant un `spawn_blocking` qui peut être abandonné ; ou dédier un thread par device avec un canal borné et un état `stalled` indépendant du verrou. | `FakeTransport` qui bloque : premier appel ⇒ `Stalled` dans le budget, deuxième ⇒ immédiat, aucun thread supplémentaire ; test de poison/panic ; test de 16 devices bloqués. |
| **V-05** | **Moyen / I (dispo)** | `V4L2_DESIGN.md:329-334` (§4.3), `:246-262` (§3.5), `:386-397` (§4.7) | **Pas de budget global d'ioctl.** Les caps sont par contrôle (256) et par menu (64), mais un device piégé peut exposer 256 contrôles × 64 menus ; avec 16 devices et un tick de 2 s, on atteint des milliers de `QUERYMENU`/`G_CTRL` par tick. Le budget de 2 s est advisory et ne limite pas le nombre d'ioctls ; cela peut saturer l'USB/le CPU et allumer la LED sur certaines caméras. | Budget **global** par tick (ex. 512 ioctls), chargement **paresseux** des menus (à la demande UI), lecture des contrôles **à la demande** (bail/lease comme les capteurs) plutôt qu'à chaque tick, et un seul device en vol à la fois. Journaliser le dépassement et exposer `truncated`. | `FakeTransport` avec 16×256 contrôles : nombre d'ioctls ≤ budget ; test « aucun client ⇒ aucune lecture de contrôle » ; test LED documenté sur matériel. |
| **V-06** | Faible / A | `V4L2_DESIGN.md:274` (§4.1) | `O_RDWR` est utilisé pour **toutes** les opérations, y compris la lecture seule ; cela élargit inutilement l'accès et empêche un `DeviceAllow=… r` côté `tccd`. | Séparer `OpenMode::Read` (`O_RDONLY`) et `OpenMode::Write` (`O_RDWR`) ; le lecteur ne doit pas exposer `set_ctrl` (traits `V4l2Reader`/`V4l2Writer`). | Test : transport read-only refuse `set` au niveau type ; `DeviceAllow=char-video4linux r` suffit pour `GetWebcamDataJSON`. |
| **V-07** | Faible / A | `V4L2_DESIGN.md:286-288` (§4.1) | La règle `QUERYCAP` est ambiguë : « `V4L2_CAP_VIDEO_CAPTURE` et `V4L2_CAP_DEVICE_CAPS` cohérents » ne dit pas que `device_caps` est **autoritaire** quand le flag est posé. Un nœud metadata/codec pourrait passer si seul `capabilities` est lu (le refus `index > 0` couvre le cas nominal, pas tous les pilotes). | Règle exacte : `caps = if capabilities & V4L2_CAP_DEVICE_CAPS != 0 { device_caps } else { capabilities }` ; exiger `V4L2_CAP_VIDEO_CAPTURE`, refuser `V4L2_CAP_META_CAPTURE`, refuser si `index` sysfs absent **et** `device_caps` non capture. | Fixtures : nœud metadata avec `capabilities=VIDEO_CAPTURE`/`device_caps=META_CAPTURE` ⇒ refus ; codec-only ⇒ refus. |
| **V-08** | Faible / A | `V4L2_DESIGN.md:331-340` (§4.3) | `V4L2_CTRL_FLAG_NEXT_COMPOUND` est utilisé sans besoin (aucun type composé supporté), et le design ne dit pas de **masquer** les flags `NEXT_CTRL`/`NEXT_COMPOUND` dans l'`id` retourné avant stockage/`S_CTRL`. Un id flaggé serait rejeté par le noyau ou ambigu. | Utiliser `NEXT_CTRL` seul, masquer `NEXT_CTRL|NEXT_COMPOUND` avant stockage, tester la terminaison `EINVAL` et un id flaggé dans `FakeTransport`. | Test d'énumération avec ids flaggés ; vérifier que `S_CTRL` reçoit l'id nu. |
| **V-09** | **Info (fermé)** | `V4L2_DESIGN.md:453-462` (§4.1) ; `device.rs:50-104` | **Règle révisée.** Le bit d'écriture monde n'est **pas** un critère de refus en lecture seule : `O_RDONLY` ne confère aucun droit d'écriture, et `root:video 0666` + ACL `uaccess` (`crwxrwxrwx+`) est le cas réel d'une webcam FHD. Les contrôles stricts restent : caractère, major 81, minor sysfs, `uid == 0`, `O_NOFOLLOW`, `O_RDONLY`. `gid` et mode sont journalisés. | Risque résiduel : un autre utilisateur local disposant de l'ACL/du groupe `video` peut modifier les contrôles UVC via `/dev/videoN` — c'est la politique udev/ACL de la distribution, **pas une élévation via `tccd`**, qui n'ouvre qu'en lecture et ne compile pas `S_CTRL`. | Maintenir `O_RDONLY` et l'absence de `S_CTRL` ; tout futur chemin d'écriture (`tccd-user`, `O_RDWR`) doit revérifier ACL/groupe/siège avant `S_CTRL`. | `accepts_world_writable_node_for_read_only_use` ; `rejects_wrong_type_major_minor_or_owner` ; réel `/dev/video0` `0666` accepté. |
| **V-10** | Info | `V4L2_DESIGN.md:673` (§7.3) | L'action polkit `configure` **n'existe pas** : le fichier `packaging/polkit/…policy` déclare `profiles`, `settings`, `power-limits`, `charging`, `display`, `prime-select`, `tomte-run`, `packages` (aucune action LUKS, hors périmètre). | Utiliser `com.tuxedocomputers.tccd1.settings` (déjà `auth_admin`) ou ajouter `com.tuxedocomputers.tccd1.webcam` avec `auth_admin`, sans `_keep`, et documenter. Ne pas inventer d'ID. | Test de policy : action inexistante ⇒ `NotAuthorized` ; `validate.sh`. |
| **V-11** | Info | `V4L2_DESIGN.md:639` (§7.1) | Le JSON expose l'`id` numérique brut du contrôle. Le design interdit qu'un CID vienne du client, mais le champ invite un futur implémenteur à le renvoyer. | Retirer `id` du payload public, ou le documenter « lecture seule, jamais accepté en entrée » ; le setter ne prend qu'une clé texte. | Test : `SetWebcamControl` avec un id numérique/une clé inconnue ⇒ `InvalidArgs`. |
| **V-12** | Info | `V4L2_DESIGN.md:698-713` (§8.1) ; `packaging/udev/99-…rules:7-8,14` | La règle udev est **tag-only** (correct, aucun `RUN+=`), mais le commentaire affirme que `tccd` « écoute le netlink udev » : c'est faux (H-12), et `ACTION=="add|change"` ajoute du bruit `change`/uaccess sans consommateur. | Corriger le commentaire (polling 2 s) ; passer à `ACTION=="add"` (à valider avec `qa-integration`) ; aucun `RUN+=`, `IMPORT{program}`, `$env{}`. | `grep -n "RUN+=\|IMPORT"` vide ; `udevadm test` ; pas de boucle (aucun processus lancé). |
| **V-13** | Info | `V4L2_DESIGN.md:341-345` (§4.3), `:863` (Annexe A) | La normalisation des noms est reprise de `cameractrls.py` (MIT). Le design ne mentionne pas la conservation de la notice MIT/attribution dans le fichier dérivé. | Ajouter un en-tête « dérivé de cameractrls.py (MIT), copyright … » dans `control.rs` (ou documenter la provenance) ; vérifier la compatibilité GPL-3.0-or-later (MIT est compatible). | Revue licence/CI (`cargo deny` ne couvre pas le code recopié) ; relecture `security`. |
| **V-14** | Info | `V4L2_DESIGN.md:370-372` (§4.5), `:477-481` (§5) | Les chaînes du device (`name`, `card`, `bus_info`, libellés de menu) viennent du matériel et sont exposées en JSON/QML et dans les logs. Le design prévoit UTF-8 lossy et suppression des contrôles pour les menus ; il ne généralise pas aux noms/bus_info/erreurs. | Sanitiser **toutes** les chaînes device : borne 32/128 octets, suppression des contrôles (dont `\n`), pas de HTML côté QML, jamais de chaîne device dans un chemin. | Tests de noms piégés (`\n`, `\0`, UTF-8 invalide, 4 Kio) ; revue QML (`textFormat`). |
| **V-15** | Info | `V4L2_DESIGN.md:299-305` (§4.2) | Les 5 ioctl sont acceptés (voir §1) ; `REVIEW_HARDWARE.md` §4 (4 ioctls) doit être amendé, pas réécrit : ajouter une note datée « QUERYMENU validé par security le 2026-11-06 ». | Note datée dans `REVIEW_HARDWARE.md` §4 et dans `DECISIONS.md` si une décision V4L2 y est ajoutée. | Revue documentaire. |
| **V-16** | Info | `V4L2_DESIGN.md:323-327` (§4.2), `:740-745` (§9.1) | Les tests ABI prévus couvrent `queryctrl` mais pas les offsets de `querymenu` (`payload=8`, `reserved=40`), ni `GRABBED`, ni `EINTR`. Une régression de layout passerait. | Ajouter des `static_assert`/tests : `size_of::<v4l2_querymenu>() == 44`, `align_of == 4`, offsets 8/40, `reserved = 0` avant l'appel, `GRABBED=0x0200`, `EXECUTE_ON_WRITE=0x0100`, boucle `EINTR` bornée. | Tests unitaires ; compilation croisée aarch64 si supportée. |

---

## 2bis. Implémentation lecture seule (2026-11-06)

### Constats vérifiés

| Élément | État | Preuve |
|---|---|---|
| **4 ioctl seulement** | **Conforme** | `abi.rs` définit `QUERYCAP`, `QUERYCTRL`, `QUERYMENU`, `G_CTRL` ; commentaire explicite « aucun `S_CTRL` » (`abi.rs:15-22`) ; test statique `no_write_or_streaming_ioctl`. |
| **Allowlist 15 CIDs** | **Conforme** | `control.rs:64` `ALLOWED_CIDS: [u32; 15]`, `cid_allowed` (`:88`), test `reads_only_allowlisted_controls` ; les CIDs énumérés hors liste sont ignorés. |
| **`unsafe` confiné** | **Conforme** | `real.rs` seul `#![allow(unsafe_code)]`, un helper `libc::ioctl` ; `abi.rs`/`control.rs`/`device.rs`/`fake.rs` `#![forbid(unsafe_code)]` ; garde `tests/v4l2_unsafe_guard.rs` (3 tests). |
| **Ouverture `O_RDONLY`** | **Conforme** | `real.rs:56-64` : `read(true)`, `O_NONBLOCK`, `O_NOFOLLOW`, `O_CLOEXEC` ; chemin reconstruit depuis `videoN`, jamais `device.path` ; `fstat` + `validate_node` (`device.rs:50-95`). |
| **Pas de flux** | **Conforme** | Aucun `REQBUFS`/`QBUF`/`STREAMON`/`mmap` ; garde statique ; aucun `read`/`write` de flux. |
| **Budget** | **Partiel** | `MAX_IOCTLS_PER_READ=512`, global par `read_all` (`mod.rs:55,310-347`), `truncated` ; menus lus paresseusement ; **pas de budget temporel** (V-18). |
| **`device_caps`** | **Conforme** | `effective_caps()` (`mod.rs:80-88`) ; test `capabilities_effective_device_caps`. |
| **Nœud** | **Conforme** | `validate_node` : caractère, major 81, minor sysfs, `uid == 0`, `O_NOFOLLOW` ; **world-write accepté en lecture seule** (`O_RDONLY`), `gid`/mode loggés (`device.rs:50-104`) ; `chown` non-root refusé. |
| **Lecture à la demande** | **Conforme** | `WebcamRuntime::controls_requested` exige `!dry_run` **et** le bail capteurs (`hardware/webcam.rs:399-407`) ; cache 30 s (`CONTROLS_REFRESH`) et ré-énumération si les nœuds changent (`:410-435`). |
| **Payload sans `id` brut** | **Conforme** | `UvcControlData` n'a pas de champ `id` (`hardware/webcam.rs:161-180`) ; test `unique_key_never_exposes_raw_id` ; QML ne reçoit que `key`. |
| **`dry_run`** | **Conforme** | `controls_requested` retourne faux en dry-run ; aucun ioctl de contrôle en `--dev-session` ; tests fake. |
| **Unit systemd** | **Conforme** | `tccd.service:66,69` : `DeviceAllow=/dev/tuxedo_io rw` **et** `DeviceAllow=char-video4linux r` ; `DevicePolicy=closed` ; `PrivateDevices=no`. Aucun `rw` vidéo (V-17 fermé). |

### Statut des findings

| ID | Statut | Preuve / réserve |
|---|---|---|
| **V-01** | **Fermé (lecture)** | Allowlist **normative** de 15 CIDs, testée ; l'écriture est traitée en **§2ter** (même allowlist). |
| **V-02** | **Hors jalon 2 (ouvert)** | Aucun `S_CTRL` n'est défini ; l'authz du futur setter reste à faire. |
| **V-03** | **Fermé (lecture)** | `O_RDONLY` implémenté et unité autorisant `char-video4linux r` (V-17 fermé). L'écriture est dans `tccd-user` (**§2ter**), `tccd` reste `r`. |
| **V-04** | **Partiel** | Budget en **nombre** (512) et `truncated` ; pas de budget temporel/stalled (V-18). |
| **V-05** | **Fermé (lecture)** | Budget global, menus paresseux, lecture à la demande via bail, cache 30 s ; tests `budget_truncates_long_control_lists`, `reads_menu_entries_lazily_and_maps_value`. |
| **V-06** | **Fermé** | `O_RDONLY` ; plus de `O_RDWR` dans le lecteur. |
| **V-07** | **Fermé** | `device_caps` autoritaire ; test. |
| **V-08** | **Fermé** | `NEXT_CTRL` seul, flags masqués (`mask_query_flags`). |
| **V-09** | **Fermé (Info, règle révisée)** | `0666` + ACL `uaccess` accepté en lecture ; justification `O_RDONLY` ; `chown` non-root refusé ; test `accepts_world_writable_node_for_read_only_use` ; preuve réelle `/dev/video0` (11 contrôles). Risque résiduel udev/ACL documenté. |
| **V-11** | **Fermé** | Aucun `id` dans le payload ; test `unique_key_never_exposes_raw_id`. |

### Nouveaux findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **V-17** | **Fermé** | `packaging/systemd/tccd.service:66,69` | Corrigé : `DeviceAllow=char-video4linux r` ajouté ; `DevicePolicy=closed` autorise la lecture, jamais l'écriture. Preuve réelle : `/dev/video0` ouvert en lecture, 11 contrôles. | Plus de blocage d'ouverture en production ; l'écriture est dans `tccd-user` (**§2ter**), `tccd` reste `r`. | Vérifier sur matériel (`G_CTRL`, `controls_supported=true`) ; `systemd-analyze security`. | Ouverture réelle + `strace` ; test fake conservé. |
| **V-18** | Faible / A | `v4l2/real.rs:86-108` ; `v4l2/mod.rs:55,250-347` | Le budget est un **compteur d'ioctl**, pas une échéance : un ioctl UVC bloqué immobilise le thread `spawn_blocking` du worker webcam ; aucun état `stalled`/cooldown. | Perte de la télémétrie webcam/contrôles jusqu'au timeout noyau (souvent ~5 s) ; pas de blocage du daemon principal (worker dédié). | Ajouter un `stalled` par device avec cooldown (comme le design), ou documenter que le timeout UVC noyau borne l'appel ; test `FakeReader` bloquant. | Test fake bloquant : le worker ne boucle pas, l'état expose `error`/`stalled`. |
| **V-19** | Info | `v4l2/control.rs:64` | L'allowlist lecture seule est **plus étroite** que la table §2.1 du design (15 CIDs : brightness/contrast/saturation/hue/gamma/exposure/gain/power line/WB température/sharpness/backlight/exposure abs/focus abs/focus auto/zoom). | Parité partielle (pas d'`exposure_auto`, `white_balance_automatic`, menus) tant que l'écriture n'est pas livrée ; assumé pour le jalon 2. | Documenter dans `FEATURE_PARITY`/`HARDWARE_MATRIX` ; étendre avec l'écriture, pas avant. | Revue de la liste. |

### Preuves exécutées

```text
cargo test -p tcc-hw --lib v4l2                 -> 39 passed
cargo test -p tcc-hw --test fake_v4l2           -> 8 passed
cargo test -p tcc-hw --test v4l2_unsafe_guard   -> 3 passed
cargo test -p tcc-hw --test real_v4l2 -- --ignored --nocapture -> 1 passed
```

**Preuve matérielle (2026-11-06, laptop TUXEDO réel)** : `/dev/video0` est
`crwxrwxrwx+ root video` (`0666` + ACL `uaccess`) ; `available=true`,
**11 contrôles** de l'allowlist lus (`brightness`, `contrast`, `saturation`, `hue`, `gamma`,
`gain`, `power_line_frequency`, `white_balance_temperature`, `sharpness`,
`backlight_compensation`, `exposure_time_absolute`), `truncated=false`, `error=None`.
`/dev/video2` (IR) : `available=true`, **0 contrôle** allowlist. Les nœuds metadata sont filtrés.
Le test opt-in `real_v4l2` (ignoré par défaut) est la preuve reproductible : il exige au moins un
nœud lisible et un contrôle, et imprime les 11 contrôles.

Aucun test QEMU/uvcvideo ; les tests d'écriture, `EBUSY`, LED, suspend et hotplug restent à faire.

---

## 2ter. Implémentation écriture UVC (2026-11-07)

### Constats vérifiés

| Élément | État | Preuve |
|---|---|---|
| Ouverture `O_RDWR` | **Conforme** | `real_write.rs:32,100-104` : `read+write` (O_RDWR), `O_NONBLOCK` / `O_NOFOLLOW` / `O_CLOEXEC` ; `fstat` sur le fd `:62-71` ; `validate_node` `device.rs:70-105` (caractère, major 81, minor sysfs, uid 0) ; test symlink `:220-228`. |
| `device_caps` capture | **Conforme** | `check_capabilities` `mod.rs:590-605`, `effective_caps` `:94-100` : refus meta-capture, index > 0, absence de `VIDEO_CAPTURE`. |
| Allowlist | **Conforme** | `control.rs:64-95` : **15 CIDs** exacts, `cid_allowed` ; test `allowlist_is_exactly_the_documented_set` `:361-373`. |
| Types et flags | **Conforme** | `write.rs:304-318` ; `kind_from_raw` `control.rs:268-277` ; `is_writable` `:226-228` refuse `DISABLED`/`GRABBED`/`READ_ONLY`/`WRITE_ONLY`/`INACTIVE` ; `is_action_or_payload` `:232-234` ; test `:423-441`. |
| Séquence ioctl | **Conforme** | `find_control` `write.rs:278-324` ; `validate_requested_value` `:327-393` ; `G_CTRL` avant `:198`, `S_CTRL` `:209-213`, relecture `:214-220`. |
| Budget / verrou / quarantaine | **Conforme (UVCW-02 fermé, atténué)** | 64 ioctl `:48,619-633` ; **porte globale `gate: Mutex<()>`** `:115-186` (une seule séquence par processus) ; `try_lock` par device `:188-195` ; `EBUSY`/`EIO` `:496-513` ; quarantaine 30 s bornée à 16 avec éviction de la plus ancienne `:51,298-310`. Le délai 2 s reste advisory autour d'un ioctl bloquant : la porte empêche l'empilement, pas un blocage noyau unique. |
| Isolation `unsafe` | **Conforme (source)** | `write.rs` `#![forbid(unsafe_code)]` `:32` ; `real_write.rs` `#![allow(unsafe_code)]` `:13` ; `S_CTRL` uniquement `abi.rs`/`real_write.rs` ; garde `tests/v4l2_unsafe_guard.rs` **4/4**. |
| Isolation compilation `tccd` | **Accepté (dette documentée, UVCW-01)** | `v4l2/mod.rs:44-45` compile `real_write`/`write` sans `cfg` ; `tccd` dépend de `tcc-hw` (`tcc-daemon/Cargo.toml:18`) et embarque la dépendance. `tccd` ne référence aucun symbole du writer dans ses sources et n'appelle jamais `S_CTRL` ; `DeviceAllow=char-video4linux r` (`tccd.service:69`) bloque l'ouverture write. Test de graphe/symboles reporté (note ci-dessous). |
| Service `User1` | **Livré, conforme aux correctifs** | `session_service.rs:380-487` (`SetUvcControl`, `spawn_blocking`, erreurs typées) ; **corps D-Bus borné 64 Kio avant désérialisation** `:493-530,615-647` ; bus de session `:736-743` ; rate limit UID `:409-432` ; device énuméré `uvc.rs:357-365` ; **cache 1 s** `GetUvcWriteSupported` `uvc.rs:45-51,328-347` ; **EACCES/EPERM → NotSupported** `uvc.rs:439-478`. |

### Findings écriture

| ID | Sév. | Fichier:ligne | Constat | Remédiation | Test |
|---|---|---|---|---|---|
| **UVCW-01** | **Moyen / architecture** | `v4l2/mod.rs:44-45` ; `tcc-hw/src/lib.rs:88` ; `tcc-daemon/Cargo.toml:18` ; `tcc-user/tests/uvc_write_contract.rs:499-526` | Le module d'écriture est compilé sans feature/`cfg` ; `tccd` dépend de `tcc-hw` et compile donc `real_write`/`write`. Le test « tccd ne référence pas UVC write » ne vérifie que les sources, pas le graphe de dépendances ni le binaire. `DeviceAllow=char-video4linux r` empêche l'écriture noyau, donc pas d'exploitation directe. | Feature `uvc-write` activée seulement par `tcc-user`/tests, ou crate séparée ; test `cargo tree -e features`/symbole ; corriger la revendication d'isolation. | `cargo tree -e features -p tcc-daemon` sans `uvc-write` ; absence de `UvcWriterController` dans le binaire (test CI). |
| **UVCW-02** | **Moyen / dispo** | `write.rs:445-467` ; `real_write.rs:76-97` ; `session_service.rs:439-440` ; `tccd-user.service:61-62` | `WRITE_TIMEOUT` n'est vérifié qu'**avant** chaque ioctl ; `O_NONBLOCK` ne rend pas `S_CTRL` interruptible. Un pilote bloqué tient le thread `spawn_blocking` et le `try_lock` du device ; plusieurs devices peuvent consommer les `TasksMax=32`. | Semaphore global d'écritures en vol (1–2), thread dédié par device avec watchdog, `Stalled` sur échéance, `TasksMax`/`MemoryMax` revus ; documenter l'ioctl non annulable. | Fake bloquant : deuxième appel `Busy`/`Stalled`, aucun thread supplémentaire ; test 16 devices. |
| **UVCW-03** | **Faible** | `write.rs:505-532` ; `real_write.rs:46-72` | Le chemin est reconstruit depuis sysfs puis ouvert ; `fstat` vérifie caractère/major/minor/uid, mais aucune identité stable (`bus_info`, vendor/product/serial) n'est comparée. Un device remplacé avec le même minor entre la lecture sysfs et l'ouverture peut recevoir l'écriture. Après ouverture, le fd est stable. | Comparer `QUERYCAP.bus_info`/identité USB à l'énumération, ou passer un token opaque et ouvrir une seule fois ; test fake avec `bus_info` différent. | Fake swap : sysfs `video0` mais `bus_info` différent ⇒ refus avant `S_CTRL`. |
| **UVCW-04** | **Faible** | `write.rs:198-220` | Après `S_CTRL`, la relecture `G_CTRL` peut différer (arrondi/clamp firmware) ; le code retourne `InvalidValue` sans restaurer `previous`. | Restaurer `previous` en best-effort (budget), retourner `PartialApply`/`WriteFailed` avec la valeur réelle, ou un rapport `changed=true` avec la valeur constatée ; test clamp. | Fake qui clampe 42→40 : `previous` restauré ou rapport explicite. |
| **UVCW-05** | **Faible** | `write.rs:377-390` ; `real_write.rs:137-158` | Pour `INTEGER_MENU`, `QUERYMENU` est appelé avec la valeur demandée comme `index` et le `MenuQuery.value` retourné est ignoré ; une valeur qui est un index valide mais pas une valeur de menu peut passer. | Pour `IntegerMenu`, comparer `MenuQuery.value` à la valeur demandée (ou parcourir les entrées) avant `S_CTRL` ; test menu sparse. | Fake `integer_menu(&[(0, 10), (1, 20)])`, demande 1 ⇒ refus avant écriture. |
| **UVCW-06** | **Faible / dispo (service)** | `session_service.rs:380-440` ; `uvc.rs:330-333,413-431` | `device_path`/`control_key` sont des `String` zbus désérialisées avant la validation de taille ; un corps D-Bus volumineux peut allouer. `GetUvcWriteSupported` n'est pas rate-limité ; `EACCES` à l'ouverture tombe dans `Failed`, pas `NotSupported`. | Enveloppe bornée (64 Kio) avant désérialisation, rate limit `GetUvcWriteSupported`, mapper `EACCES`/`PolicyDenied` en `NotSupported`/`PermissionDenied` ; test corps 1 Mio. | D-Bus avec `ss` 1 Mio ⇒ `InvalidArgs` sans allocation ; `chmod 000` ⇒ `NotSupported`. |
| **UVCW-07** | Info | `write.rs:254-260` | `mark_stalled` vide **toute** la map si elle atteint 64 entrées, alors que `MAX_DEVICES=16`. | Évincer l'entrée la plus ancienne, ne jamais vider globalement ; test. | 64 devices fake ⇒ seules les plus anciennes sont oubliées. |
| **UVCW-08** | Info | `device.rs:77,89-94` ; `real_write.rs:71` ; `V4L2_DESIGN.md` §4.1 | `validate_node` ignore le major sysfs (`Some((_, expected_minor))`) et exige `uid == 0` pour l'écriture, alors que le design dit que `tccd-user` n'a pas cette exigence (le nœud `root:video` est normal). L'autorisation réelle reste l'ouverture `O_RDWR` (ACL/`video`). | Comparer major **et** minor sysfs ; ajouter `NodeUse::Read/Write` ; aligner le commentaire « validé pour lecture » et le design. | Fake `dev=80:0` ⇒ refus ; nœud `chown user` ⇒ refus documenté. |

### Statut des correctifs UVCW (2026-11-07)

| ID | Statut | Preuve / reste |
|---|---|---|
| **UVCW-01** | **Accepté (dette documentée)** | Module `write`/`real_write` toujours compilé dans `tcc-hw` ; `tccd` ne référence aucun symbole du writer dans ses sources et n'appelle jamais `S_CTRL` ; `DeviceAllow=char-video4linux r`. Le test de graphe de features/symboles est **reporté** à `packaging-release`/`linux-platform` ; la revendication est « jamais appelé par `tccd` », pas « jamais compilé ». Ne bloque pas le packaging dans cet état. |
| **UVCW-02** | **Fermé (atténué)** | Porte globale `Mutex<()>` `write.rs:115-186` couvrant toute la séquence ; appels concurrents `Busy` ; test deux devices `global_gate_serializes_different_devices` `:1146-1181`. Résidu documenté : un ioctl noyau bloqué tient **un** thread et la porte ; pas de watchdog annulable. |
| **UVCW-03** | **Fermé** | `verify_identity` `write.rs:520-582` compare `driver` (`uevent DRIVER=`), `card` (préfixe sysfs) et `bus_info` dérivé de `/sys/.../device` après `QUERYCAP` ; mismatch ⇒ `Stalled` avant `S_CTRL`. Tests `identity_mismatch_is_stalled_before_write` / `matching_identity_allows_write` `:945-972`. |
| **UVCW-04** | **Fermé** | Sur relecture différente, restauration best-effort de `previous` `write.rs:244-263`, erreur `InvalidValue` contenant après/demandé/précédent et l'état de restauration. Test `readback_mismatch_attempts_restore_and_reports` `:1030-1057`. |
| **UVCW-05** | **Fermé** | `INTEGER_MENU` parcourt `min..=max` et compare `MenuQuery.value` `write.rs:444-467`. Test `integer_menu_matches_values_not_indexes` `:896-924`. |
| **UVCW-06** | **Fermé** | Bornes 64 octets validées avant usage coûteux (`uvc.rs:400-437`) ; corps D-Bus **64 Kio** avant désérialisation (`session_service.rs:493-530,615-647`) ; cache **1 s** de `GetUvcWriteSupported` (`uvc.rs:45-51,328-347`) ; `EACCES`/`EPERM` → `NotSupported` (`uvc.rs:439-478`). Tests contract **9/9**, cache et permission. |
| **UVCW-07** | **Fermé** | `MAX_TRACKED_DEVICES = MAX_DEVICES = 16` ; éviction de l'entrée la plus ancienne, plus de vidage global (`write.rs:298-310`). Test `stalled_map_evicts_oldest_not_everything` `:1095-1111`. |
| **UVCW-08** | **Fermé fonctionnellement / commentaire résiduel** | Major **et** minor sysfs comparés (`device.rs:81-98`), test `rejects_wrong_type_major_minor_or_owner` `:228-231`. Reste cosmétique : l'en-tête `device.rs:21-23` et le log `:112` parlent encore de « lecture » ; à aligner par le propriétaire. Aucun impact sécurité. |

### Verdict écriture

- **Aucun Critique/Élevé.**
- **UVCW-02…08 : fermés** (UVCW-08 : commentaire résiduel cosmétique) ; preuves ciblées ci-dessous.
- **UVCW-01 : accepté comme dette documentée** ; `tccd` ne référence/câble aucun writer et n'appelle jamais `S_CTRL`, `DeviceAllow=char-video4linux r` ; test de graphe/symboles reporté ; **ne bloque pas le packaging** dans cet état.
- **1. Sliders GUI en release mono-utilisateur : OUI, sous conditions** : `GetUvcWriteSupported` doit piloter l'affichage, le service de session doit être présent, et un smoke test caméra réelle doit être fait avant de revendiquer l'écriture ; sinon exposer les contrôles en lecture seule.
- **2. Bloquants restants** :
  - **écriture réelle caméra** : pas de blocage code, mais **preuve manuelle requise** avant de déclarer les sliders supportés ;
  - **watchdog** : **non bloquant** ; la porte globale limite à une séquence par processus, un ioctl noyau bloqué tient toutefois un thread/la porte — résidu documenté, watchdog annulable hors périmètre ;
  - **multi-session `try_lock`** : **non bloquant sécurité** ; le verrou est par processus, le noyau arbitre deux sessions ; à tester/documenter (pas d'escalade) ;
  - **ACL** : **non bloquant code** ; l'ouverture `O_RDWR`/ACL `uaccess`/`video` est l'autorisation ; `EACCES`/`EPERM` → `NotSupported` ; tester ACL perdue/siège.
- **3. UVCW-01** : **accepté, ne bloque pas le packaging** tant que `tccd` reste sans référence/call path vers le writer et `DeviceAllow=char-video4linux r` ; réévaluer si `rw` ou un chemin `tccd` est ajouté.

### Conditions d'acceptation du service `User1`

1. **Bus de session uniquement** (`com.tuxedocomputers.tccd.User`, `User1`), même UID que la GUI ; aucune inscription sur le bus système, aucun polkit.
2. **Device réellement énuméré** : `device_path` doit être `/dev/videoN` canonique et présent dans la dernière énumération `tcc-hw` (`uvc.rs:357-365`) ; metadata, `..`, zéros de tête et chemin inventé refusés.
3. **Clé allowlist** : `controlKey` ASCII `[a-z0-9_]{1,64}` ; le backend ré-énumère et ne trouve que les 15 CIDs ; aucun CID brut ni clé privée acceptés.
4. **Rate limit par UID** avant `spawn_blocking` (20/s, burst 20) ; fallback nom unique si l'UID est introuvable ; map bornée.
5. **Erreurs typées** : `InvalidArgs`, `NotSupported`, `OutOfRange`, `Busy`, `Stalled`, `Failed`, `LimitsExceeded` ; mapper `EACCES`/`PolicyDenied` en `NotSupported`/`PermissionDenied`, jamais de fuite d'errno/path brut.
6. **Concurrence bornée** : sémaphore global d'écritures en vol, `try_lock` par device, watchdog/`Stalled` ; `TasksMax=32`/`MemoryMax=128M` compatibles avec un ioctl bloqué (UVCW-02).
7. **`writeSupported` honnête** : faux si backend absent ou aucun device énuméré ; la GUI ne montre les sliders que si le service de session répond ; aucun repli root.
8. **ACL `uaccess`/groupe `video`** : l'ouverture `O_RDWR` est la seule autorisation ; `tccd-user` sans capabilities, `NoNewPrivileges=yes`, aucun `DeviceAllow` ; `tccd` reste `DeviceAllow=char-video4linux r`.
9. **Corps D-Bus borné** avant désérialisation (UVCW-06) et audit `caller_uid`/device/clé/résultat sans donnée d'image.
10. **Validation matérielle** : au moins une caméra réelle (interne + USB), `EBUSY` avec aperçu, ACL perdue, deux sessions, suspend/hotplug.

### Preuves écriture

```text
cargo test -p tcc-hw                               -> 284 passed (265 lib + 19 intégration)
cargo test -p tcc-hw --lib v4l2                    -> 62 passed
cargo test -p tcc-hw --test fake_v4l2              -> 8 passed
cargo test -p tcc-hw --test v4l2_unsafe_guard      -> 4 passed
cargo test -p tcc-user                             -> 169 passed (122 lib + intégration)
cargo test -p tcc-user --lib uvc                   -> 12 passed
cargo test -p tcc-user --lib rate_limit            -> 6 passed
cargo test -p tcc-user --test uvc_write_contract   -> 9 passed
cargo test --workspace                             -> 1527 passed
```

**Non exécuté** : écriture réelle sur caméra, `EBUSY` réel, LED, suspend/hotplug, deux sessions, ACL perdue, vrai pilote bogué, tests QML des sliders.

> **Note d'acceptation UVCW-01 (2026-11-07, `tuxedo-hardware`)** : l'isolation
> de compilation est **acceptée comme dette documentée** pour ce lot.
> `tcc-hw` reste compilé avec `write.rs`/`real_write.rs` chez tous ses
> consommateurs, donc `tccd` embarque la dépendance. En revanche, `tccd` ne
> référence aucun symbole du writer et n'appelle jamais `S_CTRL` (garde source
> `v4l2_unsafe_guard`, `DeviceAllow=char-video4linux r`). La revendication
> « `S_CTRL` jamais compilé » est remplacée par « jamais appelé par `tccd` ».
> Un test de graphe de features/symboles n'est pas ajouté ici (fragile sur
> binaire strippé) ; il est reporté à `packaging-release`/`linux-platform` si
> une feature `uvc-write` ou une crate séparée est retenue.
> **Décision `security` : accepté comme risque documenté ; ne bloque pas le
> packaging** tant que `tccd` ne référence/câble aucun writer et conserve
> `DeviceAllow=char-video4linux r`.

---

## 3. Conformité point par point

| Point demandé | Verdict | Preuve / réserve |
|---|---|---|
| **1. ABI / unsafe** | **Conforme avec réserves** | 5 ioctls exacts, vérifiés par compilation contre `/usr/include/linux/videodev2.h` (Annexe A) : `0x80685600/0xC008561B/0xC008561C/0xC0445624/0xC02C5625`, tailles 104/8/8/68/44, offsets `queryctrl` 8/40/44/48/52/56. `v4l2_querymenu` en `[u8;32]` reproduit le layout `packed` du noyau (taille 44, `reserved` à 40) sans union Rust. `unsafe` confiné à `real.rs`, `deny` au crate, `forbid` sur les modules sûrs. Réserves : static asserts `querymenu`, masquage des flags, `EINTR` (V-08, V-16). |
| **2. Ouverture du device** | **Conforme** | Nom dérivé de `webcam::enumerate` (`valid_video_name`, `webcam.rs:161-164`), chemin construit, **`O_RDONLY`** + `O_NONBLOCK` + `O_NOFOLLOW` + `O_CLOEXEC` (`real.rs:56-64`), `fstat` (type, major 81, minor sysfs, `uid == 0` ; world-write accepté en lecture seule), `QUERYCAP` capture ; `DeviceAllow=char-video4linux r` (V-17 fermé). Preuve réelle `/dev/video0` `0666`+ACL acceptée. |
| **3. Allowlist CID** | **Conforme (lecture + écriture session)** | `ALLOWED_CIDS` de **15** CIDs, `cid_allowed` normatif (`control.rs:64-95`), test `reads_only_allowlisted_controls` ; l'écriture `find_control` filtre la même liste et les flags (`write.rs:304-318`) ; §2ter. |
| **4. Privilèges** | **Conforme (lecture + session)** | `tccd` a `PrivateDevices=no`, `DevicePolicy=closed`, `DeviceAllow=/dev/tuxedo_io rw` **et** `DeviceAllow=char-video4linux r` (`tccd.service:66,69`) : lecture UVC possible, aucune écriture. L'écriture est dans `tccd-user` (bus de session, non privilégié) ; §2ter. |
| **5. Exposition D-Bus** | **Conforme (session) / repli root fermé** | `User1.SetUvcControl` est livré sur le **bus de session** (même UID, aucun polkit) ; `GetWebcamDataJSON` reste en lecture système. `Manager1.SetWebcamControl` (repli root) reste **fermé** : ne pas l'ouvrir sans `CheckAuthorization` (V-02) ; la policy refuse les setters `Manager1` (`conf:111-138`). Persistance : **uniquement** `Admin1` (V-10), jamais `Manager1`. |
| **6. udev / hotplug** | **Conforme avec réserves** | Règle tag-only, aucun `RUN+=`/`IMPORT{program}`/shell ; ré-énumération 2 s ; pas de boucle possible (aucun processus lancé). Commentaire faux (H-12) et `add|change` à restreindre (V-12). |
| **7. Tests** | **Partiel (lecture + écriture fake)** | 56 tests lib V4L2 + 8 `fake_v4l2` + 4 garde statique + 10 `tcc-user` uvc + 6 rate_limit + 6 contract ; ABI, allowlist, `device_caps`, budget, EINTR, menus, flags, `try_lock`, `stalled`, relecture, unsafe, no-streaming. Manquent : matériel réel, ioctl bloqué, écriture réelle, LED, hotplug, deux sessions ; §2ter. |
| **8. Licence / dépendances** | **Conforme** | Module local GPL-3.0-or-later recommandé ; `v4l`/`v4l2-sys-mit` (MIT) et `v4l2r` (MIT/Apache-2.0) sont compatibles GPL-3.0-or-later, mais ajoutent du FFI généré et une grande surface `unsafe` ; `v4l2-ctl` (GPL-2.0+, commande externe) et `cameractrls.py` (Python) sont rejetés à juste titre. Réserve : attribution MIT pour la normalisation recopiée (V-13). |

---

## 4. Décisions demandées

| # | Question du design | Décision `security` |
|---|---|---|
| 1 | 5 ioctls au lieu de 4 | **Accepté.** `VIDIOC_QUERYMENU` est nécessaire aux menus ; aucun ioctl de flux. Mettre à jour `REVIEW_HARDWARE.md` §4 par une note datée (V-15). |
| 2 | CID privés : refus définitif ou table phase 4 ? | **Refus par défaut en phase 1–3**, table statique `(vendor,product) -> &[u32]` en phase 4, chaque entrée revue par `security`. Un préréglage avec clé privée est ignoré avec un log unique. |
| 3 | Préréglage par profil ou par device ? | **Par device d'abord** (parité) ; le champ `webcam.preset` par profil est une extension phase 3 à valider par `rust-core`, pas une condition de la phase 1–2. |
| 4 | `Manager1` volatile ou tout `Admin1` ? | **Volatile dans `Manager1` uniquement avec authz interne** (V-02) ; **persistant dans `Admin1`** exclusivement. Ne pas rouvrir la policy avant l'authz. |
| 5 | Hotplug : polling ou netlink ? | **Polling 2 s en phase 1–3** (aucune dépendance, aucun privilège) ; netlink (`neli`, MIT) en phase 4 si la latence est un problème, avec revue de dépendance. |
| 6 | `DeviceAllow` : `char-video4linux rw` ? | **`r` dans `tccd`** (lecture seule) ; **pas de `rw` dans `tccd`** si l'écriture va dans `tccd-user` (V-03). Si le repli `tccd` root est retenu : `rw` + V-01 + V-02 + revue du delta. `PrivateDevices=yes` : non. |
| 7 | Où écrire les contrôles ? | **Recommandé : `tccd-user`** (session, non privilégié, ACL uaccess ou groupe `video`). **Repli acceptable : `tccd`** avec authz + allowlist. **Rejeté : `tccd` sans authz, ou `tccd-admin`.** |

---

## 5. Actions bloquantes

1. **V-17 — fermé** : `DeviceAllow=char-video4linux r` présent (`tccd.service:69`) ; reste la preuve matérielle (`G_CTRL`, `controls_supported=true`, `strace`).
2. **V-02/V-03 — écriture** : **tranché** : `tccd-user` (session, non privilégié), `tccd` reste `O_RDONLY`/`DeviceAllow=char-video4linux r` ; ne pas ouvrir `Manager1.SetWebcamControl` (repli root) sans authz polkit + allowlist + revue. **UVCW-02…08 fermés ; UVCW-01 accepté comme dette documentée** ; voir **§2ter**.
3. **V-18 — cycle de vie** : budget temporel/`stalled` pour un ioctl bloqué ; test `FakeReader` bloquant ; V-19 (allowlist 15) est une limite de parité, pas un blocage.
4. **ABI** : static asserts `querymenu` (44/8/40), `GRABBED`, `EINTR`, masquage des flags ; test d'énumération `NEXT_CTRL` jusqu'à `EINVAL` (déjà couvert pour l'essentiel).
5. **Documentation** : corriger le commentaire udev (H-12), `ACTION=="add"`, l'action polkit `configure` (V-10), pointer la décision 5 ioctls (V-15) et la lecture seule 15 CID (V-19).
6. **Tests et matériel** : plan §6, `cargo deny`/`audit`, revue `unsafe` ligne à ligne, attribution MIT (V-13).

---

## 6. Plan de validation matérielle

| Étape | Matériel / environnement | Vérification | Preuve attendue |
|---|---|---|---|
| 1 | Caméra UVC interne (Chicony/1bcf) + USB Logitech C920/Brio | `v4l2-ctl --list-ctrls` vs `GetWebcamDataJSON` : mêmes contrôles, bornes, menus | diff JSON, `journalctl`, aucune LED à l'énumération |
| 2 | Caméra avec `PRIVACY`, focus auto, menus *sparse* | écriture `brightness`, `contrast`, `white_balance_temperature`, `exposure_auto` ; relecture ; refus des CIDs hors allowlist | valeurs relues, logs, `EINVAL`/`NotSupported` attendus |
| 3 | Aperçu Qt Multimedia actif | écriture pendant le streaming : `EBUSY` ou succès selon pilote ; back-off 1/2/4/8 s, 3 essais max | logs, pas de boucle, pas de blocage du worker |
| 4 | Débranchement/rebranchement à chaud | préréglage appliqué **une fois**, pas de double application | compteur d'ioctls, logs `device`, état `pending` |
| 5 | Suspend/reprise | device ré-énuméré, préréglage réappliqué, pas de thread orphelin | `systemctl suspend`, logs, compteur de threads |
| 6 | Deux sessions / bascule de siège | `tccd` (r) lit, `tccd-user` écrit ; ACL perdue ⇒ `EACCES`, pas de retry infini | test multi-utilisateur, `loginctl` |
| 7 | Permissions | `chmod 0666 /dev/videoN` ⇒ **accepté en lecture** (cas réel `0666`+ACL) ; `chown` non-root ⇒ refus ; symlink `/dev/videoN` ⇒ `O_NOFOLLOW` refuse ; major/minor faux ⇒ refus ; vérifier que seul `O_RDONLY` est ouvert | tests + `strace -e openat,fstat` ; preuve `/dev/video0` 11 contrôles |
| 8 | `DeviceAllow` | `tccd` avec `char-video4linux r` : lecture OK, `S_CTRL` `EPERM` ; `tccd-user` écrit ; `systemd-analyze security` | scores et logs |
| 9 | `strace` | exactement les 5 ioctl sur le device, aucun `REQBUFS/QBUF/STREAMON/read/mmap` | trace filtrée |
| 10 | Pilote bogué / device piégé | `FakeTransport` + si possible un device qui renvoie `EIO`/bornes incohérentes : pas de panic, `unsupported`/`stalled` | logs, état UI |
| 11 | Clevo / Uniwill / AMD / Intel / NVIDIA, noyaux 6.x, X11 + Wayland | aucune régression du kill switch, des profils ou de la veille | rapport signé |

**Critères d'arrêt** : aucun panic, aucun accès hors des 5 ioctl, aucune écriture hors du device, aucun
thread non borné, `controlsSupported` honnête, LED non allumée par la seule énumération (ou documenté).

---

## 7. Limites de la revue

- **Code lecture seule existant** : la revue de design porte sur `V4L2_DESIGN.md`, mais l'étape 1
  (`crates/tcc-hw/src/v4l2/**`) est implémentée ; l'ABI a été vérifiée par compilation contre le
  header local, pas contre toutes les versions supportées. Les valeurs `_IOC` sont stables, mais un
  test CI de compilation reste requis.
- **Test matériel partiel** : la **lecture** est validée sur une caméra réelle (`/dev/video0`,
  `0666`+ACL, 11 contrôles ; `video2` IR 0). Restent à prouver : LED, `EBUSY`, suspend, hotplug,
  bascule de siège/ACL, et tout le chemin d'écriture. Aucun test QML, aucun vrai polkit, aucun vrai
  bus système.
- **Pas de fuzzing kernel** : les ioctl sont testés contre un `FakeTransport`, pas contre `uvcvideo`.
  Un pilote bogué peut encore produire des valeurs incohérentes ; la relecture et les bornes limitent
  l'impact, pas les bugs du noyau.
- **Choix d'architecture tranché (2026-11-07)** : écriture dans `tccd-user` (session), `tccd`
  lecture seule ; **UVCW-02…08 fermés**, **UVCW-01 accepté comme dette documentée** ; restent la
  preuve caméra réelle, le watchdog (résidu documenté) et les tests multi-session/ACL.
- **Multithread** : le design ne décrit pas le pool `spawn_blocking` de `tccd` ni son interaction avec
  `TasksMax` ; à vérifier avec `linux-platform`.
- **Licence** : l'attribution MIT de la normalisation recopiée doit être confirmée par une revue
  juridique légère ; `cargo deny` ne voit pas le code recopié.

### 7.1 Cohérence documentaire

- `V4L2_DESIGN.md` est aligné sur la nouvelle règle en §4.1 (`:453-462`) et dans le plan manuel
  (`:1214` : `chmod 0666` accepté en lecture). **Une phrase reste incohérente** à `:1161`
  (« `0666`/`chown` non-root refusés par `tccd` ») : elle doit être corrigée par le propriétaire en
  « `0666` accepté en lecture, `chown` non-root refusé ». Ce n'est pas modifié ici (périmètre
  `docs/security/**`).
- `CHECKLIST.md` a été mis à jour : V4L2 testé sur matériel réel (`/dev/video0`, 11 contrôles),
  V-09 fermé avec la règle `O_RDONLY`, V-17 fermé, compteurs de tests réels ; **écriture de session
  livrée** (`write.rs`/`real_write.rs`, `User1.SetUvcControl`) et suivie en **§2ter**.
- `REVIEW_V4L2.md` (ce document) est la référence de la règle permissions : lecture `uid 0`,
  caractère, major/minor, `O_NOFOLLOW`, `O_RDONLY`, world-write non refusé ; écriture session
  `O_RDWR` avec ACL `uaccess`/`video`, `fstat` identique ; **UVCW-02…08 fermés, UVCW-01 accepté**,
  preuve caméra réelle à faire.

---

## Annexe A — Preuve ABI (compilation locale, 2026-11-06)

```text
$ cc -Wall -Wextra -o v4l2abi v4l2abi.c && ./v4l2abi
QUERYCAP=0x80685600 size=104
G_CTRL=0xc008561b size=8
S_CTRL=0xc008561c size=8
QUERYCTRL=0xc0445624 size=68 off_name=8 off_min=40 off_max=44 off_step=48 off_def=52 off_flags=56
QUERYMENU=0xc02c5625 size=44 off_index=4 off_reserved=40
```

Le source compile `#include <linux/videodev2.h>` et imprime `sizeof`/`offsetof`. Il confirme :
- les 5 constantes du design (§4.2, `V4L2_DESIGN.md:299-305`) ;
- les tailles 104/8/68/44 et les offsets `queryctrl` ;
- que `v4l2_querymenu` fait **44 octets** avec `reserved` à **40**, ce que la structure Rust
  `{ u32, u32, [u8;32], u32 }` reproduit sans `packed` (taille 44, alignement 4). Un `u64` nu
  aurait donné 24 octets (et perdu le nom) ; une union Rust `[u8;32]`/`u64` sans `packed` aurait
  donné 48 octets/alignement 8 et changé la constante ioctl. Le `[u8;32]` du design est le bon
  compromis.

## Annexe B — Rappel des contraintes projet applicables

- D1 : un bug de parsing dans `tccd` ne doit pas devenir une primitive root ; d'où V-03.
- D2 : état mutable dans `/var/lib`, `/etc` en lecture seule ; `webcam.json` 0644 root.
- D7 : aucun override de développement dans les paquets ; le module V4L2 n'en introduit pas.
- AD-8 : les setters `Manager1` restent sur `tccd` et seront rouverts par un polkit interne ; V-02
  applique cette règle.
