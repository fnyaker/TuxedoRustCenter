# Qualité — critères et revues croisées

La qualité prime sur la vitesse. Ce document fixe les portes de qualité et la
matrice de revue entre agents.

## 1. Portes de qualité

### Code Rust

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Aucun `unwrap()`/`expect()` dans le code de production.
- Aucun `unsafe` non isolé, non commenté et non testé.
- Erreurs typées, timeouts, dégradation propre.
- Documentation des APIs publiques et des chemins sysfs/ioctl.

### Matériel

- Tests avec racines sysfs factices et `FakeTransport`.
- Aucun test CI ne dépend d'un laptop TUXEDO.
- Toute écriture sysfs validée (plage, type, longueur, propriétaire).
- Comportement défini si le matériel, le driver ou le capteur est absent.
- Smoke test matériel réel documenté pour chaque fonctionnalité (Clevo, Uniwill,
  AMD, Intel, NVIDIA).
- Pas de polling permanent inutile ; event-driven par défaut.

### D-Bus

- Contrat versionné (`tcc-ipc`), validation des entrées/sorties.
- Toute nouvelle méthode documentée dans `docs/DBUS_API.md`.
- Aucune méthode d'écriture non authentifiée pour les données persistantes ou
  sensibles.
- Les writes persistants passent par `tccd-admin` + polkit (D1/D2).
- Test sur bus de session et test d'entrées invalides.

### GUI Qt 6

- `qmllint` sans erreur (faux positifs documentés).
- `qmltestrunner` : tests design system et adaptateur LiveData.
- Aucun composant web, aucun secret, aucune logique privilégiée.
- Accessibilité clavier, contraste, HiDPI, i18n FR/EN/DE, °C/°F.
- Parité des clés de traduction testée.
- Mode dégradé propre si le daemon est absent ou renvoie une erreur.

### Packaging

- Aucun fichier dans `/opt`.
- Chemins FHS exacts et permissions vérifiées.
- `systemd-analyze verify` et `systemd-analyze security`.
- `desktop-file-validate`, `appstreamcli validate`, `xmllint`.
- Install/désinstall dans un conteneur propre.
- Migration/cohabitation TCC documentée et testée.
- Dépendances runtime justes, pas de dépendance dure inutile.

### Sécurité

- Revue `security` obligatoire pour tout nouveau chemin privilégié, méthode
  D-Bus d'écriture, unit systemd, règle udev/polkit ou dépendance.
- Pas de régression sur D1–D7.
- `cargo audit` et `cargo deny` dans la CI.
- Fuzzing/property testing des entrées JSON/D-Bus critiques.

## 2. Matrice de revue

| Changement | Auteur | Relecture obligatoire |
|---|---|---|
| `tcc-hw` / ioctl / sysfs | `tuxedo-hardware` | `security` + `linux-platform` |
| Daemon / D-Bus / PPD | `linux-platform` | `rust-core` + `security` |
| Modèles/config/performance | `rust-core` | `qa-integration` |
| GUI/QML/design | `qt-ui` | `qa-integration` + `rust-core` (pont) |
| Tests/fixtures/CI test | `qa-integration` | propriétaire du code testé |
| Packaging/systemd/polkit | `packaging-release` | `security` + `linux-platform` |
| Documentation sécurité | `security` | `linux-platform` / `packaging-release` |

Une revue n'est pas un simple « OK » : elle cite les fichiers, les tests exécutés
et les risques résiduels.

## 3. Preuves attendues dans un rapport d'agent

- Fichiers créés/modifiés.
- APIs publiques et contrats D-Bus.
- Commandes exécutées et résultats (`cargo test`, clippy, fmt, QML, packaging).
- Tests matériels réels effectués, ou explicitement « non testé sur matériel ».
- Risques résiduels et procédure de validation sur laptop TUXEDO.
- Points d'intégration pour les autres agents.

## 4. Budgets de performance

À mesurer, pas à supposer :

- daemon inactif : < 1 % CPU, < 30 Mo RSS ;
- GUI : < 200 Mo RSS, démarrage < 1,5 s ;
- pas de polling capteurs si aucun client ne le demande ;
- boucle ventilateurs : 1 s, lectures minimales ;
- état PPD/batterie : événements + fallback 5 s ;
- pas de fuite de tâches/threads après changement de profil ou veille.

## 5. Critères de sortie de jalon

Un jalon est terminé lorsque :

1. les fonctionnalités sont conformes à `docs/FEATURE_PARITY.md` ou les écarts
   sont documentés ;
2. les portes de qualité ci-dessus passent ;
3. les tests d'intégration couvrent les cas nominaux et les entrées invalides ;
4. `security` a relu les surfaces privilégiées ;
5. `packaging-release` a validé le FHS et l'installation ;
6. `docs/STATUS.md` est à jour avec ce qui a réellement été testé.
