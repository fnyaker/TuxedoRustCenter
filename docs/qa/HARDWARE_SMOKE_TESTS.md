# Smoke tests matériels — laptop TUXEDO

Ces tests sont **manuels**, **hors CI**, et ne doivent jamais être exécutés sur
une machine de production sans sauvegarde. La CI n'a pas de laptop TUXEDO :
tout ce qui suit est un complément de validation réelle, pas une exigence de
build.

Matériel visé : Clevo et Uniwill (TUXEDO), CPU Intel/AMD, GPU AMD/Intel/NVIDIA,
écran interne eDP/LVDS/DSI, clavier RGB ou blanc, lightbar, webcam UVC.

---

## 0. Préparation

1. **Sauvegarder** `/etc/tcc` et `/etc/tuxedo-rust-center` si présents.
2. **Arrêter l'ancien TCC** (`systemctl stop tccd.service`) pour éviter deux
   écrivains sysfs concurrents. Ne pas réinstaller l'ancien paquet pendant les
   tests.
3. Vérifier que le noyau expose les modules attendus :
   ```sh
   lsmod | grep -E 'tuxedo|tuxi'
   ls -l /dev/tuxedo_io /sys/class/hwmon /sys/class/power_supply
   ```
4. Travailler avec le daemon de développement **non root** quand c'est
   possible :
   ```sh
   ./scripts/dev-session.sh
   ```
   Les tests d'écriture matérielle réelle nécessitent le daemon root installé
   ou lancé sous systemd ; ne jamais bricoler les permissions de `/sys`.
5. Relever l'état initial :
   ```sh
   tccd --check
   tccd-user --list
   cargo test --workspace          # côté hôte, pas de matériel
   ```
6. Noter : modèle (`cat /sys/class/dmi/id/product_sku`), noyau
   (`uname -r`), versions des modules, sortie de `systeminfo`/`inxi -Fz` si
   disponible.

Critère d'arrêt : toute écriture qui ne correspond pas à la valeur attendue,
tout ventilateur qui s'emballe ou s'arrête anormalement, toute surchauffe ou
instabilité → revenir au mode auto (`ClearTempProfile`, redémarrage du daemon)
et documenter.

---

## 1. Détection device / DMI / quirks

| Étape | Commande / action | Attendu |
|---|---|---|
| 1.1 | `cat /sys/class/dmi/id/product_sku` | SKU reconnu (Clevo/Uniwill/TUXEDO) |
| 1.2 | `tccd --check` | PPD et alimentation affichés ; profil résolu cohérent |
| 1.3 | `busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd com.tuxedocomputers.tccd GetStateJSON` | `contractVersion=1`, `revision` ≥ 0 |
| 1.4 | `ls /sys/devices/platform/tuxedo_platform_profile` | profils ODM listés (si présent) |
| 1.5 | Changer de modèle factice (impossible) | ne pas tester les quirks d'un autre modèle |

À consigner : SKU, présence `tuxedo_platform_profile`, backend ventilateur
(`tuxi`, `pwm`, `tuxedo-io`), interface (`Clevo`/`Uniwill`).

---

## 2. Alimentation / PPD / bascule automatique

| Étape | Action | Attendu |
|---|---|---|
| 2.1 | Débrancher le secteur | `power=battery`, profil batterie appliqué |
| 2.2 | Rebrancher | `power=ac`, profil AC appliqué |
| 2.3 | `powerprofilesctl set power-saver` / `balanced` / `performance` | profil matriciel correspondant |
| 2.4 | Changer PPD puis débrancher dans la même seconde | une seule transition après debounce (~750 ms), pas d'oscillation |
| 2.5 | `SetTempProfile` sur un profil | override actif, `manualOverride=true` |
| 2.6 | Changer PPD ou AC/BAT | override invalidé, matrice reprend |
| 2.7 | Arrêter PPD (`systemctl stop power-profiles-daemon`) | fallback AC/BAT, pas de crash |
| 2.8 | Sans batterie (desktop) ou batterie inconnue | état `unknown`, profil par défaut stable |

Vérifier les logs : une transition confirmée par changement, pas de spam.

---

## 3. CPU

| Étape | Action | Attendu |
|---|---|---|
| 3.1 | Profil `performance` | governor/EPP cohérents, fréquences max |
| 3.2 | Profil `Quiet` | max réduit, turbo off selon profil |
| 3.3 | `cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor` | même valeur sur tous les CPU en ligne |
| 3.4 | Changer `onlineCores` | cœurs hors ligne/online, cpu0 toujours en ligne |
| 3.5 | Revenir au profil par défaut | valeurs restaurées |
| 3.6 | Tester sur Intel `intel_pstate` **et** AMD `amd-pstate-epp` si disponible | pas d'écriture sur un chemin absent |
| 3.7 | `scalingMaxFrequency=-1` | mode `Reduced` appliqué sans invention de fréquence |

Consigner : driver (`intel_pstate`, `amd-pstate-epp`, `acpi-cpufreq`), valeurs
min/max, EPP disponibles.

---

## 4. Ventilateurs

| Étape | Action | Attendu |
|---|---|---|
| 4.1 | `GetFanDataJSON` | `available=true`, `backend`, `fans[]` cohérents |
| 4.2 | Profil `Silent` | vitesses basses, pas d'arrêt brutal |
| 4.3 | Profil `High Performance` | montée progressive, températures stables |
| 4.4 | Courbe personnalisée | la vitesse suit la température avec hystérésis |
| 4.5 | `ClearTempProfile` | retour au mode auto firmware |
| 4.6 | Arrêter le daemon | ventilateurs rendus au firmware (`set_fans_auto`) |
| 4.7 | Multi-fan (CPU + GPU) | chaque ventilateur suit sa courbe |
| 4.8 | Uniwill | réécriture à chaque tick (quirk), pas de dérive |
| 4.9 | Sirius 16 Gen1/Gen2 | backend `pwm` ignoré, `tuxi` seul |
| 4.10 | Absence de capteur de température | dernière consigne conservée, pas de valeur inventée |

Surveiller : bruit, RPM, température CPU/GPU toutes les 5 s pendant 5 min.

---

## 5. Rétroéclairage

| Étape | Action | Attendu |
|---|---|---|
| 5.1 | Profil avec `useBrightness` | luminosité appliquée |
| 5.2 | `amdgpu_bl0` | pas de saut 16 bits/8 bits (quirk) |
| 5.3 | Écran externe branché | seul l'écran interne change |
| 5.4 | Veille/reprise | luminosité restaurée, pas de noir |

---

## 6. Charge

| Étape | Action | Attendu |
|---|---|---|
| 6.1 | `cat /sys/class/power_supply/BAT0/charge_control_end_threshold` | seuil écrit |
| 6.2 | Profil de charge `high`/`medium`/`low` | valeur acceptée par le firmware |
| 6.3 | Priorité de charge | valeur acceptée |
| 6.4 | Seuil start < end | validation refuse l'inverse sans écrire |
| 6.5 | Machine sans seuils | méthode indisponible, pas d'erreur fatale |

Consigner le modèle et les valeurs acceptées (certaines machines n'exposent
qu'un sous-ensemble).

---

## 7. Éclairage clavier / lightbar

| Étape | Action | Attendu |
|---|---|---|
| 7.1 | Détection | zones RGB, clavier blanc ou per-key correctement listés |
| 7.2 | Luminosité | valeur brute cohérente avec `max_brightness` |
| 7.3 | Couleur RGB | couleur appliquée, pas de dérive |
| 7.4 | Mode `static` / `breathing` | bascule sans redémarrage |
| 7.5 | Lightbar | synchronisée avec le clavier |
| 7.6 | UPower / capot fermé | comportement attendu (selon modèle) |

---

## 8. Refresh rate (écran interne uniquement)

| Étape | Session | Attendu |
|---|---|---|
| 8.1 | X11 | taux changé via XRandR, résolution conservée |
| 8.2 | Hyprland | `monitors` reflète le taux, socket natif |
| 8.3 | Sway | IPC Sway, résolution conservée |
| 8.4 | KDE | KScreen/`kscreen-doctor -j` |
| 8.5 | GNOME | état « non supporté » (stub), aucun crash |
| 8.6 | Wayland générique | état « non supporté » |
| 8.7 | **Écran externe HDMI/DP branché** | l'externe ne change **jamais** de mode |
| 8.8 | Taux non disponible | erreur claire, pas de fallback silencieux |
| 8.9 | Taux déjà actif (±0,5 Hz) | aucune écriture |
| 8.10 | Verrouillage/déverrouillage | réapplication au déverrouillage |

Commandes utiles :

```sh
tccd-user --list
tccd-user --dry-run --set-refresh 60
xrandr --verbose                     # X11
hyprctl monitors -j                 # Hyprland
swaymsg -t get_outputs              # Sway
kscreen-doctor -j                   # KDE
```

---

## 9. TDP / ODM / NVIDIA / PRIME

| Étape | Action | Attendu |
|---|---|---|
| 9.1 | Profils ODM | liste et défaut cohérents avec le firmware |
| 9.2 | TDP PL1/PL2/PL4 | valeurs dans min/max, croissantes |
| 9.3 | Valeur hors plage | refusée, pas d'écriture |
| 9.4 | cTGP NVIDIA | offset appliqué dans les limites |
| 9.5 | `prime-select` | état iGPU/dGPU/on-demand cohérent, pas de commande shell |
| 9.6 | Retour au profil par défaut | valeurs d'origine restaurées |

---

## 10. Webcam

| Étape | Action | Attendu |
|---|---|---|
| 10.1 | Kill switch Clevo | webcam coupée/allumée |
| 10.2 | Énumération V4L2 | périphériques et contrôles listés |
| 10.3 | Réglages UVC (phase ultérieure) | lecture seule aujourd'hui ; écriture après implémentation V4L2 |
| 10.4 | Débranchement à chaud | pas de crash, état « non disponible » |
| 10.5 | Preview Qt Multimedia | native, pas de composant web |

---

## 11. Veille / reprise et arrêt

| Étape | Action | Attendu |
|---|---|---|
| 11.1 | `systemctl suspend` | `PrepareForSleep(true)` : ventilateurs auto, workers arrêtés en ≤ 3 s, daemon reste actif |
| 11.2 | Réveil | `PrepareForSleep(false)` : backends réouverts, workers/fans recréés, profil réappliqué, bail capteurs désactivé |
| 11.3 | `systemctl restart tccd` | aucun état bloqué, ventilateurs auto au stop |
| 11.4 | `SIGTERM` | arrêt propre, pas de zombie |
| 11.5 | `SIGHUP` | config rechargée, profil réappliqué |

---

## 12. Sécurité et privilèges

| Étape | Action | Attendu |
|---|---|---|
| 12.1 | `systemd-analyze security tccd.service` | score conforme, `ProtectKernelTunables=no` |
| 12.2 | Utilisateur non privilégié → `SetTempProfile` | fonctionne (méthode non sensible) |
| 12.3 | Utilisateur non privilégié → opération admin | refus polkit (quand implémenté) |
| 12.4 | Lien symbolique dans `/var/lib/tuxedo-rust-center` | refusé (`O_NOFOLLOW`) |
| 12.5 | Fichier de config `world-writable` | refusé/loggué |
| 12.6 | `busctl introspect` | pas de méthode d'écriture arbitraire |

---

## 13. Performance

| Mesure | Commande | Budget |
|---|---|---|
| daemon inactif CPU/RSS | `pidstat -p $(pgrep -n tccd) 1 60` ; `ps -o rss= -p $(pgrep -n tccd)` | < 1 %, < 30 Mo |
| GUI RSS | `ps -o rss= -p $(pgrep -n tuxedo-rust-center)` | < 200 Mo |
| démarrage GUI | `hyperfine --warmup 3 './target/release/tuxedo-rust-center'` (ou timestamp de log) | < 1,5 s |
| boucle ventilateurs | logs/trace 1 s | pas de lecture inutile |
| absence de fuite | changer 20 fois de profil, observer threads/RSS | stable |

---

## 14. Preuves à joindre

- modèle, SKU, noyau, distribution ;
- sortie `tccd --check`, `GetStateJSON`, `GetFanDataJSON` ;
- valeurs sysfs avant/après pour chaque domaine testé ;
- logs journald filtrés (`journalctl -u tccd -u tccd-user --since ...`) ;
- captures ou tableau des taux/modes pour le refresh rate ;
- toute anomalie avec heure, profil, état AC/BAT et PPD.

Ces preuves ne doivent contenir aucun numéro de série complet, aucune adresse
MAC, aucun secret.
