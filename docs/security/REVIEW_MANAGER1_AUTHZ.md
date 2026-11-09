# Revue de sécurité — autorisation polkit des setters `Manager1`

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue de l'autorisation `tccd1.charging` / `tccd1.settings` des 8 setters `Manager1` de `tccd` |
| Périmètre | `docs/security/**` uniquement — aucun code modifié |
| Code lu | `crates/tcc-daemon/src/dbus.rs` (`SettersAuthorizer`, `PolkitSettersAuthorizer`, `AllowAllAuthorizer`, `prepare_write`, 8 setters, `map_hw_error`), `main.rs` (injection `--dev-session`), `rate_limit.rs`, `sensor_lease.rs`, `hardware/lighting.rs`, `crates/tcc-authz/src/lib.rs` |
| Intégration lue | `packaging/dbus/com.tuxedocomputers.tccd.conf`, `packaging/polkit/com.tuxedocomputers.tccd.policy`, `scripts/release/dbus-policy-test.sh` |
| Preuves exécutées | `cargo test -p tcc-daemon --bins` sous `dbus-run-session` ⇒ **152/152**, dont quota UID, DialogGate, lighting, `map_hw_error`, audit de complétion, veille. Vrai polkit, root réel et bus système **non testés**. |
| Statut | **Chaîne d'autorisation correcte et fail-closed.** 2 findings Moyens (dialog storm/quotas, interrupteur global lighting contourné par la policy ouverte), 2 Faibles, 4 Info. Aucun Critique/Élevé. **Correctifs 2026-11-06 : M1-01…M1-05 fermés ; M1-06/M1-08 restent Info.** |

---

## Statut des correctifs (2026-11-06)

> Instantané après correction. Les constats de §1/§2 restent l'état d'origine ; ce tableau est la
> source de vérité. Preuve exécutée : `cargo test -p tcc-daemon --bins` sous `dbus-run-session`
> ⇒ **152/152**. Vrai polkit, root et bus système **non rejoués**.

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **M1-01** | **Fermé** | `HardwareManager.writes` est un `tcc_authz::AuthorizationLimiter` (nom **+ UID**) et `authz_gate` un `DialogGate` (`dbus.rs:546-550,588-592,634-655`). Tests `uid_bucket_limits_multiple_connections` (10 acceptés, puis `LimitsExceeded` partagé par UID) et `concurrent_dialog_is_refused_by_gate` (second dialogue refusé). La policy est repassée en **deny** (`com.tuxedocomputers.tccd.conf:111-138`) et `dbus-policy-test.sh:165-175` attend le refus. |
| **M1-02** | **Fermé** | `ensure_lighting_enabled` est appelé dans `SetKeyboardStatesJSON` et `SetLightbarStatesJSON` avant `prepare_write` (`dbus.rs:612-625,1134,1169`) ; test `lighting_switch_disables_keyboard_and_lightbar_setters` (aucun dialogue, état inchangé). |
| **M1-03** | **Fermé (client)** | `map_hw_error` renvoie un message générique sans chemin (`dbus.rs:1351-1373`) ; test `hw_error_mapping_hides_paths` (aucun `/sys` ni `/`). Le log serveur conserve `%error` (peut contenir un chemin) : à trancher si la règle « aucun chemin en log » s'applique aussi aux warnings. |
| **M1-04** | **Fermé** | `HardwareBridge::production` n'accepte aucun authorizer ; `dev_session` force `dry_run: true` + `DevSessionAuthorizer` ; `test_with_authorizer*` est `#[cfg(test)]` ; `HardwareManager::new` a `debug_assert!(authorizer.is_none() || dry_run || cfg!(test))` (`dbus.rs:395-470,577-581`) ; tests `production_bridge_is_polkit_only`, `dev_session_bridge_forces_dry_run_and_authorizer` + garde source `dev_session_authorizer_is_only_wired_for_dev_session`. |
| **M1-05** | **Fermé** | `AuditSink` injectable (`dbus.rs:353-362`) ; `audit_completion` sur les 8 setters ; tests `completion_audit_records_success` (result `ok`, method/action/caller/uid/duration) et `completion_audit_records_hardware_error` (result `error`), sans payload. |
| **M1-06** | **Ouvert (Info)** | Couverture : 152/152 tests daemon. Manquent : vrai polkit, root réel, bus système, timeout 120 s, écriture matérielle non `dry_run`. |
| **M1-08** | **Ouvert (Info)** | `caller_identity` (résolution UID) précède toujours la validation ; des appels invalides peuvent provoquer une résolution D-Bus par appel, sans dialogue ni écriture. |

**Verdict après correctifs** : **M1-01…M1-05 sont fermés côté code et tests**. La policy reste en
**deny** ; sa réouverture est **possible** après les preuves système (vrai polkit autorisé/refusé/
annulé/indisponible, root, `dbus-policy-test.sh` en root, écriture matérielle réelle). M1-06/M1-08 sont
des réserves de couverture/coût, non des blocages de code.

### Réouverture de la policy des 8 setters — verdict

**Oui sur le plan code**, **non sans les preuves système**. Conditions cumulatives avant de remplacer
les 8 `deny` par 8 `allow` membre par membre (aucun allow d'interface, `Properties` toujours absent) :

1. vrai polkit sur bus système : autorisé / refusé / annulé / polkit arrêté, pour `SetChargingProfile`
   et `SetFnLockStatus` au minimum ;
2. root réel : `CheckAuthorization` appelé, aucun court-circuit, résultat selon polkit ;
3. `dbus-policy-test.sh` exécuté **en root** avec les 8 `allow` (et non plus les `deny`) ;
4. écriture matérielle réelle (charge, clavier, lightbar, Fn-Lock) sur laptop TUXEDO, sans `dry_run` ;
5. audit journald : lignes `authorized` + `completion` (`ok`/`error`), `caller_uid`, sans payload ;
6. test multi-connexions même UID + `DialogGate` rejoué sur le bus système ;
7. revue `security` du diff de policy (8 lignes) et de l’unité, puis mise à jour de `CHECKLIST.md`.

Bloquants restants : **aucun blocage de code** ; les points 1–6 sont des preuves manuelles/root, et
le point 7 une revue. Si l’un manque, garder la policy en deny.

---

## 1. Verdict

Le chemin d'écriture est bien ordonné : **identité → validation → rate limit → polkit → écriture**. Le
sujet est `system-bus-name` du nom unique, l'UID est résolu avec timeout, root **n'est pas
court-circuité**, et polkit absent/timeout/refus produit `AccessDenied` sans écriture (vérifié par
tests). Les entrées invalides ne déclenchent ni jeton ni dialogue. `AllowAllAuthorizer` n'est injecté
que si `--dev-session` (bus de session + `dry_run`), et aucun chemin de production ne l'utilise.

Deux points doivent être traités **avant de considérer la policy ouverte comme sûre** :
1. **M1-01** : pas de bucket par UID ni de `DialogGate` ; la policy autorise désormais les setters au
   bus, donc un utilisateur peut ouvrir plusieurs connexions et multiplier les dialogues polkit
   (H-05, resté ouvert).
2. **M1-02** : la policy ouverte réactive H-06 — les setters clavier/lightbar **ignorent**
   `keyboard_backlight_control_enabled` et écrivent quand même (après polkit) alors que l'utilisateur
   a désactivé le contrôle.

---

## 2. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **M1-01** | **Moyen / B** | `rate_limit.rs:14-22,78-118` ; `dbus.rs:460-485` ; `dbus.rs:373-390` ; `tcc-authz/src/lib.rs:465-546,548-650` ; `packaging/dbus/…conf:107-137` | Le limiteur d'écriture est **par nom unique** uniquement (5/s, burst 10) et `PolkitSettersAuthorizer` appelle `check_authorization` **sans `AuthorizationLimiter` ni `DialogGate`**. Depuis que la policy autorise les 8 setters au bus, un utilisateur peut ouvrir N connexions (`:1.x` distinctes), obtenir N buckets et déclencher N dialogues polkit ; le `DialogGate` de `tcc-authz` (un seul dialogue global) n'est pas utilisé par `tccd`. | **Dialog storm / DoS**, harcèlement de l'utilisateur par des prompts polkit répétés, charge sur le bus ; pas d'escalade (chaque écriture reste autorisée par polkit). H-05 reste ouvert. | Ajouter un bucket **par UID** (en plus du nom unique) et un `DialogGate` global autour de `CheckAuthorization` (attente 2 s, `Busy` au-delà), avec seuils adaptés (ex. 2/s burst 4 pour les setters). Réutiliser `tcc-authz::AuthorizationLimiter`/`DialogGate` ; auditer `rate_limited`/`busy`. | Test multi-connexions même UID : N appels ⇒ nombre de dialogues borné, `LimitsExceeded`/`Busy` au-delà ; test 2 s de gate. |
| **M1-02** | **Moyen / B** | `dbus.rs:876-930` (`SetKeyboardStatesJSON`/`SetLightbarStatesJSON`) ; `dbus.rs:1030-1062` (`apply_keyboard_states`/`apply_lightbar_state`) ; `hardware/lighting.rs:49-56,229` ; `packaging/dbus/…conf:129-136` ; `REVIEW_HARDWARE.md` H-06 | La policy **autorise** désormais les setters lighting au bus. Or ces setters appellent directement `apply_keyboard_states`/`apply_lightbar_state` **sans** `ensure_control_enabled(&settings)`. Le worker lighting respecte l'interrupteur global (`hardware/lighting.rs:229`), mais le chemin D-Bus le contourne : un utilisateur authentifié peut modifier clavier/lightbar alors que `keyboard_backlight_control_enabled = false`. | Contournement d'un réglage utilisateur global (intégrité/consentement), régression H-06 que le deny bus neutralisait jusqu'ici. Pas d'escalade de privilège (polkit requis). | Ajouter `ensure_control_enabled` (ou un équivalent) dans les deux setters **avant** l'écriture, et refuser avec `NotSupported`/`AccessDenied` si le contrôle est désactivé ; ou maintenir le deny bus tant que H-06 n'est pas corrigé. Tester le cas « contrôle désactivé + setter ⇒ refus, aucune écriture ». | Test D-Bus : `keyboard_backlight_control_enabled=false`, `SetKeyboardStatesJSON` ⇒ refus, état inchangé, hardware non appelé (fake). |
| **M1-03** | **Faible / A** | `dbus.rs:1069-1081` (`map_hw_error`) | Les erreurs `HwError` sont converties en `fdo::Error` avec `error.to_string()`, qui contient le **chemin sysfs** pour `NotFound`, `InvalidPath`, `UnsafeTarget`, `UnsafePermissions`, `Io`. Le message est renvoyé au client D-Bus. | Fuite d'information locale (chemins `/sys/...`) vers tout appelant autorisé ou non ; contraire à la règle « messages génériques, pas de chemin » de l'admin/daemon. Pas de secret. | Mapper par catégorie (`NotSupported`, `InvalidArgs`, `Failed`) avec un message fixe ; journaliser le détail côté daemon avec `method`, `caller_uid`, `action`. | Test : injecter un `HwError::NotFound { path }` via un fake et vérifier que la réponse ne contient ni `/` ni `/sys`. |
| **M1-04** | **Faible / A** | `dbus.rs:366-401` (`SettersAuthorizer`, `AllowAllAuthorizer`) ; `dbus.rs:351-352` (`HardwareBridge.authorizer`) ; `main.rs:404-425` | `AllowAllAuthorizer` est **public** et `HardwareBridge.authorizer` est public : n'importe quel code du crate peut l'injecter. Aujourd'hui seul `main.rs` le fait sous `args.dev_session` (qui force le bus de session et `dry_run`), et aucun chemin de production n'a été trouvé. La garantie est **conventionnelle**, pas de type. | Une refactorisation ou une future option CLI pourrait ouvrir les setters sans polkit en production. Risque latent, pas actuel. | Rendre le type `pub(crate)`/`#[cfg(any(test, feature = "dev-session"))]`, le renommer `DevSessionAuthorizer`, ajouter un `debug_assert!(dry_run)` à la construction et un test CI qui vérifie qu'aucune unité installée ne passe `--dev-session` et que le type n'apparaît que dans `main.rs`/tests. | `grep` CI : `AllowAllAuthorizer` uniquement dans `main.rs` (sous `dev_session`) et tests ; `validate.sh` sur les units. |
| **M1-05** | Info | `dbus.rs:487-534` (`authorize_setter`), `:539-572` (`prepare_write`) ; setters `:722-955` | L'audit enregistre `authorized` **avant** l'écriture, mais **pas** le résultat final (`ok`/`error`) avec `method`, `caller_uid`, `action`. Un échec matériel est renvoyé au client sans ligne d'audit de completion. | Traçabilité incomplète : on sait qui a été autorisé, pas si l'écriture a abouti. Pas de fuite (les logs ne contiennent pas de payload). | Une ligne de completion par setter (`result=ok/error`, `duration_ms`, `method`, `action`, `caller_uid`) après `blocking_hw`/`update_settings`, sans valeur ni chemin. | Test d'audit : succès et échec matériel produisent deux lignes distinctes, aucune valeur. |
| **M1-06** | Info | `dbus.rs:1545-1665` (tests) | Couverture : refus polkit, autorisé, entrée invalide avant polkit, polkit indisponible, action Fn-Lock. **Manquent** : rate limit (nom/UID), mapping d'action des 8 setters, root réel, vrai polkit, timeout 120 s, écriture matérielle réelle. | Une régression sur `SetChargeType`/`SetKeyboardStatesJSON` (mauvais action ID) ou sur les quotas ne serait pas détectée. | Test paramétré sur les 8 setters vérifiant l'action (`charging`/`settings`) et le refus ; test « 100 appels invalides ne consomment pas le bucket » ; test root/polkit réel en intégration manuelle. | `dbus.rs` tests + `dbus-policy-test.sh`. |
| **M1-07** | Info | `packaging/dbus/…conf:100-136` ; `dbus.rs:679-719` | `SetSensorDataCollectionStatus` est autorisé au bus **sans polkit** (seulement rate-limité) : il arme le bail capteurs 5 min (H-02). Ce n'est pas un des 8 setters matériels, mais c'est une méthode d'écriture au sens large. | Un utilisateur local peut maintenir la collecte active (bornée 5 min, lectures 2/s) ; coût CPU/`nvidia-smi` déjà mitigé. Pas dans le périmètre des 8 setters. | Documenter l'exception ; si le bail devient coûteux, appliquer polkit `tccd1.settings` ou un quota par UID. | Test de quota multi-connexions. |
| **M1-08** | Info | `dbus.rs:539-572` | `caller_identity` (dont `GetConnectionUnixUser`, timeout 500 ms) est exécuté **avant** la validation ; une entrée invalide ne consomme pas le bucket et peut donc provoquer des résolutions d'UID en boucle. | Coût/DoS léger (une résolution D-Bus + un log par appel invalide), pas d'écriture ni de dialogue. | Valider d'abord les arguments purs (bornes/types) puis résoudre l'identité, ou plafonner aussi les requêtes invalides par connexion. | Test de 100 appels invalides : pas de dialogue, coût borné. |

---

## 3. Conformité vérifiée

| Point demandé | Verdict | Preuve |
|---|---|---|
| **Ordre identité → validation → rate limit → polkit → écriture** | **Conforme** | `prepare_write` (`dbus.rs:539-572`) : `caller_identity`, `validate()`, `rate_limit`, `authorize_setter`, puis le setter écrit. Les setters n'écrivent qu'après `prepare_write` (`:722-955`). |
| **Sujet `system-bus-name`** | **Conforme** | `tcc-authz` construit `("system-bus-name", {"name": <unique>})` (`lib.rs:370-380`) ; test `subject_and_flags_are_forwarded_without_cache` ; jamais `unix-process`. |
| **Pas de court-circuit root** | **Conforme** | `AuthorizationRequest::with_caller_uid(0)` passé à polkit ; `tcc-authz` teste `root_is_not_short_circuited` ; `authorize_setter` n'a aucune branche `uid == 0`. |
| **Fail-closed polkit absent/timeout/refus** | **Conforme** | `check_authorization` convertit timeout/indisponibilité en `Denied` (`lib.rs:325-367`) ; `PolkitSettersAuthorizer` mappe tout `Ok(_)` non autorisé et `Err(_)` en `AccessDenied` (`dbus.rs:377-390`) ; test `unavailable_polkit_fails_closed`. |
| **`AllowAllAuthorizer` limité à `--dev-session`** | **Conforme avec réserve** | Injection conditionnelle `args.dev_session.then(...)` (`main.rs:404-425`) ; `dry_run` inclut `dev_session` (`main.rs:317`) ; bus de session forcé. Aucun chemin de production trouvé. Réserve M1-04 (type public). |
| **Audit sans payload** | **Conforme partiellement** | Logs `invalid_args`, `rate_limited`, `denied`, `authorized` avec `caller`, `caller_uid`, `method`, `action`, `result` ; aucun payload. Réserve M1-05 (pas de completion `ok/error`). |
| **Aucune écriture si refus/invalidité** | **Conforme** | Tests : `setter_denied_by_polkit_does_not_write` (état inchangé, `AccessDenied`), `invalid_input_is_rejected_before_polkit` (0 appel authorizer), `unavailable_polkit_fails_closed` (révision inchangée). |
| **Rate limit conservé** | **Partiel** | Bucket 5/s burst 10 par nom unique (`dbus.rs:460-485`, `rate_limit.rs`) et audit UID ; **pas de bucket UID ni de `DialogGate`** (M1-01). |
| **Policy : membres exacts** | **Conforme** | 8 `<allow … send_member="Set…"/>` (charge ×5, clavier, lightbar, Fn-Lock) (`conf:107-136`) ; aucun `allow send_interface="…Manager1"` global ; `Properties` absent (`conf:54-57`) ; actions `tccd1.charging`/`.settings` `auth_admin` (`policy:53-65,83-95`). |
| **Policy test** | **Conforme** | `dbus-policy-test.sh:165-175` vérifie les 8 setters autorisés au bus ; **exécution root non rejouée**. |
| **Tests** | **Partiel** | 131/131 tests daemon ; 5 tests `Manager1` (refus, autorisé, invalide, indisponible, Fn-Lock). Manquent M1-06. |
| **Écriture réelle** | **Non testée** | Les tests `Manager1` utilisent `dry_run=true` et un `SysfsRoot` inexistant ; le vrai chemin `blocking_hw` n'est pas exercé. |

---

## 4. Actions bloquantes

1. **M1-01** — ajouter bucket **UID** + `DialogGate` avant de considérer la policy ouverte comme sûre
   (H-05). Sans cela, un utilisateur peut multiplier les dialogues polkit via plusieurs connexions.
2. **M1-02** — faire respecter `keyboard_backlight_control_enabled` par `SetKeyboardStatesJSON` et
   `SetLightbarStatesJSON` (ou garder le deny bus) : la policy ouverte réactive H-06.
3. **M1-03** — ne plus renvoyer de chemin sysfs dans les erreurs D-Bus (mapping générique + log
   serveur).
4. **M1-04** — verrouiller `AllowAllAuthorizer` (visibilité/feature/test CI) pour garantir
   « jamais en production » au niveau du code, pas seulement de `main.rs`.
5. **M1-05** — compléter l'audit de completion (`ok`/`error`) sans payload.

---

## 5. Limites

- **Vrai polkit non testé** : le `PolkitSettersAuthorizer` de production (bus système, dialogue,
  timeout 120 s) n'est pas exercé ; les tests injectent `FakeAuthorizer`.
- **Root non testé** : le comportement polkit pour `uid 0` dépend des defaults de la distribution ;
  le code ne court-circuite pas, mais aucune preuve réelle n'est produite.
- **Bucket par UID** : la revue confirme l'absence ; le correctif nécessite `tcc-authz` ou une
  modification de `rate_limit.rs` (hors périmètre `security`).
- **Policy** : `dbus-policy-test.sh` n'a pas été exécuté en root ; la policy a été lue, pas testée sur
  bus système.
- **Matériel** : aucun setter réel n'a été exécuté ; `dry_run=true` dans tous les tests.
- **H-06** : la revue ne corrige pas le comportement lighting ; elle constate que la policy ouverte le
  rend atteignable et demande le correctif avant release.
