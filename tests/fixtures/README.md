# Fixtures de test — TuxedoRustCenter

Ces fichiers sont des **données de test versionnées**, jamais lues par le code
de production. Ils ne contiennent aucun secret, aucune donnée personnelle et
aucun extrait de machine réelle.

## `legacy/`

Reproduit une arborescence `/etc/tcc` de TUXEDO Control Center original :

| Fichier | Contenu |
|---|---|
| `settings` | Réglages camelCase, `stateMap`, `keyboardBacklightStates` historique, champs inconnus. |
| `profiles` | Quatre profils : `__office__`, `__profile_silent__`, un profil personnalisé 165 Hz, un profil sans identifiant. |
| `fantables` | Courbes non triées, courbe sans nom et doublon de nom pour exercer la migration. |

Points volontairement couverts :

- `refreshRate: -1` et `xResolution: -1` / `yResolution: -1` ;
- `scalingMaxFrequency: -1` (`FrequencyLimit::Reduced`) ;
- champs inconnus (`customField`, `futureSettings`, `tccVersion`) ;
- courbes non triées et noms de courbes manquants/dupliqués ;
- identifiant de profil absent (génération déterministe d'un UUID côté code).

## `dbus/`

Payloads représentatifs de l'API D-Bus de transition :

| Fichier | Contenu |
|---|---|
| `state_snapshot.json` | `StateSnapshot` nominal (`balanced`/`battery`, matrice). |
| `state_snapshot_manual_override.json` | Override manuel cohérent. |
| `state_snapshot_pre_first_apply.json` | Avant la première application (PPD `null`, profil `null`). |
| `active_profile.json` | `ActiveProfile` d'un preset. |
| `active_profile_gaming.json` | `ActiveProfile` d'un profil personnalisé 165 Hz. |
| `profile_collection.json` | `ProfileCollection` courant avec champ inconnu `x-extension`. |
| `profile_collection_resolution.json` | Quatre profils pour la résolution : résolution seule, résolution+taux, taux seul, aucun changement. |
| `invalid/` | JSON malformé, champs manquants, enum inconnue, contrat incompatible, override incohérent, identifiants dupliqués, valeurs hors plage. |
| `invalid/profile_collection_bad_resolution.json` | Profil avec `xResolution=16385` et `useResolution=true` : rejeté par `ProfileCollection::validate` avant application. |

La taille excessive des payloads d'état/profils n'est pas stockée : les tests
la génèrent à l'exécution (`MAX_REMOTE_JSON_BYTES + 1`) pour ne pas alourdir
le dépôt.

### Service de session `User1` — `GetDisplayInfoJSON`

| Fichier | Contenu |
|---|---|
| `display_info_full.json` | Rapport complet : session Wayland/Hyprland, `eDP-1` courant 2560×1600@60, quatre modes sur deux résolutions (60/120/165 Hz). C'est la forme canonique produite par `tcc_user::DisplayInfo`. |
| `display_info_backend_unavailable.json` | `available=false`, backend indisponible, `error="backend absent"`, écran interne vide. |
| `display_info_no_internal.json` | Session X11, backend disponible, uniquement un écran externe : `hasInternalDisplay=false`, `supported=false`, aucun mode interne. |
| `display_info_contract.json` | Résumé **de contrat** (pas un payload réseau) : version, nom de service, objet, interface, méthodes (affichage en lecture, écriture UVC de session) et indicateur `readOnly`. |
| `invalid/display_info_wrong_types.json` | Types faux (`width` chaîne, `modes` objet, `refreshHz` chaîne). |
| `invalid/display_info_zero_dimensions.json` | Dimensions `0` dans le mode courant et les modes. |
| `invalid/display_info_oversized_dimensions.json` | `16385`/`20000` px et taux `1001` Hz. |
| `invalid/display_info_external_modes.json` | Payload valide pour l'écran interne, plus `externalOutput`/`externalOutputs` (HDMI/DP) qui doivent être ignorés. |

Le contrat public est `com.tuxedocomputers.tccd.User1` sur le bus **de
session** ; `SESSION_CONTRACT_VERSION = 1`. Les fixtures sont consommées par
`crates/tcc-user/tests/display_info_contract.rs` et le test QML
`tst_agent_data.qml`.


### Payloads matériels `Manager1.GetHardwareSnapshotJSON`

| Fichier | Contenu |
|---|---|
| `hardware_snapshot_full.json` | Snapshot complet : `device`, `cpu`, `display`, `capabilities`, `charging` (drapeaux d'écriture + `chargeTypes`), `lighting`, `odm`/TDP, capteurs, `nvidia`, `webcam` (contrôles UVC réels), `fnLock`, `prime`, `ycbcr`, plus une clé inconnue `x-extension`. |
| `hardware_snapshot_empty.json` | `{}` : toutes les sections de premier niveau prennent leur valeur par défaut. |
| `hardware_snapshot_partial.json` | Sections partielles tolérées ; `ycbcr` reste complet car sa struct n'a pas `#[serde(default)]`. |
| `hardware_snapshot_charge_types.json` | Liste `chargeTypes` non vide (compatibilité ascendante) et drapeaux `*Writable`. |
| `hardware_snapshot_no_gpu.json` | Appareil sans dGPU : `capabilities.hasDedicatedGpu=false`, `sensors.dgpu=null`, NVIDIA et PRIME indisponibles. |
| `hardware_snapshot_webcam_without_controls.json` | Webcam sans contrôle lisible (`controlsSupported=false`, `controlsError="EACCES"`). |
| `invalid/hardware_snapshot_wrong_type_bool.json` | Type faux sur `device.available`. |
| `invalid/hardware_snapshot_wrong_type_array.json` | Type faux sur `charging.startThresholds`. |
| `invalid/hardware_snapshot_partial_ycbcr.json` | Section `ycbcr` partielle : refusée par le type daemon (non `default`). |
| `invalid/hardware_snapshot_partial_sensors.json` | Section `sensors` partielle : refusée par le type daemon (non `default`). |
| `invalid/hardware_snapshot_out_of_range.json` | Seuils de charge, TDP, cTGP et contrôle UVC hors bornes : **parsés et conservés** (payload de sortie sans `validate()`). |
| `invalid/hardware_snapshot_webcam_oversized.json` | 1 device, 65 contrôles et un menu de 129 entrées : dépasse volontairement les bornes d'affichage de la GUI (16/64/128). |

Les bornes de production et d'affichage diffèrent volontairement :

- daemon / `tcc-hw` : 16 devices, 256 contrôles, 64 menus
  (`crates/tcc-hw/src/v4l2/mod.rs`) ;
- pont GUI / QML : 16 devices, 64 contrôles, 128 menus
  (`crates/tcc-gui/src/daemon_client.rs`, `qml/core/HardwareData.qml`).

La fixture `hardware_snapshot_webcam_oversized.json` est la seule à dépasser
les bornes d'affichage ; elle reste sous la règle des 64 Kio et sert au test
de troncature QML. Aucune fixture ne dépasse 4 Mio.

### Payloads `Admin.Hw` (`AdminHw1`)

Les méthodes `Admin.Hw` utilisent des types D-Bus natifs (`ai`, `s`, `i`), pas
du JSON. Ces fixtures décrivent donc les arguments/erreurs du contrat, pas un
payload réseau :

| Fichier | Contenu |
|---|---|
| `admin_hw_tdp.json` | `SetTdpValues` avec trois watts `[35, 45, 65]` et la relecture attendue. |
| `admin_hw_tdp_empty.json` | `SetTdpValues([])` : maxima firmware explicites, relecture des bornes hautes. |
| `admin_hw_odm.json` | `SetOdmProfile("performance")`, source `platform_profile`. |
| `admin_hw_ctgp.json` | `SetNvidiaCtgpOffset(25)`, borne dynamique. |
| `admin_hw_errors.json` | Noms D-Bus de `PolicyDenied`, `PartialApply`, `NotSupported`. |
| `invalid/admin_hw_tdp_too_many.json` | Quatre valeurs TDP (`InvalidArgs`, `MAX_TDP_VALUES=3`). |
| `invalid/admin_hw_tdp_out_of_range.json` | Valeurs hors `1..=1000` (`Validation`). |
| `invalid/admin_hw_odm_too_long.json` | Nom ODM de 65 caractères (`InvalidArgs`, `MAX_HW_STRING_CHARS=64`). |
| `invalid/admin_hw_odm_backslash.json` | Nom ODM contenant `\` (`InvalidArgs`). |
| `invalid/admin_hw_ctgp_out_of_range.json` | 201 W (`Validation`, borne statique 200 W). |
| `invalid/admin_hw_wrong_argument_type.json` | `values` est une chaîne au lieu d'un tableau. |
| `invalid/admin_hw_unknown_error.json` | Code `Timeout` absent de `AdminHwErrorCode`. |

## `tests/support/`

- `fixtures.rs` : localisation des fixtures à partir de `CARGO_MANIFEST_DIR`.
- `fake_sysfs.rs` : builder de racine sysfs factice (`TempDir`), inclus par les
  tests d'intégration de `tcc-hw`.

## Règles d'ajout

1. Une fixture doit rester **petite** (< 64 Kio) et lisible.
2. Aucune donnée issue d'un vrai laptop (numéro de série, EDID, nom d'utilisateur).
3. Aucune valeur dépendante de l'heure, de l'ordre de lecture ou d'un chemin
   propre à la machine. Les chemins conventionnels du contrat (`/dev/video0`,
   `/usr/bin/prime-select`) sont autorisés et documentés.
4. Un cas invalide doit être isolé dans `invalid/` et documenté par un test.
5. Les fixtures ne sont pas du code de production : ne pas les importer depuis
   `crates/*/src`.
