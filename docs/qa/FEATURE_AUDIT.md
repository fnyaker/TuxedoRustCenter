# Audit des fonctions GUI — état après les vagues 2026-11

Statut : réaudit indépendant `qa-integration`, **2026-11-07**, périmètre
`crates/tcc-gui/qml/**`, `crates/tcc-gui/src/**`, `crates/tcc-user/**` et
`docs/FEATURE_PARITY.md`. Aucun code de production n'a été modifié.

Mise à jour après les lots du 2026-11-07 : **collecte capteurs automatique**,
**carte iGPU dédiée**, **ODM/TDP/cTGP éditables par profil**, **backend UVC**
et **vagues UI** (barre Enregistrer flottante, panneau tray compact, icône
launcher/app_id, nettoyage Support/Profils/Réglages/Outils/Dashboard/Sidebar).
Les statuts tiennent compte du code et des tests réellement présents, pas des
commentaires de jalon.

Méthode : relecture des pages QML, composants, singletons `core/` et du pont
Rust, puis vérification de la présence réelle d'un contrat/backend pour chaque
action. Une action qui affiche un toast n'est **jamais** considérée comme
branchée. Une action dont le contrôle a été retiré est **fermée** ; un backend
livré mais non raccordé à l'UI reste **partiel**.

Légende :

- **fermé** : action réelle branchée (`Admin1`, `Admin.Hw`, `User1`, daemon) ou
  contrôle factice retiré ; aucun écart UI restant.
- **read-only** : valeur réelle affichée, écriture volontairement absente ou
  non exposée.
- **désactivé** : contrôle encore présent mais inactif, sans backend ou sans
  effet.
- **démo** : valeur/action encore alimentée par `DemoData`, un timer ou une
  valeur factice.
- **stub** : action visible sans backend réel (toast, navigation sans cible).
- **masqué** : bloc/contrôle retiré ou caché faute de backend ; comportement
  volontaire, fonction à implémenter si la parité l'exige.
- **partiel** : fonction branchée mais une sous-fonction manque.
- **défense** : écart de validation/robustesse, sans action GUI fausse.

---

## 1. Inventaire mis à jour

| # | Écran | Élément | Statut 2026-11-07 | Fichier:ligne | Preuve / reste |
|---|---|---|---|---|---|
| D1 | Dashboard | Modèle d'appareil | **fermé** | `DashboardPage.qml:185-189` ; `HardwareData.qml:66-80` | DMI réel (`HardwareData.deviceLabel`) ; aucune valeur démo en live. |
| D2 | Dashboard | Mode graphique | **fermé** | `DashboardPage.qml:25,331-368` ; `HardwareData.qml:131-133` | `HardwareData.prime.mode` réel ; badge masqué si `prime.available=false`. |
| D3 | Dashboard | État agent/backend | **fermé** | `DashboardPage.qml:20-24,375-389,863-928` ; `AgentData.qml` | `User1` réel : backend, connecteur, mode courant, erreurs. |
| D4 | Dashboard | Compteur de cœurs | **fermé** | `DashboardPage.qml:178-183` ; `HardwareData.qml:81-86` | `HardwareData.cpuCoresText` issu de `cpu.totalCores/onlineCores`. |
| D5 | Dashboard | Carte GPU dédiée (dGPU) | **fermé** | `DashboardPage.qml:67-68,741-745` | `objectName:"dashboardDgpuCard"` ; masquée seulement sur snapshot live fiable sans dGPU. |
| D6 | Dashboard | Jauge ventilateur GPU | **fermé** | `DashboardPage.qml:49-62,813-825` | `fans[]` filtré par `target`/`label` GPU ; jauge masquée si aucun ventilateur GPU, plus de `available:false` codé en dur. |
| D7 | Dashboard | Courbes CPU/GPU | **fermé** | `DashboardPage.qml:980-1049` | En live sans échantillons : tableau vide, plus de `DemoData` en live. |
| D8 | Dashboard | Badge de refresh rate | **fermé** | `DashboardPage.qml:375-389` | Taux réel `AgentData.currentModeLabel` ; masqué si agent absent. |
| D9 | Dashboard | Carte iGPU dédiée | **fermé** | `DashboardPage.qml:71-83,578-738` ; `HardwareData.qml:466-473` | Jauges temp/fréquence/puissance avec `tempAvailable`/`freqAvailable`/`powerAvailable` ; trois graphes de 48 points/2 s ; aucun 0 inventé. |
| D10 | Dashboard/Réglages | Activation manuelle de la collecte | **fermé** | `DashboardPage.qml:555-565` ; `SettingsPage.qml:1371-1429` | Bouton Dashboard et `ToggleRow` Réglages supprimés ; la carte Réglages est passive. `AppState.requestSensorCollection` reste un signal interne testé, sans appelant depuis une page. |
| D11 | Dashboard/Réglages | Collecte automatique / bail | **fermé** | `DashboardPage.qml:133-141` ; `Main.qml:132-137,540-558` ; `daemon_client.rs:1671-1678,1845-1862` | Bail renouvelé seulement si Dashboard affiché **et** fenêtre visible ; expiration 10 s sans `false` ; note passive `sensors.hint`/`sensors.pendingHint`. |
| P1 | Profils | Création/dup./suppression/import | **fermé** | `ProfilesPage.qml:71-90,111-150,192-250` | Live via `Admin1` ; la simulation ne subsiste qu'en mode démo badgé. |
| P2 | Profils | Icônes AC/BAT | **fermé** | `LiveData.qml:94-121,217-260` ; `ProfileCard.qml:113-132` ; `ProfilesPage.qml:54,66-68` | `assignedAc/Bat` annotés depuis les six cellules `stateMatrix` ; icônes et filtre « in use » réels, aucune assignation inventée. |
| E1 | Éditeur | Liste des taux | **fermé** | `ProfileEditorPage.qml:123-150,1024-1048` ; `AgentData.qml:222-250` | Taux réels de la résolution choisie, aucun `DemoData.refreshModes`. |
| E2 | Éditeur | Résolution | **fermé** | `ProfileEditorPage.qml:955-1050` ; `ProfileRules.qml:161-170` ; `profile_edit.rs:628-636` | Toggle + select réels, DTO `useResolution/xResolution/yResolution`, tests Rust/QML. |
| E3 | Éditeur | Assignation AC/BAT | **masqué** | `ProfileEditorPage.qml:875-883` ; `SettingsPage.qml:1432-1635` | Toggles morts supprimés ; note vers la matrice PPD×AC/BAT de Réglages. Pas d'assignation directe par profil, choix assumé. |
| E4 | Éditeur | LED ODM | **masqué** | `ProfileEditorPage.qml` (aucun `odmLed`) | Contrôle factice retiré ; LED ODM non implémentée. |
| E5 | Éditeur | ODM/TDP/cTGP | **fermé** | `ProfileEditorPage.qml:1404-1511` ; `ProfileRules.qml:84-115,226-239` ; `profile_edit.rs:211-249,736-744` | Éditables **par profil** (`odmProfile`, `tdpValues`, `ctgpOffset`), persistés via `Admin1.SaveProfilesJSON` ; `tccd` applique le profil actif (`hardware/odm.rs:326-475`, `hardware/nvidia.rs:170-205`). Preuve réelle ODM `power_save`/`enthusiast` via `tuxedo-io` (`docs/STATUS.md`). Réglages garde `Admin.Hw` transitoire. |
| E6 | Éditeur | Courbe de ventilation | **fermé** | `ProfileEditorPage.qml:1316-1401,1513-1520` ; `FanCurveEditor.qml` ; `ProfileRules.qml:194-215` | Dialogue CPU réel, `customFanCurve` transporté et validé côté Rust. |
| E7 | Éditeur | Courbe par défaut | **démo** | `ProfileEditorPage.qml:379-381` ; `DemoData.qml:110` | Nouveau profil/courbe absente : `DemoData.fanCurve` sert de valeur initiale. |
| E8 | Éditeur | Cœurs/EPP | **fermé** | `ProfileEditorPage.qml:55-69,274-286,1108-1147` | `cpuMaxCores`/`availableEpp` réels ; repli « Auto » sans invention. |
| E9 | Éditeur | Backend/output/mode courant | **fermé** | `ProfileEditorPage.qml:1055-1085` | `AgentData` réel ; lignes masquées si non disponibles. |
| E10 | Éditeur | Préréglages webcam | **stub** | `ProfileEditorPage.qml:1255-1270` | Bouton « Gérer » navigue vers Tools, qui n'a pas d'édition de préréglages UVC. |
| E11 | Éditeur | Kill switch webcam | **read-only** | `ProfileEditorPage.qml:1238-1250` | État supporté/absent affiché ; aucun interrupteur. |
| T1 | Outils | Éclairage clavier/lightbar | **fermé** | `ToolsPage.qml:100-120,180-241,610-860` ; `settings_edit.rs` | Luminosité/couleur/mode/lightbar persistés via `Admin1`, appliqués par le worker. |
| T2 | Outils | Couleur par zone | **partiel** | `ToolsPage.qml:206-221` | La même couleur est dupliquée sur toutes les zones ; pas d'édition zone par zone. |
| T3 | Outils | Kill switch webcam | **read-only** | `ToolsPage.qml:980-989` | État réel, badge lecture seule ; écriture non exposée. |
| T4 | Outils | Contrôles UVC | **partiel** | `ToolsPage.qml:919-950` ; `HardwareData.qml:430,914` ; `session_service.rs:380-520` | Lecture fermée (allowlist 15 CID, 11 contrôles sur `/dev/video0`) ; backend d'écriture `tcc-hw` livré ; `User1.SetUvcControl` livré et testé (allowlist, rate limit, erreurs typées) ; sliders GUI et écriture réelle restent à faire. |
| T5 | Outils | Preview webcam | **fermé** | `ToolsPage.qml` (aucun `preview`) | Bouton factice retiré (masqué) ; preview native non implémentée. |
| S1 | Réglages | Fn-Lock | **fermé** | `SettingsPage.qml:1333-1366` | Interrupteur persistant via `Admin1` ; ligne info si non inscriptible, masqué si non supporté. |
| S2 | Réglages | Type de charge | **fermé** | `SettingsPage.qml:1789-1840` | Select si `chargeTypeWritable` ; type jamais envoyé sinon. |
| S3 | Réglages | Backend/output agent | **fermé** | `SettingsPage.qml:71-90,1853-1950` | `AgentData` réel ; ancien toggle de simulation supprimé. |
| S4 | Réglages | PRIME | **read-only** | `SettingsPage.qml:1994-2030` | Mode/offload réels, aucun switch ; `Admin.Prime` absent. |
| S5 | Réglages | ODM/TDP/cTGP/YCbCr | **partiel** | `SettingsPage.qml:2035-2315` ; `ProfileEditorPage.qml:1404-1511` | ODM/TDP/cTGP désormais par profil via `Admin1` ; `Admin.Hw` reste l'application immédiate transitoire ; YCbCr reste lecture seule. |
| S6 | Réglages | Tomte | **masqué** | `SettingsPage.qml` (aucune carte) | Bloc retiré ; backend absent. |
| S7 | Réglages | Aquaris | **masqué** | `SettingsPage.qml` (aucune carte) | Bloc retiré ; backend absent. |
| S8 | Réglages | Toggle « simuler agent absent » | **fermé** | `AppState.qml` (aucun `demoAgentMissing`) | Contrôle de démonstration supprimé. |
| F1 | Support | Diagnostic système | **masqué** | `SupportPage.qml:1-100` | Timer/ticket factices retirés ; collecte `systeminfos` absente. |
| F2 | Support | Ouvrir le diagnostic | **masqué** | `SupportPage.qml:1-100` | Bouton retiré avec le diagnostic. |
| F3 | Support | WebFAI | **masqué** | `SupportPage.qml:1-100` | Carte retirée ; backend absent. |
| F4 | Support | AnyDesk | **masqué** | `SupportPage.qml:1-100` | Carte retirée ; backend absent. |
| F5 | Support | Contact en ligne | **fermé** | `SupportPage.qml:20-26,63-83` | URL fixe via `Qt.openUrlExternally`, erreur signalée. |
| F6 | Info | Versions/modèle/noyau | **fermé** | `InfoPage.qml:25-145` | Versions réelles, DMI réel ; « non disponible » hors live. |
| F7 | Info | Changelog | **fermé** | `InfoPage.qml:15-20,200-221` | Changelog statique explicitement badgé « statique ». |
| C1 | Global | Repli `LiveData` | **démo** | `LiveData.qml:50-57,186-188` | Repli `DemoData` uniquement daemon déconnecté, badgé démo. |
| C2 | Global | Assignations live | **fermé** | `LiveData.qml:94-121,245-260` ; `ProfilesPage.qml:54,66-68` | Les six cellules PPD×AC/BAT sont lues ; filtre « in use » et icônes complets. |
| C3 | Global | Payload matériel de démo | **démo** | `HardwareData.qml:701-850` | `_demo()` invente charge/lighting/ODM/NVIDIA/PRIME/webcam hors live. |
| C4 | Global | `demoAgentMissing` | **fermé** | `AppState.qml` | Supprimé. |
| C5 | Global | `DemoData` | **démo** | `DemoData.qml:1-183` | Conservé pour le mode démo ; pas de vérité métier. |
| C6 | Tray | Fn-Lock | **fermé** | `SystemTray.qml:57-118,296-310` ; `tst_tray.qml:193-256` | Toggle relié à `Admin1.SaveSettingsJSON`, erreurs révertées ; entrée informative si non inscriptible. |
| C7 | Tray | Fermer = masquer | **partiel** | `Main.qml:141-150` | Implémenté, non couvert par un test QML de bout en bout (module `Tcc`). |
| C8 | Tray | Mode graphique | **read-only** | `SystemTray.qml:38-39,313-318` | Entrée informative, pas de switch ; `Admin.Prime` absent. |
| N1 | Sidebar | Aquaris | **stub** | `Sidebar.qml:162-174` | Entrée visible « bientôt » + toast ; parité dit « masqué ». |
| N2 | Éditeur | Courbe GPU | **partiel** | `ProfileEditorPage.qml:85-86` ; `ProfileRules.qml:205-215` | Seule la courbe CPU est éditable ; le Rust préserve `tableGPU`. |
| N3 | Réglages | Borne cTGP dynamique | **fermé** | `hardware_view.rs:309,530,586` ; `HardwareData.qml:87-89,407-410` ; `tst_admin_hw.qml:280-283` | `maxCtgpOffset` exposé et utilisé ; repli statique 200 W seulement si absent. |
| N4 | Réglages | Monotonie TDP client | **fermé** | `SettingsPage.qml:332-360` ; `ProfileRules.qml:91-107` ; `tst_admin_hw.qml:356-360` | Valeurs TDP croissantes vérifiées côté QML (Réglages et éditeur) et côté serveur ; l'éditeur refuse l'égalité, Réglages et `tcc-core` l'acceptent, sans risque de refus tardif. |
| N5 | GUI/agent | Bornes `DisplayInfoView` | **fermé** | `display_view.rs:73-97,189-211` ; `daemon_client.rs:2761` | `ModeView::is_valid` + `sanitize()` avant QML : dimensions/taux invalides écartés côté Rust. |
| N6 | GUI/agent | Modes externes | **défense** | `AgentData.qml:147-157` ; `display_view.rs` | `externalOutput` ignoré ; un mode externe placé sous `internalOutput.modes` resterait accepté, faute de connecteur par mode. |

### Synthèse par statut

| Statut | Nombre | Éléments |
|---|---:|---|
| fermé | 34 | D1, D2, D3, D4, D5, D6, D7, D8, D9, D10, D11, P1, P2, E1, E2, E5, E6, E8, E9, T1, T5, S1, S2, S3, S8, F5, F6, F7, C2, C4, C6, N3, N4, N5 |
| read-only | 4 | E11, T3, S4, C8 |
| désactivé | 0 | — |
| masqué | 8 | E3, E4, S6, S7, F1, F2, F3, F4 |
| démo | 4 | E7, C1, C3, C5 |
| partiel | 5 | T2, T4, S5, C7, N2 |
| stub | 2 | E10, N1 |
| défense | 1 | N6 |

**Total lignes : 58.** Seule la défense N6 reste ouverte (mode externe non
distinguable sous `internalOutput`) ; N3–N5 sont corrigées et testées.

### Écritures persistantes et transitoires

| Voie | Portée | Statut |
|---|---|---|
| `Admin1.SaveProfilesJSON` | CRUD profils, dont `odmProfile`, `tdpValues`, `ctgpOffset` | **branché** : DTO validé, transaction `.bak`, rollback, tests QML/Rust. |
| `Admin1.SaveSettingsJSON` | charge, Fn-Lock, éclairage, matrice PPD×AC/BAT, minuterie | **branché** : `SettingsPage`, tray Fn-Lock, tests `tst_hardware_writes`/`tst_tray`. |
| `Admin.Hw` | ODM/TDP/cTGP appliqués immédiatement | **transitoire** : distinct des valeurs par profil ; conservé dans Réglages avec consentement PPD. |
| `User1.SetUvcControl` | écriture UVC de session | **service livré/testé** ; sliders GUI en cours ; écriture réelle non testée. |
| Setters `Manager1` | — | **deny** dans la policy ; aucune écriture GUI ne les utilise. |
| PRIME, YCbCr, kill switch webcam | — | **read-only** : aucune méthode d'écriture exposée. |

### Fonctions réellement branchées

- **Profils** : CRUD, import/export, sauvegarde, conflit et brouillon via
  `Admin1.SaveProfilesJSON` (`ProfilesPage.qml`, `profile_edit.rs`,
  `profile_transfer.rs`) ; ODM/TDP/cTGP par profil inclus.
- **Réglages globaux** : charge (profil/priorité/seuils/type), matrice
  PPD×AC/BAT, Fn-Lock, éclairage via `Admin1.SaveSettingsJSON`
  (`SettingsPage.qml`, `settings_edit.rs`).
- **Dashboard** : collecte automatique sur Dashboard visible + fenêtre visible,
  carte iGPU avec disponibilité par métrique, jauge ventilateur GPU issue de
  `GetFanDataJSON`.
- **Affichage** : résolution + taux via `User1`/agent, DTO profil et
  `profile_edit.rs` ; tests `tst_real_capabilities.qml`, `tst_agent_data.qml`.
- **Ventilateurs** : courbe CPU via `Admin1`, éditeur QML et validation Rust.
- **UVC** : lecture via `tccd` ; écriture de session par `User1.SetUvcControl`
  (backend `tcc-hw`), sans sliders GUI pour l'instant.
- **Info/Support** : versions/DMI réels, URL de support fixe, aucun faux
  diagnostic.
- **Tray/autostart/blocker** : panneau compact, Fn-Lock via `Admin1`,
  `~/.config/autostart`, logind.

### Non testable matériel (validation CI impossible)

| Fonction | Ce qui est testé | Reste manuel |
|---|---|---|
| ODM/TDP/cTGP | QML, persistance `Admin1`, bornes, rollback, fakes `Admin.Hw` ; ODM réel `power_save`/`enthusiast` sur laptop | TDP PL sur matériel qui l'expose ; cTGP NVIDIA réel ; coexistence PPD propriétaire. |
| Charge type | `Admin1` + QML, validation | firmware `charge_type` réel |
| Fn-Lock | `Admin1` + QML + tray | écriture réelle `tuxedo_keyboard/fn_lock` |
| Lighting | `Admin1`, DTO zones, QML | clavier RGB/per-key/lightbar réels |
| Résolution/taux | contrat `User1`, fixtures, QML | sessions X11/Hyprland/Sway/KDE/GNOME réelles |
| Courbe fan | validation Rust/QML, fakes hwmon | application réelle des ventilateurs |
| UVC | lecture allowlist, fixtures ; backend d'écriture et contrat `User1.SetUvcControl` testés | écriture réelle caméra, sliders GUI, device occupé/quarantaine |

---

## 2. Règles de visibilité

### Implémenté

`HardwareData.liveKnown` (`isLive && !stale`) conditionne
`visibleIfAvailable`/`visibleIfPresent` (`HardwareData.qml:125-135`). Un bloc
n'est masqué que sur un snapshot live fiable ; pendant le chargement, le mode
démo ou un payload périmé, il reste visible pour éviter le clignotement et les
faux « absent ».

Les pages appliquent ces règles :

- Dashboard : carte iGPU (`dashboardIgpuCard`), carte dGPU, badge agent, mode
  graphique et historique GPU
  (`DashboardPage.qml:67-83,578-745,980-1049`).
- Réglages : charge, PRIME, capacités/ODM/TDP/cTGP, Fn-Lock, YCbCr
  (`SettingsPage.qml:1653-1950,1994-2315`).
- Outils : éclairage et webcam (`ToolsPage.qml:27-35,610,866`).
- Éditeur : webcam, ODM/TDP/cTGP et contrôles d'affichage
  (`ProfileEditorPage.qml:89-99,786-899,1404-1511`).
- Tray : Fn-Lock/PRIME (`SystemTray.qml:34-39,296-318`).

Preuves QML : `tst_hardware_visibility.qml` (absence confirmée, démo/chargement
conservés, blocs absents), `tst_hardware_data.qml` (bail capteurs et
disponibilité iGPU par métrique), `tst_admin_hw.qml`
(`capabilitiesVisible=false` sans matériel/service, borne cTGP, monotonie TDP),
`tst_info_support.qml` (mode graphique/agent masqués), `tst_real_capabilities.qml`
(listes réelles, aucun mode inventé).

### Exceptions conservées

1. Daemon absent : bandeau global + badge démo, pas une capability matérielle.
2. Agent absent : bandeau explicite, contrôles refresh/résolution masqués.
3. Capteur individuel absent : jauge « non disponible » conservée.
4. Valeur périmée : dernière valeur + avertissement.
5. Admin absent : données lisibles, écritures désactivées.
6. Capability partielle : carte conservée, sous-contrôles non supportés
   masqués (ex. charge sans type inscriptible).
7. Mode démo : badge démo requis ; `DemoData` ne doit jamais alimenter une
   action réelle.
8. Capteur partiel iGPU/dGPU : la jauge et la série de la métrique absente
   restent vides, sans zéro inventé ; les autres métriques continuent.

### Limite de test

`findChild` d'un bloc masqué reste non nul (l'objet existe, `visible=false`).
Les tests vérifient donc les booléens `*Visible` qui pilotent `visible`, ce qui
est le contrat de l'UI. Une assertion directe sur `Item.visible` dépend du
parent visible et n'est pas fiable sous `qmltestrunner` offscreen.

---

## 3. Capabilities disponibles / manquantes

### Disponibles et utilisées

| Capability | Source | Usage |
|---|---|---|
| `device.*`, `cpu.*`, `display.*`, `capabilities.*` | `HardwareSnapshot` → `HardwareView` → `HardwareData` | Dashboard, Info, éditeur, visibilité |
| `sensors.enabled/available`, `sensors.*Available` | idem | Note de collecte, graphes iGPU, jauges par métrique |
| `charging.*Writable`, `chargeTypes`, seuils/listes | idem | Réglages charge + type |
| `lighting.keyboard/lightbar.writable/modes/states` | idem | Outils éclairage |
| `fnLock.supported/enabled/writable` | idem | Réglages + tray Fn-Lock |
| `odm.available/tdpAvailable/platformProfile/tdp` | idem | Éditeur (valeurs par profil), Réglages Admin.Hw, visibilité |
| `nvidia.available/ctgpOffset/maxCtgpOffset` | idem | Éditeur cTGP, borne dynamique |
| `prime.available/supported/mode/requiresOffloading` | idem | Dashboard + Réglages (lecture) |
| `ycbcr.available/ports/pendingModeReapply` | idem | Réglages (lecture) |
| `webcam.switchAvailable/controlsSupported/devices` | idem | Outils/éditeur (lecture UVC) |
| `agent.*` (backend, output, modes, current) | `User1` → `AgentData` | Dashboard, éditeur, Réglages |
| `fan.available/mode/writable/fans[]` (+ `target`/`label`) | `GetFanDataJSON` | Dashboard, jauge GPU |

### Manquantes / incomplètes

| Capability | Conséquence | Source |
|---|---|---|
| `webcam.controlsWritable` réellement lu par QML | forcé `false` dans `HardwareData.qml:430,914` ; sans impact tant que les sliders UVC ne sont pas livrés | `HardwareData.qml` |
| `User1.GetUvcWriteSupported` consommé par la GUI | sliders UVC absents ; le backend et le service existent déjà | `session_service.rs`, `ToolsPage.qml` |
| `webcam` kill-switch writable | état seul, aucun toggle | daemon expose l'état, pas de méthode GUI |
| `prime.writable`/`Admin.Prime` | bascule PRIME absente | `HardwareView::Prime`, aucune UI d'écriture |
| `external mode` par mode | impossible de distinguer un mode externe sous `internalOutput` | contrat `User1` |
| `Admin.Hw` vs valeurs par profil | deux chemins d'écriture ODM/TDP/cTGP, assumés et documentés | `SettingsPage.qml:2035-2315`, `ProfileEditorPage.qml:1404-1511` |

---

## 4. Croisement avec `FEATURE_PARITY.md`

| Item parité | Statut parité | Réalité UI | Écart |
|---|---|---|---|
| Dashboard plusieurs GPU/PRIME | `[~]` | cartes iGPU/dGPU dédiées, mode réel `HardwareData.prime`, lecture seule | à jour |
| dGPU + ventilateur si exposé | `[x]` | carte masquée sans dGPU ; jauge GPU issue de `fans[]`, masquée sans ventilateur GPU | conforme |
| Éditeur taux disponibles | `[~]` | taux réels `AgentData` | à jour, parité en retard |
| Résolution | `[x]` | branchée, DTO/Rust/QML testés | conforme |
| Assignation automatique | `[~]` | matrice PPD×AC/BAT éditable, icônes AC/BAT réelles ; éditeur sans toggles, note vers la matrice | conforme, écart d'UI assumé |
| LED ODM | `[~]` | contrôle retiré, LED non pilotée | **masqué/à implémenter** |
| ODM/TDP/cTGP | `[x]`/`[~]` | ODM/TDP/cTGP par profil via `Admin1`, `Admin.Hw` transitoire ; ODM réel validé, TDP/cTGP non exposés sur ce laptop | conforme UI ; validation matérielle partielle |
| Courbe CPU/GPU | `[x]` | courbe CPU éditable ; GPU non éditable | **partiel GPU** |
| Lighting | `[~]` | luminosité/couleur/mode/lightbar branchés ; par zone non | **partiel** |
| Webcam UVC lecture | `[x]` | contrôles affichés en lecture seule | conforme |
| UVC écriture/préréglages | `[~]` | `User1.SetUvcControl` livré/testé ; sliders GUI en cours ; écriture réelle non testée | **partiel GUI** |
| Support diagnostic/WebFAI/AnyDesk | `[ ]` | cartes masquées, contact réel | **masqué** |
| Info version/changelog | `[~]` | valeurs réelles, changelog statique badgé | conforme |
| Fn-Lock Réglages | `[x]` | interrupteur `Admin1` | conforme |
| Fn-Lock tray | `[x]` | toggle tray relié à `Admin1` | conforme |
| Charge | `[x]` | profil/priorité/seuils/type via `Admin1` | conforme |
| Matrice PPD × AC/BAT | `[x]` | éditable, persistée et annotée dans les profils | conforme |
| État agent session | `[x]` | `User1` réel | conforme |
| Contact/support en ligne | `[ ]` | URL fixe réelle | **écart inverse** |
| Vue détail/compacte profils | `[ ]` | vue liste/compacte existe | **écart inverse** |
| cTGP | `[~]` | éditable par profil et via `Admin.Hw` ; masqué sans NVIDIA | conforme UI, matériel absent |
| PRIME | `[~]` | lecture seule, `Admin.Prime` absent | conforme |
| Tomte/Aquaris | `[ ]` | cartes masquées ; entrée Aquaris sidebar encore stub | **partiel** |

---

## 5. Reste à implémenter, classé par effort/risque

| Élément | État actuel | Effort | Risque | Décision recommandée |
|---|---|---|---|---|
| PRIME / `Admin.Prime` | lecture seule, aucun switch | élevé | **élevé** (root, `/boot`, `prime-select`) | service séparé + polkit, validation matérielle dGPU |
| UVC sliders GUI + écriture réelle | service/backend livrés, GUI lecture seule | moyen | **élevé** (écriture matérielle) | consommer `GetUvcWriteSupported`, sliders bornés, revue `security`, test caméra |
| Préréglages UVC par device | absents | moyen | moyen | persistance par device après stabilisation de l'écriture |
| Kill switch webcam global | état seul | moyen | **élevé** (ioctl/écriture) | méthode protégée, confirmation, restauration |
| Support diagnostic `systeminfos` | masqué | moyen | moyen (collecte/privacité) | collecte locale, suppression, jamais d'upload auto |
| WebFAI | masqué | élevé | **élevé** (USB/root) | allowlist + polkit, hors première version |
| AnyDesk | masqué | moyen | **élevé** (dépôt tiers) | ne pas retenir sans revue sécurité |
| Tomte | masqué | moyen | **élevé** (modules noyau/Secure Boot) | décision produit + service restreint |
| Aquaris | nav stub/carte masquée | élevé | moyen (BLE) | masquer l'entrée nav tant que non implémenté |
| Couleur lighting par zone | partiel | moyen | faible | éditer chaque zone (le DTO accepte déjà N zones) |
| Courbe GPU | partiel | moyen | faible | éditer `tableGPU` (Rust préserve déjà) |
| Mode externe sous `internalOutput` | défense | moyen | moyen | ajouter le connecteur par mode au contrat `User1` ou rejeter les modes incohérents |
| Courbe fan par défaut via `DemoData` | démo | faible | faible | constante Rust ou profil par défaut, retirer `DemoData.fanCurve` |
| YCbCr écriture | read-only | élevé | **élevé** (debugfs) | hors périmètre recommandé |
| ODM LED | masqué | faible | faible | implémenter seulement si le matériel l'expose |
| TDP PL / cTGP réels | UI prête, matériel non exposé | faible | moyen | smoke test sur laptop avec TDP/NVIDIA, sans bloquer la CI |

### Top 10 des restants (priorité risque/parité)

1. **PRIME / `Admin.Prime`** — seul gros bloc de parité avec impact root.
2. **UVC sliders GUI + écriture réelle** — service livré, mais écriture
   matérielle non raccordée à l'UI ; revue sécurité et test caméra requis.
3. **Kill switch webcam global** — état lu mais pas pilotable.
4. **Diagnostic/WebFAI/AnyDesk/Tomte** — backends absents, cartes masquées.
5. **Aquaris** — entrée nav encore visible alors que le backend est absent.
6. **Couleur lighting par zone** — une seule couleur dupliquée sur toutes les zones.
7. **Courbe GPU** — jauge GPU corrigée, courbe GPU non éditable.
8. **Préréglages UVC par device** — dépend de l'écriture GUI.
9. **Mode externe sous `internalOutput`** — défense de contrat `User1`.
10. **Validation TDP PL / cTGP réels** — UI et persistance prêtes, matériel absent.

---

## 6. Recommandations de tests restantes

1. QML : vérifier `visible` réel des cartes absentes dans un parent visible
   (ou conserver les booléens `*Visible`, contrat actuel).
2. QML : fermer = masquer de bout en bout (C7), si le module `Tcc` devient
   chargeable sous `qmltestrunner`.
3. QML : couleur lighting par zone (N zones distinctes).
4. QML : sliders UVC quand `User1.SetUvcControl` sera raccordé
   (`writeSupported`, device occupé, hors plage, quarantaine).
5. Contrat : mode externe sous `internalOutput` (N6), ajouter le connecteur
   par mode ou un schéma qui interdit l'ambiguïté.
6. Matériel : smoke tests ODM/TDP/cTGP (Clevo/Uniwill, NVIDIA), Fn-Lock,
   lighting, charge type, courbe fan, résolution réelle par compositeur,
   écriture UVC réelle.

---

## 7. Preuves exécutées

```sh
crates/tcc-gui/tools/qml-check.sh       # qmllint OK ; 280 passed, 0 failed
cargo test -p tcc-gui --quiet           # 186 passed, 0 failed
scripts/release/validate.sh --repo      # 350 vérifications, 0 échec
```

Résultat déjà consigné pour le périmètre Rust complet :

```sh
cargo test --workspace --quiet          # 1489 tests OK (run UVC)
```

Les tests QML ciblés `tst_hardware_visibility`, `tst_hardware_data`,
`tst_admin_hw`, `tst_hardware_writes`, `tst_real_capabilities`,
`tst_profile_editor`, `tst_tray`, `tst_info_support` et `tst_agent_data`
couvrent respectivement la visibilité, le bail capteurs et l'iGPU, Admin.Hw et
les bornes, charge/Fn-Lock/lighting, résolution/CPU réels, ODM/TDP/cTGP par
profil et barre flottante, le tray compact/Fn-Lock, Info/Support et le service
`User1`.

---

## 8. Vagues UI 2026-11-07

| Vague | Éléments livrés / supprimés | Preuve |
|---|---|---|
| Barre Enregistrer flottante | `editorFloatingBar` ancrée en bas : état (propre/modifié/invalide/erreur), Annuler, Enregistrer, spinner ; espace réservé pour ne pas masquer le dernier champ ; absente en lecture seule. | `ProfileEditorPage.qml:715-716,1518-1590` ; `tst_profile_editor.qml:297-339`. |
| Tray compact | `TrayPanel` sans décoration : profil actif, bascule rapide, état PPD × AC/BAT, Fn-Lock, power blocker, autostart, ouvrir/quitter ; menu natif conservé en secours. | `SystemTray.qml:31-32,133-165,190-212` ; `TrayPanel.qml` ; `tst_tray.qml:258-331`. |
| Icône launcher / app_id | `setDesktopFileName("tuxedo-rust-center")` pour l'association Wayland/.desktop ; icône de fenêtre depuis le thème, repli PNG 256 px ; icônes 32/48/64/128/256 + scalable. | `qapplication_shim.cpp:17-31` ; `packaging/icons/hicolor/**` ; `packaging/desktop/tuxedo-rust-center.desktop:12`. |
| Nettoyage Support/Profils | Bandeau « support partiel » et sous-titre diagnostic retirés ; fonction morte `actionUnavailable()` et message erroné sur l'écriture des profils supprimés. | `SupportPage.qml` ; `ProfilesPage.qml` ; `docs/ui/UI_CLEANUP.md`. |
| Nettoyage Réglages/Outils | Helpers/propriétés morts (`profileNames`, `odmVisible`, `fnLockInitial`, etc.) supprimés ; notes de jalon dépassées (phase 1, charge read-only, webcam, minuterie shell) retirées ; toggles AC/BAT morts remplacés par une note vers la matrice. | `SettingsPage.qml` ; `ToolsPage.qml` ; `ProfileEditorPage.qml:875-883`. |
| Dashboard/Sidebar | Second graphe CPU redondant supprimé ; bouton de collecte remplacé par une note passive ; infobulles Sidebar affirmant à tort que la matrice n'était pas exposée, supprimées ; `AppCheckBox.qml` supprimé (aucune référence). | `DashboardPage.qml:555-565` ; `Sidebar.qml` ; absence de `AppCheckBox.qml`. |
| Capteurs/iGPU | Carte iGPU dédiée, disponibilités par métrique, historiques 48 points/2 s, aucun 0 inventé ; collecte automatique liée à Dashboard + fenêtre visible. | `DashboardPage.qml:71-141,578-738` ; `HardwareData.qml:466-473` ; `tst_hardware_data.qml:409-434,471-500`. |

Preuves de validation de ces vagues : `qmllint` OK, **280 tests QML passés**
(0 échec) et **186 tests Rust `tcc-gui` passés** (0 échec), rejoués le
2026-11-07. Les détails de suppression et les clés i18n orphelines sont dans
`docs/ui/UI_CLEANUP.md` (document non modifié par ce réaudit).
