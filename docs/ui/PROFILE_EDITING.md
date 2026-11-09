# Édition et sauvegarde des profils — design UI (phases 1 et 2)

Statut : **implémenté (phases 1 et 2)** — édition/enregistrement via
`Admin1.SaveProfilesJSON` ; ODM/TDP/cTGP via `Admin.Hw` (§14). Voir §10 pour
l'état réel et les limites.
Périmètre : `crates/tcc-gui/**`, sans nouvelle logique privilégiée.
Références : `docs/security/ADMIN_DESIGN.md`, `docs/DBUS_API.md`,
`docs/ui/UI_DESIGN.md`, `docs/FEATURE_PARITY.md`.

---

## 1. Objectif

Rendre l'éditeur de profil et les réglages modifiables, puis persister via
`tccd-admin` (phases 1 et 2) :

- `Admin1.SaveProfilesJSON(ProfileCollection)` ;
- `Admin1.SaveSettingsJSON(Settings)` ;
- rechargement de `tccd` par `Control1.ReloadConfig` (déclenché par l'admin,
  jamais par la GUI) ;
- rafraîchissement de l'UI depuis `GetProfilesJSON`/`GetStateJSON` après succès ;
- réglages globaux ODM/TDP/cTGP via `Admin.Hw` (transitoires, §14).

La GUI ne doit jamais écrire `/etc` ni `/var/lib`, ni appeler `pkexec`.

## 2. Flux utilisateur

```
Édition locale (modèle QML/Rust)
        │
        ├─ validation locale (types, bornes, noms uniques, IDs)
        │
        ├─ « Enregistrer » ──► Admin1.SaveProfilesJSON (ou SaveSettingsJSON)
        │                         │
        │                         ├─ refus polkit ──► message, formulaire conservé
        │                         ├─ erreur typée ──► message par champ
        │                         └─ succès ──► relecture GetProfilesJSON
        │                                            + StateChanged / refresh
        └─ « Annuler » ──► restauration du dernier modèle live
```

- Un seul enregistrement à la fois : bouton désactivé, indicateur « enregistrement ».
- Le dialogue polkit est présenté par le système ; la GUI ne doit pas tenter de
  le contourner ni de réessayer en boucle.
- Après succès, l'UI affiche un bandeau court et met à jour le profil actif si
  `StateChanged`/`GetStateJSON` a changé.
- Après refus ou erreur, aucune modification n'est perdue dans le formulaire.

## 3. Champs éditables en phase 1

| Section | Champs | Validation locale |
|---|---|---|
| Général | nom, description | nom non vide, ≤ **128** (`MAX_PROFILE_NAME_LEN`), unicité (casse/espaces normalisés) |
| Affichage | luminosité + `useBrightness` | 0–100 |
| Affichage | refresh rate + `useRefRate` | taux disponible pour le connecteur interne |
| Affichage | résolution + `useResolution` | mode `WxH` réel exposé par `User1` |
| CPU | cœurs en ligne, governor max, min/max kHz, EPP, `noTurbo` | max = `cpu.totalCores` réel, EPP ∈ `cpu.availableEpp`, cohérence min ≤ max |
| Webcam | `status` + `useStatus` | booléen |
| Ventilateurs | `useControl`, profil, min/max/offset, courbes | min ≤ max, points triés, plage 0–100 |
| ODM/TDP/NVIDIA | `odmProfile.name`, `odmPowerLimits.tdpValues`, `nvidiaPowerCTRLProfile.cTGPOffset` | ODM ≤64 `[A-Za-z0-9_-]`, TDP 1–3 valeurs 1–1000 strictement croissantes, cTGP 0–`ctgpMax`/200 |

Les champs dont le matériel n'est pas disponible restent affichés désactivés
avec un badge « non disponible » (pas de valeur inventée). Les réglages
ODM/TDP/cTGP sont **portés par le profil** (voie normale) ; la carte
`Admin.Hw` de Réglages reste une application immédiate transitoire.

## 4. Modèle et validation

- La GUI conserve un `ProfileCollection` typé (DTO Rust, pas de `JSON.parse`
  libre côté QML).
- Validation locale avec les mêmes règles que `tcc-core` lorsque c'est possible
  (longueurs, bornes, IDs `^[A-Za-z0-9_-]{1,64}$`, noms uniques).
- Le daemon reste l'autorité : toute erreur `Admin1.Error.Validation` est
  affichée par champ si le message le permet, sinon en bandeau.
- Les presets ne sont jamais envoyés comme personnalisés : l'admin les filtre,
  mais la GUI doit les marquer en lecture seule.

## 5. Erreurs à afficher

| Erreur `Admin1.Error.*` | Message UI |
|---|---|
| `NotAuthorized` | « Autorisation refusée ou annulée » |
| `PolicyKitUnavailable` | Code de compatibilité ascendante : l'admin actuel replie ce cas sur `NotAuthorized`. La GUI le distingue s'il apparaît un jour. |
| `Busy` | « Une autre opération est en cours, réessayez » |
| `PayloadTooLarge` | « Données trop volumineuses » (ne devrait pas arriver) |
| `Validation` | message du champ concerné |
| `InUse` | « Ce profil est utilisé par un état » |
| `ReloadFailed` | « Écriture effectuée mais rechargement du daemon échoué » : le fichier précédent est restauré lorsqu'il est exploitable, sinon le nouveau est conservé. L'UI invite à vérifier l'état avant de réessayer. |
| `WriteFailed` | « Écriture impossible ; aucune modification appliquée » |
| `PolicyKitUnavailable` | « Service d'autorisation indisponible » |

Aucun chemin absolu, contenu de fichier ou secret ne doit apparaître.

## 6. Concurrence et cohérence

- Si `StateChanged` arrive pendant l'édition : ne pas écraser le formulaire,
  afficher un bandeau « le profil actif a changé » avec « Recharger ».
- Si un autre client enregistre : la relecture post-succès détecte un contenu
  différent ; proposer « Recharger » plutôt que de fusionner silencieusement.
- Les IDs supprimés ne sont pas réutilisés (règle à trancher côté `tcc-core`).
- La matrice d'état n'est pas encore exposée par `tccd` : elle reste en lecture
  seule dans les réglages jusqu'à l'ajout du contrat.

## 7. Sécurité

- Aucun appel direct à `tccd-admin` sans passer par le proxy `Admin1`.
- Aucun `pkexec`, aucun shell, aucun chemin fourni par l'utilisateur.
- Taille du payload vérifiée côté GUI (≤ 1 Mio) avant envoi.
- L'interface `Admin1` n'est pas exposée aux utilisateurs non autorisés par la
  policy D-Bus ; la GUI doit gérer proprement l'absence du service
  (« service d'administration non installé »).
- Aucune écriture tant que `tccd-admin` n'est pas déclaré validé par
  `packaging-release` (`TCC_WITH_ADMIN=1`).

## 8. Tests attendus

- QML/Rust : formulaire invalide bloqué, valeurs aux bornes acceptées,
  pastilles « non disponible », conservation des modifications après refus,
  bandeau après erreur `ReloadFailed`, dialogue « profil actif modifié ».
- Pont Rust : sérialisation bornée, rejet des réponses > 1 Mio, mapping des
  erreurs `Admin1.Error.*`, busy state, un seul appel concurrent.
- E2E : faux service `Admin1` sur bus de session, succès + relecture,
  `NotAuthorized`, `Busy`, `ReloadFailed`.
- Non testable localement : dialogue polkit réel, activation D-Bus système,
  rollback admin sur bus système (voir `docs/security/REVIEW_ADMIN_PHASE1.md`).

## 9. Hors périmètre de l'éditeur de profil

- TDP/ODM/cTGP en écriture : implémentés en phase 2, mais **hors du profil**,
  dans Réglages → ODM/TDP/cTGP (`Admin.Hw`, §14).
- PRIME en écriture (admin phase 2, pas dans l'éditeur).
- Tomte/WebFAI (phase 3).
- Matrice PPD × alimentation éditable (contrat `tccd` à ajouter).
- Contrôles UVC webcam.
- Import/export de profils vers un fichier arbitraire.


---

## 10. Implémentation (phase 1)

### 10.1 Détection du service

- Le worker unique interroge `Admin1` au démarrage puis toutes les **15 s**.
- `org.freedesktop.DBus.NameHasOwner` et `ListActivatableNames` sont utilisés
  **sans activer** le service : un `Ping` toutes les 2 s lancerait
  `tccd-admin` (root) en permanence, contrairement à AD-1.
- Si le nom est possédé : `Ping` + `GetContractVersion` (2 s de timeout) +
  `GetVersion`, version comparée à `ADMIN_CONTRACT_VERSION`.
- Si le nom est seulement activable : service considéré comme installé
  (l'enregistrement l'activera), version inconnue.
- Sinon : `adminAvailable = false`, bandeau « service non installé/activé »,
  formulaire en lecture seule, bouton Enregistrer désactivé.
- `refreshAdmin()` force une nouvelle détection (bouton « Réessayer »).

### 10.2 DTO Rust et validation

- QML produit un **DTO d'édition borné** (`ProfileRules.toEdit`, 64 Kio max)
  avec les seuls champs autorisés ; Rust le désérialise dans `ProfileEdit`
  (`deny_unknown_fields`) puis l'applique sur la dernière `ProfileCollection`
  **validée** par `tcc-core`.
- `customFanCurve` est optionnel (`null` = ne pas changer) ; seule la table
  **CPU** est fournie. Rust applique les points sur la courbe existante en
  conservant `name` et `tableGPU`, puis revalide la collection.
- Champs ODM ajoutés au DTO : `odmProfile` (`Option<String>`), `tdpValues`
  (`Option<Vec<f64>>`, 0–3 valeurs, 1–1000 W, strictement croissantes),
  `ctgpOffset` (`Option<f64>`, 0–200). `None` = ne pas changer ; `[]` efface
  les limites TDP. Ils sont appliqués au `Profile` puis revalidés par
  `tcc-core` et `Admin1`.
- La collection complète est revalidée et sérialisée ; borne **1 Mio** avant
  `SaveProfilesJSON`.
- Presets : refus local (`PresetReadOnly`) et UI en lecture seule ; jamais
  envoyés comme personnalisés.
- Aucune correction silencieuse : une valeur invalide renvoie un champ nommé.

### 10.3 Champs éditables

Nom, description, luminosité + `useBrightness`, refresh + `useRefRate`, cœurs,
`useMaxPerfGov`, fréquences min/max (kHz), EPP, `noTurbo`, webcam
`status`/`useStatus`, ventilateurs `useControl`/profil/min/max/offset,
**courbe CPU personnalisée** (mode `custom` uniquement), et ODM/TDP/cTGP :
profil ODM, limites TDP (si exposées), offset cTGP (si NVIDIA).

Capacités réelles utilisées par l'éditeur :

- **Cœurs** : maximum = `cpu.totalCores` du snapshot ; si la topologie n'est
  pas exposée, la valeur du profil sert de borne (jamais `DemoData`).
- **EPP** : liste = `cpu.availableEpp`. La valeur déjà présente dans le profil
  est conservée même si elle a disparu de la liste. Liste vide → une seule
  entrée « Auto » (aucune liste inventée), champ désactivé. Le champ est
  modélisé par **valeur** (`eppValue`) et non par index partagé : changer de
  profil ne peut pas réutiliser l'EPP du précédent, et le DTO ne contient que
  la valeur du profil édité. L'infobulle (icône d'aide, aussi accessible au
  clavier) rappelle que la valeur est enregistrée par profil et que
  `power-profiles-daemon` reste propriétaire de l'EPP effectif (D4) : elle est
  conservée mais peut ne pas être appliquée tant que PPD le pilote.
- **Gouverneurs** : `cpu.availableGovernors` conditionne le toggle
  « gouverneur performance ». Liste vide = capacité non exposée, toggle
  laissé utilisable ; aucune liste de gouverneurs n'est affichée.
- **Écran interne** : `display.hasInternalDisplay` / `internalConnector`
  conditionnent les contrôles ; les résolutions et taux réels viennent de
  `AgentData` (`User1`, bus de session).
- **Résolution** : liste des `WxH` uniques de `internalOutput.modes` ; la
  valeur courante du profil est sélectionnée si elle existe. Un mode enregistré
  disparu rend le formulaire invalide jusqu'au choix d'un mode réel.
- **Refresh** : liste des taux de la résolution choisie, triés décroissants ;
  aucune combinaison absente n'est proposée.
- **Agent indisponible/non supporté** : contrôles désactivés, valeurs du profil
  préservées, bandeau + « Actualiser l'agent ».

La courbe accepte 2 à 256 points `{temp, speed}`, températures 0–100 °C
strictement croissantes, vitesses 0–100 %. Elle est validée localement
(`fanCurve`) puis par Rust (mêmes bornes). Dans le dialogue, l'aperçu
`FanCurveChart` est éditable : glisser un point (clamp entre voisins à 1 °C
d'écart), double-clic pour ajouter, clic droit ou Suppr pour retirer (minimum
deux points). Le tableau de champs et l'aperçu partagent les mêmes lignes via
`previewRows` ; toute interaction repasse par `recompute()` et la validation.
Le nom de la courbe et la table GPU existants ne sont **jamais** modifiés par
la GUI ; le GPU n'est pas éditable.

Sections **masquées** (pas grisées) quand le matériel est absent sur un
snapshot live fiable : webcam (aucune caméra/switch), carte ODM/TDP/cTGP.
Pendant le chargement, la démo ou un payload périmé (`stale`), les sections
restent visibles avec leur état habituel (pas de flicker).

La carte ODM/TDP/cTGP de l'éditeur contient les champs du profil :
sélecteur ODM (« Ne pas changer » + choix `platform`/`io`), champs TDP
seulement si le firmware expose des limites, offset cTGP seulement si NVIDIA
est présent. La source (`platform_profile`/`tuxedo-io`) est affichée et PPD
est signalé. Le toggle « LED du bouton ODM » reste retiré (aucun pilotage LED
exposé).

Lecture seule : assignation AC/BAT (réglages), résolution, PRIME,
création/suppression/import/export.

### 10.4 Flux d'enregistrement

1. `AppState.requestProfileSave(id, dtoJson)` → `DaemonClient.saveProfile`.
2. Refus immédiat si admin absent / identifiant invalide / DTO trop grand ;
   un seul enregistrement à la fois (`saving`).
3. Le worker relit **d'abord** `GetProfilesJSON` et compare à la collection
   utilisée pour l'édition ; toute différence annule l'envoi (`Conflict`).
   Il applique ensuite l'édition, sérialise, appelle `SaveProfilesJSON`
   (timeout **240 s** : polkit 120 s + reload 30 s + rollback/reload 30 s +
   marge), puis relit `GetProfilesJSON`/`GetStateJSON`.
   La file de commandes n'est **pas fusionnée** : chaque commande est traitée
   dans l'ordre, un `SaveProfile` ne peut pas être jeté derrière un `Refresh`.
4. Publication : nouvelles données puis `saveSucceeded`/`saveError`.
5. Succès : rechargement du formulaire + toast ; échec : formulaire conservé,
   bandeau avec message traduit (détail = champ ou identifiant, jamais un
   chemin).

### 10.5 Erreurs traduites

`NotAuthorized`, `PolicyKitUnavailable` (compatibilité), `Conflict`, `Busy`, `PayloadTooLarge`,
`InvalidPayload`/`InvalidArgs`, `Validation`, `InUse`, `ReloadFailed`,
`WriteFailed`, `RateLimited`, `NotSupported`, plus les codes client
`AdminUnavailable`, `Timeout`, `Transport`, `NoData`, `NotFound`,
`PresetReadOnly`, `Unknown`.

### 10.6 Concurrence

- Si les profils live changent pendant une édition (`brouillon`), le
  formulaire n'est pas écrasé : bandeau « le profil a changé » + « Recharger ».
- Un changement de profil actif pendant l'édition déclenche le même bandeau.
- Pendant `saving`, les mises à jour reçues sont ignorées ; le succès recharge
  explicitement.
- Un second `SaveProfilesJSON` concurrent est refusé (`Busy`), côté GUI
  (`saving`) et côté admin (write gate).
- **Conflit multi-clients** : le bandeau de conflit désactive « Enregistrer »
  et le worker relit la collection juste avant l'écriture (`Conflict` si elle a
  changé). Il reste une **course étroite** entre cette relecture et l'écriture :
  `Admin1` n'expose pas de révision/etag, donc un écrivain tiers peut encore
  être écrasé sur une fenêtre de quelques millisecondes. Un futur contrat devra
  ajouter un CAS (`If-Match`/`revision`).
- **Champs `Option`** (`onlineCores`, `scalingMinFrequency`,
  `scalingMaxFrequency`) : `null` dans le DTO signifie « ne pas modifier » ;
  `-1` pour la fréquence max signifie « réduit ». Un renommage ne bride donc
  jamais le CPU et n'invente aucune fréquence.
- **Timeout client** : un `Timeout` (240 s) n'annule pas l'opération admin ; la
  GUI relit l'état et n'envoie pas de nouvelle tentative automatique.

---

## 11. Réglages globaux (phase 1)

### 11.1 Champs modifiables

| Champ (`Settings`) | UI |
|---|---|
| `fahrenheit` | segmented °C/°F (Apparence) |
| `cpu_settings_enabled` | interrupteur « Contrôle CPU » |
| `fan_control_enabled` | interrupteur « Contrôle ventilateurs » |
| `keyboard_backlight_control_enabled` | interrupteur « Contrôle éclairage » |
| `charging_profile` | liste « Profil de charge » (`charging.profiles`) |
| `charging_priority` | liste « Priorité de charge » (`charging.priorities`) |
| `charging_start_threshold` | liste « Seuil de début » (`charging.startThresholds`) |
| `charging_end_threshold` | liste « Seuil de fin » (`charging.endThresholds`) |
| `charging_type` | liste « Type de charge » (`chargeTypes` ou ABI standard) |
| `fn_lock_status` | interrupteur Fn-Lock (si `writable`/`enabledKnown`) |
| `keyboard_states` | Outils → Éclairage clavier (zones, luminosité, RVB, mode) |
| `lightbar_states` | Outils → Éclairage lightbar (luminosité, RVB) |
| `state_matrix` | tableau 3×2 PPD × AC/BAT (fallback AC/BAT par cellule vide) |
| `shutdown_time` | minuterie d'extinction `HH:MM` + `clearShutdownTime` |
| `ppd_override` | interrupteur « Autoriser TCC à écrire platform_profile » (requis pour « Forcer » ODM) |

Tout le reste — `stateMap`, PRIME, YCbCr, `extra` — est **préservé tel quel**
et affiché en lecture seule. Les champs de charge, Fn-Lock et éclairage sont
optionnels : absent/`null` = ne pas changer. ODM/TDP/cTGP
ne passent pas par `Settings` : voir §14.

### 11.2 Flux

1. Le worker relit `GetSettingsJSON` à chaque cycle → `tcc_core::Settings`
   désérialisé et re-sérialisé (JSON canonique, champs inconnus conservés).
   Le pont publie aussi des champs typés (quatre booléens + profil/priorité/
   seuils de charge) : QML ne parse pas de JSON.
2. L'UI envoie un **DTO borné** (64 Kio, `deny_unknown_fields`) via
   `AppState.requestSettingsSave` : 4 booléens obligatoires, 4 champs de
   charge optionnels, `stateMatrix` optionnel,
   `shutdownTime`/`clearShutdownTime` optionnels et `ppdOverride` optionnel.
   Un champ `null` ou absent signifie « ne pas changer » ; il n'y a jamais
   d'effacement silencieux.
3. Le worker relit `GetSettingsJSON` juste avant l'écriture et compare à la
   base d'édition ; toute différence annule l'envoi (`Conflict`, même logique
   que les profils). La course résiduelle (pas de révision dans `Admin1`) reste
   documentée.
3bis. Les valeurs de charge sont validées dans Rust contre le **dernier
   instantané matériel** du worker (`charging.profiles/priorities/
   startThresholds/endThresholds/chargeTypes`), jamais contre des listes
   fournies par QML : profil/priorité hors liste, seuil hors liste, seuil hors
   0–100, `début ≥ fin`, type hors liste (ou hors ABI standard si la liste est
   vide) ou `chargeTypeWritable=false` → `Validation` (nom de champ sûr),
   aucune écriture.
3ter. `stateMatrix`, quand il est fourni, **remplace** la matrice complète (une
   liste partielle est acceptée, une liste vide efface la matrice → fallback
   daemon). Validation Rust : PPD ∈ {power-saver, balanced, performance},
   power ∈ {ac, battery}, clé unique, `profileId` syntaxiquement valide et
   présent dans la **collection live** du worker (`last_profiles`) ; sinon
   `Validation` avec la clé de cellule (`stateMatrix[balanced/ac]`).
3ter. `keyboardStates`/`lightbarStates` : 0–64 zones, luminosité `u8`,
   canaux `-1` (non supporté) ou `0..=255`, mode `static`/`breathing` ; un
   mode inconnu ou un canal hors bornes → `Validation`. `keyboardStates: []`
   efface, `lightbarStates: null` efface explicitement ; absent = préserve.
   `fnLockStatus` est un booléen optionnel.
3quater. `shutdownTime`, quand il est fourni, doit être `HH:MM` sur 24 h
   (`00:00`–`23:59`, deux chiffres par champ) ; il est normalisé puis écrit.
   `clearShutdownTime: true` efface l'heure et est **exclusif** avec un
   `shutdownTime` non nul (les deux ensemble → `Validation`). `null`/absent ne
   change rien. Le daemon applique via logind et publie l'état réel par
   `Manager1.GetShutdownTimerJSON` (`enabled`, `scheduledAt`, `source`,
   `error`), relu à chaque cycle par la GUI.
4. `Admin1.SaveSettingsJSON` (timeout 240 s), puis relecture
   `GetSettingsJSON`/`GetStateJSON`. Payload sérialisé borné à 1 Mio.
5. Un seul enregistrement à la fois (`savingSettings`), `SaveOutcome` toujours
   émis (succès, refus, conflit, timeout, file pleine) : les interrupteurs ne
   restent jamais bloqués.

### 11.3 UI et erreurs

- Interrupteurs désactivés si `Admin1` est absent ou pendant un enregistrement ;
  bandeau « service d'administration indisponible », « enregistrement… »,
  erreur traduite et « conflit » avec « Recharger ».
- Après refus, les interrupteurs reviennent aux valeurs réellement lues (pas
  d'état optimiste durable) ; le message d'erreur est conservé. Un brouillon de
  charge est en revanche **conservé** pour permettre la correction, avec le
  bandeau d'erreur et, pour `Conflict`, « Recharger ».
- Charge : le brouillon local n'est envoyé que par « Appliquer » ; il compare
  chaque champ à sa valeur initiale et n'envoie que les champs modifiés.
  « Réinitialiser » revient à la valeur lue. Les listes proviennent du dernier
  instantané matériel ; sans instantané réel (`HardwareData.isLive`, non
  périmé) ou sans `Admin1`, l'édition est désactivée.
- Matrice : brouillon 3×2 (PPD × AC/BAT), cellule vide = « Fallback AC/BAT ».
  « Appliquer » n'est actif que si au moins une cellule a changé et qu'aucune
  cellule ne référence un profil disparu ; « Réinitialiser » revient à la
  matrice relue. Après refus, le brouillon est conservé ; après succès, la
  matrice persistée est relue.
- Minuterie : brouillon interrupteur + heure/minute ; « Appliquer » n'est actif
  que si le brouillon diffère de `Settings.shutdownTime`. « Annuler la
  planification » envoie `clearShutdownTime: true`. Le statut affiché vient de
  `GetShutdownTimerJSON` (planifié/annulé/indisponible + erreur générique) ;
  l'édition est désactivée si le statut est indisponible, sans admin, ou
  pendant un enregistrement. Après refus, le brouillon est conservé ; après
  succès, `shutdownTime` est relu et le statut se rafraîchit au cycle suivant.
- `fahrenheit` est appliqué à `Strings.fahrenheit` **après relecture** du
  daemon, pas seulement localement (les unités de toute l'UI suivent).
- Un `Timeout` n'annule pas l'écriture côté admin : l'UI relit l'état et ne
  réessaie jamais automatiquement.
- **Réglages invalides** : `GetSettingsJSON` est validé par
  `tcc_core::Settings::validate()` (schéma, IDs, matrice, charge) et les
  références `stateMap`/`stateMatrix` sont vérifiées contre la collection
  relue. Un payload incohérent met la page en **lecture seule** avec un
  bandeau (`settings.error`), sans jamais servir de base d'édition.
- **Arrêt du worker** : un surveillant unique (pas un thread par appel)
  détecte la fin du worker et remet `saving`/`savingSettings` à faux sous
  ~250 ms, marque la connexion perdue et affiche l'erreur — l'UI ne reste pas
  bloquée jusqu'au garde-fou de 300 s.

### 11.4 Limites

- PRIME et YCbCr restent en lecture seule. ODM, TDP et cTGP sont des écritures
  transitoires via `Admin.Hw` (§14), jamais persistées dans `Settings` ni dans
  un profil.
- **Charge** : profil, priorité, seuils et type sont modifiables via
  `Admin1.SaveSettingsJSON`. Le type dépend du firmware : la liste backend ou
  les valeurs ABI standard sont proposées, et le daemon refuse un type inconnu.
  Les setters historiques `Manager1` restent hors UI.
- Les listes de charge dépendent du matériel : si le firmware n'expose pas
  `thresholdsAvailable` ou les listes, les contrôles correspondants sont
  masqués/désactivés et toute valeur envoyée hors listes est refusée.
- **Matrice** : semi-éditable par construction (3 PPD × 2 alimentations) ;
  les clés PPD inconnues ne sont pas représentables. La matrice est remplacée
  en bloc à chaque « Appliquer » ; une cellule vide déclenche le fallback
  `stateMap` du daemon. Un profil supprimé entre-temps est refusé côté Rust et
  signalé dans la cellule.
- **Minuterie** : l'heure est locale ; le fuseau, les transitions DST et
  l'échéance réelle (prochaine occurrence de `HH:MM`, aujourd'hui si encore à
  venir, sinon demain) sont calculés par `tccd`/logind, pas par la GUI. La GUI
  n'exécute ni `systemctl` ni `shutdown`. Si logind échoue, la sauvegarde peut
  réussir mais le statut expose l'erreur générique et la planification n'est
  pas active.
- **Minuterie** : `source=error` (ou erreur logind non vide) affiche un
  badge/bandeau d'erreur et désactive l'édition ; `enabled=false` est conservé
  et l'état n'est jamais présenté comme « annulé ». Le prochain succès réactive
  l'édition.
- **Inhibiteur de veille** : ce n'est pas un champ de `Settings`. Il vit
  uniquement dans le worker (fd logind `idle`/`block`), n'est jamais persisté et
  est libéré à la désactivation ou à la fermeture de la GUI. Aucun rapport avec
  `shutdownTime` (extinction planifiée) ni avec la veille explicite.
- **Autostart** : réglage UI local (fichier XDG `autostart/…desktop`), pas un
  champ de `Settings` ; écriture atomique dans le répertoire utilisateur, sans
  privilège ni shell. Il ne déclenche pas l'application du profil.
- `Settings` n'est pas validé champ par champ côté GUI pour les booléens : le
  daemon reste l'autorité ; `extra` et les champs non modifiables ne sont
  jamais réécrits par la GUI.
- Pas de CAS côté `Admin1` : la relecture pré-enregistrement réduit fortement
  l'écrasement inter-clients sans l'éliminer sur une fenêtre de quelques
  millisecondes.

---

## 12. Création, duplication et suppression (phase 1)

Toutes les mutations passent par `Admin1.SaveProfilesJSON`, avec le même
pipeline que l'édition : relecture `GetProfilesJSON`, comparaison à la base
d'édition (`Conflict` si un autre client a changé la collection), application
en mémoire, validation `tcc-core`, sérialisation bornée à 1 Mio, puis
rechargement `GetProfilesJSON`/`GetStateJSON`.

### 12.1 Création

- « Nouveau profil » ouvre l'éditeur en mode brouillon (nom localisé
  « Nouveau profil », description vide, valeurs de `tcc_core::Profile::default`,
  cœurs/fréquences inconnus envoyés `null`).
- L'**ID est généré côté Rust** (`ProfileId::generate()`) au moment de
  l'enregistrement : aucun UUID en QML. L'éditeur reçoit ensuite
  `createdProfileId` et bascule sur le profil créé.
- Nom obligatoire, unique (casse/espaces normalisés), ≤ `MAX_PROFILE_NAME_LEN`
  (128, la borne de `tcc-core` ; une limite plus stricte à 64 serait
  incompatible avec les profils existants).
- Refus si la collection atteint `MAX_PROFILES` (256) → code
  `TooManyProfiles`.

### 12.2 Duplication

- Action sur un profil **non-preset** ; les presets sont refusés
  (`PresetReadOnly`, entrée de menu désactivée).
- Clone **profond** (`Profile: Clone`), nouvel ID généré côté Rust, nom unique
  `X (copie)`, `X (copie 2)`, … tronqué si nécessaire pour rester ≤ 128.
- La copie est sélectionnée après succès (`createdProfileId`).

### 12.3 Suppression

- Confirmation explicite (`ConfirmDialog`), presets non supprimables.
- Le daemon reste l'autorité sur les références : si le profil est utilisé par
  `stateMap`/`stateMatrix`, `Admin1` renvoie `InUse` — la GUI affiche « Ce
  profil est utilisé par un état… » et **ne supprime rien**.
- **Pas de tombstones** : un ID supprimé peut en théorie être regénéré (UUID) ;
  aucune réutilisation volontaire n'est faite, mais un futur contrat devra
  garantir l'unicité historique.
- Supprimer le profil actif est possible si le daemon l'accepte ; la bascule
  suivante résout un autre profil.

### 12.4 UI et erreurs

- Boutons accessibles clavier, désactivés si `Admin1` est absent ou pendant un
  enregistrement (`saving`) ; bandeau « service d'administration
  indisponible » et infobulles.
- Menu contextuel : « Dupliquer » et « Supprimer » désactivés pour les presets
  (raison accessible).
- Erreurs traduites (`InUse`, `Conflict`, `PresetReadOnly`, `TooManyProfiles`,
  `NotAuthorized`, `Busy`, `ReloadFailed`, `Timeout`, …), formulaire/liste
  conservés après échec.
- Un seul enregistrement à la fois, file bornée, timeout 240 s, `SaveOutcome`
  toujours émis (`saveAction` distingue `save`/`create`/`duplicate`/`delete`).

### 12.5 Limites

- L'édition de la matrice d'état et les tombstones restent hors périmètre.
- L'import/export est décrit au §13 (format versionné, conflits, bornes).
- Pas de CAS côté `Admin1` : la relecture pré-enregistrement réduit le risque
  d'écrasement inter-clients sans l'éliminer (fenêtre de quelques ms).
- Les noms sont limités à 128 caractères (contrat `tcc-core`), pas 64.

## 13. Import / export de profils (phase 1)

### 13.1 Format d'échange

Export et import utilisent la même enveloppe versionnée :

```json
{
  "exportVersion": 1,
  "profiles": [ { "id": "…", "name": "…", "display": { … }, "cpu": { … } } ]
}
```

- `exportVersion` vaut `1` ; toute autre valeur est refusée
  (`InvalidImport`), jamais interprétée comme « compatible ».
- Les profils sont des `tcc_core::Profile` complets, indentés (`pretty`),
  avec leurs champs inconnus (`Profile::extra`) préservés.
- **Export** : uniquement les profils **personnalisés**. Les presets sont
  filtrés ; une collection sans profil personnalisé renvoie
  `NothingToExport`.
- **Import** : accepte l'enveloppe **ou** un tableau nu de profils (format
  historique `ConfigHandler.importProfiles` de TCC). L'enveloppe refuse les
  clés inconnues (`deny_unknown_fields`) ; le tableau nu est validé profil par
  profil.
- Aucun `JSON.parse` applicatif en QML : la lecture, le parse et la
  sérialisation sont faits en Rust (`profile_transfer.rs`).

### 13.2 Bornes et validation

| Contrainte | Valeur | Application |
|---|---|---|
| Taille du fichier | 1 Mio (`MAX_TRANSFER_BYTES`) | `symlink_metadata` **avant** lecture, puis taille du buffer |
| Taille du chemin | 4096 octets | rejet `PathInvalid` |
| Chemin | absolu, sans `..`, sans caractère de contrôle | rejet `PathInvalid` |
| Fichier | régulier, non symlink (import) | rejet `ReadFailed` |
| Profils | `MAX_PROFILES` = 256 | import rejeté **en bloc** (`TooManyProfiles`) |
| Nom | ≤ 128 caractères (`MAX_PROFILE_NAME_LEN`) | troncature lors du renommage |
| Version | `exportVersion == 1` | sinon `InvalidImport` |

- Un seul profil invalide rejette **tout** l'import : aucune écriture
  partielle, la collection de base n'est pas modifiée.
- Les erreurs ne contiennent ni chemin ni contenu de fichier ; seuls des
  noms de champ / codes stables remontent à l'UI.

### 13.3 Conflits

- **ID déjà présent** (ou ID vide/invalide) : un nouvel ID est généré
  (`ProfileId::generate()`), jamais d'écrasement.
- **Nom déjà présent** (comparaison trim + casse ignorée) : suffixe
  `X (importé)`, `X (importé 2)`, … tronqué pour rester ≤ 128 caractères.
- Les presets présents dans le fichier sont ignorés et comptés (`skipped`).
- La collection relue juste avant l'écriture doit être identique à la base
  d'édition, sinon `Conflict` (même garde que le CRUD).
- L'écriture passe par `Admin1.SaveProfilesJSON` (polkit) ; la GUI ne touche
  jamais `/var/lib` ni `/etc`. `tccd` valide à nouveau et applique.

### 13.4 UI

- Bouton « Importer » : désactivé sans `Admin1` (écriture) ou pendant un
  enregistrement ; raison dans `Accessible.description`.
- Bouton « Exporter » : disponible sans admin (écriture locale non
  privilégiée), désactivé sans profil personnalisé.
- `FileDialog` Qt Quick natif (`SaveFile` / `OpenFile`, filtre `*.json`) :
  le chemin est **toujours** choisi par l'utilisateur, jamais déduit du
  contenu. Seules les URL `file://` sont converties en chemin local.
- Annulation : toast « Export/Import annulé », aucun fichier écrit, aucun
  profil ajouté.
- Succès : toast « N profils exportés » ou résumé
  « N importés · M renommés · K presets ignorés ».
- Échec : bandeau d'erreur traduit (`InvalidImport`, `PathInvalid`,
  `ReadFailed`, `FileWriteFailed`, `NothingToExport`, `Conflict`,
  `TooManyProfiles`, `NotAuthorized`, `Timeout`, …).
- Un seul transfert à la fois (`saving`), file bornée, aucun thread par appel.

---

## 14. ODM / TDP / cTGP : profil (voie normale) et `Admin.Hw` (immédiat)

Deux chemins coexistent :

- **Profil** : `Profile.odmProfile.name`, `odmPowerLimits.tdpValues` et
  `nvidiaPowerCTRLProfile.cTGPOffset` sont édités dans l'éditeur et enregistrés
  via `Admin1.SaveProfilesJSON` ; le daemon les applique à l'activation du
  profil. C'est la voie normale, comme le TCC officiel.
- **Réglages → Capacités** : `Admin.Hw` applique immédiatement ODM/TDP/cTGP de
  façon **transitoire** (avancé, non persisté). Ces valeurs ne remplacent pas
  celles du profil.

### 14.1 Contrat et détection

- Service `com.tuxedocomputers.tccd.Admin.Hw`, objet
  `/com/tuxedocomputers/tccd/Admin/Hw`, interface
  `com.tuxedocomputers.tccd.AdminHw1`, contrat **1**.
- Détection **sans activation** (`NameHasOwner`, puis `ListActivatableNames`),
  toutes les 15 s ; `Ping`/`GetContractVersion`/`GetVersion` uniquement si le
  nom est déjà possédé. Aucun setter au démarrage.
- Timeouts : 10 s en lecture (`GetTdpValues`), 180 s en écriture
  (`SetOdmProfile`, `SetOdmProfileOverride`, `SetTdpValues`,
  `SetNvidiaCtgpOffset`). Un timeout client n'annule pas l'écriture côté
  admin : aucune nouvelle tentative automatique.
- Une seule écriture à la fois (`adminHwPending`) ; une seconde demande est
  refusée avec `Busy`.

### 14.2 ODM et PPD

- **Dans le profil** : sélecteur ODM (« Ne pas changer » + `platform.choices`
  sinon `io.choices`), enregistré dans `odmProfile.name` et appliqué à
  l'activation. Le badge indique `platform_profile`/`tuxedo-io` ; PPD peut
  ignorer l'application si `platform_profile` lui appartient.
- **Carte immédiate** : le profil ODM se choisit parmi `platform.choices` si
  disponible, sinon `odm.io.choices` (fallback `tuxedo-io`). La valeur initiale
  est `platform.current`, sinon `io.defaultProfile`, sinon le premier choix.
  « Appliquer » envoie `SetOdmProfile` et n'est actif que si `odmWritable` est
  vrai.
- Si `owner=ppd`, TCC n'écrit pas `platform_profile` : badge « PPD
  propriétaire », « Appliquer » désactivé, « Forcer » disponible seulement si
  `ppdOverride` est enregistré.
- `ppdOverride` est un **consentement persistant** dans `Settings`
  (`Admin1.SaveSettingsJSON`). La GUI refuse localement `SetOdmProfileOverride`
  tant qu'il est faux ; le service renvoie de toute façon `PolicyDenied`.
- Texte explicite : PPD reste la source de vérité par défaut ; l'override est
  un choix conscient et journalisé.

### 14.3 TDP et cTGP

- **TDP dans le profil** : champs seulement si `hardware.odm.tdpAvailable` et
  `hardware.odm.tdp[]` non vide. 1–3 valeurs PL1/PL2/PL4, 1–1000 W,
  strictement croissantes ; `tdpValues: []` efface. Sur les modèles où le TDP
  officiel passe par le profil ODM (`io.tdpAvailable=false`), les champs sont
  masqués.
- **cTGP dans le profil** : 0…`ctgpMax` (`nvidia.maxCtgpOffset` sinon 200),
  masqué sans NVIDIA ; appliqué à l'activation.
- **Carte immédiate** : mêmes bornes, écriture transitoire via
  `SetTdpValues`/`SetNvidiaCtgpOffset`.

### 14.4 UI, erreurs et états

- La carte immédiate est masquée sans `Admin.Hw` ; une notice + « Réessayer »
  reste visible si le matériel existe. L'éditeur de profil reste éditable
  (écriture via `Admin1`), avec les champs masqués si le matériel n'expose
  rien.
- Pendant l'écriture : badge « Application en cours… », boutons désactivés,
  brouillon conservé.
- Erreurs traduites : `PolicyDenied`, `PartialApply`, `InvalidArgs`/
  `Validation`, `NotSupported`, `WriteFailed`, `RateLimited`, `Busy`,
  `NotAuthorized`, `Timeout`, `Transport`, service arrêté. Le formulaire est
  **conservé** ; après succès, relecture matérielle et brouillons
  resynchronisés.
- Aucun chemin, contenu de fichier ou secret n'est affiché ; les détails sont
  bornés et filtrés.

### 14.5 Limites

- Les valeurs du **profil** sont persistantes et réappliquées à chaque
  activation ; les écritures **`Admin.Hw`** sont transitoires.
- PPD propriétaire de `platform_profile` peut ignorer le profil ODM ; le
  TDP PL n'est pas exposé sur les modèles où le TDP officiel passe par le
  profil ODM (`io.tdpAvailable=false`).
- `Admin.Hw` n'est pas installé par défaut : sans lui, la carte est absente et
  le reste de l'UI fonctionne normalement.
- Pas de CAS côté admin : une lecture avant écriture réduit les conflits sans
  les éliminer.

### 14.6 Tests

- Rust (`cargo test -p tcc-gui --features dev-overrides`, bus de session
  `dbus-run-session`) : faux `Admin.Hw` pour la sonde sans activation, le
  contrat, `GetTdpValues`, `SetOdmProfile`/`SetOdmProfileOverride`,
  `SetTdpValues`, `SetNvidiaCtgpOffset`, le refus PPD (`PolicyDenied`), une
  application partielle (`PartialApply`) et le mapping stable des codes
  d'erreur.
- QML (`tst_admin_hw.qml`) : masquage sans service ou sans matériel, garde de
  consentement avant « Forcer », bornes TDP/cTGP, conservation du brouillon
  après erreur, carte YCbCr seule avec message d'absence du service.
- Visibilité : `tst_hardware_visibility.qml` vérifie que la carte Capacités
  disparaît quand service et matériel sont absents.

### 14.7 Bornes locales alignées sur le serveur (AH-09)

| Champ | Règle serveur | Validation GUI |
|---|---|---|
| TDP count | `1..=3` (`MAX_TDP_VALUES`) | liste 1–3, 4 entrées refusées avant appel |
| TDP valeurs | `1..=1000` W | champ hors bornes → message rouge, aucun envoi |
| TDP descripteurs | `pl1`, `pl2`, `pl4` (ordre firmware) | ordre et unicité vérifiés contre `hardware.odm.tdp[]` ; doublon/ordre inattendu refusé |
| ODM nom | non vide, ≤ 64, ASCII graphique, sans `\\` | mêmes règles (`odmNameValid`), la valeur vient des choix matériels |
| cTGP | `0..=200` statique, borne dynamique si backend | `0..=ctgpMax`, négatif refusé, `ctgpMax` = `nvidia.maxCtgpOffset` ou 200 |

Les erreurs locales utilisent des clés sures (`InvalidArgs`, `Validation`,
`settings.tdpDescriptorError`) et n'affichent jamais la valeur saisie.
`adminHwSucceeded` n'est publié qu'après la réponse du service : aucun succès
optimiste.

---

## 15. Sources réelles de l'éditeur

L'éditeur ne consomme plus `DemoData` pour les capacités produit :

- `HardwareData.cpuCaps` (`totalCores`, `onlineCores`, `availableEpp`,
  `availableGovernors`) ;
- `HardwareData.displayInfo` (`hasInternalDisplay`, `internalConnector`,
  `connected`) ;
- `HardwareData.device` n'est pas utilisé pour éditer un profil (identité
  globale), mais sert au Dashboard/Info.
- Le taux de refresh et la résolution ne sont plus complétés par `DemoData` :
  seuls les modes de `User1` sont proposés. Les limites par compositeur sont
  documentées dans `UI_DESIGN.md` §21.

---

## 16. Résolution et taux réels (`User1`)

### 16.1 DTO `ProfileEdit`

`ProfileEdit` (Rust, `deny_unknown_fields`) transporte :

| Champ | Type | Sémantique |
|---|---|---|
| `useResolution` | `Option<bool>` | `Some(true/false)` = demande explicite ; `None` (ancien client) = ne pas modifier |
| `xResolution` | `Option<u32>` | largeur cible, `null` si non demandée |
| `yResolution` | `Option<u32>` | hauteur cible, `null` si non demandée |

Validation Rust : si `useResolution == Some(true)`, les deux dimensions
doivent être présentes et dans `1..=16384` ; sinon
`Validation("resolution")`. `Some(false)`/`None` conservent les dimensions
existantes de `profile.display` (pas d'effacement). `ProfileRules.toEdit` n'envoie `xResolution`/`yResolution` que
si la résolution est demandée et qu'un mode réel est sélectionné.

### 16.2 Sélection QML

- `resolutionIndex` pointe une résolution unique de `AgentData.resolutions()`.
- `refreshIndex` pointe un taux de `AgentData.refreshRates(w, h)` ; le
  changement de résolution réinitialise le taux sur le mode courant de cette
  résolution, sinon sur le premier taux disponible.
- `resolutionMissing`/`refreshMissing` signalent qu'un mode enregistré n'est
  plus exposé ; `form.resolutionExists`/`refreshExists` passent à `false` et
  `ProfileRules` renvoie `editor.error.resolutionUnavailable` /
  `editor.error.refreshUnavailable`.
- Agent indisponible : `effectiveUseResolution`, `effectiveRefreshRate`,
  `effectiveWidth`/`effectiveHeight` reprennent les valeurs du profil pour ne
  jamais les effacer lors d'une édition CPU/ventilateurs.

### 16.3 Tests

- Rust : parsing `DisplayInfoView`, rejet des types faux et des modes
  externes, `ProfileEdit` résolution appliquée/préservée, dimensions
  manquantes/hors bornes.
- QML : `tst_agent_data` (modes, résolutions, taux, erreurs),
  `tst_real_capabilities` (sélection, mode absent, agent indisponible,
  préservation), `tst_info_support` (Dashboard/Réglages).
- Faux service `User1` sur bus de session : sonde, contrat, lecture du
  rapport, refus de contrat incompatible.

### 16.4 Limites

- Les modes dépendent du compositeur ; un backend non supporté expose
  `available=false`/`supported=false`.
- L'agent peut être endormi : l'UI propose une actualisation explicite.
- Aucune écriture d'affichage ne passe par la GUI ; `tccd-user` applique et
  refuse un mode absent.

---

## 17. Écritures matérielles persistantes (charge, Fn-Lock, éclairage)

`SettingsEdit` transporte quatre champs optionnels supplémentaires :

| Champ | Type | Sémantique |
|---|---|---|
| `chargingType` | `Option<String>` | `null`/absent = ne pas changer ; ≤64, `[A-Za-z0-9._:-]`, valeur dans `chargeTypes` ou ABI standard |
| `fnLockStatus` | `Option<bool>` | `null` = ne pas changer |
| `keyboardStates` | `Option<Vec<KeyboardStateEdit>>` | absent/`null` = préserver ; `[]` efface ; 0–64 zones, mode/borne validés |
| `lightbarStates` | `Option<Option<LightbarStateEdit>>` | absent = préserver ; `null` explicite = effacer ; objet = appliquer |

- Le DTO est validé dans Rust avant `SaveSettingsJSON` : aucune valeur QML
  n'est crue sur parole. Les listes de types de charge viennent du dernier
  instantané matériel ; une liste vide autorise uniquement les valeurs ABI
  standard.
- L'éclairage est édité dans **Outils** (clavier blanc/RGB, zones, lightbar),
  pas dans le profil. On/off = luminosité 0. Une couleur uniforme est
  appliquée aux zones (limite assumée).
- Fn-Lock n'est proposé que si `fnLock.supported && writable &&
  enabledKnown` ; après écriture, l'état réel relu par le worker fait foi.
- Aucun setter `Manager1` n'est appelé pour ces trois domaines.

---

## 18. Assignations AC/BAT affichées dans les profils

`LiveData` ne déduit plus `assignedAc`/`assignedBat` du profil actif. Il lit la
matrice persistée exposée par `SettingsData`
(`GetSettingsJSON.stateMatrix`) :

- chaque profil est annoté `assignedAc=true` s'il apparaît dans au moins une
  cellule `*/ac`, `assignedBat=true` pour une cellule `*/battery` ;
- `LiveData.assignedAc`/`assignedBat` (propriétés singulières utilisées par la
  sidebar) suivent la cellule `(PPD courant, source)` ;
- matrice absente ou daemon déconnecté → aucune icône, aucune valeur inventée ;
- les toggles AC/BAT de l'éditeur restent désactivés : la matrice se modifie
  dans Réglages, et l'éditeur reflète désormais l'état réel.

Le tray Fn-Lock utilise le même DTO `SettingsEdit` que Réglages
(`fnLockStatus`), avec relecture/refus resynchronisés ; il n'appelle jamais
`Manager1`.
