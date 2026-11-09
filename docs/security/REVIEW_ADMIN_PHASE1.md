# Revue de sécurité — `tccd-admin` phase 1

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue ciblée de l'implémentation `tccd-admin` phase 1 (`Admin1`, `tcc-authz`, `Control1`, transaction) |
| Périmètre | `crates/tcc-authz/src/lib.rs`, `crates/tcc-admin/src/**`, `crates/tcc-daemon/src/dbus.rs` (§ `Control1`), `crates/tcc-core/src/config.rs` (`save_*_with_backup`), `crates/tcc-ipc/src/admin.rs`, `packaging/{dbus,polkit,systemd}`, `docs/security/ADMIN_DESIGN.md`, delta jalon 2 de `THREAT_MODEL.md` |
| Statut | **implémentation solide ; 1 finding Moyen, 6 Faibles, 3 Informations. Aucune vulnérabilité Critique ou Élevée.** L'activation de `TCC_WITH_ADMIN=1` reste bloquée par **A1-01** et par l'absence de preuve de bout en bout (polkit réel, bus système, `Control1` réel). |

---

## Statut des correctifs (2026-11-06)

> **Instantané ~16:10.** Les constats des §1-§9 restent l'état d'origine (revue du matin) ; ce tableau
> est la source de vérité sur les correctifs. Les compteurs de tests et de `validate.sh` cités dans
> les §2/§5/§6 sont ceux de la revue initiale ; les valeurs courantes sont celles ci-dessous. `Fermé` = correctif présent et vérifié dans le code ou
> par un test ciblé ; `Partiel` = une partie subsiste ; `Ouvert` = inchangé.
>
> **Rejoué** : `tcc-authz` **19**, `tcc-admin` lib **33**, binaire **5**, `admin_dbus` **12**,
> CLI **4**, transaction **12** (compteurs admin mis à jour avec l'addendum T) ; `tcc-daemon`
> (bins) **114** ; `tcc-core` lib **159** ; `validate.sh --repo` **220/220** ;
> `systemd-analyze security` admin **1.6**. **Non rejoué** : `dbus-policy-test.sh` (root), vrai
> polkit, vrai `Control1`, `cargo deny/audit`.

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **A1-01** | **Fermé** | Cache supprimé : `Control { reload_tx, reload_lock }`, `request_reload` sérialisé sans mémorisation (`tcc-daemon/src/dbus.rs:893-935`). Tests `control_reload_always_requests_a_fresh_report`, `control_reload_reflects_a_write_between_calls`, `control_reload_serializes_concurrent_calls` (114 tests daemon). Limite : le test « écriture entre deux » utilise un serveur simulé, pas la boucle réelle de `tccd` — le rollback réel reste manuel (§8). |
| **A1-02** | **Fermé** | `CallerSlot` RAII + `DialogPermit` : l'entrée `in_flight` est retirée au `Drop`, y compris sur annulation (`tcc-authz/src/lib.rs:583-650`) ; test `cancelled_dialog_releases_the_caller_slot` (19 tests). |
| **A1-03** | **Fermé** | `Activity::enter`/`ActiveCall` protègent un appel long ; `touch()` n'est appelé qu'après `Authorized` (`tcc-admin/src/service.rs:58-135,350,546,592,635`). Tests `denied_call_does_not_extend_activity`, `authorized_call_extends_activity` (24 tests lib). Vérifié : `Ping`, `GetVersion` et le getter `ContractVersion` ne touchent plus l'activité. |
| **A1-04** | **Fermé** | `read_previous` vérifie régulier, `expected_owner`, `mode & 0o022 == 0`, borne, et ouvre `O_NOFOLLOW|O_CLOEXEC` (`tcc-admin/src/transaction.rs:278-312`) ; `restore_previous` fait `fchmod 0644` sur le fd **avant** `rename` (`:321-368`). Tests `group_writable_previous_is_refused`, `previous_with_unexpected_owner_is_refused`, `symlink_is_refused`, `oversize_previous_is_refused`, + tests `tcc-core` de mode/propriétaire. |
| **A1-05** | **Fermé** | `SavedInfo.applied` + `reload_label()` → `ok`/`skipped` (`transaction.rs:81-99`), audit `reload=…` (`service.rs:452-456,652-658`). Tests `audit_label_distinguishes_noop_and_write`, `preset_only_payload_is_a_noop`. |
| **A1-06** | **Fermé** | Policy Admin1 : `Ping`, `GetVersion`, `GetContractVersion`, `SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig` autorisés un par un ; **aucun allow `Properties`** sur `.Admin` (A1-11 corrigé) ; `Properties.Set` absent ; phases 2/3 toujours refusées (`packaging/dbus/…conf:148-202`). `validate.sh` 220/220. Script `dbus-policy-test.sh` étendu (22 vérifications nommées, Ping/GetVersion/Properties inclus) — **exécution root non rejouée** ; le « 38/38 » annoncé par `packaging-release` n'est pas reproduit ici. |
| **A1-07** | **Fermé** | `Saved.preserved_backup` (`tcc-core/src/config.rs:281-292`) ; `.bak` réécrit seulement si le précédent est désérialisable **et** valide (`save_typed_with_backup:876-898`) ; `.bak.corrupt` 0600, une génération (`:1033-1055`) ; `BackupStatus::{None,Written,Preserved}` et rollback désactivé si précédent inexploitable (`transaction.rs:127-135,246-270`). Tests : `corrupt_current_preserves_healthy_backup`, `corrupt_previous_is_not_restored_after_reload_failure`, + 7 tests `tcc-core` (corrompu, mauvais type, validation, surdimensionné, mode insécable, `.bak` jamais écrasé). |
| **A1-08** | **Ouvert** (Info) | Une nouvelle connexion au bus système par vérification polkit (`tcc-authz/src/lib.rs:288`) et par rechargement (`tcc-admin/src/service.rs:236`). Coût uniquement, aucun impact de sécurité. |
| **A1-09** | **Fermé** (documentation) | Documentation mise à jour : setters `Manager1` explicitement « non polkit, non persistants, fermés au bus » (`HARDWARE_MATRIX.md:501`, `packaging/SECURITY.md:227,377`) ; deny toujours en place (`conf:111-138`). Le risque résiduel TR-02 demeure : ne pas rouvrir sans `CheckAuthorization`. |
| **A1-10** | **Ouvert** (Info) | Vrai polkit, policy système et vrai `Control1` toujours non exercés par les tests (authorizer/reloader injectés). Checklist §8. |

### Nouveaux findings (2026-11-06)

| ID | Sév. | Fichier:ligne | Constat | Remédiation |
|---|---|---|---|---|
| **A1-11** | **Fermé** (Faible) | `packaging/dbus/…conf:150-171` ; `tcc-admin/src/service.rs` `GetContractVersion` | `Properties.Get`/`GetAll` n'étaient pas restreints à une interface : toute propriété future de l'objet Admin aurait été lisible sans polkit. **Corrigé** : aucun allow `Properties` sur `.Admin` (le deny global s'applique) ; la version passe par `Admin1.GetContractVersion` (allow explicite, `conf:163-165`). Reste à confirmer en root que `Properties.Get/GetAll` sont refusés. | Garde CI : `grep` interdit `Properties.Get`/`GetAll` pour `.Admin` ; test root dans `dbus-policy-test.sh`. |
| **A1-12** | Info | `tcc-core/src/config.rs:1033-1055` | `.bak.corrupt` : 0600 root, une génération, `rename` sans suivre le lien ; **aucune borne de taille** à la quarantaine (déplacement, pas lecture) et la preuve précédente est **écrasée** à chaque nouvelle corruption. Répertoire root-only : risque nul en pratique. | Documenter (une génération, 0600) ; ajouter éventuellement un compteur/rotation si la conservation de plusieurs preuves devient un besoin. |
| **A1-13** | Info | `tcc-admin/src/transaction.rs:278-312` | Un fichier précédent illisible, d'un autre uid ou avec `mode & 0o022 != 0` fait échouer **toute** écriture (`WriteFailed`) avant la transaction : fail-closed, mais l'API ne peut pas réparer un fichier chmodé par erreur (intervention root hors API). | Documenter dans `DBUS_API.md`/`ADMIN_DESIGN` ; message d'erreur générique déjà en place. |
| **A1-14** | Info | `tcc-admin/src/transaction.rs:222-242` | Si le précédent n'est pas exploitable, le rollback est désactivé : après un `ReloadConfig` en échec, le **nouveau** fichier (validé) reste sur disque et l'appelant reçoit `ReloadFailed`. Comportement explicite mais à ne pas confondre avec « rien n'a changé ». | Documenter dans l'API (état du fichier sur `ReloadFailed`) et dans `ADMIN_DESIGN` §4.2. |

---

## 1. Résumé exécutif

### 1.1 Verdict par domaine

| Domaine | Verdict | Commentaire |
|---|---|---|
| **Polkit** (`tcc-authz`) | **Conforme** | Sujet `system-bus-name` du nom unique, jamais `unix-process` ; flags `AllowUserInteraction` ; `is_challenge` refusé ; absence/erreur/timeout ⇒ refus ; pas de court-circuit root ; pas de cache ; limiteur **nom + UID** ; dialogue unique. 18 tests. |
| **`Admin1`** (`tcc-admin`) | **Conforme avec réserves** | Ordre identité → taille → limiteur → polkit → verrou → validation → transaction effectivement respecté. Borne 1 Mio **après** allocation zbus (résiduel). Presets filtrés, références croisées, erreurs sans chemin, audit sans payload, refus non-root, sortie après 30 s. |
| **Transaction/rollback** | **Conforme** | `.bak` 0600, cible 0644, écriture atomique `fsync`/`rename`, restauration octet pour octet, pas de temporaire résiduel, rollback sur échec de rechargement **et** sur erreur de transport. Un défaut de garantie : **A1-01** (rapport de rechargement périmé). |
| **`Control1`** | **Conforme avec réserve** | Root revérifié dans le code (`GetConnectionUnixUser == 0`), policy root-only, rapport sans chemin, timeout 30 s, réponse honnête (`ok=false`) en cas d'échec/arrêt. La **coalescence 500 ms** peut renvoyer un rapport antérieur à l'écriture (A1-01). |
| **Policy D-Bus / polkit** | **Conforme** | Membres `Admin1` listés un par un, phase 2/3 explicitement refusées, `Control1` root-only, aucun `Properties.Set`, aucun allow d'interface, aucun `exec.path`/`allow_gui`, IDs d'action cohérents (`profiles`/`settings`). Petit écart : `ContractVersion`/`Ping`/`GetVersion` refusés (A1-06). |
| **Unité systemd** | **Conforme** | `PrivateDevices=yes`, `DevicePolicy=closed`, capacités vides, `ProtectSystem=strict`, `ReadWritePaths=/var/lib`, `MemoryMax=192M`, `TasksMax=16`, `LimitCORE=0`, `Restart=no`, `RuntimeMaxSec=5min`, pas de `StateDirectory`/`[Install]`. Score **1.6 OK**. |
| **Tests** | **Bons mais partiels** | 18 (`tcc-authz`) + 18 (lib admin) + 5 (binaire) + 7 (`admin_dbus`, désormais **parallèle-safe**) + 4 (CLI) + 5 (transaction). Les refus, le rollback et `Busy` sont prouvés **avec des backends injectés** ; le vrai polkit, la policy système et le vrai `Control1` ne sont pas exercés. |

### 1.2 Les trois points à retenir

1. **A1-01 (Moyen)** — la coalescence de `Control1` peut renvoyer le rapport d'un rechargement **commencé avant l'écriture**. L'admin conclut au succès sans rollback alors que `tccd` garde l'ancienne configuration en mémoire. Cela **annule la garantie transactionnelle** et peut laisser des limites matérielles (charge/TDP) différentes de ce que l'UI affiche, jusqu'au prochain rechargement.
2. **A1-02/A1-03 (Faibles)** — deux fuites de robustesse : `DialogGate` peut marquer un appelant « occupé » à vie si sa tâche est annulée pendant l'attente ; `activity.touch()` avant autorisation permet à un client **refusé** de maintenir le processus root jusqu'à `RuntimeMaxSec=5min`.
3. **Preuves manquantes** — aucun test n'exerce le **vrai** polkit, la **policy système** (le test D-Bus tourne sur le bus de session) ni le **vrai** `Control1`. `dbus-policy-test.sh` (root) n'a pas été exécuté ici. Ce sont des critères bloquants avant `TCC_WITH_ADMIN=1`.

---

## 2. Méthode, instantané et preuves

### 2.1 Instantané

Revue statique + exécution des tests, sur l'état du dépôt au **2026-11-06 ~15:45** (`tcc-admin/src/service.rs` 15:39, `transaction.rs` 15:38, `tcc-authz/src/lib.rs` 15:36). Aucun code n'a été modifié (périmètre `docs/security/**`).

### 2.2 Exécuté

| Commande | Résultat |
|---|---|
| `dbus-run-session -- cargo test -p tcc-admin -p tcc-authz` | **57 tests OK** : `tcc-authz` 18, lib admin 18, binaire 5, `admin_dbus` **7/7 en parallèle**, CLI 4, transaction 5 |
| `validate.sh --repo` | **218/218** vérifications |
| `systemd-analyze verify packaging/systemd/tccd-admin.service` | seule erreur : binaire absent (attendu avant packaging) |
| `systemd-analyze security --offline=true …/tccd-admin.service` | **1.6 OK** |
| `grep` ciblés | aucun `Command::new`, aucun shell, aucun `unsafe` dans `tcc-admin`/`tcc-authz` ; aucun `CheckAuthorization` dans `tccd` |

### 2.3 Non exécuté

- **`dbus-policy-test.sh`** (root requis ; sortie 77 hors root) : la policy système n'a pas été testée fonctionnellement.
- **Vrai polkit** : seuls des `Authorizer`/`PolicyKitBackend` injectés ont été utilisés.
- **Vrai `Control1`** : les tests de transaction utilisent un `ReloadNotifier` factice.
- **Matériel TUXEDO**, installation réelle, `cargo deny`/`audit`, linters distro.

---

## 3. Findings

| ID | Sév. | Fichier:ligne | Impact | Remédiation | Test |
|---|---|---|---|---|---|
| **A1-01** | **Moyen** | `tcc-daemon/src/dbus.rs:929-953` ; `tcc-admin/src/transaction.rs:124-139` | Rapport de rechargement **périmé** (coalescence 500 ms) : l'admin croit avoir rechargé le nouveau fichier, `tccd` garde l'ancien ; **pas de rollback**. Garantie transactionnelle annulée ; limites matérielles désynchronisées de l'UI. | Supprimer la coalescence pour `Control1` (root-only, rechargement bon marché) **ou** ajouter une **génération** (mtime+taille des 3 fichiers, ou compteur de chargement) au rapport et exiger `génération > pré-écriture` ; mettre à jour `ADMIN_DESIGN` §2.4. | Test d'intégration : rechargement lent contrôlé, écriture entre deux appels, second appel doit recharger (ou détecter le périmé). |
| **A1-02** | Faible | `tcc-authz/src/lib.rs:583-609` | `DialogGate` insère l'appelant dans `in_flight` **avant** d'attendre le sémaphore ; si la tâche est annulée pendant l'attente, l'entrée n'est jamais retirée → `CallerBusy` permanent pour ce nom jusqu'à l'arrêt (≤ 5 min). Fuite bornée, pas de privilège. | Garde RAII dès l'insertion (Drop), ou insérer après acquisition avec un autre mécanisme de sérialisation par appelant. | Test : annuler le future pendant l'attente, puis vérifier qu'un nouvel appel du même nom réussit. |
| **A1-03** | Faible | `tcc-admin/src/service.rs:501,547,590` ; `service.rs:98-118` | `activity.touch()` **avant** identité/autorisation : tout client local (même refusé/rate-limité) maintient le processus root vivant jusqu'à `RuntimeMaxSec=5min` au lieu de 30 s. Bruit journald, durée de vie root étendue. | Toucher l'activité **après** autorisation réussie (ou après le limiteur pour les appels acceptés) ; conserver `RuntimeMaxSec` en garde-fou. | Test unitaire avec horloge injectée : refus ⇒ activité non touchée. |
| **A1-04** | Faible | `tcc-admin/src/transaction.rs:176-202,210-257` | `read_previous` ne vérifie **ni propriétaire ni permissions** (contrairement à `ConfigStore::read_current_bounded`) et n'utilise pas `O_CLOEXEC` ; `restore_previous` fait `chmod 0644` **après** `rename` (fenêtre 0600, crash → 0600). Défense en profondeur : les octets relus sont réécrits en root. | Réutiliser le lecteur vérifié de `ConfigStore` (ou répliquer `verify_file` : régulier, uid 0, `mode & 0o022 == 0`), ajouter `O_CLOEXEC`, `fchmod` le fd temporaire avant `rename`. | Test : fichier `0666`/autre uid refusé ; permissions finales 0644 ; symlink déjà couvert. |
| **A1-05** | Faible | `tcc-admin/src/service.rs:398-408,527-530` | Payload composé **uniquement de presets** : retourne `true` et l'audit indique `reload=ok` alors qu'**aucun** rechargement n'a eu lieu. Peut masquer un bug client. | Auditer `reload=skipped` (ou déclencher le rechargement, bon marché) ; distinguer « no-op » de « écrit ». | Étendre `preset_only_payload_is_a_noop` pour vérifier le statut de rechargement rapporté/audité. |
| **A1-06** | Faible | `packaging/dbus/…conf` bloc Admin (allow Introspectable/Peer + 3 membres ; deny `Ping`/`GetVersion` ; **aucun** `Properties.Get`) ; `tcc-admin/src/service.rs:465-486` | `Ping`, `GetVersion` et la propriété `ContractVersion` sont **inaccessibles** via le bus, alors que le design exige une comparaison de contrat. Méthodes mortes ; tests qui contournent la policy (bus de session). | Soit autoriser `Properties.Get` + `Ping`/`GetVersion` (lecture seule, sans effet de bord) **après** correction d'A1-03, soit les retirer du contrat et documenter. | Cas `dbus-policy-test.sh` pour `Properties.Get` et `Ping`. |
| **A1-07** | Faible | `tcc-admin/src/service.rs:414-421,442-448` | La validation croisée appelle `load_settings`/`load_profiles`, qui peut **renommer** un fichier corrompu en `.bak` (effet de bord de `ConfigStore`) avant de retourner `Validation`. Surprenant ; pas de perte au-delà du comportement de récupération existant. | Utiliser une lecture sans récupération (`ConfigStore::read_only()` ou helper dédié) pour la validation ; ou documenter explicitement. | Test : settings corrompus + `SaveProfilesJSON` ⇒ `Validation` **et** aucun `.bak` créé. |
| **A1-08** | Info | `tcc-authz/src/lib.rs:288` ; `tcc-admin/src/service.rs:193` | Une **nouvelle connexion** au bus système par vérification polkit et par rechargement. Coût uniquement (pas de fuite, pas de privilège). | Envisager une connexion réutilisée dans `ControlReloader`/`ZbusPolicyKit` (processus court). | — |
| **A1-09** | Info | `crates/tcc-daemon/Cargo.toml` (pas de `tcc-authz`) ; `packaging/dbus/…conf:110-133` | Les setters `Manager1` de `tccd` n'ont **aucune** autorisation applicative ; ils sont uniquement fermés par la policy. Hors phase 1, mais toute réouverture sans `CheckAuthorization` recrée S-01. | Ne jamais retirer les deny sans authz + test ; prévoir le branchement `tcc-authz` dans `tccd` (AD-8). | Test d'introspection/policy qui échoue si un setter est rouvert sans test d'autz. |
| **A1-10** | Info | `tcc-admin/tests/*`, `tcc-authz` | Preuves manquantes : vrai polkit, policy système, vrai `Control1`, matériel. | Voir §6 (checklist machine réelle). | — |

---

## 4. Détail des findings principaux

### A1-01 — La coalescence de `Control1` peut valider un rechargement antérieur à l'écriture

**Constat.** `Control::reload_coalesced` (`tcc-daemon/src/dbus.rs:929-953`) mémorise
`(Instant, AdminReloadReport)` **après** la fin d'un rechargement, et renvoie ce rapport sans
recharger si un nouvel appel arrive **moins de 500 ms** après. Le rapport ne contient ni révision ni
empreinte des fichiers (`AdminReloadReport { ok, warnings }`, `tcc-ipc/src/admin.rs:56-64`).

Séquence problématique :

```
t0  rechargement R1 demandé ; il lit les fichiers (ancien état)
t1  R1 terminé, rapport ok mémorisé (last = (t1, ok))
t2  l'admin écrit profiles.json (nouveau contenu, t1 < t2 < t1+500 ms)
t3  l'admin appelle Control1.ReloadConfig (t3 < t1+500 ms)
    → l'interface renvoie le rapport de R1 (ok) SANS recharger
    → commit() conclut au succès, pas de rollback
    → tccd garde l'ancien profiles.json en mémoire
```

Ce n'est pas seulement une hypothèse d'attaquant : deux enregistrements rapprochés (par exemple
réglages puis profils depuis le même écran, ou un utilisateur qui valide deux fois) suffisent — le
second rechargement est coalescé avec le premier et `tccd` n'applique pas le second fichier. Le
client reçoit `true`, l'audit dit `reload=ok`, l'utilisateur voit « enregistré ».

**Impact.** La garantie « écrit puis recharge, sinon rollback » (`AD-6`) n'est plus vraie. Des
limites matérielles (charge, TDP dans un profil) peuvent rester à l'ancienne valeur alors que l'UI
affiche la nouvelle, jusqu'au prochain SIGHUP/redémarrage. Ce n'est pas une élévation de privilège
(l'appelant est authentifié), mais c'est une **atteinte à l'intégrité de l'état appliqué** et un
faux succès.

**Remédiation (au choix, par ordre de préférence).**

1. **Supprimer la coalescence de `Control1`.** La méthode est root-only et le rechargement est bon
   marché (lecture ≤ 1 Mio × 3 + réapplication). La coalescence n'apporte rien face à un appelant
   unique (l'admin) et introduit le bug. Conserver éventuellement un anti-rebond pour SIGHUP, qui ne
   passe pas par `Control1`.
2. **Ajouter une génération.** `AdminReloadReport` gagne `generation: u64` (compteur incrémenté à
   chaque chargement réussi, ou `mtime_ns ^ size` des trois fichiers). L'admin lit la génération
   **après** l'écriture (ou exige `generation > generation_pré_écriture`) et rejette un rapport
   périmé ; `Control1.ReloadConfig(generation_min: u64)` peut refuser de coalescer si la génération
   mémorisée est trop ancienne.
3. **Coalescence conditionnelle** : ne réutiliser le rapport que si les fichiers n'ont pas changé
   depuis (mtime+taille comparés à ceux du chargement mémorisé).

**Test de non-régression.** Test d'intégration avec un `Control` réel et un canal de rechargement
piloté : (a) déclencher R1, (b) modifier un fichier entre R1 et l'appel suivant, (c) appeler
`ReloadConfig` dans la fenêtre de 500 ms, (d) vérifier que le rechargement a bien eu lieu (compteur
du canal) ou que le rapport est refusé. Ajouter un test `admin_dbus` avec un reloader qui simule
« rapport ok mais contenu non rechargé » n'a de sens que si `Control1` est réellement exercé : le
test doit porter sur `tccd`, pas sur un faux.

**Mise à jour documentaire.** `ADMIN_DESIGN.md` §2.4 prescrit la coalescence 500 ms : la décision
doit être corrigée (suppression ou génération) et `THREAT_MODEL.md` delta §2 doit refléter la
garantie réelle.

### A1-02 — Fuite de `in_flight` dans `DialogGate` en cas d'annulation

`acquire` (`tcc-authz/src/lib.rs:583-609`) :

```rust
{ let mut in_flight = …; if !in_flight.insert(caller.to_owned()) { return Err(CallerBusy); } }
match timeout(wait, semaphore.acquire()).await { … }
```

Si le future est abandonné pendant `timeout(...)` (déconnexion du client, annulation de la tâche
zbus, arrêt), ni `Ok` ni `Err` n'est traité : `in_flight` conserve l'entrée et l'appelant reçoit
`CallerBusy` pour toutes ses tentatives suivantes jusqu'à la fin du processus. Impact limité
(disponibilité, un seul nom, ≤ `RuntimeMaxSec`), mais le mécanisme est censé être un garde-fou, pas
une source de blocage. Remédiation : un garde `Drop` inséré avant l'attente (qui retire l'entrée
même en cas d'abandon), ou insérer l'appelant **après** l'acquisition du sémaphore et gérer la
sérialisation par appelant autrement.

### A1-03 — `activity.touch()` avant autorisation

Les trois méthodes d'écriture appellent `self.activity.touch()` avant `caller_identity`, la taille
et polkit (`service.rs:501,547,590`). Un client local peut appeler `SaveProfilesJSON` en boucle :
la policy l'autorise à atteindre le handler, le limiteur le refuse (`RateLimited`), mais l'activité
a déjà été repoussée. Le processus root ne sort donc pas après 30 s ; il vit jusqu'à
`RuntimeMaxSec=5min`. Aucun privilège, mais la durée de vie du processus root est contrôlée par un
utilisateur non authentifié. Remédiation : déplacer `touch()` après l'autorisation (ou après le
limiteur pour les appels acceptés) ; garder `RuntimeMaxSec`.

### A1-04 — Contrôles de lecture/restauration plus faibles que `ConfigStore`

`read_previous` (`transaction.rs:176-202`) ouvre avec `O_NOFOLLOW` et vérifie « fichier régulier +
taille », mais **pas** le propriétaire ni le mode, contrairement à
`ConfigStore::read_current_bounded` qui appelle `verify_file` (`config.rs`). Les octets relus sont
réécrits en root par `restore_previous` : si le fichier était contrôlé par un autre uid (hypothèse
qui suppose un répertoire compromis), on blanchirait son contenu. En pratique le répertoire est
`root:root` non inscriptible et `ProtectSystem=strict` ; c'est de la défense en profondeur.
`restore_previous` fait en outre `chmod 0644` **après** `rename` : un crash entre les deux laisse la
cible en 0600 (acceptée par `ConfigStore`, mais différente de la permission documentée).
Recommandation : réutiliser le lecteur vérifié de `ConfigStore` (ou répliquer `verify_file`), ajouter
`O_CLOEXEC`, et `fchmod` le fd temporaire en 0644 **avant** le `rename` (comme `write_atomic`).

### A1-05 — Succès et audit « reload=ok » sans rechargement (presets uniquement)

Si le payload ne contient que des presets, `save_profiles_inner` retourne
`Ok(SavedInfo { bytes, backup:false })` **sans** appeler la transaction
(`service.rs:398-408`). Le handler journalise ensuite `result=ok` et `reload=ok`
(`:527-530`). C'est un no-op légitime, mais l'audit affirme un rechargement qui n'a pas eu lieu.
Corriger le libellé (`reload=skipped`) ou recharger quand même.

### A1-06 — `ContractVersion`/`Ping`/`GetVersion` refusés par la policy

Le bloc Admin de la policy autorise `Introspectable`, `Peer`, `SaveProfilesJSON`,
`SaveSettingsJSON`, `ReloadConfig`, puis refuse explicitement `Ping`, `GetVersion` et tout le reste
(`packaging/dbus/…conf`, section 6/6bis). Aucun `Properties.Get` n'est autorisé pour `.Admin` : la
propriété `ContractVersion` (`service.rs:468-472`) est donc **injoignable**. Or `tcc-ipc` impose une
comparaison stricte de version (`ensure_admin_contract_compatible`) et `ADMIN_DESIGN` la prévoit.
Deux options : (a) autoriser `Properties.Get`, `Ping`, `GetVersion` (lecture seule, sans polkit) —
mais après avoir corrigé A1-03 pour que ces appels non authentifiés ne prolongent pas la vie du
processus ; (b) retirer ces méthodes du contrat et documenter qu'il n'y a pas de négociation de
version. Les tests `admin_dbus` utilisent le bus de session et **ne voient pas** cette policy ; le
seul test qui la couvre est `dbus-policy-test.sh` (root).

### A1-07 — Effet de bord de la validation croisée

`save_profiles_inner` charge les settings (`:414-421`) et `save_settings_inner` charge les profils
(`:442-448`). `ConfigStore::load_*` **renomme** un fichier corrompu en `.bak` avant de retourner des
valeurs par défaut avec un avertissement. Le service transforme l'avertissement en `Validation`,
mais le renommage a déjà eu lieu. Ce n'est pas une faille (récupération documentée), mais un appel
refusé peut modifier le disque. Utiliser une lecture sans récupération pour la validation.

---

## 5. Conformité vérifiée (avec preuves)

### 5.1 Polkit (`tcc-authz`)

| Exigence | Preuve |
|---|---|
| Sujet `system-bus-name` du **nom unique de l'appelant** | `lib.rs:295-302` (`system_bus_subject`), validation `:382-401` (préfixe `:`, caractères sûrs, ≤ 255) ; test `subject_is_system_bus_name_and_never_unix_process` (`:734-742`) |
| Jamais `unix-process`, jamais un PID client | aucune occurrence de `unix-process` dans le crate ; le backend ne reçoit que `caller_bus_name` (`:261-273`) |
| Flags `AllowUserInteraction` | `:299` (1 si interactif, 0 sinon) ; tests `subject_and_flags_are_forwarded_without_cache`, `non_interactive_request_sets_flags_to_zero` |
| `is_challenge == true` **refusé** | `RawAuthorization::decision` (`:227-237`) ; tests `challenge_is_never_authorized`, `denial_and_challenge_are_refused` |
| Absence / erreur / timeout ⇒ refus | `:351-367` (timeout → `Denied(Timeout)`, erreur → `Denied(PolicyKitUnavailable)`) ; tests `missing_polkit_is_fail_closed`, `timeout_is_fail_closed` |
| Pas de court-circuit root | `:346-349` (UID requis, aucun cas uid 0) ; test `root_is_not_short_circuited` (backend appelé avec uid 0 et refus propagé) |
| Pas de cache local | aucun cache dans le crate ; test `subject_and_flags_are_forwarded_without_cache` (2 appels ⇒ 2 `CheckAuthorization`) |
| Limite par nom **et** UID | `AuthorizationLimiter` (`:464-544`) : deux maps, refus si l'un est épuisé ; test `limiter_enforces_rate_and_uid_budgets` |
| Dialogue unique + un par appelant | `DialogGate` (`:560-638`), `Semaphore(1)` + `HashSet` ; test `dialog_gate_refuses_same_caller_and_serializes_others` |
| Timeout borné | défaut 120 s, bornes 1–300 s (`:51-55,147-157`) ; `check_authorization_with` enveloppe l'appel (`:351-362`) |
| Validation structurelle avant appel | `:147-157,346-349` |

### 5.2 `Admin1` — ordre et invariants

| Exigence | Preuve |
|---|---|
| Identité de l'appelant (nom unique + UID), fail-closed | `service.rs:261-279` (`header.sender()` obligatoire, `GetConnectionUnixUser` avec timeout 500 ms, toute erreur ⇒ `NotAuthorized`) |
| Borne de taille **avant parse** | `ensure_payload_size` (`:372-379`) appelée avant `serde_json::from_str` (`:511,557`) ; **mais** la `String` a déjà été allouée par zbus (résiduel, §7) |
| Limiteur **avant** polkit | `authorize_write` (`:282-297`) → `AuthorizationLimiter::allow(caller, uid)` ; refus journalisé `rate_limited` |
| Polkit avec dialogue unique | `PolkitAuthorizer::authorize` (`:154-173`) : `DialogGate` puis `check_authorization` ; `Busy`/`Denied`/`Unavailable` mappés |
| Verrou d'écriture | `write_gate` `Semaphore(1)` (`:220,238,517-523,563-569`) ; test `write_gate_is_exclusive` et `concurrent_write_returns_busy` (preuve déterministe via reloader bloquant) |
| Parse + validation + références | `save_profiles_inner` (`:381-428`) : parse, filtre presets, `validate()`, chargement de l'autre fichier, `validate_settings_references`, transaction ; `save_settings_inner` (`:430-453`) : `Settings::validate` (schéma inclus), profils sur disque, références |
| Presets jamais persistés | `:393-408` ; tests `preset_only_payload_is_a_noop`, `empty_payload_deletes_custom_profiles` |
| Refus d'un profil référencé supprimé | `:423-425,448-450` ; test `removing_a_referenced_profile_is_refused` |
| Erreurs sans chemin/contenu | `error.rs:82-114` (champ seul ; ID d'`InUse` filtré sur `[A-Za-z0-9_-]{1,64}`) ; tests `validation_keeps_only_the_field`, `in_use_keeps_safe_ids_only` |
| Audit sans payload | `service.rs:344-370` (`bytes`, `backup`, `reload`, `caller`, `caller_uid`, `action`, `result`, `duration_ms`) ; aucun contenu JSON |
| Refus non-root | `main.rs:122-127` (code 77) ; `is_root` via `/proc/self/status` (`:202-215`) |
| Sortie après inactivité | `service.rs:39,98-118` ; `main.rs:183` ; test `idle_watch_requests_shutdown` |
| Aucun shell/pkexec/chemin en paramètre | aucun `Command::new`/shell dans `tcc-admin`/`tcc-authz` ; signatures : `s`, `()` uniquement ; `ADMIN_DESIGN` AD-11 |

### 5.3 Transaction/rollback

| Exigence | Preuve |
|---|---|
| `.bak` 0600 de la version précédente | `config.rs::save_typed_with_backup` → `write_atomic(..., 0o600)` ; test `successful_save_writes_backup_0600` |
| Cible 0644 | `write_atomic(..., 0o644)` avec `fchmod` sur fd avant `rename` ; tests de mode |
| Écriture atomique + `fsync` + `rename` + `fsync` dir | `config.rs::write_atomic` ; idem restauration (`transaction.rs:210-257`) |
| Restauration **octet pour octet** | `transaction.rs::restore_previous` ; test `reload_warning_rolls_back_exact_bytes` |
| Rollback sur erreur de transport | `commit` traite `Err(ReloadError)` comme échec (`:131-139`) ; test `transport_failure_also_rolls_back` |
| Pas de temporaire résiduel | `restore_previous` supprime le temporaire en cas d'erreur et le test vérifie l'absence de `.restore.*` (`:314-320`) |
| Idempotence | contenu identique ⇒ `Saved { backup: None }` sans réécriture (`config.rs`) ; test `identical_content_is_idempotent` |
| Concurrence | `Semaphore(1)` + `try_acquire` ⇒ `Busy` ; test D-Bus `concurrent_write_returns_busy` |
| Écriture uniquement sous `/var/lib`, jamais `/etc` | `verify_write_scope` (`config.rs`), `validate_state_root`, `ProtectSystem=strict` + `ReadWritePaths` ; `ConfigStore::system()` fixe `write_root = STATE_DIR` |

> **Double fichier (tombstones)** : une suppression de profil écrit `profiles.json` **et**
> `settings.json` dans une seule transaction, avec un seul rechargement et rollback des deux ; voir
> l'addendum T ci-dessous et `ADMIN_DESIGN.md` §4.2.1.

### 5.4 `Control1`

| Exigence | Preuve |
|---|---|
| Root revérifié dans le code même si la policy est cassée | `dbus.rs:956-990` : `resolve_caller_uid` puis `is_root_caller(Some(0))` ; UID inconnu ⇒ refus |
| `header.sender()` non usurpable | nom unique fourni par le bus ; jamais un champ client |
| Root-only au niveau policy | allow `user="root"`, deny explicite en default (`conf:39-42,199-206`) ; `validate.sh:130` vérifie le bloc |
| Réponse sans chemin/contenu | `AdminReloadReport` produit depuis `config.report_warnings` (catégories `settings/profiles/fantables: <kind>` et « références invalides », `tcc-daemon/src/config.rs:147,185`) ; `from_json_bounded` + `validate` bornent (64 avertissements, 512 chars, pas de caractères de contrôle) |
| Timeout et arrêt propre | `RELOAD_REPLY_TIMEOUT = 30 s` (`:893,944`) ; canal fermé/timeout ⇒ rapport `ok=false` (`:938-950`) ; l'admin traite `ok=false` comme échec ⇒ rollback |
| Ne bloque pas la boucle de transition | le handler n'écrit rien : il envoie une `ReloadRequest` et attend ; la boucle principale traite la demande dans son `select!` (`main.rs:329-347`) |
| Perte de bus / arrêt | `connection_lost`/SIGTERM ⇒ `break` ; les `oneshot` en attente sont annulés ⇒ `ReloadError` côté admin ⇒ rollback |
| Coalescence | **présente** (`:929-953`) — c'est A1-01 |

### 5.5 Policy, polkit, unité

| Exigence | Preuve |
|---|---|
| `Admin1` membres listés, pas d'allow d'interface | `validate.sh` échoue sur un allow `Admin1` sans `send_member` ; 3 membres autorisés (`SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig`), le reste refusé |
| Phase 2/3 refusées | denies explicites `SetTdpValues`, `SetOdmProfile`, `SetNvidiaCtgpOffset`, `SetPrimeMode`, `TomteRun`, `PackagesApply` ; **les méthodes LUKS n'existent pas (hors périmètre)** |
| `Control1` root-only | allow root + deny default ; code revérifie l'UID |
| Pas de `Properties.Set` | aucun allow, aucun allow d'interface |
| Pas d'`exec.path`/`allow_gui` | `validate.sh` et `.policy` (9 actions, aucune annotation) |
| IDs d'action cohérents | `ACTION_PROFILES = …tccd1.profiles`, `ACTION_SETTINGS = …tccd1.settings` (`service.rs:35-37`) ; présents dans la policy (`:38,53`) |
| Unité phase 1 | `PrivateDevices=yes`, `DevicePolicy=closed`, `CapabilityBoundingSet=` vide, `ProtectSystem=strict`, `ReadWritePaths=/var/lib/tuxedo-rust-center`, `MemoryMax=192M`, `TasksMax=16`, `LimitCORE=0`, `Restart=no`, `RuntimeMaxSec=5min`, pas de `StateDirectory`/`[Install]`, `SystemCallFilter=@system-service` |
| Score de hardening | `systemd-analyze security --offline=true` : **1.6 OK** |
| Non installé par défaut | `install-tree.sh:40,64-81,123-127,139-146` : `TCC_WITH_ADMIN=0` par défaut, refus si binaire absent, unité + activation conditionnelles |

### 5.6 Tests

| Suite | Résultat | Ce qui est prouvé |
|---|---|---|
| `tcc-authz` (lib) | 18/18 | sujet, flags, challenge, absence, timeout, root non court-circuité, pas de cache, limiteur nom+UID, dialogue unique, bornes de map |
| `tcc-admin` (lib) | 18/18 | activité/inactivité, borne de taille, no-op presets, suppression de profils, `InUse`, schéma invalide, JSON invalide, verrou exclusif, refus sans écriture |
| `tcc-admin` (binaire) | 5/5 | arguments, `--force` refusé, racine, aide/version |
| `admin_dbus` | **7/7 en parallèle** | `Ping`/`GetVersion`/propriété, save+réload, JSON invalide typé, payload trop gros, refus polkit, `Busy` concurrent (preuve déterministe), rollback sur avertissement |
| `cli` | 4/4 | parsing CLI |
| `transaction` | 5/5 | `.bak` 0600, rollback exact, échec de transport, pas de version précédente, idempotence |

**Ce que les tests ne prouvent pas** : le **vrai** `CheckAuthorization`, l'application de la **policy
système**, le **vrai** `Control1` et le rollback via `tccd` réel. `admin_dbus` monte le service sur
le bus de session avec le **nom unique** de la connexion : il ne teste ni le nom bien connu, ni la
policy, ni polkit. Les `Authorizer`/`ReloadNotifier` sont injectés.

---

## 6. Actions bloquantes avant d'activer `TCC_WITH_ADMIN=1`

| # | Action | Justification |
|---|---|---|
| 1 | **Corriger A1-01** (supprimer la coalescence de `Control1`, ou ajouter une génération) + test de non-régression sur `tccd` | La transaction peut rapporter un faux succès et laisser l'ancienne configuration active |
| 2 | **Test de bout en bout sur machine réelle / VP avec bus système** : dialogue polkit autorisé, refusé, annulé, polkit arrêté ; vérifier qu'aucun fichier n'est écrit en cas de refus | Aucun test actuel n'exerce le vrai polkit ni la policy système |
| 3 | **Exécuter `scripts/release/dbus-policy-test.sh` en root** et archiver la sortie (Admin1 3 méthodes, phases 2/3 refusées, `Control1` non-root refusé, `Properties.Set` refusé) | La policy n'est vérifiée que statiquement (`validate.sh`) |
| 4 | **Test réel du rollback via `tccd`** : écrire, provoquer un `ok=false` (fichier invalide introduit volontairement ou arrêt de `tccd`), vérifier la restauration octet pour octet, les permissions et l'absence de temporaires | Le rollback n'est testé qu'avec un `ReloadNotifier` factice |
| 5 | **Trancher A1-06** (autoriser `Properties.Get`/`Ping`/`GetVersion` ou les retirer) et corriger A1-03 en conséquence | Sans cela, la version de contrat est inutilisable et des méthodes mortes subsistent |
| 6 | **Corriger A1-02/A1-04** (garde RAII, lecteur vérifié, `O_CLOEXEC`, `fchmod` avant `rename`) ou les documenter comme risques acceptés | Robustesse et défense en profondeur, coût faible |
| 7 | **Archiver les preuves** : `validate.sh --repo` 218/218, `systemd-analyze security` 1.6, tests, échantillon journald (audit sans payload), permissions `.bak`/cible, absence de temporaire | Critères de sortie de `CHECKLIST.md` §9 |
| 8 | **`cargo deny`/`audit` + SBOM** avant publication (non exécutés ici) | A8 / supply chain |

---

## 7. Risques résiduels et ce qui n'est pas testable localement

| # | Risque | Niveau | Statut |
|---|---|---|---|
| R1 | **Borne 1 Mio après allocation** : zbus désérialise la `String` avant `ensure_payload_size` ; un message de 128 Mio peut tuer l'admin (OOM cgroup) avant tout contrôle. Aucune écriture n'a lieu (fail-closed), mais `StartLimitBurst=5/60 s` peut bloquer l'admin ~60 s. | Faible (disponibilité) | Accepté et documenté (AD-R1) ; `MemoryMax=192M` + `Restart=no` |
| R2 | **Durée de vie du processus root** prolongée par des appels refusés (A1-03) | Faible | À corriger (A1-03) ou à accepter ; borné par `RuntimeMaxSec=5min` |
| R3 | **`SaveProfilesJSON("[]")` supprime tous les profils personnalisés** | Faible (destructif) | Intentionnel et testé ; l'UI doit confirmer ; les profils référencés par `state_map`/`state_matrix` sont protégés (`InUse`) |
| R4 | **Setters `Manager1` toujours sans autorisation applicative** | Élevé si rouverts | Refusés par la policy ; ne pas rouvrir sans `tcc-authz` dans `tccd` (A1-09/TR-02) |
| R5 | **Profil temporaire global** (`SetTempProfile`) | Moyen | Hors phase 1 ; TR-01 du delta jalon 2 |
| R6 | **Vrai polkit non testé** (agent, session inactive, cache `_keep`) | Moyen | Bloquant §6 |
| R7 | **Policy système non testée fonctionnellement** (`dbus-policy-test.sh` root) | Moyen | Bloquant §6 |
| R8 | **Rollback via `tccd` réel non testé** (coalescence, arrêt, timeout) | Moyen | Bloquant §6 |
| R9 | **`ControlReloader`/`ZbusPolicyKit` : nouvelle connexion par appel** | Info | Coût uniquement |
| R10 | **Aucun test matériel** (phase 1 n'écrit pas le matériel, mais le rechargement réapplique le profil actif) | Faible | La réapplication réelle reste non validée (H-13) |

**Non testable localement** : polkit réel et agent, bus système et policy appliquée, installation
réelle des paquets, `cargo deny`/`audit`, linters distro, comportement sous session verrouillée ou
distante, matériel TUXEDO.

---

## 8. Checklist de validation sur machine réelle

### 8.1 Préparation

- [ ] Paquet construit avec `TCC_WITH_ADMIN=1` (binaire présent) ou binaire installé manuellement.
- [ ] `systemctl status tccd-admin` : **inactive** au boot, aucune activation automatique.
- [ ] `systemd-analyze security tccd-admin.service` ≥ 1.6 archivé.
- [ ] `validate.sh --repo` et `dbus-policy-test.sh` (root) verts.

### 8.2 Polkit (bus système)

- [ ] Utilisateur **non administrateur** : `busctl call … Admin1.SaveSettingsJSON s "$(tcc-cli settings)"` ⇒ dialogue polkit ; **annuler** ⇒ `org.tuxedocomputers.tccd.Admin1.Error.NotAuthorized` ; **aucun** fichier modifié (mtime inchangé).
- [ ] Administrateur : approuver ⇒ `b true` ; `settings.json` mis à jour ; `.bak` 0600 ; `tccd` a rechargé (journal).
- [ ] `systemctl stop polkit` (VM) ⇒ nouvel appel ⇒ `NotAuthorized`, aucun fichier écrit, journal `polkit_unavailable`.
- [ ] Root : appeler en root ⇒ `CheckAuthorization` est bien appelé (journal/trace) ; si polkit autorise root, succès ; sinon refus — **documenter** le comportement observé, aucun court-circuit.
- [ ] Deux appels simultanés : le second reçoit `Busy` ; un seul fichier écrit ; un seul dialogue.

### 8.3 Rollback et panne

- [ ] Écrire un `settings.json` valide, puis **arrêter `tccd`** et appeler `SaveSettingsJSON` ⇒ `ReloadFailed` ; l'ancien contenu est restauré octet pour octet ; permissions 0644 ; `.bak` 0600 ; aucun `.restore.*`.
- [ ] `kill -9` de `tccd-admin` pendant une écriture ⇒ fichier cible soit ancien soit nouveau, **jamais** tronqué ; temporaires 0600 sans contenu sensible ; prochaine activation fonctionnelle.
- [ ] Redémarrer la machine : profils/settings persistés, `tccd` les charge ; `tccd-admin` ne redémarre pas tout seul.

### 8.4 Audit et fichiers

- [ ] `journalctl -u tccd-admin` : une ligne par appel avec `caller`, `caller_uid`, `action`, `result`, `bytes`, `backup`, `reload` ; **aucun** payload, chemin ou secret.
- [ ] `/var/lib/tuxedo-rust-center` : `settings.json`/`profiles.json` 0644 root, `.bak` 0600, aucun temporaire résiduel.
- [ ] `systemctl stop tccd` puis `StartLimit` : 10 activations échouées ⇒ unité en échec, pas de boucle infinie.

### 8.5 Policy et contrat

- [ ] `busctl introspect com.tuxedocomputers.tccd.Admin /com/tuxedocomputers/tccd/Admin` en non-root : OK (Introspectable).
- [ ] Méthode phase 2 (`SetTdpValues`) ⇒ `AccessDenied` **au niveau bus** ; **aucune méthode LUKS** (hors périmètre).
- [ ] `Control1.ReloadConfig` en non-root ⇒ `AccessDenied` bus ; en root ⇒ rapport JSON.
- [ ] Décider/autoriser `Properties.Get` si la négociation de version est conservée (A1-06), sinon documenter son absence.

---

## 9. Conclusion

L'implémentation est **nettement au-dessus** de l'état décrit par `ADMIN_DESIGN` : polkit est
réellement câblé avec le bon sujet, fail-closed, sans cache ni court-circuit root ; la transaction
est sérialisée, sauvegardée, atomique et rollbackée ; `Control1` est root-only revérifié et ne bloque
pas la boucle de transition ; l'unité phase 1 est bien durcie (`1.6`). Les défauts restants sont
**un défaut de garantie** (A1-01) et des **brutalités de bord** (A1-02…A1-07), pas des primitives
d'escalade. L'activation en production doit attendre la correction d'A1-01 et les preuves de bout en
bout (§6), pas une réécriture.

**Prochaine revue** : après correction d'A1-01, exécuter la checklist §8 et mettre à jour
`THREAT_MODEL.md` (delta §2/§4) et `ADMIN_DESIGN.md` §2.4 (coalescence). La phase 2 (TDP/ODM/cTGP/
PRIME) devra reprendre ce document comme base, avec un delta de durcissement (AD-12) et une revue
`security` dédiée avant toute activation.

---

## 10. Verdict après correctifs (2026-11-06)

### 10.1 `TCC_WITH_ADMIN=1` peut-il être activé ?

**Oui, sous réserve de la validation manuelle ci-dessous.** Aucun finding **Critique**, **Élevé** ou
**Moyen** ne reste ouvert : A1-01 (le seul Moyen) est fermé structurellement (cache supprimé,
sérialisation sans mémorisation, trois tests dédiés) ; A1-02→A1-07 sont fermés avec tests ;
A1-08/A1-10 sont des informations (coût, preuves manquantes) ; A1-09 est fermé côté documentation ;
A1-11 est fermé côté policy. Le seul **Faible** encore ouvert est **S1-02** (course inter-clients
sur `SaveSettingsJSON`, sans révision/CAS), documenté et sans escalade.
Le code phase 1 peut donc être installé **après** avoir rejoué les preuves qui ne sont pas
reproductibles dans cet environnement.

### 10.2 Ce qui reste obligatoirement manuel (non testable ici)

1. **Vrai polkit sur le bus système** : dialogue autorisé, refusé, annulé, `polkit` arrêté ⇒
   `NotAuthorized` et **aucune** écriture. Aucun test automatisé n'exerce l'agent polkit réel.
2. **Policy système appliquée** : `scripts/release/dbus-policy-test.sh` en **root** (Admin1/
   `Control1`/phases 2-3/`Properties.Set`). Le script a été étendu (22 vérifications nommées) mais
   n'a pas été exécuté ici ; le « 38/38 » annoncé par `packaging-release` reste à confirmer.
3. **Rollback réel via `tccd`** : provoquer un `ok=false` (arrêt de `tccd`, timeout, fichier rendu
   invalide) et vérifier la restauration octet pour octet, les permissions (`0644` cible / `0600`
   `.bak`), l'absence de `.restore.*`, et le comportement lorsque le précédent est inexploitable
   (`ReloadFailed` sans restauration).
4. **Installation réelle** : paquet `TCC_WITH_ADMIN=1`, `tccd-admin` **inactif au boot**, activation
   D-Bus seulement, `systemd-analyze security` 1.6 archivé, unit non installée avec
   `TCC_WITH_ADMIN=0`.
5. **Audit journald** : une ligne par appel, `caller`/`caller_uid`/`action`/`result`/`backup`/`reload`,
   **aucun** payload, chemin ou secret.
6. **`cargo deny`/`cargo audit` + SBOM**, linters distro, installation dans un conteneur propre.
7. **A1-11 (corrigé)** : aucun allow `Properties` sur `.Admin` ; vérifier en root que
   `Properties.Get/GetAll` sont refusés et que `GetContractVersion()` répond.
8. **Documentation** : consigner A1-12 (`.bak.corrupt`), A1-13 (précédent non réparable via l'API)
   et A1-14 (`ReloadFailed` avec nouveau fichier conservé) dans `DBUS_API.md`/`ADMIN_DESIGN.md`.

### 10.3 Risques résiduels inchangés (hors phase 1)

- **TR-02** : setters `Manager1` sans autorisation applicative, fermés par la policy ; ne pas
  rouvrir sans `CheckAuthorization` dans `tccd`.
- **TR-01** : profil temporaire global (`SetTempProfile`), hors périmètre admin.
- **AD-R1** : message zbus de 128 Mio alloué avant la borne 1 Mio ; `MemoryMax` protège, l'OOM et
  `StartLimit` peuvent bloquer l'admin ~60 s.
- **A1-11** : corrigé (aucun allow `Properties` sur `.Admin`) ; preuve root (`dbus-policy-test.sh`) à rejouer.
- Phase 2/3 non implémentées et refusées par la policy (aucune activation accidentelle possible).

### 10.4 Conclusion

La phase 1 est **prête pour une activation contrôlée** : le correctif A1-01 a supprimé la seule
atteinte à la garantie transactionnelle, et les garde-fous (polkit fail-closed, transaction
conditionnelle, sérialisation, audit) sont couverts par des tests ciblés. L'activation doit rester
**conditionnée** au rejeu des preuves système (polkit réel, policy root, rollback réel, installation),
qui sont les seules à ne pas pouvoir être produites dans cet environnement.

L'addendum T ajoute **T1-01** (Faible, cohérence) : rollback d'une suppression quand `settings.json`
est absent — **fermé** (suppression durable du fichier créé, tests dédiés) ; **AD-R13** est également
fermé (diff d'état seul). Il ne reste aucun finding code bloquant.

---

## Addendum — écriture GUI via Admin1 (2026-11-06)

### A.1 Verdict

La GUI utilise désormais `Admin1.SaveProfilesJSON` pour enregistrer l'éditeur de profil. **Aucune
écriture privilégiée n'est ajoutée côté client** : pas d'accès `/etc`/`/var/lib`, pas de `pkexec`,
pas de shell, pas de chemin fourni par l'utilisateur. Le DTO est borné, typé, `deny_unknown_fields`,
et la collection complète est **revalidée deux fois** (GUI Rust puis `tccd-admin`).

**Aucun finding Critique ou Élevé.** Quatre findings **Moyens** portent sur la **fiabilité et
l'intégrité** de l'enregistrement, pas sur les privilèges : perte silencieuse d'un `SaveProfile` dans
la file, timeout client plus court que le pire cas admin, écriture en « dernier écrivain gagne » sans
révision, et valeurs par défaut substituées pour des champs CPU inconnus. Deux findings Faibles et
trois Informations complètent la revue.

### A.2 Périmètre et preuves exécutées

| Élément | Valeur |
|---|---|
| Fichiers lus | `crates/tcc-gui/src/profile_edit.rs` (567 l.), `src/daemon_client.rs` (proxy `Admin1`, commande `SaveProfile`, détection, mapping), `qml/core/{AdminState,ProfileRules,LiveData}.qml`, `qml/pages/ProfileEditorPage.qml`, `tests/qml/tst_profile_{rules,editor}.qml`, `docs/ui/PROFILE_EDITING.md` |
| Exécuté | `cargo test -p tcc-gui --bins` sous `dbus-run-session` : **50/50** (dont `profile_edit`, faux `Admin1`, mapping d'erreurs) |
| Non exécuté | Tests QML (`qmltestrunner` local silencieux/incompatible ; la CI les lance sur Qt 6.10 Arch) ; vrai polkit ; activation D-Bus système ; test à deux clients |
| Policy vérifiée | `Admin1` : `Ping`, `GetVersion`, `GetContractVersion`, `SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig` ; **aucun** allow `Properties` sur `.Admin` (A1-11 corrigé) ; phases 2/3 refusées |

### A.3 Findings

| ID | Sév. | Fichier:ligne | Impact | Remédiation | Test |
|---|---|---|---|---|---|
| **G1-01** | **Moyen** | `daemon_client.rs:585-594` (garde + drain), `:471-481` (`saving`), `:700-708` (publication), `:823-845` (`apply_save`, seul reset) | Un `SaveProfile` peut être **silencieusement supprimé**. La garde `if !matches!(command, Command::SaveProfile { .. })` protège le save **courant**, mais pas un save présent dans la file : si le worker est occupé et que la tête est un `Refresh` (périodique ou manuel), puis que le save et une commande suivante arrivent avant le prochain `recv`, le drain `while let Ok(newer) = commands.try_recv()` remplace la commande par la **dernière** et jette le save. Aucun `SaveOutcome` n'est publié → `saving` reste vrai (seul `apply_save` le remet à faux) → **enregistrement perdu et éditeur bloqué**, les tentatives suivantes répondant `Busy`. Fenêtre étroite mais non nulle. | Ne jamais drainer par-dessus un `SaveProfile` : n'accepter la coalescence que si la commande drainée est elle aussi coalescible (sinon la remettre en file ou la traiter), et garantir qu'un save accepté produit **toujours** un `SaveOutcome` ; garde-fou « `saving` sans commande » (timeout ou compteur). | Test worker : worker occupé, file `[Refresh (dépilé), SaveProfile, Refresh]` ⇒ le save doit être traité **ou** rejeté avec un `SaveOutcome::Err`, jamais abandonné ; `saving` doit revenir à faux. |
| **G1-02** | **Moyen** | `daemon_client.rs:70` (`ADMIN_SAVE_TIMEOUT = 120 s`), `tcc-authz` (`DEFAULT_AUTHORIZATION_TIMEOUT = 120 s`), `service.rs:49` (`CONTROL_TIMEOUT = 30 s`), `transaction.rs:207-235` (second reload après rollback) | Le pire cas admin est **~150–180 s** (dialogue polkit 120 s + `ReloadConfig` 30 s, et jusqu'à 30 s de plus si un rollback déclenche un second rechargement) : la GUI peut afficher `Timeout` alors que l'admin **écrit ensuite** avec succès. Faux échec, formulaire conservé, risque de double soumission pendant que la première est encore en cours. | Porter le timeout client au-dessus du pire cas (≥ 240 s) ou borner le dialogue polkit côté admin ; documenter qu'un `Timeout` **n'annule pas** l'opération et que la GUI doit relire l'état avant de réessayer. | Faux admin lent (> 120 s) : vérifier le code renvoyé et l'état final ; test d'intégration avec un délai contrôlé. |
| **G1-03** | **Moyen** | `daemon_client.rs:609-625` (collection complète), `ProfileEditorPage.qml:135-137` (`savePossible` ignore `conflict`), `tcc-admin/src/service.rs` (aucune révision) | Écriture en **dernier écrivain gagne** : la GUI renvoie **toute** la collection issue de `last_profiles` (jusqu'à ~2 s de retard). Si un autre client admin a enregistré entre-temps, ses changements sont **écrasés silencieusement**, y compris sur d'autres profils. Le bandeau de conflit est affiché mais **n'empêche pas** « Enregistrer ». | Ajouter une **révision/etag** à `SaveProfilesJSON` (rejet si la collection a changé) ou bloquer `savePossible()` quand `conflict` est vrai (obliger « Recharger ») ; à défaut, documenter explicitement la sémantique et éviter d'écraser les profils non édités. | Test à deux clients : A modifie P1, B (stale) enregistre P2 ⇒ B doit être refusé (`Conflict`/`InUse` dédié) ou P1 préservé ; test QML `conflict ⇒ savePossible=false`. |
| **G1-04** | **Moyen** | `profile_edit.rs:49,52` (`onlineCores: u32`, `scalingMinKhz: u64`), `:319-327` (`apply_fields` écrit toujours `Some(...)`), `ProfileEditorPage.qml:255` (`cores` défaut 1), `:258-259` (`freqMax` défaut 0.4 GHz) | Le DTO **force** des champs que `tcc-core` modélise en `Option` (`onlineCores`, `scaling_min_frequency`, `scaling_max_frequency`) : `apply_fields` les réécrit **inconditionnellement**. Pour un profil où `onlineCores` est absent, le formulaire vaut `1` et un simple renommage enregistre `onlineCores = 1` → **le daemon limite le CPU à un cœur**. Les fréquences sont pires : `freqMax` absent devient `0.4` GHz et `freqMin` absent devient `0` (min < `MIN_CPU_FREQUENCY_KHZ`, donc refus de validation) ; si `freqMin` est connu mais `freqMax` ne l'est pas, un plafond à 400 MHz peut être enregistré. | Rendre ces champs `Option` dans `ProfileEdit` et **ne pas les modifier s'ils sont absents** (ou bloquer l'édition avec un badge « non disponible ») ; ne jamais envoyer une valeur inventée pour un champ inconnu. | Test Rust : profil sans `onlineCores` + édition du nom ⇒ `onlineCores` reste `None` ; test QML : champ inconnu ⇒ save désactivé. |
| **G1-05** | Faible | `daemon_client.rs:950-965` (`NameHasOwner`/`ListActivatableNames` sans timeout), `:1016-1037`, `:1057-1075` | 1) Ces deux appels n'ont **pas** de timeout explicite (contrairement à `Ping`/`GetContractVersion` à 2 s) : un bus lent bloque le worker. 2) Si le service est installé mais que l'activation échoue (binaire absent, `StartLimit`), `ListActivatableNames` le déclare disponible → l'erreur remonte en `Transport`/`Unknown`, pas en `AdminUnavailable`. | Encadrer les appels bus d'un timeout court ; mapper `ServiceUnknown`/échec d'activation vers `AdminUnavailable` ; re-prober immédiatement après un échec de transport. | Faux bus/serveur qui accepte le nom mais échoue à l'activation ⇒ code `AdminUnavailable`, pas `Unknown`. |
| **G1-06** | Faible | `daemon_client.rs:1062-1063` (`PolicyKitUnavailable` → `NotAuthorized`) ; `docs/ui/PROFILE_EDITING.md:51` (« ≤ 64 »), `:85`, `:176` | 1) `PolicyKitUnavailable` n'est **pas distingué** de `NotAuthorized` : l'admin ne l'émet pas (il replie sur `NotAuthorized`), donc la distinction est impossible côté GUI — la doc §5 le liste pourtant comme une erreur séparée. 2) La doc dit « nom ≤ 64 » alors que la limite réelle est **128** (`MAX_PROFILE_NAME_LEN`). 3) Le message `ReloadFailed` (« sauvegarde conservée ») est ambigu depuis A1-07 (rollback si précédent exploitable, sinon nouveau fichier conservé). | Aligner la doc (`PolicyKitUnavailable` = alias de `NotAuthorized` ou code distinct si l'admin évolue ; 128 caractères) et préciser le message `ReloadFailed` (état du fichier). | Revue de doc + test QML de mapping. |
| **G1-07** | Info | `ProfileEditorPage.qml:79-86` (`DemoData.refreshModes`) | La liste des taux proposés vient de `DemoData` : l'utilisateur peut enregistrer un taux **non supporté** par le connecteur, que l'agent refusera à l'activation (état d'erreur, pas de changement). | Exposer les modes réels via un futur contrat (`tccd`/agent) et n'afficher que ceux-là ; en attendant, signaler « valeur de démonstration ». | Test avec un taux absent des modes réels ⇒ avertissement UI. |
| **G1-08** | Info | `LiveData.qml:143-169` ; `tests/qml/tst_profile_{rules,editor}.qml` | `JSON.parse` n'est **pas libre** : `decode_profiles` (Rust) valide et re-sérialise la collection canonique avant QML ; `LiveData` vérifie la forme et retombe sur la démo. En revanche, la couverture de tests est partielle : le faux `Admin1` prouve succès + refus + payload stocké + probe, mais **pas** `Busy`, `ReloadFailed`, `Timeout`, ni le contrat incompatible ; les tests QML n'ont pas été rejoués localement. | Ajouter des cas faux-admin `Busy`/`ReloadFailed` et un test du drop de commande (G1-01) ; exécuter les tests QML en CI Qt 6.10 et archiver le résultat. | Nouvelles fonctions dans `daemon_client`/`tst_profile_editor`. |
| **G1-09** | Info | `daemon_client.rs:180-189,522-530` | `ClientError::detail` peut contenir un détail de transport (adresse de bus, nom de service) et `lastError` est publié tel quel vers QML. Aucun secret, mais un chemin/socket peut apparaître dans l'UI en cas d'erreur. | Vérifier l'affichage de `lastError` (texte brut, pas de HTML) et, si nécessaire, borner/nettoyer le détail avant publication. | Revue QML + test de sanitisation. |

### A.4 Conformité vérifiée

| Exigence | Preuve |
|---|---|
| Aucune écriture directe, `pkexec`, shell, chemin utilisateur | `grep` : aucun `fs::write`/`Command::new` dans `tcc-gui` ; seuls des appels D-Bus (`Admin1`, `tccd`) ; aucun paramètre de chemin dans `SaveProfile`/DTO |
| Payload borné et typé | DTO ≤ 64 Kio (`profile_edit.rs:28,247-250`), `deny_unknown_fields` (`:39-40`), collection sérialisée ≤ 1 Mio (`:299-302`), vérification client avant D-Bus (`daemon_client.rs:1017-1019`) |
| Pas d'injection de champs inconnus | `deny_unknown_fields` + `ProfileCollection::validate` après application (`profile_edit.rs:291-297`) ; QML ne peut pas ajouter de clé (DTO fixe) ; les champs `extra` viennent de la collection serveur, pas de QML |
| Presets non éditables | `apply_edit` refuse `is_preset()` (`:286-288`) ; UI `readOnly` (`ProfileEditorPage.qml:29-33`) ; l'admin filtre de toute façon les presets |
| IDs/noms validés | `validate_profile_id` (`:274-279`), nom non vide/≤128/sans contrôle (`:72-80`), unicité revalidée par `tcc-core` (`:438-447`) |
| Pas de `JSON.parse` libre côté QML | seul `LiveData.qml:145`, sur la collection **canonique validée** par Rust (`decode_profiles`, `daemon_client.rs:1112-1137`) ; forme vérifiée, repli démo |
| Détection sans activation root | `NameHasOwner` puis `ListActivatableNames` (`:950-965`) ; `Ping`/`GetContractVersion`/`GetVersion` **seulement si possédé** (`:968-995`), timeout 2 s, contrat comparé strictement (`:981-983`) |
| Erreurs sans chemin/contenu | mapping `Admin1.Error.*` → codes stables (`:1057-1075`) ; `sanitize_detail` rejette `/`, `\`, contrôles, non-ASCII, borne 120 (`profile_edit.rs:226-237`) ; QML affiche en texte brut |
| Formulaire conservé après refus | `apply_save` ne touche pas au formulaire ; QML teste `test_saveErrorKeepsFormAndDirty` |
| Un seul save | `saving` (`daemon_client.rs:472-481`) + `Busy` admin ; test `test_savingDisablesSave` |
| Conflit de profil actif signalé | `ProfileEditorPage.qml:185-188` (bandeau, pas d'écrasement du brouillon) |
| Contrat futur refusé | `ensure_admin_contract_compatible` → `AdminStatus::unavailable("Contract")` → bouton désactivé |
| Tests Rust | **50/50** (`cargo test -p tcc-gui --bins`, `dbus-run-session`) : DTO (14 tests), pont/erreurs/faux admin (29), probes, payload stocké, refus |

### A.5 Ce qui reste manuel / non testable localement

1. **Vrai polkit** : dialogue autorisé/refusé/annulé, `polkit` arrêté ⇒ `NotAuthorized`, aucune
   écriture ; vérifier que le formulaire est conservé et que le message est correct.
2. **Activation D-Bus réelle** : service endormi → « Enregistrer » doit l'activer une seule fois ;
   service installé mais activation impossible (binaire absent/`StartLimit`) → message
   « indisponible », pas une erreur de transport.
3. **Rollback réel** (`ReloadFailed`) et **timeout** (G1-02) : vérifier l'état du fichier et de l'UI.
4. **Deux clients** : A modifie P1, B enregistre P2 depuis une collection périmée (G1-03) ; vérifier
   l'absence d'écrasement silencieux ou le refus.
5. **Tests QML** sur Qt 6.10/Arch (`qmltestrunner`), y compris conflit et conservation du formulaire.
6. **Budget** : profil proche de 1 Mio, réponse `GetProfilesJSON` proche de 4 Mio, latence du worker.
7. **Matériel réel** : un profil sans `onlineCores` (G1-04) ne doit pas brider le CPU après un
   renommage.

### A.6 Recommandation

L'écriture GUI **ne change pas le verdict de sécurité** de la phase 1 : elle reste non privilégiée,
passe par le service admin et bénéficie des mêmes garde-fous (polkit, transaction, audit). Les
findings G1-01/G1-03/G1-04 doivent être corrigés **avant de déclarer l'édition de profil « terminée »**
(perte silencieuse, écrasement inter-clients, modification matérielle non demandée) ; G1-02 avant de
valider l'UX de sauvegarde. Aucun de ces findings ne bloque `TCC_WITH_ADMIN=1` **du point de vue
sécurité**, mais ils bloquent la porte qualité « pas de perte silencieuse » de `QUALITY.md` §1.

---

## Addendum — réglages globaux via Admin1 (2026-11-06)

### S.1 Verdict

La page Réglages n'enregistre que **quatre booléens** (`fahrenheit`, `cpuSettingsEnabled`,
`fanControlEnabled`, `keyboardBacklightControlEnabled`) via `Admin1.SaveSettingsJSON`. Le DTO est
fermé (`deny_unknown_fields`), borné (64 Kio), appliqué sur une copie des réglages relus, et la
collection complète est revalidée par `tccd-admin`. **Aucune écriture directe, aucun chemin, aucun
shell, aucun `JSON.parse` de production.** Aucun finding Critique/Élevé ; **2 Faibles et 2 Info**.

Exécuté : `cargo test -p tcc-gui --bins` sous `dbus-run-session` ⇒ **86/86**, dont 13 tests
spécifiques aux réglages. Non exécuté : tests QML (`qmltestrunner` Qt 6.10 requis), vrai polkit,
service admin réel.

### S.2 Findings

| ID | Sév. | Fichier:ligne | Impact | Remédiation | Test |
|---|---|---|---|---|---|
| **S1-01** | **Fermé** (Faible) | `daemon_client.rs:2561-2583` (`decode_settings`) | `decode_settings` vérifie que la réponse est un objet JSON, désérialise `Settings`, puis **appelle `Settings::validate()`** (`:2581-2583`) ; un modèle incohérent est rejeté avant d'alimenter le formulaire. | — (corrigé) | Tests : schéma faux / référence absente / champ obligatoire manquant ⇒ `GetSettingsJSON` rejeté. |
| **S1-02** | **Ouvert** (Faible / A) | `settings_edit.rs:94-104` ; `daemon_client.rs:863-874` | Le conflit est un **check-then-act** : `base == current` est vérifié avant l'appel, puis l'admin écrit sans révision. Un autre client peut modifier les réglages entre la relecture et `SaveSettingsJSON` ; ses changements sont écrasés (dernier écrivain gagne). La course est **documentée** dans le code (`settings_edit.rs:331`), mais non fermée côté `Admin1`. | Ajouter une révision/etag à `SaveSettingsJSON` (rejet `Conflict` si la version a changé) ; à défaut, conserver le comportement et le documenter dans `DBUS_API.md`. | Test à deux clients : A modifie `chargingStartThreshold`, B enregistre un booléen depuis une base périmée ⇒ `Conflict` ou changement de A préservé. |
| **S1-03** | Info | `daemon_client.rs:700-715` (`saving`) et `:575-593` (`saving_settings`) ; `service.rs` `write_gate` | Les sauvegardes de profil et de réglages ont **deux drapeaux distincts** : elles peuvent être en vol simultanément ; l'admin sérialise et renvoie `Busy` à la seconde. Aucune perte (le formulaire est conservé), mais l'utilisateur peut voir un refus évitable. | Optionnel : un drapeau global de mutation côté GUI, ou documenter que `Busy` est attendu en cas d'édition simultanée. | Test QML : deux sauvegardes rapprochées ⇒ la seconde est refusée avec `Busy`, formulaire conservé. |
| **S1-04** | Info | `daemon_client.rs:595-613`, `:1101-1125` ; `ADMIN_SAVE_STALL` (`:84`) | Si le worker meurt **après** avoir accepté `SaveSettings` (canal fermé, thread Qt disparu), `apply_save_settings` n'est jamais appelé et `savingSettings` reste vrai ; il n'est libéré qu'à la **tentative suivante** (contrôle `saving_settings_since > 300 s`), pas par un timer. | Réinitialiser `savingSettings` à la perte de connexion/arrêt du worker, ou ajouter un timer QML de secours ; test de fermeture du canal. | Test : worker arrêté après acceptation ⇒ l'UI se débloque sans attendre 300 s. |

### S.3 Conformité vérifiée

| Exigence | Preuve |
|---|---|
| Seuls les 4 booléens sont modifiés | `SettingsEdit` à 4 champs (`settings_edit.rs:25-32`) ; `apply_settings_edit` n'écrit que ces 4 champs (`:66-81`) ; test `only_allowed_fields_are_changed` |
| `extra`/`stateMap`/`stateMatrix`/charge préservés | `base.clone()` puis 4 affectations ; tests `only_allowed_fields_are_changed`, `state_matrix_serialization_is_preserved` (extra, charge, seuils, `stateMatrix`) |
| Payload ≤ 1 Mio | DTO ≤ 64 Kio (`settings_edit.rs:19-20`, `:50-55`) et `Settings` sérialisé ≤ 1 Mio (`:21-22`, `:76-79`) ; vérification avant D-Bus (`daemon_client.rs:1504-1506`) ; test `oversized_settings_payload_is_rejected` |
| `deny_unknown_fields` | `settings_edit.rs:26` ; tests `unknown_field_is_rejected`, `missing_field_is_rejected` |
| Conflit avant écriture | `prepare_settings_save` compare `base` et `current` (`settings_edit.rs:94-104`) ; le worker relit les réglages juste avant l'envoi (`daemon_client.rs:863-874`) ; test `concurrent_change_is_a_conflict`, `settings_pipeline_rejects_concurrent_change` — **réserve S1-02** (race résiduelle) |
| `savingSettings` toujours libéré | `apply_save_settings` le remet à faux sur `Ok`/`Err` (`daemon_client.rs:1101-1125`) ; `try_queue` en échec le libère (`:602-613`) ; **réserve S1-04** (worker mort) |
| Erreurs assainies | `SaveError::detail()` applique `sanitize_detail` (ASCII graphique, sans `/`/`\`, contrôles, borne 120) (`profile_edit.rs:235-258`) ; QML affiche en texte brut (`SettingsPage.qml:259-264`) |
| Aucun chemin / shell / écriture directe | `grep` : aucun `fs::write`, `Command::new`, `pkexec`, `/etc`, `/var/lib` dans `settings_edit.rs` ni `SettingsPage.qml` ; seul `JSON.stringify` côté QML (`SettingsPage.qml:145-146`), jamais `JSON.parse` en production |
| Un seul DTO par action | `SettingsPage.qml:134-146` construit exactement 4 clés ; test `toggleSendsFullDto` vérifie `Object.keys(dto).length == 4` |
| Détection admin / contrat | `AdminState.readySettings` + `save_settings` refusent si admin absent ; contrat comparé strictement (phase 1) |

### S.4 Ce qui reste manuel

1. Vrai polkit (autorisé/refusé/annulé) et service `tccd-admin` réel.
2. Tests QML sur Qt 6.10 (`tst_settings_page.qml`), non rejoués ici.
3. Deux clients modifiant les réglages en parallèle (S1-02) ; vérifier le comportement `Conflict`.
4. Arrêt du worker/fermeture du canal pendant une sauvegarde (S1-04).
5. Budget : `Settings` proche de 1 Mio (champ `extra` volumineux) et latence du worker.

### S.5 Recommandation

La surface est **conforme aux garde-fous de la phase 1** et ne bloque pas l'activation. S1-01 est
**corrigé** (`decode_settings` valide `Settings`) ; **S1-02 reste ouvert** (Faible) : course
inter-clients sur `SaveSettingsJSON`, à fermer par une révision côté admin plutôt que dans la GUI,
ou à accepter explicitement.

---

## Addendum — tombstones d'IDs de profils (2026-11-06)

### T.1 Verdict

`Settings.deletedProfileIds` est implémenté dans `tcc-core` (validation, borne **1024**, IDs gérés
par TCC refusés) et **branché** dans `tcc-admin` : `SaveProfilesJSON` refuse un nouvel ID tombstoné,
ajoute les IDs supprimés avant écriture, et persiste `profiles.json` + `settings.json` dans une
transaction à un seul rechargement. `SaveSettingsJSON` est append-only. **Aucun Critique, Élevé ni
Moyen ouvert.** **T1-01 est fermé** (correctif + tests) et **AD-R13 est fermé** (diff d'état seul).
Restent **3 Info** dans cet addendum (T1-02…T1-04), les Info A1-08/A1-10/A1-12/A1-13/A1-14, et
**S1-02 (Faible, addendum S)**, seul finding **Faible** code ouvert de la revue admin (course
inter-clients sur `SaveSettingsJSON`, sans révision/CAS). Le mécanisme ferme l'écart AD-R6 hors
root ; la réutilisation d'ID n'était pas une frontière de privilège.

Exécuté : `cargo test -p tcc-admin` ⇒ **33 lib + 5 binaire + 12 `admin_dbus` + 12 `transaction` +
4 `cli`**, 0 échec ; `cargo test -p tcc-core --lib settings::` ⇒ **14/14** (dont
`mark_profile_deleted_*`). Tests ciblés : `etc_profile_is_not_tombstoned_by_state_diff`,
`reload_failure_without_previous_settings_restores_absence`,
`reload_failure_restores_absence_of_settings_file`. Non exécuté : vrai polkit, crash entre les deux
`rename`, service admin réel.

### T.2 Correctifs fermés

**T1-01 — fermé (Faible / A).** `RestoreTarget` mémorise désormais `existed` et `remove_if_absent`
(`transaction.rs:142-160`) ; la transaction double fichier lit les deux cibles avec
`restore_unusable=true` et `remove_if_absent=true` (`:246-256`) ; au rollback, `restore_target`
restaure les octets exacts ou **supprime** un fichier absent avant (`:358-368`), via `remove_file`
+ `fsync` du répertoire (`:335-353`). Un échec de suppression ⇒ `WriteFailed`. Tests :
`reload_failure_restores_absence_of_settings_file` (`tests/transaction.rs:422`) et
`reload_failure_without_previous_settings_restores_absence` (`service.rs:1324`). La transaction
mono-fichier conserve son comportement (`failed_save_without_previous_keeps_valid_file`).

**AD-R13 — fermé.** `service.rs:514-521` utilise `load_profiles_state_only_no_recovery`
(`config.rs:606-614`) : la collection « précédente » est uniquement `profiles.json` de
`/var/lib`, **sans repli `/etc`**. Test `etc_profile_is_not_tombstoned_by_state_diff`
(`service.rs:1179`) : un profil `/etc` n'est ni tombstoné ni modifié, et reste importable dans
l'état.

### T.3 Findings ouverts (Info)

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **T1-02** | Info | `service.rs:602-616` | `SaveSettingsJSON` refuse un tombstone **ajouté**, mais **réajoute silencieusement** un tombstone omis au lieu de renvoyer `Validation`. | Fail-safe (aucune suppression), mais un client qui croit avoir purgé reçoit `true` sans effet sur ce champ ; audit `reload=ok` trompeur pour ce champ. | Documenter dans `DBUS_API.md` ; optionnel : renvoyer `Validation("deletedProfileIds")` sur toute différence (plus strict, casse les clients qui ne roundtripent pas le champ). | Test : payload sans tombstone ⇒ soit réajouté (actuel), soit `Validation` (option). |
| **T1-03** | Info | `tccd` (aucune occurrence de `deleted_profile_ids`) | `tccd` ne vérifie pas qu'un profil **vivant** est tombstoné ; seul l'admin applique l'invariant. Un état incohérent (root, migration) est chargé silencieusement. | Pas d'escalade ; diagnostic tardif, l'UI peut afficher un profil dont l'ID est « supprimé ». | `warn!` au chargement si `settings.is_profile_id_deleted(id)` pour un profil de la collection ; ne pas refuser (évite la perte de config). | Test daemon : config incohérente ⇒ warning, démarrage réussi. |
| **T1-04** | Info | `profile_transfer.rs:245-252` | L'import ne régénère l'ID que sur conflit ou ID invalide ; un ID **libre mais tombstoné** est conservé, puis l'admin rejette **tout** l'import (`Validation`). | Fail-closed, aucune escalade ; UX : import entier refusé pour un seul profil. | Exposer `deletedProfileIds` au chemin d'import et régénérer l'ID en collision (même règle que le conflit) ; message dédié. | Test : import avec un ID tombstoné ⇒ ID régénéré, import accepté. |

### T.4 Conformité vérifiée

| Exigence | Preuve |
|---|---|
| Borne 1024, IDs valides, uniques, non gérés par TCC | `validate.rs:38-43,731-755` ; `settings.rs:297-354` ; tests `settings.rs:494-578` |
| Refus d'un nouvel ID tombstoné | `service.rs:515-528` ; test `deleted_profile_id_cannot_be_recreated` (`:1059`) |
| Ajout des IDs supprimés **avant** écriture | `service.rs:526-552` ; `mark_profile_deleted` idempotent |
| Suppression + tombstone : même transaction | `service.rs:555-578` ; `transaction.rs:232-305` ; tests `tests/transaction.rs:323,379` |
| Diff de tombstones limité à l'état `/var/lib` (AD-R13 fermé) | `load_profiles_state_only_no_recovery` (`config.rs:606-614`) appelé par `service.rs:514-521` ; test `etc_profile_is_not_tombstoned_by_state_diff` (`service.rs:1179`) |
| Rollback des deux sur reload refusé, y compris suppression d'un `settings.json` créé (T1-01 fermé) | `RestoreTarget.existed`/`remove_if_absent` (`transaction.rs:142-160`) ; `restore_target`/`remove_file` (`:335-383`) ; tests `tests/transaction.rs:422`, `service.rs:1324` ; test nominal `reload_failure_rolls_back_profiles_and_settings` (`service.rs:1213`) |
| `.bak` 0600 par fichier, cible 0644 | `config.rs:1065,1080` ; tests `tests/transaction.rs:77,107` |
| `SaveSettingsJSON` append-only | `service.rs:602-616` ; tests `tombstones_cannot_be_removed_via_settings`, `tombstones_cannot_be_added_via_settings` |
| Aucun payload/ID dans l'audit | `audit_tombstones` (`service.rs:427-441`) : compteurs seulement |
| Aucune nouvelle surface D-Bus/polkit | aucun membre ajouté ; `SaveProfilesJSON`/`SaveSettingsJSON` déjà `…profiles`/`…settings` |
| `SaveProfilesJSON` n'écrit que des tombstones dans `settings.json` | settings relus sur disque puis `mark_profile_deleted` (`service.rs:514-552`) ; aucun champ client (température, charge, matrice) |

### T.5 Ce qui reste manuel

1. Vrai polkit : suppression de profil (dialogue, refus, annulation), `SaveSettingsJSON` sans tombstone.
2. Crash entre les deux `rename` (couper l'admin entre `profiles.json` et `settings.json`) : vérifier
   la paire, le retry et l'absence de tombstone (AD-R7).
3. `settings.json` absent : cas couvert par les tests (`tests/transaction.rs:422`,
   `service.rs:1324`) ; rejouer sur le service réel avec un `ReloadConfig` forcé en échec.
4. `.bak` : contenu et mode 0600 des **deux** fichiers ; restauration manuelle d'une paire cohérente.
5. `tccd` : état incohérent (profil vivant tombstoné, root/migration) ⇒ warning attendu (T1-03).


