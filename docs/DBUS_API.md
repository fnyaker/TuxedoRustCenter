# API D-Bus — daemon, matrice matérielle et administration (phase 1)

Statut : API de transition. La couche JSON historique reste disponible ; une
interface typée `com.tuxedocomputers.tccd.Manager1` expose la matrice matérielle
(lecture complète, setters sûrs limités). Les payloads restent des chaînes JSON
validées côté daemon. `com.tuxedocomputers.tccd.Admin1` (service séparé, à la
demande) porte les écritures persistantes protégées par polkit, et
`com.tuxedocomputers.tccd.Control1` le rechargement root-only de `tccd`.

## Identifiants

| Élément | Valeur |
|---|---|
| Bus | système |
| Service | `com.tuxedocomputers.tccd` |
| Objet | `/com/tuxedocomputers/tccd` |
| Interface historique | `com.tuxedocomputers.tccd` |
| Interface typée | `com.tuxedocomputers.tccd.Manager1` |
| Interface de contrôle | `com.tuxedocomputers.tccd.Control1` (root uniquement) |
| Service admin (activé à la demande) | `com.tuxedocomputers.tccd.Admin` |
| Objet admin | `/com/tuxedocomputers/tccd/Admin` |
| Interface admin | `com.tuxedocomputers.tccd.Admin1` |
| Propriété | `ContractVersion` (`u`) : historique/`Manager1` = 1, `Admin1` = 1 (introspection uniquement ; préférer `Manager1.GetContractVersion` ou `Admin1.GetContractVersion`) |
| Signal | `StateChanged(s snapshot_json)` (interface historique) |

Le nom et l'interface historiques sont conservés pour faciliter la transition
depuis TUXEDO Control Center. Les interfaces `historique`, `Manager1` et
`Control1` coexistent sur le même objet ; `Admin1` vit dans un processus root
distinct (D1), activé par D-Bus et arrêté après 30 s d'inactivité.

## Cohabitation avec l'ancien TCC

L'ancien TCC peut être installé et occuper `com.tuxedocomputers.tccd`. Pour
tester la réécriture sans l'arrêter :

```sh
# daemon root, nom alternatif
tccd --bus-name com.tuxedocomputers.tccd.Dev

# clients non privilégiés
TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev tccd-user
TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev tcc-cli status
```

En production, l'ancien service doit être désactivé avant d'installer/démarrer
le nouveau : les deux ne doivent jamais écrire sysfs en même temps. Le nom
alternatif est un mécanisme de développement, pas une configuration supportée
à long terme.

Pour un développement sans root et sans conflit, le daemon peut utiliser le
bus de session :

```sh
cargo run -p tcc-daemon -- --dev-session --bus-name com.tuxedocomputers.tccd.Dev

TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev cargo run -p tcc-user
TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev cargo run -p tcc-cli -- status
```

`--dev-session` force le dry-run : aucune écriture matérielle.

Pour valider le daemon root sur le bus **système** sans écrire le matériel, un
`--dry-run` distinct existe (sans forcer le bus de session) :

```sh
# arrêter l'ancien TCC pour libérer com.tuxedocomputers.tccd
sudo systemctl stop tccd.service
cargo build --release -p tcc-daemon
sudo ./target/release/tcc-daemon --dry-run --start

# lectures/policy dans un autre terminal
tcc-cli status

# Ctrl-C puis restauration
sudo systemctl start tccd.service
```

- `--dry-run` exige root, utilise le bus système et le nom normal
  (`com.tuxedocomputers.tccd`, ou `--bus-name`), et ne quitte pas : seul
  `--check` affiche l'état et sort.
- Toutes les écritures matérielles sont neutralisées (CPU, backlight,
  ventilateurs, charge, lighting, ODM/TDP, NVIDIA cTGP, webcam, YCbCr).
- Les setters `Manager1` renvoient un **succès simulé** (`true`), mettent à
  jour l'état en mémoire et ne persistent rien.
- Les lectures, le bail capteurs, `GetShutdownTimerJSON`, `Control1.ReloadConfig`
  et `tccd-admin` (séparé) fonctionnent normalement.
- Limites : la migration/persistance de configuration `/var/lib`, l'inhibiteur
  de veille et la minuterie d'extinction logind restent actifs. Pour une
  validation passive, ne pas configurer `shutdownTime`.

## Méthodes

| Méthode | Entrée | Sortie | Notes |
|---|---|---|---|
| `Ping()` | — | `b` | Vivant. |
| `GetVersion()` | — | `s` | Version du daemon. |
| `GetStateJSON()` | — | `s` | `tcc_ipc::StateSnapshot` sérialisé. |
| `GetActiveProfileJSON()` | — | `s` | `tcc_ipc::ActiveProfile` ou `null`. |
| `GetProfilesJSON()` | — | `s` | `tcc_core::ProfileCollection` complet. |
| `GetSettingsJSON()` | — | `s` | `tcc_core::Settings`. |
| `GetFanDataJSON()` | — | `s` | État des ventilateurs (voir section dédiée). |
| `SetTempProfile(s id)` | id de profil (≤ 64 octets) | `b` | Override **global** temporaire jusqu'au prochain changement d'état. Rate-limité 5/s (burst 10) par nom unique, audité. Pas encore protégé par polkit ; déplacement vers `Manager1` planifié. |
| `ClearTempProfile()` | — | `b` | Supprime l'override. Même budget d'écriture que `SetTempProfile`. |

`SetTempProfile` et `ClearTempProfile` sont les seuls setters de l'interface de
compatibilité JSON. Ils sont **globaux** (ils affectent la machine, pas une
session utilisateur) et **non authentifiés par polkit** à ce stade : la
vérification polkit de `tccd` reste spécifiée mais non implémentée (R-04). Les
garde-fous actuels sont :

- identifiant borné à `MAX_PROFILE_ID_LEN` (64 octets) avant résolution ;
- token bucket par **nom unique D-Bus** : 5 écritures/s, burst 10, puis
  `org.freedesktop.DBus.Error.LimitsExceeded` ;
- `ClearTempProfile` partage le même budget ;
- audit journald : `caller` (nom unique), `caller_uid` (via
  `GetConnectionUnixUser`, échec non fatal) et `profile_id` ; le payload brut
  n'est jamais journalisé ;
- map des appelants nettoyée (TTL 5 min) et bornée à 1024 entrées.

Le déplacement de l'override temporaire vers `com.tuxedocomputers.tccd.Manager1`
avec une politique polkit dédiée est planifié. La portée reste globale tant que
l'agent de session ne fournit pas d'identité de session exploitable.

Les écritures **persistantes** ne passent pas par `tccd` : elles passent par
`Admin1` (`tccd-admin`) puis par le rechargement root-only `Control1`. Un
`SIGHUP` reste accepté comme mécanisme de service (systemd), mais n'est plus le
contrat privilégié utilisé par l'administration.

## Interface `com.tuxedocomputers.tccd.Manager1`

Cette interface est servie sur le même objet que l'interface historique.

`GetContractVersion()` est la voie supportée pour la négociation de contrat
côté `tccd` : elle renvoie `IPC_CONTRACT_VERSION` (= 1). La propriété
`ContractVersion` de l'interface historique reste déclarée pour
l'introspection, mais **les propriétés ne sont pas une API stable** : les
clients ne doivent pas dépendre de `Properties.Get`/`GetAll` (P-12). La policy
D-Bus peut retirer ces accès et n'autoriser que la méthode.

| Méthode | Entrée | Sortie | Notes |
|---|---|---|---|
| `GetContractVersion()` | — | `u` | Version du contrat IPC (`IPC_CONTRACT_VERSION` = 1). **Voie supportée** ; ne pas dépendre de la propriété. |
| `GetShutdownTimerJSON()` | — | `s` | État de la minuterie d'extinction (lecture seule, sans polkit) : `{enabled, scheduledAt, source, error}`. **Limité à 2/s (burst 5) par nom unique**, bucket indépendant du snapshot. Voir section dédiée. |
| `GetHardwareSnapshotJSON()` | — | `s` | `hardware::HardwareSnapshot` sérialisé en `camelCase`. Lecture sans collecte capteurs ; réarme le bail si active. **Limité à 2/s (burst 5) par nom unique** ; une lecture refusée ne réarme pas le bail. |
| `SetSensorDataCollectionStatus(b)` | `b` | `b` | `true` ouvre une session de collecte de **5 min max**, avec bail d'inactivité de 10 s ; `false` désarme immédiatement. Rate-limité (5/s, burst 10) et audité comme les setters matériels. |
| `GetSensorDataCollectionStatus()` | — | `b` | État du bail. Lecture bon marché, non limitée et sans effet sur le bail. |
| `SetChargingProfile(s)` | profil | `b` | Non persistant. Validé par `tcc-hw`, rate-limité, audité. Polkit `…tccd1.charging`. |
| `SetChargingPriority(s)` | priorité | `b` | Non persistant. Polkit `…tccd1.charging`. |
| `SetChargeStartThreshold(i)` | `0..=100` | `b` | Non persistant, cohérence début < fin vérifiée par `tcc-hw`. Polkit `…tccd1.charging`. |
| `SetChargeEndThreshold(i)` | `0..=100` | `b` | Non persistant. Polkit `…tccd1.charging`. |
| `SetChargeType(s)` | `Standard`, `Fast`, `Custom`, ... | `b` | Non persistant ; `Settings` ne stocke pas ce champ. Polkit `…tccd1.charging`. |
| `SetKeyboardStatesJSON(s)` | `Vec<KeyboardState>` JSON (≤ 64 Kio, ≤ 64 zones) | `b` | Non persistant, couleurs `-1` ou `0..=255`. Polkit `…tccd1.settings`. |
| `SetLightbarStatesJSON(s)` | `LightbarState` JSON ou `null` | `b` | Non persistant, couleurs `0..=255`. Polkit `…tccd1.settings`. |
| `SetFnLockStatus(b)` | `b` | `b` | Global, non persistant. Polkit `…tccd1.settings`. |

`SetSensorDataCollectionStatus` reste sans polkit (bail local sans écriture
matérielle, exception documentée) ; il est rate-limité par nom+UID, audité et
  refusé pendant la veille.

### Minuterie d'extinction — `GetShutdownTimerJSON`

Méthode de lecture, **sans polkit** et sans écriture. Le réglage reste
`Settings.shutdownTime` (`"HH:MM"` local, `null`/vide = désactivé), modifié
uniquement par `Admin1.SaveSettingsJSON` puis rechargé par `tccd`.

```json
{
  "enabled": true,
  "scheduledAt": "2026-11-06 22:30",
  "source": "settings",
  "error": null
}
```

- `enabled` : extinction planifiée et acceptée par logind.
- `scheduledAt` : échéance locale, ou `null`.
- `source` : `settings` (échéance issue des réglages), `logind` (réservé à une
  future lecture de l'état logind), `none`, ou `error` (réglage invalide ou
  logind en échec).
- `error` : message générique (`logind indisponible`, `heure d'extinction
  invalide`), ou `null` ; jamais de chemin ni de secret. Une heure invalide
  conserve `error=heure d'extinction invalide` même si l'ancienne échéance a
  été annulée avec succès ; `source=error` et `enabled=false`.
- Aucun setter D-Bus : la seule voie est `Settings.shutdownTime` via
  `Admin1`, ce qui évite un nouvel appel privilégié.
- `tccd` appelle logind (`ScheduleShutdown("poweroff", usec)` /
  `CancelScheduledShutdown`) sur le bus système ; ces méthodes ne sont pas
  exposées aux clients. Si logind est absent, l'état expose
  `enabled=false` et `error=logind indisponible`.
- La minuterie n'est pas réconciliée pendant la veille ; elle l'est à la
  reprise et après un reload.
- La lecture D-Bus est limitée à **2/s, burst 5** par nom unique
  (`LimitsExceeded` au-delà). Le bucket est séparé de
  `GetHardwareSnapshotJSON` ; les lectures internes du daemon ne sont pas
  limitées et aucun état n'est réarmé.

### Snapshot matériel

`GetHardwareSnapshotJSON` renvoie les domaines suivants, tous en lecture seule :

```json
{
  "charging": {
    "available": false,
    "profiles": [],
    "profile": null,
    "priorities": [],
    "priority": null,
    "startThreshold": null,
    "endThreshold": null,
    "thresholdsAvailable": false,
    "applied": true,
    "errors": []
  },
  "lighting": {
    "available": false,
    "enabled": true,
    "keyboard": {},
    "lightbar": {},
    "states": [],
    "lightbarState": null,
    "applied": true,
    "errors": []
  },
  "odm": {
    "available": false,
    "platformProfile": {},
    "tdpAvailable": false,
    "tdp": [],
    "interface": null,
    "moduleVersion": null,
    "applied": true,
    "errors": []
  },
  "sensors": {
    "enabled": false,
    "available": false,
    "cpu": {},
    "igpu": null,
    "dgpu": null,
    "gpus": []
  },
  "nvidia": {
    "available": false,
    "attributeAvailable": false,
    "nvidiaSmiAvailable": false,
    "ctgpOffset": null
  },
  "webcam": {
    "switchAvailable": false,
    "enabled": null,
    "controlsSupported": false,
    "devices": []
  },
  "fnLock": { "supported": false, "enabled": null },
  "prime": {
    "available": false,
    "platformSupported": false,
    "supported": false,
    "mode": null
  },
  "ycbcr": {
    "available": false,
    "ports": [],
    "pendingModeReapply": false
  }
}
```

Chaque domaine peut contenir des champs supplémentaires (erreurs, capacités
matérielles) définis par `crates/tcc-daemon/src/hardware/**` ; les champs
`available: false` sont normaux sur matériel non-TUXEDO.

### Collecte de capteurs

- Désactivée par défaut : aucun fichier capteur n'est lu tant que
  `SetSensorDataCollectionStatus(true)` n'a pas été appelé. La reprise après
  veille réinitialise également le bail à « désactivé » : la GUI doit le
  renouveler explicitement si elle collecte encore.
- **Bail d'inactivité de 10 s** : `true` l'arme, `false` le désarme
  immédiatement.
- **Plafond absolu de session : 5 min.** Il n'est pas repoussé par les
  lectures ni par un nouveau `true` tant que la session est active. À
  l'échéance, la supervision coupe la collecte (`sensors.enabled=false`, log
  warning). Un nouveau `SetSensorDataCollectionStatus(true)` explicite ouvre
  une nouvelle session de 5 min : la GUI peut collecter durablement en
  renouvelant le bail, sans qu'une boucle de lectures le fasse à sa place.
- `GetHardwareSnapshotJSON` **réarme uniquement le bail d'inactivité**, et
  seulement si la lecture est acceptée par le limiteur de lecture (2/s,
  burst 5 par nom unique). Une rafale de lectures ne peut donc ni maintenir la
  collecte indéfiniment ni dépasser le plafond de session.
- SetSensorDataCollectionStatus est rate-limité (5/s, burst 10) et audité
  (`caller`, `caller_uid`) comme les setters matériels.
- Une tâche de supervision (1 s, une seule tâche Tokio, sans thread dédié)
  applique les deux échéances.
- `GetSensorDataCollectionStatus` est une lecture atomique bon marché : non
  limitée et sans effet sur le bail.
- Le worker publie `sensors.enabled: false` et des valeurs vides quand la
  collecte est coupée ; le snapshot reste lisible.
- L'activation prend effet au tick capteurs suivant (2 s), sans I/O dans le
  handler D-Bus.

**Impact GUI** : une page dashboard peut lire le snapshot au plus ~2 fois/s
(burst 5) et doit renouveler explicitement `SetSensorDataCollectionStatus(true)`
lorsque `GetSensorDataCollectionStatus()` renvoie `false` (fin de session de
5 min). Les lectures seules ne suffisent plus à maintenir la collecte au-delà de
ce plafond ; c'est volontaire pour borner le coût `nvidia-smi`/sysfs.

### Setters : sécurité

- **Aucune persistance** : la modification est appliquée au matériel (sauf
  `--dev-session`) et à `RuntimeState.settings` en mémoire, puis la révision
  est incrémentée pour que les workers réconcilient.
- **Rate limit dédié** par nom unique D-Bus **et par UID** : 5 écritures/s,
  burst 10 partagé entre les connexions d'un même UID, puis
  `org.freedesktop.DBus.Error.LimitsExceeded`. Les entrées invalides ne
  consomment pas de jeton.
- **Un seul dialogue polkit global** : `tcc-authz::DialogGate`, un dialogue par
  appelant, attente bornée (2 s) puis `AccessDenied` générique (`result=busy`).
  Aucun cache d'autorisation.
- **Interrupteur global lighting** : si
  `keyboardBacklightControlEnabled=false`, `SetKeyboardStatesJSON` et
  `SetLightbarStatesJSON` sont refusés (`AccessDenied`) avant tout dialogue ou
  écriture. `SetFnLockStatus` reste indépendant : il commute une fonction
  clavier matérielle, pas le rétroéclairage.
- **Veille** : pendant `Preparing`/`Suspended`/`Resuming`, tous les setters
  `Manager1` sont refusés (`AccessDenied`, `result=sleeping`) avant polkit.
- **Erreurs matérielles génériques** : les chemins sysfs, valeurs et détails
  d'ioctl restent dans journald ; le client ne reçoit que
  `NotSupported`/`InvalidArgs`/`AccessDenied`/`Failed` avec un message fixe.
- **Audit de complétion** : après l'écriture (ou l'erreur), une ligne
  `result=ok|error`, `method`, `action`, `caller`, `caller_uid`,
  `duration_ms` est émise, sans payload ni valeur.
- **Audit journald** : `caller` (nom unique), `caller_uid`
  (`GetConnectionUnixUser`, échec non fatal), action. Les payloads JSON
  d'éclairage ne sont jamais journalisés.
- **Autorisation polkit** : chaque setter de charge/lighting/Fn-Lock appelle
  `tcc-authz` après validation et rate limit, avec le sujet
  `system-bus-name` de l'appelant et l'UID résolu. Actions :
  - `com.tuxedocomputers.tccd1.charging` : cinq setters de charge ;
  - `com.tuxedocomputers.tccd1.settings` : clavier, lightbar, Fn-Lock.
  polkit absent, en timeout ou refusé ⇒ `org.freedesktop.DBus.Error.AccessDenied`
  générique, aucune écriture, audit `result=denied`. Root n'est pas
  court-circuité. Aucun cache local.
- En `--dev-session` (bus de session, dry-run) uniquement, un authorizer
  permissif (`DevSessionAuthorizer`, `pub(crate)`) est injecté pour les tests ;
  la production utilise toujours polkit. Un `debug_assert` refuse un
  authorizer injecté sans `dry_run`.
- Membres que `packaging-release` doit ouvrir dans la policy D-Bus :
  `SetChargingProfile`, `SetChargingPriority`, `SetChargeStartThreshold`,
  `SetChargeEndThreshold`, `SetChargeType`, `SetKeyboardStatesJSON`,
  `SetLightbarStatesJSON`, `SetFnLockStatus`. Actions polkit associées :
  `…tccd1.charging` et `…tccd1.settings`.
- **Validation** : bornes de taille, plages de seuils, couleurs, type de
  charge connu, listes matérielles vérifiées par `tcc-hw` en mode réel.
- En `--dry-run` (et `--dev-session`), les setters valident puis mettent à
  jour l'état en mémoire sans écrire le matériel ni persister, et renvoient un
  **succès simulé** (`true`). En `--dry-run` sur le bus système, polkit reste
  réellement appliqué (validation de la policy) ; seul `--dev-session` utilise
  l'authorizer permissif.

### Domaines volontairement en lecture seule

Les écritures **TDP/PL1/PL2/PL4, ODM/`platform_profile`, NVIDIA cTGP, PRIME et
YCbCr 4:2:0** ne sont pas exposées sur `Manager1`. Elles sont réservées à
`tccd-admin` + polkit (D1/D5). De même, le kill switch webcam n'est pas exposé
comme setter sur `Manager1` ; les contrôles UVC restent lisibles dans le
snapshot et leur écriture est portée exclusivement par
`User1.SetUvcControl` (service de session non privilégié).

## Service `com.tuxedocomputers.tccd.Admin` — interface `Admin1`

Service **root distinct**, activé par D-Bus, sans état persistant, arrêté après
**30 s** sans appel. Il écrit uniquement
`/var/lib/tuxedo-rust-center/{profiles,settings}.json` ; `/etc` reste en
lecture seule. Aucune méthode n'accepte de chemin, de commande ou de secret.

| Méthode | Entrée | Sortie | Action polkit |
|---|---|---|---|
| `Ping()` | — | `b` | — |
| `GetVersion()` | — | `s` | — |
| `GetContractVersion()` | — | `u` | — |
| `SaveProfilesJSON(s)` | `ProfileCollection` JSON ≤ 1 Mio | `b` | `com.tuxedocomputers.tccd1.profiles` |
| `SaveSettingsJSON(s)` | `Settings` JSON ≤ 1 Mio | `b` | `com.tuxedocomputers.tccd1.settings` |
| `ReloadConfig()` | — | `b` | `com.tuxedocomputers.tccd1.settings` |

`GetContractVersion()` renvoie `ADMIN_CONTRACT_VERSION` (= 1) : c'est la
**voie supportée** pour la négociation de contrat. Un client dont la version
diffère doit refuser d'appeler. La propriété `ContractVersion` reste déclarée
pour l'introspection interne, mais **les propriétés ne sont pas une API
stable** : les clients ne doivent pas dépendre de `Properties.Get`/`GetAll`
(A1-11). La policy D-Bus peut retirer ces accès.

### Séquence d'une écriture

1. résolution du **nom unique** et de l'UID de l'appelant
   (`GetConnectionUnixUser`) ; échec ⇒ `NotAuthorized` ;
2. borne de taille ≤ 1 Mio **avant** `serde_json::from_str` ;
3. limiteur de débit : 2 appels/s, burst 4, par nom unique **et** par UID ;
4. `CheckAuthorization` polkit avec le sujet
   `("system-bus-name", {"name": <unique>})`, `flags = 1` (interaction),
   `details = {}`, timeout 120 s ; un seul dialogue global et un seul par
   appelant (attente max 2 s ⇒ `Busy`) ;
5. verrou `Semaphore(1)` : un second appel concurrent reçoit `Busy` sans
   attendre ;
6. parsing typé, validation `ProfileCollection::validate` /
   `Settings::validate`, filtrage des presets, vérification des références
   contre les fichiers sur disque ; pour `SaveProfilesJSON`, lecture de
   l'ancienne collection et calcul des suppressions (tombstones) avant toute
   écriture ;
7. lecture de l'ancienne version (≤ 1 Mio, `O_CLOEXEC | O_NOFOLLOW`,
   propriétaire/mode vérifiés), mémorisation de son **existence** et de son
   contenu, validation pour le type cible, écriture `save_*_with_backup`
   (`.bak` **0600**, cible 0644, atomique), appel `Control1.ReloadConfig` ;
   une suppression de profil écrit `profiles.json` **et** `settings.json`
   avant un unique rechargement ;
8. si le rapport n'est pas `ok` : rollback de tous les fichiers écrits par
   écriture atomique — restauration des octets exacts si le fichier existait,
   suppression s'il était absent avant (transaction double fichier, T1-01).
   En transaction mono-fichier, un précédent illisible n'est pas restauré
   (A1-07). Nouveau rechargement best-effort, puis `ReloadFailed`. Un `.bak`
   sain n'est jamais écrasé par un contenu illisible (`backup=preserved`, warn
   journald).

`SaveProfilesJSON` filtre les profils `is_preset()` : ils ne sont jamais
persistés. Un payload non vide composé uniquement de presets est accepté sans
effet ; une collection vide supprime les profils personnalisés (si aucune
référence ne l'interdit).

### Tombstones de profils

Un identifiant de profil supprimé est **tombstoné** dans
`Settings.deletedProfileIds` et ne peut plus jamais être recréé par l'API.

- Avant écriture, `tccd-admin` lit l'ancienne collection **d'état**
  (`/var/lib/tuxedo-rust-center/profiles.json`) via
  `ConfigStore::load_profiles_state_only_no_recovery`, jamais le repli `/etc`
  (AD-R13). Un fichier d'état absent vaut collection vide (première écriture) ;
  un contenu invalide ou trop volumineux est refusé (`Validation`), une erreur
  d'I/O ou de sécurité est refusée (`WriteFailed`), sans quarantaine. Les
  profils `/etc` ne participent donc **jamais** au diff et ne peuvent pas être
  tombstonés : `SaveSettingsJSON` continue de valider les références contre la
  collection effective (état + repli `/etc`), tandis que `SaveProfilesJSON`
  valide contre la nouvelle collection et les presets fusionnés.
- Tout ID présent dans la nouvelle collection mais absent de l'ancienne
  collection d'état et déjà tombstoné est refusé avec `Validation`
  (`deletedProfileIds`), **sans révéler l'identifiant**, et aucune écriture n'a
  lieu.
- Chaque profil supprimé est marqué par `Settings::mark_profile_deleted`
  (idempotent). Un preset, le profil personnalisé par défaut ou un ID invalide
  est refusé (`Validation`). La liste est bornée à **1024** entrées
  (`MAX_DELETED_PROFILE_IDS`) ; un dépassement est refusé avant écriture.
- Si au moins un profil est supprimé, `profiles.json` et `settings.json` sont
  écrits dans la **même transaction** : `.bak` 0600 pour chaque fichier,
  écritures atomiques, un seul rechargement. La transaction mémorise
  l'**existence initiale** de chaque fichier : si `Control1.ReloadConfig` n'est
  pas `ok`, un fichier présent est restauré à l'octet près et un fichier absent
  avant l'écriture est **supprimé** (T1-01), afin qu'aucun tombstone partiel ne
  subsiste. La même restauration des deux fichiers s'applique si l'écriture du
  second échoue.
- `SaveSettingsJSON` ne peut **ni ajouter ni retirer** un tombstone : toute
  entrée absente des réglages courants est refusée (`Validation`,
  `deletedProfileIds`) et tout tombstone existant omis du payload est réajouté.
  Il n'existe aucun `unmark` : seule une intervention root hors API peut
  réutiliser un ID.
- Un `settings.json` ou `profiles.json` courant au contenu illisible
  (JSON/validation) est refusé (`Validation`) plutôt que remplacé : l'ensemble
  des tombstones ne peut pas être préservé avec certitude. Une erreur d'I/O ou
  de sécurité est refusée (`WriteFailed`). La réparation relève d'une
  intervention root.
- Les compteurs `deletedProfiles`, `tombstonesAdded` et `tombstonesPreserved`
  sont audités dans journald, sans identifiant ni payload.

### Erreurs

Namespace `org.tuxedocomputers.tccd.Admin1.Error.*` :
`NotAuthorized`, `InvalidArgs`, `PayloadTooLarge`, `Validation`, `InUse`,
`ReloadFailed`, `WriteFailed`, `RateLimited`, `Busy`, `NotSupported`. Les
messages sont génériques : jamais de chemin, de valeur brute ni de contenu de
fichier. Un refus polkit **n'utilise pas**
`org.freedesktop.DBus.Error.AccessDenied`, pour que le client distingue un
refus d'autorisation d'un refus du bus. `Validation` peut porter le champ
`deletedProfileIds` (recréation d'un ID tombstoné, preset supprimé, limite de
1024 atteinte, ajout/retrait de tombstone via `SaveSettingsJSON`) ou
`profiles.<champ>` / `settings.<champ>` pour les autres champs ; l'identifiant
fautif n'est jamais renvoyé.

### Audit

Une ligne par appel : `method`, `action`, `caller`, `caller_uid`, `result`,
`duration_ms`, `bytes`, `backup`, `reload`. `backup` vaut `written` (nouveau
`.bak`), `preserved` (un `.bak` sain a été conservé car le fichier courant
était illisible) ou `none`. Un `preserved` produit aussi un `warn` journald,
sans chemin dans la réponse D-Bus. `reload` vaut `ok` après un rechargement
réel, `skipped` pour un no-op accepté (payload composé uniquement de presets)
et `failed`/`warning` selon le rapport. Aucun payload, nom de profil, chemin
ou secret.

Lorsqu'une suppression de profils ajoute des tombstones, une ligne
supplémentaire porte `deletedProfiles`, `tombstonesAdded` et
`tombstonesPreserved` (compteurs uniquement, jamais d'identifiant).

### Phase 1 uniquement

`SetTdpValues`, `SetOdmProfile`, `SetNvidiaCtgpOffset`, `SetPrimeMode` et les
opérations Tomte/WebFAI ne sont **pas** exposés. Ils relèvent des phases 2 et
3, avec revue `security` et durcissement dédié. Le changement de mot de passe
LUKS est **hors périmètre (décision utilisateur), non implémenté** : aucune
méthode, aucune action polkit, aucune signature.

## Service `com.tuxedocomputers.tccd.Admin.Hw` — interface `AdminHw1`

Service root **séparé** (AD-12 B2, phase 2), activé à la demande par D-Bus,
sans état persistant, arrêté après **30 s** sans appel. Il porte uniquement
les limites matérielles transitoires TDP/ODM/cTGP et n'exécute aucune commande
externe.

| Élément | Valeur |
|---|---|
| Service | `com.tuxedocomputers.tccd.Admin.Hw` |
| Objet | `/com/tuxedocomputers/tccd/Admin/Hw` |
| Interface | `com.tuxedocomputers.tccd.AdminHw1` |
| Contrat | `ADMIN_HW_CONTRACT_VERSION = 1` (comparaison stricte) |
| Binaire | `/usr/libexec/tuxedo-rust-center/tccd-admin-hw` |
| Unité attendue | `tccd-admin-hw.service` (créée par `packaging-release`) |
| Action polkit | `com.tuxedocomputers.tccd1.power-limits` |

| Méthode | Entrée | Sortie | Polkit |
|---|---|---|---|
| `Ping()` | — | `b` | non |
| `GetVersion()` | — | `s` | non |
| `GetContractVersion()` | — | `u` | non |
| `GetTdpValues()` | — | `ai` | non (rate limité) |
| `SetTdpValues(ai)` | 0 à 3 watts (`[]` = maxima firmware) | `b` | oui |
| `SetOdmProfile(s)` | nom allowlist | `b` | oui |
| `SetOdmProfileOverride(s)` | nom allowlist | `b` | oui + consentement |
| `SetNvidiaCtgpOffset(i)` | watts | `b` | oui |

### Service de session `com.tuxedocomputers.tccd.User` — interface `User1`

Service **de session** (bus de session), porté par le processus non privilégié
`tccd-user`. Il expose les capacités d'affichage réelles du compositeur pour
que la GUI ne propose que des modes applicables, et l'unique voie d'écriture
des contrôles UVC.

| Élément | Valeur |
|---|---|
| Service | `com.tuxedocomputers.tccd.User` |
| Objet | `/com/tuxedocomputers/tccd/User` |
| Interface | `com.tuxedocomputers.tccd.User1` |
| Contrat | `SESSION_CONTRACT_VERSION = 1` (méthodes additives) |

| Méthode | Entrée | Sortie | Notes |
|---|---|---|---|
| `Ping()` | — | `b` | vivacité |
| `GetVersion()` | — | `s` | version de `tccd-user` |
| `GetContractVersion()` | — | `u` | comparaison stricte |
| `GetDisplayInfoJSON()` | — | `s` | rapport JSON, cache 1 s |
| `GetUvcWriteSupported()` | — | `b` | vrai si au moins un device V4L2 est énuméré et si le backend d'écriture est instancié ; résultat mis en cache 1 s |
| `SetUvcControl(ssi)` | `devicePath`, `controlKey`, `value` | `b` | `true` si écrit, `false` si la valeur était déjà courante (idempotence) |

Le service reste **non privilégié** : bus de session, aucune action polkit,
aucun `DeviceAllow` systemd. L'accès au nœud caméra repose sur l'ACL `uaccess`
de logind ou le groupe `video`, comme pour toute application de session. Aucun
périphérique n'est ouvert au démarrage : `GetUvcWriteSupported` ne lit que
l'énumération sysfs, et `SetUvcControl` ouvre `/dev/videoN` uniquement à
l'appel, dans une tâche bloquante bornée.

`GetUvcWriteSupported` est mis en cache **1 s** (global au service) : un client
ne peut pas marteler l'énumération V4L2, et une méthode de capability ne
renvoie jamais `LimitsExceeded`. Le corps D-Bus des requêtes `User1` est borné
à **64 Kio avant désérialisation** (`InvalidArgs("message.size")`), ce qui
protège aussi `SetUvcControl` d'un `ss` surdimensionné.

#### `SetUvcControl` — écriture UVC

Validation, dans cet ordre :

1. `devicePath` : forme canonique `/dev/videoN`, 64 octets au plus, sans zéro
   de tête ni composant `..`. Le chemin doit correspondre exactement à un nœud
   de capture **réellement énuméré** par `tcc-hw` au moment de l'appel : un
   chemin inventé, un nœud metadata ou un device absent est refusé.
2. `controlKey` : non vide, 64 octets au plus, ASCII `[a-z0-9_]+`. Les clés
   sont celles exposées en lecture par `tccd`
   (`brightness`, `contrast`, `saturation`, `hue`, `gamma`,
   `white_balance_temperature`, `exposure_absolute`, `exposure`, `gain`,
   `sharpness`, `focus_absolute`, `focus_auto`, `power_line_frequency`,
   `backlight_compensation`, `zoom_absolute`).
3. `value` : entier. Les bornes, le pas et les index de menu sont validés par
   le backend `tcc-hw` (`QUERYCTRL`/`QUERYMENU`), puis la valeur est relue
   après `VIDIOC_S_CTRL` ; une valeur négative n'est acceptée que si le
   contrôle l'autorise.

L'allowlist des CIDs reste **fermée aux 15 contrôles** documentés dans
`docs/hardware/V4L2_DESIGN.md`. Un contrôle hors allowlist ou en lecture seule
est refusé (`NotSupported`), y compris lorsqu'il est énuméré par le pilote.

Débit : **20 écritures/s, burst 20, par UID appelant**. Si l'UID ne peut pas
être résolu (`GetConnectionUnixUser`, timeout 500 ms), le nom unique D-Bus sert
de clé de secours. Au-delà : `org.freedesktop.DBus.Error.LimitsExceeded`.

Erreurs stables :

| Erreur D-Bus | Cause |
|---|---|
| `org.freedesktop.DBus.Error.InvalidArgs` | `devicePath` non canonique/non énuméré, `controlKey` invalide |
| `org.freedesktop.DBus.Error.NotSupported` | contrôle hors allowlist, non inscriptible, backend absent, `EACCES`/`EPERM` à l'ouverture (ACL `uaccess`/groupe `video` absent) |
| `org.tuxedocomputers.tccd.User1.Error.OutOfRange` | valeur hors bornes, incompatible avec le pas ou le menu |
| `org.tuxedocomputers.tccd.User1.Error.Busy` | device déjà en écriture (`try_lock`) ou `EBUSY` |
| `org.tuxedocomputers.tccd.User1.Error.Stalled` | `EIO`, débranchement, timeout ou budget d'ioctl ; quarantaine 30 s |
| `org.freedesktop.DBus.Error.Failed` | échec d'énumération ou du backend, budget épuisé |
| `org.freedesktop.DBus.Error.LimitsExceeded` | limite de débit atteinte |

Limites connues :

- une autre application (aperçu vidéo, navigateur, `camera`) peut détenir le
  device : l'écriture échoue alors proprement en `Busy`, sans blocage du
  service ;
- `EACCES`/`EPERM` à l'ouverture signifie ACL `uaccess`/groupe `video` absent :
  le service répond `NotSupported`, jamais `Failed` ; la GUI doit proposer un
  état de permission, sans repli root ;
- `tccd` (daemon root) conserve les contrôles UVC en **lecture seule**
  (`controlsWritable: false`) et n'appelle jamais `VIDIOC_S_CTRL` ; la seule
  voie d'écriture est ce service de session ;
- un changement de contrôle UVC n'est pas persisté dans un profil : la
  restauration par profil relève d'une étape ultérieure ;
- la GUI doit appeler `GetUvcWriteSupported` avant d'activer un réglage et
  traiter les erreurs ci-dessus comme des états utilisateur (device occupé,
  contrôle non supporté), jamais comme un crash.

```json
{
  "available": true,
  "session": {
    "type": "wayland",
    "desktop": "Hyprland",
    "waylandDisplay": "wayland-1",
    "display": null
  },
  "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
  "internalOutput": {
    "name": "eDP-1",
    "connected": true,
    "enabled": true,
    "hasInternalDisplay": true,
    "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60.0,
                     "preferred": false, "current": true },
    "modes": [
      { "width": 2560, "height": 1600, "refreshHz": 60.0,
        "preferred": false, "current": true },
      { "width": 2560, "height": 1600, "refreshHz": 165.0,
        "preferred": false, "current": false }
    ]
  },
  "supported": true,
  "dryRun": false,
  "error": null
}
```

- `internalOutput.modes` ne contient que les modes de l'écran interne ; un
  écran externe n'apparaît jamais.
- `available=false` si le backend de session est indisponible ;
  `supported=false` si aucun écran interne connecté.
- Si le nom est déjà possédé ou le bus de session absent, `tccd-user`
  journalise un avertissement et continue sans ce service ; il n'y a ni crash
  ni écriture.
- La GUI doit vérifier `GetContractVersion` et traiter `GetDisplayInfoJSON`
  comme une donnée non fiable (bornes, types, modes réellement listés).

## Sécurité et séquence

1. identité (`header.sender()` + `GetConnectionUnixUser`, timeout 500 ms) ;
2. validation syntaxique **avant** tout jeton/polkit ;
3. rate limit écritures **2/s burst 4** par nom unique et UID ;
4. `CheckAuthorization` avec le sujet `system-bus-name`, flags `1` ;
   polkit absent/refusé/timeout ⇒ `NotAuthorized` fail-closed ;
5. `Semaphore(1)` : deuxième écriture concurrente ⇒ `Busy` sans attente ;
6. backend matériel dans `spawn_blocking`, vérification, rollback best-effort ;
7. audit journald sans valeur brute ni chemin.

**Limite de taille** : le corps D-Bus est refusé au-delà de **64 Kio**
(`MAX_HW_MESSAGE_BODY_BYTES`) **avant** la désérialisation des arguments, avec
`InvalidArgs("message.size")` ; le service reste vivant. La file de messages
de la connexion est en outre bornée (`max_queued(8)`). Les arguments
légitimes tiennent dans quelques octets (≤ 3 `i32`, ≤ 64 octets de nom). zbus 5
fixe la trame socket maximale à 128 Mio et ne l'expose pas : ce garde évite la
seconde allocation (`Vec`/`String`) et le dispatch, mais une trame proche de
128 Mio peut encore atteindre `MemoryMax=128M`. `packaging-release` doit
relever `MemoryMax` (256M recommandé) ou accepter un redémarrage du service
sur une trame maximale ; aucune écriture matérielle n'est effectuée dans ce cas.

`GetTdpValues` est une lecture : pas de polkit, pas de verrou, mais rate limit
**2/s burst 5** et identité résolue pour l'audit.

### TDP

`SetTdpValues` accepte **0 à 3** entiers (ordre firmware `pl1`, `pl2`, `pl4`).
Un tableau vide est une **consigne explicite** : appliquer les maxima firmware
(comportement original de `TdpControl::set_values(&[])`), avec audit
`values=empty, maxima=true`. Plus de 3 valeurs ⇒ `InvalidArgs("tdp.count")` ;
valeur hors `1..=1000` ⇒ `Validation("tdp.values")`. Les bornes firmware et la
monotonie `PL1 ≤ PL2 ≤ PL4` sont vérifiées par `tcc-hw` **avant** la première
écriture. Un profil dont `tdpValues=[]` n'est pas un appel `SetTdpValues` :
`tccd` ne modifie rien dans ce cas.

Après écriture, `GetTdpValues` est relu. Si la cible n'est pas atteinte :
rollback best-effort vers les valeurs relues avant l'appel ; rollback complet
⇒ `WriteFailed`, rollback partiel ⇒ `PartialApply("pl1,pl2")` (descripteurs
seulement). Une cible identique à l'état courant est un `noop` sans écriture.

### ODM et PPD

`SetOdmProfile` valide le nom (non vide, ≤ 64 caractères ASCII graphiques sans
`\`) puis l'allowlist **runtime** : `platform_profile_choices` et
`TuxedoIo::odm_profiles`. Un nom hors liste ⇒ `Validation("odm.name")`.

La voie est choisie dans cet ordre :

1. nom dans `platform_profile` : si PPD possède le nom (ou état inconnu) ⇒
   `PolicyDenied("ppd.platform_profile")`, **aucune** bascule silencieuse vers
   `tuxedo_io` même si le nom existe dans les deux listes ; sinon écriture
   sysfs, vérification de `current()`, et **restauration de l'ancien profil**
   si la vérification échoue (`WriteFailed` si restauré, `PartialApply` sinon) ;
2. nom uniquement dans les profils ODM `tuxedo_io` : autorisé même si PPD est
   **`Owned` ou `Unknown`** (décision AH-02 : PPD ne possède que
   `platform_profile`, pas les profils ODM du module `tuxedo_io`) ; pas de
   lecture de vérification ; l'audit porte explicitement `source=io` et
   `ppd=owned|unknown` ;
3. sinon `Validation("odm.name")`.

`SetOdmProfileOverride` exige en plus `Settings.ppd_override == true`, relu
sur `/var/lib` **à chaque appel** (aucun cache) ; sinon
`PolicyDenied("ppd.override_disabled")` **avant polkit** (pas de dialogue
inutile). Avec consentement, l'écriture `platform_profile` est autorisée
malgré PPD possédé ; un état PPD **inconnu** reste refusé (`PolicyDenied`).
`tccd` lit désormais `settings.ppd_override` dans ses deux constructions de
`HardwareInputs`, journalise un avertissement `ppd_override=true : TCC peut
écrire platform_profile malgré power-profiles-daemon`, et réapplique le profil
avec le même consentement après reload. Le flag reste un consentement
persistant : polkit est exigé à l'activation, puis `tccd` peut réécrire
`platform_profile` de façon cohérente (transitoire côté profil).

### cTGP

`SetNvidiaCtgpOffset` accepte `0..=200` (borne statique de sécurité). La borne
dynamique est lue via `Manager1.GetHardwareSnapshotJSON` (timeout 2 s, JSON
borné à 4 Mio) : `min(200, maxPowerLimit - defaultPowerLimit)`. Si `tccd` est
absent, en timeout ou si les champs manquent, la borne statique 200 W
s'applique (`bound=static` dans l'audit). Aucun `nvidia-smi` n'est exécuté.
Valeur identique à l'offset courant ⇒ `noop`. Après écriture, `offset()` est
relu ; en cas d'écart, l'ancienne valeur est restaurée une fois (best-effort)
puis `WriteFailed`. L'absence d'attribut `ctgp_offset` ⇒
`NotSupported("ctgp_offset")`.

### Erreurs

Namespace `org.tuxedocomputers.tccd.AdminHw1.Error.*` :
`NotAuthorized`, `InvalidArgs`, `Validation`, `PolicyDenied`,
`NotSupported`, `WriteFailed`, `PartialApply`, `RateLimited`, `Busy`.
Les messages sont des **codes de raison** (`tdp.count`, `tdp.values`, `tdp`,
`odm.name`, `platform_profile`, `ctgp_offset`, `nvidia.ctgpOffset`,
`ppd.platform_profile`, `ppd.override_disabled`, `message.size`) : jamais de
texte libre, de valeur thermique, de chemin ni de sortie de commande.

### Portée

Les setters sont **transitoires** (AD-7) : aucune persistance, aucun fichier
écrit. `tccd` peut réappliquer le profil actif au prochain changement d'état.
`Admin.Hw` ne modifie jamais `Admin1` ; le service `Admin.Prime` et
`SetPrimeMode` sont hors de cette implémentation.

## Interface `com.tuxedocomputers.tccd.Control1`

Servie par `tccd` sur `/com/tuxedocomputers/tccd`, **réservée à root**.

| Méthode | Entrée | Sortie | Notes |
|---|---|---|---|
| `ReloadConfig()` | — | `s` | `AdminReloadReport` JSON : `{ "ok": bool, "warnings": [...] }`. |

- `tccd` vérifie `header.sender()` puis `GetConnectionUnixUser == 0` **même si
  la policy D-Bus est cassée** ; sinon `org.freedesktop.DBus.Error.AccessDenied`
  (sans détail) et audit `result=denied`.
- `ok = false` dès qu'un avertissement de chargement est présent (fichier
  ignoré, corrompu, trop gros, références invalides). Les avertissements du
  rapport sont **sanitizés** : catégorie seulement, jamais de chemin.
- **Aucun cache de rapport** : chaque appel déclenche un rechargement réel et
  retourne son propre rapport. Les appels concurrents sont sérialisés par un
  verrou (pas de rechargements simultanés), mais un second appel rapproché
  relit bien les fichiers : une écriture entre deux `ReloadConfig` est donc
  visible dans le second rapport (A1-01).
- Le handler ne recharge pas lui-même : il envoie une demande à la boucle
  principale (canal borné) et attend le rapport (timeout 30 s). La boucle de
  bascule n'est jamais bloquée par un client D-Bus.

## `StateSnapshot`

```json
{
  "contractVersion": 1,
  "revision": 1,
  "ppdProfile": "balanced",
  "powerSource": "battery",
  "manualOverride": false,
  "activeProfile": {
    "id": "__profile_silent__",
    "name": "Quiet",
    "description": "",
    "isPreset": true,
    "source": "stateMatrix"
  }
}
```

- `ppdProfile` : `power-saver`, `balanced`, `performance`, une valeur libre, ou `null`.
- `powerSource` : `ac`, `battery`, `unknown`.
- `source` : `manualOverride`, `stateMatrix`, `legacyStateMap`, `defaultProfile`.
- `revision` : monotone ; un client peut ignorer une révision déjà vue.

## Profils

`GetProfilesJSON` renvoie la collection complète (`tcc_core::ProfileCollection`)
avec les champs historiques camelCase :

```json
[
  {
    "id": "__office__",
    "name": "Office",
    "description": "",
    "display": {
      "brightness": 60,
      "useBrightness": true,
      "refreshRate": -1,
      "useRefRate": false,
      "xResolution": -1,
      "yResolution": -1,
      "useResolution": false
    },
    "cpu": { "...": "..." },
    "webcam": { "...": "..." },
    "fan": { "...": "..." },
    "odmProfile": { "name": "enthusiast" },
    "odmPowerLimits": { "tdpValues": [25, 35, 35] },
    "nvidiaPowerCTRLProfile": { "cTGPOffset": 0 }
  }
]
```

Pour le refresh rate, les clients doivent utiliser :

- `display.useRefRate` ;
- `display.refreshRate` (`-1` ou absent = ne pas changer) ;
- l'écran interne uniquement (`eDP-*`, `LVDS-*`, `DSI-*`).

## Ventilateurs — `GetFanDataJSON`

Payload produit par `tcc_daemon::fan::FanData` (sérialisation `camelCase`) :

```json
{
  "available": true,
  "backend": "tuxi",
  "writable": true,
  "controlEnabled": true,
  "mode": "manual",
  "fans": [
    {
      "index": 0,
      "label": "cpu",
      "target": "cpu",
      "temp": 62,
      "speed": 35,
      "applied": 44
    },
    {
      "index": 1,
      "label": "gpu1",
      "target": "gpu",
      "temp": 55,
      "speed": 0,
      "applied": 30
    }
  ]
}
```

| Champ | Type | Sens |
|---|---|---|
| `available` | `bool` | Un backend de ventilation a été détecté. |
| `backend` | `string\|null` | `tuxi`, `pwm`, `tuxedo-io` ou `null`. |
| `writable` | `bool` | Le backend accepte les écritures de vitesse. |
| `controlEnabled` | `bool` | Le profil actif et `settings.fanControlEnabled` demandent le contrôle. |
| `mode` | `string` | `off` (aucun backend), `auto` (firmware) ou `manual` (daemon). |
| `fans[].index` | `u8` | Index 0-based. |
| `fans[].label` | `string` | Libellé matériel (`cpu`, `gpu1`, `fan1`). |
| `fans[].target` | `string` | Courbe appliquée : `cpu` ou `gpu`. |
| `fans[].temp` | `i32\|null` | Température en °C ; `null` si le capteur ne répond pas. |
| `fans[].speed` | `u8\|null` | Vitesse **mesurée**, en pourcentage. |
| `fans[].applied` | `u8\|null` | Dernière **consigne** écrite par le daemon. |

Notes :

- Sans matériel compatible, la méthode réussit et renvoie
  `{"available": false, "mode": "off", "fans": []}` : un client n'a pas à
  traiter un échec D-Bus pour une absence de ventilateur pilotable.
- `speed` et `applied` peuvent diverger (rampe de l'EC, mode auto, capteur
  imprécis) ; l'UI doit afficher `speed` comme valeur mesurée.
- En mode `auto` ou `off`, la télémétrie n'est rafraîchie qu'une fois toutes
  les 5 s ; en mode `manual`, à chaque tick (1 s).
- Le payload est en lecture seule : aucune méthode D-Bus ne permet de fixer
  une vitesse arbitraire. La régulation suit le profil actif.

## Exemples

```sh
busctl --system call com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd \
  com.tuxedocomputers.tccd GetStateJSON

busctl --system call com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd \
  com.tuxedocomputers.tccd SetTempProfile s __office__

busctl --system monitor com.tuxedocomputers.tccd
```

Matrice matérielle (interface typée) :

```sh
busctl --system call com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd \
  com.tuxedocomputers.tccd.Manager1 GetHardwareSnapshotJSON

busctl --system call com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd \
  com.tuxedocomputers.tccd.Manager1 SetSensorDataCollectionStatus b true
```

Ou avec le CLI fourni :

```sh
cargo run -p tcc-cli -- status
cargo run -p tcc-cli -- profiles
cargo run -p tcc-cli -- fans
cargo run -p tcc-cli -- set-temp __office__
```

Administration (root, activation D-Bus) :

```sh
# Diagnostic (aucun polkit)
busctl --system call com.tuxedocomputers.tccd.Admin \
  /com/tuxedocomputers/tccd/Admin \
  com.tuxedocomputers.tccd.Admin1 Ping

# Écriture : déclenche polkit si nécessaire
busctl --system call com.tuxedocomputers.tccd.Admin \
  /com/tuxedocomputers/tccd/Admin \
  com.tuxedocomputers.tccd.Admin1 SaveProfilesJSON s "$(tcc-cli profiles)"

# Rechargement root-only côté daemon
busctl --system call com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd \
  com.tuxedocomputers.tccd.Control1 ReloadConfig
```

## Sécurité

- La GUI et le CLI sont non privilégiés.
- Les méthodes de lecture ne modifient rien.
- `SetTempProfile` ne peut cibler qu'un profil existant, ne persiste pas et
  borne l'identifiant à 64 octets.
- `SetTempProfile`/`ClearTempProfile` sont **globaux**, rate-limités par nom
  unique D-Bus (5/s, burst 10), audités (`caller`, `caller_uid`,
  `profile_id`), mais **pas encore protégés par polkit**. La vérification
  `CheckAuthorization` est planifiée, pas implémentée.
- Le déplacement vers `Manager1` est planifié pour permettre une portée par
  session et une politique polkit dédiée.
- `Manager1` n'expose que des setters non persistants pour charge/lighting/
  Fn-Lock, rate-limités, audités et **protégés par polkit** (`…tccd1.charging`
  / `…tccd1.settings`). TDP/ODM/cTGP/PRIME/YCbCr restent en lecture seule ; les
  écritures persistantes passent par `Admin1` + polkit, puis
  `Control1.ReloadConfig`.
- `Admin1` exige polkit pour chaque écriture (`profiles`, `settings`) et pour
  `ReloadConfig`, avec le sujet `system-bus-name` de l'appelant, jamais un PID.
  polkit absent, en erreur ou en timeout ⇒ refus fail-closed, sans cache local
  et sans court-circuit root. `Ping`/`GetVersion` restent libres.
- `Control1.ReloadConfig` est root-only (policy D-Bus + revérification
  `GetConnectionUnixUser == 0`), sérialisé entre appels concurrents (aucun
  cache de rapport) et audité.
- Toute écriture admin est bornée à 1 Mio avant parsing, validée, sérialisée
  par `Semaphore(1)`, sauvegardée en `.bak` 0600 et annulée si `tccd` renvoie
  `ok=false`.
- `GetHardwareSnapshotJSON` est limité à 2 lectures/s (burst 5) par nom unique ;
  une lecture refusée ne réarme pas le bail capteurs. La session de collecte
  est plafonnée à 5 min et renouvelable uniquement par un
  `SetSensorDataCollectionStatus(true)` explicite, lui-même rate-limité.
- Si la connexion au bus système est perdue, le daemon s'arrête en erreur pour
  que systemd le redémarre ; il ne reste pas vivant sans nom D-Bus.
- Aucune méthode n'accepte de chemin de fichier, de commande ou de secret.
- Les réponses JSON doivent être traitées comme non fiables par le client :
  valider la version de contrat, les identifiants et les plages.
