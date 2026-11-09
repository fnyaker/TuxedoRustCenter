# Revue de sécurité — tray Qt et autostart

| Champ | Valeur |
|---|---|
| Version | 1.0 |
| Date | 2026-11-06 |
| Auteur | agent `security` |
| Objet | icône tray native SNI, menu, fermeture/masquage, autostart XDG et glue `QApplication` |
| Périmètre | `docs/security/**` uniquement — aucun code ni packaging modifié |
| Code lu | `crates/tcc-gui/qml/tray/SystemTray.qml`, `qml/Main.qml`, `qml/components/TrayMenu.qml`, `qml/core/{AutostartData,PowerBlockerData,LiveData,AppState}.qml`, `qml/pages/SettingsPage.qml` ; `src/main.rs`, `src/autostart.rs`, `src/daemon_client.rs` (worker, `setAutostart`, `setPowerBlocker`, `setTempProfile`), `src/qapplication_shim.cpp`, `build.rs` ; `docs/ui/UI_DESIGN.md`, `docs/DEV_SETUP.md` ; `distro/{deb/control.in,rpm/*.spec,arch/PKGBUILD}` |
| Preuves exécutées | `cargo test -p tcc-gui --bins` ⇒ **153/153** ; `autostart::tests` ⇒ **11/11** ; `main::tests` ⇒ **3/3** (dont `--tray` et arguments inconnus). Tests QML `tst_tray.qml`/`tst_autostart.qml` **présents mais non exécutés** dans cet environnement (`qmltestrunner` sans rapport). **Non testés** : SNI réel, GNOME sans extension AppIndicator, Wayland/X11 réel, fermeture/quit via `Main.qml`, connexion/déconnexion de l'hôte SNI, autostart au login réel. |
| Verdict | **Aucun Critique/Élevé/Moyen/Faible ouvert.** T2-01, T2-02, T2-04, T2-05, T2-06 et T2-08 sont **fermés** (code + tests). T2-03 (symlinks d'ancêtres/`openat2`) et le suffixe PID de T2-04 sont **acceptés** pour un autostart utilisateur non privilégié ; T2-07 (accessibilité/extension GNOME) reste **Info**. Aucune logique privilégiée, aucune commande shell, aucun secret. |

---

## 1. Findings

> **Instantané initial.** Le statut courant est en §1.1 ; les sévérités ci-dessous décrivent
> l'état avant correctifs.

| ID | Sév. | Fichier:ligne | Constat | Impact | Remédiation | Test |
|---|---|---|---|---|---|---|
| **T2-01** | **Faible / B** | `qml/tray/SystemTray.qml:29-36` ; `qml/Main.qml:135-140` | `AppState.trayAvailable` n'est renseigné qu'au `Component.onCompleted` ; aucun `onAvailableChanged`. Si l'hôte SNI disparaît après le démarrage pendant que la fenêtre est masquée, l'icône disparaît mais `onClosing` continue de masquer la fenêtre. Inversement, si l'hôte apparaît après un `--tray` sans tray, la fenêtre a été affichée et `onClosing` quittera l'application. | Application inaccessible ou comportement de fermeture incohérent ; pas de privilège. | Mettre à jour `AppState.trayAvailable` sur `availableChanged` ; si `available=false` et fenêtre masquée, la réafficher ; si `available=true` en `--tray`, appliquer la politique attendue. | Test QML avec disponibilité simulée (vrai/faux après chargement), fermeture associée. |
| **T2-02** | **Moyen / B (conditionnel)** | `src/autostart.rs:165-175` | `create_dir_all(directory)` ne fixe ni mode ni propriétaire ; le répertoire `autostart/` prend `0777 & umask`. Avec un umask `002`/`000` (ou un répertoire préexistant `0775`/`0777`), un autre utilisateur local du même groupe peut déposer un `.desktop` exécuté à la prochaine ouverture de session de la victime. Le fichier écrit, lui, est bien `0644` (`:194-196`). | Élévation locale conditionnelle (autre utilisateur → code au login de la victime), pas d'escalade root. Nécessite un umask permissif et un home traversable. | Créer `autostart/` en `0700`/`0755` explicite, `set_permissions`, vérifier `uid == geteuid()` et `mode & 0o022 == 0` pour le répertoire et ses ancêtres ; refuser sinon. | Test avec umask `000`/`002` et répertoire préexistant `0777` ⇒ mode final sûr ou refus. |
| **T2-03** | Info / B | `src/autostart.rs:80-93,170-196` | `starts_with(HOME)` est **lexical** : `$XDG_CONFIG_HOME` (ou un ancêtre) peut être un lien symbolique vers l'extérieur de `$HOME`. `write_entry` refuse un lien sur `autostart/` et sur le fichier cible (`:170-179`), mais ne résout pas les ancêtres et reste exposé à une course entre `symlink_metadata` et `create_dir_all`/`rename` (TOCTOU). | Écriture hors du `$HOME` réel si la configuration utilisateur pointe ailleurs ; nécessite le contrôle du répertoire de config ; aucune escalade. | `openat2(RESOLVE_BENEATH\|RESOLVE_NO_SYMLINKS)` ou `realpath` + revalidation sous `$HOME` ; `O_NOFOLLOW`/`openat` pour chaque composant ; documenter le cas `XDG_CONFIG_HOME` symlink. | Test `XDG_CONFIG_HOME` symlink hors HOME ⇒ refus ; test course non déterministe. |
| **T2-04** | Info | `src/autostart.rs:182-200` | Le temporaire est `.<fichier>.tmp.<pid>` : un temporaire laissé par un crash bloque `create_new` jusqu'à réutilisation du PID ; il n'est jamais nettoyé et aucun `fsync` du répertoire n'est fait après `rename`. | Autostart impossible à activer (disponibilité) ou entrée perdue après coupure ; pas de sécurité. | Suffixe aléatoire (`getrandom`/UUID), nettoyage des temporaires anciens, `fsync` du répertoire ; test temporaire préexistant. | Rust : pré-créer le temporaire ⇒ nettoyage/retry ; vérifier l'absence de fichier résiduel. |
| **T2-05** | Info | `src/autostart.rs:130-149` | `is_enabled` ne vérifie que `X-GNOME-Autostart-enabled=false`/`Hidden=true` ; il ne vérifie ni `Type=Application` ni `Exec=/usr/bin/tuxedo-rust-center --tray`. Un fichier édité ou étranger est affiché « activé ». | État UI/checkbox trompeur ; l'utilisateur contrôle déjà le fichier, aucune escalade. | Parser et vérifier `Type`/`Exec` (ou exposer « non vérifié ») ; documenter la sémantique exacte. | Test `Exec=/bin/sh` ⇒ désactivé/non vérifié. |
| **T2-06** | Info | `qml/tray/SystemTray.qml:105-124` ; `qml/Main.qml:126-131` ; `src/daemon_client.rs:53-56,1190-1194` | `MenuItem.checked: modelData.active` est une liaison QML ; le clic écrit `checked` et peut casser la liaison. Si `SetTempProfile` est refusé (rate limit, profil disparu), `LiveData.resolvedActiveId` ne change pas, l'`Instantiator` ne reconstruit pas le menu et l'entrée cliquée peut rester cochée. Fenêtre masquée : polling à **10 s**, donc l'état externe peut retarder. | État visuel faux (profil supposé actif) ; pas de privilège. | Reforcer `checked` depuis `LiveData` à la fin de la commande/erreur, ou reconstruire le modèle ; rafraîchir sur `StateChanged` ; test de refus. | QML : `AppState` refuse l'activation ⇒ l'entrée revient au profil réel. |
| **T2-07** | Info | `qml/tray/SystemTray.qml:29-36` ; `qml/pages/SettingsPage.qml:147-148,1510-1517` | L'absence de SNI/extension GNOME est signalée dans Réglages, mais l'icône tray n'a qu'un tooltip, sans nom/role accessible explicite ; l'accessibilité du menu natif n'est pas testée. | Accessibilité/visibilité ; pas de sécurité. | Documenter l'extension AppIndicator ; ajouter des propriétés accessibles si Qt les expose ; test Orca/Accerciser. | Manuel. |
| **T2-08** | Info | `qml/components/TrayMenu.qml:99,243-258` ; `qml/pages/SettingsPage.qml:1713` | L'aperçu `TrayMenu` de la sidebar est **fonctionnel** (profils, blocker, quitter) mais diverge du tray réel : version issue de `DemoData.guiVersion`, pas d'autostart, `unsupportedReason` toujours `settings.autostartNoHome` même quand l'erreur est `PathInvalid`. | Incohérence UI/i18n, données de démo ; pas de sécurité. | Aligner l'aperçu et le menu réel ; `Qt.application.version` ; mapper `AutostartData.error` vers le motif exact. | QML : vérifier version/erreur/absence d'action privilégiée. |

### 1.1 Statut des correctifs (2026-11-06)

| ID | Statut | Preuve / ce qui reste |
|---|---|---|
| **T2-01** | **Fermé** (limite Qt documentée) | `Binding` sur `AppState.trayAvailable` (`SystemTray.qml:35-43`) ; `onTrayAvailableChanged` appelle `ensureWindowReachable` (`Main.qml:126-131,452-455`) ; la limite « `SystemTrayIcon.available` n'a pas de signal `availableChanged` » est documentée (`SystemTray.qml:36-39`). Un changement réel de disponibilité reste non observable par l'API Qt : limite d'environnement, pas de finding code. |
| **T2-02** | **Fermé** | `ensure_secure_directory` vérifie `uid == geteuid()` et `mode & 0o022 == 0` (`autostart.rs:242-271`) ; création en `0700` (`DirBuilder::mode(0o700)`, `:255`) ; fichier final `0644` et même uid vérifiés (`:226-232`) ; code `InsecurePermissions` (`:39,52`). Tests `insecure_directories_are_refused`, `secure_directories_are_created_0700`. |
| **T2-03** | **Accepté (Info)** | Les symlinks directs (`autostart/`, fichier, `$XDG_CONFIG_HOME` final) sont refusés ; les **ancêtres** intermédiaires ne sont pas résolus (`openat2` non utilisé). Pour un autostart **utilisateur non privilégié**, le risque est limité au répertoire de l'utilisateur ; aucune escalade root. Non bloquant ; à durcir si l'autostart devient privilégié. |
| **T2-04** | **Fermé** | Nettoyage du temporaire avant `create_new` (`autostart.rs:203-206`) et `fsync` du répertoire best-effort après `rename` (`:220-224`) ; test `temporary_file_is_removed_on_write_failure`. Le suffixe **PID** reste : acceptable car le répertoire est `0700` et le temporaire est supprimé avant création. |
| **T2-05** | **Fermé** | `is_enabled` exige `Type=Application` et `Exec=/usr/bin/tuxedo-rust-center --tray` (`autostart.rs:150-166`) ; tests `entry_requires_type_and_expected_exec`, `explicit_disable_or_hidden_is_reported_as_off`. |
| **T2-06** | **Fermé** | `profileAttempt` est incrémenté à chaque clic et inclus dans le modèle de l'`Instantiator` (`SystemTray.qml:29-33,51-54,117-129`) : le menu est reconstruit même si `SetTempProfile` est refusé, ce qui restaure l'état coché réel. Test QML à rejouer. |
| **T2-07** | **Accepté (Info)** | Accessibilité/extension GNOME restent des vérifications manuelles ; aucune régression de sécurité. La notice Réglages et le repli fenêtre sont conservés. |
| **T2-08** | **Fermé** | `TrayMenu` utilise `Qt.application.version` (`TrayMenu.qml:107`), expose l'autostart (`:226-240`) ; `SettingsPage` mappe `InsecurePermissions`/erreur réelle (`:1713-1715`) ; tests `tst_autostart` (dont `InsecurePermissions`). |

**Bilan** : plus aucun finding **non-Info** ouvert. T2-03 et le suffixe PID sont acceptés pour un
autostart utilisateur ; T2-07 est une limite d'accessibilité/plateforme.

---

## 2. Autostart — conformité vérifiée

| Point | État | Preuve |
|---|---|---|
| Chemin `$XDG_CONFIG_HOME` ou `$HOME/.config` | **Conforme** | `resolve_config_home` : XDG absolu et sous HOME, sinon repli `.config` (`autostart.rs:78-94`) ; tests `config_home_resolution_is_strict`. |
| Refus hors HOME / `..` / relatif | **Conforme (lexical)** | `starts_with(home)`, `contains_parent`, `is_absolute` (`:80-106`) ; tests `config_home_resolution_is_strict`, `autostart_path_validation`. Réserve **T2-03** (symlinks d'ancêtres). |
| Refus symlink cible / dossier | **Conforme** | `symlink_metadata` avant écriture, lecture et suppression (`:130-139,170-179,204-213`) ; test `symlinks_and_directories_are_refused`. |
| Écriture atomique | **Conforme** | Temporaire `create_new` 0600, `fsync`, `chmod 0644`, `rename`, puis `fsync` du répertoire best-effort (`autostart.rs:207-224`) ; test `write_enable_read_remove_is_atomic_and_idempotent`. |
| Permissions finales | **Conforme** | Fichier `0644` et même uid vérifiés après écriture (`:226-232`) ; répertoire `autostart/` créé `0700` et `$XDG_CONFIG_HOME`/`autostart` vérifiés (uid, `mode & 0o022 == 0`) via `ensure_secure_directory` (`:242-271`) ; tests `secure_directories_are_created_0700`, `insecure_directories_are_refused`. |
| Contenu `Exec=/usr/bin/tuxedo-rust-center --tray` | **Conforme** | `desktop_entry` (`:109-122`) ; test `desktop_entry_has_required_keys_and_tray_flag`. |
| Aucun shell / commande | **Conforme** | Contenu statique, aucun `Command::new`, aucun `sh -c` dans `autostart.rs`. |
| Suppression propre | **Conforme** | `remove_entry` refuse symlink/dossier, idempotent sur absence (`:204-213`) ; test. |
| Erreurs sans chemin dans l'UI | **Conforme** | `AutostartError::code` (`:41-58`) ; mapping `AutostartData.errorKey` (dont `InsecurePermissions`) ; aucun chemin dans QML. |
| Pas d'écriture privilégiée | **Conforme** | Aucune action root/polkit ; fichier utilisateur uniquement. |

---

## 3. Tray — conformité vérifiée

| Point | État | Preuve |
|---|---|---|
| Aucune logique privilégiée | **Conforme** | `SystemTray.qml` ne fait qu'émettre des signaux/`AppState` ; aucun accès `/etc`, `/var/lib`, D-Bus direct, fichier ou commande. |
| Actions limitées | **Conforme** | Ouvrir (`:83-92`), profils (`:105-132`), autostart (`:136-151`), blocker (`:153-165`), Fn-Lock **désactivé** (`:169-177`), mode graphique **désactivé** (`:179-183`), version (`:187-190`), quitter (`:192-196`). |
| Profil temporaire | **Conforme** | `AppState.requestProfileActivation` → `daemon.setTempProfile` ; ID validé côté GUI (`daemon_client.rs:783-795`) ; limiteur daemon **5/s burst 10** (`dbus.rs:156-164`). |
| État synchronisé | **Conforme** | `LiveData` poussé par `DaemonClient` ; menu lit `LiveData`/`HardwareData`/`PowerBlockerData`/`AutostartData` ; `profileAttempt` force la reconstruction après un refus (**T2-06 fermé**) ; polling masqué 10 s (`daemon_client.rs:53-56,1190-1194`). |
| Blocker | **Conforme** | `Inhibit("idle", block)` non privilégié ; `PowerBlockerData` ; fd libéré à la fin du worker/processus (revue `REVIEW_SESSION_FEATURES.md`, SF-03 fermé). |
| Aucun secret | **Conforme** | Aucun champ de secret, LUKS hors périmètre ; tooltip = nom de profil. |
| Fermeture → masquage | **Conforme** | `onClosing` masque si `trayAvailable && !quitting` (`Main.qml:143-148`) ; `ensureWindowReachable` réaffiche si la disponibilité devient fausse (`:126-131,452-455`). |
| Quitter | **Conforme** | `quitRequested` → `AppState.quitting=true`, `Qt.quit()` (`:116-119`) ; pas de persistance. |
| Tray indisponible | **Conforme** | Loader Error ⇒ `trayAvailable=false`, fenêtre normale, notice Réglages (`Main.qml:77-84`, `SettingsPage.qml:147-148,1510-1517`) ; timer 750 ms (`Main.qml:142-148`). |

---

## 4. Qt Labs Platform et glue

| Point | État | Preuve |
|---|---|---|
| Module natif, pas de web | **Conforme** | `import Qt.labs.platform 1.1` ; aucun `WebEngine`/`WebView`/HTML dans la GUI (grep). |
| Glue C++ minimale | **Conforme** | `qapplication_shim.cpp` : `new QApplication`, `exec`, identité, `delete` ; aucune logique métier. |
| `QApplication` requis | **Conforme** | `build.rs` lie `Core`, `Gui`, `Quick`, `QuickControls2`, **`Widgets`** et compile le shim ; `main.rs` crée `qapp::Application` avant le moteur QML. |
| Dépendances packaging | **Conforme (à rejouer)** | Debian `libqt6widgets6`, `qml6-module-qt-labs-platform` ; RPM `qt6-qtbase-gui`, `qt6-qtdeclarative` ; Arch `qt6-base`, `qt6-declarative` (`packaging/README.md:68-82`). |
| Repli module absent | **Conforme** | `Loader.Error` ⇒ fenêtre normale, notice, pas de crash. |
| `--tray` | **Conforme** | `parse_launch_mode` strict (`main.rs:127-140`) ; propriété initiale `startInTray` ; tests `tray_flag_is_parsed`, `unknown_arguments_are_rejected`. |

---

## 5. Tests — présents et manquants

**Présents**

- Rust `autostart` **11/11** : entrée desktop, écriture/lecture/suppression atomique, `0644`, répertoires `0700`, permissions `InsecurePermissions`, `Type`/`Exec` exacts, temporaire nettoyé, symlinks/dossiers refusés, résolution XDG/HOME/`..`, fichier surdimensionné.
- Rust `main` **3/3** : `--tray`, doublon idempotent, arguments inconnus refusés, mode fenêtre par défaut.
- QML `tst_tray.qml` : menu/modèle, actions (ouvrir, profil, blocker, quitter), toggles, échec autostart, tray indisponible sans crash ; vérifie `AppState.trayAvailable == tray.available`.
- QML `tst_autostart.qml` : toggle, erreurs `NoHome`/`PathInvalid`/`InsecurePermissions`/`WriteFailed`, notice tray indisponible.

**Manquants**

1. SNI réel : hôte `StatusNotifierWatcher`, clic simple/double, menu, tooltip.
2. GNOME sans extension AppIndicator et avec extension ; Wayland et X11.
3. `Main.qml` : fermeture → masquage, `Quitter`, `--tray` de bout en bout (fenêtre masquée/affichée), `ensureWindowReachable`. La limite `available` constante est documentée.
4. Autostart réel au login (session suivante), `XDG_CONFIG_HOME` symlink d'ancêtre (T2-03).
5. Refus `SetTempProfile` (rate limit) et remise à l'état coché : code présent (`profileAttempt`), test QML à rejouer.
6. Accessibilité (Orca/Accerciser) et i18n du menu.
7. Libération de l'inhibiteur après `Quitter` (`systemd-inhibit --list` / `loginctl inhibit`).

---

## 6. Risques résiduels

- **Tray absent** (GNOME sans extension, Wayland sans SNI, module QML absent) : repli fenêtre normale ; l'utilisateur perd le menu tray, pas la fonctionnalité principale. Documenté dans `UI_DESIGN`/`DEV_SETUP`.
- **Disponibilité SNI (T2-01)** : `SystemTrayIcon.available` est constant sans signal dans Qt Labs Platform ; un changement d'hôte en cours de vie n'est pas observable. Le `Binding` et `ensureWindowReachable` couvrent le cas où la propriété changerait malgré tout. Limite d'environnement documentée, pas un finding code.
- **Autostart** : symlinks d'ancêtres (T2-03) acceptés pour un usage utilisateur ; suffixe PID du temporaire (T2-04) accepté (répertoire `0700`, temporaire supprimé avant `create_new`).
- **i18n/accessibilité (T2-07)** : menu natif traduit par clés, pas de test a11y ; extension GNOME requise pour le tray.
- **Packaging** : `libqt6widgets6` est une dépendance dure du binaire (pas seulement du tray) ; le module QML reste optionnel au runtime.

---

## 7. Actions bloquantes

**Aucune.** Les deux réserves initiales sont fermées : T2-02 (permissions/uid du répertoire
autostart, `0700`, `InsecurePermissions`) et T2-01 (binding + réaffichage). T2-04, T2-05, T2-06 et
T2-08 sont également fermés. **T2-03** (ancêtres symlinkés/`openat2`) et le **suffixe PID** sont
acceptés pour un autostart utilisateur non privilégié : aucun accès root, aucun contenu arbitraire,
répertoire `0700` et temporaire nettoyé. Aucune action root/polkit n'est nécessaire.

---

## 8. Verdict final

Le tray et l'autostart sont **conformes au modèle de moindre privilège** : aucune logique
privilégiée, aucune commande shell, aucun secret, écriture limitée au répertoire utilisateur,
permissions vérifiées et erreurs typées sans chemin. **Aucun finding non-Info ouvert** ; il ne
reste que des limites d'environnement (SNI/extension GNOME, accessibilité) et deux réserves
acceptées (T2-03, suffixe PID). **Aucun blocage de release** pour ces surfaces.

---

## 9. Limites manuelles

- **SNI réel** : KDE Plasma, GNOME + extension AppIndicator, Sway/wlroots ; vérifier `SystemTrayIcon.available`, le menu, le clic et le tooltip.
- **Wayland/X11** : comportement de `QSystemTrayIcon`/SNI, notamment sans hôte.
- **Fermeture/quit** : fermer la fenêtre avec tray disponible (doit masquer), `Quitter` (doit terminer), tuer l'hôte SNI (T2-01).
- **Inhibiteur** : après `Quitter`, vérifier qu'aucun inhibiteur `idle` ne subsiste (`systemd-inhibit --list`).
- **Autostart** : activer, se déconnecter/reconnecter, vérifier le lancement `--tray` ; tester `XDG_CONFIG_HOME` valide, hors HOME, symlink, umask `000`/`002`, fichier préexistant `0777`, temporaire résiduel.
- **Accessibilité/i18n** : navigation clavier du menu natif, lecteur d'écran, FR/EN/DE.
- **Packaging** : installation sans `qml6-module-qt-labs-platform` (repli) et avec `libqt6widgets6` absent (échec attendu au chargement — dépendance dure).
- **QML tests** : exécuter `tst_tray.qml`/`tst_autostart.qml` avec un `qmltestrunner` fonctionnel (non obtenu ici).
