# Revue de sécurité — implémentation `Admin.Hw` (TDP / ODM / cTGP)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue de l'implémentation réelle du service phase 2 `tccd-admin-hw` / `AdminHw1` et conditions de branchement GUI |
| Périmètre | `docs/security/**` uniquement — aucun code ni packaging modifié |
| Code lu | `crates/tcc-admin/src/bin/tccd-admin-hw.rs`, `src/hw.rs`, `src/runtime.rs`, `src/service.rs` (`PolkitAuthorizer`), `crates/tcc-ipc/src/{lib.rs,admin.rs}`, `crates/tcc-hw/src/{tdp,platform_profile,nvidia,device}.rs`, `crates/tcc-daemon/src/main.rs` (override PPD), `crates/tcc-gui/src/daemon_client.rs` (client `Admin.Hw`), `packaging/systemd/tccd-admin-hw.service`, `packaging/dbus/com.tuxedocomputers.tccd{,.Admin.Hw.service}.conf`/service, `packaging/polkit/com.tuxedocomputers.tccd.policy`, `scripts/release/{install-tree,dbus-policy-test}.sh` |
| Design de référence | `docs/security/ADMIN_PHASE2_DESIGN.md` (P2-01…P2-10, §2.4, §3, §5, §9) |
| Preuves exécutées | `cargo check --workspace` **OK** ; `cargo test -p tcc-admin` ⇒ **61 lib + 5 main + 2 binaire hw + 12 `admin_dbus` + 4 `admin_hw_dbus` + 4 CLI + 12 transaction = 100/100** ; `cargo test -p tcc-gui --bins` ⇒ **167/167** (dont `tst_admin_hw` QML **non exécuté**) ; `cargo test -p tcc-daemon --bins` ⇒ **203/203** ; `systemd-analyze security --offline=true tccd-admin-hw.service` ⇒ **1.9**. **Non exécutés** : vrai polkit, bus système/policy root, `/dev/tuxedo_io` réel, `platform_profile`/`ctgp_offset` réels, PPD réel, `dbus-policy-test.sh` (root). |
| Verdict | **Aucun Critique/Élevé/Moyen/Faible ouvert côté code.** AH-01/03/04/05/06/08/**09** sont **fermés** ; AH-02 est **accepté et documenté** (platform_profile refusé, voie io-only autorisée) ; AH-07 et AH-11 restent **Info** (libellé d'audit `GetTdpValues`, borne cTGP UI statique/ordre de valeurs TDP côté client). **Activation `TCC_WITH_ADMIN_HW=1` possible côté code**, sous réserve des preuves manuelles root/polkit/matériel. |

---

## 1. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **AH-01** | **Moyen / dispo** | `hw.rs:1084-1103` (`SetTdpValues(Vec<i32>)`), `:1107-1127` (`SetOdmProfile(String)`), `:598-606`, `:973-981` ; `tccd-admin-hw.service:93` | Les arguments D-Bus sont désérialisés en `Vec<i32>`/`String` **avant** tout contrôle : `values.len() > 3` et `name.len() > 64` sont vérifiés une fois l'allocation faite. Un appelant local non authentifié peut envoyer un `ai`/`s` proche de la limite D-Bus (~128 Mio) ; `MemoryMax=128M` tue alors le service avant polkit. `StartLimitBurst=20/60 s` permet de bloquer ensuite les activations. | Déni de service du service TDP/ODM/cTGP (pas d'écriture, pas d'escalade). Même classe qu'AD-R1, mais sans borne 1 Mio côté Admin.Hw. | Borner la taille du message au niveau de la connexion zbus (`max_message_size`) ou refuser dans un désérialiseur borné ; à défaut, documenter et tester la limite + `MemoryMax`/`StartLimitBurst`. Ne pas compter sur le seul contrôle post-allocation. | Test D-Bus avec `ai` de 1 Mio/64 Mio ⇒ erreur typée ou rejet, service vivant ; test de rafale d'activations. |
| **AH-02** | **Moyen / politique** | `hw.rs:846-866` ; test `:1659` (`odm_io_fallback_is_allowed_with_ppd_owned`) ; design §2.4 | Un nom présent **uniquement** dans `TuxedoIo::odm_profiles()` est écrit via `tuxedo_io` même si PPD est `Owned` ou `Unknown`. Le design l'autorise explicitement (PPD ne connaît pas ces profils), mais la criterion de revue « PPD possédé/inconnu ⇒ refus » est alors contredite. | Si la politique attendue est un refus global ODM quand PPD est propriétaire/inconnu, c'est un contournement de D4 par la voie io. Sinon, exception légitime mais à documenter. Aucune escalade root. | Trancher : (a) refuser **toute** écriture ODM si `ppd.blocks_platform_write()` (sauf `Override` consenti) ; ou (b) documenter formellement l'exception io-only (PPD ne gère pas `tuxedo_io`) et l'ajouter à `DBUS_API.md`/design. | Test `Owned`/`Unknown` + nom io-only ⇒ refus (a) ou succès documenté (b) ; test nom dans les deux listes ⇒ refus (déjà présent). |
| **AH-03** | **Moyen / cohérence** | `crates/tcc-daemon/src/main.rs` (ancien `:114,443`, corrigé — voir §1.1) ; `hw.rs:522-527,782-786` ; design §9.8 | `SetOdmProfileOverride` lit `Settings.ppd_override` sur disque (`SystemOverrideConsent`), mais `tccd` construit `HardwareInputs` avec `.with_ppd_override(false)` en dur (deux sites), avec un commentaire « aucun réglage utilisateur n'existe encore » alors que le champ existe (`Settings.ppd_override`, GUI). | L'override Admin.Hw est **transitoire** et sera ignoré/incohérent lors de la réapplication par `tccd` ; le daemon peut continuer à considérer PPD propriétaire et entrer en conflit avec le choix consenti. Le prérequis d'activation §9.8 n'est pas satisfait. | Lire `settings.ppd_override` dans `tccd` et le propager aux deux `HardwareInputs` ; ou retirer `SetOdmProfileOverride` tant que ce câblage n'est pas fait. | Test daemon : settings `ppd_override=true` ⇒ `HardwareInputs.ppd_override=true` et ODM appliqué ; `false` ⇒ PPD respecté. |
| **AH-04** | **Faible / A** | `hw.rs:818-825` ; design §5.3 | Si l'écriture `platform_profile` réussit mais que la relecture ne renvoie pas le nom cible, le service retourne `WriteFailed` **sans réappliquer l'ancien profil**, pourtant disponible dans `platform.current` avant l'écriture. | Profil ODM laissé dans un état inattendu ; pas d'escalade, mais la transaction best-effort annoncée par le design n'est pas honorée. | Restaurer `previous` s'il est connu et non vide, relire, puis `WriteFailed` (`restore=ok|failed`) ; sinon `PartialApply`/`WriteFailed` explicite. | Fake ODM : écriture OK, relecture ≠ cible ⇒ ancien profil restauré ; relecture après restauration. |
| **AH-05** | **Faible / A** | `hw.rs:260-289` (`RealCtgpBackend::max_offset` → `None`), `:585-591`, `:869-942` ; design §2.4/§5.4 | La borne **dynamique** cTGP n'est jamais lue : `RealCtgpBackend::max_offset` renvoie toujours `None` (pas d'instantané `tccd`, pas de `nvidia-smi`), donc seule la borne statique **200 W** s'applique. Une valeur inférieure à 200 mais supérieure au maximum réel peut être tentée ; le driver/la vérification la refusent, mais l'API ne la rejette pas avant écriture. | Écriture refusée par le matériel ou offset hors plage réelle ; audit `bound=static`, pas d'escalade. | Implémenter la lecture de `max_offset` depuis `GetHardwareSnapshotJSON` avec timeout 2 s, ou documenter explicitement que la borne statique 200 W est la seule barrière jusqu'à la phase 2.1. | Fake cTGP `max=150` ⇒ `151` refusé avant écriture ; réel : offset max lu. |
| **AH-06** | Info | `hw.rs:755-786` ; design §3.4 | `SetOdmProfileOverride` vérifie le consentement **après** le rate-limit, polkit et le verrou (`authorize_write` puis `write_gate` puis `override_consent`). Avec `ppd_override=false`, l'utilisateur déclenche donc un dialogue polkit avant d'être refusé (`PolicyDenied("ppd.override_disabled")`). | Dialogue inutile, fatigue d'authentification ; borné par le rate-limit 2/s et le dialogue unique. Pas d'escalade. | Lire le consentement (lecture `/var/lib`, bon marché) **avant** `authorize_write`, ou ajouter un refus pré-polkit ; conserver le rate-limit avant le dialogue. | Faux authorizer compteur : `ppd_override=false` ⇒ 0 appel polkit, `PolicyDenied`. |
| **AH-07** | Info | `hw.rs:945-969` ; `:736-752` ; `:846-866` | Audit incomplet : les erreurs d'identité, de validation pré-autorisation, et de `GetTdpValues` ne produisent pas de ligne ; la voie `tuxedo_io` réussie journalise `polkit="none"` alors que l'action `power-limits` a été vérifiée ; le rollback TDP ne journalise pas explicitement `restore=ok|failed` (le code d'erreur le suggère). | Traçabilité partielle ; pas de fuite de secret. | Une ligne par appel (y compris refus précoces), `polkit=…` exact, `restore=ok|failed|none` explicite ; garder les valeurs hors logs. | Capteur `tracing` : un événement par méthode/refus ; champs attendus. |
| **AH-08** | Info | `tccd-admin-hw.service:1-5` ; `packaging/README.md:7,19,42` ; `packaging/dbus/com.tuxedocomputers.tccd.Admin.Hw.service:2-5` | Les commentaires d'unité et le README affirment encore que `tccd-admin-hw` **n'existe pas** / n'est pas livré, alors que le binaire existe, que `install-tree.sh` l'installe sous `TCC_WITH_ADMIN_HW=1` et que le service D-Bus est fourni. `systemd-analyze verify` échoue en arbre faute de binaire au chemin installé (attendu hors buildroot). | Confusion packaging/release ; risque d'activer ou non le service sur une documentation fausse. Aucune faille d'exécution. | Mettre à jour commentaires/README ; vérifier l'unité dans le buildroot (`TCC_WITH_ADMIN_HW=1`), pas dans l'arbre source. | `install-tree.sh` avec/sans binaire ; `systemd-analyze verify` sur buildroot. |
| **AH-09** | Info | `crates/tcc-gui/src/daemon_client.rs:1032-1078` ; `AdminHwData.qml` ; design §2.4 | Le client GUI existe et compile, mais **aucune page QML n'appelle** `requestOdmApply`/`requestTdpApply`/`requestCtgpApply` ; `AdminHwData` n'est pas consommé par les pages. Les bornes locales ne reflètent pas le contrat : TDP accepte **8** valeurs (serveur **3**), ODM **128** caractères (serveur **64**), cTGP **-200..200** (serveur **0..200**). | Plumbing non exposé aujourd'hui ; en cas de branchement, erreurs tardives après validation serveur (voire après polkit pour cTGP négatif). | Avant d'exposer l'UI : aligner les bornes client sur `MAX_TDP_VALUES`/`MAX_HW_STRING_CHARS`/`0..=MAX_CTGP_STATIC_OFFSET_WATTS`, ajouter les contrôles QML, tests fake `Admin.Hw` et tests de refus. | Tests client : 4 valeurs TDP, nom 65 caractères, cTGP -1 ⇒ refus local sans appel D-Bus. |
| **AH-10** | Info | `tccd-admin-hw.service:17,51,56,101` | `CapabilityBoundingSet=` vide, `DeviceAllow=/dev/tuxedo_io rw`, `ProtectKernelTunables=no` : les écritures sysfs (`platform_profile`, `ctgp_offset`) et les ioctl supposent que root sans capacité suffit. `DeviceAllow` sur un chemin absent peut aussi faire échouer l'activation selon systemd. | Si le noyau exige `CAP_SYS_ADMIN`/`CAP_DAC_OVERRIDE`, le service est inutilisable ; si `DeviceAllow` échoue sans module, l'activation échoue. Hypothèse à mesurer. | Validation matérielle Clevo **et** Uniwill : ouverture `/dev/tuxedo_io`, écriture sysfs avec capacités vides, `strace`, `systemd-analyze security` ; ajuster `AmbientCapabilities`/`DeviceAllow` seulement avec preuve. | Smoke test root réel sur les deux plateformes ; `systemctl start` sans module. |

> **Instantané initial.** Les constats du §1 décrivent l'état avant correctifs ; le statut courant
> et les preuves sont en **§1.1**.

### 1.1 Statut des correctifs (2026-11-06)

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **AH-01** | **Fermé** | `BoundedAdminHwInterface` (`hw.rs:1368-1400`) refuse un corps > **64 Kio** avant dispatch (`InvalidArgs("message.size")`) ; `max_queued(8)` (`tccd-admin-hw.rs:158`) ; tests D-Bus 65 Kio et 20 000 `i32` (`admin_hw_dbus.rs:279-316`) ; `MemoryMax=256M` (`tccd-admin-hw.service:98`). |
| **AH-02** | **Accepté (décision)** | `platform_profile` reste refusé si PPD `Owned`/`Unknown` ; la voie `tuxedo_io` reste autorisée sous PPD (PPD ne gère pas ces profils), avec audit `source=io` et `ppd=…` (`hw.rs:1010-1040`). Décision explicite ; test `odm_io_fallback_is_allowed_with_ppd_owned`. |
| **AH-03** | **Fermé** | `tccd` propage `Settings.ppd_override` (`main.rs:84-94,128-135,459-510`) ; tests `hardware_inputs_follow_ppd_override_setting`, `hardware_runtime_reloads_ppd_override_flag`. L'ancienne référence `main.rs:114,443` était obsolète. |
| **AH-04** | **Fermé** | Rollback `platform_profile` vers `previous_platform` + relecture ; `PartialApply("platform_profile")` si la restauration échoue (`hw.rs:955-990`) ; tests `odm_platform_verification_failure_restores_previous`, `odm_platform_rollback_failure_is_partial_apply`. |
| **AH-05** | **Fermé** | `TccdCtgpBounds` lit `Manager1.GetHardwareSnapshotJSON` (timeout 2 s, JSON ≤ 4 Mio) et calcule `max-default`, borné à 200 W ; repli statique (`hw.rs:190-260,685-690`) ; tests `ctgp_dynamic_bound_from_provider_is_used`, `ctgp_static_bound_applies_when_provider_unavailable`. |
| **AH-06** | **Fermé** | Consentement lu **avant** polkit pour `SetOdmProfileOverride` ; refus `PolicyDenied("ppd.override_disabled")` sans dialogue (`hw.rs:895-910`) ; test `odm_override_without_consent_skips_polkit`. |
| **AH-07** | **Partiel (Info)** | Refus précoces, `restore=ok|failed` et `polkit` de la voie io sont désormais audités. **Réserve** : `GetTdpValues` et les refus de validation pré-autorisation journalisent encore `polkit=power-limits` alors qu'aucun `CheckAuthorization` n'a eu lieu (`hw.rs:870-880,1100-1140`). Corriger le champ en `polkit="none"` pour ces cas. |
| **AH-08** | **Fermé** | Commentaires unité/README corrigés (« binaire livré, service non validé, non installé par défaut ») ; `MemoryMax=256M` ; `systemd-analyze security` **1.9**. `systemd-analyze verify` échoue encore dans l'arbre source faute de binaire installé (attendu). |
| **AH-09** | **Fermé** | UI branchée et bornes alignées : `odmNameValid` ≤ 64 ASCII graphiques sans `\` (`SettingsPage.qml:287-297`), `tdpCountValid` 1..3 + `tdpDescriptorsValid` impose `pl1/pl2/pl4` sans doublon (`:339-358`), `ctgpValid` `0..HardwareData.ctgpMax` (`:379-382`) ; `ctgpMax` retombe sur 200 W (voir AH-11). `tst_admin_hw` couvre ODM/TDP/cTGP, override et `PolicyDenied`/`PartialApply` ; tests QML à exécuter réellement. |
| **AH-11** | **Info** | `HardwareData.qml:86-88,356-360` ; `hardware_view.rs:279-288` ; `hardware/nvidia.rs:42-60` | `HardwareData.ctgpMax` ne reçoit jamais de borne dynamique : ni `NvidiaData` ni `HardwareView.Nvidia` n'exposent `maxCtgpOffset`, donc `_normalize` met `-1` et `ctgpMax` retombe sur **200 W**. De même, la validation TDP client vérifie l'ordre des **descripteurs** et les plages, mais pas la monotonie des **valeurs** `PL1 ≤ PL2 ≤ PL4` (`SettingsPage.qml:339-367`). | La borne dynamique et la monotonie restent appliquées côté serveur (`TccdCtgpBounds`, `TdpControl::set_values`) ; l'UI peut donc recevoir un refus tardif (`Validation`) pour une valeur qu'elle aurait pu bloquer. Pas d'escalade. | Exposer `maxCtgpOffset` (ou `maxPowerLimit - defaultPowerLimit`) dans `HardwareView`, et vérifier `pl1 <= pl2 <= pl4` côté QML ; tests `tst_admin_hw`. | Test UI : cTGP 200 avec max dynamique 150 ⇒ refus local ; TDP 100/50 ⇒ refus local ; test serveur conservé. |
| **AH-10** | **Accepté / manuel** | `MemoryMax=256M` aligné ; `CapabilityBoundingSet=` vide, `DeviceAllow=/dev/tuxedo_io rw`, `ProtectKernelTunables=no` compensé. Les écritures sysfs et l'activation sans module restent à prouver sur Clevo/Uniwill. |

---

## 2. Conformité vérifiée

### 2.1 Polkit et ordre

| Point | État | Preuve |
|---|---|---|
| Sujet `system-bus-name` du nom unique | **Conforme** | `caller_identity` résout le nom unique puis l'UID (`hw.rs:444-460`) ; `PolkitAuthorizer` construit `AuthorizationRequest::new(caller, action).with_caller_uid(uid)` (`service.rs:198-215`) ; `tcc-authz` envoie `system-bus-name` (`check_authorization_with`). |
| Fail-closed | **Conforme** | `Denied`/`Unavailable`/`Busy` ⇒ `NotAuthorized`/`Busy`, aucun repli root (`hw.rs:489-519`). |
| Pas de court-circuit root | **Conforme** | Aucun test `uid == 0` avant polkit ; `uid` sert seulement à refuser un appelant inconnu. |
| Action `power-limits` | **Conforme** | `ACTION_POWER_LIMITS` (`hw.rs:44`) ; policy `auth_admin`, `allow_active`, sans `_keep` (`packaging/polkit/…policy:68-86`). |
| Pas de cache | **Conforme** | `PolkitAuthorizer` n'a aucun cache ; chaque écriture appelle `CheckAuthorization`. |
| Ordre identité → validation → rate limit → polkit → verrou | **Conforme au nominal** | Méthode D-Bus : `caller_identity` puis `set_tdp`/`set_odm`/`set_ctgp` ; validation syntaxique avant `authorize_write` ; `authorize_write` fait limiteur puis polkit (`hw.rs:463-520`) ; `write_gate.try_acquire` après (`:607-614`). AH-06 sur l'override. |
| Audit | **Partiel** | AH-07. |

### 2.2 TDP

| Point | État | Preuve |
|---|---|---|
| Bornes syntaxiques 1–1000, 1–3 valeurs | **Conforme** | `TDP_MIN_WATTS/TDP_MAX_WATTS` (`hw.rs:70-72`), `MAX_TDP_VALUES` (`tcc-ipc`), `set_tdp` (`:598-606`). |
| Bornes firmware avant écriture | **Conforme** | `TdpControl::set_values` valide entier, `[min,max]` et monotonie **avant** `apply_watts` (`tdp.rs:158-205`). |
| Ordre PL1/PL2/PL4 | **Conforme** | `TDP_DESCRIPTORS` (`tdp.rs:39`) ; `apply_watts` séquentiel ; test. |
| No-op | **Conforme** | Comparaison aux valeurs courantes avant écriture (`hw.rs:633-652`). |
| Vérification après écriture | **Conforme** | `tdp_limits()` après `tdp_set` (`:665-682`). |
| Rollback best-effort | **Conforme** | `rollback_tdp` restaure tous les descripteurs puis relit (`:684-718`). |
| `PartialApply` | **Conforme** | `failed_descriptors` → `PartialApply("pl1,…")` (`:684-718`). |
| Matériel absent | **Conforme** | `limits().is_empty()` ⇒ `NotSupported("tdp")` (`:626-629`). |
| Échec de validation sans écriture | **Conforme** | `set_values` valide tout avant `apply_watts` ; `Validation`/`NotSupported` ne rollback pas (aucune écriture). |

### 2.3 ODM

| Point | État | Preuve |
|---|---|---|
| Allowlist runtime | **Conforme** | `PlatformProfile::choices()` + `TuxedoIo::odm_profiles()` (`hw.rs:788-799`) ; nom hors liste ⇒ `Validation("odm.name")`. |
| PPD `Owned`/`Unknown` ⇒ refus platform | **Conforme** | `ppd.blocks_platform_write()` (`:801-812`) ; test `odm_ppd_owned_refuses_platform_write`. |
| Pas de fallback silencieux platform → io | **Conforme** | Si le nom est dans les deux listes, la voie platform est choisie et PPD bloque avant l'écriture ; test `odm_name_in_both_lists_is_not_silently_routed_to_io`. |
| `Override` exige `Settings.ppd_override` | **Conforme côté Admin.Hw** | `override_consent()` lit `ConfigStore::system()` (`hw.rs:522-527,782-786`) ; `SystemOverrideConsent` (`:365-400`). |
| Override pris en compte par `tccd` | **Non conforme** | AH-03. |
| Race PPD auditée | **Conforme (audit seul)** | Seconde sonde après écriture, `ppd_race` (`:826-841`) ; pas de rollback, conforme design §3.3. |
| Rollback platform sur vérification ratée | **Non conforme** | AH-04. |
| Voie `tuxedo_io` avec PPD actif | **Exception à trancher** | AH-02. |

### 2.4 cTGP

| Point | État | Preuve |
|---|---|---|
| Borne statique 0–200 | **Conforme** | `set_ctgp` (`hw.rs:869-872`) ; `MAX_CTGP_STATIC_OFFSET_WATTS`. |
| Borne dynamique | **Absente** | AH-05 ; `RealCtgpBackend::max_offset` → `None` (`:287-289`). |
| No-op | **Conforme** | `previous == watts` ⇒ `true` sans écriture (`:900-914`). |
| Vérification | **Conforme** | `offset()` après écriture (`:923-937`). |
| Rollback best-effort | **Conforme** | Une tentative de restauration sur échec/vérification (`:915-942`). |

### 2.5 Surface et processus

| Point | État | Preuve |
|---|---|---|
| Aucune commande shell/`nvidia-smi`/`prime-select` | **Conforme** | `hw.rs` n'a aucun `Command::new` ; `RealCtgpBackend` n'utilise pas `NvidiaSmi` ; `Admin.Hw` n'expose pas PRIME. |
| Aucun chemin client | **Conforme** | Entrées `ai`/`i`/`s` uniquement ; aucun chemin/device/nom de commande. |
| Erreurs sans chemin/valeur | **Conforme** | `AdminHwError` : champs fixes, descripteurs `pl1/pl2/pl4` ; `HwError::Io` mappé en `WriteFailed` sans chemin (`map_*`). |
| Audit sans payload | **Conforme (partiel)** | TDP : `count`/descripteurs, jamais les watts ; cTGP : `bound`/`noop`, pas la valeur ; ODM : nom allowlisté ; AH-07 sur les lignes manquantes. |
| Rate limit | **Conforme** | Écritures 2/s burst 4, lectures 2/s burst 5, par nom **et** UID (`AuthorizationLimiter`, `hw.rs:60-68,463-479`). |
| `Semaphore(1)` | **Conforme** | `try_acquire` ⇒ `Busy`, sans file (`:607-614,763-769,874-880`). |
| Sortie 30 s | **Conforme** | `HW_IDLE_TIMEOUT=30s`, `spawn_idle_watch` (`tccd-admin-hw.rs:165-169`). |
| Runtime borné | **Conforme** | 2 workers async + 4 bloqueurs max (`runtime.rs:16-45`), `TasksMax=16`. |
| Root requis | **Conforme** | `is_root()` lit `Uid:` effectif (`tccd-admin-hw.rs:193-210`), code 77 sinon. |
| Sortie après erreur | **Conforme** | `Restart=no`, `RuntimeMaxSec=5min`, `TimeoutStopSec=10`. |

### 2.6 Packaging / unité

| Point | État | Preuve |
|---|---|---|
| `DeviceAllow=/dev/tuxedo_io rw` | **Conforme** | `tccd-admin-hw.service:49-51` ; `DevicePolicy=closed`. |
| `ProtectKernelTunables=no` compensé | **Conforme** | `ReadOnlyPaths=/proc/sys /sys/kernel /sys/module`, CPU, classes, `/etc`, `/var/lib` (`:56-71`) ; les chemins ODM/cTGP restent écrivables. |
| Capacités vides | **Conforme** | `CapabilityBoundingSet=` / `AmbientCapabilities=` (`:101-102`) ; **à prouver** (AH-10). |
| `PrivateDevices=no` | **Conforme** | `:49` avec `DevicePolicy=closed` et un seul device. |
| `TasksMax=16`, `MemoryMax=256M` | **Conforme** | `:98-101` ; runtime 7 threads ; corps D-Bus borné 64 Kio. |
| `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `NoNewPrivileges` | **Conforme** | `:38-45`. |
| `ReadOnlyPaths` des réglages | **Conforme** | `/etc/tuxedo-rust-center`, `/var/lib/tuxedo-rust-center` (`:71`) : consentement lu, jamais écrit. |
| `@system-service` autorise `ioctl` | **Conforme** | Vérifié par `systemd-analyze syscall-filter @system-service` ; `ioctl` présent. |
| `systemd-analyze security` | **Conforme** | Score **1.9** (cible ≤ 3.0). |
| Installation conditionnelle | **Conforme** | `TCC_WITH_ADMIN_HW=1` ; binaire requis sinon `install-tree.sh` échoue ; `TCC_WITH_ADMIN=1` seul n'installe pas Admin.Hw. |
| Docs d'unité/README à jour | **Conforme** | Commentaires/README corrigés (« binaire livré, service non validé ») ; vérifier le buildroot. |

---

## 3. Tests — couverture réelle

**Présents**

- `tcc-admin` : **61 lib** (dont **27 `hw::tests`**), 5 main, 2 binaire `tccd-admin-hw`, 12 `admin_dbus`, **4 `admin_hw_dbus`**, 4 CLI, 12 transaction = **100/100**.
- `hw::tests` couvrent : refus d'arité/valeurs avant backend, TDP absent, no-op, application/vérification, rollback, `PartialApply`, PPD `Owned` (platform refusé), override sans consentement (sans polkit), `Unknown` + override refusé, nom dans les deux listes, nom inconnu, cTGP bornes/no-op/absent, borne dynamique, rollback ODM, refus polkit → aucun backend, `Busy`, rate limit écriture/lecture, mapping des codes.
- `admin_hw_dbus` : Ping/GetVersion/GetContractVersion/GetTdp/SetTdp, tableau vide → `InvalidArgs`, **payload 65 Kio et 20 000 `i32` rejetés sans tuer le service**, introspection limitée aux méthodes phase 2.
- GUI : **167/167** ; `tst_admin_hw` QML présent (ODM/TDP/cTGP, override, `PolicyDenied`/`PartialApply`), mais **non exécuté ici** ; client `Admin.Hw` (probe, timeouts 180 s/10 s, mapping) compile.

**Non couverts / à prouver**

1. Vrai polkit (autorisé/refusé/annulé/absent), sans cache, sujet `system-bus-name`.
2. `dbus-policy-test.sh` en root : membres, `Properties.*` refusés, phase 1 inchangée.
3. Matériel : `/dev/tuxedo_io` Clevo/Uniwill, PL1/PL2/PL4, rollback réel, sysfs `platform_profile`/`ctgp_offset`, PPD réel.
4. Race PPD, override réellement relu par `tccd`, consentement `SaveSettingsJSON`.
5. Payload D-Bus géant (AH-01), concurrence réelle, timeout/annulation.
6. cTGP dynamique, GPU absent/suspendu, vérification post-écriture.
7. Audit journald complet, absence de valeur/path.
8. GUI : contrôles ODM/TDP/cTGP, `PartialApply`/`PolicyDenied`, confirmation, absence d'appel tray.

---

## 4. Décision GUI — état du branchement

L'UI est désormais **branchée** : `SettingsPage` expose ODM/TDP/cTGP et l'override PPD, avec
`AdminHwData.available/pending`, notices d'erreur (`PolicyDenied`, `PartialApply`, etc.) et
`tst_admin_hw` ; le client Rust applique les timeouts 180 s/10 s et n'effectue **aucun retry
automatique**. Les conditions restantes sont :

1. **Bornes client = contrat** : **conforme** — TDP `tdpCountValid` 1..3 + ordre `pl1/pl2/pl4`,
   ODM `odmNameValid` ≤ 64, cTGP `ctgpValid` `0..HardwareData.ctgpMax` (`SettingsPage.qml:287-297,
   336-382`, `HardwareData.qml:86-88`).
2. **Pas de succès optimiste** : `AdminHwData.succeeded` n'est publié qu'après retour ; `pending`
   désactive les contrôles ; un `Timeout` impose une relecture, sans second appel.
3. **Erreurs distinctes** : `PolicyDenied` (PPD/override), `PartialApply` (« appliqué
   partiellement »), `NotSupported`, `NotAuthorized`, `RateLimited`, `Busy`, `Timeout`,
   `AdminUnavailable`, `Contract` — mapping présent dans `AdminHwData.qml`.
4. **Transitoire** : la note `settings.capabilitiesNote` doit rester affichée ; pas de persistance
   implicite.
5. **Override** : bouton séparé, `force=true` seulement si PPD propriétaire **et**
   `ppd_override` vrai ; la GUI ne l'envoie pas sinon (`submitOdm`).
6. **Sonde** : `GetContractVersion`/`Ping` sans activation au démarrage ; `AdminHwData.available`
   conditionne les contrôles.
7. **Confirmation** : le dialogue bloquant n'est pas encore systématique pour TDP/cTGP ; à décider
   côté UX (le service demande déjà polkit). Ne pas l'ajouter dans le tray.
8. **Tests** : `tst_admin_hw` présent ; exécution QML réelle à faire, notamment refus `PartialApply`
   et `PolicyDenied`, et absence d'appel si `adminHwAvailable=false`.
9. **Activation** : côté code, les conditions sont levées ; la validation matérielle reste
   indispensable (voir §5).

---

## 5. Risques résiduels et limites manuelles

- **AH-01** : fermé ; la borne 64 Kio + `max_queued(8)` + `MemoryMax=256M` restent à éprouver sur
  bus système réel (payload géant QA).
- **AH-02** : décision documentée (platform_profile refusé, io-only autorisé) ; dépend de la
  politique udev/PPD de la distribution.
- **AH-03** : câblage `ppd_override` fermé ; la persistance réelle (Admin1 → `/var/lib` → `tccd`)
  reste à tester sur machine.
- **AH-04/AH-05** : rollback ODM et borne cTGP dynamique fermés côté code ; vérification matérielle
  (restauration réelle, offset GPU) à faire.
- **AH-07** : Info — libellé `polkit` de `GetTdpValues` et des refus pré-autorisation.
- **AH-09** : fermé — bornes UI alignées (TDP 1..3 + descripteurs, ODM 64, cTGP 0..max).
- **AH-11** : Info — `ctgpMax` reste 200 (borne dynamique non exposée) et monotonie TDP valeurs
  vérifiée côté serveur seulement.
- **AH-10** : capacités vides, `DeviceAllow` et sysfs à prouver sur Clevo/Uniwill.

**Limites manuelles** : vrai polkit/system bus (autorisé/refusé/annulé/absent, sans cache) ;
`dbus-policy-test.sh` root ; `/dev/tuxedo_io` réel (TDP PL1/PL2/PL4, rollback partiel, `DeviceAllow`) ;
`platform_profile` et `ctgp_offset` réels ; PPD réel (`Owned`/`Activatable`/`Absent`, race) ;
`Settings.ppd_override` écrit par `Admin1` puis relu par `tccd` ; payload 65 Kio/20k `i32` sur bus
système ; audit journald ; builds RPM/Arch (et deb) + installation/désinstallation propres ;
`systemd-analyze verify/security` sur buildroot.

---

## 6. Verdict et actions bloquantes

**Verdict** : **aucun finding code non-Info ouvert**. L'architecture `Admin.Hw` est saine (service
séparé root à la demande, polkit fail-closed, entrées bornées, aucune commande/path, transaction et
audit). **L'activation `TCC_WITH_ADMIN_HW=1` est possible côté code**, avec AH-02 accepté et
AH-07/AH-11 en Info. Aucune action bloquante **code** ne reste.

**Preuves manuelles restantes (bloquantes pour l'activation)** :

1. Vrai polkit root : autorisé/refusé/annulé/absent, deux appels ⇒ deux dialogues, sujet
   `system-bus-name`.
2. `dbus-policy-test.sh` en root : `Admin.Hw` joignable, membres listés, `Properties.*` refusés,
   phase 1 inchangée.
3. `/dev/tuxedo_io` Clevo **et** Uniwill : TDP 1–3 valeurs, PL1/PL2/PL4, rollback et
   `PartialApply`, `DeviceAllow` avec capacités vides.
4. `platform_profile`/`ctgp_offset` réels : ODM platform + io, PPD `Owned`/`Activatable`/`Absent`,
   race PPD, cTGP offset dynamique et rollback.
5. `Settings.ppd_override` : écriture via `Admin1`, relecture par `Admin.Hw` et `tccd`,
   réapplication cohérente.
6. Payload géant QA (65 Kio/20k `i32`) sur bus système réel et service vivant.
7. Builds RPM/Arch (et deb), installation/désinstallation, `systemd-analyze verify/security`,
   audit journald.

**Non bloquantes** : AH-07 (libellé `polkit` de `GetTdpValues`), AH-11 (borne cTGP UI statique
et monotonie TDP côté client), confirmation UX TDP/cTGP, exécution des tests QML.
