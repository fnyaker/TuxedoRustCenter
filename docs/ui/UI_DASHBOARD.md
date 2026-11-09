# Dashboard, Réglages et autostart — lot UX

Date : 2026-11-07. Périmètre : `qml/pages/DashboardPage.qml`,
`qml/pages/SettingsPage.qml`, `qml/Main.qml`, `qml/core/BridgeState.qml`
(nouveau), tests QML associés. Aucun composant web, aucune logique privilégiée,
`Strings.qml` non modifié. Ce lot ne touche pas à `docs/ui/UI_DESIGN.md`
(réservé à un autre agent) ni aux pages Profils/Éditeur.

---

## 1. Dashboard — suppression des cartes redondantes

| Carte supprimée | Contenu retiré | Où l'information reste |
|---|---|---|
| « État du système » (`dash.systemState`) | PPD, alimentation, PRIME, backend/output/mode agent, refresh rate, gouverneur | Badges de la carte du profil actif, carte Processeur/GPU, sidebar, page Réglages |
| « Disponibilité des capteurs » (`dash.sensors`) | Lignes CPU/iGPU/dGPU/ventilateurs GPU et CPU | Jauges et états « non disponible » des cartes matérielles |
| « Historique » (`dash.history`) | Deux graphes CPU/dGPU de 60 s | Graphes intégrés aux cartes Processeur et GPU dédié |

Le Dashboard ne duplique plus les diagnostics d'agent (`display.backend`,
`display.output`, `display.current`) : ils restent dans Réglages → Affichage.

## 2. Graphes déplacés

| Graphe | Emplacement | `objectName` | Visibilité |
|---|---|---|---|
| Température CPU | Carte **Processeur**, sous les jauges et la ligne ventilateurs | `dashboardCpuChart` (+ valeur `dashboardCpuChartValue`, bloc `dashboardCpuChartBlock`) | `cpuChartVisible` : démo, ou live avec collecte active et CPU disponible |
| Température dGPU | Carte **GPU dédié**, sous les jauges | `dashboardDgpuChart` (+ valeur `dashboardDgpuChartValue`, bloc `dashboardDgpuChartBlock`) | `dgpuChartVisible` : dGPU présent et collecte utilisable |
| Température/fréquence/puissance iGPU | Carte **GPU intégré**, inchangé | `dashboardIgpuTempChart`, `dashboardIgpuFreqChart`, `dashboardIgpuPowerChart` | Disponibilité par métrique, inchangée |

Chaque graphe conserve sa légende (pastille + libellé) et la valeur courante
(format °C/°F selon les réglages). Aucune carte vide, aucun graphe orphelin.
La ligne « cœurs CPU » de la carte dGPU, doublon de l'en-tête Processeur, a été
retirée avec le séparateur devenu inutile.

## 3. Réglages — suppression des textes techniques

- Carte « Collecte des capteurs » (`settingsSensorsCard`) supprimée avec son
  badge, ses états expiré/en attente/périmé et sa note de bail. La collecte est
  automatique et ses états utiles restent sur le Dashboard.
- Notes permanentes supprimées : `settings.autostartNote` (écriture atomique,
  shell), `settings.graphicsNote` (action de redémarrage inexistante),
  `settings.capabilitiesNote` (doublon du badge « non persisté »),
  `settings.switchingNote`, `settings.powerBlockerNote`.
- Les deux dernières, réellement utiles (PPD propriétaire, durée de vie de
  l'inhibiteur), sont désormais en **infobulle** sur le badge de la matrice et
  sur la ligne « Bloquer la mise en veille ».
- `settings.hardwareNote` (courte, utile) est conservée.

## 4. Autostart — état initial du pont

**Cause** : `DaemonClient::default()` résout l'autostart XDG avant le chargement
du QML. `Main.qml` ne poussait les valeurs que dans des handlers
`*Changed` ; aucune modification n'ayant lieu, `AutostartData.available` restait
`false` avec `error=""`, et l'UI affichait le repli « Répertoire utilisateur
introuvable ».

**Correctif** :

- nouveau singleton `qml/core/BridgeState.qml` (`pushInitial(daemon)`) qui pousse
  explicitement l'état initial de `AutostartData`, `PowerBlockerData`,
  `AdminState` et `AgentData` (propriétés calculées tôt ou état du worker) ;
- `Main.qml` l'appelle en tête de `Component.onCompleted`, avant toute lecture
  des pages ;
- `SettingsPage` réserve « Répertoire utilisateur introuvable » au code
  `NoHome` ; `available=false` + `error=""` affiche « Indisponible » ;
- `available=true` + `error=""` n'affiche aucun message et laisse le toggle
  actionnable.

## 5. Vérifications

| Commande | Résultat |
|---|---|
| `crates/tcc-gui/tools/qml-check.sh` | `qmllint` OK ; **320 tests QML passés**, 0 échec |
| `cargo test -p tcc-gui` | **192 tests Rust passés**, 0 échec |
| `cargo clippy -p tcc-gui --all-targets -- -D warnings` | OK |
| `cargo fmt --all -- --check` | OK |

Tests ajoutés/mis à jour : `tst_autostart` (poussée initiale sans signal,
`NoHome` seul, absence de faux message), `tst_settings_page` (carte capteurs
absente, notes techniques non permanentes), `tst_info_support` (cartes
redondantes absentes, graphes CPU/dGPU dans leurs cartes, absence de doublon),
`tst_hardware_visibility` (graphe CPU conservé sans GPU, graphe dGPU masqué),
`tst_navigation` (cibles inter-pages stables).

## 6. Clés i18n devenues orphelines (Strings.qml non modifié)

À retirer ou réutiliser par le propriétaire de `qml/core/Strings.qml` :

`dash.history`, `dash.historyCpu`, `dash.historyGpu`, `dash.systemState`,
`dash.sensors`, `dash.graphMode`, `dash.refreshRate`, `dash.governor`,
`sensors.title`, `sensors.state`, `sensors.leaseNote`,
`settings.graphicsNote`, `settings.capabilitiesNote`, `settings.autostartNote`.

Orphelins préexistants confirmés (hors lot) : `dash.agent`,
`dash.internalPanel`, `dash.cpuLoad`, `settings.readOnlyNote`,
`settings.switchingManualHint`, `settings.graphicsApply`, `settings.odmSource`,
`settings.ctgpCurrent`, `settings.notInstalled`, `settings.connected`.
`settings.switchingNote` et `settings.powerBlockerNote` restent référencées
(infobulles) et ne sont donc pas orphelines.

## 7. Limites

- Pas de validation sur laptop TUXEDO réel ; les tests QML utilisent des
  payloads `Manager1` et un faux pont `daemon` pour la poussée initiale.
- `Main.qml` n'est pas instanciable par `qmltestrunner` (type C++ `Tcc`) : la
  logique de poussée est testée via `BridgeState.pushInitial`, appelée au même
  endroit dans `Main.qml`.
- Les captures d'écran de référence n'ont pas été régénérées dans ce lot ; le
  rendu est vérifié par `qmllint` et les tests d'arborescence.
