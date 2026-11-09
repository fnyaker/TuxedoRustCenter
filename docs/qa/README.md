# QA — index

| Document | Contenu |
|---|---|
| [`TEST_PLAN.md`](TEST_PLAN.md) | Stratégie, matrice de couverture, conventions, fixtures, trous, fuzzing, performance. |
| [`CI_TEST_JOBS.md`](CI_TEST_JOBS.md) | Jobs CI proposés (non appliqués) pour `rust-core`. |
| [`HARDWARE_SMOKE_TESTS.md`](HARDWARE_SMOKE_TESTS.md) | Smoke tests manuels sur laptop TUXEDO (hors CI). |
| [`BUGS.md`](BUGS.md) | Bugs corrigés (historique non-régression, dont BUG-QA-003) et observations. |
| [`../../tests/fixtures/README.md`](../../tests/fixtures/README.md) | Description des fixtures legacy et D-Bus. |

## Commandes rapides

```sh
cargo test -p tcc-core -p tcc-ipc -p tcc-hw -p tcc-user -p tcc-cli
cargo test -p tcc-admin
dbus-run-session -- cargo test -p tcc-daemon -p tcc-admin -p tcc-gui
cargo test -p tcc-core --test known_issues
crates/tcc-gui/tools/qml-check.sh
scripts/release/validate.sh --repo
```

## Règles

- La QA ne modifie pas le code de production.
- Un bug = un test minimal + une fiche dans `BUGS.md` ; le test reste actif
  après correction, ou `#[ignore]` documenté tant que le correctif n'est pas
  livré.
- Aucun test ne dépend d'un laptop TUXEDO, de root, du réseau ou du bus système.
- **LUKS est hors périmètre** (décision utilisateur) : aucun test fonctionnel,
  aucun formulaire/champ/secret ; seule l'absence d'identifiants GUI est
  vérifiée. Tray/session : tests QML `offscreen`, sans tray réel ni logind.
- Les totaux de tests courants sont dans `docs/STATUS.md`, pas recopiés ici.
