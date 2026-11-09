# Revue de sécurité — veille/reprise (`PrepareForSleep`)

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | revue de l'implémentation veille/reprise de `tccd` (`sleep.rs`, hooks `main.rs`, `FanWorker::suspend`) |
| Périmètre | `docs/security/**` uniquement — aucun code modifié |
| Documents lus | `docs/linux/PLATFORM_DESIGN.md` §2.2.1, `docs/security/REVIEW_HARDWARE.md` (H-01/H-13), `docs/FEATURE_PARITY.md` §9 |
| Code lu | `crates/tcc-daemon/src/sleep.rs`, `main.rs` (§`DaemonSleepHooks`, boucle `select!`), `fan.rs` (§`suspend`/`stop_worker`), `hardware/mod.rs` (§`shutdown`), `watchers.rs` (`read_ppd_initial`/`read_power_initial`), `sensor_lease.rs` (`disarm`) |
| Preuves exécutées | `cargo test -p tcc-daemon --bins sleep` ⇒ **15/15** (statut après correctifs) ; suite daemon **152/152**. Aucun `systemctl suspend` réel, aucun logind injecté, aucun test matériel. |
| Statut | Implémentation **solide sur le nominal**, 4 findings Moyens (annulation/arrêt, reload pendant veille, inhibitor, timeout de reprise), 1 Faible, 4 Info. Aucun Critique/Élevé. **Correctifs 2026-11-06 : SL-01…SL-05 et SL-07/SL-09 fermés ; SL-06/SL-08 restent Info.** |

---

## Statut des correctifs (2026-11-06)

> Instantané après correction. Les constats de §1/§2 restent l'état d'origine. Preuves exécutées :
> `cargo test -p tcc-daemon --bins sleep` ⇒ **15/15** ; suite daemon complète **152/152**. Aucun
> `systemctl suspend` réel, aucun logind réel, aucun matériel.

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **SL-01** | **Fermé** | Workers : `pause` fait `take` + `abort_all()` **synchrone** (`main.rs:121-133`, `hardware/mod.rs:368-381`), test `abort_all_stops_tasks_without_awaiting`. Ventilateurs : `handle.abort()` **avant** toute attente (`fan.rs:660-670`), puis `set_auto` final dans une tâche détachée avec verrou `runtime` (`:677-700`), tests `suspend_aborts_loop_before_auto`, `suspend_returns_fans_to_auto_and_releases_manual`. |
| **SL-02** | **Fermé** | `DeferredReload` (`sleep.rs:147-170`) ; SIGHUP et `Control1.ReloadConfig` diffèrent pendant `is_suspended()` (`main.rs:547-601`), rejoués après reprise (`:653-680`). Test `deferred_reload_coalesces_requests`. |
| **SL-03** | **Fermé** | Inhibitor `sleep/delay` pris **au démarrage** (`main.rs:524-528`), libéré après handoff (`:675-677`), ré-acquis après reprise (`:679-681`) ; repli signal-only si `Inhibit` échoue (`sleep.rs:275-325`), test `inhibitor_fd_is_acquired_and_failure_falls_back`. Sémantique réelle à confirmer par `systemctl suspend`. |
| **SL-04** | **Fermé** | `RESUME_BUDGET=5s` (`sleep.rs:34`) + timeouts PPD/UPower en reprise (`main.rs:226-240`), test `slow_resume_is_bounded` ; **démarrage** encadré par `bounded_read(RESUME_READ_TIMEOUT, ...)` (`main.rs:349-356`). |
| **SL-05** | **Fermé** | `spawn_logind_supervisor` avec back-off 1→30 s (`sleep.rs:329-360`), test `logind_supervisor_stops_when_receiver_is_closed` ; canal fermé géré dans `main.rs` (`sleep_watch_open`, log `closed`). |
| **SL-07** | **Fermé** | `read_preparing_for_sleep` (timeouts 1 s) + `SleepStateMachine::suspended()` au démarrage (`sleep.rs:84-95,250-273`, `main.rs:341-344,494-497`), test `startup_suspended_resumes_on_false` ; `sleeping` initialisé à `initially_suspended` (`main.rs:409`). |
| **SL-09** | **Fermé** | `HardwareManager.sleeping` (`dbus.rs:431,615-628`) refuse les setters pendant la veille ; test `setter_is_refused_while_sleeping` (aucun dialogue, état inchangé). |
| **SL-06** | **Info (inchangé)** | `SIGKILL` pendant un mode manuel reste non couvrable ; documenté. |
| **SL-08** | **Info (inchangé)** | Statut matériel non remis à zéro à la reprise : valeurs d'avant la veille jusqu'au premier tick. |

**Verdict après correctifs** : **aucun bloquant de code** restant. Les trois réserves précédentes sont
fermées (abort fan avant attente, inhibitor pris au démarrage, timeout PPD au démarrage). Il reste la
validation **manuelle** `systemctl suspend` (comportement logind réel, fans, 10 cycles, matériel).

---

## 1. Verdict

La machine à états est **correcte et bien testée** (doublons, annulation, arrêt pendant veille, cycles
répétés). Le branchement logind est **fail-safe** : logind absent ⇒ avertissement unique, reconnexion
toutes les 5 s, daemon vivant. La reprise recrée les workers, redémarre les ventilateurs, réinitialise
le bail capteurs et réapplique le profil : les tests unitaires et le test de hooks le vérifient.

Les risques restants sont **opérationnels**, pas des privilèges :
1. **SL-01** : le budget de 3 s peut annuler `hooks.suspend()` au milieu de l'arrêt ; les workers sont
   consommés/détachés et peuvent survivre à la veille puis cohabiter avec les nouveaux workers.
2. **SL-02** : `SIGHUP` et `Control1.ReloadConfig` **contournent** le garde `is_suspended()` et peuvent
   écrire CPU/backlight pendant `Preparing`/`Suspended`.
3. **SL-03** : `PrepareForSleep` est un signal, pas un inhibitor ; logind peut suspendre avant la fin
   du handoff (risque déjà documenté).
4. **SL-04** : la reprise n'a **aucun budget** et `read_ppd_initial`/`read_power_initial` n'ont pas de
   timeout : un PPD/UPower bloqué gèle la boucle principale après réveil.

---

## 2. Findings

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **SL-01** | **Moyen / I** | `sleep.rs:151-160` (`timeout` puis `suspend_finished`), `main.rs:164-180` (`join!`), `main.rs:121-125` (`pause`), `hardware/mod.rs:363-388` (`shutdown`), `fan.rs:651-689` (`stop_worker`) | Le budget de 3 s **annule le future** `hooks.suspend()` ; `HardwareWorkers::shutdown` consomme `self` et n'aborte les tâches qu'à sa propre échéance interne. Si l'annulation tombe avant, les `JoinHandle` sont **droppés (détachés)**, pas abortés ; `stop` est vrai mais un worker bloqué en `spawn_blocking` peut finir son tick après le réveil. L'état passe quand même à `Suspended`, et `hardware.resume()` démarre de **nouveaux** workers : deux jeux peuvent écrire le matériel. Côté ventilateurs, un `set_auto` final peut précéder une écriture PWM encore en vol. | Rendre `pause()` **non annulable** : `take` + `stop` + `abort_all()` synchrones avant tout `await`, ou conserver les handles pour les abort au timeout ; ne passer à `Suspended` qu'après arrêt/abort effectif. Dans `stop_worker`, `abort()` le handle au timeout puis refaire `set_auto` (dernier écrivain). À la reprise, vérifier qu'aucune ancienne tâche ne tourne avant de démarrer les nouvelles. | `Fake` worker bloquant > budget : suspendre, vérifier `Suspended` seulement après abort, zéro tâche ancienne à la reprise, aucune double écriture ; test 3 s/10 cycles. |
| **SL-02** | **Moyen / I** | `main.rs:494-541` (SIGHUP et `reload_requests`), `main.rs:653-700` (`reload` → `apply_values`/`apply_profile`) ; garde présent seulement `main.rs:464-493` | Les branches `notify` et `tick` vérifient `!sleep_machine.is_suspended()`, mais **SIGHUP** et **`Control1.ReloadConfig`** ne le font pas. `reload()` écrit les réglages/profil (CPU, backlight via `apply_profile`) pendant `Preparing`/`Suspended`, contrairement au commentaire « Pendant la veille, ne pas écrire CPU/backlight ». | Dans les branches SIGHUP/reload, si `is_suspended()` : différer la demande (file) et la rejouer à `Awake`, ou répondre `ok=false`/`ReloadFailed` sans écrire. Auditer `deferred=true`. | Test avec machine en `Suspended` : `Control1`/SIGHUP ⇒ aucune écriture, rapport explicite ; au réveil, rechargement unique. |
| **SL-03** | **Moyen / I** | `sleep.rs:9-20` (doc), `sleep.rs:194-239` (watcher) ; `PLATFORM_DESIGN.md:198-203` | `PrepareForSleep` est un **signal** : logind ne bloque pas la suspension jusqu'à la fin du handler. Le budget de 3 s borne le travail mais **n'empêche pas** le noyau de suspendre avant le retour auto des ventilateurs ou la fin de l'arrêt des workers. Le design l'accepte explicitement ; l'impact (EC en manuel pendant la veille) reste réel. | Prendre un **inhibitor `delay`** (`org.freedesktop.login1.Manager.Inhibit("sleep", "tccd", "hardware handoff", "delay")`) au début de `Preparing`, le libérer après `Suspended`/annulation/reprise ; si `Inhibit` échoue, journaliser et continuer (repli signal-only). `InhibitDelayMaxSec` par défaut (5 s) couvre le budget de 3 s. | `systemctl suspend` avec `dbus-monitor`/`loginctl inhibit` : vérifier le fd d'inhibitor, la libération, et que les ventilateurs sont auto avant la coupure ; test `InhibitDelayMaxSec=1` (dégradé, log). |
| **SL-04** | **Moyen / I** | `main.rs:182-245` (`resume`), `main.rs:190-191` (`read_ppd_initial`/`read_power_initial`), `watchers.rs:45-61` | `hooks.resume()` est attendu **sans timeout** ; les deux lectures PPD/UPower sont des appels zbus **sans timeout**. Un service bloqué (PPD/UPower figé) bloque la boucle `select!` après le réveil : D-Bus/UI gelés, workers non redémarrés, profil non réappliqué. | Encadrer chaque lecture d'un `timeout(2 s)` (repli sysfs/dernière valeur) et `hooks.resume()` d'un budget global (ex. 5 s) ; journaliser `resume_timeout`, continuer avec les workers et réappliquer le profil au tick suivant. | Faux PPD/UPower qui ne répond pas : reprise terminée dans le budget, daemon réactif, profil réappliqué ; test unitaire avec `SleepHooks` lent. |
| **SL-05** | Faible / A | `main.rs:447`, `main.rs:542` ; `sleep.rs:194-239` | Le watcher logind est `tokio::spawn` sans supervision ; si la tâche meurt (panic, bug zbus) ou si le canal se ferme, la branche `Some(start) = sleep_rx.recv()` ignore `None` et la gestion de veille s'arrête **silencieusement** (aucun log, aucune relance). | Garder un `JoinHandle`/superviseur ; sur `None`, `warn!` et relancer le watcher avec back-off borné ; test de fermeture du canal. | Test : fermer `sleep_rx`, vérifier le log et la relance (ou l'arrêt documenté). |
| **SL-06** | Info | `fan.rs:651-689` ; `PLATFORM_DESIGN.md:157-158,198-203` | `SIGKILL`/crash pendant un tick manuel ne peut pas exécuter `set_auto` ; aucun mécanisme in-process ne peut couvrir un `SIGKILL`. Risque résiduel documenté. | Ne pas promettre mieux : documenter, et recommander un retour auto au prochain démarrage (déjà fait par la réconciliation). | Test destructif manuel (hors CI) : `kill -9` pendant 100 %, vérifier l'état au reboot. |
| **SL-07** | Info | `sleep.rs:69` (`Awake`), `main.rs:448` | Un redémarrage de `tccd` **pendant** la veille démarre en `Awake` : il peut appliquer le profil/écrire le matériel alors que la machine est suspendue, et ne verra pas le `false` s'il est arrivé avant son abonnement. | Lire `PreparingForSleep` (propriété logind) au démarrage ou différer la première application tant que logind est en préparation ; test « restart pendant suspend ». | Test manuel : `systemctl stop tccd; systemctl suspend` (ou restart forcé) ; vérifier l'absence d'écriture pendant la veille. |
| **SL-08** | Info | `hardware/mod.rs` `HardwareStatus` ; `main.rs:121-143` | À la reprise, le statut matériel n'est pas remis à zéro : l'UI peut afficher des valeurs d'avant la veille jusqu'au premier tick. Aucun impact sécurité. | Optionnel : publier `unavailable`/`stale` au `pause` et au `resume` jusqu'au premier tick. | Test UI : valeurs grisées après réveil. |
| **SL-09** | Info | `dbus.rs` setters `Manager1` (refusés au bus) | Si les setters charge/éclairage sont réactivés (H-06), ils n'ont **aucun garde veille** : ils pourraient écrire pendant `Preparing`/`Suspended`. Aujourd'hui la policy les refuse, donc risque nul. | Ajouter le même garde `is_suspended()` que la boucle avant toute réactivation des setters. | Test : setter pendant veille simulée ⇒ refus/`Busy`. |

---

## 3. Conformité vérifiée

| Point demandé | Verdict | Preuve / réserve |
|---|---|---|
| **Machine à états** | **Conforme** | `SleepStateMachine` (`sleep.rs:60-126`) ; transitions `Awake→Preparing→Suspended→Resuming→Awake`, `Ignore` sur doublons ; tests `suspend_then_resume_runs_hooks_once`, `duplicate_suspend_is_ignored`, `spurious_and_duplicate_resume_are_ignored`. |
| **Doublons** | **Conforme** | `true` en `Preparing`/`Suspended` ignoré ; `false` en `Awake`/`Resuming` ignoré ; test dédié. |
| **Veille annulée** | **Conforme avec réserve** | `false` en `Preparing` ⇒ `Resume` (`sleep.rs:103-106`) ; test `cancelled_sleep_resumes`. En production, le handler est attendu **inline** dans la boucle : un `false` arrivé pendant `Preparing` est traité **après** la fin du `true`, donc l'état est déjà `Suspended` ; le résultat (reprise) est correct, mais la réaction n'est pas immédiate. |
| **Budget 3 s** | **Conforme pour la borne** | `SUSPEND_BUDGET` (`sleep.rs:32`) + `timeout` (`:151-159`) ; `FanWorker::SUSPEND_WAIT_TIMEOUT=1.5 s` et `HardwareWorkers::SHUTDOWN_TIMEOUT=3 s` en parallèle. Réserve : l'annulation du future laisse des tâches détachées (SL-01). |
| **Arrêt workers / fans** | **Conforme au nominal, partiel en cas de timeout** | `fan.suspend()` : `set_auto` avant et après l'attente (`fan.rs:660-689`) ; `hardware.pause()` prend et arrête les workers (`main.rs:121-125`) ; `shutdown` aborte à l'échéance (`hardware/mod.rs:363-388`). SL-01. |
| **Reprise / réouverture backends** | **Conforme au nominal** | `hardware.resume()` recrée les workers (`main.rs:128-143`), `FanWorker::start` (`:223-233`), test `sleep_hooks_restart_components_and_reset_sensors` ; les fd sont rouverts par la nouvelle détection. Réserve SL-01 (tâches anciennes) et SL-04 (timeout). |
| **Réinitialisation du bail capteurs** | **Conforme** | `sensor_lease.disarm()` en tête de `resume` (`main.rs:185`) ; `SensorLease::disarm` remet `enabled=false` (`sensor_lease.rs:115-119`) ; test `!sensor_lease.is_enabled()`. |
| **Réapplication du profil** | **Conforme** | `refresh_after_resume` écrit les inputs et `force_update` (`main.rs:108-118`), puis `apply_profile` (`:235-244`) ; test `hardware.force_update`. |
| **Pas de fuite de tâches** | **Partiel** | `hardware.shutdown` abort à l'échéance ; mais le `timeout` de `handle_event` peut annuler avant, en détachant les handles (SL-01). Le test `ten_cycles_leave_machine_awake_without_leak` utilise des hooks factices, pas de vraies tâches. |
| **logind absent / erreur** | **Conforme** | `watch_logind` : `warn` unique puis `debug`, reconnexion 5 s, jamais fatal (`sleep.rs:189-225`) ; tests `logind_absence_does_not_terminate_the_watcher`, `logind_watcher_stops_when_receiver_is_closed`. |
| **Arrêt pendant la veille** | **Conforme** | Aucun `false` synthétique ; `shutdown` idempotent ; log dédié (`main.rs:567-574`) ; test `shutdown_while_suspended_never_resumes`. |
| **Pas d'écriture CPU/backlight pendant `Preparing`** | **Partiel** | `notify` et `tick` sont gardés (`main.rs:464-493`) ; **SIGHUP/`Control1` ne le sont pas** (SL-02). Les setters `Manager1` sont refusés au bus aujourd'hui (SL-09 si réactivés). |
| **fd perdus** | **Conforme au nominal** | Workers recréés après reprise ; `FanWorker` redémarré ; première erreur de fd possible, loguée, sans arrêt du daemon (documenté `PLATFORM_DESIGN.md:200-205`). |
| **Risque signal non-inhibitor** | **Accepté, mais réductible** | Documenté `PLATFORM_DESIGN.md:198-203` ; recommandation inhibitor `delay` (SL-03). |
| **Tests** | **Partiels** | 9/9 tests unitaires ; aucun test de budget/annulation réelle, de reprise lente, de logind réel ni de `systemctl suspend`. |

---

## 4. Actions bloquantes avant de déclarer la veille « terminée »

1. **SL-01** — arrêt non annulable : `abort_all()`/handles conservés, état `Suspended` seulement après
   arrêt effectif, vérification qu'aucune ancienne tâche ne tourne à la reprise.
2. **SL-02** — garde `is_suspended()` sur SIGHUP et `Control1.ReloadConfig` (différer ou refuser sans
   écrire).
3. **SL-04** — timeouts sur `read_ppd_initial`/`read_power_initial` et sur `hooks.resume()`.
4. **SL-03** — inhibitor `delay` ou acceptation explicite du risque, avec preuve `systemctl suspend`
   que les ventilateurs sont auto avant la coupure.
5. **Validation manuelle** §5 sur Clevo et Uniwill (le code ne peut pas être validé en CI).

---

## 5. Plan de validation manuelle

| # | Scénario | Vérification | Preuve |
|---|---|---|---|
| 1 | `sudo systemctl suspend` au repos | logs `veille`/`reprise`, fans auto (`pwm_enable`), profil réappliqué, `GetSensorDataCollectionStatus=false` | `journalctl -u tccd`, `sensors`/hwmon, `busctl` |
| 2 | Veille avec courbe manuelle (PWM > 0) | retour auto **avant** la coupure ; pas de PWM manuel au réveil | relevé `fan1_input`/`pwm1` avant/après |
| 3 | Veille annulée (réveil immédiat / `loginctl`) | `false` traité, fans/profil restaurés, pas de double `resume` | logs, état |
| 4 | SIGHUP/`Control1` pendant la veille (simulé) | aucune écriture CPU/backlight, rapport différé/refusé | logs, `strace` |
| 5 | PPD/UPower arrêtés puis veille/reprise | reprise bornée, pas de gel D-Bus, profil réappliqué | logs, `timeout` |
| 6 | 10 cycles veille/reprise | aucune tâche/worker en double (`/proc/<pid>/task`), pas de fd fuité (`/proc/<pid>/fd`) | compteurs avant/après |
| 7 | `systemctl stop tccd` pendant la veille | pas de réveil synthétique, arrêt propre | logs, état systemd |
| 8 | logind masqué/absent | `tccd` démarre, warn unique, pas de crash ; veille non gérée (documenté) | logs |
| 9 | `kill -9` pendant 100 % manuel | risque résiduel : vérifier le retour auto au reboot | relevé manuel |
| 10 | Clevo + Uniwill, AMD + Intel | pas de régression ventilateurs/charge/TDP après reprise | rapport signé |

---

## 6. Limites de la revue

- **Aucune veille réelle exécutée** : les tests utilisent `RecordingHooks` et un faux `hardware`
  (`SysfsRoot` inexistant) ; le timing, les fd réels et le comportement du noyau ne sont pas couverts.
- **Pas d'injection logind** : le watcher est testé en absence/fermeture, pas avec un vrai signal
  `PrepareForSleep` ni un `false` concurrent.
- **SL-01/SL-04** sont des courses de timing : la revue statique les identifie, mais seule une
  validation avec un worker bloqué (faux backend) et un PPD/UPower figé les confirmera.
- **SL-03** dépend de `InhibitDelayMaxSec` et du comportement de logind sur la distribution ; à
  mesurer, pas à supposer.
- **Matériel** : aucun test Clevo/Uniwill/AMD/Intel ; le retour auto des ventilateurs et la
  réouverture de `tuxedo_io`/hwmon/backlight restent à prouver.
- **Setters `Manager1`** : la policy les refuse aujourd'hui ; SL-09 est une condition de leur
  réactivation (H-06), pas un risque actuel.

---

## Annexe — Preuves exécutées

```text
$ cargo test -p tcc-daemon --bins sleep
running 9 tests
test sleep::tests::shutdown_while_suspended_never_resumes ... ok
test sleep::tests::cancelled_sleep_resumes ... ok
test sleep::tests::duplicate_suspend_is_ignored ... ok
test sleep::tests::logind_watcher_stops_when_receiver_is_closed ... ok
test sleep::tests::spurious_and_duplicate_resume_are_ignored ... ok
test sleep::tests::suspend_then_resume_runs_hooks_once ... ok
test sleep::tests::ten_cycles_leave_machine_awake_without_leak ... ok
test tests::sleep_hooks_restart_components_and_reset_sensors ... ok
test sleep::tests::logind_absence_does_not_terminate_the_watcher ... ok
test result: ok. 9 passed; 0 failed
```

Aucun `systemctl suspend`, aucun `loginctl inhibit`, aucun test matériel n'a été exécuté.
