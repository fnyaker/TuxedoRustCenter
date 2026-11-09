# Validation manuelle — runbook

Ce document liste les vérifications qui ne peuvent pas être faites en CI :
root, polkit réel, bus système, logind, matériel TUXEDO, tray/SNI, caméra.
Exécuter les sections dans l'ordre, consigner les résultats dans la dernière
table.

Références : `docs/RELEASE_CHECKLIST.md`, `docs/security/REVIEW_*.md`,
`docs/qa/HARDWARE_SMOKE_TESTS.md`.

---

## 0. Préparation

```sh
cd /chemin/vers/TuxedoRustCenter
cargo build --release --workspace
cd crates/tcc-gui && ./tools/qml-check.sh && cd ../..
```

Vérifier que le TCC original ne monopolise pas le nom D-Bus pendant les tests
de production :

```sh
systemctl status tccd.service          # ancien TCC
sudo systemctl stop tccd.service
sudo systemctl disable tccd.service    # seulement si remplacement définitif
```

Pour les tests sans root, utiliser `--dev-session` et un nom dédié
`com.tuxedocomputers.tccd.Dev`.

## 1. Vérifications non-root (sûres)

```sh
./scripts/release/validate.sh --repo
./scripts/release/validate.sh --tree build/pkgroot
systemd-analyze verify packaging/systemd/tccd.service
systemd-analyze security --offline=true packaging/systemd/tccd.service
xmllint --noout packaging/dbus/com.tuxedocomputers.tccd.conf
xmllint --noout packaging/polkit/com.tuxedocomputers.tccd.policy
```

Session / matériel :

```sh
./target/release/tcc-daemon --check
./target/release/tcc-user --list
./target/release/tcc-user --dry-run --set-refresh 60
v4l2-ctl --list-devices
v4l2-ctl -d /dev/video0 --list-ctrls
```

Attendu :

- `tccd --check` : PPD, secteur/batterie, profil résolu ;
- `tccd-user --list` : backend de session, écran interne uniquement, modes ;
- `--dry-run` : plan `WxH@Hz` sur `eDP-*`, jamais `DP-*`/`HDMI-*` ;
- webcam : contrôles listés, `controlsSupported=true` dans
  `GetHardwareSnapshotJSON` après bail capteurs.

Caméra via le daemon de dev (lecture seule) :

```sh
TCC_DBUS_BUS=session ./target/release/tcc-daemon --dev-session \
  --bus-name com.tuxedocomputers.tccd.Dev &
TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
  ./target/release/tcc-cli sensors on
TCC_DBUS_BUS=session TCC_DBUS_SERVICE=com.tuxedocomputers.tccd.Dev \
  ./target/release/tcc-cli hardware | jq '.webcam'
kill %1
```

Note : en `--dev-session`, le dry-run n'ouvre pas `/dev/video*` ; utiliser le
test d'intégration opt-in `cargo test -p tcc-hw --test real_v4l2 -- --ignored`
pour la lecture réelle.

## 2. Tray / autostart (session graphique réelle)

```sh
./target/release/tcc-gui --tray
```

- L'icône apparaît (KDE/Hyprland ; GNOME nécessite l'extension AppIndicator).
- Menu : Ouvrir, profils temporaires, Autostart, Bloquer la veille, Fn-Lock,
  mode graphique, Version, Quitter.
- Fermer la fenêtre → masquage ; Quitter → fin du processus.
- Autostart : activer, vérifier
  `~/.config/autostart/tuxedo-rust-center.desktop` (0644, `Exec=... --tray`),
  se déconnecter/reconnecter.
- Tray indisponible : la fenêtre reste accessible ; aucune erreur QML.
- Power blocker : activer, vérifier `systemd-inhibit --list` (what=idle,
  who=TuxedoRustCenter), désactiver, vérifier la disparition ; fermer la GUI →
  inhibition libérée.

## 3. Runtime sur bus système sans désinstaller le TCC officiel

Le TCC officiel (`/opt/tuxedo-control-center`) possède `tccd.service` et le nom
`com.tuxedocomputers.tccd`. Pour tester notre daemon sans écriture matérielle :

```sh
sudo ./scripts/validation/runtime-system-check.sh
```

Le script arrête temporairement l'ancien service, lance
`target/release/tcc-daemon --dry-run --start`, vérifie Manager1/Control1 et la
policy, puis **redémarre automatiquement l'ancien TCC** (trap). Il ne
désinstalle rien et n'écrit aucun réglage matériel.

Résultat attendu : démarrage, lectures Manager1, `Control1` root, setters
refusés à `nobody`, logs `dry-run: aucune écriture`.

## 4. Remplacement complet avec sauvegarde / rollback

Sauvegarde obligatoire (TCC officiel + `/etc/tcc` + base pacman) :

```sh
sudo ./scripts/validation/backup-official.sh
```

Le script écrit dans `/var/backups/tuxedo-official-<date>/` : `/opt`
(≈450 Mo), fichiers système du paquet, `/etc/tcc`, entrée pacman, métadonnées.

Installation du paquet avec admin :

```sh
./scripts/release/build.sh --arch --with-admin --no-deb
sudo ./scripts/validation/install-replacement.sh \
  /var/backups/tuxedo-official-<date> \
  build/packages/tuxedo-rust-center-0.1.0-1-x86_64.pkg.tar.zst --yes
```

Le script arrête/désactive l'ancien, supprime `tuxedo-control-center-bin`,
restaure `/etc/tcc`, installe notre paquet, active `tccd.service`.

Rollback complet :

```sh
sudo ./scripts/validation/rollback-official.sh /var/backups/tuxedo-official-<date>
```

Il supprime `tuxedo-rust-center`, restaure `/opt`, les fichiers système,
`/etc/tcc`, l'entrée pacman, puis réactive `tccd.service`. Si l'entrée pacman
est invalide, réinstaller `tuxedo-control-center-bin` depuis AUR.

## 5. Admin phase 1 (root + polkit)

Contrôle read-only automatique :

```sh
sudo ./scripts/validation/root-check.sh
```

Il vérifie la policy, les unités installées, `DeviceAllow`, `Control1`
root-only, `Admin1` (si installé), les permissions de `/var/lib` et l'audit.
Il n'installe et ne modifie rien.

Installer la policy et le service admin :

```sh
sudo install -m 0644 packaging/polkit/com.tuxedocomputers.tccd.policy \
  /usr/share/polkit-1/actions/
sudo install -m 0644 packaging/dbus/com.tuxedocomputers.tccd.conf \
  /usr/share/dbus-1/system.d/
sudo install -m 0644 packaging/systemd/tccd-admin.service \
  /usr/lib/systemd/system/
sudo systemctl daemon-reload
```

Vérifier la policy (root) :

```sh
sudo ./scripts/release/dbus-policy-test.sh
```

Attendu : SetTempProfile, lectures, Admin1 phase 1 passent ; Properties refusé ;
8 setters `Manager1` refusés ; Control1 non-root refusé.

Tester `tccd-admin` :

```sh
systemctl status tccd-admin.service
busctl --system call com.tuxedocomputers.tccd.Admin \
  /com/tuxedocomputers/tccd/Admin \
  com.tuxedocomputers.tccd.Admin1 Ping
```

- `Ping`/`GetVersion`/`GetContractVersion` sans polkit ;
- `SaveProfilesJSON`/`SaveSettingsJSON` : dialogue polkit (autorisé, refusé,
  annulé) ;
- `.bak` 0600 dans `/var/lib/tuxedo-rust-center`, cible 0644 ;
- vérifier `journalctl -u tccd-admin` : `caller_uid`, action, résultat,
  `backup=written|none|preserved`, `reload=ok|skipped`, aucun payload/chemin ;
- rollback : arrêter `tccd`, enregistrer un profil → `ReloadFailed`, contenu
  précédent restauré, aucun `.restore.*` ;
- tombstones : supprimer un profil, vérifier `deletedProfileIds`, recréer le
  même ID → refus ; recréer avec un ID neuf → OK.

## 6. Veille / reprise

```sh
sudo systemctl restart tccd.service
journalctl -u tccd -f
sudo systemctl suspend
```

Après réveil :

- `tccd` toujours actif, workers/fans recréés, profil réappliqué ;
- ventilateurs en auto avant la suspension (vérifier bruit/`tcc-cli fans`) ;
- bail capteurs désactivé ;
- aucun log d'erreur bloquant ;
- `systemd-inhibit --list` : plus d'inhibiteur `tccd`.

## 7. Caméra réelle

```sh
cargo test -p tcc-hw --test real_v4l2 -- --ignored --nocapture
```

Attendu : device `video0`, `controlsSupported=true`, contrôles allowlist
(brightness, contrast, etc.), aucune écriture ; `v4l2-ctl --list-ctrls` pour
comparer.

## 8. Packaging admin

```sh
./scripts/release/build.sh --with-admin
# deb/rpm/arch selon distro
```

- `tccd-admin` installé uniquement avec `--with-admin` ;
- unité **inactive** au boot, activée par D-Bus à la demande ;
- `systemd-analyze security tccd-admin` ≤ 2.5 ;
- désinstallation propre, `/etc/tcc` et `/var/lib` conservés selon politique.

## 9. Journal des preuves

| Date | Environnement | Test | Résultat | Notes |
|---|---|---|---|---|
| 2026-11-06 | KDE Wayland, laptop dev | `tccd --check` | OK | PPD balanced/AC → Office |
| 2026-11-06 | KDE Wayland, laptop dev | `tccd-user --list` | OK | KDE, eDP-1 désactivé (DP-2 actif), 26 modes |
| 2026-11-06 | KDE Wayland, laptop dev | dry-run 60 Hz | OK | plan eDP-1, DP-2 intact |
| 2026-11-06 | KDE Wayland, laptop dev | webcam V4L2 | **OK après correctif** | `/dev/video0` 0666+ACL lu en O_RDONLY, 11 contrôles ; `video2` IR 0 contrôle allowlist ; tests réels opt-in |
| 2026-11-06 | bus système, root | `root-check.sh` | Partiel | policy 42/42 OK ; runtime = TCC officiel `/opt`, score 9.4, pas de Manager1 |
| 2026-11-06 | bus système, root | `runtime-system-check.sh` | **8 PASS / 0 échec** | Manager1 lectures, setters/Control1 refusés à nobody, Control1 root, ancien TCC restauré |
| 2026-11-06 | bus système, root | `tccd-admin` réel | **OK** | installé, Ping/GetVersion, SaveSettings polkit root, `.bak` 0600, reload, audit |
| 2026-11-07 | bus système, root | `tccd-admin-hw` | **OK (partiel matériel)** | service actif, Ping/Version/Contract, policy 61/61, `GetTdpValues` NotSupported, `Properties` refusé, `DeviceAllow` |
| 2026-11-07 | bus système, root | `root-check.sh` | **23 PASS / 0 échec** | Admin1 + Admin.Hw actifs, Control1 root-only, policy complète |
| 2026-11-07 | laptop réel | ODM `tuxedo_io` | **OK** | `odm.available=true`, choix power_save/enthusiast/overboost, tdpAvailable=false attendu |
| 2026-11-07 | laptop réel | ODM par profil | **OK** | édition du profil + activation : logs `profil ODM appliqué profile=enthusiast/power_save source="tuxedo-io"` |
| 2026-11-06 | laptop réel | installation Arch + admin | **OK après corrections** | `tccd` actif, Office appliqué ; fan2 corrigé ; admin démarre |
| 2026-11-06 | laptop réel | `admin-save-test.sh` | **OK** | `SaveSettingsJSON` polkit root, reload, restauration, `.bak` 0600, audit `backup=written reload=ok` |
| 2026-11-06 | KDE Wayland | tray SNI | **OK** | icône enregistrée (`StatusNotifierItem`), GUI tray ~166 MiB |
| 2026-11-06 | KDE Wayland | agent session | **OK** | `tccd-user` actif, backend `kde`, interne `eDP-1` |
| 2026-11-06 | laptop réel | `systemctl suspend` | **OK** | veille 14 s, fans auto, workers arrêtés, reprise + profil réappliqué, inhibitor réacquis |
| 2026-11-06 | laptop réel | arrêt `tccd` | OK | ventilateurs rendus au firmware (repos) |
| | | | | |

## 10. Ce qui reste explicitement non validable ici

- vrai polkit interactif (dialogue, annulation) ;
- root/bus système et rollback réel ;
- `systemctl suspend` réel ;
- tray SNI réel GNOME/KDE/Hyprland ;
- matériel TUXEDO Clevo/Uniwill (fans, charge, LED, TDP, cTGP, PRIME) ;
- paquets installés/désinstallés sur une machine cible.
