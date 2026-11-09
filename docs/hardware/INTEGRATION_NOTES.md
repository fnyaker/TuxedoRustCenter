# Notes d'intégration — matrice matérielle (pour `linux-platform`)

Document court, orienté checklist. La référence complète est
`docs/hardware/HARDWARE_MATRIX.md`.

## 1. Branchement minimal

```rust
// main.rs (fichier linux-platform)
mod hardware;

// après la création de RuntimeState et du SysfsRoot :
let hw_inputs = Arc::new(tokio::sync::RwLock::new(
    hardware::HardwareInputs::new(
        state.settings.clone(),
        state.active_profile().cloned(),
        inputs.ppd.is_some(),
        state.revision,
    )
    .with_ppd_override(false), // réglage utilisateur explicite
));
let hw_status = Arc::new(hardware::HardwareStatus::default());
let sensors_enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
let hardware = hardware::HardwareWorkers::start(
    hw_inputs.clone(),
    hw_status.clone(),
    root.clone(),
    dry_run,
    sensors_enabled.clone(),
).await;
```

## 2. À chaque changement

| Événement | Action |
|---|---|
| Profil actif changé | mettre à jour `HardwareInputs` puis `hardware.notify()` |
| `Settings` modifiés (SIGHUP, setter D-Bus) | idem, **incrémenter `revision`** |
| `SetSensorDataCollectionStatus(b)` | `sensors_enabled.store(b, Relaxed)` |
| Reprise après veille | reconstruire `HardwareInputs`, `hardware.notify()` |
| SIGTERM/SIGINT | `hardware.shutdown().await` |

La révision est le **seul** déclencheur d'écriture : un tick périodique sans
changement de révision ne fait que rafraîchir la télémétrie.

## 3. Setters D-Bus

Ordre recommandé : validation D-Bus → appel `tcc-hw` → persistance →
`HardwareInputs` + `notify()`.

| Méthode originale | Appel `tcc-hw` |
|---|---|
| `SetChargingProfile(s)` | `charging::ChargingController::set_profile` |
| `SetChargingPriority(s)` | `charging::ChargingController::set_priority` |
| `SetChargeStartThreshold(i)` | `charging::ChargingController::set_start_threshold` |
| `SetChargeEndThreshold(i)` | `charging::ChargingController::set_end_threshold` |
| `SetChargeType(s)` | `charging::ChargingController::set_charge_type` |
| `SetKeyboardStatesJSON(s)` | `lighting::LightingController::apply_keyboard` **après** `hardware::lighting::ensure_control_enabled` |
| `SetLightbarStatesJSON(s)` | `lighting::LightingController::apply_lightbar` **après** `ensure_control_enabled` |
| `SetFnLockStatus(b)` | `fn_lock::FnLock::set` |
| `ODM` / TDP | `platform_profile::PlatformProfile::apply` / `tdp::TdpControl::set_values` |
| cTGP | `nvidia::CtgpControl::set_offset_watts` |
| YCbCr | `ycbcr::apply(root, card, port, enabled)` |
| webcam on/off | `webcam::WebcamSwitch::set` |
| PRIME | **aucune API dans `tcc-hw`** : implémentation directe dans `tccd-admin` + polkit (H-01) |
| `SetSensorDataCollectionStatus(b)` | `AtomicBool` (pas d'appel matériel immédiat) |

## 4. Lecture des états

```rust
let status = hw_status.snapshot();           // HardwareSnapshot (serde camelCase)
let charging = hw_status.charging();         // ChargingData
let sensors  = hw_status.sensors();          // SensorData (enabled/available)
```

Ces appels ne bloquent pas et tolèrent un verrou empoisonné.

`nvidia-smi` est mis en cache 10 s par instance (`NVIDIA_MIN_QUERY_INTERVAL`),
échecs compris : la collecte à 2 s ne peut pas lancer un processus root à cette
fréquence. Aucun réglage n'est nécessaire côté daemon.

## 5. Points de vigilance

1. **PPD (D4)** : ne pas écrire `platform_profile` si `ppd_active` sans
   `ppd_override`. `OdmData.platform_profile.owner` indique le propriétaire
   effectif à l'UI.
2. **Collecte capteurs** : ne pas appeler `HardwareStatus::sensors()` en
   boucle sans client ; le worker ne lit rien si `enabled == false`.
3. **Révision** : oublier de l'incrémenter signifie « ne rien réécrire » ; une
   valeur inchangée n'est de toute façon pas réécrite par `tcc-hw`.
4. **Veille** : `tccd-sleep` doit arrêter/reprendre le daemon ; les workers
   rouvrent `/dev/tuxedo_io` tout seuls, mais l'état des capteurs RAPL repart
   de zéro (premier échantillon sans puissance).
4bis. **Éclairage** : `ensure_control_enabled(&settings)` est obligatoire dans
   les setters. Le worker respecte déjà `keyboard_backlight_control_enabled` ;
   `dbus.rs` doit s'aligner (finding H-06).
5. **Webcam** : `controlsSupported` est toujours `false` (pas d'ioctl V4L2).
   Ne pas exposer les réglages UVC dans l'UI tant que ce n'est pas levé.
5bis. **Setters** : charge/LED/Fn-Lock sont rate-limités et audités mais **non
   polkit**, et **non persistants** (SIGHUP les écrase). Le documenter dans
   `docs/DBUS_API.md` (H-11). TDP/ODM/cTGP/PRIME/YCbCr ne sont pas exposés en
   écriture.
6. **Debugfs** : sans montage de `/sys/kernel/debug`, YCbCr est simplement
   `available: false`.
7. **`crates/tcc-daemon/tests/hardware_workers.rs`** : harnais qui inclut le
   module via `#[path]` pour que la CI compile/tests sans `mod hardware;` dans
   `main.rs`. Une fois le branchement fait, ce harnais peut être remplacé par
   les tests d'intégration de `qa-integration`.
8. **`crates/tcc-daemon/tests/no_admin_reachability.rs`** : garde statique
   bloquante. Elle échoue si `tccd` référence `prime::set_mode`,
   `PRIME_SWITCH_TIMEOUT` ou lance une commande externe directement. Équivalent
   manuel : `grep -rn --include='*.rs' -E 'prime::set_mode|PRIME_SWITCH_TIMEOUT' crates/tcc-daemon/src --exclude-dir=bin`.

## 6. Ce qui reste à décider (hors de ce périmètre)

- noms exacts des méthodes `Manager1` et types `tcc-ipc` associés ;
- réglage UI « TCC prioritaire sur PPD » (`ppd_override`) ;
- politique de reprise après veille (réapplication complète vs relecture) ;
- autorisation `unsafe` pour les contrôles UVC (décision `security`) ;
- validation de `DeviceAllow=/dev/tuxedo_io` et de l'accès debugfs avec le
  hardening systemd.
