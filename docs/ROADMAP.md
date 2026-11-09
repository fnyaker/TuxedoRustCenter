# Roadmap — TuxedoRustCenter

## Jalon 0 — Cadre (terminé)

- [x] Analyse de TUXEDO Control Center original.
- [x] Architecture cible et checklist de parité.
- [x] Sous-agents OpenCode et règles projet.
- [x] Workspace Cargo initial.

## Jalon 1 — Socle Rust (terminé)

- [x] `tcc-core` : modèles profils/settings, migration `/etc/tcc`, validation, sauvegarde `.bak` (151 tests).
- [x] `tcc-core` : moteur de bascule `(PPD, AC/BAT)`, override temporaire, tests.
- [x] `tcc-core` : courbes de ventilateurs et logique pure.
- [x] `tcc-ipc` : types partagés daemon ↔ GUI, version de contrat, contrat admin (89 tests).
- [x] `tcc-hw` : abstraction sysfs, power supply, CPU, backlight, ioctl `tuxedo-io` (195 tests).
- [x] Packaging FHS initial : systemd, D-Bus, polkit, tmpfiles, udev (aucun `/opt`).
- [x] `tcc-daemon` : binaire, config, watchers PPD/UPower, service D-Bus minimal (9 tests).
- [x] `tcc-user` : détection session, X11/XRandR, Hyprland, Sway, KDE `kscreen-doctor` (76 tests).
- [x] `tcc-user` : mode agent D-Bus pour appliquer le refresh rate du profil actif.
- [x] `tcc-gui` : design system + shell QML Dashboard/Profiles/Settings/Support/Info (données factices).
- [x] `tcc-gui` : binaire Rust Qt 6 natif via cxx-qt, chargement du shell QML, thème et i18n.
- [x] `tcc-gui` : pont Rust ↔ QML `DaemonClient` (connexion, profil actif, PPD, alimentation).
- [x] `tcc-gui` : adaptateur `LiveData` et profils/dashboard/sidebar live, fallback démo.
- [x] `docs/security/THREAT_MODEL.md` : modèle de menace et checklist.
- [x] `docs/security/DECISIONS.md` : décisions D1–D7 intégrées aux règles projet.
- [x] Séparer `tccd` et `tccd-admin` (D1) : design sécurité, crates `tcc-authz`/`tcc-admin`, packaging conditionnel.
- [x] `cargo fmt`, `clippy`, tests ; CI fournie (cargo deny/audit à exécuter).
- [x] `tcc-cli` : diagnostic D-Bus (`status`, `profiles`, `settings`, `set-temp`).
- [x] `docs/STATUS.md` : état vérifié sur la machine de développement.

## Jalon 2 — Daemon et profils (en cours)

- [x] Application CPU et backlight.
- [x] `tcc-gui` : écriture de profils via `Admin1` (éditeur, validation, erreurs).
- [x] `tcc-gui` : import/export de profils (enveloppe versionnée, conflits résolus).
- [x] Ventilateurs : `FanWorker` tuxi/pwm/tuxedo-io, courbes, hystérésis, garde-fous, `GetFanDataJSON`.
- [x] Charge, lighting, TDP/ODM, Fn-Lock, capteurs, NVIDIA cTGP (lecture), PRIME (lecture), YCbCr.
- [x] Webcam kill switch + énumération (contrôles UVC à arbitrer).
- [~] Branchement GUI des nouveaux domaines (capteurs, charge, lighting).
- [x] `Manager1` : snapshot matériel, capteurs on/off, setters charge/lighting/Fn-Lock rate-limités/audités.
- [~] D-Bus typé complet + compatibilité JSON (Manager1 partiel, `tccd-admin` à venir).
- [x] Persistance atomique des profils via `Admin1.SaveProfilesJSON` (transaction, `.bak`, rollback).
- [x] Persistance des réglages globaux via `Admin1.SaveSettingsJSON`.
- [x] Design `tccd-admin` + polkit (`docs/security/ADMIN_DESIGN.md`).
- [x] `tccd-admin` phase 1 : polkit, `Admin1`, persistance profils/settings, transaction/rollback.
- [x] `Control1.ReloadConfig` root-only avec coalescence.
- [ ] `tccd-admin` phase 2 : TDP/ODM/cTGP/PRIME.
- [ ] Phase 3 : Tomte/WebFAI/diagnostic (service séparé) ; **LUKS hors périmètre**.
- [x] Tests d'intégration fake sysfs/D-Bus, fixtures et plan QA (65 tests ajoutés).
- [x] Audit sécurité du jalon 1 (`REVIEW_JALON1.md`).
- [x] Corrections bloquantes R-01..R-09 (packaging, policy, rate limit, audit, systemd).
- [x] Corrections hardware H-01/H-02/H-04/H-05 (cache nvidia-smi, prime isolé, timeout commandes, policy).
- [~] Polkit `CheckAuthorization` : implémenté pour `tccd-admin` ; setters `Manager1` encore refusés par la policy.
- [ ] Validation matérielle réelle (DeviceAllow, debugfs, charge, LED, TDP, PRIME).

## Jalon 3 — GUI Qt 6

- [x] Intégration cxx-qt.
- [x] Dashboard temps réel (capteurs/état/profils via D-Bus, repli démo).
- [~] Profils + éditeur complet (lecture seule ; écriture via `Admin1` à brancher).
- [~] Settings, i18n FR/EN/DE, thèmes ; tray native à faire.

## Jalon 4 — Parité matérielle

- [x] Charge, lighting, TDP/ODM, NVIDIA cTGP (lecture), Prime (lecture), YCbCr, Fn-Lock.
- [~] Webcam : kill switch, énumération, contrôles UVC en lecture seule ; écriture hors jalon 2.
- [x] Refresh rate X11.
- [x] Refresh rate Wayland : Hyprland, Sway, KDE, GNOME/Mutter (validation réelle à faire).

## Jalon 5 — Services annexes

- [ ] Aquaris BLE, Tomte, WebFAI, support, minuterie ; LUKS exclu.

## Jalon 6 — Packaging et durcissement

- [x] deb/rpm/PKGBUILD construits, installés et validés en conteneurs Debian/Fedora/Arch.
- [x] Policy D-Bus fail-closed, setters Manager1 refusés, polkit sans exec.path.
- [x] Policy Admin1/Control1 (42/42) et unité admin phase 1 (`TCC_WITH_ADMIN=1`), non installée par défaut.
- [x] systemd hardening initial, polkit, D-Bus policy, tmpfiles, udev.
- [ ] Audit sécurité final, performance, accessibilité.
- [x] Installation/désinstallation testées en conteneurs Debian/Fedora/Arch.
- [~] Migration/cohabitation avec le TCC original documentée (`docs/packaging/MIGRATION.md`) ; test bout en bout avec l'ancien TCC à faire.
