# Revue de sécurité — backend de session GNOME/Mutter

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | backend Wayland GNOME de `tccd-user` (`org.gnome.Mutter.DisplayConfig`) |
| Périmètre | `docs/security/**` uniquement — aucun code modifié |
| Code lu | `crates/tcc-user/src/backend/gnome.rs`, `backend/mod.rs` (`create`, `plan_refresh`), `crates/tcc-hw/src/display.rs` (`mode_for_rate`, `apply_internal_refresh_rate`) |
| Preuves exécutées | `cargo test -p tcc-user --lib` ⇒ **90/90**, dont **12** tests GNOME (fake Mutter : apply interne, serial périmé, refus, mode absent, timeout, parsing). Aucune session GNOME réelle. |
| Verdict | **Conforme, aucun finding Critique/Élevé.** 4 Info. Le backend est non privilégié, n'écrit que l'écran interne, préserve les écrans externes et borne les appels. |

---

## 1. Constats vérifiés

| Point | État | Preuve |
|---|---|---|
| **Écran interne uniquement** | **Conforme** | `ensure_internal(output)` + `validate_internal_output_name` avant tout apply (`gnome.rs:414-423`) ; `plan` ne remplace que le mode du connecteur interne (`:233-300`) ; test `plan_preserves_external_mode`. |
| **Écrans externes préservés** | **Conforme** | Le mode courant de chaque autre connecteur est relu ; s'il est absent, l'application est **refusée** plutôt que modifiée (`:255-275`) ; test dédié. |
| **Pas de persistance compositeur** | **Conforme** | `method = 1` (`temporary`), jamais `method = 0` (`:48-49,355-370`). |
| **Serial Mutter** | **Conforme** | `GetCurrentState` renvoie le serial ; `ApplyMonitorsConfig(serial, …)` ; sur `AccessDenied`/`InvalidArgs`, relecture + **une seule** tentative (`:426-445`) ; test `stale_serial_retries_once`. |
| **Timeouts** | **Conforme** | `DBUS_TIMEOUT=3 s` sur `GetCurrentState`, `ApplyMonitorsConfig` et la création du proxy (`:47,342-370,378-395`) ; test `timeout_is_bounded`. |
| **Validation des entrées** | **Conforme** | Identifiant de mode ≤ 256, sans caractère de contrôle (`:455-470`) ; modes Mutter filtrés (id vide/trop long, dimensions ≤ 0, taux non fini/non positif) (`:120-160`) ; test `malformed_modes_are_filtered`. |
| **Pas de commande / shell** | **Conforme** | Uniquement D-Bus session ; aucun `Command`, aucun chemin, aucun `env` hérité. |
| **Pas de privilège** | **Conforme** | Agent utilisateur (`tccd-user`), `CapabilityBoundingSet=` vide, aucune action root. |

---

## 2. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **G-01** | Info | `gnome.rs:444-446` ; `backend/mod.rs:45-55` | `GnomeBackend::is_available()` retourne **toujours `true`** ; `create` choisit le backend selon la session, pas selon la présence de Mutter. Si Mutter est absent (session GNOME X11 mal classée, service arrêté), `list_outputs`/`apply` échouent avec `NotSupported`/`Display`, pas `Unsupported`. | L'UI peut afficher une erreur au lieu de « non supporté » ; pas de privilège, pas d'écriture. | Faire de `is_available` un vrai probe léger (nom Mutter possédé) ou retourner `Unsupported` sur `ServiceUnknown` dans `apply_internal_refresh_rate`. | Test fake sans service ⇒ `Unsupported`/message dédié. |
| **G-02** | Info | `gnome.rs:120-190,342-355` | `GetCurrentState` est désérialisé sans borne explicite du nombre de moniteurs/modes/logical monitors ; un compositeur de session malveillant (même utilisateur) peut provoquer une allocation/CPU importante dans l'agent. | DoS local de l'agent non privilégié ; pas d'escalade, pas d'accès aux autres sessions. | Plafonner (ex. 32 moniteurs, 512 modes, 64 logical) et ignorer/rejeter au-delà ; ou s'appuyer sur la limite zbus et documenter. | Fake Mutter avec 10⁶ modes ⇒ rejet borné. |
| **G-03** | Info | `gnome.rs:467-500` | `map_dbus_error` inclut `{other}` (texte zbus) dans `HwError::Display`, affiché/loggué. Pas de secret, mais un nom de service/chemin D-Bus peut apparaître. | Fuite d'information locale mineure. | Mapper par catégorie (`NotSupported`, `Display`) sans `{other}` ; journaliser le détail côté agent. | Test : erreur `ServiceUnknown` ⇒ message sans nom de service. |
| **G-04** | Info | `gnome.rs:196-204` | Bloc `if !monitor.builtin && output.internal { }` vide (commentaire seul) ; code mort. | Aucun impact sécurité ; ambiguïté de lecture. | Supprimer le bloc ou le remplacer par un log de cohérence. | Revue. |

---

## 3. Limites

- **Aucune session GNOME réelle** : les tests utilisent un faux Mutter sur bus de session ; le comportement réel (Wayland, fractional scaling, multiples moniteurs, modes préférés) reste à valider.
- **Mutter peut changer son API** : `GetCurrentState`/`ApplyMonitorsConfig` sont stables mais non versionnés ; le test fake ne couvre pas une futur version.
- **Properties de moniteur** : `ApplyMonitorsConfig` reçoit des propriétés de moniteur vides ; Mutter peut réinitialiser des propriétés non liées au mode (ex. under-scanning) sans que les tests le détectent. À vérifier sur session réelle.
- **Serial** : la course entre `GetCurrentState` et `ApplyMonitorsConfig` est couverte par un retry unique ; un compositeur qui change en boucle peut faire échouer l'application, sans risque de sécurité.
- **Pas de test de session verrouillée** : l'agent ne s'applique pas si la session est absente ; le verrouillage n'est pas traité (FEATURE_PARITY).
