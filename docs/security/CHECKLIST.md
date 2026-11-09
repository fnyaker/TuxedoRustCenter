# Checklist de sécurité — TuxedoRustCenter

Compagnon opérationnel de `docs/security/THREAT_MODEL.md`.
Chaque case doit être cochée avec une **preuve** (test, commande, sortie archivée), pas seulement une intention.

Le projet compte désormais **9 crates** (`tcc-core`, `tcc-ipc`, `tcc-hw`, `tcc-daemon`, `tcc-admin`, `tcc-authz`, `tcc-user`, `tcc-gui`, `tcc-cli`). Les sections 0 à 11 restent la
**checklist de design** ; l'état réellement vérifié au 2026-11-06 est en §0bis. Une case de design
non cochée n'est **pas** fermée, et un point `[~]` ci-dessous ne doit pas être présenté comme prouvé
(vrai polkit, bus système, matériel réel).

**Légende d'état** : `[x]` preuve locale/CI ; `[~]` implémenté mais preuve système/manuelle
manquante ; `[ ]` non fait ou non prouvé.

**Légende**

| Tag | Signification | Conséquence |
|---|---|---|
| **B** | **Bloquant** | Aucune release, aucun paquet, aucun merge sur `main` tant que ce n'est pas fait |
| **I** | **Important** | À faire avant la release ; peut être différé d'un jalon avec justification écrite |
| **A** | **Amélioration** | Durcissement souhaitable, backlog |

**Convention de preuve** : `[x] (preuve: <test|commande|fichier>)`.

---

## 0bis. État au 2026-11-06

> Instantané vérifié par lecture du code, des units et des revues `docs/security/REVIEW_*.md`, avec
> les tests exécutés localement. Les éléments **non rejoués** (vrai polkit, bus système, matériel
> réel, veille réelle) restent `[~]` même si le code est présent.

Cette section remplace les mentions obsolètes : **7 crates → 9** ; « daemon minimal » → `Admin1`
phase 1, `tcc-authz`, setters `Manager1` autorisés, veille `PrepareForSleep`, design V4L2 ;
« polkit non implémenté » → `tcc-authz` + polkit admin/setters (avec les réserves M1/SL).

| Domaine | État | Preuve / réserve |
|---|---|---|
| **Workspace** | **[x]** | 9 crates dans `crates/` ; workspace Cargo, `Cargo.lock` versionné. |
| **Config D2** | **[x]** | État mutable dans `/var/lib/tuxedo-rust-center`, `0644 root:root`, écriture atomique, `.bak` `0600` (`tcc-core/src/config.rs:1065`) ; paire `profiles.json`+`settings.json` non atomique mais rollback des deux, y compris suppression d'un fichier créé (`ADMIN_DESIGN.md` §4.2.1, T1-01 fermé) ; `load_settings_no_recovery`/`load_profiles_no_recovery` et `load_profiles_state_only_no_recovery` (`:551-617`) ; tests `backup_save_writes_0644_and_previous_content_0600`, `corrupt_previous_is_not_restored_after_reload_failure`, `etc_profile_is_not_tombstoned_by_state_diff`. Migration `/etc/tcc` : `[~]`, pas de preuve sur installation réelle. |
| **`Admin1` phase 1** | **[~]** | `SaveProfilesJSON`/`SaveSettingsJSON`+`Control1` implémentés ; **tombstones d'IDs** (`deletedProfileIds` ≤ 1024, refus de recréation, transaction double fichier profils+settings, `SaveSettingsJSON` append-only, diff d'état seul, rollback d'un settings créé) ; tests `tcc-admin` **33 lib + 5 binaire + 12 `admin_dbus` + 12 `transaction` + 4 `cli`** ; findings A1/G1/S1/**T1** documentés (**T1-01 et AD-R13 fermés** ; T1-02…T1-04 Info) ; vrai polkit et `Control1` réel non rejoués. |
| **`Admin.Hw` phase 2** | **[~]** | `AdminHw1` (TDP/ODM/cTGP) implémenté : enveloppe **64 Kio** + `max_queued(8)`, `MemoryMax=256M`, PPD/override, cTGP dynamique via `Manager1`, rollback ODM/`PartialApply`, audit ; tests `tcc-admin` **61 lib + 5 main + 2 hw + 12 `admin_dbus` + 4 `admin_hw_dbus` + 4 CLI + 12 transaction** ; `systemd-analyze` **1.9** ; **AH-01/03/04/05/06/08/09 fermés**, AH-02 accepté, **AH-07/AH-11 Info** (libellé polkit `GetTdpValues` ; borne cTGP UI statique/monotonie TDP client) ; vrai polkit, `/dev/tuxedo_io`, PPD et matériel non rejoués. |
| **Polkit admin** | **[~]** | `tcc-authz` : sujet `system-bus-name`, flags `1`, aucun court-circuit root, fail-closed (`19` tests) ; `systemd-analyze security` admin **1.6** ; **vrai polkit non exécuté**. |
| **Polkit setters `tccd`** | **[~]** | `prepare_write` → `authorize_setter` (`dbus.rs:539-572`), `PolkitSettersAuthorizer` fail-closed ; **152/152** tests daemon. **Policy en `deny`** (`com.tuxedocomputers.tccd.conf` §4bis, 8 membres refusés) ; **M1-01…M1-05 fermés** (`REVIEW_MANAGER1_AUTHZ.md`). Réouverture conditionnée aux preuves système (vrai polkit, root, `dbus-policy-test.sh`, matériel). Root et vrai polkit non testés. |
| **Rate limiting** | **[~]** | Admin : nom unique **+ UID** (`tcc-authz::AuthorizationLimiter`, 2/s burst 4) ; `tccd` : **nom unique seulement** (5/s burst 10, `rate_limit.rs:14-22`) → **M1-01**. |
| **`Control1`** | **[~]** | Interface root-only, UID 0 revérifié, rechargement réel sans cache (A1-01 fermé, tests daemon) ; policy root-only ; vrai bus système non rejoué. |
| **Veille/reprise** | **[~]** | `PrepareForSleep` implémenté (`sleep.rs`), **15/15** tests sleep, `FanWorker::suspend`, workers recréés, bail capteurs `disarm`, profil réappliqué (`REVIEW_SLEEP.md`). **SL-01…SL-05, SL-07, SL-09 fermés ; SL-06/SL-08 Info** ; `systemctl suspend` réel et matériel non testés. |
| **V4L2 webcam** | **[~]** | **Lecture validée sur matériel réel + écriture UVC de session livrée** (`crates/tcc-hw/src/v4l2/**` : 5 ioctl, allowlist **15** CIDs, `unsafe` confiné, `O_RDONLY`/`O_RDWR`) ; `User1.SetUvcControl` sur le **bus de session** (`tccd-user`), device énuméré, identité driver/card/bus_info, porte globale, rate limit **20/s/UID**, corps D-Bus **64 Kio**, cache 1 s, erreurs typées, aucun polkit ; `tccd` reste `DeviceAllow=char-video4linux r` (`tccd.service:69`) ; preuve lecture `/dev/video0` `0666`+ACL `uaccess`, **11 contrôles**, `video2` IR 0 ; **UVCW-02…07 fermés**, **UVCW-08 fonctionnellement fermé** (commentaire résiduel), **UVCW-01 accepté comme dette documentée** ; tests `tcc-hw` **284/284** (`v4l2` **62/62**, fake **8/8**, garde **4/4**), `tcc-user` **169/169** (uvc **12/12**, rate_limit **6/6**, contract **9/9**), workspace **1527/1527** ; écriture réelle, ACL, deux sessions, `EBUSY` réel non testés. |
| **Policy D-Bus** | **[~]** | Membres `Admin1`/`Manager1` listés un par un, `Properties` absent, `Control1` root-only ; **42/42** annoncé par `dbus-policy-test.sh` (non rejoué en root) ; **setters `Manager1` en `deny`** (vérifié 2026-11-06) ; aucune action/méthode LUKS. |
| **Packaging** | **[~]** | FHS, aucun `/opt`, units `tccd`/`tccd-user`/`tccd-admin`, `tccd-admin` **non installé par défaut** (`TCC_WITH_ADMIN=0`) ; scores `systemd-analyze security` **3.3 (`tccd`) / 1.6 (`tccd-admin`)** à rejouer ; `PrivateDevices=no` + `DeviceAllow=/dev/tuxedo_io rw` + `char-video4linux r` (`tccd.service:66,69`) ; validation matérielle H-13 non faite. |
| **GUI** | **[x]** | `cargo test -p tcc-gui --bins` **90/90** ; **125 tests QML** locaux ; aucune écriture directe ; dernière validation Qt 6.10 à faire en CI. |

**Ne pas marquer fermé** : vrai polkit, bus système en root, `Control1` réel, suspend/reprise réel,
matériel Clevo/Uniwill, findings SL/M1/V non corrigés et T1-02…T1-04 (Info). **T1-01 et AD-R13 sont
fermés** (tests dédiés).

---

## 0. Portes de design (avant d'écrire du code privilégié)

- [ ] **B** — Le modèle de privilèges est écrit noir sur blanc : `tccd` root minimal, `tccd-admin` séparé (décision D1), `tccd-user` non privilégié, `tcc-gui` non privilégié, **aucun setuid**.
- [ ] **B** — La localisation des données est tranchée (décision D2) : état mutable hors de `/etc`, `/etc` en lecture seule pour le daemon.
- [ ] **B** — Toute méthode D-Bus est classée : `Manager1` (lecture / profil temporaire) ou `Admin1` (sensible, polkit). Aucune méthode « non classée ».
- [ ] **B** — Chaque action polkit est listée avec son niveau (`auth_admin` vs `auth_admin_keep`) et son interface.
- [ ] **B** — Chaque chemin sysfs utilisé est documenté (fichier, attribut, plage, source) dans `docs/` ou dans les doc-comments de `tcc-hw`.
- [ ] **B** — Chaque commande externe est dans l'allowlist avec son chemin absolu et la validation de ses arguments.
- [-] **Hors périmètre** — Le transport des secrets LUKS (D5) reste une règle historique pour tout futur secret ; aucune fonctionnalité LUKS n'est livrée.
- [ ] **I** — Les env overrides de développement sont confinés à une feature Cargo non release (décision D7).
- [ ] **I** — Le comportement en cas de PPD/UPower/compositeur absent est spécifié (état dégradé, pas d'échec de démarrage).
- [ ] **A** — Le prérequis de sécurité plateforme (Secure Boot/IMA) est documenté.

**Preuve** : revue de `docs/security/THREAT_MODEL.md` §9 + `docs/ARCHITECTURE.md` §3.2/§3.10, signée `linux-platform` + `security`.

---

## 1. `tcc-core` — modèles, validation, migration

- [ ] **B** — `#![forbid(unsafe_code)]`, `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]` (adapter si trop strict, mais `unwrap`/`expect` interdits).
- [ ] **B** — `validate_profile` / `validate_settings` centralisés, retournant des erreurs typées (`thiserror`), **sans clamp silencieux**.
- [ ] **B** — Bornes : taille de fichier, nombre de profils (≤ 256), points de courbe (≤ 64), longueurs de chaînes, profondeur JSON.
- [ ] **B** — Rejet des `\0` et caractères de contrôle dans tous les champs texte ; `id` conforme à `^[A-Za-z0-9_-]{1,64}$`.
- [ ] **B** — Invariants croisés : `charge_start < charge_end`, `fan_min ≤ fan_max`, `PL1 ≤ PL2`, `scaling_min ≤ scaling_max`, `refresh_rate.is_finite()` et borné.
- [ ] **B** — Aucun `serde(other)` sur un champ de sécurité ; `deny_unknown_fields` évalué champ par champ.
- [ ] **B** — Migration `/etc/tcc` → nouveau format : lecture tolérante, sauvegarde `.bak`, **jamais** d'écrasement silencieux, idempotente.
- [ ] **B** — Persistance : écriture atomique (`O_TMPFILE`/tmp + `fsync` + `renameat` + `fsync` dir), permissions finales `0644 root:root`, vérification propriétaire/mode à la lecture.
- [ ] **B** — Tests : corpus valide/invalide, `proptest` « jamais de panique », test d'atomicité avec symlink et répertoire 0777.
- [ ] **I** — Aucune donnée de profil n'entre dans un chemin de fichier (l'`id` n'est jamais un nom de fichier).
- [ ] **A** — `cargo fuzz` sur `parse_profile`, `parse_settings`, `fan_curve`, `migrate_legacy`.

**Preuve** : `cargo test -p tcc-core`, rapport de fuzzing court archivé, `docs/security/` mise à jour avec les bornes retenues.

---

## 2. `tcc-hw` — accès matériel

- [ ] **B** — `SysFsRoot` injectable ; **aucun** chemin sysfs ne contient de chaîne issue d'une entrée utilisateur (constantes ou gabarits + index entier).
- [ ] **B** — Ouverture : `openat2` avec `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV` (ou `openat`+`O_NOFOLLOW|O_CLOEXEC` composant par composant) ; vérification post-ouverture via `/proc/self/fd/N` (préfixe `/sys/`).
- [ ] **B** — Chaque champ a des bornes documentées avec leur source (sysfs `min`/`max`, ABI, datasheet) ; test min-1/min/max/max+1.
- [ ] **B** — Capacité non détectée ⇒ `Unsupported`, jamais une écriture tentée.
- [ ] **B** — `unsafe` uniquement dans des wrappers ioctl isolés, avec commentaire `// SAFETY:` **et** un test ; `#![deny(unsafe_op_in_unsafe_fn)]`.
- [ ] **B** — `ioctl` : tailles de structures vérifiées, index bornés, pointeurs non nuls, retours négatifs traduits en erreurs typées.
- [ ] **B** — Aucune lecture de `/proc/*/environ`, aucune lecture croisée de processus.
- [ ] **B** — Détection d'erreur d'écriture sysfs propagée (pas de `Ok(())` sur échec).
- [ ] **I** — Un test à base de sysfs factice échoue si une écriture hors plage est tentée (journal des écritures).
- [ ] **I** — Les valeurs lues sont considérées comme non fiables (parsing tolérant, pas de `unwrap`).
- [ ] **A** — `cargo geiger`/`cargo deny` + revue `unsafe` listée dans `docs/security/UNSAFE.md`.

**Preuve** : `cargo test -p tcc-hw`, `grep -rn "unsafe" crates/tcc-hw` accompagné d'une justification par occurrence.

---

## 3. `tcc-daemon` — surface D-Bus et privilèges

### 3.1 Service et cycle de vie

- [ ] **B** — Refus de démarrer si le nom est déjà pris (`request_name` avec `DO_NOT_QUEUE`) ; `NameLost` ⇒ arrêt propre.
- [ ] **B** — Démarrage tolérant : PPD, UPower, `tuxedo_io`, agent de session absents ⇒ démarrage réussi, état dégradé exposé.
- [ ] **B** — `StartLimitBurst` respecté (test : 10 échecs ⇒ unit `failed`, pas de boucle).
- [~] **B** — `Type=dbus` (état actuel, `tccd.service:19`) ; `sd_notify`/`WatchdogSec` **non implémenté** — à décider ou à documenter comme non nécessaire ; pas de PID file maison (état actuel).
- [ ] **I** — Écriture d'un état lisible (`/run/tuxedo-rust-center/status.json`, 0644) pour l'UI et le diagnostic, sans secret.

### 3.2 Autorisation

- [~] **B** — Chaque méthode d'écriture `Admin1` **et chaque setter `Manager1`** appelle `CheckAuthorization` avec un sujet **`system-bus-name` = nom unique de l'appelant** (`Admin1` : `tcc-admin` ; setters : `tccd` `authorize_setter`). Vrai polkit non rejoué.
- [ ] **B** — Test : sujet `unix-process` fourni par le client ⇒ ignoré ; PID du daemon ⇒ jamais utilisé.
- [ ] **B** — **Aucun cache local d'autorisation** (AD-4) : seul le cache polkit décidé par l'administrateur système existe ; vérifier qu'aucune implémentation n'en ajoute (le code actuel n'en a pas).
- [ ] **B** — Aucune action polkit n'utilise `exec.path` / `allow_gui` (test : parsing des fichiers `.policy`).
- [ ] **B** — Test matriciel méthode × rôle : root (**polkit décide, pas de court-circuit** : autorisé ou refusé selon la politique), utilisateur actif (polkit), utilisateur inactif (refus), nom inconnu (refus).
- [ ] **B** — Un test énumère les méthodes via l'introspection : toute méthode `Set*`/`Apply*` non présente dans la table de classification fait **échouer** le test.

### 3.3 Validation et robustesse

- [ ] **B** — Aucun `unwrap`/`expect`/`panic!`/`todo!` dans le code de production ; vérifié par `clippy` + `grep` CI.
- [ ] **B** — Taille max par message (1 Mio JSON, 64 Kio typé), taille max par chaîne, profondeur JSON bornée.
- [~] **B** — Rate limiting **par nom unique + UID** pour l'admin (`tcc-authz`, 2/s burst 4) et **par nom unique** pour `tccd` (5/s burst 10) ; plafond de handlers concurrents ; dépassement ⇒ `LimitsExceeded`. Le bucket UID de `tccd` manque (**M1-01**), à corriger avant d'ouvrir les setters au bus.
- [ ] **B** — Test de rafale : 10 000 appels ⇒ majorité rejetée, daemon réactif ensuite, RSS stable.
- [ ] **B** — Collecte capteurs comptée par client, arrêt sur déconnexion et expiration.
- [ ] **B** — Aucune méthode ne renvoie d'objet `serde_json::Value` brut dans une signature D-Bus.
- [ ] **I** — Réponses d'erreur homogènes, sans détail interne (pas de chemin de fichier, pas de stack).

### 3.4 Fichiers et chemins

- [ ] **B** — Aucune écriture hors de `ReadWritePaths` ; test avec `strace` montrant l'absence d'`open` hors des répertoires autorisés.
- [ ] **B** — Vérification propriétaire/mode/type à chaque lecture de config ; fichier non conforme ⇒ refus + log d'audit.
- [ ] **B** — Aucune opération sur un chemin fourni par l'appelant (test : payload contenant `/etc/passwd`, `..`, NUL ⇒ rejet).
- [ ] **I** — Écriture atomique + `fsync` ; test de coupure simulée.

### 3.5 Commandes externes

- [ ] **B** — Aucun `sh -c`, `bash -c`, `Command::new` avec chemin non absolu, `env` non nettoyé.
- [ ] **B** — Allowlist centralisée (module unique) ; `clippy.toml` interdit `Command::new` ailleurs.
- [ ] **B** — `env_clear()` + environnement minimal fixe ; test qui vérifie `/proc/<pid>/environ` de l'enfant.
- [ ] **B** — Timeout + `kill` + sortie plafonnée sur chaque commande.
- [ ] **B** — Test : binaire malveillant homonyme dans un répertoire du PATH utilisateur ⇒ jamais exécuté.
- [ ] **B** — Aucun contenu téléchargé n'est exécuté (test : absence de `https`/`http` dans le code du daemon).

### 3.6 LUKS — hors périmètre

- [-] **Hors périmètre (décision utilisateur), non implémenté** : aucune action polkit, aucune méthode
  D-Bus, aucun service, aucun transport de secret, aucun `cryptsetup`. D5 reste la règle historique
  pour tout futur secret ; les cases LUKS ne sont ni à cocher ni à implémenter.

### 3.7 Logs

- [ ] **B** — Aucun secret, aucun environnement, aucun payload brut dans `tracing`.
- [ ] **B** — Test automatique : subscriber qui échoue si un motif interdit apparaît (`pass`, `KEY=`, `TOKEN=`, `/home/`).
- [ ] **B** — `StandardOutput=journal`, aucun fichier de log applicatif.
- [ ] **I** — Journal d'audit pour les actions sensibles (`action`, `caller`, `result`), sans valeur sensible.

**Preuve** : `cargo test -p tcc-daemon`, sortie de `busctl`/`dbus-send` en utilisateur non privilégié, rapport `systemd-analyze security`.

---

## 4. `tcc-user` — agent de session

- [ ] **B** — Refuse de tourner si `geteuid() == 0` (test dédié).
- [ ] **B** — N'expose **aucune** méthode entrante sur le bus de session (ou méthode typée sans nom d'output ni commande) ; test d'introspection vide.
- [ ] **B** — Filtre d'output interne : `^(eDP|LVDS|DSI)-[0-9]+$` + recoupement DRM ; tests de rejet (`HDMI-1`, `DP-1`, `eDP-1/../x`, vide, NUL).
- [ ] **B** — Taux validé : fini, `1.0..=1000.0`, présent dans les modes annoncés, tolérance 0,5 Hz ; sinon aucune application.
- [ ] **B** — Aucune écriture dans la configuration persistante du compositeur (GNOME `method=1`, pas de sauvegarde KDE/Hyprland).
- [ ] **B** — Vérification du socket compositeur : dans `$XDG_RUNTIME_DIR`, propriétaire = uid, mode 0700, pas de symlink.
- [ ] **B** — Ne s'applique pas si la session est verrouillée/inactive ; réessai au signal suivant.
- [ ] **B** — Idempotence : mode déjà correct ⇒ zéro appel au compositeur (compteur de test).
- [ ] **B** — Écran externe : configuration strictement inchangée avant/après (test avec 2 outputs).
- [ ] **I** — Une seule application en vol par output, anti-rebond, pas de blocage du daemon si l'agent est absent.
- [ ] **A** — Backend non supporté ⇒ état « non supporté » explicite dans l'UI, aucun fallback dangereux.

**Preuve** : `cargo test -p tcc-user`, test d'intégration avec un faux compositeur (Hyprland/Sway mockés).

---

## 5. `tcc-gui` — client non privilégié

- [ ] **B** — Aucune écriture dans `/etc`, `/var/lib`, `/sys` ; aucun appel `Admin1` sans passer par le dialogue polkit.
- [ ] **B** — Aucune donnée de la GUI n'est utilisée pour construire un chemin de fichier privilégié.
- [ ] **B** — Import/export : validation côté client **et** côté daemon (le client n'est jamais la seule barrière).
- [-] **Hors périmètre** — Secrets LUKS : aucune fonctionnalité, aucun secret traité par la GUI.
- [~] **B** — **Tray** : aucune logique privilégiée ; actions via `AppState`/D-Bus (profils temporaires, blocker, autostart) ; Fn-Lock/mode graphique en lecture seule ; fermeture masque si tray disponible, quitte sinon ; repli fenêtre sinon ; aucune commande shell, aucun secret (`REVIEW_TRAY.md`, T2-01/T2-06/T2-07/T2-08). **SNI réel/GNOME et tests QML non rejoués.**
- [~] **B** — **Autostart** : `$XDG_CONFIG_HOME` sous `$HOME` (ou `.config`), refus `..`/symlink direct, répertoire `0700` avec vérification uid/mode, fichier `0644` avec `Exec` fixe et atomique, aucun shell (`REVIEW_TRAY.md`, T2-02/T2-04/T2-05 ; T2-03 accepté). **Login réel non testé.**
- [ ] **B** — La GUI traite toute réponse du daemon comme non fiable (affichage échappé, pas d'évaluation).
- [ ] **B** — Aucun composant web (vérification de la liste des dépendances et des imports Qt).
- [~] **B** — Écritures GUI : `SettingsEdit` fermé (`deny_unknown_fields`, 64 Kio/1 Mio, `extra` et tombstones préservés), validation charge/Fn-Lock/lighting/matrice/résolution, envoi par `Admin1`/`Admin.Hw` ; aucun setter `Manager1` charge/lighting/Fn-Lock. Exceptions : `SetTempProfile`/`ClearTempProfile` et `SetSensorDataCollectionStatus` (rate-limités, non matériels). **UIW-01** (brightness non bornée au max matériel) et **UIW-03** (masquage d’erreur) ouverts ; `REVIEW_UI_WRITES.md`.
- [~] **B** — `profile-hardware-writes` (ODM/TDP/cTGP par profil) : `ProfileEdit` fermé (ODM ≤ 64 syntaxe, TDP ≤ 3 valeurs 1..1000 strictement croissantes ou liste vide, cTGP 0..200), sauvegarde `Admin1.SaveProfilesJSON` (polkit `profiles`), application `tccd` validée firmware avant écriture, `platform_profile` refusé si PPD. **PHW-02/PHW-03 corrigés** (ODM absent/`None` sans écriture, borne cTGP dynamique/statique exposée et refus hors borne, profil `tdpValues=[]` = **no-op**, `Admin.Hw.SetTdpValues([])` = **maxima explicite** ; tests `odm` 18/18, `nvidia` 9/9, `admin tdp` 9/9, `admin_hw_dbus` 5/5) ; **PHW-04 partiel accepté** (core 16 vs hw 3, aucune écriture ; résumé import recommandé) ; **PHW-05 Info** (commentaire/validation locale) ; **PHW-01** accepté mono / ouvert multi ; `REVIEW_PROFILE_HW_WRITES.md`. Matériel réel non testé.
- [ ] **I** — Messages d'erreur honnêtes : « non supporté » ≠ « erreur », pas de faux succès.
- [ ] **A** — Affichage du résultat des opérations sensibles (Prime, installation de paquets) avec état par étape.

**Preuve** : revue de dépendances, test d'intégration avec un daemon mocké qui renvoie des réponses hostiles.

---

## 6. Intégration système et packaging

### 6.1 systemd

- [ ] **B** — `tccd.service` : `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `ProtectProc=invisible`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK`, `CapabilityBoundingSet=` vide, `SecureBits=noroot-locked`, `LimitCORE=0`.
- [ ] **B** — Exception matérielle documentée : `ProtectKernelTunables=no` (sinon `/sys` RO ⇒ plus aucune écriture matérielle).
- [~] **B** — `PrivateDevices` : l'unité **actuelle** utilise `PrivateDevices=no`, `DevicePolicy=closed`, `DeviceAllow=/dev/tuxedo_io rw` (`tccd.service:64-66`) ; l'option `PrivateDevices=yes` + `BindPaths=-/dev/tuxedo_io` + `DeviceAllow=char-tuxedo_io rw` **n'est pas implémentée** et reste une alternative à valider (H-13).
- [ ] **B** — `StateDirectory`/`RuntimeDirectory`/`CacheDirectory` utilisés ; modes restrictifs.
- [ ] **B** — Aucun `User=` non-root pour `tccd` (il doit être root) ; aucune unit ne lance `tccd-user` en root.
- [ ] **B** — `systemd-analyze verify` passe ; `systemd-analyze security tccd.service` exécuté, score archivé et justifié.
- [ ] **B** — `tccd-user.service` est une unit **utilisateur** (`/usr/lib/systemd/user/`), `PartOf=graphical-session.target`, sans capacité.
- [~] **B** — Veille : gérée **dans `tccd`** via `PrepareForSleep` (`sleep.rs`), plus de `tccd-sleep.service` ; `systemctl suspend` réel et validation Clevo/Uniwill à faire (`REVIEW_SLEEP.md`, SL-01…SL-05).
- [ ] **I** — `CPUQuota`, `MemoryMax`, `TasksMax` cohérents avec les budgets de performance.

### 6.2 D-Bus

- [ ] **B** — Politique installée dans `/usr/share/dbus-1/system.d/`, `<deny own>` pour tous sauf root, interfaces filtrées.
- [ ] **B** — Test en utilisateur non privilégié : `busctl introspect` OK, `busctl call` sur `Admin1` ⇒ refus explicite.
- [ ] **B** — Aucune méthode d'écriture de fichier arbitraire exposée (test d'introspection + revue).
- [ ] **B** — Le daemon re-vérifie l'autorisation (défense en profondeur) même si la politique D-Bus autorise.
- [ ] **I** — Signal `ActiveProfileChanged` consommable par l'agent sans privilège.

### 6.3 Polkit

- [ ] **B** — Fichier dans `/usr/share/polkit-1/actions/`, une action par opération, `allow_any=no`, `allow_inactive=no`.
- [ ] **B** — Aucun `exec.path`, aucun `allow_gui`.
- [ ] **B** — `pkcheck` vérifié pour chaque action (autorisé / refusé).
- [ ] **I** — Traductions FR/EN/DE des `description`/`message`.

### 6.4 udev

- [ ] **B** — Aucune règle `RUN+=` avec interpréteur (Python, shell) ; utiliser un tag systemd ou netlink dans `tccd`.
- [ ] **B** — Aucune interpolation de `$env{…}`/`$devnode` dans une ligne de commande shell.
- [ ] **I** — `udevadm test` archivé.

### 6.5 Fichiers et permissions

- [ ] **B** — Aucun fichier dans `/opt` (`grep -r "/opt" packaging/` vide).
- [ ] **B** — Aucun fichier world-writable : `find "$pkgroot" -perm -o+w` ⇒ vide.
- [ ] **B** — Aucun setuid/setgid : `find "$pkgroot" -perm -4000 -o -perm -2000` ⇒ vide.
- [ ] **B** — Aucune capability de fichier : `getcap -r "$pkgroot"` ⇒ vide.
- [ ] **B** — Binaires `0755 root:root`, config `0644 root:root`, répertoires d'état `0750 root:root`, `/etc` non modifiable par le daemon.
- [ ] **B** — Scripts de post-installation : idempotents, sans `curl | sh`, sans écriture hors FHS, sans démarrage de service dans un contexte utilisateur.
- [ ] **I** — `conffiles` (Debian) pour `/etc/tuxedo-rust-center/*` ; comportement de désinstallation documenté (config conservée).
- [ ] **A** — Profil AppArmor/SELinux pour `tccd` (Ubuntu/Fedora).

### 6.6 Journalisation et diagnostic

- [ ] **B** — Journald uniquement ; aucun `/var/log`.
- [ ] **B** — Diagnostic : aperçu obligatoire, consentement explicite, aucune exécution en root, aucun contenu téléchargé exécuté.
- [ ] **B** — Aucune donnée sensible collectée par défaut ; envoi manuel.
- [ ] **I** — Commande `tcc-cli diagnose --dry-run` qui liste ce qui serait collecté.

**Preuve** : arborescence de paquet inspectée (`dpkg -c`, `rpm -qpl`, `bsdtar -tf`), sorties archivées dans la PR de release.

---

## 7. CI et chaîne d'approvisionnement

- [ ] **B** — `Cargo.lock` versionné ; `cargo build --locked` / `cargo test --locked` en CI.
- [ ] **B** — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` bloquants.
- [ ] **B** — `cargo deny check` avec `deny.toml` : `unknown-git = "deny"`, `wildcards = "deny"`, `yanked = "deny"`, licences compatibles GPL-3.0-or-later, `multiple-versions = "warn"`.
- [ ] **B** — `cargo audit` bloquant (advisories) + job quotidien.
- [ ] **B** — Aucune dépendance `git` ; aucune dépendance sans version précise ; `Cargo.toml` en `=`/`^` maîtrisé.
- [ ] **B** — Chaque `build.rs` est listé et revu (allowlist dans `docs/security/SUPPLY_CHAIN.md`).
- [ ] **B** — `#![forbid(unsafe_code)]` dans `tcc-core`, `tcc-ipc`, `tcc-gui`, `tcc-cli` ; `unsafe` de `tcc-hw` documenté et testé.
- [ ] **B** — `clippy.toml` : `disallowed-methods` pour `std::process::Command::new` (hors module `exec`), `std::fs::write`, `std::fs::File::create`, `std::env::var` dans le daemon.
- [ ] **B** — Aucun secret dans le dépôt (scan `gitleaks`/`trufflehog` ou `git grep` de motifs).
- [ ] **B** — SBOM généré et archivé à chaque release.
- [ ] **I** — `cargo fuzz` : smoke 60 s par PR sur `parse_profile` + `dbus_payload` ; campagne longue avant release.
- [ ] **I** — `cargo vet` ou équivalent pour `zbus`, `zvariant`, `serde`, `tokio`, `zeroize`.
- [ ] **A** — Build reproductible vérifié (`SOURCE_DATE_EPOCH`, comparaison de deux builds).

---

## 8. Tests d'entrées invalides (matrice minimale)

À exécuter pour **chaque** méthode D-Bus et **chaque** champ de profil.

| Catégorie | Cas | Attendu |
|---|---|---|
| Taille | message 10 Mio, chaîne 1 Mio, 10⁶ éléments de liste | `LimitsExceeded`, pas d'allocation proportionnelle |
| Profondeur | JSON imbriqué 1000 niveaux | erreur typée, pas de stack overflow |
| Type | `i` au lieu de `u`, `b` au lieu de `s`, `a{sv}` inattendu | erreur de désérialisation propre |
| Nombre | `i32::MIN`, `i32::MAX`, `-1`, `0`, NaN, ±Inf | rejet si hors plage, jamais d'écriture |
| Chaîne | NUL, `\n`, caractères de contrôle, UTF-8 invalide, 128+ chars | rejet |
| Identifiant | `../x`, `/etc/passwd`, `a/b`, vide, 65 chars, doublon | rejet |
| Enum | `governor=evil`, `charge_type=xxx`, `ppd=???` | rejet ou fallback **documenté**, jamais d'écriture |
| Croisé | `start=90, end=20`, `min>max`, `PL1>PL2` | rejet |
| Concurrence | 2 écritures simultanées, 100 appels en rafale | sérialisation, état cohérent |
| Rejeu | même appel 1000× | idempotence ou rate limit |
| Absence | attribut sysfs manquant, device débranché, PPD absent | état « non supporté », pas de panique |
| Chemin | `/proc/self/fd` inspection après opération | aucune ouverture hors périmètre |

**Preuve** : table de tests paramétrée (`rstest`), rapport de couverture des branches de validation.

---

## 9. Avant release — checklist finale

- [ ] **B** — Tous les points **B** des sections 1 à 8 sont cochés.
- [ ] **B** — Aucun finding **Critique** ou **Élevé** ouvert dans `THREAT_MODEL.md` §6/§7.
- [~] **B** — cTGP par profil : `tccd` applique la borne dynamique `max_power_limit - default_power_limit` (0..200) ou `static` 200, l'expose dans `NvidiaData.maxCtgpOffset` et refuse hors borne avant écriture ; tests fake 60/90 W + offset 40 ⇒ refus (`REVIEW_PROFILE_HW_WRITES.md` PHW-03). **Matériel réel non testé** ; fallback `static` 200 W à durcir si le driver n'est pas prouvé.
- [~] **B** — TDP : distinction explicite `profile.tdpValues=[]` = **ne pas changer** (worker `tccd`) vs `Admin.Hw.SetTdpValues([])` = **maxima firmware explicite** (audit `values=empty, maxima=true`, no-op si déjà aux maxima) ; tests `odm` 18/18, `admin tdp` 9/9, `admin_hw_dbus` 5/5 (`REVIEW_PROFILE_HW_WRITES.md` PHW-03).
- [ ] **B** — Support multi-utilisateur : `SetTempProfile` et l'application déclenchée par l'état sont gatés (polkit `temp-profile` ou portée session) ; **bloquant si le support multi-utilisateur est revendiqué**, sinon release explicitement mono-utilisateur (TR-01/PHW-01).
- [ ] **B** — Threat model relu (version à jour, date, déclencheurs) et signé par `security` + `linux-platform`.
- [ ] **B** — Test d'autorisation D-Bus rejoué sur une VM propre, en utilisateur non privilégié.
- [-] **Hors périmètre** — Test LUKS : non applicable (aucune fonction LUKS).
- [ ] **B** — `systemd-analyze security` archivé ; aucune régression non justifiée.
- [ ] **B** — `dpkg -c`/`rpm -qpl`/`bsdtar -tf` relus : chemins FHS, permissions, absence de `/opt`, absence de setuid.
- [ ] **B** — SBOM + `Cargo.lock` + hash des artefacts archivés.
- [ ] **B** — `SECURITY.md` à la racine du dépôt (canal de signalement, politique de divulgation, délais) — **créé par le mainteneur**.
- [ ] **B** — Revue `unsafe` signée ; liste des blocs et tests associés.
- [ ] **I** — Campagne de fuzzing ≥ 24 h sans crash sur les cibles critiques.
- [ ] **I** — Revue de `cargo tree` (doublons, dépendances abandonnées, mainteneurs).
- [ ] **I** — Documentation utilisateur des limites (ce que TCC ne protège pas, prérequis Secure Boot).
- [ ] **A** — Test d'intrusion externe du bus D-Bus et des actions polkit.
- [ ] **B** — `docs/RELEASE_CHECKLIST.md` §2–5 rejoué (sécurité, admin phase 1, matériel, packaging) ; aucun finding **M1-01/M1-02**, **SL-01…SL-05** ou **V-01/V-02** ouvert sans acceptation écrite.
- [~] **B** — Écriture UVC de session : `tccd` reste `O_RDONLY`/`DeviceAllow=char-video4linux r` et n'appelle jamais `S_CTRL` ; `SetUvcControl` uniquement sur le bus de session `tccd-user` (même UID, device énuméré, allowlist 15, rate limit UID, erreurs typées, aucun polkit) ; **UVCW-02…08 fermés**, **UVCW-01 accepté comme dette documentée** ; gate restant : smoke test caméra réelle, ACL perdue, `EBUSY` avec aperçu, deux sessions ; watchdog non bloquant documenté. Sliders GUI autorisés sous `GetUvcWriteSupported` après smoke test. `REVIEW_V4L2.md` §2ter.
- [ ] **B** — Aucun code V4L2 en release sans la revue d'étape 0 de `REVIEW_V4L2.md` (allowlist normative, authz setter, `DeviceAllow`).

---

## 10. Preuves attendues avant release (renvoi `docs/RELEASE_CHECKLIST.md`)

Les preuves ci-dessous complètent `docs/RELEASE_CHECKLIST.md` §2 (sécurité), §3 (`tccd-admin`),
§4 (matériel) et §5 (packaging). Aucune ne peut être remplacée par un test unitaire.

| # | Preuve attendue | Référence |
|---|---|---|
| 1 | Vrai polkit admin : autorisé / refusé / annulé / polkit arrêté, sur bus système | `RELEASE_CHECKLIST.md` §3 ; `REVIEW_ADMIN_PHASE1.md` §8 |
| 2 | Vrai polkit setters `Manager1` (charge, lighting, Fn-Lock) après **M1-01/M1-02** ; root refusé ou autorisé selon polkit, jamais court-circuité | `REVIEW_MANAGER1_AUTHZ.md` |
| 3 | `dbus-policy-test.sh` en **root** : 42/42 + refus `Properties.*`/`Admin1` ; policy `Manager1` remise en `deny` tant que M1-01/M1-02 ouverts | `RELEASE_CHECKLIST.md` §3 ; `REVIEW_MANAGER1_AUTHZ.md` |
| 4 | `Control1.ReloadConfig` réel : succès, timeout, `tccd` arrêté, rollback ; aucun `.restore.*` résiduel | `RELEASE_CHECKLIST.md` §3 |
| 5 | Config réelle : `load_*_no_recovery` sur fichier corrompu, `.bak` `0600`, migration `/etc/tcc` sans écrasement | `CHECKLIST` §0bis ; `RELEASE_CHECKLIST.md` §3/§6 |
| 6 | Veille réelle `systemctl suspend` : fans auto, profil réappliqué, bail capteurs `false`, 10 cycles sans tâche/fd en double ; **SL-01…SL-05** traités | `REVIEW_SLEEP.md` §5 |
| 7 | Matériel Clevo/Uniwill/AMD/Intel/NVIDIA : charge, lighting, Fn-Lock, TDP/ODM/cTGP, PRIME, `DeviceAllow`, `tuxedo_io` | `RELEASE_CHECKLIST.md` §4 ; `REVIEW_HARDWARE.md` H-13 |
| 8 | V4L2 : étape 0 acceptée (5 ioctl, allowlist, `DeviceAllow=… r`) ; lecture réelle `/dev/video0` (`0666`+ACL, 11 contrôles) ; écriture session livrée (`User1.SetUvcControl`, fake/CI) ; **UVCW-02…08 fermés**, **UVCW-01 accepté** ; écriture réelle, ACL, `EBUSY`, suspend/hotplug, deux sessions non testés | `REVIEW_V4L2.md` §2ter |
| 9 | Packaging : `validate.sh` défaut/`--with-admin`, install/purge deb/rpm/Arch, `systemd-analyze verify/security`, aucun `/opt`/setuid, `tccd-admin` absent par défaut | `RELEASE_CHECKLIST.md` §5 |
| 10 | `cargo deny`, `cargo audit`, SBOM, `Cargo.lock`, `unsafe` review signée | `RELEASE_CHECKLIST.md` §1/§2 |
| 11 | GUI/QML Qt 6.10 (`qmllint`, `qmltestrunner`), `cargo test -p tcc-gui --bins` ; aucune écriture directe | `RELEASE_CHECKLIST.md` §1/§7 |

**Règle** : un point `[~]` de §0bis ne devient `[x]` qu'après la preuve correspondante listée ci-dessus ;
tant que la preuve manque, il reste `[~]` et la release est bloquée s'il est **B**.

---

## 11. Répartition des responsabilités

| Domaine | Agent responsable | Points de la checklist |
|---|---|---|
| Modèles, validation, atomicité, tests purs, fuzzing | `rust-core` | §1, §7, §8 |
| sysfs, ioctl, plages matérielles, display backends | `linux-platform` | §2, §4 |
| D-Bus, polkit, commandes externes, units | `linux-platform` + `security` | §3, §6 |
| GUI, secrets côté client, absence de privilège | `qt-ui` | §5 |
| Threat model, revues, arbitrages, rapport de release | `security` | §0, §9, §11 |

**Règle d'escalade** : tout écart à un point **B** doit être écrit dans la PR avec justification, approuvé par `security`, et suivi d'une entrée dans `THREAT_MODEL.md` §9 (risque résiduel). Un point **B** non coché sans justification bloque la release.
