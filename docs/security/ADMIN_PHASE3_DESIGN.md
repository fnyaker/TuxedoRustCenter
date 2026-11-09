# Design sécurité — phase 3, périmètre retenu (hors LUKS)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | design **prêt à implémenter** des opérations sensibles restantes : Tomte, WebFAI, AnyDesk (conditionnel), diagnostic système |
| Périmètre d'écriture | `docs/security/**` uniquement — aucun code modifié |
| Références | `docs/security/DECISIONS.md` (D1/D2/D6), `docs/security/ADMIN_DESIGN.md` (AD-1…AD-11), `docs/security/ADMIN_PHASE2_DESIGN.md` (P2-01/B2), `docs/ARCHITECTURE.md` §3.2/§3.10, `docs/linux/PLATFORM_DESIGN.md` |

> ## Décision utilisateur — LUKS hors périmètre
>
> **Le changement de phrase de passe de chiffrement (LUKS) est HORS PÉRIMÈTRE, NON IMPLÉMENTÉ.**
> Décision utilisateur du 2026-11-06. Ce document ne définit **aucun** service, méthode D-Bus,
> signature, action polkit, capacité, accès `cryptsetup`, transport de phrase par fd, ni accès aux
> périphériques bloc pour LUKS. Les références existantes dans le dépôt sont listées en §9 pour
> suppression ou requalification — elles ne doivent pas être implémentées.

---

## 1. Décisions clés

| ID | Décision | Justification | Alternative rejetée |
|---|---|---|---|
| **P3-01** | **LUKS exclu** : aucune opération de changement de phrase, aucun service, aucune action polkit, aucun accès block device | décision utilisateur ; supprime la surface la plus critique (secrets + `CAP_SYS_ADMIN`) | service LUKS séparé (rejeté par décision) |
| **P3-02** | **Trois services** : `Admin.Tomte`, `Admin.WebFai`, `Admin.Diagnostics` ; `Admin.AnyDesk` seulement si le volet AnyDesk est retenu | trois profils de privilèges distincts : outil système TUXEDO, installation de paquets/téléchargement, collecte réseau | service unique « phase 3 » (mélange exec, paquets, réseau, données de diagnostic) |
| **P3-03** | **PackageKit d'abord** pour WebFAI/AnyDesk ; pas d'`apt-get`/`dnf`/`pacman` direct dans un service root durci | PackageKit est la couche distro polkitée qui gère signatures, transactions et scripts de maintenance ; un `apt-get` direct obligerait à ouvrir `/usr`, `/etc`, `/var` et de nombreuses capacités | `apt-get install` direct (repli documenté uniquement, §5.4) |
| **P3-04** | **Aucun téléchargement exécuté en root** : téléchargement HTTPS + checksum + signature, stockage 0700, puis installation par PackageKit ; jamais `sh fichier.sh` | corrige l'anti-patron de l'original (`systeminfos.sh` téléchargé depuis GitHub puis exécuté via `pkexec sh -c`) | script distant exécuté, `pkexec`, `curl \| sh` |
| **P3-05** | **Tomte** : `tuxedo-tomte` par chemin absolu, allowlist d'arguments, `env_clear`, timeout, sortie bornée, polkit ; pas de shell | conserve la parité sans `pkexec` ni concaténation | `pkexec tuxedo-tomte …`, `yes \| …` |
| **P3-06** | **Diagnostic** : `systeminfos` **du paquet** (ou binaire local vérifié), jamais téléchargé/exécuté ; collecte dans un cache root 0700, aperçu et consentement avant envoi | évite l'exfiltration silencieuse et l'exécution de code distant | téléchargement GitHub + `pkexec sh`, upload automatique |
| **P3-07** | **AnyDesk reporté par défaut** : détection en lecture seule ; installation/lancement seulement si décision explicite, via dépôt signé et session utilisateur | outil d'accès distant tiers : risque élevé, valeur incertaine | installation automatique d'un dépôt tiers |
| **P3-08** | **Polkit** : actions `tomte-run`, `webfai`, `anydesk`, `diagnostics` (et `packages` si `PackagesApply` est retenu), `auth_admin` sans `_keep`, sujet `system-bus-name` | cohérent avec D1/AD-3/AD-4 | cache `_keep`, `exec.path`, `allow_gui` |
| **P3-09** | **Aucun secret LUKS** dans ces services ; les seules données sensibles sont les rapports de diagnostic, redactés et consentis | D5 reste une règle, sans objet ici | collecte automatique de logs bruts |

---

## 2. Périmètre et ordre de livraison

| Étape | Contenu | Statut proposé |
|---|---|---|
| **3.0** | Nettoyage des références LUKS (§9), policy/actions, docs | **bloquant avant tout code phase 3** |
| **3.1** | Diagnostic système : détection de `systeminfos`, collecte bornée, aperçu, upload HTTPS, nettoyage | **premier livrable** (pas de paquets, pas de device, pas de secret) |
| **3.2** | WebFAI : détection, installation/suppression via PackageKit, lancement en session | deuxième livrable |
| **3.3** | Tomte : statut, exécution allowlistée, polkit | troisième livrable (outil système à privilèges mesurés) |
| **3.4** | AnyDesk : détection seule | **par défaut** ; installation/lancement **reportés** |
| **3.5** | AnyDesk installation/lancement (si décision explicite) | hors release initiale ; revue dédiée |

**Pourquoi ce sont les premiers** : le diagnostic ne touche ni aux paquets ni aux devices et peut être
testé sans privilège de package manager ; WebFAI via PackageKit est borné à une transaction gérée par
la distribution ; Tomte est un outil TUXEDO opaque qui peut nécessiter des capacités et des écritures
système larges, donc mesuré en dernier. AnyDesk reste un accès distant tiers : détection seulement
tant qu'une décision explicite ne l'autorise pas.

---

## 3. Contrat D-Bus

### 3.1 Constantes (`tcc-ipc`)

```rust
pub const DBUS_ADMIN_TOMTE_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Tomte";
pub const DBUS_ADMIN_TOMTE_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Tomte";
pub const DBUS_ADMIN_TOMTE_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminTomte1";
pub const ADMIN_TOMTE_CONTRACT_VERSION: u32 = 1;

pub const DBUS_ADMIN_WEBFAI_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.WebFai";
pub const DBUS_ADMIN_WEBFAI_PATH: &str = "/com/tuxedocomputers/tccd/Admin/WebFai";
pub const DBUS_ADMIN_WEBFAI_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminWebFai1";
pub const ADMIN_WEBFAI_CONTRACT_VERSION: u32 = 1;

pub const DBUS_ADMIN_ANYDESK_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.AnyDesk";
pub const DBUS_ADMIN_ANYDESK_PATH: &str = "/com/tuxedocomputers/tccd/Admin/AnyDesk";
pub const DBUS_ADMIN_ANYDESK_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminAnyDesk1";
pub const ADMIN_ANYDESK_CONTRACT_VERSION: u32 = 1;

pub const DBUS_ADMIN_DIAG_SERVICE: &str = "com.tuxedocomputers.tccd.Admin.Diagnostics";
pub const DBUS_ADMIN_DIAG_PATH: &str = "/com/tuxedocomputers/tccd/Admin/Diagnostics";
pub const DBUS_ADMIN_DIAG_INTERFACE: &str = "com.tuxedocomputers.tccd.AdminDiagnostics1";
pub const ADMIN_DIAG_CONTRACT_VERSION: u32 = 1;
```

Chaque service expose aussi `Ping() -> b`, `GetVersion() -> s`, `GetContractVersion() -> u`.

### 3.2 Méthodes

| Service | Méthode | Signature | Polkit | Timeout | Idempotence |
|---|---|---|---|---|---|
| Tomte | `TomteStatus` | `-> s` | non (lecture) | 15 s | oui |
| Tomte | `TomteRun` | `as -> s` | `tomte-run` | 120 s | dépend de la sous-commande |
| WebFAI | `WebFaiDetect` | `-> s` | non (lecture) | 15 s | oui |
| WebFAI | `WebFaiInstall` | `-> s` | `webfai` | 10 min | oui (paquet déjà installé ⇒ no-op) |
| WebFAI | `WebFaiRemove` | `-> s` | `webfai` | 10 min | oui |
| WebFAI | `WebFaiLaunch` | `-> s` | non (session utilisateur) | 5 s | non |
| AnyDesk | `AnyDeskDetect` | `-> s` | non (lecture) | 15 s | oui |
| AnyDesk | `AnyDeskInstall` | `-> s` | `anydesk` | 10 min | oui |
| AnyDesk | `AnyDeskRemove` | `-> s` | `anydesk` | 10 min | oui |
| AnyDesk | `AnyDeskLaunch` | `-> s` | non (session) | 5 s | non |
| Diagnostics | `DiagnosticsCollect` | `-> s` | `diagnostics` | 60 s | non (nouveau ticket) |
| Diagnostics | `DiagnosticsUpload` | `s -> s` | `diagnostics` | 120 s | non (ticket unique) |
| Diagnostics | `DiagnosticsCleanup` | `s -> s` | `diagnostics` | 15 s | oui |

Règles communes :

- **Taille** : rapports JSON ≤ **256 Kio**, `TomteRun` ≤ 8 arguments de ≤ 64 octets, ticket
  `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$` (UUID v4).
- **Sortie** : `s` contient un JSON borné, schéma versionné ; jamais de sortie brute de commande.
- **Concurrence** : `Semaphore(1)` par service ; second appel ⇒ `Busy` immédiat (pas de file).
- **Rate limit** : écritures 2/s burst 4 par nom unique + UID (réutiliser `tcc-authz`) ; lectures
  2/s burst 5. Aucun dialogue pour un refus de limite.
- **Annulation** : pas d'annulation d'une transaction de paquet en cours (risque d'état dpkg/rpm
  incohérent) ; `TomteRun` peut tuer l'enfant au timeout ; `DiagnosticsUpload` peut être interrompu
  et nettoyé. Le client qui ferme la connexion n'annule pas l'opération ; documenter.
- **Confirmation** : l'admin ne peut pas confirmer à la place de l'utilisateur. L'UI doit afficher
  avant l'appel : paquet exact, source/dépôt, redémarrage éventuel, conséquence d'AnyDesk (accès
  distant), et pour le diagnostic l'aperçu + destination. Le dialogue polkit suit.

### 3.3 Erreurs

Namespace par service (`org.tuxedocomputers.tccd.AdminTomte1.Error.*`, etc.) avec les codes :

| Code | Sens |
|---|---|
| `NotAuthorized` | polkit refusé/absent/timeout, identité inconnue |
| `InvalidArgs` | argument hors grammaire/allowlist, ticket invalide |
| `NotSupported` | outil/paquet absent, distribution non supportée |
| `PolicyDenied` | opération refusée par une politique locale (ex. AnyDesk non retenu, dépôt non signé) |
| `DownloadFailed` | HTTPS/TLS/statut HTTP/redirection refusée |
| `SignatureInvalid` | checksum ou signature invalide |
| `PackageFailed` | transaction PackageKit/distro en échec |
| `Timeout` | commande/transaction au-delà du budget |
| `Busy` | opération déjà en cours |
| `RateLimited` | quota dépassé |
| `WriteFailed` | stockage cache/ticket impossible |

Messages = codes de raison ASCII bornés (`tomte.args`, `webfai.package`, `webfai.signature`,
`anydesk.repo`, `diag.ticket`, ...), jamais de sortie de commande, de chemin ou de contenu.

---

## 4. Services et unités systemd

### 4.1 `tccd-admin-tomte.service`

```ini
[Unit]
Description=TuxedoRustCenter on-demand Tomte system tool
Documentation=file:/usr/share/doc/tuxedo-rust-center/ADMIN_PHASE3_DESIGN.md
After=dbus.service
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin.Tomte
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-tomte
Restart=no
TimeoutStartSec=10
TimeoutStopSec=30
RuntimeMaxSec=5min
KillMode=control-group
UMask=0077
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=full
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
DevicePolicy=closed
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
KeyringMode=private
RemoveIPC=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=512M
TasksMax=32

# À MESURER avec `tuxedo-tomte` réel : commencer vide, ajouter uniquement
# les capacités strictement nécessaires (DAC_OVERRIDE, CHOWN, FOWNER, SETFCAP,
# SETGID, SETUID, SYS_MODULE si un sous-commande charge un module), avec
# justification dans packaging/SECURITY.md.
CapabilityBoundingSet=
AmbientCapabilities=

# Pas de [Install] : activation D-Bus uniquement.
```

Notes :

- `ProtectSystem=full` est un **point de départ** : Tomte peut écrire dans `/etc`/`/var` mais pas
  `/usr`. Si la mesure montre qu'il installe dans `/usr` ou charge des modules, **ne pas élargir à
  `/usr` sans revue** : envisager de déléguer l'installation à PackageKit ou de retirer la
  sous-commande du périmètre.
- `NoNewPrivileges=yes` peut casser des sous-commandes qui exécutent des binaires setuid ; le test
  réel tranche. Ne pas le désactiver sans justification et revue.
- Aucun `ReadWritePaths` initial : la mesure `strace` définit la liste minimale. Ne pas mettre `/`.
- `CAP_SYS_MODULE` est une capacité très large : si Tomte l'exige, la limiter à la seule
  sous-commande qui en a besoin (service séparé ou refus de la sous-commande).

### 4.2 `tccd-admin-webfai.service`

```ini
[Unit]
Description=TuxedoRustCenter on-demand WebFAI Creator management
After=dbus.service
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin.WebFai
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-webfai
Restart=no
TimeoutStartSec=10
TimeoutStopSec=60
RuntimeMaxSec=15min
KillMode=control-group
UMask=0077
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/cache/tuxedo-rust-center
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
DevicePolicy=closed
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
KeyringMode=private
RemoveIPC=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=256M
TasksMax=16

CapabilityBoundingSet=
AmbientCapabilities=
```

Notes :

- PackageKit est contacté par D-Bus système (`org.freedesktop.PackageKit`) : le service n'exécute
  **pas** `apt-get`/`dnf`/`pacman`, ne monte rien, n'écrit pas `/usr`.
- Le cache `/var/cache/tuxedo-rust-center` est créé par `CacheDirectory=tuxedo-rust-center` (mode
  `0700`) ou par l'unité ; les fichiers sont `0600`, vérifiés par `O_NOFOLLOW`/`fstat`.
- Si PackageKit est absent (Arch minimal), le service renvoie `NotSupported` ; le repli direct
  (§5.4) exige une unité séparée et une revue, pas un élargissement silencieux de celle-ci.
- Un seul téléchargement/une seule transaction à la fois (`Semaphore(1)`), `Busy` au second appel.

### 4.3 `tccd-admin-anydesk.service` (conditionnel)

Même unité que WebFAI (mêmes privilèges : réseau + PackageKit, aucun device), avec :

- `BusName=com.tuxedocomputers.tccd.Admin.AnyDesk` ;
- `ExecStart=.../tccd-admin-anydesk` ;
- `RuntimeMaxSec=15min` ;
- **pas de dépôt tiers ajouté** : `AnyDeskInstall` refuse si le paquet n'est pas déjà disponible
  dans un dépôt signé configuré (`PolicyDenied("anydesk.repo")`).

Si le volet AnyDesk n'est pas retenu, **ne pas installer l'unité ni l'action polkit** ; seule
`AnyDeskDetect` reste dans la GUI, via une lecture `tccd`/paquet.

### 4.4 `tccd-admin-diagnostics.service`

```ini
[Unit]
Description=TuxedoRustCenter on-demand system diagnostic collection
After=dbus.service
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=dbus
BusName=com.tuxedocomputers.tccd.Admin.Diagnostics
ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-diagnostics
Restart=no
TimeoutStartSec=10
TimeoutStopSec=30
RuntimeMaxSec=5min
KillMode=control-group
UMask=0077
User=root
Group=root

NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/cache/tuxedo-rust-center
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
DevicePolicy=closed
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
KeyringMode=private
RemoveIPC=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
SecureBits=noroot-locked
LimitCORE=0
MemoryMax=256M
TasksMax=16

CapabilityBoundingSet=
AmbientCapabilities=
```

Notes :

- `ProtectHome=yes` empêche la collecte dans les répertoires utilisateurs : c'est volontaire.
- `ProtectSystem=strict` garde `/etc`/`/var` en lecture seule ; le rapport est écrit dans le cache
  autorisé. Aucun accès aux devices.
- `systeminfos` est un **binaire/script du paquet** (`/usr/bin/systeminfos` ou
  `/usr/bin/tuxedo-systeminfo`) : résolu par allowlist absolue, jamais téléchargé, jamais `pkexec`,
  jamais `sh -c`. Si absent ⇒ `NotSupported`.
- L'outil peut avoir besoin d'un `DISPLAY`/`XAUTHORITY` pour certaines collectes ; **ne pas** hériter
  de l'environnement de session (D6). La collecte se limite aux informations système ; les données
  graphiques sont collectées par l'agent de session si nécessaire, jamais en root.

### 4.5 D-Bus activation

Ajouter `packaging/dbus/com.tuxedocomputers.tccd.Admin.{Tomte,WebFai,AnyDesk,Diagnostics}.service`
avec `Name=`, `Exec=`, `User=root`, `SystemdService=`. Installation conditionnelle :
`TCC_WITH_ADMIN_PHASE3=1` (et `TCC_WITH_ANYDESK=1` pour AnyDesk), **absent par défaut**.

---

## 5. Téléchargements, signatures et paquets

### 5.1 Principe

**Aucun téléchargement n'est exécuté.** Le flux est : télécharger (HTTPS) → vérifier taille →
vérifier SHA-256 → vérifier signature → stocker `0600` → installer le **paquet** via PackageKit →
supprimer le fichier (succès ou échec). Aucun `.deb`/`.rpm`/`.pkg.tar.zst` n'est exécuté directement,
aucun script n'est lancé.

### 5.2 WebFAI

- **Détection** : `WebFaiDetect` interroge PackageKit (`GetPackageById`/`SearchNames`) pour
  `tuxedo-webfai-creator` ; repli lecture `dpkg-query`/`rpm -q`/`pacman -Q` en allowlist absolue,
  jamais `which` avec un nom non constant. Retourne `installed`, `version`, `source`.
- **Installation** : `WebFaiInstall` privilégie le **paquet du dépôt** `tuxedo-webfai-creator` via
  PackageKit (`InstallPackageNames`). Aucun ajout de dépôt. Si un artefact direct est imposé par la
  distribution :
  - URL **constante** dans le binaire (pas de paramètre client), HTTPS uniquement ;
  - taille ≤ 512 Mio, timeout 10 min, redirections HTTP refusées ;
  - SHA-256 et signature Ed25519/minisign avec clé publique **embarquée** ; échec ⇒
    `SignatureInvalid`, fichier supprimé ;
  - fichier `0600` dans `/var/cache/tuxedo-rust-center/downloads`, propriétaire root, pas de lien ;
  - installation par **PackageKit `InstallFiles`** (transaction gérée par la distro) ;
  - jamais `dpkg -i`/`rpm -i`/`pacman -U` direct sans validation de signature du paquet.
- **Suppression** : `WebFaiRemove` via PackageKit (`RemovePackage`), no-op si absent.
- **Lancement** : `WebFaiLaunch` **ne s'exécute pas dans le service root**. Il demande à
  `tccd-user` (session) de lancer le programme installé (`tuxedo-webfai-creator`) en tant
  qu'utilisateur, avec `env_clear` + `DISPLAY`/`WAYLAND_DISPLAY` fournis par l'agent. Si le programme
  exige root pour écrire une clé USB, l'UI l'explique : TCC ne l'élève pas.

### 5.3 AnyDesk (si retenu)

- Détection seule par défaut : paquet `anydesk` présent/absent + version, sans exécution.
- Installation : uniquement si le paquet est dans un dépôt **déjà configuré et signé** ; jamais
  d'ajout de dépôt tiers, jamais de clé importée automatiquement. Sinon `PolicyDenied`.
- Lancement : session utilisateur, jamais root ; pas d'auto-start, pas de mot de passe unattended,
  pas de configuration silencieuse. L'UI affiche un avertissement d'accès distant.
- Recommandation : **ne pas livrer** l'installation/lancement en 3.1–3.3 ; détection seule.

### 5.4 Repli sans PackageKit

Si une distribution n'a pas PackageKit :

- le service **renvoie `NotSupported`** ; il n'exécute pas `apt-get`/`dnf`/`pacman` dans l'unité
  durcie actuelle ;
- un repli direct exige une unité **séparée** `tccd-admin-packages.service` avec :
  - `ProtectSystem=full` (ou `strict` + `ReadWritePaths` mesurés : `/usr`, `/etc`, `/var`, `/boot`),
  - capacités mesurées (`CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER`, `CAP_SETFCAP`, `CAP_SETGID`,
    `CAP_SETUID`, éventuellement `CAP_SYS_ADMIN`), justifiées une par une ;
  - `NoNewPrivileges` à valider (les scripts de maintenance peuvent échouer) ;
  - revue `security` explicite.
- Cette unité n'est **pas** la même que WebFAI/Tomte : elle ne doit pas exposer d'autres méthodes.

---

## 6. Tomte

### 6.1 Allowlist d'arguments

`TomteRun(as args)` :

- `args.len() ∈ 1..=8` ; chaque argument `^[A-Za-z0-9_.-]{1,64}$` (pas d'espace, pas de `/`, pas de
  NUL, pas de `..`) ;
- premier argument ∈ `{AUTOMATIC, listjson, description, block, unblock, configure, remove, reconfigure}` ;
- `description`, `block`, `unblock`, `configure`, `remove` : second argument = nom de module
  `^[a-z0-9-]{1,64}$` ; `all` autorisé uniquement pour `unblock`/`reconfigure` si l'original le
  permet, jamais pour `remove` ;
- aucun autre argument, aucun `-`/`--` (sinon injection d'option), aucun `|`, `;`, `&&`, `$()`, quote.

### 6.2 Exécution

- `Command::new("/usr/bin/tuxedo-tomte")` (allowlist `/usr/bin`, `/usr/sbin`, chemin absolu résolu),
  `args` séparés, `env_clear()`, `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, `LC_ALL=C`, `LANG=C`,
  `HOME=/root`, `stdin=null`, `stdout`/`stderr` bornés 64 Kio, timeout 120 s, kill du groupe.
- `stdout` n'est **jamais** renvoyé brut : rapport JSON `{ok, code, summary}` ; `stderr` uniquement
  journalisé après nettoyage (pas de secret, pas de contenu de fichier).
- `TomteStatus` : `dpkg-query -W`/`rpm -q`/`pacman -Q` en allowlist ou `tuxedo-tomte listjson`
  (lecture) ; `listjson` borné 256 Kio, validé comme JSON, pas de contenu brut.
- `NoNewPrivileges`/capacités : à mesurer ; si une sous-commande exige `CAP_SYS_MODULE` ou l'écriture
  `/usr`, elle est **retirée** du périmètre ou déplacée dans un service dédié, pas ajoutée en bloc.

### 6.3 État / versions

`TomteStatus` retourne `{installed, version, modules:[{name, state, version}]}` borné (≤ 128 modules) ;
aucune commande client ne peut demander un chemin/version arbitraire.

---

## 7. Diagnostic système

### 7.1 Collecte

- `DiagnosticsCollect` exige polkit `diagnostics` (évite le profilage silencieux).
- Outil : `/usr/bin/systeminfos` ou `/usr/bin/tuxedo-systeminfo` (allowlist absolue), **du paquet** ;
  si absent ⇒ `NotSupported`, **aucun téléchargement**, aucun `pkexec`, aucun `sh -c`.
- Arguments figés (aucun argument client), `env_clear()` + variables minimales ; pas de `DISPLAY`
  hérité. Ticket généré côté serveur (UUID v4), pas fourni par le client.
- Sortie bornée **4 Mio**, timeout 60 s, `stdout`/`stderr` séparés ; le rapport est écrit dans
  `/var/cache/tuxedo-rust-center/diagnostics/<ticket>/` (0700 root), fichier `0600`, `O_NOFOLLOW`.
- **Redaction** avant stockage et avant upload : suppression des motifs
  `/etc/shadow`, `PRIVATE KEY`, `BEGIN OPENSSH`, `LUKS`, `password`, `token`, `secret`, `Bearer `,
  `/home/`, `XDG_`, `SSH_AUTH_SOCK`, variables d'environnement, historique shell ; liste blanche de
  fichiers si l'outil le permet. Un rapport qui contient un motif interdit est **refusé**, pas
  tronqué silencieusement.
- Aucun secret LUKS, aucune clé, aucun contenu de `/etc` sensible.

### 7.2 Aperçu et envoi

- `DiagnosticsUpload(ticket)` : l'UI affiche d'abord un **aperçu** du rapport et la destination ;
  consentement explicite ; polkit `diagnostics`.
- HTTPS uniquement, TLS validé (rustls, pas de `danger_accept_invalid_certs`), pas de redirection
  vers HTTP, timeout 120 s, taille ≤ 16 Mio, endpoint **constant** dans le binaire ; aucun token
  dans argv/env/log ; ticket opaque.
- Succès ⇒ suppression du rapport ; échec ⇒ rapport conservé pour réessai, TTL 24 h, nettoyage au
  démarrage du service (`DiagnosticsCleanup` idempotent).
- Aucun envoi automatique, aucun rapport en clair dans `/tmp`, aucun chemin fourni par le client.

### 7.3 Nettoyage

`DiagnosticsCleanup(ticket)` supprime le répertoire du ticket ; ticket invalide ⇒ `InvalidArgs` ;
ticket inconnu ⇒ `NotSupported`/no-op idempotent. `DiagnosticsCollect` purge les tickets > 24 h avant
d'en créer un nouveau.

---

## 8. Autorisation et audit

- Polkit : actions **existantes** `tomte-run`, `packages` ; **nouvelles** `webfai`, `anydesk`,
  `diagnostics` ; `luks-change-key` **déjà retirée** (hors périmètre, vérifié 2026-11-06).
- Toutes `auth_admin`, **sans `_keep`** ; sujet `system-bus-name` du nom unique ; flags `1` ; details
  `{}` ; timeout 120 s ; `is_challenge` refusé ; polkit absent/timeout ⇒ `NotAuthorized` (fail-closed).
- Rate limit : 2/s burst 4 par nom unique **et** UID (`tcc-authz`), avant polkit.
- Audit : `action`, `method`, `caller`, `caller_uid`, `result`, `duration_ms`, `package`/`arg` (nom
  de paquet/argument issu de l'allowlist), `ticket` (UUID), jamais de sortie de commande, de contenu
  de diagnostic, de secret, de chemin.
- `Busy` : une opération par service ; `RateLimited` avant dialogue ; `PolicyDenied` pour AnyDesk non
  retenu/dépôt non signé.

---

## 9. Références LUKS à retirer / requalifier

> **Aucune de ces références ne doit être implémentée.** Elles sont listées pour le nettoyage
> demandé par la décision utilisateur.

**Vérifié le 2026-11-06** : aucune action `luks*` dans `packaging/polkit`, aucun membre `Luks*` dans
`packaging/dbus` (seul un commentaire de périmètre subsiste, `conf:178`), aucun test LUKS dans
`scripts/release/dbus-policy-test.sh` ; `validate.sh` échoue si une référence de packaging
réapparaît. Les références restantes sont **documentaires**.

| Fichier | Référence | Action |
|---|---|---|
| `packaging/polkit/com.tuxedocomputers.tccd.policy` | action `com.tuxedocomputers.tccd1.luks-change-key` | **déjà supprimée** (vérifié 2026-11-06) |
| `packaging/dbus/com.tuxedocomputers.tccd.conf` | denies `LuksListTargets`, `LuksChangePassphrase` | **déjà supprimés** (vérifié 2026-11-06) |
| `scripts/release/dbus-policy-test.sh` | tests de refus LUKS | **déjà retirés** ; `validate.sh` vérifie l'absence de mot-clé LUKS dans le packaging |
| `docs/security/ADMIN_DESIGN.md` | §2.6 LUKS, D5 détaillé, action `luks-change-key`, §5.3, §6, TA-10, AD-10 (`tccd-admin-luks`) | **déjà neutralisé** (2026-11-06) : §2.6 remplacé par une note, action/AD-10 désactivées, D5 historique |
| `docs/security/DECISIONS.md` | D5 LUKS | **déjà requalifié** : règle historique, aucune fonction LUKS |
| `docs/linux/PLATFORM_DESIGN.md` | §LUKS, `tccd-admin-luks`, D5 | **à retirer/requalifier** (design opérationnel) |
| `docs/DBUS_API.md` | phase 3 LUKS / méthodes | **à requalifier** : LUKS ne sera pas exposé |
| `docs/RELEASE_CHECKLIST.md` | test LUKS image loop | **déjà à jour** (`[x]` hors périmètre) |
| `docs/security/CHECKLIST.md` | §3.6 LUKS | **déjà `[-]` hors périmètre** |
| `docs/security/THREAT_MODEL.md` | S-10, F-02, action `luks-change-key`, §8.6, `LimitCORE` LUKS | **déjà requalifié** : historique/hors périmètre, action XML retirée |
| `docs/security/REVIEW_ADMIN_PHASE1.md` | mentions phase 3 LUKS | **déjà nettoyé** (méthodes LUKS retirées des tableaux) |
| `docs/ARCHITECTURE.md` | mentions historiques LUKS (§1, §2, §3.10, §5) ; la décision est déjà mise à jour | aligner les phrases historiques ou les marquer « hors périmètre » |
| `docs/FEATURE_PARITY.md` | déjà mis à jour (`[-]`) | aucune action |
| `docs/security/ADMIN_PHASE2_DESIGN.md` | mention « phase 3 LUKS » | **déjà corrigé** (« LUKS hors périmètre ») |
| `docs/INTEGRATION_PLAN.md` | schéma `tccd-admin (LUKS/Prime/…)`, §phases | retirer LUKS du schéma et des phases |
| `docs/ui/UI_DESIGN.md` | écran Outils LUKS, champs masqués, `ToolCapabilities` | retirer l'écran et les capacités LUKS |
| `docs/ui/PROFILE_EDITING.md` | « LUKS/Tomte/WebFAI (phase 3) » | retirer LUKS |
| `docs/ROADMAP.md` | phase 3 / services annexes | déjà « LUKS exclu » ; vérifier les autres mentions |

### Références hors `docs/security/**` (vérifiées 2026-11-06)

| Fichier:ligne | Contenu | Recommandation |
|---|---|---|
| `docs/DBUS_API.md:377` | « opérations LUKS/Tomte/WebFAI ne sont pas exposées… phases ultérieures » | **À requalifier** : LUKS ne sera pas exposé. |
| `docs/linux/PLATFORM_DESIGN.md:591,773,822,880,900-905,1064,1090,1093` | service LUKS, action, fd, `cryptsetup`, phase 3 | **À retirer** : design opérationnel LUKS. |
| `docs/ARCHITECTURE.md:17,187,203,250,525` | mentions de l'original / risques historiques | **Historique acceptable** si l'analyse de l'original reste clairement identifiée. |
| `docs/ARCHITECTURE.md:536` | règle opérationnelle `cryptsetup --key-file -` | **À retirer** (D5 reste une règle générique sans LUKS). |
| `docs/ARCHITECTURE.md:260,572,583,587` | décision LUKS exclu / D5 historique | **Acceptable** (déjà à jour). |
| `docs/ui/UI_DESIGN.md:352,501,561,567` | écran Outils LUKS, champs masqués, capacités | **À retirer** : aucune UI LUKS. |
| `docs/INTEGRATION_PLAN.md:13,23,96` | schéma `tccd-admin (LUKS/…)`, squelette | **À retirer/requalifier**. |
| `docs/INTEGRATION_PLAN.md:86` | règle « aucun secret LUKS en argv/env » | **Historique acceptable** (règle générique). |
| `docs/qa/TEST_PLAN.md:107,331` | tests LUKS manuels / phase 1 | **À requalifier** « hors périmètre, non testé ». |
| `docs/HANDOFF.md:84` | phase 3 « LUKS/Tomte/WebFAI, service LUKS séparé » | **À retirer/requalifier** ; `:92` est déjà correct. |
| `docs/FEATURE_PARITY.md:115,326` | `[-]` hors périmètre | **Acceptable**. |
| `docs/ROADMAP.md:49,73` | « LUKS hors périmètre / exclu » | **Acceptable**. |
| `docs/RELEASE_CHECKLIST.md:41` | LUKS hors périmètre, action retirée | **Acceptable**. |
| `docs/STATUS.md:188,196,234,253` | LUKS non implémenté / exclu | **Acceptable**. |
| `docs/packaging/{MIGRATION.md:21,README.md:108,303,SECURITY.md:23,377,436,440,VALIDATION.md:62,63,416}` | absence de LUKS / règle D5 | **Acceptable** (pas de design opérationnel). |
| `docs/user/GUIDE.md:161` | LUKS non implémenté | **Acceptable** (limite utilisateur). |

---

## 10. Tests

| Domaine | Cas | Attendu |
|---|---|---|
| **Polkit** | refus, annulé, polkit arrêté, timeout, root | `NotAuthorized`, aucune commande/paquet |
| **Concurrence** | deux installs/`TomteRun` simultanés | `Busy`, un seul enfant/transaction |
| **Rate limit** | 10 appels rapides | `RateLimited` avant polkit, sans dialogue |
| **Tomte** | args hors allowlist, `--`, `|`, module `../x`, 9 args | `InvalidArgs`, aucun exec |
| **Tomte** | timeout, code non nul, sortie > 64 Kio | `Timeout`/`PackageFailed`, sortie non relayée |
| **WebFAI** | paquet absent, déjà installé, dépôt absent | `NotSupported`/no-op/`PolicyDenied` |
| **WebFAI** | checksum faux, signature fausse, TLS invalide, redirection HTTP | `SignatureInvalid`/`DownloadFailed`, fichier supprimé, aucune installation |
| **WebFAI** | PackageKit absent | `NotSupported`, aucun `apt-get` |
| **AnyDesk** | non retenu/dépôt non signé | `PolicyDenied`, aucun téléchargement |
| **Diagnostic** | ticket invalide/inconnu, path traversal | `InvalidArgs`/`NotSupported`, pas de lecture hors cache |
| **Diagnostic** | secret dans la sortie, > 4 Mio, timeout | rapport refusé, aucun upload |
| **Diagnostic** | upload TLS/refus/timeout, cleanup | état cohérent, rapport conservé ou supprimé selon règle |
| **Cache** | symlink, fichier world-readable, mauvais owner | refus (`O_NOFOLLOW`/`fstat`) |
| **Systemd** | `systemd-analyze verify/security` | scores archivés, aucune capacité non justifiée |
| **Policy** | membres exacts, `Properties` absent, actions | `dbus-policy-test.sh` + `xmllint` |

Pas de test LUKS : la fonction est hors périmètre.

---

## 11. Checklist avant activation

1. **Nettoyage LUKS** §9 effectué (action polkit, policy, docs, tests).
2. Vrai polkit pour `tomte-run`, `webfai`, `anydesk`, `diagnostics` (autorisé/refusé/annulé/absent).
3. PackageKit présent et transactions testées sur la distribution cible ; sinon `NotSupported`
   (pas de repli direct sans unité séparée revue).
4. Signatures/checksums WebFAI vérifiés avec un artefact de test **et** un artefact officiel ; TLS
   avec un certificat valide et un certificat invalide.
5. `systeminfos` du paquet : sortie réelle, redaction, aperçu, upload vers l'endpoint de test ;
   absence de secrets.
6. Tomte : `strace` des sous-commandes, capacités/`ReadWritePaths` mesurés, `NoNewPrivileges` validé
   ou exception documentée.
7. Aucun téléchargement exécuté : `grep`/revue montre l'absence de `sh`, `bash`, `pkexec`,
   `Command::new` sur un fichier téléchargé.
8. Units `systemd-analyze verify`, scores archivés ; `PrivateDevices`, `DevicePolicy`, caps vides
   (sauf Tomte justifié).
9. `validate.sh`, `dbus-policy-test.sh` en root, `xmllint`, installation/purge.
10. AnyDesk : décision écrite (détection seule ou hors périmètre).

---

## 12. Risques résiduels

| ID | Risque | Niveau | Décision |
|---|---|---|---|
| P3-R1 | Les scripts de maintenance d'un paquet s'exécutent en root via PackageKit | Moyen | inhérent à l'installation de paquets ; dépôts signés, paquets allowlistés, polkit, audit ; ne pas prétendre à un bac à sable |
| P3-R2 | Tomte peut nécessiter des capacités/écritures larges (`/usr`, modules) | Moyen | mesure obligatoire ; retirer la sous-commande ou scinder le service ; pas d'élargissement en bloc |
| P3-R3 | AnyDesk = accès distant tiers | Élevé (si retenu) | reporté par défaut ; détection seule ; installation/lancement sur décision explicite + revue |
| P3-R4 | Diagnostic : données système sensibles, upload réseau | Moyen | redaction, aperçu, consentement, endpoint fixe, TLS, TTL 24 h |
| P3-R5 | Téléchargement compromis (TLS/signature) | Moyen | HTTPS + checksum + signature + PackageKit ; aucun exécution directe |
| P3-R6 | `systeminfos` absent ou interface instable | Faible | `NotSupported`, pas de téléchargement, version de l'outil exposée |
| P3-R7 | PackageKit absent sur certaines distributions | Faible | `NotSupported` ; repli direct séparé et revu |
| P3-R8 | Timeout/kill d'une transaction de paquet | Moyen | ne pas tuer PackageKit en cours ; attendre la transaction, sinon `Busy`/`Timeout` et état relu |
| P3-R9 | Cache de diagnostic conservé après crash | Faible | TTL 24 h + nettoyage au démarrage |
| P3-R10 | AnyDesk lance un démon root | Élevé (si retenu) | refuser l'auto-start, lancement session, pas de configuration unattended |

---

## 13. Conclusion

La phase 3 se limite à **Tomte, WebFAI, AnyDesk (conditionnel) et diagnostic**, avec des services
séparés, PackageKit pour les paquets, aucune exécution de téléchargement, et des allowlists strictes.
**LUKS est hors périmètre** : aucun design opérationnel, aucune action polkit, aucune capacité, aucun
accès block device. Le premier livrable est le **diagnostic** (pas de paquets/devices), puis WebFAI,
puis Tomte ; AnyDesk reste en détection seule jusqu'à décision explicite. Les références LUKS listées
en §9 doivent être retirées ou requalifiées avant d'ouvrir la phase 3.
