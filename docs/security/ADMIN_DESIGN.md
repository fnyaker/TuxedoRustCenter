# Design review — `tccd-admin` et polkit

| Champ | Valeur |
|---|---|
| Version | 1.1 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | design du service admin root à la demande, de l'autorisation polkit et de la persistance (tombstones d'IDs) |
| Statut | design de référence ; **phase 1 implémentée** et revue dans `REVIEW_ADMIN_PHASE1.md` ; tombstones branchés (`AD-13`, §4.2.1) ; preuves système (vrai polkit, `Control1`, matériel) encore requises |
| Dépend de | D1/D2 (`docs/security/DECISIONS.md`) ; D5 conservée comme règle historique pour tout futur secret, **sans fonctionnalité LUKS** ; `REVIEW_JALON1.md` R-04/R-07/R-09, `REVIEW_HARDWARE.md` H-02 (résolu)/H-05/H-10 |
| Références externes | `polkit(8)`, `org.freedesktop.PolicyKit1` (D-Bus), `pkexec(1)` (pour ce qu'il ne faut **pas** faire) |

---

## 0. Contexte et périmètre

### 0.1 État au jalon 2 (vérifié avant rédaction)

- `tccd` expose `com.tuxedocomputers.tccd` (compat JSON) et `Manager1` (matrice matérielle).
- La politique D-Bus est **fail-closed** : `deny send_destination` global, puis allow par interface ou
  par membre (`packaging/dbus/com.tuxedocomputers.tccd.conf:39-148`). Les 8 setters `Manager1`
  sont explicitement **refusés au niveau du bus** (`:106-133`) en attendant polkit — c'est l'état
  décrit par la demande.
- `GetHardwareSnapshotJSON` est limité à 2 lectures/s (burst 5) et la collecte capteurs est plafonnée
  à 5 min par session (`docs/DBUS_API.md:96-98,185-216`) : **H-02 est résolu**, il n'est pas rouvert ici.
- Le fichier polkit déclare 9 actions mais **aucune n'est appelée par le code** (`packaging/polkit/…policy:10-18`) :
  c'est R-04, ouvert. `REVIEW_HARDWARE.md` H-11 (doc qui prétend le contraire) reste à corriger.
- `tccd-admin.service` existe, n'est pas installé (`install-tree.sh:64-68`), a `Restart=no`,
  `CapabilityBoundingSet=` vide, pas de `[Install]`, pas de `StateDirectory`
  (`packaging/systemd/tccd-admin.service:1-76`) : R-07 est correctement préparé.
- Aucun crate `tcc-admin`, aucune constante `Admin` dans `tcc-ipc`, aucun `Control1` dans `tccd`.

> **Mise à jour (2026-11-06)** : la phase 1 est implémentée (`tcc-admin`, `Admin1`, `Control1`) et
> les actions `profiles`/`settings` sont réellement appelées (`REVIEW_ADMIN_PHASE1.md`). Les autres
> actions polkit restent non appelées (phase 2/3). Le reste de §0.1 décrit l'état d'origine.

### 0.2 Objectif

Ouvrir **un** chemin d'écriture privilégié, à la demande, authentifié, audité, borné et révocable,
pour :

1. la **persistance** des profils et réglages (phase 1) ;
2. les **limites matérielles** TDP/ODM/cTGP et la **bascule PRIME** (phase 2) ;
3. les opérations **sensibles** Tomte/WebFAI/diagnostic (phase 3) ; **LUKS est hors périmètre
   (décision utilisateur), non implémenté**.

Principe directeur : **un bug de parsing dans `tccd` (qui traite des messages en continu) ne doit
jamais devenir une primitive root d'administration**. Corollaire : `tccd` reste sans écriture
persistante et sans opération sensible ; l'admin est un processus séparé, court, sans état.

### 0.3 Ce que ce document ne fait pas

- Il ne modifie aucun code (périmètre `docs/security/**`).
- Il ne tranche pas l'UX (nombre de dialogues polkit) : il fixe les **règles de sécurité** et les
  signatures ; l'UI peut regrouper les appels, pas les contourner.
- La phase 3 (Tomte/WebFAI/diagnostic) est un design ; **LUKS est hors périmètre, non implémenté**
  (décision utilisateur). D5 est conservée comme règle historique pour tout futur secret, mais aucune
  fonctionnalité LUKS n'est livrée.

---

## 1. Décisions d'architecture

| ID | Décision | Justification | Alternative rejetée |
|---|---|---|---|
| **AD-1** | Binaire séparé `tccd-admin`, crate `tcc-admin`, activé par D-Bus, **sans état**, sortie après 30 s d'inactivité | D1 ; réduit la fenêtre du processus root ; un crash ne laisse rien à réparer | service résident (surface permanente) ; `pkexec` (primitive argv root) |
| **AD-2** | Un **seul** nom `com.tuxedocomputers.tccd.Admin`, objet `/com/tuxedocomputers/tccd/Admin`, interface `com.tuxedocomputers.tccd.Admin1`, **membres listés un par un** dans la policy | nomenclature imposée ; fail-closed (H-10) | allow d'interface entière (statu quo `:140-141`) |
| **AD-3** | Toute méthode d'écriture appelle `CheckAuthorization` avec le sujet **`system-bus-name` du nom unique de l'appelant**, **même pour uid 0** | évite le confused deputy ; pas de « fallback root » | `unix-process` (PID fourni par le client), court-circuit uid 0 |
| **AD-4** | polkit absent, en erreur ou en timeout ⇒ **refus** (`NotAuthorized`) et audit | fail-closed ; root peut toujours éditer `/var/lib` hors API | fallback root, cache local d'autorisation |
| **AD-5** | La persistance passe **exclusivement** par `tcc_core::ConfigStore::system()` (donc `/var/lib`), avec écriture atomique + `.bak` 0600 | D2 ; règles déjà testées (`tcc-core/src/config.rs:607-656,910-977`) | écriture directe, `/etc`, chemin fourni par l'appelant |
| **AD-6** | Après écriture, l'admin appelle `tccd` par une méthode **root-only** `Control1.ReloadConfig()` ; les appels sont **sérialisés par mutex, sans cache** (chaque appel recharge réellement, corrigé A1-01) ; **rollback** si le rechargement échoue **et** que le contenu précédent est exploitable (sinon `ReloadFailed` sans restauration, A1-07) ; `.bak` écrit seulement si le précédent est valide | explicite, typé, auditable, sans PID file ni `systemctl` | SIGHUP (nécessite PID/`systemctl kill`), signal public (non authentifiable par la policy) |
| **AD-7** | Les setters hardware de phase 2 sont **transitoires** (non persistés) : ils s'appliquent au matériel jusqu'à la prochaine réconciliation de `tccd` | une seule autorité de persistance (les profils) ; évite qu'un admin réécrive silencieusement un profil utilisateur | persistance implicite dans le profil actif (effet de bord invisible) |
| **AD-8** | Les setters `Manager1` existants (charge/éclairage/Fn-Lock) **restent sur `tccd`** et seront réactivés par un polkit interne (`tccd`) au jalon 2.5 ; l'admin ne les duplique pas | une seule implémentation par sémantique ; évite deux chemins concurrents | déplacer tous les setters dans l'admin (latence + duplication) |
| **AD-9** | Module d'autorisation **partagé** `tcc-authz` (sujet, flags, timeout, audit, anti-dialog-storm, buckets **paramétrables par service**), utilisé par `tccd` et `tccd-admin` | une seule implémentation de `CheckAuthorization` à auditer ; les seuils restent par service (admin 2/s burst 4, daemon 5/s burst 10, lectures 2/s burst 5) | deux copies divergentes |
| **AD-10** | **LUKS hors périmètre** (décision utilisateur) : aucun service `tccd-admin-luks`, aucune capacité `CAP_SYS_ADMIN` LUKS, aucune action polkit. Les services phase 3 (Tomte/WebFAI/diagnostic) restent séparés selon `ADMIN_PHASE3_DESIGN.md` | la surface LUKS (secrets + block devices) est supprimée ; on garde la séparation par service pour les opérations restantes | service LUKS séparé (rejeté par décision) |
| **AD-11** | Aucun `pkexec`, `exec.path`, `exec.argv1`, `allow_gui`, aucun shell, aucun `env` hérité dans l'admin | F-06/S-12 ; checklist §6.3 | annotations polkit « pratiques » |
| **AD-13** | **Tombstones d'IDs de profils** : `Settings.deletedProfileIds` (≤ **1024**, IDs valides non gérés par TCC, uniques, **permanents**) ; `SaveProfilesJSON` refuse un nouvel ID tombstoné (`Validation("deletedProfileIds")`) et ajoute les IDs supprimés **dans la même transaction** que les profils ; le diff porte **uniquement sur l'état `/var/lib`**, jamais les profils `/etc` ; `SaveSettingsJSON` ne peut ni ajouter ni retirer un tombstone | garantit « pas de réutilisation d'ID » sans historique externe ; la suppression et son tombstone sont indissociables du point de vue de l'API ; aucune nouvelle méthode D-Bus ni action polkit | (a) ignorer l'écart avec `ARCHITECTURE.md` §3.8 ; (b) fichier d'historique séparé (surface, permissions et rollback supplémentaires) ; (c) annulation de tombstone (réintroduit la réutilisation) |

**Décision à arbitrer (AD-12)** : phase 2 dans la même unité que la phase 1. La phase 1 n'a besoin
d'**aucune** capacité ni périphérique ; la phase 2 exige `ProtectKernelTunables=no` et
`DeviceAllow=/dev/tuxedo_io`. Recommandation `security` : accepter ce relâchement **au moment de la
phase 2 uniquement**, avec le delta documenté (§5.3) ; si le budget le permet, préférer un second
service `…Admin.Hw` (même binaire, `--mode=hardware`) pour garder la phase 1 à capacité nulle.

> **AD-12 tranché (2026-11-06)** — voir `docs/security/ADMIN_PHASE2_DESIGN.md` §1 (P2-01) et §4 :
> option **B2**, deux services séparés `com.tuxedocomputers.tccd.Admin.Hw` et
> `com.tuxedocomputers.tccd.Admin.Prime`, binaires distincts, unité phase 1 **inchangée**.
> L'option A est rejetée
> (mélange parser JSON / ioctl / exec dans un même processus root). Le `--mode=hardware` envisagé
> ci-dessus est abandonné : `ADMIN_DESIGN` §5.4 interdit qu'un argument de ligne de commande
> influence une opération ; deux `[[bin]]` sont préférés.

---

## 2. Contrat D-Bus `Admin1`

Répartition des rôles côté client : **lecture** via `tccd` (compat/`Manager1`), **écriture
persistante et opérations sensibles** via `com.tuxedocomputers.tccd.Admin/Admin1`. Aucune méthode de
`Admin1` ne renvoie de configuration : après un `Save*`, le client relit `tccd` (`GetProfilesJSON`,
`GetSettingsJSON`, `GetStateJSON`) pour vérifier l'état effectif.

### 2.1 Constantes (à ajouter dans `tcc-ipc/src/lib.rs`)

```rust
/// Service admin, activé à la demande (D1).
pub const DBUS_ADMIN_SERVICE: &str = "com.tuxedocomputers.tccd.Admin";
/// Objet admin (distinct de /com/tuxedocomputers/tccd).
pub const DBUS_ADMIN_PATH: &str = "/com/tuxedocomputers/tccd/Admin";
/// Interface admin versionnée.
pub const DBUS_ADMIN_INTERFACE: &str = "com.tuxedocomputers.tccd.Admin1";
/// Interface de contrôle interne de tccd (root uniquement).
pub const DBUS_CONTROL_INTERFACE: &str = "com.tuxedocomputers.tccd.Control1";
/// Version du contrat admin (stricte, comme IPC_CONTRACT_VERSION).
pub const ADMIN_CONTRACT_VERSION: u32 = 1;
```

`tcc-ipc` **reste sans dépendance `zbus`** (charte du crate) : il ne contient que les constantes et
les types `serde` des rapports (`AdminReloadReport`), pas les proxies.

### 2.2 Invariants communs à toutes les méthodes

| Invariant | Règle |
|---|---|
| Version | Propriété `ContractVersion` (`u`) = `ADMIN_CONTRACT_VERSION`. Un client dont la version diffère **refuse** d'appeler (comparaison stricte, comme `ensure_contract_compatible`). |
| Taille | Chaîne JSON ≤ **1 Mio** (`tcc_core::config::DEFAULT_MAX_CONFIG_BYTES`). Vérifiée **avant** `serde_json::from_str`. Au-delà : `PayloadTooLarge`. |
| Types | Aucun `a{sv}` ouvert, aucun `serde_json::Value` en signature. Les payloads JSON sont des **modèles typés** (`ProfileCollection`, `Settings`) sérialisés par le client à partir de ce que `tccd` lui a renvoyé. |
| Identité | `header.sender()` obligatoire (nom unique). Résolution `GetConnectionUnixUser` **avant** polkit ; échec ⇒ `NotAuthorized` (fail-closed). |
| Autorisation | `CheckAuthorization` avant **toute** lecture/écriture sensible (aucune énumération LUKS : hors périmètre). |
| Concurrence | `Semaphore(1)` : un seul appel d'écriture à la fois ; un second appel concurrent reçoit `Busy` **immédiatement** (pas de file). |
| Débit | Token bucket par nom unique (2 appels/s, burst 4) **et** par UID (2/s, burst 4) sur les méthodes d'écriture ; `RateLimited` au-delà. |
| Audit | `tracing::info!/warn!` avec `action`, `method`, `caller`, `caller_uid`, `result`, `duration_ms`. **Jamais** de payload, de contenu de fichier, de secret, de chemin absolu. |
| Erreurs | Namespace `org.tuxedocomputers.tccd.Admin1.Error.*` (§2.7). Les messages sont génériques : pas de chemin, pas de contenu de fichier, pas de valeur brute au-delà du nom de champ. |
| Idempotence | `Save*` est idempotent (même contenu ⇒ même résultat, une écriture). Aucun nonce : un rejeu par un client authentifié est sans effet supplémentaire. |
| Pas de chemin | Aucune méthode n'accepte de chemin de fichier, de device ou de commande. Les seules chaînes libres sont : nom de profil ODM, mode PRIME (énuméré), JSON de modèles. |

### 2.3 Phase 1 — persistance

#### `SaveProfilesJSON(s json) -> b`

- **Action polkit** : `com.tuxedocomputers.tccd1.profiles` (**nouvelle**, `auth_admin`, sans cache — §3.2).
- **Entrée** : `tcc_core::ProfileCollection` JSON (`camelCase`), ≤ 1 Mio.
- **Traitement** :
  1. `serde_json::from_str::<ProfileCollection>` — erreur ⇒ `InvalidArgs("profiles: <champ>")`.
  2. Rejet des profils **presets** (`profile.is_preset()`, `tcc-core/src/profile.rs:499-501`) : ils
     sont **filtrés**, jamais persistés (sinon une mise à jour des presets serait écrasée). Un payload
     ne contenant que des presets est accepté et **sans effet** (`true`).
  3. `ProfileCollection::validate()` (≤ 256 profils, ids `^[A-Za-z0-9_-]{1,64}$`, noms uniques,
     bornes de tous les champs) — erreur ⇒ `Validation(<champ>)`.
  4. Charger les `Settings` **sur disque** (lecture stricte, sans quarantaine) et la collection
     d'état précédente via `load_profiles_state_only_no_recovery` (`service.rs:514-521`,
     `tcc-core/src/config.rs:606-614`) : **aucun repli `/etc`**, les profils packagés ne sont jamais
     comparés ni tombstonés (AD-R13). Un contenu illisible ⇒ `Validation("settings")`/
     `Validation("profiles")` ; une erreur d'I/O ⇒ `WriteFailed` ; **aucune écriture**.
  5. **Tombstones d'identifiants (AD-13)** — avec les `Settings` déjà chargées et la collection
     d'état seule :
     - tout ID de la nouvelle collection **absent de l'état précédent** et présent dans
       `settings.deleted_profile_ids` ⇒ `Validation("deletedProfileIds")`, **aucune écriture**
       (`service.rs:515-528`) ; un ID déjà présent dans l'ancienne collection reste accepté
       (réparation administrateur) ;
     - les IDs présents dans l'ancienne collection et absents de la nouvelle sont ajoutés à
       `deleted_profile_ids` via `Settings::mark_profile_deleted` (`tcc-core/src/settings.rs:297-354` :
       idempotent, ajout en fin de liste, ordre conservé) avant l'écriture. Un ID géré par TCC
       (presets, presets legacy, `DEFAULT_CUSTOM_PROFILE_ID`, `tcc-core/src/profile.rs:117-134`) ou
       une liste déjà pleine (`MAX_DELETED_PROFILE_IDS = 1024`, `validate.rs:38-43`) ⇒
       `Validation("deletedProfileIds")` et **aucun tombstone partiel** (`service.rs:526-552`) ;
     - la liste est **permanente** : aucune méthode d'annulation, et `SaveSettingsJSON` ne peut
       pas la modifier (voir ci-dessous).
  6. `settings.validate()` **après** ajout des tombstones, puis
     `validate_settings_references(&settings, &merged)` ⇒ `Validation` ou `InUse` si un profil
     encore référencé disparaît ; aucune écriture, donc aucun tombstone.
  7. Transaction : si des tombstones ont été ajoutés, **double fichier** `profiles.json` +
     `settings.json` avec un seul rechargement (§4.2.1) ; sinon transaction simple
     `profiles.json` (§4.2). Puis `Control1.ReloadConfig`.
- **Portée de l'écriture `settings.json`** : ce chemin n'exige que l'action `profiles`, mais le
  contenu écrit est le `Settings` **relu sur disque** auquel seuls des tombstones sont ajoutés —
  jamais un `Settings` fourni par le client. Aucun autre champ (température, charge, matrice,
  `extra`) ne peut être modifié via `SaveProfilesJSON`.
- **Sortie** : `true` si écrit **et** rechargé sans avertissement ; `true` également pour un payload
  composé uniquement de presets (no-op, audit `reload=skipped`). Sinon erreur typée ; en cas de
  rechargement refusé, `ReloadFailed` et restauration du contenu précédent **seulement s'il était
  exploitable** (sinon le nouveau fichier validé reste sur disque, A1-07).

#### `SaveSettingsJSON(s json) -> b`

- **Action polkit** : `com.tuxedocomputers.tccd1.settings` (existante, `auth_admin`).
- **Entrée** : `tcc_core::Settings` JSON, ≤ 1 Mio.
- **Validation** : `Settings::validate()` (schéma strict : `schema_version == SETTINGS_SCHEMA_VERSION`,
  `tcc-core/src/settings.rs:42,371`), puis `validate_settings_references(&new_settings, profiles sur disque)`.
- **Tombstones append-only** : `Settings::validate()` vérifie la **forme** (`≤ 1024`, IDs valides,
  uniques, non gérés par TCC, `validate.rs:731-755`) mais **pas** la permanence, qui dépend du
  fichier courant. L'admin compare donc au `settings.json` relu :
  - tout tombstone **absent du courant** ⇒ `Validation("deletedProfileIds")` (ajout refusé,
    `service.rs:602-607`) : impossible d'épuiser la borne sans suppression de profil ;
  - tout tombstone courant **omis** par le payload est **réajouté** avant validation
    (`service.rs:608-616`) : une suppression silencieuse est impossible ;
  - un `settings.json` courant illisible ⇒ `Validation("settings")`, jamais remplacé : la liste
    des tombstones est inconnue et ne doit pas être écrasée.
  Seul `SaveProfilesJSON` crée des tombstones.
- **Traitement** : transaction simple `settings.json` (§4.2), puis `Control1.ReloadConfig`.
- **Interdit** : `SaveSettingsJSON` ne peut pas créer un profil ; toute référence absente ⇒ `InUse`.

#### `ReloadConfig() -> b`

- **Action polkit** : `com.tuxedocomputers.tccd1.settings`.
- **Rôle** : demande à `tccd` de relire les fichiers sur disque (après une restauration manuelle, un
  dépannage, ou pour forcer une réconciliation). N'écrit rien.
- **Sortie** : `true` si `tccd` a rechargé **sans avertissement**.

#### `Ping() -> b`, `GetVersion() -> s`

Diagnostic, **sans** polkit, **sans** rate limit (pas d'effet de bord). Utile pour que
`dbus-policy-test.sh` distingue « service absent » de « autorisation refusée ».

### 2.4 Notification de `tccd` après écriture — décision AD-6

Ajouter à `tccd` une interface **root uniquement** :

```rust
#[zbus::interface(name = "com.tuxedocomputers.tccd.Control1")]
impl Control {
    /// Recharge profils + réglages depuis /var/lib et réapplique.
    /// Retourne un JSON `AdminReloadReport { ok: bool, warnings: Vec<String> }`.
    async fn reload_config(&self, #[zbus(header)] h: Header<'_>, …) -> fdo::Result<String>;
}
```

- **Contrôle** : la policy D-Bus n'autorise `Control1` que pour `<policy user="root">` (§6) ; `tccd`
  **re-vérifie** en plus que l'UID de l'appelant est 0 (`GetConnectionUnixUser`) — défense en
  profondeur, même si la policy est cassée.
- **Réponse honnête** : `ok = false` dès qu'un `LoadWarning` est produit (fichier ignoré, corrompu,
  trop gros). L'admin utilise ce booléen pour décider du rollback.
- **Sérialisation sans cache** (correction A1-01) : les appels concurrents sont sérialisés par un
  mutex, mais **aucun rapport n'est mémorisé** : chaque `ReloadConfig` déclenche une relecture réelle
  des fichiers. Un rapport ne peut donc jamais être antérieur à une écriture effectuée entre deux
  appels. La réponse reste honnête en cas d'arrêt/timeout (`ok=false`), ce qui déclenche le rollback
  côté admin.
- **Refus** : hors root ⇒ `org.freedesktop.DBus.Error.AccessDenied` (sans détail). Le refus est audité.
- **Pourquoi pas SIGHUP** : `tccd` n'a plus de PID file (R-20) ; `systemctl kill -s HUP` est une
  commande externe, non typée, non auditable par notre code et couplée à systemd. **Pourquoi pas un
  signal D-Bus** : un signal peut être émis par n'importe quel processus ; le vérifier exigerait de
  résoudre l'émetteur — autant utiliser un appel de méthode, qui porte déjà `header.sender()`.
- **Pourquoi pas `systemctl reload tccd`** : dépendance à systemd dans un service root activé par
  D-Bus, et `ExecReload` devrait alors exister (`kill -HUP $MAINPID`), soit exactement le mécanisme
  qu'on veut éviter.

### 2.5 Phase 2 — limites matérielles et PRIME (transitoires)

| Méthode | Signature | Action polkit | Bornes / validation | Effet |
|---|---|---|---|---|
| `SetTdpValues(ai values)` | tableau de 1 à 3 entiers (watts) | `…tccd1.power-limits` | `TdpControl::set_values` : entier, `[min,max]` firmware, séquence croissante `PL1 ≤ PL2 ≤ PL4` (`tcc-hw/src/tdp.rs:131-173`). Liste vide **interdite** ici (contrairement à l'API interne) : `InvalidArgs`. | ioctl `tuxedo_io` ; **non persisté** |
| `SetOdmProfile(s name)` | nom ∈ `platform_profile_choices` ou profils `tuxedo-io` | `…tccd1.power-limits` | allowlist **découverte au runtime** (`platform_profile.rs:175-202`), ≤ 64 caractères ASCII graphiques | sysfs/ioctl ; **non persisté** |
| `SetNvidiaCtgpOffset(i watts)` | `0..=min(200, max_limit − default_limit)` | `…tccd1.power-limits` | `CtgpControl::set_offset_watts` ; si `nvidia-smi` indisponible, borne statique documentée (`nvidia.rs:45`) | sysfs `ctgp_offset` ; **non persisté** |
| `SetPrimeMode(s mode)` | `intel` \| `nvidia` \| `on-demand` | `…tccd1.prime-select` | énumération stricte (`PrimeMode::argument`, `prime.rs:95-107`) ; `Off`/`Unknown` rejetés | `prime-select <mode>` ; **persistant par nature** (config système), effet à la prochaine session |

**Sémantique de non-persistance (AD-7).** Ces quatre méthodes appliquent la valeur au matériel et
retournent. `tccd` réapplique le profil actif à la prochaine révision (changement de profil, SIGHUP,
reprise de veille) : la valeur transitoire peut donc être écrasée. Le contrat doit le dire
explicitement (`docs/DBUS_API.md`) et l'UI doit afficher « temporaire ». La persistance d'une limite
passe **uniquement** par `SaveProfilesJSON` (édition du profil) sous l'action `profiles`.

**Cohérence avec `tccd`.** L'admin écrit directement dans `tcc-hw` ; il ne met pas à jour
`RuntimeState` de `tccd` (processus distinct). Après une écriture de phase 2, appeler
`Control1.ReloadConfig()` est **inutile** (aucun fichier modifié) ; en revanche, l'UI doit relire
`GetHardwareSnapshotJSON` pour afficher la valeur réelle. Documenter.

**Ordre d'application TDP.** Écriture séquentielle PL1→PL2→PL4 non transactionnelle
(`tdp.rs:189-201`) ; en cas d'échec au milieu, retourner `PartialApply { applied: [pl1], failed: "pl2" }`
dans le message d'erreur (sans valeur brute) et journaliser l'état atteint.

### 2.6 Phase 3 — opérations sensibles (design seulement)

> **LUKS est hors périmètre (décision utilisateur), non implémenté.** Les volets restants
> (Tomte/WebFAI/diagnostic) sont décrits dans `ADMIN_PHASE3_DESIGN.md` ; ils ne réutilisent aucun
> secret LUKS et n'accèdent à aucun block device.

#### LUKS — hors périmètre

> **Hors périmètre (décision utilisateur), non implémenté.** Aucune méthode `Luks*`, aucun service
> `tccd-admin-luks`, aucune action polkit `luks-change-key`, aucun accès `cryptsetup`/block device,
> aucun transport de phrase par fd. D5 reste une règle historique pour tout futur secret, mais aucune
> fonctionnalité LUKS n'est livrée. Voir `ADMIN_PHASE3_DESIGN.md`.

#### `TomteRun(as args) -> s` — polkit `…tomte-run`

- `Command::new("/usr/bin/tuxedo-tomte")`, `env_clear()`, `PATH` fixe, `stdin=null`, timeout 120 s,
  sortie ≤ 64 Kio, `args` validés : premier argument ∈ allowlist
  `{AUTOMATIC, listjson, description, block, unblock, configure, remove, reconfigure}` ; les
  sous-arguments sont validés par sous-commande (nom de module `^[a-z0-9-]{1,64}$`). Jamais `sh -c`,
  jamais `yes |`, jamais d'argument issu d'un fichier téléchargé.

#### `PackagesApply(as packages) -> s` — polkit `…packages`

- Allowlist **statique** de paquets WebFAI (constante du binaire), aucun ajout de dépôt, aucun script
  distant exécuté, `apt-get install -y --no-install-recommends <pkg>` (ou `dnf` selon la distro),
  `env_clear()`, timeout 300 s, sortie bornée et traduite. Refus de tout nom hors allowlist.

### 2.7 Codes d'erreur

```rust
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.tuxedocomputers.tccd.Admin1.Error")]
pub enum AdminError {
    /// polkit a refusé, est absent, ou a expiré. Jamais de détail.
    NotAuthorized,
    /// Entrée syntaxiquement invalide (champ nommé, valeur jamais reprise).
    InvalidArgs(String),
    /// Chaîne JSON au-delà de 1 Mio.
    PayloadTooLarge,
    /// Modèle valide syntaxiquement mais rejeté par la validation.
    Validation(String),
    /// Un profil référencé par l'état disparaîtrait.
    InUse(String),
    /// tccd n'a pas pu recharger ; le fichier précédent a été restauré si possible.
    ReloadFailed,
    /// Écriture impossible (I/O). Détail dans le journal uniquement.
    WriteFailed,
    /// Limite de débit atteinte.
    RateLimited,
    /// Une écriture est déjà en cours.
    Busy,
    /// Matériel absent / opération non supportée.
    NotSupported,
}
```

Correspondance : `NotAuthorized` → `org.freedesktop.DBus.Error.AccessDenied` est **interdit** ici
(il ferait croire à un refus du bus) ; on garde le namespace applicatif pour que le client distingue
« polkit a dit non » d'un problème de transport. Le test `dbus_authz` vérifie que le message ne
contient ni chemin ni valeur.

---

## 3. Polkit

### 3.1 Appel `CheckAuthorization`

```
destination  org.freedesktop.PolicyKit1
path         /org/freedesktop/PolicyKit1/Authority
interface    org.freedesktop.PolicyKit1.Authority
method       CheckAuthorization((sa{sv}) subject, s action_id, a{ss} details, u flags, s cancellation_id)
retour       (b is_authorized, b is_challenge, a{ss} details)
```

- **Sujet** : `("system-bus-name", { "name": <nom unique de l'appelant> })`.
  Jamais `unix-process` (le PID du client serait fourni par le client), jamais le nom unique de
  l'admin ni celui de `tccd`.
- **`action_id`** : celui de la méthode (§3.2), constante compilée.
- **`details`** : `{}`. Jamais de payload, de nom de profil, de valeur matérielle : les détails
  polkit apparaissent dans les dialogues et journaux.
- **`flags`** : `1` (`AllowUserInteraction`) pour toutes les actions `auth_admin*`. Sans ce flag,
  `auth_admin` échoue systématiquement pour un utilisateur non root.
- **`cancellation_id`** : `""` (pas d'annulation programmatique en v1).
- **Timeout** : 120 s (dialogue humain). Au-delà : `NotAuthorized` + audit `timeout`.
- **Résultat** : `is_authorized == true` **et** `is_challenge == false` ⇒ autorisé. Sinon refus.
  Ne jamais réessayer automatiquement, ne jamais interpréter `is_challenge` comme un succès.
- **Cache local : aucun.** Le cache polkit (`auth_admin_keep` = 5 min) est le seul, et il est décidé
  par l'administrateur système via les defaults.
- **uid 0** : `CheckAuthorization` est appelé **même pour root** (AD-3). Si la distribution autorise
  implicitement root, polkit retourne `true` ; sinon root est refusé par notre API — il peut toujours
  éditer `/var/lib` directement, ce n'est pas une régression de sécurité. **Aucun court-circuit.**

### 3.2 Actions et IDs

Le fichier `packaging/polkit/com.tuxedocomputers.tccd.policy` doit être modifié ainsi :

| Action | Défaut | Cache | Méthodes |
|---|---|---|---|
| `com.tuxedocomputers.tccd1.profiles` | **nouvelle** | `auth_admin` | `SaveProfilesJSON` |
| `com.tuxedocomputers.tccd1.settings` | existante | `auth_admin` | `SaveSettingsJSON`, `ReloadConfig` |
| `com.tuxedocomputers.tccd1.power-limits` | existante | `auth_admin` | `SetTdpValues`, `SetOdmProfile`, `SetNvidiaCtgpOffset` |
| `com.tuxedocomputers.tccd1.prime-select` | existante | `auth_admin` | `SetPrimeMode` |
| `com.tuxedocomputers.tccd1.tomte-run` | existante | `auth_admin` | `TomteRun` |
| `com.tuxedocomputers.tccd1.packages` | existante | `auth_admin` | `PackagesApply` |
| `com.tuxedocomputers.tccd1.assign-profile` | existante | `auth_admin_keep` | **à retirer** : remplacée par `profiles` |

**Pourquoi `profiles` sans `_keep`.** Un profil contient les limites TDP/ODM/cTGP : le sauvegarder
est une modification **persistante** de limites matérielles. Avec `auth_admin_keep`, une première
authentification ouvrirait 5 minutes pendant lesquelles une sauvegarde pourrait modifier le TDP sans
nouveau dialogue — c'est exactement le contournement que R-09 cherchait à fermer. `assign-profile`
(`_keep`) devient redondant et plus faible : il doit être supprimé pour qu'il ne reste pas une voie
d'authentification cachée.

**Pourquoi `power-limits` sans `_keep`.** Écritures matérielles immédiates (thermique). Le coût UX
est assumé ; l'UI doit regrouper les modifications d'un même écran en **un** appel.

Ajout XML exact :

```xml
<!-- Écriture persistante des profils (contient TDP/ODM/cTGP) : pas de cache. -->
<action id="com.tuxedocomputers.tccd1.profiles">
    <description>Save TuxedoRustCenter hardware profiles</description>
    <description xml:lang="fr">Enregistrer les profils matériels TuxedoRustCenter</description>
    <description xml:lang="de">TuxedoRustCenter-Hardwareprofile speichern</description>
    <message>Authentication is required to save hardware profiles</message>
    <message xml:lang="fr">Une authentification est requise pour enregistrer les profils matériels</message>
    <message xml:lang="de">Zum Speichern der Hardwareprofile ist eine Authentifizierung erforderlich</message>
    <defaults>
        <allow_any>no</allow_any>
        <allow_inactive>no</allow_inactive>
        <allow_active>auth_admin</allow_active>
    </defaults>
</action>
```

Aucune annotation `org.freedesktop.policykit.exec.*`, aucun `allow_gui` (déjà garanti par
`validate.sh:368` et le test de politique).

### 3.3 Absence, erreur, timeout de polkit

| Cas | Comportement |
|---|---|
| `org.freedesktop.PolicyKit1` absent du bus (`ServiceUnknown`) | `NotAuthorized`, log `warn` avec le nom d'erreur D-Bus uniquement, pas de repli |
| Erreur de transport / timeout du bus | `NotAuthorized`, audit `error` |
| `CheckAuthorization` retourne `is_authorized=false` | `NotAuthorized`, audit `denied` |
| `is_challenge=true` et `is_authorized=false` | `NotAuthorized` (l'utilisateur a annulé ou n'a pas les droits) |
| Timeout 120 s | `NotAuthorized`, audit `timeout` |
| uid de l'appelant illisible | `NotAuthorized` **avant** polkit (identité inconnue) |

Aucun mode « polkit indisponible ⇒ root autorisé ». Un système sans polkit n'a pas d'admin
fonctionnel ; c'est documenté et testé.

### 3.4 Anti-dialog-storm et anti-DoS

Un utilisateur local peut déclencher une activation D-Bus puis un dialogue polkit en boucle. Trois
garde-fous, dans cet ordre :

1. **Bucket avant polkit** : 2 appels/s (burst 4) par nom unique **et** par UID → `RateLimited`
   sans dialogue.
2. **Un seul dialogue global** : `Semaphore(1)` autour de `CheckAuthorization` avec
   `AllowUserInteraction` ; attente max 2 s puis `Busy`. Empêche N dialogues simultanés.
3. **Un seul dialogue par appelant** : si le même nom unique a déjà une autorisation en cours,
   `Busy` immédiat.

Ces limites sont dans `tcc-authz`, donc partagées avec `tccd` (AD-9). Elles ne remplacent pas
`TasksMax`/`MemoryMax` (§5.3).

**Risque résiduel (AD-R1)** : un attaquant peut saturer les 5 tentatives de `StartLimitBurst` en
provoquant des échecs d'activation (message de 128 Mio, cf. §7) et bloquer l'admin ~60 s. Impact
disponibilité, pas d'intégrité. Mitigation : `StartLimitBurst=30` sur une fenêtre de 10 s (§5.2) et
documentation.

### 3.5 Audit

Événement structuré, une ligne par appel :

```text
action=save-profiles method=SaveProfilesJSON caller=:1.42 caller_uid=1000
polkit=com.tuxedocomputers.tccd1.profiles result=authorized duration_ms=812
bytes=18234 deletedProfiles=1 tombstonesAdded=1 tombstonesPreserved=0 backup=written reload=ok
```

- `result` ∈ `{authorized, denied, timeout, error, rate_limited, busy}`.
- `bytes` : taille du payload validé (pas le contenu).
- `deletedProfiles`, `tombstonesAdded`, `tombstonesPreserved` : **compteurs seuls**, jamais d'ID
  (`audit_tombstones`, `service.rs:427-441`).
- `backup` ∈ `{none, written, preserved}` (`written` = nouveau `.bak` ; `preserved` = `.bak` sain
  conservé parce que le contenu remplacé n'était pas exploitable).
- `reload` ∈ `{ok, skipped, warning, error}` : `skipped` pour un no-op (presets uniquement), jamais
  `ok` sans rechargement.
- **Jamais** : payload JSON, nom de profil, valeur TDP, chemin, contenu de fichier, secret.
- Sur refus : `warn!` avec `caller`, `caller_uid`, `action`, `result` — pas de motif détaillé.
- Les tests `audit.rs` échouent si un motif interdit apparaît (cf. `REVIEW_JALON1` §4.4).

### 3.6 Tests polkit

| Fichier | Contenu |
|---|---|
| `crates/tcc-authz/tests/polkit_subject.rs` | le sujet envoyé est `system-bus-name(<unique>)` et non `unix-process` ; flags = 1 ; `details` vide ; `action_id` exact |
| `crates/tcc-authz/tests/polkit_denied.rs` | faux `PolicyKit1` sur bus de session : `is_authorized=false` ⇒ `NotAuthorized` ; `is_challenge=true` ⇒ refus ; aucun cache (deux appels = deux `CheckAuthorization`) |
| `crates/tcc-authz/tests/polkit_absent.rs` | service absent ⇒ `NotAuthorized`, aucun repli root |
| `crates/tcc-authz/tests/polkit_storm.rs` | 20 appels/s ⇒ `RateLimited` avant polkit ; 2 dialogues concurrents ⇒ un seul, l'autre `Busy` |
| `crates/tcc-admin/tests/admin_audit.rs` | subscriber de test : champs présents, payload absent |

Le faux `PolicyKit1` est un service zbus de session qui enregistre les arguments reçus ; il ne
nécessite pas root. Les tests root (vraie policy + vrai polkit) sont dans
`scripts/release/dbus-policy-test.sh` (extension §8).

---

## 4. Persistance

### 4.1 Chemins et permissions

| Élément | Valeur | Source |
|---|---|---|
| Racine d'état | `/var/lib/tuxedo-rust-center` | `tcc-core/src/config.rs:102`, D2 |
| Fichiers | `settings.json`, `profiles.json`, `fantables.json` (0644 root:root) | `ConfigPaths`, `write_atomic` (`:1548`) |
| Sauvegarde | `<fichier>.bak` **0600** root:root, une génération **par fichier** ; `profiles.json.bak` et `settings.json.bak` ne forment **pas** une paire atomique (§4.2.1) | `save_typed_with_backup` (`config.rs:998,1065,1080`), tests `tests/transaction.rs:77-137` |
| Temporaires | `.<fichier>.tmp.<pid>.<n>` 0600, supprimés en cas d'erreur | `unique_temp_path` (`:1520`) |
| `/etc` | **jamais écrit** : `ConfigStore` refuse (`:1093-1110`), `ProtectSystem=strict` + `ReadWritePaths` le garantissent au niveau noyau | D2 |
| Répertoire | 0755 root:root (`StateDirectoryMode=0755`, tmpfiles) | `tccd.service:29-34` |

Le magasin utilisé est **`ConfigStore::system()`** (état `/var/lib`, admin `/etc` en lecture seule,
`LoadPolicy::for_root_daemon()` : propriétaire 0, refus de `mode & 0o022`). Aucun override
`TCC_CONFIG_DIR` (feature absente du release, D7).

### 4.2 Transaction d'écriture (prescriptive)

Ce bloc décrit l'écriture d'**un seul** fichier (`SaveSettingsJSON`, ou `SaveProfilesJSON` sans
suppression). Une suppression de profil qui ajoute des tombstones écrit **deux** fichiers : voir
§4.2.1.

```
1. verrou d'écriture (Semaphore(1))             -> sinon Busy
2. parse + validate(payload)                    -> sinon InvalidArgs/Validation/PayloadTooLarge
3. validate_settings_references(...)            -> sinon InUse
4. ancien = read_previous(cible)                (régulier, uid attendu, mode & 0o022 == 0,
                                                  ≤ 1 Mio, O_NOFOLLOW|O_CLOEXEC)
   ancien_exploitable = désérialisable + validé pour le type (sinon rollback désactivé)
5. si ancien_exploitable : write .bak (0600, atomique)
   sinon : conserver le .bak existant (preserved_backup) — jamais écraser une version saine
6. write_atomic(cible, 0644, fchmod avant rename, fsync, rename, fsync dir)
7. report = tccd.Control1.ReloadConfig()        (sérialisé, sans cache)
8. si report.ok == false :
     a. si ancien_exploitable : write_atomic(cible, ancien) puis ReloadConfig()
     b. sinon : laisser le nouveau fichier valide ; retour ReloadFailed sans restauration
9. audit (backup=none|written|preserved, reload=ok|skipped) + retour true
```

- **Sauvegarde conditionnelle (A1-07)** : le `.bak` n'est réécrit que si le contenu remplacé est
  désérialisable **et** valide pour le type cible. Sinon un `.bak` sain existant est conservé
  (`preserved_backup`) et le nouveau contenu validé remplace quand même la cible. En transaction
  **mono-fichier**, le rollback est désactivé si le précédent n'est pas exploitable : le fichier
  écrit reste alors sur disque et l'appelant reçoit `ReloadFailed` — état explicite, jamais de
  restauration d'octets corrompus. La transaction **double fichier** (§4.2.1) restaure au contraire
  l'état initial exact (octets ou absence), car elle doit annuler la suppression et son tombstone.
- **Quarantaine de lecture** : sur un fichier corrompu, `ConfigStore` déplace la preuve vers
  `.bak.corrupt` (nom fixe, une génération, 0600 root) au lieu d'écraser un `.bak` sain.

- **Pourquoi une sauvegarde avant écriture** : `ConfigStore` ne sauvegarde que les fichiers
  *corrompus* (`.bak` sur erreur de lecture). Une écriture valide écrase l'ancienne version sans
  copie ; pour une action administrateur, on veut un point de retour. La sauvegarde est créée par le
  **même** mécanisme atomique (`write_atomic` paramétré par le mode) — pas de `fs::copy`.
- **Fenêtre de crash** : si l'admin meurt entre (6) et (7), le fichier est écrit et valide, `tccd`
  le chargera au prochain SIGHUP/démarrage. Si l'admin meurt pendant (8a), le fichier peut rester
  nouveau : documenté, sans corruption (écriture atomique).
- **Répertoire absent** : créé par systemd (`StateDirectory`), pas par l'admin ; si absent ⇒
  `WriteFailed` (on ne fait pas de `create_dir_all` dans un chemin privilégié arbitraire).
- **API réelle dans `tcc-core`** :
  ```rust
  pub struct Saved {
      pub backup: Option<PathBuf>,
      pub preserved_backup: Option<PathBuf>,
  }
  impl ConfigStore {
      pub fn save_settings_with_backup(&self, value: &Settings) -> Result<Saved, CoreError>;
      pub fn save_profiles_with_backup(&self, value: &ProfileCollection) -> Result<Saved, CoreError>;
  }
  ```
  `write_atomic` prend un paramètre `mode: u32` (0644 fichier, 0600 `.bak`) ; tests : mode final,
  contenu du `.bak`, `.bak` sain conservé si le courant est corrompu, refus `/etc`.

### 4.2.1 Transaction double fichier — suppression de profil + tombstone

Déclencheur : au moins un ID supprimé dans `SaveProfilesJSON` (`tcc-admin/src/service.rs:555-578`).
La suppression et son tombstone doivent être persistés ensemble : un tombstone sans suppression
bloque inutilement un ID ; une suppression sans tombstone autorise sa réutilisation.

Séquence (`tcc-admin/src/transaction.rs:232-305`) :

1. Lire et mémoriser l'état initial des deux cibles via
   `read_previous_target_blocking(..., restore_unusable=true, remove_if_absent=true)`
   (`transaction.rs:452-503`) : existence (`existed`), octets exacts, contrôles régulier /
   propriétaire / `mode & 0o022 == 0` / ≤ 1 Mio / `O_NOFOLLOW|O_CLOEXEC`. La lecture initiale de
   `service.rs` a déjà validé les documents ; les octets exacts sont conservés pour un rollback
   à l'état initial (A1-07/A1-14).
2. Écrire `profiles.json` via `save_profiles_with_backup` : `.bak` **0600** si le précédent est
   exploitable, cible **0644**, `fsync`, `rename`, `fsync` du répertoire.
3. Écrire `settings.json` via `save_settings_with_backup` : mêmes garanties, `.bak` **distinct**.
   Si cette écriture échoue : `rollback` des **deux** cibles (restauration des octets ou
   suppression d'un fichier créé), puis `WriteFailed` ; aucun rechargement.
4. Un **seul** `Control1.ReloadConfig` après les deux écritures.
5. Si le rapport n'est pas `ok` : `rollback` des **deux** cibles
   (`rollback`/`restore_target`, `transaction.rs:358-383`) :
   - `previous` présent ⇒ restauration atomique des octets exacts ;
   - fichier **absent avant** (`existed == false`) ⇒ suppression durable (`remove_file` + `fsync`
     du répertoire), y compris un `settings.json` créé par la transaction (**T1-01 fermé**) ;
   - puis second `ReloadConfig` best-effort ;
   - toutes les actions ont réussi ⇒ `ReloadFailed` ;
   - une restauration ou suppression a échoué ⇒ `WriteFailed` (état potentiellement mixte, à
     réparer depuis la paire `.bak`).

Ordre **profils puis settings** : un crash entre les deux `rename` laisse au pire une suppression
effective sans tombstone (ID réutilisable par un client admin authentifié), jamais un tombstone sur
un profil encore vivant — ce dernier contredirait l'invariant « jamais de tombstone sans
suppression » et fausserait l'état exposé à l'UI.

Limites et fenêtres de crash :

- **Deux `rename` non atomiques** : un crash entre (2) et (3) laisse `profiles.json` à jour et
  `settings.json` ancien. Un **retry** de la même requête ne répare pas : la diff est calculée
  contre la collection déjà écrite, donc `deleted` est vide et le tombstone n'est jamais ajouté.
  Réparation : recréer puis supprimer l'ID via l'API (si non référencé) ou édition root. Risque
  **AD-R7**.
- **Absence initiale (T1-01 fermé)** : `RestoreTarget.existed` mémorise l'existence. Au rollback,
  un fichier créé par la transaction (`settings.json`) est **supprimé** de façon durable au lieu
  d'être laissé avec un tombstone ; si la suppression échoue ⇒ `WriteFailed`. Tests
  `reload_failure_restores_absence_of_settings_file` (`tests/transaction.rs:422`) et
  `reload_failure_without_previous_settings_restores_absence` (`service.rs:1324`). La transaction
  mono-fichier conserve au contraire le nouveau fichier valide sans précédent
  (`failed_save_without_previous_keeps_valid_file`).
- **`.bak` non appariables** : chaque `.bak` est la version précédente de **son** fichier, écrite
  juste avant son `rename`. Après un crash, `profiles.json.bak` et `settings.json.bak` peuvent
  provenir de générations différentes : une restauration manuelle doit prendre la paire cohérente
  d'un même commit, jamais un `.bak` isolé. Le rollback automatique, lui, utilise les octets relus
  au début de la transaction (paire cohérente).
- **Tests** : `profiles_and_settings_are_written_in_one_transaction` et
  `reload_failure_rolls_back_profiles_and_settings_together` (`crates/tcc-admin/tests/transaction.rs:323,379`) ;
  côté service, `reload_failure_rolls_back_profiles_and_settings` (`service.rs:1213-1250`).
  L'absence initiale de `settings.json` est couverte (`tests/transaction.rs:422`,
  `service.rs:1324`) ; le crash entre les deux `rename` n'est pas simulable en test unitaire
  (AD-R7).

### 4.3 Validation et invariants croisés

| Contrôle | Où |
|---|---|
| Bornes de tous les champs (noms, couleurs, TDP, fréquences, fan curves) | `ProfileCollection::validate`, `Settings::validate` |
| `schema_version` strict | `Settings` (`settings.rs:371`) |
| Unicité id/nom de profil, ≤ 256 profils | `validate.rs:619-645` |
| `state_matrix`/`state_map` pointent vers des profils existants | `validate_settings_references` (`validate.rs:888-911`) |
| Profils presets non persistés | filtre `is_preset()` (`profile.rs:121-123,499-501`) |
| Tombstones `deletedProfileIds` ≤ 1024, IDs valides, uniques, non gérés par TCC | `Settings::validate` (`validate.rs:731-755`) ; `ProfileId::is_tcc_managed` (`profile.rs:117-134`) |
| Recréation d'un ID tombstoné refusée ; suppression + tombstone dans la même transaction | `service.rs:515-578` |
| Diff de tombstones limité à l'état `/var/lib`, jamais les profils `/etc` | `load_profiles_state_only_no_recovery` (`config.rs:606-614`) ; test `etc_profile_is_not_tombstoned_by_state_diff` (`service.rs:1179`) |
| Rollback restaure l'absence initiale (suppression d'un `settings.json` créé) | `RestoreTarget.existed`/`remove_if_absent` (`transaction.rs:142-160,358-383`) ; tests `tests/transaction.rs:422`, `service.rs:1324` |
| `start < end`, `PL1 ≤ PL2 ≤ PL4`, `min ≤ max` | déjà dans `tcc-core`/`tcc-hw` |
| Taille ≤ 1 Mio avant parse | admin |
| Aucun chemin | contrat (aucun paramètre de chemin) |

**Interdiction explicite** : l'admin ne « corrige » jamais une valeur (pas de clamp silencieux). Une
valeur hors bornes ⇒ `Validation(champ)`, fichier inchangé.

**Hygiène `extra`** : les champs JSON inconnus sont conservés (`Profile::extra`, `Settings::extra`)
et donc persistés. L'admin **ne les inspecte pas** ; il journalise un `warn!` si une clé ressemble à
un secret (`(?i)(pass|secret|token|credential)`) sans rejeter — un rejet casserait des champs
légitimes (`keyboardBacklightStates`) et la détection est heuristique. Documenter que la config n'est
pas un coffre-fort.

### 4.4 Concurrence

- Un seul processus admin possible : le nom D-Bus est demandé avec `DO_NOT_QUEUE` (zbus l'impose,
  `zbus-5.13.2/src/connection/builder.rs:62`) ; un second binaire échoue au démarrage.
- zbus exécute chaque méthode dans une tâche : le `Semaphore(1)` de l'admin est **indispensable**.
- Pas de verrou fichier (`flock`) : une seule instance suffit ; un `flock` ajouterait un chemin
  d'échec et un fichier de plus. À reconsidérer si un jour un CLI admin est ajouté.
- `ReloadConfig` de `tccd` : mutex de sérialisation **sans cache** ; chaque appel recharge réellement.

### 4.5 Taille, migration, temporaires

- 1 Mio par fichier (`DEFAULT_MAX_CONFIG_BYTES`) ; un profil ≤ 4 Kio de description, ≤ 256 profils
  ⇒ la borne est structurellement suffisante.
- **Migration** : l'admin ne migre pas. La migration `/etc/tcc` reste dans `tccd` au premier
  démarrage (`config.rs:66-98`). L'admin refuse un `schema_version` inconnu (`Validation`).
- **Nettoyage** : ajouter à `packaging/tmpfiles/tuxedo-rust-center.conf` :
  ```conf
  # Fichiers temporaires d'écriture atomique laissés par un crash (kill -9).
  r! /var/lib/tuxedo-rust-center/.*.tmp.*   - - - 1d
  ```
  (âge 1 jour ; les temporaires sont 0600 root, jamais lisibles par un utilisateur).

---

## 5. Processus `tccd-admin` et durcissement

### 5.1 Cycle de vie

| Aspect | Décision |
|---|---|
| Activation | D-Bus uniquement : `/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service` (`SystemdService=tccd-admin.service`, `User=root`) ; **aucun** `[Install]`, jamais au boot |
| Démarrage | refus si `geteuid() != 0` (lecture `/proc/self/status`, pas d'`unsafe`) ; aucune lecture de config, aucun device ouvert |
| Inactivité | sortie propre après **30 s sans appel en cours** (`IDLE_TIMEOUT`). Un refus ne repousse pas l'échéance ; un dialogue polkit long est protégé par `Activity::enter`/`ActiveCall` (le processus ne s'arrête pas au milieu d'une opération). Le nom est relâché, la prochaine requête réactive ; `RuntimeMaxSec=5min` plafonne la durée de vie. Implémentation : compteur d'appels en cours + `Instant` de dernière autorisation + supervision 1 s |
| Erreur fatale | sortie non nulle (systemd marque l'activation en échec), pas de boucle : `Restart=no` |
| Signaux | `SIGTERM`/`SIGINT` ⇒ sortie immédiate ; aucun état à sauver |
| État | **aucun** : pas de cache de profils, pas de cache polkit, pas de device persistant, pas de fichier de PID |
| Logs | `tracing` → journald ; filtre **fixe** `info` en release (pas de `RUST_LOG` honoré), pas de fichier de log |
| Version | `GetVersion` renvoie `CARGO_PKG_VERSION` ; `ContractVersion` = `ADMIN_CONTRACT_VERSION` |

**Pourquoi 30 s** : assez long pour enchaîner une rafale d'appels d'une même action UI (les
activations répétées coûtent ~100 ms), assez court pour borner la durée de vie d'un processus root
qui contient la logique polkit. À ajuster après mesure ; ne jamais dépasser 5 min (`RuntimeMaxSec`
en garde-fou).

### 5.2 Unit systemd — phase 1 (à installer)

```ini
[Unit]
Description=TuxedoRustCenter on-demand administrative actions (config persistence)
Documentation=file:/usr/share/doc/tuxedo-rust-center/ADMIN_DESIGN.md
After=dbus.service
# Activation D-Bus uniquement. Jamais au boot.
StartLimitIntervalSec=10
StartLimitBurst=30

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin
# Une action échoue et se termine ; l'activation suivante relance un processus neuf.
Restart=no
TimeoutStartSec=10
TimeoutStopSec=10
RuntimeMaxSec=5min

UMask=0077
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/tuxedo-rust-center
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
KeyringMode=private
RemoveIPC=yes
RestrictAddressFamilies=AF_UNIX
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
CapabilityBoundingSet=
AmbientCapabilities=
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=192M
TasksMax=16
```

Notes :

- `PrivateDevices=yes` est possible **en phase 1** (aucun périphérique) ; il devra passer à `no`
  avec `DevicePolicy=closed` + `DeviceAllow=/dev/tuxedo_io rw` en phase 2.
- `SystemCallFilter=@system-service` doit être validé (pas d'`ioctl` en phase 1) ;
  `systemd-analyze security` cible ≤ **2.5**.
- `RuntimeMaxSec=5min` : garde-fou si la sortie sur inactivité est cassée.
- Pas de `StateDirectory` (D1) ; pas de `ConfigurationDirectory` (lecture seule inutile ici).

### 5.3 Delta par phase (documenté et revu à chaque palier)

| Option | Phase 1 (config) | Phase 2 (TDP/ODM/cTGP/PRIME) | Phase 3 (Tomte/WebFAI/diagnostic, hors LUKS) |
|---|---|---|---|
| `ProtectKernelTunables` | `yes` | **`no`** (sysfs `platform_profile`, `ctgp_offset`) | `yes` |
| `PrivateDevices` | `yes` | **`no`** + `DevicePolicy=closed` + `DeviceAllow=/dev/tuxedo_io rw` | selon service (`ADMIN_PHASE3_DESIGN.md`) |
| `DevicePolicy`/`DeviceAllow` | `closed` (aucun) | `closed` + `/dev/tuxedo_io` | `closed` ; aucun block device LUKS (hors périmètre) |
| `CapabilityBoundingSet` | **vide** | **vide** (root possède sysfs ; à mesurer) | vide / mesuré par service |
| `ProtectSystem` | `strict` | `strict` + `ReadWritePaths=/var/lib` | `strict` (cache/paquets selon service) |
| `SystemCallFilter` | `@system-service` | **à valider** (`ioctl`, `execve` pour `prime-select`) — probablement désactivé tant que non mesuré | à valider (Tomte/PackageKit) |
| `MemoryMax` | 192M | 192M | 256M |
| `TasksMax` | 16 | 32 | 32 |
| `LimitCORE` | 0 | 0 | 0 |
| `KeyringMode` | private | private | private |
| Chemin d'installation | `/usr/libexec/tuxedo-rust-center/tccd-admin` | idem (même binaire) | `tccd-admin-{tomte,webfai,diagnostics}` |

**Règle de revue** : toute ligne qui passe de `yes` à `no` ou toute capacité ajoutée doit être
justifiée dans `packaging/SECURITY.md` avec la mesure associée (`strace`, `systemd-analyze security`),
et relue par `security` avant publication.

### 5.4 Interdits (testés)

- Aucun `pkexec`, aucun `org.freedesktop.policykit.exec.path`, `exec.argv1`, `allow_gui`.
- Aucun `sh -c`, `bash -c`, `|`, `&&` dans une commande.
- Aucune commande dont le chemin vient d'un `PATH` ou d'un téléchargement.
- Aucun argument de ligne de commande du binaire ne doit influencer une opération (pas de
  `tccd-admin --set-x` qui contournerait polkit) : le binaire n'accepte que `--version`/`--help`
  et `--dev-session` (dry-run, non root) pour les tests. **Aucun mode `--force`.**
- Aucun fichier de configuration lu au démarrage (un attaquant qui contrôle `/var/lib` — impossible
  en théorie — ne peut pas influencer l'initialisation avant polkit).

---

## 6. Politique D-Bus (extraits exacts à appliquer)

### 6.1 `Admin1` : membres un par un (remplace `com.tuxedocomputers.tccd.conf:138-146`)

```xml
<!-- 6. Admin : joignable, mais chaque membre est listé (fail-closed).
        Le bus ne fait pas d'autorisation par utilisateur : polkit tranche
        dans tccd-admin. Toute méthode non listée est injoignable. -->
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="org.freedesktop.DBus.Introspectable"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="org.freedesktop.DBus.Peer"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="org.freedesktop.DBus.Properties" send_member="Get"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="org.freedesktop.DBus.Properties" send_member="GetAll"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="Ping"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="GetVersion"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SaveProfilesJSON"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SaveSettingsJSON"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="ReloadConfig"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SetTdpValues"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SetOdmProfile"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SetNvidiaCtgpOffset"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"
       send_interface="com.tuxedocomputers.tccd.Admin1" send_member="SetPrimeMode"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
```

Corollaire : les méthodes de phase 3 (`TomteRun`, `PackagesApply`, diagnostic) ne sont **pas** listées
tant que leurs services n'existent pas ; les ajouter au moment de l'implémentation, avec leurs tests.
**LUKS est hors périmètre : aucune méthode `Luks*` ne doit être ajoutée.**

### 6.2 `Control1` : réservé à root

Dans `<policy user="root">` :

```xml
<allow send_destination="com.tuxedocomputers.tccd"
       send_interface="com.tuxedocomputers.tccd.Control1"/>
```

Dans `<policy context="default">`, après les allow de `tccd` (pour l'auditabilité) :

```xml
<deny send_destination="com.tuxedocomputers.tccd"
      send_interface="com.tuxedocomputers.tccd.Control1"/>
```

`tccd` re-vérifie l'UID appelant == 0 (défense en profondeur) et audite chaque rechargement
(`caller`, `result`).

### 6.3 Ce que la politique **ne peut pas** faire

- Elle ne peut pas limiter la **taille** d'un message par service : `max_message_size` est global au
  bus. Un message admin de 128 Mio est donc accepté par le bus et tué par `MemoryMax` (voir §7,
  AD-R1). À documenter, pas à « corriger » dans ce fichier.
- Elle ne peut pas exprimer « tel UID peut, tel autre non » : c'est polkit.

---

## 7. Modèle de menace — delta

### 7.1 Nouveaux actifs et acteurs

| # | Élément | Description |
|---|---|---|
| A10 | **Intégrité de la configuration persistante** | `/var/lib/tuxedo-rust-center/{settings,profiles}.json` : un profil malveillant appliqué au boot = charge désactivée, ventilateurs bloqués, TDP max. |
| A11 | **Clé de capacité admin** | capacité d'un processus root à écrire ces fichiers et à lancer prime-select/tomte/PackageKit. |
| U8 | **Utilisateur local authentifiable** | peut déclencher une activation D-Bus et un dialogue polkit ; c'est un acteur **semi-fiable** : il a le droit de faire l'action s'il s'authentifie, mais ne doit pas pouvoir déclencher autre chose, ni en boucle. |

### 7.2 Menaces et parades

| ID | Menace | Parade | Test |
|---|---|---|---|
| **TA-1** | Confused deputy : l'admin vérifie les droits de son propre processus ou d'un PID fourni par le client | sujet `system-bus-name(unique)` + résolution UID de l'appelant, jamais le PID | `polkit_subject.rs` |
| **TA-2** | Déni de service par activation/dialogues en boucle | bucket avant polkit (2/s), `Semaphore(1)` dialogues, `Busy`, `StartLimitBurst=30/10 s`, `TasksMax=16` | `polkit_storm.rs`, `dbus_limits` |
| **TA-3** | Message de 128 Mio tuant l'admin avant polkit | `MemoryMax=192M` (mort propre, pas de corruption) ; risque résiduel AD-R1 documenté ; pas de données écrites avant validation | test « message géant » (attendu : échec, fichier inchangé) |
| **TA-4** | TOCTOU entre validation et écriture | écriture atomique (`create_new` + `rename`), relecture des invariants croisés **avant** la première écriture, verrou unique | `persistence.rs` (symlink, concurrence) |
| **TA-5** | Évasion vers `/etc` ou hors `/var/lib` | aucun chemin en paramètre ; `ConfigStore` refuse `/etc` et `..` ; `ProtectSystem=strict` + `ReadWritePaths` | test `write outside` + `strace` |
| **TA-6** | Écriture partielle / crash | `rename` atomique, `.bak` conditionnel (seulement si le précédent est exploitable), rollback double fichier à l'état initial exact (restauration des octets ou suppression d'un fichier créé), temporaires 0600 nettoyés | test coupure (`kill -9` simulé) ; `tests/transaction.rs:422` |
| **TA-7** | Rejeu / double soumission | idempotence : même contenu = même résultat, une seule écriture ; `Semaphore(1)` | `concurrency.rs` |
| **TA-8** | Faux `ReloadConfig` (forcer des réappliquations) | méthode root-only, policy + re-vérification uid, sérialisation par mutex sans cache (aucun rapport réutilisé) | test policy non-root → `AccessDenied` |
| **TA-9** | Injection dans `prime-select`/`tomte`/gestionnaire de paquets | arguments énumérés, chemins absolus, `env_clear`, pas de shell, sortie bornée | `command` tests existants + `prime.rs:95-107` |
| **TA-10** | **LUKS — hors périmètre** : aucune fuite de phrase n'est possible, aucune fonction LUKS n'est implémentée ; D5 reste la règle historique pour tout futur secret | hors périmètre | décision utilisateur |
| **TA-11** | Remplacement du binaire admin | root:root 0755, `validate.sh --tree` (setuid/world-writable), intégrité du paquet | CI packaging |
| **TA-12** | Profil piégé persisté (TDP absurde, `onlineCores: 0`) | `validate()` + `validate_settings_references` avant écriture, aucune correction silencieuse | `persistence.rs` (fixtures invalides) |
| **TA-13** | Secret déposé dans `extra` puis exposé par `GetProfilesJSON` | documenté ; `warn!` heuristique ; pas de stockage de secret dans la config | revue |
| **TA-14** | L'admin devient un service résident exploitable | sortie après 30 s, `RuntimeMaxSec=5min`, `Restart=no`, aucun état | test d'inactivité |
| **TA-15** | Recréation d'un ID de profil supprimé (client admin, import, collision d'UUID) | tombstone vérifié **avant** écriture ; un ID tombstoné absent de l'ancienne collection ⇒ `Validation("deletedProfileIds")` ; la suppression ajoute le tombstone dans la même transaction ; IDs GUI (UUID v4) revérifiés par l'admin | `service.rs:1059-1103` (`deleted_profile_id_cannot_be_recreated`) |
| **TA-16** | Suppression persistée sans tombstone, ou tombstone sur un profil vivant (crash entre deux `rename`, rollback partiel) | ordre profils→settings (pas de tombstone orphelin au nominal) ; rollback des deux sur reload refusé, y compris suppression d'un fichier créé ; fenêtre résiduelle AD-R7 documentée | `tests/transaction.rs:323,379,422` ; `service.rs:1213-1250,1324` |

### 7.3 Risques résiduels (à accepter explicitement)

| # | Risque | Niveau | Décision |
|---|---|---|---|
| **AD-R1** | Un message de 128 Mio peut tuer l'admin avant polkit et consommer les tentatives d'activation → indisponibilité ~60 s | Faible (dispo) | accepté, documenté ; `StartLimitBurst=30` ; pas de limite par service possible dans dbus-daemon |
| **AD-R2** | polkit absent ⇒ admin inutilisable, même pour root | Faible (dispo) | accepté (fail-closed) ; root édite `/var/lib` directement |
| **AD-R3** | Un administrateur authentifié peut écrire n'importe quel profil valide (y compris TDP max) | Moyen | c'est la fonction ; `auth_admin` sans cache ; audit |
| **AD-R4** | La config n'est pas signée : un root compromis peut tout modifier | Moyen | hors périmètre (R2 du threat model) |
| **AD-R5** | Phase 2 relâche `ProtectKernelTunables`/`PrivateDevices` de l'unit admin | Moyen | delta documenté §5.3, revue `security` obligatoire ; option service séparé (AD-12) |
| **AD-R6** | Un ID de profil supprimé peut être recréé **hors API** : root édite `/var/lib`, retire un tombstone ou supprime un profil sans ajouter son ID | Faible | hors périmètre root (R2) ; l'API refuse la recréation et ne supprime jamais sans tombstone ; réparation root documentée |
| **AD-R7** | Crash entre les deux `rename` : suppression effective sans tombstone ; un retry de la même requête ne répare pas (`deleted` est vide) | Faible | accepté ; impact limité à un client admin authentifié (la réutilisation d'ID n'est pas une frontière de privilège) ; réparation recréer+supprimer via l'API, ou root |
| **AD-R8** | Borne **1024** atteinte : toute nouvelle suppression est refusée (`Validation("deletedProfileIds")`), sans tombstone partiel | Faible (dispo) | fail-closed ; seul un client admin authentifié peut remplir la liste (chaque suppression exige polkit) ; réparation root ; taille ≈ 70 Kio, sous 1 Mio |
| **AD-R9** | IDs générés par la GUI / importés : UUID v4 côté GUI (`profile.rs:103-105`), mais l'admin ne fait pas confiance au préfixe (format, unicité, tombstone revérifiés) ; l'import ne régénère que sur conflit ou ID invalide (`profile_transfer.rs:245-252`), donc un ID libre mais tombstoné est conservé puis **rejeté** par l'admin | Info (UX) | fail-closed, aucune escalade ; recommandation : exposer `deletedProfileIds` à l'import et régénérer l'ID en collision, pour éviter un échec global |
| **AD-R11** | `SaveSettingsJSON` réajoute silencieusement un tombstone omis au lieu de renvoyer une erreur | Info | fail-safe (aucune suppression possible) ; un client qui croit avoir purgé reçoit `true` sans effet sur ce champ ; à documenter dans `DBUS_API.md` |
| **AD-R12** | Suppression accidentelle par un client admin authentifié : l'ID est tombstoné **définitivement**, aucune annulation via l'API | Faible (UX) | inhérent à la décision « permanent » ; confirmation explicite dans l'UI ; réparation root uniquement ; l'alternative « annulation » réintroduit la réutilisation (rejetée) |

### 7.3.1 Risques fermés (preuves)

| # | Risque | Preuve |
|---|---|---|
| **AD-R10** | Rollback après reload refusé alors que `settings.json` était absent : le settings créé avec le tombstone restait sur disque | **Fermé (T1-01)** : `RestoreTarget.existed` + `remove_if_absent` (`transaction.rs:142-160`) ; suppression durable `remove_file` + `fsync` du répertoire (`:335-353,358-383`) ; tests `reload_failure_restores_absence_of_settings_file` (`tests/transaction.rs:422`) et `reload_failure_without_previous_settings_restores_absence` (`service.rs:1324`) |
| **AD-R13** | Les profils fournis par `/etc` comptaient dans la collection « précédente » et pouvaient être tombstonés par un diff d'état | **Fermé** : `load_profiles_state_only_no_recovery` (`config.rs:606-614`) utilisé par `service.rs:514-521`, **aucun repli `/etc`** ; test `etc_profile_is_not_tombstoned_by_state_diff` (`service.rs:1179`) |

---

## 8. Tests de non-régression

### 8.1 Rust

| Fichier | Contenu |
|---|---|
| `crates/tcc-authz/tests/polkit_subject.rs` | sujet, flags, action, `details` vide, pas de cache |
| `crates/tcc-authz/tests/polkit_denied.rs` | refus, challenge, timeout, absence |
| `crates/tcc-authz/tests/polkit_storm.rs` | bucket, dialogue unique, `Busy` |
| `crates/tcc-admin/tests/bounds.rs` | 1 Mio + 1 octet refusé **avant** parse ; 0 profil ; 257 profils ; id 65 octets ; TDP 4 valeurs ; TDP vide ; ODM 65 caractères ; mode PRIME `off` ; cTGP hors borne |
| `crates/tcc-admin/tests/persistence.rs` | fichier valide : 0644 root, `.bak` 0600, contenu identique ; JSON invalide : rien écrit ; profil référencé supprimé : `InUse` ; preset filtré ; `/etc` jamais touché ; symlink cible refusé |
| `crates/tcc-admin/tests/transaction.rs` | reload OK ⇒ `true` ; reload KO ⇒ rollback + `ReloadFailed` si le précédent est exploitable, sinon `ReloadFailed` sans restauration (mono-fichier) ; `.bak` conditionnel ; crash simulé entre write et reload ⇒ fichier valide, pas de temp résiduel ; **double fichier** : profils+settings écrits ensemble, rollback des deux sur reload refusé, **`settings.json` créé supprimé si absent avant**, `WriteFailed` si une restauration/suppression échoue |
| `crates/tcc-admin/src/service.rs` (tests) | **tombstones** : recréation d'un ID supprimé refusée (`Validation("deletedProfileIds")`), nouvel ID accepté, borne 1024 respectée avant écriture, `SaveSettingsJSON` append-only (ajout refusé, retrait réajouté), suppression d'un preset refusée, rollback profils+settings, **profil `/etc` non tombstoné** (`etc_profile_is_not_tombstoned_by_state_diff`), **absence initiale de `settings.json` restaurée** (`reload_failure_without_previous_settings_restores_absence`) |
| `crates/tcc-admin/tests/concurrency.rs` | deux `Save*` simultanés ⇒ un seul écrit, l'autre `Busy` ; état final valide |
| `crates/tcc-admin/tests/idle.rs` | 30 s sans appel ⇒ sortie 0 ; un appel réarme |
| `crates/tcc-admin/tests/admin_audit.rs` | champs d'audit, aucun payload/secret |
| `crates/tcc-admin/tests/phase2.rs` | TDP/ODM/cTGP : bornes, refus hors plage, non-persistance (aucun fichier modifié) |

### 8.2 Intégration système (root, hors CI PR)

Extension de `scripts/release/dbus-policy-test.sh` :

1. `Admin1` : méthode listée joignable (réponse ou `NotAuthorized` polkit, **pas** `AccessDenied` bus) ;
   méthode non listée ⇒ `AccessDenied` bus.
2. `Control1.ReloadConfig` en non-root ⇒ `AccessDenied` bus ; en root ⇒ réponse.
3. `Properties.Set` sur `Admin1` ⇒ refus.
4. `Own` du nom `…Admin` par un non-root ⇒ refus.
5. `validate.sh` : Admin1 membres listés (pas d'allow d'interface), unit admin `CapabilityBoundingSet=`
   vide, `Restart=no`, pas de `[Install]`, aucun `exec.path`/`allow_gui`, aucun `RUN+=`.

### 8.3 Tests manuels (avant activation)

- `pkcheck --action-id com.tuxedocomputers.tccd1.profiles --process $$` en utilisateur non
  administrateur ⇒ dialogue, puis refus si annulé.
- `busctl call … Admin1 SaveProfilesJSON s "$(tcc-cli profiles)"` en root ⇒ succès ; en utilisateur
  ⇒ `NotAuthorized` après dialogue.
- `systemctl stop polkit` (VM) ⇒ tout appel admin ⇒ `NotAuthorized`, aucun fichier modifié.
- `journalctl -u tccd-admin` : une ligne d'audit par appel, aucun payload.
- `ls -l /var/lib/tuxedo-rust-center` : 0644, `.bak` 0600, aucun `.tmp`.
- `systemd-analyze security --offline=true tccd-admin.service` ≤ 2.5 (phase 1).

---

## 9. Critères de sortie avant activation du service

Tous **bloquants** :

1. `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` verts (dont les nouveaux tests §8.1).
2. `validate.sh --repo` étendu et vert ; `dbus-policy-test.sh` étendu et vert (root).
3. Aucun `exec.path`/`exec.argv1`/`allow_gui` dans les policies ; `grep` CI.
4. `systemd-analyze security --offline=true tccd-admin.service` exécuté, score ≤ 2.5 documenté.
5. Test réel : non-root ⇒ dialogue polkit + refus si annulé ; root ⇒ succès ; polkit arrêté ⇒ refus.
6. Fichiers après écriture : 0644 root:root, `.bak` 0600 root:root, aucun temporaire.
7. Rechargement vérifié : `tccd` applique la nouvelle config sans redémarrage ; rollback testé.
8. Audit journald vérifié (champs, absence de payload), y compris sur refus.
9. `docs/DBUS_API.md` mis à jour (Admin1, Control1, sémantique transitoire phase 2) et
   `docs/security/THREAT_MODEL.md` §5 complété des menaces TA-1…TA-16.
10. `docs/packaging/SECURITY.md` mis à jour : actions polkit réellement appliquées, delta §5.3,
    AD-R1 documenté.
11. Revue `security` signée (ce document + diff d'implémentation).
12. Phase 2 : tests matériels TDP/ODM/cTGP/PRIME sur laptop TUXEDO, avec refus polkit vérifié.
13. Tombstones : recréation refusée, borne **1024**, `SaveSettingsJSON` append-only ; transaction
    double fichier et rollback des deux testés ; **T1-01 fermé** (absence initiale restaurée) et
    **AD-R13 fermé** (diff limité à l'état `/var/lib`).

---

## 10. Plan d'implémentation (ordre prescriptif)

| Étape | Contenu | Owner | Dépend de |
|---|---|---|---|
| 1 | `tcc-ipc` : constantes `DBUS_ADMIN_*`, `DBUS_CONTROL_INTERFACE`, `ADMIN_CONTRACT_VERSION`, `AdminReloadReport` | `rust-core` | — |
| 2 | `tcc-core` : `save_*_with_backup`, `write_atomic(mode)` | `rust-core` | étape 1 |
| 3 | `tccd` : interface `Control1.ReloadConfig` + rapport + sérialisation sans cache + vérif uid + policy root-only | `linux-platform` | étape 1 |
| 4 | `tcc-authz` : `CheckAuthorization`, sujet, timeout, bucket, dialogue unique, audit | `rust-core` + `security` | étape 1 |
| 5 | `tcc-admin` phase 1 : `SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig`, transaction, tombstones d'IDs (double fichier), idle exit | `linux-platform` | étapes 1-4 |
| 6 | Packaging : policy D-Bus membres, policy polkit `profiles`, unit phase 1, activation, `install-tree.sh` | `packaging-release` | étape 5 |
| 7 | Tests d'intégration + CI (faux polkit, policy root, rollback) | `qa-integration` | étapes 5-6 |
| 8 | Activation contrôlée : paquet `TCC_WITH_ADMIN=1`, tests manuels §8.3 | `packaging-release` + `security` | étape 7 |
| 9 | Phase 2 : méthodes TDP/ODM/cTGP/PRIME, delta hardening, tests matériels | `tuxedo-hardware` + `linux-platform` | étape 8, AD-12 tranché |
| 10 | Phase 3 : Tomte/WebFAI/diagnostic (services séparés) ; **LUKS hors périmètre, non implémenté** | `linux-platform` + `security` | étape 9 |

**Chemin critique** : 1 → 3 → 4 → 5 → 6 → 7 → 8. Les étapes 2 et 3 peuvent être parallélisées.

---

## Annexe A — Récapitulatif des signatures

| Méthode | Signature D-Bus | Action polkit | Persistant | Phase |
|---|---|---|---|---|
| `Ping` | `() -> b` | — | — | 1 |
| `GetVersion` | `() -> s` | — | — | 1 |
| `SaveProfilesJSON` | `s -> b` | `…tccd1.profiles` | oui (`/var/lib` ; profils + tombstones dans `settings.json`) | 1 |
| `SaveSettingsJSON` | `s -> b` | `…tccd1.settings` | oui (`/var/lib`) | 1 |
| `ReloadConfig` | `() -> b` | `…tccd1.settings` | non | 1 |
| `SetTdpValues` | `ai -> b` | `…tccd1.power-limits` | non | 2 |
| `SetOdmProfile` | `s -> b` | `…tccd1.power-limits` | non | 2 |
| `SetNvidiaCtgpOffset` | `i -> b` | `…tccd1.power-limits` | non | 2 |
| `SetPrimeMode` | `s -> b` | `…tccd1.prime-select` | oui (config système) | 2 |
| `TomteRun` | `as -> s` | `…tccd1.tomte-run` | oui (système) | 3 |
| `PackagesApply` | `as -> s` | `…tccd1.packages` | oui (système) | 3 |

Propriété : `ContractVersion` (`u`) sur `Admin1`. Objet : `/com/tuxedocomputers/tccd/Admin`.

## Annexe B — Checklist de revue du diff d'implémentation

- [ ] Aucun `unwrap`/`expect`/`panic!` hors tests ; `#![forbid(unsafe_code)]` (sauf `secret.rs` de
      phase 3, isolé et commenté).
- [ ] Aucun `Command::new` hors module `exec` allowlisté ; `env_clear()` partout.
- [ ] Aucun chemin construit depuis une entrée D-Bus.
- [ ] Toute méthode d'écriture commence par `authorize(...)` puis `bucket(...)` puis `validate(...)`.
- [ ] Le verrou `Semaphore(1)` couvre lecture-écriture-reload.
- [ ] Les tombstones sont **append-only** : `SaveProfilesJSON` refuse un nouvel ID tombstoné,
      `SaveSettingsJSON` ne peut ni ajouter ni retirer un tombstone, la borne 1024 est respectée
      avant toute écriture.
- [ ] Le diff de tombstones porte **uniquement** sur `profiles.json` d'état (`/var/lib`), jamais
      sur les profils `/etc` (AD-R13).
- [ ] Une suppression de profil écrit `profiles.json` **et** `settings.json` dans la même
      transaction (un seul rechargement) ; le rollback restaure les deux, `.bak` 0600 par fichier,
      et **supprime un fichier créé** qui était absent avant (T1-01).
- [ ] Le rollback est testé (reload KO ⇒ ancien fichier restauré à l'octet près, fichier créé
      supprimé ; échec de restauration ⇒ `WriteFailed`).
- [ ] Les erreurs D-Bus ne contiennent ni chemin, ni valeur brute, ni sortie de commande.
- [ ] Les logs ne contiennent aucun payload, secret, chemin de fichier utilisateur.
- [ ] La policy D-Bus liste chaque membre ; `validate.sh` et `dbus-policy-test.sh` passent.
- [ ] Aucune action polkit `exec.*` ; `profiles` sans `_keep` ; `assign-profile` retirée.
- [ ] `tccd-admin` refuse de démarrer hors root et sort après 30 s d'inactivité.
- [ ] `install-tree.sh` n'installe l'unité/activation qu'avec un binaire présent et `TCC_WITH_ADMIN=1`.
- [ ] `docs/DBUS_API.md`, `THREAT_MODEL.md`, `packaging/SECURITY.md` mis à jour.
