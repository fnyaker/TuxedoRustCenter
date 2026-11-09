# Checklist de release — TuxedoRustCenter

Statut : à cocher avant toute publication. Les preuves locales et les limites
connues sont dans `docs/STATUS.md`, `docs/HANDOFF.md` et
`docs/security/REVIEW_*.md`.

Légende : `[x]` vérifié localement/CI, `[ ]` à faire, `[~]` partiel.

---

Runbook détaillé : `docs/validation/MANUAL_VALIDATION.md`.
Scripts : `./scripts/validation/nonroot-check.sh` et
`sudo ./scripts/validation/root-check.sh`.

## 1. Build et qualité

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `cargo test --workspace --quiet`
- [x] `cargo build --release --workspace`
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- [x] QML : `crates/tcc-gui/tools/qml-check.sh` (`qmllint` + `qmltestrunner`)
- [ ] `cargo deny check` (CI)
- [ ] `cargo audit` (CI)
- [ ] MSRV `1.85` vérifiée avec la toolchain dédiée (CI)
- [ ] `cargo llvm-cov` ou équivalent (facultatif, non installé localement)

## 2. Sécurité

- [ ] `docs/security/REVIEW_JALON1.md` : aucun finding Critique/Élevé ouvert.
- [ ] `docs/security/REVIEW_HARDWARE.md` : findings H-03/H-13 (validation
      matérielle) traités ou explicitement acceptés.
- [ ] `docs/security/REVIEW_ADMIN_PHASE1.md` : A1-01..A1-11 fermés, addendum
      GUI G1-01..G1-09 fermés, addendum final à jour.
- [ ] `docs/security/THREAT_MODEL.md` : delta jalon 2 relu après les derniers
      changements (settings, veille).
- [~] V4L2 lecture seule implémentée et revue (`REVIEW_V4L2.md`) ; écriture
      hors jalon 2 ; `DeviceAllow=char-video4linux r` à valider.
- [ ] Revues `REVIEW_MANAGER1_AUTHZ.md` (M1) et `REVIEW_SLEEP.md` (SL)
      rouvertes après correctifs, puis policy `Manager1` réouverte.
- [ ] Aucun `unwrap`/`expect` en production (`cargo clippy` + grep).
- [ ] Aucun `unsafe` hors wrappers documentés et revus.
- [ ] Aucune commande shell interpolée ; allowlist et arguments séparés.
- [x] Aucun secret en argv/env/D-Bus ; fonction LUKS **hors périmètre** (décision utilisateur, action polkit retirée).
- [ ] Aucun faux formulaire LUKS dans la GUI (`rg 'luksCurrent|luksNew|luksConfirm'` vide).

## 3. `tccd-admin` phase 1 (activation `TCC_WITH_ADMIN=1`)

- [ ] Vrai polkit sur bus système :
  - [ ] autorisé ;
  - [ ] refusé ;
  - [ ] annulé (dialogue fermé) ;
  - [ ] polkit arrêté/indisponible.
- [ ] `dbus-policy-test.sh` exécuté en root : `Ping`, `GetVersion`,
      `GetContractVersion`, `SaveProfilesJSON`, `SaveSettingsJSON`,
      `ReloadConfig` ; refus `Properties.*`, phases 2/3, setters `Manager1`.
- [ ] `SaveProfilesJSON` : `.bak` 0600, cible 0644, presets filtrés,
      références croisées, `InUse`.
- [ ] `SaveSettingsJSON` : champs `extra`/`stateMatrix`/charge préservés, seuls les 4 interrupteurs modifiés.
- [ ] Validation croisée admin : fichier corrompu refusé **sans** quarantaine (`load_*_no_recovery`).
- [ ] Création/duplication/suppression de profils : IDs/noms uniques, presets refusés, `InUse`, conflit.
- [ ] Tombstones : suppression → `deletedProfileIds`, recréation du même ID refusée, transaction profils+settings, rollback des deux.
- [ ] Rollback réel si `Control1.ReloadConfig` échoue (`ok=false`, timeout,
      `tccd` arrêté) : contenu précédent restauré, pas de `.restore.*`.
- [ ] Un seul enregistrement à la fois (`Busy`), pas de perte de commande.
- [ ] Audit journald : `caller_uid`, action, résultat, `backup`, `reload`,
      aucune valeur/payload/chemin.
- [ ] Sortie après 30 s d'inactivité ; unité inactive au boot ;
      `systemd-analyze security` ≤ 2.5.
- [ ] `Properties.Get/GetAll` refusés sur `tccd` et `Admin` ;
      `Manager1.GetContractVersion` autorisé.
- [ ] Deux clients GUI : conflit détecté, pas d'écrasement silencieux
      (résidu de course documenté).

## 4. Matériel réel TUXEDO

Suivre `docs/qa/HARDWARE_SMOKE_TESTS.md` :

- [ ] CPU : online cores, governor, min/max, EPP, `no_turbo`, reapply.
- [ ] Ventilateurs : tuxi, pwm, tuxedo-io ; courbes, min/max/offset,
      hystérésis, retour auto, arrêt/reprise après veille.
- [ ] Charge : profil, priorité, seuils, type ; valeurs disponibles.
- [ ] Éclairage : blanc, RGB, per-key, lightbar, modes.
- [ ] ODM/`platform_profile` : PPD propriétaire, override explicite.
- [ ] TDP PL1/PL2/PL4 : lecture, validation, non-régression.
- [ ] NVIDIA cTGP : lecture, limites.
- [ ] PRIME : état iGPU/dGPU/on-demand ; bascule réservée admin (phase 2).
- [ ] Webcam : kill switch, énumération, UVC (si implémenté).
- [ ] YCbCr 4:2:0 : debugfs, disponibilité, non-régression.
- [ ] Fn-Lock : lecture/écriture selon disponibilité.
- [ ] Refresh rate : X11 + Hyprland/Sway/KDE ; écran externe jamais modifié.
- [x] Veille/reprise : `systemctl suspend` réel validé (fans auto, workers recréés, profil réappliqué).
- [ ] `DeviceAllow=/dev/tuxedo_io`, ordre de chargement du module.
- [ ] Aucun panic si capteur/backend absent.

## 5. Packaging et FHS

- [ ] `validate.sh --repo`, `--tree` défaut et `--with-admin`.
- [ ] deb : `lintian` 0 erreur, install/purge dans `debian:trixie`.
- [ ] rpm : `rpmlint` 0 erreur, install/remove dans Fedora.
- [ ] Arch : `namcap` 0 erreur, install/remove dans Arch.
- [ ] Aucun fichier dans `/opt`, aucun setuid, aucun runtime Node/Python.
- [ ] `systemd-analyze verify` sur `tccd`, `tccd-user`, `tccd-admin`.
- [ ] `desktop-file-validate`, `appstreamcli validate`, `xmllint`.
- [ ] `udevadm verify`, `systemd-tmpfiles --dry-run`.
- [x] `tccd-admin` absent des paquets par défaut ; paquet avec admin construit/installé/validé.
- [ ] `tccd-sleep.service` absent (remplacé par `PrepareForSleep`).

## 6. Migration / cohabitation TCC original

- [ ] `/etc/tcc` importé au premier démarrage, aucune suppression.
- [ ] Service `tccd.service` original arrêté/désactivé avant installation.
- [ ] Nom D-Bus `com.tuxedocomputers.tccd` libre.
- [ ] Postinst refuse d'activer si `/opt/tuxedo-control-center` subsiste.
- [ ] Désinstallation propre, configuration conservée/purgée selon package.
- [ ] Test de coexistence documenté dans `docs/packaging/MIGRATION.md`.

## 6bis. Tray et session

- [ ] Tray native : icône, menu, profils temporaires, autostart, quitter.
- [ ] Autostart `~/.config/autostart/tuxedo-rust-center.desktop` avec `--tray`.
- [ ] Dépendances Qt : `Qt6Widgets`, `Qt.labs.platform`, `QtQuick.Dialogs` installées.
- [ ] Power save blocker : logind `idle/block`, libéré à la fermeture.
- [ ] Tray indisponible (GNOME sans extension) : fenêtre normale, pas de crash.

## 7. Performance et accessibilité

- [x] Daemon inactif : 0,33 % CPU / 15,1 MiB RSS (dev, capteurs off).
- [~] GUI Xvfb logicielle : 127 MiB / 0,6 % CPU, démarrage ~300 ms ; backend GL/llvmpipe 267 MiB à re-mesurer sur GPU réel.
- [ ] Aucun polling capteurs sans client.
- [ ] Ventilateurs : boucle 1 s, lectures minimales.
- [ ] Aucune fuite de tâches/threads après profils, veille, admin.
- [ ] Navigation clavier, focus visible, contraste, HiDPI.
- [ ] Parité i18n FR/EN/DE (test automatique).
- [ ] °C/°F appliqué aux jauges et seuils.

## 8. Documentation

- [ ] `docs/STATUS.md` reflète les preuves réelles.
- [ ] `docs/FEATURE_PARITY.md` sans `[x]` non prouvé.
- [ ] `docs/HANDOFF.md` à jour pour la reprise.
- [ ] `docs/DBUS_API.md` couvre `Manager1`, `Control1`, `Admin1`.
- [ ] `docs/packaging/MIGRATION.md` et `SECURITY.md` à jour.
- [x] Guide utilisateur initial (`docs/user/GUIDE.md`) ; à relire après les dernières fonctions.
- [ ] Changelog et licence GPL-3.0-or-later présents dans les paquets.

---

## Commandes de référence

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --quiet
cargo build --release --workspace
cd crates/tcc-gui && ./tools/qml-check.sh
./scripts/release/validate.sh --repo
./scripts/release/validate.sh --tree build/pkgroot
./scripts/release/dbus-policy-test.sh   # root
systemd-analyze security --offline=true packaging/systemd/tccd.service
```
