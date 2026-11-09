# Décisions de sécurité — à appliquer dès le jalon 1

Résumé opérationnel de `docs/security/THREAT_MODEL.md`. Ces décisions sont
bloquantes pour l'implémentation et le packaging.

## D1 — Séparer les opérations sensibles dans `tccd-admin`

- `tccd` : daemon root principal, parse des messages D-Bus en continu, applique les profils matériels.
- `tccd-admin` : service root distinct, **activé à la demande**, sans état persistant, uniquement pour :
  Prime, Tomte, WebFAI/paquets, diagnostic, assignation persistante de profil. **LUKS est hors
  périmètre (décision utilisateur).**
- Chaque méthode `tccd-admin` vérifie polkit.
- Objectif : un bug de parsing dans `tccd` ne devient jamais une primitive root sur les opérations
  administratives.

## D2 — État mutable dans `/var/lib`, configuration admin dans `/etc`

- `/etc/tuxedo-rust-center/` : configuration administrateur, **lecture seule** pour le daemon.
- `/var/lib/tuxedo-rust-center/` : profils/settings mutables, root:root 0644, écritures atomiques.
- Permet `ProtectSystem=strict` sans `ReadWritePaths=/etc`.
- Migration : importer `/etc/tcc/*` et les anciens fichiers, sauvegarde `.bak` root-owned 0600.

## D3 — Compatibilité JSON historique gelée

- L'API JSON legacy est en **lecture seule**, taille bornée, schéma figé.
- Aucune nouvelle méthode non typée.
- Un seul setter de compatibilité éventuel : `SetTempProfileByName`.
- Les nouveaux contrats sont typés et versionnés dans `tcc-ipc`.

## D4 — PPD propriétaire de `platform_profile` et de l'EPP

- PPD reste la source de vérité pour `platform_profile` et l'EPP.
- TCC utilise PPD comme signal de bascule et conserve la main sur les limites CPU, cœurs,
  ventilateurs, affichage, webcam, charge.
- Toute dérogation est explicite, optionnelle, journalisée.

## D5 — Règle historique pour tout futur secret (aucune fonctionnalité LUKS)

> **LUKS est hors périmètre (décision utilisateur), non implémenté.** Aucune phrase de passe, aucun
> `cryptsetup`, aucun service ou action polkit LUKS. Cette décision reste la règle pour tout **futur**
> secret, sans plan de travail LUKS.

- Jamais de secret en argv, en environnement, dans une chaîne D-Bus ou dans un log.
- Transport par fd `h` (memfd scellé) si un secret doit un jour être traité ; sinon memfd/`O_TMPFILE`
  0600 + `unlink` immédiat.
- `Zeroizing<Vec<u8>>`, `mlock` best-effort, `LimitCORE=0`, `PR_SET_DUMPABLE=0`.

## D6 — Le daemon ne lit jamais `/proc/*/environ`

- L'agent de session `tccd-user` fournit `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`,
  `XDG_CURRENT_DESKTOP` et l'identité de session.
- Évite la fuite de secrets d'autres processus et la complexité de recherche de session.

## D7 — Overrides de développement verrouillés

- `TCC_SYSFS_ROOT` et `TCC_CONFIG_DIR` sont derrière une feature Cargo `dev-overrides`.
- Feature **absente des paquets de release**.
- Refus si `geteuid() == 0`.
- L'unit systemd ne transmet jamais ces variables.

## Règles système

- Aucun setuid.
- `tccd.service` : `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`,
  `PrivateTmp=yes`, `ProtectProc=invisible`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`,
  `RestrictSUIDSGID=yes`, `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`,
  `RestrictAddressFamilies=AF_UNIX AF_NETLINK`, `CapabilityBoundingSet=` vide,
  `SecureBits=noroot-locked`, `LimitCORE=0`.
- **`ProtectKernelTunables=no`** : sinon `/sys` passe en lecture seule et casse toutes les
  écritures matérielles (ventilateurs, TDP, charge, LED).
- `ReadWritePaths=/var/lib/tuxedo-rust-center /run/tuxedo-rust-center`.
- Vérifier `PrivateDevices` + `DeviceAllow` sur matériel réel ; prévoir un repli documenté.
- Vérifier que `SystemCallFilter=@system-service` ne filtre pas `ioctl`.
- Tester avec `systemd-analyze security`.

## Règles de code

- Aucune commande shell interpolée ; `Command::new` avec arguments séparés.
- Aucun téléchargement exécuté en root.
- Aucune écriture sysfs sans validation de plage et de type.
- `openat2`/`O_NOFOLLOW`, vérification fichier régulier/propriétaire/permissions.
- `#![forbid(unsafe_code)]` par défaut ; `unsafe` isolé, documenté, revu.
- `deny(clippy::unwrap_used)` sur le code de production.
- Toute réponse D-Bus et tout JSON importé sont traités comme non fiables.

## Points à valider

- Politique PPD/TCC si l'utilisateur veut un profil TCC qui contredit PPD.
- `PrivateDevices` + `DeviceAllow` sur les différents modèles TUXEDO.
- Périmètre final de `tccd-admin` (Prime/Tomte/WebFAI/diagnostic/assignation persistante ; **LUKS exclu**).
