# Revue de sécurité — matrice matérielle TUXEDO

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue ciblée des écritures matérielles privilégiées, des commandes externes et de la surface D-Bus `Manager1` du jalon 2 |
| Documents lus | `docs/hardware/HARDWARE_MATRIX.md`, `docs/hardware/INTEGRATION_NOTES.md`, `docs/security/{THREAT_MODEL.md,DECISIONS.md,CHECKLIST.md,REVIEW_JALON1.md}`, `docs/packaging/SECURITY.md` |
| Code lu | `crates/tcc-hw/src/{charging,lighting,platform_profile,tdp,device,fn_lock,ycbcr,webcam,nvidia,prime,sensors,command,sysfs}.rs`, `crates/tcc-daemon/src/{hardware/**,dbus.rs,main.rs,sensor_lease.rs,rate_limit.rs}` |
| Intégration lue | `packaging/systemd/*.service`, `packaging/polkit/*.policy`, `packaging/dbus/*.conf`, `scripts/release/{validate.sh,dbus-policy-test.sh,install-tree.sh}`, `.github/workflows/ci.yml` |
| Verdict | **Aucune vulnérabilité critique.** 3 bloquants, 6 importants, 5 améliorations. Le point le plus sérieux est un **DoS non authentifié** (lectures + bail capteurs) que la remédiation R-05 n'a pas couvert ; le second est la **documentation qui annonce polkit sur des setters qui n'en ont pas**. Les écritures sysfs elles-mêmes sont bien encadrées. |

---

## Statut H-01 → H-14 (2026-11-06)

> **Instantané ~15:00, dépôt en mouvement.** Les constats des §1 à §8 restent l'état d'origine ; ce
> tableau donne l'état **vérifié** des correctifs. Certains sont depuis corrigés dans le corps du
> document (H-02 figure encore dans les risques principaux §1.3 : il est **fermé** ci-dessous). Preuves rejouées : lecture des fichiers cités,
> `cargo test -p tcc-hw --lib` (182) et `--test hardware_workers` (58), `validate.sh --repo`
> (133/133). Aucun test matériel n'a été exécuté (H-13).
>
> **Note de correspondance** : la demande de mise à jour mentionnait « H-01 cache `nvidia-smi` » et
> « H-02 `prime::set_mode` ». Dans **ce** document, H-01 est l'isolation de la bascule PRIME et H-02
> le DoS capteurs ; le cache `nvidia-smi` de 10 s est une mesure **nouvelle** rattachée à H-02. Les
> identifiants ci-dessous sont ceux du document, pas ceux du message.

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **H-01** | **Fermé** | `prime::set_mode`, `PRIME_SWITCH_TIMEOUT` et l'allowlist d'écriture ont été **retirés** de `tcc-hw` ; `crates/tcc-daemon/tests/no_admin_reachability.rs` échoue si `tccd` les référence (jetons interdits, commentaires ignorés, `src/bin/` exclu pour le futur admin) ; `HARDWARE_MATRIX.md:163` acte l'implémentation directe dans `tccd-admin` |
| **H-02** | **Fermé** | lecture snapshot limitée 2/s burst 5, bail 10 s **plafonné à 5 min** (`sensor_lease.rs:30`, `rate_limit.rs:26`), `SetSensorDataCollectionStatus` passe par `begin_write` (`dbus.rs:543`) ; **en plus** cache `nvidia-smi` 10 s (`nvidia.rs:24-52`) → coût borné même sous bail actif |
| **H-03** | **Fermé** | `command.rs:14,152` : `env_clear()` puis `PATH`/`LC_ALL`/`LANG` explicites |
| **H-04** | **Fermé** | `join_reader(reader, deadline)` + `READ_GRACE` (`command.rs:173-193,242-253`) : plus de `join()` sans échéance ; tests de timeout conservés |
| **H-05** | **Ouvert** | bucket toujours par **nom unique** (`rate_limit.rs:80-87`), pas de bucket UID ; `cleanup()` toujours O(n) sous mutex (`:128-145`) ; **aucun `Semaphore`** de handlers (grep vide) → contournement par multi-connexions et sérialisation sous rafale |
| **H-06** | **Partiel** | les setters ignorent toujours `keyboard_backlight_control_enabled` (grep vide dans `dbus.rs`) et ne persistent pas ; **mais ils sont injoignables** : la policy refuse les 8 setters au niveau du bus (`dbus.conf:111-133`) → risque neutralisé tant que polkit n'est pas branché ; à corriger **avant** réactivation |
| **H-07** | **Ouvert** | pas d'allowlist des valeurs inscriptibles : `ChargeType::is_known()` accepte `Unknown`/`N/A` (`charging.rs:92-129`, `dbus.rs:588-614`) |
| **H-08** | **Ouvert** | `MAX_CTGP_OFFSET_WATTS = 200` inchangé (`nvidia.rs:58`) ; le cache 10 s ne change pas la borne, qui reste non alignée sur `power.max_limit − power.default_limit` |
| **H-09** | **Ouvert (parité)** | profil `custom` toujours à `tdp_values: []` (`presets.rs:219`) → maxima du firmware au premier tick/changement ; non transactionnel ; documenté comme parité avec `ODMPowerLimitWorker.ts:44-47` |
| **H-10** | **Fermé** | `dbus.conf:90-133` : chaque méthode `Manager1` autorisée est listée **par membre**, les setters sont explicitement refusés ; plus d'`allow send_interface` global |
| **H-11** | **Fermé** | `HARDWARE_MATRIX.md:202-205,215,501-503` et `DBUS_API.md:394-417` disent explicitement « non polkit, non persistant » ; l'annonce « polkit `hardware` » a disparu |
| **H-12** | **Partiel** | `#![allow(dead_code)]` retiré de `hardware/mod.rs` ; **le commentaire udev est toujours faux** (« tccd écoute le netlink », `99-…rules:7-8`, alors que `grep netlink` = 0) ; contrôles UVC toujours hors périmètre (`controlsSupported:false`), décision §4 inchangée |
| **H-13** | **Ouvert** | aucune campagne matérielle : `DeviceAllow` sans module chargé, debugfs avec `ProtectKernelTunables=no`, `CapabilityBoundingSet=` vide non validés (cf. §6.2) |
| **H-14** | **Ouvert** | aucun job packaging en CI (`validate.sh`, `systemd-analyze`, `xmllint`, `dbus-policy-test.sh`) — même constat que R-23 |

**Décompte : 6 fermés, 2 partiels, 6 ouverts.**

Restent ouverts et **bloquants pour l'activation de `tccd-admin` phase 2** : H-05 (anti-DoS), H-06
(interrupteurs globaux avant réactivation des setters), H-07/H-08/H-09 (bornes matérielles) et H-13
(validation réelle). H-12 est un écart documentaire à corriger avant packaging.

---

## 0. Méthode, preuves et avertissements

### 0.1 Instantané de la revue

L'arborescence a **changé pendant la revue** : la matrice matérielle a été branchée dans
`tccd` en cours de séance (`mod hardware;`, interface `Manager1`, `sensor_lease.rs`).
La revue porte sur l'état suivant, horodaté :

| Fichier | mtime au moment de la revue |
|---|---|
| `crates/tcc-daemon/src/dbus.rs` | 14:26 |
| `crates/tcc-daemon/src/main.rs` | 14:25 |
| `crates/tcc-daemon/src/sensor_lease.rs` | 14:27 |
| `crates/tcc-hw/src/*.rs` (matrice) | 13:41 – 13:50 |
| `packaging/dbus/com.tuxedocomputers.tccd.conf` | 14:21 |

Les findings sont reproductibles tels quels sur cet instantané ; la §8 liste les points à
re-vérifier si le code bouge encore (notamment `dbus.rs`, propriété de `linux-platform`).

### 0.2 Ce qui a été exécuté (citations autorisées comme preuve)

| Commande | Résultat |
|---|---|
| `cargo test -p tcc-hw --lib` | **182 tests OK**, 0 échec |
| `cargo test -p tcc-daemon --test hardware_workers` | **58 tests OK**, 0 échec |
| `bash scripts/release/validate.sh --repo` | **133 vérifications réussies, 0 échec** |
| `systemd-analyze security --offline=true` | `tccd` **3.3 OK**, `tccd-admin` **3.1 OK**, `tccd-user` **3.6 OK** |
| `systemd-analyze verify` sur les 3 units | aucune erreur de syntaxe (seul « binaire absent » attendu) |
| `bash scripts/release/dbus-policy-test.sh` | **SKIP** (nécessite root) — non exécuté |
| script d'analyse `unwrap/expect/panic` hors `#[cfg(test)]` sur les 12 modules + workers | **0 occurrence** |
| `grep -rn "unsafe"` sur les 12 modules + `tcc-daemon/src/hardware` | **0 occurrence** hors commentaires |
| `grep -rn "netlink"` / `"sh -c"` / `"Command::new"` dans `tccd` | aucun netlink, aucun shell, aucun `Command::new` dans `tccd` |
| `grep -rn "CheckAuthorization\|PolicyKit"` dans `crates/**` | **aucun résultat** |

### 0.3 Ce qui n'a **pas** été fait

1. **Aucun test sur laptop TUXEDO.** Aucune écriture réelle (charge, LED, `platform_profile`,
   TDP, cTGP, Fn-Lock, debugfs, kill switch webcam) n'a été effectuée.
2. **Aucun test dynamique privilégié** : pas de `tccd` root, pas de `busctl` sur le bus
   système, pas de `pkcheck`, pas de `dbus-monitor`.
3. **`dbus-policy-test.sh` non exécuté** (root requis, sortie 77). La politique D-Bus est
   revue statiquement + par `validate.sh` (133 checks), pas fonctionnellement.
4. **`cargo deny`/`cargo audit` non exécutés** (binaires absents de l'environnement).
5. **Pas de fuzzing**, pas de `cargo geiger`, pas de `strace` du daemon.
6. **Hors périmètre** : `tcc-hw/src/{fan.rs,backlight.rs,cpu.rs,tuxedo_io.rs}`,
   `tcc-daemon/src/fan.rs`, `tcc-user`, GUI — déjà revus ou propriété d'autres jalons.
7. **Hypothèses non vérifiées** : comportement de `DeviceAllow` sans module chargé ;
   résolution de `/sys/kernel/debug` avec `ProtectKernelTunables=no` ; sémantique exacte
   du bail capteurs côté client (voir H-02) ; comportement du noyau pour `Unknown`/`N/A`
   écrits dans `charge_type` (H-07).

---

## 1. Résumé exécutif

### 1.1 Ce qui a changé par rapport à `REVIEW_JALON1.md`

| Finding jalon 1 | État constaté |
|---|---|
| R-01 polkit `exec.path` | **corrigé** — aucune annotation `exec.*`, aucun `allow_gui` (`polkit:10-31,151-153` ; `validate.sh` échoue si réapparition) |
| R-02 policy D-Bus ouverte | **corrigé** — `deny` global puis allow par interface/membre (`dbus.conf:39-103`) |
| R-03 `SetTempProfile` non borné | **corrigé** — identifiant borné, rate limit par nom unique, audit `caller_uid` (`dbus.rs:129-176`) |
| R-04 aucun polkit | **toujours ouvert** — déclaré et documenté comme non appliqué (`polkit:10-18`) |
| R-05 anti-DoS | **partiel** — écritures limitées, **lectures et collecte capteurs non limitées** (H-02) |
| R-06 `tccd-sleep` | **corrigé** — retiré du paquet, fichier `.example` non installé |
| R-07 `tccd-admin` | **corrigé** — non installé sans binaire (`install-tree.sh:64-68`), `Restart=no`, capacités vides |
| R-08 deny neutralisé | **corrigé** — ordre refus→allow, testé par `validate.sh:357-360` |
| R-09 polkit permissif | **corrigé** — `allow_any=no`, `allow_inactive=no`, Prime/Tomte sans `_keep` (LUKS hors périmètre) |
| R-20 perte du nom D-Bus | **corrigé** — `watch_connection` arrête le daemon (`dbus.rs:306-333`, `main.rs:319-325`) |
| R-22 `DeviceAllow`/module | **atténué** — `Wants=`/`After=modprobe@tuxedo_io.service` (`tccd.service:7-16`), **non validé** |

### 1.2 La matrice est maintenant active — ce que cela change

Contrairement à l'état du début de revue, `tccd` :

- démarre 9 workers matériels + le worker ventilateurs (`main.rs:18,218-232,263-264`) ;
- publie `com.tuxedocomputers.tccd.Manager1` (`dbus.rs:457-680,843`) avec **11 méthodes
  dont 9 `Set*`** : charge (profil, priorité, seuils ×2, type), clavier, lightbar,
  Fn-Lock, et l'armement de la collecte capteurs ;
- expose `SetSensorDataCollectionStatus` et `GetHardwareSnapshotJSON` ;
- **n'expose pas** d'écriture TDP, ODM, cTGP, PRIME, YCbCr (lecture seule) — point 7 vérifié.

Conséquence : la surface privilégiée du daemon qui parse des messages en continu a
augmenté. C'est précisément le scénario que D1/D3 cherchent à contenir, et c'est pourquoi
les findings ci-dessous portent sur l'**orchestration** et non sur les pilotes.

### 1.3 Les cinq risques principaux

| # | Risque | Sévérité |
|---|---|---|
| 1 | **DoSv non authentifiée par les lectures et le bail capteurs** : tout client local maintient la collecte active indéfiniment (`GetHardwareSnapshotJSON` réarme le bail, aucune limite sur les lectures) → lecture sysfs toutes les 2 s **et un `nvidia-smi` root toutes les 2 s** (~43 000 processus/jour). C'est S-09/F-10, non couvert par R-05. | **Moyen / B** |
| 2 | **Setters matériels non authentifiés** (charge, LED, Fn-Lock) accessibles à tout utilisateur local, alors que `HARDWARE_MATRIX.md:196` annonce « polkit `hardware` » et que `packaging/…policy:78-91` déclare une action `charging` jamais appelée. Sur poste multi-utilisateur, un utilisateur modifie le matériel d'un autre ; les interrupteurs globaux (`keyboard_backlight_control_enabled`) sont ignorés. | **Moyen / I** |
| 3 | **`join_reader` sans échéance** : si un descendant de `nvidia-smi`/`prime-select` garde un tube ouvert, le thread lecteur bloque pour toujours → le worker `spawn_blocking` gèle, `shutdown` expire, le domaine ne se réconcilie plus (perte de contrôle thermique/charge). | **Moyen / I** |
| 4 | **`prime::set_mode` non isolé au niveau crate** : `tccd` dépend de `tcc-hw` et pourrait appeler la bascule PRIME ; aucune barrière de compilation. Aujourd'hui non appelé (vérifié), mais D1 repose sur cette séparation. | **Élevé (latent) / B avant `TCC_WITH_ADMIN=1`** |
| 5 | **Aucune validation matérielle** des hypothèses de durcissement : `DeviceAllow=/dev/tuxedo_io` seul nœud autorisé, debugfs inscriptible via `ProtectKernelTunables=no`, `CapabilityBoundingSet=` vide alors que 0 capacité est peut-être insuffisante sur certains pilotes. Toute erreur ici se traduit par « matériel non piloté » silencieux. | **Moyen / B packaging** |

### 1.4 Répartition

**Bloquants (B) : 3** — H-01, H-02, H-13.
**Importants (I) : 6** — H-03, H-04, H-05, H-06, H-11, H-14.
**Améliorations (A) : 5** — H-07, H-08, H-09, H-10, H-12.

Aucun finding **Critique** : aucune primitive root générique, aucun secret, aucune
commande shell, aucun chemin fourni par le client n'atteint une écriture privilégiée.

---

## 2. Findings

### 2.1 Synthèse

| ID | Sév. | Fichier:ligne | Impact | Remédiation | Test |
|---|---|---|---|---|---|
| **H-01** | Élevé (latent) / B | `tcc-hw/src/prime.rs:212-224` ; `tcc-daemon/src/hardware/prime.rs:92-104` | `tccd` peut techniquement invoquer la bascule PRIME (root, reconnexion session) ; D1 n'est garanti que par convention | Isoler `set_mode` dans un crate/module non dépendance de `tccd` ; test CI d'interdiction d'appel | `grep` CI + test de graphe `cargo tree` |
| **H-02** | Moyen / B | `tcc-daemon/src/dbus.rs:464-499` ; `sensor_lease.rs:64-82` ; `hardware/sensors.rs:32,416-425` | DoS CPU/batterie non authentifié : collecte capteurs maintenue à vie + `nvidia-smi` root toutes les 2 s ; lectures non limitées | Limiter les lectures (bucket), plafonner le bail (durée max), compteur par client, arrêt sur déconnexion | Test : 100 `GetHardwareSnapshotJSON` → rejet ; bail max ; `nvidia-smi` espacé |
| **H-03** | Moyen / I | `tcc-hw/src/command.rs:131-137` | L'enfant hérite de l'environnement du daemon (`PATH`, `BASH_ENV`, `IFS` pour `prime-select`, script shell) | `env_clear()` + `PATH`/`LANG`/`LC_ALL`/`HOME` fixes | Test `/proc/<pid>/environ` |
| **H-04** | Moyen / I | `tcc-hw/src/command.rs:145-157,193-207` | Blocage permanent du thread lecteur si un descendant garde le tube → worker matériel gelé, `shutdown` expiré | Échéance de lecture + fermeture des fd + `join` borné | Test avec enfant qui laisse un descendant |
| **H-05** | Moyen / I | `rate_limit.rs:7-18,80-87` ; `dbus.rs:389-419` ; `tests/dbus_limits.rs` (une connexion) | Un UID ouvre N connexions → N×10 écritures ; `cleanup()` O(n) sous mutex en saturation | Second bucket par UID (uid déjà résolu) + plafond global ; test multi-connexions | `dbus_limits.rs` : N connexions même uid |
| **H-06** | Moyen / I | `dbus.rs:617-660` vs `hardware/lighting.rs:204` ; `dbus.rs:421-431` | Setters LED ignorent `keyboard_backlight_control_enabled` ; réglages non persistés et écrasés par SIGHUP ; cross-utilisateur | Respecter les interrupteurs ; persistance via `tccd-admin`+polkit ou documenter « volatil » | Test : contrôle désactivé → setter refuse |
| **H-07** | Faible / A | `tcc-hw/src/charging.rs:92-129` ; `dbus.rs:588-614` | `SetChargeType("Unknown"/"N/A")` accepté (`is_known`) alors que ces valeurs sont en lecture seule côté ABI | Allowlist des valeurs inscriptibles | Test des 9 valeurs |
| **H-08** | Faible / A | `tcc-hw/src/nvidia.rs:45,274-305` | Offset cTGP borné à 200 W sans lien avec `power.max_limit - power.default_limit` | Borner par la capacité NVIDIA quand disponible | Test avec limites simulées |
| **H-09** | Faible / A | `tcc-hw/src/tdp.rs:136-141,189-201` ; `hardware/odm.rs:319-331` | Profil `custom` (`tdp_values: []`) ⇒ PL1/PL2/PL4 = maxima du firmware ; écriture non transactionnelle | Valeurs explicites dans `custom` par défaut, log « TDP max », réessai | Test parse + journal |
| **H-10** | Faible / A | `packaging/dbus/…conf:85-88` | `Manager1` autorisée en bloc : toute méthode future joignable sans mise à jour de la politique | Énumérer les membres dès stabilisation + test d'introspection | `validate.sh` + test classification |
| **H-11** | Faible / I | `docs/hardware/HARDWARE_MATRIX.md:196,238-239` | La doc annonce « polkit `hardware` » et « persister » pour des setters qui ne font ni l'un ni l'autre | Corriger la matrice et `docs/DBUS_API.md` ; trancher le modèle | Revue de doc |
| **H-12** | Faible / A | `packaging/udev/…rules:9-10` vs `hardware/webcam.rs:35` | Commentaire « tccd écoute le netlink udev » faux (aucun netlink) ; contrôles UVC absents | Corriger le commentaire ; décision UVC §4 | `grep netlink` |
| **H-13** | Moyen / B | `packaging/systemd/tccd.service:59-66` ; `docs/packaging/SECURITY.md:107-132` | Aucune validation matérielle : `DeviceAllow`, debugfs, `CapabilityBoundingSet=` vide, userns (agent) | Campagne de smoke tests §6 | Rapport de test signé |
| **H-14** | Moyen / I | `.github/workflows/ci.yml` (aucun job packaging) | `validate.sh --repo`, `systemd-analyze`, `xmllint`, `dbus-policy-test.sh` ne tournent pas en CI : les régressions R-01/R-02 peuvent revenir sans échec | Job `packaging` bloquant | CI rouge sur policy modifiée |

### 2.2 Détail des findings bloquants et importants

---

#### H-01 — `prime::set_mode` n'est pas isolé de `tccd` au niveau du crate

**Sévérité : Élevé (latent) — B avant `TCC_WITH_ADMIN=1`.**

**Constat.** `crates/tcc-hw/src/prime.rs:212` expose :

```rust
pub fn set_mode(mode: PrimeMode) -> Result<()> { … command::run(program, &[argument], 60 s) }
```

`tccd` dépend de `tcc-hw` (`crates/tcc-daemon/Cargo.toml`), donc la fonction est
**joignable depuis n'importe quel code du daemon**. La seule séparation est documentaire
(`prime.rs:27-35`, `HARDWARE_MATRIX.md:162,396`, `INTEGRATION_NOTES.md:65`).
Vérification à l'instantané : `tccd` ne l'appelle pas —

```
$ grep -rn "set_mode" crates/tcc-daemon/src
crates/tcc-daemon/src/hardware/prime.rs:96:  self.state = Some(prime::detect(...))
crates/tcc-daemon/src/hardware/prime.rs:100: state.mode = prime::query_mode(...)
```

— mais rien n'empêche un futur diff de l'ajouter, et le futur `Manager1`/setter PRIME
sera écrit précisément dans `dbus.rs`, le fichier qui a le plus changé pendant la revue.

**Pourquoi une feature Cargo ne suffit pas.** `cargo build --workspace` unifie les
features d'une dépendance commune (resolver 3) : si `tccd-admin` active
`tcc-hw/admin`, `tccd` compilé dans le même build reçoit aussi le code. Une feature ne
serait fiable qu'avec des builds séparés (`cargo build -p tccd`), ce que rien n'impose.

**Remédiation recommandée.** Déplacer `set_mode`, `PRIME_SWITCH_TIMEOUT` et l'allowlist
`PRIME_SELECT` dans un crate `tcc-hw-admin` (ou un module `tcc-admin/src/prime.rs`) dont
**seul** `tccd-admin` dépend. À défaut : test CI bloquant
`! grep -rn "prime::set_mode" crates/tcc-daemon/src` + `cargo tree -p tccd` sans le crate
admin, et un commentaire `// SÉCURITÉ : ne jamais appeler depuis tccd` sur la fonction.

**Test.** `tests/no_admin_reachability.rs` (grep du source, exécuté en CI) : échoue si
`tcc-daemon/src/**` mentionne `set_mode`, `prime-select` ou `tccd-admin`.

---

#### H-02 — DoS non authentifié : lectures illimitées et bail capteurs réarmable

**Sévérité : Moyen — B.**

**Constat.** Trois mécanismes se combinent.

1. `GetHardwareSnapshotJSON` (`dbus.rs:464-469`) appelle `self.sensors.touch(Instant::now())`
   **avant** de produire l'instantané, sans aucune limite de débit :
   ```rust
   async fn get_hardware_snapshot_json(&self) -> fdo::Result<String> {
       self.sensors.touch(Instant::now());
       …
   }
   ```
2. `SensorLease::touch` (`sensor_lease.rs:78-82`) repousse le deadline de 10 s tant que
   le bail est actif ; le bail est **global** (pas de compteur par client), et une lecture
   suffit à le réarmer.
3. Le worker capteurs (`hardware/sensors.rs:32`) tourne toutes les 2 s et, quand le bail
   est actif, appelle `nvidia_smi.sensors()` (`hardware/sensors.rs:416-425`) →
   `command::run` → **un processus `nvidia-smi` root toutes les 2 secondes**
   (`nvidia.rs:141-147`, timeout 2 s).

Un client local non privilégié peut donc, en appelant `GetHardwareSnapshotJSON` toutes
les 9 s (ou `SetSensorDataCollectionStatus(true)`, lui aussi non authentifié et **non
limité**), maintenir indéfiniment :

- la lecture sysfs hwmon/RAPL/DRM toutes les 2 s ;
- le fork de `nvidia-smi` (~43 000 processus/jour, ~2 s de CPU cumulé/jour mais surtout
  des variations de fréquence GPU et de la consommation) ;
- la sérialisation JSON complète de la matrice à chaque lecture, sans plafond de fréquence.

C'est exactement S-09 dans `THREAT_MODEL.md` (« `SetSensorDataCollectionStatus(true)` pour
maintenir le daemon en collecte permanente ») et F-10. La remédiation R-05 n'a limité que
les **écritures** (`rate_limit.rs`, `dbus.rs:389-419`) : les lectures n'ont aucun bucket,
et le bail n'a pas de plafond absolu.

**Aggravant.** Le bail est réarmé par *n'importe quel* client : un client qui se déconnecte
laisse 10 s de collecte (acceptable), mais un client qui reste (ou plusieurs) maintient la
collecte pour tout le monde. L'UI légitime et l'attaquant sont indiscernables.

**Remédiation.**

1. Token bucket **lecture** par nom unique (50/s, burst 100) sur les méthodes de lecture
   coûteuses (`GetHardwareSnapshotJSON`, `GetStateJSON`, `GetProfilesJSON`) →
   `LimitsExceeded`.
2. Plafond absolu du bail : pas plus de N secondes cumulées sans « propriétaire » actif
   (par ex. 5 min), ou bail par nom unique avec arrêt sur `NameOwnerChanged` — c'est ce
   que `THREAT_MODEL.md` §8.8 demande déjà (« compteur par client, arrêt automatique à la
   déconnexion »).
3. Découpler l'armement du bail de la lecture : `SetSensorDataCollectionStatus` pour
   armer, `GetHardwareSnapshotJSON` pour lire *sans* réarmer indéfiniment (ou réarmer
   seulement si le bail a moins de X secondes).
4. Plafonner la fréquence de `nvidia-smi` (déjà 5 min pour les limites, mais la collecte
   de capteurs le lance à chaque tick) : cache de 1 s minimum dans `NvidiaSmi::sensors`.
5. Test : 100 lectures en rafale → rejet ; bail laissé 60 s sans lecture → collecte
   coupée ; `nvidia-smi` appelé au plus une fois par seconde dans un test instrumenté.

**Note.** `SetSensorDataCollectionStatus` ne consomme **pas** de jeton d'écriture
(`dbus.rs:472-493`) alors qu'il modifie un état global : il devrait passer par
`begin_write` ou par un bucket de lecture.

---

#### H-13 — Aucune validation sur matériel réel des hypothèses de durcissement

**Sévérité : Moyen — B packaging.**

**Constat.** Trois affirmations de `docs/packaging/SECURITY.md:107-132` ne sont pas
vérifiées par un test :

| Hypothèse | Réalité du code | Risque si fausse |
|---|---|---|
| `DeviceAllow=/dev/tuxedo_io rw` suffit avec `DevicePolicy=closed` | `tccd.service:64-66` ; dépend du module chargé au démarrage (`Wants=/After=modprobe@`, non testé) | si le nœud n'est pas résolu : `open` refusé → ODM/TDP/webcam indisponibles, daemon démarre en dégradé (pas d'escalade) |
| `CapabilityBoundingSet=` vide suffit pour toutes les écritures sysfs | `tccd.service:100-101` | si un attribut exige `CAP_DAC_OVERRIDE` (nœud non lisible par root), écriture refusée |
| debugfs inscriptible avec `ProtectKernelTunables=no` | `tccd.service:68-71` | si debugfs absent/RO : YCbCr `available:false` (dégradé, pas d'escalade) |

Le risque est **de disponibilité**, pas d'élévation. Mais un `DevicePolicy=closed` mal
réglé peut aussi produire un **contournement apparent** : si `tccd` échoue à ouvrir
`/dev/tuxedo_io`, il ne pilote plus le matériel et l'utilisateur croit que TCC applique
un profil (ventilateurs restent en BIOS). C'est A5.

**Remédiation.** Campagne de smoke tests §6 avec preuve par domaine (`available:true`,
valeur relue), `systemd-analyze security` archivé, et décision documentée pour chaque
capacité ajoutée.

---

#### H-03 — `command::run` n'applique pas `env_clear()`

**Sévérité : Moyen — I** (checklist §3.5, classé B dans `CHECKLIST.md`).

**Constat.** `crates/tcc-hw/src/command.rs:131-137` :

```rust
let mut command = Command::new(program);
command
    .args(args)
    .env("LC_ALL", "C")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
```

Pas de `env_clear()`, pas de `PATH` explicite. Les trois programmes sont des chemins
absolus (`NVIDIA_SMI`, `PRIME_SELECT`, `DPKG_QUERY`, `command.rs:37-56`), donc `PATH` ne
sert pas à les résoudre — mais :

- `prime-select` est un script shell : `BASH_ENV`, `ENV`, `IFS`, `CDPATH` sont héritées ;
- `nvidia-smi`/`dpkg-query` héritent de `LD_*` (non filtrés pour un processus non setuid) ;
- sous systemd l'environnement est vide (`tccd.service:46` le documente), donc l'impact
  réel est faible ; en `--dev-session` et dans les tests, l'environnement est celui de
  l'utilisateur — et `tests/dbus_limits.rs` lance déjà `tccd --dev-session`.

**Remédiation.**

```rust
command
    .env_clear()
    .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
    .env("HOME", "/")
    .env("LANG", "C")
    .env("LC_ALL", "C")
    .stdin(Stdio::null())
    …
```

**Test.** Lancer `command::run("/bin/sh", &["-c","env"], 2s)` avec un environnement témoin
contenant `TCC_TEST_SECRET=…` et vérifier que la sortie ne le contient pas
(ou lire `/proc/<pid>/environ` de l'enfant).

---

#### H-04 — `join_reader` peut bloquer indéfiniment

**Sévérité : Moyen — I.**

**Constat.** `command.rs:145-157` : les deux tubes sont lus dans des threads, puis :

```rust
let status = wait_with_timeout(&mut child, timeout)?;   // tue l'enfant à l'échéance
let stdout = join_reader(stdout_reader)?;                // <-- aucun timeout
let stderr = join_reader(stderr_reader)?;
```

`join_reader` (`command.rs:193-207`) fait `handle.join()`. Si l'enfant a lancé un
descendant qui hérite de `stdout`/`stderr` (script `prime-select` lançant un démon,
wrapper de distribution, `nvidia-smi` enveloppé), le tube n'est jamais fermé → le thread
lecteur ne reçoit pas d'EOF → `join()` bloque **pour toujours**. Le `kill` de
`wait_with_timeout` ne tue que l'enfant direct.

Conséquence : le `spawn_blocking` du worker matériel reste bloqué ; `HardwareWorkers::shutdown`
expire après 3 s (`hardware/mod.rs:164,365-375`) en laissant la tâche bloquée ; le domaine
concerné (ODM, NVIDIA, PRIME ou capteurs) ne publie plus d'état et n'applique plus rien,
alors que le daemon continue. Perte de contrôle thermique (A5).

**Remédiation.** Donner une échéance au lecteur : par exemple, après le `kill`, fermer les
descripteurs côté parent (`drop(child.stdout.take())` ne suffit pas, un descendant peut
toujours détenir la copie), ou implémenter la lecture avec `poll()`/`read` non bloquant et
une échéance globale (`Instant + timeout`), puis `join` avec un délai court. Une
alternative robuste : double-fork interdit, `Command::before_exec` interdit aussi — donc
`poll()` + deadline est la voie simple.

**Test.** `run("/bin/sh", &["-c", "sleep 5 & exec /bin/true"], 100 ms)` doit retourner une
erreur de délai **et** ne pas bloquer plus de ~200 ms (mesuré).

---

#### H-05 — Limiteur par nom unique contournable ; `cleanup()` O(n) sous mutex

**Sévérité : Moyen — I.**

**Constat.** `rate_limit.rs:7-18` l'assume : bucket par **nom unique** D-Bus, « pas par
UID ». Or un utilisateur local ouvre autant de connexions qu'il veut : chaque connexion
reçoit 10 jetons de burst (H-05 ; `MAX_TRACKED_CALLERS=1024` borne la mémoire, pas le
débit). Un UID avec 1024 connexions obtient ≈10 240 écritures immédiates, soit 1024 fois
le seuil annoncé de 5/s. Le test `tests/dbus_limits.rs` ne couvre qu'**une** connexion
(commentaire l. 8-10).

Second point : `cleanup()` (`rate_limit.rs:95-123`) s'exécute à chaque `allow()` dès que
`buckets.len() >= MAX_TRACKED_CALLERS` ; il fait un `retain` sur 1024 entrées **et** une
boucle `while` avec `min_by_key` (O(n) chacun) en tenant le `std::sync::Mutex`
(`dbus.rs:397-410`). Sous rafale, chaque appel paie ~2×1024 comparaisons ; les handlers
D-Bus sont concurrents (zbus spawn par méthode), donc plusieurs threads se sérialisent sur
ce mutex.

Troisième point : **aucun plafond de handlers concurrents** (`THREAT_MODEL.md` §8.8 exige
un `Semaphore(32)` et un rejet immédiat). Chaque appel de setter commence par
`resolve_caller_uid` (`dbus.rs:396`, deux allers-retours D-Bus avec 500 ms de timeout)
**avant** le limiteur : N appels simultanés = N tâches zbus, chacune avec un proxy et une
résolution d'UID, sans borne. Avec `MemoryMax=256M`, un flood de méthodes peut saturer la
cgroup avant que le limiteur n'ait consommé le moindre jeton.

Enfin, `resolve_caller_uid` échoue « ouvert » (`None`) : l'écriture passe et seul l'audit
perd l'UID (`dbus.rs:271-291`). Acceptable pour un champ informatif, **inacceptable** si
cette valeur devient un jour la base d'un `CheckAuthorization` — il faudra alors refuser
l'appel si l'identité n'est pas résolue.

**Remédiation.** (1) Second bucket par UID (`resolve_caller_uid` est déjà appelé — le
résultat est disponible), (2) plafond global (par ex. 200/s toutes connexions), (3) borner
le coût de `cleanup` : éviction d'un lot au lieu d'une boucle par appel, ou nettoyage dans
la tâche de supervision (1 s) plutôt que dans `allow()`, (4) `Semaphore(32)` sur les
setters avec rejet immédiat (`LimitsExceeded`), et résolution d'UID **après** le limiteur
pour ne pas payer deux RTT sur un appel déjà rejeté, (5) test multi-connexions.

**Test.** `tests/dbus_limits.rs` : ouvrir 5 connexions depuis le même UID, faire 11 appels
chacune → au plus 10 succès au total (bucket UID) ; mesurer que la 100ᵉ seconde ne bloque
pas.

---

#### H-06 — Les setters Manager1 ignorent les interrupteurs globaux et ne sont pas persistés

**Sévérité : Moyen — I.**

**Constat A — interrupteurs ignorés.** `SetKeyboardStatesJSON` (`dbus.rs:617-636`) et
`SetLightbarStatesJSON` (`dbus.rs:639-660`) écrivent immédiatement les LED sans consulter
`settings.keyboard_backlight_control_enabled`, alors que le worker, lui, la respecte
(`hardware/lighting.rs:204`). Un client local peut donc allumer/éteindre le clavier alors
que l'utilisateur a explicitement désactivé le contrôle, et sur une machine multi-utilisateur
modifier l'éclairage d'une autre session (même classe que R-03 : matériel global, appelant
non authentifié).

**Constat B — non-persistance.** `update_settings` (`dbus.rs:421-431`) mute
`RuntimeState.settings` en mémoire et incrémente la révision, sans jamais appeler
`ConfigStore::save_settings`. Deux conséquences :

- au redémarrage, les valeurs sont perdues (documenté « non persistant ») ;
- **un `SIGHUP` les écrase** : `reload()` (`main.rs:410-427`) recharge `Settings` depuis
  `/var/lib` et remplace l'objet en mémoire. L'UI peut afficher « appliqué » alors que la
  valeur a disparu. C'est une incohérence d'état, pas une faille.

**Constat C — `SetChargeType` ne peut pas persister** : `Settings` n'a pas de champ
`charge_type` (`dbus.rs:610-612`), donc le setter ne fait que rafraîchir la télémétrie.

**Remédiation.** (1) Refuser les setters d'éclairage si
`keyboard_backlight_control_enabled == false` (ou exposer un `Force` explicite et
journalisé) ; (2) pour la persistance durable, passer par `tccd-admin` + polkit
(D1/D2) — sinon documenter « volatil, réappliqué jusqu'au prochain SIGHUP » dans
`docs/DBUS_API.md` et rafraîchir l'UI sur SIGHUP ; (3) test « contrôle désactivé → setter
refuse, LED inchangées ».

---

#### H-11 — La documentation annonce polkit et de la persistance qui n'existent pas

**Sévérité : Faible — I** (risque de revue et d'attente de sécurité).

**Constat.**

- `docs/hardware/HARDWARE_MATRIX.md:196` : `SetChargingProfile` → « polkit `hardware` ».
  Or `grep -rn "CheckAuthorization" crates/` est **vide**, et l'action `hardware` n'existe
  plus (renommée `charging`/`power-limits` dans `packaging/…policy:64-91`).
- `HARDWARE_MATRIX.md:238-239` : `SetKeyboardStatesJSON`/`SetLightbarStatesJSON` →
  « valider + persister ». Le code ne persiste pas (`dbus.rs:421-431`).
- `dbus.conf:17-25` et `packaging/SECURITY.md:189-218` sont, eux, exacts (« non encore
  appliquées »). La contradiction est donc interne à la doc matérielle.

**Remédiation.** Corriger `HARDWARE_MATRIX.md` §3.1/3.2 et `docs/DBUS_API.md` : soit
« setters non authentifiés, rate-limités, audités, non persistants » (état réel), soit
décider l'authentification et l'implémenter avant de l'annoncer. Le modèle retenu doit
être écrit dans `docs/DBUS_API.md` (section Sécurité) et testé.

---

#### H-14 — La CI ne rejoue pas les validations de packaging

**Sévérité : Moyen — I.**

**Constat.** `.github/workflows/ci.yml` contient `fmt`, `clippy`, `test`, `integration`,
`qml`, `test-arch`, `msrv`, `gui`, `deny`, `audit` — **aucun job packaging** :
`scripts/release/validate.sh --repo` (133 checks, exécuté manuellement ici),
`systemd-analyze verify|security`, `xmllint` et `dbus-policy-test.sh` ne sont pas dans la
CI. Les deux régressions les plus graves du jalon 1 (R-01 `exec.path`, R-02 policy
ouverte) ont précisément eu lieu dans ces fichiers, et ne seraient détectées aujourd'hui
que par une exécution manuelle.

**Remédiation.** Job `packaging` bloquant :

```yaml
- run: scripts/release/validate.sh --repo
- run: systemd-analyze verify packaging/systemd/*.service
- run: systemd-analyze security --offline=true packaging/systemd/tccd.service
- run: xmllint --noout packaging/dbus/*.conf packaging/polkit/*.policy
```

et un job séparé (conteneur privilégié, non requis sur PR externe) pour
`dbus-policy-test.sh`.

---

### 2.3 Améliorations

#### H-07 — `SetChargeType` accepte des valeurs ABI en lecture seule

`ChargeType::from_sysfs` (`charging.rs:92-106`) mappe `"Unknown"` et `"N/A"` vers des
variantes, et `is_known()` (`:127-129`) les considère comme valides : le setter
(`dbus.rs:588-614`) écrit donc `Unknown` ou `N/A` dans `charge_type`, valeurs que l'ABI
`sysfs-class-power` documente comme non inscriptibles. Le noyau rejettera (`EINVAL`), ce
qui remonte en `fdo::Error::Failed` avec le message noyau — pas de faille, mais une
erreur utilisateur trompeuse. **Remédiation** : allowlist des seules valeurs inscriptibles
(`Trickle`, `Fast`, `Standard`, `Adaptive`, `Custom`, `LongLife`, `Bypass`) + test.

#### H-08 — Borne cTGP non alignée sur la capacité réelle

`MAX_CTGP_OFFSET_WATTS = 200.0` (`nvidia.rs:45`), validé et écrit (`:274-305`). Un
*offset* cTGP réel se compte en dizaines de watts au plus ; la borne réelle est
`power.max_limit - power.default_limit`, disponible via `nvidia-smi` (`:117-134`).
**Remédiation** : quand `NvidiaSmi::power_limits()` est disponible, borner à
`min(200, max - default)` ; sinon borne statique basse documentée avec la source driver.
Test avec limites simulées (déjà partiellement couvert par les fixtures).

#### H-09 — TDP : maxima du firmware sur profil vide (parité) et écriture non transactionnelle

`TdpControl::set_values(&[])` applique les maxima (`tdp.rs:27-28,136-141`) ; parité
**vérifiée dans l'original** (`ODMPowerLimitWorker.ts:44-47` : « Default to max values »),
donc ce n'est pas une régression. Mais le profil `custom` livré par défaut a
`tdp_values: []` (`presets.rs:219`), et `SetTempProfile` (R-03, non authentifié) permet de
le sélectionner → PL1/PL2/PL4 au maximum, à chaque révision et à chaque boot. Impact
thermique (A1), pas d'élévation. **Remédiation** : valeurs explicites dans `custom` par
défaut, ou log `info!("TDP max appliqué (profil sans valeurs)")` pour que l'utilisateur
comprenne ; documenter la non-transactionnalité (`tdp.rs:30-33`) et réessayer sur
`notify()`.

#### H-10 — `Manager1` autorisée en bloc dans la politique D-Bus

`dbus.conf:85-88` :

```xml
<allow send_destination="com.tuxedocomputers.tccd"
       send_interface="com.tuxedocomputers.tccd.Manager1"/>
```

Aujourd'hui cela couvre 11 méthodes dont 9 `Set*` ; demain toute méthode ajoutée
(p. ex. un `SetPowerLimits`) sera joignable sans toucher la politique. La philosophie
fail-closed de R-02 est donc à moitié appliquée (compat = membre par membre, Manager1 =
interface entière). **Remédiation** : énumérer les membres `Manager1` dès que la liste est
stable, et ajouter le test d'introspection qui échoue sur une méthode non classée
(`CHECKLIST.md` §3.2).

#### H-12 — Commentaire udev inexact et contrôles UVC absents

`packaging/udev/99-tuxedo-rust-center-webcam.rules:9-10` affirme que `tccd` « écoute le
netlink udev » ; `grep -rn netlink crates/` est vide : le worker ré-énumère toutes les 2 s
(`hardware/webcam.rs:35`). `HARDWARE_MATRIX.md:403-407` décrit correctement le polling.
**Remédiation** : corriger le commentaire (ou implémenter le netlink, mais le polling est
documenté comme un compromis assumé), et trancher les contrôles UVC (§4). Profiter du
branchement effectif pour retirer `#![allow(dead_code)]` de `hardware/mod.rs:64` (il
masquerait désormais un setter ou un worker jamais appelé).

---

## 3. Ce qui est déjà conforme (preuves)

### 3.1 Écritures sysfs privilégiées

| Exigence | Preuve |
|---|---|
| Chemins confinés, aucun chemin fourni par le client | `sysfs.rs:122-142` (refus absolu, `..`, `RootDir`) ; **aucune** méthode D-Bus n'accepte de chemin (`grep` des paramètres `String` : charge, JSON d'éclairage, id de profil) |
| `O_NOFOLLOW`, `fstat`, propriétaire/permissions | `sysfs.rs:239-249,337-368` ; `ownership_is_safe` : `uid == 0 && mode & 0o002 == 0` |
| Écriture atomique d'une valeur, rejet des caractères de contrôle | `sysfs.rs:243-249` (un seul `write_all`), `310-334` |
| `platform_profile` : propriété PPD appliquée **dans la couche matérielle** (défense en profondeur) | `platform_profile.rs:175-183` renvoie `PolicyDenied` si `PpdOwned` ; le worker traduit en « non appliqué » (`hardware/odm.rs:263-269`) |
| `platform_profile` : valeur ∈ `*_choices`, idempotence, validation de chaîne | `platform_profile.rs:184-202`, `217-235` |
| Charge : profil/priorité ∈ listes noyau, longueur ≤ 64, ASCII graphique, pas de `\` | `charging.rs:477-506`, `580-603`, `62` |
| Charge : seuils `0..=100`, `start < end` (y compris seuil unique relu), ∈ listes disponibles, `Custom` avant seuils | `charging.rs:302-350`, `416-475` |
| Charge : idempotence (valeur identique non réécrite) et relecture après écriture | `charging.rs:495-513`, `hardware/charging.rs:155-161` |
| Charge : type ∈ ABI | `charging.rs:92-129`, `383-401` |
| LED : chemins issus de la détection uniquement, couleurs `0..=255`, luminosité plafonnée à `max_brightness`, `buffer_input` relâché même en erreur (`Drop`) | `lighting.rs:521-523,314-370` ; `BufferGuard` (`:526-573`) |
| TDP : entier fini, `[min,max]` firmware, monotonie `PL1 ≤ PL2 ≤ PL4` validées **avant la première écriture** | `tdp.rs:131-173,205-221` |
| TDP : aucune écriture si le firmware n'expose rien | `tdp.rs:132-135` |
| cTGP : fini, entier, `0..=200`, idempotent, attribut présent | `nvidia.rs:274-305` |
| Fn-Lock : booléen uniquement, attribut absent ⇒ `NotSupported` | `fn_lock.rs:46-65` |
| YCbCr : `card ≤ 16`, port `[A-Za-z0-9-]` sans bord `-`, pas de `/`, double barrière (port énuméré + `force_path`) | `ycbcr.rs:81-115`, `hardware/ycbcr.rs:110-136` |
| YCbCr : debugfs absent ⇒ `NotSupported`, pas d'erreur fatale | `ycbcr.rs:137-141` |
| Webcam : kill switch par ioctl seulement ; `/dev/videoN` dérivé d'un nom `videoN` validé ; `controlsSupported=false` honnête | `webcam.rs:92-158` ; `hardware/webcam.rs:110-117` |
| Aucun `unsafe` dans la matrice | `grep -rn unsafe` sur les 12 modules + workers : 0 occurrence hors commentaires |

### 3.2 Commandes externes

| Exigence | Preuve |
|---|---|
| Allowlist de chemins absolus, impossible de fournir un chemin runtime | `command.rs:37-56` + signature `program: &'static str` (`:120-124`) ; `run` refuse un chemin relatif (`:125-130`, test `:274-277`) |
| Arguments séparés, aucun shell | `command.rs:131-137` ; `grep "sh -c\|/bin/sh"` : uniquement les tests |
| stdin fermé, stdout/stderr capturés | `command.rs:135-137` |
| Timeout + `kill` + `wait` (pas de zombie) | `command.rs:210-230`, test `:280-289` |
| Sortie bornée à 64 Kio par flux, UTF-8 lossy | `command.rs:32,177-191` |
| Erreurs nettoyées (contrôle supprimé, 512 chars) | `command.rs:233-243`, test `:292-297` |
| `nvidia-smi` : lecture seule, 2 s, arguments figés | `nvidia.rs:47-58,117-168` |
| `prime-select query` : lecture seule, 3 s, argument unique `query` | `prime.rs:189-199,47` |
| `dpkg-query` : arguments figés (`-W`, `-f=${Version}\n`, `nvidia-prime`) | `prime.rs:50-51,254-265` |
| `prime-select <mode>` : argument issu d'une **énumération** (`intel`/`nvidia`/`on-demand`), jamais d'une chaîne | `prime.rs:95-107,212-224` |
| Aucun `Command::new` dans `tccd` | `grep` : seul `tcc-hw` exécute |

### 3.3 Surface D-Bus `Manager1` (point 7)

| Exigence | Preuve |
|---|---|
| Setters limités en débit | `dbus.rs:389-419` : token bucket 5/s burst 10 **partagé** entre tous les setters matériels (`writes`), `LimitsExceeded` |
| Audit de l'appelant | `dbus.rs:412-417` : `caller`, `caller_uid`, `action`, sans payload ; rejet aussi journalisé (`:401-406`) |
| Bornes d'entrée avant usage | identifiant ≤ 64 o (`dbus.rs:266-273`) ; seuils `0..=100` (`:692-699`) ; types de charge allowlist (`:598-603`) ; JSON éclairage ≤ 64 Kio (`:701-752`) ; ≤ 64 zones (`:709-713`) ; couleurs `-1..=255` / `0..=255` (`:714-726`, `:738-749`) |
| Validation matérielle en seconde barrière | `ChargingController` revalide contre les listes noyau (`charging.rs:477-513`) ; `LightingController` plafonne et valide (`lighting.rs:314-370`) |
| Aucune écriture TDP/ODM/cTGP/PRIME/YCbCr exposée | liste des méthodes `Manager1` : `GetHardwareSnapshotJSON`, `SetSensorDataCollectionStatus`, `GetSensorDataCollectionStatus`, `SetChargingProfile/Priority`, `SetChargeStart/EndThreshold`, `SetChargeType`, `SetKeyboardStatesJSON`, `SetLightbarStatesJSON`, `SetFnLockStatus` — **aucun** `SetTdp*`, `SetOdm*`, `SetCtgp*`, `SetPrime*`, `SetYcbcr*` |
| `tccd` ne bascule pas PRIME | `main.rs`/`hardware/prime.rs` : `detect` + `query_mode` uniquement (H-01 pour la barrière de compilation) |
| Lecture de la matrice non bloquante | `hardware/mod.rs:202-305` : `RwLock` standard, clones courts, tolérance au poison |
| `dry_run` respecté par tous les setters | `dbus.rs:512,534,556,577,604,627,649,672` |
| Collecte capteurs désactivée par défaut | `sensor_lease.rs:34-49,155-164` ; worker gaté (`hardware/mod.rs:419-423`) : aucune lecture si le drapeau est faux |
| Arrêt sur perte du bus | `dbus.rs:306-333` + `main.rs:319-325,334-336` |

### 3.4 Cohérence D1–D7 (point 8)

| Décision | État | Preuve / écart |
|---|---|---|
| **D1** séparation `tccd`/`tccd-admin` | Respecté à l'exécution, **non garanti à la compilation** | `tccd-admin` non installé sans binaire (`install-tree.sh:64-68`) ; `set_mode` joignable depuis `tccd` (H-01) |
| **D2** état dans `/var/lib`, `/etc` RO | Respecté | `tccd.service:51-53` ; `ConfigStore` (règle `/etc`) ; les setters Manager1 ne persistent rien (H-06) |
| **D3** compat JSON gelée, lecture seule | Respecté sauf 2 setters documentés | `DBUS_API.md` ; `dbus.conf:61-83` énumère membre par membre ; **écart mineur** : D3 parle de `SetTempProfileByName`, le code expose `SetTempProfile`/`ClearTempProfile` |
| **D4** PPD propriétaire, dérogation explicite/journalisée | Respecté **par construction** | `with_ppd_override(false)` codé en dur (`main.rs:68-71,205`) : aucune dérogation n'est atteignable ; `platform_profile.rs:175-183` refuse côté hw. Conséquence : le `ppd_override` de `HardwareInputs` est du code mort (H-06 note) |
| **D5** secrets LUKS par fd | Non applicable (pas de code LUKS) | `packaging/SECURITY.md:23` ; `LimitCORE=0` sur les deux units |
| **D6** jamais `/proc/*/environ` | Respecté | `grep "/proc"` : uniquement `/proc/self/status` (`config.rs:1016`, `main.rs:530`, `security.rs:30`, `sysfs.rs:283`) |
| **D7** overrides dev verrouillés | Respecté | feature `dev-overrides` absente des units ; `validate.sh:281-282` échoue si `TCC_SYSFS_ROOT`/`TCC_CONFIG_DIR` actifs ; `main.rs` ne les lit pas |
| Règles système (hardening) | Respecté | scores `3.3/3.1/3.6` ; exceptions documentées (`packaging/SECURITY.md:107-132`) ; `ProtectKernelTunables=no` justifié |
| Aucun setuid, aucun `/opt` | Respecté | `validate.sh` (133 checks) ; `find -perm /6000` contrôlé en CI de packaging par `validate.sh --tree` |

---

## 4. Décision demandée : contrôles UVC V4L2 (point 4)

**Constat.** `crates/tcc-hw/src/webcam.rs:27-44` documente l'absence d'ioctls V4L2 et
propose deux options ; `hardware/webcam.rs:116` publie `controls_supported: false` ;
`HARDWARE_MATRIX.md:27,409-412,482` le confirme. Aucun `unsafe` V4L2 n'existe (vérifié).

**Recommandation `security` : trois options, dans cet ordre.**

1. **Jalon 2 (recommandé) : hors périmètre, état explicite.** Livrer
   `controlsSupported:false`, ne pas exposer l'écran de réglages avancés, et documenter
   l'écart dans `FEATURE_PARITY.md` (parité différée) + `HARDWARE_MATRIX.md`. Aucun code
   `unsafe` n'entre dans la base avant le jalon 4. C'est la seule option qui ne crée
   **aucune** surface privilégiée nouvelle dans `tccd` avant la revue de branchement
   complète.
2. **Jalon 4 : module `tcc-hw/src/v4l2.rs` dédié (option recommandée pour la parité).**
   Contraintes non négociables :
   - `#![deny(unsafe_code)]` **conservé** au niveau du crate ; `unsafe` uniquement dans un
     sous-module `real` avec `#[allow(unsafe_code)]`, sur le modèle exact de
     `tuxedo_io::real` (`tuxedo_io.rs:191-295`), avec `// SAFETY:` par appel ;
   - 4 ioctls seulement : `VIDIOC_QUERYCAP`, `VIDIOC_QUERYCTRL`, `VIDIOC_G_CTRL`,
     `VIDIOC_S_CTRL` ; structures `v4l2_control`/`v4l2_queryctrl` recopiées du noyau et
     **vérifiées par un test de taille/offset** (comme les constantes ioctl de
     `tuxedo_io.rs:1068`) ;
   - ouverture `O_NOFOLLOW|O_CLOEXEC` sur `/dev/videoN` où `N` provient déjà de
     `valid_video_name` (`webcam.rs:161-164`) ; `fstat` → périphérique caractère,
     **major `81`** (VIDEO_MAJOR) et minor attendu ; jamais un chemin fourni par un client ;
   - **allowlist de contrôles issue du noyau** : `VIDIOC_QUERYCTRL` avec
     `V4L2_CTRL_FLAG_NEXT_CTRL`, puis `VIDIOC_QUERYMENU` pour les menus ; un `S_CTRL`
     n'est accepté que pour un `id` de cette liste, avec `min ≤ value ≤ max` et `step`
     respecté ; jamais un `id` fourni par l'UI sans recoupement ;
   - valeurs persistées dans `/var/lib/tuxedo-rust-center/webcam.json` (root:root 0644),
     validées au chargement (`tcc-core::validate`), jamais appliquées depuis udev ;
   - transport abstrait `V4l2Transport` + fake, comme `TuxedoIoTransport`, pour que la CI
     teste la logique sans webcam ;
   - `controls_supported` ne passe à `true` que si le module est compilé **et** qu'au
     moins un contrôle est énuméré.
3. **Crate `v4l` auditée : acceptable mais pas préférable.** Licence MIT compatible, mais
   elle ajoute du FFI généré (`v4l2-sys-mit`/bindgen) et une surface d'audit bien plus
   grande que ~150 lignes locales ; il faudrait l'épingler, la mettre dans `cargo vet`, et
   revoir son `unsafe`. À réserver au cas où l'option 2 serait refusée.

**Interdit dans tous les cas** : `unsafe` dans `tcc-daemon/src/hardware/webcam.rs` ou dans
un setter D-Bus ; `ioctl` exécuté sur le runtime async (toujours `spawn_blocking`) ;
contrôle UVC piloté par une trame udev ; `id`/valeur de contrôle non validés contre
l'énumération noyau.

---

## 5. Disponibilité et contournement (point 5)

| Sujet | Constat | Risque | Recommandation |
|---|---|---|---|
| `DeviceAllow=/dev/tuxedo_io` | `tccd.service:64-66` + `Wants=/After=modprobe@` (`:7-16`) ; non testé | si module absent au démarrage : règle perdue → `open` refusé → ODM/TDP/webcam indisponibles, **daemon vivant** (dégradé) | test sur modèle à chargement tardif ; à défaut `DeviceAllow=char-*` documenté |
| `DevicePolicy=closed` | seul `/dev/tuxedo_io` + pseudo-périphériques | correct aujourd'hui ; tout futur besoin (`/dev/dri`, i2c) exigera un `DeviceAllow` revu | revue `security` obligatoire pour tout ajout |
| debugfs (`force_yuv420_output`) | n'est **pas** un nœud de périphérique : `DevicePolicy` ne le couvre pas ; accessible via `ProtectKernelTunables=no` | écriture root dans `/sys/kernel/debug` non filtrée par le cgroup devices ; barrière = validation de chemin (`ycbcr.rs:81-115`) + port énuméré | documenter explicitement l'exception ; test « debugfs non monté ⇒ `available:false` » |
| `CapabilityBoundingSet=` vide | `tccd.service:100-101` | si un attribut sysfs exige `CAP_DAC_OVERRIDE`, écriture refusée (dégradé) | mesurer par domaine ; ajouter la capacité minimale une par une |
| Module `tuxedo_io` absent | `ensure_io()` réessaie toutes les 6 ticks (30 s ODM, ~12 s webcam) | pas de crash (vérifié) ; pas de reprise des domaines déjà « disponibles » si le device disparaît | test « module déchargé/rechargé » |
| `ProtectSystem=strict` + `ReadWritePaths` | `/var/lib` + `/run` uniquement | si un worker veut écrire ailleurs (cache), échec silencieux | revue à chaque nouveau chemin |
| `tccd-user` (hors périmètre) | `ProtectSystem=strict` en unité utilisateur | userns restreints ⇒ unité en échec, agent absent | déjà documenté (P-04), à tester |

---

## 6. Actions bloquantes avant packaging et smoke tests matériels

### 6.1 Bloquants

| # | Action | Fichier | Owner |
|---|---|---|---|
| 1 | Isoler `prime::set_mode` (crate/module séparé) + test CI d'interdiction d'appel depuis `tccd` | `tcc-hw/src/prime.rs:212` ; CI | `tuxedo-hardware` + `security` |
| 2 | Limiter les lectures et plafonner le bail capteurs (bucket lecture, plafond absolu, compteur par client, cache `nvidia-smi`) | `dbus.rs:464-499` ; `sensor_lease.rs` ; `hardware/sensors.rs:416-425` | `linux-platform` |
| 3 | Campagne de smoke tests matériels (§6.2) avec preuve par domaine et `systemd-analyze security` archivé | units + rapport | `packaging-release` + `tuxedo-hardware` |

### 6.2 Smoke tests à exécuter sur laptop TUXEDO (Clevo **et** Uniwill si possible)

| Domaine | Test | Attendu |
|---|---|---|
| Démarrage | `systemctl start tccd` sans module `tuxedo_io` chargé | démarrage OK, `snapshot.odm.available=false`, pas de boucle |
| `tuxedo_io` | `modprobe tuxedo_io` puis attendre 2 ticks ODM | interface + version renseignées, `odm.available=true` |
| Charge | `SetChargingProfile`, `SetChargeStartThreshold(60)`, `SetChargeEndThreshold(80)` via `busctl` en utilisateur non privilégié | écriture réelle, relecture 60/80, `charge_type=Custom`, pas d'`EINVAL` |
| Charge | `SetChargeStartThreshold(95)` avec fin 80 | refus (`InvalidArgs`), **aucune** écriture |
| Charge | redémarrage du daemon | valeurs **non** réappliquées si `charge_type != Custom` |
| Éclairage | `SetKeyboardStatesJSON` blanc puis RGB 3 zones puis per-key | bonne zone, `buffer_input` revenu à 0, pas de clignotement |
| Éclairage | `keyboard_backlight_control_enabled=false` + setter | refus ou aucun effet (H-06) |
| ODM | profil `performance` puis `power_save` (PPD absent) | `platform_profile` suit, LED/ventilateur cohérents |
| ODM | PPD actif, profil TCC différent | **aucune** écriture (`owner:ppd`), log unique |
| TDP | preset `High Performance` (60/60/70) puis `custom` (vide) | valeurs exactes puis maxima ; pas de PL1 > PL4 |
| cTGP | offset 15 W sur dGPU NVIDIA | relu 15, `nvidia-smi` cohérent ; refus à 500 |
| PRIME | lecture seule dans `tccd` ; `tccd-admin` **non installé** | `GetPrimeDataJSON` sans `prime-select` lancé en écriture |
| Webcam | kill switch sur Clevo, `WebcamSWAvailable` | bascule réelle, `controlsSupported:false` |
| Webcam | Uniwill | `switchAvailable:false`, aucune erreur |
| Fn-Lock | `SetFnLockStatus(true/false)` | `fn_lock` suit, persiste après reboot matériel |
| YCbCr | debugfs monté, port DP externe | écriture `1`, `pendingModeReapply`, écran OK ; debugfs non monté ⇒ `available:false` |
| Capteurs | collecte désactivée 60 s | aucun `nvidia-smi` (`pgrep`/`strace`), `enabled:false` |
| Capteurs | collecte active | 1 `nvidia-smi`/s max, valeurs cohérentes avec `powerstat` |
| Veille | `systemctl suspend` puis reprise | workers reprennent, TDP/charge/webcam réappliqués |
| D-Bus | `busctl` non-root sur `Manager1.SetChargeType` / `SetKeyboardStatesJSON` | joignable (parité) **ou** refusé si la décision H-06 change ; jamais `Admin1` |
| D-Bus | 11 appels rapides `SetFnLockStatus` | `LimitsExceeded` au 11ᵉ |
| Hardening | `strace -f -e trace=openat,ioctl` sur 5 min | aucun `openat` hors `/sys`/`/proc/self`/`/dev/tuxedo_io`/`/var/lib` ; `ioctl(TIX)…` attendus |

---

## 7. Risques résiduels acceptés (à acter)

| # | Risque | Niveau | Justification |
|---|---|---|---|
| M-01 | Tout utilisateur local peut changer le profil temporaire, la charge, les LED et le Fn-Lock | Moyen | Parité avec l'original ; matériel partagé (R1 du threat model) ; rate limit + audit en place ; à documenter dans `docs/DBUS_API.md` (H-11) |
| M-02 | Aucune action polkit n'est appliquée par le code | Élevé (régression) | Déclaré non implémenté (`polkit:10-18`, `packaging/SECURITY.md:189-218`) ; bloquant jalon 2 (R-04) |
| M-03 | Collecte capteurs maintenue par un client | Moyen | **Ne pas accepter** : c'est H-02 (bloquant) |
| M-04 | TDP max sur profil `custom` | Faible | Parité vérifiée ; à rendre visible (H-09) |
| M-05 | `DeviceAllow`/debugfs/`CapabilityBoundingSet` non validés | Moyen | H-13 (bloquant packaging) |

---

## 8. Limites de la revue et re-vérification

1. **Cible mouvante** : `dbus.rs` et `sensor_lease.rs` ont été modifiés pendant la revue
   (14:25–14:27). Les findings H-02/H-05/H-06/H-10 doivent être rejoués si `dbus.rs`
   change encore ; les numéros de ligne peuvent dériver de quelques unités.
2. **Revue statique** : aucun test root, aucun bus système, aucun laptop. Les conclusions
   de disponibilité (H-04, H-13, §5) sont des raisonnements sur le code et la documentation
   systemd, pas des mesures.
3. **`dbus-policy-test.sh` non exécuté** (root requis) : la politique est validée par
   `validate.sh` (statique) et par revue, pas fonctionnellement.
4. **`cargo deny`/`audit` non exécutés** ; aucune nouvelle dépendance n'a été introduite
   dans `tcc-hw` (`libc`, `thiserror`, `tracing` uniquement), ce qui limite le risque.
5. **Pas de fuzzing** des parseurs introduits (`parse_keyboard_states`,
   `parse_lightbar_state`, parsing CSV `nvidia-smi`, JSON `kscreen`/compositeur) : cibles
   recommandées pour `qa-integration`.
6. **Non couverts** : `fan.rs` (PWM, 1550 lignes) et `tuxedo_io.rs` (ioctl) — déjà revus
   au jalon 1 ou propriété `tuxedo-hardware` ; à réauditer si le chemin TDP/ventilateurs
   change.
7. **Hypothèses explicites** : le noyau rejette `Unknown`/`N/A` dans `charge_type` (H-07) ;
   `DeviceAllow` avec nœud absent ne fait pas échouer l'unité mais perd la règle (H-13) ;
   un descendant de `nvidia-smi`/`prime-select` peut garder un tube ouvert (H-04).

### Points à re-vérifier avant de clore

- [ ] `grep -rn "set_mode" crates/tcc-daemon/src` : toujours vide, avec test CI.
- [ ] `GetHardwareSnapshotJSON` : bucket de lecture présent, bail plafonné.
- [ ] `SetKeyboardStatesJSON` : comportement défini quand
      `keyboard_backlight_control_enabled == false`.
- [ ] `packaging/polkit` : aucune action `exec.*` (déjà garanti par `validate.sh`).
- [ ] `docs/hardware/HARDWARE_MATRIX.md` : polkit/persistance corrigés.
- [ ] CI : job packaging ajouté et rouge sur une policy modifiée.
- [ ] Rapport de smoke tests matériels signé.

---

## Annexe A — preuves exécutées

```text
$ cargo test -p tcc-hw --lib
test result: ok. 182 passed; 0 failed

$ cargo test -p tcc-daemon --test hardware_workers
test result: ok. 58 passed; 0 failed

$ bash scripts/release/validate.sh --repo
  vérifications réussies : 133
  échecs                 : 0

$ systemd-analyze security --offline=true packaging/systemd/{tccd,tccd-admin,tccd-user}.service
tccd.service       → 3.3 OK
tccd-admin.service → 3.1 OK
tccd-user.service  → 3.6 OK

$ bash scripts/release/dbus-policy-test.sh
SKIP: le test D-Bus nécessite root (service de test root, client non privilégié)

$ grep -rn "CheckAuthorization\|PolicyKit" crates/    # aucun résultat
$ grep -rn "netlink" crates/                          # aucun résultat
$ grep -rn "unsafe" crates/tcc-hw/src/{charging,lighting,platform_profile,tdp,device,fn_lock,ycbcr,webcam,nvidia,prime,sensors,command}.rs crates/tcc-daemon/src/hardware/
  # uniquement des commentaires (webcam.rs:32,38 ; hardware/webcam.rs:17)
```

## Annexe B — matrice setters × rôle (état réel, à acter)

| Méthode | Interface | Auth | Rate limit | Audit | Persistance | Effet matériel |
|---|---|---|---|---|---|---|
| `SetTempProfile` / `ClearTempProfile` | compat | non | 5/s burst 10 | oui | non | profil complet (CPU, luminosité, + matrice) |
| `SetChargingProfile` / `Priority` | `Manager1` | non | 5/s burst 10 (partagé) | oui | **non** (H-06) | sysfs `tuxedo_keyboard` |
| `SetChargeStart/EndThreshold` | `Manager1` | non | idem | oui | **non** | sysfs `power_supply`, force `Custom` |
| `SetChargeType` | `Manager1` | non | idem | oui | n/a | sysfs `charge_type` |
| `SetKeyboardStatesJSON` | `Manager1` | non | idem | oui | **non** | LED `multi_intensity`/`brightness` |
| `SetLightbarStatesJSON` | `Manager1` | non | idem | oui | **non** | LED lightbar |
| `SetFnLockStatus` | `Manager1` | non | idem | oui | **non** | sysfs `fn_lock` |
| `SetSensorDataCollectionStatus` | `Manager1` | non | **aucun** (H-02) | oui | n/a | active la collecte 10 s |
| `GetHardwareSnapshotJSON` | `Manager1` | non | **aucun** (H-02) | non | n/a | réarme le bail |
| `SetTdp*`, `SetOdm*`, `SetCtgp*`, `SetPrime*`, `SetYcbcr*` | — | — | — | — | — | **non exposés** |

## Annexe C — décisions à trancher

| # | Question | Recommandation `security` |
|---|---|---|
| 1 | Les setters charge/éclairage restent-ils non authentifiés ? | Oui pour le jalon 2 (parité, rate limit + audit), **à condition** de corriger la doc (H-11) et de respecter les interrupteurs globaux (H-06). Toute persistance durable doit passer par `tccd-admin` + polkit. |
| 2 | Faut-il un `temp-profile` polkit ? | Non par défaut (tray) ; oui en option « multi-utilisateur strict », avec audit. |
| 3 | Contrôles UVC | Option 1 (hors périmètre jalon 2), puis option 2 (module dédié) au jalon 4 ; option 3 seulement après revue de la crate. |
| 4 | `Manager1` en bloc ou membre par membre ? | Membre par membre dès que la liste est stable + test d'introspection. |
| 5 | `ppd_override` | Supprimer le paramètre mort ou l'implémenter avec un champ `Settings` validé, un réglage UI explicite et un log à chaque écriture sous PPD. |

---

## Prochaine revue (après `tccd-admin` phase 1)

À re-vérifier, dans cet ordre :

1. **H-05** : bucket par **UID** en plus du nom unique, `cleanup()` hors du chemin chaud (tâche de
   supervision) et `Semaphore` de handlers — test de rafale avec N connexions du même UID.
2. **H-06** : avant toute réactivation des setters `Manager1`, respect de
   `keyboard_backlight_control_enabled`/`fan_control_enabled` et décision explicite sur la
   persistance (volatile documentée ou passage par `tccd-admin` + polkit).
3. **H-07 / H-08 / H-09** : allowlist des `charge_type` inscriptibles, borne cTGP alignée sur
   `nvidia-smi`, et valeurs TDP du profil `custom` (ou log « TDP max appliqué »).
4. **H-10 / H-11** : conserver la policy membre par membre et la doc « non polkit » lors de
   l'ajout des méthodes `Admin1`/`Control1` ; vérifier qu'aucun `allow` d'interface ne réapparaît.
5. **H-12** : corriger le commentaire udev (ou implémenter le netlink) et statuer sur V4L2
   (hors périmètre jalon 2, module dédié au jalon 4 — §4).
6. **H-13** : campagne de smoke tests matériels (§6.2) avec preuve par domaine et
   `systemd-analyze security` archivé.
7. **H-14** : job CI `packaging` bloquant.
8. **H-01 / H-02** : rejouer `no_admin_reachability` et les tests de bail capteurs après le
   branchement de `tccd-admin`, pour vérifier qu'aucune régression n'a été introduite par le
   nouveau service.
