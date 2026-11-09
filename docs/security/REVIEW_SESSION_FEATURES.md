# Revue de sécurité — fonctions de session (minuterie, power blocker, réglages persistés)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | minuterie d'extinction, inhibiteur de veille, matrice PPD × alimentation et charge persistées |
| Périmètre | `docs/security/**` uniquement — aucun code modifié |
| Code lu | `crates/tcc-daemon/src/shutdown_timer.rs`, `dbus.rs` (`GetShutdownTimerJSON`), `main.rs` (`reconcile_shutdown_timer`), `docs/DBUS_API.md` ; `crates/tcc-gui/src/power_blocker.rs`, `daemon_client.rs` (worker), `qml/core/{PowerBlockerData,ShutdownTimerData,SettingsData}.qml`, `qml/pages/{ToolsPage,SettingsPage}.qml` ; `crates/tcc-gui/src/settings_edit.rs` ; `crates/tcc-core/src/{settings.rs,validate.rs}` |
| Intégration lue | `packaging/dbus/com.tuxedocomputers.tccd.conf`, `packaging/polkit/com.tuxedocomputers.tccd.policy`, `scripts/release/dbus-policy-test.sh` |
| Preuves exécutées | `cargo test -p tcc-daemon --bins` ⇒ **152/152** ; `cargo test -p tcc-gui --bins` ⇒ **90/90** ; `cargo test -p tcc-hw --lib v4l2` etc. non concernés. Vrai logind, DST réelle et session graphique réelle **non testés**. |
| Verdict | **Timer, blocker et persistance sont globalement conformes.** 1 finding **Moyen** (écran LUKS encore présent, capture de secrets + faux succès), 3 Faibles, 5 Info. Aucun Critique/Élevé. **SF-01 est bloquant pour la cohérence de release LUKS.** |

---

## 1. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **SF-01** | **Moyen / B** | `crates/tcc-gui/qml/pages/ToolsPage.qml:20-56,224-320` ; `Strings.qml:556-570` | **Un écran LUKS subsiste alors que LUKS est hors périmètre.** `submitLuks` capture trois champs mot de passe, valide la longueur/confirmation, puis un `Timer` de 1,4 s affiche « Mot de passe modifié sur toutes les partitions chiffrées » et efface les champs. Aucun appel D-Bus/admin n'existe. | Saisie de vrais secrets dans un écran sans backend, conservation en mémoire QML, **faux succès** (l'utilisateur peut croire sa phrase changée), contradiction directe avec la décision utilisateur et la policy sans action LUKS. | **Retirer la carte LUKS** (formulaire, timer, chaînes) ou la remplacer par une note « hors périmètre, non implémenté » **sans champ de saisie**. Ne jamais conserver de secret QML. | Test QML : aucun champ `luks*`/`tools.luks` ; `grep -R "luksCurrent\|luksNew\|luksConfirm"` vide. |
| **SF-02** | **Faible** | `shutdown_timer.rs:233-240,267-289` | Si une heure invalide arrive alors qu'une planification `applied` existe, `reconcile` appelle `set_error("heure invalide")` **puis** `cancel()` ; si l'annulation réussit, `cancel` appelle `set_disabled()` et **efface l'erreur**. L'UI affiche « désactivé sans erreur » au lieu de « heure invalide ». | Diagnostic trompeur, pas d'écriture non désirée ; l'ancienne planification est bien annulée. | Annuler d'abord, puis poser l'erreur ; ou faire `cancel` sans `set_disabled` quand il est appelé pour une erreur de parsing. | Test : planification valide, puis `reconcile(Some("25:00"), true/false)` ⇒ `status.error == "heure d'extinction invalide"` et `enabled=false`. |
| **SF-03** | **Faible** | `daemon_client.rs:907-930,1132-1140` ; `power_blocker.rs:94-120` | Si la commande `SetPowerBlocker(false)` est refusée par la file (`Busy`/worker arrêté), l'erreur est affichée mais le `fd` d'inhibition **reste détenu** jusqu'à l'arrêt du worker/GUI. L'UI montre `powerBlocked=true` + erreur ; aucune libération locale. | Inhibition d'inactivité persistante alors que l'utilisateur croit l'avoir désactivée ; pas d'escalade, mais blocage involontaire jusqu'à fermeture. | Sur échec d'envoi d'une désactivation, libérer localement le fd (ou rendre la commande de désactivation prioritaire/hors file) ; afficher l'état réel. | Test : file pleine + `blocked=false` ⇒ fd fermé, `powerBlocked=false`, erreur visible. |
| **SF-04** | Faible / Info | `power_blocker.rs:150-185` ; `daemon_client.rs:1098-1140` | Aucun suivi de `NameOwnerChanged` pour logind : si logind redémarre, l'inhibiteur peut être perdu côté service alors que la GUI conserve `powerBlocked=true` ; aucune ré-inhibition. | État affiché faux ; l'inhibition réelle a disparu, ou un fd orphelin reste ouvert. Pas de privilège. | Écouter le propriétaire de `org.freedesktop.login1` et ré-inhiber/échouer proprement ; au minimum documenter. | Faux logind : drop du nom ⇒ état/erreur cohérents. |
| **SF-05** | Info | `dbus.rs:969-980` ; `conf:105` | `GetShutdownTimerJSON` est lisible **sans polkit ni rate limit** ; expose `enabled`, `scheduledAt`, `source`, `error` à tout utilisateur local. | Information de planification (pas un secret) ; pas de modification possible. Un client peut interroger en boucle (coût faible). | Acceptable ; ajouter un rate limit si le coût devient mesurable, ou documenter. | Test 100 lectures ⇒ coût borné (si limite ajoutée). |
| **SF-06** | Info | `shutdown_timer.rs:37-44,215-265` ; `DBUS_API.md:131-160` | `TimerSource::Logind` est réservé mais **jamais alimenté** : une extinction planifiée par un autre outil n'est pas détectée. TCC peut afficher `enabled=false` alors que logind a une échéance, et `ScheduleShutdown`/`CancelScheduledShutdown` peuvent remplacer/annuler une planification externe. | État incomplet, conflit possible avec un autre planificateur ; pas d'écriture non autorisée. | Lire l'état logind (`ScheduledShutdown`/propriété) ou documenter explicitement que TCC ne gère que sa propre échéance ; ne pas annuler une échéance non appliquée par TCC. | Fake logind avec échéance externe ⇒ `source=logind` ou message explicite ; test réel. |
| **SF-07** | Info | `shutdown_timer.rs:88-103,317-334` | DST : une heure locale ambiguë (automne) choisit **la première occurrence** ; une heure inexistante (printemps) est refusée. Aucun test DST. | Décalage d'une heure possible autour des transitions ; comportement documenté mais non prouvé. | Test avec `TZ=Europe/Paris` autour des transitions ; documenter le choix. | Test DST dans `shutdown_timer.rs`. |
| **SF-08** | Info | `settings_edit.rs:338-350` | Le conflit reste un **check-then-act** : `base != current` vérifié avant `SaveSettingsJSON`, sans révision/CAS. La matrice/charge/timer peuvent être écrasés par un autre client entre la relecture et l'écriture. | Dernier écrivain gagne (risque déjà connu S1-02) ; pas d'écriture hors admin. | Révision/etag côté `Admin1` ou documenter ; éviter d'écraser les cellules non modifiées. | Deux clients ; test QML conflit. |
| **SF-09** | Info | `settings_edit.rs:142-215` ; `SettingsPage.qml:280-330` | Les profils/priorités/seuils de charge sont validés contre `ChargeCapabilities` du **dernier instantané matériel** ; sans instantané, l'enregistrement est refusé (`Validation`). C'est fail-closed, mais l'UI dépend d'une lecture matérielle. | Refus légitime si le matériel n'expose pas la capacité ; pas de valeur arbitraire. | Documenter ; afficher « non disponible » plutôt qu'une erreur générique. | Test sans capabilities ⇒ refus, état inchangé. |
| **SF-10** | Info | `power_blocker.rs:160-180` | `LogindInhibitor` mappe **tout** `MethodError` sur `Refused` ; un `ServiceUnknown` peut être affiché « refusé » au lieu de « indisponible ». | Message UI imprécis, pas de privilège. | Distinguer `ServiceUnknown`/`NameHasNoOwner` ⇒ `Unavailable`. | Test fake `ServiceUnknown`. |

---

## 2. Conformité vérifiée

### 2.1 Minuterie d'extinction

| Point | État | Preuve |
|---|---|---|
| Parsing `HH:MM` | **Conforme** | `parse_hhmm` strict 5 caractères, chiffres, `00..23`/`00..59` (`shutdown_timer.rs:336-355`) ; `Settings::validate` revalide (`validate.rs:640-660`) ; GUI `normalize_shutdown_time` (`settings_edit.rs:305-330`). |
| Heure locale / DST | **Conforme avec réserve** | `Local.from_local_datetime` ; `Ambiguous` ⇒ earliest, `None` ⇒ erreur (`:88-103`) ; SF-07 (DST non testée). |
| Prochaine occurrence | **Conforme** | Si `target <= now`, `date.succ` (`:317-334`) ; tests `schedules_future_time_today`, `past_time_is_scheduled_tomorrow`. |
| logind | **Conforme** | `ScheduleShutdown("poweroff", usec)` / `CancelScheduledShutdown`, timeouts 2 s, bus système, aucun shell (`:120-180`). |
| Idempotence | **Conforme** | Même `usec` non reprogrammé ; `force` ne reprogramme que si l'échéance diffère (`:215-265`) ; tests `same_deadline_is_not_rescheduled`, `changed_time_is_rescheduled`. |
| Reload / reprise | **Conforme** | `reconcile_shutdown_timer` au démarrage, SIGHUP, `Control1`, reprise (`main.rs:536,566,617,663,708`) ; `force=true`. |
| Pendant veille | **Conforme** | `reconcile` retourne si `suspended` (`:215-218`) ; les appels passent `sleep_machine.is_suspended()`. |
| Lecture D-Bus | **Conforme** | `GetShutdownTimerJSON` lecture seule, sans polkit (`dbus.rs:969-980`), allow exact (`conf:105`), policy test (`dbus-policy-test.sh:165-167`). |
| Aucun setter | **Conforme** | Seule voie : `Settings.shutdownTime` via `Admin1.SaveSettingsJSON` (polkit `settings`) ; aucun `SetShutdown*`. |
| Erreurs | **Conforme avec SF-02** | Messages génériques, pas de chemin/secret ; SF-02 sur l'effacement de l'erreur. |

### 2.2 Power save blocker

| Point | État | Preuve |
|---|---|---|
| `Inhibit("idle", …, "block")` | **Conforme** | `power_blocker.rs:150-185` ; `what="idle"` uniquement ⇒ une veille explicite n'est pas bloquée ; `block` (pas `delay`). |
| Sans root/polkit | **Conforme** | Bus système, aucune action polkit, aucun `systemctl`/shell ; logind reste l'autorité. |
| fd `OwnedFd` | **Conforme** | Stocké dans `PowerBlocker`, `release()`/`Drop` le ferme (`:94-120`) ; tests `activation_and_release_close_the_fd`, `drop_releases_the_inhibitor`. |
| Désactivation / fermeture | **Conforme au nominal** | `set_blocked(false)` libère ; worker détient le blocker (`daemon_client.rs:1098-1140`) et le drop à la sortie ; SF-03 sur le refus de commande. |
| Session graphique | **Conforme** | `XDG_SESSION_TYPE` ∈ x11/wayland/mir (`:190-205`) ; sinon `NotGraphical`, aucun appel ; logind reste l'autorité finale. |
| Timeout | **Conforme** | `INHIBIT_TIMEOUT=2 s` autour de connexion + proxy + `Inhibit` (`:150-185`). |
| Pas de persistance | **Conforme** | Aucun fichier, aucun réglage ; l'inhibiteur ne survit pas à la GUI. |
| Erreurs | **Conforme avec SF-10** | Codes `NotGraphical`/`Unavailable`/`Refused`/`Timeout` ; pending libéré par `apply_power_blocker` (`daemon_client.rs:1668-1690`). |

### 2.3 Matrice / charge / réglages persistés

| Point | État | Preuve |
|---|---|---|
| DTO fermé et borné | **Conforme** | `SettingsEdit` `deny_unknown_fields` (`settings_edit.rs:38-65`), 64 Kio (`:29-30`), payload final ≤ 1 Mio (`:165-170`). |
| Profils de matrice | **Conforme** | `apply_state_matrix_edit` : PPD ∈ 3 valeurs, power ∈ ac/battery, ID validé, profil présent, doublon refusé (`:237-280`). |
| Charge | **Conforme** | Profil/priorité/seuils vérifiés contre `ChargeCapabilities` (`:176-230`) ; `Settings::validate` final ; type de charge non modifiable. |
| `extra`/`stateMap` préservés | **Conforme** | `base.clone()` puis seuls les champs présents sont écrits ; tests `only_allowed_fields_are_changed`, `state_matrix_serialization_is_preserved`. |
| Conflit | **Conforme avec SF-08** | `prepare_settings_save` compare `base`/`current` ; course résiduelle. |
| `InUse` | **Conforme** | Références croisées revalidées par `Admin1` (`Settings::validate`, `validate_settings_references`) ; GUI refuse un profil absent. |
| Aucune écriture hors `SaveSettingsJSON` | **Conforme** | GUI → `AppState.requestSettingsSave` → worker `SaveSettings` → `Admin1.SaveSettingsJSON` ; aucun `fs`/chemin. |
| LUKS | **Non conforme (SF-01)** | Écran LUKS toujours présent dans `ToolsPage.qml`, sans backend, avec faux succès. |

---

## 3. Policy vérifiée

| Élément | État | Preuve |
|---|---|---|
| `GetShutdownTimerJSON` | **Autorisé en lecture** | `conf:105` ; `dbus-policy-test.sh:165-167`. |
| Setter de minuterie | **Aucun** | Aucun `SetShutdown*` ; `Settings.shutdownTime` via `Admin1.SaveSettingsJSON`. |
| `Admin1.SaveSettingsJSON` | **Polkit `settings`** | `policy:53-65` (`auth_admin`, pas de `_keep`). |
| `Manager1` setters | **Deny** | 8 membres `deny` (`conf:111-138`) ; policy test attend le refus. |
| LUKS | **Absent de la policy** | Aucune action `luks*`, aucun membre `Luks*` ; seul commentaire `conf:178`. |
| `GetShutdownTimerJSON` sans polkit | **Acceptable** | Lecture seule, information de planification ; pas de secret. |

---

## 4. Actions bloquantes

1. **SF-01** : retirer l'écran LUKS/QML et ses chaînes, ou le remplacer par une note sans champ de saisie ; aucun secret QML, aucun faux succès. **Bloquant pour la cohérence de release LUKS.**
2. **SF-02** : corriger l'effacement de l'erreur « heure invalide » quand une planification précédente est annulée.
3. **SF-03** : sur refus de désactivation du blocker, libérer le fd ou garantir une nouvelle tentative ; ne pas laisser une inhibition silencieuse.

Les SF-04…SF-10 sont des améliorations/documentation ; SF-08 est le risque inter-clients déjà connu (S1-02).

---

## 5. Limites manuelles

- **Vrai logind** : `ScheduleShutdown`/`CancelScheduledShutdown` (remplacement d'une échéance existante, annulation externe), lecture de l'état, comportement après reprise.
- **DST réelle** : `TZ=Europe/Paris` autour des transitions de mars/octobre ; vérifier l'échéance affichée et `usec`.
- **Session graphique réelle** : X11/Wayland, session inactive/verrouillée, `Inhibit` autorisé/refusé ; logind redémarré (SF-04).
- **Fermeture GUI** : fd libéré (vérifier `loginctl inhibit`/`systemd-inhibit --list`), queue pleine (SF-03).
- **Matériel** : charge profil/priorité/seuils (capacités réelles), matrice PPD × AC/BAT avec profils personnalisés.
- **Deux clients** : conflit de settings (SF-08), `Admin1` réel, polkit.
- **LUKS** : aucun test, fonctionnalité retirée ; vérifier l'absence d'écran et de chaînes après SF-01.
