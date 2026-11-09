# Nettoyage UI — ancien TCC, jalons et doublons

Date : 2026-11-07. Périmètre : pages `Support`, `Info`, `Tools`, `Settings`,
`Profiles`, `Dashboard`, composants QML et tests associés.
Aucun composant web, aucune logique privilégiée, `Strings.qml` non modifié.

---

## 1. Lot 1 — résidus de l'ancien TCC et des jalons

| Élément | Fichier | Décision | Raison |
|---|---|---|---|
| Bandeau « Fonctions de support partielles » (`supportUnavailableNotice`) | `qml/pages/SupportPage.qml` | Supprimé | Annonçait le diagnostic intégré, WebFAI et AnyDesk « sans backend » : note « à venir » pour des services hors périmètre, sans action réelle. La page ne garde que les canaux de contact officiels. |
| Sous-titre « Diagnostic et assistance TUXEDO » | `qml/pages/SupportPage.qml` | Supprimé | Le diagnostic n'est pas implémenté ; le titre « Support » et la carte de contact suffisent, sans promesse trompeuse. |
| Fonction morte `actionUnavailable()` et son message « écriture des profils pas encore branchée » | `qml/pages/ProfilesPage.qml` | Supprimée | Aucun appelant ; le texte était faux depuis l'arrivée de `Admin1.SaveProfilesJSON`. |
| Helpers morts `profileNames()`, `profileIndexById()`, `modelRange()`, `themeLabels()` | `qml/pages/SettingsPage.qml` | Supprimés | Aucune référence dans les pages ni les tests. |
| Propriétés mortes `odmVisible`, `tdpVisible`, `ctgpVisible` | `qml/pages/SettingsPage.qml` | Supprimées | Déclarées mais jamais utilisées par la vue ; `hwPresent`/`capabilitiesVisible` portent la vraie règle. Tests `tst_hardware_visibility.qml` ajustés sur `hwPresent`. |
| Propriété morte `fnLockInitial` | `qml/pages/SettingsPage.qml` | Supprimée | Écrite, jamais lue ; l'état réel relu par le worker fait foi. |
| Suffixe « Seuls ces réglages sont modifiables en phase 1 » | `qml/pages/SettingsPage.qml` | Retiré de l'UI | Faux depuis la charge, la matrice PPD, le Fn-Lock, la minuterie et le power-blocker. La note matérielle utile est conservée. |
| Note « Le type de charge et les setters directs restent en lecture seule » | `qml/pages/SettingsPage.qml` | Retirée | Contredisait le sélecteur de type de charge, éditable quand le firmware l'expose. |
| Note « Enregistré via le service d'administration… setters Manager1 non utilisés » | `qml/pages/ToolsPage.qml` | Retirée | Détail d'implémentation interne, sans valeur utilisateur ; les badges « lecture seule » restent sur les contrôles concernés. |
| Note webcam « écriture UVC non exposée (hors jalon 2) » | `qml/pages/ToolsPage.qml` | Retirée | Référence de jalon obsolète ; l'écriture UVC est en cours côté session (`tccd-user`). Les badges par contrôle restent. |
| Note minuterie « tccd gère le fuseau/DST, la GUI n'exécute pas de shell » | `qml/pages/ToolsPage.qml` | Retirée | Jargon interne ; les états et l'heure locale suffisent. |
| Infobulles « Le daemon n'expose pas encore la matrice d'état » | `qml/components/Sidebar.qml` | Supprimées | Faux depuis l'éditeur de matrice PPD × AC/BAT ; les lignes AC/BAT restent cliquables vers le profil. |
| Second graphe CPU redondant dans la carte Processeur | `qml/pages/DashboardPage.qml` | Supprimé | La carte « Historique » en bas de page affiche déjà CPU + GPU ; un seul graphe par métrique. |
| `AppCheckBox.qml` | `qml/components/AppCheckBox.qml` | Fichier supprimé | Aucune référence QML, test ou ressource (vérifié par `grep` sur `crates/tcc-gui`). La case à cocher standard n'est utilisée nulle part. |
| Commentaires « lecture seule au jalon 1 », « qrc au jalon 6 » | `qml/components/FanCurveChart.qml`, `qml/components/AppIcon.qml` | Mis à jour | Le code est livré ; les commentaires décrivaient un état de jalon dépassé. |

### Conservé avec justification

| Élément | Fichier | Raison |
|---|---|---|
| Carte « Capacités » ODM/TDP/cTGP + consentement PPD | `qml/pages/SettingsPage.qml` | Écritures `Admin.Hw` transitoires réellement implémentées et testées (`tst_admin_hw.qml`, hors périmètre d'écriture de ce lot). Elles restent distinctes des valeurs par profil de l'éditeur ; suppression = régression fonctionnelle et tests hors périmètre. À réévaluer si l'IA cible retire les écritures transitoires. |
| États « non disponible / non supporté / lecture seule » PRIME, dGPU, cTGP, YCbCr, Lighting, Fn-Lock, webcam | pages et composants | Le matériel peut exister sur un autre laptop TUXEDO ; l'état doit rester exposé. Aucun backend inventé. |
| `webcamControlsError` | `qml/pages/ToolsPage.qml` | Référencé par `tst_webcam_controls.qml` (hors périmètre d'écriture) ; conservé comme surface de test. |
| `chargeTypeSelectable` | `qml/pages/SettingsPage.qml` | Référencé par `tst_hardware_writes.qml` (hors périmètre). |
| Badges « Live / Démo » et repli `DemoData` | Dashboard, Profils, Réglages, Info | Mode démo structurel de secours quand le daemon est absent ; badges honnêtes sur la source. |
| Changelog statique et ses marqueurs | `qml/pages/InfoPage.qml` | Information réelle livrée avec l'application ; le badge et la légende « statique » évitent de la confondre avec les versions du daemon. |
| `fans.readOnlyHint` | `qml/pages/DashboardPage.qml` | Exact : le Dashboard observe, le pilotage est porté par le profil. |
| `sensors.leaseNote` | `qml/pages/SettingsPage.qml` | Décrit le bail réel de 10 s ; carte désormais passive. |

---

## 2. Lot 2 — collecte capteurs automatique et carte iGPU

| Élément | Fichier | Décision | Raison |
|---|---|---|---|
| Bouton « Activer/Désactiver la collecte » + `AppState.requestSensorCollection` | `qml/pages/DashboardPage.qml` | Supprimés | La collecte est automatique ; le bail est réarmé tant que le Dashboard est visible. Remplacés par une note passive (`dashboardSensorsNote`, `sensors.hint` / `sensors.pendingHint`). |
| `ToggleRow` « Collecter les capteurs » | `qml/pages/SettingsPage.qml` | Supprimé | Carte `settingsSensorsCard` passive : badge d'état, avertissements expiré/périmé, note de bail. Aucun bouton de rafraîchissement dans cette carte. |
| `InfoRow` iGPU dans la carte dGPU | `qml/pages/DashboardPage.qml` | Supprimé | Doublon remplacé par la carte iGPU dédiée. La ligne « cœurs CPU » reste dans la carte dGPU. |
| Carte iGPU dédiée `dashboardIgpuCard` | `qml/pages/DashboardPage.qml` | Ajoutée | Jauges temp/fréquence/puissance avec `tempAvailable`/`freqAvailable`/`powerAvailable`, historiques locaux (48 points max) et trois graphes natifs. Carte masquée si l'iGPU est absent d'un snapshot fiable. |

Détails d'implémentation :

- `pushSamples()` n'échantillonne une métrique iGPU que si `sensorsLive` et le
  booléen `*Available` correspondant sont vrais ; aucun `0` inventé.
- L'historique iGPU est conservé quand on quitte le Dashboard ou qu'un bail
  expire ; il n'est vidé que sur un snapshot fiable où l'iGPU a disparu.
- Couleurs : température `Theme.gauge`, fréquence `Theme.info`, puissance
  `Theme.accentText`, `showGrid: false`.
- Repli démo : le payload `_demo()` de `HardwareData` n'expose pas encore les
  booléens par métrique ; la page accepte `undefined` **uniquement** en mode
  démo pour garder les valeurs de démonstration. La puissance iGPU n'ayant pas
  de valeur de démo dédiée (`0`), sa jauge reste indisponible en démo plutôt
  que d'afficher un `0` trompeur. À corriger dans `HardwareData.qml`
  (fichier interdit à ce lot).

---

## 3. Clés i18n devenues orphelines (Strings.qml non modifié)

À retirer ou à réutiliser par le propriétaire de `qml/core/Strings.qml` :

| Clé | Origine du retrait |
|---|---|
| `support.partialTitle`, `support.partialBody` | Bandeau Support supprimé |
| `support.subtitle` | Sous-titre « Diagnostic et assistance » retiré (diagnostic non implémenté) |
| `settings.settingsOnlyHint` | Mention « phase 1 » retirée |
| `settings.chargeReadOnly` | Note contredite par le type de charge éditable |
| `tools.lighting.readOnly` | Note d'implémentation retirée |
| `tools.timer.note` | Note d'implémentation retirée |
| `nav.assignmentLiveHint` | Infobulles Sidebar retirées |
| `toast.profileWriteUnavailable` | Fonction morte supprimée |
| `sensors.toggle`, `sensors.toggleHint` | Interrupteur de collecte supprimé |
| `sensors.enable`, `sensors.disable` | Boutons de collecte supprimés |
| `sensors.expiredHint` | Formulation « Réactivez la collecte » devenue fausse |

Toutes ces clés existent en FR/EN/DE. **`tools.webcam.readOnly` a été retiré**
avec l'arrivée des éditeurs UVC : remplacé par `tools.webcam.uvcWriteUnavailable`
(indisponibilité), `tools.webcam.uvcApplying` (attente) et les messages d'erreur
inline (voir `UI_DESIGN.md` §4.5).

Orphelins préexistants repérés (hors retrait de ce lot, non exhaustif, les clés
dynamiques `fan.mode.*`, `odm.mode.*`, `fans.mode.*` et les pluriels
`profiles.*One/Many` étant exclues) : `common.comingSoon`,
`common.notSupported`, `common.optional`, `common.unsavedChanges`,
`common.duplicate`, `common.search`, `common.all`, `nav.main`, `nav.secondary`,
`dash.agent`, `dash.internalPanel`, `dash.cpuLoad`, `settings.readOnlyNote`,
`settings.switchingManualHint`, `settings.graphicsApply`, `settings.odmSource`,
`settings.ctgpCurrent`, `settings.notInstalled`, `settings.connected`,
`tools.lighting.brightness`, `tools.lighting.color`, `tools.lighting.static`,
`tools.lighting.breathing`, `tools.lighting.syncLid`, `tools.lighting.buffer`,
`tools.lighting.state`, `tools.lighting.zonesCount`, `tools.lighting.fix`,
`tools.webcam.brightness`, `tools.webcam.contrast`, `tools.webcam.exposure`,
`tools.webcam.preset`, `state.ok`, `state.warning`, `state.error`,
`state.info`, `toast.language`, `toast.diagnostics`.

---

## 4. Nettoyages différés — traités après le lot

| Fichier | Action | État |
|---|---|---|
| `qml/core/Strings.qml` | Clés orphelines du §3 retirées (42 lignes FR/EN/DE) ; `editor.assignmentMatrixHint` ajouté ; plus de texte « Réactivez la collecte ». | **fait** |
| `qml/core/HardwareData.qml` | `tempAvailable`/`freqAvailable`/`powerAvailable` ajoutés au payload `_demo()` ; repli `usingDemo` supprimé du Dashboard ; disponibilité par métrique utilisée aussi pour le dGPU. | **fait** |
| `qml/pages/ProfileEditorPage.qml` | Commentaires « phase 1 » corrigés ; toggles AC/BAT morts supprimés, remplacés par une note vers la matrice d'état. | **fait** |
| `docs/ui/UI_DESIGN.md` | Ligne `AppCheckBox` retirée ; bail capteurs automatique et graphes iGPU documentés. | **fait** |
| `tests/qml/tst_design_system.qml` | Commentaire « jalon 3 » actualisé. | **fait** |
| `qml/Main.qml` | Commentaire « Jalon 1/3 » actualisé. | **fait** |
| `tools/webcam` | `tools.webcam.readOnly` retiré ; remplacé par la note `uvcWriteUnavailable`, l'état `uvcApplying` et les erreurs inline des éditeurs UVC. | **fait** |

---

## 5. Vérifications exécutées

| Commande | Résultat |
|---|---|
| `crates/tcc-gui/tools/qml-check.sh` | `qmllint` OK ; **280 tests QML passés**, 0 échec (273 avant le lot ; +3 tests ajoutés ici pour l'iGPU, les disponibilités par métrique et la carte capteurs passive ; le reste de l'évolution vient des modifications parallèles). |
| `cargo test -p tcc-gui` | **186 tests Rust passés**, 0 échec. |
| `cargo clippy -p tcc-gui --all-targets -- -D warnings` | OK. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Échec **hors périmètre** dans le code de test de `tcc-user` (collision `runtime`/`runtime()` vers la ligne 636), non modifié par ce lot. `tcc-gui` compile et passe clippy. |
| `cargo fmt -p tcc-gui -- --check` | OK. |
| `cargo fmt --all -- --check` | Échec **hors périmètre** dans `tcc-user` (réordonnancement d'imports `uvc` et `session_service.rs` modifiés en parallèle). Aucun fichier Rust de ce lot n'est concerné. |

Limites : pas de validation sur laptop TUXEDO réel dans ce lot. `Strings.qml`
et `ProfileEditorPage.qml` ont été modifiés en parallèle par un autre agent
pendant le lot ; les clés orphelines ont été revérifiées après ces changements.
