# tccd-user — agent de session et mode d'affichage

Statut : implémentation du jalon 1 avec mode agent D-Bus (lecture JSON de
`tccd`). Ce document décrit le fonctionnement de `crates/tcc-user`, ses limites
et le contrat attendu de `tcc-ipc`. Il complète
`docs/linux/PLATFORM_DESIGN.md` §5.

---

## 1. Rôle et principes

`tccd-user` est un **agent non privilégié** lancé dans la session graphique
(`tccd-user.service`, service systemd utilisateur). Il applique le **mode de
l'écran interne** : résolution et/ou taux de rafraîchissement. Il ne touche ni
à `/etc`, ni à `/var/lib`, ni à `/dev/tuxedo_io`, ni à sysfs.

Principes non négociables :

1. **Refus root** : `ensure_not_root()` lit l'UID effectif et refuse `0`.
   `--help`/`--version` restent disponibles.
2. **Écran interne uniquement** : le connecteur doit commencer par `eDP`,
   `LVDS` ou `DSI` (règle `tcc_hw::display`). Aucun écran externe n'est
   modifié, même en miroir (X11 refuse un CRTC partagé).
3. **Résolution optionnelle et exacte** : si le profil ne demande pas de
   résolution, elle est conservée. Si elle est demandée, le mode
   `largeur × hauteur × taux` doit exister tel quel dans `OutputInfo.modes` ;
   sinon erreur typée, aucune substitution.
4. **Tolérance 0,5 Hz** : un taux déjà appliqué ne provoque aucune écriture.
   L'idempotence compare résolution **et** taux.
5. **Aucun shell, aucun `/proc/*/environ`** (D6). Les sockets IPC sont
   utilisés nativement ; l'environnement de session est celui du service
   utilisateur.
6. **Erreurs typées**, pas d'`unwrap`/`expect` en production.
7. **Pas de nom d'output arbitraire** en argument CLI : seuls le taux et une
   résolution `WxH` sont acceptés. Les backends revalident le nom interne avant
   toute commande.

---

## 2. Modules

| Fichier | Rôle |
|---|---|
| `session.rs` | `SessionEnv`, `SessionType`, `SessionInfo`, détection pure et testable |
| `security.rs` | UID effectif, refus root, validation répertoire runtime/socket |
| `ipc.rs` | Hyprland (socket brut) et Sway (i3-ipc), réponses bornées |
| `backend/mod.rs` | factory, planification `--dry-run`, validation des noms, format Hz |
| `agent.rs` | mode agent : bus système, parsing JSON, cible, backoff, signaux |
| `backend/x11.rs` | XRandR natif via `x11rb` |
| `backend/hyprland.rs` | socket IPC Hyprland + `keyword monitor` |
| `backend/sway.rs` | IPC Sway + `output ... mode ...` |
| `backend/kde.rs` | KDE Plasma via `kscreen-doctor -j` |
| `backend/gnome.rs` | GNOME Wayland via `org.gnome.Mutter.DisplayConfig` |
| `backend/stub.rs` | Wayland générique : `NotSupported` documenté |
| `cli.rs` | parsing d'arguments, rapport JSON, application, codes de sortie |
| `main.rs` | logging + `cli::run` |

---

## 3. Détection de session

Variables lues : `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP` (`:`-séparé),
`WAYLAND_DISPLAY`, `DISPLAY`, `XDG_RUNTIME_DIR`,
`HYPRLAND_INSTANCE_SIGNATURE`, `SWAYSOCK`.

| `XDG_SESSION_TYPE` | Environnement | Backend |
|---|---|---|
| `x11` | `DISPLAY` | X11 |
| `wayland` | desktop Hyprland ou signature Hyprland | Hyprland |
| `wayland` | desktop sway | Sway |
| `wayland` | desktop KDE/Plasma | KDE |
| `wayland` | desktop GNOME | GNOME (Mutter, Wayland uniquement) |
| `wayland` | autre | Wayland générique (stub) |
| `tty` | — | non supporté |
| absent | `WAYLAND_DISPLAY` | inférence Wayland |
| absent | `DISPLAY` seul | X11 |
| absent | rien | non supporté |

L'ordre de détection Wayland est : Hyprland, Sway, KDE, GNOME, puis Wayland
générique (stub). `SessionInfo::has_supported_backend()` retourne vrai pour
X11, Hyprland, Sway, KDE et GNOME. GNOME n'est accepté que si la session est
Wayland (`XDG_SESSION_TYPE=wayland` ou `WAYLAND_DISPLAY` présent) : une session
GNOME X11 utilise XRandR.

---

## 4. Backends

### 4.1 X11 — `x11rb` / XRandR

- Connexion : `x11rb::connect(None)` (100 % Rust, aucune libX11 de dev),
  `randr_query_version(1, 6)` (minimum 1.2).
- Liste : `GetScreenResourcesCurrent`, puis pour chaque output
  `GetOutputInfo`, et pour son CRTC `GetCrtcInfo`.
- Taux d'un mode : `dot_clock / (htotal * vtotal)` (Hz). Les modes
  entrelacés TV ne sont pas ciblés.
- Mode courant : `crtc.mode`. Modes préférés : les `num_preferred` premiers
  modes de l'output.
- Application : `SetCrtcConfig` en conservant CRTC, position (`x`,`y`) et
  rotation. `config_timestamp` vient de `GetScreenResourcesCurrent`.
- **Refus si le CRTC contient un autre output** (miroir) : changer le mode
  toucherait un écran externe. Le message l'explique.
- Échec `SetCrtcConfig` (`status != Success`) → `HwError::Display`.

### 4.2 Hyprland

- Socket : `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`.
- Lecture : commande `j/monitors`, JSON parsé par `serde_json`.
  Champs utilisés : `name`, `width`, `height`, `refreshRate`, `x`, `y`,
  `scale`, `transform`, `disabled`, `availableModes`.
- Modes : `availableModes` de la forme `2560x1440@165.00Hz`.
- Application :
  `keyword monitor <name>,<W>x<H>@<Hz>,<X>x<Y>,<scale>,transform,<t>`.
  Position, scale et transform sont repris du moniteur courant. Le taux est
  formaté sans zéros inutiles (`format_hz`).
- Réponse `ok` attendue ; toute autre réponse est une erreur remontée à l'UI.
- Un moniteur `disabled` est refusé.

### 4.3 Sway

- Socket : `$SWAYSOCK`.
- Protocole i3-ipc : magic `i3-ipc`, longueur et type en **boutisme natif**,
  payload JSON.
- Lecture : `GET_OUTPUTS` (type 3). `refresh` est en millihertz.
- Application : `RUN_COMMAND` (type 0) avec
  `output <name> mode <W>x<H>@<Hz>Hz`. Sway conserve position/scale/transform.
- Réponse `{"success": true}` attendue ; `{"success": false, "error": ...}`
  est remonté.

### 4.4 KDE Plasma

- Lecture : `/usr/bin/kscreen-doctor -j` (timeout 3 s, processus tué au-delà).
- Application : `kscreen-doctor output.<name>.mode.<id>`, sans shell.
- L'écran interne est filtré par `tcc-hw` ; un output externe est refusé avant
  toute commande.

### 4.5 GNOME Wayland — Mutter DisplayConfig

- Bus de session, service `org.gnome.Mutter.DisplayConfig`, objet
  `/org/gnome/Mutter/DisplayConfig`.
- `GetCurrentState()` : serial, moniteurs et modes
  (`a(siiddada{sv})`), moniteurs logiques. Les propriétés `is-current`,
  `is-preferred` et `is-builtin` sont lues défensivement ; une propriété
  absente ou d'un autre type n'est jamais fatale.
- La résolution courante est celle du mode `is-current`. Le backend applique le
  `DisplayMode` complet fourni par `tcc-hw` (résolution et/ou taux) ; aucun
  autre mode n'est sélectionné par le backend.
- Application : `ApplyMonitorsConfig(serial, 1, logical_monitors, {})` avec
  `method=1` (**temporary**, jamais persistant dans `monitors.xml`). Le mode
  est la **chaîne exacte** retournée par Mutter, jamais un `WxH@Hz` reformaté.
- Seul le connecteur interne (`eDP-*`, `LVDS-*`, `DSI-*`) reçoit le nouveau
  mode. Les autres moniteurs conservent leur mode courant ; si celui-ci n'est
  pas exposé, l'application est refusée plutôt que de risquer de modifier un
  écran externe. Un groupe logique en miroir peut échouer si Mutter exige des
  modes compatibles : l'échec est remonté, sans écriture partielle.
- Serial périmé (`AccessDenied`/`InvalidArgs` à l'application) : relecture de
  `GetCurrentState`, reconstruction du plan et **une seule** nouvelle tentative.
- Timeout de 3 s par appel Mutter ; pas de blocage du thread de
  `spawn_blocking`. Session verrouillée/absente ou Mutter indisponible :
  erreur typée, le prochain cycle de l'agent réessaie.
- GNOME **Wayland uniquement** : la factory refuse GNOME sur une session X11
  (XRandR reste utilisé). Aucune commande shell, aucun fallback XRandR sous
  Wayland.

Test manuel GNOME Wayland :

```sh
# session GNOME Wayland, outil CLI
tccd-user --list            # backend name=gnome, available=true, eDP-1
tccd-user --set-refresh 60 --dry-run
tccd-user --set-refresh 60  # doit changer uniquement le taux interne
# vérifier dans GNOME Réglages > Écrans que le mode temporaire est appliqué
# puis rebasculer sur un autre taux ; un écran HDMI doit rester inchangé
```

Limites GNOME : Mutter peut refuser un taux non listé, un groupe miroir
incompatible ou une session verrouillée ; `ApplyMonitorsConfig` reste
temporaire et n'est pas restauré par le compositeur après déconnexion.

### 4.6 Wayland générique

Stub propre, `HwError::NotSupported("Wayland générique non supporté…")`.

Aucun fallback XRandR sous Wayland : l'état « non supporté » est exposé.

---

## 4bis. Sélection et application du mode interne (`tcc-hw`)

`tcc_hw::display` expose une planification pure et une application :

```rust
plan_internal_display_mode(outputs, width, height, refresh_hz, tolerance)
apply_internal_display_mode(backend, width, height, refresh_hz, tolerance)
```

Règles :

- `width`/`height` `None` **ou** `(0, 0)` : conserve la résolution courante et ne
  change que le taux (`refresh_hz` requis dans ce cas).
- `refresh_hz` `None` : conserve le taux courant ; utile pour appliquer une
  résolution seule.
- Si une résolution est demandée, le mode doit exister **exactement** dans
  `OutputInfo.modes` à cette largeur/hauteur et avec un taux dans la tolérance
  (0,5 Hz). Un taux absent à cette résolution ⇒ `DisplayModeNotFound`, aucune
  écriture, aucun fallback vers une autre résolution ou un taux voisin hors
  tolérance.
- Seul l'écran interne connecté (`eDP`/`LVDS`/`DSI`) est sélectionné. Les
  modes des écrans externes ne sont jamais consultés pour le matching.
- Si le mode courant correspond déjà à la résolution et au taux demandés, le
  résultat est `AlreadyApplied` : aucune écriture compositeur.
- Une résolution partielle (`width` sans `height`), nulle ou hors
  `MAX_DISPLAY_DIMENSION` (16 384) est refusée avant tout accès backend.
- Le backend reçoit un `DisplayMode` complet déjà vérifié ; il conserve
  position, scale, rotation et les autres outputs (X11 CRTC, Hyprland
  `keyword monitor`, Sway `output … mode`, KDE id de mode, GNOME mode exact).

Priorité des champs de profil :

1. `display.use_refresh_rate` + `refresh_rate` → taux cible ;
2. `display.use_resolution` + `x_resolution`/`y_resolution` → résolution
   cible ;
3. les deux peuvent être combinés ; `refreshRate = -1` (ou `useRefRate=false`)
   signifie « ne pas changer le taux » et n'empêche pas une résolution seule ;
4. sans aucun des deux, l'agent ne crée même pas de backend.

Limites Wayland/compositeurs :

- un mode non listé par le compositeur n'est jamais inventé ; le CLI
  `--list` expose `internal_output.modes` (largeur, hauteur, taux, préféré,
  courant) pour que l'UI ne propose que des combinaisons applicables ;
- GNOME/Mutter applique un mode **temporaire** ; KDE utilise l'id de mode
  existant ; Hyprland/Sway reformatent `WxH@Hz` ; X11 sélectionne le mode
  XRandR ;
- une session verrouillée, un compositeur absent ou un mode refusé produisent
  une erreur typée ; l'agent réessaie avec backoff ;
- aucun écran externe (HDMI/DP) n'est modifié, y compris en miroir : les
  backends refusent un CRTC partagé ou un output non interne.

Test manuel (session réelle, sans privilège root) :

```sh
tccd-user --list                         # modes disponibles de l'écran interne
tccd-user --set-resolution 1920x1080 --dry-run
tccd-user --set-resolution 1920x1080     # conserve le taux courant
tccd-user --set-resolution 1920x1080 --set-refresh 165 --dry-run
tccd-user --set-resolution 1920x1080 --set-refresh 165
# vérifier qu'un écran HDMI/DP externe conserve son mode
```

## 5. Sécurité des sockets

- `$XDG_RUNTIME_DIR` : chemin absolu, répertoire réel (pas un lien),
  propriétaire = UID de l'agent, mode `0700`.
- Socket : chemin absolu sans `..`, pas un lien symbolique, type socket UNIX,
  propriétaire = UID, non inscriptible par le monde.
- Répertoire parent du socket : propriétaire = UID, non inscriptible par
  groupe/autres.
- Timeouts lecture/écriture de 3 s, réponses limitées à 8 Mio.
- `HYPRLAND_INSTANCE_SIGNATURE` validée (`[A-Za-z0-9._-]`, pas `..`).
- Les credentials du pair ne sont pas interrogés : `peer_cred()` n'est pas
  stable sur la MSRV ; la validation du répertoire 0700 et du socket possédé
  par l'utilisateur protège le chemin.

---

## 6. CLI

```text
tccd-user --list [--dry-run]
tccd-user --set-refresh <hz> [--dry-run]
tccd-user --set-resolution <WxH> [--set-refresh <hz>] [--dry-run]
tccd-user --help | --version
tccd-user
```

- `--list` : rapport JSON sur stdout, logs sur stderr. Code 0 même si le
  backend est non supporté (diagnostic) ; `supported: false` et `error`.
- `--set-refresh` : applique le taux sur l'écran interne, résolution courante.
  Code 0 si appliqué ou déjà appliqué, 1 en cas d'échec/non supporté, 2
  argument invalide.
- `--set-resolution <WxH>` : applique la résolution sur l'écran interne. Sans
  `--set-refresh`, le taux courant est conservé et doit exister à cette
  résolution. `WxH` accepte `x`/`X`, refuse 0, les dimensions partielles et
  > 16 384.
- `--set-resolution` + `--set-refresh` : une seule opération résolution+taux.
- `--dry-run` : planifie sans appliquer (aucune écriture compositeur) ; affiche
  le mode qui serait appliqué ou l'erreur `DisplayModeNotFound`.
- Sans argument : mode agent D-Bus (voir §8), reste actif jusqu'à
  SIGTERM/SIGINT.
- Aucune option `--output` : impossible de cibler un écran arbitraire.

**Données pour l'UI** : `internal_output.modes` expose chaque mode disponible
avec `width`, `height`, `refresh_hz`, `preferred` et `current`, ainsi que le
`current_mode`. `qt-ui` doit proposer uniquement ces combinaisons (et non des
valeurs libres) ; une résolution/taux absent est refusé côté agent. Le backend
est la seule source de vérité pour les modes réellement applicables.

Format du rapport `--list` :

```json
{
  "version": "0.1.0",
  "session": {
    "session_type": "wayland",
    "desktop": "Hyprland",
    "wayland_display": "wayland-1",
    "display": ":0",
    "uid": 1000
  },
  "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
  "internal_output": {
    "name": "eDP-1",
    "connected": true,
    "enabled": true,
    "current_mode": { "width": 2560, "height": 1440, "refresh_hz": 60.0,
                      "preferred": false, "current": true },
    "modes": [
      { "width": 2560, "height": 1440, "refresh_hz": 165.0,
        "preferred": false, "current": false }
    ]
  },
  "supported": true,
  "dry_run": false,
  "error": null
}
```

---

## 7. Tests

| Sujet | Test |
|---|---|
| Détection session | X11, Hyprland (desktop et signature), Sway, KDE, GNOME, tty, Wayland inconnu, inférence sans `XDG_SESSION_TYPE` |
| UID/root | parsing `/proc/self/status`, refus UID 0 |
| Sockets | socket possédé, mauvais propriétaire, lien symbolique, `..`, mode runtime |
| Sélection écran | externe ignoré, interne seul, résolution courante |
| Résolution | résolution seule (taux conservé), résolution+taux, mode absent, 0/None, partielle refusée, idempotence, externe préservé |
| CLI résolution | `WxH`/`WXH`, combinaison avec `--set-refresh`, `--dry-run`, valeurs invalides |
| Tolérance | 59,94 ≈ 60, 165,2 ≈ 165, mode absent |
| Hyprland | JSON réaliste, parsing `WxH@RHz`, commande avec position/scale/transform, refus externe |
| Sway | JSON `GET_OUTPUTS` réaliste, millihertz, commande, output inactif |
| GNOME/Mutter | parsing `GetCurrentState`, propriétés `a{sv}` absentes/typées, écran externe ignoré, mode absent, serial périmé → un retry, refus, timeout, faux service D-Bus session |
| X11 | calcul `dot_clock/(htotal*vtotal)`, totaux nuls |
| Agent JSON | `ActiveProfile` + `ProfileCollection`, `null`, `useRefRate=false`, `refreshRate=-1`, id absent, JSON invalide, payload trop grand |
| Agent décision | cible inchangée (profil/taux), changement de profil ou de taux, cible `None` |
| Agent robustesse | daemon absent simulé (pas de panic), backend non supporté, backoff plafonné |
| CLI | toutes les formes d'arguments, conflits, plages Hz, JSON |

Commandes de validation :

```sh
cargo fmt -p tcc-user -- --check
cargo test -p tcc-user
cargo clippy -p tcc-user --all-targets -- -D warnings
cargo check --workspace
```

---

## 8. Mode agent D-Bus

### 8.1 Boucle de service

Sans argument, `tccd-user` reste actif jusqu'à `SIGTERM`/`SIGINT`. `cli::run`
crée un runtime Tokio `current_thread` uniquement pour ce mode (les commandes
`--list`/`--set-refresh` restent synchrones).

- toutes les 2 s : lecture du profil actif, résolution de la cible, application
  si la cible a changé ;
- daemon absent ou réponse invalide : log `debug`, backoff 2 s → 4 s → 8 s →
  10 s (plafond) ;
- backend non supporté ou échec d'application : log `warn`, backoff séparé,
  l'agent ne quitte pas ;
- arrêt propre sur `SIGTERM`/`SIGINT` (y compris pendant l'attente).

### 8.2 Lecture D-Bus

| Élément | Valeur |
|---|---|
| Bus | système |
| Service | `com.tuxedocomputers.tccd` |
| Objet | `/com/tuxedocomputers/tccd` |
| Interface | `com.tuxedocomputers.tccd` |
| Méthodes | `GetActiveProfileJSON() -> s`, `GetProfilesJSON() -> s` |

- La connexion et le proxy sont **conservés entre les cycles** (R-10) : une
  seule authentification D-Bus, puis réutilisation. Ils sont invalidés et
  reconstruits après une erreur de transport.
- Un seul budget de 3 s couvre tout le cycle (connexion/proxy + deux appels) :
  une absence de daemon ne bloque pas l'agent plus de ~3 s.
- Chaque payload est borné à 4 Mio **avant** parsing.
- JSON traité comme non fiable : `serde_json`, puis
  `ActiveProfile::validate` et `ProfileCollection::validate`.
- `GetActiveProfileJSON` peut renvoyer `null` (aucun profil actif) : parsé en
  `Option<ActiveProfile>`.
- **Compatibilité** : si le daemon renvoie un `Profile` complet (ancienne
  forme), l'agent n'en extrait que l'identifiant ; les réglages sont relus dans
  `ProfileCollection`, validée avant toute application.

### 8.3 Résolution de la cible

1. Profil actif → identifiant.
2. Recherche du profil dans `ProfileCollection`.
3. `display.effective_refresh_rate()` :
   - `None` (`useRefRate=false`, `refreshRate=-1`) ⇒ pas de changement de taux ;
   - taux non fini ou hors `[1, 1000] Hz` ⇒ réponse rejetée.
4. `display.effective_resolution()` :
   - `None` (`useResolution=false`) ⇒ pas de changement de résolution ;
   - largeur/hauteur hors `[1, 16 384]` ⇒ réponse rejetée.
5. Si ni taux ni résolution ne sont demandés ⇒ aucune cible, aucun backend.
6. Validation complète de la collection avant application.
7. Mémorisation de `(profile_id, refresh_hz, resolution)` : la cible n'est
   réappliquée que si le profil, le taux ou la résolution change.
8. Une cible `None` efface la mémoire : revenir au même mode force une
   nouvelle vérification (le backend répondra `AlreadyApplied` si rien n'a
   bougé).

### 8.4 Application

- `apply_internal_display_mode` : écran interne uniquement, résolution et/ou
  taux, tolérance 0,5 Hz. `width`/`height` `None` conservent la résolution ;
  `refresh_hz` `None` conserve le taux. Une combinaison absente ⇒
  `DisplayModeNotFound`, aucune écriture.
- Création paresseuse du backend ; X11/sockets/compositeur sont exécutés dans
  `tokio::task::spawn_blocking` pour ne pas bloquer la boucle async.
- Après une erreur d'application, le backend est jeté et recréé au prochain
  essai (compositeur redémarré, connexion X11 perdue).

### 8.5 Sécurité

- Refus root vérifié dans le CLI **et** dans l'agent.
- Aucun nom d'output, aucune commande, aucun chemin reçu du daemon : seuls le
  profil, le taux et une résolution bornée sont lus, le backend choisit
  l'écran interne.
- Aucun shell, aucune lecture `/proc/*/environ`, aucune écriture `/etc` ou
  `/var/lib`.
- Pas de `unwrap`/`expect` en production ; erreurs typées.

### 8.6 Tests

- parsing `ActiveProfile` + `ProfileCollection` (et forme `Profile` complète) ;
- `null`, `useRefRate=false`, `refreshRate=-1` ⇒ aucune cible ;
- id absent, JSON invalide, payload trop grand ⇒ erreur typée ;
- décision « cible inchangée » (profil, taux, résolution) ;
- résolution seule, résolution+taux, mode absent, 0/None, résolution partielle
  refusée ;
- source daemon absente simulée ⇒ pas de panic ;
- service D-Bus absent ⇒ cycle borné par le timeout de 3 s ;
- `kscreen-doctor` bloqué ⇒ timeout et kill de l'enfant ;
- backend non supporté ⇒ cycle `Unsupported`, pas de sortie ;
- backoff plafonné à 10 s.

---

## 9. Contrat D-Bus attendu de `tcc-ipc`

Le daemon actuel expose les méthodes JSON ci-dessus. `tcc-ipc` fournit déjà les
types typés (`DisplayModeRequest`, `SessionAgentRequest`,
`SessionAgentResponse`) mais ils ne sont pas encore branchés. Le contrat
ci-dessous reste la cible ; il est à valider avec `rust-core`/`security`.

### 9.1 Bus et noms

- Bus **système** pour la communication avec `tccd`.
- `tccd` possède `com.tuxedocomputers.tccd`, objet
  `/com/tuxedocomputers/tccd`, interface typée
  `com.tuxedocomputers.tccd.Manager1`.
- `tccd-user` ne possède **aucun** nom système. Il s'abonne au signal
  `DisplayModeApplyRequested` et appelle une méthode de rapport sur `tccd`.
- Le daemon vérifie l'UID de l'expéditeur (`GetConnectionUnixUser`) et
  l'associe à la session active. Un client local ne peut pas usurper un autre
  utilisateur.

### 9.2 Signal `DisplayModeApplyRequested`

Signature proposée (types natifs D-Bus, pas de JSON) :

```text
DisplayModeApplyRequested(
    s request_id,
    s profile_id,
    u width,
    u height,
    d refresh_hz,
    d tolerance_hz
)
```

- `request_id` : identifiant opaque pour corréler la réponse.
- `width`/`height` : résolution demandée ; `0` signifie « conserver la
  résolution courante ».
- `refresh_hz` : taux demandé ; `0` signifie « conserver le taux courant ».
- `tolerance_hz` : 0,5 Hz par défaut.
- La cible est refusée si la résolution et le taux sont tous deux absents, ou
  si la combinaison n'existe pas dans les modes de l'écran interne.
- Aucun nom d'output : `tccd-user` choisit l'écran interne détecté.
- Le daemon peut aussi exposer une méthode de lecture
  `GetDisplayTarget() -> (u width, u height, d refresh_hz, b use_refresh_rate)`
  pour le démarrage ou la reprise.

### 9.3 Rapport `ReportDisplayModeResult`

Méthode sur `com.tuxedocomputers.tccd.Manager1` (appelée par `tccd-user`) :

```text
ReportDisplayModeResult(
    s request_id,
    s outcome,      -- "applied" | "already_applied" | "unsupported" | "error"
    s backend,      -- "x11" | "hyprland" | "sway" | ...
    s message
)
```

Le daemon expose l'état à l'UI (`display_mode_supported`,
`display_mode_last_error`, `display_mode_backend`) et journalise les échecs.

### 9.4 Identité de session (D6)

`tccd-user` publie les informations que le daemon ne doit pas aller chercher
dans `/proc` :

```text
GetSessionInfo() -> (
    u uid,
    s session_id,        -- logind session id si disponible
    s session_type,      -- "x11" | "wayland" | "tty"
    s desktop,           -- XDG_CURRENT_DESKTOP brut
    s wayland_display,
    s display,
    b agent_ready
)
```

Le service étant utilisateur, `session_id` peut être lu depuis
`XDG_SESSION_ID`/logind et validé avec l'UID. Aucune lecture
`/proc/*/environ` côté daemon.

### 9.5 Capacités d'affichage — service de session `User1`

Le service est publié sur le **bus de session**, pas sur le bus système :

```text
Service   com.tuxedocomputers.tccd.User
Objet     /com/tuxedocomputers/tccd/User
Interface com.tuxedocomputers.tccd.User1
Contrat   1
```

Méthodes **lecture seule** :

| Méthode | Sortie |
|---|---|
| `Ping()` | `b` |
| `GetVersion()` | `s` |
| `GetContractVersion()` | `u` |
| `GetDisplayInfoJSON()` | `s` |

`GetDisplayInfoJSON` retourne :

```json
{
  "available": true,
  "session": {
    "type": "wayland",
    "desktop": "Hyprland",
    "waylandDisplay": "wayland-1",
    "display": null
  },
  "backend": { "kind": "hyprland", "name": "hyprland", "available": true },
  "internalOutput": {
    "name": "eDP-1",
    "connected": true,
    "enabled": true,
    "hasInternalDisplay": true,
    "currentMode": { "width": 2560, "height": 1600, "refreshHz": 60.0,
                     "preferred": false, "current": true },
    "modes": [
      { "width": 2560, "height": 1600, "refreshHz": 60.0,
        "preferred": false, "current": true },
      { "width": 2560, "height": 1600, "refreshHz": 165.0,
        "preferred": false, "current": false }
    ]
  },
  "supported": true,
  "dryRun": false,
  "error": null
}
```

- `internalOutput.modes` ne contient **que** les modes de l'écran interne
  connecté ; un écran externe n'apparaît jamais.
- `available=false` si le backend de session n'a pas pu être créé ou a échoué ;
  `supported=false` si aucun écran interne connecté.
- Le service conserve un cache de **1 s** pour éviter d'interroger le
  compositeur à chaque appel GUI.
- Aucune méthode d'écriture : la GUI modifie le profil via `Admin1`, l'agent
  applique le mode.
- Si le nom est déjà possédé ou le bus de session absent (`DBUS_SESSION_BUS_ADDRESS`
  non fourni par le gestionnaire systemd utilisateur), l'agent journalise et
  continue sans ce service.

### 9.6 Version et compatibilité

- Méthode `GetContractVersion() -> u` sur `com.tuxedocomputers.tccd.User1` ;
  le client compare strictement avec `SESSION_CONTRACT_VERSION` (= 1).
- Une version incompatible ⇒ la GUI n'utilise pas les modes du service et
  affiche un état dégradé ; elle ne tente pas d'appel « au hasard ».
- Tous les payloads sont bornés (chaînes ≤ 256 octets, listes ≤ 128 modes).

### 9.7 Ce que `tccd-user` ne fera jamais par D-Bus

- Changer un mode sur un écran externe.
- Changer la résolution sans demande explicite.
- Exécuter une commande arbitraire ou un chemin fourni par l'appelant.
- Lire `/proc/*/environ`.
- Écrire `/etc`, `/var/lib` ou sysfs.

---

## 10. Limites actuelles et suite

1. GNOME Mutter DisplayConfig est implémenté sous Wayland ; `zwlr_output_manager_v1`
   reste un stub propre. KDE est implémenté via `kscreen-doctor`.
2. Le mode agent **poll** `GetActiveProfileJSON` toutes les 2 s, faute de
   signal D-Bus dédié dans le contrat actuel. Une même connexion/proxy est
   réutilisée entre les cycles ; elle est recréée après erreur. Les types
   typés `tcc-ipc` (`SessionAgentRequest`/`Response`) ne sont pas encore
   branchés ; un signal pousserait les changements sans latence.
3. Pas de watcher hotplug d'écran : une modification de connecteur est prise en
   compte au prochain cycle (création du backend, `list_outputs`).
4. Hyprland : les champs non reproduits du moniteur (VRR, bitdepth, HDR,
   mirror) peuvent revenir à leur valeur par défaut lors d'un
   `keyword monitor`. Position/scale/transform sont préservés ; documenter
   toute extension future.
5. X11 : un CRTC partagé (miroir) est refusé pour ne pas toucher un écran
   externe. Un mode nécessitant un CRTC dédié n'est pas tenté.
6. Sway : un `$SWAYSOCK` placé dans un répertoire accessible au groupe/autres
   est refusé (sécurité) ; utiliser `$XDG_RUNTIME_DIR` (0700).
7. Pas de restauration automatique à l'arrêt : le taux appliqué reste celui du
   compositeur.
8. Environnement systemd utilisateur : `XDG_CURRENT_DESKTOP`,
   `WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE` et `SWAYSOCK` doivent être
   importés dans le gestionnaire systemd utilisateur (la plupart des bureaux le
   font). Sinon l'agent expose `unsupported` ; utiliser
   `systemctl --user import-environment` ou l'intégration systemd du
   compositeur.
9. Compatibilité : un daemon qui renverrait `GetActiveProfileJSON` sous la
   forme d'un `Profile` complet est accepté (seul l'id est utilisé), mais ce
   n'est pas le contrat cible. Une divergence de version de `tcc-ipc` doit être
   traitée par la couche typée, pas masquée indéfiniment.
10. Service de session : bus de session uniquement, lecture seule, cache 1 s.
    Le nom `com.tuxedocomputers.tccd.User` n'est possédé que par un agent ;
    un second agent journalise un avertissement et continue sans interface de
    modes. Aucun redémarrage automatique du service n'est tenté par l'agent.
