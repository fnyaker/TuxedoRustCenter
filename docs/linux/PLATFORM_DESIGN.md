# Conception plateforme Linux — TuxedoRustCenter

Statut : document de référence pour `tcc-hw`, `tcc-daemon`, `tcc-user` et le packaging.
Il complète `AGENTS.md`, `docs/ARCHITECTURE.md` et `docs/FEATURE_PARITY.md`.
Toutes les décisions ci-dessous sont vérifiées dans le code original
`tuxedo-control-center-master/` (fichier cité) ou dans l'ABI noyau.

---

## 1. Principes d'accès matériel

1. **Tout passe par `tcc-hw`.** Le daemon n'ouvre ni ne parse sysfs directement.
2. **Racine injectable, verrouillée en production.** `SysfsRoot` porte `/sys`
   en production. La variable `TCC_SYSFS_ROOT` n'est lue que si la feature
   Cargo `dev-overrides` est active, que l'UID effectif n'est pas `0`, et que
   le chemin existe (D7). Elle est absente des paquets de release et les unités
   systemd ne la transmettent jamais. Les tests unitaires utilisent
   `SysfsRoot::new(tempdir)` directement.
3. **Écritures contrôlées.** Chemin relatif validé (pas de `..`, pas d'absolu),
   `open(O_WRONLY|O_NOFOLLOW|O_CLOEXEC|O_TRUNC)`, `fstat` sur le descripteur,
   refus des liens symboliques et des non-fichiers réguliers. Sur la racine de
   production (`SysfsRoot::system`), le fichier doit en plus être possédé par
   root et ne pas être inscriptible par le monde. `O_TRUNC` est ignoré par
   kernfs mais rend les fichiers de test déterministes.
4. **Aucune commande shell.** Les rares commandes externes (`prime-select`,
   `nvidia-smi`, `kscreen-doctor`, `tuxedo-tomte`, `systeminfos`) sont
   exécutées par allowlist, avec `Command::new(nom_absolu)` et arguments
   séparés, après validation des entrées. Jamais de `sh -c`.
5. **Event-driven.** Signaux D-Bus PPD/UPower/logind, netlink udev, inotify.
   Le polling capteurs n'est activé que si un client D-Bus demande la collecte.
6. **Dégradation propre.** Matériel absent ⇒ `HwError::NotFound`/`NotSupported`,
   listes vides, état `unsupported` exposé à l'UI. Jamais de panic, jamais de
   valeur inventée.
7. **`unsafe` isolé.** Uniquement `tuxedo_io::real`, avec commentaires de
   sûreté. Le reste du crate interdit `unsafe` (`#![deny(unsafe_code)]`).

---

## 2. Tableau des chemins par fonctionnalité

### 2.1 CPU

| Fonction | Chemin / mécanisme | Accès |
|---|---|---|
| Topologie | `/sys/devices/system/cpu/{possible,present,online,kernel_max}` | R |
| Cœur en ligne | `/sys/devices/system/cpu/cpuN/online` (absent pour `cpu0`) | R/W |
| Pilote cpufreq | `.../cpuN/cpufreq/scaling_driver` | R |
| Governor | `.../cpuN/cpufreq/scaling_governor` | R/W |
| Governors dispo. | `.../cpuN/cpufreq/scaling_available_governors` | R |
| EPP | `.../cpuN/cpufreq/energy_performance_preference` | R/W |
| EPP dispo. | `.../cpuN/cpufreq/energy_performance_available_preferences` | R |
| Fréquences | `.../cpuN/cpufreq/{cpuinfo_min_freq,cpuinfo_max_freq}` (kHz) | R |
| Bornes pilotées | `.../cpuN/cpufreq/{scaling_min_freq,scaling_max_freq}` (kHz) | R/W |
| Fréquence courante | `.../cpuN/cpufreq/scaling_cur_freq` (kHz) | R |
| Fréquences dispo. | `.../cpuN/cpufreq/scaling_available_frequencies` (kHz) | R |
| Turbo Intel | `/sys/devices/system/cpu/intel_pstate/no_turbo` | R/W |
| Intel pstate | `/sys/devices/system/cpu/intel_pstate/{max_perf_pct,min_perf_pct,num_pstates,status,turbo_pct}` | R |
| AMD pstate | `/sys/devices/system/cpu/amd_pstate/status` (`active`, `passive`, `guided`, `off`) | R |
| Boost ACPI/AMD | `/sys/devices/system/cpu/cpufreq/boost` | R/W |
| RAPL paquet | `/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/{name,energy_uj,enabled,constraint_0_*}` | R |
| RAPL iGPU | `/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/intel-rapl:0:1/{name,energy_uj,...}` | R |

Décisions :

- Les fréquences sont manipulées en **kHz** partout (unité sysfs), comme
  `CpuController` de l'original.
- `tcc-hw::cpu` valide le governor et l'EPP contre les listes
  `*_available_*` **avant** toute écriture. Contrairement à l'original, aucune
  écriture partielle n'est faite si une politique refuse la valeur.
- Une fréquence est bornée à `[scaling_min, cpuinfo_max]` (max) ou
  `[cpuinfo_min, scaling_max]` (min), puis alignée sur
  `scaling_available_frequencies` si le fichier existe.
- `no_turbo` n'est écrit que si `/sys/.../intel_pstate/no_turbo` existe ;
  `boost` seulement si `cpufreq/boost` existe.
- **PPD reste propriétaire de `platform_profile` et de l'EPP par défaut**
  (voir §4). `tcc-hw` expose les setters, mais `tcc-daemon` ne les appelle
  qu'en mode explicite « TCC prioritaire ».
- L'original écrit `useCores(1)` et ne touche pas `cpu0` : reproduit.
- Le `no_turbo` historique est lu/écrit via `intel_pstate`, pas via le
  governor `powersave`.

### 2.2 Ventilateurs

Ordre de priorité imposé (identique à `FanControlWorker` original) :

1. **`tuxi`** — hwmon `tuxedo_tuxi_sensors`
   - `/sys/class/hwmon/hwmonN/name == "tuxedo_tuxi_sensors"`
   - `fanM_input`, `fanM_label`, `fanM_max`, `tempM_input`, `tempM_label`
   - `pwmM`, `pwmM_enable` dans `/sys/bus/platform/devices/tuxedo_fan_control/`
2. **`pwm`** — hwmon `tuxedo` (mêmes attributs)
3. **`tuxedo-io`** — `/dev/tuxedo_io` (Clevo/Uniwill, voir §2.12)

Décisions :

- La correspondance ventilateur ↔ température se fait par label
  (`fanM_label` commence par `cpu`/`gpu`, `cpu\d+` normalisé en `cpu`), avec
  fallback sur le premier capteur, comme `FanControlHwmon`. Un label absent ne
  tue plus le daemon (l'original appelle `process.exit(0)`).
- `pwm_enable` : `1` = manuel, `2` = automatique.
- Sur Sirius 16 Gen1/Gen2, seul `tuxi` est utilisé (quirk original) pour éviter
  la double écriture et la charge CPU.
- Les écritures sont dédupliquées : une consigne inchangée ne touche pas
  sysfs. Exception assumée : sur Uniwill (`tuxedo-io`), la vitesse est
  réécrite à chaque tick (quirk TUXEDO documenté dans l'original, l'EC
  dérive).
- Le filtre médian sur 13 échantillons de l'original n'est pas reproduit :
  l'hystérésis de 2 % assure l'anti-oscillation avec une réaction d'une
  seconde. Différence documentée, à réévaluer si des oscillations sont
  observées sur matériel réel.

État d'implémentation (jalon 2) :

- **Fait** — `tcc-hw::fan` implémente les trois backends derrière le trait
  `FanControl` : nombre de ventilateurs, libellé, rôle CPU/GPU, température,
  vitesse mesurée, écriture de vitesse, mode manuel/auto, vitesse minimale et
  « fans off available ».
- **Fait** — détection `detect` / `detect_with` avec priorité `tuxi` > `pwm` >
  `tuxedo-io`, sans panic si `/dev/tuxedo_io` est absent (`None`).
- **Fait** — `tcc-daemon::fan::FanWorker` : tick 1 s, indépendant de la boucle
  de bascule, accès matériel en `spawn_blocking`, arrêt propre avec retour en
  auto.
- **Fait** — `GetFanDataJSON` expose backend, mode, températures, vitesses
  mesurées et consignes appliquées (voir `docs/DBUS_API.md`).
- **Reste hwmon** — les machines dont le module `tuxedo_fan_control` n'expose
  pas `fanM_pwm_enable` sont traitées en **lecture seule** : le daemon ne
  force pas le mode manuel et n'écrit pas de PWM. Le support d'un `pwmM` sans
  `pwm_enable` reste à valider sur matériel réel.
- **Fait** — la veille/reprise est gérée par `org.freedesktop.login1.Manager.PrepareForSleep` :
  arrêt borné des workers avec retour auto, recréation des backends et
  réapplication du profil à la reprise (voir §2.2.1). `tccd-sleep.service`
  n'est plus nécessaire.

Logique de courbe (daemon) :

- `tcc_core::fan::FanCurveEvaluator` : offset, bornes min/max du profil puis
  hystérésis de 2 %.
- Trois garde-fous repris de l'original : plancher de température critique
  (30 % dès 80 °C, 40 % dès 90 °C — `FanUtils.manageCriticalTemperature`),
  limitation de chute à 2 %/tick au-dessus de 20 %
  (`FanControlLogic.limitFallingFanSpeed`) et vitesse minimale matérielle
  (`FanControlLogic.applyHwFanLimitations`). Ce dernier diffère
  volontairement d'un cas : si `fansOffAvailable` est faux, l'original
  pouvait renvoyer une valeur sous le minimum ; ici le minimum est toujours
  respecté.
- Les courbes nommées `Silent`, `Quiet`, `Balanced`, `Cool`, `Freezy` sont
  intégrées au daemon à partir des `defaultFanProfiles` de l'original,
  encodées par points de rupture avec une rampe bornée à 1 °C (jamais moins
  refroidissantes que l'escalier d'origine). Priorité : courbe `Custom` du
  profil, puis `FanTableSet` (migration `/etc/tcc/fantables`), puis courbe
  intégrée du même nom, puis `Balanced`.

Risques matériels :

- **Écriture PWM** : une valeur aberrante peut bloquer un ventilateur à
  l'arrêt ou à 100 %. Les index et pourcentages sont validés (`0..=100`), les
  courbes sont bornées et le mode auto est restauré à l'arrêt du daemon et
  avant la veille. Un `SIGKILL` ou un plantage pendant un tick manuel reste un
  risque résiduel : `PrepareForSleep` couvre la veille nominale, pas un arrêt
  brutal du processus.
- **Absence de matériel** : aucun backend → aucune écriture, statut
  `available: false`. Aucun fallback heuristique vers un autre backend.
- **Capteur manquant** : si un capteur individuel disparaît, sa consigne
  précédente est conservée (pas de mise à zéro) ; si **aucun** capteur n'est
  lisible, le worker rend les ventilateurs au firmware plutôt que de les
  verrouiller en manuel sans consigne.
- **Arrêt/veille** : retour en auto sur `SIGTERM`/`SIGINT` et avant
  `PrepareForSleep(true)` ; reprise avec recréation du backend. La validation
  matérielle réelle (Clevo, Uniwill) reste à faire.

### 2.2.1 Veille/reprise (logind)

`tccd` s'abonne au signal
`org.freedesktop.login1.Manager.PrepareForSleep(bool)` sur le bus système
(`/org/freedesktop/login1`). Aucun service `tccd-sleep` n'est requis : le
daemon reste vivant pendant la veille et se réconcilie lui-même.

Flux :

1. `true` (avant veille) : `FanWorker::suspend` rend les ventilateurs au
   firmware (mode auto + libération du mode manuel Uniwill), puis
   `HardwareWorkers::pause` arrête les workers. Budget global **3 s**
   (`sleep::SUSPEND_BUDGET`) ; les deux arrêts sont concurrents. La
   configuration n'est ni modifiée ni supprimée.
2. `false` (reprise) : les workers matériels sont recréés (nouvelle détection
   `tuxedo_io`/hwmon/backlight, les fd pouvant être invalides), `FanWorker`
   est redémarré, le bail capteurs est réinitialisé à « désactivé », PPD/UPower
   sont relus (sysfs en fallback), puis le profil actif est réappliqué
   (CPU, luminosité, ventilation via le worker) et les entrées matérielles
   sont réconciliées.
3. `false` pendant la préparation = veille annulée : le daemon redémarre ce
   qu'il venait d'arrêter. Les signaux dupliqués sont ignorés par une machine
   à états (`Awake → Preparing → Suspended → Resuming → Awake`).
4. Si logind est absent, un `warn` unique est émis puis une reconnexion est
   tentée toutes les 5 s ; le daemon continue sans gestion de veille.
5. Si le daemon s'arrête pendant la veille, aucun réveil synthétique n'est
   déclenché ; l'arrêt des composants est idempotent.
6. **Inhibitor** : `tccd` prend
   `Inhibit("sleep", "tccd", "hardware handoff", "delay")` **au démarrage** et
   conserve le fd. Il le libère après le handoff effectif (avant l'état
   `Suspended`) puis le ré-acquiert après `Resumed`, pour retarder la veille
   suivante. Si l'inhibitor échoue (logind absent, quota, `Inhibit` en erreur),
   un `warn` est émis et le daemon continue en repli signal-only.
   `InhibitDelayMaxSec` doit couvrir le budget de 3 s.
7. **Redémarrage pendant la veille** : au démarrage, `tccd` lit la propriété
   `PreparingForSleep` de logind. Si elle est vraie, il démarre en état
   `Suspended` sans appliquer le profil ni démarrer workers/fan, puis attend
   `false`. Si la propriété est indisponible, il démarre normalement.
8. **Reload pendant la veille** : `SIGHUP` et `Control1.ReloadConfig` sont
   refusés/différés (`ok=false`, avertissement explicite) pendant
   `Preparing`/`Suspended` ; une demande coalescée est rejouée une seule fois
   après la reprise.
9. **Arrêt non annulable** : `HardwareWorkers::abort_all` (stop + abort des
   tâches) est synchrone avant toute attente ; `FanWorker::stop_worker`
   `abort()` le handle au timeout avant le dernier `set_auto`. Le budget de
   3 s ne laisse donc pas de tâches tokio actives pour cohabiter avec la
   reprise (un tick déjà dans `spawn_blocking` reste une limite résiduelle).
10. **Reprise bornée** : lectures PPD/UPower limitées à 2 s chacune avec repli
    dernière valeur/sysfs ; la lecture PPD **au démarrage** est également
    bornée à 2 s (repli `None`/sysfs). `hooks.resume()` est limité à 5 s.
    Au-delà, la boucle reprend et le profil est réappliqué au tick suivant. Le
    watcher logind est supervisé et relancé avec back-off (1 s → 30 s) s'il
    meurt.

Limites : l'inhibitor `delay` est best-effort. S'il est refusé, ou si
`InhibitDelayMaxSec` est trop court, `PrepareForSleep` reste un **signal** :
logind peut suspendre avant la fin du handler. Le budget de 3 s borne le
travail ; un noyau qui suspend immédiatement peut interrompre l'arrêt (retour
auto best-effort). L'inhibitor ne couvre pas un `SIGKILL` de `tccd`. Un `SIGKILL` pendant un tick
manuel peut encore laisser l'EC en manuel ; un backend dont le fd est perdu
est rouvert par la nouvelle détection, mais le premier tick après reprise peut
échouer une fois (logué, sans arrêt du daemon).

Test manuel :

```sh
sudo systemctl suspend
# au réveil :
journalctl -u tccd | grep -E "veille|reprise"
# vérifier le retour auto, la réapplication du profil et
# GetSensorDataCollectionStatus=false (bail réinitialisé)
```

### 2.2.2 Minuterie d'extinction (logind)

`tccd` lit `Settings.shutdownTime` au démarrage, après chaque reload
(SIGHUP/`Control1`) et après un changement de réglages. La valeur est une
heure **locale** `"HH:MM"` (24 h) ; `null` ou vide désactive.

- Conversion en échéance absolue (`usec` epoch) via `chrono::Local` : si
  l'heure est déjà passée aujourd'hui, l'échéance est le lendemain. Une heure
  locale inexistante (transition DST) produit `error=heure d'extinction
  invalide` plutôt qu'un décalage silencieux.
- Appels logind natifs uniquement, avec timeout 2 s :
  `ScheduleShutdown("poweroff", usec)` et `CancelScheduledShutdown()`.
  Aucun `systemctl`, `shutdown` ou shell.
- Idempotence : une même échéance n'est pas reprogrammée à chaque tick. Une
  erreur logind n'est pas réessayée en boucle ; elle l'est au prochain reload,
  changement de réglage ou reprise de veille.
- Pendant `Preparing`/`Suspended`, aucune réconciliation n'est faite ; elle
  reprend après `Resumed`. Le timer ne réveille pas la machine.
- État exposé en lecture seule par `Manager1.GetShutdownTimerJSON` :
  `{enabled, scheduledAt, source, error}`, limité à 2 lectures/s (burst 5)
  par nom unique, bucket séparé du snapshot. `source=settings` désigne une
  échéance issue des réglages ; `logind` est réservé à une future lecture de
  l'état logind ; `none` signifie aucune échéance active ; `error` signale un
  réglage invalide ou un échec logind. Une heure invalide (`"25:99"`) annule
  l'ancienne échéance si nécessaire mais **conserve l'erreur visible** et
  `enabled=false` ; elle n'est pas reprogrammée tant que le réglage reste
  invalide. Une valeur `null`/vide, elle, produit l'état `disabled` sans
  erreur.
- logind absent ou en erreur : `warn` unique côté daemon, état `enabled=false`
  avec `error=logind indisponible`, aucune panique. Les réglages restent
  inchangés et la GUI peut afficher l'état non supporté.
- Limites : l'échéance est calculée dans le fuseau local courant ; un
  changement de fuseau/DST entre le calcul et l'échéance est résolu par
  logind au moment voulu. Aucun test de veille réelle ; la validation
  `ScheduleShutdown`/`CancelScheduledShutdown` sur bus système réel reste à
  faire.

### 2.3 Rétroéclairage

| Fonction | Chemin | Accès |
|---|---|---|
| Énumération | `/sys/class/backlight/*` | R |
| Maximum | `/sys/class/backlight/<nom>/max_brightness` | R |
| Valeur écrite | `/sys/class/backlight/<nom>/brightness` | R/W |
| Valeur réelle | `/sys/class/backlight/<nom>/actual_brightness` (optionnel) | R |
| Type | `/sys/class/backlight/<nom>/type` | R |

- Quirk `amdgpu_bl*` : `actual_brightness` peut être sur 16 bits
  (`0..=0xffff`) alors que `max_brightness` est sur 8 bits. La valeur
  d'affichage est ramenée sur `0..=255` ; le pourcentage est calculé sur la
  valeur normalisée et `max_brightness`.
- L'original écrit sur **tous** les drivers détectés. `set_all_percent`
  reproduit ce comportement et retourne un rapport `applied`/`failures`.
- `pick_primary` préfère `intel_backlight`, `amdgpu_bl0`, `acpi_video0`
  pour l'affichage d'une valeur unique dans l'UI.
- Les erreurs par périphérique n'interrompent pas les autres écritures.

### 2.4 Alimentation et charge

| Fonction | Chemin | Accès |
|---|---|---|
| Énumération | `/sys/class/power_supply/*` | R |
| Type | `/sys/class/power_supply/<nom>/type` (`Mains`, `Battery`, `USB`, ...) | R |
| Secteur | `/sys/class/power_supply/<nom>/online` | R |
| Capacité | `/sys/class/power_supply/<nom>/capacity` (0..100) | R |
| Statut | `/sys/class/power_supply/<nom>/status` (`Charging`, `Discharging`, `Full`, `Not charging`) | R |
| Présence | `/sys/class/power_supply/<nom>/present` | R |
| Seuils standard | `.../charge_control_start_threshold`, `.../charge_control_end_threshold` | R/W |
| Type de charge | `.../charge_type` | R/W |
| Profil TUXEDO | `/sys/devices/platform/tuxedo_keyboard/charging_profile/charging_profile` | R/W |
| Profils dispo. | `/sys/devices/platform/tuxedo_keyboard/charging_profile/charging_profiles_available` | R |
| Priorité TUXEDO | `/sys/devices/platform/tuxedo_keyboard/charging_priority/charging_prio` | R/W |
| Priorités dispo. | `/sys/devices/platform/tuxedo_keyboard/charging_priority/charging_prios_available` | R |

Décisions :

- `PowerState` = `Ac`, `Battery`, `Unknown`.
  - Priorité à un supply `Mains` : `online=1` ⇒ AC, tous `online=0` ⇒ Battery.
  - Sans supply `Mains`, déduction depuis `status` de batterie
    (`Discharging` ⇒ Battery, `Charging`/`Full` ⇒ AC).
  - Ni l'un ni l'autre ⇒ `Unknown`. L'original retournait AC par défaut ; ce
    choix est écarté car il déclenche le profil « secteur » sur une machine
    sans source identifiable (VM, conteneur).
- Les supply `USB`/USB-C sont ignorés pour décider AC/BAT (ils ne reflètent pas
  forcément la source active). Ils restent lisibles par sysfs.
- La charge TUXEDO (`charging_profile`, `charging_priority`) reste dans
  `/sys/devices/platform/tuxedo_keyboard`, pas dans `power_supply`.
- L'écriture des seuils passe par `tcc-hw::SysfsRoot` et devra être validée
  contre `*_available_thresholds` quand le noyau les expose (module
  `charging` du jalon 4).
- UPower est la source préférée du daemon pour les événements ; sysfs reste le
  fallback et la source de vérité en cas de divergence.

### 2.5 Éclairage clavier / lightbar

| Élément | Chemin | Accès |
|---|---|---|
| Blanc (TUXEDO) | `/sys/devices/platform/tuxedo_keyboard/leds/white:kbd_backlight` | R/W |
| Blanc (NB05) | `/sys/bus/platform/devices/tuxedo_nb05_kbd_backlight/leds/white:kbd_backlight` | R/W |
| Zones RGB | `/sys/devices/platform/tuxedo_keyboard/leds/rgb:kbd_backlight[_1|_2]` | R/W |
| Lightbar | `/sys/class/leds/rgb:lightbar` | R/W |
| Luminosité | `<led>/brightness`, `<led>/max_brightness` | R/W |
| Couleur | `<led>/multi_intensity` (`R G B`) | R/W |
| Buffer d'entrée | `<led>/device/controls/buffer_input` (`0`/`1`) | R/W |

- Modes supportés par l'original : statique et respiration, par zone ; le mode
  « per-key » n'est pas exposé par ces attributs.
- Pour les LED RGB, l'original écrit `multi_intensity` puis bascule
  `buffer_input` à `1` pour pousser l'état, puis `0`. Reproduire exactement.
- UPower/logind fournit l'état du couvercle (`LidIsClosed`) pour
  l'extinction automatique ; ne pas lire `/proc`.
- Écriture de `multi_intensity` : une seule valeur `"R G B"`, donc
  `write_string` est adapté (les espaces sont autorisés, pas les sauts de
  ligne).

### 2.6 Webcam

| Fonction | Mécanisme |
|---|---|
| Kill switch Clevo | ioctl `R_CL_WEBCAM_SW` / `W_CL_WEBCAM_SW` (`/dev/tuxedo_io`) |
| Contrôles UVC | V4L2 (`VIDIOC_QUERYCTRL`, `VIDIOC_G_CTRL`, `VIDIOC_S_CTRL`) sur `/dev/video*` |
| Hotplug | netlink udev, `SUBSYSTEM=="video4linux"` |
| Préréglages | `/var/lib/tuxedo-rust-center/webcam.json` |
| Aperçu | Qt Multimedia natif dans la GUI |

- L'original utilisait `cameractrls.py` appelé par udev. Remplacé par V4L2
  natif dans `tcc-hw` (jalon 4) et un watcher udev dans le daemon.
- Uniwill n'implémente pas le kill switch : `HwError::NotSupported`.
- La règle udev du projet ne lance **aucun** processus (voir
  `packaging/udev/99-tuxedo-rust-center-webcam.rules`) ; elle tague seulement
  l'événement pour le moniteur netlink.

### 2.7 ODM / platform_profile / TDP

| Fonction | Chemin / ioctl |
|---|---|
| Profil TUXEDO | `/sys/bus/platform/devices/tuxedo_platform_profile/platform_profile` |
| Choix TUXEDO | `/sys/bus/platform/devices/tuxedo_platform_profile/platform_profile_choices` |
| ACPI standard | `/sys/firmware/acpi/platform_profile` |
| Choix ACPI | `/sys/firmware/acpi/platform_profile_choices` |
| Fallback Clevo | `W_CL_PERF_PROFILE` (`quiet`, `power_saving`, `performance`, `entertainment`) |
| Fallback Uniwill | `W_UW_PERF_PROF` (`power_save`, `enthusiast`, `overboost`) |
| Profils dispo. Uniwill | `R_UW_PROFS_AVAILABLE` |
| TDP lire | `R_UW_TDP0/1/2` (PL1/PL2/PL4) |
| TDP min/max | `R_UW_TDP*_MIN`, `R_UW_TDP*_MAX` |
| TDP écrire | `W_UW_TDP0/1/2` |

- Ordre de l'original (`ODMProfileWorker`) : `tuxedo_platform_profile` >
  ACPI `platform_profile` > `tuxedo-io`.
- Quirk « pas de platform_profile » pour les modèles IBP G10 AMD, IBM15A10,
  IBM16A10 (liste à porter dans `tcc-core`/quirks device).
- **Conflit PPD** : `platform_profile` est aussi écrit par PPD. Par défaut,
  TCC ne l'écrit pas quand PPD est actif ; il se contente de lire les choix.
  Toute dérogation est une option explicite, loguée, et doit être documentée
  dans l'UI.
- Un TDP hors `[min,max]` est refusé par `tcc-hw` avant l'ioctl.
- Uniwill expose 0 à 3 TDP selon le firmware ; `tdp_count()` sonde
  `R_UW_TDP*_MIN` comme l'original.

### 2.8 NVIDIA cTGP et PRIME

| Fonction | Mécanisme |
|---|---|
| Offset cTGP | `/sys/devices/platform/tuxedo_nvidia_power_ctrl/ctgp_offset` (R/W) |
| Limites NVIDIA | `nvidia-smi --format=csv,noheader,nounits --query-gpu=power.default_limit` et `power.max_limit` (allowlist, sans shell) |
| PRIME dispo. | `/usr/bin/prime-select` (allowlist) |
| PRIME offload | `/var/lib/ubuntu-drivers-common/requires_offloading` (R) |
| eDP sur dGPU | `/sys/class/drm/*eDP*/status` + `/sys/class/drm/*eDP*/device/device/vendor` (`0x10de`) |

- `ctgp_offset` est appliqué seulement si le fichier existe **et**
  `nvidia-smi` est présent (comportement original).
- Quirk IBM15A10 : PRIME supporté seulement si l'écran interne n'est pas câblé
  au dGPU. La détection se fait par connecteur/EDID (voir `PrimeWorker`).
- `prime-select` n'est jamais lancé sans confirmation utilisateur, via polkit.
- Le daemon ne redémarre pas le serveur X/Wayland : il expose l'état et
  l'utilisateur applique le changement au prochain logout.

### 2.9 YCbCr 4:2:0

| Fonction | Chemin |
|---|---|
| Forçage par port | `/sys/kernel/debug/dri/<card>/<port>/force_yuv420_output` (`0`/`1`) |

- `debugfs` doit être monté (`/sys/kernel/debug`). Si absent, la fonction est
  `unsupported`, pas une erreur fatale.
- L'original écrivait avec `appendFileSync` une valeur `0`/`1` sur un fichier
  qui peut être en lecture seule ; le daemon doit vérifier l'écriture et
  journaliser l'échec.
- Le changement de YCbCr peut nécessiter une réapplication du mode DisplayPort
  par le compositeur (`ModeReapplyPending` dans l'original). Le daemon expose
  cet état à l'UI ; le backend d'affichage de `tcc-user` peut réappliquer le
  mode courant.

### 2.10 Fn-Lock

| Fonction | Chemin |
|---|---|
| État | `/sys/devices/platform/tuxedo_keyboard/fn_lock` (`0`/`1`, R/W) |

- Absent ⇒ non supporté. Aucune autre source.

### 2.11 DMI et quirks

| Champ | Chemin |
|---|---|
| SKU | `/sys/class/dmi/id/product_sku` |
| Produit | `/sys/class/dmi/id/product_name` |
| Carte | `/sys/class/dmi/id/board_name`, `board_vendor` |
| Châssis | `/sys/class/dmi/id/chassis_vendor` |
| Système | `/sys/class/dmi/id/sys_vendor` |

- `is_tuxedo()` teste `sys_vendor`, `board_vendor`, `chassis_vendor`
  (insensible à la casse). Ce n'est **pas** un prérequis de démarrage.
- Le modèle Uniwill est lu via `R_UW_MODEL_ID` ; le SKU DMI sert aux quirks
  TUXEDO (`product_sku`).
- Les quirks device (EPP non supporté, platform_profile absent, Sirius, IBP,
  etc.) appartiennent à `tcc-core` ; `tcc-hw` fournit les faits bruts.

### 2.12 `/dev/tuxedo_io` — ioctl

Source : `src/native-lib/tuxedo_io_lib/tuxedo_io_ioctl.h` (GPL-3.0-or-later,
TUXEDO Computers GmbH). Encodage Linux :

```text
_IOC(dir, type, nr, size) = (dir << 30) | (size << 16) | (type << 8) | nr
_IO(type, nr)   = dir 0, size 0
_IOW(type, nr)  = dir 1, size = sizeof(argument)
_IOR(type, nr)  = dir 2, size = sizeof(argument)
```

**Piège important** : le header passe des types pointeur à `_IOR`/`_IOW`
(`int32_t*`, `char*`). La taille encodée est donc `sizeof(pointeur)` (8 sur
x86_64), pas `sizeof(int32_t)`. Les valeurs ci-dessous sont vérifiées par
compilation du header C sur x86_64.

| Nom | Valeur (x86_64) | Sens | Argument |
|---|---|---|---|
| `R_MOD_VERSION` | `0x8008EC00` | R | `char*`, 20 octets |
| `R_HWCHECK_CL` | `0x8008EC05` | R | `int32*`, 1 = Clevo |
| `R_HWCHECK_UW` | `0x8008EC06` | R | `int32*`, 1 = Uniwill |
| `R_CL_HW_IF_STR` | `0x8008ED00` | R | `char*` |
| `R_CL_FANINFO1/2/3` | `0x8008ED10/11/12` | R | `int32*` (vitesse + 2 temp.) |
| `R_CL_WEBCAM_SW` | `0x8008ED13` | R | `int32*` |
| `R_CL_FLIGHTMODE_SW` | `0x8008ED14` | R | `int32*` |
| `R_CL_TOUCHPAD_SW` | `0x8008ED15` | R | `int32*` |
| `W_CL_FANSPEED` | `0x4008EE10` | W | `int32` empaqueté 3×8 bits |
| `W_CL_FANAUTO` | `0x4008EE11` | W | `int32` masque |
| `W_CL_WEBCAM_SW` | `0x4008EE12` | W | `int32` 0/1 |
| `W_CL_FLIGHTMODE_SW` | `0x4008EE13` | W | `int32` 0/1 |
| `W_CL_TOUCHPAD_SW` | `0x4008EE14` | W | `int32` 0/1 |
| `W_CL_PERF_PROFILE` | `0x4008EE15` | W | `int32` |
| `R_UW_HW_IF_STR` | `0x8008EF00` | R | `char*` |
| `R_UW_MODEL_ID` | `0x8008EF01` | R | `int32*` |
| `R_UW_FANSPEED/2` | `0x8008EF10/11` | R | `int32*` (0..200) |
| `R_UW_FAN_TEMP/2` | `0x8008EF12/13` | R | `int32*` (°C) |
| `R_UW_MODE` | `0x8008EF14` | R | `int32*` |
| `R_UW_MODE_ENABLE` | `0x8008EF15` | R | `int32*` |
| `R_UW_FANS_OFF_AVAILABLE` | `0x8008EF16` | R | `int32*` |
| `R_UW_FANS_MIN_SPEED` | `0x8008EF17` | R | `int32*` (%) |
| `R_UW_TDP0/1/2` | `0x8008EF18/19/1A` | R | `int32*` (W) |
| `R_UW_TDP0/1/2_MIN` | `0x8008EF1B/1C/1D` | R | `int32*` (W) |
| `R_UW_TDP0/1/2_MAX` | `0x8008EF1E/1F/20` | R | `int32*` (W) |
| `R_UW_PROFS_AVAILABLE` | `0x8008EF21` | R | `int32*` |
| `W_UW_FANSPEED/2` | `0x4008F010/11` | W | `int32` (0..200) |
| `W_UW_MODE` | `0x4008F012` | W | `int32` |
| `W_UW_MODE_ENABLE` | `0x4008F013` | W | `int32` 0/1 |
| `W_UW_FANAUTO` | `0x0000F014` | W | aucun (`_IO`) |
| `W_UW_TDP0/1/2` | `0x4008F015/16/17` | W | `int32` (W) |
| `W_UW_PERF_PROF` | `0x4008F018` | W | `int32` |

Sémantique :

- **Clevo** : 3 ventilateurs. `R_CL_FANINFO_n` = bits 0-7 vitesse brute
  (0..255), bits 8-15 temp1, bits 16-23 temp2. L'original utilise **temp2** ;
  une température ≤ 1 signifie ventilateur absent. `W_CL_FANSPEED` contient les
  trois vitesses empaquetées ; il faut relire les autres avant d'écrire.
  `W_CL_FANAUTO` = `0b1111` (bits 0..3). `fans_min_speed` = 20 %,
  `fans_off_available` = vrai (constantes).
- **Uniwill** : 2 ventilateurs, échelle 0..200 (`0xC8`). Un TDP à 0 en lecture
  de température signifie capteur absent. `GetNumberTDPs` sonde les minima ;
  `W_UW_FANAUTO` est un `_IO` sans argument.
- **Version** : `MOD_API_MIN_VERSION = "0.2.6"`. Une version plus ancienne
  produit un log d'avertissement, pas un refus d'ouverture : les fonctions
  utilisées sont stables. L'UI doit afficher la version et l'état de
  compatibilité.
- **Sûreté** : tous les index (`fan < fan_count`, `tdp < 3`), pourcentages
  (0..100) et TDP (`min..=max`) sont validés avant l'ioctl. Le fichier est
  ouvert `O_RDWR|O_NOFOLLOW|O_CLOEXEC`.
- **Arrêt** : `set_fans_auto()` puis `set_mode_enable(false)` avant toute
  sortie (l'original le fait dans `FanControlTuxedoIO.exit`) ; `shutdown()`
  encapsule ce best-effort.

---

## 3. D-Bus système et session

### 3.1 power-profiles-daemon (PPD)

| Élément | Valeur |
|---|---|
| Bus | système |
| Nom | `net.hadess.PowerProfiles` |
| Objet | `/net/hadess/PowerProfiles` |
| Interface | `net.hadess.PowerProfiles` |
| Propriété | `ActiveProfile` type `s` (`power-saver`, `balanced`, `performance`) |
| Propriété | `Profiles` type `aa{sv}` (métadonnées, `Driver`, `PlatformDriver`) |
| Propriété | `PerformanceDegraded` type `s` (`reason` ou vide) |
| Méthode | `HoldProfile(s profile, s reason, s application_id) -> u cookie` |
| Méthode | `ReleaseProfile(u cookie)` |
| Signal | `org.freedesktop.DBus.Properties.PropertiesChanged` |

- Lecture initiale par `org.freedesktop.DBus.Properties.Get`, puis abonnement
  à `PropertiesChanged` sur l'interface `net.hadess.PowerProfiles`.
- PPD peut être absent (machines sans PPD, conteneurs) : fallback AC/BAT pur.
- Valeurs inconnues : conserver la chaîne brute, loguer une fois, utiliser le
  fallback.
- **Ne pas** appeler `HoldProfile` par défaut : cela empêcherait PPD de changer
  de profil et créerait un conflit. Utilisable seulement pour une action
  ponctuelle explicite.
- Ne pas écrire `platform_profile`/EPP quand PPD est actif (politique par
  défaut, §4).

### 3.2 UPower et logind

| Élément | Valeur |
|---|---|
| Bus | système |
| Nom | `org.freedesktop.UPower` |
| Objet | `/org/freedesktop/UPower` |
| Interface | `org.freedesktop.UPower` |
| Propriété | `OnBattery` type `b` |
| Propriété | `LidIsClosed` type `b` |
| Signal | `DeviceAdded(o)`, `DeviceRemoved(o)` |
| Objet batterie | `/org/freedesktop/UPower/devices/battery_BAT0` |
| Interface batterie | `org.freedesktop.UPower.Device` |
| Propriétés | `State` (u), `Percentage` (d), `IsPresent` (b), `TimeToEmpty` (x), `WarningLevel` (u) |
| Signal | `PropertiesChanged` |

`State` UPower : `1` charging, `2` discharging, `3` empty, `4` fully charged,
`5` pending charge, `6` pending discharge. Conversion vers
`BatteryStatus`/`PowerState`.

- UPower est prioritaire pour les événements et l'état AC/BAT. En cas
  d'absence ou d'erreur, `SysfsPowerStateReader` (voir §2.4) prend le relais.
- `org.freedesktop.login1` (`/org/freedesktop/login1`,
  `org.freedesktop.login1.Manager`, signal `PrepareForSleep(b)`) est utilisé
  pour suspendre/reprendre proprement les workers. Ce n'est pas un substitut à
  UPower pour l'état AC.
- Sur batterie, `TimeToEmpty` peut valoir 0 : ne pas l'interpréter comme une
  mesure valide.

### 3.3 API du daemon TCC

| Élément | Valeur |
|---|---|
| Bus | système |
| Nom | `com.tuxedocomputers.tccd` (compatibilité) |
| Objet | `/com/tuxedocomputers/tccd` |
| Interface compat. | `com.tuxedocomputers.tccd` (méthodes `Get*JSON`, setters) |
| Interface typée | `com.tuxedocomputers.tccd.Manager1` |
| Signaux | `ActiveProfileChanged`, `StateChanged`, `SensorDataChanged`, `ModeReapplyPendingChanged`, `DisplayModeApplyRequested` |

- L'API JSON historique est une **couche de compatibilité en lecture seule**
  (D3) : taille bornée, schéma figé, aucune nouvelle méthode non typée. Les
  seuls setters de compatibilité sont `SetTempProfile`/`ClearTempProfile` :
  globaux, bornés en taille, rate-limités par nom unique et audités, mais
  **pas encore protégés par polkit** (R-03). La source de vérité reste
  `Manager1` avec des types D-Bus natifs ; le déplacement de l'override
  temporaire vers `Manager1` est planifié.
- `Manager1` est implémentée sur le même objet : instantané matériel JSON,
  collecte capteurs à la demande, setters non persistants charge/lighting/
  Fn-Lock (validés, rate-limités, audités et autorisés par polkit :
  `…tccd1.charging` / `…tccd1.settings`). TDP/ODM/cTGP/PRIME/YCbCr restent en
  lecture seule dans le snapshot ; leurs écritures sont réservées à
  `tccd-admin` + polkit.
- `Control1` (`com.tuxedocomputers.tccd.Control1`) est servie sur le même objet
  et réserve `ReloadConfig` à root (policy D-Bus + revérification
  `GetConnectionUnixUser == 0`), sérialisé entre appels concurrents sans
  cache de rapport, et renvoie un `AdminReloadReport` JSON frais.
- `tccd` possède `com.tuxedocomputers.tccd` ; `tccd-admin` possède
  `com.tuxedocomputers.tccd.Admin` et n'est activé qu'à la demande (D1).
  Phase 1 : `Admin1` (`Ping`, `GetVersion`, `SaveProfilesJSON`,
  `SaveSettingsJSON`, `ReloadConfig`) avec **polkit réel** pour chaque
  écriture. La policy D-Bus liste les membres un par un et reste fail-closed.
- Un bug de parsing dans `tccd` ne doit jamais donner accès aux opérations
  Prime/Tomte/WebFAI : elles vivent dans des services admin distincts, avec
  vérification polkit méthode par méthode.
- **LUKS est hors périmètre (décision utilisateur)** : aucun service, aucune
  action polkit, aucune procédure de changement de phrase de passe.
- Les types exacts sont dans `tcc-ipc` (hors périmètre de ce document).

### 3.4 Bus de session (compositeurs)

Voir §5. `tcc-user` parle au compositeur sur le bus de session ou via les
sockets IPC natifs ; il ne publie pas de service système.

---

## 4. Bascule automatique et relation avec PPD

Entrées : `(ppd: Option<PpdProfile>, power: PowerState)` + override manuel.

1. Clé `(ppd, power)` ; matrice configurable `state_matrix`.
2. Fallback `stateMap.power_ac` / `stateMap.power_bat` (compatibilité).
3. Fallback profil par défaut.
4. Debounce 500–1000 ms.
5. Tout changement de PPD **ou** d'alimentation invalide l'override manuel
   temporaire.
6. PPD absent ⇒ clé `(None, power)` ⇒ fallback AC/BAT.
7. PPD inconnu ⇒ fallback AC/BAT et log unique.

**Politique de propriété (défaut recommandé)** :

| Réglage | Propriétaire par défaut |
|---|---|
| `platform_profile` (ACPI/TUXEDO) | PPD s'il est actif, sinon TCC |
| EPP (`energy_performance_preference`) | PPD s'il est actif, sinon TCC |
| Limites CPU (`scaling_min/max`, governor, `no_turbo`, cœurs) | TCC |
| Ventilateurs | TCC |
| Luminosité / refresh rate | TCC |
| Webcam | TCC |
| Charge | TCC |
| TDP / cTGP | TCC |
| YCbCr / Fn-Lock | TCC |

Toute dérogation (« TCC prioritaire sur PPD ») est un booléen explicite dans
les settings, logué à chaque application, et affiché dans l'UI. Sans cette
option, `tcc-daemon` ne doit jamais écrire `platform_profile` ni l'EPP quand
PPD est présent.

---

## 5. Refresh rate de l'écran interne

### 5.1 Contrat

- Cible **exclusive** : connecteur dont le nom commence par `eDP`, `LVDS` ou
  `DSI` (insensible à la casse). Les écrans externes ne sont jamais modifiés.
- Résolution **optionnelle** : si le profil ne la demande pas
  (`useResolution=false`), elle est conservée et seul le taux change. Si elle
  est demandée, le mode `largeur × hauteur × taux` doit exister exactement.
- Si le mode courant est déjà résolution + taux (tolérance 0,5 Hz), aucune
  écriture.
- Si le mode demandé n'existe pas (résolution ou taux) : erreur typée
  `DisplayModeNotFound`, pas de fallback arbitraire.
- `width`/`height` `None`/`0` ⇒ résolution courante ; `refresh_hz` `None` ⇒
  taux courant (résolution seule).
- Si aucun backend n'est disponible : état `unsupported`, pas d'erreur fatale.
- `tcc-user` sérialise les demandes par output et coalesce les demandes
  rapprochées (une seule application par rafale).
- **D6** : `tccd` ne lit jamais `/proc/*/environ`. `tccd-user` publie via D-Bus
  typé `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP`
  et l'identité de session que le daemon doit connaître. L'agent utilise
  uniquement son propre environnement pour parler au compositeur.

Le contrat Rust correspondant est `tcc_hw::display` :
`OutputInfo`, `DisplayMode`, `InternalOutput`, `DisplayModeBackend`,
`DisplayModePlan`, `ApplyOutcome`, `plan_internal_display_mode`,
`apply_internal_display_mode`, `apply_internal_refresh_rate` (compatibilité
taux seul).

### 5.2 X11 (XRandR natif, `x11rb`)

1. Ouvrir la connexion X11 (`DISPLAY` de la session de l'agent, `XAUTHORITY`
   déjà présent dans l'environnement du service utilisateur).
2. `RandR::query_version(1, 6)` puis `GetScreenResourcesCurrent` sur la racine.
3. Pour chaque output : `GetOutputInfo` (nom, connexion, CRTC) et
   `GetCrtcInfo` (mode courant, position, rotation).
4. Filtrer les noms internes. Récupérer les modes via `GetScreenResourcesCurrent`
   (`mode_info` : largeur, hauteur, `dot_clock`, `htotal`, `vtotal`), calculer
   `refresh = dot_clock / (htotal * vtotal)`.
5. Choisir le mode à la résolution demandée (ou courante si `useResolution`
   est faux) et au taux cible dans la tolérance.
6. `SetCrtcConfig(crtc, mode, Output::None? non: [output], x, y, rotation)`
   en conservant position/rotation. Un `BadMatch` signifie que le mode n'est
   pas valide pour cet output : échec propre, pas de retry en boucle.
7. Ne jamais toucher aux CRTC des autres outputs ; refuser un CRTC partagé en
   miroir.
8. Si le serveur X n'est pas accessible (session verrouillée, mauvais
   `XAUTHORITY`), retourner `Unsupported`/erreur et réessayer au prochain
   changement de profil ou au prochain signal de session.

Différence avec l'original : plus de scraping de `/proc/*/environ`, de `w` ni
de `xrandr`. L'agent tourne dans la session et hérite des variables correctes.

### 5.3 Hyprland (socket IPC natif)

- Chemin : `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`.
- Lecture : commande `j/monitors` (JSON). Champs utiles : `name`,
  `disabled`, `width`, `height`, `refreshRate`, `availableModes`
  (`WxH@R`).
- Écriture : `dispatch keyword monitor <nom>,<W>x<H>@<R>,<x>x<y>,<scale>`.
  Cette commande applique immédiatement la règle. Ne pas utiliser
  `hyprctl` (binaire externe) : le socket est stable et sans shell.
- Refuser tout nom qui n'est pas un connecteur interne, même si le compositeur
  l'expose.

### 5.4 GNOME (Mutter DisplayConfig)

- Bus de session, nom `org.gnome.Mutter.DisplayConfig`, objet
  `/org/gnome/Mutter/DisplayConfig`, interface `org.gnome.Mutter.DisplayConfig`.
- `GetCurrentState() -> (u serial, a((ssss)a(siiddada{sv})a{sv}) monitors, a(iiduba(ssss)a{sv}) logical_monitors, a{sv} properties)`.
- Filtrer `monitors` sur le connecteur (`ssss` = connector, vendor, product,
  serial). Le connecteur `eDP-1`/`LVDS-1`/`DSI-1` est le critère.
- Choisir le mode `(siiddada{sv})` à la résolution demandée (ou courante) et
  au taux cible (le champ `d` est la fréquence). Le mode choisi doit être
  présent dans la liste Mutter.
- `ApplyMonitorsConfig(u serial, u method, a(iiduba(ssss)a{sv}) logical_monitors, a{sv} properties)`.
  - `method = 1` (temporaire) pour un changement de profil ; `2` (persistant)
    est réservé à une action utilisateur explicite.
  - Le `serial` doit être celui de `GetCurrentState` ; en cas de
    `AccessDenied`/serial périmé, relire l'état et échouer proprement.
- Conserver les moniteurs logiques existants ; ne modifier que le mode du
  moniteur qui contient l'output interne (résolution et/ou taux).

### 5.5 KDE (KScreen)

- D-Bus : `org.kde.KScreen`, objet `/org/kde/kscreen`, interface
  `org.kde.KScreen` (`getConfig`, `applyConfig`). L'API est peu stable et
  dépend de la version de Plasma.
- Fallback documenté : `/usr/bin/kscreen-doctor` (allowlist, arguments
  séparés, aucun shell) :
  - lecture : `kscreen-doctor -j` (JSON) ;
  - écriture : `kscreen-doctor output.eDP-1.mode.2560x1440@165`.
- Ne jamais utiliser `sh -c`. Valider le nom de sortie et les nombres avant
  l'appel.
- Si ni D-Bus ni `kscreen-doctor` ne fonctionne : `unsupported`.

### 5.6 Sway

- Socket : `$SWAYSOCK`.
- Protocole i3-ipc : en-tête `"i3-ipc"` + `u32 length` + `u32 type`,
  natif little-endian.
- Lecture : `GET_OUTPUTS` (type 3, réponse JSON) ; filtrer `name` interne et
  `current_mode`/`modes`.
- Écriture : `RUN_COMMAND` (type 0) avec
  `output <nom> mode <W>x<H>@<R>Hz` (ou `mode --rate <R>` si la résolution est
  déjà correcte). Vérifier la réponse `success`.
- Ne pas toucher aux autres outputs.

### 5.7 wlroots générique

- Protocole `zwlr_output_manager_v1` via `wayland-client`.
- `zwlr_output_manager_v1.head` → `zwlr_output_head_v1` :
  `name`, `description`, `enabled`, `current_mode`, `modes`.
- Chaque `zwlr_output_mode_v1` fournit `size(w,h)` et `refresh(mHz)`.
- Construire une configuration avec
  `zwlr_output_manager_v1.create_configuration(serial)`, activer uniquement le
  head interne avec le mode choisi, conserver les autres heads dans leur état
  courant, puis `apply()` / `test()`.
- Gérer `finished`, `cancelled`, `failed` sans bloquer le daemon.

### 5.8 Détection et sélection

| Situation | Décision |
|---|---|
| Sessions TTY | `unsupported` |
| Wayland non supporté | `unsupported`, pas de fallback XRandR |
| X11 | XRandR natif uniquement |
| Hyprland | socket IPC |
| GNOME | Mutter DisplayConfig |
| KDE | D-Bus KScreen, fallback `kscreen-doctor` |
| Sway | IPC Sway |
| Autre wlroots | `zwlr_output_manager_v1` |
| Aucun backend | `unsupported` |

La sélection de l'écran se fait **par nom de connecteur**, jamais par index 0 :
un laptop peut avoir un écran externe branché, et le panneau interne peut être
relié à l'iGPU ou au dGPU.

---

## 6. systemd, D-Bus, polkit, udev, tmpfiles

### 6.1 Unités

| Unité | Type | Rôle |
|---|---|---|
| `tccd.service` (système) | `Type=dbus`, `BusName=com.tuxedocomputers.tccd` | Daemon privilégié |
| `tccd-admin.service` (système) | `Type=dbus`, activation à la demande | PRIME, Tomte, WebFAI (D1) |
| `tccd-admin-hw.service` (système, phase 2) | `Type=dbus`, activation à la demande | TDP/ODM/cTGP (`Admin.Hw`) |
| `tccd-user.service` (utilisateur) | `Type=simple` | Agent de session, refresh rate |

`tccd.service` :

- `After=dbus.service`, `Wants=power-profiles-daemon.service` (si présent),
  **sans** `After=power-profiles-daemon.service` : PPD est lui-même
  `After=multi-user.target` et `tccd` est `WantedBy=multi-user.target` ; un
  ordre explicite crée un cycle et systemd supprime le démarrage de `tccd`.
- `Restart=on-failure`, `RestartSec=1`, `TimeoutStopSec=10`.
- `StateDirectory=tuxedo-rust-center`, `RuntimeDirectory=tuxedo-rust-center`,
  `ConfigurationDirectory=tuxedo-rust-center` (création seule, jamais écrite
  par le daemon).
- Hardening : `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=yes`,
  `PrivateTmp=yes`, `ProtectProc=invisible`, `ProtectControlGroups`,
  `ProtectKernelModules`, `ProtectClock`, `ProtectHostname`,
  `RestrictNamespaces`, `RestrictRealtime`, `RestrictSUIDSGID`,
  `LockPersonality`, `MemoryDenyWriteExecute`, `SecureBits=noroot-locked`,
  `LimitCORE=0`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK`.
- Exceptions indispensables, justifiées :
  - `ProtectKernelTunables=no` : écritures sysfs (cpufreq, backlight, LED,
    charge) ; décision bloquante ;
  - `PrivateDevices=no` + `DevicePolicy=closed` +
    `DeviceAllow=/dev/tuxedo_io rw` : le nœud caractère n'est pas recréé par
    un `/dev` privé sur toutes les versions de systemd (à valider sur matériel
    réel, repli documenté) ;
  - `ReadWritePaths=/var/lib/tuxedo-rust-center /run/tuxedo-rust-center` (D2),
    **pas** `/etc` ;
  - `CapabilityBoundingSet=` vide par défaut (root propriétaire des fichiers
    sysfs). Si un pilote expose un nœud non lisible par root sans
    `CAP_DAC_OVERRIDE`, l'ajouter explicitement et documenter.
- `SystemCallFilter=@system-service` est **à valider** avec
  `systemd-analyze syscall-filter` : `ioctl` doit rester autorisé. En cas de
  doute, ne pas activer le filtre plutôt que de casser le matériel.
- **Perte du nom D-Bus (R-20)** : `tccd` surveille sa connexion au bus
  (`GetId` toutes les 2 s) et sort en erreur après deux échecs consécutifs.
  `Restart=on-failure` le redémarre. `StartLimitIntervalSec=60` et
  `StartLimitBurst=5` sont recommandés dans l'unité (fichier possédé par
  `packaging-release`) pour éviter une boucle de redémarrage en cas de bus
  durablement indisponible.

`tccd-admin.service` (phase 1 : configuration uniquement) :

- Aucun `[Install]` : activation D-Bus à la demande via
  `/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service`.
- `ProtectSystem=strict`, `ReadWritePaths=/var/lib/tuxedo-rust-center`,
  `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`,
  `ProtectProc=invisible`, `ProtectKernelTunables=yes`,
  `RestrictAddressFamilies=AF_UNIX`, `SecureBits=noroot-locked`,
  `CapabilityBoundingSet=` vide, `LimitCORE=0`, `UMask=0077`,
  `Restart=no`, `RuntimeMaxSec=5min`.
- Phase 1 : chaque méthode d'écriture vérifie polkit (`profiles`,
  `settings`) et le processus sort après 30 s d'inactivité. `Ping`/`GetVersion`
  restent sans polkit.
- Runtime Tokio **borné** : 2 workers async + au plus 4 threads bloquants
  (`spawn_blocking` pour les transactions disque), soit 7 threads avec le
  thread principal. Le nombre de cœurs n'est jamais utilisé. `TasksMax=16`
  laisse de la marge ; des garde-fous `const` dans `main.rs` refusent tout
  dimensionnement qui dépasserait la limite de l'unité.

`tccd-user.service` :

- `PartOf=graphical-session.target`, `After=graphical-session.target`,
  `ConditionEnvironment=WAYLAND_DISPLAY` **non** : le service doit aussi
  tourner sous X11.
- `Restart=on-failure`, `NoNewPrivileges=yes`, `ProtectSystem=strict`,
  `ProtectHome=read-only`, `RestrictAddressFamilies=AF_UNIX`,
  `MemoryDenyWriteExecute`.
- Pas de `PrivateTmp=yes` : le socket X11 `/tmp/.X11-unix` doit rester visible
  au backend XRandR. Wayland utilise `$XDG_RUNTIME_DIR`.
- Pas de `PrivateDevices=yes` : l'agent n'ouvre aucun périphérique et certaines
  configurations interdisent les namespaces non privilégiés.
- Aucun accès `/dev/tuxedo_io`, aucune écriture `/etc`.

### 6.1.1 Mode `--dry-run` root (validation manuelle)

Le daemon accepte `--dry-run`, avec ou sans `--start`. Le dry-run effectif vaut
`--check || --dev-session || --dry-run` :

- `--check` : charge la configuration, affiche l'état et quitte (comportement
  inchangé) ;
- `--dev-session` : bus de session, sans root, authorizer permissif ;
- `--dry-run` : **bus système**, **nom D-Bus normal**, **root requis**, aucune
  écriture matérielle.

`--dry-run` ne force donc pas le bus de session et ne quitte pas après
l'affichage. Il sert à valider le daemon face au bus système et aux tools
(`tcc-cli`, `tccd-user`, admin séparé) sans toucher au matériel.

Écritures supprimées en dry-run :

- CPU (governor, EPP, fréquences, cœurs, turbo/boost) ;
- rétroéclairage ;
- ventilateurs, y compris le retour auto à la veille/arrêt ;
- charge (profil, priorité, seuils, type) ;
- éclairage clavier/lightbar ;
- ODM/`platform_profile`/TDP et NVIDIA cTGP ;
- webcam (kill switch et contrôles UVC) ;
- YCbCr 4:2:0 ;
- setters `Manager1` : succès **simulé** (`true`), état en mémoire mis à jour,
  aucune écriture sysfs/ioctl et aucune persistance.

Les lectures, le bail de capteurs, `GetShutdownTimerJSON`,
`Control1.ReloadConfig` et `tccd-admin` (lancé à part) continuent de
fonctionner. Les workers matériels publient leurs capacités et télémétries
réelles ; seules les écritures sont neutralisées.

Limites assumées : `--dry-run` ne désactive pas la migration/persistance de
configuration sous `/var/lib` (comportement normal du daemon), ni l'inhibiteur
de veille logind, ni la minuterie d'extinction logind si `shutdownTime` est
configuré. Pour une validation totalement passive, utiliser une configuration
sans `shutdownTime` et un état déjà migré.

Procédure de validation manuelle (l'ancien TCC doit être arrêté pour libérer le
nom D-Bus) :

```sh
sudo systemctl stop tccd.service          # ancien TCC (/opt)
cargo build --release -p tcc-daemon
sudo ./target/release/tcc-daemon --dry-run --start

# autre terminal : lectures et policy
tcc-cli status
busctl --system introspect com.tuxedocomputers.tccd \
  /com/tuxedocomputers/tccd

# Ctrl-C, puis restauration de l'ancien service
sudo systemctl start tccd.service
```

Les unités systemd n'utilisent pas `--dry-run` : c'est uniquement un outil de
validation manuelle.

### 6.1.2 Service `Admin.Hw` phase 2 (TDP / ODM / cTGP)

Service root séparé (AD-12 B2), activé à la demande :

| Élément | Valeur |
|---|---|
| Service D-Bus | `com.tuxedocomputers.tccd.Admin.Hw` |
| Objet | `/com/tuxedocomputers/tccd/Admin/Hw` |
| Interface / contrat | `com.tuxedocomputers.tccd.AdminHw1` / `1` |
| Binaire | `/usr/libexec/tuxedo-rust-center/tccd-admin-hw` |
| Unité | `tccd-admin-hw.service` (fournie, non installée par défaut) |

Le binaire partage le runtime borné de `tcc-admin` (2 workers async + 4
threads bloquants) et sort après 30 s sans appel. Il exige root.

Unit attendue par `packaging-release` (design sécurité §4.4) :
`Type=dbus`, `BusName=com.tuxedocomputers.tccd.Admin.Hw`,
`RuntimeMaxSec=5min`, `TasksMax=16`, `MemoryMax=256M` (recommandé ; 128M est insuffisant face à une trame zbus de 128 Mio), `UMask=0077`,
`PrivateDevices=no` + `DevicePolicy=closed` +
`DeviceAllow=/dev/tuxedo_io rw`, `ProtectKernelTunables=no` compensé par
`ReadOnlyPaths=/proc/sys /sys/kernel /sys/module /sys/devices/system/cpu
/sys/class/{leds,backlight,power_supply,hwmon,thermal,drm}`,
`CapabilityBoundingSet=` vide, `NoNewPrivileges=yes`, `RestrictAddressFamilies=AF_UNIX`,
`SystemCallFilter=@system-service`, `StartLimitBurst=20`. Aucun `execve`
n'est nécessaire : le service n'exécute ni `nvidia-smi`, ni `prime-select`,
ni shell. `ReadWritePaths` n'est pas requis (aucune écriture fichier).

Backends et chemins :

- TDP : `TuxedoIo::open()` **paresseux** dans la méthode, puis
  `TdpControl::limits/set_values` (`/dev/tuxedo_io`, `R_UW_TDP*`/`W_UW_TDP*`).
  Un tableau vide applique les maxima firmware (consigne explicite, audit
  `values=empty, maxima=true`) ; un profil `tdpValues=[]` n'appelle pas le
  setter et ne change rien ;
- ODM : `PlatformProfile::detect/apply` (`tuxedo_platform_profile` ou ACPI)
  et fallback `TuxedoIo::odm_profiles/set_odm_profile` ;
- cTGP : `CtgpControl::offset/set_offset_watts`
  (`tuxedo_nvidia_power_ctrl/ctgp_offset`), sans `nvidia-smi`.

Le service ne persiste rien et ne lit ni n'écrit `/etc`. `SetOdmProfileOverride`
lit `Settings.ppd_override` dans `/var/lib` à chaque appel (lecture stricte,
fail-closed), **avant polkit** pour éviter un dialogue inutile. `tccd` lit le
même flag dans ses deux constructions de `HardwareInputs` et journalise
`ppd_override=true : TCC peut écrire platform_profile malgré
power-profiles-daemon`. Les écritures sont sérialisées (`Semaphore(1)`),
rate-limitées 2/s burst 4, vérifiées après coup et rollbackées best-effort en
cas d'échec. Les lectures `GetTdpValues` sont limitées à 2/s burst 5, sans
polkit.

- **Taille D-Bus (AH-01)** : le corps est refusé au-delà de 64 Kio avant
  désérialisation (`InvalidArgs("message.size")`) et la file de connexion est
  bornée à 8 messages (`max_queued(8)`). zbus 5 fige la trame socket
  à 128 Mio et ne l'expose pas ; `MemoryMax=128M` peut être atteint par une
  trame maximale. `packaging-release` doit relever `MemoryMax` à 256M (ou
  documenter le redémarrage). Aucune écriture matérielle dans ce cas.
- **ODM / PPD (AH-02)** : le fallback `tuxedo_io` est **autorisé** avec PPD
  `Owned` ou `Unknown` ; PPD ne possède que `platform_profile`, pas les
  profils ODM du module. `platform_profile` reste refusé si PPD est possédé,
  sauf `SetOdmProfileOverride` consenti. L'audit porte `source=io` et
  `ppd=owned|unknown`.
- **Rollback ODM (AH-04)** : si la relecture de `platform_profile` diffère de
  la cible, l'ancien profil est réappliqué ; `WriteFailed` si restauré,
  `PartialApply("platform_profile")` sinon.
- **cTGP (AH-05)** : borne dynamique lue via
  `Manager1.GetHardwareSnapshotJSON` (timeout 2 s, JSON ≤ 4 Mio), repli
  statique 200 W (`bound=static`). Aucun `nvidia-smi`.

Le service `Admin.Prime` et `SetPrimeMode` ne font pas partie de cette
implémentation. La policy D-Bus et le fichier d'activation
`com.tuxedocomputers.tccd.Admin.Hw.service` restent à créer par
`packaging-release`.

### 6.2 D-Bus

- Policy : `/usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf`.
- Les noms `com.tuxedocomputers.tccd` et `com.tuxedocomputers.tccd.Admin`
  sont possédés par root uniquement.
- `Admin1` est joignable par tout utilisateur local, mais **chaque membre est
  listé** dans la policy ; toute méthode non listée est injoignable. Le bus ne
  fait pas d'autorisation par utilisateur : polkit tranche dans `tccd-admin`
  (phase 1 implémentée pour `profiles`/`settings`).
- `Control1` est réservé à `<policy user="root">`, avec un `deny` explicite
  dans `context="default"` ; `tccd` re-vérifie l'UID 0.
- `SetTempProfile`/`ClearTempProfile` sont globaux, limités à 5 écritures/s
  (burst 10) par nom unique D-Bus, avec audit `caller`, `caller_uid` et
  `profile_id` (sans payload brut). Voir `docs/DBUS_API.md`.
- `tccd-admin` est activé à la demande par
  `/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service`
  (`SystemdService=tccd-admin.service`), jamais au boot.
- Aucune interface n'expose d'écriture de fichier arbitraire.
- Le rate limit couvre aussi `ClearTempProfile` ; étendre la même politique aux
  futurs setters matériels.
- Les setters `Manager1` utilisent un authorizer polkit injectable. Le seul
  authorizer permissif (`DevSessionAuthorizer`) est construit par
  `HardwareBridge::dev_session`, qui force `dry_run` ; la production passe par
  `HardwareBridge::production` sans authorizer. Un `debug_assert` et un test
  source interdisent l'injection hors `--dev-session`.

### 6.3 polkit

Phase 1 réellement appliquée par `tcc-authz` :

| Action | Défaut | Usage |
|---|---|---|
| `com.tuxedocomputers.tccd1.profiles` | `auth_admin` (sans cache) | `SaveProfilesJSON` |
| `com.tuxedocomputers.tccd1.settings` | `auth_admin` | `SaveSettingsJSON`, `ReloadConfig` |

Actions prévues pour les phases suivantes, non encore appelées :
`…tccd1.power-limits` (TDP/ODM/cTGP), `…tccd1.prime-select`,
`…tccd1.tomte-run`, `…tccd1.packages`. Ne pas les présenter comme actives.

Le client `tcc-authz` envoie `CheckAuthorization` avec :

- sujet `("system-bus-name", {"name": <nom unique de l'appelant>})`, jamais un
  PID fourni par le client ;
- `details = {}` et `flags = 1` (`AllowUserInteraction`) ;
- timeout borné (120 s par défaut) ;
- décision `is_authorized && !is_challenge` uniquement ;
- polkit absent/erreur/timeout ⇒ refus, sans cache local ni court-circuit
  root. Un système sans polkit n'a pas d'admin fonctionnel (AD-R2).

Le bucket anti-dialog-storm (2 appels/s, burst 4, par nom unique et par UID) et
le verrou de dialogue unique vivent dans `tcc-authz`, avec des seuils
paramétrés par service (AD-9). `tccd` applique `tcc-authz` aux setters
`Manager1` charge/lighting/Fn-Lock (`…tccd1.charging` / `…tccd1.settings`).
`SetTempProfile`/`ClearTempProfile` (interface historique) restent sans polkit
et sont couverts par le rate limit et la policy, R-03/R-04.

### 6.4 udev

- Règle webcam :
  `SUBSYSTEM=="video4linux", ACTION=="add|change", KERNEL=="video[0-9]*", TAG+="tuxedo-rust-center-webcam"`.
- Aucun `RUN+=` : le daemon écoute le netlink udev et applique les préréglages
  V4L2 lui-même. Cela supprime la dépendance Python et l'exécution de script
  depuis udev.
- Ajouter des règles `uaccess` si nécessaire pour l'aperçu GUI (logind le fait
  déjà pour `video4linux`).

### 6.5 tmpfiles

- `/run/tuxedo-rust-center` (0755 root:root)
- `/var/lib/tuxedo-rust-center` (0755 root:root)
- `/etc/tuxedo-rust-center` (0755 root:root) — créé par le paquet, présent
  aussi dans tmpfiles pour les installations non packagées.

### 6.6 Chemins FHS

| Contenu | Chemin |
|---|---|
| GUI | `/usr/bin/tuxedo-rust-center` |
| Daemon | `/usr/libexec/tuxedo-rust-center/tccd` |
| Admin à la demande | `/usr/libexec/tuxedo-rust-center/tccd-admin` |
| Agent session | `/usr/libexec/tuxedo-rust-center/tccd-user` |
| Unit système | `/usr/lib/systemd/system/tccd.service` |
| Unit admin | `/usr/lib/systemd/system/tccd-admin.service` (activation D-Bus) |
| Unit utilisateur | `/usr/lib/systemd/user/tccd-user.service` |
| Configuration admin (RO daemon) | `/etc/tuxedo-rust-center/` |
| État mutable (profils/settings/fantables/webcam) | `/var/lib/tuxedo-rust-center/*.json` |
| Runtime | `/run/tuxedo-rust-center/` |
| QML/assets | `/usr/share/tuxedo-rust-center/` |
| Desktop | `/usr/share/applications/` |
| Icônes | `/usr/share/icons/hicolor/scalable/apps/` |
| AppStream | `/usr/share/metainfo/` |
| Policy D-Bus | `/usr/share/dbus-1/system.d/` |
| Activation D-Bus admin | `/usr/share/dbus-1/system-services/` |
| Polkit | `/usr/share/polkit-1/actions/` |
| Udev | `/usr/lib/udev/rules.d/` |
| tmpfiles | `/usr/lib/tmpfiles.d/` |
| Logs | journald (aucun `/var/log`) |

**Aucun `/opt`.** Aucun runtime Node.js/Python.

### 6.7 Migration depuis l'original

1. Détecter `/etc/tcc/settings`, `/etc/tcc/profiles`, `/etc/tcc/fantables`,
   `/etc/tcc/webcam`.
2. Valider chaque JSON (taille, types, plages, IDs).
3. Convertir vers le schéma `schema_version = 1`, ajouter la matrice PPD
   (valeurs par défaut), conserver `stateMap` pour compatibilité.
4. Sauvegarder l'ancien répertoire (`/var/lib/tuxedo-rust-center/backup/` ou
   `.bak` à côté, root:root 0600).
5. Écrire atomiquement l'état mutable dans `/var/lib/tuxedo-rust-center/*.json`
   (fichier temporaire même répertoire, `fsync`, `rename`, `fchmod` explicite),
   `0644 root:root`.
6. Ne jamais écraser un fichier non reconnu sans sauvegarde, et ne jamais
   écrire dans `/etc/tuxedo-rust-center` (configuration administrateur, D2).

---

## 7. Capteurs et performance

| Capteur | Source | Remarque |
|---|---|---|
| Température CPU | hwmon `coretemp`/`k10temp` `tempN_input` (m°C) | Unité à convertir en °C |
| Fréquence CPU | `scaling_cur_freq` (kHz) ou `cpuinfo_cur_freq` | Moyenne sur politiques en ligne |
| Puissance CPU | RAPL `energy_uj` (µJ) | Delta / temps, overflow à gérer |
| Ventilateurs | `fanN_input` (tr/min), tuxi/tuxedo-io | Vitesse en % si PWM |
| GPU AMD | `/sys/class/drm/card*/device/hwmon/hwmonN/{temp1_input,power1_average,freq1_input}` | Selon pilote |
| GPU Intel | RAPL iGPU + `i915` freq | Pas toujours exposé |
| GPU NVIDIA | `nvidia-smi --query-gpu=...` (allowlist) | Commande externe validée |

- Aucun polling par défaut. Le daemon active un timer de collecte (1 s max)
  seulement après `SetSensorDataCollectionStatus(true)` et l'arrête quand le
  dernier client se déconnecte.
- Budget : daemon inactif < 1 % CPU, < 30 Mo RSS. La collecte doit être
  désactivée à l'écran verrouillé si aucun client n'est présent.
- RAPL : `energy_uj` peut déborder (`max_energy_range_uj`) ; gérer le wrap.
- Les valeurs manquantes sont `None`, jamais 0 inventé.

---

## 8. Risques et dégradation

| Risque | Impact | Mitigation |
|---|---|---|
| Matériel non-TUXEDO | Aucun sysfs `tuxedo_*`, pas de `/dev/tuxedo_io` | Tous les modules sont optionnels ; état `unsupported` dans l'UI ; aucun refus de démarrage |
| `/dev/tuxedo_io` absent | Ventilateurs/TDP/webcam indisponibles | Fallback hwmon `tuxi`/`pwm` ; message clair |
| Version ioctl ancienne | Nouveaux attributs indisponibles | `module_version` + `api_compatible` exposés ; log, pas de crash |
| ABI sysfs instable | Attribut renommé/supprimé | Lecture optionnelle, `is_unavailable`, tests de régression |
| debugfs non monté | YCbCr indisponible | `unsupported`, pas d'erreur |
| PPD absent | Pas de dimension PPD | Fallback AC/BAT |
| PPD/écriture concurrente | Conflit `platform_profile`/EPP | PPD propriétaire par défaut ; option explicite et loguée |
| Session verrouillée / TTY | Refresh rate impossible | Agent retourne `unsupported`/erreur ; réessai au prochain signal |
| Compositeur Wayland inconnu | Refresh rate indisponible | `unsupported`, jamais de fallback XRandR |
| Écran externe | Mode modifié par erreur | Filtre strict eDP/LVDS/DSI + test dédié |
| Mode absent | Résolution changée par erreur | `DisplayModeNotFound`, aucune substitution |
| Veille/reprise | Ventilateur bloqué, fd invalide | `PrepareForSleep` + `pause/resume` + réouverture des backends |
| TDP/cTGP hors plage | Instabilité, surchauffe | Validation firmware min/max avant ioctl |
| Écriture sysfs malveillante | Escalade via symlink | `O_NOFOLLOW` + `fstat` + chemin relatif validé |
| Commande externe détournée | Injection | Allowlist, `Command::new`, args séparés, validation |
| Dépendance compromise | Exécution arbitraire | `cargo deny`, `cargo audit`, revue `unsafe` |

### Dégradation attendue par matériel

- **Non-TUXEDO** : CPU/backlight/alimentation/affichage fonctionnent si le
  noyau les expose ; ventilateurs TUXEDO, TDP, cTGP, charge TUXEDO, lighting,
  Fn-Lock, webcam switch sont `unsupported`.
- **TUXEDO Clevo** : ioctl Clevo, 3 ventilateurs, webcam, profils ODM,
  lighting, charge ; pas de TDP.
- **TUXEDO Uniwill** : ioctl Uniwill, 2 ventilateurs, TDP, profils ODM, LEDs ;
  pas de kill switch webcam.
- **Sirius 16 Gen1/Gen2** : `tuxi` uniquement, pas de double backend PWM.
- **VM/conteneur** : DMI absent, pas de batterie, `/dev/tuxedo_io` absent ;
  daemon démarre, UI en mode dégradé.

---

## 9. Points à intégrer dans `tcc-daemon` et `tcc-user`

### `tcc-daemon`

1. Construire `SysfsRoot::from_env_or_system()` une seule fois. En release,
   cette fonction retourne `/sys` ; `TCC_SYSFS_ROOT` n'agit que si `tcc-hw` est
   compilé avec `dev-overrides`, hors root (D7).
2. Utiliser `SysfsPowerStateReader` + UPower (priorité UPower pour les
   signaux), avec debounce et gestion `Unknown`.
3. `CpuController` : appliquer governor/EPP/fréquences/cœurs selon la
   politique PPD, en ignorant EPP/`platform_profile` si PPD est actif (D4).
4. `BacklightDevice::set_all_percent` pour la luminosité (comportement
   original multi-drivers).
5. `TuxedoIo::open()` paresseux : ne pas échouer au démarrage si absent ;
   exposer `interface`, `module_version`, `api_compatible`.
6. `shutdown()` sur SIGTERM ; `pause/resume` sur `PrepareForSleep`, avec
   recréation des workers et réouverture des backends après reprise.
7. D-Bus : conserver l'interface historique et servir
   `com.tuxedocomputers.tccd.Manager1` sur le même objet. `Manager1` expose
   `GetHardwareSnapshotJSON`, la collecte capteurs à la demande et des setters
   non persistants pour charge/lighting/Fn-Lock. TDP/ODM/cTGP/PRIME/YCbCr ne
   sont **pas** exposés en écriture (réservés à `tccd-admin` + polkit).
   `Control1.ReloadConfig` est root-only, sérialisé entre appels concurrents
   (aucun cache de rapport) et renvoie un `AdminReloadReport` JSON frais.
8. Démarrer `HardwareWorkers` (charge, lighting, ODM/TDP, capteurs, NVIDIA,
   webcam, Fn-Lock, PRIME lecture, YCbCr) avec `HardwareInputs`,
   `HardwareStatus` et `sensors_enabled`; mettre à jour les entrées et
   `hardware.notify()` à chaque changement de profil, réglages, PPD ou reload.
   `hardware.shutdown()` avant l'arrêt.
9. Rate limit, validation et polkit de tous les setters. `SetTempProfile`,
   `ClearTempProfile` restent sans polkit (override temporaire global, R-04) ;
   les setters `Manager1` charge/lighting/Fn-Lock appellent `tcc-authz` après
   validation et rate limit par **nom+UID**, avec sujet `system-bus-name`,
   `DialogGate` global, fail-closed, sans court-circuit root. La policy D-Bus
   peut alors ouvrir ces membres. Les setters lighting respectent
   `keyboardBacklightControlEnabled` ; Fn-Lock reste indépendant. Les messages
   D-Bus d'erreur matérielle sont génériques (détail dans journald) et un audit
   `ok|error` est émis après chaque écriture. Les opérations
   Prime/Tomte/WebFAI et l'assignation persistante vivent dans `tccd-admin`
   (D1), pas dans `tccd`.
10. Migration `/etc/tcc` avant la première écriture ; état mutable dans
    `/var/lib/tuxedo-rust-center` (D2), écritures atomiques, sauvegardes `.bak`
    root:root 0600.
11. Logs `tracing` : transitions d'état, profil appliqué, erreurs matérielles,
    jamais de secret (D5).
12. Ne jamais lire `/proc/*/environ` (D6) : l'environnement de session est
    fourni par `tccd-user` (D-Bus), jamais reconstruit depuis `/proc`.
13. Vérifier la feature `dev-overrides` à la compilation du paquet : elle doit
    être absente des binaires release.

### `tccd-admin`

1. Service root distinct, activation D-Bus uniquement, sans état persistant,
   sortie après 30 s d'inactivité ; `Restart=no` côté unité.
2. Phase 1 implémentée : `SaveProfilesJSON`, `SaveSettingsJSON`,
   `ReloadConfig` via `Admin1`, avec transaction `.bak`/rollback et appel
   `Control1.ReloadConfig`. Le rollback n'utilise qu'un précédent valide pour
   le type cible ; un précédent corrompu est traité comme absent. Un `.bak`
   sain n'est jamais remplacé par un contenu illisible (`preserved`), et
   l'audit expose `backup=written|preserved|none`. Aucune méthode de `tccd`
   n'expose ces opérations.
3. Chaque méthode d'écriture vérifie polkit (`profiles`, `settings`) avec le
   sujet `system-bus-name` de l'appelant, un bucket par nom/UID et un seul
   dialogue global. `Ping`/`GetVersion` sont libres.
4. Phase 2 (TDP/ODM/cTGP/PRIME) et phase 3 (Tomte/WebFAI/diagnostic) : non
   implémentées. La phase 3 vit dans un service séparé, jamais dans l'unité de
   configuration.
5. Aucune lecture de `/proc/*/environ` (D6).

### `tcc-user`

1. Détecter le compositeur (variables d'environnement, sockets, D-Bus de
   session) sans commande shell.
2. Implémenter `DisplayModeBackend` par backend (X11, Hyprland, GNOME, KDE,
   Sway, wlroots) et publier `kind()`/`backend_name()` à l'UI.
3. Filtrer eDP/LVDS/DSI via `tcc_hw::display::select_internal_output`.
4. Utiliser `apply_internal_refresh_rate` (tolérance 0,5 Hz) pour la logique
   commune ; sérialiser les demandes.
5. Lire le profil actif et écouter `ActiveProfileChanged` sur le bus système ;
   ne jamais écrire la configuration.
6. Retourner un état (`applied`, `already_applied`, `unsupported`, erreur) au
   daemon pour l'affichage UI.
7. Ne pas dépendre de `XRANDR`/`wlr` si la session ne les expose pas : état
   `unsupported` immédiat.

### Conversions `tcc-hw` → `tcc-core`/`tcc-ipc`

| `tcc-hw` | Cible |
|---|---|
| `power_supply::PowerState` | `tcc_core::PowerSource` |
| `cpu::CpuPolicy` | champs du profil CPU (kHz) |
| `cpu::ScalingDriver` | quirk/info UI |
| `backlight::BacklightDevice` | pourcentage UI |
| `display::OutputInfo`/`DisplayMode` | payload refresh rate `tcc-ipc` |
| `display::ApplyOutcome` | état UI (appliqué/non supporté/erreur) |
| `tuxedo_io::Interface`/`module_version` | diagnostic système |

Ces conversions sont documentées mais **pas** implémentées dans `tcc-hw` pour
éviter une dépendance circulaire.

---

## 10. Écarts assumés par rapport à l'original

Chaque écart est justifié et doit être validé par les tests de non-régression.

| Original | Fichier source | Décision TuxedoRustCenter |
|---|---|---|
| Défaut AC quand aucun `Mains` n'est trouvé | `common/classes/StateUtils.ts` | `PowerState::Unknown` : ne pas appliquer un profil « secteur » sur une machine sans source identifiable |
| Quirk amdgpu assigné puis écrasé par le constructeur | `common/classes/DisplayBacklightController.ts` | Quirk réellement appliqué dans `backlight.rs` (valeur d'affichage 8 bits, pourcentage normalisé) |
| `process.exit(0)` si un label de ventilateur ne correspond pas | `service-app/classes/FanControlHwmon.ts` | Log + fallback, jamais d'arrêt du daemon |
| Governor validé au fil des écritures, écriture partielle possible | `common/classes/CpuController.ts` | Validation globale avant toute écriture |
| Refresh rate par `xrandr`, scraping de `w` et `/proc/*/environ` | `common/classes/XDisplayRefreshRateController.ts`, `service-app/classes/DisplayRefreshRateWorker.ts` | XRandR natif dans l'agent de session ; aucun scraping, aucun `xrandr` |
| Écriture YCbCr par `appendFileSync` sans vérification | `service-app/classes/YCbCr420WorkaroundWorker.ts` | `SysfsRoot::write_string` contrôlé, échec journalisé |
| Chemins `/opt`, `cameractrls.py` via udev, `pkexec /bin/sh -c` | `common/classes/TccPaths.ts`, `src/dist-data/99-webcam.rules` | FHS, udev sans processus, allowlist sans shell |
| `engine` PPD absent | — | Nouvelle entrée `(ppd, power)`, fallback AC/BAT, PPD propriétaire de `platform_profile`/EPP |
| Écran interne choisi par regex `eDP|LVDS` sur la sortie `xrandr` | `XDisplayRefreshRateController.ts` | Sélection par nom de connecteur, DSI inclus, écrans externes explicitement refusés |

Points encore ouverts (à trancher par les agents concernés) :

- restauration du taux de rafraîchissement à l'arrêt du daemon / changement de
  profil vers « ne pas changer » ;
- API D-Bus exacte de KScreen selon les versions de Plasma (fallback
  `kscreen-doctor` documenté) ;
- support des compositeurs Wayland supplémentaires (Cosmic, River, ...).

---

## 11. Matrice matérielle — état d'implémentation

Les domaines suivants sont implémentés dans `tcc-hw` et exposés par des
workers autonomes sous `crates/tcc-daemon/src/hardware/**` :

| Domaine | Modules | Branchement |
|---|---|---|
| Charge | `tcc_hw::charging` + `hardware::charging` | `mod hardware;`, `HardwareWorkers::start` |
| Éclairage | `tcc_hw::lighting` + `hardware::lighting` | idem |
| ODM / `platform_profile` | `tcc_hw::platform_profile` + `hardware::odm` | idem |
| TDP PL1/PL2/PL4 | `tcc_hw::tdp` + `hardware::odm` | idem |
| Identification modèle / quirks | `tcc_hw::device` | utilisé par ODM/PRIME |
| Fn-Lock | `tcc_hw::fn_lock` + `hardware::fn_lock` | idem |
| Capteurs CPU/GPU/RAPL/NVIDIA | `tcc_hw::sensors` + `hardware::sensors` | + `AtomicBool` de collecte |
| NVIDIA cTGP | `tcc_hw::nvidia` + `hardware::nvidia` | idem |
| Webcam (kill switch, V4L2) | `tcc_hw::webcam` + `hardware::webcam` | idem |
| PRIME (état) | `tcc_hw::prime` + `hardware::prime` | lecture seule ; **aucune API de bascule dans `tcc-hw`** (H-01), bascule à implémenter dans `tccd-admin` avec polkit |
| YCbCr 4:2:0 | `tcc_hw::ycbcr` + `hardware::ycbcr` | idem |
| Commandes allowlistées | `tcc_hw::command` | `nvidia-smi`, `prime-select`, `dpkg-query` |

La référence complète — chemins, validations, payloads D-Bus proposés, matrice
par machine, écarts assumés, risques et procédure de validation sur laptop
réel — est dans **`docs/hardware/HARDWARE_MATRIX.md`**. Ce document reste la
source de vérité pour les décisions matérielles ; `linux-platform` y trouvera
la liste exacte des APIs à appeler depuis ses setters D-Bus et le détail du
drapeau de collecte de capteurs.

Point d'intégration minimal : `crates/tcc-daemon/tests/hardware_workers.rs`
compile déjà le module via `#[path]`. Il suffit d'ajouter `mod hardware;` dans
`main.rs` et de suivre la séquence décrite dans
`docs/hardware/HARDWARE_MATRIX.md` §2.1.

Contraintes rappelées :

- `platform_profile` n'est **pas** écrit quand PPD est actif, sauf override
  explicite (`HardwareInputs.ppd_override`), conformément à D4 ;
- la collecte de capteurs n'est active que si `SetSensorDataCollectionStatus`
  l'a demandée ;
- les workers n'écrivent qu'au changement de révision, jamais à chaque tick,
  afin de ne pas écraser un réglage externe ;
- les contrôles UVC webcam ne sont pas implémentés (pas d'`unsafe` hors du
  wrapper `tuxedo_io`) ; décision attendue de `security` (module isolé revu
  ou crate auditée).

