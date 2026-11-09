# Plan d'intégration — Jalon 1 → Jalon 2

Ce document décrit comment assembler les travaux des agents et ce qui doit être vrai
avant de passer au jalon 2.

## 1. Frontières de modules

```
tcc-core  ←  tcc-ipc  ←  tcc-daemon  ←  tcc-gui
   ↑                         ↑              │
   └──────── tcc-hw ────────┘              │ actions sensibles
                                             ▼
                                      tccd-admin (Prime/
                                      Tomte/WebFAI, à la demande)
                             ↑
                         tcc-user (refresh rate)
```

- `tcc-core` : aucune dépendance matérielle, aucun D-Bus, aucun Qt.
- `tcc-ipc` : types sérialisables partagés, aucune dépendance à `tcc-hw`/Qt.
- `tcc-hw` : accès noyau, types locaux ; conversions vers `tcc-core` documentées.
- `tcc-daemon` : orchestration, D-Bus, workers, persistance de l'état mutable dans `/var/lib`.
- `tcc-admin` : binaire root séparé, activé à la demande, pour Prime/Tomte/WebFAI/diagnostic ; LUKS exclu.
- `tcc-user` : agent de session, backends compositeur.
- `tcc-gui` : Qt 6, client D-Bus, aucune logique privilégiée.

## 2. Contrats à stabiliser

### `tcc-core` → `tcc-daemon`

- `Profile`, `Settings`, `StateKey`, `PpdProfile`, `PowerSource`.
- `StateResolver::resolve(&self, input) -> ResolvedProfile`.
- `ConfigStore::load()/save()`.
- `validate_profile`, `validate_settings`.

### `tcc-core` → `tcc-gui` (via `tcc-ipc`)

- Snapshot de profil actif.
- Liste des profils preset/custom.
- Snapshot d'état `(ppd, power, active_profile_id)`.
- Info refresh rate : modes disponibles, support du backend, erreur éventuelle.

### `tcc-hw` → `tcc-daemon`

- `PowerStateReader` (AC/batterie).
- `CpuControl`, `BacklightControl`, `FanControl`, `ChargingControl`, etc.
- `TuxedoIo` pour les ioctls.
- `DisplayModeBackend` est consommé par `tcc-user`, pas par le daemon.

### `tcc-daemon` ↔ `tcc-user`

Le daemon expose le profil actif et les changements. L'agent de session applique
uniquement la partie affichage. Contrat minimal :

- lecture du profil actif ;
- signal de changement ;
- demande d'application de mode avec identifiant de profil et output interne ;
- accusé de réception/état.

## 3. Démo de fin de jalon 1

Sans matériel TUXEDO réel, avec un sysfs factice :

1. Démarrer `tccd` en mode `dev-overrides` (jamais en root) avec `TCC_SYSFS_ROOT=tests/fixtures/sys`.
2. Charger des profils migrés depuis l'original.
3. Simuler `PPD=balanced` + `AC` → profil Office.
4. Simuler `PPD=balanced` + `Battery` → profil Quiet.
5. Simuler `PPD=performance` + `AC` → High Performance.
6. Simuler la disparition de PPD → fallback `stateMap`.
7. Vérifier qu'un override manuel est conservé jusqu'au prochain changement d'état.
8. Vérifier que la config est écrite atomiquement et rechargée.
9. Lancer le shell QML avec des données factices et vérifier la navigation.

## 4. Portes de qualité

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo deny check` (une fois ajouté)
- `cargo audit` (une fois ajouté)
- Aucun `unsafe` non documenté.
- Aucun `unwrap()`/`expect()` dans le code de production.
- Aucune commande shell interpolée.
- Aucun chemin `/opt` dans le packaging.
- Aucun composant web dans la GUI.
- Aucun secret en argv/env/chaîne D-Bus (LUKS non implémenté).
- `ProtectKernelTunables=no` et `ProtectSystem=strict` dans l'unit `tccd`.
- `/etc` non écrit par le daemon ; état mutable dans `/var/lib`.
- Overrides dev derrière `dev-overrides`, refusés en root.

## 5. Deuxième vague d'agents

Une fois le jalon 1 assemblé :

1. **`linux-platform`** implémente `tcc-daemon` (config réelle, watchers PPD/UPower,
   service D-Bus, application CPU/backlight/fan), `tcc-admin` (squelette Prime/Tomte/WebFAI/diagnostic)
   et `tcc-user` (X11/Hyprland).
2. **`qt-ui`** branche le shell QML sur `tcc-ipc` via un client D-Bus Rust.
3. **`rust-core`** ajoute les tests d'intégration, les benchmarks, la CI et les
   optimisations.
4. **`security`** audite les premiers chemins privilégiés, la politique D-Bus et
   le hardening systemd.

## 6. Risques d'intégration

- Modèles `tcc-hw` trop liés au matériel : prévoir des conversions explicites.
- Divergence entre les types `tcc-core` et les payloads D-Bus : `tcc-ipc` doit être
  la seule frontière.
- Refresh rate Wayland : ne pas bloquer le daemon si l'agent de session est absent.
- PPD/TCC : ne pas écrire `platform_profile`/EPP en conflit ; documenter la politique.
- GUI Qt : ne pas laisser de logique métier dans QML.
