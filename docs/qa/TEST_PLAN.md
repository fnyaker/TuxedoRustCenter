# Plan de tests — TuxedoRustCenter

Statut : **jalon 2**. Ce document est la référence QA du projet : matrice de
couverture, conventions, fixtures, trous connus, stratégie fuzzing et budgets de
performance. Il est maintenu par `qa-integration` ; les propriétaires de code
fournissent les tests unitaires de leurs modules.

Documents liés : `AGENTS.md`, `docs/QUALITY.md`, `docs/ARCHITECTURE.md`,
`docs/FEATURE_PARITY.md`, `docs/security/DECISIONS.md`, `docs/STATUS.md`,
`docs/HANDOFF.md`, `docs/qa/BUGS.md`, `docs/qa/CI_TEST_JOBS.md`,
`docs/qa/HARDWARE_SMOKE_TESTS.md`, `docs/qa/FEATURE_AUDIT.md`.

---

## 1. Principes

1. **Déterminisme** : aucun test ne dépend de l'heure réelle, du hasard non
   seedé, de l'ordre de `read_dir`, du réseau ou d'un état global du poste.
   Le temps est injecté en `Duration` (`StateResolver`), les mutations utilisent
   un xorshift seedé.
2. **Pas de `sleep`** : les seuls délais autorisés sont les timeouts de
   production (D-Bus 3 s) et ils sont testés via des sources simulées, pas en
   attendant réellement.
3. **Timeouts** : tout appel D-Bus ou réseau a une borne ; les tests qui
   touchent un bus sont soit sur un bus de session dédié (`dbus-run-session`),
   soit documentés comme « non applicables » et **explicitement** ignorés.
4. **Aucun privilège** : les tests tournent en utilisateur normal. Aucun
   `sudo`, aucun accès `/sys`, `/dev` ou `/etc` réel.
5. **Aucun matériel réel en CI** : les tests matériels sont des smoke tests
   manuels (`docs/qa/HARDWARE_SMOKE_TESTS.md`). La CI utilise des racines sysfs
   factices (`TempDir`) et `FakeTransport` pour les ioctls.
6. **Pas de réseau** : aucun test n'appelle Internet, un dépôt ou un service
   distant.
7. **Fixtures lisibles** : `tests/fixtures/**` et `tests/support/**` sont
   versionnés, relus et stables. Voir `tests/fixtures/README.md`.
8. **Aucune modification de production par la QA** : un bug reproductible donne
   un test minimal (actif si le correctif est livré, `#[ignore]` documenté
   sinon) et une fiche dans `docs/qa/BUGS.md`.
9. **Pas d'`unwrap` dans les helpers de production** ; les tests peuvent en
   utiliser, mais les helpers partagés doivent produire des messages d'échec
   explicites.

---

## 2. État observé

Les totaux évoluent à chaque passage d'agent. **La référence courante est
`docs/STATUS.md`** (section « Vérifié sur la machine de développement »), qui
donne le nombre de tests workspace et les tests manuels `#[ignore]`. Ne pas
recopier ces totaux ici : ils seraient obsolètes au prochain commit.

La contribution QA est stable et se limite à :

- des fixtures versionnées (`tests/fixtures/**`) et des tests cross-crate
  (migration, IPC, admin, matériel, GUI QML) ; le détail des fichiers est en
  §3 et les totaux restent dans `docs/STATUS.md` ;
- les non-régressions BUG-QA-001/002/003, actives et exécutées par
  `cargo test` ;
- les fixtures `HardwareSnapshot`/`Admin.Hw` ajoutées le 2026-11-06, avec
  tests Rust et QML dédiés.

Les tests `known_issues.rs` (BUG-QA-001 et BUG-QA-002) sont désormais **actifs**
et exécutés par `cargo test`.

---

## 3. Matrice fonctionnalité × type de test

Légende : `oui` = couvert, `partiel` = couvert sur une partie du contrat,
`non` = non couvert à ce jour, `n/a` = sans objet.

| Fonctionnalité | Unitaire | Intégration | Fake sysfs | D-Bus | QML | Matériel réel | Notes |
|---|---|---|---|---|---|---|---|
| Profils / settings / fan (modèles) | oui | oui | n/a | partiel | partiel | non | fixtures legacy + IPC |
| Validation et bornes | oui | oui | n/a | oui | partiel | non | anomalies agrégées |
| Migration `/etc/tcc` | oui | oui | n/a | n/a | n/a | non | idempotence, symlinks, champs inconnus |
| Persistance atomique | oui | oui | n/a | n/a | n/a | non | `.bak`, permissions, O_NOFOLLOW |
| Matrice PPD × AC/BAT | oui | oui | n/a | oui | oui | non | 3×2 éditable via `Admin1`, fallback AC/BAT |
| Debounce / override manuel | oui | oui | n/a | partiel | non | non | temps injecté, pas de sleep |
| PPD absent / inconnu | oui | oui | n/a | partiel | non | non | fallback `stateMap` |
| Alimentation inconnue | oui | oui | oui | oui | non | non | profil par défaut |
| D-Bus daemon (JSON) | partiel | partiel | n/a | oui (session) | non | non | `dbus-run-session` requis |
| Payload `GetHardwareSnapshotJSON` étendu | oui | oui | n/a | partiel | partiel | non | fixtures full/partial/invalides ; `device`, `cpu`, `display`, `capabilities`, charge, webcam ; bornes 16/64/128 |
| Contrat `Admin.Hw` (`AdminHw1`) | oui | oui | n/a | oui (session) | n/a | non | fixtures TDP/ODM/cTGP, `PolicyDenied`, `PartialApply`, `NotSupported` |
| Signal `StateChanged` | non | non | n/a | non | non | non | **trou** jalon 2 |
| Client D-Bus non privilégié | oui | oui | n/a | partiel | non | non | `tcc-cli`, `tcc-user` |
| `Admin1` profils/settings | oui | oui | n/a | oui | partiel | non | polkit, transaction, rollback, InUse |
| `Control1.ReloadConfig` | partiel | oui | n/a | oui (session, non-root) | non | non | root réel = smoke manuel |
| `tcc-authz` / polkit | oui | non | n/a | non | non | non | fake backend, fail-closed |
| Lecture stricte `load_*_no_recovery` | oui | oui | n/a | oui | n/a | non | 12 tests unitaires + 2 non-régressions admin (BUG-QA-003) |
| Veille/reprise `PrepareForSleep` | oui | partiel | n/a | non | non | non | 8 tests d'état + hooks ; logind réel manuel |
| CPU cpufreq / governor / EPP | oui | oui | oui | non | non | non | intel_pstate fixture |
| Ventilateurs tuxi/pwm/io | oui | oui | oui | partiel | non | non | `GetFanDataJSON` |
| Rétroéclairage | oui | oui | oui | non | non | non | quirk amdgpu |
| Power supply / batterie | oui | oui | oui | non | non | non | AC/BAT/unknown |
| DMI / détection TUXEDO | oui | oui | oui | non | non | non | vendor + SKU |
| Refresh rate X11 | oui | partiel | n/a | partiel | non | non | XRandR non exécuté en CI |
| Refresh rate Hyprland/Sway/KDE | oui | partiel | n/a | partiel | non | non | parsing IPC testé unitairement |
| Refresh rate GNOME/Mutter | partiel | non | n/a | non | non | non | agent `DisplayConfig`, session réelle à valider |
| Agent de session `tccd-user` | oui | oui | n/a | partiel | non | non | `resolve_target`, CLI, service `User1` sur bus session |
| Service de session `User1` / `GetDisplayInfoJSON` | oui | oui | n/a | oui (session) | oui | non | fixtures full/unavailable/no-internal, contrat, erreurs, bornes 16 384/1 000 Hz |
| Résolution (agent/CLI) | oui | oui | n/a | n/a | partiel | non | résolution seule, résolution+taux, taux seul, aucun changement, `--dry-run` |
| `tcc-cli` | oui | oui | n/a | partiel | non | non | binaire réel, sans daemon |
| GUI — pont `DaemonClient` | oui | partiel | n/a | non | partiel | non | 78 tests Rust ; pas de test cxx-qt de bout en bout |
| GUI — LiveData | n/a | n/a | n/a | non | oui | non | JSON valide/invalide |
| GUI — design system / thèmes | n/a | n/a | n/a | n/a | oui | non | tokens, contraste partiel |
| GUI — i18n FR/EN/DE | n/a | n/a | n/a | n/a | oui | non | parité des clés |
| GUI — états loading/empty/error | n/a | n/a | n/a | n/a | oui | non | composants |
| GUI — éditeur profils / réglages | oui | partiel | n/a | oui | oui | non | `SettingsEdit`, conflit, `saving`, QML |
| GUI — tray / autostart | oui | partiel | n/a | non | oui | non | `--tray`, fallback, menu, `~/.config/autostart` |
| GUI — power blocker | oui | partiel | n/a | non | oui | non | fd logind, file pleine, sans persistance |
| GUI — minuterie d'extinction | oui | oui | n/a | oui | oui | non | `source=error`, annulation, rate-limit |
| Tombstones d'IDs de profil | oui | oui | n/a | oui | non | non | transaction settings+profils, ID non réutilisable |
| LUKS | n/a | n/a | n/a | n/a | n/a | n/a | **hors périmètre, non testé, aucune méthode/action** |
| Charge / lighting / Fn-Lock | oui | oui | oui | partiel | non | non | profils, seuils, zones RGB, lightbar |
| Webcam V4L2 | oui | partiel | oui | non | non | non | énumération ; kill switch non testé |
| TDP / ODM / NVIDIA / PRIME | partiel | partiel | partiel | oui (Admin.Hw) | non | non | Admin.Hw fixtures ; setters `Manager1` non testés |
| Admin phase 1 / polkit | oui | oui | n/a | oui | partiel | non | profils/settings ; LUKS hors périmètre, non testé, aucune action/méthode |
| Packaging / systemd / udev | non | oui (script) | n/a | non | non | non | `scripts/release/validate.sh --repo` |
| Performance | manuel | manuel | n/a | non | manuel | manuel | budgets non gatés |
| Sécurité entrées JSON/D-Bus | oui | oui | n/a | oui | partiel | non | mutations + fixtures |

### Détail des tests cross-crate ajoutés

| Fichier | Frontière couverte |
|---|---|
| `crates/tcc-core/tests/migration_roundtrip.rs` | fixture `/etc/tcc` → modèles → validation → `ConfigStore` → JSON |
| `crates/tcc-core/tests/state_engine.rs` | presets + settings + matrice + override + debounce |
| `crates/tcc-core/tests/migration_robustness.rs` | mutations/troncatures legacy, UTF-8 invalide, NUL |
| `crates/tcc-core/tests/known_issues.rs` | non-régression BUG-QA-001/002 (tests actifs) |
| `crates/tcc-core/tests/performance_manual.rs` | micro-mesures manuelles (ignorées) |
| `crates/tcc-ipc/tests/dbus_contract.rs` | payloads D-Bus valides/invalides + profil → demande d'affichage |
| `crates/tcc-ipc/tests/payload_robustness.rs` | mutations/troncatures de snapshots et collections |
| `crates/tcc-ipc/tests/admin_hw_fixtures.rs` | contrat statique `AdminHw1` : bornes TDP/ODM/cTGP, noms d'erreur, fixtures invalides |
| `crates/tcc-daemon/tests/hardware_fixtures.rs` | `HardwareSnapshot` complet/partiel/invalide, device sans GPU, webcam sans contrôles, bornes d'affichage, mutations/troncatures |
| `crates/tcc-admin/tests/admin_hw_fixtures.rs` | arguments de fixtures appliqués via D-Bus : TDP/ODM/cTGP, `PolicyDenied`, `PartialApply`, `NotSupported` |
| `crates/tcc-hw/tests/fake_sysfs_integration.rs` | racine sysfs complète, écritures bornées, multi-fan |
| `crates/tcc-user/tests/agent_and_cli_contract.rs` | payloads daemon → cible de refresh/résolution, CLI, planification |
| `crates/tcc-user/tests/display_info_contract.rs` | fixtures `GetDisplayInfoJSON`, contrat `User1`, service D-Bus session, nom déjà pris |
| `crates/tcc-user/tests/cli_resolution.rs` | `--set-resolution`/`--dry-run`, arguments invalides, absence de backend |
| `crates/tcc-cli/tests/cli_contract.rs` | binaire réel, aide/version, absence de daemon |
| `crates/tcc-admin/tests/transaction.rs` | transaction `.bak`/rollback settings et profils |
| `crates/tcc-admin/tests/admin_dbus.rs` | `Admin1` polkit refusé/absent, InUse, Busy, rollback D-Bus |
| `crates/tcc-admin/tests/cli.rs` | binaire `tccd-admin`, refus non-root |
| `crates/tcc-gui/tests/qml/tst_i18n_parity.qml` | parité FR/EN/DE |
| `crates/tcc-gui/tests/qml/tst_states.qml` | états chargement/vide/erreur et accessibilité de base |
| `crates/tcc-gui/tests/qml/tst_hardware_snapshot_v2.qml` | nouvelles sections du snapshot normalisées, payload partiel, bornes webcam 16/64/128 |
| `crates/tcc-gui/tests/qml/tst_agent_data.qml` | rapport `User1` normalisé, modes invalides/externes écartés, backend indisponible |

### Couverture admin/GUI/veille — audit indépendant phase 1

| Sujet | Couverture vérifiée | Preuves principales | Reste à faire |
|---|---|---|---|
| `Admin1` — polkit autorisé/refusé/absent | oui | `tcc-admin/src/service.rs` tests ; `admin_dbus.rs` AllowAll/DenyAll/Unavailable | polkit réel (dialogue, annulation) hors CI |
| `Admin1` — concurrence | oui | `GatedReloader` + `Busy`, sans sleep | rafale multi-clients réelle |
| `Admin1` — transaction `.bak`/rollback | oui | 9 tests `tcc-admin/tests/transaction.rs` (settings **et** profils) | rollback après échec `Control1` réel |
| `Admin1` — conflit / référence | oui | `InUse` via service + nouveau test D-Bus | conflit de révision côté client (pas d'ETag admin) |
| `GetContractVersion` / propriété | oui | `admin_dbus` | — |
| `Control1.ReloadConfig` root-only | partiel | test non-root `tcc-daemon/tests/control.rs` ; root `#[ignore]` manuel | validation root réelle sur bus système |
| GUI — `SettingsEdit` | oui | `settings_edit.rs` : `extra`, `stateMatrix`, charge, conflit, tailles | test cxx-qt de bout en bout |
| GUI — éditeur profils / réglages | oui | QML `tst_profile_editor`, `tst_settings_page`, `tst_profile_rules` | création/duplication/import-export/tray |
| GUI — timeout / file pleine / `saving` | oui | `daemon_client.rs` : file bornée, timeout admin, `run_save` toujours un outcome ; QML `saving` | propriétés Qt réelles sous charge |
| Veille — cycles, annulation, doublons | oui | 8 tests `sleep::tests` + hooks + 10 cycles workers | logind/systemd réel |
| Veille — logind absent / arrêt pendant veille | oui | `logind_absence...`, `shutdown_while_suspended...` | logind réellement absent/en veille |
| `load_*_no_recovery` | oui | oui | 12 tests `config::tests::strict_*` + 2 tests D-Bus admin (BUG-QA-003) | couverture déjà en place |

**Constats** :

- Les tests admin sont parallèle-safe : chaque test D-Bus ouvre sa propre
  connexion et cible son nom unique, les écritures sont bornées à un
  `TempDir`.
- Les tests de veille sont déterministes ; le seul délai est le timeout
  volontaire de `logind_absence_does_not_terminate_the_watcher` (200 ms) et
  la reconnexion de production (5 s).
- Les tests d'intégration `hardware_workers.rs` conservent des `sleep` de
  50–100 ms (OBS-QA-006) ; ils restent hors du périmètre admin/GUI/veille
  mais sont un risque de flakiness.
- **BUG-QA-003 corrigé** : `tcc-admin` utilise désormais
  `load_settings_no_recovery`/`load_profiles_no_recovery` pour sa validation
  croisée (`NotFound` → valeurs par défaut, autre erreur → `Validation`).
  Les deux tests D-Bus sont actifs et vérifient qu'aucun `.bak`/quarantaine
  n'est créé.

### Tray / session / LUKS — audit indépendant

| Sujet | Vérification | Preuves |
|---|---|---|
| LUKS | **hors périmètre** : aucun formulaire, champ, timer, secret ou action | `rg "luksCurrent|luksNew|luksConfirm" crates/tcc-gui` → aucun résultat ; seul un commentaire de périmètre dans `ToolsPage.qml` |
| Tray natif | menu, tooltip, profils, quitter, ouverture/fermeture | `tst_tray.qml` ; `SystemTray.qml` ; `--tray` parsé dans `main.rs` |
| Tray indisponible | repli fenêtre normale sans crash | `test_unavailableTrayDoesNotCrash`, `Main.qml` `ensureStartupVisible` |
| Fermeture = masquer | `onClosing` masque si tray disponible, `Quitter` force `Qt.quit` | revue de code `Main.qml` ; pas de test QML de `Main` (module `Tcc` requis) |
| Autostart | chemin XDG sous `$HOME`, contenu `--tray`, refus hors HOME/symlink | tests `autostart.rs` (TempDir), `tst_autostart.qml`, `tst_tray.qml` |
| Échec d'autostart | toggle revient à l'état réel, pas d'intention fantôme | nouveau `test_autostartFailureRevertsToggle` |
| Power blocker | toggle, fd libéré au drop/queue pleine, session non graphique | tests `power_blocker.rs`, `tst_power_blocker.qml` |
| Minuterie | `source=error` distinct de `none`, heure invalide, annulation, rate-limit | `tst_tools_timer.qml`, tests `shutdown_timer.rs` et `dbus.rs` |
| Tombstones | ID supprimé non recréable, transaction settings+profils | `settings_tombstones.rs`, `service.rs`, `transaction.rs`, `admin_dbus.rs` |

**Constats tray/session** :

- Aucun test ne dépend d'un tray réel, d'un bureau, de root ou de logind : le
  runner est `offscreen`, les backends sont injectés, l'autostart utilise un
  `TempDir`.
- `--tray` est couvert par les tests unitaires de `main.rs` (doublon accepté,
  argument inconnu refusé) mais pas par un lancement réel de la GUI en CI.
- La fermeture « masquer au lieu de quitter » est revue dans `Main.qml` mais
  n'a pas de test automatisé : instancier `Main.qml` demande le module cxx-qt
  `Tcc`, absent du runner QML isolé.
- LUKS ne doit faire l'objet d'aucun test fonctionnel : la seule vérification
  est l'absence d'identifiants dans la surface GUI.

---

## 4. Conventions d'écriture

### Nommage et organisation

- Tests unitaires : dans le module, `#[cfg(test)] mod tests`.
- Tests d'intégration : `crates/<crate>/tests/<sujet>.rs`, pas de logique
  dupliquée avec les tests unitaires ; viser les frontières entre crates.
- Fixtures : `tests/fixtures/<domaine>/...`, helpers : `tests/support/...`.
- Un test = un comportement ; le nom décrit le contrat, pas l'implémentation
  (`ppd_absent_falls_back...`, pas `test_observe_2`).
- Les tests de bug sont nommés `bug_<id>_<comportement>` ; ils restent actifs
  après correction et sont `#[ignore]` uniquement tant que le correctif n'est
  pas livré, avec la référence `docs/qa/BUGS.md`.

### Environnement et processus

- **Ne jamais modifier `std::env` dans un test parallèle** : depuis Rust 2024
  `set_var` est `unsafe`, et cela crée des courses entre tests. Pour tester un
  binaire avec un environnement donné, lancer `std::process::Command` avec
  `.env(...)`.
- **Ne pas contacter le bus système**. `tcc-cli` et `tcc-user` pointent vers un
  socket de session inexistant pour tester l'absence de daemon.
- **Pas de root** : `tcc-user` refuse root ; les tests doivent rester non
  privilégiés. `tccd` n'est pas lancé en root en CI.
- **Pas de réseau** : aucune résolution DNS, aucun téléchargement.
- **Pas de `sleep`** : utiliser un temps injecté, un canal borné, ou un timeout
  de production. Un test qui « attend que ça arrive » est rejeté en revue.
- **Temps réel interdit pour la logique** : `StateResolver` reçoit `Duration`.
- **Fichiers temporaires** : `tempfile::TempDir` uniquement, jamais `/tmp`
  codé en dur.
- **Pas de chemin absolu de développeur** : `fixtures.rs` résout la racine via
  `CARGO_MANIFEST_DIR`.

### Timeouts et bornes

- Timeout D-Bus : 3 s (constante de production `DBUS_CALL_TIMEOUT`).
- Payload daemon : 4 Mio (`MAX_REMOTE_JSON_BYTES`), refusé avant parsing.
- Fichiers de configuration : 1 Mio (`DEFAULT_MAX_CONFIG_BYTES`).
- Mutations de robustesse : itérations bornées (256–512) et seed fixe.
- Aucune boucle non bornée dans un test.

---

## 5. Fixtures et helpers

### `tests/fixtures/legacy/`

Trois fichiers nommés comme dans `/etc/tcc` (`settings`, `profiles`,
`fantables`) pour tester `LegacyPaths` sans transformation. Voir
`tests/fixtures/README.md` pour le détail des champs et les cas volontairement
couverts : sentinelles `-1`, champs inconnus, courbes non triées, identifiant
manquant.

### `tests/fixtures/dbus/`

Payloads `StateSnapshot`, `ActiveProfile`, `ProfileCollection`, `HardwareSnapshot`
et cas invalides. Les nouveaux snapshots matériels couvrent `device`, `cpu`,
`display`, `capabilities`, la charge avec `chargeTypes`/drapeaux d'écriture,
ODM/TDP, NVIDIA, PRIME, YCbCr, Fn-Lock et une webcam avec contrôles UVC réels ;
les cas invalides incluent types faux, sections imbriquées partielles, valeurs
hors bornes, device sans GPU, webcam sans contrôles et webcam surdimensionnée.
Les fixtures `admin_hw_*.json` décrivent les arguments/erreurs de `Admin.Hw`
(types D-Bus natifs, pas de JSON réseau). Voir `tests/fixtures/README.md`.

Les fixtures de session (`display_info_full.json`,
`display_info_backend_unavailable.json`, `display_info_no_internal.json`,
`display_info_external_modes.json`) couvrent `GetDisplayInfoJSON` : mode
courant, plusieurs résolutions/taux, backend indisponible, absence d'écran
interne, écrans externes ignorés, dimensions `0`/`> 16384` et taux hors
`1..=1000`. `display_info_contract.json` est un résumé de contrat (version,
service, interface, méthodes), pas un payload réseau.
`profile_collection_resolution.json` couvre la résolution seule, la
résolution+taux, le taux seul et l'absence de changement.

Les cas de taille excessive (> 4 Mio) sont générés à l'exécution pour ne pas
versionner 4 Mio. La seule fixture volumineuse est
`invalid/hardware_snapshot_webcam_oversized.json` (65 contrôles, 129 menus,
~60 Kio) pour exercer la troncature d'affichage.

### `tests/support/fake_sysfs.rs`

Builder de racine sysfs complète :

- CPU/cpufreq (`intel_pstate`, governors, EPP, fréquences, online) ;
- `power_supply` AC + batterie (+ seuils de charge) ;
- backlight (dont quirk `amdgpu_bl*`) ;
- hwmon `tuxedo_tuxi_sensors` et `tuxedo` + `tuxedo_fan_control` PWM ;
- `tuxedo_keyboard` (charge, Fn-Lock, LEDs clavier/lightbar) ;
- `tuxedo_platform_profile` + ACPI générique ;
- DRM/debugfs YCbCr ;
- DMI ;
- webcam V4L2 minimale.

Il ne dépend d'aucun chemin absolu et ne crée rien hors du `TempDir`.

### `tests/support/fixtures.rs`

Résout `tests/fixtures/**` depuis `CARGO_MANIFEST_DIR` (`crates/<crate>` →
`../..`). Inclus par `#[path = "../../../tests/support/fixtures.rs"]` ; jamais
compilé en production.

### Règles d'ajout

- Une fixture par cas sémantique, pas par variante de formatage.
- Un cas invalide = un fichier dans `invalid/` et un test qui cite le nom.
- Pas de secrets, pas de données personnelles, pas de dump matériel réel.
- Toute fixture ajoutée est référencée dans un test (sinon elle est supprimée).

---

## 6. Commandes

```sh
# Suites stables (sans Qt, sans root) — cible principale QA
cargo test -p tcc-core -p tcc-ipc -p tcc-hw -p tcc-user -p tcc-cli

# Workspace complet (nécessite Qt 6 pour tcc-gui)
cargo test --workspace

# Overrides de développement refusés en root (D7)
cargo test -p tcc-core --features dev-overrides

# D-Bus : les tests daemon/admin/gui doivent voir un bus de session
dbus-run-session -- cargo test -p tcc-daemon -p tcc-admin -p tcc-gui

# Fixtures matérielles : snapshot complet/invalide, device sans GPU, webcam
cargo test -p tcc-daemon --test hardware_fixtures
cargo test -p tcc-ipc --test admin_hw_fixtures
dbus-run-session -- cargo test -p tcc-admin --test admin_hw_fixtures

# Service de session User1 et résolution (bus de session)
cargo test -p tcc-user --test agent_and_cli_contract
cargo test -p tcc-user --test cli_resolution
dbus-run-session -- cargo test -p tcc-user --test display_info_contract

# Admin : transaction, polkit factice, InUse, D-Bus
cargo test -p tcc-admin
dbus-run-session -- cargo test -p tcc-admin --test admin_dbus

# Authz polkit (backend fake, fail-closed)
cargo test -p tcc-authz

# Lecture stricte sans récupération (aucun fichier créé/modifié)
cargo test -p tcc-core --lib config::tests::strict

# Veille/reprise : machine à états, hooks, cycles workers
cargo test -p tcc-daemon sleep::tests
cargo test -p tcc-daemon hardware_runtime_pause_resume_ten_cycles

# Tray, autostart, power blocker, minuterie, tombstones
cargo test -p tcc-gui --bin tcc-gui autostart
cargo test -p tcc-gui --bin tcc-gui power_blocker
cargo test -p tcc-daemon shutdown_timer
cargo test -p tcc-ipc --test settings_tombstones

# Tests QML et lint du shell
crates/tcc-gui/tools/qml-check.sh

# Invariants packaging/FHS/systemd/D-Bus/polkit côté dépôt
scripts/release/validate.sh --repo

# Tests de non-régression des bugs corrigés (actifs)
cargo test -p tcc-core --test known_issues

# Non-régression BUG-QA-003 (validation croisée admin sans quarantaine)
dbus-run-session -- cargo test -p tcc-admin --test admin_dbus quarantine

# Mesures manuelles
cargo test -p tcc-core --release --test performance_manual -- --ignored --nocapture
```

Contrôles obligatoires avant revue :

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

---

## 7. Trous de couverture connus

Priorisés par risque :

1. **Signal `StateChanged`** : aucun test d'émission/abonnement D-Bus. Le
   daemon teste les méthodes sur bus de session, pas le signal. À couvrir au
   jalon 2 avec un client de test zbus.
2. **Bus système** : aucun test ne valide la policy D-Bus, polkit ou un appel
   non privilégié réel. C'est volontaire (pas de root), mais la policy doit
   être validée par `security` et un test d'intégration conteneurisé.
3. **Pont GUI `DaemonClient`** : couvert côté pipeline Rust (file bornée,
   timeout, conflit, `saving`, mapping d'erreurs) et QML, mais sans test
   cxx-qt de bout en bout avec un vrai `QObject`/bus.
4. **Pages QML** : éditeur de profil et réglages globaux couverts (save,
   conflit, erreurs, `saving`) ; **Support et Info restent des démos**
   (diagnostic factice, WebFAI/AnyDesk/Tomte/DemoData), et le tray natif,
   `Main.qml` (fermeture/masquage) et l'audit d'accessibilité complet ne sont
   pas automatisés.
5. **`tccd-admin` phase 1** : polkit factice, transaction et D-Bus couverts ;
   le polkit réel (autorisé/refusé/annulé, service arrêté) et le root sur bus
   système restent manuels ; Tomte/WebFAI hors phase 1. **LUKS est hors
   périmètre (décision utilisateur) : non implémenté, non testé, aucune action
   ni méthode D-Bus/polkit.**
6. **TDP/ODM, NVIDIA, PRIME** : les contrôles ioctl existent et `Admin.Hw`
   est couvert sur bus de session avec des fixtures (`admin_hw_fixtures.rs`) ;
   il manque encore un test d'intégration D-Bus du `Manager1` pour les setters
   TDP/ODM/cTGP et la bascule PRIME. La charge, le lighting, le Fn-Lock et
   l'énumération webcam sont couverts par la racine factice ; les contrôles
   UVC et le kill switch webcam restent à tester sur matériel.
7. **Packaging/systemd/udev** : `scripts/release/validate.sh --repo` couvre les
   invariants dépôt ; installation/désinstallation réelles, polkit et unités
   restent validées par les conteneurs de `packaging-release`.
8. **Fuzzing continu** : seules les mutations déterministes existent ; aucune
   cible `cargo-fuzz` ni corpus persistant.
9. **Couverture de code** : aucun seuil mesuré (`cargo llvm-cov` proposé, non
   gaté).
10. **Performance** : budgets non gatés, mesures manuelles uniquement.
11. **Matériel réel** : aucun test automatisé ; smoke tests manuels.
12. **GNOME/Mutter** : agent `DisplayConfig` implémenté mais non validé sur
    session réelle ; wlroots générique reste un stub.
13. **Veille réelle** : `PrepareForSleep` est couvert par des hooks et une
    machine à états ; `systemctl suspend`/reprise, logind réellement absent et
    fuite de tâches à long terme restent des smoke tests matériels.
14. **Fonctions GUI factices/read-only** : voir l'inventaire détaillé
    `docs/qa/FEATURE_AUDIT.md` (54 entrées : 24 fermées, 6 read-only,
    7 masquées, 4 démo, 6 partielles, 2 stubs, 1 désactivée, 4 défenses).
    Les priorités restantes sont PRIME/`Admin.Prime`, UVC écriture et kill
    switch webcam, icônes/toggles AC/BAT, couleur lighting par zone, jauge et
    courbe GPU, Fn-Lock tray, Support/Tomte/Aquaris, et les bornes
    cTGP/TDP/`DisplayInfoView`.
15. **Tolérance du `HardwareSnapshot`** : les sections de premier niveau sont
    `#[serde(default)]`, mais `ycbcr` et `sensors` ne le sont pas sur leur
    struct. Un payload partiel de ces deux sections est refusé
    (`crates/tcc-daemon/tests/hardware_fixtures.rs`), alors que la vue GUI
    (`HardwareView`) tolère tout. À trancher par `tuxedo-hardware` /
    `linux-platform` : ajouter `#[serde(default)]` ou documenter le contrat
    comme strict. Aucun impact actuel : le daemon est le seul producteur.
16. **Pas de validation de plage côté client** : `HardwareSnapshot` n'a pas de
    `validate()` ; les seuils de charge, TDP, cTGP et contrôles UVC hors bornes
    sont parsés et conservés (fixture `hardware_snapshot_out_of_range.json`).
    C'est acceptable pour un payload de sortie, mais la GUI ne doit jamais en
    déduire une autorisation d'écriture. À couvrir si un client hors daemon
    consomme le snapshot.
17. **Capabilities d'écriture non exposées à QML** : `HardwareView::Charging`
    ignore encore `writeSupported`, `profileWritable`, `priorityWritable`,
    `thresholdsWritable`, `chargeTypeAvailable`, `chargeTypeWritable` et
    `chargeTypes` ; `HardwareData.qml` force `webcam.controlsWritable=false`.
    Sans impact tant que `tccd` refuse ces écritures, mais à brancher avant
    d'ouvrir l'édition UVC/charge avancée (`qt-ui`).
18. **`DisplayInfoView` privé et non validant** : la vue Rust de
    `GetDisplayInfoJSON` vit dans `tcc-gui/src/display_view.rs` (module privé,
    lié à cxx-qt). Elle est tolérante mais ne valide ni les dimensions ni les
    taux ; `decode_display_info` borne seulement le nombre de modes à 256.
    Les bornes `1..=16384` / `1..=1000 Hz` sont appliquées dans
    `AgentData.qml` (test QML). Un rapport d'agent bogué ou malveillant peut
    donc traverser le pont Rust avant d'être filtré par QML. À couvrir si la
    vue devient publique ou si un consommateur non-QML l'utilise.
19. **Modes externes injectés sous `internalOutput`** : les champs
    `externalOutput`/`externalOutputs` sont ignorés (fixture
    `display_info_external_modes.json`), mais un payload qui placerait des
    modes externes directement dans `internalOutput.modes` serait accepté par
    la vue et QML (aucun nom de connecteur par mode). Le producteur
    `tccd-user` ne copie que l'écran interne ; le risque n'existe que si
    l'agent est remplacé ou bogué.

---

## 8. Fuzzing et property testing

### Existant (sans dépendance supplémentaire)

- `crates/tcc-core/tests/migration_robustness.rs` : 512 mutations + troncatures
  sur `profiles`, 256 sur `settings` et `fantables`, seed fixe.
- `crates/tcc-ipc/tests/payload_robustness.rs` : 512 mutations de
  `StateSnapshot`, 256 d'`ActiveProfile`, 512 de `ProfileCollection`, plus
  troncatures.
- `crates/tcc-daemon/tests/hardware_fixtures.rs` : 512 mutations + troncatures
  du `HardwareSnapshot` complet, avec re-sérialisation des payloads valides.
- Invariant : **aucune panique**, aucun dépassement de taille, aucune
  allocation non bornée au-delà des limites documentées.

### Cibles `cargo-fuzz` proposées (à porter par `rust-core`)

| Cible | Entrée | Oracle |
|---|---|---|
| `fuzz_legacy_settings` | octets | `migrate_settings` + `Settings::validate` |
| `fuzz_legacy_profiles` | octets | `migrate_profiles` + `ProfileCollection::validate` |
| `fuzz_legacy_fantables` | octets | `migrate_fan_tables` + validation |
| `fuzz_state_snapshot` | octets | `StateSnapshot` parse + `validate` |
| `fuzz_profile_collection` | octets | parse + validate + re-sérialisation |
| `fuzz_display_request` | octets | `DisplayModeRequest` parse + validate (écran interne) |
| `fuzz_session_request` | octets | `SessionAgentRequest` parse + validate |

Règles :

- corpus initial = `tests/fixtures/**` ;
- taille maximale d'entrée 1 Mio ;
- pas de réseau, pas de système de fichiers hors `TempDir` ;
- campagne PR : 60 s par cible ; campagne nightly : 10 min par cible ;
- un crash donne un fichier de repro dans `tests/fixtures/fuzz-regressions/`
  et un test de régression déterministe.

### Property tests proposés (`proptest`, dev-dependency à ajouter par `rust-core`)

- `StateResolver` : après confirmation, `current().key == candidate` ; un
  override valide ne change jamais sans changement de clé confirmé ; le
  debounce ne « perd » jamais une transition.
- Migration : `migrate(serialize(migrate(x))) == migrate(x)` (idempotence) sur
  des profils arbitraires valides.
- `FanCurveEvaluator` : sortie toujours dans `[0, 100]`, monoticité locale
  entre deux points consécutifs, hystérésis dans le seuil.
- Validation : aucune valeur non finie n'est acceptée ; les messages citent le
  champ fautif.

---

## 9. Performance

Budgets de `AGENTS.md` / `docs/QUALITY.md` :

| Composant | Budget | Méthode |
|---|---|---|
| daemon inactif | < 1 % CPU, < 30 Mo RSS | `pidstat`, `ps` après 60 s |
| GUI | < 200 Mo RSS, démarrage < 1,5 s | `hyperfine`, `ps` |
| boucle ventilateurs | 1 s, lectures minimales | revue code + trace |
| état PPD/batterie | événements + fallback 5 s | revue code + logs |
| pas de polling capteurs sans client | vérifié par revue | `GetFanDataJSON`/dashboard |

Procédure manuelle (release) :

```sh
cargo build --release --workspace
# daemon (session de dev, non root)
./scripts/dev-session.sh
pidstat -p "$(pgrep -n tccd)" 1 60
ps -o pid,%cpu,rss,etime,cmd -p "$(pgrep -n tccd)"
# GUI
QT_QPA_PLATFORM=offscreen timeout 5 ./target/release/tuxedo-rust-center &
sleep 2
ps -o pid,%cpu,rss,etime,cmd -p "$(pgrep -n tuxedo-rust-center)"
```

- Les micro-mesures `performance_manual.rs` sont `#[ignore]` et ne sont **pas**
  des gates CI.
- Mesures locales indicatives (release, 2026-11-06) : migration + validation
  d'une fixture legacy ≈ 56 µs par itération ; résolution d'état à chaud
  ≈ 17,7 ns par appel. Ces valeurs servent de référence de non-régression
  locale, pas de seuil CI.
- Tant que la variance des runners n'est pas caractérisée, aucun seuil de temps
  n'est bloquant en CI. Une régression > 2× sur une mesure manuelle ouvre une
  fiche.

---

## 10. Gestion des bugs

1. Reproduire avec une fixture ou un payload minimal.
2. Ajouter un test qui échoue, nommé `bug_<id>_...` ; le garder `#[ignore]`
   avec la référence `docs/qa/BUGS.md` tant que le correctif n'est pas livré,
   puis le réactiver.
3. Documenter : sévérité, fichier/ligne, impact, correctif proposé, propriétaire.
4. Ne pas corriger le code d'un autre agent dans son périmètre : la QA propose
   le patch, le propriétaire l'applique.

Voir `docs/qa/BUGS.md` pour les fiches en cours.

---

## 11. Definition of done QA

- [ ] Les tests ajoutés sont déterministes (pas de sleep, pas d'heure réelle).
- [ ] Aucun test ne dépend d'un laptop, de root, du réseau ou du bus système.
- [ ] Les cas nominaux **et** les entrées invalides sont couverts.
- [ ] Une fixture ajoutée est référencée par au moins un test.
- [ ] `cargo fmt`, `clippy -D warnings`, tests des crates modifiées passent.
- [ ] Les tests QML passent avec `QT_QPA_PLATFORM=offscreen`.
- [ ] Les trous et bugs connus sont documentés, pas masqués par un `ignore`
  silencieux.
