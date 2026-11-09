# Migration et cohabitation avec TUXEDO Control Center

Ce document décrit la reprise des données de l'original, le conflit de service
`tccd.service`, le nom D-Bus partagé, et les procédures de désinstallation.

Règle d'or : **l'ancien et le nouveau daemon ne doivent jamais piloter le
matériel en même temps.** Ils écrivent les mêmes attributs sysfs et
revendiquent le même nom D-Bus.

---

## 1. Conflit structurel

| Élément | TCC original | TuxedoRustCenter | Conséquence |
|---|---|---|---|
| Unité systemd | `/usr/lib/systemd/system/tccd.service` | même chemin | conflit de fichier |
| Unité veille | `tccd-sleep.service` | retirée (R-06) | l'ancienne unité est supprimée avec le paquet original ; aucune unité de veille n'est livrée |
| Nom D-Bus | `com.tuxedocomputers.tccd` | même nom | `request_name` échoue pour le second |
| Configuration | `/etc/tcc/` | `/etc/tuxedo-rust-center` + `/var/lib/tuxedo-rust-center` | migration |
| Politique D-Bus | `com.tuxedocomputers.tccd-dbus.conf` | `com.tuxedocomputers.tccd.conf` | fichiers distincts |
| Polkit | action `com.tuxedocomputers.tccd` | actions `.profiles`, `.settings`, `.power-limits`, `.charging`, `.display`, `.prime-select`, `.tomte-run`, `.packages` ; LUKS hors périmètre | identifiants distincts |
| Udev | `99-webcam.rules` (Python) | `99-tuxedo-rust-center-webcam.rules` (sans `RUN+=`) | fichiers distincts |
| Autostart tray | `tuxedo-control-center-tray.desktop` | intégré à la GUI | fichier original à retirer |

Les paquets déclarent donc :

- Debian : `Conflicts: tuxedo-control-center`, `Replaces: tuxedo-control-center`,
  `Provides: tuxedo-control-center` (et `tuxedofancontrol`) ;
- RPM : `Conflicts`, `Obsoletes: tuxedo-control-center < version`, `Provides` ;
- Arch : `conflicts=`, `replaces=`, `provides=`.

Cela force le retrait de l'ancien paquet avant l'installation du nouveau.

---

## 2. Installation depuis les paquets

### Debian / Ubuntu

```sh
# 1. Sauvegarder les profils et réglages de l'ancien TCC (GUI → export).
sudo cp -a /etc/tcc /var/backups/tcc-$(date +%F) 2>/dev/null || true

# 2. Installer : le paquet original est retiré par Conflicts.
sudo apt install ./tuxedo-rust-center_0.1.0-1_amd64.deb

# 3. Vérifier que l'ancien daemon n'est plus actif.
systemctl status tccd.service --no-pager
sudo journalctl -u tccd -n 50 --no-pager

# 4. Démarrer si nécessaire (le postinst le fait sauf détection /opt).
sudo systemctl start tccd.service
```

`apt remove tuxedo-control-center` peut être nécessaire avant si dpkg refuse
le conflit (installation manuelle antérieure). Les données `/etc/tcc` sont
conservées par le paquet original lors de sa désinstallation ? **Non** :
`after_remove.sh` de l'original supprime `/etc/tcc` sur `remove`/`purge`.
Sauvegarder **avant** toute désinstallation.

### Fedora / RPM

```sh
sudo cp -a /etc/tcc /var/backups/tcc-$(date +%F) 2>/dev/null || true
sudo dnf install ./tuxedo-rust-center-0.1.0-1.fcXX.x86_64.rpm
sudo systemctl start tccd.service    # le scriptlet active, ne démarre pas
sudo systemctl status tccd.service
```

### Arch Linux

```sh
sudo cp -a /etc/tcc /var/backups/tcc-$(date +%F) 2>/dev/null || true
sudo pacman -U tuxedo-rust-center-0.1.0-1-x86_64.pkg.tar.zst
sudo systemctl start tccd.service
```

Le paquet Arch active `tccd.service`, sans le démarrer (installation possible
en chroot).

### Agent de session

`tccd-user.service` est activé pour toutes les sessions graphiques par le lien
`/usr/lib/systemd/user/graphical-session.target.wants/tccd-user.service`. Il
démarre à la prochaine ouverture de session. Pour le démarrer immédiatement :

```sh
systemctl --user start tccd-user.service
systemctl --user status tccd-user.service
```

Sous Hyprland/Sway, `graphical-session.target` n'est pas toujours atteint :
ajouter `systemctl --user start tccd-user.service` au démarrage du compositeur,
ou activer le service avec `systemctl --user enable tccd-user.service`.

---

## 3. Migration des données

La migration est faite par `tccd` **au premier démarrage**, pas par les
scripts de paquet (pas d'écriture root dans un chroot d'installation).

1. Détection de `/etc/tcc/{settings,profiles,fantables,webcam}`.
2. Lecture bornée et validation stricte (taille, types, plages, IDs).
3. Conversion vers `schema_version = 1`, ajout de la matrice PPD × alimentation
   avec les valeurs par défaut ; `stateMap` historique conservé (D3).
4. Écriture atomique dans `/var/lib/tuxedo-rust-center/*.json`
   (fichier temporaire, `fsync`, `rename`, `fchmod` 0644 root:root).
5. `/etc/tcc` n'est **jamais** supprimé ni modifié par la migration.

> **Sauvegarde** : contrairement à ce qu'annonçait la documentation du jalon 1,
> la migration n'écrit pas de copie `.bak` de `/etc/tcc` (REVIEW_JALON1 R-18).
> L'original reste intact, mais une sauvegarde manuelle avant migration est
> recommandée : `sudo cp -a /etc/tcc /var/backups/tcc-$(date +%F)`.
> Les fichiers d'état corrompus dans `/var/lib` sont, eux, sauvegardés en
> `.bak` 0600 par `ConfigStore`.

Vérification :

```sh
sudo ls -l /var/lib/tuxedo-rust-center/
sudo journalctl -u tccd | grep -i migration
```

En cas de JSON corrompu, le daemon conserve l'ancien fichier, écrit une
sauvegarde et repart sur les valeurs par défaut ; l'UI affiche l'état dégradé.
Consulter `journalctl -u tccd`.

> La migration est couverte par les tests de `tcc-core` (corpus valide,
> invalide, idempotence, sauvegarde). Elle n'est pas exécutée par les
> scriptlets de paquet, volontairement.

---

## 4. Résidus d'une installation hors paquet

Une installation de l'original depuis l'archive `/opt` laisse des fichiers que
le gestionnaire de paquets ne possède pas. Les détecter après installation :

```sh
ls -d /opt/tuxedo-control-center 2>/dev/null
ls -l /usr/bin/tuxedo-control-center 2>/dev/null
ls -l /etc/xdg/autostart/tuxedo-control-center-tray.desktop 2>/dev/null
ls -l /usr/lib/udev/rules.d/99-webcam.rules 2>/dev/null
ls -l /usr/share/dbus-1/system.d/com.tuxedocomputers.tccd-dbus.conf 2>/dev/null
ls -l /usr/share/polkit-1/actions/com.tuxedocomputers.tccd-service.policy 2>/dev/null
ls -l /usr/share/polkit-1/actions/com.tuxedocomputers.tomte-access.policy 2>/dev/null
```

Que faire :

| Résidu | Action |
|---|---|
| `/opt/tuxedo-control-center` | sauvegarder puis supprimer **manuellement**, après arrêt de l'ancien service |
| `/usr/bin/tuxedo-control-center` (lien) | supprimer (pointe vers `/opt`) |
| autostart tray | supprimer : la nouvelle GUI gère son tray |
| `99-webcam.rules` | supprimer : il exécute `python3 cameractrls.py` |
| `com.tuxedocomputers.tccd-dbus.conf` | supprimer : politique permissive de l'original |
| `*.policy` TCC original | supprimer : contient `pkexec` et des chemins `/opt` |

Le paquet ne supprime jamais ces fichiers : ils ne lui appartiennent pas, et
une suppression automatique pourrait détruire une configuration utilisateur.
Le `postinst` Debian et le `%post` RPM affichent un avertissement si `/opt` est
détecté, et **ne démarrent pas** `tccd` dans ce cas.

Après nettoyage :

```sh
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo systemctl start tccd.service
```

---

## 5. Désinstallation

### Debian / Ubuntu

```sh
sudo apt remove tuxedo-rust-center     # conserve /var/lib et /etc
sudo apt purge tuxedo-rust-center      # supprime /var/lib/tuxedo-rust-center
```

- `prerm` arrête `tccd.service` ;
- `postrm remove` désactive les units ;
- `postrm purge` supprime uniquement `/var/lib/tuxedo-rust-center` ;
- `/etc/tuxedo-rust-center` et `/etc/tcc` ne sont **jamais** supprimés par le
  paquet.

### RPM

```sh
sudo dnf remove tuxedo-rust-center
```

`%preun` désactive les units et les arrête dans l'ordre sûr. L'état
`/var/lib/tuxedo-rust-center` est conservé : le supprimer manuellement si
souhaité.

### Arch

```sh
sudo pacman -R tuxedo-rust-center        # conserve /var/lib et /etc
sudo pacman -Rns tuxedo-rust-center      # retire aussi les dépendances orphelines
```

`pre_remove` arrête les services ; `/var/lib` et `/etc` sont conservés.

### Retour à l'original

1. désinstaller TuxedoRustCenter ;
2. réinstaller `tuxedo-control-center` ;
3. l'ancien service reprend `tccd.service` et le nom D-Bus ;
4. les profils exportés depuis l'UI peuvent être réimportés.

Les formats de données ne sont pas rétro-compatibles : la migration est à sens
unique. Sauvegarder `/etc/tcc` avant la première migration.

---

## 6. Dépannage

### `tccd` ne démarre pas

```sh
systemctl status tccd.service --no-pager
journalctl -u tccd -b --no-pager
```

Causes fréquentes :

- **cycle d'ordonnancement systemd** au boot :
  `Found ordering cycle: tccd.service/start after power-profiles-daemon.service`
  puis `Job tccd.service/start deleted to break ordering cycle`. PPD est
  `After=multi-user.target` et `tccd` est `WantedBy=multi-user.target` : ne
  jamais mettre `After=power-profiles-daemon.service` dans `tccd.service`
  (garder `Wants=`). Après correction : `systemctl daemon-reload` puis
  `systemctl enable --now tccd.service` ;
- `tccd-admin.service` / `tccd-admin-hw.service` sont **statiques** et
  activés par D-Bus à la demande : `systemctl enable` sur ces unités répond
  « no installation config (WantedBy=…) », c'est normal ;
- nom D-Bus déjà pris par l'ancien daemon (`Name already taken`) → arrêter
  l'ancien service, ou tester avec un nom alternatif :
  `tccd --bus-name com.tuxedocomputers.tccd.Dev` ;
- `/dev/tuxedo_io` absent (driver non chargé) : le daemon doit démarrer en
  mode dégradé ; vérifier `modprobe tuxedo_io` et `dmesg` ;
- permission refusée sur un attribut sysfs : vérifier le mode du fichier et
  `systemd-analyze security tccd.service` ;
- `ProtectSystem=strict` + un chemin oublié dans `ReadWritePaths` : le
  journal indique `Read-only file system`.

Diagnostic non privilégié :

```sh
tccd --check          # dry-run, aucune écriture matérielle
tcc-cli status
tcc-cli profiles
```

### Veille / reprise

`tccd-sleep.service` n'est plus installé (REVIEW_JALON1 R-06). La veille sera
gérée par `tccd` via `org.freedesktop.login1.Manager.PrepareForSleep` :
réapplication du profil et réouverture de `/dev/tuxedo_io` au réveil. **Ce
n'est pas encore implémenté** (owner : `linux-platform`).

En attendant, `tccd` reste actif pendant la veille. Si le firmware réinitialise
les limites CPU/ventilateurs au réveil, elles sont réappliquées au prochain
changement d'état ou après `systemctl reload-or-restart tccd.service`
(`SIGHUP` recharge la configuration). Ne pas réintroduire l'ancienne unité
`tccd-sleep.service` : elle est notée 9.2 UNSAFE par
`systemd-analyze security`.

### L'agent `tccd-user` échoue au démarrage

Sur les distributions qui restreignent les namespaces utilisateur non
privilégiés (par exemple `kernel.apparmor_restrict_unprivileged_userns=1`),
les options `ProtectSystem=`, `ProtectHome=` et `ProtectKernel*` d'une unité
**utilisateur** peuvent empêcher le démarrage. Repli documenté, à appliquer
consciemment :

```sh
systemctl --user edit tccd-user.service
```

```ini
[Service]
ProtectSystem=
ProtectHome=
ProtectKernelTunables=
ProtectKernelModules=
ProtectControlGroups=
ProtectClock=
ProtectHostname=
```

L'agent reste non privilégié et ne touche ni `/etc`, ni sysfs, ni
`/dev/tuxedo_io` : ce repli réduit la défense en profondeur, pas le privilège.

### Le taux de rafraîchissement ne change pas

```sh
tccd-user --list                 # backend et modes disponibles
tccd-user --dry-run --set-refresh 60
journalctl --user -u tccd-user -b --no-pager
```

- GNOME/Wayland générique : non supporté à ce jalon (état exposé dans l'UI) ;
- KDE : `kscreen-doctor` doit être installé ;
- X11 : le CRTC ne doit pas être partagé avec un écran externe (miroir).

### Vérifier l'installation

```sh
systemd-analyze verify /usr/lib/systemd/system/tccd.service
systemd-analyze security --offline=true /usr/lib/systemd/system/tccd.service
desktop-file-validate /usr/share/applications/tuxedo-rust-center.desktop
appstreamcli validate --no-net /usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml
ls /opt 2>/dev/null && echo "ANORMAL: /opt présent"
find /usr/libexec/tuxedo-rust-center -maxdepth 1 -type f -printf '%m %p\n'
```
