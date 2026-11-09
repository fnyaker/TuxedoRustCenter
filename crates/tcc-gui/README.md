# tcc-gui — shell Qt 6 natif

GUI de TuxedoRustCenter. **Jalon 1** : design system et shell QML complets.
**Jalon 1.5** : profils et état actif lus en direct du daemon via le pont Rust
`DaemonClient` (cxx-qt), avec repli explicite sur `DemoData` quand le daemon est
absent. L'écriture de profil n'est pas branchée : l'éditeur est en lecture
seule pour les profils du daemon.

Aucun composant web : pas de QtWebEngine, QtWebView, HTML/CSS/JS applicatif ni
Angular. Qt Quick Controls 2 (style Basic) + composants dessinés en QML.

## Structure

```
crates/tcc-gui/
├── Cargo.toml                 # binaire + cxx-qt
├── build.rs                   # module QML `Tcc` (cxx-qt)
├── src/
│   ├── main.rs                # QApplication (tray), --tray, moteur QML
│   ├── qapplication_shim.cpp  # glue QtWidgets/QApplication (tray Qt Labs)
│   ├── daemon_client.rs       # pont Rust ↔ QML (D-Bus tccd/Manager1, worker unique)
│   ├── hardware_view.rs       # vue typée de HardwareSnapshot (lecture seule)
│   ├── fan_view.rs            # vue typée de GetFanDataJSON (lecture seule)
│   ├── profile_edit.rs        # DTO d'édition borné + validation + application
│   ├── power_blocker.rs       # inhibiteur de veille logind (fd, non persistant)
│   ├── autostart.rs           # fichier XDG autostart (atomique, sans privilège)
│   ├── profile_transfer.rs    # import/export JSON borné (enveloppe versionnée)
│   ├── settings_edit.rs       # DTO réglages + charge + matrice + minuterie
│   └── (profile_edit.rs couvre aussi création/duplication/suppression)
├── qml/
│   ├── Main.qml               # ApplicationWindow, pont daemon, navigation
│   ├── core/
│   │   ├── qmldir             # singletons Theme/Strings/AppState/DemoData/LiveData
│   │   ├── Theme.qml          # jetons (couleurs, typo, espaces, mouvement)
│   │   ├── Strings.qml        # i18n FR/EN/DE + formats °C/°F
│   │   ├── AppState.qml       # page, préférences, toasts, demandes d'action
│   │   ├── LiveData.qml       # adaptateur profils/état (JSON daemon → affichage)
│   │   ├── HardwareData.qml   # adaptateur matériel (capteurs, charge, ODM, …)
│   │   ├── AdminState.qml     # état Admin1 + erreurs d'enregistrement
│   │   ├── ProfileRules.qml   # validation locale + DTO d'édition
│   │   ├── SettingsData.qml   # réglages globaux live (booléens, charge, matrice)
│   │   ├── ShutdownTimerData.qml # statut réel de la minuterie (Manager1)
│   │   ├── PowerBlockerData.qml  # inhibiteur de veille (état + erreurs)
│   │   ├── AutostartData.qml     # autostart XDG (état + erreurs)
│   │   └── DemoData.qml       # ⚠ repli + données sans contrat
│   ├── components/            # design system (voir docs/ui/UI_DESIGN.md)
│   ├── pages/                 # Dashboard, Profils, Éditeur, Outils, Réglages,
│   │                          # Support, À propos
│   ├── tray/SystemTray.qml    # tray natif Qt.labs.platform (tolérant)
│   └── qtquickcontrols2.conf
├── assets/
│   ├── fonts/                 # Titillium Web + OFL-1.1.txt
│   ├── icons/                 # SVG TCC normalisés
│   ├── brand/                 # logos
│   └── ATTRIBUTION.md
├── tests/qml/
│   ├── tst_design_system.qml  # jetons, i18n, composants (15 cas)
│   ├── tst_live_data.qml      # adaptateur LiveData (15 cas)
│   ├── tst_hardware_data.qml  # adaptateur HardwareData (13 cas)
│   ├── tst_navigation.qml     # liens Dashboard → éditeur / Réglages §PRIME
│   ├── tst_profile_rules.qml  # validation locale + mapping d'erreurs admin
│   ├── tst_profile_editor.qml # éditeur : admin, DTO, presets, conflit
│   ├── tst_fan_curve_editor.qml # courbe CPU : points, validation, apply
│   ├── tst_settings_page.qml  # réglages/charge/matrice : validation, conflit
│   ├── tst_profiles_page.qml  # CRUD + import/export de profils
│   ├── tst_tools_timer.qml    # minuterie : brouillon, annulation, statut
│   ├── tst_power_blocker.qml  # inhibiteur de veille : toggle, états, erreurs
│   ├── tst_autostart.qml      # autostart : toggle, erreurs, tray indisponible
│   ├── tst_tray.qml           # tray natif : menu, actions, toggles
│   ├── tst_webcam_controls.qml # UVC lecture seule : valeurs, menus, absence
│   └── tst_states.qml         # états dégradés (qa-integration)
└── tools/qml-check.sh         # lint + tests
```

## Aperçu

Build puis lancement (le module QML `Tcc` est généré par `build.rs` et lié au
binaire, il n'est pas chargeable par `qmlscene`) :

```sh
cargo build -p tcc-gui
./target/debug/tcc-gui

# daemon de test sur le bus de session : la feature `dev-overrides` est
# requise pour que TCC_DBUS_BUS/TCC_DBUS_SERVICE soient lus (D7/R-14).
TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
  cargo run -p tcc-gui --features dev-overrides
```

Validation de chargement sans écran :

```sh
QT_QPA_PLATFORM=offscreen timeout 5 ./target/debug/tcc-gui
```

Lint et tests :

```sh
crates/tcc-gui/tools/qml-check.sh
# équivalent manuel :
/usr/lib/qt6/bin/qmllint crates/tcc-gui/qml/Main.qml \
  crates/tcc-gui/qml/core/*.qml crates/tcc-gui/qml/components/*.qml \
  crates/tcc-gui/qml/pages/*.qml crates/tcc-gui/tests/qml/*.qml
/usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml
```

`cargo check -p tcc-gui` doit rester vert ; `cargo build -p tcc-gui` régénère
le module QML cxx-qt consommé à l'exécution.

## Sécurité du pont Rust ↔ QML

- **Un seul worker D-Bus** : les invokables QML (`refresh()`,
  `setTempProfile()`, `clearTempProfile()`) empilent une commande dans une file
  bornée (`sync_channel(8)`) traitée par le thread `tcc-dbus` ; aucun thread
  n'est créé par appel et une file pleine renvoie l'état « occupé » (`busy`).
- **Réponses bornées** : `GetStateJSON`/`GetProfilesJSON` sont rejetées
  au-delà de 4 Mio **avant** `serde_json::from_str`.
- **JSON canonique** : QML ne reçoit que la re-sérialisation des modèles
  validés (`StateSnapshot`, `ProfileCollection`), jamais la chaîne brute.
- **Overrides dev** : `TCC_QML_DIR`, `TCC_DBUS_SERVICE` et `TCC_DBUS_BUS` ne
  sont lus qu'avec la feature `dev-overrides`, absente des paquets release.
- **Données matérielles en lecture seule** : `GetHardwareSnapshotJSON`
  (`Manager1`) est borné à 4 Mio et re-sérialisé depuis `hardware_view.rs` ; la
  collecte capteurs est un bail de 10 s renouvelé uniquement quand le Dashboard
  est visible (la sortie de page laisse expirer sans envoyer `false`).
  Aucun setter charge/éclairage/Fn-Lock/TDP/ODM/PRIME/YCbCr n'est appelé.
- **Ventilateurs** : `GetFanDataJSON` est lu toutes les 2 s (vitesse, consigne,
  mode, backend) et affiché en lecture seule ; le pilotage reste au daemon.
- **Enregistrement de profil** : DTO borné (64 Kio) validé puis appliqué sur la
  collection validée, collection sérialisée bornée à 1 Mio et envoyée à
  `Admin1.SaveProfilesJSON` par le worker unique. Détection `Admin1` sans
  activation D-Bus ; presets et champs hors phase 1 en lecture seule.
- **Réglages globaux** : °C/°F et les trois contrôles (CPU, ventilateurs,
  éclairage) sont enregistrés via `Admin1.SaveSettingsJSON` ; seuls ces quatre
  champs changent, `extra`/matrice/charge sont préservés.
- **Collection de profils** : création (ID généré côté Rust), duplication
  (clone profond, nom unique, nouvel ID) et suppression (confirmation,
  presets refusés, `InUse` honoré) passent par `Admin1.SaveProfilesJSON`,
  avec relecture/conflit et file bornée.

## Données : `LiveData` vs `DemoData`

- `qml/core/LiveData.qml` : adaptateur des profils (`GetProfilesJSON`), du
  profil actif, du PPD, de la source d'alimentation et de l'override manuel.
  Il valide la forme, normalise les unités noyau (kHz → GHz, sentinelles `-1`)
  et **retombe intégralement sur `DemoData`** si le daemon est absent ou si le
  payload est invalide. `JSON.parse` n'est exécuté qu'une fois par payload.
- `qml/core/DemoData.qml` : repli explicite + données encore sans contrat
  (capteurs, agent de session, charge, outils, identité appareil). Il est
  signalé dans l'UI par un badge « Démo ».

Règles :

- ne jamais ajouter de logique métier dans QML (validation, plages, application) ;
- les pages n'importent pas le module cxx-qt `Tcc` : elles lisent `LiveData` et
  émettent des signaux `AppState` (`refreshRequested`,
  `profileActivationRequested`, `tempProfileClearRequested`) traduits en D-Bus
  par `Main.qml` ;
- aucune écriture privilégiée en QML ; la collecte simulée ne tourne que
  lorsque le dashboard est visible.

Détail des règles de repli et des limites : `docs/ui/UI_DESIGN.md` §15.

## Reste à faire côté Rust/daemon

- Écrire les profils (`saveProfile`, duplication, suppression, import/export) :
  l'éditeur est en lecture seule pour les profils du daemon.
- Exposer la matrice PPD × AC/BAT, les capacités d'affichage et un snapshot
  capteurs événementiel (voir `docs/ui/UI_DESIGN.md` §15.7).
- Charger `qml/` et `assets/` depuis `/usr/share/tuxedo-rust-center/`.
- Le tray réel sera un `QSystemTrayIcon` + menu Qt Quick piloté par Rust ;
  `components/TrayMenu.qml` décrit uniquement la présentation.

## Accessibilité et performance

- Contrastes AA documentés, focus clavier, cibles ≥ 32 px, rôles accessibles.
- `reduceMotion` désactive toutes les animations.
- Graphiques en `Shape`/`PathSvg` (pas de QtCharts), jauges natives, aucun
  effet de flou ; seules les icônes sont teintées via `MultiEffect` (backend
  RHI requis).

## Licences

GPL-3.0-or-later pour le code ; police Titillium Web sous SIL OFL 1.1 ;
icônes et logos TCC sous GPL-3.0-or-later (© TUXEDO Computers GmbH).
Voir `assets/ATTRIBUTION.md` et `docs/ui/UI_DESIGN.md` §11.
