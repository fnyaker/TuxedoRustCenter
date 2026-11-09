# Handoff — état courant et reprise après compaction

Dernière mise à jour : 2026-11-07.

Document de reprise pour l'agent principal. Références : `AGENTS.md`,
`docs/ARCHITECTURE.md`, `docs/FEATURE_PARITY.md`, `docs/QUALITY.md`,
`docs/STATUS.md`, `docs/RELEASE_CHECKLIST.md`, `docs/security/DECISIONS.md`.

---

## 1. État vérifié

- Workspace Cargo : 9 crates (`tcc-core`, `tcc-ipc`, `tcc-hw`, `tcc-daemon`,
  `tcc-user`, `tcc-gui`, `tcc-cli`, `tcc-authz`, `tcc-admin`).
- Dernière validation complète : **1527 tests Rust OK** (run UVC), **320 tests
  QML OK**, 0 échec. Recompter après les lots UX (dashboard/courbe/EPP) en
  cours.
- `cargo clippy --workspace --all-targets -- -D warnings` : OK.
- `cargo fmt --all -- --check` : OK.
- `cargo build --release --workspace` : OK.
- `scripts/release/validate.sh --repo` : **350 checks OK** ; policy D-Bus
  **61/61** (Admin1/Control1/Admin.Hw/User1).
- `systemd-analyze security` : `tccd` **3.3**, `tccd-user` **3.6**,
  `tccd-admin` **1.6**, `tccd-admin-hw` **1.9**.
- Paquet Arch construit et installé sur la machine ; il remplace le TCC
  officiel. Sauvegarde et rollback :
  `/var/backups/tuxedo-official-*` + `scripts/validation/rollback-official.sh`.
- Performance : daemon **0,33 % CPU / 15,1 MiB RSS** ; GUI Xvfb logicielle
  **127 MiB / 0,6 % CPU**, démarrage ~300 ms ; GL llvmpipe à re-mesurer.

## 2. Fonctionnalités livrées

- **Daemon** : profils/settings, bascule PPD × AC/BAT, override temporaire,
  CPU, backlight, ventilateurs (tuxi/pwm/tuxedo-io), charge, lighting,
  ODM/TDP, Fn-Lock, capteurs à la demande, NVIDIA cTGP, PRIME (lecture),
  YCbCr, webcam (kill switch + énumération + contrôles UVC en lecture),
  veille/reprise `PrepareForSleep`, minuterie d'extinction logind.
- **ODM/TDP/cTGP par profil** : `ProfileEdit` persiste `odmProfile`,
  `tdpValues`, `ctgpOffset` via `Admin1.SaveProfilesJSON` ; `tccd` applique le
  profil actif. ODM réel validé sur laptop :
  `profil ODM appliqué profile=enthusiast/power_save source="tuxedo-io"`.
  TDP PL non exposé sur ce laptop (`io.tdpAvailable=false`) ; cTGP masqué sans
  NVIDIA. Un nom ODM absent de l'allowlist ne déclenche **aucune** écriture.
- **D-Bus** : legacy JSON + `Manager1` (lecture/bail, `GetContractVersion`,
  `GetShutdownTimerJSON`) + `Control1.ReloadConfig` root-only + `Admin1`
  phase 1 + `Admin.Hw` phase 2 + `User1` (session). Policy fail-closed,
  `Properties` absent, setters `Manager1` en **deny**.
- **Admin phase 1** : polkit `system-bus-name`, `SaveProfilesJSON`,
  `SaveSettingsJSON`, `ReloadConfig`, transaction `.bak` 0600, rollback
  profils+settings, tombstones d'IDs, audit sans payload.
- **Admin.Hw** : ODM, TDP (`SetTdpValues([])` = maxima firmware explicite,
  audit `values=empty, maxima=true`), cTGP borné dynamiquement
  (`maxCtgpOffset`, source `dynamic`/`static`), rollback.
- **Session** : X11, Hyprland, Sway, KDE, **GNOME/Mutter**, écran interne
  uniquement, taux + résolution et backend. Service `User1`.
- **GUI Qt 6** : shell QML, dashboard live, profils (CRUD + import/export),
  éditeur avec ODM/TDP/cTGP par profil et barre Enregistrer flottante,
  réglages (interrupteurs, charge, matrice PPD, Admin.Hw transitoire),
  minuterie, tray natif avec panneau compact, autostart, power blocker,
  i18n FR/EN/DE, thèmes, accessibilité clavier, fallback démo.
- **Dashboard** : collecte capteurs automatique quand la page est affichée et
  la fenêtre visible ; carte iGPU (temp/fréquence/puissance) avec graphes
  d'historique locaux ; graphes CPU/dGPU intégrés aux cartes ; cartes
  redondantes supprimées.
- **Profils** : grille 1–4 colonnes, EPP réellement par profil (liaison
  `SelectRow` corrigée), tooltip PPD, courbe ventilateur drag & drop.
- **UVC** : lecture (15 CID, `unsafe` confiné) ; **backend d'écriture**
  `tcc-hw` livré (allowlist, `S_CTRL` isolé, relecture, budget, quarantaine).
  `User1.SetUvcControl` en cours d'implémentation.
- **Sécurité** : audits `REVIEW_JALON1`, `REVIEW_HARDWARE`,
  `REVIEW_ADMIN_PHASE1`, `REVIEW_MANAGER1_AUTHZ`, `REVIEW_SLEEP`,
  `REVIEW_V4L2`, `REVIEW_USER_BACKENDS`, `REVIEW_TRAY`,
  `REVIEW_SESSION_FEATURES`, `REVIEW_PROFILE_HW_WRITES`. Aucun Critique/Élevé
  ouvert en mono-utilisateur ; PHW-01 bloque le multi-utilisateur.
- **LUKS** : hors périmètre, non implémenté, action polkit/packaging/docs
  retirées.

## 3. Travaux en cours

- `tuxedo-hardware` : terminé (backend UVC + doc V4L2).
- `linux-platform` : terminé (`User1.SetUvcControl`, `GetUvcWriteSupported`,
  rate limit 20/s, erreurs typées ; workspace 49 suites OK).
- `qt-ui` (UX) : terminé (barre Enregistrer flottante, icône launcher,
  tray compact ; 320 tests QML après le lot UVC).
- `qt-ui` (nettoyage) : terminé (restes ancien TCC/jalons supprimés, carte
  iGPU + graphes, collecte capteurs automatique).
- `qt-ui` (UVC) : terminé (sliders, switches, menus, erreurs ; 192 tests
  Rust GUI, 288 QML).
- `security` : revue UVC close (UVCW-01 accepté, UVCW-02..08 fermés).
- `qa-integration` : audit à jour.
- Ensuite : paquet final, tests réels caméra/Fn-Lock/lighting/ODM.

## 4. Prochaines actions

1. **Smoke test UVC réel** : écrire brightness/contrast sur `/dev/video0`
   depuis la GUI, vérifier la relecture, `Busy` si caméra occupée, ACL
   `uaccess`/groupe `video`.
2. **Revue sécurité UVC** : terminée (UVCW-01 accepté, UVCW-02..08 fermés).
4. **Lots UX/nettoyage** : intégrer, valider QML, corriger les orphelins i18n,
   mettre à jour `UI_DESIGN.md`/`UI_CLEANUP.md`.
5. **Preuves manuelles root/polkit** : vrai polkit admin, refus/annulation,
   `.bak` 0600, rollback, `Control1` root, policy root.
6. **Validation matérielle** : UVC écriture réelle, charge, LED, Fn-Lock,
   ODM, veille, sessions GNOME/KDE/Hyprland, tray SNI réel.
7. **Décider du périmètre restant** : `Admin.Prime` (pas de dGPU testable),
   Tomte/WebFAI/AnyDesk/Aquaris/diagnostic (backends absents), Aquaris BLE.
8. **Multi-utilisateur** : PHW-01/M1-01 (activation globale sans polkit) à
   traiter avant d'exposer l'app à plusieurs sessions.
9. **S1-02** : course inter-clients `SaveSettingsJSON` sans révision/CAS.
10. Audit final, `cargo deny`/`audit`, packaging release.

## 5. Décisions / points ouverts

- **LUKS hors périmètre** (décision utilisateur) : ne pas implémenter le
  changement de mot de passe ; ne pas réintroduire d'action/méthode.
- **ODM/TDP/cTGP par profil** : voie normale, persistée via `Admin1`.
  `Admin.Hw` reste une application immédiate transitoire. PPD reste
  propriétaire de `platform_profile`/EPP (D4).
- **PPD vs ODM** : PPD peut ignorer l'ODM du profil ; l'UI le signale.
  Fallback `tuxedo-io` uniquement si `platform_profile` absent.
- **TDP** : `profile.tdpValues=[]` = ne pas changer ; `Admin.Hw.SetTdpValues([])`
  = maxima firmware explicite.
- **cTGP** : borne `max-default` si `nvidia-smi` fournit les limites, sinon
  200 W statique documenté (hypothèse driver non validée).
- **S1-02** : unique finding code faible ouvert (course sans CAS).
- **AD-12** : phase 2 en services séparés `Admin.Hw` et `Admin.Prime`.
- **Setters `Manager1`** : code authz prêt, policy en deny ; réouverture
  seulement après preuves root/polkit.
- **Rate limits** : `tccd` setters limités par nom+UID ; lectures encore
  par nom unique.
- **Tombstones** : permanents ; réparation root hors API ; borne 1024.
- **Paquet installé** : le TCC officiel n'est plus installé ; rollback
  documenté.

## 6. Commandes utiles

```sh
cargo test --workspace
cargo test --workspace --quiet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd crates/tcc-gui && ./tools/qml-check.sh
./scripts/release/validate.sh --repo
./scripts/release/validate.sh --tree build/pkgroot
./scripts/release/dbus-policy-test.sh   # root
cargo build --release --workspace
./scripts/dev-session.sh
```

## 7. Fichiers récents importants

- `docs/security/ADMIN_DESIGN.md`, `ADMIN_PHASE2_DESIGN.md`,
  `ADMIN_PHASE3_DESIGN.md`, `REVIEW_*.md`, `DECISIONS.md`, `THREAT_MODEL.md`,
  `REVIEW_PROFILE_HW_WRITES.md`.
- `docs/hardware/HARDWARE_MATRIX.md`, `V4L2_DESIGN.md`.
- `docs/qa/TEST_PLAN.md`, `BUGS.md`, `HARDWARE_SMOKE_TESTS.md`,
  `FEATURE_AUDIT.md`.
- `docs/packaging/README.md`, `MIGRATION.md`, `SECURITY.md`, `VALIDATION.md`.
- `docs/ui/UI_DESIGN.md`, `PROFILE_EDITING.md`, `UI_CLEANUP.md` (à venir).
- `docs/user/GUIDE.md`, `docs/RELEASE_CHECKLIST.md`,
  `docs/validation/MANUAL_VALIDATION.md`,
  `scripts/validation/{nonroot-check,root-check,runtime-system-check}.sh`,
  `scripts/validation/{backup-official,install-replacement,rollback-official}.sh`.
- `crates/tcc-ipc/src/admin.rs`, `crates/tcc-core/src/{config,settings,profile}.rs`,
  `crates/tcc-authz/src/lib.rs`, `crates/tcc-admin/src/{service,transaction,hw}.rs`,
  `crates/tcc-daemon/src/{dbus,main,sleep,shutdown_timer,fan}.rs`,
  `crates/tcc-daemon/src/hardware/{odm,nvidia}.rs`,
  `crates/tcc-gui/src/{daemon_client,profile_edit,settings_edit,power_blocker,autostart}.rs`,
  `crates/tcc-hw/src/v4l2/**`, `crates/tcc-user/src/session_service.rs`.

## 8. Agents / sessions réutilisables

- `qt-ui` (ODM/TDP/cTGP + UX) : `ses_e4f8613f2ffe3EIJkvVHpXw0dC`
- `qt-ui` (nettoyage UI) : `ses_e49b1cfa1ffezGJzD0hUJmSelg`
- `linux-platform` (Admin.Hw TDP) : `ses_e4fd929c0ffeYJesUMWxh5hrJE`
- `linux-platform` (User1 UVC) : `ses_e49b13bb7ffeE8aXEHiJgRvTBo`
- `rust-core` : `ses_e4edb255affeZ0v137v3G4oUZR`
- `security` : `ses_e4f8613f1ffelj2zfgFM9RMQoW`
- `qa-integration` : `ses_e4f357015ffeVEC53E3eKPr3JW`
- `packaging-release` : `ses_e4f357014ffe2feGIaxUqGI3JN`
- `tuxedo-hardware` : `ses_e4f3169ceffe7Yxt01SH77ap8z`

## 9. Règles à ne pas perdre

- **Sous-agents toujours en `background=true`** ; paralléliser les périmètres
  disjoints, sérialiser les écritures de fichiers/contrats/installations.
- Un fichier n'a qu'un propriétaire à la fois ; relire `AGENTS.md`.
- Pas de `/opt`, pas de setuid, pas de shell interpolé, pas de secret en argv.
- Écritures persistantes via `tccd-admin` + polkit ; `tccd` ne persiste que
  dans `/var/lib/tuxedo-rust-center`.
- Toute nouvelle méthode D-Bus d'écriture exige policy + authz + test.
- LUKS reste exclu ; ne pas le réintroduire.
- Tests déterministes, sans root, réseau ni laptop TUXEDO.
