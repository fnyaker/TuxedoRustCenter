# Architecture — TuxedoRustCenter

Réécriture 100 % Rust de TUXEDO Control Center (TCC).
Ce document synthétise l'analyse du projet original et fixe l'architecture cible.
Il doit être lu avec `AGENTS.md` et `docs/FEATURE_PARITY.md`.

---

## 1. Objet

Le projet original `tuxedo-control-center-master/` est une application Electron/Angular avec un daemon Node.js privilégié et un addon C++ (N-API) pour `tuxedo-io`. Il gère :

- des profils matériels (CPU, ventilateurs, affichage, webcam, charge, TDP/ODM, NVIDIA) ;
- un dashboard de capteurs ;
- une bascule automatique de profil selon l'alimentation AC/batterie ;
- un service D-Bus système consommé par la GUI ;
- des services annexes (Aquaris, Tomte, WebFAI, support, change de mot de passe LUKS, etc.).

La cible conserve ces fonctions, mais remplace toute la pile par :

- une GUI Qt 6 native, sans composant web ;
- un daemon Rust ;
- un agent de session Rust non privilégié ;
- des bibliothèques natives Linux (sysfs, hwmon, ioctl, D-Bus, XRandR, Wayland) ;
- une installation FHS standard.

---

## 2. Analyse de l'existant

### 2.1 Pile et processus

| Couche | Technologie originale | Remplacement cible |
|---|---|---|
| GUI | Angular + Angular Material + Electron | Qt 6 natif (QML/Qt Quick ou Widgets), cxx-qt |
| Daemon | Node.js packagé (`pkg`), root | Rust (tokio + zbus) |
| IPC GUI ↔ daemon | D-Bus système | D-Bus système, contrat versionné |
| Accès `tuxedo-io` | addon C++ N-API, ioctl | crate Rust `tcc-hw`, ioctl encapsulés |
| Capteurs/contrôle | sysfs, hwmon, RAPL, nvidia-smi | équivalents Rust natifs |
| Webcam UVC | script Python `cameractrls.py` | V4L2 Rust natif |
| Aquaris | Node BLE (`node-ble`) | Bluetooth Rust (BlueZ D-Bus / btleplug) |
| Packaging | Electron Builder, `/opt` | CMake/Cargo + paquets deb/rpm/Arch, FHS |

### 2.2 Arborescence originale utile

```
src/
├── ng-app/                 GUI Angular
├── e-app/                  processus principal Electron + IPC
├── service-app/            daemon Node.js (tccd)
├── common/                 modèles, contrôleurs sysfs, logique partagée
├── native-lib/             addon C++ tuxedo-io
└── dist-data/              units systemd, D-Bus, polkit, udev, desktop
```

Fichiers de référence particulièrement importants :

- `src/service-app/classes/TuxedoControlCenterDaemon.ts` : orchestration, config, profils, workers.
- `src/service-app/classes/StateSwitcherWorker.ts` : bascule AC/batterie.
- `src/common/classes/StateUtils.ts` : lecture de l'état d'alimentation.
- `src/common/classes/TccPaths.ts` : chemins (à remplacer, car `/opt`).
- `src/service-app/classes/TccDBusInterface.ts` + `TccDBusService.ts` : API D-Bus.
- `src/common/classes/ConfigHandler.ts` : lecture/écriture JSON.
- `src/common/models/TccProfile.ts`, `TccSettings.ts`, `TccFanTable.ts`, `DefaultProfiles.ts`.
- `src/service-app/classes/*Worker.ts` : CPU, fan, backlight, webcam, charge, GPU, ODM, TDP, Prime.
- `src/service-app/classes/DisplayRefreshRateWorker.ts` + `src/common/classes/XDisplayRefreshRateController.ts` : refresh rate X11.
- `src/native-lib/tuxedo_io_lib/*` : ioctl Clevo/Uniwill.
- `src/ng-app/app/*` : écrans et fonctionnalités UI.

### 2.3 Daemon original

- Démarre en root, refuse de tourner sans root.
- Un PID file `/var/run/tccd.pid`.
- Charge `/etc/tcc/settings`, `/etc/tcc/profiles`, `/etc/tcc/webcam`, `/etc/tcc/fantables`.
- Instancie des workers avec des intervalles propres :
  - état : 2 s ;
  - CPU : 10 s ;
  - ventilateurs : 1 s ;
  - charge : 10 s ;
  - webcam : 2 s ;
  - display refresh rate : quelques secondes ;
  - GPU/RAPL : à la demande ;
  - D-Bus : 1,5 s.
- Expose des méthodes D-Bus renvoyant souvent des chaînes JSON.
- Gère `SIGHUP` pour recharger la config et `SIGTERM`/`SIGINT` pour sortir.
- Service `tccd-sleep.service` arrête le daemon avant la veille et le redémarre après.

### 2.4 API D-Bus originale

- Nom : `com.tuxedocomputers.tccd`.
- Objet : `/com/tuxedocomputers/tccd`.
- Interface : `com.tuxedocomputers.tccd`.
- Nombreuses méthodes `Get*JSON()` et quelques setters (`SetTempProfile`, `SetCharge*`, `SetFnLockStatus`, etc.).
- Signal `ModeReapplyPendingChanged`.
- Politique D-Bus dans `src/dist-data/com.tuxedocomputers.tccd-dbus.conf`.
- Action polkit `com.tuxedocomputers.tccd` pour lancer le daemon et écrire la config.

La cible peut conserver ce nom pour la compatibilité descendante, mais doit ajouter une interface typée et versionnée. Si l'original et la réécriture doivent coexister, prévoir un namespace alternatif documenté.

### 2.5 Configuration originale

Fichiers JSON, root:root, mode 0644 :

- `/etc/tcc/settings` :
  - `fahrenheit`, `stateMap` (`power_ac`, `power_bat`), `shutdownTime`,
  - `cpuSettingsEnabled`, `fanControlEnabled`, `keyboardBacklightControlEnabled`,
  - `ycbcr420Workaround`, `chargingProfile`, `chargingPriority`,
  - `keyboardStates`, `lightbarStates`.
- `/etc/tcc/profiles` : profils personnalisés.
- `/etc/tcc/fantables` : courbes de ventilateurs.
- `/etc/tcc/webcam` : préréglages webcam.

Un profil contient :

```
id, name, description,
display { brightness, useBrightness, refreshRate, useRefRate, xResolution, yResolution, useResolution },
cpu { onlineCores, useMaxPerfGov, scalingMinFrequency, scalingMaxFrequency, governor,
      energyPerformancePreference, noTurbo },
webcam { status, useStatus },
fan { useControl, fanProfile, minimumFanspeed, maximumFanspeed, offsetFanspeed, customFanCurve },
odmProfile { name },
odmPowerLimits { tdpValues },
nvidiaPowerCTRLProfile { cTGPOffset }
```

La migration doit accepter ces clés telles quelles, puis ajouter les nouvelles clés (matrice PPD, `schema_version`).

### 2.6 Intégration système originale

- `tccd.service` : `Type=simple`, `ExecStart=/opt/.../tccd --start`, `ExecStop=... --stop`.
- `tccd-sleep.service` : stop/start autour de la veille.
- `com.tuxedocomputers.tccd-dbus.conf` : autorise le root à posséder le nom, tout le monde à envoyer/recevoir.
- `com.tuxedocomputers.tccd-service.policy` : polkit avec `auth_admin_keep`, chemin `/opt`.
- `99-webcam.rules` : udev appelle `python3 cameractrls.py` au branchement d'une webcam.
- `tuxedo-control-center-tray.desktop` : autostart de la tray.

### 2.7 Couche matérielle originale

| Domaine | Mécanisme original |
|---|---|
| CPU | `/sys/devices/system/cpu`, cpufreq, governors, EPP, `no_turbo`, online cores |
| RAPL | `/sys/devices/virtual/powercap/intel-rapl/...` |
| Ventilateurs | `tuxedo_tuxi_sensors` (hwmon), PWM hwmon `tuxedo`, `/dev/tuxedo_io` (Clevo/Uniwill) |
| Backlight | `/sys/class/backlight`, quirk amdgpu `actual_brightness` 0xffff |
| Webcam | kill switch `tuxedo-io` (Clevo), contrôles UVC via `cameractrls.py` |
| Charge | `/sys/devices/platform/tuxedo_keyboard/charging_profile`, `charging_priority`, `charge_control_*` |
| Clavier/lightbar | `/sys/.../leds/rgb:kbd_backlight*`, `white:kbd_backlight`, `rgb:lightbar`, UPower |
| ODM | `tuxedo_platform_profile`, `/sys/firmware/acpi/platform_profile` |
| TDP | `tuxedo-io` : PL1/PL2/PL4 |
| NVIDIA cTGP | `/sys/devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset`, `nvidia-smi` |
| Prime | `prime-select` |
| YCbCr 4:2:0 | `/sys/kernel/debug/dri/<card>/<port>/force_yuv420_output` |
| Fn-Lock | `/sys/devices/platform/tuxedo_keyboard/fn_lock` |
| DMI | `/sys/class/dmi/id/product_sku`, vendor/board/chassis |
| GPU | DRM, hwmon AMD, `nvidia-smi`, RAPL Intel iGPU |

### 2.8 Bascule d'état originale

`StateUtils.determineState()` :

1. état par défaut : AC ;
2. parcourt `/sys/class/power_supply/*` ;
3. cherche un supply de type `Mains` ;
4. lit `online` : vrai → `power_ac`, faux → `power_bat`.

`StateSwitcherWorker` applique ensuite `settings.stateMap[state]`, avec un profil temporaire manuel qui reste actif tant que l'état ne change pas.

**Limite** : aucun support de power-profiles-daemon, aucun état combiné.

### 2.9 Refresh rate original

- Uniquement X11.
- `XDisplayRefreshRateController` retrouve les variables d'environnement de la session via `/proc`, filtre les greeters, puis appelle `xrandr`.
- Détecte `eDP*`/`LVDS*`, lit les modes et applique le taux demandé via `xrandr --output ... --mode ... --rate ...`.
- Aucun support Wayland. Aucun agent de session séparé.

**Cible** : conserver X11, ajouter Wayland via un agent utilisateur, et ne cibler que l'écran interne.

### 2.10 UI originale

Écrans :

- Dashboard (moniteur système) : CPU temp/freq/fan/power, iGPU/dGPU temp/freq/power, profil actif, mode graphique ;
- Profiles : preset/custom, filtres, clone, nouveau, import/export ;
- Éditeur de profil : nom, description, états d'activation, écran (luminosité, refresh rate, résolution), CPU, webcam, ventilateurs, ODM/TDP, NVIDIA cTGP ;
- Tools : changement de mot de passe LUKS, minuterie d'extinction, éclairage, webcam ;
- Aquaris : ventilateur/pompe/LED Bluetooth ;
- Settings : global (Celsius/Fahrenheit, CPU, ventilateurs, éclairage), YCbCr, charge, thème, Prime, Tomte ;
- Support : diagnostic système, WebFAI, contact ;
- Info : version, changelog, mentions ;
- Tray : profil temporaire, autostart, inhibiteur de veille, Fn-Lock, mode graphique ;
- Langues : anglais/allemand (extensible).

Style : interface claire, cartes, sidebar, jauges, accent rouge `#E30016`, jauges `#E22131`, police Titillium Web.

### 2.11 Dettes et limites de l'original

- Electron/Angular : empreinte mémoire et démarrage lourds.
- Daemon Node.js packagé : difficile à auditer, dépendances nombreuses.
- `/opt` imposé partout (units, polkit, udev).
- `cameractrls.py` : dépendance Python et parsing de commandes.
- Changement de mot de passe LUKS : concaténation de shell avec les mots de passe (risque majeur).
- `pkexec /bin/sh -c` : surface d'attaque importante.
- Refresh rate : X11 seulement.
- Bascule : AC/batterie seulement.
- Certaines méthodes D-Bus renvoient du JSON non typé.
- Beaucoup de polling, y compris pour des états peu changeants.

---

## 3. Architecture Rust cible

### 3.1 Workspace

```
Cargo.toml (workspace)
crates/
├── tcc-core/      modèles, profils, settings, état, fan curves, migration, validation
├── tcc-ipc/       types D-Bus partagés, sérialisation, version de contrat
├── tcc-hw/        sysfs, hwmon, ioctl tuxedo-io, DMI, DRM, backlight, V4L2, display backends
├── tcc-daemon/    binaire tccd : zbus, workers, orchestration, persistance
├── tcc-user/      binaire tccd-user : agent de session, refresh rate
├── tcc-gui/       binaire Qt 6 (cxx-qt + QML)
├── tcc-cli/       diagnostic, profils, capteurs (non privilégié)
├── tcc-authz/     client polkit partagé (`CheckAuthorization`, fail-closed)
└── tcc-admin/     binaire tccd-admin : service `Admin1` root à la demande
```

Règles :

- `tcc-core` ne dépend d'aucun matériel ni de Qt ; logique pure, testable.
- `tcc-hw` expose des traits et des implémentations réelles + fake.
- `tcc-daemon` et `tcc-gui` dépendent de `tcc-ipc`, pas l'inverse.
- `tcc-authz` ne dépend pas de `tcc-hw` ; `tcc-admin` ne dépend pas des workers
  matériels du daemon.
- Pas de dépendance circulaire. Les types D-Bus sont la frontière.
- MSRV et édition à fixer par `rust-core` ; proposer edition 2024 et une MSRV récente stable.

### 3.2 Processus et privilèges

```
┌────────────────────────────────────────────────────┐
│ GUI tcc-gui (utilisateur)                          │
│ Qt 6, aucun privilège                              │
└──────────────┬──────────────────────┬──────────────┘
               │ D-Bus système        │ D-Bus système (actions sensibles)
┌──────────────▼───────────────┐  ┌───▼──────────────────────────┐
│ tccd (root, systemd)         │  │ tccd-admin (root, à la demande)│
│ profils, workers, matériel   │  │ Prime, Tomte, WebFAI, diag.   │
└──────────────┬───────────────┘  └──────────────────────────────┘
               │ D-Bus système (signal profil actif)
┌──────────────▼───────────────┐
│ tccd-user (utilisateur)      │
│ refresh rate, compositeur    │
└──────────────────────────────┘
```

- `tccd` : root uniquement pour les écritures sysfs/ioctl et la persistance. Pas de session graphique.
- `tccd-admin` : service root distinct, activé à la demande, sans état persistant. Phase 1 : écriture persistante des profils/settings (`Admin1.SaveProfilesJSON`, `SaveSettingsJSON`) puis `Control1.ReloadConfig` root-only sur `tccd`. Phase 2 (design `ADMIN_PHASE2_DESIGN.md`) : services **séparés** `Admin.Hw` (TDP/ODM/cTGP, `DeviceAllow=/dev/tuxedo_io`) et `Admin.Prime` (`prime-select`, `ReadWritePaths=/boot`) ; ne pas mélanger le parser JSON de configuration avec les ioctl/exec. Phase 3 : Tomte, WebFAI, diagnostic, dans un service encore plus restreint ; **le changement de mot de passe LUKS est hors périmètre (décision utilisateur)**. Chaque méthode vérifie polkit avec le sujet `system-bus-name` de l'appelant, même pour root. Un bug de parsing dans `tccd` ne donne jamais accès à ces opérations.
- Les écritures persistantes vont exclusivement dans `/var/lib/tuxedo-rust-center` ; `/etc` reste en lecture seule pour les deux services.
- `tcc-gui` : utilisateur, aucune écriture privilégiée. Consomme l'API D-Bus.
- `tccd-user` : service systemd utilisateur, lancé dans la session graphique. Applique le refresh rate via le compositeur. Aucun privilège.
- `tccd` peut exposer des méthodes d'écriture de config protégées par polkit ; la GUI ne doit jamais écrire `/etc` ni `/var/lib` directement.

### 3.3 Flux D-Bus

Deux niveaux :

1. **API de compatibilité** : nom `com.tuxedocomputers.tccd`, objet `/com/tuxedocomputers/tccd`, méthodes historiques (`GetProfilesJSON`, `GetSettingsJSON`, `SetTempProfile`, etc.). Utile aux outils tiers et à la migration.
2. **API typée versionnée** : nouvelle interface (ex. `com.tuxedocomputers.tccd.Manager1`) avec types D-Bus natifs (`a{sv}`, structs) et signaux :
   - `ActiveProfileChanged(profile_id, profile_json)` ;
   - `StateChanged(ppd_profile, power_source, active_profile_id)` ;
   - `SensorDataChanged(json)` ;
   - `DisplayModeApplyRequested(json)` ou événement dédié.

La GUI doit pouvoir fonctionner sans l'API de compatibilité. L'API typée est la source de vérité pour le nouveau client.

### 3.4 Modèle de données

Types principaux :

```rust
struct Profile {
    id: ProfileId,
    name: String,
    description: String,
    display: DisplayProfile,
    cpu: CpuProfile,
    webcam: WebcamProfile,
    fan: FanProfile,
    odm_profile: OdmProfile,
    odm_power_limits: OdmPowerLimits,
    nvidia_power_ctrl_profile: NvidiaPowerCtrlProfile,
}

struct DisplayProfile {
    brightness: u8,
    use_brightness: bool,
    refresh_rate_hz: Option<f32>, // None ou -1 historique = ne pas changer
    use_refresh_rate: bool,
    x_resolution: Option<u32>,
    y_resolution: Option<u32>,
    use_resolution: bool,
}

enum PowerSource { Ac, Battery, Unknown }

enum PpdProfile { PowerSaver, Balanced, Performance, Unknown(String) }

struct StateKey {
    ppd: Option<PpdProfile>,
    power: PowerSource,
}

struct Settings {
    schema_version: u32,
    fahrenheit: bool,
    state_map: LegacyStateMap,              // power_ac / power_bat
    state_matrix: BTreeMap<StateKey, ProfileId>, // nouveau
    cpu_settings_enabled: bool,
    fan_control_enabled: bool,
    keyboard_backlight_control_enabled: bool,
    ycbcr420_workaround: ...,
    charging_profile: Option<String>,
    charging_priority: Option<String>,
    keyboard_states: ...,
    lightbar_states: ...,
}
```

Sérialisation :

- `serde` avec `rename_all = "camelCase"` ou alias explicites pour lire l'original.
- Champs inconnus conservés dans un `extra: serde_json::Map` si nécessaire pour la migration.
- IDs stables, générés de façon déterministe ou aléatoire cryptographique.
- Validation centralisée dans `tcc-core::validate`.

### 3.5 Moteur de bascule automatique

#### Entrées

- **PPD** : proxy `net.hadess.PowerProfiles`, objet `/net/hadess/PowerProfiles`, interface `net.hadess.PowerProfiles`, propriété `ActiveProfile` (`s`), signal `PropertiesChanged`. Les valeurs usuelles : `power-saver`, `balanced`, `performance`.
- **Alimentation** : UPower `org.freedesktop.UPower` propriété `OnBattery` et/ou lecture sysfs `/sys/class/power_supply/*` type `Mains` + `online`. Utiliser UPower en priorité, sysfs en fallback.
- **Override manuel** : profil temporaire demandé par la GUI ou la tray.

#### Algorithme

```
inputs = { ppd, power, manual_override }
key = StateKey { ppd, power }

profile =
    manual_override (si actif et état inchangé)
    sinon state_matrix[key]
    sinon legacy_state_map[power]
    sinon default_profile

si key a changé :
    manual_override = None

si key est stable depuis debounce_ms :
    apply(profile)
```

- Debounce 500–1000 ms sur les signaux PPD/UPower pour éviter les rafales.
- Un changement de PPD **ou** d'alimentation invalide l'override temporaire, comme l'original le fait pour AC/BAT.
- Si PPD est absent : `state_matrix` n'a pas de clé, fallback AC/BAT.
- Si PPD est inconnu : fallback AC/BAT, log une fois.
- Si aucune batterie/AC n'est détecté : `Unknown`, utiliser le profil par défaut sans oscillation.
- Reappliquer le profil si la config change (`SIGHUP` ou signal D-Bus).

#### Matrice par défaut proposée

| PPD | AC | Batterie |
|---|---|---|
| `power-saver` | Max Energy Save | Max Energy Save |
| `balanced` | Office | Quiet |
| `performance` | High Performance | Office |
| absent | `stateMap.power_ac` | `stateMap.power_bat` |
| inconnu | fallback AC/BAT | fallback AC/BAT |

Cette matrice est un point de départ ; elle doit être éditable dans l'UI.

#### Conflit avec PPD

PPD écrit aussi `platform_profile` et l'EPP. Deux stratégies sont possibles :

1. **PPD propriétaire (défaut recommandé)** : TCC n'écrit pas `platform_profile` ni l'EPP global quand PPD est actif ; il utilise PPD comme signal et conserve la main sur les limites CPU, les cœurs, les ventilateurs, l'affichage, la webcam et la charge.
2. **TCC prioritaire** : TCC applique ses champs ODM/EPP après PPD, avec un réglage explicite et un log d'avertissement.

Le défaut doit être la stratégie 1 pour éviter les combats de writers. Toute dérogation doit être une option consciente, documentée dans les settings.

### 3.6 Refresh rate de l'écran interne

#### Modèle

- `DisplayProfile.refresh_rate_hz` : `Option<f32>`, `use_refresh_rate`.
- Migration : `refreshRate == -1` → `None` ; `useRefRate == false` → ne pas appliquer.
- Résolution optionnelle conservée pour la parité, mais le refresh rate est la priorité.

#### Sélection de l'écran

- X11 : output dont le nom commence par `eDP`, `LVDS` ou `DSI`. Vérifier via `xrandr`/XRandR que l'output est connecté.
- Wayland : le compositeur expose les outputs ; filtrer sur le connecteur interne (`eDP-1`, `LVDS-1`, `DSI-1`, etc.). Ne jamais modifier les autres outputs.
- Ne pas se baser uniquement sur l'index 0 : un laptop peut avoir un écran externe branché.

#### Backends

| Session | Backend | Détails |
|---|---|---|
| X11 | XRandR via `x11rb` | `GetScreenResourcesCurrent`, `GetCrtcInfo`, `GetOutputInfo`, `SetCrtcConfig` ; choisir le mode au taux le plus proche |
| Hyprland | socket IPC natif | `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` ; commandes `monitors` / `keyword monitor` ; parsing JSON |
| GNOME | D-Bus `org.gnome.Mutter.DisplayConfig` | `GetCurrentState`, `ApplyMonitorsConfig`, respecter le serial et la méthode |
| KDE | KScreen D-Bus / `kscreen-doctor` | privilégier D-Bus ; fallback CLI documenté et sans shell |
| Sway | IPC Sway | commande `output <name> mode <width>x<height>@<rate>Hz` |
| wlroots générique | `zwlr_output_manager_v1` | via `wayland-client` ; configuration puis `apply` |
| Autre Wayland | aucun | état « non supporté », pas de fallback XRandR |

#### Application

1. Lire les modes disponibles de l'écran interne.
2. Chercher le taux demandé à la résolution courante (tolérance 0,5 Hz).
3. Si trouvé et différent du taux actuel : appliquer.
4. Si non trouvé : log, état d'erreur, ne pas changer la résolution sauf demande explicite.
5. Sérialiser les applications par output ; coalescer les demandes rapprochées.
6. Ne pas appliquer si la session est verrouillée/absente ; réessayer au prochain signal.
7. Ne jamais faire planter le daemon en cas d'échec.

#### Agent de session

- Service `tccd-user.service` (systemd user), lancé avec `graphical-session.target`.
- Se connecte au bus système, lit le profil actif et écoute `ActiveProfileChanged`.
- Applique uniquement la partie affichage (refresh rate/résolution).
- Aucun privilège, aucune écriture `/etc`.
- Si l'agent n'est pas lancé, le daemon expose `display_mode_supported=false` et l'UI le signale.
- La luminosité reste dans le daemon (sysfs), car elle ne nécessite pas le compositeur.

#### Multi-GPU / PRIME

- L'écran interne peut être relié à l'iGPU ou au dGPU. L'identification se fait par connecteur et EDID, pas par GPU.
- Ne pas toucher au mode des écrans externes lors d'un changement de profil.
- En mode PRIME, le compositeur reste la seule source de vérité pour les modes.

### 3.7 Couche matérielle

Traits proposés :

```rust
trait CpuControl { ... }
trait FanControl { ... }
trait BacklightControl { ... }
trait WebcamSwitch { ... }
trait ChargingControl { ... }
trait LightingControl { ... }
trait OdmProfileControl { ... }
trait TdpControl { ... }
trait NvidiaCtgpControl { ... }
trait DisplayModeBackend { ... }
```

- `SysfsRoot` injectable (`/sys` réel ou répertoire de test).
- `tuxedo-io` : constantes ioctl recopiées et commentées depuis `tuxedo_io_ioctl.h`, wrapper `unsafe` isolé, validation des index/valeurs.
- Détection : DMI + modèle Uniwill (`R_UW_MODEL_ID`) + quirks par device.
- Ventilateurs : priorité `tuxi` > `pwm` > `tuxedo-io`, comme l'original ; courbes CPU/GPU, hystérésis, min/max/offset.
- CPU : governors, EPP, min/max, no_turbo, online cores, quirks `intel_pstate`/`amd-pstate-epp`/`acpi-cpufreq`.
- Backlight : sysfs, quirk amdgpu.
- Webcam : kill switch `tuxedo-io` si disponible ; contrôles UVC via V4L2 (brightness, contrast, exposure, etc.) ; preview via Qt Multimedia natif.
- Charge : profils/priorités/seuils/types via sysfs.
- Lighting : LED sysfs, UPower pour le lid, modes statique/respiration, zones RGB/per-key/white.
- ODM/TDP/NVIDIA : mêmes interfaces que l'original, avec validation des plages.
- Hotplug : udev netlink/inotify pour power supply, DRM, backlight, LED ; timers seulement en fallback.

### 3.8 Configuration, migration et persistance

- Configuration administrateur, lecture seule pour le daemon : `/etc/tuxedo-rust-center/` (valeurs par défaut, overrides packagés).
- État mutable (profils/settings/fantables/webcam) : `/var/lib/tuxedo-rust-center/{settings,profiles,fantables,webcam}.json`, root:root 0644.
- `schema_version` obligatoire.
- Écritures atomiques : fichier temporaire dans le même répertoire, `fsync`, `rename`, `fchmod` explicite, propriétaire root.
- Lecture tolérante : fichier absent/corrompu → sauvegarde `.bak` root:root 0600, valeurs par défaut, log.
- Migration depuis `/etc/tcc/*` au premier démarrage : lire, valider, convertir, sauvegarder l'ancien.
- Les overrides de développement (`TCC_SYSFS_ROOT`, `TCC_CONFIG_DIR`) sont derrière une feature Cargo `dev-overrides`, absente des paquets release, et refusés si `geteuid() == 0`.
- Import/export de profils : validation stricte (taille, types, plages, noms, doublons), pas de chemins arbitraires.
- Les IDs de profils ne doivent pas être réutilisés après suppression.

### 3.9 Packaging et installation FHS

| Contenu | Chemin |
|---|---|
| GUI | `/usr/bin/tuxedo-rust-center` |
| Daemon | `/usr/libexec/tuxedo-rust-center/tccd` |
| Admin à la demande | `/usr/libexec/tuxedo-rust-center/tccd-admin` |
| Agent session | `/usr/libexec/tuxedo-rust-center/tccd-user` |
| Unit système | `/usr/lib/systemd/system/tccd.service` |
| Unit admin | `/usr/lib/systemd/system/tccd-admin.service` (ou activation D-Bus) |
| Unit utilisateur | `/usr/lib/systemd/user/tccd-user.service` |
| Config admin (RO daemon) | `/etc/tuxedo-rust-center/` |
| État mutable | `/var/lib/tuxedo-rust-center/` |
| Runtime | `/run/tuxedo-rust-center/` |
| QML/assets | `/usr/share/tuxedo-rust-center/` |
| Desktop | `/usr/share/applications/` |
| Icônes | `/usr/share/icons/hicolor/scalable/apps/` |
| Metainfo | `/usr/share/metainfo/` |
| D-Bus policy | `/usr/share/dbus-1/system.d/` |
| Polkit | `/usr/share/polkit-1/actions/` |
| Udev | `/usr/lib/udev/rules.d/` |
| Logs | journald |

- Aucun `/opt`, aucun runtime Node/Python.
- Utiliser `GNUInstallDirs` (CMake) et les chemins de `cargo-deb`/`cargo-generate-rpm` ou des manifests distro.
- Unit `tccd.service` : `After=dbus.service`, `Wants=power-profiles-daemon.service` (si présent), `Restart=on-failure`.
- `tccd-sleep.service` pour stop/start autour de la veille, ou `ExecStop`/`ExecStart` via `systemd-sleep` hooks.
- `tccd-user.service` : `PartOf=graphical-session.target`, `After=graphical-session.target`.

### 3.10 Sécurité

Modèle de menace :

- utilisateur local non privilégié ;
- client D-Bus malveillant ;
- profil/import JSON piégé ;
- symlink/TOCTOU sur sysfs ou config ;
- valeur matérielle hors plage ;
- DoS du daemon (boucles, allocations, appels répétés) ;
- fuite de secrets (mot de passe LUKS, tokens) ;
- dépendances compromises.

Principes :

- GUI sans privilège, daemon root minimal, agent session utilisateur, `tccd-admin` séparé pour les opérations sensibles.
- Pas de setuid ; polkit pour les actions sensibles.
- Politique D-Bus restrictive ; authentification et rate limiting sur les méthodes sensibles.
- Validation centralisée des plages et allowlists.
- `openat2`/`O_NOFOLLOW` pour les fichiers de config ; vérifier type, propriétaire, permissions.
- Aucune commande shell ; `Command::new` avec arguments séparés.
- Secrets (règle générique, aucune fonction LUKS implémentée) : jamais en argv/env/chaîne D-Bus ; transport par fd, `zeroize`, `LimitCORE=0`.
- `/etc` en lecture seule pour le daemon ; état mutable dans `/var/lib`.
- systemd hardening : `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `PrivateDevices` avec `DeviceAllow` ciblé, `RestrictAddressFamilies`, `SystemCallFilter`, `CapabilityBoundingSet`, `MemoryDenyWriteExecute`, `LockPersonality`, `LimitCORE=0`.
- **`ProtectKernelTunables=no`** : indispensable pour conserver les écritures `/sys` (ventilateurs, TDP, charge, LED).
- Audit : `cargo audit`, `cargo deny`, revue `unsafe`, fuzzing des entrées JSON/D-Bus.

Les décisions détaillées D1–D7 sont dans `docs/security/DECISIONS.md`.

### 3.11 Performance

- Event-driven par défaut : signaux PPD/UPower, udev, inotify, D-Bus.
- Capteurs : polling uniquement quand un client a demandé la collecte (`SetSensorDataCollectionStatus`).
- Ventilateurs : boucle 1 s incontournable, mais lectures minimales et courbes précalculées.
- CPU validation : 5–10 s, ou événements cpufreq si le coût est maîtrisé.
- État : signaux + fallback 5 s.
- GUI : modèles Rust → QML, pas de logique JS lourde, pas de re-render inutile ; QML profiler.
- Budgets indicatifs : daemon inactif < 1 % CPU, < 30 Mo RSS ; GUI < 200 Mo RSS, démarrage < 1,5 s. À mesurer et ajuster.

### 3.12 Tests

- Unitaires : matrice d'état, courbes de ventilateurs, migration, validation, parsing des modes.
- Intégration : sysfs factice, D-Bus mocké, profils JSON réels de l'original.
- Propriétés/fuzzing : JSON de profils, méthodes D-Bus, noms/chemins.
- UI : Qt Quick Test ou Qt Test, captures de référence.
- Matériel : tests smoke optionnels sur machine TUXEDO, jamais requis en CI.
- CI : conteneurs Debian/Fedora/Arch, `cargo fmt`, `clippy`, `test`, `deny`, `audit`.

---

## 4. Jalons

1. **Socle** : workspace, CI, modèles, config, migration, tests purs.
2. **Daemon minimal** : D-Bus, PPD/UPower, moteur de bascule, CPU, backlight, ventilateurs.
3. **GUI shell** : design system Qt 6, sidebar, dashboard, profils.
4. **Parité matérielle** : charge, lighting, webcam, TDP/ODM, NVIDIA, Prime, YCbCr, Fn-Lock.
5. **Refresh rate** : X11 + agent session + backends Wayland.
6. **Services annexes** : Aquaris, Tomte, WebFAI, support, minuterie, tray (LUKS exclu).
7. **Packaging, hardening, performance, i18n, documentation**.

---

## 5. Décisions par défaut et points ouverts

Décisions par défaut :

- Qt Quick Controls 2 + cxx-qt (UI native, itérative, bonne séparation Rust/QML).
- Nom D-Bus historique `com.tuxedocomputers.tccd` + interface typée additionnelle.
- `tccd-admin` séparé pour Prime/Tomte/WebFAI (D1) ; LUKS exclu du périmètre.
- État mutable dans `/var/lib/tuxedo-rust-center`, `/etc` en lecture seule pour le daemon (D2).
- Compatibilité JSON legacy gelée en lecture seule (D3).
- PPD propriétaire de `platform_profile`/EPP ; TCC propriétaire du reste (D4).
- D5 (secrets LUKS par fd) reste une règle de sécurité, mais aucune fonction LUKS n'est implémentée.
- Pas de lecture de `/proc/*/environ` ; l'agent de session fournit l'environnement (D6).
- Overrides dev derrière la feature `dev-overrides`, refusés en root (D7).
- Matrice par défaut ci-dessus.
- Hyprland prioritaire pour le développement Wayland, puis GNOME/KDE/Sway/wlroots.
- Packaging cible : deb, rpm, Arch (PKGBUILD).
- Migration automatique depuis `/etc/tcc`.

Points à trancher par les agents :

- Faut-il conserver strictement l'API D-Bus JSON historique, ou seulement une couche de compatibilité ?
- Qt Quick vs Qt Widgets pour les écrans denses (éditeur de profil, tableaux) ?
- Gestion exacte du conflit PPD/TCC si l'utilisateur veut un profil TCC qui contredit PPD.
- Support des compositeurs Wayland supplémentaires (Cosmic, River, etc.).
- Politique de restauration du refresh rate à l'arrêt du daemon.
- Périmètre exact des services annexes (Aquaris, Tomte, WebFAI) dans la première version.
