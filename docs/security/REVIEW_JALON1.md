# Revue de sécurité — jalon 1

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue statique du jalon 1 (workspace Rust, packaging, docs) |
| Références | `AGENTS.md`, `docs/ARCHITECTURE.md` §3.10, `docs/security/THREAT_MODEL.md` (v1.0), `docs/security/DECISIONS.md`, `docs/security/CHECKLIST.md` |
| Portée code | `crates/{tcc-core,tcc-ipc,tcc-hw,tcc-daemon,tcc-user,tcc-gui,tcc-cli}`, `packaging/**`, `.github/workflows/ci.yml`, `deny.toml`, `docs/{DBUS_API.md,linux/PLATFORM_DESIGN.md,ui/UI_DESIGN.md}` |
| Verdict | **Jalon 1 non publiable en l'état** : 5 findings bloquants, tous dans l'intégration système (polkit, politique D-Bus, units, anti-DoS). Le cœur Rust est globalement sain. |

> **Périmètre 2026-11-06 — LUKS hors périmètre (décision utilisateur).** Les mentions LUKS de ce
> document sont **historiques** (état du jalon 1 / proposition de l'époque) ; elles ne sont pas un
> design à implémenter. Aucune action `luks-change-key`, méthode `Luks*`, capacité ou accès
> `cryptsetup` ne doit être ajouté. D5 reste une règle historique pour tout futur secret.

---

## Statut des corrections (2026-11-06)

> **Historique.** Les mentions « aucun `CheckAuthorization` » des §2/§6 et de l'annexe C
décrivent l'état **à la date de la revue initiale** (avant `tcc-authz`/`tcc-admin`) ; l'état courant
est décrit ci-dessous et dans `REVIEW_ADMIN_PHASE1.md`.
>
> **Instantané ~15:00, dépôt en mouvement.** Les constats des §1 à §6 (verdicts de §1.1, risques de
> §1.2, findings, conformité, remédiations, checklist) restent l'état d'origine et ne sont **pas**
> réécrits : ce tableau est la seule source de vérité sur l'avancement. Plusieurs y sont depuis
> corrigés — par exemple §1.1 indique « `clippy.toml` absent » (il existe, R-17), le packaging est
> noté « bloquant » (R-01/R-02/R-06/R-07/R-09 sont fermés).
> `Fermé` exige une preuve présente dans le dépôt ; `Partiel` = une partie du risque subsiste ;
> `Ouvert` = rien n'a changé côté risque.
>
> **Preuves rejouées par `security`** : lecture des fichiers cités, `validate.sh --repo`
> (**133/133** à l'instantané de cette section ; **220/220** depuis l'ajout des contrôles
> Admin1/Control1), `systemd-analyze verify|security` (3.3 / **1.6** / 3.6), `cargo test -p tcc-hw
> --lib` (182) et `--test hardware_workers` (58). **Non rejoués ici** : `dbus-policy-test.sh` (root
> requis, sortie 77), `lintian`/`rpmlint`/`namcap` et le décompte « policy 17/17 » — rapportés par
> `packaging-release`, non reproduits dans cette session (outils absents). Les cellules ci-dessous
> ne s'appuient donc que sur ce qui est vérifiable dans le dépôt.

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **R-01** | **Fermé** | `packaging/polkit/…policy` : 9 actions, **aucune** annotation `exec.path`/`exec.argv1`/`allow_gui` (grep vide) ; garde `validate.sh:368` ; `validate.sh --repo` 133/133 |
| **R-02** | **Fermé** | `dbus.conf:44-47` (deny global) puis allows par membre (`:61-133`) ; plus aucun `allow send_destination` sans `send_interface`/`send_member` ; garde `validate.sh:351-360` ; test fonctionnel root à exécuter |
| **R-03** | **Partiel** | rate limit 5/s burst 10 + audit `caller`/`caller_uid` + id ≤ 64 o (`dbus.rs:129-176`) ; bail capteurs 10 s plafonné 5 min + lecture 2/s (`sensor_lease.rs:30`, `rate_limit.rs:26`) ; **polkit toujours absent** et portée **globale** (matériel de la machine, pas de session) — cf. `ADMIN_DESIGN.md` AD-7/AD-8 |
| **R-04** | **Partiel** | `tcc-authz` implémente `CheckAuthorization` (sujet `system-bus-name`, fail-closed, challenge refusé, limiteur nom+UID, `DialogGate` RAII) et `tcc-admin` phase 1 l'applique à `SaveProfilesJSON`/`SaveSettingsJSON` (`tcc-admin/src/service.rs:282-342,493-578`) ; `Control1.ReloadConfig` est root-only revérifié dans le code (`tcc-daemon/src/dbus.rs:945-995`). **Reste ouvert** : l'authz interne de `tccd` pour les 8 setters `Manager1` (aujourd'hui refusés par la policy, `dbus.conf:111-138`) et la portée globale du profil temporaire (R-03). Preuves : `REVIEW_ADMIN_PHASE1.md` ; `tcc-authz` **19** tests, `tcc-admin` lib **24** + `admin_dbus` **7/7** + transaction **7** ; **821 tests workspace** (`docs/STATUS.md:84`). |
| **R-05** | **Partiel** | `MemoryMax=256M`, `TasksMax=64`, `CPUQuota=25%` (`tccd.service:93-95`) ; lecture snapshot 2/s burst 5 ; **manquent** le `Semaphore` de handlers concurrents et toute borne **avant** désérialisation (zbus accepte 128 Mio) → risque résiduel AD-R1 |
| **R-06** | **Fermé** | unit retirée du paquet, conservée en `.example` non installé (`tccd-sleep.service.example:3-20`), garde `validate.sh:150-157` ; **suivi** : `PrepareForSleep` n'est pas implémenté dans `tccd` (réapplication au réveil absente) |
| **R-07** | **Fermé** | `install-tree.sh:64-68` refuse `TCC_WITH_ADMIN=1` sans binaire ; unit non installée ; `CapabilityBoundingSet=` vide, `Restart=no`, pas de `[Install]`, `DevicePolicy=closed` sans allow (`tccd-admin.service:27,64,73-76`) |
| **R-08** | **Fermé** | ordre refus→allow (`dbus.conf:44-47`) ; `validate.sh:357-360` compare la première règle deny et la première allow ; la règle morte a disparu |
| **R-09** | **Fermé** | 9 actions `…tccd1.*`, `allow_any=no`/`allow_inactive=no`, actions sensibles sans `_keep` (Prime/Tomte/paquets ; LUKS hors périmètre) (`policy:123-136`) ; garde `validate.sh:385-405`. Affinage `profiles` / retrait d'`assign-profile` porté par `ADMIN_DESIGN.md` §3.2 |
| **R-10** | **Partiel** | proxy **persistant** + budget unique de 3 s par cycle (`tcc-user/src/agent.rs:110-150`) ; **souscription `StateChanged` toujours absente** : polling 2 s (`agent.rs:36`) |
| **R-11** | **Partiel** | timeout `KSCREEN_TIMEOUT` + `run_command_with_timeout` (`kde.rs:151,255-290`) ; `LD_PRELOAD`/`LD_AUDIT` retirés mais **pas `env_clear()`** ; application toujours **persistante** `output.<name>.mode.<id>` (`kde.rs:9`) |
| **R-12** | **Ouvert** | `DisplayModeRequest.internal_output` toujours présent (`tcc-ipc/src/display.rs:297`) |
| **R-13** | **Ouvert** | `is_internal_connector` toujours par préfixe (`tcc-hw/src/display.rs:31-36`) : pas de regex `-<chiffres>` ni recoupement `/sys/class/drm` |
| **R-14** | **Partiel** | GUI : `TCC_DBUS_*`/`TCC_QML_DIR` gatés par `dev-overrides` (`daemon_client.rs:778-800`, `main.rs:15`) ; **CLI et agent** lisent encore `TCC_DBUS_SERVICE`/`TCC_DBUS_BUS` sans gate (`tcc-cli/src/main.rs:18,26`, `tcc-user/src/agent.rs:178,186`) |
| **R-15** | **Fermé** | worker unique `tcc-dbus` + `sync_channel(8)` borné (`daemon_client.rs:48,254`) ; plus de thread par invokable |
| **R-16** | **Fermé** | `MAX_RESPONSE_BYTES = 4 Mio` vérifié **avant** `serde_json::from_str` (`daemon_client.rs:43,703-707`), tests dédiés (`:824-1055`) |
| **R-17** | **Partiel** | `clippy.toml` présent (msrv) mais **sans `disallowed-methods`** (choix documenté `clippy.toml:1-20`) ; `#![forbid(unsafe_code)]` + `deny(unwrap/expect)` désormais dans `tccd`, `tcc-cli`, `tcc-gui` |
| **R-18** | **Ouvert** | aucun `.bak` de migration (`grep "\.bak" crates/tcc-core/src/migration.rs` = 0) ; `packaging/README.md:73-75` l'annonce toujours |
| **R-19** | **Ouvert** | `GetSettingsJSON` renvoie l'objet complet (`dbus.rs:99-104`) : pas de vue filtrée, pas de test sur `extra` |
| **R-20** | **Fermé** | `watch_connection` (`dbus.rs:306-333`) + arrêt sur perte de bus (`main.rs:319-325,334-336`) ; `StartLimitIntervalSec=60`/`Burst=5` (`tccd.service:15-16`) |
| **R-21** | **Fermé** | `tccd-user.service:49-52` : `UMask=0077`, `LimitCORE=0`, `MemoryMax=128M`, `TasksMax=32` ; dépendance user-namespaces documentée (`:21-25`, `packaging/SECURITY.md:76-80`) |
| **R-22** | **Partiel** | `Wants=`/`After=modprobe@tuxedo_io.service` (`tccd.service:7-16`) + `DeviceAllow=/dev/tuxedo_io rw` (`:64-66`) ; **aucune validation matérielle** de l'ordre de chargement → `REVIEW_HARDWARE.md` H-13 |
| **R-23** | **Partiel** | `--locked` (9 occurrences) et `dbus-run-session` pour les tests daemon ; **aucun job packaging** dans `.github/workflows/ci.yml` (`validate.sh`, `systemd-analyze`, `xmllint`, `dbus-policy-test.sh`) ; actions épinglées par tag |

**Décompte : 10 fermés, 9 partiels, 4 ouverts.** Les findings les plus structurants encore **non
fermés** sont **R-04** (authz interne de `tccd` manquante ; polkit est appliqué à `Admin1`) et
**R-03** (portée globale du profil temporaire). `tccd-admin` phase 1 est implémenté et validé en
test ; son activation reste conditionnée aux preuves système (`REVIEW_ADMIN_PHASE1.md` §10).

---

## 0. Périmètre, méthode et avertissements

### 0.1 Ce qui a été lu et analysé

- l'intégralité de `docs/security/{THREAT_MODEL.md,DECISIONS.md,CHECKLIST.md}` ;
- tous les fichiers `.rs` des 7 crates (lecture intégrale des modules sensibles :
  `tcc-core/src/{config.rs,migration.rs,validate.rs}`, `tcc-ipc/src/{lib.rs,state.rs,session.rs,display.rs}`,
  `tcc-hw/src/{sysfs.rs,tuxedo_io.rs,cpu.rs,display.rs}`,
  `tcc-daemon/src/*`, `tcc-user/src/*` et ses backends, `tcc-gui/src/daemon_client.rs`, `tcc-cli/src/main.rs`) ;
- `packaging/**` en totalité (systemd, D-Bus, polkit, tmpfiles, udev, README) ;
- `deny.toml`, `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`, `rust-toolchain.toml` ;
- `docs/DBUS_API.md`, `docs/linux/PLATFORM_DESIGN.md` §6, `docs/STATUS.md` ;
- le code original de référence pour les points de comparaison
  (`src/dist-data/com.tuxedocomputers.tomte-access.policy`, `tomteAPI.ts`, `dist-data/*dbus.conf`).

### 0.2 Ce qui a été exécuté (et peut donc être cité comme preuve)

| Commande | Résultat |
|---|---|
| `systemd-analyze verify packaging/systemd/tccd.service` | seule erreur : binaire `/usr/libexec/tuxedo-rust-center/tccd` absent (attendu avant packaging) |
| `systemd-analyze security --offline=true` sur les 4 units | scores **3.5 (tccd)**, **4.0 (tccd-user)**, **4.4 (tccd-admin)**, **9.2 UNSAFE (tccd-sleep)** |
| `systemd-analyze security` avec `DeviceAllow=/dev/definitely-absent-node rw` | systemd accepte et conserve la spécification de chemin (aucune erreur de syntaxe) |
| lecture de `man dbus-daemon`, `man pkexec`, `man systemd.resource-control` | sémantique des règles D-Bus, des annotations polkit et de `DeviceAllow` |
| lecture du code de `zbus 5.13.2` (registry local) | `DoNotQueue` toujours actif ; `MAX_MESSAGE_SIZE = 128 Mio` ; dispatch des méthodes en tâches concurrentes |
| script d'analyse AST-naïf (`#[cfg(test)]` exclu) | **0** `unwrap()`/`expect()` dans le code de production des 7 crates |

### 0.3 Ce qui n'a **pas** été fait — hypothèses non vérifiées

1. **Aucun test dynamique privilégié** : `tccd` n'a pas été lancé en root, aucun `busctl call` réel n'a été
   exécuté contre le bus système, aucune policy D-Bus n'a été chargée, aucun `pkcheck` n'a été lancé.
   Les findings D-Bus/polkit sont donc **statiques**, appuyés sur la documentation officielle citée.
2. **`cargo deny` et `cargo audit` n'ont pas été exécutés** : les binaires `cargo-deny` et `cargo-audit`
   sont absents de l'environnement de revue. Seule la configuration (`deny.toml`) et le workflow ont été relus.
3. **Aucun test matériel** : `DeviceAllow=/dev/tuxedo_io`, `PrivateDevices=no`, `ProtectKernelTunables=no`
   et `CapabilityBoundingSet=` vide n'ont pas été validés sur laptop TUXEDO.
4. **Comportement de `Type=dbus` à la perte du nom** (bus redémarré) : non testé ; voir R-20, formulé
   comme un point à vérifier et non comme un défaut avéré.
5. **Résolution de `DeviceAllow` par chemin quand le module n'est pas chargé** : non testée ; voir R-22.
6. Aucun fuzzing, aucun pentest, aucune analyse de binaire (les binaires ne sont pas construits ici).

---

## 1. Résumé exécutif

### 1.1 Verdict par domaine

| Domaine | Verdict | Commentaire |
|---|---|---|
| `tcc-core` (config/validation/migration) | **Bon** | Écritures atomiques, `O_NOFOLLOW`, vérification propriétaire/mode, bornes typées, `.bak` sur corruption. Un seul manque : pas de `.bak` du legacy `/etc/tcc` malgré la doc. |
| `tcc-hw` (sysfs/ioctl) | **Bon** | Chemins confinés, `O_NOFOLLOW` en écriture, `fstat` post-ouverture, valeur unique par `write`, rejet des caractères de contrôle, `unsafe` limité à 4 appels ioctl commentés. |
| `tcc-ipc` | **Bon** | Types bornés, `#[forbid(unsafe_code)]`, validation de version de contrat. Un champ à retirer (`internal_output` fourni par le client). |
| `tcc-user` | **Correct** | Refus root, validation de socket/compositeur, timeouts, réponses bornées, aucune surface D-Bus entrante. Points faibles : polling + reconnexion D-Bus à chaque cycle, `kscreen-doctor` sans timeout et persistant. |
| `tcc-daemon` | **Insuffisant pour la mise en réseau locale** | Le code est propre, mais la politique D-Bus, l'absence de polkit, de rate limiting et de bornes mémoire rendent la surface inacceptable dès qu'une méthode d'écriture existe. |
| `tcc-gui` / `tcc-cli` | **Correct** | Aucun privilège, validation des réponses, `PlainText` partout. Points faibles : overrides d'environnement non gatés, threads non bornés, pas de plafond de taille sur les réponses. |
| `packaging` | **Bloquant** | Polkit contient une annotation `exec.path` (primitive d'exécution root) ; politique D-Bus ouverte ; `tccd-admin` pointe vers un binaire inexistant avec `CAP_SYS_ADMIN` ; `tccd-sleep` note 9.2 UNSAFE. |
| CI / supply chain | **Partiel** | `deny.toml` correct, `Cargo.lock` sans dépendance git, un seul `build.rs` (cxx-qt). Manque : `--locked`, actions non épinglées par SHA, `clippy.toml` absent, pas de test de politique D-Bus/polkit. |

### 1.2 Les cinq risques principaux

| # | Risque | Sévérité | Où |
|---|---|---|---|
| 1 | **`pkexec` root via l'action polkit `tomte`** : `org.freedesktop.policykit.exec.path=/usr/bin/tuxedo-tomte` + `allow_gui=true` transforment une action « tomte » en exécution root d'un binaire tiers avec des arguments arbitraires, avec cache `auth_admin_keep` (5 min). C'est exactement F-06/S-12 de l'original, que D5 interdit. | **Élevé / B** | `packaging/polkit/com.tuxedocomputers.tccd.policy:74-75` |
| 2 | **Politique D-Bus ouverte** : tout utilisateur peut envoyer *n'importe quel message* à `tccd`, sans restriction d'interface ni de membre ; toute méthode future sera exposée par défaut. | **Élevé / B** | `packaging/dbus/com.tuxedocomputers.tccd.conf:30-39` |
| 3 | **`SetTempProfile` non authentifié et global à la machine** : n'importe quel processus local change le profil matériel de tous les utilisateurs, sans trace de l'appelant ni limite de fréquence. | **Élevé / B** | `crates/tcc-daemon/src/dbus.rs:82-105`, `runtime.rs:188-210` |
| 4 | **Aucune application de polkit, alors que la doc et le packaging l'affirment** : la doc dit « méthodes sensibles contrôlées par polkit » et les actions sont installées, mais aucun code d'autorisation n'existe. La prochaine méthode d'écriture (jalon 2) sera donc non protégée par construction. | **Élevé / B** (risque de régression) | absence dans `crates/tcc-daemon/src/**`, contradiction avec `docs/linux/PLATFORM_DESIGN.md:691-692,710-711` et `docs/DBUS_API.md:150-157` |
| 5 | **Absence d'anti-DoS** : messages acceptés jusqu'à 128 Mio (limite zbus), dispatch concurrent, aucune limite de débit, aucune `MemoryMax`/`TasksMax` dans l'unit → un utilisateur local peut faire gonfler la RSS du daemon root et perdre le contrôle thermique. | **Moyen / B** | `crates/tcc-daemon/src/dbus.rs`, `packaging/systemd/tccd.service:7-27` |

> **Note datée (2026-11-06)** : le risque n°4 est **partiellement traité** — `tcc-authz`/`tcc-admin`
> appliquent `CheckAuthorization` à `Admin1` (voir « Statut des corrections », R-04). Les setters
> `Manager1` de `tccd` restent sans autorisation interne (fermés par la policy).

### 1.3 Répartition

**Bloquants (B) : 8** — R-01, R-02, R-03, R-04, R-05, R-07, R-09, R-17.
**Importants (I) : 9** — R-06, R-08, R-10, R-11, R-12, R-13, R-21, R-22, R-23.
**Améliorations (A) : 6** — R-14, R-15, R-16, R-18, R-19, R-20 (dont 2 écarts documentaires,
R-18 et §2.4).

Aucun finding **Critique** (pas de root immédiat, pas de fuite de secret) : à ce stade, **aucun code LUKS,
aucun secret, aucune commande shell n'existe**. Le risque critique est **anticipé** et se matérialisera
au jalon 5 si R-01/R-04 ne sont pas traités maintenant.

---

## 2. Findings

### 2.1 Bloquants

---

#### R-01 — Action polkit `tomte` : primitive d'exécution root (`exec.path` + `allow_gui`)

| | |
|---|---|
| **Sévérité** | Élevé — **B** |
| **Fichier** | `packaging/polkit/com.tuxedocomputers.tccd.policy:62-76` (annotations : l. 74-75) |
| **Références** | S-12, F-06, D5, checklist §6.3 (« Aucun `exec.path`, aucun `allow_gui` ») |

**Constat.** L'action `com.tuxedocomputers.tccd.tomte` déclare :

```xml
<annotate key="org.freedesktop.policykit.exec.path">/usr/bin/tuxedo-tomte</annotate>
<annotate key="org.freedesktop.policykit.exec.allow_gui">true</annotate>
```

`pkexec(8)` documente ce comportement (vérifié localement) : « To use another action, use the
`org.freedesktop.policykit.exec.path` annotation on an action with the value set to the full path of
the program. » Autrement dit, `pkexec /usr/bin/tuxedo-tomte <n'importe quels arguments>` consulte
**cette action**, avec `allow_any=allow_inactive=allow_active=auth_admin_keep` et `allow_gui=true`.
Le fichier est une copie quasi littérale de `tuxedo-control-center-master/src/dist-data/com.tuxedocomputers.tomte-access.policy:10-25`
(seul l'ID d'action change).

**Scénario.** Un utilisateur local active (ou un processus de sa session active) lance
`pkexec /usr/bin/tuxedo-tomte <args>`. Une seule authentification administrateur donne ensuite,
pendant 5 minutes (`auth_admin_keep`, cache polkit), l'exécution root du binaire tiers avec des
arguments arbitraires. Un binaire `tuxedo-tomte` qui interprète mal ses arguments (sous-commandes,
noms de modules) devient une surface root générique — c'est la définition de S-12/F-06. Le fait que le
binaire soit root-owned ne change rien : la primitive est l'**exécution root avec argv contrôlé**,
pas l'écriture du binaire.

**Aggravant.** L'action est installée par le paquet (`packaging/README.md:16`) **avant** que
`tccd-admin` existe. Le risque est donc actif indépendamment de l'avancement du projet.

**Remédiation.** Supprimer l'action `tomte` et les deux annotations. Les opérations Tomte passent par
`tccd-admin` (D1) avec `CheckAuthorization` sur `com.tuxedocomputers.tccd1.tomte-run` et une allowlist
de sous-commandes/arguments (le binaire est invoqué par `Command::new("/usr/bin/tuxedo-tomte")`,
arguments séparés, `env_clear()`, timeout). Aucune annotation `exec.*`, jamais d'`allow_gui` pour une
action `auth_admin`. Ajouter un test CI qui parse tous les fichiers `.policy` et échoue sur
`exec.path`, `exec.argv1`, `allow_gui` (voir §4.5).

**Test.** `grep -r "exec.path\|allow_gui" /usr/share/polkit-1/actions/` ne doit rien retourner pour
nos actions ; `pkcheck --action-id com.tuxedocomputers.tccd1.tomte-run --process $$` doit échouer
sans dialogue quand l'utilisateur n'est pas administrateur.

---

#### R-02 — Politique D-Bus : toute méthode, tout utilisateur, sans restriction d'interface

| | |
|---|---|
| **Sévérité** | Élevé — **B** |
| **Fichier** | `packaging/dbus/com.tuxedocomputers.tccd.conf:30-39` (allow global : l. 34 et 37) |
| **Références** | S-01, S-03, F-01, D3, checklist §6.2 |

**Constat.** Le bloc `context="default"` contient :

```xml
<allow send_destination="com.tuxedocomputers.tccd"/>
<allow receive_sender="com.tuxedocomputers.tccd"/>
<allow send_destination="com.tuxedocomputers.tccd.Admin"/>
<allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
```

Ces règles n'ont **ni `send_interface` ni `send_member`**. Conséquences vérifiables :

1. tout utilisateur peut appeler `SetTempProfile`, `ClearTempProfile` et **toute méthode ajoutée plus
   tard** (la politique est *fail-open* à l'échelle du service) ;
2. `org.freedesktop.DBus.Properties.Set` est autorisé sur toute propriété présente ou future ;
3. aucune règle ne restreint `org.freedesktop.DBus.*` au strict nécessaire ;
4. l'interface `Manager1` (déclarée dans `tcc-ipc/src/lib.rs:62` mais non implémentée) et `Admin1`
   seraient joignables depuis `tccd` sans modification de la politique.

**Aggravant — règle morte.** La ligne 33 `<deny ... send_interface="org.freedesktop.DBus" send_member="UpdateActivationEnvironment"/>`
est placée **avant** l'allow global de la ligne 34. `dbus-daemon(1)` est explicite (vérifié localement) :
« The last rule that matches the message determines whether it may be sent. » La règle de refus est
donc annulée par l'allow qui suit. Toute future règle `deny` insérée au même endroit sera également
sans effet.

**Scénario.** `busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
com.tuxedocomputers.tccd SetTempProfile s __profile_performance__` depuis un compte non privilégié
change le profil matériel de la machine (R-03). Un client peut aussi marteler n'importe quelle
méthode future, y compris une méthode `Admin1` si elle était accidentellement servie par `tccd`.

**Remédiation.** Réécrire le bloc en *fail-closed* : `deny` global **en premier**, puis allow par
interface/membre. Proposition complète en §4.1. Tant que `SetTempProfile` reste sur l'interface de
compatibilité, énumérer les membres un par un : une méthode ajoutée sans mise à jour de la politique
n'est alors pas joignable (le test CI de §4.5 le détecte).

**Test.** En utilisateur non privilégié : `busctl introspect` OK ; `busctl call … GetStateJSON` OK ;
`busctl call … <méthode non listée>` → `AccessDenied` au niveau bus ; test CI qui parse le `.conf` et
échoue sur tout `allow send_destination` sans `send_interface`/`send_member`.

---

#### R-03 — `SetTempProfile` / `ClearTempProfile` : non authentifiés, globaux, non tracés

| | |
|---|---|
| **Sévérité** | Élevé — **B** |
| **Fichier** | `crates/tcc-daemon/src/dbus.rs:82-105` ; `crates/tcc-daemon/src/runtime.rs:182-210` |
| **Références** | S-01, S-09, A1/A5, R1 (risque résiduel documenté) |

**Constat.** Les deux méthodes sont exposées sans aucun contrôle :

```rust
#[zbus(name = "SetTempProfile")]
async fn set_temp_profile(&self, id: String) -> fdo::Result<bool> { … }
```

Le code est correct *techniquement* : l'identifiant est résolu dans `ProfileCollection`
(`runtime.rs:190-194`), un identifiant inconnu renvoie `InvalidArgs`, rien n'est persisté, aucune
donnée n'entre dans un chemin de fichier. Mais :

- **aucune authentification** ni vérification de l'émetteur (`header().sender()` n'est jamais lu ;
  `grep -rn "sender()\|credentials" crates` → aucun résultat) ;
- **portée globale** : l'override s'applique au daemon, donc à la machine, pas à la session de
  l'appelant. Sur un poste multi-utilisateur (ou avec un processus compromis dans une session), un
  utilisateur impose un profil (gouverneur CPU, fréquences, **luminosité**) aux autres ;
- **aucune limite de fréquence** : chaque appel prend le verrou en écriture et déclenche
  `notify_one()` → une boucle d'appels provoque une rafale d'écritures sysfs (gouverneur, luminosité)
  à chaque tick de 250 ms (`main.rs:27,168-179`) ;
- **aucune trace de l'appelant** dans les logs : `tracing::info!(profile = …, source = …)`
  (`main.rs:225-232`) ne journalise ni l'UID ni le nom unique D-Bus.

**Scénario.** (a) Un utilisateur non privilégié appelle `SetTempProfile("__profile_performance__")`
toutes les 100 ms : la machine reste bridée en performance, la luminosité est réécrite en boucle et
l'utilisateur légitime ne comprend pas pourquoi. (b) Un processus compromis dans une session
utilise le même canal pour dégrader la machine (ventilateurs/gouverneur) sans laisser de trace
nominative dans journald.

**Remédiation (deux options, à arbitrer avec `linux-platform`).**

- **Option A (recommandée, préserve la parité tray)** : garder la méthode ouverte mais
  (1) la déplacer sur `Manager1`, (2) la borner à un *token bucket* par nom unique (5 écritures/s,
  burst 10) avec `LimitsExceeded`, (3) journaliser `caller_uid` + `caller_unique_name` + `profile_id`
  (jamais le payload), (4) documenter explicitement le risque résiduel R1 (hardware partagé) dans le
  threat model, (5) au jalon 4, restreindre l'override à la session de l'appelant quand un agent de
  session est présent.
- **Option B (multi-utilisateur strict)** : `CheckAuthorization` avec le sujet `system-bus-name` de
  l'appelant sur `com.tuxedocomputers.tccd1.temp-profile` (`auth_admin_keep`).

**Test.** `tcc-daemon/tests/rate_limit.rs` : 10 000 appels → majorité rejetée, RSS stable (± 5 Mo),
un appel légitime suivant réussit. Test d'audit : un subscriber de test vérifie la présence de
`caller_uid` et l'absence du payload brut.

---

#### R-04 — Aucune application de polkit ; la documentation affirme le contraire

| | |
|---|---|
| **Sévérité** | Élevé — **B** (bloque l'entrée du jalon 2, pas encore exploitable) |
| **Fichier** | absence dans `crates/tcc-daemon/src/**` ; `packaging/polkit/com.tuxedocomputers.tccd.policy:10-18` (commentaire) |
| **Docs contradictoires** | `docs/linux/PLATFORM_DESIGN.md:691-692` (« les méthodes sensibles sont contrôlées par polkit côté daemon »), `:710-711` (« Le daemon vérifie l'UID appelant et … `CheckAuthorization` »), `docs/DBUS_API.md:150-157`, `packaging/dbus/com.tuxedocomputers.tccd.conf:9-11` |
| **Références** | S-12, D1, checklist §3.2 (4 points B) |

**Constat.** `grep -rn "PolicyKit\|CheckAuthorization\|polkit" --include="*.rs" crates` ne retourne
**aucun résultat**. Les quatre actions sont pourtant installées par le paquet et un commentaire du
fichier de politique affirme qu'elles « sont vérifiées par tccd ». Aucun test d'autorisation n'existe
(`crates/tcc-daemon/tests/` est absent : les tests D-Bus sont deux `#[tokio::test]` dans `dbus.rs:224-266`).

**Impact.** Le risque immédiat est nul (aucune méthode sensible n'est encore exposée). Le risque réel
est **organisationnel et de régression** : le jalon 2 ajoute `SetSettings`, `ApplyProfile`,
`SetTempProfileByName`, etc. ; rien dans le code, la CI ou la revue ne force l'appel à
`CheckAuthorization`. Un relecteur qui lit `PLATFORM_DESIGN.md` conclura à tort que c'est fait.

**Remédiation.**

1. Implémenter dès le jalon 2 un module `authz` unique : `CheckAuthorization(action,
   subject=system-bus-name(caller), details={}, flags=AllowUserInteraction)` avec cache par nom
   unique (TTL ≤ 60 s, invalidé sur `NameOwnerChanged`), et `AccessDenied` **sans** détail.
2. Ajouter dans `tcc-ipc` un tableau `SENSITIVE_METHODS` (interface, membre, action) et un test
   d'introspection qui **échoue** si une méthode `Set*`/`Apply*`/`Admin*` n'y figure pas
   (checklist §3.2, dernier point).
3. Corriger la documentation : marquer polkit comme « spécifié, non implémenté » tant que le code
   n'existe pas, et supprimer l'affirmation du commentaire du `.policy`.

**Test.** `tcc-daemon/tests/polkit.rs` avec une autorité simulée : sujet `system-bus-name`, refus
d'un `unix-process` fourni par le client, cache invalidé après `NameOwnerChanged`.

---

#### R-05 — Anti-DoS absent : messages 128 Mio, dispatch concurrent, pas de limite de ressources

| | |
|---|---|
| **Sévérité** | Moyen — **B** |
| **Fichier** | `crates/tcc-daemon/src/dbus.rs:82-92` (`id: String` non borné) ; `packaging/systemd/tccd.service:7-27` (aucune `MemoryMax`/`TasksMax`/`CPUQuota`) |
| **Références** | S-08, S-09, A5, checklist §3.3 (4 points B) |

**Constat.**

- zbus accepte les messages entrants jusqu'à `MAX_MESSAGE_SIZE = 128 * 1024 * 1024`
  (`zbus-5.13.2/src/message/header.rs:20`) ; le daemon ne fixe aucune limite et ne vérifie pas la
  longueur de `id` avant la résolution (la `String` est déjà allouée par la désérialisation).
- zbus dispatche chaque appel dans une tâche (`object_server/mod.rs:390-400`), donc les appels sont
  traités **en parallèle** : rien ne sérialise 10 appels de 128 Mio.
- aucun *token bucket*, aucun `Semaphore`, aucun compteur de clients (`grep "rate_limit\|Semaphore"` →
  aucun résultat).
- l'unit n'a ni `MemoryMax`, ni `TasksMax`, ni `CPUQuota` (contrairement à `THREAT_MODEL.md` §8.8).

**Scénario.** Un utilisateur local ouvre 8 connexions et envoie des `SetTempProfile` avec une chaîne
de ~128 Mio : ~1 Gio de RSS dans un processus root, sans limite cgroup. Sans `MemoryMax`, l'OOM
killer peut tuer des voisins (session graphique) ou `tccd` lui-même → perte du contrôle thermique et
de la charge (A5). Une simple boucle d'appels courts sature aussi les écritures sysfs (R-03).

**Remédiation.** (1) Ajouter `MemoryMax=256M`, `TasksMax=64`, `CPUQuota=25%` à `tccd.service`.
(2) Token bucket par nom unique (lecture 50/s burst 100, écriture 5/s burst 10, admin 1/10 s) →
`LimitsExceeded`. (3) `Semaphore(32)` sur les handlers et rejet immédiat si saturé.
(4) Rejeter `id` au-delà de `MAX_PROFILE_ID_LEN` (64) dès l'entrée du handler, et documenter que la
borne mémoire réelle vient du cgroup, pas du parseur. (5) Tester 10 000 appels + un payload de
10 Mio avec mesure de RSS.

**Test.** `tcc-daemon/tests/dbus_limits.rs`.

---

#### R-07 — `tccd-admin.service` : binaire absent, capacités larges, aucun filtre de périphériques

| | |
|---|---|
| **Sévérité** | Moyen — **B** (packaging) / Élevé si publié tel quel avec le binaire |
| **Fichier** | `packaging/systemd/tccd-admin.service:8-11` (Type/BusName/ExecStart), `:39-43` (capacités), `:16-18` (`StateDirectory`) ; `packaging/dbus/com.tuxedocomputers.tccd.Admin.service:5` |
| **Références** | D1, checklist §6.1/§6.5 |

**Constat.**

- `ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin` : **le binaire n'existe pas** (aucun crate
  `tccd-admin`). Le fichier d'activation D-Bus est néanmoins installé (`packaging/README.md:15`) ;
  toute activation échoue et laisse l'unit en `failed`.
- `CapabilityBoundingSet=CAP_SYS_ADMIN CAP_DAC_OVERRIDE CAP_CHOWN` : jeu très large pour un service
  root joignable par activation D-Bus depuis n'importe quel compte local. `CAP_DAC_OVERRIDE` en
  particulier annule les protections de permissions sur *tous* les fichiers.
- Aucun `DevicePolicy=`, aucun `DeviceAllow=` : la valeur par défaut est `auto` (tous les
  périphériques accessibles, création de nœuds comprise).
- `StateDirectory=tuxedo-rust-center` contredit D1 (« sans état persistant »).
- `ProtectKernelTunables=yes` est correct ici, mais `ProtectKernelLogs`, `KeyringMode`, `RemoveIPC`,
  `SystemCallFilter`, `MemoryMax` manquent.

**Scénario.** Si le paquet est publié avant l'implémentation (le README dit explicitement que l'unité
est fournie), un utilisateur local peut provoquer des activations répétées (bruit journald, unit en
échec). Le jour où le binaire arrive avec ces capacités, une erreur de parsing dans un service
D-Bus root donne `CAP_SYS_ADMIN` + `CAP_DAC_OVERRIDE` — exactement ce que D1 cherche à éviter.

**Remédiation.** (1) Ne pas installer l'unit ni le fichier d'activation tant que `tccd-admin`
n'existe pas, **ou** livrer un binaire qui refuse toute méthode avec `NotSupported` (fail-closed).
(2) Au jalon 5: `CapabilityBoundingSet=` réduit au strict nécessaire après mesure (`strace -f -e
trace=capset,ioctl`), justification écrite pour chaque capacité conservée. (3) `DevicePolicy=closed`
+ `DeviceAllow=/dev/tuxedo_io rw`. (4) Supprimer `StateDirectory`.
(5) Ajouter `MemoryMax=`, `KeyringMode=private`, `RemoveIPC=yes`, `ProtectKernelLogs=yes`.

**Test.** `systemd-analyze verify` + `systemd-analyze security --offline` archivés ; test
d'activation D-Bus avec un utilisateur non privilégié → échec explicite, pas de boucle infinie.

---

#### R-09 — Polkit : défauts trop permissifs et actions trop grossières

| | |
|---|---|
| **Sévérité** | Moyen — **B** pour l'action sensible (historique ; LUKS hors périmètre) |
| **Fichier** | `packaging/polkit/com.tuxedocomputers.tccd.policy:20-60` |
| **Références** | D5, `THREAT_MODEL.md` §8.2 règles 2-4, checklist §6.3 |

**Constat.**

| Action | Défauts actuels | Attendu (D5/§8.2) |
|---|---|---|
| `configure` (l. 20-32) | `auth_admin_keep` partout, couvre settings **et** assignation persistante | `allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin_keep` |
| `hardware` (l. 34-46) | `auth_admin_keep` partout, couvre TDP, charge, YCbCr **et PRIME** | une action par opération ; PRIME doit vivre dans `tccd-admin` (D1) |
| `luks` (l. 48-60) | **Historique, hors périmètre** : action non conservée | — |
| `tomte` (l. 62-76) | voir R-01 | à supprimer |

Conséquences : une seule authentification pour « configurer les profils » ouvre 5 minutes de toutes
les opérations sensibles, y compris PRIME (LUKS hors périmètre) ; une session inactive (SSH sans siège)
peut déclencher un dialogue d'authentification administrateur ; l'utilisateur ne sait pas ce qu'il
autorise (« TCC » au lieu de « changer le mode graphique »).

**Remédiation.** Une action par opération, IDs versionnés `com.tuxedocomputers.tccd1.<verbe>`,
`allow_any=no`, `allow_inactive=no`, `auth_admin` pour PRIME/Tomte/WebFAI/paquets,
`auth_admin_keep` **uniquement** pour l'assignation de profil. Traductions FR/EN/DE. Proposition
complète en §4.2.

**Test.** `pkcheck` par action ; test CI de parsing : toute action dont `<allow_any>` n'est pas `no`
échoue, toute action avec `_keep` hors assignation échoue.

---

### 2.2 Importants

---

#### R-06 — `tccd-sleep.service` : `systemctl` dans une unit, score 9.2 UNSAFE

| | |
|---|---|
| **Sévérité** | Moyen — **I** |
| **Fichier** | `packaging/systemd/tccd-sleep.service:14-15` |
| **Preuve** | `systemd-analyze security --offline=true` → **9.2 UNSAFE** (aucun durcissement) |

**Constat.** `ExecStart=/usr/bin/systemctl stop tccd.service` et `ExecStop=/usr/bin/systemctl start
tccd.service`. Invoquer `systemctl` depuis une unité est un anti-pattern : la commande attend la fin
d'un job qui appartient à la même transaction que l'unité courante (`Before=sleep.target`), ce qui
peut bloquer ou retarder la mise en veille. `ExecStop` redémarre `tccd` même si l'arrêt a été demandé
par un administrateur (`systemctl stop tccd-sleep` ne devrait pas relancer `tccd`).

**Scénario.** Un `systemctl suspend` reste bloqué plusieurs secondes (voire échoue) ; un
`systemctl stop tccd` manuel est annulé par l'arrêt de `tccd-sleep`. Impact : indisponibilité du
contrôle thermique (A5) et UX dégradée.

**Remédiation.** Supprimer l'unité. `tccd` s'abonne à `org.freedesktop.login1` (`PrepareForSleep`) et
réapplique le profil au réveil — il a déjà le runtime tokio et zbus ; c'est aussi ce que prévoit
`PLATFORM_DESIGN.md:11-13`. À défaut, utiliser un hook `/usr/lib/systemd/system-sleep/` (binaire Rust
dédié, pas de shell) ou `systemctl --no-block`. Si l'unité est conservée : ajouter `UMask=0077`,
`ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `CapabilityBoundingSet=`,
`RestrictAddressFamilies=AF_UNIX`.

---

#### R-08 — Règle `deny UpdateActivationEnvironment` sans effet (ordre des règles)

| | |
|---|---|
| **Sévérité** | Faible — **I** (correctness/sécurité documentaire) |
| **Fichier** | `packaging/dbus/com.tuxedocomputers.tccd.conf:33-34` |

Voir R-02 §« règle morte » : la règle de refus précède un allow global qui la neutralise
(`dbus-daemon(1)` : dernière règle gagnante). Aujourd'hui sans impact (le daemon n'implémente pas
`org.freedesktop.DBus`), mais le fichier donne une fausse impression de protection et le motif sera
recopié. **Remédiation** : réordonner (refus par défaut en tête, allows spécifiques ensuite) et
ajouter le test de politique.

---

#### R-10 — `tccd-user` : une connexion D-Bus par cycle de polling, aucune souscription aux signaux

| | |
|---|---|
| **Sévérité** | Moyen — **I** |
| **Fichier** | `crates/tcc-user/src/agent.rs:32-40` (constantes), `:237-273` (`fetch_payloads`) |

**Constat.** `poll_once` appelle `source.fetch()` toutes les 2 s ; `fetch_payloads` crée une
**nouvelle** `zbus::Connection` à chaque cycle, puis deux appels (`GetActiveProfileJSON`,
`GetProfilesJSON`) avec 3 s de timeout chacun. Aucun abonnement à `StateChanged` (le signal existe
pourtant, `dbus.rs:113-121`) ni à `PropertiesChanged` : le contrat « event-driven par défaut » de
`ARCHITECTURE.md` §3.11 n'est pas respecté, et 2 appels + 1 connexion toutes les 2 s représentent
~43 000 authentifications/jour sur le bus système pour un agent inactif.

**Scénario.** Un daemon lent (ou un attaquant qui provoque R-05) fait attendre 6 s par cycle ; l'agent
ne réagit à aucun changement réel avant le cycle suivant ; le bruit D-Bus rend l'audit plus difficile.

**Remédiation.** Une seule connexion longue durée, souscription à `StateChanged`, polling de secours
(30 s) uniquement si le signal n'est pas reçu depuis N secondes. Conserver la validation actuelle
(elle est bonne) et la borne `MAX_REMOTE_JSON_BYTES = 4 Mio` (`agent.rs:32`).

---

#### R-11 — Backend KDE : `kscreen-doctor` sans timeout, sans `env_clear`, et application persistante

| | |
|---|---|
| **Sévérité** | Moyen — **I** |
| **Fichier** | `crates/tcc-user/src/backend/kde.rs:28-30` (constantes), `:145-163` (`read_state`), `:229-233` (application) |
| **Références** | `THREAT_MODEL.md` §8.7, §8.10.5, checklist §4 |

**Constat.** Le chemin est absolu (`/usr/bin/kscreen-doctor`, l. 28) — conforme. En revanche :
`Command::new(self.binary).arg("-j").output()` (l. 146-149) et `.arg(&argument).output()` (l. 230-233)
n'ont **ni timeout ni `env_clear()`** ; le processus est lancé dans `spawn_blocking`
(`agent.rs:450-457`), donc un `kscreen-doctor` bloqué immobilise un thread de blocage indéfiniment
(pas de reprise). Par ailleurs `output.<nom>.mode.<id>` (l. 229) **persiste** le mode dans la
configuration KScreen de l'utilisateur, ce qui contredit §8.10.5 (« pas d'écriture persistante ») et
peut entrer en conflit avec le compositeur.

L'argument est bien construit à partir de composants validés (nom d'output filtré, id de mode
`[A-Za-z0-9_-]+`, l. 217-227) : pas d'injection possible.

**Remédiation.** `env_clear()` + environnement fixe (`PATH=/usr/bin:/bin`, `LANG=C`), timeout 3 s +
`kill`, sortie déjà bornée (`MAX_OUTPUT_BYTES`, l. 29) ; privilégier l'API D-Bus KScreen (sans
sauvegarde) et documenter le fallback CLI comme dégradé.

---

#### R-12 — `DisplayModeRequest.internal_output` : connecteur fourni par le client

| | |
|---|---|
| **Sévérité** | Faible — **I** (latent, aucune API exposée aujourd'hui) |
| **Fichier** | `crates/tcc-ipc/src/display.rs:289-298` (champ l. 296-297) ; utilisé par `session.rs:242-246` (tests) |

**Constat.** Le contrat prévu pour l'agent de session transporte `internal_output: Option<String>`.
`THREAT_MODEL.md` §8.10.3 et S-14 disent l'inverse : « l'output cible est **découvert par l'agent** …
jamais un output dont le nom vient du client ». La validation actuelle (`is_internal_output_name`)
limite la casse, mais elle n'est pas la barrière prévue (R-13).

**Scénario (futur).** Un client du bus système envoie `internal_output = "eDP-1"` alors que l'écran
interne est `eDP-2` après un changement de GPU/PRIME : l'agent applique sur un connecteur arbitraire
(pas de vérification d'existence), ou échoue sans raison. Aujourd'hui inexploitable (aucun service
D-Bus dans `tccd-user`, confirmé par `grep -rn "serve_at" crates/tcc-user` → aucun résultat).

**Remédiation.** Retirer le champ, ou l'ignorer explicitement côté agent (découverte systématique) et
le documenter comme champ informatif en lecture seule. Mettre à jour `docs/DBUS_API.md` en
conséquence.

---

#### R-13 — Filtre « écran interne » : préfixe au lieu de la regex documentée, pas de recoupement DRM

| | |
|---|---|
| **Sévérité** | Faible — **I** |
| **Fichier** | `crates/tcc-hw/src/display.rs:25-36` ; `crates/tcc-user/src/backend/mod.rs:86-99` |

**Constat.** `is_internal_connector` fait `name.trim().to_ascii_lowercase().starts_with(prefix)` sur
`["edp","lvds","dsi"]`. Acceptent donc `eDP-evil`, `edp-`, `DSI0`, `eDP-1x`. Le filtre documenté
(`^(eDP|LVDS|DSI)-[0-9]+$`) et le recoupement `/sys/class/drm/card*-eDP-*` (checklist §4) ne sont pas
implémentés. `validate_output_name` (`backend/mod.rs:86-95`) restreint le jeu de caractères et bloque
donc l'injection dans une commande compositeur — c'est la seule raison pour laquelle le risque reste
faible.

**Remédiation.** Implémenter la regex (ou un parseur équivalent sans dépendance : préfixe exact +
suffixe `-<chiffres>`) et vérifier l'existence du connecteur dans `/sys/class/drm/`. Tests :
`HDMI-1`, `DP-1`, `eDP-evil`, `eDP-1/../x`, vide, NUL → rejet ; `eDP-1`, `LVDS-1`, `DSI-1` → accepté.

---

#### R-21 — `tccd-user.service` : dépendance aux user namespaces, `UMask`/`LimitCORE` absents

| | |
|---|---|
| **Sévérité** | Faible — **I** |
| **Fichier** | `packaging/systemd/tccd-user.service:14-37` |

**Constat.** `ProtectSystem=strict` dans une unité **utilisateur** repose sur les namespaces de montage
(créés via user namespace non privilégié). Sur une distribution avec
`kernel.unprivileged_userns_clone=0` (ou `user.max_user_namespaces=0`), l'unité échoue au démarrage →
agent absent → refresh rate « non supporté » sans explication. Manquent aussi `UMask=0077` et
`LimitCORE=0` (checklist §6.1), ainsi que `ProtectKernelLogs=yes`.

**Remédiation.** Documenter la dépendance, prévoir un drop-in de repli (`ProtectSystem=full` ou
désactivation) et exposer l'état dans l'UI ; ajouter `UMask=0077`, `LimitCORE=0`,
`ProtectKernelLogs=yes`, `MemoryMax=128M`.

---

#### R-22 — `DeviceAllow=/dev/tuxedo_io rw` : dépend de l'ordre de chargement du module

| | |
|---|---|
| **Sévérité** | Faible — **I** (disponibilité) |
| **Fichier** | `packaging/systemd/tccd.service:46-51` |
| **Hypothèse non vérifiée** | comportement exact de systemd quand le nœud n'existe pas encore au démarrage |

**Constat.** `systemd-analyze security` affiche la règle résolue (`/dev/tuxedo_io:rw`) et
`/dev/tuxedo_io` existe sur la machine de revue. `systemd.resource-control(5)` précise que les listes
d'autorisation « should only reference device groups which are resolvable at the time the unit is
started. Any device groups not resolvable then are not added to the device allow list. » Pour un
**chemin** de nœud, la résolution se fait aussi au démarrage : si le module `tuxedo_io` n'est pas
chargé, la règle peut être perdue (le nœud restant alors refusé par `DevicePolicy=closed`) ou, selon
la version de systemd, faire échouer l'unité. Aucun `Wants=modprobe@tuxedo_io.service` /
`After=` n'est présent.

**Impact.** Perte de fonctionnalité (`tuxedo_io` indisponible jusqu'au redémarrage) au boot, pas
d'escalade. Sur les modèles où le module est chargé à la demande par udev, le risque est réel.

**Remédiation.** Ajouter `Wants=modprobe@tuxedo_io.service` + `After=modprobe@tuxedo_io.service`, ou
ouvrir le device paresseusement (déjà le cas côté code : `PLATFORM_DESIGN.md:854`) et documenter.
Tester sur matériel sans module préchargé.

---

### 2.3 Améliorations

| ID | Sév. | Fichier:ligne | Constat | Remédiation |
|---|---|---|---|---|
| **R-14** | Faible / A | `crates/tcc-gui/src/daemon_client.rs:318-331` ; `crates/tcc-cli/src/main.rs:13-24` ; `crates/tcc-user/src/agent.rs:114-133` ; `crates/tcc-gui/src/main.rs:65-71` | `TCC_DBUS_SERVICE`, `TCC_DBUS_BUS` et `TCC_QML_DIR` sont lus **dans les binaires release**, sans feature `dev-overrides` (contrairement à D7). Impact limité (clients non privilégiés), mais `TCC_QML_DIR` fait charger du QML depuis un chemin arbitraire et `TCC_DBUS_SERVICE` permet de pointer vers un faux service. | Gater derrière `dev-overrides` (comme `tcc-hw`/`tcc-core`), ou au minimum valider le nom D-Bus et documenter. |
| **R-15** | Faible / A | `crates/tcc-gui/src/daemon_client.rs:109-149` | Un `std::thread::spawn` + un runtime tokio **par appel QML** (`refresh`, `set_temp_profile`, `clear_temp_profile`), sans coalescence ni jointure ; une boucle QML crée des threads sans borne. | Un worker unique + `mpsc` borné + indicateur « en vol ». |
| **R-16** | Faible / A | `crates/tcc-gui/src/daemon_client.rs:266-288` | Aucune borne de taille avant `serde_json::from_str` (contrairement à `tcc-user`, `agent.rs:32,227-235` qui plafonne à 4 Mio). Un service hostile peut faire allouer jusqu'à 128 Mio. | Vérifier `state_json.len()`/`profiles_json.len()` avant parsing ; pousser vers QML le JSON **re-sérialisé** depuis le modèle validé plutôt que la chaîne brute. |
| **R-17** | Faible / B (garde-fou) | `crates/tcc-daemon/src/main.rs`, `crates/tcc-cli/src/main.rs`, absence de `clippy.toml` | Les deux binaires n'ont ni `#![forbid(unsafe_code)]` ni `deny(clippy::unwrap_used)`; `clippy.toml` avec `disallowed-methods` (`Command::new`, `fs::write`, `File::create`) est absent alors que la checklist §7 le classe **B**. Aujourd'hui le code est propre (0 `unwrap`/`expect` hors tests, vérifié), mais rien ne l'impose. | Ajouter les attributs, `clippy.toml`, `[workspace.lints]`, et un job CI `grep` de contrôle. |
| **R-18** | Faible / A | `crates/tcc-core/src/migration.rs` (aucun `.bak`) ; `packaging/README.md:73-75` ; `ARCHITECTURE.md` §3.8 | La migration laisse `/etc/tcc` intact (bien) mais **ne crée pas** la copie `.bak` root:root 0600 annoncée par la doc. Écart documentaire, pas de faille (la source est root-owned et non modifiée). | Soit implémenter la sauvegarde, soit corriger la doc. |
| **R-19** | Faible / A | `crates/tcc-daemon/src/dbus.rs:66-79` ; `crates/tcc-core/src/{profile.rs:465,settings.rs:229}` | `GetProfilesJSON`/`GetSettingsJSON` exposent **toute** la configuration à tout utilisateur local, y compris les champs inconnus (`extra`) conservés tels quels. Aucun secret aujourd'hui, mais le passthrough `extra` ré-émettra automatiquement tout champ futur. | Définir une vue exposée (allowlist de champs) ou un test qui vérifie qu'un champ `settings.extra.<sensible>` n'est pas émis. |
| **R-20** | Faible / A | `crates/tcc-daemon/src/dbus.rs:140-164` ; `packaging/systemd/tccd.service:8-13` |`Type=dbus` + aucun traitement de perte de nom : si le bus redémarre, la connexion zbus tombe et le daemon continue sans nom (le contrôle matériel continue, mais GUI/agent voient un service absent) ; `Restart=on-failure` sans `StartLimit*` explicite. | Surveiller la connexion et sortir (redémarrage systemd) ; ajouter `StartLimitIntervalSec=60`/`StartLimitBurst=5`. Comportement exact de `Type=dbus` à vérifier par test. |
| **R-23** | Faible / I | `.github/workflows/ci.yml:21,31,48,101-116` | Actions épinglées par tag (`@v4`, `@v2`), pas par SHA ; `cargo test`/`clippy` sans `--locked` ; aucun `systemd-analyze verify`, aucun test de politique D-Bus/polkit, aucun `gitleaks`. `cargo deny`/`audit` existent (bon) mais n'ont pas été exécutés dans cet environnement. | Épingler les actions par SHA, `--locked` partout, job `packaging-lint` (voir §4.5). |

### 2.4 Points de documentation à corriger (pas des vulnérabilités, mais des affirmations fausses)

| Doc | Ligne | Affirmation | Réalité |
|---|---|---|---|
| `docs/DBUS_API.md` | 150-157 | « Sécurité : la politique D-Bus … » ; n'indique pas que `SetTempProfile` est global et non authentifié | À compléter (R-02/R-03) |
| `docs/linux/PLATFORM_DESIGN.md` | 691-692, 710-711 | « méthodes sensibles contrôlées par polkit côté daemon », « le daemon vérifie l'UID appelant et … `CheckAuthorization` » | Aucun code à la date de la revue (R-04, depuis partiel) |
| `packaging/polkit/…policy` | 10-18 | « Les actions ci-dessous sont vérifiées par tccd » | Aucun code à la date de la revue (R-04, depuis partiel) |
| `packaging/README.md` | 73-75 | sauvegarde `.bak` de l'ancien `/etc/tcc` | non implémentée (R-18) |
| `packaging/systemd/tccd.service` | 24-27 | « les fichiers publics … 0644 » avec `UMask=0077` | exact (fchmod explicite vérifié) — RAS |
| `docs/linux/PLATFORM_DESIGN.md` | 705-708 | PRIME dans `hardware`, Tomte par action `pkexec` | contredit D1/D5 (R-01/R-09) |

---

## 3. Ce qui est déjà conforme (avec preuves)

Ces points sont **vérifiés dans le code**, pas supposés. Ils constituent une base solide à préserver.

### 3.1 `tcc-core` — persistance et validation

| Exigence | Preuve |
|---|---|
| Lecture sans suivi de lien symbolique | `config.rs:910-918` : `O_NOFOLLOW \| O_CLOEXEC` |
| Vérification après ouverture (fd, pas chemin) | `config.rs:515-521` (`file.metadata()` + `verify_file`) |
| Type/propriétaire/permissions | `config.rs:694-702`, `729-761` : `S_ISREG`, `uid == 0` en production (`LoadPolicy::for_root_daemon`, l. 164-173), refus de `mode & 0o022` |
| Écriture atomique | `config.rs:925-945` : `create_new` + `mode(0o600)`, `fchmod(0o644)`, `sync_all`, `rename`, `fsync` du répertoire |
| Refus d'écrire sous `/etc` | `config.rs:838-863` (`validate_state_root`), `607-656` (double vérification avant/après `canonicalize`), `488-495` (pas de `.bak` sous `/etc`) |
| Récupération sur fichier corrompu | `config.rs:658-692` : `.bak` 0600, valeurs par défaut, avertissement typé |
| Bornes de taille | `DEFAULT_MAX_CONFIG_BYTES = 1 Mio` (`config.rs:80`), lecture `take(limit+1)` |
| Validation avant sauvegarde | `save_settings`/`save_profiles`/`save_fan_tables` appellent `validate()` (`config.rs:404-452`) |
| Migration validée | `migration.rs:212,268,336` ; test `invalid_profile_in_legacy_file_is_rejected` (l. 561) |
| Overrides dev refusés en root | `config.rs:991-1040` ; `tcc-hw/src/sysfs.rs:80-113` ; `tcc-daemon/src/config.rs:37-51` |
| Aucun `unwrap`/`expect` en production | `#![cfg_attr(not(test), deny(...))]` (`lib.rs:37`) **et** vérification par script : 0 occurrence hors `#[cfg(test)]` |

### 3.2 `tcc-hw` — sysfs et ioctl

| Exigence | Preuve |
|---|---|
| Chemins confinés | `sysfs.rs:122-142` : refus des chemins absolus, `..`, `RootDir`, `Prefix` |
| Écriture `O_NOFOLLOW` + `fstat` | `sysfs.rs:336-368` (`open_for_write`), erreur `ELOOP` traduite en `UnsafeTarget` |
| Propriétaire/permissions en production | `sysfs.rs:365-368` + `strict_ownership` actif seulement pour `/sys` (`SysfsRoot::system`, l. 62-69) |
| Valeur unique, sans caractère de contrôle | `sysfs.rs:243-249` (un seul `write_all`), `310-334` (NUL, contrôle, 4 Kio max) |
| `unsafe` isolé et commenté | `tuxedo_io.rs:191-295` : 4 appels `libc::ioctl`, chacun avec un bloc `// SAFETY:` ; constantes issues du header |
| Device ouvert sans suivi de lien | `tuxedo_io.rs:218-231` : `O_NOFOLLOW \| O_CLOEXEC` |
| Index/valeurs bornés | `tuxedo_io.rs` (modèle `fake` et constantes `MAX_*`), `cpu.rs:293-502` (validation contre les listes sysfs) |
| Capacité absente ⇒ `NotSupported` | `cpu.rs:337-492` (`HwError::NotSupported`), `apply.rs:155-174` |

### 3.3 `tcc-ipc`

`#![forbid(unsafe_code)]` (`lib.rs:38`) ; bornes explicites (`MAX_REFRESH_RATE_HZ = 1000`,
`MAX_RESOLUTION = 16 384`, `MAX_PROFILES = 256`, `MAX_PROFILE_ID_LEN = 64`,
`MAX_DESCRIPTION_LEN = 4096`, `validate.rs:31-73`) ; validation de version stricte
(`ensure_contract_compatible`, `lib.rs:83-92`) ; aucun `serde_json::Value` dans un contrat exposé
(`lib.rs:34-35`) ; `StateSnapshot::validate` vérifie la cohérence `manualOverride`.

### 3.4 `tcc-user`

| Exigence | Preuve |
|---|---|
| Refus root | `security.rs:38-50` + double contrôle `agent.rs:298-300` + `cli.rs:139` |
| Socket compositeur : absolu, sans `..`, socket UNIX, uid, pas de symlink, parent non inscriptible groupe/monde | `security.rs:103-177` |
| `$XDG_RUNTIME_DIR` : répertoire réel, uid, mode 0700 | `security.rs:63-100` |
| Timeouts IPC | `security.rs:24,206-207` ; `agent.rs:34,244-268` |
| Réponses bornées | `security.rs:21` (8 Mio), `ipc.rs:61-118`, `agent.rs:32,227-235` |
| Aucune surface D-Bus entrante | aucun `serve_at`/`name()` dans `tcc-user` (vérifié) |
| Payload distant non fiable | `agent.rs:151-196` : parse, `ActiveProfile::validate`, `ProfileCollection::validate`, borne `1.0..=1000.0`, `is_finite` |
| Application uniquement sur un mode annoncé | `tcc-hw/src/display.rs:258-287` (`mode_for_rate`), idempotence (`needs_rate_change`), tolérance 0,5 Hz |
| Aucune commande arbitraire | un seul `Command::new` (chemin absolu, `kde.rs:28`), arguments construits à partir de composants validés |
| Aucun `/proc/*/environ` | `security.rs:9-10` (seul `/proc/self/status`) |

### 3.5 `tcc-daemon` / `tcc-gui` / `tcc-cli`

- Nom D-Bus : `DO_NOT_QUEUE` garanti par zbus (`builder.rs:62` de zbus 5.13.2) ; `AllowReplacement`
  et `ReplaceExisting` désactivés par défaut → pas de mise en file ni de remplacement par un tiers.
- Aucune commande shell, aucun `Command::new` dans `tccd`, `tcc-gui`, `tcc-cli`.
- Aucune écriture de fichier arbitraire : `Get*JSON` sont en lecture ; `SetTempProfile` ne persiste
  rien et ne peut cibler qu'un profil existant (`runtime.rs:190-194`).
- `--check` non root n'écrit rien (migration conditionnée à `is_root()`, `config.rs:69`,
  `persist_defaults_if_needed` l. 161-164).
- `validate_bus_name` strict (`main.rs:326-339`) : alphanumériques + `._-`, longueur ≤ 255, pas de
  `..`.
- GUI : timeouts 3 s sur chaque appel (`daemon_client.rs:18,269-276`), validation
  `StateSnapshot::validate` + `ProfileCollection::validate` (l. 278-288), échec gracieux si le thread
  D-Bus ne démarre pas (l. 102-106), `connected=false` + message d'erreur plutôt qu'un panic.
- QML : **tous** les textes en `Text.PlainText` (`AppLabel.qml:43`, `AppSelect.qml:34,86`,
  `AppToolTip.qml:29`, `Gauge.qml:130,165`, `FanCurveChart.qml:96,128`) ; aucun `JSON.parse`, aucun
  `eval`, aucun `Qt.createQmlObject` (vérifié) → pas d'injection HTML/QML via un nom de profil importé.
- `tcc-hw`/`tcc-core`/`tcc-ipc` : `#![forbid(unsafe_code)]` ou `#![deny(unsafe_code)]` avec une
  exception unique et documentée.

### 3.6 Intégration système (ce qui est bon)

- `tccd.service:37-44,56-71` : `NoNewPrivileges`, `ProtectSystem=strict` **sans** `ReadWritePaths=/etc`
  (D2 respecté), `ProtectHome`, `PrivateTmp`, `ProtectProc=invisible`, `ProtectKernelModules`,
  `ProtectControlGroups`, `ProtectClock`, `ProtectHostname`, `RestrictNamespaces`, `RestrictRealtime`,
  `RestrictSUIDSGID`, `LockPersonality`, `MemoryDenyWriteExecute`, `RestrictAddressFamilies=AF_UNIX
  AF_NETLINK`, `SecureBits=noroot-locked`, `LimitCORE=0`, `CapabilityBoundingSet=` vide,
  `UMask=0077`. `ProtectKernelTunables=no` est **la bonne décision** et elle est commentée (l. 53-56).
  Score `systemd-analyze security` : **3.5 OK**.
- `tccd.service:18-23` : `StateDirectory`/`RuntimeDirectory`/`ConfigurationDirectory` cohérents avec
  la doc ; `tmpfiles` (`packaging/tmpfiles/…conf:12-14`) : trois répertoires root:root, aucun
  world-writable.
- **Aucun `/opt`** dans `packaging/` (seule occurrence : la phrase du README qui l'interdit).
- **Aucun setuid/setgid/capability de fichier** déclaré.
- `udev` : `TAG+=` + `ENV{}` uniquement, **aucun `RUN+=`**, aucun interpréteur
  (`99-tuxedo-rust-center-webcam.rules:11`) — S-19/F-11 corrigé par construction.
- `tccd-admin.service:45` : pas de `[Install]` (activation à la demande) — conforme D1, sous réserve
  de R-07.
- `tcc-user.service` : unité **utilisateur** (`packaging/README.md:12` → `/usr/lib/systemd/user/`),
  `PartOf=graphical-session.target`, sans capacité, `RestrictAddressFamilies=AF_UNIX` — conforme.
- `deny.toml` : `yanked = "deny"`, `wildcards = "deny"`, `unknown-git = "deny"`, registre crates.io
  uniquement, liste de licences compatible GPL-3.0-or-later ; **aucune dépendance git** dans
  `Cargo.lock` ; 146 paquets ; **un seul `build.rs`** (`tcc-gui/build.rs`, cxx-qt) à revoir.

---

## 4. Propositions de remédiation

### 4.1 `packaging/dbus/com.tuxedocomputers.tccd.conf` (v2)

Principe : refus par défaut **d'abord**, allow par interface/membre **ensuite** (dernière règle
gagnante). Ne jamais réintroduire un `allow send_destination` sans `send_interface`.

```xml
<busconfig>
  <type>system</type>

  <policy user="root">
    <allow own="com.tuxedocomputers.tccd"/>
    <allow own="com.tuxedocomputers.tccd.Admin"/>
    <allow send_destination="com.tuxedocomputers.tccd"/>
    <allow send_destination="com.tuxedocomputers.tccd.Admin"/>
    <allow receive_sender="com.tuxedocomputers.tccd"/>
    <allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
  </policy>

  <policy context="default">
    <deny own="com.tuxedocomputers.tccd"/>
    <deny own="com.tuxedocomputers.tccd.Admin"/>

    <!-- 1. Refus explicite : tout ce qui n'est pas listé ci-dessous est refusé. -->
    <deny send_destination="com.tuxedocomputers.tccd"/>
    <deny send_destination="com.tuxedocomputers.tccd.Admin"/>

    <!-- 2. Introspection / propriétés en LECTURE uniquement. -->
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Introspectable"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Peer"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Properties" send_member="Get"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Properties" send_member="GetAll"/>

    <!-- 3. Couche JSON de compatibilité (D3) : lecture seule, membre par membre.
            Toute nouvelle méthode non listée ici est injoignable : fail-closed. -->
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="Ping"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="GetVersion"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="GetStateJSON"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="GetActiveProfileJSON"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="GetProfilesJSON"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd" send_member="GetSettingsJSON"/>

    <!-- 4. API typée lecture + profil temporaire (jalon 2). -->
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd.Manager1"/>

    <!-- 5. Signaux : les clients reçoivent StateChanged. -->
    <allow receive_sender="com.tuxedocomputers.tccd"/>

    <!-- 6. Admin : joignable, mais polkit tranche dans tccd-admin ; aucune autre interface. -->
    <allow send_destination="com.tuxedocomputers.tccd.Admin"
           send_interface="com.tuxedocomputers.tccd.Admin1"/>
    <allow send_destination="com.tuxedocomputers.tccd.Admin"
           send_interface="org.freedesktop.DBus.Introspectable"/>
    <allow send_destination="com.tuxedocomputers.tccd.Admin"
           send_interface="org.freedesktop.DBus.Peer"/>
    <allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
  </policy>
</busconfig>
```

Points d'attention :

- si `SetTempProfile` reste sur l'interface de compatibilité, ajouter **deux** allows de membre
  (`SetTempProfile`, `ClearTempProfile`) et accepter que la politique ne distingue pas lecture et
  écriture. **Recommandation** : les déplacer sur `Manager1` (R-03) et garder la compatibilité
  strictement en lecture.
- `Properties.Set` n'est volontairement autorisé nulle part : une propriété inscriptible ajoutée par
  erreur sera refusée par le bus.
- tester le fichier avec `dbus-daemon --config-file=… --print-address --nofork` sur un bus de test,
  puis `busctl` en utilisateur non privilégié.

### 4.2 `packaging/polkit/com.tuxedocomputers.tccd.policy` (v2)

```xml
<policyconfig>
  <vendor>TUXEDO Computers GmbH</vendor>
  <vendor_url>https://www.tuxedocomputers.com/</vendor_url>

  <!-- Assignation de profil : fréquente, non destructive → _keep assumé. -->
  <action id="com.tuxedocomputers.tccd1.assign-profile">
    <description>Assign a profile to a power state</description>
    <description xml:lang="fr">Assigner un profil à un état d'alimentation</description>
    <description xml:lang="de">Ein Profil einem Energiezustand zuordnen</description>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_admin_keep</allow_active>
    </defaults>
  </action>

  <!-- Limites matérielles : une action par famille, sans cache. -->
  <action id="com.tuxedocomputers.tccd1.power-limits">   <!-- TDP, cTGP -->
    ...
    <allow_active>auth_admin</allow_active>
  </action>
  <action id="com.tuxedocomputers.tccd1.charging">        <!-- seuils, profil, priorité -->
    ...
    <allow_active>auth_admin</allow_active>
  </action>
  <action id="com.tuxedocomputers.tccd1.prime-select">
    ...
    <allow_active>auth_admin</allow_active>
  </action>
  <!-- LUKS hors périmètre (décision utilisateur) : aucune action luks-change-key. -->
  <action id="com.tuxedocomputers.tccd1.tomte-run">
    ...
    <allow_active>auth_admin</allow_active>
  </action>
</policyconfig>
```

Règles : `allow_any=no`, `allow_inactive=no` partout ; **aucune** annotation `org.freedesktop.policykit.*` ;
message explicite (« changer le mode graphique », « installer un paquet »), pas « TCC » ; IDs versionnés.

### 4.3 systemd

`tccd.service` — ajouts (le reste du fichier est bon) :

```ini
[Unit]
StartLimitIntervalSec=60
StartLimitBurst=5
Wants=modprobe@tuxedo_io.service
After=modprobe@tuxedo_io.service

[Service]
MemoryMax=256M
TasksMax=64
CPUQuota=25%
ProtectKernelLogs=yes
KeyringMode=private
ProcSubset=pid
# Après validation sur matériel réel (ioctl autorisé dans @system-service) :
# SystemCallFilter=@system-service
# SystemCallFilter=~@mount @swap @obsolete @cpu-emulation @debug @keyring @module @raw-io
# SystemCallErrorNumber=EPERM
```

`tccd-admin.service` — à ne **pas** installer avant le binaire ; cible :

```ini
# LUKS hors périmètre : la proposition historique `block-*`/CAP_SYS_ADMIN est retirée.
DevicePolicy=closed
DeviceAllow=/dev/tuxedo_io rw
CapabilityBoundingSet=   # vide ; mesurer par service
MemoryMax=512M
KeyringMode=private
ProtectKernelLogs=yes
# pas de StateDirectory
```

`tccd-sleep.service` — **supprimer**, remplacer par un abonnement `org.freedesktop.login1`
`PrepareForSleep` dans `tccd` (réapplication au réveil) ou un hook `system-sleep` en Rust.

`tccd-user.service` — `UMask=0077`, `LimitCORE=0`, `ProtectKernelLogs=yes`, `MemoryMax=128M` ;
documenter la dépendance aux user namespaces (R-21).

### 4.4 Plan de tests (à porter par `qa-integration`)

| Fichier | Contenu | Couvre |
|---|---|---|
| `tcc-daemon/tests/dbus_authz.rs` | matrice méthode × rôle (root, utilisateur actif, inactif, nom inconnu) ; introspection → toute méthode non classée fait échouer | R-02, R-04 |
| `tcc-daemon/tests/dbus_limits.rs` | 10 000 appels, payload 10 Mio, 8 connexions concurrentes, RSS ± 5 Mo, réactivité après rejet | R-05 |
| `tcc-daemon/tests/policy_files.rs` | parse `packaging/dbus/*.conf` et `packaging/polkit/*.policy` : aucun `allow send_destination` sans interface/membre ; aucun `exec.path`/`exec.argv1`/`allow_gui` ; `<allow_any>` = `no` ; aucune action LUKS | R-01, R-02, R-09 |
| `tcc-daemon/tests/audit_log.rs` | subscriber de test : `caller_uid` présent, payload brut absent, aucun `/home/` | R-03, S-18 |
| `tcc-core/tests/config_atomicity.rs` (compléter) | symlink sur la cible, parent 0777, symlink de répertoire vers `/etc`, coupure entre `write` et `rename` | §3.1 |
| `tcc-user/tests/socket_security.rs` (compléter) | échange TOCTOU entre validation et `connect`, parent appartenant à un autre uid, socket non-UTF8 | §3.4 |
| `tcc-user/tests/display_filter.rs` | `HDMI-1`, `DP-1`, `eDP-evil`, `eDP-1/../x`, vide, NUL → rejet | R-13 |
| `cargo fuzz` | `parse_profile`, `parse_settings`, `dbus_payload`, `fan_curve`, `display_modes` | S-02/S-04/S-08 |

### 4.5 CI / supply chain

```yaml
# .github/workflows/ci.yml — ajouts
- run: cargo test --workspace --locked
- run: cargo build --workspace --release --locked
- name: Vérification des units
  run: |
    systemd-analyze verify packaging/systemd/*.service
    systemd-analyze security --offline=true packaging/systemd/tccd.service
- name: Politique D-Bus / polkit
  run: cargo test -p tcc-daemon --test policy_files
- name: Interdits
  run: |
    ! grep -rn "exec\.path\|allow_gui" packaging/polkit/
    ! grep -rn "RUN+=" packaging/udev/
    ! grep -rn "/opt" packaging/ | grep -v README
- name: Secrets
  uses: gitleaks/gitleaks-action@<sha>
```

Épingler `actions/checkout`, `Swatinem/rust-cache`, `EmbarkStudios/cargo-deny-action`,
`rustsec/audit-check` par **SHA de commit**. Ajouter `clippy.toml` + `[workspace.lints]` (R-17).

---

## 5. Checklist des actions avant packaging

### 5.1 Bloquants (aucun paquet ne doit sortir sans)

| # | Action | Fichier | Owner suggéré | Preuve attendue |
|---|---|---|---|---|
| 1 | Supprimer l'action polkit `tomte` et **toutes** les annotations `exec.*`/`allow_gui` | `packaging/polkit/…policy:62-76` | `packaging-release` + `security` | test `policy_files` + `grep` CI |
| 2 | Réécrire la politique D-Bus en fail-closed (allow par interface/membre) | `packaging/dbus/…conf` | `packaging-release` | `busctl` en non-root + test CI |
| 3 | Déplacer `SetTempProfile`/`ClearTempProfile` sur `Manager1` ; rate limit + audit `caller_uid` | `crates/tcc-daemon/src/dbus.rs` | `linux-platform` | `dbus_limits.rs`, `audit_log.rs` |
| 4 | Implémenter la vérification polkit + `SENSITIVE_METHODS` | `crates/tcc-daemon/src/authz.rs` (nouveau) | `linux-platform` + `security` | `dbus_authz.rs`, `polkit.rs` |
| 5 | Ajouter `MemoryMax`/`TasksMax`/`CPUQuota` + token bucket | `tccd.service`, `dbus.rs` | `linux-platform` | `systemd-analyze security`, `dbus_limits.rs` |
| 6 | Ne pas installer `tccd-admin.service`/activation tant que le binaire n'existe pas | `packaging/**` | `packaging-release` | arborescence de paquet inspectée |
| 7 | Remplacer `tccd-sleep.service` (ou le supprimer) | `packaging/systemd/tccd-sleep.service` | `packaging-release` + `linux-platform` | `systemd-analyze security` ≥ 4.0 |
| 8 | Corriger les défauts polkit (`allow_any=no`, `allow_inactive=no`, `auth_admin` pour Prime/Tomte/paquets ; aucune action LUKS) | `packaging/polkit/…policy` | `packaging-release` | `pkcheck` par action |
| 9 | Rétablir la vérité documentaire (polkit non implémenté, `SetTempProfile` global, `.bak` legacy) | `docs/**`, commentaires | `security` + `linux-platform` | revue de doc |

### 5.2 Importants avant release (peuvent être différés d'un jalon avec justification écrite)

| # | Action | Owner |
|---|---|---|
| 10 | `DevicePolicy=closed` + `DeviceAllow` minimal pour `tccd-admin` | `packaging-release` |
| 11 | `tccd-user` : connexion D-Bus persistante + souscription `StateChanged` | `linux-platform` |
| 12 | `kscreen-doctor` : timeout + `env_clear` + décision sur la persistance | `tuxedo-hardware`/`linux-platform` |
| 13 | Retirer `internal_output` du contrat client | `rust-core` |
| 14 | Filtre connecteur interne : regex + recoupement DRM | `tuxedo-hardware` |
| 15 | `Wants=modprobe@tuxedo_io.service` + test matériel | `packaging-release`/`tuxedo-hardware` |
| 16 | Overrides d'environnement GUI/CLI/agent derrière `dev-overrides` | `qt-ui` + `linux-platform` |
| 17 | `clippy.toml` + lints dans les binaires | `rust-core` |
| 18 | Tests de politique D-Bus/polkit + `--locked` + actions épinglées | `qa-integration` |
| 19 | `UMask`/`LimitCORE`/`MemoryMax` pour `tccd-user` | `packaging-release` |

### 5.3 Avant le jalon 2 (recommandé)

- Classification exhaustive des méthodes (`Manager1` / `Admin1`) dans `tcc-ipc`, avec test
  d'introspection bloquant.
- Décision écrite sur la portée de `SetTempProfile` (session vs machine) et sur le risque résiduel R1.
- Ajout de `docs/security/UNSAFE.md` (4 blocs ioctl, justification, tests) — checklist §2 A.
- `cargo deny check` + `cargo audit` exécutés et sorties archivées.

---

## 6. Limites de la revue

1. **Revue statique uniquement.** Aucun test dynamique privilégié : pas de daemon root lancé, pas de
   bus système instrumenté, pas de `pkcheck`, pas de `udevadm test`, pas de machine TUXEDO.
2. **Sémantique D-Bus** appuyée sur `man dbus-daemon` (dernière règle gagnante) et sur zbus 5.13.2.
   Une vérification empirique sur un bus de test reste nécessaire avant de considérer R-02/R-08 clos.
3. **`cargo deny`/`cargo audit` non exécutés** (outils absents). Le jugement supply chain porte sur la
   configuration et l'arbre de dépendances, pas sur les avis RUSTSEC du jour.
4. **Comportement de `Type=dbus` à la perte du nom** non testé (R-20) ; résolution de `DeviceAllow`
   sans module chargé non testée (R-22). Ces deux points sont explicitement marqués comme hypothèses.
5. **Aucune analyse binaire** : pas de `cargo geiger`, pas de `strace` du daemon, pas de vérification
   que le binaire release ne contient pas `dev-overrides` (il faudra `cargo tree -e features`).
6. **Pas de pentest** ni de fuzzing ; les findings de type DoS sont des raisonnements sur le code et
   les limites documentées, pas des mesures.
7. **Pas de revue du contenu du dossier `tuxedo-control-center-master/`** au-delà des fichiers
   d'intégration utilisés comme source de comparaison.
8. **Non couvert volontairement** : LUKS (pas de code), Aquaris/BLE, Prime, Tomte, WebFAI, diagnostic
   système — à revoir aux jalons 5-6 avec les exigences S-10/S-11/S-16.

---

## Annexe A — commandes exécutées (reproductibles)

```sh
# Units
systemd-analyze verify packaging/systemd/tccd.service
systemd-analyze security --offline=true packaging/systemd/{tccd,tccd-admin,tccd-user,tccd-sleep}.service

# Absence de garde-fous
grep -rn "PolicyKit\|CheckAuthorization\|polkit" --include="*.rs" crates
grep -rn "rate_limit\|token_bucket\|Semaphore\|LimitsExceeded" --include="*.rs" crates
grep -rn "Command::new" --include="*.rs" crates
grep -rn "sender()\|get_connection_credentials" --include="*.rs" crates

# unwrap/expect hors modules de test (script Python, cf. §0.2)
python3 - <<'PY'  # coupe chaque fichier au premier #[cfg(test)] puis compte
...
PY

# Dépendances
grep -n "git+" Cargo.lock            # aucun
find . -name build.rs -not -path "./target/*" -not -path "./tuxedo-control-center-master/*"

# Documentation de référence
man dbus-daemon | col -b | sed -n '555,660p'
man pkexec     | col -b | sed -n '60,102p'
man systemd.resource-control | col -b | sed -n '1046,1120p'
```

## Annexe B — matrice méthode × rôle attendue (jalon 2)

| Méthode | Interface | Utilisateur actif | Utilisateur inactif | Root | Polkit |
|---|---|---|---|---|---|
| `Ping`, `GetVersion` | compat | autorisé | autorisé | autorisé | non |
| `GetStateJSON`, `GetActiveProfileJSON`, `GetProfilesJSON`, `GetSettingsJSON` | compat | autorisé | autorisé | autorisé | non |
| `StateChanged` (signal) | compat | reçu | reçu | reçu | non |
| `SetTempProfile`, `ClearTempProfile` | `Manager1` | autorisé + rate limit (+ portée session au jalon 4) | refusé | autorisé | non (option B : `auth_admin_keep`) |
| `SetSettings`, `SaveProfile`, `DeleteProfile` | `Admin1` | refusé au niveau bus | refusé | autorisé | `com.tuxedocomputers.tccd1.configure` |
| `SetPowerLimits`, `SetCharging*` | `Admin1` | refusé au niveau bus | refusé | autorisé | `…tccd1.power-limits` / `…charging` |
| `PrimeSelect` | `Admin1` | refusé | refusé | autorisé | `…tccd1.prime-select` |
| `LuksChangeKey` | — | — | — | — | **Hors périmètre** (historique, non implémenté) |
| `TomteRun` | `Admin1` | refusé | refusé | autorisé | `…tccd1.tomte-run` (`auth_admin`) |

Toute méthode absente de ce tableau doit faire **échouer** le test d'introspection (checklist §3.2).

---

## Annexe C — Preuves, risques résiduels, points d'intégration

Conforme à `docs/QUALITY.md` §3 (« Preuves attendues dans un rapport d'agent »).

### C.1 Fichiers créés / modifiés

| Fichier | Nature |
|---|---|
| `docs/security/REVIEW_JALON1.md` | **créé** — présente revue |

Aucun autre fichier n'a été modifié : la revue est en lecture seule sur le code, conformément au
périmètre `docs/security/**`.

### C.2 APIs publiques et contrats D-Bus concernés par les findings

| Contrat | Emplacement | Finding |
|---|---|---|
| `com.tuxedocomputers.tccd` (compat) — `SetTempProfile`, `ClearTempProfile` | `crates/tcc-daemon/src/dbus.rs:35-121` | R-02, R-03 |
| `com.tuxedocomputers.tccd` — `GetStateJSON`, `GetActiveProfileJSON`, `GetProfilesJSON`, `GetSettingsJSON` | `crates/tcc-daemon/src/dbus.rs:49-79` | R-19 |
| `com.tuxedocomputers.tccd.Manager1` (constante seulement) | `crates/tcc-ipc/src/lib.rs:62` | R-02, R-04 |
| `com.tuxedocomputers.tccd.User1` (constantes + types, aucun service) | `crates/tcc-ipc/src/lib.rs:64-68`, `crates/tcc-ipc/src/{session.rs,display.rs}` | R-12 |
| `com.tuxedocomputers.tccd.Admin1` (aucune implémentation) | `packaging/dbus/com.tuxedocomputers.tccd.Admin.service`, `packaging/systemd/tccd-admin.service` | R-07 |
| Actions polkit `com.tuxedocomputers.tccd.*` | `packaging/polkit/com.tuxedocomputers.tccd.policy` | R-01, R-09 |

### C.3 Commandes exécutées et résultats exacts

| Commande | Résultat | Utilisé pour |
|---|---|---|
| `systemd-analyze verify packaging/systemd/tccd.service` | `Command /usr/libexec/tuxedo-rust-center/tccd is not executable` (seul défaut) | §0.2, R-07 |
| `systemd-analyze security --offline=true` | tccd **3.5 OK**, tccd-user **4.0 OK**, tccd-admin **4.4 OK**, tccd-sleep **9.2 UNSAFE** | §1.1, §3.6, R-06 |
| `systemd-analyze security` (unité témoin avec `DeviceAllow=/dev/definitely-absent-node`) | la spécification est acceptée telle quelle | R-22 |
| `man dbus-daemon` (§ policy) | « last rule that matches … determines whether it may be sent » | R-02, R-08 |
| `man pkexec` (§ ACTION AND AUTHORIZATIONS) | `org.freedesktop.policykit.exec.path` sélectionne l'action pour ce programme | R-01 |
| `man systemd.resource-control` (`DeviceAllow`, `DevicePolicy`) | listes résolues au démarrage ; `closed` autorise null/zero/full/random/urandom | R-22 |
| lecture `zbus-5.13.2` | `DoNotQueue` toujours actif ; `MAX_MESSAGE_SIZE = 128 Mio` ; dispatch par tâche | R-05, §3.5 |
| script `unwrap`/`expect` hors `#[cfg(test)]` | **0** occurrence sur les 7 crates | §3.1, R-17 |
| `grep -rn "PolicyKit\|CheckAuthorization"` | aucun résultat **à la date de la revue initiale** (depuis : `tcc-authz`/`tcc-admin`, cf. statut R-04) | R-04 |
| `grep -rn "rate_limit\|Semaphore\|LimitsExceeded"` | aucun résultat | R-05 |
| `grep -n "git+" Cargo.lock` | aucun résultat | §3.6, R-23 |
| `cargo deny check` / `cargo audit` | **non exécutés** (binaires absents) | §0.3 |
| `cargo fmt` / `cargo clippy` / `cargo test` | **non exécutés** par `security` (déjà vérifiés par `rust-core`, voir `docs/STATUS.md`) | — |

### C.4 Tests matériels

**Aucun test matériel réel n'a été effectué.** Les points suivants restent à valider sur laptop
TUXEDO (Clevo et Uniwill) avant de considérer les units et le chemin sysfs comme conformes :

1. `systemctl start tccd` avec `CapabilityBoundingSet=` **vide** : vérifier que toutes les écritures
   sysfs (cpufreq, backlight, LED, charge) réussissent sans `CAP_DAC_OVERRIDE`.
2. Démarrage **sans** module `tuxedo_io` chargé, puis `modprobe tuxedo_io` : vérifier l'ouverture de
   `/dev/tuxedo_io` (R-22).
3. `DevicePolicy=closed` + `DeviceAllow=/dev/tuxedo_io rw` : confirmer que le daemon ouvre le nœud et
   que rien d'autre n'est nécessaire (pas de `/dev/rtc`, pas de TTY).
4. `ProtectKernelTunables=no` : confirmer que `/sys` reste inscriptible et que `ProtectSystem=strict`
   n'empêche pas les écritures (les deux options sont indépendantes, à prouver par `strace`).
5. `SystemCallFilter=@system-service` : `strace -f -e trace=ioctl` pour vérifier qu'`ioctl` n'est pas
   filtré, avant d'activer le filtre (aujourd'hui commenté, `tccd.service:79-81`).
6. `tccd-user.service` sur une distribution avec `kernel.unprivileged_userns_clone=0` : vérifier si
   l'unité démarre (R-21).
7. Un cycle veille/reprise complet pour la stratégie de remplacement de `tccd-sleep` (R-06).

### C.5 Risques résiduels après cette revue

| # | Risque | Niveau | Statut |
|---|---|---|---|
| RR-1 | Les findings D-Bus/polkit sont **statiques** : le comportement réel de `dbus-daemon` vis-à-vis de l'ordre des règles et de `Properties.Set` doit être confirmé sur un bus de test | Faible | À valider avant clôture de R-02/R-08 |
| RR-2 | `cargo deny`/`audit` non exécutés : une advisory RUSTSEC peut être ouverte dans les 146 paquets | Moyen | CI déjà configurée ; exécuter et archiver |
| RR-3 | `tccd` continue de fonctionner sans nom D-Bus si le bus redémarre (R-20) | Faible | À vérifier par test |
| RR-4 | Le risque R1 du threat model (un utilisateur modifie le matériel partagé) reste **accepté** tant que `SetTempProfile` n'est pas restreint à la session (R-03) | Moyen | Décision à acter avec `linux-platform` |
| RR-5 | Aucune revue du futur `tccd-admin` : les capacités actuelles de l'unité sont provisoires et **ne doivent pas être publiées** (R-07) | Élevé | Bloqué par la checklist §5.1 item 6 |

### C.6 Points d'intégration pour les autres agents

| Agent | Actions attendues | Findings |
|---|---|---|
| `packaging-release` | polkit sans `exec.path`, politique D-Bus fail-closed, suppression/remplacement de `tccd-sleep`, `tccd-admin` non installé, durcissement `tccd`/`tccd-user`, actions CI épinglées | R-01, R-02, R-06, R-07, R-08, R-09, R-21, R-22, R-23 |
| `linux-platform` | module `authz` polkit + `SENSITIVE_METHODS`, déplacement de `SetTempProfile` sur `Manager1`, rate limiting, audit `caller_uid`, abonnement `PrepareForSleep`, connexion D-Bus persistante dans l'agent | R-03, R-04, R-05, R-06, R-10, R-20 |
| `tuxedo-hardware` | filtre connecteur interne (regex + DRM), timeout/`env_clear` sur `kscreen-doctor`, décision de persistance KDE | R-11, R-13 |
| `rust-core` | retrait de `internal_output`, `clippy.toml` + lints binaires, bornes mémoire côté parseurs | R-12, R-17 |
| `qt-ui` | plafond de taille des réponses, worker unique (threads bornés), overrides `dev-overrides` | R-14, R-15, R-16 |
| `qa-integration` | `dbus_authz.rs`, `dbus_limits.rs`, `policy_files.rs`, `audit_log.rs`, extension `config_atomicity.rs`/`socket_security.rs`, cibles de fuzzing | §4.4 |

### C.7 Procédure de re-revue

1. `packaging-release` applique R-01, R-02, R-06, R-07, R-08, R-09 et fournit les sorties
   `systemd-analyze verify|security` et un test `busctl` en utilisateur non privilégié.
2. `linux-platform` fournit `dbus_authz.rs`, `dbus_limits.rs`, `audit_log.rs` verts et la décision
   écrite sur la portée de `SetTempProfile`.
3. `security` rejoue la matrice méthode × rôle (annexe B), vérifie `docs/DBUS_API.md` et
   `PLATFORM_DESIGN.md` corrigés, puis met à jour `THREAT_MODEL.md` (statut des scénarios S-01,
   S-03, S-08, S-09, S-12) et `CHECKLIST.md` §3, §6.
4. Clôture : aucun finding Élevé ouvert, `cargo deny`/`audit` verts, score `systemd-analyze security`
   archivé.

---

## Prochaine revue (après `tccd-admin` phase 1)

À re-vérifier en priorité, avec preuve :

1. **R-04** : vérifier en conditions réelles le `CheckAuthorization` déjà câblé dans `tccd-admin`
   (polkit réel, policy système, refus si polkit absent) ; implémenter l'authz interne de `tccd`
   pour les setters `Manager1` avant leur réouverture (aujourd'hui refusés par la policy).
2. **R-03 / R-05** : décision écrite sur la portée du profil temporaire (session vs globale), bucket
   par UID, `Semaphore` de handlers, borne avant désérialisation ; test de rafale multi-connexions.
3. **R-10 / R-11** : souscription `StateChanged` dans l'agent, `env_clear()` sur `kscreen-doctor`,
   décision sur la persistance KScreen.
4. **R-12 / R-13** : retrait d'`internal_output` du contrat client, filtre connecteur interne par
   regex + recoupement `/sys/class/drm`.
5. **R-17 / R-23** : `disallowed-methods` par crate, job CI `packaging` (`validate.sh --repo`,
   `systemd-analyze verify|security`, `xmllint`, `dbus-policy-test.sh`).
6. **R-18 / R-19** : `.bak` de migration (ou correction de la doc), vue filtrée de `GetSettingsJSON`.
7. **Preuves à archiver** : `validate.sh --repo`, `dbus-policy-test.sh` **en root**, `systemd-analyze
   security` de `tccd` et `tccd-admin`, `cargo deny`/`audit`, et la sortie des tests `tcc-admin`.
