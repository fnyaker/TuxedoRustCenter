# Matrice matérielle TUXEDO — APIs, chemins et intégration

Statut : implémentation jalon 2 (matrice matérielle). Propriétaire :
`tuxedo-hardware`. Branchement daemon/D-Bus : `linux-platform`.
Document de référence : `docs/linux/PLATFORM_DESIGN.md`, `docs/security/DECISIONS.md`.

Ce document décrit **ce qui est fourni** par `tcc-hw` et
`tcc-daemon/src/hardware/**`, les **payloads D-Bus proposés** et les points de
branchement restant à faire côté `linux-platform`. Il ne remplace pas
`docs/DBUS_API.md` : les méthodes ne sont pas encore déclarées, seules les
structures de données et les workers existent.

---

## 1. Vue d'ensemble

| Domaine | Module `tcc-hw` | Worker daemon | État |
|---|---|---|---|
| Charge (profils/priorités/seuils/type) | `charging` | `hardware::charging` | Fait, testé |
| Éclairage clavier/lightbar | `lighting` | `hardware::lighting` | Fait, testé |
| ODM `platform_profile` + fallback `tuxedo-io` | `platform_profile`, `tuxedo_io` | `hardware::odm` | Fait, testé ; sources `platformProfile` et `io` séparées |
| TDP PL1/PL2/PL4 | `tdp` | `hardware::odm` | Fait, testé ; `tdpAvailable` dépend du firmware Uniwill, pas de `platform_profile` |
| Identification modèle/quirks + DMI UI | `device`, `dmi` | `hardware::device` | Fait, testé |
| Capacités CPU (topologie/governors/EPP) | `cpu` | `hardware::cpu` | Fait, testé ; application dans `apply.rs` |
| Connecteur interne DRM | `display` | `hardware::display` | Fait, testé ; modes/refresh via `tccd-user` |
| Capabilities batterie/AC/GPU | `power_supply`, `sensors` | `hardware::capabilities` | Fait, testé ; sans collecte de capteurs |
| Fn-Lock | `fn_lock` | `hardware::fn_lock` | Fait, testé ; `fnLockStatus` appliqué |
| Capteurs CPU/GPU/RAPL/NVIDIA | `sensors` | `hardware::sensors` | Fait, testé |
| NVIDIA cTGP | `nvidia` | `hardware::nvidia` | Fait, testé |
| Webcam (kill switch + énumération + contrôles UVC lecture/écriture) | `webcam`, `v4l2` | `hardware::webcam` (lecture) + `tccd-user` `User1` (écriture) | Lecture testée sur caméra ; écriture backend + service testés, **écriture réelle non testée** |
| PRIME (état uniquement) | `prime` | `hardware::prime` (lecture) | Fait, testé ; **aucune bascule dans `tcc-hw`** (H-01) |
| YCbCr 4:2:0 debugfs | `ycbcr` | `hardware::ycbcr` | Fait, testé |
| Commandes externes allowlistées | `command` | — | Fait, testé |

Tests : `cargo test -p tcc-hw` (**265 tests unitaires + 19 tests
d'intégration**, dont `fake_v4l2`, `v4l2_unsafe_guard` et `real_v4l2`
opt-in), `cargo test -p tcc-daemon` (**217 bins**, 1 ignoré),
`cargo test -p tcc-daemon --test hardware_workers` (**107 tests**, 1 ignoré)
et `cargo test -p tcc-daemon --test no_admin_reachability` (**3 tests**).
Aucun test par défaut n'ouvre de caméra réelle : `real_v4l2` et le test
worker équivalent sont `#[ignore]`. Aucun test CI ne dépend d'un laptop
TUXEDO.

---

## 2. Points de branchement pour `linux-platform`

### 2.1 Démarrage

Ajouter au binaire daemon (une seule ligne de module, puis l'appel) :

```rust
mod hardware;

let hw_inputs = std::sync::Arc::new(tokio::sync::RwLock::new(
    hardware::HardwareInputs::new(
        state_settings.clone(),   // tcc_core::Settings
        state_profile.clone(),    // Option<tcc_core::Profile>
        ppd_active,
        revision,
    )
    .with_ppd_override(user_wants_tcc_platform_profile),
));
let hw_status = std::sync::Arc::new(hardware::HardwareStatus::default());
let sensors_enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
let hardware = hardware::HardwareWorkers::start(
    hw_inputs.clone(),
    hw_status.clone(),
    root.clone(),
    dry_run,
    sensors_enabled.clone(),
).await;

// À chaque changement de profil, de réglages, SIGHUP ou reprise après veille :
*hw_inputs.write().await = hardware::HardwareInputs::new(settings, profile, ppd_active, revision)
    .with_ppd_override(user_wants_tcc_platform_profile);
hardware.notify();

// À l'arrêt (SIGTERM/SIGINT) :
hardware.shutdown().await;
```

`crates/tcc-daemon/tests/hardware_workers.rs` inclut le module via
`#[path = "../src/hardware/mod.rs"]` : la matrice compile et ses tests tournent
en CI **avant** le branchement. Il suffit ensuite d'ajouter `mod hardware;` à
`main.rs` (fichier possédé par `linux-platform`).

`revision` doit être incrémentée à **chaque** modification effective de
`Settings` ou du profil actif. Les workers appliquent leurs réglages quand la
révision change, et ne réécrivent rien sur les ticks périodiques (évite
d'écraser un réglage fait par un autre outil). Un `notify()` sans changement de
révision ne provoque qu'un rafraîchissement de télémétrie.

### 2.2 Politique PPD (D4)

`HardwareInputs.ppd_active` indique la présence de
`net.hadess.PowerProfiles`. `HardwareInputs.ppd_override` est l'override
**explicite** « TCC prioritaire » (faux par défaut). Le worker ODM n'écrit
`platform_profile` que si :

- PPD est absent ; ou
- `ppd_override == true` (à exposer dans les réglages, loguer à chaque
  application).

Sans override, `OdmData.platform_profile.owner == "ppd"` et
`applied == null` : l'UI doit afficher « géré par power-profiles-daemon ».

### 2.3 Collecte de capteurs à la demande

`SetSensorDataCollectionStatus(true|false)` doit faire :

```rust
sensors_enabled.store(status, std::sync::atomic::Ordering::Relaxed);
```

Le worker ne lit **aucun** fichier tant que le drapeau est faux ; il continue
de publier `sensors.enabled == false`. Aucun polling capteur n'est actif par
défaut (budget `docs/QUALITY.md` §4).

### 2.4 Accès aux instantanés

`HardwareStatus` expose un accesseur par domaine et un instantané complet :

```rust
hw_status.device()?;           // -> hardware::device::DeviceData (DMI)
hw_status.cpu_capabilities()?; // -> hardware::cpu::CpuCapabilitiesData
hw_status.display()?;          // -> hardware::display::DisplayData (DRM)
hw_status.capabilities()?;     // -> hardware::capabilities::CapabilitiesData
hw_status.charging()?;   // -> hardware::charging::ChargingData
hw_status.lighting()?;
hw_status.odm()?;
hw_status.sensors()?;
hw_status.nvidia()?;
hw_status.webcam()?;
hw_status.fn_lock()?;
hw_status.prime()?;
hw_status.ycbcr()?;
hw_status.snapshot();    // -> HardwareSnapshot (serde, camelCase)
```

Les lectures sont non bloquantes (verrou `RwLock` standard, clone court) et
tolèrent un verrou empoisonné.

### 2.5 Réglages (setters)

`linux-platform` peut soit appeler directement `tcc-hw` (comme `apply.rs` le
fait pour le CPU/backlight), soit passer par les helpers ci-dessous. L'ordre
recommandé pour un setter D-Bus :

1. valider l'entrée D-Bus (taille, type, allowlist) ;
2. appeler `tcc-hw` (validation matérielle + écriture) ;
3. persister `Settings` (couche config de `linux-platform`) ;
4. mettre à jour `HardwareInputs` et `hardware.notify()`.

Les workers étant idempotents, une réapplication au tick suivant est sans
effet si la valeur est déjà correcte.

| Action | API `tcc-hw` |
|---|---|
| Profil de charge | `charging::ChargingController::set_profile` |
| Priorité de charge | `charging::ChargingController::set_priority` |
| Seuils (cohérents) | `charging::ChargingController::set_thresholds` |
| Seuil unique | `set_start_threshold` / `set_end_threshold` |
| Type de charge | `charging::ChargingController::set_charge_type` |
| Clavier | `lighting::LightingController::apply_keyboard` |
| Lightbar | `lighting::LightingController::apply_lightbar` |
| Profil ODM | `platform_profile::PlatformProfile::apply(.., ownership)` |
| TDP | `tdp::TdpControl::set_values` / `apply_defaults` |
| Fn-Lock | `fn_lock::FnLock::set` / `status` |
| cTGP | `nvidia::CtgpControl::set_offset_watts` |
| YCbCr | `ycbcr::apply(root, card, port, enabled)` |
| Webcam | `webcam::WebcamSwitch::set` |
| Bascule PRIME | **aucune API dans `tcc-hw`** : implémentation directe dans `tccd-admin`, avec sa propre allowlist `prime-select` et polkit (D1, finding H-01) |

Pour l'éclairage, `linux-platform` doit conserver `LightingController` (ou le
recréer) pour appliquer un changement immédiat depuis `SetKeyboardStatesJSON` ;
le worker ne s'en charge qu'à la révision suivante. `LightingController::detect`
est bon marché et peut être appelé par le setter.

**Impératif (finding H-06 de la revue sécurité)** : tout setter d'éclairage
doit appeler `hardware::lighting::ensure_control_enabled(&settings)` **avant**
d'écrire. Le worker respecte déjà l'interrupteur
`keyboard_backlight_control_enabled` ; le setter actuel de `linux-platform`
(`dbus.rs`) le contourne encore. Tant qu'il n'est pas corrigé, l'écriture LED
est possible alors que l'utilisateur a désactivé le contrôle : comportement à
documenter comme *volatil et non gated* côté `docs/DBUS_API.md`, ou à corriger
côté `linux-platform`.

`Settings.chargingType` et `Settings.fnLockStatus` ne créent pas de nouveau
setter D-Bus : ils sont persistés par `linux-platform`, puis appliqués par les
workers au changement de révision. Le type de charge est validé contre
`ChargeType` (casse normalisée) avant écriture ; le Fn-Lock n'est appliqué que
si l'attribut est présent. Un échec ne bloque pas les autres domaines.

### 2.6 Arrêt et veille

- `HardwareWorkers::shutdown()` arrête les tâches (timeout 3 s). Aucun domaine
  n'a d'état volatil à rendre au firmware ; le retour en auto des ventilateurs
  reste la responsabilité de `FanWorker`.
- Après reprise de veille, `linux-platform` doit reconstruire `HardwareInputs`
  (même révision ou révision+1) et appeler `notify()`. Les compteurs RAPL sont
  réinitialisés par le worker de capteurs quand la collecte est coupée ; un
  premier échantillon après reprise ne fournit pas de puissance.
- Les interfaces `/dev/tuxedo_io` (ODM/TDP/webcam) sont rouvertes
  périodiquement si le module n'était pas chargé au démarrage.

---

## 3. Payloads D-Bus proposés

Les noms reprennent ceux de l'original (`TccDBusInterface`) pour la couche de
compatibilité. Les structures sont sérialisables en JSON `camelCase` ; une
interface typée `Manager1` peut les exposer en `a{sv}`. `docs/DBUS_API.md` reste
la source de vérité du contrat effectivement exposé.

> **Modèle de sécurité réel (jalon 2).** Les setters `Manager1` de charge,
> d'éclairage et de Fn-Lock sont **rate-limités et audités** par
> `linux-platform`, mais **ne sont pas protégés par polkit** : aucun
> `CheckAuthorization` n'existe dans `tccd`. Ils **ne persistent pas** (un
> `SIGHUP` recharge `Settings` et écrase la valeur). TDP, ODM, cTGP, PRIME et
> YCbCr ne sont **pas exposés en écriture**. Ne pas annoncer polkit ni de
> persistance dans l'UI tant que `tccd-admin` n'a pas repris ces opérations
> (D1/D2, finding H-11).

### 3.0 Device, CPU, capabilities et connecteur interne

Ces sections en lecture seule sont ajoutées à `HardwareSnapshot`. Elles ne
créent aucune méthode D-Bus : `linux-platform` les expose via
`GetHardwareSnapshotJSON`.

```json
{
  "device": {
    "available": true,
    "vendor": "TUXEDO Computers GmbH",
    "productName": "TUXEDO Book X",
    "boardName": "Board",
    "boardVendor": "TUXEDO Computers",
    "chassisVendor": "TUXEDO",
    "sku": "IBM15A10",
    "isTuxedo": true,
    "model": "IBM15A10",
    "kernelRelease": "6.12.0-tuxedo",
    "architecture": "x86_64",
    "tuxedoDriverVersion": "0.3.9",
    "errors": []
  },
  "cpu": {
    "totalCores": 16,
    "onlineCores": 16,
    "availableGovernors": ["performance", "powersave"],
    "availableEpp": ["default", "performance", "balance_performance",
                     "balance_power", "power"],
    "hasCpuControl": true,
    "writable": true,
    "scalingDriver": "amd-pstate-epp",
    "errors": []
  },
  "display": {
    "available": true,
    "hasInternalDisplay": true,
    "internalConnector": "eDP-1",
    "connected": true,
    "source": "drm",
    "errors": []
  },
  "capabilities": {
    "hasBattery": true,
    "hasAc": true,
    "hasIgpu": true,
    "hasDedicatedGpu": true,
    "errors": []
  }
}
```

- `device` vient de `/sys/class/dmi/id` (`tcc-hw::dmi`) : aucun numéro de
  série, aucune donnée sensible. `model` est le nom stable
  `TuxedoDevice::as_str()` déduit du SKU ; `null` si le SKU est inconnu (même
  si `isTuxedo` est vrai via un vendor). `isTuxedo` combine vendors DMI et SKU.
- `kernelRelease` est lu depuis `/proc/sys/kernel/osrelease` (chemin fixe,
  borné à 128 caractères, caractères de contrôle retirés). `architecture` est
  la constante de compilation `std::env::consts::ARCH`. `tuxedoDriverVersion`
  vient de `/sys/module/tuxedo_io/version` s'il existe ; aucune commande n'est
  exécutée. Champs absents ⇒ `null`/chaîne vide, jamais une erreur fatale.
- `cpu` provient de `tcc-hw::cpu`. `totalCores` = CPU présents,
  `onlineCores` = liste `online` du noyau. `availableGovernors`/`availableEpp`
  sont les unions réelles des politiques cpufreq, sans doublon.
  `hasCpuControl` = au moins une politique cpufreq active. `writable` =
  `hasCpuControl && !dry_run` ; ce n'est pas une mesure POSIX.
- `display` ne décrit que le connecteur DRM (`eDP-*`, `LVDS-*`, `DSI-*`).
  `hasInternalDisplay` est vrai dès qu'un connecteur interne est exposé, même
  déconnecté. **Les modes, le backend de session, le taux courant et
  `availableRefreshModes` restent fournis par `tccd-user`.** `tccd` ne doit
  jamais inventer de taux ni de backend ; si l'agent est absent, la GUI affiche
  « non supporté ».
- `capabilities` est dérivé sans collecte de capteurs :
  `hasBattery` = au moins une batterie `present` (`tcc-hw::power_supply`) ;
  `hasAc` = au moins un adaptateur `Mains` exposé ; `hasIgpu`/`hasDedicatedGpu`
  = rôles lus dans `/sys/class/drm` par `tcc-hw::sensors::gpu_presence`.
  Aucun `nvidia-smi`, aucun bail capteur, aucune commande externe.
- Les sections portent `#[serde(default)]` : un ancien payload sans ces clés se
  désérialise avec des valeurs neutres (`available=false`, listes vides,
  capabilities fausses).

### 3.1 Charge

| Méthode | Source | Remarque |
|---|---|---|
| `GetChargingProfilesAvailable() -> as` | `ChargingData.profiles` | |
| `GetCurrentChargingProfile() -> s` | `.profile` | `""` si absent |
| `SetChargingProfile(s) -> b` | `set_profile` | setter rate-limité/audité, **non polkit**, non persistant |
| `GetChargingPrioritiesAvailable() -> as` | `.priorities` | |
| `GetCurrentChargingPriority() -> s` | `.priority` | |
| `SetChargingPriority(s) -> b` | `set_priority` | |
| `GetChargeStartAvailableThresholds() -> as` | `.startThresholds` | l'original renvoie du JSON `s` |
| `GetChargeEndAvailableThresholds() -> as` | `.endThresholds` | |
| `GetChargeStartThreshold() -> i` | `.startThreshold` | `-1` si absent |
| `SetChargeStartThreshold(i) -> b` | `set_start_threshold` | |
| `GetChargeEndThreshold() -> i` | `.endThreshold` | |
| `SetChargeEndThreshold(i) -> b` | `set_end_threshold` | |
| `GetChargeType() -> s` | `.chargeType` | |
| `SetChargeType(s) -> b` | `set_charge_type` | setter direct non persistant ; `Settings.chargingType` réapplique la valeur persistée à la révision |

`ChargingData` (JSON) :

```json
{
  "available": true,
  "profiles": ["high_capacity", "balanced", "stationary"],
  "profile": "balanced",
  "priorities": ["charge_battery", "performance"],
  "priority": "performance",
  "battery": "BAT0",
  "startThreshold": 60,
  "endThreshold": 90,
  "startThresholds": [40, 50, 60],
  "endThresholds": [80, 85, 90, 100],
  "chargeType": "Fast",
  "chargeTypeAvailable": true,
  "chargeTypeWritable": true,
  "chargeTypes": [],
  "thresholdsAvailable": true,
  "writeSupported": true,
  "profileWritable": true,
  "priorityWritable": true,
  "thresholdsWritable": true,
  "applied": true,
  "errors": []
}
```

`writeSupported` et les drapeaux `*Writable` valent `false` en `dry_run`.
`chargeTypeWritable` est vrai si l'attribut `charge_type` existe et hors
`dry_run` : `Settings.chargingType` est appliqué au changement de révision.

- **Validation** : `tcc-core` stocke `chargingType` comme une chaîne ouverte,
  mais l'application matérielle n'accepte que les valeurs standard de
  `ChargeType` (casse normalisée : `Trickle`, `Fast`, `Standard`, `Adaptive`,
  `Custom`, `LongLife`, `Bypass`). `Unknown`, `N/A` et les valeurs hors ABI ne
  sont **jamais** écrites : warn unique, `applied=false` et erreur dans le
  payload. Une valeur constructeur non standard est donc refusée sans
  écriture.
- **Backend absent** : warn unique, aucune écriture, `chargeTypeAvailable` et
  `chargeTypeWritable` restent faux.
- **Seuils** : le pilote exige `Custom` pour honorer
  `charge_control_*`. Le type demandé est appliqué **après** les seuils ; si
  l'utilisateur choisit un type non `Custom` en même temps que des seuils, le
  type final est celui demandé et les seuils peuvent être ignorés par le
  firmware. Aux révisions suivantes, tant que `chargeType` n'est plus `Custom`,
  les seuils ne sont plus réappliqués : c'est la règle héritée qui évite de
  réactiver un mode désactivé. C'est documenté et visible dans `chargeType`
  publié.
- `chargeTypes` reste vide : l'ABI noyau expose le type courant, pas une liste
  des types réellement acceptés. L'UI ne doit pas inventer de choix ; une
  valeur standard peut encore être refusée par le firmware, auquel cas
  l'écriture échoue et `applied=false`.

### 3.2 Éclairage

| Méthode | Source |
|---|---|
| `GetKeyboardCapabilitiesJSON() -> s` | `LightingData.keyboard` |
| `GetLightbarCapabilitiesJSON() -> s` | `LightingData.lightbar` |
| `GetKeyboardStatesJSON() -> s` | `LightingData.states` |
| `GetLightbarStatesJSON() -> s` | `LightingData.lightbarState` |
| `SetKeyboardStatesJSON(s) -> b` | valider + `ensure_control_enabled` + `apply_keyboard` ; **non persistant** (écrasé par SIGHUP) |
| `SetLightbarStatesJSON(s) -> b` | valider + `ensure_control_enabled` + `apply_lightbar` ; **non persistant** |

`kind` vaut `white`, `rgb`, `per-key` ou `none`. `maxRed/Green/Blue` sont
absents (`null`) pour un clavier blanc. Chaque bloc `keyboard`/`lightbar`
expose `writable` (matériel présent, contrôle global activé, hors `dry_run`)
et `modes` (réellement appliqués : `["static"]` uniquement ; le mode
« respiration » de l'original n'a pas d'équivalent dans l'ABI LED).

### 3.3 ODM / TDP

| Méthode | Source | Remarque |
|---|---|---|
| `ODMProfilesAvailable() -> as` | `OdmData.platformProfile.choices` ou `OdmData.io.choices` | lecture seule ; le fallback `tuxedo-io` est une source **distincte** |
| `ODMTDPInfoJSON() -> s` | `OdmData.tdp` | **lecture seule** (proposition ; pas d'écriture TDP exposée) |
| `GetODMDataJSON() -> s` | `OdmData` | proposition d'agrégat, lecture seule |

`OdmData` :

```json
{
  "available": true,
  "platformProfile": {
    "available": false,
    "source": null,
    "current": null,
    "choices": [],
    "defaultProfile": null,
    "acpiQuirkBlocked": false,
    "owner": "ppd",
    "writable": false,
    "applied": null
  },
  "io": {
    "available": true,
    "interface": "uniwill",
    "moduleVersion": "0.3.9",
    "odmAvailable": true,
    "choices": ["power_save", "enthusiast", "overboost"],
    "defaultProfile": "overboost",
    "tdpAvailable": false
  },
  "odmWritable": true,
  "tdpAvailable": false,
  "tdpWritable": false,
  "tdp": [],
  "interface": "uniwill",
  "moduleVersion": "0.3.9",
  "applied": true,
  "errors": []
}
```

Exemple réel : InfinityBook Pro AMD Gen9 (`IBP14A09MK1`), sans
`platform_profile`, avec `tuxedo_io` Uniwill 0.3.9. `available` est vrai grâce
à `io.odmAvailable` ; `tdpAvailable` reste faux car le firmware n'expose pas
`R_UW_TDP*_MIN` (confirmé par `Admin.Hw.GetTdpValues` ⇒ `NotSupported("tdp")`).

Sur un modèle Uniwill qui expose le TDP, `io.tdpAvailable` et `tdpAvailable`
passent à vrai et `tdp` contient 1 à 3 entrées :

```json
{
  "io": {
    "available": true,
    "interface": "uniwill",
    "moduleVersion": "0.3.9",
    "odmAvailable": true,
    "choices": ["power_save", "enthusiast", "overboost"],
    "defaultProfile": "overboost",
    "tdpAvailable": true
  },
  "tdpAvailable": true,
  "tdpWritable": true,
  "tdp": [
    { "descriptor": "pl1", "current": 35, "min": 10, "max": 80 },
    { "descriptor": "pl2", "current": 45, "min": 10, "max": 100 },
    { "descriptor": "pl4", "current": 60, "min": 20, "max": 120 }
  ]
}
```

Règles de capacité :

- `available` = `platformProfile.available || io.odmAvailable ||
  io.tdpAvailable`. Une machine sans `platform_profile` mais avec des profils
  ODM `tuxedo-io` est donc disponible.
- `io.available` = `/dev/tuxedo_io` ouvert et interface reconnue ; il ne
  signifie pas que ODM/TDP sont exposés (`io.odmAvailable`,
  `io.tdpAvailable`).
- `io.choices`/`io.defaultProfile` viennent de `R_UW_PROFS_AVAILABLE` et de la
  logique `GetDefaultODMPerformanceProfile` de l'original. `defaultProfile`
  est **informatif** : il n'est jamais appliqué implicitement.
- Si un profil demande `Some(name)` absent de `io.choices`, le worker journalise
  un warn, ne fait **aucune écriture** et renvoie une erreur (`applied=false`).
  Si le profil n'a pas d'ODM (`None`), le worker ne touche pas au firmware.
- `tdpAvailable` ne dépend **pas** de `platform_profile` : il reflète
  `R_UW_TDP*_MIN` (1 à 3 limites `pl1`/`pl2`/`pl4`, watts entiers).
- `odmWritable`/`tdpWritable` valent faux en `dry_run` et true seulement si la
  source correspondante est réellement pilotable. Le fallback `io.choices`
  n'est considéré comme pilotable que si `platformProfile.available=false`
  (même logique que `ODMProfileWorker` : on ne mélange pas les deux sources).
  Aucune écriture n'est faite pendant la détection.
- **TDP de profil** : `tdpValues=[]` (ou absence de profil TDP) signifie
  « ne pas changer » ; le worker n'appelle pas `set_values`. Le primitif
  `tcc-hw::tdp::TdpControl::set_values([])` applique les maxima firmware
  (test `empty_values_apply_maxima`) pour une action explicite
  `Admin.Hw.SetTdpValues([])` ; la validation d'arité de `Admin.Hw` doit
  autoriser ce cas (hors périmètre de ce module).
- `interface` et `moduleVersion` restent au niveau racine pour compatibilité ;
  `io.*` est la source à utiliser par la nouvelle GUI.

### 3.4 Fn-Lock, NVIDIA, webcam, PRIME, YCbCr

Champs de capacité ajoutés (tous en lecture seule) :

```json
{
  "fnLock": { "supported": true, "enabled": true, "writable": true },
  "nvidia": {
    "available": true,
    "writable": true,
    "attributeAvailable": true,
    "nvidiaSmiAvailable": true,
    "ctgpOffset": 15,
    "defaultPowerLimit": 80.0,
    "maxPowerLimit": 230.0,
    "maxCtgpOffset": 150,
    "maxCtgpOffsetSource": "dynamic"
  },
  "prime": {
    "available": true,
    "writable": false,
    "supported": true,
    "mode": "on-demand",
    "internalDisplayOnNvidia": false
  },
  "webcam": {
    "switchAvailable": true,
    "enabled": true,
    "controlsSupported": true,
    "controlsWritable": false,
    "controlsTruncated": false,
    "devices": []
  }
}
```

- `fnLock.writable` décrit le **matériel** ; l'écriture D-Bus reste soumise à
  sa politique d'autorisation séparée. `writable=false` si l'attribut
  `fn_lock` est absent ou si le daemon est en `dry_run`.
  `Settings.fnLockStatus` (`null` = ne pas toucher) est appliqué au changement
  de révision, une seule tentative par révision : matériel absent ⇒ warn
  unique et no-op, puis `enabled` est relu après l'écriture. Le firmware peut
  ne pas conserver l'état après reprise ; la valeur publiée reste l'état réel
  relu, pas la valeur demandée.
- `nvidia.writable` exige `ctgp_offset` **et** `nvidia-smi` et vaut `false` en
  `dry_run`. `attributeAvailable` seul ne suffit pas.
- `nvidia.maxCtgpOffset` est la borne réelle :
  `floor(maxPowerLimit - defaultPowerLimit).clamp(0, 200)` quand les deux
  limites sont disponibles, sinon `200` avec
  `maxCtgpOffsetSource="static"`. `dynamic` signifie « mesuré via
  `nvidia-smi` » ; `static` est la borne de sécurité historique, **pas** une
  borne matérielle. Tout offset hors `0..=maxCtgpOffset` est refusé avant
  écriture (`applied=false`, warn dédupliqué). `Admin.Hw` utilise la même
  règle.
- `prime.writable` est **toujours `false`** dans `tccd` : la bascule appartient
  à `tccd-admin` (D1). `supported` reste vrai si le matériel/outil le permet ;
  la GUI doit distinguer les deux.
- `webcam.controlsWritable` est **toujours `false` dans `tccd`** : le daemon
  reste en lecture. L'écriture passe exclusivement par `tccd-user`
  (`User1.SetUvcControl`) ; `controlsSupported` n'implique aucune capacité
  d'écriture dans le snapshot système.


| Méthode | Source |
|---|---|
| `GetFnLockSupported() -> b` | `fn_lock.supported` |
| `GetFnLockStatus() -> b` | `fn_lock.enabled` |
| `SetFnLockStatus(b)` | `FnLock::set` |
| `GetNVIDIAPowerCTRLAvailable() -> b` | `nvidia.available` |
| `GetNVIDIAPowerCTRLDefaultPowerLimit() -> d` | `nvidia.defaultPowerLimit` |
| `GetNVIDIAPowerCTRLMaxPowerLimit() -> d` | `nvidia.maxPowerLimit` |
| `GetNvidiaDataJSON() -> s` | `NvidiaData` (agrégat proposé) |
| `WebcamSWAvailable() -> b` | `webcam.switchAvailable` |
| `GetWebcamSWStatus() -> b` | `webcam.enabled` |
| `GetWebcamDataJSON() -> s` | `WebcamData` (nœuds V4L2, contrôles en lecture) |
| `GetPrimeState() -> s` | `prime.mode` (`iGPU`/`dGPU`/`on-demand`/`off`) |
| `GetPrimeDataJSON() -> s` | `PrimeData` |
| `GetForceYUV420OutputSwitchAvailable() -> b` | `ycbcr.available` |
| `GetYcbcrDataJSON() -> s` | `YcbcrData` |
| `SetSensorDataCollectionStatus(b)` | drapeau `AtomicBool` |
| `GetIGpuInfoValuesJSON()` / `GetDGpuInfoValuesJSON()` | `sensors.igpu` / `.dgpu` |
| `GetIGpuAvailable()` / `GetDGpuAvailable()` | booléens dérivés |
| `GetCpuPowerValuesJSON()` | `sensors.cpu` |

Signal additionnel proposé : `ModeReapplyPendingChanged(b)` émis quand
`YcbcrData.pendingModeReapply` passe à vrai (le champ décrit une modification
**du tick courant**, pas un état persistant).

### 3.5 Notes pour `linux-platform` et `qt-ui`

`linux-platform` :

- `HardwareSnapshot` est déjà complet : ne pas recalculer `device`, `cpu` ou
  `display` dans `dbus.rs`/`main.rs`, et ne pas ajouter de lecture sysfs dans
  les handlers D-Bus. Les nouveaux champs sont publiés par les workers
  `device`, `cpu` et `display` au même rythme que les autres domaines.
- Les sections portent `#[serde(default)]` : la compatibilité avec un ancien
  payload est assurée, mais un client qui désérialise dans une structure propre
  doit lui aussi tolérer les champs inconnus.
- `odm.available` inclut désormais `platformProfile.available`,
  `io.odmAvailable` et `io.tdpAvailable` : ne pas recalculer cette condition
  dans `dbus.rs`. Pour une action ODM, préférer `io.choices` quand
  `platformProfile.available=false`.
- `GetDisplayModeInfo`/les modes de rafraîchissement restent l'affaire de
  `tccd-user`. `display.internalConnector` ne remplace pas le backend de
  session et ne doit pas être utilisé pour appliquer un mode.
- `dry_run` met `cpu.writable`, `charging.*Writable`, `lighting.*.writable`,
  `odm.tdpWritable`, `nvidia.writable` et `fnLock.writable` à `false`. Aucun
  setter ne doit être exposé comme actif dans cet état.
- `Settings.chargingType` et `Settings.fnLockStatus` sont appliqués par les
  workers au changement de révision. `linux-platform` doit persister la valeur
  puis incrémenter la révision ; il ne doit pas écrire `charge_type` ou
  `fn_lock` directement depuis un handler D-Bus (sauf setter Fn-Lock existant,
  qui reste protégé par sa politique).

`qt-ui` :

- Afficher le modèle avec `device.productName` (repli `device.model`, puis
  « inconnu ») et le badge TUXEDO avec `device.isTuxedo`. Ne pas déduire le
  modèle du vendor seul.
- Page Info : `device.kernelRelease`, `device.architecture` et
  `device.tuxedoDriverVersion` remplacent les valeurs de `DemoData` (F6).
  `null` signifie « non exposé », pas une erreur.
- Dashboard/visibilité : `capabilities.hasBattery`/`hasAc` conditionnent les
  widgets d'alimentation ; `display.hasInternalDisplay` conditionne le bloc
  refresh ; `capabilities.hasIgpu`/`hasDedicatedGpu` conditionnent les cartes
  GPU (sans attendre la collecte de capteurs). Une capability fausse masque la
  carte, elle ne doit pas afficher une jauge vide.
- ODM : ne pas masquer la carte sur `platformProfile.available=false`. Si
  `io.odmAvailable=true`, afficher `io.choices` et `io.defaultProfile` (source
  `tuxedo-io`) ; c'est le cas de l'InfinityBook Pro AMD Gen9. Les curseurs TDP
  ne doivent apparaître que si `io.tdpAvailable=true` (sinon le firmware ne les
  expose pas, même si le profil ODM est disponible).
- Utiliser `cpu.totalCores`, `cpu.onlineCores`, `cpu.availableGovernors` et
  `cpu.availableEpp` à la place de `DemoData`. Si `hasCpuControl=false`,
  masquer les contrôles CPU dépendants plutôt que d'afficher des zéros.
- Utiliser `writable` pour distinguer « matériel absent » (`available=false`),
  « supporté mais non autorisé » (`writable=false`, ex. PRIME/PPD) et
  « action possible ». Un `writable=false` ne doit pas être rendu comme un
  toggle actif.
- `charging.chargeTypeWritable` et `fnLock.writable` peuvent être vrais : ce
  sont des choix utilisateur persistés dans `Settings`, pas des setters
  matériels directs. `chargeTypes` vide signifie « pas de liste inventée » ;
  proposer uniquement les valeurs standard documentées, et masquer/grisser si
  l'écriture échoue.
- `webcam.controlsWritable=false` : le snapshot `tccd` reste informationnel.
  Pour l'écriture, interroger `User1.GetUvcWriteSupported` sur le bus de
  session puis appeler `User1.SetUvcControl` (voir `DBUS_API.md`).
- Pour l'affichage, `display.internalConnector`/`connected` servent uniquement
  à la visibilité ; le taux, le backend et les modes disponibles viennent de
  l'agent de session. Agent absent ⇒ état « non supporté », jamais de valeur
  inventée.

---

## 4. Chemins et contrôles

### 4.1 Charge — `tcc-hw/src/charging.rs`

| Attribut | Accès | Source original |
|---|---|---|
| `devices/platform/tuxedo_keyboard/charging_profile/charging_profile` | R/W | `ChargingWorker` |
| `.../charging_profile/charging_profiles_available` | R | idem |
| `.../charging_priority/charging_prio` | R/W | idem |
| `.../charging_priority/charging_prios_available` | R | idem |
| `class/power_supply/<bat>/charge_control_start_threshold` | R/W | `PowerSupplyController` |
| `.../charge_control_end_threshold` | R/W | idem |
| `.../charge_control_start_available_thresholds` | R | non standard, repli `0..=100` |
| `.../charge_control_end_available_thresholds` | R | idem |
| `.../charge_type` | R/W | `ChargeType` (ABI `sysfs-class-power`) |

Décisions :

- validation **avant** toute écriture : liste `*_available_*` si présente,
  `0..=100`, `start < end`, type `Custom` écrit avant les seuils ;
- une valeur identique n'est pas réécrite ;
- `charge_type` n'accepte que les valeurs de l'ABI noyau (`Unknown`, `N/A`,
  `Trickle`, `Fast`, `Standard`, `Adaptive`, `Custom`, `LongLife`, `Bypass`) ;
- les seuils persistés ne sont réappliqués au démarrage que si
  `charge_type == Custom` (restaure l'intention sans réactiver un mode
  désactivé).

### 4.2 Éclairage — `tcc-hw/src/lighting.rs`

Chemins : voir section 2.5 de `PLATFORM_DESIGN.md`. Détection :

1. `white:kbd_backlight` (TUXEDO puis NB05) ;
2. sinon pilotes HID ITE (`tuxedo-keyboard-ite`, `ite_829x`, `ite_8291`,
   `tuxedo_nb04_kbd_backlight`) puis chemins statiques
   `rgb:kbd_backlight[_1|_2]` ;
3. `zones > 4` ⇒ per-key ; sinon RGB.

Écriture : `brightness` sur la première LED, `buffer_input=1`,
`multi_intensity` par zone, `buffer_input=0` **garanti** même en cas d'erreur
(correction d'un bug de l'original).

### 4.3 ODM/TDP — `tcc-hw/src/platform_profile.rs`, `tdp.rs`

| Source | Chemin / ioctl |
|---|---|
| TUXEDO platform | `bus/platform/devices/tuxedo_platform_profile/platform_profile[_choices]` |
| ACPI platform | `firmware/acpi/platform_profile[_choices]` |
| ODM Clevo | lecture `R_HWCHECK_CL`, profils statiques, écriture `W_CL_PERF_PROFILE` |
| ODM Uniwill | `R_HWCHECK_UW`, `R_UW_MODEL_ID`, `R_UW_PROFS_AVAILABLE`, `W_UW_PERF_PROF` |
| TDP Uniwill | `R_UW_TDP{0,1,2}`, `R_UW_TDP{0,1,2}_MIN`, `R_UW_TDP{0,1,2}_MAX`, `W_UW_TDP{0,1,2}` |

- **Ordre de préférence** : `tuxedo_platform_profile` > ACPI
  `platform_profile` (sauf quirk) > `tuxedo-io`. Le chemin `tuxedo-io` reste
  actif même quand `platform_profile` est absent ; les deux capacités sont
  exposées séparément (`platformProfile` et `io`).
- **Profils Uniwill** : `R_UW_PROFS_AVAILABLE` = 2 ⇒ `power_save`,
  `enthusiast` ; = 3 ⇒ ajoute `overboost`. Valeurs d'écriture
  `W_UW_PERF_PROF` : `0x01`, `0x02`, `0x03`. Un compte < 2 ⇒ aucun profil
  (`NotSupported`, jamais une liste inventée).
- **TDP Uniwill** : nombre de limites = dernier index dont
  `R_UW_TDPn_MIN >= 0` + 1 (max 3). Descripteurs `pl1`, `pl2`, `pl4`, valeurs
  en watts entiers. Aucune limite ⇒ `tdpAvailable=false` et `tdp=[]`.
- **Détection sans écriture** : `odm_profiles`, `default_odm_profile`,
  `tdp_count`, `tdp`, `tdp_min`, `tdp_max` sont des lectures ; aucun
  `set_*` n'est appelé pendant la détection.
- Quirks appliqués : IBP G10 AMD, IBM15A10, IBM16A10 ⇒ ACPI ignoré.
- TDP : validation `[min, max]`, entiers, séquence croissante ; liste vide ⇒
  maxima. `docs/hardware/HARDWARE_MATRIX.md` §7 documente le risque de
  non-transactionnalité.

### 4.4 Fn-Lock — `tcc-hw/src/fn_lock.rs`

`devices/platform/tuxedo_keyboard/fn_lock` (booléen). Absent ⇒
`supported: false`.

### 4.5 Capteurs — `tcc-hw/src/sensors.rs`

| Capteur | Chemin |
|---|---|
| Température CPU | `class/hwmon/*/temp1_input` (`coretemp`, `k10temp`, `zenpower`, `cpu_thermal`) |
| Fréquence CPU | moyenne `cpuN/cpufreq/scaling_cur_freq` (repli `cpuinfo_cur_freq`) |
| Puissance CPU | RAPL `intel-rapl:0` (`package-0`), delta `energy_uj` avec *wrap* |
| GPU AMD/Intel | `class/drm/cardN/device/{uevent,hwmon/*}` + `pp_dpm_sclk` / `gt_*_freq_mhz` |
| GPU NVIDIA | `nvidia-smi --query-gpu=...` (allowlist, 2 s max) |

Règles : aucune valeur inventée (`None`), premier échantillon RAPL sans
puissance, classification AMD via la liste `AmdDeviceIDs.ts`, Intel = intégré,
NVIDIA = discret.

Coût : `nvidia-smi` est mis en cache par instance
(`NVIDIA_MIN_QUERY_INTERVAL = 10 s`, configurable via
`NvidiaSmi::with_min_interval`), y compris pour les échecs. Même si la collecte
tourne toutes les 2 s, le daemon lance **au plus un processus `nvidia-smi`
root toutes les 10 s**, et seulement si une GPU NVIDIA est détectée.

### 4.6 NVIDIA cTGP — `tcc-hw/src/nvidia.rs`

`devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset` (R/W, watts).
Disponibilité complète = attribut **et** `nvidia-smi`, comme l'original.
Offset validé entier, `0..=200` (borne de sécurité alignée sur la validation
des profils), non réécrit si identique. Limites `nvidia-smi` relues toutes les
5 minutes par le worker, et de toute façon mises en cache 10 s par
`NvidiaSmi` (une exécution au plus toutes les 10 s).

### 4.7 PRIME — `tcc-hw/src/prime.rs`

`prime-select` + `dpkg-query` (détection TUXEDO) + marqueur
`/var/lib/ubuntu-drivers-common/requires_offloading` + routage eDP.
**Lecture seule dans `tccd`**. `set_mode`/`PRIME_SWITCH_TIMEOUT` ont été
**retirés** de `tcc-hw` (finding H-01) : la bascule sera implémentée
directement dans `tccd-admin`, avec sa propre allowlist et polkit. Aucune
dépendance partagée ne doit la réintroduire dans `tccd` ;
`crates/tcc-daemon/tests/no_admin_reachability.rs` échoue si `tccd` la
référence.

### 4.8 Webcam — `tcc-hw/src/webcam.rs`

Kill switch Clevo (`R_CL_WEBCAM_SW`/`W_CL_WEBCAM_SW`), énumération
`class/video4linux/videoN/{name,index,dev}`. Uniwill ⇒ non supporté.

Le hotplug est géré par ré-énumération toutes les 2 s (worker), sans watcher
udev : cela couvre le branchement/débranchement sans dépendre du netlink, au
prix d'un coût négligeable (lecture de 2 à 6 attributs). Si `linux-platform`
ajoute un watcher udev, `HardwareWorkers::notify()` suffit à déclencher une
ré-énumération immédiate.

Les **contrôles UVC** sont conçus dans `docs/hardware/V4L2_DESIGN.md`
(revue `docs/security/REVIEW_V4L2.md`). **Étape 1 livrée** : lecture seule
dans `tccd`, quatre ioctl (`QUERYCAP`, `QUERYCTRL`, `QUERYMENU`, `G_CTRL`),
allowlist fermée de 15 CIDs, budget et menus paresseux. `S_CTRL` est isolé
dans `v4l2/write.rs`/`real_write.rs` pour `tccd-user` ; le chemin de lecture
(`tccd`) ne le définit pas et ne l'appelle jamais.

- `WebcamData.controlsSupported` est vrai si au moins un contrôle a été lu.
- Chaque device expose `identity` (vendor/produit USB), `busInfo`, `controls`
  (clé, nom, type, bornes, valeur, menus, flags, catégorie) et
  `controlsError` ; l'`id` noyau n'est jamais exposé.
- La lecture est **à la demande** : elle suit le bail de collecte
  (`GetHardwareSnapshotJSON`/`SetSensorDataCollectionStatus`) et est mise en
  cache 30 s ; aucun ioctl de contrôle au tick de 2 s, aucun lecture sans
  client. `dry_run` n'ouvre aucun device.
- **Permissions du nœud** : la lecture accepte un nœud `root:video` avec
  écriture monde (`0666`/`0666`+ACL `uaccess`, `crwxrwxrwx+`), fréquent sur
  les webcams FHD. `O_RDONLY` ne donne aucun droit d'écriture ; le mode est
  journalisé en debug. Les contrôles stricts restent : type caractère, major 81, minor
  sysfs, uid root, `O_NOFOLLOW`. Le chemin d'écriture (`tccd-user`) ouvre en
  `O_RDWR` : l'ACL `uaccess`/le groupe `video` est appliqué par le noyau à
  l'ouverture, sans vérification dupliquée et sans repli root.
- **Packaging requis pour la lecture** : `tccd.service` doit ajouter
  `DeviceAllow=char-video4linux r`. Sans cela, le device est
  `available: false` et l'erreur est une permission refusée. Ce fichier n'est
  pas encore modifié (action `packaging-release`).
- **Validation réelle** (2026-11-06, webcam FHD `2b7e:c858`) :
  `UvcReaderController::system(...).read_all()` lit 11 contrôles sur
  `/dev/video0` ; le worker publie `controlsSupported: true`. Caméra IR
  (`video2`) : 0 contrôle de l'allowlist, pas d'erreur.
- **Écriture UVC (étape 2) — livrée** : `UvcWriterController::system`
  (`tcc-hw`) consommé par `tccd-user`, exposé par le service de session :

  ```text
  com.tuxedocomputers.tccd.User — User1
    GetUvcWriteSupported() -> b
    SetUvcControl(s devicePath, s controlKey, i value) -> b
  ```

  `true` = valeur changée, `false` = déjà courante (no-op). `devicePath` doit
  être `/dev/videoN` canonique et énuméré ; allowlist 15 CIDs, validation
  type/min/max/pas/menu, `QUERYCTRL`/`G_CTRL` avant `S_CTRL`, relecture,
  budget 64 ioctl, timeout 2 s, `try_lock`, quarantaine 30 s.
  **Une seule écriture est en vol par processus** (porte globale) : les appels
  concurrents renvoient `Busy` immédiatement ; un `S_CTRL` bloqué n'immobilise
  qu'un thread `spawn_blocking`. Watchdog service encore à ajouter.
  Après `QUERYCAP`, `driver`/`card`/`busInfo` sont comparés au device sysfs ;
  un mismatch (device remplacé) renvoie `Stalled` avant `S_CTRL`. Si la
  relecture après `S_CTRL` diffère, la valeur précédente est restaurée en
  best-effort et `InvalidValue` est retournée avec l'état constaté. Erreurs :
  `InvalidArgs`, `NotSupported`, `OutOfRange`, `Busy`, `Stalled`, `Failed`,
  `LimitsExceeded` (20 écritures/s, burst 20, par UID).
- **`tccd-user` n'a pas de `DeviceAllow`** : accès par ACL `uaccess`/groupe
  `video`, aucune action polkit. `tccd` conserve `r` uniquement ; aucun repli
  root ; `EACCES` = état « non supporté ».
- **Limites** : écriture réelle non testée sur caméra physique ; une autre
  application peut détenir la caméra (`Busy`) ; un firmware peut
  arrondir/ignorer une écriture (relecture ⇒ erreur) ; préréglages persistés
  non implémentés (étape 3) ; sliders GUI en cours. Le payload `WebcamData` de
  `tccd` reste en lecture (`controlsWritable=false`).

### 4.9 YCbCr — `tcc-hw/src/ycbcr.rs`

`kernel/debug/dri/<carte>/<port>/force_yuv420_output` ; `port` est le nom de
connecteur **sans** préfixe carte (`DP-1`). Validation stricte du chemin.
`pendingModeReapply` décrit une modification du tick.

---

## 5. Matrice par machine

| Machine | Charge TUXEDO | Seuils standard | Lighting | ODM | TDP | cTGP | Webcam switch | Fn-Lock |
|---|---|---|---|---|---|---|---|---|
| Non-TUXEDO | non | si noyau | non | ACPI si présent | non | non | non | non |
| Clevo (générique) | si module | si batterie | selon modèle | `tuxedo-io` ou ACPI | non | selon modèle | **oui** | si module |
| Uniwill (générique) | si module | si batterie | selon modèle | `tuxedo_platform_profile`/`tuxedo-io` | **oui** | selon modèle | non | si module |
| Sirius 16 Gen1/Gen2 | si module | si batterie | RGB | selon firmware | selon firmware | non | non | si module |
| IBP G10 AMD / IBM15A10 / IBM16A10 | si module | si batterie | selon modèle | `tuxedo-io` (ACPI ignoré) | selon firmware | non | selon modèle | si module |
| IBM15A10 | si module | si batterie | selon modèle | `tuxedo-io` | selon firmware | non | selon modèle | si module ; PRIME limité si eDP sur dGPU |

Un attribut absent n'est jamais une erreur : la ligne correspondante est
`available: false` et l'UI doit l'afficher ainsi.

**TDP Uniwill** : `oui` dans le tableau signifie « selon la table du module
noyau ». Le module `tuxedo_io` ne définit `tdp_min_defs` que pour une liste de
SKU/boards (`uw_id_tdp`, `tuxedo_io.c`) ; un board absent comme `GXxHRXx`
(InfinityBook Pro AMD Gen9, SKU `IBP14A09MK1 / IBP15A09MK1`) renvoie
`-ENODEV` sur `R_UW_TDP*_MIN` ⇒ `io.tdpAvailable=false`, `tdp=[]`. Les profils
ODM (`R_UW_PROFS_AVAILABLE`) restent disponibles sur ce modèle.

---

## 6. Revue sécurité demandée

Opérations sensibles introduites (à relire par `security`) :

1. **Écritures sysfs privilégiées** : charge (profil, priorité, seuils, type),
   LED (brightness/multi_intensity/buffer_input), `platform_profile`, TDP
   (ioctl), `ctgp_offset`, `fn_lock`, `force_yuv420_output` (debugfs).
   Toutes passent par `SysfsRoot` (`O_NOFOLLOW`, `fstat`, refus des liens et
   des fichiers non root en production) et sont validées avant écriture.
2. **Commandes externes** : `nvidia-smi`, `prime-select`, `dpkg-query`.
   Chemins absolus allowlistés, arguments séparés, `env_clear` +
   `PATH`/`LC_ALL`/`LANG` fixes, stdin fermé, timeout, **échéance de lecture**
   (100 ms après la fin du processus), sortie bornée à 64 Kio. Aucun shell,
   aucun héritage de `HOME`/`PWD`/`BASH_ENV`/`IFS`. Un descendant qui retient
   `stdout` ne bloque plus le worker : le thread lecteur est détaché et une
   erreur typée est retournée (limite : un thread détaché par invocation
   pathologique, sans impact sur la régulation).
3. **Bascule PRIME** : **aucune API d'écriture dans `tcc-hw`** depuis le
   retrait de `set_mode` (H-01). Elle sera implémentée dans `tccd-admin` +
   polkit (D1), avec sa propre allowlist. `tccd` ne fait que lire l'état ;
   `tests/no_admin_reachability.rs` verrouille l'absence d'appel et de
   `Command::new` direct dans `tccd`.
4. **`/var/lib/ubuntu-drivers-common/requires_offloading`** : lecture
   d'existence d'un chemin absolu constant, aucune donnée utilisateur.
5. **Debugfs** : montage potentiellement absent ; chemin construit et validé,
   aucune écriture hors de `kernel/debug/dri/<0..16>/<port>`.
6. **Charge** : une valeur hors liste noyau est refusée ; `charge_type` limité
   à l'ABI. Un profil importé malveillant ne peut pas écrire de chaîne
   arbitraire.
7. **Points ouverts** : contrôle UVC (nécessite `unsafe` ou une crate V4L2 —
   voir §7) ; `DeviceAllow` de `/dev/tuxedo_io` ; accès debugfs avec
   `ProtectKernelTunables=no`.
8. **Setters `Manager1`** : non polkit, non persistants, écritures globales
   accessibles à tout utilisateur local (rate-limitées et auditées). Le modèle
   cible est `tccd-admin` + polkit (D1/D2) ; en attendant, `docs/DBUS_API.md`
   doit le dire explicitement (H-11).
9. **Interrupteur d'éclairage** : le worker le respecte, mais les setters
   `dbus.rs` le contournent encore (H-06). `ensure_control_enabled` est fourni
   pour que `linux-platform` ferme l'écart.

---

## 7. Risques et validations restantes sur laptop réel

| Sujet | Risque | Validation attendue |
|---|---|---|
| Charge | Le firmware peut ignorer `charge_control_*` si le driver batterie ne supporte pas `Custom` | Clevo + Uniwill : vérifier lecture/écriture réelle et absence d'erreur `EINVAL` |
| Seuils | Certains EC arrondissent ou refusent une valeur pourtant listée | Lire la valeur après écriture sur 2 modèles |
| Lighting RGB | L'ordre des zones et `buffer_input` varient selon le contrôleur ITE | Vérifier couleurs par zone et clignotement sur clavier 3 zones et per-key |
| Luminosité clavier | L'écriture directe `<led>/brightness` doit produire le même effet que `UPower.SetBrightness` | Comparer les deux chemins sur un modèle blanc et un RGB |
| ODM | Sur IBP G10, l'ACPI est ignoré ; vérifier que `tuxedo-io` applique bien le profil | Contrôler la LED/le bruit du ventilateur après changement |
| TDP | Séquence non croissante refusée ; vérifier qu'aucun profil officiel n'utilise PL1 > PL4 | Tester les presets TCC sur Uniwill |
| cTGP | L'offset n'est appliqué que si `nvidia-smi` est présent | Vérifier sur dGPU NVIDIA + driver propriétaire |
| PRIME | `dpkg-query` absent hors Debian ⇒ PRIME `unsupported` (parité) | Vérifier TUXEDO OS ; documenter Fedora/Arch |
| Webcam | Uniwill n'a pas de kill switch ; les nœuds V4L2 varient | Vérifier `WebcamSWAvailable` sur Clevo |
| `nvidia-smi` | Coût d'un processus root ; cache 10 s mais premier appel à chaque démarrage | Mesurer la fréquence réelle (`pidstat -C nvidia-smi`) et vérifier qu'elle reste ≤ 1/10 s |
| PRIME | Bascule absente de `tccd` ; l'admin n'existe pas encore | Vérifier que `tccd` n'expose aucune méthode PRIME d'écriture et que `tccd-admin` reste non installé sans binaire |
| YCbCr | debugfs peut être en lecture seule ou non monté | Vérifier écriture + réapplication de mode |
| Capteurs | RAPL `uncore` n'est pas toujours l'iGPU ; i915 sans hwmon | Comparer avec `powerstat`/`intel_gpu_top` |
| Couvercle | L'original n'appliquait pas les changements externes quand `LidIsClosed` était vrai. Notre worker ne surveille pas les changements sysfs externes : aucune dépendance à UPower n'est nécessaire dans `tcc-hw`. Si un watcher sysfs est ajouté plus tard, `linux-platform` devra fournir l'état du couvercle | Vérifier qu'aucune écriture n'a lieu couvercle fermé sur un modèle RGB |
| Veille | Réouverture des backends après reprise | Vérifier ventilateurs, TDP, charge et webcam après `systemctl suspend` |
| Contrôles UVC | **Lecture implémentée** (étape 1, `tcc-hw::v4l2`) ; écriture non implémentée | `DeviceAllow=char-video4linux r` requis (packaging à suivre) ; écriture prévue dans `tccd-user` après revue ; module isolé, allowlist normative |

### Écarts assumés documentés

1. Luminosité clavier écrite via sysfs, pas UPower.
1bis. Les SKU réels `IBP14A10MK1` / `IBP15A10MK1` sont reconnus comme
   `IBPG10AMD` : l'original ne pouvait pas les identifier (clé littérale avec
   espace), donc son quirk `platform_profile` ACPI ne s'appliquait jamais sur
   ces modèles. Correction explicite.
2. Mode « respiration » jamais appliqué (aucun attribut LED ; l'original ne
   l'appliquait pas non plus).
3. Les workers n'écrivent qu'au changement de révision, pas à chaque tick
   (évite d'écraser un réglage externe).
4. Seuils de charge réappliqués au démarrage si `charge_type == Custom`.
5. `platform_profile` non écrit sous PPD sans override explicite (D4).
6. TDP non croissant refusé.
7. `buffer_input` toujours relâché en cas d'erreur.
8. `pendingModeReapply` est un événement par tick, pas un latch.
9. PRIME relu toutes les 30 s (détection complète toutes les 5 min) au lieu de
   10 s.
10. `nvidia-smi` exécuté avec timeout et sortie bornée (l'original pouvait
    bloquer indéfiniment).
11. Contrôles UVC webcam : lecture livrée et testée sur caméra ; backend
    d'écriture + service `User1` livrés (tests fake), **écriture réelle non
    testée** ; préréglages persistés et CID privés restent hors périmètre.

---

## 8. Tests fournis

| Fichier | Contenu |
|---|---|
| `crates/tcc-hw/src/charging.rs` | listes disponibles, hors plage, fichiers absents, `Custom` avant seuils, cohérence début/fin, idempotence, type invalide |
| `crates/tcc-hw/src/lighting.rs` | blanc, RGB 3 zones, per-key, HID, lightbar, `buffer_input`, absence, plafonnement |
| `crates/tcc-hw/src/platform_profile.rs` | priorité TUXEDO > ACPI, quirk modèle, refus PPD, choix inconnu |
| `crates/tcc-hw/src/tdp.rs` | `FakeTransport` Uniwill/Clevo, bornes, non-entier, décroissant, vide ⇒ maxima |
| `crates/tcc-hw/src/device.rs` | table SKU complète, fallback Uniwill, quirks |
| `crates/tcc-hw/src/fn_lock.rs` | lecture/écriture, absent, contenu invalide |
| `crates/tcc-hw/src/sensors.rs` | CPU/RAPL (delta, wrap), AMD iGPU/dGPU, Intel, NVIDIA sans outil, connecteurs |
| `crates/tcc-hw/src/nvidia.rs` | cTGP, bornes, parsing CSV, `[N/A]` |
| `crates/tcc-hw/src/prime.rs` | routage eDP, modes, état absent |
| `crates/tcc-hw/src/webcam.rs` | Clevo/Uniwill, énumération, attributs corrompus |
| `crates/tcc-hw/src/ycbcr.rs` | chemins, traversée refusée, debugfs absent, énumération |
| `crates/tcc-hw/src/command.rs` | allowlist, timeout, code non nul, environnement nettoyé, descendant qui retient `stdout`, nettoyage |
| `crates/tcc-hw/src/nvidia.rs` | cache capteurs/limites, cache des échecs, parsing CSV, `[N/A]` ; lanceur injecté, **aucun processus ni script temporaire** dans les tests (déterminisme, pas de `ETXTBSY`) |
| `crates/tcc-hw/src/display.rs` | connecteur interne DRM : connecté/déconnecté, externe ignoré, absence, parsing `cardN-eDP-1` |
| `crates/tcc-daemon/src/hardware/*` | ticks, révisions, dry-run, hystérésis d'écriture, interrupteur d'éclairage, sérialisation camelCase ; DMI, topologie CPU/EPP, connecteur DRM |
| `crates/tcc-hw/src/sensors.rs` | `gpu_presence` iGPU/dGPU depuis DRM, absence, sans collecte de capteurs |
| `crates/tcc-hw/src/v4l2/write.rs` | allowlist/step/menus/booléens, device absent/busy/EIO+quarantaine, relecture, budget, `try_lock`, factory |
| `crates/tcc-user` (`User1`) | service de session, validation, rate limit, erreurs typées (hors périmètre de ce document, voir `DBUS_API.md`) |
| `crates/tcc-hw/src/v4l2/real_write.rs` | flags `O_RDWR`, symlink `ELOOP`, device absent, noms invalides |
| `crates/tcc-hw/tests/v4l2_unsafe_guard.rs` | `S_CTRL` uniquement dans `abi.rs`/`real_write.rs`, `unsafe` confiné aux deux transports |
| `crates/tcc-daemon/src/hardware/capabilities.rs` | batterie/AC présents ou absents, batterie retirée, rôles GPU, camelCase, champs absents |
| `crates/tcc-daemon/src/hardware/device.rs` | DMI, `kernelRelease`, architecture, version `tuxedo_io`, absence tolérée |
| `crates/tcc-daemon/src/hardware/odm.rs` | io seul sans platform_profile, ODM présent/TDP absent, choix io, TDP borné, PPD prioritaire (fallback io non utilisé), absence des deux ; nom ODM inconnu ⇒ aucune écriture, profil sans ODM ⇒ aucune écriture, TDP vide ⇒ aucune écriture, TDP non vide ⇒ écriture |
| `crates/tcc-daemon/src/hardware/nvidia.rs` | borne cTGP dynamique 150 W (max 300/default 150), 150 accepté/151 refusé, absente ⇒ 200 statique, clamp 0/200, camelCase |
| `crates/tcc-hw/src/tuxedo_io.rs` | profils Uniwill selon `R_UW_PROFS_AVAILABLE`, `tdp_count` partiel/absent, limites TDP factices |
| `crates/tcc-daemon/src/hardware/charging.rs` | `chargingType` valide appliqué et relu, invalide non écrit, backend absent no-op, idempotence par révision |
| `crates/tcc-daemon/src/hardware/fn_lock.rs` | `fnLockStatus` `Some(true/false)` appliqué, absence no-op, relecture après écriture, idempotence par révision, dry-run |
| `crates/tcc-daemon/src/hardware/mod.rs` | désérialisation d'un `HardwareSnapshot` sans les nouvelles sections (`serde(default)`) |
| `crates/tcc-daemon/tests/hardware_workers.rs` | démarrage/arrêt sans matériel, collecte désactivée, schéma JSON stable |
| `crates/tcc-daemon/tests/no_admin_reachability.rs` | `tccd` n'appelle pas la bascule PRIME et ne lance aucune commande externe directement |
