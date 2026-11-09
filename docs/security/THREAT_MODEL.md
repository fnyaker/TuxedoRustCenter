# Modèle de menace — TuxedoRustCenter

| Champ | Valeur |
|---|---|
| Version | 1.0 (jalon 1) |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Portée | workspace `tcc-core`, `tcc-ipc`, `tcc-hw`, `tcc-daemon`, `tcc-user`, `tcc-gui`, `tcc-cli` + intégration système (systemd, D-Bus, polkit, udev) |
| Références | `AGENTS.md`, `docs/ARCHITECTURE.md` §3.10, `docs/FEATURE_PARITY.md` §12, `docs/ROADMAP.md` |
| Déclencheurs de révision | ajout d'une méthode D-Bus, d'un chemin sysfs, d'un service annexe (Aquaris, Tomte, WebFAI, Prime), changement de politique PPD, ajout d'une dépendance Cargo, changement des units systemd |

> **Périmètre 2026-11-06 — LUKS hors périmètre.** Le changement de phrase de passe LUKS est
> **non implémenté** (décision utilisateur). Aucune action polkit, aucune méthode D-Bus, aucun service,
> aucune capacité, aucun accès `cryptsetup`/block device ne doit être ajouté. Les sections S-10, F-02,
> §8.6 et les mentions LUKS ci-dessous sont **historiques** (vulnérabilité de l'original ou règle D5) :
> elles ne sont pas des travaux à réaliser. Voir `ADMIN_PHASE3_DESIGN.md`.

---

## 0. Avertissement de périmètre — ce qui a été audité et ce qui ne l'a pas été

**Lu et analysé :**

- l'intégralité de `docs/` et `AGENTS.md` ;
- le squelette Cargo (`Cargo.toml`, `Cargo.lock`, les 7 crates) ;
- les fichiers d'intégration système de l'original : `tuxedo-control-center-master/src/dist-data/{tccd.service, tccd-sleep.service, com.tuxedocomputers.tccd-dbus.conf, com.tuxedocomputers.tccd-service.policy, com.tuxedocomputers.tomte-access.policy, 99-webcam.rules}` ;
- les chemins sensibles de l'original : `TccDBusInterface.ts`, `TccDBusService.ts`, `TuxedoControlCenterDaemon.ts`, `ConfigHandler.ts`, `TccPaths.ts`, `SingleProcess.ts`, `Utils.ts`, `ChargingWorker.ts`, `LightingListener.ts`, `XDisplayRefreshRateController.ts`, `cryptAPI.ts`, `systemInfosAPI.ts`, `tomteAPI.ts`, `pgmsService.ts`.

**Ce qui n'existe pas encore et n'a donc pas pu être audité :**

- aucune ligne de logique métier Rust : les 7 crates totalisent ~30 lignes de `main.rs`/`lib.rs` (un `tracing_subscriber::fmt`, un `println!` et des constantes) ;
- aucune unit systemd, aucune politique D-Bus, aucun fichier polkit, aucun `deny.toml`, aucun workflow CI dans ce dépôt ;
- aucune implémentation de `tcc-core::validate`, de `ConfigStore`, du service D-Bus, des backends d'affichage.

**Conséquence méthodologique.** Ce document est un modèle de menace **prospectif** : il fixe les exigences et les tests que les autres agents doivent satisfaire. Les findings de la section 6 portent sur **le code original** (lecture statique, sans exécution ni test dynamique) et servent de sources de vérité pour les vulnérabilités à ne pas reproduire. Les findings de la section 7 portent sur **l'état du dépôt au jour de la rédaction** (risques organisationnels, pas des vulnérabilités logicielles).

Aucun test d'intrusion, aucune exécution de l'original, aucune analyse dynamique n'a été réalisée. Les probabilités sont qualitatives.

---

## 1. Actifs à protéger

| # | Actif | Propriétaire | Impact si compromis | Criticité |
|---|---|---|---|---|
| A1 | **Intégrité du matériel** (TDP/PL1-PL2, limites de charge, courbes ventilateurs, cTGP, platform_profile) | utilisateur, OEM | Surchauffe, usure accélérée, batterie endommagée, arrêt thermique, danger physique en cas d'emballement | Critique |
| A2 | **Intégrité de la configuration profils/settings** (`/etc/tuxedo-rust-center`, `/var/lib/tuxedo-rust-center`) | root | Profil malveillant appliqué au boot → charge désactivée, ventilateurs bloqués à 0, CPU bridé | Élevé |
| A3 | **Secrets LUKS** — **hors périmètre, non implémenté** (conservé comme actif historique : aucune fonction LUKS n'est livrée) | utilisateur | Déchiffrement du disque, perte définitive d'accès si la clé est corrompue | **Hors périmètre** |
| A4 | **Intégrité de la session graphique** (mode d'affichage, résolution, taux de rafraîchissement) | utilisateur | Écran noir / mode non supporté / output externe perturbé ; dégradation UX, perte de travail non enregistré | Moyen |
| A5 | **Disponibilité du daemon** (`tccd`) | système | Perte du contrôle thermique et de la bascule automatique, ventilation en mode BIOS, surchauffe en charge | Élevé |
| A6 | **Confidentialité des données système** (environnements de processus, logs de diagnostic, numéros de série, MAC) | utilisateur | Fuite d'informations vers un tiers lors du support, profilage | Moyen |
| A7 | **Réputation du projet** | projet | Publication d'un CVE critique, perte de confiance, régression vers Electron | Élevé |
| A8 | **Intégrité de la chaîne de build** (crates, `Cargo.lock`, artefacts) | projet | Backdoor dans un binaire root distribué à des milliers de machines | Critique |
| A9 | **Intégrité du bus système** (nom `com.tuxedocomputers.tccd`) | root | Usurpation du service, interception des réponses, confusion de client | Élevé |

---

## 2. Acteurs et capacités

| Acteur | Accès | Capacités | Objectif plausible |
|---|---|---|---|
| **U1 — utilisateur local non privilégié** | session graphique, bus système en émission, PATH personnel | appelle toute méthode D-Bus autorisée, lance des processus, écrit dans `$HOME`, peut créer des fichiers dans `/tmp`, contrôle son environnement | obtenir root, désactiver la charge pour « gagner » de la performance, perturber un autre utilisateur, exfiltrer des secrets |
| **U2 — client D-Bus malveillant** | peut être un simple script `busctl`/`zbus` | forge des signatures, envoie des rafales, rejoue des appels, envoie des tailles et des types inattendus, écoute les signaux | DoS, fuzzing sauvage du parseur root, contournement d'autorisation |
| **U3 — processus compromis dans la session** | session entière (GUI, agent, compositeur IPC) | lit les variables d'environnement de la session, écrit des fichiers utilisateur, peut se faire passer pour la GUI auprès du daemon, peut altérer `$XDG_RUNTIME_DIR` | pivot vers le daemon root via D-Bus, injection dans l'agent de session, attaque du compositeur |
| **U4 — dépendance supply-chain compromise** | crates.io, GitHub, script de support téléchargé | exécute du code au build ou à l'exécution, peut publier une version malveillante d'une crate, modifier une branche `main` | backdoor root, exfiltration |
| **U5 — attaquant physique** | accès au châssis, port USB/Thunderbolt, console | vol, Evil Maid sur `/boot`, DMA, cold boot, remplacement de binaire non signé | extraction de la clé LUKS, persistance |
| **U6 — opérateur légitime maladroit** | root, terminal | lance des commandes destructrices, applique un profil inadapté | indisponibilité, perte de configuration |
| **U7 — service tiers** (PPD, UPower, compositeur, périphérique BLE Aquaris, GitHub) | fournit des données au daemon | renvoie des valeurs inattendues ou malformées, disparaît, oscille | déstabilisation de la bascule, crash du daemon |

**Non-acteurs / hors périmètre :** root déjà compromis (le modèle ne protège pas contre un root hostile), noyau compromis, firmware malveillant, side-channels micro-architecturaux.

---

## 3. Frontières de confiance

```
        ┌─────────────────────────────────────────────────────────────┐
        │  U1/U3 : session utilisateur (non fiable)                   │
        │  ┌──────────────┐   ┌─────────────────────┐                 │
        │  │ tcc-gui (Qt) │   │ tccd-user (agent)   │                 │
        │  └──────┬───────┘   └──────┬──────────────┘                 │
        └─────────┼──────────────────┼────────────────────────────────┘
        F1 (D-Bus système)          F8 (signaux D-Bus système, lecture)
                  │                  │  F9 (socket compositeur, même uid)
        ┌─────────▼──────────────────▼────────────────────────────────┐
        │  tccd — root (frontière de privilège)                        │
        │  ┌───────────┐  ┌────────────┐  ┌──────────┐  ┌───────────┐  │
        │  │ D-Bus API │→ │ validation │→ │ workers  │→ │ tcc-hw    │  │
        │  └───────────┘  └────────────┘  └──────────┘  └─────┬─────┘  │
        │  ┌───────────┐  ┌────────────┐                      │        │
        │  │ config FS │  │ cmd externes│                     │        │
        │  └───────────┘  └────────────┘                      │        │
        └──────────────────────────────────────────────────────┼───────┘
        F5 (fichiers)   F6 (exec)                             F3/F4 (sysfs, ioctl)
                  │           │                                │
             /etc, /var/lib  Tomte, prime-select,        /sys, /dev/tuxedo_io,
                             WebFAI, systeminfos         DRM, V4L2, hwmon
        ─────────────────────────────────────────────────────────────────────
        F7 : PPD / UPower (D-Bus système, données non fiables)
        F10 : Aquaris BLE (périphérique non fiable)
        F11 : crates.io / GitHub (build et support)
```

| # | Frontière | Nature de la donnée | Confiance | Contrôles obligatoires |
|---|---|---|---|---|
| **F1** | GUI/agent ↔ `tccd` via bus système | appels de méthode, signaux | **non fiable** | politique D-Bus fine, polkit par action, validation typée, rate limit, taille bornée |
| **F2** | client D-Bus arbitraire ↔ `tccd` | idem F1 | **non fiable** | identique F1 + vérification du *sender* (nom unique, uid via bus) |
| **F3** | `tccd` ↔ sysfs | valeurs numériques, énumérations | kernel de confiance, **valeurs lues non fiables** | allowlist de chemins construits, plages, `openat2`/`O_NOFOLLOW`, vérif. après ouverture |
| **F4** | `tccd` ↔ `/dev/tuxedo_io` | ioctl + structures | kernel de confiance, **structures à valider** | `unsafe` isolé, tailles/sentinelles vérifiées, index bornés, retour d'erreur propagé |
| **F5** | `tccd` ↔ fichiers de config | JSON | **non fiable à l'import**, root-owned au repos | `openat2`/`O_NOFOLLOW`, vérif. propriétaire/mode/type, bornes de taille, validation, écriture atomique |
| **F6** | `tccd` ↔ commandes externes | argv, env, stdout | **non fiable** (PATH, env, contenu téléchargé) | allowlist de binaires absolus, pas de shell, `env_clear`, timeouts, sortie bornée |
| **F7** | `tccd` ↔ PPD / UPower | propriétés D-Bus | **non fiable** | validation des valeurs, fallback, debounce, ne jamais paniquer sur une réponse inattendue |
| **F8** | `tccd` ↔ `tccd-user` | signaux de profil | semi-fiable (même bus) | payload borné, agent qui ne fait qu'appliquer l'affichage, aucune commande arbitraire |
| **F9** | `tccd-user` ↔ compositeur | IPC/socket/session bus | **non fiable** (même uid) | filtrage de l'output interne, modes validés contre la liste du compositeur, socket dans `$XDG_RUNTIME_DIR` vérifié (uid, mode 0700, pas de symlink) |
| **F10** | Aquaris BLE | trames GATT | **non fiable** | parseur borné, pas d'écriture privilégiée pilotée par le périphérique, appairage explicite |
| **F11** | crates.io, GitHub, script de support | code exécuté / données | **non fiable** | `Cargo.lock`, `cargo deny`, pas de contenu distant exécuté en root, jamais de `main` non épinglée |

---

## 4. Hypothèses et non-objectifs

**Hypothèses :**

1. `tccd` tourne en root sous systemd, avec un jeu de capacités réduit, et **aucun** binaire setuid n'existe dans le projet.
2. Le bus système est correctement configuré par la distribution ; notre politique est une couche de défense supplémentaire, pas la seule.
3. `/sys` et `/dev` sont montés normalement ; les attributs sysfs sont root-owned.
4. L'utilisateur est autorisé à changer son propre profil et ses propres réglages : ce n'est **pas** une frontière de sécurité entre deux utilisateurs, mais une frontière entre utilisateur et noyau.
5. PPD reste propriétaire de `platform_profile` et de l'EPP (décision par défaut d'`ARCHITECTURE.md` §3.5) ; TCC n'écrit ces attributs que sur option explicite.
6. La configuration est root-owned ; un utilisateur non privilégié ne peut jamais écrire dans `/etc/tuxedo-rust-center` ni `/var/lib/tuxedo-rust-center`.

**Non-objectifs :**

- protéger contre un attaquant ayant déjà root ;
- protéger contre un noyau ou un firmware compromis ;
- confidentialité des secrets vis-à-vis de root : root peut toujours lire la mémoire du daemon. On protège contre les **autres** utilisateurs locaux et contre la persistance dans les logs/argv/core dumps ;
- anti-DMA / IOMMU / cold boot ;
- signature des profils (aucun modèle de signature prévu) ;
- sécurité réseau (le daemon ne doit avoir **aucun** accès réseau, cf. §8.3).

---

## 5. Scénarios de menace

Échelle de sévérité : **Critique** = root ou secret ; **Élevé** = intégrité matérielle, contournement d'autorisation, disponibilité prolongée ; **Moyen** = DoS limité, fuite d'information partielle, dégradation UX ; **Faible** = durcissement manquant sans impact direct.
Colonne « Statut » : **B** = bloquant pour toute release, **I** = important, **A** = amélioration.

### S-01 — Appel D-Bus forgé sur une méthode d'écriture

| | |
|---|---|
| **Vecteur** | U1/U2 appelle `SetChargeStartThreshold`, `SetTempProfile`, `SetKeyboardStatesJSON`, `SetFnLockStatus` sans authentification. C'est exactement ce que permet la politique D-Bus de l'original (`com.tuxedocomputers.tccd-dbus.conf:11-14`) et ce que fait le daemon (aucun contrôle d'autorisation dans `TccDBusInterface.ts`). |
| **Impact** | Modification de l'état matériel sans consentement : charge désactivée (batterie vide au prochain débranchement), rétroéclairage sur une valeur absurde, profil temporaire imposé. A1 + A2. |
| **Probabilité** | **Certaine** sur l'original (un `busctl call` suffit). |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | Séparation lecture/écriture au niveau de la politique D-Bus ; polkit pour toute écriture persistante ; interface `Manager1` (lecture + profil temporaire) distincte de `Admin1` (écritures persistantes, Prime, Tomte, WebFAI/diagnostic) ; le daemon re-vérifie systématiquement l'autorisation, même si la politique D-Bus l'autorise. |
| **Tests de non-régression** | `tcc-daemon/tests/dbus_authz.rs` : en tant qu'utilisateur non privilégié, `busctl call ... Admin1.SetChargingProfile` → `AccessDenied` au niveau bus **et**, si la couche bus est désactivée, `org.freedesktop.DBus.Error.AccessDenied` au niveau daemon. Test paramétré sur **toutes** les méthodes `Set*` (liste générée depuis l'introspection pour qu'une nouvelle méthode non classée fasse échouer le test). |

### S-02 — Confusion de type / signature D-Bus

| | |
|---|---|
| **Vecteur** | U2 envoie `i` au lieu de `u`, `as` vide, `a{sv}` avec des clés inattendues, `s` de 10 Mo, `h` (fd) inattendu, ou une valeur négative là où le code attend un entier positif. Variante historique : méthodes qui renvoient/acceptent du **JSON en chaîne** (`GetProfilesJSON`, `SetKeyboardStatesJSON`) — le typage est alors inexistant. |
| **Impact** | Panique du daemon (si `unwrap`/`expect`), écriture d'une valeur aberrante dans sysfs, désynchronisation d'état, épuisement mémoire. A5 + A1. |
| **Probabilité** | Moyenne (fuzzing trivial). |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | API typée `Manager1`/`Admin1` avec structs `zvariant` explicites ; **aucun** `serde_json::Value` dans une signature D-Bus ; pour la couche de compatibilité JSON, taille max 1 Mio, `serde` avec types stricts, rejet des champs inconnus quand c'est possible ; signatures `h` **non utilisées** pour LUKS (hors périmètre ; D5 reste la règle pour tout futur secret) ; toute erreur de désérialisation → erreur D-Bus typée, jamais de panique. |
| **Tests de non-régression** | tests de désérialisation hostiles (corpus : profondeur 1000, 10⁶ éléments, types croisés) ; `cargo fuzz` cible `dbus_payload` ; test « aucune méthode ne panique sur entrée aléatoire » avec `proptest`. |

### S-03 — Usurpation du nom de service / interception

| | |
|---|---|
| **Vecteur** | U1 tente de posséder `com.tuxedocomputers.tccd` avant le démarrage du daemon, ou enregistre `com.tuxedocomputers.tccd.Admin` pour capter des appels, ou exploite un `NameOwnerChanged` mal géré. Variante : le daemon demande le nom sans `DBUS_NAME_FLAG_DO_NOT_QUEUE`, ce qui permet à un tiers de se mettre en file. |
| **Impact** | Client trompé : la GUI envoie ses commandes à un faux daemon ; réponses falsifiées ; phishing d'authentification polkit. A9. |
| **Probabilité** | Faible si la politique est correcte (elle l'interdit : `<deny own>` pour tout sauf root), moyenne en cas d'oubli. |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | `<deny own>` explicite pour tous sauf root ; `request_name` avec `DO_NOT_QUEUE` → échec immédiat si le nom est pris ; signal `NameLost` traité comme erreur fatale et arrêt du daemon (mieux vaut pas de daemon qu'un daemon qui ne contrôle plus rien) ; la GUI vérifie le **nom unique** du propriétaire, pas seulement le nom bien connu. |
| **Tests de non-régression** | test d'intégration avec un faux propriétaire de nom sur un bus de test : `tccd` doit refuser de démarrer ; test que la GUI rapporte « daemon indisponible » au lieu de retomber silencieusement sur un autre nom. |

### S-04 — Profil ou import JSON piégé

| | |
|---|---|
| **Vecteur** | U1 importe un fichier de profil (bouton import, ou fichier déposé dans un répertoire migré) contenant : `refreshRate: 0` ou `NaN`, `brightness: 100000`, `onlineCores: 0`, `scalingMaxFrequency: 0`, `fanProfile` avec une courbe de 1 M points, `id` avec `/` ou `..`, clés dupliquées, JSON imbriqué à 1000 niveaux, ou `"name": "\u0000…"`. |
| **Impact** | Application d'un profil destructeur (ventilateurs bloqués, CPU à 0 MHz, écran noir), panique au parsing, écriture hors du répertoire prévu si l'`id` sert de nom de fichier. A1 + A2 + A4. |
| **Probabilité** | Moyenne (l'import est une fonctionnalité utilisateur). |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | Validation centralisée `tcc_core::validate_profile` / `validate_settings` : taille max du fichier, nombre max de profils, bornes de chaque champ, invariants croisés (`charge_start < charge_end`, `fan_min ≤ fan_max`, `PL1 ≤ PL2`, `governor ∈ liste disponible`, `refresh_rate.is_finite() && 1.0..=1000.0`), `id` conforme à `^[A-Za-z0-9_-]{1,64}$`, rejet des NUL et des caractères de contrôle dans les chaînes, `schema_version` connue. **Aucun** chemin construit à partir d'une valeur de profil. |
| **Tests de non-régression** | corpus de fichiers malveillants dans `tcc-core/tests/fixtures/invalid/` ; `proptest` : « toute entrée invalide produit une erreur typée, jamais une panique, et ne mute pas l'état » ; `cargo fuzz` cible `parse_profile` ; test de migration qui rejoue de vrais fichiers `/etc/tcc/profiles` et vérifie qu'aucun profil n'est accepté hors bornes. |

### S-05 — Symlink / TOCTOU sur les fichiers de configuration

| | |
|---|---|
| **Vecteur** | U1/U3 crée `/var/lib/tuxedo-rust-center/…` ou un fichier temporaire, puis un lien symbolique vers `/etc/shadow`, `/etc/sudoers.d/x`, `~/.ssh/authorized_keys`. Le daemon root écrit/renomme en suivant le lien. Variante TOCTOU : vérification `stat` puis ouverture (fenêtre entre les deux). Variante répertoire : le daemon fait `mkdir -p` puis écrit sans vérifier que le répertoire appartient bien à root et n'est pas world-writable — c'est ce que fait `ConfigHandler.writeConfig` (`ConfigHandler.ts:211-241`). |
| **Impact** | Écrasement arbitraire de fichier en root → escalade de privilèges. A1/A2 → root. |
| **Probabilité** | Faible à moyenne selon l'emplacement réel des fichiers (les répertoires root-owned protègent), mais la conception actuelle n'offre **aucune** garantie structurelle. |
| **Sévérité** | Critique — **B** |
| **Mitigation attendue** | `openat2` avec `RESOLVE_BENEATH \| RESOLVE_NO_SYMLINKS \| RESOLVE_NO_MAGICLINKS \| RESOLVE_NO_XDEV` (kernel ≥ 5.6), ou à défaut `openat` + `O_NOFOLLOW \| O_CLOEXEC` sur chaque composant ; `fstat` du fd ouvert (et non du chemin) : `S_ISREG`, `st_uid == 0`, `st_mode & 0o022 == 0` ; écriture sur le fd ; `fchmod(0644)` explicite après ouverture (le `mode` de `open` ne s'applique qu'à la création et ne corrige pas un fichier existant) ; `fsync` fichier + répertoire ; `renameat` depuis un fichier temporaire créé dans le même répertoire avec `O_TMPFILE` ou nom aléatoire ; vérification du répertoire parent (propriétaire root, non world-writable) avant toute création. |
| **Tests de non-régression** | `tcc-core/tests/config_atomicity.rs` : symlink placé à la place du fichier cible → l'écriture doit échouer avec `EACCES`/`ELOOP` et la cible rester inchangée ; répertoire 0777 → refus ; crash simulé entre `write` et `rename` → fichier précédent intact ; permissions finales vérifiées `0644` root:root ; `strace` doit montrer `openat2` (ou `O_NOFOLLOW`) et non `open`. |

### S-06 — Traversée de chemin / écriture sysfs arbitraire

| | |
|---|---|
| **Vecteur** | Un champ de profil (nom de LED, zone RGB, nom d'output, nom de ventilateur, chemin de fichier `fantables`) est concaténé dans un chemin sysfs ou un chemin de fichier. Exemple : `keyboardStates` de l'original contient des chaînes qui finissent dans des chemins de LED (`LightingListener.ts`), et `SetKeyboardStatesJSON` accepte du JSON arbitraire d'un utilisateur non privilégié (`TccDBusInterface.ts:300-309`). |
| **Impact** | Écriture root dans un attribut sysfs arbitraire (ex. `…/reset`, `…/uevent`, ou un autre device), ou écriture dans un fichier arbitraire. A1 → root. |
| **Probabilité** | Moyenne sur l'original, à supprimer par construction dans la réécriture. |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | **Aucun chemin ne contient de donnée utilisateur.** Les chemins sont des constantes ou des gabarits + index entier borné. Quand un nom est inévitable (output d'affichage, LED), il est validé contre une allowlist **découverte au runtime** (énumération sysfs/DRM/compositeur) et comparé en égalité stricte, jamais par préfixe. Après ouverture, vérifier le chemin réel via `readlink("/proc/self/fd/N")` et exiger le préfixe attendu (`/sys/`). |
| **Tests de non-régression** | test avec un nom d'output `../../etc/passwd`, `..`, `eDP-1/../../x`, chaîne vide, NUL → rejet ; test qui vérifie qu'un attribut sysfs d'un device non listé n'est jamais ouvert (`/proc/self/fd` inspecté après l'opération, avec un sysfs factice). |

### S-07 — Valeur hors plage écrite dans le matériel

| | |
|---|---|
| **Vecteur** | U1 appelle `SetChargeStartThreshold(-5)` ou `(0)`, `SetChargeEndThreshold(0)`, un TDP à 500 W, une luminosité à 255 sur un backlight plafonné à 100, un taux de rafraîchissement inexistant. Les noyaux clampent parfois, mais pas toujours, et `start > end` a une sémantique destructrice (batterie qui ne charge plus). `ChargingWorker.setChargeStartThreshold` (`ChargingWorker.ts:230-245`) écrit la valeur reçue **sans aucun clamp** et retourne `true` même en cas d'échec. |
| **Impact** | Batterie non chargée (indisponibilité au prochain débranchement), surchauffe, mode d'affichage invalide, matériel en état incohérent persistant après redémarrage. A1. |
| **Probabilité** | Certaine sur l'original. |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | Pour chaque champ : (1) plage statique documentée dans `tcc-hw` avec la source (datasheet, driver, attribut `min`/`max` sysfs quand il existe) ; (2) rejet explicite hors plage avec erreur typée, **pas** de clamp silencieux (le clamp silencieux masque les bugs et fausse l'UI) ; (3) invariants croisés vérifiés avant écriture ; (4) relecture après écriture et comparaison quand le matériel le permet ; (5) refus si la capacité n'est pas détectée (ne jamais écrire dans un attribut absent). |
| **Tests de non-régression** | table de tests par attribut : valeur min-1, min, max, max+1, `i32::MIN`, `i32::MAX`, NaN/Inf quand applicable → écriture refusée pour les hors-plages, acceptée aux bornes ; test « start ≥ end » rejeté ; test avec sysfs factice qui enregistre chaque écriture et vérifie qu'aucune valeur hors plage n'a été écrite. |

### S-08 — DoS par allocation ou récursion (JSON bomb)

| | |
|---|---|
| **Vecteur** | U1/U2 envoie un message D-Bus de plusieurs Mo contenant un tableau gigantesque, un JSON à 10 000 niveaux, ou un profil avec 10⁶ points de courbe. Le daemon root alloue jusqu'à l'OOM ; sous systemd avec `MemoryMax`, il est tué ; sans limite, il fait tomber la machine ou déclenche l'OOM killer sur des processus voisins. |
| **Impact** | Perte du contrôle thermique/charge, instabilité du système, redémarrage. A5. |
| **Probabilité** | Élevée (trivial). |
| **Sévérité** | Moyen — **B** |
| **Mitigation attendue** | Limite de taille au niveau bus (`max_message_size` de la politique D-Bus si possible) **et** dans le daemon (rejet > 1 Mio pour les payloads JSON, > 64 Kio pour les appels typés) ; limites structurelles : ≤ 256 profils, ≤ 64 points par courbe, ≤ 128 caractères par nom, profondeur JSON bornée ; `MemoryMax=` dans l'unit ; une seule tâche de parsing lourde à la fois (sémaphore) ; `catch_unwind` interdit comme stratégie — le code ne doit pas paniquer. |
| **Tests de non-régression** | test avec un payload de 10 Mio → `LimitsExceeded` immédiat, RSS du daemon stable (± 5 Mo) ; test avec 10⁶ points de courbe → rejet ; mesure RSS avant/après dans le test. |

### S-09 — DoS par rafale d'appels / polling forcé

| | |
|---|---|
| **Vecteur** | U2 appelle `GetChargingProfilesAvailable` en boucle (chaque appel déclenche du travail), ou `SetSensorDataCollectionStatus(true)` pour maintenir le daemon en collecte permanente (l'original ne l'arrête qu'après 10 s sans appel, `TccDBusInterface.ts:97-105` et `196-198`), ou spamme `SetTempProfileById` pour provoquer des rechargements de profil en cascade. |
| **Impact** | CPU 100 %, batterie vidée, I/O sysfs saturés, latence de la bascule. A5. |
| **Probabilité** | Élevée. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | Token bucket **par nom unique d'expéditeur** (lectures 50/s, écritures 5/s, actions admin 1/10 s) ; plafond global ; `Semaphore` sur les handlers ; collecte capteurs comptée par client avec arrêt sur `NameOwnerChanged` et expiration ; coalescence des demandes de profil (une seule application par fenêtre de debounce) ; `CPUQuota=`/`MemoryMax=` dans l'unit. |
| **Tests de non-régression** | test de rafale : 10 000 appels → au moins 99 % rejetés après le seuil, RSS/CPU stables, daemon toujours réactif (un appel légitime suivant réussit) ; test « 50 clients activent la collecte puis se déconnectent → collecte arrêtée ». |

### S-10 — Fuite de la phrase de passe LUKS — **hors périmètre, historique**

> **Hors périmètre (décision utilisateur), non implémenté.** Cette section décrit une vulnérabilité
> de l'**original** et la règle D5 conservée pour tout futur secret ; elle ne constitue **pas** un
> travail à réaliser. Aucune action polkit, méthode D-Bus, capacité ou accès `cryptsetup` ne doit être
> ajouté.

| | |
|---|---|
| **Vecteur** | Quatre fuites distinctes, toutes présentes dans l'original (`cryptAPI.ts:30-38`) : **(a)** la phrase est concaténée dans une commande `pkexec /bin/sh -c "…"` → visible dans `/proc/<pid>/cmdline`, **lisible par tout utilisateur local** ; **(b)** une apostrophe dans la phrase casse le quoting et permet l'injection de commande ; **(c)** erreurs et `stdout`/`stderr` remontés dans l'UI et les logs ; **(d)** la phrase transite en clair dans un message D-Bus/IPC (mémoire, éventuellement `dbus-monitor`) et reste dans le tas sans être effacée. |
| **Impact** | Divulgation de la clé de chiffrement du disque à tout utilisateur local pendant l'opération (a), exécution de commande root (b), persistance dans les logs (c). A3 → perte totale de confidentialité du disque. |
| **Probabilité** | Certaine sur l'original (a est une simple lecture de `/proc`). |
| **Sévérité** | **Historique (original) — hors périmètre, non applicable à la réécriture** |
| **Mitigation historique** | D5 est conservée comme règle pour tout **futur** secret : jamais argv/env/log/core, descriptor si un secret doit un jour être traité, `zeroize`, `LimitCORE=0`. **Aucune fonction LUKS n'est livrée ; ces éléments ne sont pas un plan de travail.** |
| **Tests de non-régression** | **Non applicables** : aucun code LUKS, aucune action polkit, aucun `cryptsetup`. |

### S-11 — Injection de commande / détournement de PATH

| | |
|---|---|
| **Vecteur** | Trois chaînes réelles de l'original : **(a)** `ticketNumber` interpolé entre guillemets dans `pkexec env … sh <script> "${ticketNumber}"` (`systemInfosAPI.ts:156-169`) → `$(…)`/backticks exécutés en root ; **(b)** `which tuxedo-systeminfo` (`systemInfosAPI.ts:66`) résolu via le PATH **de l'utilisateur**, puis le chemin obtenu est exécuté en root via pkexec → un binaire utilisateur dans `~/.local/bin/tuxedo-systeminfo` devient root ; **(c)** `DISPLAY`/`XAUTHORITY`/`XDG_*` seulement `replace('"','')` (première occurrence) puis injectés dans `sh -c` (`systemInfosAPI.ts:160-164`). |
| **Impact** | Exécution de code arbitraire en root (escalade locale complète), sous réserve d'une authentification polkit que l'utilisateur est incité à fournir. A1→root. |
| **Probabilité** | Moyenne (nécessite une interaction, mais le scénario est crédible : l'utilisateur clique « Support »). |
| **Sévérité** | Critique — **B** |
| **Mitigation attendue** | Aucun `sh -c`, jamais. `Command::new("/chemin/absolu")` + `args([...])` séparés ; `env_clear()` puis injection d'un environnement minimal et **fixe** (`PATH=/usr/sbin:/usr/bin:/sbin:/bin`, `LANG=C`, `LC_ALL=C`) ; résolution des binaires dans une allowlist de chemins absolus, sans `which` ; arguments validés par type (un numéro de ticket est un `u32`, pas une chaîne) ; timeouts et capture bornée ; jamais d'exécution de contenu téléchargé (cf. S-16). |
| **Tests de non-régression** | test avec `ticketNumber = "$(id)"`, `` `id` ``, `; id`, `\n`, `"` → aucune exécution, le binaire reçoit la chaîne littérale ou l'appel est rejeté ; test avec un `tuxedo-systeminfo` malveillant dans un répertoire du PATH utilisateur → jamais exécuté ; test que l'environnement de l'enfant est exactement celui attendu (`/proc/<pid>/environ`). |

### S-12 — Abus de polkit (confused deputy)

| | |
|---|---|
| **Vecteur** | L'original déclare l'action `com.tuxedocomputers.tccd` avec `org.freedesktop.policykit.exec.path` pointant sur le **binaire du daemon** et `org.freedesktop.policykit.exec.allow_gui=true` (`com.tuxedocomputers.tccd-service.policy:18-25`) : polkit autorise donc l'exécution de `tccd` avec **des arguments arbitraires** en root. Variantes dans la réécriture : sujet polkit forgé (vérifier l'identité du **daemon** au lieu de celle de l'appelant), `AllowUserInteraction=true` sans borne, cache d'autorisation trop long, action unique couvrant Prime + installation de paquets. |
| **Impact** | Contournement complet du modèle de privilèges : l'utilisateur autorise « TCC » et obtient root. A1/A3. |
| **Probabilité** | Élevée si le modèle original est repris. |
| **Sévérité** | Critique — **B** |
| **Mitigation attendue** | **(1)** **Aucune** action polkit d'exécution de binaire. Pas de `pkexec`, pas de `exec.path`. **(2)** Une action par opération sensible, avec `defaults` explicites : `allow_any=no`, `allow_inactive=no`, `allow_active=auth_admin` (Prime, Tomte, WebFAI/paquets, diagnostic) ou `auth_admin_keep` (assignation de profil) — jamais `yes` pour une opération destructive. **(3)** Vérification via `org.freedesktop.PolicyKit1.Authority.CheckAuthorization` avec un sujet **`system-bus-name` portant le nom unique de l'appelant**, jamais `unix-process` avec le PID du daemon (qui passerait toujours), ni le PID fourni par le client. **(4)** `AllowUserInteraction=true` uniquement après validation de l'entrée, et une seule interaction par opération. **(5)** Cache de décision par nom unique, TTL court, invalidé sur changement de propriétaire. **(6)** Distinguer explicitement `NotAuthorized` (pas d'identité) et `Challenge` (interaction possible). |
| **Tests de non-régression** | `tcc-daemon/tests/polkit.rs` avec un mock d'autorité : appelant non autorisé → refus sans interaction ; `unix-process` fourni par le client → ignoré ; sujet vérifié = nom unique de l'appelant ; après `NameOwnerChanged`, le cache est invalidé ; test d'introspection : aucune action installée ne contient `exec.path`. |

### S-13 — Élévation via setuid / capacités / fichiers privilégiés

| | |
|---|---|
| **Vecteur** | Un binaire setuid est introduit « pour simplifier » (raccourci classique), ou le daemon reçoit un jeu de capacités large (`CAP_SYS_ADMIN`, `CAP_DAC_OVERRIDE`) qui annule le bénéfice du moindre privilège, ou un script de post-installation crée un fichier world-writable dans un répertoire du PATH root. |
| **Impact** | Surface d'attaque root permanente, indépendante du daemon. A1. |
| **Probabilité** | Faible si la règle « aucun setuid » est vérifiée en CI. |
| **Sévérité** | Élevé — **B** |
| **Mitigation attendue** | Interdiction absolue de setuid/setgid/file capabilities sur les binaires du projet ; `CapabilityBoundingSet=` aussi vide que possible (objectif : vide, vérifié sur matériel) ; `SecureBits=noroot-locked` ; `NoNewPrivileges=yes` ; test CI qui échoue si un binaire produit porte le bit setuid (`find -perm -4000`). |
| **Tests de non-régression** | contrôle de packaging : `find "$pkgroot" -perm -4000 -o -perm -2000` → vide ; `getcap -r "$pkgroot"` → vide ; `systemd-analyze security tccd.service` avec un score cible documenté. |

### S-14 — Agent de session détourné / commandes d'affichage arbitraires

| | |
|---|---|
| **Vecteur** | L'agent `tccd-user` expose une méthode D-Bus sur le bus de session qui accepte une chaîne de commande (ou un nom d'output, ou un mode) provenant d'un processus quelconque de la session. U3 (processus compromis, ou simple script) l'appelle avec `output=HDMI-1`, `rate=1`, `mode=640x480`, ou une chaîne Hyprland contenant `; exec …`. |
| **Impact** | Écran externe éteint/reconfiguré (perte de travail), mode non supporté (écran noir), injection dans la configuration du compositeur (Hyprland `keyword monitor` accepte une ligne de configuration arbitraire). A4. |
| **Probabilité** | Moyenne si l'agent expose une API. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | **(1)** L'agent **n'expose aucune méthode** : il est client du bus système (lecture du profil actif + signal) et client du compositeur. Aucune surface entrante. **(2)** S'il faut une API (jalon 4), elle est typée `{profile_id, target_rate_hz}` — jamais de nom d'output ni de chaîne de commande fournie par le client. **(3)** L'output cible est **découvert par l'agent** et filtré sur `^(eDP|LVDS|DSI)-[0-9]+$`, recoupé avec `/sys/class/drm/card*-eDP-*` ; jamais « le premier output ». **(4)** Le taux doit être fini, dans `1.0..=1000.0`, et correspondre à un mode **réellement annoncé** par le compositeur pour cet output ; tolérance 0,5 Hz. **(5)** La résolution n'est changée que si le profil la demande **et** que le mode existe ; sinon erreur, pas de fallback. **(6)** Refus d'exécution si `geteuid() == 0` (l'agent refuse d'être root). **(7)** Aucune écriture dans la configuration persistante du compositeur : GNOME via `ApplyMonitorsConfig(method=1)` (temporaire), pas `method=2`. |
| **Tests de non-régression** | test unitaire du filtre de connecteur (`HDMI-1`, `DP-1`, `eDP-1/../x`, `` → rejet ; `eDP-1`, `LVDS-1`, `DSI-1` → acceptation) ; test « mode absent de la liste → aucune application » ; test « agent lancé en root → sortie immédiate avec code d'erreur » ; test Hyprland/Sway avec un nom d'output validé puis injection tentée dans les arguments → la commande envoyée ne contient que les composants validés ; test « écran externe présent → sa configuration est identique avant/après ». |

### S-15 — Écran externe ou état de session perturbé

| | |
|---|---|
| **Vecteur** | Application du mode à tous les outputs, application pendant que la session est verrouillée ou en cours de changement, application sur un TTY, réapplication en boucle parce que le compositeur « corrige » aussitôt. |
| **Impact** | Écran noir au déverrouillage, perte de la disposition multi-écrans, oscillation de mode. A4. |
| **Probabilité** | Moyenne. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | Vérifier l'état de session (logind `LockedHint`/`Active`) avant application ; différer et réessayer au prochain signal ; une seule application en vol par output ; anti-rebond ; ne jamais réappliquer si le mode effectif est déjà correct ; aucune restauration agressive à l'arrêt de l'agent (documenter le comportement choisi). |
| **Tests de non-régression** | test « session verrouillée → aucune tentative » ; test « mode déjà appliqué → zéro appel au compositeur » (compteur d'appels) ; test « deux demandes rapprochées → une seule application ». |

### S-16 — Supply chain et contenu distant exécuté

| | |
|---|---|
| **Vecteur** | **(a)** une crate compromise ou un `build.rs` malveillant s'exécute au build et injecte du code dans `tccd` ; **(b)** une dépendance git non épinglée ; **(c)** **déjà présent dans l'original** : téléchargement de `https://raw.githubusercontent.com/.../refs/heads/main/files/usr/bin/systeminfos.sh` (`systemInfosAPI.ts:30-31`), écriture en root dans `/usr/libexec/` (`:103-136`), puis exécution en root (`:156-169`) — aucune vérification de hash ni de signature, branche **mutable**. |
| **Impact** | Backdoor root distribuée à tous les utilisateurs, ou exécution root d'un script contrôlé par un tiers. A8 + A1. |
| **Probabilité** | Faible pour (a) avec `Cargo.lock` + audit, **élevée** pour (c) : la compromission d'un dépôt GitHub ou d'une AC est un scénario réaliste. |
| **Sévérité** | Critique — **B** |
| **Mitigation attendue** | **(1)** `Cargo.lock` versionné, builds `--locked`, aucune dépendance `git`, `cargo deny check sources` en CI. **(2)** `cargo audit` + `cargo deny` (advisories, licences, doublons, wildcards) bloquants. **(3)** Revue de chaque `build.rs` et de chaque dépendance `proc-macro`. **(4)** Pas de contenu distant exécuté en root : le script de support est **embarqué dans le paquet** (versionné, signé par le paquet), ou exécuté **sans privilèges** et ne collecte que des informations lisibles par l'utilisateur ; si une mise à jour est nécessaire, elle passe par le gestionnaire de paquets. **(5)** Jamais de `main`/`master` non épinglée. |
| **Tests de non-régression** | CI : `cargo deny check` échoue sur git/wildcard/advisory ; test de packaging qui vérifie l'absence de `build.rs` non listé dans une allowlist ; revue manuelle trimestrielle de `cargo tree`. |

### S-17 — Attaquant physique / Evil Maid

| | |
|---|---|
| **Vecteur** | Modification de la configuration root-owned hors ligne (montage du disque depuis un live USB), remplacement du binaire dans `/usr/libexec`, ajout d'une unit systemd, modification de la politique D-Bus ou de l'action polkit. |
| **Impact** | Persistance, exécution root au boot, modification silencieuse des limites matérielles. A1/A2. |
| **Probabilité** | Faible (nécessite un accès physique), mais l'impact est total. |
| **Sévérité** | Moyen — **A** |
| **Mitigation attendue** | Défense en profondeur fournie par le système (Secure Boot, IMA, dm-verity, partition `/` chiffrée) — **hors de notre contrôle**, à documenter comme prérequis de sécurité. Côté projet : aucune configuration chargée depuis un chemin non root-owned, vérification propriétaire/mode à chaque lecture, aucune exécution de script depuis `/var` ou `/tmp`, et un `--check-config` en CLI qui signale un fichier non conforme. |
| **Tests de non-régression** | test « config appartenant à l'utilisateur → refus + log d'audit » ; test « politique D-Bus absente → le daemon démarre mais journalise un avertissement fort » ; documentation des prérequis. |

### S-18 — Fuite d'information par les logs, le diagnostic ou l'environnement

| | |
|---|---|
| **Vecteur** | Logs `tracing` contenant un profil complet, une variable d'environnement, un chemin de fichier utilisateur ; journald lisible par le groupe `systemd-journal` ; collecte `systeminfos` incluant des données sensibles puis envoi automatique. Dans l'original : lecture de `/proc/*/environ` de **tous** les processus non-root (`XDisplayRefreshRateController.ts:83-93`) — ce qui donne au daemon root accès aux secrets d'environnement de toute la session (tokens, clés d'API) et les expose à un bug de log. |
| **Impact** | Fuite de secrets d'autres applications, de données personnelles, de la topologie du système. A6. |
| **Probabilité** | Moyenne. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | **(1)** Ne jamais lire `/proc/*/environ` : l'agent de session connaît déjà `$DISPLAY`/`$XDG_SESSION_TYPE` par construction (`tccd-user` est lancé dans la session). **(2)** Liste blanche de champs loggés ; jamais de payload brut, jamais d'environnement, jamais de secret ; identifiants de profil oui, contenu non. **(3)** Niveau `debug`/`trace` désactivé en production (filtre `EnvFilter` non contrôlable par l'utilisateur dans l'unit). **(4)** Collecte de diagnostic : aperçu obligatoire, consentement explicite, aucune donnée sensible par défaut, envoi manuel, jamais en root. **(5)** `journald` : ne pas journaliser plus que nécessaire ; `ProtectProc=invisible` empêche de toute façon la lecture croisée. |
| **Tests de non-régression** | subscriber de test qui capture tous les événements d'une session d'exécution et échoue si un motif interdit apparaît (mot de passe, `KEY=`, `TOKEN=`, `/home/`) ; test que le niveau de log ne peut pas être élevé par D-Bus ; revue du script de diagnostic (liste des commandes exécutées). |

### S-19 — Hotplug webcam / udev

| | |
|---|---|
| **Vecteur** | La règle udev de l'original exécute `/usr/bin/python3 …/cameractrls.py -s $devnode,$env{ID_VENDOR_ID},…` à chaque ajout de nœud `video4linux` (`99-webcam.rules:1`) : exécution d'un interpréteur Python en root dans le contexte udev, avec des variables d'environnement interpolées dans argv, sans timeout, et un script qui lit `/etc/tcc/webcam`. Une webcam malveillante (ou un `video4linux` logiciel comme `v4l2loopback` créé par un utilisateur) déclenche le chemin. |
| **Impact** | Exécution root dans un contexte udev (bloque les événements suivants si le script est lent), dépendance Python dans la base de confiance, surface d'attaque inutile. A1. |
| **Probabilité** | Faible pour l'exécution arbitraire (chemins root-owned), moyenne pour le déni de service udev et l'application de réglages à un device non désiré. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | Aucun `RUN+=` avec interpréteur : utiliser un tag (`TAG+="systemd"`, `ENV{SYSTEMD_WANTS}=…`) ou, mieux, la détection netlink `NETLINK_KOBJECT_UEVENT` dans `tccd` (déjà prévue par `ARCHITECTURE.md` §3.7). Filtrer sur `ID_VENDOR_ID`/`ID_MODEL_ID` connus ou sur une propriété udev dédiée ; appliquer les réglages UVC depuis `tccd` via V4L2, avec validation des valeurs. |
| **Tests de non-régression** | `udevadm test` sur un device factice : aucune exécution de processus ; test unitaire du parseur d'événements netlink (trames tronquées, champs manquants, NUL) ; test « device inconnu → ignoré ». |

### S-20 — Signal PPD/UPower incohérent ou oscillant

| | |
|---|---|
| **Vecteur** | U7 (ou un utilisateur qui change PPD, ce qui est autorisé par conception) fait osciller `ActiveProfile` plusieurs fois par seconde ; UPower renvoie `OnBattery` puis `unknown` ; PPD disparaît. |
| **Impact** | Rechargement de profil en cascade → écritures sysfs répétées (usure, logs), ventilateurs en dents de scie, sur-consommation. A5. |
| **Probabilité** | Moyenne. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | Debounce 500–1000 ms (déjà au contrat) ; hystérésis sur l'état ; une seule application en vol ; journalisation des transitions avec anti-duplication ; fallback AC/BAT documenté ; jamais de panique si PPD/UPower est absent, inconnu, ou renvoie un type inattendu. |
| **Tests de non-régression** | test « 100 signaux en 1 s → une seule application » ; test « PPD absent → fallback stateMap » ; test « valeur `unknown` → pas d'oscillation » ; test de propriété : la séquence d'états produit toujours une application terminale. |

### S-21 — Redémarrage en boucle / indisponibilité auto-infligée

| | |
|---|---|
| **Vecteur** | Une erreur de configuration ou un matériel absent provoque un `panic` au démarrage ; `Restart=on-failure` sans `StartLimit` boucle indéfiniment. Ou le daemon refuse de démarrer si le bus est indisponible, laissant le matériel sans contrôle. |
| **Impact** | Bruit journald, CPU, et surtout absence de contrôle thermique (A5). |
| **Probabilité** | Moyenne. |
| **Sévérité** | Moyen — **I** |
| **Mitigation attendue** | `StartLimitIntervalSec`/`StartLimitBurst` ; dégradation gracieuse : le daemon démarre même si PPD, UPower, tuxedo_io ou l'agent de session sont absents ; état « non supporté » exposé à l'UI ; aucun `unwrap`/`expect` sur les chemins de production. |
| **Tests de non-régression** | test de démarrage avec bus absent, `/dev/tuxedo_io` absent, `/sys` partiel → démarrage réussi, état dégradé signalé ; test « 10 échecs consécutifs → unit en `failed`, pas de boucle ». |

---

## 6. Constats sur l'existant (code original)

Sévérité et statut selon la même échelle. Ces constats sont **la justification** des exigences de la section 8 ; ils ne sont pas des bugs de la réécriture.

| ID | Sév. | Statut | Fichier:ligne | Constat | Scénario |
|---|---|---|---|---|---|
| **F-01** | Élevé | B | `dist-data/com.tuxedocomputers.tccd-dbus.conf:11-14` | La politique par défaut autorise **tout** utilisateur à envoyer n'importe quel message au daemon et à recevoir ses signaux. Aucune restriction par interface ni par méthode. | S-01 |
| **F-02** | **Historique / hors périmètre** | B (original) | `e-app/backendAPIs/cryptAPI.ts:30-38` | Phrase de passe LUKS concaténée dans `pkexec /bin/sh -c "…"` : visible dans `/proc/<pid>/cmdline`, injection par apostrophe. **Vulnérabilité de l'original, non reproduite** : LUKS n'est pas implémenté. | S-10 (historique) |
| **F-03** | Élevé | B | `service-app/classes/ChargingWorker.ts:230-245` ; `TccDBusInterface.ts:376-383` | Seuils de charge acceptés depuis le bus **sans authentification ni clamp**, écrits directement dans sysfs ; retourne `true` même si l'écriture a échoué. | S-01, S-07 |
| **F-04** | Élevé | B | `TccDBusInterface.ts:300-309` ; `LightingListener.ts:234-268` | `SetKeyboardStatesJSON`/`SetLightbarStatesJSON` acceptent du JSON arbitraire d'un utilisateur non privilégié, `JSON.parse` sans schéma ni borne ; une exception casse le worker, les valeurs pilotent des écritures sysfs. | S-02, S-06 |
| **F-05** | Élevé | B | `TuxedoControlCenterDaemon.ts:941-958`, `:976-987` | Le daemon root lit un **chemin arbitraire** fourni en argument (`--new_profiles /tmp/x`) et l'écrit dans `/etc/tcc` sans validation ni vérification de propriétaire ; `getPathArgument` fait un `replace("'", '')` inutile (valeur de retour ignorée), indice d'une intention de quoting shell. | S-04, S-05 |
| **F-06** | Critique | B | `dist-data/com.tuxedocomputers.tccd-service.policy:18-25` | Action polkit unique (`auth_admin_keep`) autorisant l'exécution du binaire **`tccd` avec arguments arbitraires** en root, `allow_gui=true`. Primitive d'exécution root générique. | S-12 |
| **F-07** | Critique | B | `e-app/backendAPIs/systemInfosAPI.ts:30-31`, `:103-136`, `:156-169`, `:66` | (a) script téléchargé depuis une branche **mutable** de GitHub, écrit en root dans `/usr/libexec`, exécuté en root sans vérification de hash ni de signature ; (b) `ticketNumber` interpolé dans `sh -c` → injection root ; (c) `which tuxedo-systeminfo` résolu via le PATH utilisateur puis exécuté en root ; (d) `DISPLAY`/`XAUTHORITY` insuffisamment filtrés. | S-11, S-16 |
| **F-08** | Moyen | I | `common/classes/XDisplayRefreshRateController.ts:83-93`, `:70` | Lecture de `/proc/<pid>/environ` de tous les processus non-root par le daemon root (fuite potentielle de secrets d'autres applications), via un `sh -c` complexe ; parsing de `ls -l` pour deviner l'utilisateur. | S-18 |
| **F-09** | Moyen | B | `common/classes/ConfigHandler.ts:186-241` | Écritures non atomiques (`writeFileSync` direct), `mkdir` récursif 0755 sans vérification du propriétaire/mode existant, `mode` inopérant sur un fichier déjà présent, aucun `O_NOFOLLOW`, aucune vérification de propriétaire. | S-05 |
| **F-10** | Moyen | I | `TccDBusService.ts:38` (boucle 1,5 s) ; `TccDBusInterface.ts:97-105`, `:196-198` | Aucun rate limiting ; la collecte capteurs peut être maintenue indéfiniment par n'importe quel client ; méthodes coûteuses appelables en boucle. | S-09 |
| **F-11** | Moyen | I | `dist-data/99-webcam.rules:1` | Exécution de `python3` en root depuis udev à chaque `video4linux` ajouté, avec `$devnode`/`$env{…}` interpolés ; aucune borne de temps ; interpréteur Python dans la base de confiance. | S-19 |
| **F-12** | Faible | A | `dist-data/tccd-sleep.service:9-10` | `ExecStart=/bin/bash -c "systemctl stop tccd"` : shell inutile, `systemctl` résolu par PATH dans un service root. Utiliser `ExecStart=/usr/bin/systemctl stop tccd.service` ou un hook `systemd-sleep` natif. | S-11 |
| **F-13** | Faible | A | `service-app/classes/SingleProcess.ts:78-86` ; `TuxedoControlCenterDaemon.ts:251-259` | PID file écrit sans permissions explicites puis utilisé pour envoyer `SIGHUP` en root ; risque de PID recyclé. Préférer `sd_notify`/`systemctl reload`. | S-21 |
| **F-14** | Faible | A | `common/classes/TccPaths.ts:21-32` | `/var/log/tccd/log` (hors journald), `/var/run` au lieu de `/run`, `/opt` partout. | S-18 |
| **F-15** | Faible | A | `service-app/classes/TuxedoControlCenterDaemon.ts:97-99` | Refus si non-root : correct, mais c'est ce qui force le modèle `pkexec <daemon> <args>` (donc F-06). La séparation « daemon long-running » / « CLI root » est la cause racine. | S-12 |

**Ce qui est bien dans l'original et doit être conservé :** refus de tourner sans root côté daemon ; vérification de l'existence des chemins sysfs avant écriture (`SysFsPropertyIO.isAvailable`) ; distinction lecture/écriture par attribut ; `debounce` déjà présent dans certains workers ; politique de repli AC/BAT.

---

## 7. Constats sur le dépôt actuel (état du squelette)

Ces points ne sont pas des vulnérabilités mais des **risques organisationnels** : en l'absence de garde-fous, les mauvais choix de l'original seront reproduits par défaut.

| ID | Sév. | Statut | Constat | Recommandation |
|---|---|---|---|---|
| **G-01** | Élevé | B | Aucune politique D-Bus, aucune action polkit, aucune unit systemd dans le dépôt. Le contrat D-Bus (`tcc-ipc/src/lib.rs`) déclare `DBUS_SERVICE`/`DBUS_PATH`/`DBUS_INTERFACE` sans aucune notion d'autorisation ni de séparation lecture/écriture. | Ajouter `tcc-ipc` : constantes d'interfaces `Manager1`/`Admin1` **distinctes**, et un tableau `SENSITIVE_METHODS` exploité par les tests d'autorisation. Créer les fichiers d'intégration dès le jalon 2 (pas au jalon 6). |
| **G-02** | Élevé | B | Aucun `deny.toml`, aucun workflow CI, aucun `cargo audit`, aucun `SECURITY.md`. `Cargo.lock` contient 103 paquets non audités à ce jour. | Poser `deny.toml` + CI dès le jalon 1 (voir `CHECKLIST.md` §7). |
| **G-03** | Élevé | B | `docs/DEV_SETUP.md:70-72` prévoit `TCC_SYSFS_ROOT` / `TCC_CONFIG_DIR` injectables **sans restriction**. Dans un daemon root, une variable d'environnement contrôlée par l'appelant est une frontière de privilège : `TCC_CONFIG_DIR=/home/user/x` + symlink = écriture root arbitraire ; `TCC_SYSFS_ROOT` = écriture dans un faux sysfs. | Les overrides de développement doivent être derrière une feature Cargo `dev-overrides` **non compilée dans les paquets de release**, et refusés si `geteuid() == 0` ; l'unit systemd ne doit jamais les transmettre ; `Environment=` explicite et fermé. |
| **G-04** | Moyen | I | Aucune validation implémentée (`tcc-core/src/lib.rs` = 7 lignes) : toute la politique de validation est encore à écrire, et rien n'empêche un futur contributeur d'ajouter des `serde` permissifs. | `#![forbid(unsafe_code)]` + `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` dans `tcc-core`, `tcc-ipc`, `tcc-daemon`, `tcc-user` ; `trybuild`/`compile_fail` non nécessaire, mais `clippy.toml` avec `disallowed-methods` (`std::process::Command::new` interdit hors du module `exec` allowlisté). |
| **G-05** | Moyen | I | `tcc-gui` ne dépend que de `tcc-ipc` et n'a **aucune** couche de validation : la GUI devra être traitée comme hostile par le daemon. | Documenter dans `tcc-ipc` que « tout payload venant de la GUI est non fiable » ; tests d'intégration qui se connectent au daemon en se faisant passer pour la GUI. |
| **G-06** | Faible | A | `[profile.release]` : `strip = "symbols"` empêche l'analyse de crash ; pas de `panic = "abort"`. | Conserver `strip = "symbols"` mais produire un paquet `-dbg` séparé (`split-debuginfo`) ; envisager `panic = "abort"` pour `tccd` (réduit la surface d'unwinding à travers les frontières FFI) avec `Restart=on-failure`. |
| **G-07** | Faible | A | Aucun test de permissions de fichiers, aucun `deny(warnings)` documenté. | Voir `CHECKLIST.md` §6. |

---

## 8. Exigences et propositions techniques

### 8.1 Politique D-Bus (bus système)

Objectifs : (1) seul root possède le nom ; (2) les opérations destructrices ne sont **pas** atteignables par un utilisateur non privilégié même en cas de bug du daemon ; (3) les lectures restent ouvertes (GUI + agent).

**Architecture recommandée : séparation en deux services.**

- `com.tuxedocomputers.tccd` — `tccd`, root, **lecture + profil temporaire + collecte capteurs**. Aucune commande externe, aucun secret, aucune écriture `/etc`.
- `com.tuxedocomputers.tccd.Admin` — `tccd-admin`, root, **activé à la demande**, sans état persistant, uniquement : Prime, Tomte, WebFAI/paquets, diagnostic, assignation persistante de profil. Chaque méthode vérifie polkit. Le processus se termine après l'opération (`Type=oneshot` ou activation D-Bus avec `Restart=no`). **LUKS est hors périmètre.**

Bénéfice : un bug de parsing dans `tccd` (qui traite des messages en continu) ne donne jamais accès aux opérations administratives. C'est la mesure qui réduit le plus le risque résiduel.

Proposition pour le nom principal :

```xml
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
    "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>

  <!-- Seul root possède et émet sur le nom principal. -->
  <policy user="root">
    <allow own="com.tuxedocomputers.tccd"/>
    <allow own="com.tuxedocomputers.tccd.Admin"/>
    <allow send_destination="com.tuxedocomputers.tccd"/>
    <allow send_destination="com.tuxedocomputers.tccd.Admin"/>
    <allow receive_sender="com.tuxedocomputers.tccd"/>
    <allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
  </policy>

  <policy context="default">
    <!-- Personne d'autre ne peut prendre ces noms. -->
    <deny own="com.tuxedocomputers.tccd"/>
    <deny own="com.tuxedocomputers.tccd.Admin"/>

    <!-- Interface de lecture / profil temporaire : ouverte. -->
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="com.tuxedocomputers.tccd.Manager1"/>
    <allow receive_sender="com.tuxedocomputers.tccd"/>

    <!-- Introspection et lecture de propriétés uniquement. -->
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Introspectable"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Peer"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Properties"
           send_member="Get"/>
    <allow send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Properties"
           send_member="GetAll"/>
    <deny  send_destination="com.tuxedocomputers.tccd"
           send_interface="org.freedesktop.DBus.Properties"
           send_member="Set"/>

    <!-- Interface d'administration : joignable (polkit tranche ensuite),
         mais JAMAIS utilisable pour obtenir un fd arbitraire ou un nom. -->
    <allow send_destination="com.tuxedocomputers.tccd.Admin"
           send_interface="com.tuxedocomputers.tccd.Admin1"/>
    <allow receive_sender="com.tuxedocomputers.tccd.Admin"/>
  </policy>
</busconfig>
```

Notes de mise en œuvre :

- `<deny own>` explicite : la valeur par défaut du bus système est déjà « deny », mais l'explicite documente l'intention et survit à une politique globale permissive.
- La politique D-Bus ne peut **pas** exprimer polkit : elle filtre par nom/interface/membre, pas par identité autorisée. Elle ne remplace donc jamais le contrôle dans le daemon.
- Si l'option « service unique » est retenue (pas de `tccd-admin`), alors `Admin1` doit être **refusée** dans la politique pour tous sauf root, et les opérations sensibles passent par des actions polkit **sur le bus de session** via un helper lancé par l'UI — option moins bonne, car elle recrée un binaire root à arguments. À éviter.
- Ajouter `send_destination_prefix` n'est pas nécessaire : un nom exact évite les collisions de namespace.
- Vérifier la politique effective avec `busctl introspect` + `dbus-send` en tant qu'utilisateur non privilégié, et avec `busctl monitor` (root) pour confirmer qu'aucun `Set` non autorisé ne passe.

### 8.2 Polkit

Règles :

1. **Aucune** action `org.freedesktop.policykit.exec.*`. Pas de `pkexec`. Pas d'annotation `exec.path`.
2. Une action par opération, ID stable et versionné : `com.tuxedocomputers.tccd1.<verbe>`.
3. `allow_any=no`, `allow_inactive=no` par défaut ; `allow_active` selon le risque.
4. `auth_admin` (sans `_keep`) pour Prime, Tomte, WebFAI/paquets, diagnostic ; `auth_admin_keep` uniquement pour l'assignation de profil (fréquente, non destructive). **Aucune action LUKS.**
5. Vérification côté daemon avec sujet `system-bus-name` = nom unique de l'appelant (voir S-12).
6. Un refus polkit doit produire `org.freedesktop.DBus.Error.AccessDenied` **sans** divulguer la raison exacte.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC
 "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
 "https://specifications.freedesktop.org/PolicyKit/1.0/policyconfig.dtd">
<policyconfig>
  <vendor>TuxedoRustCenter</vendor>
  <vendor_url>https://github.com/fnyaker/TuxedoRustCenter</vendor_url>

  <action id="com.tuxedocomputers.tccd1.assign-profile">
    <description>Assign a profile to a power state</description>
    <description xml:lang="fr">Assigner un profil à un état d'alimentation</description>
    <message>Authentication is required to change the automatic profile assignment</message>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_admin_keep</allow_active>
    </defaults>
  </action>

  <!-- LUKS hors périmètre : aucune action `luks-change-key` ne doit être déclarée. -->

  <action id="com.tuxedocomputers.tccd1.prime-select">
    <description>Change the graphics mode (PRIME)</description>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_admin</allow_active>
    </defaults>
  </action>

  <action id="com.tuxedocomputers.tccd1.package-install">
    <description>Install or remove a support package</description>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_admin</allow_active>
    </defaults>
  </action>
</policyconfig>
```

Vérification : `pkcheck --action-id com.tuxedocomputers.tccd1.prime-select --process $$` (doit demander une authentification), `grep -r exec.path /usr/share/polkit-1/actions/` (doit ne rien retourner pour nos actions). **Aucun test LUKS** (action absente).

### 8.3 Durcissement systemd

**Exceptions matérielles à documenter explicitement** (ce sont les pièges classiques) :

| Option | Valeur | Raison |
|---|---|---|
| `ProtectKernelTunables` | **no** (obligatoire) | passe `/proc/sys` **et `/sys`** en lecture seule → casse toute écriture sysfs (ventilateurs, TDP, charge). C'est l'option la plus souvent proposée qui casserait tout. |
| `PrivateDevices` | **yes** + `BindPaths=-/dev/tuxedo_io` + `DeviceAllow=char-tuxedo_io rw` | nécessaire pour l'ioctl ; le `-` rend le bind optionnel sur les machines sans `tuxedo_io`. Repli documenté si le bind échoue sur la version de systemd cible : `PrivateDevices=no` + `DevicePolicy=closed` + `DeviceAllow=`. |
| `ProtectSystem` | **strict** | `/sys` n'est pas concerné (seuls `/usr`, `/boot`, `/efi`, `/etc` deviennent RO). `/etc` RO implique que le daemon **n'écrit pas** `/etc` (cf. décision D2). |
| `RestrictAddressFamilies` | `AF_UNIX AF_NETLINK` | `AF_UNIX` pour D-Bus, `AF_NETLINK` pour udev. Aucun accès réseau. |
| `MemoryDenyWriteExecute` | **yes** pour `tccd` et `tccd-user` (Rust, pas de JIT) ; **jamais** pour `tcc-gui` (Qt/QML) | |
| `SystemCallFilter` | `@system-service` + ajout explicite de `ioctl` si le filtre le retire | à vérifier avec `strace -f -e trace=ioctl` et un test matériel ; documenter le résultat. |

Proposition pour `tccd.service` :

```ini
[Unit]
Description=TuxedoRustCenter privileged daemon
Documentation=man:tccd(8)
After=dbus.service upower.service
Wants=power-profiles-daemon.service
Requires=dbus.service
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=notify
NotifyAccess=main
ExecStart=/usr/libexec/tuxedo-rust-center/tccd
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure
RestartSec=2s
WatchdogSec=30s

User=root
Group=root
UMask=0022
LimitCORE=0

NoNewPrivileges=yes
CapabilityBoundingSet=
AmbientCapabilities=
SecureBits=noroot-locked

ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
DevicePolicy=closed
DeviceAllow=char-tuxedo_io rw
BindPaths=-/dev/tuxedo_io

ProtectKernelTunables=no
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
ProcSubset=pid

RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallFilter=~@mount @swap @obsolete @cpu-emulation @debug @keyring @module @raw-io
SystemCallErrorNumber=EPERM
RestrictAddressFamilies=AF_UNIX AF_NETLINK
RemoveIPC=yes
KeyringMode=private

ReadOnlyPaths=/etc/tuxedo-rust-center
ReadWritePaths=/var/lib/tuxedo-rust-center
StateDirectory=tuxedo-rust-center
StateDirectoryMode=0750
RuntimeDirectory=tuxedo-rust-center
RuntimeDirectoryMode=0755
CacheDirectory=tuxedo-rust-center

[Install]
WantedBy=multi-user.target
```

Proposition pour `tccd-user.service` (session, **jamais root**) :

```ini
[Unit]
Description=TuxedoRustCenter session agent (display refresh rate)
PartOf=graphical-session.target
After=graphical-session.target
Requisite=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-user
Restart=on-failure
RestartSec=2s
UMask=0077

NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=%t
ProtectHome=read-only
PrivateTmp=no            # exception : socket X11 dans /tmp/.X11-unix
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
ProtectProc=invisible
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallFilter=@system-service
RestrictAddressFamilies=AF_UNIX
CapabilityBoundingSet=
AmbientCapabilities=

[Install]
WantedBy=graphical-session.target
```

Points de vigilance :

- `ProtectHome=read-only` suffit pour `XAUTHORITY` ; ne pas mettre `ProtectHome=yes` si le backend X11 doit lire un fichier d'autorisation dans `$HOME`.
- `%t` (runtime dir) est nécessaire : `ProtectSystem=strict` rend `/run` RO.
- Le service doit être **utilisateur** (`systemctl --user`), jamais `User=` dans une unit système.
- `LimitCORE=0` pour `tccd-admin` (règle D5 historique, applicable à tout futur secret ; aucune fonction LUKS).
- `systemd-analyze security tccd.service` doit être exécuté et le score archivé ; toute régression > 0,5 doit être justifiée.

### 8.4 Validation des entrées

Règles générales applicables à `tcc-core` :

1. **Refuser, ne pas corriger.** Une valeur hors plage est une erreur typée (`ValidationError::OutOfRange { field, value, min, max }`), jamais un clamp silencieux.
2. **Taille avant contenu** : refuser un message/fichier trop gros avant de parser.
3. **Chaînes** : longueur maximale par champ, rejet de `\0`, des caractères de contrôle (`char::is_control`, sauf `\n`/`\t` dans les descriptions si l'UI les gère), normalisation NFC pour les noms affichés, pas de normalisation pour les identifiants.
4. **Identifiants** : `^[A-Za-z0-9_-]{1,64}$`, unicité vérifiée, jamais réutilisés après suppression via l'API : tombstones `deletedProfileIds` (≤ 1024, permanents), refus de recréation et transaction double fichier (`ADMIN_DESIGN.md` AD-13/§4.2.1).
5. **Nombres** : plages issues du matériel quand disponible, sinon constantes documentées avec leur source ; `f32`/`f64` refusés si `!is_finite()`.
6. **Énumérations** : désérialisation en `enum` avec `#[serde(other)]` **interdit** pour les champs de sécurité (un `other` silencieux transforme une valeur inconnue en comportement par défaut) ; utiliser `#[serde(deny_unknown_fields)]` là où c'est compatible avec la migration.
7. **Invariants croisés** : vérifiés après le parsing de l'objet complet, avant toute écriture.
8. **Capacité** : toute écriture est conditionnée à la détection préalable de l'attribut ; l'absence de capacité est un état « non supporté », pas une erreur fatale.

Table minimale des bornes (à compléter par `linux-platform` avec les sources driver) :

| Champ | Borne | Source |
|---|---|---|
| `display.brightness` | `0..=max_brightness` lu dans sysfs | `/sys/class/backlight/*/max_brightness` |
| `display.refresh_rate_hz` | fini, `1.0..=1000.0`, présent dans les modes de l'output | XRandR / compositeur |
| `cpu.online_cores` | `1..=num_present` | `/sys/devices/system/cpu/present` |
| `cpu.scaling_min/max_frequency` | dans `scaling_available_frequencies`, min ≤ max | cpufreq |
| `cpu.governor` | ∈ `scaling_available_governors` | cpufreq |
| `charge.start_threshold` | `0..=100` **et** `start < end` | ABI power_supply |
| `charge.end_threshold` | `0..=100` **et** `end > start` | ABI power_supply |
| `odm_power_limits.pl1/pl2` | `min..=max` du device, `pl1 ≤ pl2` | driver ODM |
| `fan.curve` | ≤ 64 points, temp `0..=120` °C croissante, vitesse `0..=255` ou `0..=max_pwm` | hwmon/`tuxi` |
| `nvidia.ctgp_offset` | `0..=max_offset` | `tuxedo_nvidia_power_ctrl` |

### 8.5 Écritures système et fichiers

- **Config au repos** : répertoire `0755 root:root`, fichiers `0644 root:root`. Vérification à chaque lecture : type régulier, `st_uid == 0`, `st_mode & 0o022 == 0`. Un fichier non conforme est **refusé** avec un log d'audit, pas silencieusement accepté.
- **Écriture atomique** : `O_TMPFILE` dans le répertoire cible (ou nom aléatoire `.<name>.tmp.<random>`), écriture, `fsync`, `fchmod(0644)`, `fchown(0,0)`, `renameat` vers la cible, `fsync` du répertoire. En cas d'erreur, le fichier temporaire est supprimé et l'ancien reste intact.
- **Ouverture** : `openat2(..., RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV)` ; repli `openat` + `O_NOFOLLOW|O_CLOEXEC` composant par composant si le noyau est < 5.6 (à documenter, avec un test qui détecte le noyau).
- **Sysfs** : chemins constants ou gabarits + index entier ; jamais de `Path::join` avec une chaîne utilisateur ; après ouverture, `readlink("/proc/self/fd/N")` doit commencer par `/sys/` ; écriture unique par `open` (certains attributs ne supportent qu'un `write` par fd) ; erreur `EINVAL`/`EPERM` traduite, jamais ignorée.
- **Ne jamais** utiliser `std::fs::write`/`File::create` sur un chemin fourni par l'appelant. `clippy.toml` : `disallowed-methods` pour `std::fs::write`, `std::fs::File::create`, `std::process::Command::new` hors modules autorisés.
- **Migration** : lire `/etc/tcc/*`, copier vers `/etc/tuxedo-rust-center/` (ou `/var/lib/…`, cf. D2), conserver une sauvegarde `.bak` root-owned 0600, ne jamais écraser un fichier existant sans confirmation.

### 8.6 LUKS — hors périmètre

> **LUKS est hors périmètre (décision utilisateur), non implémenté.** Aucune découverte de volume,
> aucune vérification de phrase, aucun `cryptsetup`, aucun transport de secret par fd, aucune action
> polkit `luks-change-key`, aucun service `tccd-admin-luks`. D5 reste la règle historique pour tout
> futur secret (jamais argv/env/log/core, `zeroize`, `LimitCORE=0`), mais aucune fonctionnalité LUKS
> n'est livrée. Voir `ADMIN_PHASE3_DESIGN.md`.

### 8.7 Commandes externes

Allowlist (à compléter avec `linux-platform`) :

| Binaire | Chemin absolu | Usage | Contraintes |
|---|---|---|---|
| `prime-select` | `/usr/bin/prime-select` | mode graphique | argument ∈ `{intel, nvidia, on-demand}` |
| `nvidia-smi` | `/usr/bin/nvidia-smi` | lecture GPU | lecture seule, timeout 2 s, sortie bornée |
| `tuxedo-tomte` | `/usr/bin/tuxedo-tomte` | modules | sous-commande ∈ allowlist, argument ∈ liste de modules, jamais de `yes |` |
| `kscreen-doctor` | `/usr/bin/kscreen-doctor` | fallback KDE | **dernier recours**, argument construit depuis des composants validés |
| `systeminfos` | *(embarqué dans le paquet)* | diagnostic | **non root**, données montrées avant envoi |

Règles : `Command::new(abs)` + `args()` ; `env_clear()` puis `envs([("PATH", "/usr/sbin:/usr/bin:/sbin:/bin"), ("LANG", "C.UTF-8")])` ; `stdin(Stdio::piped())` ; timeout (`tokio::time::timeout`, puis `kill`) ; taille de sortie plafonnée (1 Mio) ; code de sortie et `stderr` traduits ; aucun `sh`, `bash`, `env`, `xargs`, `yes`, `which`. Interdiction d'exécuter un fichier dont le chemin provient d'une recherche PATH ou d'un téléchargement.

### 8.8 Rate limiting et anti-DoS

- Token bucket par **nom unique** d'expéditeur, rechargé à la connexion, détruit à `NameOwnerChanged`.
- Trois classes : lecture (50/s, burst 100), écriture (5/s, burst 10), admin (1/10 s). Dépassement → `org.freedesktop.DBus.Error.LimitsExceeded`.
- Plafond global d'appels concurrents (`Semaphore`, ex. 32) ; file bornée ; rejet immédiat si saturé (pas de mise en attente infinie).
- Taille maximale de message : 1 Mio (JSON de compatibilité), 64 Kio (API typée), 4 Kio (nom de profil).
- Collecte capteurs : compteur par client, arrêt automatique à la déconnexion, intervalle minimal 1 s, arrêt total quand aucun client.
- `CPUQuota=25%`, `MemoryMax=256M`, `TasksMax=64` dans l'unit (budgets `ARCHITECTURE.md` §3.11 : < 1 % CPU, < 30 Mo RSS en veille).

### 8.9 Logs sans secrets

- `tracing` avec champs structurés ; jamais de payload brut, jamais d'environnement, jamais de secret.
- Niveau par défaut `info` ; `debug`/`trace` uniquement via `RUST_LOG` **non exposé** dans l'unit (sinon n'importe quel utilisateur capable de modifier l'environnement du service — ce qui suppose root — pourrait élargir les logs ; le vrai risque est la fuite accidentelle en production).
- Identifiants de profil et codes d'erreur oui ; contenu de profil non.
- Journalisation d'audit dédiée pour les actions sensibles : `action`, `caller_unique_name`, `caller_uid`, `result` (jamais le secret, jamais la valeur brute si elle est sensible).
- Aucun fichier de log applicatif : journald uniquement (`StandardOutput=journal`).

### 8.10 Refresh rate — exigences spécifiques Wayland

1. **`tccd-user` n'est jamais root.** Au démarrage : si `geteuid() == 0`, refuser de continuer (code de sortie non nul, message explicite). Test unitaire dédié.
2. **Aucune commande arbitraire.** L'agent ne reçoit ni chaîne shell, ni nom d'output, ni mode. Il lit le profil actif (bus système), applique le taux demandé, et rien d'autre.
3. **Filtrage de l'output interne** : `^(eDP|LVDS|DSI)-[0-9]+$` **et** recoupement avec le type de connecteur DRM (`/sys/class/drm/card*-eDP-*`) ; jamais « premier output », jamais un output dont le nom vient du client.
4. **Validation des modes** : le taux doit exister dans la liste annoncée par le compositeur pour cet output et la résolution courante ; tolérance 0,5 Hz ; refus sinon, avec état d'erreur exposé à l'UI.
5. **Pas d'écriture persistante** dans la configuration du compositeur : GNOME `ApplyMonitorsConfig(method=1)` (temporaire), KDE via l'API KScreen sans sauvegarde, Hyprland/Sway via IPC uniquement.
6. **Hyprland** : socket dans `$XDG_RUNTIME_DIR` — vérifier que `$XDG_RUNTIME_DIR` appartient à l'uid courant et est en 0700, que le socket n'est pas un symlink, que l'`INSTANCE_SIGNATURE` correspond ; commande construite par concaténation de composants validés (nom d'output allowlisté, mode issu de `availableModes`), jamais de texte libre.
7. **Sway/wlroots** : `zwlr_output_manager_v1` préféré à l'IPC texte ; sinon même logique de construction validée.
8. **Session** : ne pas appliquer si verrouillée/absente (logind `LockedHint`, `Active`) ; différer et réessayer sur le prochain signal ; jamais bloquer le daemon.
9. **Idempotence** : ne rien faire si le mode effectif est déjà correct ; une seule application en vol par output.
10. **Arrêt** : ne pas restaurer agressivement l'ancien mode (source de conflits avec le compositeur) ; documenter le choix.

### 8.11 Supply chain

- `Cargo.lock` versionné, `cargo build --locked`, `cargo deny check` et `cargo audit` **bloquants** en CI.
- `deny.toml` : `unknown-git = "deny"`, `wildcards = "deny"`, `yanked = "deny"`, liste de licences compatible GPL-3.0-or-later, `multiple-versions = "warn"`.
- Revue manuelle de chaque `build.rs`, de chaque `proc-macro` et de chaque dépendance `unsafe` ; objectif à terme : `cargo vet` avec un ensemble d'audits pour les crates critiques (`zbus`, `zvariant`, `serde`, `tokio`, `zeroize`).
- Pas de contenu distant exécuté ; pas de branche mutable ; si un script de diagnostic doit être mis à jour, il passe par le paquet (signé/versionné) ou est exécuté sans privilèges.
- SBOM généré à chaque release (`cargo cyclonedx` ou équivalent) et archivé.
- `#![forbid(unsafe_code)]` dans `tcc-core`, `tcc-ipc`, `tcc-gui`, `tcc-cli` ; `unsafe` isolé et documenté dans `tcc-hw` uniquement.

---

## 9. Risque résiduel et décisions à arbitrer

| # | Risque résiduel | Niveau | Acceptation |
|---|---|---|---|
| R1 | Un utilisateur autorisé peut toujours modifier **ses** réglages matériels (charge, TDP) — c'est la fonction du produit | Accepté | À documenter : ce n'est pas une frontière entre utilisateurs |
| R2 | Root compromis peut tout faire | Accepté | Hors périmètre |
| R3 | Compromission d'une dépendance non auditée malgré `cargo audit` | Moyen | Réduit par `cargo vet` + revue ; accepté avec surveillance |
| R4 | Compositeur Wayland exotique non supporté | Faible | État « non supporté » explicite, pas de fallback dangereux |
| R5 | Attaquant physique | Moyen | Dépend de Secure Boot/IMA, hors périmètre projet |

### Décisions à trancher (arbitrage `linux-platform` + `security`)

| ID | Question | Recommandation `security` | Impact si non tranché |
|---|---|---|---|
| **D1** | Séparer les opérations sensibles dans `tccd-admin` (bus name distinct) ou tout garder dans `tccd` ? | **Séparer.** Réduction majeure de la surface : Prime/Tomte/WebFAI/diagnostic inaccessibles depuis le daemon qui parse des messages en continu. | Un bug de parsing dans `tccd` devient une primitive root sur les opérations admin |
| **D2** | Les profils/settings modifiables par la GUI vont-ils dans `/etc/tuxedo-rust-center` ou `/var/lib/tuxedo-rust-center` ? | **`/var/lib`** pour l'état mutable (root:root 0644), `/etc` pour la configuration administrateur en lecture seule. Permet `ProtectSystem=strict` sans `ReadWritePaths=/etc`. Si la parité impose `/etc`, alors `ReadWritePaths=/etc/tuxedo-rust-center` + écritures atomiques, et le risque est documenté. | Soit on écrit dans `/etc` depuis un daemon (mauvais), soit on garde `/etc` RO et il faut déplacer les fichiers |
| **D3** | Conserver la couche de compatibilité JSON historique ? | Oui, mais **gelée** : lecture seule + un seul setter (`SetTempProfileByName`) si nécessaire, taille bornée, aucun nouveau champ. | Surface de parsing non typée permanente |
| **D4** | Politique PPD/TCC sur `platform_profile`/EPP | PPD propriétaire par défaut (déjà acté) ; TCC n'écrit qu'avec une option explicite, journalisée. | Conflit de writers → valeurs imprévisibles, difficile à auditer |
| **D5** | **Règle historique pour tout futur secret** (aucune fonction LUKS) : `s` ou `h` ? | **`h`** (memfd) — le secret ne traverse pas le message marshallé ; `zeroize`, `LimitCORE=0`. | Si `s` : présence en clair dans la mémoire du bus |
| **D6** | Le daemon lit-il `/proc/*/environ` ? | **Jamais.** L'agent de session fournit `$DISPLAY`/`$XDG_SESSION_TYPE`. | Fuite de secrets d'autres applications + complexité |
| **D7** | Env overrides de développement (`TCC_SYSFS_ROOT`, `TCC_CONFIG_DIR`) | Feature Cargo `dev-overrides` désactivée en release, refus si `euid == 0`. | Écriture root dans des chemins choisis par l'appelant |

---

## 10. Plan d'audit et critères de sortie

### 10.1 Outils

| Outil | Cible | Fréquence | Bloquant |
|---|---|---|---|
| `cargo fmt --all --check` | style | chaque PR | oui |
| `cargo clippy --workspace --all-targets -- -D warnings` | qualité | chaque PR | oui |
| `cargo test --workspace` | logique | chaque PR | oui |
| `cargo deny check` | licences, sources, advisories, doublons | chaque PR | oui |
| `cargo audit` | RUSTSEC | chaque PR + quotidien | oui |
| `cargo fuzz` (cibles §10.3) | parseurs | nightly court + campagne avant release | oui avant release |
| `cargo geiger` / revue `unsafe` | `tcc-hw` | à chaque ajout d'`unsafe` | oui |
| `systemd-analyze security` | units | à chaque modification d'unit | oui (score archivé) |
| `systemd-analyze verify` | units | idem | oui |
| `busctl`/`dbus-send` (tests d'autorisation) | D-Bus | chaque PR | oui |
| `pkcheck` | polkit | avant release | oui |
| `udevadm test` | règle udev (si conservée) | avant release | oui |
| `strace` ciblé | appels système du daemon (vérifier `openat2`, absence de `execve` inattendu) | avant release | non (revue) |
| `diffoscope` / rebuild | reproductibilité | avant release | non |

### 10.2 Tests de non-régression obligatoires (localisation proposée)

| Fichier | Contenu |
|---|---|
| `crates/tcc-core/tests/validate_profile.rs` | corpus valide/invalide, bornes, invariants croisés |
| `crates/tcc-core/tests/config_atomicity.rs` | symlink, répertoire 0777, crash simulé, permissions finales |
| `crates/tcc-core/tests/migration.rs` | rejeu de vrais `/etc/tcc/*`, idempotence, sauvegarde |
| `crates/tcc-core/tests/no_panic.rs` | `proptest` sur tous les parseurs |
| `crates/tcc-ipc/tests/dbus_types.rs` | signatures, désérialisation hostile, taille max |
| `crates/tcc-daemon/tests/dbus_authz.rs` | matrice méthode × rôle (root, user, inconnu) |
| `crates/tcc-daemon/tests/polkit.rs` | sujet `system-bus-name`, refus, cache invalidé |
| `crates/tcc-daemon/tests/rate_limit.rs` | rafales, stabilité RSS, réactivité après rejet |
| `crates/tcc-daemon/tests/no_exec.rs` | aucune `execve` hors allowlist (via `strace` en CI ou mock de `Command`) |
| `crates/tcc-hw/tests/sysfs_bounds.rs` | sysfs factice, aucune écriture hors plage, chemins confinés |
| `crates/tcc-user/tests/display_filter.rs` | filtre connecteur, modes, refus en root, idempotence |
| `crates/tcc-gui/tests/no_privilege.rs` | la GUI n'appelle aucune méthode `Admin1` sans polkit, ne lit aucun fichier root |

### 10.3 Cibles de fuzzing

- `parse_profile` (JSON profil complet, y compris migration `refreshRate: -1`) ;
- `parse_settings` ;
- `dbus_payload` (désérialisation `zvariant` des structs `Manager1`/`Admin1`) ;
- `fan_curve` (liste de points, min/max/offset) ;
- `display_modes` (parsing XRandR/compositeur) ;

Corpus : fichiers réels de `tuxedo-control-center-master` + générés ; conserver les crashs comme tests de régression.

### 10.4 Critères de sortie avant packaging

1. Tous les points **B** de `CHECKLIST.md` cochés.
2. Aucun finding **Critique** ou **Élevé** ouvert.
3. `systemd-analyze security tccd.service` exécuté, score documenté et justifié.
4. Test d'autorisation D-Bus passé en tant qu'utilisateur non privilégié sur une VM réelle.
5. **Test LUKS : non applicable** (hors périmètre, aucune fonction LUKS).
6. `cargo deny`/`audit` verts, SBOM généré, `Cargo.lock` inchangé pendant le build.
7. Revue `unsafe` signée (liste des blocs, justification, tests).
8. Threat model relu et versionné ; `docs/security/CHECKLIST.md` signée par les agents concernés.

---

## Delta jalon 2 (2026-11-06)

> **Instantané ~15:30, dépôt en mouvement.** Cette section **ajoute** au modèle d'origine (§1-§10)
> sans le réécrire : les scénarios S-01 à S-21 restent valides. Elle décrit la surface **réellement
> présente** au jalon 2, y compris ce qui est implémenté mais **non activé par le paquet**.
> Preuves rejouées par `security` : lecture du code, `validate.sh --repo` (**220/220**),
> `cargo test -p tcc-authz` (**19/19**), `cargo test -p tcc-admin` (lib **24**, `admin_dbus`
> **7/7 en parallèle**, CLI **4**, transaction **7**), `tcc-daemon` bins (**114**), `tcc-core`
> (lib **159**), `systemd-analyze security --offline=true` (tccd **3.3**, tccd-admin **1.6**,
> tccd-user **3.6**).
> **Non rejoués** : `dbus-policy-test.sh` (root requis), polkit réel, matériel TUXEDO,
> `lintian`/`rpmlint`/`namcap`, `cargo deny`/`audit`.

### 1. Nouveaux actifs

| # | Actif | Emplacement / preuve | Impact si compromis |
|---|---|---|---|
| **A10** | **Configuration persistante** : profils, réglages, courbes de ventilation | `/var/lib/tuxedo-rust-center/{profiles,settings,fantables}.json`, 0644 root, `.bak` 0600 (`tcc-core/src/config.rs:68,925-945`) | Un profil piégé appliqué au boot : charge désactivée, ventilateurs bloqués, TDP max (A1/A2) |
| **A11** | **État matériel global** : charge, LED, `platform_profile`, TDP PL1/PL2/PL4, cTGP, `fn_lock`, `force_yuv420_output` | `tcc-hw/src/{charging,lighting,platform_profile,tdp,nvidia,fn_lock,ycbcr}.rs`, workers `tcc-daemon/src/hardware/**` | Modification du matériel de **tous** les utilisateurs de la machine (A1) |
| **A12** | **Nom et contrat d'administration** `com.tuxedocomputers.tccd.Admin` / `Admin1` | `tcc-ipc/src/lib.rs:82-110`, `tcc-ipc/src/admin.rs` | Usurpation → phishing d'authentification polkit, faux succès d'écriture (A9) |
| **A13** | **Configuration polkit et unit admin** | `packaging/polkit/…policy` (9 actions), `packaging/systemd/tccd-admin.service` | Un `exec.path`/`allow_gui` ou une action trop permissive transformerait polkit en primitive root (S-12) |
| **A14** | **Secrets LUKS — hors périmètre** (aucun actif livré ; D5 reste la règle historique pour tout futur secret) | non implémenté | Divulgation d'une phrase de passe = perte totale de confidentialité (A3) | **Hors périmètre** |
| **A15** | **Budget capteurs/CPU** (bail, `nvidia-smi`) | `tcc-daemon/src/sensor_lease.rs:28-30`, `tcc-hw/src/nvidia.rs:49-52` | DoS énergétique/thermique par collecte permanente (A5) |
| **A16** | **Mode d'affichage persistant** (KDE `kscreen-doctor`) | `tcc-user/src/backend/kde.rs:9` (`output.<name>.mode.<id>`) | Mode non désiré persistant après session (A4) — écart §8.10.5 |

### 2. Nouvelles surfaces d'attaque (état réel)

| Surface | Accessible à | Contrôles **présents** | Statut |
|---|---|---|---|
| **Legacy `com.tuxedocomputers.tccd`** (lecture + `SetTempProfile`/`ClearTempProfile`) | tout utilisateur local | rate limit 5/s burst 10 par **nom unique**, audit `caller`/`caller_uid`/`profile_id`, id ≤ 64 o (`tcc-daemon/src/dbus.rs:129-200`) ; **pas de polkit** ; portée **globale** (machine, pas session) | **Partiel** : fonctionne comme prévu, mais modification matérielle inter-utilisateurs (R-03) |
| **`Manager1` — lectures** | tout utilisateur local | `GetHardwareSnapshotJSON` 2/s burst 5 (`rate_limit.rs:26-28`), `GetSensorDataCollectionStatus` gratuit ; JSON non fiable côté client | Vérifié |
| **`Manager1` — bail capteurs** | tout utilisateur local | `SetSensorDataCollectionStatus` passe par le bucket d'écriture (`dbus.rs:546`) ; bail 10 s, **plafond 5 min** (`sensor_lease.rs:28-30`), réarmement par lecture soumis au bucket ; cache `nvidia-smi` 10 s (`nvidia.rs:49-52`) | **Vérifié** (H-02 fermé) |
| **`Manager1` — 8 setters matériels** | tout utilisateur local **au niveau bus** | Présents dans le binaire (`dbus.rs:571-735`) mais **refusés par la policy** (`packaging/dbus/…conf:111-138`) ; aucun `CheckAuthorization` dans `tccd` (`tccd` ne dépend pas de `tcc-authz`) | **Fermé par la policy**, pas par l'autorisation : toute réouverture accidentelle exposerait des setters non authentifiés (H-05/H-10) |
| **`Control1.ReloadConfig`** | **root uniquement** | Policy : allow `user="root"`, deny explicite en default (`conf:39-42,199-206`) ; `tccd` re-vérifie `GetConnectionUnixUser == 0` (`dbus.rs:945-995`) ; appels **sérialisés sans cache** (chaque appel recharge réellement, A1-01) ; rapport **sans chemin ni contenu** (`main.rs:450-454`) | **Vérifié** (code + tests `Control1`) |
| **`Admin1` phase 1** (`SaveProfilesJSON`, `SaveSettingsJSON`, `ReloadConfig`) | tout utilisateur local au niveau bus, **polkit tranche** | Implémenté dans `tcc-admin` : polkit `system-bus-name`, `AllowUserInteraction=true` par défaut (`tcc-authz/src/lib.rs:106-115`), payload ≤ 1 Mio (`tcc-admin/src/transaction.rs:33`), validation + références croisées + filtrage des presets (`service.rs:394,423,448`), transaction `.bak` **conditionnelle** + rollback si le précédent est exploitable (`transaction.rs`, A1-07), rate limit par nom **et UID** (`tcc-authz AuthorizationLimiter`), dialogue unique (`DialogGate`), sortie après 30 s sans appel en cours (`service.rs:39`), unit non installée par défaut (`TCC_WITH_ADMIN=0`) | **Implémenté, vérifié en test unitaire** (`admin_dbus` 7/7 en parallèle, `transaction` 7) ; **aucun test de bout en bout avec le vrai polkit système** ; `Control1` réel non exercé (voir §4) |
| **`tcc-authz`** (client polkit) | appelé par `tcc-admin` uniquement | Sujet `system-bus-name` (`lib.rs:295-302`), flags `1`, `details={}`, timeout borné, `is_challenge=true` **jamais** autorisé, **fail-closed** si polkit absent/erreur/timeout (`lib.rs:15,163-213`) ; **19 tests** | **Vérifié en test**, pas en conditions réelles |
| **Bail capteurs** | tout utilisateur local | voir ci-dessus | Vérifié |
| **Commandes externes** (`nvidia-smi`, `prime-select`, `dpkg-query`) | `tccd` (lecture) ; `tccd-admin` (phase 2/3, futur) | allowlist de chemins absolus typés `&'static str`, `env_clear()` + `PATH`/`LANG` (`command.rs:150-160`), args séparés, timeout + `READ_GRACE` sur les lecteurs (`command.rs:173-193,242-253`), sortie 64 Kio, erreurs nettoyées ; cache `nvidia-smi` 10 s | **Vérifié** (`tcc-hw` 182 tests) |
| **Écritures sysfs/debugfs** | `tccd` root | `SysfsRoot` : chemins relatifs sans `..`, `O_NOFOLLOW`, `fstat`, propriétaire root, valeur unique ≤ 4 Kio, rejet des contrôles (`sysfs.rs`) ; `force_yuv420_output` sous `kernel/debug/dri/<0..=16>/<port>` validé (`ycbcr.rs:81-115`) ; `ProtectKernelTunables=no` requis | **Vérifié statiquement**, **non validé matériellement** (H-13) |
| **Packaging** | système | `tccd-admin` non installé sans `TCC_WITH_ADMIN=1` + binaire (`install-tree.sh:40-81`), unit phase 1 à capacités **vides**, `PrivateDevices=yes`, `ProtectKernelTunables=yes`, `MemoryMax=192M`, `TasksMax=16`, `Restart=no`, pas de `[Install]` (`tccd-admin.service`) ; `tccd-sleep` retiré ; `DeviceAllow=/dev/tuxedo_io` + `ProtectKernelTunables=no` pour `tccd` (`tccd.service:64-71`) | Vérifié statiquement ; **`DeviceAllow`/debugfs non testés sur matériel** |

### 3. Attaquants et scénarios (delta)

| # | Scénario | Acteur | Effet | Parade actuelle | Statut |
|---|---|---|---|---|---|
| **D-01** | Profil temporaire imposé à la machine (CPU, luminosité, matrice) | U1/U3 | Matériel de tous les utilisateurs modifié, sans persistance | rate limit + audit ; **pas de polkit**, portée globale | **Partiel** (R-03) |
| **D-02** | N connexions D-Bus pour contourner le bucket par nom unique | U1/U3 | N×10 écritures immédiates ; `cleanup()` O(n) sous mutex | `MAX_TRACKED_CALLERS=1024` borne la mémoire ; **pas de bucket UID**, **pas de `Semaphore`** | **Ouvert** (H-05/R-05) |
| **D-03** | Message de 128 Mio sur `Admin1`/`Manager1` **avant** validation | U2 | OOM cgroup de l'admin/daemon (`MemoryMax`), activation en échec | `MemoryMax` + `Restart=no` ; `StartLimitBurst=5` peut bloquer l'admin ~60 s | **Partiel** (AD-R1) |
| **D-04** | Dialogue polkit en boucle (activation + appels répétés) | U1 | Harcèlement/DoS de l'utilisateur, bruit journald | bucket **avant** polkit (nom + UID), `DialogGate` (un dialogue), `StartLimit` | **Vérifié en test**, pas en réel |
| **D-05** | Profil JSON piégé persisté (`onlineCores: 0`, TDP absurde, `refreshRate: NaN`) | U1 via `SaveProfilesJSON` (authentifié) ou import | Application destructrice au boot | `validate()` + `validate_settings_references` + filtrage des presets **avant** écriture (`service.rs:394-448`) ; aucune correction silencieuse | **Vérifié en test** (`transaction.rs`), pas matériellement |
| **D-06** | Symlink/TOCTOU sur `/var/lib` | U1 (si le répertoire devenait inscriptible) | Écriture root hors chemin | Répertoire 0755 root, `O_NOFOLLOW`, `fstat`, écriture atomique (`ConfigStore`), `ProtectSystem=strict` + `ReadWritePaths` | **Vérifié** (tests `config.rs`) |
| **D-07** | DoS capteurs / `nvidia-smi` (bail maintenu, lectures en boucle) | U1 | CPU, batterie, forks root | 2 lectures/s, bail 10 s, session 5 min, cache 10 s | **Fermé** (H-02) |
| **D-08** | Abus du service admin root à la demande | U1 | Processus root répétés, OOM, dialogues | activation D-Bus uniquement, `Restart=no`, sortie 30 s, `RuntimeMaxSec=5min`, caps vides, polkit | **Implémenté** ; **non testé en conditions réelles** |
| **D-09** | Bascule PRIME par `tccd` | U1 via un futur setter | Reconfiguration graphique, écran noir, persistance système | `prime::set_mode` **retiré de `tcc-hw`** ; test statique `no_admin_reachability.rs` ; bascule future dans `tccd-admin` (phase 3 séparée, AD-10) | **Fermé pour la surface actuelle** (H-01) |
| **D-10** | **LUKS — hors périmètre, non implémenté** : aucune phrase, aucun service, aucune action | — | — | Décision utilisateur ; D5 historique | **Non applicable** |
| **D-11** | `GetSettingsJSON` expose `extra` (champ JSON libre ré-émis à tous) | U1 (lecture) | Fuite d'un futur champ sensible déposé dans `extra` | Aucune vue filtrée ; `extra` est ré-émis tel quel (`dbus.rs:102-107`) | **Ouvert** (R-19) |
| **D-12** | Usurpation du nom admin / faux succès | U1 | Phishing polkit, client trompé | `<deny own>` + seul root possède (`conf:41-42`) ; `DO_NOT_QUEUE` (zbus) ; `Admin1` ne renvoie pas de configuration | Vérifié statiquement |

### 4. Mitigations vérifiées vs partielles vs non validées

**Vérifiées (code + test ou garde statique)**

| Contrôle | Preuve |
|---|---|
| Policy D-Bus fail-closed, Admin1/Manager1/Control1 membres listés | `packaging/dbus/…conf`, `validate.sh --repo` 220/220 (dont absence d'allow sans interface/membre, deny avant allow ; Admin1 `Properties.Get/GetAll`/`Ping`/`GetVersion` inclus, A1-06) |
| Setters `Manager1` injoignables | deny explicites `conf:110-133` |
| `Control1` root-only | allow root + deny default + vérif uid dans `tccd` (`dbus.rs:970-980`) |
| `tcc-authz` fail-closed (sujet, flags, challenge, timeout, absence) | 19 tests `tcc-authz` ; `lib.rs:295-320` |
| `Admin1` bornes et transaction | payload 1 Mio (`transaction.rs:33`), `.bak` conditionnel + rollback si précédent exploitable (A1-07), filtrage presets, références croisées ; tests `transaction.rs` (7) + `admin_dbus` (7/7 **en parallèle**) |
| Bail capteurs borné | `sensor_lease.rs:28-30`, `rate_limit.rs:26-28` |
| Commands allowlistées, `env_clear`, timeouts, cache | `tcc-hw/src/command.rs`, `nvidia.rs`, 182 tests `tcc-hw` |
| Écritures sysfs `O_NOFOLLOW` + `fstat` | `tcc-hw/src/sysfs.rs` |
| Hardening systemd | scores 3.3 / 1.6 / 3.6 ; `validate.sh` 220/220 |
| Admin non installé par défaut, caps vides | `install-tree.sh:40-81`, `tccd-admin.service`, `TCC_WITH_ADMIN=0` |

**Partielles (contrôle présent mais insuffisant ou non prouvé)**

| Contrôle | Ce qui manque |
|---|---|
| Rate limiting | bucket par **nom unique** uniquement dans `tccd` ; **pas de bucket UID**, **pas de `Semaphore`** de handlers (H-05/R-05) ; contournable par multi-connexions |
| Borne de taille | vérifiée **après** désérialisation : un message zbus de 128 Mio alloue avant le contrôle ; seul `MemoryMax` protège (AD-R1) |
| Polkit | implémenté dans `tcc-authz`/`tcc-admin`, **pas branché dans `tccd`** ; aucun test de bout en bout avec le vrai agent polkit ni la policy système ; `admin_dbus` est désormais **parallèle-safe** (nom unique de connexion) |
| `Admin1` | non installé par défaut ; pas de test root/bus système ; pas de test d'échec de `Control1` en conditions réelles (fake notifier) ; `Properties.Get/GetAll` élargit la lecture aux futures propriétés (A1-11) |
| Portée des setters matériels | globale (machine) ; aucune notion de session dans `tccd` |
| `GetSettingsJSON` | expose l'objet complet, `extra` inclus (R-19) |
| Validation matérielle | aucune (pas de laptop TUXEDO) : `DeviceAllow`, debugfs, `CapabilityBoundingSet=` vide, ordre `modprobe` (H-13) |
| CI packaging | `validate.sh`/`systemd-analyze`/`xmllint`/`dbus-policy-test.sh` absents de la CI (H-14/R-23) ; protections uniquement locales |

**Non validées / non implémentées**

- Polkit réel (dialogue système, `auth_admin`, cache `_keep`), sur bus système et en root.
- Test fonctionnel de la policy D-Bus (`dbus-policy-test.sh`, root requis).
- Toute écriture matérielle réelle (charge, LED, `platform_profile`, TDP, cTGP, `fn_lock`, YCbCr, webcam).
- Phase 2 (`SetTdpValues`, `SetOdmProfile`, `SetNvidiaCtgpOffset`, `SetPrimeMode`) : refusée par la policy, **non implémentée** dans `tcc-admin`.
- Phase 3 (Tomte/WebFAI/diagnostic) : design seulement ; **LUKS hors périmètre, non implémenté**.
- Contrôles UVC V4L2 : lecture + écriture session implémentées (`tcc-hw`, `tccd-user`), tests fake/CI ; écriture réelle, ACL, deux sessions, LED, `EBUSY`, suspend/hotplug non testés ; **UVCW-01 accepté, UVCW-02…08 fermés**.
- `cargo deny`/`cargo audit`, `lintian`/`rpmlint`/`namcap`, installation réelle dans un conteneur.

### 5. Risques résiduels acceptés / à trancher

| # | Risque | Niveau | Décision |
|---|---|---|---|
| **TR-01** | Profil temporaire **global** (pas de portée session) | Moyen | **À trancher** : accepté en mono-utilisateur, à restreindre au jalon 4 si l'agent expose une identité de session (R-03/AD-7) |
| **TR-02** | Réouverture des setters `Manager1` | Élevé si mal fait | **Bloqué** tant que `tccd` n'appelle pas `CheckAuthorization` ; ne jamais retirer les deny sans authz + test dédié (AD-8) |
| **TR-03** | AD-12 : phase 2 dans la même unité admin (relâche `ProtectKernelTunables`/`PrivateDevices`) | Moyen | **À trancher** ; recommandation `security` : service séparé `…Admin.Hw` |
| **TR-04** | Tombstones d'IDs de profil absents | Faible | **Résolu** : `Settings.deletedProfileIds` (≤ 1024, permanents), refus de recréation, transaction double fichier (`ADMIN_DESIGN.md` AD-13/§4.2.1) ; **T1-01 fermé** (absence initiale restaurée) et **AD-R13 fermé** (diff d'état seul) |
| **TR-05** | V4L2 `unsafe` (lecture + écriture UVC) | Moyen | **Revu 2026-11-07** : `unsafe` confiné `real.rs`/`real_write.rs`, `S_CTRL` isolé, allowlist 15, budget 64, `try_lock`, quarantaine ; écriture session livrée (`tccd-user`) ; **UVCW-02…07 fermés**, **UVCW-08 fonctionnellement fermé** (commentaire résiduel), **UVCW-01 accepté comme dette documentée** ; matériel réel non testé |
| **TR-06** | Absence de laptop TUXEDO | Moyen | Accepté sous réserve des smoke tests H-13 avant release |
| **TR-07** | Message 128 Mio → OOM admin + `StartLimit` | Faible (dispo) | Accepté (AD-R1), documenté |
| **TR-08** | `extra` de `GetSettingsJSON` | Faible | Accepté ; documenter que la config n'est pas un coffre-fort (R-19) |
| **TR-09** | Admin non installé par défaut : la fonctionnalité d'écriture n'est pas livrée | — | État voulu tant que les tests réels (§4) ne sont pas faits |
| **TR-10** | Écritures GUI étendues (charge, Fn-Lock, lighting, matrice, résolution) via `Admin1`/`Admin.Hw` | Faible | polkit `settings`/`profiles`/`power-limits`, validation admin, aucun setter `Manager1` charge/lighting/Fn-Lock ; exceptions non matérielles rate-limitées ; réserves **UIW-01/UIW-03** (`REVIEW_UI_WRITES.md`) |
| **TR-11** | Écritures matérielles **par profil** (ODM/TDP/cTGP) appliquées à l'activation sans polkit | Faible (mono) / Moyen (multi) | Contenu autorisé par `Admin1.SaveProfilesJSON` (polkit `profiles`, sans cache), activation `SetTempProfile` globale et rate-limitée (TR-01) ; `platform_profile` refuse PPD, TDP validé firmware avant écriture ; **PHW-02/PHW-03 corrigés** (ODM absent/`None` sans écriture, borne cTGP dynamique/statique exposée et refus hors borne, profil `tdpValues=[]` = no-op, `Admin.Hw.SetTdpValues([])` = maxima explicite) ; **PHW-04 partiel accepté** (core 16 vs hw 3, aucune écriture) ; PHW-05 Info ; **PHW-01 multi ouvert** ; `REVIEW_PROFILE_HW_WRITES.md` |
| **TR-12** | Écriture UVC de session (`tccd-user`, `User1.SetUvcControl`) | Faible / Moyen (dispo) | Allowlist 15, device énuméré, `O_RDWR` / `O_NOFOLLOW`, `fstat` major+minor, identité driver/card/bus_info, budget 64, **porte globale**, `try_lock`, quarantaine 16, rate limit 20/s/UID, corps D-Bus 64 Kio, cache 1 s, `EACCES`/`EPERM` → `NotSupported`, bus de session, aucun polkit ; **UVCW-01 accepté (dette documentée)**, **UVCW-02…07 fermés**, **UVCW-08 fonctionnellement fermé** ; watchdog résiduel documenté, matériel réel non testé ; `REVIEW_V4L2.md` §2ter |

### 6. Correspondance surface → décision → revue → preuve

| Surface | Décision | Revue | Preuve / test |
|---|---|---|---|
| Profils/settings persistants | **D2** (`/var/lib`, `/etc` RO), **D3** (JSON gelé) | `REVIEW_JALON1` R-18/R-19 ; `ADMIN_DESIGN` AD-5 | `ConfigStore` (`config.rs`) ; `tcc-admin/tests/transaction.rs` |
| `SetTempProfile`/`ClearTempProfile` | **D3** (setter unique admis), **D1** (portée) | `REVIEW_JALON1` R-03 ; `ADMIN_DESIGN` AD-7/AD-8 | `dbus_limits.rs` ; audit `dbus.rs:129-200` |
| `Manager1` setters | **D1** (écritures sensibles hors `tccd`) | `REVIEW_HARDWARE` H-05/H-06/H-10 | deny policy `conf:110-133` |
| Bail capteurs | **D4** (PPD) non concerné ; budget `QUALITY.md` §4 | `REVIEW_HARDWARE` H-02 | `sensor_lease.rs` tests ; `rate_limit.rs` |
| `Control1.ReloadConfig` | **D1** (séparation), **D2** (rechargement `/var/lib`) | `ADMIN_DESIGN` AD-6 | policy root-only + `dbus.rs:956-996` ; test `admin_dbus` (fake notifier) |
| `Admin1` phase 1 | **D1** (service séparé), **D2**, **D5** (pas de secret) | `ADMIN_DESIGN` AD-1/AD-2/AD-3/AD-5 ; `REVIEW_JALON1` R-04 | `tcc-authz` 19 tests ; `tcc-admin` lib 24 + `admin_dbus` 7/7 + transaction 7 ; unit phase 1 |
| Polkit | **D1** (chaque méthode admin vérifie polkit) | `ADMIN_DESIGN` §3 | `tcc-authz/src/lib.rs` (sujet/flags/challenge) |
| PRIME | **D1** (admin uniquement), **D5** non concerné | `REVIEW_HARDWARE` H-01 | `prime::set_mode` retiré ; `no_admin_reachability.rs` |
| Commandes externes | Règles de code (`DECISIONS.md` : pas de shell) | `REVIEW_HARDWARE` H-03/H-04 | `command.rs` ; 182 tests `tcc-hw` |
| Écritures sysfs/debugfs | Règles système (`ProtectKernelTunables=no`) ; **D4** pour `platform_profile` | `REVIEW_HARDWARE` H-13 | `sysfs.rs` ; `platform_profile.rs:175-183` (refus PPD) |
| Écritures matérielles par profil (ODM/TDP/cTGP) | **D1** (contenu admin), **D3** (JSON gelé), **D4** (PPD) | `REVIEW_PROFILE_HW_WRITES.md` PHW-01…PHW-06 | `odm` 18/18 ; `nvidia` 9/9 ; `admin tdp` 9/9 ; `admin_hw_dbus` 5/5 ; `profile_edit` 27/27 ; `profile_transfer` 18/18 ; `validate` 23/23 ; matériel réel non testé |
| Écriture UVC de session | **D1** (hors root), **D5** non concerné | `REVIEW_V4L2.md` §2ter (UVCW-01…08) | `tcc-hw` 284/284 ; `v4l2` 62/62 ; `fake_v4l2` 8/8 ; garde 4/4 ; `tcc-user` 169/169 ; uvc 12/12 ; rate_limit 6/6 ; contract 9/9 ; workspace 1527/1527 ; matériel réel non testé |
| Overrides de développement | **D7** | `REVIEW_JALON1` R-14 | GUI gatée ; CLI/agent non gatés (R-14 partiel) |
| Packaging/hardening | Règles système ; **D1/D2** | `REVIEW_JALON1` R-01/R-02/R-06/R-07/R-09/R-21/R-22 | `validate.sh` 220/220 ; `systemd-analyze` 3.3/1.6/3.6 |

### 7. Cinq priorités de sécurité avant release

1. **Rendre polkit réellement effectif et le prouver** : test de bout en bout sur bus **système** en
   root (dialogue, refus, annulation, polkit arrêté ⇒ refus), puis seulement installer `tccd-admin`
   (`TCC_WITH_ADMIN=1`). Tant que ce test n'existe pas, garder l'admin non installé et les setters
   `Manager1` refusés par la policy.
2. **Fermer l'anti-DoS D-Bus** : bucket par **UID** en plus du nom unique, `Semaphore` de handlers,
   borne **avant** désérialisation (ou acceptation explicite d'AD-R1), test de rafale multi-connexions
   et de message géant ; `admin_dbus` est désormais parallèle-safe (nom unique de connexion), conserver cette propriété.
3. **Valider le matériel et rouvrir les setters proprement** : campagne H-13 (`DeviceAllow`,
   debugfs, caps vides, module tardif), puis réactivation des setters `Manager1` **uniquement** avec
   `CheckAuthorization` dans `tccd` et respect des interrupteurs globaux (H-06).
4. **Rendre la CI packaging bloquante** : `validate.sh --repo`, `systemd-analyze verify|security`,
   `xmllint`, `dbus-policy-test.sh` (job root), `cargo deny`/`audit` ; archiver les scores.
5. **Trancher les risques résiduels avant d'ouvrir la phase 2** : AD-12 (unité séparée ou non),
   portée du profil temporaire (TR-01), décision V4L2 (TR-05), et documenter `GetSettingsJSON`/
   `extra` (TR-08). TR-04 est résolu par AD-13 ; T1-01 et AD-R13 sont fermés. Aucune phase 2 sans
   ces décisions écrites.

### 8. Ce qui n'est pas vérifiable localement

- Le comportement du **vrai** polkit (dialogue, agent, cache `_keep`, session inactive/distante) :
  seuls des authorizers factices ont été testés.
- Le test fonctionnel de la policy D-Bus sur bus système (`dbus-policy-test.sh`, root).
- Toute écriture matérielle (charge, LED, `platform_profile`, TDP, cTGP, `fn_lock`, YCbCr, kill
  switch webcam) et les hypothèses systemd associées.
- L'installation réelle des paquets et les linters distro (`lintian`, `rpmlint`, `namcap`), ainsi que
  `cargo deny`/`audit` dans cet environnement.
- La coexistence avec l'ancien TCC (nom D-Bus historique) et la migration `/etc/tcc` sur une machine
  réelle.
