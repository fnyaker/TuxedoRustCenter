# Revue de sécurité — écritures GUI récentes (réglages, résolution, UVC, webcam)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-07 |
| Auteur | agent `security` |
| Objet | `SettingsEdit`, écritures GUI, résolution interne, UVC lecture seule et kill switch webcam |
| Périmètre | `docs/security/**` uniquement — aucun code ni packaging modifié |
| Code lu | `crates/tcc-gui/src/settings_edit.rs`, `profile_edit.rs`, `daemon_client.rs`, `hardware_view.rs` ; `qml/pages/{SettingsPage,ProfileEditorPage,ToolsPage}.qml`, `qml/core/{HardwareData,ProfileRules,SettingsData,AdminHwData}.qml` ; `crates/tcc-core/src/{settings,validate,profile}.rs` ; `crates/tcc-ipc/src/display.rs` ; `crates/tcc-hw/src/display.rs` ; `crates/tcc-user/src/{agent.rs,backend/*.rs}` ; `crates/tcc-hw/src/v4l2/**` |
| Preuves exécutées | `cargo test -p tcc-gui --bins` ⇒ **167/167** ; `cargo test -p tcc-admin` ⇒ **100/100** ; `cargo test -p tcc-user` ⇒ **143/143** ; `cargo test -p tcc-hw --lib display` ⇒ **23/23** ; `cargo test -p tcc-core --lib settings` ⇒ **25/25**. Tests QML (`tst_admin_hw`, `tst_settings_page`, `tst_profile_editor`, `tst_tools_timer`, `tst_i18n_parity`) **présents, non exécutés ici**. |
| Verdict | **Aucun Critique/Élevé/Moyen.** 2 **Faibles** (brightness lighting non bornée côté Rust, erreurs masquées par les cartes) et 4 Info. Aucune écriture privilégiée directe : `Admin1`/`Admin.Hw` restent les seules voies de persistance et d’écriture matérielle. |

---

## 1. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **UIW-01** | **Faible / A** | `settings_edit.rs:337-390` ; `validate.rs:840-877` | `keyboardStates.brightness` et `lightbarStates.brightness` sont des `u8` acceptés **sans borne au maximum matériel**. `apply_lighting_edit` vérifie le nombre de zones et les canaux RGB, `Settings::validate` ne vérifie que les couleurs/count. Un client `Admin1` authentifié peut persister 255 alors que `HardwareView.Keyboard.maxBrightness` vaut 3 ou 100. | Valeur persistée inapplicable ; le worker lighting peut clamper/échouer. Pas d’escalade (l’appelant est déjà autorisé par polkit). | Passer les capacités lighting (maxBrightness) à `SettingsEdit` ou valider dans `Admin1` avec le snapshot ; refuser `brightness > max`. | Test : keyboard/lightbar brightness 255 avec max 3 ⇒ `Validation` ; slider QML déjà borné. |
| **UIW-02** | Info | `daemon_client.rs:2382,2400` | La GUI utilise bien `Admin1`/`Admin.Hw` pour la persistance et les limites, **mais** appelle aussi `SetTempProfile`/`ClearTempProfile` (interface historique) et `Manager1.SetSensorDataCollectionStatus` (bail capteurs). Aucun setter `Manager1` charge/lighting/Fn-Lock n’est appelé. | Les exceptions sont non matérielles et rate-limitées ; le bail capteurs est borné à 5 min. La formulation « aucun setter Manager1 » est littéralement inexacte. | Documenter ces deux exceptions dans `THREAT_MODEL`/`DBUS_API` ; ne pas les confondre avec les setters matériels fermés par la policy. | Revue policy : `SetSensorDataCollectionStatus` reste autorisé, `SetCharge*`/`SetLighting*`/`SetFnLock*` refusés. |
| **UIW-03** | **Faible / B** | `SettingsPage.qml:168-187,1814` ; `HardwareData.qml:89-92` | Les cartes sont masquées par `visibleIfAvailable()` en live connu. `capabilitiesVisible` exige `AdminHwData.available` : si le matériel expose ODM/TDP/cTGP mais que `Admin.Hw` manque, la carte — et donc la notice `hwServiceMissing` — est **invisible**. Même mécanisme pour `chargeUnavailable`, `lightingUnavailable`, `fnLock`/`odm`/`tdp` en erreur. | L’utilisateur ne voit ni le service manquant ni l’erreur de lecture ; diagnostic faux, pas de privilège. | Afficher les notices d’erreur/service **hors** des cartes conditionnelles ; distinguer « absent confirmé » de « erreur de lecture » ; test QML avec `AdminHwData.available=false` et TDP présent. | QML : service absent + TDP disponible ⇒ notice visible ; charging error ⇒ notice visible. |
| **UIW-04** | Info | `profile_edit.rs:137-150,628-637` ; `display.rs:365-455` ; `agent.rs:263-269` | `ProfileEdit` accepte toute résolution **1..16384** sans vérifier qu’elle existe dans les modes de l’agent ; la QML le fait (`resolutionExists`), mais un client `Admin1` peut persister une résolution absente. `tcc-user` sélectionne uniquement un mode réel et échoue proprement (`DisplayModeNotFound`), sans modification partielle. | Aucune touche externe ; profil inapplicable, erreur tardive. Pas d’escalade. | Soit valider contre `AgentData` côté GUI uniquement (déjà), soit documenter que `Admin1` ne peut pas connaître les modes ; conserver l’échec propre. | Test : profil 1920×1080 absent ⇒ `tcc-user` ne change rien ; QML propose seulement les modes réels. |
| **UIW-05** | Info | `hardware_view.rs:315,350` ; `daemon_client.rs:3447-3475` ; `ToolsPage.qml:916-921,981-984` | `VideoDevice.controls_error` et `Webcam.errors` ne sont **pas clampés** par `sanitize_hardware_view` (contrairement aux noms/chemins/menus) ; `UvcValue.value` reste un `serde_json::Value` brut. Les messages peuvent contenir `/dev/videoN` ou être longs. | Fuite de chemins système non sensibles et rendu UI non borné ; aucune écriture. | Clamper `controls_error`/`errors` (128 chars, sans chemin) et borner la forme de `UvcValue.value` ; garder l’affichage en texte brut. | Test payload avec erreur de 1 Kio/contenant `/dev/video0` ⇒ tronquée/assainie. |
| **UIW-06** | Info | `ProfileEdit:70-71,660-661` ; `ToolsPage.qml` (lecture seule) ; `Settings` (aucun champ global) | Le kill switch webcam global reste **read-only** dans Outils ; seul le profil porte `webcamStatus`/`useStatus` via `Admin1.SaveProfilesJSON`. | Pas de régression : le choix par profil est la sémantique documentée. Un toggle global ajouterait une surface d’écriture supplémentaire. | **Recommandation : garder le kill switch via le profil.** Si la parité exige un toggle global, ajouter `Settings.webcamEnabled` avec `Admin1.SaveSettingsJSON` (polkit `settings`) et application par le worker ; ne pas rouvrir `Manager1`. | Revue de parité + test worker sur changement de profil. |
| **UIW-07** | Info | `crates/tcc-core/src/settings.rs:261-270` | Le commentaire de `ppd_override` dit encore « n’est pas encore lu par `tccd` » alors que `main.rs` le propage désormais. | Documentation obsolète, aucune faille. | Mettre à jour le commentaire lors du prochain passage `rust-core`. | Revue documentaire. |

---

## 2. `SettingsEdit` — conformité

| Point | État | Preuve |
|---|---|---|
| DTO fermé | **Conforme** | `#[serde(deny_unknown_fields)]` (`settings_edit.rs:38-43`) ; types stricts (`Option<bool>`, `u8`, `i16`, enums par `match`). |
| Taille | **Conforme** | DTO ≤ **64 Kio** (`:31,120-125`) ; `Settings` sérialisé ≤ **1 Mio** avant D-Bus (`:33,229-233`). |
| `extra` / `stateMap` / champs non modifiés | **Conforme** | `base.clone()` puis affectations ciblées (`:180-227`) ; tests `only_allowed_fields_are_changed`, `state_matrix_serialization_is_preserved`. |
| Tombstones | **Conforme** | `deleted_profile_ids` n’est pas dans le DTO ; `base.clone()` le conserve ; `Settings::validate` + `Admin1` append-only. Test dédié absent (couvert par la revue tombstones). |
| Charge | **Conforme** | Profil/priorité/seuils validés contre `ChargeCapabilities` (`:244-279`) ; `chargingType` validé forme + allowlist ABI/backend (`:287-327`). |
| Fn-Lock | **Conforme** | `Option<bool>` strict ; application par le worker, disponibilité masquée côté UI. |
| Lighting | **Conforme avec UIW-01** | Nombre de zones ≤ `MAX_KEYBOARD_STATES`, canaux `-1..=255`, modes `static|breathing` ; brightness non bornée au max matériel. |
| Matrice | **Conforme** | PPD ∈ 3 valeurs, power ∈ ac/battery, ID valide, profil présent, doublon refusé (`:423-463`). |
| Minuterie | **Conforme** | `HH:MM` 24 h, `clearShutdownTime` exclusif (`:475-520`). |
| Aucun payload arbitraire | **Conforme** | Aucun `Value` en entrée, aucun chemin, aucun `Command` ; revalidation `Settings::validate` avant sérialisation. |

---

## 3. UI — écritures et affichage

| Point | État | Preuve |
|---|---|---|
| Aucune écriture privilégiée GUI | **Conforme** | `SettingsEdit`/`ProfileEdit` sont purs ; envoi par `Admin1.SaveSettingsJSON`/`SaveProfilesJSON` et `Admin.Hw` (polkit) ; aucun accès `/etc`, `/var/lib`, `/sys`, `/dev`. |
| Pas de setter `Manager1` matériel | **Conforme** | `Manager1` proxy n’expose que `GetHardwareSnapshotJSON`, `SetSensorDataCollectionStatus`, `GetShutdownTimerJSON` (`daemon_client.rs:2392-2404`) ; charge/lighting/Fn-Lock fermés par la policy. |
| Exceptions non matérielles | **Info (UIW-02)** | `SetTempProfile`/`ClearTempProfile` (interface historique) et bail capteurs. |
| Masquage / erreurs | **Faible (UIW-03)** | `visibleIfAvailable` + `capabilitiesVisible` masquent service/erreur quand le matériel est indisponible. |
| Demo badge | **Conforme** | Dashboard/Info utilisent `LiveData.isLive` (`DashboardPage.qml:99-103`, `InfoPage.qml:49-52`) ; `HardwareData.usingDemo`/`stale` restent des états internes ; messages « demo » séparés. |
| `PlainText` | **Conforme** | `AppLabel`/`AppToolTip`/`AppSelect`/`Gauge`/`FanCurveChart` en `Text.PlainText` ; les rares `Text` restants affichent des libellés statiques ou numériques. |
| i18n | **Conforme** | Clés FR/EN/DE présentes pour `hwServiceMissing`, `tdpRange`, `tdpDescriptorError`, `hwCtgpRange`, `resolution*`, `webcam.readOnly` ; parité QML à rejouer. |

---

## 4. Résolution interne

| Point | État | Preuve |
|---|---|---|
| Bornes 1..16384 | **Conforme** | `MAX_RESOLUTION_DIMENSION=16_384` (`profile_edit.rs:31,137-150`) ; `MAX_RESOLUTION` (`validate.rs:65`) ; `MAX_DISPLAY_DIMENSION` (`display.rs:32,365-381`) ; `agent.rs:263-269`. |
| Modes réels uniquement | **Conforme à l’application** | QML `resolutionExists`/`AgentData` ; `plan_internal_display_mode` cherche la combinaison exacte et échoue sinon (`display.rs:393-455`). Réserve UIW-04 (stockage Admin1). |
| Écrans externes jamais touchés | **Conforme** | `DisplayModeRequest::validate` refuse un output non interne (`tcc-ipc/display.rs:300-360`) ; `select_internal_output` ; X11 refuse un CRTC partagé (`x11.rs:258-266`). |
| Idempotence | **Conforme** | `plan` compare résolution + taux avec tolérance ; `AlreadyApplied` ne touche pas le matériel. |
| Échec sans modification partielle | **Conforme** | Une seule opération backend après planification ; mode introuvable ⇒ `DisplayModeNotFound`, aucune écriture ; X11 `SetCrtcConfig` atomique. |
| Persistance | **Conforme** | `ProfileEdit` écrit `useResolution`/`xResolution`/`yResolution` après validation ; profil appliqué par `tcc-user`. |

---

## 5. UVC — lecture seule et conditions d’une future écriture

**État actuel** : `ToolsPage` affiche les contrôles en lecture seule (`controlsWritable: false`, `HardwareData.qml:380`) ; aucun `SetWebcamControl`, aucun `VIDIOC_S_CTRL` compilé (`abi.rs:20`, `mod.rs:6`), aucun preview/`Camera`/`VideoOutput`. `tccd` ouvre en `O_RDONLY` et `DeviceAllow=char-video4linux r`. Les bornes d’affichage sont 16 devices / 64 contrôles / 128 menus (`daemon_client.rs:3447-3475`). **L’écriture reste hors jalon 2.**

Conditions minimales avant d’activer l’écriture :

1. **Session uniquement** : `tccd-user`, même utilisateur, ACL `uaccess`/groupe `video` ; jamais `tccd` root ni `tccd-admin`.
2. **API dédiée** : `VIDIOC_S_CTRL` isolé, allowlist normative de CIDs, valeurs/menus validés, aucun `id` brut client.
3. **`unsafe` confiné** : wrapper unique, tailles/offsets testés, `EINTR`, pas de streaming/`REQBUFS`.
4. **Budget** : un device en vol, budget global par tick, `EBUSY`/back-off, état `stalled`.
5. **D-Bus session** : méthode authentifiée par l’identité de session, rate-limit, pas de chemin/device fourni par le client.
6. **Tests** : fake + caméra réelle, privacy/exposure, LED, permissions retirées.

---

## 6. Kill switch webcam global

Le kill switch est **par profil** (`webcam.status`/`use_status`, `ProfileEdit:70-71,660-661`) et la carte Outils est en lecture seule. **Recommandation : conserver ce modèle** ; un toggle global ajouterait un champ `Settings` et une surface `Admin1` supplémentaire sans besoin de parité démontré. Si un toggle global est requis, le faire via `Admin1.SaveSettingsJSON` (polkit `settings`) + worker, jamais via `Manager1`.

---

## 7. Verdict

**Aucun finding Critique/Élevé/Moyen.** Les écritures GUI sont **fermées, validées et routées par polkit** ; aucune écriture privilégiée directe, aucun chemin client, aucun payload arbitraire. **UIW-01** (brightness lighting) et **UIW-03** (erreurs masquées) sont des corrections de qualité à faire avant release ; **UIW-02/04/05/06/07** sont Info. **Aucune action bloquante de sécurité.** L’UVC en écriture reste hors périmètre ; le kill switch global reste profile-scoped.

**Limites manuelles** : vrai polkit, session réelle (X11/Wayland), modes réels et écrans externes, webcam réelle, tests QML, i18n/a11y, `Admin1`/`Admin.Hw` sur bus système.
