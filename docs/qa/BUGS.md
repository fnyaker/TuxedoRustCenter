# Bugs et écarts connus — QA

Ce fichier recense les anomalies **reproductibles** trouvées par
`qa-integration`. Chaque fiche indique un test minimal, le fichier concerné,
l'impact, le correctif livré ou proposé, et le propriétaire. Les bugs corrigés
restent documentés pour l'historique et la non-régression.

> **Note** : `crates/tcc-core/tests/known_issues.rs` garde son nom, mais ne
> contient plus de test `#[ignore]` pour BUG-QA-001 et BUG-QA-002. Ces deux
> tests s'exécutent avec `cargo test` et protègent les invariants corrigés.
> Les deux tests de non-régression BUG-QA-003, dans
> `crates/tcc-admin/tests/admin_dbus.rs`, sont également actifs.

Exécution des tests de non-régression :

```sh
cargo test -p tcc-core --test known_issues
```

---

## BUG-QA-001 — perte de `keyboardBacklightStates` illisible à la migration

- **Statut** : **corrigé** par `rust-core`.
- **Sévérité initiale** : moyenne (perte de données de configuration
  utilisateur).
- **Propriétaire** : `rust-core` (`tcc-core`).
- **Fichier** : `crates/tcc-core/src/migration.rs:193-219`.
- **Tests de non-régression** :
  - `crates/tcc-core/src/migration.rs::tests::unreadable_keyboard_backlight_states_are_preserved_verbatim` ;
  - `crates/tcc-core/tests/known_issues.rs::migrate_settings_preserves_unreadable_keyboard_backlight_states` (actif, non ignoré).

### Correctif livré

`migrate_settings` n'utilise plus `serde_json::from_value(value)`, qui consomme
la valeur. La désérialisation emprunte `&value` (`&Value` est un
`Deserializer`) ; en cas d'échec, la valeur d'origine est réinsérée **telle
quelle** dans `settings.extra` avant d'ajouter la note :

```rust
Err(_) => {
    settings
        .extra
        .insert("keyboardBacklightStates".to_owned(), value);
    notes.push(
        "keyboardBacklightStates illisible, conservé dans les champs inconnus"
            .to_owned(),
    );
}
```

Le test unitaire vérifie que la valeur JSON d'origine est conservée sans
transformation, que la note l'annonce, et que la clé survit à la sérialisation
puis à une seconde migration.

### Historique

Avant le correctif, la valeur était retirée de `extra` par
`settings.extra.remove("keyboardBacklightStates")` puis perdue quand la
conversion vers `Vec<KeyboardState>` échouait (mode inconnu, type incorrect).
La note affirmait pourtant que la valeur était conservée.

---

## BUG-QA-002 — collision de nom entre un profil utilisateur et un preset

- **Statut** : **corrigé, décision documentée** par `rust-core`.
- **Sévérité initiale** : moyenne (le client GUI pouvait rejeter toute la
  collection de profils et repasser en mode démo).
- **Propriétaires** : `rust-core` (`presets.rs`, `validate.rs`) et
  `linux-platform` (`tcc-daemon/src/config.rs`).
- **Fichier** : `crates/tcc-core/src/presets.rs:263-350`.
- **Tests de non-régression** :
  - `presets::tests::merge_renames_loaded_profile_on_preset_name_collision` ;
  - `presets::tests::merge_renames_loaded_profiles_sharing_a_name` ;
  - `presets::tests::merge_renames_preset_customized_to_another_preset_name` ;
  - `presets::tests::merge_truncates_renamed_names_without_exceeding_the_limit` ;
  - `crates/tcc-core/tests/known_issues.rs::merged_presets_keep_profile_names_unique`
    (actif, non ignoré).

### Décision documentée

- **L'identifiant est la clé d'unicité et de référence** (`stateMap`,
  `stateMatrix`, API D-Bus). Il n'est jamais modifié pour résoudre un conflit
  de nom : cela casserait les références.
- Les **noms restent uniques** pour la validation et la GUI
  (`ProfileCollection::validate`, comparaison casse/espaces ignorés).
- En cas de collision, c'est le **profil chargé** qui est renommé, **jamais le
  preset** : `Office` → `Office (custom)`, puis `Office (custom 2)`, etc.
- Le renommage est **déterministe**, **idempotent** et **tronqué** à
  `MAX_PROFILE_NAME_LEN` sans couper le suffixe.
- La règle est documentée dans `docs/rust/INTEGRATION.md` §3 (« Moteur de
  bascule ») et `docs/rust/CONVENTIONS.md` §4 (« Sérialisation et
  compatibilité »).

### Correctif livré

`merge_with_default_profiles` réserve les noms des presets qui vont réellement
être ajoutés, suit les noms déjà pris par les profils chargés, puis renomme le
profil en conflit avec `free_profile_name`. Les presets absents sont ajoutés
après, avec leur nom canonique. La collection fusionnée est valide et un
deuxième passage ne change plus les noms.

### Historique

Avant le correctif, la fusion ne dédupliquait que les identifiants. Un profil
utilisateur valide nommé `Office` avec l'identifiant `custom-office` était
conservé, puis le preset `__office__` (nom `Office`) était ajouté :
`ProfileCollection::validate` refusait la collection fusionnée. Le daemon ne
validant pas après fusion (`tcc-daemon/src/config.rs`), la GUI recevait un
payload invalide et basculait en mode démo.

---

## BUG-QA-003 — la validation croisée admin met les fichiers corrompus en quarantaine

- **Statut** : **corrigé** par `linux-platform` (`tcc-admin`).
- **Sévérité initiale** : moyenne (effet de bord inattendu sur disque ; le
  contenu n'était pas perdu, il était déplacé en `.bak`, mais l'API de lecture
  stricte livrée n'était pas utilisée).
- **Propriétaires** : `linux-platform` (`tcc-admin`) et `rust-core`
  (`tcc-core`, intégration de `load_*_no_recovery`).
- **Fichier corrigé** : `crates/tcc-admin/src/service.rs` (`save_profiles_inner`
  et `save_settings_inner`).
- **Tests de non-régression (actifs, non ignorés)** :
  - `crates/tcc-admin/tests/admin_dbus.rs::admin_validation_does_not_quarantine_a_corrupt_settings_file` ;
  - `crates/tcc-admin/tests/admin_dbus.rs::admin_settings_validation_does_not_quarantine_a_corrupt_profiles_file`.

### Vérifier la non-régression

```sh
dbus-run-session -- cargo test -p tcc-admin --test admin_dbus quarantine
```

Les deux tests passent : `2 passed; 0 failed; 0 ignored`.

### Correctif livré

`SaveProfilesJSON` et `SaveSettingsJSON` utilisent désormais les lectures
strictes livrées :

```rust
let settings = match self.store.load_settings_no_recovery() {
    Ok(value) => value,
    Err(StrictLoadError::NotFound { .. }) => Settings::default(),
    Err(_) => return Err(AdminError::Validation("settings".to_owned())),
};
// ...
let profiles = match self.store.load_profiles_no_recovery() {
    Ok(value) => value,
    Err(StrictLoadError::NotFound { .. }) => ProfileCollection::new(Vec::new()),
    Err(_) => return Err(AdminError::Validation("profiles".to_owned())),
};
```

`NotFound` conserve le comportement « premier démarrage » (valeurs par défaut) ;
toute autre erreur (`Invalid`, `TooLarge`, `Io`) produit un refus `Validation`
sans toucher au fichier. Aucun `.bak`, `.bak.corrupt` ou temporaire n'est créé,
renommé ou supprimé par la validation croisée. Aucun contournement n'est
conservé : les anciens appels `load_settings()`/`load_profiles()` ont été
retirés.

### Historique

Avant le correctif, `ConfigStore::load_settings`/`load_profiles` étaient des
lectures **avec récupération** : un fichier JSON corrompu était renommé en
`.bak` (mode 0600) avant de retourner les valeurs par défaut et un
avertissement. L'admin détectait ensuite `has_warning`, mais le renommage avait
déjà eu lieu. Une simple validation croisée pouvait donc déplacer le fichier
corrompu, contrairement à la garantie de lecture stricte. C'était le point
A1-07 de `docs/security/REVIEW_ADMIN_PHASE1.md`.

---

## Observations (pas des bugs confirmés)

### Périmètre LUKS — hors scope, aucun test fonctionnel

- **Décision utilisateur** : changement de mot de passe LUKS non implémenté,
  hors périmètre.
- **Vérification** : aucun identifiant `luksCurrent`/`luksNew`/`luksConfirm`
  ni appel de changement de phrase dans `crates/tcc-gui` ; seul un commentaire
  de périmètre dans `ToolsPage.qml`.
- **Conséquence QA** : aucun formulaire, champ, timer ou secret à tester ;
  les docs `docs/qa/**` ne contiennent aucune procédure LUKS.

### OBS-QA-001 — `tcc-cli` ignore les arguments après une commande simple

- **Fichier** : `crates/tcc-cli/src/main.rs:64-88`.
- **Constat** : `tcc-cli status foo` ou `tcc-cli help foo` sont acceptés ;
  seuls `set-temp` vérifie l'absence d'arguments supplémentaires.
- **Impact** : faible (faute de frappe silencieuse), pas de sécurité.
- **Proposition** : rejeter tout argument résiduel pour toutes les commandes.

### OBS-QA-002 — `is_internal_connector` est un filtre de préfixe permissif

- **Fichier** : `crates/tcc-hw/src/display.rs:31-38`.
- **Constat** : `is_internal_connector("eDP-malicious;args")` renvoie vrai. La
  validation stricte (`tcc_ipc::is_internal_output_name` et
  `tcc_user::backend::validate_internal_output_name`) est appliquée avant
  toute commande compositeur, donc il n'y a pas de faille démontrée.
- **Impact** : défense en profondeur ; un futur appel direct à
  `InternalOutput::try_new` avec un nom non validé pourrait accepter un nom
  douteux.
- **Proposition** : aligner `is_internal_connector` sur la grammaire stricte de
  `tcc-ipc` (préfixe + suffixe ASCII), ou documenter explicitement que ce
  prédicat est un simple pré-filtre.

### OBS-QA-003 — le pont GUI ne borne pas la taille des réponses D-Bus

- **Statut** : **résolu pendant la rédaction** par `qt-ui`.
- **Fichier** : `crates/tcc-gui/src/daemon_client.rs`.
- **Constat initial** : `tccd-user` bornait les payloads à 4 Mio
  (`MAX_REMOTE_JSON_BYTES`), le pont GUI ne bornait pas `GetStateJSON` /
  `GetProfilesJSON` côté client.
- **Correction observée** : `MAX_RESPONSE_BYTES = 4 * 1024 * 1024` et
  `ClientError::ResponseTooLarge` sont désormais vérifiés **avant**
  `serde_json::from_str`, avec test unitaire associé.
- **Reste à faire** : factoriser la constante avec `tcc-ipc`/`tcc-user` si
  `rust-core` le juge utile.

### OBS-QA-004 — test D-Bus du daemon silencieusement sauté sans bus

- **Fichier** : `crates/tcc-daemon/src/dbus.rs:1500`.
- **Statut** : **atténué en CI**. Le job `integration` exécute désormais
  `dbus-run-session -- cargo test -p tcc-daemon --locked`
  (`.github/workflows/ci.yml`), et le job Arch fait de même pour les tests
  daemon.
- **Constat** : si `zbus::Connection::session()` échoue, le test retourne
  `Ok(())` sans marquer « skipped ». En local sans bus de session, il ne teste
  rien.
- **Proposition** : à terme, utiliser un mécanisme de skip explicite
  (`eprintln!` + `return`) ou un feature flag CI pour distinguer « non
  applicable » de « réussi ».

### OBS-QA-005 — schema futur de `settings` réinterprété comme legacy

- **Fichier** : `crates/tcc-core/src/migration.rs:185-191`.
- **Constat** : toute version différente de `SETTINGS_SCHEMA_VERSION` est
  réécrite en version courante avant validation ; un fichier `schemaVersion: 2`
  produit par une version future serait « migré » à l'envers.
- **Impact** : faible tant qu'une seule version existe, risque de perte de
  données lors d'un downgrade.
- **Proposition** : refuser explicitement une version supérieure à la version
  courante et conserver le fichier intact.

### OBS-QA-006 — sleeps fixes dans les tests de workers du daemon

- **Fichier** : `crates/tcc-daemon/tests/hardware_workers.rs:38,64,68`.
- **Constat** : les tests attendent 50 à 100 ms pour observer l'état des
  workers. Sur un runner chargé, une itération peut dépasser ce délai et rendre
  le test instable ; à l'inverse, le test ne prouve pas qu'aucun travail ne
  reste à faire après l'attente.
- **Impact** : flakiness potentielle (non observée localement).
- **Proposition** : utiliser `tokio::time::pause()`/`advance()` ou un canal de
  notification interne, avec un timeout de garde ; à défaut, augmenter les
  marges et documenter le test comme non déterministe.
- **Propriétaire** : `linux-platform` / `tuxedo-hardware`.

### OBS-QA-007 — fermeture vers le tray non automatisée

- **Fichier** : `crates/tcc-gui/qml/Main.qml` (`onClosing`) et
  `crates/tcc-gui/src/main.rs` (`--tray`).
- **Constat** : le comportement « fermer masque au lieu de quitter, `Quitter`
  termine » est implémenté et revu, mais `Main.qml` ne peut pas être instancié
  par `qmltestrunner` isolé car il importe le module cxx-qt `Tcc`. Aucun test
  QML/Rust de bout en bout ne couvre cette transition.
- **Impact** : faible ; une régression fermerait la GUI au lieu de la masquer.
- **Proposition** : extraire la logique `onClosing` dans un composant QML
  testable, ou ajouter un test d'intégration GUI avec le module `Tcc` chargé.
- **Propriétaire** : `qt-ui` / `qa-integration`.

---

## Passes fixtures `HardwareSnapshot`/`Admin.Hw` et `User1`/résolution — aucun nouveau bug

Passes QA des 2026-11-06/07 : ajout des fixtures `hardware_snapshot_*` et
`admin_hw_*`, des tests `crates/tcc-daemon/tests/hardware_fixtures.rs`,
`crates/tcc-ipc/tests/admin_hw_fixtures.rs`,
`crates/tcc-admin/tests/admin_hw_fixtures.rs` et du test QML
`tst_hardware_snapshot_v2.qml`, puis des fixtures `display_info_*`,
`profile_collection_resolution.json` et des tests
`crates/tcc-user/tests/display_info_contract.rs`,
`crates/tcc-user/tests/cli_resolution.rs`,
`agent_and_cli_contract.rs` et `tst_agent_data.qml`.

**Aucun bug produit reproductible nouveau.** Les écarts observés sont des
limites de contrat déjà connues, consignées ici pour les propriétaires :

### OBS-QA-008 — sections imbriquées non tolérantes dans `HardwareSnapshot`

- **Fichiers** : `crates/tcc-daemon/src/hardware/ycbcr.rs:54`
  (`YcbcrData`), `crates/tcc-daemon/src/hardware/sensors.rs:79`
  (`SensorData`).
- **Constat** : les sections de premier niveau sont `#[serde(default)]`, mais
  ces deux structs ne le sont pas. Un `{"ycbcr":{"available":false}}` ou un
  `{"sensors":{"enabled":true}}` est refusé, alors que toutes les autres
  sections acceptent un objet partiel. La GUI (`HardwareView`) tolère tout.
- **Test minimal** :
  `crates/tcc-daemon/tests/hardware_fixtures.rs::missing_required_nested_sections_are_rejected`.
- **Impact** : faible ; le daemon est l'unique producteur du snapshot. Le jour
  où un payload sera relu ou versionné, la tolérance sera asymétrique.
- **Proposition** : ajouter `#[serde(default)]` à `YcbcrData`/`SensorData`, ou
  documenter le contrat comme strict et retirer l'attente de tolérance.
- **Propriétaire** : `tuxedo-hardware` / `linux-platform`.

### OBS-QA-009 — aucune validation de plage côté client

- **Fichier** : `crates/tcc-daemon/tests/hardware_fixtures.rs` (fixture
  `invalid/hardware_snapshot_out_of_range.json`).
- **Constat** : `HardwareSnapshot` n'a pas de `validate()`. Des seuils de
  charge `150`, un TDP `min > max`, un cTGP `9999` et un contrôle UVC
  `min > max` sont parsés et conservés.
- **Impact** : nul tant que le snapshot sert uniquement à l'affichage et que
  les écritures passent par des méthodes validées. Risque si un futur client
  traite ces valeurs comme des autorisations.
- **Proposition** : documenter « payload de sortie non fiable, jamais une
  autorisation » et, si un client externe apparaît, ajouter une validation de
  plage côté consommateur (pas côté daemon, qui doit rester tolérant).
- **Propriétaire** : `rust-core` / `qt-ui`.

### OBS-QA-010 — capabilities d'écriture non transmises à QML

- **Statut** : **partiellement fermé au 2026-11-07** — les capacités de charge
  sont désormais exposées ; seul `webcam.controlsWritable` reste forcé à
  `false`.
- **Fichiers** : `crates/tcc-gui/src/hardware_view.rs:111-125` (`Charging`),
  `crates/tcc-gui/qml/core/HardwareData.qml:380`.
- **Constat** : `writeSupported`, `profileWritable`, `priorityWritable`,
  `thresholdsWritable`, `chargeTypeAvailable`, `chargeTypeWritable` et
  `chargeTypes` sont maintenant décodés et consommés par Réglages
  (`tst_hardware_writes.qml`). `HardwareData.qml` force encore
  `webcam.controlsWritable=false` au lieu de lire le snapshot.
- **Impact** : aucun comportement faux aujourd'hui (`tccd` publie
  `controlsWritable=false` et n'ouvre pas l'écriture UVC), mais l'UI ne pourra
  pas activer l'édition UVC quand elle existera.
- **Proposition** : lire `webcam.controlsWritable` avec le futur contrat
  d'écriture UVC, puis ajouter un test QML de visibilité des contrôles.
- **Propriétaire** : `qt-ui` (avec `linux-platform` pour UVC session).

### OBS-QA-011 — `DisplayInfoView` ne valide pas les dimensions ni les taux

- **Fichiers** : `crates/tcc-gui/src/display_view.rs:19`,
  `crates/tcc-gui/src/daemon_client.rs:2723` (`decode_display_info`),
  `crates/tcc-gui/qml/core/AgentData.qml:159`.
- **Constat** : `DisplayInfoView` désérialise `u32`/`f32` sans contrôle de
  plage ; `decode_display_info` tronque seulement à 256 modes. Les bornes
  `1..=16384` et `1..=1000 Hz` sont appliquées dans `AgentData.qml`. Les
  fixtures `invalid/display_info_zero_dimensions.json` et
  `invalid/display_info_oversized_dimensions.json` passent donc le parsing
  Rust et sont rejetées côté QML.
- **Impact** : faible aujourd'hui (le service `User1` est non privilégié et
  le producteur réel sélectionne l'écran interne), mais un agent bogué ou
  remplacé peut injecter des valeurs absurdes dans la vue Rust avant le
  filtrage QML.
- **Proposition** : valider dimensions/taux dans `DisplayInfoView` (ou dans
  `decode_display_info`) avec les mêmes bornes, et ajouter un test Rust. Ne
  pas dépendre uniquement de QML pour une donnée D-Bus non fiable.
- **Propriétaire** : `qt-ui` / `linux-platform`.

### OBS-QA-012 — modes externes non identifiables dans `internalOutput.modes`

- **Fichier** : `crates/tcc-gui/qml/core/AgentData.qml` (`_modes`).
- **Constat** : les champs `externalOutput`/`externalOutputs` sont ignorés et
  la fixture `display_info_external_modes.json` le vérifie. En revanche, un
  mode externe placé directement dans `internalOutput.modes` serait accepté :
  le rapport ne porte pas de nom de connecteur par mode.
- **Impact** : nul avec le producteur actuel (`tcc-user` ne copie que
  `internal_output.modes` de l'écran interne). Risque uniquement si l'agent
  est remplacé ou bogué.
- **Proposition** : conserver la règle « seul l'agent est source de
  `internalOutput` » et, si le contrat évolue, ajouter un champ
  `output`/`connector` par mode pour permettre un filtrage défensif.
- **Propriétaire** : `linux-platform` / `qt-ui`.

### Vérification de la passe `User1`/résolution

- `crates/tcc-user/tests/display_info_contract.rs` : 13 tests (fixtures,
  contrat, service D-Bus, nom déjà pris, backend indisponible).
- `crates/tcc-user/tests/agent_and_cli_contract.rs` : 20 tests, dont
  résolution seule, résolution+taux, taux seul, aucun changement et collection
  hors bornes.
- `crates/tcc-user/tests/cli_resolution.rs` : 5 tests boîte noire
  (`--set-resolution`, `--dry-run`, code 2 pour arguments invalides, code 1
  propre sans backend).
- `crates/tcc-gui/tests/qml/tst_agent_data.qml` : 7 tests, dont bornes exactes
  `16385` et backend indisponible.
- **Aucun nouveau bug produit reproductible** ; les deux observations
  ci-dessus sont des limites de défense en profondeur.

### Maintenance de test

`crates/tcc-user/tests/agent_and_cli_contract.rs` attendait encore
`refresh_hz: f32` alors que `tcc_user::agent::DisplayTarget::refresh_hz` est
passé à `Option<f32>`. Les trois assertions ont été alignées sur
`Some(120.0)`/`Some(165.0)` (aucune modification de production) ; ce n'était
pas un bug produit mais un test obsolète qui bloquait `cargo test --workspace`.

