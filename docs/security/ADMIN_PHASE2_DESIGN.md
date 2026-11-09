# Design sécurité — `tccd-admin` phase 2 (TDP / ODM / cTGP / PRIME)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | design **prêt à implémenter** de la phase 2 de `tccd-admin` : limites matérielles (TDP, ODM, cTGP) et bascule PRIME |
| Périmètre d'écriture | `docs/security/**` uniquement — aucun code modifié |
| Documents de référence | `docs/security/ADMIN_DESIGN.md` (AD-1…AD-12), `docs/security/DECISIONS.md` (D1/D2/D4/D5), `docs/security/REVIEW_ADMIN_PHASE1.md`, `docs/security/REVIEW_HARDWARE.md`, `docs/ARCHITECTURE.md` §3.5/§3.7, `docs/DBUS_API.md`, `packaging/{dbus,polkit,systemd}` |
| État de départ vérifié | phase 1 implémentée (`Admin1`, polkit `tccd1.profiles`/`.settings`, transaction + `Control1`) ; policy refusant les membres phase 2 (`com.tuxedocomputers.tccd.conf:176-204`) ; unité phase 1 `PrivateDevices=yes`, `ProtectKernelTunables=yes`, `CapabilityBoundingSet=` vide, score `systemd-analyze security` **1.6** ; `tcc-hw` expose `tdp`, `platform_profile`, `nvidia`, `prime` (lecture seule, `set_mode` retiré) |

---

## 0. Portée et statut

Ce document tranche **AD-12** et fixe le contrat, les unités, les transactions et les tests de la
phase 2. Il est **prescriptif** : les noms, signatures, chemins, codes d'erreur et directives systemd
sont ceux à implémenter. Les choix d'UX (nombre de dialogues, formulation) restent à `qt-ui`, dans
les limites fixées ici.

Ce qui **n'est pas** dans ce document :

- la phase 3 (Tomte/WebFAI/diagnostic), inchangée ; **LUKS est hors périmètre** ;
- l'écriture des valeurs TDP/ODM/cTGP dans les profils : elle reste du ressort de
  `Admin1.SaveProfilesJSON` (phase 1), donc de l'action `tccd1.profiles` ;
- le durcissement de `tccd` (setters `Manager1`), traité par `REVIEW_HARDWARE.md` (H-05/H-06/H-07).

**Règle de lecture** : toute affirmation « vérifié » renvoie à un fichier du dépôt ; toute
affirmation « à valider » est une hypothèse qui doit produire une preuve avant activation.

---

## 1. Décisions clés

| ID | Décision | Justification | Alternative rejetée |
|---|---|---|---|
| **P2-01** | **AD-12 = option B2** : trois services distincts — `Admin` (phase 1, **inchangé**), `Admin.Hw` (TDP/ODM/cTGP) et `Admin.Prime` (PRIME) | trois profils de privilèges réellement différents : aucun périphérique (phase 1), `/dev/tuxedo_io` + sysfs (Hw), `execve` + écritures `/boot`/`/etc`/`/var` (Prime). Un parser de profils ne doit jamais porter l'exécution de `prime-select` (même raisonnement que D1/AD-10) | option A (même unité : relâche tous les garde-fous pour toutes les méthodes) ; `Admin.Hw` unique incluant PRIME (mélange device et exec) |
| **P2-02** | **Interfaces et versions séparées** : `Admin1` reste en version **1** (aucune rupture phase 1) ; `AdminHw1` et `AdminPrime1` en version **1** chacune | la GUI sonde chaque service ; un client phase 1 continue de fonctionner si la phase 2 n'est pas installée | incrémenter `Admin1` (casserait la GUI et mélangerait les surfaces) |
| **P2-03** | Méthodes phase 2 : `SetTdpValues(ai)`, `GetTdpValues() -> ai`, `SetOdmProfile(s)`, `SetOdmProfileOverride(s)`, `SetNvidiaCtgpOffset(i)`, `SetPrimeMode(s)` | demandées ; `Override` est une méthode **distincte** pour que la policy, l'audit et le refus PPD soient explicites | un booléen `override` dans la signature (invisible dans la policy, facile à activer par erreur) |
| **P2-04** | Polkit : actions **existantes** `com.tuxedocomputers.tccd1.power-limits` et `…prime-select`, `auth_admin` **sans `_keep`**, sujet `system-bus-name`, flags `1` | aucun cache : chaque écriture thermique ou bascule graphique exige un dialogue ; réutilise `tcc-authz` (AD-3/AD-4/AD-9) | `auth_admin_keep` (5 min de contournement), `unix-process` (confused deputy) |
| **P2-05** | Erreurs : namespaces dédiés `org.tuxedocomputers.tccd.AdminHw1.Error.*` et `…AdminPrime1.Error.*` ; ajout de `PolicyDenied`, `PartialApply`, `Timeout` | l'UI doit distinguer « PPD propriétaire » de « matériel absent », et « appliqué partiellement » de « rien appliqué » | réutiliser `NotSupported`/`WriteFailed` (ambigu, pousse l'UI à réessayer) |
| **P2-06** | Les setters phase 2 sont **transitoires** (AD-7) : aucune persistance, aucun fichier écrit ; la persistance reste `SaveProfilesJSON` | une seule autorité de persistance ; un admin ne réécrit pas silencieusement un profil | persister la dernière valeur admin (effet de bord invisible) |
| **P2-07** | `platform_profile` : **refus par défaut** si PPD possède le service ; override en **deux étapes** (`Settings.ppd_override = true` via phase 1, puis `SetOdmProfileOverride` + polkit) | D4 ; le flag persistant est le consentement explicite, le polkit est l'authentification par action | override par simple argument (pas de trace durable) ; écriture inconditionnelle (combat de writers) |
| **P2-08** | PRIME implémenté **dans `tccd-admin`** (jamais dans `tcc-hw`) : allowlist de chemins, argument énuméré, `env_clear`, timeout 180 s, sortie bornée, vérification post-écriture | H-01 fermé (retrait de `prime::set_mode`) ; la bascule est une opération root sensible, pas une primitive matérielle réutilisable | réintroduire `set_mode` dans `tcc-hw` (joignable depuis `tccd`) |
| **P2-09** | Transactions : **best-effort avec rollback** pour TDP/ODM/cTGP ; **pas de rollback** pour PRIME (une seconde bascule est plus risquée que l'état intermédiaire) | TDP est non transactionnel par nature (ioctl séquentiels) ; PRIME persiste au reboot | prétendre à une atomicité inexistante ; rollback PRIME automatique |
| **P2-10** | Aucun chemin, aucune commande, aucun JSON libre sur le fil phase 2 : uniquement `ai`, `i`, `s` bornés | réduit la surface à trois scalaires ; un bug de parsing ne peut pas devenir une écriture arbitraire | `a{sv}` ou JSON de « limites » (surface de validation inutile) |

---

## 2. Contrat D-Bus

### 2.1 Constantes `tcc-ipc`

Ajouter dans `crates/tcc-ipc/src/lib.rs` (ou `src/admin.rs`), sans dépendance `zbus` :

```rust
// Phase 2 — limites matérielles (TDP / ODM / cTGP).
pub const DBUS_ADMIN_HW_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Hw";
pub const DBUS_ADMIN_HW_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Hw";
pub const DBUS_ADMIN_HW_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminHw1";
pub const ADMIN_HW_CONTRACT_VERSION: u32 = 1;

// Phase 2 — bascule PRIME (service séparé, cf. AD-12).
pub const DBUS_ADMIN_PRIME_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Prime";
pub const DBUS_ADMIN_PRIME_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Prime";
pub const DBUS_ADMIN_PRIME_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminPrime1";
pub const ADMIN_PRIME_CONTRACT_VERSION: u32 = 1;
```

`Admin1` et `ADMIN_CONTRACT_VERSION = 1` sont **inchangés**. La GUI sonde `Admin.Hw` et
`Admin.Prime` séparément (voir §2.6).

### 2.2 Services, objets et interfaces

| Service (nom bien connu) | Objet | Interface | Contrat | Unit systemd | Binaire |
|---|---|---|---|---|---|
| `com.tuxedocomputers.tccd.Admin` | `/com/tuxedocomputers/tccd/Admin` | `…Admin1` | 1 | `tccd-admin.service` | `tccd-admin` |
| `com.tuxedocomputers.tccd.Admin.Hw` | `/com/tuxedocomputers/tccd/Admin/Hw` | `…AdminHw1` | 1 | `tccd-admin-hw.service` | `tccd-admin-hw` |
| `com.tuxedocomputers.tccd.Admin.Prime` | `/com/tuxedocomputers/tccd/Admin/Prime` | `…AdminPrime1` | 1 | `tccd-admin-prime.service` | `tccd-admin-prime` |

Les trois binaires vivent dans le crate `tcc-admin` (deux `[[bin]]` supplémentaires), partagent
`tcc-authz`, `tcc-ipc`, `tcc-core`. Le crate ajoute une dépendance `tcc-hw` (commune aux trois
binaires, Cargo ne découpe pas par cible) : l'invariant est donc que le binaire phase 1 **n'appelle
aucune fonction matérielle et n'ouvre aucun périphérique au démarrage** — l'ouverture de
`/dev/tuxedo_io` est paresseuse et strictement locale à une méthode Hw.

### 2.3 Invariants communs

| Invariant | Règle |
|---|---|
| Identité | `header.sender()` obligatoire ; `GetConnectionUnixUser` avant polkit ; échec ⇒ `NotAuthorized` |
| Autorisation | `CheckAuthorization` avant **toute** écriture, sujet `system-bus-name`, flags `1`, timeout 120 s, `is_challenge` refusé (réutiliser `tcc-authz::check_authorization_with`) |
| Concurrence | un seul appel d'écriture à la fois par service (`Semaphore(1)`, `try_acquire` ⇒ `Busy`) ; les lectures ne prennent pas le verrou |
| Débit | écritures : 2 appels/s, burst 4, par nom unique **et** par UID ; lectures : 2/s burst 5 ; `RateLimited` sans dialogue |
| Taille | aucune chaîne > 64 caractères ; `ai` ≤ 3 éléments ; aucune allocation pilotée par le client |
| Audit | une ligne par appel (`action`, `method`, `caller`, `caller_uid`, `polkit`, `result`, `duration_ms`), jamais de valeur brute, jamais de sortie de commande |
| Erreurs | namespace du service ; messages = **codes de raison** ASCII `[a-z0-9._-]{1,64}` uniquement (jamais de texte libre, jamais de chemin) |
| Idempotence | valeur identique ⇒ aucun accès matériel, `true`, audit `noop=true` |
| Inactivité | sortie 30 s après le dernier appel **autorisé** (réutiliser `Activity`/`spawn_idle_watch`) ; `RuntimeMaxSec` couvre le pire cas d'un appel |
| Pas de chemins | aucune méthode n'accepte de chemin, device, variable d'environnement ou commande |

### 2.4 Méthodes `AdminHw1`

#### `SetTdpValues(ai values) -> b`

- **Action** : `com.tuxedocomputers.tccd1.power-limits`.
- **Entrée** : 0 à 3 entiers (watts), dans l'ordre `pl1`, `pl2`, `pl4`. `[]` est une consigne
  **explicite** : appliquer les maxima du firmware (même comportement que
  `TdpControl::set_values(&[])`), avec audit `values=empty, maxima=true` ; no-op si le matériel est
  déjà aux maxima. La distinction avec un profil est volontaire : `profile.tdpValues = []` signifie
  « ne pas changer » côté worker, tandis que `SetTdpValues([])` est une action admin explicite.
- **Validation avant polkit** (ne révèle pas les bornes firmware) : `0 <= len <= 3`, chaque valeur
  `1..=1000`. Erreur ⇒ `InvalidArgs("tdp.count")` / `Validation("tdp.values")`.
- **Traitement** :
  1. `TuxedoIo::open()` ; échec ⇒ `NotSupported("tdp")` (pas d'ouverture au démarrage).
  2. `TdpControl::new(io).limits()` ; liste vide ⇒ `NotSupported("tdp")`.
  3. Construire la cible : `[]` ⇒ maxima firmware, sinon les valeurs fournies. Appeler
     `set_values(&target)` **une seule fois** : `tcc-hw` valide l'entier, la plage firmware et la
     monotonie `PL1 ≤ PL2 ≤ PL4` **avant** la première écriture (`tdp.rs:120-176`). Toute erreur ⇒
     `Validation("tdp.values")` ou `NotSupported`.
  4. Vérifier après écriture que `limits()` reflète la cible (tolérance 0 ; pour `[]`, toutes les
     valeurs aux maxima) ; sinon rollback (§5.2).
- **Sortie** : `true` si toutes les valeurs sont appliquées **et** vérifiées.
- **Erreurs** : `NotAuthorized`, `InvalidArgs`, `Validation`, `NotSupported`, `WriteFailed`,
  `PartialApply`, `RateLimited`, `Busy`.
- **Audit** : `values=empty|set`, `maxima=true|false`, `applied=pl1,pl2`, `count`,
  `restore=ok|failed|none`, `noop`, **jamais** les watts.

#### `GetTdpValues() -> ai`

- **Sans polkit** (lecture), **rate limité** (2/s burst 5), identité résolue pour l'audit.
- Retourne les valeurs courantes dans l'ordre des descripteurs exposés (0 à 3 éléments).
- `NotSupported("tdp")` si `TuxedoIo` absent ou `tdp_count == 0`.
- Aucun état mis en cache : chaque appel relit le matériel (le but est de vérifier un setter).

#### `SetOdmProfile(s name) -> b`

- **Action** : `…power-limits`.
- **Validation** : nom non vide, ≤ 64 caractères, ASCII graphique sans `\` (même règle que
  `platform_profile.rs::validate_name`), **et** appartenant à une allowlist découverte au runtime :
  - `PlatformProfile::detect(...).choices()` (source `tuxedo_platform_profile` ou ACPI) ;
  - `TuxedoIo::odm_profiles()` (fallback ioctl).
  Un nom hors allowlist ⇒ `Validation("odm.name")` (jamais « profil inconnu : <nom> »).
- **Sélection de la voie, dans cet ordre** :
  1. si le nom ∈ `choices` : exiger que PPD ne possède pas `platform_profile` (§3.3). Refus ⇒
     `PolicyDenied("ppd.platform_profile")`. Écrire via `PlatformProfile::apply(root, name,
     ProfileOwnership::TccOwned)` ;
  2. sinon si le nom ∈ `odm_profiles` : écrire via `TuxedoIo::set_odm_profile(name)`. **Autorisé même
     si PPD est actif** : PPD ne connaît pas les profils ODM du module `tuxedo_io` (documenté dans
     `hardware/odm.rs:26-34`), il n'y a pas de conflit ;
  3. sinon `Validation("odm.name")`.
  **Interdiction** de basculer silencieusement de la voie 1 vers la voie 2 quand le même nom existe
  dans les deux listes et que PPD bloque : ce serait contourner D4 par un autre chemin.
- **Vérification** : voie 1 ⇒ relire `PlatformProfile::current()` ; voie 2 ⇒ **aucune lecture
  courante n'est exposée** par `tuxedo_io` : succès de l'ioctl = succès, audit `verify=none`.
- **Erreurs** : idem TDP + `PolicyDenied`.
- **Audit** : `source=platform|io`, `name` (nom issu de l'allowlist, donc sûr), `ppd=owned|absent|activatable`,
  `override=false`.

#### `SetOdmProfileOverride(s name) -> b`

- Même validation et même sélection de voie que `SetOdmProfile`.
- **Conditions cumulatives** :
  1. `Settings.ppd_override == true` lu **sur disque** (`ConfigStore::system()`, `/var/lib`, lecture
     seule) au moment de l'appel ; sinon `PolicyDenied("ppd.override_disabled")` — même si PPD est
     absent (le flag documente l'intention) ;
  2. polkit `…power-limits` (dialogue systématique) ;
  3. PPD possédé ⇒ écriture autorisée (override), audit `override=true`.
- **Portée** : transitoire, comme `SetOdmProfile` ; `tccd` réappliquera le profil actif au prochain
  changement (avec `ppd_override` également lu depuis les settings, cf. §7.4).
- **Erreurs** : `NotAuthorized`, `InvalidArgs`, `Validation`, `PolicyDenied`, `NotSupported`,
  `WriteFailed`, `RateLimited`, `Busy`.
- **Note d'implémentation (mise à jour 2026-11-06)** : `Settings.ppd_override` **est propagé** par
  `tccd` (`main.rs:84-94,128-135,459-510`) avec un log unique et des tests
  (`hardware_inputs_follow_ppd_override_setting`, `hardware_runtime_reloads_ppd_override_flag`).
  `Admin.Hw` le relit sur `/var/lib` avant `SetOdmProfileOverride` ; la persistance réelle
  (Admin1 → disque → relecture) reste à valider sur machine.

#### `SetNvidiaCtgpOffset(i watts) -> b`

- **Action** : `…power-limits`.
- **Validation** : `0 <= watts <= max_offset`, avec
  `max_offset = min(200, max_watts - default_watts)` si les limites NVIDIA sont connues, sinon
  `200` (borne statique, H-08). `max_watts`/`default_watts` proviennent de
  `NvidiaPowerLimits` (`nvidia.rs:79-84`), lus de préférence depuis l'instantané `tccd`
  (`GetHardwareSnapshotJSON`, timeout 2 s, taille ≤ 4 Mio) pour **ne pas exécuter `nvidia-smi`**
  dans le service Hw ; si l'instantané est indisponible/expiré, repli statique, audit `bound=static`.
- **Traitement** : `CtgpControl::set_offset_watts(watts as f64)` (entier déjà garanti par `i`),
  puis `offset()` pour vérifier. Valeur identique ⇒ `true` sans écriture (`noop=true`).
- **Erreurs** : `Validation("nvidia.ctgpOffset")`, `NotSupported("ctgp_offset")`, `WriteFailed`.
- **Audit** : `bound=dynamic|static`, `noop`, jamais la valeur.

### 2.5 Méthode `AdminPrime1`

#### `SetPrimeMode(s mode) -> b`

- **Action** : `com.tuxedocomputers.tccd1.prime-select`.
- **Entrée** : exactement `intel`, `nvidia` ou `on-demand` (casse stricte). Tout le reste, y compris
  `off`/`unknown`, ⇒ `InvalidArgs("prime.mode")` — ce sont des états de lecture, pas des modes
  demandables.
- **Pré-vol** (§6.2) : outil allowlisté résolu, `PrimeState::is_supported()`, mode courant lu par
  `prime-select query` (timeout 3 s). Mode identique ⇒ `true`, audit `noop=true`, **aucun exec de
  bascule**.
- **Exécution** : `command::run(program, &[argument], PRIME_SWITCH_TIMEOUT)` — chemin absolu,
  `env_clear` + `PATH`/`LC_ALL`/`LANG`/`HOME` fixes, `stdin=null`, stdout/stderr bornés 64 Kio,
  timeout 180 s (`command.rs:140-223`).
- **Post-vol** : `prime::query_mode` (3 s) ; si le mode relu ≠ cible ⇒ `WriteFailed` (audit
  `verify=mismatch`), **sauf** comportement `verify=deferred` validé sur matériel (§6.4).
- **Erreurs** : `NotAuthorized`, `InvalidArgs`, `Validation`, `NotSupported`, `WriteFailed`,
  `Timeout`, `RateLimited`, `Busy`.
- **Audit** : `from`, `to`, `reboot_required=true`, `verify=ok|mismatch|unavailable`, jamais la
  sortie de `prime-select`.
- **Sémantique** : persistant par nature (configuration système) ; effet complet après redémarrage
  (ou fermeture de session selon le mode). Aucun rollback (§5.5).

### 2.6 Diagnostic et sonde côté client

Chaque service expose, sans polkit et sans rate limit (aucun effet de bord) :

| Méthode | Signature | Rôle |
|---|---|---|
| `Ping` | `-> b` | vivacité |
| `GetVersion` | `-> s` | version d'implémentation (≤ 64 caractères, ASCII graphique) |
| `GetContractVersion` | `-> u` | comparaison **stricte** avec la constante attendue |

Règles côté client (leçons G1-05) :

1. `NameHasOwner` puis `ListActivatableNames` avec un **timeout de 1 s** ; ne jamais appeler
   `Ping`/`GetContractVersion` si le nom n'est pas déjà possédé (pour ne pas activer le service root
   toutes les 2 s).
2. Contrat incompatible ⇒ `Contract` et bouton désactivé (fail-closed), jamais un appel « au
   hasard ».
3. Un échec d'activation (`ServiceUnknown`, binaire absent, `StartLimit`) doit être mappé vers
   `Unavailable`, pas vers `Transport`/`Unknown`.
4. Timeouts client (incluent le dialogue polkit) : `GetTdpValues` 10 s ; `SetTdpValues`,
   `SetOdmProfile`, `SetNvidiaCtgpOffset` **180 s** ; `SetPrimeMode` **360 s**. Un `Timeout` client
   **n'annule pas** l'opération admin : relire l'état avant de réessayer.

### 2.7 Erreurs

Namespace et codes (messages = code de raison, jamais de texte libre) :

| Code | `AdminHw1` | `AdminPrime1` | Sens |
|---|---|---|---|
| `NotAuthorized` | oui | oui | polkit refusé/absent/timeout, identité inconnue |
| `InvalidArgs` | oui | oui | type/arité/valeur hors grammaire (`tdp.count`, `odm.name`, `prime.mode`) |
| `Validation` | oui | oui | valeur syntaxiquement valide mais refusée (`tdp.values`, `nvidia.ctgpOffset`) |
| `PolicyDenied` | oui | — | PPD propriétaire ou override non autorisé (`ppd.platform_profile`, `ppd.override_disabled`) |
| `NotSupported` | oui | oui | matériel/outil absent (`tdp`, `platform_profile`, `ctgp_offset`, `prime`) |
| `WriteFailed` | oui | oui | écriture/commande/vérification en échec |
| `PartialApply` | oui | — | état matériel intermédiaire non restauré (liste bornée `pl1,pl2,pl4`) |
| `Timeout` | — | oui | `prime-select` au-delà de 180 s |
| `RateLimited` | oui | oui | bucket dépassé, sans dialogue |
| `Busy` | oui | oui | écriture ou dialogue déjà en cours |

Messages exacts : `tdp.count`, `tdp.values`, `tdp`, `odm.name`, `platform_profile`, `ctgp_offset`,
`nvidia.ctgpOffset`, `ppd.platform_profile`, `ppd.override_disabled`, `prime.mode`, `prime`,
`pl1,pl2,pl4`. Toute valeur hors de cette grammaire est un bug de test, pas une donnée utilisateur.

---

## 3. Autorisation

### 3.1 Actions polkit (existantes, à réutiliser)

| Action | Défaut | Méthodes | Remarque |
|---|---|---|---|
| `com.tuxedocomputers.tccd1.power-limits` | `auth_admin`, sans cache | `SetTdpValues`, `GetTdpValues`? non, `SetOdmProfile`, `SetOdmProfileOverride`, `SetNvidiaCtgpOffset` | mise à jour des textes : mentionner TDP **et** ODM, conséquence thermique |
| `com.tuxedocomputers.tccd1.prime-select` | `auth_admin`, sans cache | `SetPrimeMode` | message à compléter : « la session/le redémarrage est requis » |

Aucune nouvelle action. Aucun `allow_any`/`allow_inactive`. Aucun `auth_admin_keep`.

### 3.2 Sujet et fail-closed

Réutiliser `tcc-admin/src/service.rs:325-390` (`authorize_write`) et `tcc-authz` :

- sujet `system-bus-name` du **nom unique** de l'appelant (jamais un PID fourni) ;
- flags `1` (`AllowUserInteraction`), `details = {}`, `cancellation_id = ""` ;
- polkit absent/erreur/timeout ⇒ `NotAuthorized` + audit `result=polkit_unavailable` ;
- `is_challenge=true` ⇒ refus ;
- **uid 0 vérifié aussi** (aucun court-circuit).

### 3.3 Propriété PPD

Fonction interne `ppd_ownership(conn) -> PpdOwnership` (timeouts 1 s) :

1. `NameHasOwner("net.hadess.PowerProfiles")` :
   - `true` ⇒ **`PpdOwned`** ;
   - `false` ⇒ `ListActivatableNames` : si présent ⇒ `PpdActivatable`, sinon `PpdAbsent` ;
   - erreur de bus ⇒ `Unknown` ⇒ traité comme **`PpdOwned`** (fail-closed).
2. `PpdOwned` interdit `SetOdmProfile` (voie `platform_profile`) ⇒ `PolicyDenied`.
3. `PpdActivatable` autorise l'écriture (PPD n'est pas actif) mais l'audit porte
   `ppd=activatable` ; si PPD démarre pendant l'appel, un contrôle **après** écriture met
   `ppd_race=true` dans l'audit (ne pas restaurer : PPD va réécrire sa propre valeur).
4. `PpdAbsent` ⇒ écriture TCC normale.

`platform_profile` n'est **jamais** écrit quand PPD est possédé, sauf `SetOdmProfileOverride`.

### 3.4 Override en deux étapes

1. **Consentement persistant** : `Settings.ppd_override = true`, écrit uniquement par
   `Admin1.SaveSettingsJSON` (action `tccd1.settings`, transaction phase 1). Le champ existe dans
   `Settings` (schéma + migration) et est exposé par l'UI ; la persistance réelle reste à valider.
2. **Authentification par action** : `SetOdmProfileOverride` + polkit `power-limits`.
3. Si le flag est faux ⇒ `PolicyDenied("ppd.override_disabled")`, **avant** lecture matérielle et
   avant le dialogue polkit (le rate-limit reste avant).
4. L'écriture reste transitoire ; `tccd` lit le même flag (`main.rs:84-94,128-135,459-510`) pour
   réappliquer de façon cohérente.

### 3.5 Anti-dialog-storm

- Limiteur `tcc-authz` (2/s burst 4, par nom unique et UID) **avant** polkit ; `RateLimited` sans
  dialogue.
- `DialogGate` global (un seul dialogue) + `Busy` si le même appelant a déjà un dialogue.
- **Limite structurelle** : le service s'arrête après 30 s ; chaque activation D-Bus crée un
  processus neuf et **réinitialise les buckets en mémoire**. La protection contre les rafales
  inter-activations repose donc sur : polkit `auth_admin` (dialogue à chaque appel), `StartLimit*`
  et la porte qualité UI. À documenter ; ne pas prétendre à un quota persistant.
- `StartLimitIntervalSec=60` / `StartLimitBurst=20` sur les unités phase 2 (une activation légitime
  peut échouer puis être relancée ; 20 laisse de la marge sans permettre un storm).

### 3.6 Audit

Champs obligatoires (une ligne par appel, `tracing`) :

```text
action=set-tdp method=SetTdpValues caller=:1.42 caller_uid=1000
polkit=com.tuxedocomputers.tccd1.power-limits result=ok duration_ms=38
values=set maxima=false applied=pl1,pl2 count=2 noop=false restore=none
```

| Méthode | Champs spécifiques (sans valeur brute) |
|---|---|
| `SetTdpValues` | `values=empty|set`, `maxima=true|false`, `count`, `applied` (descripteurs), `restore=none|ok|failed`, `noop` |
| `GetTdpValues` | `count` |
| `SetOdmProfile` / `Override` | `source=platform|io`, `name` (allowlist), `ppd`, `override` |
| `SetNvidiaCtgpOffset` | `bound=dynamic|static`, `noop` |
| `SetPrimeMode` | `from`, `to`, `reboot_required=true`, `verify` |

Interdits : valeurs TDP/cTGP, sortie de commande, chemin, contenu de fichier, environnement.

---

## 4. Séparation des privilèges — AD-12

### 4.1 Options

| Critère | **A — même unité** | **B — service séparé** (recommandé) |
|---|---|---|
| Unités | `tccd-admin.service` relâchée pour toutes les méthodes | `tccd-admin.service` **inchangée** + `tccd-admin-hw.service` (+ `-prime`) |
| `PrivateDevices` | `no` pour tous | `yes` phase 1, `no` Hw, `yes` Prime |
| `ProtectKernelTunables` | `no` pour tous | `yes` phase 1/Prime, `no` Hw |
| `CapabilityBoundingSet` | vide (mais à revalider) | vide partout ; à mesurer |
| `SystemCallFilter` | `@system-service` + `execve` + ioctl à valider pour tous | `@system-service` en Hw ; Prime à valider séparément |
| `ReadWritePaths` | `/var/lib` + `/boot` + `/etc` (PRIME) pour tous | phase 1 `/var/lib` ; Hw aucun ; Prime `/boot` (+ chemins mesurés) |
| Surface exposée par processus | `SaveProfilesJSON` (JSON 1 Mio, parser) **et** ioctl/exec | un seul type d’opération par processus |
| Compromission du parser de profils | atteint `/dev/tuxedo_io` et `prime-select` | atteint uniquement `/var/lib` (phase 1) |
| Coût | nul | 2 unités + 2 fichiers d’activation + 2 `[[bin]]` |
| Score `systemd-analyze security` | se dégrade pour la phase 1 (1.6 → ~3–4) | phase 1 reste 1.6 ; Hw/Prime mesurés séparément |
| Verdict | **rejeté** | **retenu (B2)** |

### 4.2 Option A (rejetée) — delta exact de l'unité phase 1

Pour mémoire, l'option A consiste à modifier `packaging/systemd/tccd-admin.service` ainsi :

```ini
# Delta phase 2 dans l'unité existante (phase 1 + phase 2).
RuntimeMaxSec=12min
TasksMax=64
UMask=0022

# Périphérique TDP/ODM ioctl.
PrivateDevices=no
DevicePolicy=closed
DeviceAllow=/dev/tuxedo_io rw

# Écritures sysfs (platform_profile, ctgp_offset).
ProtectKernelTunables=no

# PRIME : écritures système + initramfs.
ProtectSystem=strict
ReadWritePaths=/var/lib/tuxedo-rust-center /boot -/var/lib/ubuntu-drivers-common -/etc/prime-discrete

# prime-select et ses sous-outils (update-initramfs, update-grub).
# @system-service inclut execve ; si EPERM, ajouter les syscalls mesurés.
SystemCallFilter=@system-service
```

Capacités : `CapabilityBoundingSet=` vide (inchangé), à revalider avec les mêmes preuves que §4.6.
`DeviceAllow` : `/dev/tuxedo_io rw` uniquement.

Pourquoi c'est rejeté :

1. `SaveProfilesJSON` — un parser de **1 Mio de JSON non fiable** — s'exécute dans le même processus
   qui peut `execve` `prime-select` et écrire `/boot` ; c'est une violation directe de D1.
2. La phase 1 perd `PrivateDevices=yes` et `ProtectKernelTunables=yes` alors qu'elle n'en a pas
   besoin : tout le monde paie le risque d'une seule méthode.
3. Le score `systemd-analyze security` de la phase 1 passerait de **1.6** à ~3–4, sans possibilité
   de le mesurer par méthode.
4. `ReadWritePaths=/boot` et l'`execve` deviennent permanents pour toutes les activations, y compris
   un simple `SaveSettingsJSON`.

### 4.3 Recommandation B2 (deux nouveaux services)

Scinder encore Hw et Prime, car leurs besoins sont incompatibles :

- **Hw** : `no` devices (sauf `/dev/tuxedo_io`), `ProtectKernelTunables=no`, **aucun `execve`**,
  aucune écriture fichier. Surface : trois scalaires.
- **Prime** : `execve` de `prime-select` et de ses sous-outils, écritures `/boot` et éventuellement
  `/etc`/`/var`, aucun périphérique, `ProtectKernelTunables=yes`, timeout long. Surface : une chaîne
  énumérée.

Un processus unique « Hw+Prime » obligerait à autoriser `execve` et des écritures système dans le
processus qui écrit `/sys` et `/dev/tuxedo_io` : exactement le mélange que D1 cherche à éviter.

### 4.4 `tccd-admin-hw.service`

```ini
[Unit]
Description=TuxedoRustCenter on-demand hardware limits (TDP, ODM, cTGP)
Documentation=file:/usr/share/doc/tuxedo-rust-center/ADMIN_DESIGN.md
After=dbus.service systemd-modules-load.service
StartLimitIntervalSec=60
StartLimitBurst=20

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin.Hw
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-hw
Restart=no
TimeoutStartSec=10
TimeoutStopSec=10
# Un appel inclut le dialogue polkit (120 s) ; marge au-delà.
RuntimeMaxSec=5min
KillMode=control-group

UMask=0077
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
# Phase 2 : accès à /dev/tuxedo_io uniquement.
PrivateDevices=no
DevicePolicy=closed
DeviceAllow=/dev/tuxedo_io rw
ProtectProc=invisible
ProcSubset=pid
# Nécessaire aux écritures /sys (platform_profile, ctgp_offset).
ProtectKernelTunables=no
# Compensation : tout le reste du noyau reste en lecture seule.
ReadOnlyPaths=/proc/sys /sys/kernel /sys/module
ReadOnlyPaths=/sys/devices/system/cpu /sys/class/leds /sys/class/backlight
ReadOnlyPaths=/sys/class/power_supply /sys/class/hwmon /sys/class/thermal /sys/class/drm
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
# ioctl est couvert par @system-service ; à confirmer par strace sur matériel réel.
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=256M
TasksMax=16

CapabilityBoundingSet=
AmbientCapabilities=

# Pas de [Install] : activation D-Bus uniquement.
# Pas de StateDirectory/ConfigurationDirectory : sans état.
```

Notes :

- `ReadOnlyPaths=/sys/kernel` couvre `debug`, `tracing`, `security` ; `/proc/sys` reste le seul
  tunable noyau potentiellement inscriptible, et il est remis en lecture seule. C'est la
  compensation minimale de `ProtectKernelTunables=no`.
- `DeviceAllow` ne restreint **pas** les écritures sysfs ; il ne limite que les nœuds de
  périphérique. La restriction sysfs vient de `ReadOnlyPaths` + de l'absence de chemin contrôlable
  dans le code.
- Si le module `tuxedo_io` n'est pas chargé au démarrage de l'unité, `DeviceAllow` sur un chemin
  inexistant est ignoré et `DevicePolicy=closed` refusera l'ouverture : la méthode renvoie
  `NotSupported`, et l'unité doit être relancée après chargement du module. Documenter ; ne pas
  ajouter `ConditionPathExists` (l'activation D-Bus échouerait sans erreur applicative).
- `MemoryMax=256M` : corps D-Bus borné à **64 Kio** (`BoundedAdminHwInterface`) et file
  `max_queued(8)` ; aucune allocation pilotée par le client au-delà de ces bornes.
- Dépendance `tcc-hw` **sans** la feature `dev-overrides` (`default-features = false`) : le binaire
  installé ne doit pas pouvoir lire `TCC_SYSFS_ROOT`. Test CI : `cargo tree -e features` ne contient
  pas `dev-overrides` pour les binaires admin.
- Score cible : **≤ 3.0**, mesuré et consigné (à comparer aux 1.6 de la phase 1).

### 4.5 `tccd-admin-prime.service`

```ini
[Unit]
Description=TuxedoRustCenter on-demand graphics mode switch (PRIME)
Documentation=file:/usr/share/doc/tuxedo-rust-center/ADMIN_DESIGN.md
After=dbus.service
StartLimitIntervalSec=60
StartLimitBurst=10

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin.Prime
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-prime
Restart=no
TimeoutStartSec=10
TimeoutStopSec=30
# polkit 120 s + prime-select 180 s + post-vérification 5 s.
RuntimeMaxSec=12min
# Tue aussi les descendants de prime-select en cas d'arrêt/timeout.
KillMode=control-group

# prime-select crée des fichiers système : 0644/0755 attendus.
UMask=0022
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=strict
# Écritures strictement nécessaires (à confirmer par strace, cf. §6.2) :
# initramfs + état ubuntu-drivers ; /etc/prime-discrete seulement s'il existe.
ReadWritePaths=/boot -/var/lib/ubuntu-drivers-common -/etc/prime-discrete
ProtectHome=yes
PrivateTmp=yes
# Aucun périphérique nécessaire.
PrivateDevices=yes
DevicePolicy=closed
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
# À valider : prime-select exécute update-initramfs/update-grub.
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=256M
TasksMax=64

CapabilityBoundingSet=
AmbientCapabilities=
```

**Décision conditionnelle (à trancher pendant la validation, §6.2)** : si `strace -f -e trace=file`
montre que `prime-select` crée des fichiers dans `/etc` autres que `/etc/prime-discrete`, ou si
`/etc/prime-discrete` n'existe pas au démarrage, **ne pas** élargir à `/etc` : passer cette seule
unité à `ProtectSystem=full` (qui laisse `/etc` et `/var` inscriptibles, mais garde `/usr` en
lecture seule) et documenter l'écart dans `packaging/SECURITY.md`. C'est le seul service root du
projet qui peut écrire `/etc` ; il n'expose qu'une méthode à argument énuméré.

Score cible : **≤ 4.5**, mesuré.

### 4.6 Capacités et `DeviceAllow` : ce qui doit être mesuré

| Élément | Hypothèse | Preuve attendue |
|---|---|---|
| `CapabilityBoundingSet=` vide suffit pour `/dev/tuxedo_io` | le nœud est `root:root 0600` ou `root:<groupe> 0660`, root est propriétaire | `stat`, ouverture + ioctl de lecture/écriture sur Clevo **et** Uniwill |
| `CapabilityBoundingSet=` vide suffit pour `platform_profile`/`ctgp_offset` | fichiers sysfs `0644 root:root` | écriture réelle sur les deux familles |
| `DeviceAllow=/dev/tuxedo_io` est résolu au démarrage | module chargé avant l'activation | `systemd-analyze security`, `strace -e openat` |
| `@system-service` autorise `ioctl` | `ioctl` est dans le groupe par défaut | `strace -f -e trace=ioctl` sans `EPERM` |
| `ProtectKernelTunables=no` + `ReadOnlyPaths` n'empêche pas `platform_profile` | le chemin reste inscriptible | écriture ODM réelle |
| Prime : `@system-service` suffit à `update-initramfs`/`update-grub` | aucun `mount`/`@privileged` requis | bascule réelle sur TUXEDO, `strace` en cas d'`EPERM` |

Toute capacité ajoutée doit être justifiée, mesurée, et revue par `security` (règle `ADMIN_DESIGN`
§5.3).

### 4.7 Ce que le bac à sable ne protège pas

- `ReadWritePaths`/`ReadOnlyPaths` ne filtrent pas les écritures **dans** un chemin autorisé :
  `ProtectKernelTunables=no` laisserait écrire n'importe quel fichier sous `/sys/devices` non listé.
  La vraie barrière est l'absence de chemin fourni par le client + un code minimal.
- Un processus root compromis peut toujours parler D-Bus et appeler `tccd`/polkit dans les limites
  de la policy ; le bac à sable réduit la casse, il ne l'annule pas.
- `DeviceAllow` ne protège pas des ioctls dangereux sur `/dev/tuxedo_io` : la validation des valeurs
  reste dans `tcc-hw` (plages firmware) et dans l'admin (compteur/entier).

---

## 5. Transactions et rollback

### 5.1 Principes

1. **Aucune écriture avant validation complète** de l'entrée et, quand c'est possible, des bornes
   matérielles.
2. **Rollback best-effort** vers les valeurs **relues avant** l'opération ; il n'est pas garanti
   (le matériel peut refuser).
3. **Aucun état persistant** : pas de journal, pas de fichier de reprise. La vérité est le matériel,
   relu à chaque appel.
4. **Une seule écriture à la fois** par service (`Semaphore(1)`), et pas de file d'attente.
5. **Vérification après écriture** quand le matériel expose une lecture ; sinon audit
   `verify=none` (ne pas prétendre à une vérification).

### 5.2 TDP

```
lire limits() (current, min, max)
valider la cible (entier, 1..=3, plage, monotonie)
si cible == current  -> noop, return true
écrire via set_values(cible)        # séquentiel PL1 -> PL2 -> PL4
relire limits()
si current == cible -> true
sinon :
    tenter set_values(précédent)    # même ordre, best-effort
    relire limits()
    si current == précédent -> WriteFailed (net : rien changé, restore=ok)
    sinon -> PartialApply(descripteurs non restaurés), restore=failed
```

- **Que restaurer ?** Les valeurs **relues avant** l'appel, pour tous les descripteurs exposés, pas
  seulement ceux de la cible (une écriture courte ne touche que les premiers, mais un rollback
  complet est plus sûr).
- **Ordre** : croissant (`pl1`, `pl2`, `pl4`) à l'application **et** au rollback ; c'est l'ordre de
  `tdp.rs:189-201` et celui que le firmware attend pour la monotonie.
- **Échec partiel** : `PartialApply("pl2,pl4")` — liste bornée aux descripteurs, jamais de valeur.
- **Crash** : si le processus meurt entre deux ioctls, l'état reste mixte. `tccd` ne réapplique le
  profil qu'au prochain changement (le tick ne fait que lire) : documenter, et recommander à l'UI de
  relire `GetTdpValues` au retour sur l'écran.

### 5.3 ODM

| Voie | Écriture | Vérification | Rollback |
|---|---|---|---|
| `platform_profile` | `PlatformProfile::apply` (une écriture sysfs, idempotente si déjà courant) | relecture `current()` | réappliquer l'ancien nom s'il est connu et non nul ; sinon `WriteFailed` sans rollback |
| `tuxedo_io` | `set_odm_profile` (un ioctl) | **impossible** (pas de lecture courante) | impossible ; succès de l'ioctl = succès, audit `verify=none` |

- Un nom hors allowlist runtime ne doit **jamais** atteindre `apply` (qui valide aussi, défense en
  profondeur).
- Si PPD possède `platform_profile`, pas d'écriture (sauf override) : aucun rollback n'est nécessaire.
- En cas d'échec après écriture (relecture différente), `WriteFailed` et audit de l'état atteint.

### 5.4 cTGP

- Écriture unique ; relire `offset()`.
- Si la relecture diffère de la cible : retenter une fois la valeur précédente (best-effort), relire,
  puis `WriteFailed` (`restore=ok|failed`).
- GPU suspendu/absent : `NotSupported` ou `WriteFailed` ; ne pas boucler, proposer un nouvel essai à
  l'utilisateur.

### 5.5 PRIME

- **Pas de rollback automatique** : une seconde bascule immédiate double le coût (initramfs) et peut
  laisser le système dans un état pire.
- Une seule exécution ; vérification par `prime-select query`.
- En cas d'échec de la commande : `WriteFailed` (code de sortie/stderr nettoyés côté journal
  uniquement) ; ne jamais relayer la sortie.
- En cas de timeout : le processus est tué (y compris ses descendants via le cgroup) ; `Timeout` ;
  l'état est relu une dernière fois pour l'audit, sans nouvelle tentative.

### 5.6 Reprise après crash

| Opération | État possible après crash | Reprise |
|---|---|---|
| TDP | mixte (PL1 écrit, PL2 non) | relire `GetTdpValues` ; réappliquer une cible complète ; l'UI signale l'état |
| ODM `platform_profile` | écrit ou non (une écriture sysfs) | relire le profil courant ; réappliquer |
| ODM `tuxedo_io` | écrit ou non (un ioctl) | pas de lecture ; réappliquer au besoin |
| cTGP | écrit ou non | relire `offset()` |
| PRIME | `prime-select` interrompu (initramfs partiel) | relancer une bascule complète ; si le système ne démarre plus, secours utilisateur (hors API) |

`RuntimeMaxSec` et `KillMode=control-group` empêchent un orphelin de survivre à l'unité.

### 5.7 Idempotence et ordre

- Un setter identique à l'état courant retourne `true` sans écrire (après polkit, donc l'utilisateur
  s'est authentifié) ; audit `noop=true`.
- L'ordre entre méthodes n'est pas garanti : la GUI les envoie séquentiellement, chacune avec son
  dialogue. Aucune méthode ne dépend d'une autre.
- Une écriture phase 2 peut être écrasée par `tccd` au prochain changement de profil : c'est la
  sémantique AD-7, documentée dans `DBUS_API.md` et affichée « temporaire » par l'UI.

---

## 6. PRIME

### 6.1 Emplacement

- Implémentation dans `crates/tcc-admin/src/prime/` (nouveau module), **pas** dans `tcc-hw`.
- `tcc-hw` fournit uniquement : `command::{PRIME_SELECT, resolve, run}`, `prime::{detect,
  query_mode, PrimeMode, PrimeState}` (lecture), `SysfsRoot`, `TuxedoDevice`.
- Test CI bloquant (`crates/tcc-daemon/tests/no_admin_reachability.rs`, déjà en place) : `tccd` ne
  doit référencer ni `prime-select`, ni `SetPrimeMode`, ni le crate admin.

### 6.2 Allowlist, arguments et pré-vol

```rust
// tcc-admin/src/prime/mod.rs
const PRIME_SWITCH_TIMEOUT: Duration = Duration::from_secs(180);
const PRIME_QUERY_TIMEOUT: Duration = Duration::from_secs(3);

fn argument(mode: &str) -> Option<&'static str> {
    match mode {
        "intel" => Some("intel"),
        "nvidia" => Some("nvidia"),
        "on-demand" => Some("on-demand"),
        _ => None,
    }
}
```

Pré-vol, dans l'ordre :

1. `command::resolve(&command::PRIME_SELECT)` ; absent ⇒ `NotSupported("prime")`.
2. `let sysfs = SysfsRoot::system(); prime::detect(&sysfs, TuxedoDevice::detect(&sysfs, None))` ;
   `!state.is_supported()` ⇒ `NotSupported("prime")` (outil non TUXEDO ou plateforme non
   compatible).
3. `state.mode == cible` ⇒ `true`, `noop=true`, aucun exec de bascule.
4. `command::run(program, &[argument], PRIME_SWITCH_TIMEOUT)`.
5. `prime::query_mode(Some(program))` ; si `Some(cible)` ⇒ `true`, sinon `WriteFailed`.

**Validation matérielle obligatoire** avant activation : `strace -f -e trace=file,execve` d'une
bascule réelle sur Clevo **et** Uniwill, pour figer :
- les chemins écrits (`/boot`, `/var/lib/ubuntu-drivers-common`, `/etc/…`, `/var/tmp`) ;
- les outils exécutés (`update-initramfs`, `update-grub`, `dpkg-query`, …) ;
- les familles d'adresses et syscalls réellement utilisées.

La liste `ReadWritePaths` de §4.5 est remplacée par le résultat mesuré, jamais élargie « au cas où ».

### 6.3 Exécution

- Chemin absolu issu d'une constante de compilation (`command::PRIME_SELECT`), jamais `PATH`.
- `Command::new(program).arg(arg)` — **jamais** de shell, jamais de concaténation.
- `env_clear()` puis `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, `LC_ALL=C`, `LANG=C`, `HOME=/root`
  (`command.rs:150-160`).
- `stdin=null`, stdout/stderr bornés à 64 Kio, lecteurs avec échéance (`command.rs:170-223`).
- Timeout 180 s : `prime-select` régénère l'initramfs ; en dessous, on tue un outil en cours
  d'écriture. Le timeout est **supérieur** au pire cas observé, pas égal.
- Aucun argument supplémentaire, aucune variable d'environnement utilisateur.

### 6.4 Vérification et état après reboot

- `prime-select query` est la source de vérité immédiate ; `PrimeState.mode` la reflète.
- La bascule **persiste** dans la configuration système ; l'effet complet (driver NVIDIA chargé,
  écran interne basculé) nécessite un redémarrage ou une reconnexion de session selon le mode.
- **Si la validation matérielle montre que `query` ne reflète le nouveau mode qu'après
  redémarrage**, ne pas convertir cela en échec : succès si le code de sortie est 0, audit
  `verify=deferred`, et l'UI affiche l'état effectif après redémarrage. Le comportement par défaut
  (vérification stricte) ne s'applique que si `query` reflète la configuration immédiatement.
- L'API ne peut pas vérifier l'état « après reboot » ; elle vérifie seulement la configuration
  écrite. L'UI doit afficher « redémarrage requis » et relire `prime` depuis l'instantané `tccd`
  après redémarrage.

### 6.5 Confirmation utilisateur

L'admin ne peut pas demander de confirmation interactive (pas de TTY, pas de `pkexec`). La
confirmation est **obligatoirement** dans l'UI avant l'appel :

- dialogue bloquant : mode cible, conséquence (« fermeture de session ou redémarrage requis »),
  avertissement « en cas d'échec, le système peut démarrer sans accélération graphique » ;
- l'utilisateur doit confirmer explicitement (pas de case pré-cochée, pas d'action par défaut
  destructive) ;
- le dialogue polkit s'affiche ensuite ; son message doit mentionner le redémarrage.

Si l'UI ne peut pas afficher cette confirmation (tray, CLI), l'appel est refusé côté client.

### 6.6 Échecs

| Cas | Code | Audit | Suite |
|---|---|---|---|
| outil absent / non TUXEDO | `NotSupported` | `result=not_supported` | aucune action |
| mode inconnu | `InvalidArgs` | `invalid_args` | aucune action |
| polkit refusé | `NotAuthorized` | `denied` | aucune action |
| commande non-zéro | `WriteFailed` | `write_failed` (stderr nettoyé, journal) | relire l'état, proposer de réessayer |
| timeout | `Timeout` | `timeout`, `killed=true` | relire l'état ; ne pas relancer automatiquement |
| relecture ≠ cible | `WriteFailed` | `verify=mismatch` | idem |
| relecture différée jusqu'au redémarrage (comportement validé) | `true` | `verify=deferred` | afficher « redémarrage requis » |
| déjà dans le mode | `true` | `noop=true` | — |

Aucun rollback. Aucune relance automatique.

---

## 7. PPD / TCC

### 7.1 Rappel D4

PPD reste propriétaire de `platform_profile` et de l'EPP. TCC possède les limites CPU, les cœurs, les
ventilateurs, l'affichage, la webcam, la charge — et, en phase 2, les limites TDP/cTGP et les profils
ODM du module `tuxedo_io` (que PPD ne connaît pas).

### 7.2 Détection

Voir §3.3. Points prescriptifs :

- `PpdOwned` = le nom `net.hadess.PowerProfiles` est **possédé** au moment de l'appel.
- `PpdActivatable` = installé mais endormi : autorisé, audité, revérifié après écriture.
- `Unknown` (bus en erreur) = **refus** (fail-closed).
- Le contrôle est refait **après** l'écriture pour détecter une course ; `ppd_race=true` dans
  l'audit, aucune restauration (PPD écrira sa propre valeur).

### 7.3 Comportement par méthode

| Méthode | PPD possédé | PPD activatable | PPD absent |
|---|---|---|---|
| `SetOdmProfile` (voie `platform_profile`) | `PolicyDenied("ppd.platform_profile")` | autorisé, audit `ppd=activatable` | autorisé |
| `SetOdmProfileOverride` (voie `platform_profile`) | autorisé **si** `Settings.ppd_override` | idem | autorisé |
| `SetOdmProfile` (voie `tuxedo_io`) | autorisé (pas de conflit) | autorisé | autorisé |
| `SetTdpValues` | autorisé (TCC possède le TDP) | autorisé | autorisé |
| `SetNvidiaCtgpOffset` | autorisé | autorisé | autorisé |
| `SetPrimeMode` | autorisé (hors `platform_profile`) | autorisé | autorisé |

**Lecture seule par défaut** : aucun setter phase 2 n'est appelé au démarrage, à l'installation ou
sur un simple changement d'état. L'UI affiche les valeurs lues (`GetHardwareSnapshotJSON`,
`GetTdpValues`) et n'active un setter qu'après une action explicite de l'utilisateur, suivie du
dialogue polkit. Un service Hw/Prime activé sans appel de méthode ne fait rien et s'arrête après
30 s.

### 7.4 Prérequis `Settings.ppd_override` — état 2026-11-06

**Implémentés** (modèle + migration, `SaveSettingsJSON`, propagation `tccd`
`main.rs:84-94,128-135,459-510`, UI, relecture disque par `Admin.Hw`) ; **reste** la validation
réelle de la persistance et de la réapplication. Détail :

1. `Settings.ppd_override: bool` (défaut `false`), `schema_version`, migration : **fait**.
2. `SaveSettingsJSON` valide le champ (booléen strict, pas d'`extra`) : **fait**.
3. `tccd` propage `Settings.ppd_override` aux `HardwareInputs`, relu à chaque rechargement :
   **fait** (`main.rs:84-94,128-135,459-510`, tests dédiés).
4. L'UI expose l'option avec un avertissement explicite : **fait** (`SettingsPage`).
5. `Admin.Hw` lit les settings **sur disque** à chaque `SetOdmProfileOverride` (pas de cache),
   via `ConfigStore::system()` : **fait** (`SystemOverrideConsent`).

Sans ce prérequis, `SetOdmProfileOverride` doit être **refusé** (`PolicyDenied`), pas contourné.

### 7.5 EPP

Aucune méthode phase 2 n'écrit l'EPP ; `SetOdmProfile` ne touche que `platform_profile` via
`PlatformProfile::apply`. Si un futur besoin apparaît, il repassera par une revue dédiée (D4).

---

## 8. Tests

### 8.1 Unitaires (`tcc-admin`, backends injectés)

Traits à introduire (dans `tcc-admin`, pas dans `tcc-hw`) :

```rust
trait TdpBackend { fn limits(&self) -> Result<Vec<TdpLimit>>; fn set_values(&self, &[f64]) -> Result<Vec<TdpLimit>>; }
trait OdmBackend { fn platform(&self) -> Result<Option<PlatformProfile>>; fn io_profiles(&self) -> Result<Vec<String>>; fn apply_platform(&self, &str) -> Result<()>; fn apply_io(&self, &str) -> Result<()>; }
trait CtgpBackend { fn offset(&self) -> Result<Option<i32>>; fn set_offset(&self, i32) -> Result<()>; fn max_offset(&self) -> Result<Option<i32>>; }
trait PrimeBackend { fn resolve(&self) -> Option<&'static str>; fn state(&self) -> PrimeState; fn query(&self) -> Option<PrimeMode>; fn switch(&self, &'static str) -> Result<()>; }
```

| Cas | Attendu |
|---|---|
| `SetTdpValues([])` | `true`, maxima firmware, audit `values=empty, maxima=true` ; `noop` si déjà aux maxima |
| `SetTdpValues([0])`, `[1001]` | `Validation("tdp.values")` |
| `SetTdpValues` 4 valeurs | `InvalidArgs("tdp.count")` |
| valeurs hors plage firmware | `Validation("tdp.values")`, **aucune** écriture |
| séquence non croissante | `Validation`, aucune écriture |
| cible == courant | `true`, `noop`, aucune écriture |
| échec au 2ᵉ ioctl, rollback OK | `WriteFailed`, `restore=ok`, valeurs précédentes |
| échec au 2ᵉ ioctl, rollback KO | `PartialApply("pl2")` |
| `GetTdpValues` sans device | `NotSupported("tdp")` |
| ODM nom inconnu / trop long / non ASCII | `Validation("odm.name")` |
| ODM voie `platform_profile`, PPD possédé | `PolicyDenied`, aucune écriture |
| ODM override sans flag | `PolicyDenied("ppd.override_disabled")` |
| ODM override avec flag | écriture, audit `override=true` |
| ODM nom présent dans les deux listes + PPD possédé | `PolicyDenied` (pas de bascule silencieuse vers ioctl) |
| ODM fallback ioctl + PPD possédé | autorisé |
| cTGP < 0 / > borne dynamique | `Validation` |
| cTGP borne dynamique indisponible | borne 200, audit `bound=static` |
| cTGP identique | `noop` |
| PRIME mode `off`/`unknown`/`INTEL` | `InvalidArgs` |
| PRIME outil absent | `NotSupported` |
| PRIME mode identique | `true`, aucun `switch` appelé |
| PRIME commande non-zéro | `WriteFailed`, sortie non relayée |
| PRIME timeout | `Timeout`, processus tué |
| PRIME post-query ≠ cible | `WriteFailed`, `verify=mismatch` |
| PRIME faux backend : `query` ne reflète le mode qu’après « reboot » simulé | `true`, `verify=deferred`, pas `WriteFailed` |
| polkit refusé | `NotAuthorized`, aucune lecture/écriture matérielle |
| deuxième appel concurrent | `Busy` |
| 5 appels en 1 s | `RateLimited` après le burst |

### 8.2 Intégration D-Bus

- Faux service `Admin.Hw`/`Admin.Prime` sur bus de session : signatures exactes, retour `b`, erreurs
  typées, introspection (méthodes et arguments), `Ping`/`GetContractVersion`.
- Vérifier qu'aucune méthode phase 2 n'est exposée par `Admin1` (introspection + appel ⇒
  `UnknownMethod`).
- Vérifier qu'un appelant non authentifié (faux authorizer) n'atteint jamais le backend (compteur
  d'appels du fake = 0).

### 8.3 Politique et unités

- `scripts/release/dbus-policy-test.sh` : nouveaux contrôles nommés —
  `Admin.Hw`/`Admin.Prime` joignables, membres listés, `Properties.*` refusés, phase 3 refusée,
  méthodes phase 2 toujours refusées sur `.Admin`, `Control1` root-only.
- `validate.sh` : `xmllint` sur les nouveaux fichiers `.conf`/`.policy`/`.service`, aucun `/opt`,
  permissions.
- `systemd-analyze verify` + `security` pour les deux unités ; scores consignés.
- Test de non-régression : la policy de la phase 1 ne change pas (diff = zéro ligne sur `.Admin`).

### 8.4 Fuzzing / propriétés

- `SetTdpValues` : longueurs 0..10 (0 = maxima explicite, 1..=3 = valeurs, > 3 = refus), valeurs
  `i32::MIN..=i32::MAX` (property test) — jamais de panique, toujours une erreur typée ou un refus
  avant écriture.
- Noms ODM : alphabet ASCII complet, longueurs 0..1024 (le rejet doit précéder toute découverte
  matérielle).
- `SetPrimeMode` : chaînes arbitraires (Unicode, NUL, 1 Mio) — rejet sans exec.

### 8.5 Validation matérielle (bloquante)

| Plateforme | TDP | ODM | cTGP | PRIME |
|---|---|---|---|---|
| Clevo (tuxedo_io) | `NotSupported` attendu (`tdp_count == 0`) | ioctl | selon GPU | si `nvidia-prime` TUXEDO |
| Uniwill | PL1/PL2/PL4 réels + rollback | `platform_profile` **et** ioctl | si dGPU | idem |
| AMD iGPU seul | `NotSupported` | ACPI `platform_profile` | `NotSupported` | `NotSupported` |
| Intel + NVIDIA | `NotSupported` | ACPI | oui | oui |
| PPD actif | autorisé | `PolicyDenied` (sauf override) | autorisé | autorisé |

Preuves : valeurs lues avant/après, `strace` des écritures, `journalctl` des lignes d'audit,
`systemd-analyze security`.

---

## 9. Checklist avant activation

**Bloquants**

1. **AD-12 appliqué** : `tccd-admin.service` inchangée (phase 1), deux nouvelles unités installées
   conditionnellement (`TCC_WITH_ADMIN_HW=1`, `TCC_WITH_ADMIN_PRIME=1`), binaires root:root 0755,
   fichiers d'activation D-Bus installés.
2. **Policy D-Bus** : membres phase 2 toujours refusés sur `.Admin`, autorisés **membre par membre**
   sur `.Hw`/`.Prime`, `Properties.*` absents, `dbus-policy-test.sh` exécuté en root et vert
   (42/42 + nouveaux cas).
3. **Polkit réel** : autorisé/refusé/annulé/absent, `auth_admin` sans cache (deux appels ⇒ deux
   dialogues), sujet `system-bus-name` vérifié.
4. **H-05 fermé** (bucket UID dans `tccd`) — sinon un utilisateur peut multiplier les connexions et
   les activations ; **H-07/H-08/H-09** traités (bornes charge/cTGP/TDP).
5. **Validation matérielle §8.5** sur Clevo, Uniwill, AMD, Intel, NVIDIA, PPD actif.
6. **PRIME** : `strace` de la bascule, `ReadWritePaths` figés, `ProtectSystem` tranché
   (`strict` ou `full` documenté), `KillMode=control-group` vérifié, redémarrage réel testé.
7. **`systemd-analyze security`** : phase 1 inchangée (1.6), Hw ≤ 3.0, Prime ≤ 4.5 ; scores
   consignés dans `packaging/SECURITY.md`.
8. **`Settings.ppd_override`** implémenté (modèle + migration + `tccd` + UI) ; `tccd` le propage
   et `Admin.Hw` le relit sur `/var/lib`. La persistance réelle et la réapplication restent à
   tester sur machine ; sinon `SetOdmProfileOverride` reste refusé.
9. **Docs** : `DBUS_API.md` (méthodes, transitoire, timeouts client, PRIME reboot),
   `HARDWARE_MATRIX.md`, `FEATURE_PARITY.md`, `packaging/SECURITY.md` (delta §4.4/§4.5).
10. **CI packaging** (H-14) : `validate.sh`, `systemd-analyze`, `xmllint`, `dbus-policy-test.sh`,
    `cargo deny`/`audit` ; aucun `/opt`, aucun world-writable.
11. **Installation/désinstallation** dans un conteneur propre : les deux services absents par
    défaut, activables sans casser la phase 1.

**Non bloquants mais à suivre**

- `StartLimitBurst` de la phase 1 (5/60 dans l'unité, 30/10 dans `ADMIN_DESIGN`) : aligner ou
  documenter.
- Ajout d'une méthode composite (un seul dialogue pour TDP+ODM+cTGP) : hors périmètre v1.
- Cooldown PRIME persistant : impossible sans état ; le dialogue polkit est la barrière.

---

## 10. Risques résiduels

| ID | Risque | Niveau | Décision |
|---|---|---|---|
| **P2-R1** | Un utilisateur authentifié peut dégrader le confort thermique (TDP bas) ou la stabilité (TDP haut) | Moyen | c'est la fonction ; plages firmware + audit + `auth_admin` sans cache |
| **P2-R2** | PRIME peut laisser le système sans accélération graphique / sans écran interne au prochain boot | Moyen | confirmation UI obligatoire, pas de rollback, documentation de secours |
| **P2-R3** | Course PPD : PPD démarre entre le contrôle et l'écriture | Faible | contrôle après écriture, audit `ppd_race`, PPD réécrit sa valeur |
| **P2-R4** | `ProtectKernelTunables=no` sur Hw : le processus root peut écrire d'autres fichiers `/sys` | Faible | `ReadOnlyPaths` étendus, code sans chemin, surface scalaire |
| **P2-R5** | `tccd` écrase une valeur transitoire au prochain changement de profil | Faible (attendu) | AD-7 documenté, UI « temporaire » |
| **P2-R6** | cTGP borné statiquement (200 W) si l'instantané `tccd` est indisponible | Faible | audit `bound=static` ; H-08 à fermer |
| **P2-R7** | Limiteur en mémoire réinitialisé à chaque activation (30 s) | Faible (dispo) | polkit sans cache, `StartLimitBurst`, UI |
| **P2-R8** | `prime-select` dépend de la distribution ; les chemins écrits ne sont pas bornés avant mesure | Moyen | `strace` obligatoire, `ReadWritePaths` mesurés, repli `ProtectSystem=full` documenté |
| **P2-R9** | TDP non transactionnel : état mixte après crash | Faible | relecture + rollback best-effort + documentation |
| **P2-R10** | Le service Hw ouvre `/dev/tuxedo_io` à chaque appel ; un driver bloqué gèle l'appel | Faible (dispo) | timeout D-Bus client 180 s, `RuntimeMaxSec`, pas de file |
| **P2-R11** | `GetTdpValues` est sans polkit : un utilisateur local peut activer le service root et lire le TDP en boucle | Faible (dispo/info) | rate limit 2/s, `StartLimitBurst=20/60`, sortie 30 s, aucune donnée sensible ; à surveiller si le service devient coûteux |

---

## 11. Intégration et propriété des fichiers

| Fichier / élément | Propriétaire | Remarque |
|---|---|---|
| `crates/tcc-ipc/src/admin.rs` (constantes, `AdminAction`, codes) | `rust-core` | pas de `zbus` |
| `crates/tcc-admin/src/hw/**`, `prime/**`, `error_hw.rs` | `linux-platform` | backends réels + traits |
| Backends `tcc-hw` (lecture/écriture TDP/ODM/cTGP) | `tuxedo-hardware` | aucune nouvelle API de commande ; `prime::set_mode` reste absent |
| `crates/tcc-admin/src/bin/tccd-admin-{hw,prime}.rs` | `linux-platform` | `--help`/`--version` uniquement |
| `packaging/systemd/tccd-admin-{hw,prime}.service` | `packaging-release` | revue `security` |
| `packaging/dbus/com.tuxedocomputers.tccd.{conf,Admin.Hw.service,Admin.Prime.service}` | `packaging-release` | membres un par un |
| `packaging/polkit/com.tuxedocomputers.tccd.policy` | `packaging-release` | textes `power-limits`/`prime-select` |
| `scripts/release/{install-tree.sh,dbus-policy-test.sh,validate.sh}` | `packaging-release` | installation conditionnelle + tests |
| `crates/tcc-core/src/settings.rs` (`ppd_override`) | `rust-core` | prérequis phase 2 |
| `crates/tcc-daemon/src/main.rs` (`with_ppd_override`) | `linux-platform` | lecture des settings |
| `docs/DBUS_API.md`, `docs/FEATURE_PARITY.md`, `HARDWARE_MATRIX.md` | `linux-platform` / `tuxedo-hardware` | mise à jour avant activation |

---

## Annexe A — Extraits de policy D-Bus

Dans `<policy user="root">` :

```xml
<allow own="com.tuxedocomputers.tccd.Admin.Hw"/>
<allow own="com.tuxedocomputers.tccd.Admin.Prime"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin.Hw"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin.Prime"/>
```

Dans `<policy context="default">` (après les deny existants) :

```xml
<deny own="com.tuxedocomputers.tccd.Admin.Hw"/>
<deny own="com.tuxedocomputers.tccd.Admin.Prime"/>
<deny send_destination="com.tuxedocomputers.tccd.Admin.Hw"/>
<deny send_destination="com.tuxedocomputers.tccd.Admin.Prime"/>

<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="org.freedesktop.DBus.Introspectable"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="org.freedesktop.DBus.Peer"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="Ping"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="GetVersion"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="GetContractVersion"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="GetTdpValues"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="SetTdpValues"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="SetOdmProfile"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="SetOdmProfileOverride"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Hw"
       send_interface="com.tuxedocomputers.tccd.AdminHw1" send_member="SetNvidiaCtgpOffset"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin.Hw"/>

<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="org.freedesktop.DBus.Introspectable"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="org.freedesktop.DBus.Peer"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="com.tuxedocomputers.tccd.AdminPrime1" send_member="Ping"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="com.tuxedocomputers.tccd.AdminPrime1" send_member="GetVersion"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="com.tuxedocomputers.tccd.AdminPrime1" send_member="GetContractVersion"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin.Prime"
       send_interface="com.tuxedocomputers.tccd.AdminPrime1" send_member="SetPrimeMode"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin.Prime"/>
```

Aucun `Properties.Get/GetAll/Set` sur les nouveaux objets (leçon A1-11).

## Annexe B — Récapitulatif des signatures

| Service | Méthode | Signature D-Bus | Polkit | Transitoire |
|---|---|---|---|---|
| `Admin.Hw` | `GetTdpValues` | `-> ai` | non | — |
| `Admin.Hw` | `SetTdpValues` | `ai -> b` | `power-limits` | oui |
| `Admin.Hw` | `GetContractVersion` | `-> u` | non | — |
| `Admin.Hw` | `SetOdmProfile` | `s -> b` | `power-limits` | oui |
| `Admin.Hw` | `SetOdmProfileOverride` | `s -> b` | `power-limits` | oui |
| `Admin.Hw` | `SetNvidiaCtgpOffset` | `i -> b` | `power-limits` | oui |
| `Admin.Prime` | `SetPrimeMode` | `s -> b` | `prime-select` | non (système) |

## Annexe C — Fichiers d'activation

`packaging/dbus/com.tuxedocomputers.tccd.Admin.Hw.service` :

```ini
[D-BUS Service]
Name=com.tuxedocomputers.tccd.Admin.Hw
Exec=/usr/libexec/tuxedo-rust-center/tccd-admin-hw
User=root
SystemdService=tccd-admin-hw.service
```

Idem pour `.Prime` avec `tccd-admin-prime`. Ces fichiers **ne sont pas installés par défaut** :
`install-tree.sh` les copie uniquement si `TCC_WITH_ADMIN_HW=1` / `TCC_WITH_ADMIN_PRIME=1`, et
seulement si les binaires correspondants existent. Une activation sans binaire doit être une erreur
d'installation, pas un échec silencieux au premier clic.

---

## Conclusion

La phase 2 est **implémentable sans affaiblir la phase 1** : AD-12 est tranché en faveur de services
séparés (B2), les quatre opérations sont réduites à des scalaires, polkit reste la barrière
principale, et chaque écriture est vérifiée ou explicitement déclarée non vérifiable. Les seuls
points bloquants sont des preuves à produire (polkit réel, policy root, validation matérielle,
`strace` PRIME, scores systemd) et deux prérequis de code hors périmètre `security`
(`Settings.ppd_override`, H-05/H-08/H-09). Aucune modification de ce document ne doit être faite
sans une revue `security` et une mise à jour de `docs/security/THREAT_MODEL.md` (delta phase 2).
