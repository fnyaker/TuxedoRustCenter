# Conventions Rust — TuxedoRustCenter

Ce document fixe les règles de code du workspace. Il complète `AGENTS.md`,
`docs/ARCHITECTURE.md` et `docs/security/DECISIONS.md`.

## 1. Frontières et dépendances

```
tcc-core  ←  tcc-ipc  ←  tcc-daemon  ←  tcc-gui
   ↑                         ↑
   └──────── tcc-hw ────────┘
                             ↑
                         tcc-user
```

- `tcc-core` : aucune dépendance matérielle, D-Bus, Qt, tokio ou tracing.
  Uniquement `serde`, `serde_json`, `thiserror`, `uuid`, `libc` (Unix).
- `tcc-ipc` : dépend de `tcc-core` pour les enums de domaine, jamais de
  `tcc-hw`/Qt/runtime.
- `tcc-hw` : ne dépend pas de `tcc-core`. Les conversions sont faites par
  `tcc-daemon` (voir `docs/rust/INTEGRATION.md`).
- Toute nouvelle dépendance doit être justifiée en revue, compatible
  GPL-3.0-or-later, et ajoutée à `deny.toml` si sa licence ne l'est pas déjà.
  Pas de runtime Node/Python, pas de crate Git non épinglée.

## 2. Erreurs

- Erreurs typées avec `thiserror` aux frontières de bibliothèque.
- `anyhow` autorisé uniquement dans les binaires, jamais dans `tcc-core` ou
  `tcc-ipc`.
- **Aucun `unwrap()`/`expect()` en production.** Les tests peuvent en utiliser.
  Les crates activent `deny(clippy::unwrap_used, clippy::expect_used)` hors
  tests.
- Ne pas masquer une erreur matérielle ou de configuration derrière un
  fallback silencieux : renvoyer une erreur ou un avertissement typé
  (`LoadWarning`, `ObserveOutcome::Pending`, `HwError::NotSupported`), et
  laisser le binaire journaliser.
- Ne pas paniquer sur une entrée JSON/D-Bus invalide. Une valeur non finie
  (`NaN`, infini) est toujours rejetée par la validation.

## 3. `unsafe`

`#![forbid(unsafe_code)]` dans `tcc-core` et `tcc-ipc`. `unsafe` n'est admis
que dans un wrapper isolé de `tcc-hw` (ioctl), avec commentaire de sûreté,
validation préalable et test. Aucun `unsafe` dans la GUI ou la logique métier.

## 4. Sérialisation et compatibilité

- Les modèles persistants utilisent les clés **camelCase** de l'original
  (`useBrightness`, `refreshRate`, `scalingMinFrequency`, `cTGPOffset`, …).
- `schemaVersion` est obligatoire dans `settings.json` ; une version future est
  refusée, pas interprétée.
- Les champs inconnus sont conservés (`serde(flatten)` dans `Profile` et
  `Settings`) pour ne pas perdre de données lors d'un aller-retour.
- `refreshRate: -1` devient `None` ; à l'écriture, `None` redevient `-1`.
- `scalingMaxFrequency: -1` devient `FrequencyLimit::Reduced`.
- Ne jamais ajouter d'alias snake_case à un champ déjà camelCase : le champ
  serait dupliqué dans `extra` et réécrit deux fois.
- `settings.ppdOverride` : booléen strict, défaut `false`. Il exprime le
  consentement à l'override PPD de la phase 2 (`Admin.Hw`), ne déclenche
  aucune écriture seul, et n'est pas lu par `tccd` tant que la phase 2 n'est
  pas câblée. Absent à la migration legacy ⇒ `false` ; type invalide ⇒ refus.
- `settings.chargingType` : type de charge demandé, liste ouverte, ≤ 64
  caractères `[A-Za-z0-9._:-]`, absent/`None` = ne pas toucher. Le daemon
  confronte la valeur aux capacités réelles avant écriture.
- `settings.fnLockStatus` : `Option<bool>` strict, absent/`None` = ne pas
  toucher ; `Some(bool)` est appliqué au chargement/reload. Matériel absent ou
  en lecture seule ⇒ no-op journalisé, jamais d'écriture arbitraire.
- `settings.deletedProfileIds` : tombstones d'IDs de profils, ordre
  d'insertion conservé, ≤ 1024, sans doublon ni preset/ID géré par TCC.
  `mark_profile_deleted` est idempotent ; aucun `unmark` n'est exposé. Un ID
  supprimé ne doit jamais être réintroduit par `SaveProfilesJSON`. Le diff
  « précédent » de l'admin utilise
  `ConfigStore::load_profiles_state_only_no_recovery()` (jamais le repli
  `/etc` de `load_profiles_no_recovery`) : un profil `/etc` n'est jamais
  tombstoné.
- Un changement de type partagé (`tcc-ipc`) impose un incrément de
  `IPC_CONTRACT_VERSION`.
- L'**identifiant** de profil est la clé d'unicité et de référence. Les noms
  doivent aussi être uniques (validation + GUI) : `merge_with_default_profiles`
  renomme le profil chargé en conflit (`Office` → `Office (custom)`) sans
  changer son identifiant, et ne renomme jamais un preset.

## 5. Configuration et sécurité (D1–D7)

- État mutable : `/var/lib/tuxedo-rust-center/`, root:root, fichiers `0644`.
- Config admin : `/etc/tuxedo-rust-center/`, **lecture seule** pour le daemon.
  `ConfigStore` refuse structurellement toute écriture sous `/etc`, y compris
  via un chemin injecté ou un lien symbolique.
- `tccd-admin` écrit exclusivement via `save_settings_with_backup` /
  `save_profiles_with_backup` sur un `ConfigStore::system()` : racine
  d'écriture `/var/lib`, `.bak` `0600`, aucun `create_dir_all`, aucun chemin
  fourni par un client (AD-5).
- Le `.bak` n'est **jamais** remplacé par un contenu non exploitable : il
  faut le même type et la même validation que `load_*`. Sinon la sauvegarde
  existante est conservée (`Saved::preserved_backup`) et la cible reçoit
  quand même la nouvelle valeur validée. Invariant : « la dernière version
  saine n'est jamais écrasée ».
- Sauvegardes de fichiers corrompus : `.bak` root-only `0600`, et
  `.bak.corrupt` (`0600`, une génération) lorsque `.bak` est déjà occupé —
  la récupération ne détruit jamais une sauvegarde saine.
- Lecture stricte : `load_settings_no_recovery`/`load_profiles_no_recovery`
  ne créent ni `.bak` ni temporaire et renvoient `StrictLoadError`
  (`NotFound`/`TooLarge`/`Io`/`Invalid`). Les utiliser pour toute validation
  croisée qui ne doit pas modifier le disque (A1-07).
- Lecture : `O_NOFOLLOW`, vérification `fstat` (fichier régulier, propriétaire,
  mode sans écriture groupe/autres), taille plafonnée à 1 Mio.
- Écriture : temporaire dans le même répertoire (`create_new`), `fchmod 0644`,
  `fsync`, `rename`, `fsync` du répertoire.
- Overrides de développement (`TCC_CONFIG_DIR`) : feature `dev-overrides`
  désactivée par défaut, absente des paquets, refusée si l'uid effectif est 0.
- Ne jamais construire un chemin à partir d'un champ utilisateur. Les noms de
  sortie d'affichage sont validés contre la liste interne (`eDP-*`, `LVDS-*`,
  `DSI-*`) et, côté matériel, contre la liste découverte au runtime.

## 6. Async et workers (binaires)

- Aucun appel bloquant dans un contexte tokio : `spawn_blocking` pour sysfs,
  lecture de fichiers et ioctls.
- Canaux bornés, backpressure, annulation coopérative, arrêt propre.
- Un worker qui meurt est signalé et supervisé ; pas de tâche détachée
  silencieuse.
- Le daemon ne lit jamais `/proc/*/environ` (D6) : l'agent de session fournit
  l'environnement graphique.
- Polling uniquement lorsqu'un client demande les capteurs ; la boucle
  ventilateurs à 1 s est la seule exception documentée.

## 7. Tests

- Tests unitaires dans le module concerné (`#[cfg(test)]`).
- Tests d'intégration : sysfs factice via `TCC_SYSFS_ROOT` (feature
  `dev-overrides`), JSON réels de `/etc/tcc` pour la migration, fichiers
  temporaires `tempfile` pour la persistance.
- La CI ne dépend d'aucun laptop TUXEDO.
- Couvrir : entrées invalides, valeurs non finies, bornes min/max, JSON
  tronqué/immense, symlinks, permissions, migration idempotente, debounce,
  override, PPD absent/inconnu, batterie inconnue.
- `cargo test --workspace` et `cargo test -p tcc-core --features dev-overrides`
  doivent passer.

## 8. Qualité et CI

Portes bloquantes :

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
```

- MSRV déclarée : `rust-version = "1.85"`, edition 2024.
- Docs publiques obligatoires (`warn(missing_docs)`), rustdoc sans lien cassé.
- Profil release : LTO thin, `codegen-units = 1`, symboles retirés.

## 9. Performance

Budgets indicatifs à mesurer, pas à supposer :

- daemon inactif : < 1 % CPU, < 30 Mo RSS ;
- GUI : < 200 Mo RSS, démarrage < 1,5 s ;
- pas d'allocation par tick dans les boucles chaudes (réutiliser les buffers,
  précalculer les courbes) ;
- event-driven par défaut (PPD, UPower, udev, inotify, D-Bus) ; timers
  coalescés ;
- benchmarks criterion à ajouter pour les courbes, la résolution d'état et la
  sérialisation avant toute optimisation.

## 10. Nommage et documentation

- Identifiants Rust en anglais, documentation et messages en français pour
  l'instant (traduction UI via Qt/i18n).
- Un module = une responsabilité. Les fonctions publiques ont un doc comment
  avec invariants et section `# Errors`.
- Les décisions non évidentes (borne, fallback, compatibilité) sont
  commentées dans le code et référencées ici ou dans
  `docs/rust/INTEGRATION.md`.
