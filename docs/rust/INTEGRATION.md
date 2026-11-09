# Intégration `tcc-core` / `tcc-ipc` — guide pour `tcc-daemon`, `tcc-user` et `tcc-gui`

Ce document décrit les contrats stabilisés au jalon 1 et les conversions
attendues avec `tcc-hw` (livré). Il ne remplace pas les doc comments : chaque
type public est documenté dans le code.

## 1. ConfigStore : deux racines, jamais `/etc` en écriture (D2)

En production :

```rust
use tcc_core::ConfigStore;

// /var/lib/tuxedo-rust-center (écriture) + /etc/tuxedo-rust-center (lecture)
let store = ConfigStore::system();

let loaded = store.load_settings()?;
if let Some(warning) = loaded.warning() {
    // journaliser : kind, message, backup_path
}
store.save_settings(&loaded.value)?; // écrit uniquement dans /var/lib
```

Règles appliquées par `ConfigStore` :

- `load_*` cherche l'état dans `/var/lib`, puis dans `/etc` (valeurs
  packagées) si le fichier d'état est absent, puis les valeurs par défaut du
  crate.
- Un fichier d'état corrompu est renommé en `.bak` (`0600`) puis les valeurs
  par défaut sont utilisées. Si un `.bak` **régulier** existe déjà, il n'est
  **jamais** écrasé : le contenu corrompu va dans `.bak.corrupt` (`0600`, une
  seule génération) et la dernière version saine reste disponible. Un `.bak`
  symbolique n'est pas suivi (il est remplacé). Un fichier **admin** corrompu
  n'est jamais renommé : il est signalé et laissé intact.
- `save_*` écrit toujours dans la racine d'état. La racine admin est
  structurellement en lecture seule.
- Toute racine d'état sous `/etc`, relative ou contenant `..` est refusée
  avant toute I/O ; le chemin canonique du répertoire est aussi vérifié.
- Le propriétaire attendu (`Some(0)` via `ConfigStore::system()`) et l'absence
  d'écriture groupe/autres sont vérifiés par `fstat` sur le descripteur.
- Les fichiers sont écrits atomiquement (`create_new` + `fchmod 0644` +
  `fsync` + `rename` + `fsync` du répertoire).

### Lecture stricte sans récupération (A1-07)

```rust
// Validation croisée sans effet de bord disque.
let settings = store.load_settings_no_recovery()?;   // Err(NotFound) si absent
let profiles = store.load_profiles_no_recovery()?;
```

`load_settings_no_recovery`/`load_profiles_no_recovery` appliquent les mêmes
contrôles que `load_*` (`O_NOFOLLOW|O_CLOEXEC`, fichier régulier, propriétaire,
`mode & 0o022 == 0`, borne `max_file_bytes`, désérialisation + validation)
mais **ne récupèrent jamais** : aucun `.bak`, `.bak.corrupt` ni temporaire
n'est créé, renommé ou écrasé. Elles consultent l'état puis la racine admin
(lecture seule) et renvoient :

- `StrictLoadError::NotFound` si aucun des deux fichiers n'existe ;
- `StrictLoadError::TooLarge` au-delà de la borne ;
- `StrictLoadError::Io` pour une erreur d'E/S ou un refus de sécurité
  (symlink, type, propriétaire, permissions) ;
- `StrictLoadError::Invalid` pour un contenu non UTF-8, un JSON invalide ou
  une validation en échec.

Un fichier d'état corrompu **n'est pas** masqué par un repli vers `/etc` :
l'erreur est prioritaire. C'est l'API à utiliser pour vérifier la cohérence
avant une écriture administrateur, ou pour refuser explicitement un fichier
corrompu. `tcc-admin` pourra remplacer sa validation locale par cette API
dans une étape ultérieure (aucun changement de son code ici).

Injection de racines (tests, packaging, overrides) :

```rust
let store = ConfigStore::with_admin_roots(
    "/tmp/tcc-state",
    "/tmp/tcc-admin",
);
```

`TCC_CONFIG_DIR` n'est lu que par `tcc_core::config::dev_state_root()` quand la
feature `dev-overrides` est compilée. La fonction refuse un chemin relatif,
`/etc`, et un uid effectif 0 (D7). L'uid effectif est lu dans
`/proc/self/status` (`Uid:`), comme dans `tcc-hw`, pour éviter tout `unsafe`.

Pour `tcc-hw`, la variable `TCC_SYSFS_ROOT` est gérée par
`SysfsRoot::from_env_or_system()` et sa propre feature `dev-overrides`. En
release, les deux features sont absentes : `tccd` lit `/sys` et écrit dans
`/var/lib/tuxedo-rust-center` uniquement. Les deux crates doivent être
compilées avec la même politique d'override ; ne jamais activer
`dev-overrides` dans un paquet ni dans une unit systemd.

### Ordre de migration recommandé

1. `read_legacy_config(&LegacyPaths::default_system())` lit `/etc/tcc/*`
   (taille bornée à 1 Mio, aucune écriture).
2. Journaliser les `notes` et avertir si un fichier est absent.
3. Écrire les valeurs migrées **dans `/var/lib`** via `ConfigStore::system()`.
4. Ne jamais écrire dans `/etc/tcc` ; laisser l'ancien répertoire intact pour
   permettre un retour arrière.
5. Valider les références avec `validate_settings_references(&settings, &profiles)`.
   Un profil manquant ne doit pas empêcher le démarrage : le résolveur retombe
   sur le profil par défaut et l'UI signale l'incohérence.

## 2. Conversions `tcc-hw` → `tcc-core`

`tcc-core` n'importe pas `tcc-hw` (frontière de dépendance). Les conversions
sont explicites, dans le daemon.

### Alimentation

`tcc_hw::power_supply::PowerState` et `tcc_core::PowerSource` ont les mêmes
variantes. Mapping direct :

| `tcc-hw` | `tcc-core` |
|---|---|
| `PowerState::Ac` | `PowerSource::Ac` |
| `PowerState::Battery` | `PowerSource::Battery` |
| `PowerState::Unknown` | `PowerSource::Unknown` |

Si le daemon lit un booléen, utiliser les helpers testés de `tcc-core` :

```rust
use tcc_core::PowerSource;

let power = PowerSource::from_on_battery(upower_on_battery); // UPower
let power = PowerSource::from_ac_online(mains_online);       // sysfs `online`
```

`None` donne `Unknown`, jamais AC par défaut. C'est une différence assumée
avec l'original : appliquer un profil secteur sur batterie est plus dangereux
qu'utiliser le profil par défaut.

### CPU

Les fréquences de `tcc_hw::cpu::CpuPolicy` (`scaling_min_khz`,
`scaling_max_khz`, `available_frequencies_khz`) et de
`tcc_core::CpuProfile` (`scaling_min_frequency`,
`scaling_max_frequency`, en kHz via `FrequencyLimit::Hz`) sont **dans la même
unité**. Aucune conversion MHz/kHz.

- `CpuPolicy.online` → `CpuProfile.online_cores` est une décision matérielle :
  le politique de haut niveau ne fixe qu'un nombre de cœurs en ligne.
- `CpuPolicy.available_governors` / `available_epps` servent à vérifier qu'une
  valeur de profil est réellement supportée avant écriture. `tcc-core` ne
  vérifie que la syntaxe et la longueur.
- `CpuProfile.scaling_max_frequency = Some(FrequencyLimit::Reduced)` signifie
  « fréquence réduite décidée par `tcc-hw` », pas une valeur en kHz.

### Affichage

| `tcc-hw` | `tcc-ipc` |
|---|---|
| `DisplayBackendKind::{X11,Hyprland,Gnome,Kde,Sway,Wlroots}` | `DisplayBackend` mêmes variantes |
| `DisplayBackendKind::Unsupported` | `DisplayBackend::Unsupported` |
| `DisplayMode { width, height, refresh_hz }` | `DisplayResolution::new(width, height)` + `refresh_rate_hz` |
| nom d'output interne | `DisplayModeInfo.internal_output` |

`tcc_ipc::is_internal_output_name` applique le même filtre que `tcc-hw`
(`eDP-*`, `LVDS-*`, `DSI-*`). Le daemon et l'agent doivent tous deux valider
`DisplayModeRequest` : le daemon pour ne pas émettre une demande externe,
l'agent pour ne pas faire confiance au bus système. Ne jamais transmettre un
nom d'output fourni par l'utilisateur sans l'avoir comparé à la liste
découverte par le compositeur.

`DisplayModeInfo::unsupported(Some("raison"))` est un état normal à exposer
dans l'UI, pas une erreur fatale.

## 3. Moteur de bascule

Avant de construire le résolveur, fusionner les profils chargés avec les
presets de référence :

```rust
use tcc_core::{default_profiles, merge_with_default_profiles};

let loaded = store.load_profiles()?.into_value();
let profiles = merge_with_default_profiles(loaded);
profiles.validate()?; // les cinq IDs TCC sont garantis présents
```

`merge_with_default_profiles` conserve le premier profil rencontré pour un
identifiant donné : un profil personnalisé qui réutilise `__office__` n'est
pas écrasé, et les presets manquants sont ajoutés. Les doublons de l'entrée
sont éliminés.

**Unicité des noms (BUG-QA-002)** : l'identifiant est la clé d'unicité
fonctionnelle (références `stateMap`/`stateMatrix`), mais
`ProfileCollection::validate` exige aussi des noms uniques (casse et espaces
de bord ignorés) et la GUI rejette un payload invalide. La fusion ne casse
jamais un preset : elle **renomme le profil chargé** en conflit avec un suffixe
explicite (`Office` → `Office (custom)`, puis `Office (custom 2)`) et conserve
son identifiant. Le résultat est idempotent.

Conséquences pour `tcc-daemon` :

- valider la collection **après** fusion (`profiles.validate()`), pas seulement
  les profils chargés ;
- journaliser les renommages pour que l'UI puisse expliquer le changement de
  nom ; aucun profil n'est supprimé ;
- ne jamais réécrire l'identifiant pour « corriger » un doublon : les
  références `stateMap`/`stateMatrix` seraient cassées.

```rust
use std::time::Duration;
use tcc_core::{StateInput, StateResolver, StateMatrix, LegacyStateMap, PowerSource,
               PpdProfile, ProfileId, ObserveOutcome};

let mut resolver = StateResolver::new(
    StateMatrix::suggested_default(), // résolvable avec default_profiles()
    LegacyStateMap::default(),        // stateMap migré
    ProfileId::new("__default_custom_profile__"),
)
.with_debounce(Duration::from_millis(750));

resolver.set_known_profiles(Some(profiles.id_set()));
resolver.set_matrix(settings.state_matrix.clone());
resolver.set_legacy_state_map(settings.state_map.clone());

let outcome = resolver.observe(
    StateInput::new(Some(PpdProfile::from_dbus("balanced")), PowerSource::Ac),
    monotonic_now(),
);

match outcome {
    ObserveOutcome::Changed { current, .. } => apply_profile(&current.profile_id),
    ObserveOutcome::Unchanged(current) => { /* rien à faire */ }
    ObserveOutcome::Pending { .. } => { /* attendre le prochain tick */ }
}
```

Points d'intégration :

- `monotonic_now()` doit être monotone (par exemple `Instant::elapsed` depuis
  le démarrage). Le moteur ne lit aucune horloge.
- Le premier `observe` est appliqué immédiatement (pas de délai au boot).
- Un changement de PPD **ou** d'alimentation redémarre le debounce. Un retour
  à la clé courante pendant la fenêtre annule la transition.
- L'override manuel (`set_manual_override`) est invalidé au moment où la
  nouvelle clé est **confirmée**, pas pendant le debounce : un flap transitoire
  de PPD ne doit pas effacer le choix de l'utilisateur.
- Après un rechargement de config ou `SIGHUP`, appeler `force_reapply()` puis
  `observe` avec le même état pour réappliquer les réglages matériels.
- `set_known_profiles(Some(ids))` évite d'appliquer un profil supprimé : la
  résolution retombe sur la matrice/le `stateMap`/le défaut. Sans ensemble,
  aucun filtrage n'est fait (tests, CLI).
- `Settings::default().state_matrix` est vide ; utiliser
  `StateMatrix::suggested_default()` ou `Settings::with_suggested_matrix()`.
  Les ID référencés (`__profile_max_energy_save__`, `__profile_silent__`,
  `__office__`, `__high_performance__`) sont fournis par `default_profiles()`.
- PPD reste propriétaire de `platform_profile` et de l'EPP (D4) ; le moteur ne
  produit qu'un identifiant de profil.

### Prérequis phase 2 : `Settings.ppd_override`

- Champ `ppdOverride` (camelCase, booléen JSON strict, défaut `false`), sans
  alias snake_case. `false` : PPD reste propriétaire de `platform_profile` et
  de l'EPP. `true` : consentement persistant permettant **uniquement** à
  `Admin.Hw.SetOdmProfileOverride` (polkit systématique) d'écrire
  `platform_profile`. Le booléen ne déclenche aucune écriture à lui seul.
- Migration `/etc/tcc/settings` : champ absent ⇒ `false` ; les champs inconnus
  restent dans `Settings::extra` ; une valeur non booléenne (`"yes"`, `1`, …)
  fait échouer la désérialisation et donc la migration (aucune coercition).
- Non branché dans `tccd` à cette étape (`with_ppd_override(false)` en dur) :
  le câblage et l'UI appartiennent à `linux-platform` / `qt-ui` (voir
  `docs/security/ADMIN_PHASE2_DESIGN.md` §7.4).
- `SETTINGS_SCHEMA_VERSION` reste `1` : le champ est optionnel avec défaut,
  donc rétrocompatible. Un incrément exigerait une migration au chargement de
  `/var/lib` (les fichiers `schemaVersion: 1` existants ne doivent pas être
  mis en quarantaine) ; il est laissé à l'étape qui branche réellement la
  phase 2.

### Tombstones d'IDs de profils

- `Settings.deletedProfileIds` (`Vec<ProfileId>`, camelCase, vide par défaut)
  mémorise les identifiants de profils supprimés qui ne doivent **jamais**
  être réutilisés. L'ordre d'insertion est conservé et sérialisé tel quel ;
  les doublons, IDs invalides et IDs gérés par TCC (`__office__`, presets
  legacy, `__default_custom_profile__`) sont refusés par
  `Settings::validate`.
- Bornes : `MAX_DELETED_PROFILE_IDS` = 1024 entrées ; un fichier au-delà est
  rejeté, jamais tronqué.
- API :
  - `Settings::is_profile_id_deleted(&ProfileId) -> bool` ;
  - `Settings::mark_profile_deleted(&mut self, &ProfileId) -> Result<(), ValidationError>`
    (idempotent, ajoute en fin de liste, refuse preset/ID invalide/borne).
- **Pas d'`unmark`** : un tombstone est permanent. Réutiliser un ID est une
  réparation root hors API (`/var/lib`), jamais une opération exposée par
  `Admin1`.
- Migration `/etc/tcc/settings` : champ absent ⇒ liste vide ; liste existante
  conservée ; champs inconnus préservés ; type invalide, ID invalide, doublon
  ou preset ⇒ échec de migration (aucune coercition). `schemaVersion` reste
  `1` (champ additif avec défaut).
- **Source de la collection précédente** : utiliser
  `ConfigStore::load_profiles_state_only_no_recovery()`, qui lit **uniquement**
  `/var/lib/tuxedo-rust-center/profiles.json` avec les garanties strictes
  (`O_NOFOLLOW|O_CLOEXEC`, uid/mode, borne, validation) et sans aucun effet de
  bord. Fichier absent ⇒ `StrictLoadError::NotFound`, à traiter comme « aucune
  collection précédente » (première écriture) ; fichier corrompu ⇒ `Invalid`
  (refuser la requête, aucun tombstone). Ne **jamais** utiliser
  `load_profiles_no_recovery()` pour ce diff : son repli `/etc` ferait
  tombstoner des profils packagés qu'un client n'a jamais vus (AD-R13). Aucun
  indicateur de source séparé n'est nécessaire : la lecture state-only rend le
  repli structurellement impossible et évite une double lecture sujette à
  course.
- **Intégration `SaveProfilesJSON`** (pour `tcc-admin`, sans modification ici) :
  comparer la collection d'état lue ci-dessus et la collection reçue. Si un ID
  **nouvellement introduit** est tombstoné, renvoyer
  `Admin1.Error.Validation("deletedProfileIds")` — `InUse` est réservé aux
  références d'état qui disparaîtraient, et le message `Validation` n'a pas à
  renvoyer l'ID. Un ID tombstoné déjà présent dans l'ancienne collection
  (état legacy transitoire) n'est pas une réutilisation. Lorsqu'un profil
  disparaît, l'admin doit appeler `mark_profile_deleted` et persister les
  settings dans la même transaction, sinon l'ID redevient réutilisable. Un
  profil issu de `/etc` n'est jamais tombstoné : il ne fait partie ni de la
  collection d'état précédente ni des suppressions à enregistrer.
- Tout client qui renvoie des settings (`Admin1.SaveSettingsJSON`, GUI, CLI)
  doit conserver `deletedProfileIds` tel que lu : ne jamais reconstruire un
  `Settings` par défaut pour une sauvegarde, sous peine d'effacer les
  tombstones et de rendre les IDs à nouveau réutilisables.

## 4. Contrat IPC

- `StateSnapshot` : `contractVersion`, `revision` monotone, `ppdProfile`,
  `powerSource`, `manualOverride`, `activeProfile`.
- `ActiveProfile::from_profile(&Profile, source)` évite de dupliquer la
  logique preset/description dans le daemon.
- `DisplayModeInfo` : état complet backend/output/modes/erreur.
- `DisplayModeRequest` : profil + taux et/ou résolution + output interne
  optionnel. `validate()` exige au moins une cible et refuse une sortie
  externe.
- `SessionAgentRequest`/`SessionAgentResponse` : actions
  `GetDisplayModeInfo`, `ApplyDisplayMode`, `ResetDisplayMode`, `Shutdown`.
  `ApplyDisplayMode` exige un `displayMode` valide ; les autres actions
  l'interdisent. `Failed` exige un message, `Info` exige un état d'affichage.
- Toujours appeler `validate()` avant émission et après réception. Un
  `contractVersion` différent est refusé (`ensure_contract_compatible`), pas
  ignoré.
- `DisplayModeInfo`/`DisplayModeRequest` peuvent être envoyés via D-Bus
  typé ou via un encodage `serde_json` borné pour la compatibilité ; ne jamais
  exposer un `serde_json::Value` libre dans une signature D-Bus.

## 5. Validation et erreurs

- `validate_profile`, `validate_settings`, `validate_profiles`,
  `validate_fan_tables`, `validate_settings_references` renvoient une erreur
  agrégée (`ValidationError::issues`) : l'UI peut surligner tous les champs
  fautifs.
- Bornes de sécurité (extrait) : id `^[A-Za-z0-9_-]{1,64}$`, refresh
  `1..=1000 Hz`, résolution `1..=16384`, CPU `1..=1024` cœurs, fréquences
  `100 MHz..=10 GHz` (kHz), TDP `1..=1000 W` croissants, cTGP `0..=200 W`,
  ventilateurs `0..=100 %`, offset `-100..=100`, températures `-40..=200 °C`.
- Une valeur hors plage est **rejetée**, jamais clampée. `tcc-hw` affine avec
  les capacités réelles et renvoie une erreur typée.
- `tcc-hw::HwError::NotSupported` est un état normal : l'UI affiche
  « indisponible », le daemon continue.

### Contrôles matériels persistants : `chargingType`, `fnLockStatus`

- `Settings.chargingType: Option<String>` (`chargingType`, absent ou `null`
  ⇒ `None`). Type de charge demandé. La liste est **ouverte** : `tcc-core` ne
  connaît pas les valeurs du firmware. Validation de forme uniquement :
  non vide, ≤ 64 caractères, `[A-Za-z0-9._:-]`. Aucune capacité matérielle
  n'est exigée par `Settings::validate`.
- `Settings.fnLockStatus: Option<bool>` (`fnLockStatus`, absent ou `null` ⇒
  `None`). `None` = ne pas toucher ; `Some(bool)` = état demandé appliqué au
  chargement et à chaque rechargement. Booléen JSON strict.
- **Le daemon reste l'autorité** : il confronte `chargingType` aux types
  réellement exposés par `tcc-hw` avant toute écriture. Si le type, le
  contrôle Fn-Lock ou le matériel est absent/non supporté, il journalise et ne
  fait **rien** (no-op) ; aucune écriture arbitraire, aucun fallback silencieux.
  La GUI peut exposer l'édition de ces champs, mais doit afficher l'état
  matériel réel et l'absence de support.
- Migration `/etc/tcc/settings` : champs absents ⇒ `None` ; type invalide,
  chaîne Fn-Lock invalide, type de charge vide/trop long/caractères hors
  charset ⇒ migration refusée (aucune coercition). `None` est omis à la
  sérialisation, ce qui équivaut à « absent » à la lecture.

## 6. Performance

- `tcc-core` est synchrone et sans allocation par tick en dehors des
  conversions ; `StateResolver` ne clone que sur changement effectif.
- `FanCurve::speed_at` est O(n) sur ≤ 256 points, sans tri à chaque appel.
  Préférer `FanCurve::normalize()` une fois au chargement.
- `ConfigStore` lit/écrit rarement ; ne pas l'appeler dans une boucle de
  capteurs.
- Les budgets (daemon < 1 % CPU / < 30 Mo RSS, GUI < 200 Mo / < 1,5 s) sont
  mesurés avec `criterion`/`perf`, pas estimés.

## 7. Fondation `tccd-admin` (AD-1…AD-12)

Cette section décrit ce qui existe réellement dans le workspace. Le design de
sécurité reste `docs/security/ADMIN_DESIGN.md` ; ce document ne fait que
préciser l'API livrée.

### 7.1 `tcc-ipc` — contrat `Admin1`

Constantes exportées à la racine du crate :

| Constante | Valeur |
|---|---|
| `DBUS_ADMIN_SERVICE` | `com.tuxedocomputers.tccd.Admin` |
| `DBUS_ADMIN_PATH` | `/com/tuxedocomputers/tccd/Admin` |
| `DBUS_ADMIN_INTERFACE` | `com.tuxedocomputers.tccd.Admin1` |
| `DBUS_CONTROL_INTERFACE` | `com.tuxedocomputers.tccd.Control1` |
| `ADMIN_CONTRACT_VERSION` | `1` |
| `ADMIN_ERROR_NAMESPACE` | `org.tuxedocomputers.tccd.Admin1.Error` |

Types publics :

- `AdminReloadReport { ok, warnings }` : réponse de
  `tccd.Control1.ReloadConfig`. `ok == true` implique `warnings` vide
  (`validate()` le vérifie). `from_json_bounded(json, max_bytes)` borne la
  taille **avant** le parsing ; `to_json()` valide avant sérialisation.
  Les avertissements sont limités à 64 entrées de 512 caractères, sans
  caractère de contrôle.
- `AdminVersionInfo { contract_version, implementation_version }` :
  `for_current("0.1.0")`, `validate()` refuse une version de contrat
  différente et une chaîne non ASCII graphique.
- `AdminErrorCode` : les 10 codes de `ADMIN_DESIGN.md` §2.7 ;
  `dbus_name()` renvoie toujours le namespace applicatif, jamais
  `org.freedesktop.DBus.Error.AccessDenied`.
- `AdminAction` : libellés d'audit stables (`save-profiles`, …) et nom de
  méthode (`SaveProfilesJSON`, …) pour les phases 1 et 2.
- `ensure_admin_contract_compatible(remote)` : comparaison stricte, erreur
  `IpcError::AdminContractMismatch`.

`tcc-ipc` ne dépend toujours pas de `zbus` : les proxies et l'interface
`Admin1` seront écrits dans `tcc-admin` par `linux-platform`.

### 7.2 `tcc-core` — persistance avec sauvegarde

```rust
use tcc_core::ConfigStore;

// Production : écritures limitées à /var/lib/tuxedo-rust-center.
let store = ConfigStore::system();

let saved = store.save_profiles_with_backup(&profiles)?;
if let Some(backup) = saved.backup {
    // chemin du .bak 0600 créé avant remplacement
}
if let Some(kept) = saved.preserved_backup {
    // .bak existant conservé : le contenu remplacé n'était pas exploitable
}
```

Sémantique garantie :

- validation du modèle avant toute I/O ;
- lecture de l'ancien contenu avec `O_NOFOLLOW`, taille bornée par
  `LoadPolicy::max_file_bytes` (1 Mio) ;
- **le `.bak` n'est remplacé que par un contenu exploitable** : l'ancien
  contenu est désérialisé et validé avec les mêmes règles que `load_*`. S'il
  est corrompu, d'un autre type ou refusé par la validation, le `.bak`
  existant est conservé intact ; `Saved::backup` reste `None` et
  `Saved::preserved_backup` porte son chemin (A1-07). La cible est quand
  même remplacée par la nouvelle valeur, déjà validée ;
- `.bak` écrit atomiquement en `0600`, puis cible en `0644`
  (`create_new` + `fchmod` + `fsync` + `rename` + `fsync` du répertoire) ;
- contenu identique ⇒ `Saved { backup: None, preserved_backup: None }`,
  aucune écriture (idempotence) ;
- aucun `create_dir_all` : un répertoire d'état absent est une erreur
  (`StateDirectory` systemd le crée) ;
- lien symbolique ou fichier non régulier sur la cible ⇒ erreur, ancien
  fichier intact ; sur le `.bak`, dès qu'il existe (remplacé ou conservé)
  ⇒ erreur, aucune écriture ;
- racine d'écriture explicite : `ConfigStore::system()` la fixe à
  `/var/lib/tuxedo-rust-center`. Un magasin construit par
  `ConfigStore::from_root()` refuse les sauvegardes tant que
  `with_write_root(root)` n'a pas été appelé (tests, outils) ;
- toute racine résolue sous `/etc`, et tout chemin hors de la racine
  d'écriture, est refusé avant I/O ;
- une politique `expected_owner = Some(0)` (celle de `system()`) exige un uid
  effectif 0 : sans `chown`, le crate ne peut pas garantir un `.bak`
  root:root depuis un processus non root.

Coût : un fichier précédent valide est désérialisé et validé une seconde fois
(≤ 1 Mio). `tccd-admin` est un processus court ; le coût est négligeable
devant l'écriture et le rechargement. Une sauvegarde conservée peut être plus
ancienne que la version remplacée : c'est le prix de la garantie « la dernière
version saine n'est jamais écrasée ».

`save_settings_with_backup` et `save_profiles_with_backup` ne vérifient pas
les références croisées : `tccd-admin` doit appeler
`validate_settings_references(&settings, &profiles)` avant la transaction,
puis `tccd.Control1.ReloadConfig` après l'écriture, et restaurer l'ancien
contenu si `AdminReloadReport::ok` est faux (AD-6).

### 7.3 `tcc-authz` — squelette

API publique : `AuthorizationRequest`, `AuthorizationResult`, `AuthzError`,
`check_authorization(&request).await`.

`check_authorization` valide la requête puis renvoie
`AuthzError::NotImplemented`. Contrat à respecter par l'implémentation :

- sujet `system-bus-name` du **nom unique du client** (jamais
  `unix-process`, jamais l'admin lui-même) ;
- `CheckAuthorization` appelé même pour uid 0, sans cache local ;
- autorisation seulement si `is_authorized && !is_challenge` ;
- absence/erreur/timeout de polkit ⇒ refus (`Denied` ou erreur), jamais de
  fallback root ;
- `caller_uid == None` ⇒ refus avant polkit.

Les constantes d'action polkit (`com.tuxedocomputers.tccd1.profiles`, …) et
le bucket anti-dialog-storm restent à implémenter dans `tcc-authz` /
`tcc-admin` (AD-9, §3.2 et §3.4 d'`ADMIN_DESIGN.md`). Aucune dépendance
`zbus`/`tokio` n'est imposée par le squelette.

### 7.4 `tcc-admin` — binaire

`crates/tcc-admin` produit `tccd-admin`. À ce stade :

- `--help` et `--version` affichent l'identité et la version de contrat ;
- aucun argument ⇒ code 78 (`EX_CONFIG`) avec un message explicite : le
  service n'est pas implémenté ;
- argument inconnu ⇒ code 2 ;
- **aucun** service D-Bus, aucun appel polkit, aucune lecture/écriture de
  fichier, aucun accès matériel, aucun `--force`.

`linux-platform` ajoutera `zbus`, `tokio`, `tcc-authz` et `tracing` au
`Cargo.toml` avec l'implémentation d'`Admin1`. Tant que ce n'est pas fait,
`packaging/systemd/tccd-admin.service` ne doit pas être installée
(`docs/security/ADMIN_DESIGN.md` §9).

