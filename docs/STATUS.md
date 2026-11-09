# État du projet

Dernière mise à jour : 2026-11-07.

Ce document liste ce qui est réellement implémenté et vérifié, avec les
limites connues. `docs/FEATURE_PARITY.md` reste la checklist de parité ;
`docs/HANDOFF.md` donne l'état de reprise et les agents en cours.

---

## 1. Fondation et tests

- Workspace Cargo de 9 crates : `tcc-core`, `tcc-ipc`, `tcc-hw`, `tcc-daemon`,
  `tcc-user`, `tcc-gui`, `tcc-cli`, `tcc-authz`, `tcc-admin`.
- Dernière validation complète : **1527 tests Rust OK**, 0 échec ;
  **320 tests QML OK** après les lots UX/nettoyage/UVC.
  Tests `#[ignore]` documentés : mesures de performance, succès root,
  caméra réelle, environnement.
- `cargo clippy --workspace --all-targets -- -D warnings` : OK.
- `cargo fmt --all -- --check` : OK.
- `cargo build --release --workspace` : OK.
- `scripts/release/validate.sh --repo` : **350 checks OK**.
- Policy D-Bus : **61/61** (Admin1, Control1, Admin.Hw, User1).
- `systemd-analyze security` : `tccd` **3.3**, `tccd-user` **3.6**,
  `tccd-admin` **1.6**, `tccd-admin-hw` **1.9**.

### Cœur

- `tcc-core` : profils, settings, migration `/etc/tcc`, validation, écritures
  atomiques (`.bak` 0600), matrice PPD × alimentation, override temporaire,
  courbes de ventilation, presets.
- `tcc-ipc` : contrats partagés, version de contrat, types `Admin1`,
  `Control1`, `Admin.Hw`, `User1`.
- `tcc-authz` : client polkit réel (`CheckAuthorization`, sujet
  `system-bus-name`), fail-closed, limitation par nom+UID, dialogue unique,
  backend injectable.

### Matériel

- `tcc-hw` : sysfs injectable, power supply, CPU/cpufreq/EPP, backlight, DMI,
  affichage, ventilateurs (tuxi/pwm/tuxedo-io), charge, lighting,
  platform_profile, TDP, Fn-Lock, YCbCr, webcam (kill switch + énumération),
  NVIDIA cTGP, PRIME (lecture), capteurs, ioctl `tuxedo-io`, backends fake.
- V4L2 : lecture (15 CID, `unsafe` confiné) et **backend d'écriture**
  (allowlist, `S_CTRL` isolé, relecture, budget 64 ioctl, quarantaine 30 s).
- ODM réel via `tuxedo_io` (Uniwill) : choix `power_save`/`enthusiast`/
  `overboost` ; `tdpAvailable=false` sur ce laptop ; cTGP masqué sans NVIDIA.

### Daemon et services

- `tcc-daemon` : chargement config, watchers PPD/UPower/sysfs, moteur de
  bascule, D-Bus legacy + `Manager1` + `Control1.ReloadConfig` root-only,
  `SetTempProfile` rate-limité/audité, capteurs à la demande, ventilateurs,
  charge, lighting, Fn-Lock, ODM/TDP, NVIDIA cTGP, veille/reprise
  (`PrepareForSleep`), minuterie d'extinction logind.
- `Admin1` phase 1 : polkit `system-bus-name`, `SaveProfilesJSON`,
  `SaveSettingsJSON`, `ReloadConfig`, transaction `.bak` 0600, rollback
  profils+settings, tombstones d'IDs, audit sans payload.
- `Admin.Hw` phase 2 : ODM, TDP, cTGP ; `SetTdpValues([])` = maxima firmware
  explicite (audit `values=empty, maxima=true`), cTGP borné
  `max-default` (`maxCtgpOffset`, source `dynamic`/`static`), rollback.
- `tcc-user` : détection de session, X11/XRandR, Hyprland, Sway, KDE
  `kscreen-doctor`, GNOME/Mutter, écran interne uniquement, refresh +
  résolution, service `User1`.
- `tcc-cli` : `status`, `profiles`, `settings`, `fans`, `hardware`, `timer`,
  `admin`, `sensors`, `ping`, `set-temp`, `clear-temp`.

### GUI Qt 6

- Shell QML natif : Dashboard, Profils, éditeur, Outils, Réglages, Support,
  Info, tray, design system, thèmes clair/sombre/système.
- Dashboard : collecte capteurs **automatique** tant que la page est affichée
  et la fenêtre visible (aucun interrupteur, aucune note de bail) ; carte iGPU
  dédiée avec jauges temp/fréquence/puissance et graphes d'historique
  (48 points, 2 s), sans valeur inventée. Graphes CPU/dGPU intégrés à leurs
  cartes ; cartes « État du système », « Disponibilité des capteurs » et
  « Historique » supprimées.
- Éditeur : barre **Enregistrer flottante** (état, Annuler, Enregistrer) ;
  **EPP par profil** (sélection réparée, tooltip PPD) ; courbe ventilateur
  éditable par **drag & drop** (ajout double-clic, suppression clic droit/Suppr).
- Profils : grille responsive **1–4 colonnes**, cartes ≥ 300 px, contenu
  plafonné à 1440 px.
- Tray : panneau compact (profil actif, bascule rapide, PPD × AC/BAT,
  Fn-Lock, power blocker, autostart, ouvrir/quitter) + menu de secours.
- Icône launcher/dock : app_id `tuxedo-rust-center`, icône de fenêtre avec
  repli PNG, tailles 32/48/64/128/256 + scalable.
- Profils : CRUD, import/export validé, presets en lecture seule, matrice
  PPD × AC/BAT éditable, activation temporaire.
- **ODM/TDP/cTGP par profil** : `ProfileEdit` persiste `odmProfile`,
  `tdpValues`, `ctgpOffset` via `Admin1.SaveProfilesJSON` ; `tccd` applique le
  profil actif. ODM réel validé :
  `profil ODM appliqué profile=enthusiast/power_save source="tuxedo-io"`.
  Un nom ODM absent de l'allowlist ne déclenche aucune écriture.
- Réglages : CPU, ventilateurs, éclairage, charge, Fn-Lock, matrice, minuterie,
  power blocker, autostart tray, `Admin.Hw` transitoire.
- Courbe ventilateur CPU/GPU, résolution/refresh réels, webcam (lecture),
  capabilities DMI/kernel/CPU/GPU, états « non disponible ».
- i18n FR/EN/DE complète, °C/°F, réduction des animations, accessibilité
  clavier de base, fallback démo si le daemon est absent.

### Packaging et exploitation

- Installation FHS, aucun `/opt` ; unités systemd, D-Bus policy, polkit,
  udev, desktop entry, icônes, metainfo.
- Paquets deb/rpm/Arch construits et validés en conteneurs ; paquet Arch
  `tuxedo-rust-center 0.1.0-1` installé sur la machine de dev.
- Le TCC officiel a été sauvegardé puis remplacé :
  `/var/backups/tuxedo-official-*` ; rollback scripté disponible.
- Migration `/etc/tcc` → `/var/lib/tuxedo-rust-center`, sans `.bak` de
  `/etc/tcc` (R-18 ouvert).

## 2. Preuves runtime sur la machine de développement

- `tccd --check` : PPD `balanced`, secteur, profil `Office` via la matrice ;
  `tccd-user --list` : backend KDE, `eDP-1`, 26 modes ; dry-run 60 Hz sans
  toucher l'écran externe.
- Stack complète sur bus de session : `tccd --dev-session`, `tcc-cli`,
  `tccd-user`, GUI branchée, badge « Connecté ».
- `Admin1` : `SaveSettingsJSON` réel via polkit root, `.bak` 0600, reload,
  audit `backup="written" reload="ok"`, `caller_uid=0` ;
  `Control1.ReloadConfig` refusé à `nobody`.
- `Admin.Hw` : `Ping`/`GetVersion`/`GetContractVersion`, policy, `DeviceAllow`,
  `GetTdpValues` → `NotSupported` sur ce matériel ; `root-check` **23 PASS /
  0 échec** (2 SKIP matériel/TDP).
- ODM par profil : édition + activation, logs réels `enthusiast` et
  `power_save` via `tuxedo-io`.
- Écritures GUI charge type/Fn-Lock/éclairage livrées via `Admin1` ; type de
  charge absent de ce firmware.
- Agent de session `User1` : modes réels, résolution + refresh appliqués par
  `tccd-user`, GUI branchée.
- Webcam réelle `/dev/video0` : 11 contrôles UVC lus ; `/dev/video2` IR filtré.
- Tray SNI enregistré sur KDE ; `tccd-user` actif (KDE Wayland).
- Veille/réveil réels : ventilateurs auto, workers recréés, profil réappliqué,
  inhibitor réacquis.
- Budgets : daemon **0,33 % CPU / 15,1 MiB RSS** ; GUI Xvfb logicielle
  **127 MiB / 0,6 % CPU**, démarrage ~300 ms ; GL llvmpipe à re-mesurer.

## 3. Sécurité

- D1–D7 appliqués : pas de setuid ; overrides dev derrière `dev-overrides`,
  refusés en root ; état mutable dans `/var/lib`, `/etc` en lecture seule ;
  pas de shell interprété ; LUKS hors périmètre ; `ProtectKernelTunables=no`,
  `ProtectSystem=strict`, `LimitCORE=0`, `DeviceAllow=/dev/tuxedo_io`.
- `tccd-admin` phase 1 et `Admin.Hw` phase 2 : polkit `system-bus-name`,
  audit, transaction + rollback ; `Properties` refusé.
- `User1` : bus de session, aucun privilège, pas de polkit ; écriture UVC
  soumise à allowlist, device énuméré, rate limit 20/s, cache 1 s de
  `GetUvcWriteSupported`, corps D-Bus borné 64 Kio.
- Revue UVC `REVIEW_V4L2.md` : UVCW-02..08 fermés (porte d'écriture globale,
  identité device, restauration, menus, quarantaine, validation service) ;
  UVCW-01 accepté comme dette documentée (`tccd` n'appelle jamais l'écriture,
  `DeviceAllow=char-video4linux r`). Sliders GUI autorisés sous condition de
  smoke test caméra réelle avant de revendiquer l'écriture.
- Revue `REVIEW_PROFILE_HW_WRITES.md` : aucun Critique/Élevé en
  mono-utilisateur ; PHW-01 (activation globale sans polkit) bloque le
  multi-utilisateur. PHW-02/03 fermés, PHW-04/05 non bloquants.
- Setters `Manager1` : code authz prêt, policy maintenue en **deny**.
- S1-02 : course inter-clients `SaveSettingsJSON` sans révision/CAS (faible,
  ouvert).

## 4. Limites connues

- **UVC écriture** : backend `tcc-hw`, service `User1.SetUvcControl` et
  sliders GUI livrés et testés (allowlist, rate limit 20/s, erreurs typées,
  `GetUvcWriteSupported`, cache 1 s). Pas encore testé en écriture réelle sur
  caméra.
- **dGPU/NVIDIA/PRIME/cTGP** : absents sur ce laptop, non testables ;
  `Admin.Prime` non implémenté.
- **TDP PL1/PL2/PL4** : non exposé par le firmware ; le TDP officiel passe par
  le profil ODM. `GetTdpValues` renvoie `NotSupported`.
- **Multi-utilisateur** : `SetTempProfile` et l'application déclenchée par
  l'état restent globaux et sans gate polkit (PHW-01/M1-01).
- **Services absents** : Tomte, WebFAI, AnyDesk, Aquaris, diagnostic système,
  ticket/contact en ligne — cartes masquées, pas de faux backend.
- **Éclairage** : zones RGB et lightbar, couleur uniforme ; pas de couleur
  par zone ni synchronisation UPower/lid.
- **Webcam** : contrôles UVC en lecture ; préréglages par device et preview
  native non implémentés.
- **Wayland** : GNOME/KDE/Hyprland/Sway validés par code/tests, validation
  réelle multi-sessions à faire ; verrouillage de session non traité.
- **Accessibilité** : navigation clavier/focus/contraste documentés, audit
  automatisé non fait.
- **Performance GL** : à re-mesurer sur GPU réel (llvmpipe ~267 MiB).
- `cargo deny`/`cargo audit` configurés en CI, non exécutés localement.
- Fuzzing : property/mutation tests, pas de fuzzing continu.

## 5. Prochaines étapes

1. `User1.SetUvcControl` + sliders GUI + revue sécurité UVC.
2. Lots UX (Enregistrer flottant, icône launcher, tray compact) et nettoyage
   des restes de l'ancienne UI ; validation QML et i18n.
3. Rebuild paquet Arch, réinstallation, tests réels UVC/Fn-Lock/lighting/ODM.
4. Preuves root/polkit complètes (autorisation, refus, annulation).
5. Décider du périmètre restant : `Admin.Prime`, Tomte/WebFAI/Aquaris,
   diagnostic.
6. Traiter PHW-01/M1-01 avant tout support multi-utilisateur.
7. Audit final, `cargo deny`/`audit`, packaging release et guide utilisateur.
