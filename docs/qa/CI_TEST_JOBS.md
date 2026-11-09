# CI de test — jobs de référence

Ce document **propose** des évolutions pour `.github/workflows/ci.yml`. Il ne
modifie pas le workflow : `rust-core` en est propriétaire, `packaging-release`
valide les jobs de release. Les commandes ont été exécutées localement le
2026-11-06 (voir `docs/qa/TEST_PLAN.md`).

> **Mise à jour** : `rust-core` a intégré les jobs **`integration`** et
> **`qml`** dans `.github/workflows/ci.yml`. Le job `integration` exécute
> `dbus-run-session -- cargo test -p tcc-daemon --locked` (et le job Arch fait
> de même) ; le job `qml` tourne dans un **conteneur Arch** avec Qt 6 /
> `qmltestrunner` et `crates/tcc-gui/tools/qml-check.sh`. Les blocs YAML
> ci-dessous restent la référence de conception ; ne pas les recopier tels
> quels si le workflow évolue.

Objectif : rendre explicites les tests qui aujourd'hui peuvent être sautés,
ajouter les tests d'intégration QA et exécuter les tests QML avec le bon
`qmltestrunner` (Qt 6, pas Qt 5).

---

## 1. Constats sur le workflow actuel

- `cargo test --workspace` couvre les tests Rust mais **pas** les tests QML :
  le job `qml` dédié exécute désormais Qt Quick Test dans un conteneur Arch.
- Le test D-Bus de `tcc-daemon` se termine avec succès quand aucun bus de
  session n'est disponible. Les jobs `integration` et Arch utilisent
  maintenant `dbus-run-session`, donc il s'exécute réellement.
- `qmltestrunner` du `PATH` peut être Qt 5 ; le job QML utilise le binaire Qt 6
  du conteneur Arch (`/usr/lib/qt6/bin/qmltestrunner`).
- Le job Arch « sans Qt » reste limité aux crates sans Qt ; les tests QML ont
  leur propre job conteneurisé.
- Les tests D-Bus de `tcc-admin` (`admin_dbus`) sont lancés hors
  `dbus-run-session` dans les jobs actuels : ils peuvent être sautés faute de
  bus de session. Recommandation : `dbus-run-session -- cargo test -p tcc-admin`
  (ou regrouper avec le daemon/GUI).
- Aucun job ne mesure la couverture ni ne fuzz ; ces ajouts peuvent être
  progressifs.

---

## 2. Job « tests d'intégration QA » (Ubuntu)

```yaml
  integration:
    name: Intégration (fixtures, D-Bus, CLI)
    runs-on: ubuntu-latest
    timeout-minutes: 25
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Dépendances D-Bus
        run: |
          sudo apt-get update
          sudo apt-get install -y dbus
      - name: Crates stables (sans Qt)
        run: |
          cargo test --no-fail-fast \
            -p tcc-core -p tcc-ipc -p tcc-hw -p tcc-user -p tcc-cli
      - name: Overrides de développement
        run: cargo test -p tcc-core --features dev-overrides
      - name: D-Bus daemon et admin sur bus de session dédié
        run: dbus-run-session -- cargo test -p tcc-daemon -p tcc-admin
```

Notes :

- `dbus-run-session` garantit que `dbus::tests::serves_methods_on_session_bus`
  s'exécute réellement au lieu de retourner tôt.
- Aucun test ne touche le bus système ni `/sys` réel.

---

## 3. Job « QML » (Ubuntu)

```yaml
  qml:
    name: QML (Qt Quick Test)
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Installer Qt 6 et les modules QML
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            qt6-base-dev qt6-declarative-dev qt6-declarative-dev-tools \
            qml6-module-qtquick qml6-module-qtquick-controls \
            qml6-module-qtquick-layouts qml6-module-qtquick-shapes \
            qml6-module-qttest libgl1-mesa-dev
      - name: Tests QML
        env:
          QT_QPA_PLATFORM: offscreen
        run: |
          /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml
```

Notes :

- Les noms de paquets sont ceux d'Ubuntu 24.04 ; `packaging-release` doit les
  vérifier sur les images cibles.
- Si `qmltestrunner` Qt 6 est ailleurs, utiliser
  `$(qtpaths6 --query QT_INSTALL_BINS)/qmltestrunner`.
- Ce job ne nécessite pas de serveur X grâce à `offscreen`.

---

## 4. Extension du job Arch

```yaml
  test-arch:
    name: Tests (Arch Linux)
    runs-on: ubuntu-latest
    container: archlinux:latest
    steps:
      - name: Installer Rust
        run: pacman -Syu --noconfirm --needed rust
      - uses: actions/checkout@v4
      - name: Crates sans Qt
        run: |
          cargo test --no-fail-fast -p tcc-core -p tcc-ipc -p tcc-hw \
            -p tcc-user -p tcc-cli
```

Aujourd'hui le job ne teste que `tcc-core` et `tcc-ipc` ; `tcc-hw`, `tcc-user`
et `tcc-cli` n'ont pas besoin de Qt et peuvent être ajoutés. `tcc-daemon`
demande D-Bus (`dbus` est présent dans l'image de base Arch, à vérifier).

---

## 5. Couverture (optionnel, non bloquant)

```yaml
  coverage:
    name: Couverture (information)
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    timeout-minutes: 30
    continue-on-error: true
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - run: cargo install cargo-llvm-cov --locked
      - name: Couverture des crates sans Qt
        run: |
          cargo llvm-cov --no-report \
            -p tcc-core -p tcc-ipc -p tcc-hw -p tcc-user -p tcc-cli
          cargo llvm-cov report --summary-only
```

Tant que les seuils ne sont pas stabilisés, le job publie un résumé sans
échouer la PR. Un seuil par crate pourra être ajouté après deux semaines de
mesures.

---

## 6. Fuzzing (proposé, nightly)

```yaml
  fuzz:
    name: Fuzzing (nightly, non bloquant)
    if: github.event_name == 'schedule'
    runs-on: ubuntu-latest
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo install cargo-fuzz --locked
      - name: Campagne courte
        run: |
          for target in fuzz_legacy_settings fuzz_legacy_profiles \
                        fuzz_state_snapshot fuzz_profile_collection; do
            cargo fuzz run "$target" -- -max_total_time=600 -max_len=1048576
          done
```

Prérequis : un crate `fuzz/` (ou `crates/tcc-fuzz`) avec `libfuzzer-sys`, créé
par `rust-core` (propriétaire des manifestes). Les cibles sont listées dans
`docs/qa/TEST_PLAN.md` section 8. Les crashs doivent être transformés en
fixtures de régression déterministes.

---

## 7. Gate de performance (à ne pas activer tout de suite)

Les budgets `< 1 % CPU` / `< 30 Mo RSS` / `< 1,5 s` sont des objectifs, pas des
mesures stables sur runner partagé. Recommandation :

1. mesurer manuellement sur laptop TUXEDO (`docs/qa/HARDWARE_SMOKE_TESTS.md`) ;
2. collecter au moins 10 mesures release ;
3. seulement ensuite, ajouter un job informatif (pas bloquant) avec une
   tolérance large (par exemple +100 % du budget avant échec).

---

## 8. Matrice des jobs cible

| Job | Qt | D-Bus | Root | Matériel | Bloquant |
|---|---|---|---|---|---|
| `fmt` | non | non | non | non | oui |
| `clippy` | oui | non | non | non | oui |
| `test` | oui | non | non | non | oui |
| `integration` | non | session | non | non | oui |
| `qml` | oui | non | non | non | oui |
| `test-arch` | non | session | non | non | oui |
| `msrv` | oui | non | non | non | oui |
| `deny` / `audit` | non | non | non | non | oui |
| `coverage` | non | non | non | non | non |
| `fuzz` | non | non | non | non | non (nightly) |
| smoke matériel | oui | oui | oui | **oui** | manuel, hors CI |

---

## 9. Points à valider avec les propriétaires

- `rust-core` : jobs `integration` et `qml` intégrés ; reste à valider cache,
  timeouts et évolutions (couverture/fuzzing).
- `packaging-release` : disponibilité de `qt6-declarative-dev-tools` et des
  modules QML sur les images cibles.
- `linux-platform` : exécution du test D-Bus sous `dbus-run-session` (aucun
  conflit avec le TCC original).
- `security` : vérifier qu'aucun job ne prend de privilège et que le fuzzing
  n'exécute pas de code arbitraire hors conteneur.
