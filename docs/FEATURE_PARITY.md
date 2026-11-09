# Parité fonctionnelle — TuxedoRustCenter

Checklist de référence pour la réécriture. « Parité » signifie : comportement équivalent à l'original, ou différence explicitement documentée et acceptée.

Statuts : `[ ]` à faire, `[~]` partiel, `[x]` fait, `[-]` hors périmètre assumé.

> Dernière revue : 2026-11-06, jalon 2 (lecture seule + `tccd-admin` phase 1 en cours).
> Les items `[x]` ont un code et des tests ; les items `[~]` fonctionnent partiellement
> ou sont limités à la lecture ; voir `docs/STATUS.md` et `docs/HANDOFF.md`.

---

## 1. Fenêtre principale et navigation

- [x] Fenêtre Qt 6 native, sans composant web (`cxx-qt` + QML uniquement).
- [~] Barre de titre / contrôles de fenêtre si le compositeur le permet (décoration système par défaut).
- [x] Sidebar : Dashboard, Profiles, Tools, profils utilisés (AC/BAT), langue, Support, Settings, Info. Aquaris masqué tant que non implémenté.
- [x] Thème clair/sombre/système.
- [x] Accent rouge TUXEDO `#E30016`, jauges `#E22131`, utilisation mesurée.
- [x] Police Titillium Web embarquée (OFL) et icônes natives.
- [~] HiDPI, redimensionnement, tailles minimales (rendu validé, pas de test automatisé HiDPI).
- [~] Navigation clavier, focus visible, tooltips, contraste accessible (design documenté, audit a11y non automatisé).
- [x] Réduction des animations (`Theme.reduceMotion`).
- [x] i18n FR/EN/DE, changement de langue sans redémarrage, parité des clés testée.
- [x] °C/°F (`Strings.fahrenheit`, réglage affiché dans Réglages).

---

## 2. Dashboard / moniteur système

- [x] Profil actif affiché et lien vers son édition.
- [x] CPU : température, fréquence, puissance, vitesse ventilateur (`GetFanDataJSON`).
- [x] iGPU : température, fréquence, puissance (selon vendor), avec historique et graphes locaux (temp/fréquence/puissance).
- [x] dGPU : température, fréquence, puissance, ventilateur si exposé.
- [~] Plusieurs GPU / PRIME : affichage cohérent selon le mode (lecture seule).
- [x] État « non disponible » + infobulle de compatibilité pour chaque capteur.
- [x] Rafraîchissement fluide sans consommation excessive (polling 2 s, historiques locaux 2 s, 48 points).
- [x] Collecte capteurs automatique tant que le Dashboard est affiché (aucun interrupteur ; bail 10 s renouvelé par les lectures).
- [x] Jauges animées et unités correctes, animées seulement si `reduceMotion=false`.
- [~] Mode graphique affiché dans Réglages (lecture seule), plus dans le Dashboard (carte redondante supprimée).
- [x] Graphes intégrés aux cartes CPU/iGPU/dGPU ; cartes « État du système », « Disponibilité des capteurs » et « Historique » redondantes supprimées.
- [x] Aucune exception si un capteur disparaît (instantané tolérant, repli démo).

---

## 3. Profils

- [x] Profils preset fournis par device (Max Energy Save, Quiet, Office, High Performance).
- [x] Profils personnalisés : création, duplication, renommage, description, suppression.
- [x] Filtres : All, Preset, Custom, In use.
- [x] Grille responsive : 1 à 4 colonnes, cartes ≥ 300 px, contenu plafonné.
- [ ] Vue détail / vue compacte.
- [~] Profil actif, profils assignés à chaque état (source courante seulement ; matrice non exposée par `tccd`).
- [x] Activation manuelle temporaire (UI) jusqu'au prochain changement d'état.
- [x] Import / export de profils avec validation (enveloppe versionnée, conflits ID/nom, taille bornée).
- [x] IDs stables, unicité des noms, limites de taille (`ProfileCollection::validate`, renommage à la fusion).
- [~] Assignation d'un profil aux états :
  - [x] AC ;
  - [x] Batterie ;
  - [x] combinaisons PPD × AC/BAT (moteur et config, UI non éditable).
- [x] Matrice PPD × alimentation éditable (3×2, fallback AC/BAT, `Admin1`).
- [x] Migration des profils `/etc/tcc/profiles` de l'original (sans `.bak` automatique de `/etc/tcc`, R-18 ouvert).

---

## 4. Éditeur de profil

### Général
- [x] Nom, description.
- [~] Indicateur ODM (LED du bouton TCC) si supporté (état ODM lu, LED non pilotée).
- [ ] Assignation automatique aux états.
- [x] Sauvegarde via `Admin1.SaveProfilesJSON`, annulation, conflit de profil actif.

### Affichage
- [x] Luminosité à l'activation (`useBrightness`, pourcentage).
- [x] Refresh rate de l'écran interne à l'activation (`useRefRate`, Hz).
  - [~] Liste des taux disponibles pour la résolution courante (backends et CLI `--list`, pas dans l'éditeur).
  - [x] X11 via XRandR (`x11rb`).
  - [x] Wayland via agent de session : Hyprland, Sway, KDE `kscreen-doctor`, GNOME/Mutter `DisplayConfig`.
  - [x] Ne jamais modifier les écrans externes (connecteur interne + tests).
  - [x] État « non supporté » propre.
- [x] Résolution à l'activation (`useResolution`, x/y) : agent session + modes réels.
- [x] Gestion des taux non disponibles : erreur claire, pas de changement arbitraire.

### CPU
- [x] Nombre de cœurs en ligne.
- [x] Governor max performance (`useMaxPerfGov`).
- [x] Fréquence min/max.
- [x] Préférence d'énergie (`energyPerformancePreference`) par profil, avec infobulle PPD ; la valeur stockée est indépendante par profil.
- [x] Désactivation du turbo (`noTurbo`).
- [~] Quirks device (EPP, intel_pstate, amd-pstate-epp, acpi-cpufreq, Gemini, Sirius) ; validation matérielle réelle en attente.

### Webcam
- [x] Activation/désactivation de la webcam à l'activation du profil (kill switch).
- [x] Kill switch matériel si disponible (Clevo/Uniwill).
- [~] Préréglages UVC par profil (écriture session en cours ; préréglages persistés non implémentés).

### Ventilateurs
- [x] Profil de ventilateur (Silent, Quiet, Balanced, custom).
- [x] Vitesse minimale / maximale / offset.
- [x] Courbe personnalisée CPU, éditable par drag & drop sur le graphe (ajout double-clic, suppression clic droit/Suppr).
- [x] Courbe personnalisée GPU.
- [~] Hystérésis et lissage (hystérésis 2 % ; filtre médian 13 échantillons non reproduit, documenté).
- [x] Plusieurs ventilateurs / interfaces (tuxi, pwm, tuxedo-io).
- [x] Mode auto restauré à la désactivation et à l'arrêt.

### ODM / TDP / NVIDIA
- [x] Profil ODM : éditable par profil, appliqué via `tuxedo_io` (preuve réelle). TDP par profil implémenté mais non exposé sur ce laptop ; cTGP masqué sans NVIDIA.
- [~] TDP PL1/PL2/PL4 : `Admin.Hw` (borne, rollback, `PartialApply`) ; `NotSupported` sur ce matériel.
- [~] Offset cTGP NVIDIA : `Admin.Hw` prêt, masqué sans NVIDIA.
- [x] Validation des plages et disponibilité.

---

## 5. Outils

- [-] Changement du mot de passe LUKS : **hors périmètre, décision utilisateur**, non implémenté.
- [x] Minuterie d'extinction : heure/minute, annulation, persistance,
  application via logind (`ScheduleShutdown`/`CancelScheduledShutdown`).
- [~] Éclairage clavier / lightbar :
  - [x] détection des capacités (blanc, zones RGB, per-key) ;
  - [x] luminosité (couche matérielle) ;
  - [x] couleur RGB (couche matérielle) ;
  - [x] mode statique / respiration (couche matérielle) ;
  - [x] lightbar (couche matérielle) ;
  - [ ] synchronisation UPower/lid ;
  - [ ] profil par état si l'original le fait.
- [~] Webcam :
  - [x] liste des périphériques (V4L2, lecture) ;
  - [x] contrôles UVC en lecture (allowlist, test réel `/dev/video0` : 11 contrôles) ;
  - [~] écriture UVC via `tccd-user` : service `User1.SetUvcControl` livré et testé (allowlist, rate limit, erreurs typées) ; sliders GUI en cours ; écriture réelle non testée ;
  - [ ] mode simple / avancé ;
  - [ ] préréglages par device ;
  - [ ] preview native Qt Multimedia (pas de HTML) ;
  - [ ] application des réglages à l'activation du profil.

---

## 6. Réglages globaux

- [x] Température °C/°F.
- [x] Contrôle CPU activé/désactivé (modifiable et persisté).
- [x] Contrôle ventilateurs activé/désactivé (modifiable et persisté).
- [x] Contrôle éclairage activé/désactivé (modifiable et persisté ; worker respecte le réglage).
- [~] YCbCr 4:2:0 par port/GPU (lecture seule).
- [x] Charge :
  - [x] profil de charge ;
  - [x] priorité de charge ;
  - [x] seuils start/end ;
  - [x] type de charge (si le firmware l'expose) ;
  - [x] valeurs disponibles et validation ;
  - [x] écriture depuis l'UI via `Admin1.SaveSettingsJSON` (type masqué si non supporté).
- [x] Thème clair/sombre/système.
- [~] PRIME / mode graphique (iGPU, dGPU, on-demand) : lecture seule.
- [ ] Tomte (non implémenté, carte masquée).
- [ ] Aquaris (non implémenté, carte masquée).
- [x] Langue.
- [x] Autostart tray (`~/.config/autostart`, `--tray`).
- [x] Power save blocker (inhibition de veille) : logind `idle/block`, fd libéré à la fermeture, sans persistance.
- [x] Fn-Lock (si supporté) : interrupteur persistant via `Admin1`, appliqué par le worker ; masqué sinon.
- [x] Bascule automatique : matrice PPD × AC/BAT appliquée par le daemon et éditable dans l'UI.
- [x] Réglages globaux sauvegardés via `Admin1.SaveSettingsJSON` (température, CPU, ventilateurs, éclairage, charge).
- [x] État de l'agent de session : service `User1`, backend, connecteur, mode courant, modes réels.

---

## 7. Support / Info

- [ ] Diagnostic système :
  - [ ] génération/collecte `systeminfos` ;
  - [ ] numéro de ticket ;
  - [ ] ouverture du formulaire en ligne ;
  - [ ] nettoyage des données temporaires.
- [ ] WebFAI Creator : détection, installation, suppression, lancement (via commandes allowlistées).
- [ ] AnyDesk : détection, installation, lancement (si l'original le propose).
- [~] Page Info : version, changelog, mentions, liens (statique).
- [ ] Contact / support en ligne.
- [ ] Aquaris : connexion Bluetooth, recherche, état, ventilateur/pompe/LED, mode démo.
- [ ] Tomte : état, modules, versions, activation/désactivation, reset.

---

## 8. Tray

- [x] Icône tray native Qt (`Qt.labs.platform`), menu et tooltip.
- [x] Ouvrir la fenêtre (clic tray, afficher/masquer).
- [x] Profils : activation temporaire (radio).
- [x] Autostart tray (`~/.config/autostart`, `--tray`).
- [x] Power save blocker.
- [x] Fn-Lock : interrupteur persistant via `Admin1`.
- [~] Mode graphique (affichage lecture seule).
- [ ] Aquaris.
- [x] Version.
- [x] Quitter.
- [x] Menu réactif, traduit, accessible ; tray indisponible → fenêtre normale.

---

## 9. Daemon / matériel

### Bascule automatique
- [x] Écoute `net.hadess.PowerProfiles` (`ActiveProfile`, `PropertiesChanged`).
- [x] Écoute UPower `OnBattery` et/ou sysfs AC.
- [x] État combiné `(ppd, power)`.
- [x] Matrice configurable (`stateMatrix` dans les settings).
- [x] Fallback AC/BAT si PPD absent.
- [x] Debounce.
- [x] Override manuel temporaire invalidé par changement d'état.
- [x] Reapply sur changement de config.
- [x] Pas de conflit avec PPD (`platform_profile`, EPP) ; politique documentée et `ppd_override=false` par défaut.
- [x] Logs clairs des transitions.
- [x] Tests avec PPD absent, PPD qui flappe, batterie inconnue.

### CPU
- [x] `/sys/devices/system/cpu` : online cores.
- [x] cpufreq : governor, min/max, available frequencies.
- [x] EPP.
- [x] `no_turbo`.
- [~] Quirks device (couverture partielle, validation matérielle en attente).
- [~] Validation/reapply périodique (application au changement, pas de reapply périodique complet).
- [ ] Restauration des défauts à l'arrêt.

### Ventilateurs
- [x] Backend `tuxi` (hwmon).
- [x] Backend `pwm` (hwmon).
- [x] Backend `tuxedo-io` (Clevo/Uniwill).
- [x] Détection des ventilateurs et températures.
- [x] Courbes CPU/GPU, min/max/offset.
- [x] Hystérésis.
- [x] Mode auto.
- [x] Min speed / fans off available.
- [~] Faible consommation CPU (tick 1 s, lectures minimales ; budget non mesuré).

### Affichage
- [x] Luminosité via `/sys/class/backlight`.
- [x] Quirk amdgpu.
- [x] Refresh rate écran interne X11.
- [~] Refresh rate écran interne Wayland : Hyprland, Sway, KDE, GNOME/Mutter ; validation sur sessions réelles à faire.
- [x] Résolution de l'écran interne (agent session, modes réels).
- [x] Ne pas toucher aux écrans externes.
- [~] Gestion session verrouillée / absente / TTY (session absente gérée, verrouillage non traité).

### Webcam
- [x] Kill switch `tuxedo-io` si disponible.
- [~] V4L2 : énumération et contrôles (lecture) implémentés et testés sur caméra réelle ; service `User1.SetUvcControl` livré (allowlist, session non privilégiée, rate limit) ; sliders GUI en cours ; écriture réelle non testée.
- [ ] Préréglages par device.
- [ ] Application au branchement (udev) si l'original le fait.
- [ ] Preview native.

### Charge
- [x] Profil, priorité, seuils, type.
- [x] Détection des capacités.
- [x] Écriture sysfs validée.
- [ ] Persistance (setters non persistants ; l'admin phase 1 ne couvre que profils/settings).

### Éclairage
- [x] Clavier blanc.
- [x] Zones RGB.
- [x] Per-key si supporté.
- [x] Lightbar.
- [ ] UPower lid.
- [x] Modes.
- [x] Écriture atomique / buffer input si nécessaire.

### ODM / TDP / NVIDIA / Prime
- [x] Détection `tuxedo_platform_profile` / ACPI.
- [x] Profils disponibles et défaut.
- [x] TDP min/max/current (lecture + validation).
- [~] cTGP offset et limites NVIDIA (lecture seule).
- [x] `prime-select` / état PRIME (lecture ; bascule réservée à l'admin).
- [x] Quirks (IBP, Gemini, Sirius, etc.).

### Capteurs
- [x] CPU temp/freq/power.
- [x] GPU AMD/Intel/NVIDIA.
- [x] RAPL.
- [x] Ventilateurs (`GetFanDataJSON`).
- [x] Collecte à la demande (bail 10 s, plafond 5 min).
- [x] Timeouts et valeurs par défaut.

### Autres
- [~] YCbCr 4:2:0 par port (lecture et validation ; écriture non exposée).
- [x] Fn-Lock (couche matérielle + UI persistante).
- [x] Détection device DMI / Uniwill / quirks.
- [x] Veille/reprise via `PrepareForSleep` : validation réelle `systemctl suspend` (fans auto, workers recréés, profil réappliqué, inhibitor réacquis).
- [x] PID/runtime propre (systemd, pas de PID file).
- [x] Logs journald.

---

## 10. D-Bus et intégration

- [x] Service système `com.tuxedocomputers.tccd` (ou namespace documenté).
- [x] API historique JSON pour compatibilité (lecture + `SetTempProfile`/`ClearTempProfile`).
- [x] API typée versionnée : `Manager1` (lecture/bail) et `Admin1` phase 1 (écriture profils/settings) utilisés par la GUI/CLI.
- [x] Signaux de changement de profil/état (`StateChanged`).
- [x] Politique D-Bus restrictive, fail-closed, testée (42/42, Admin1/Control1 inclus).
- [~] Polkit pour les actions sensibles : appliqué à `Admin1` phase 1 (`system-bus-name`, fail-closed) ; setters `Manager1` encore refusés par la policy.
- [x] Aucune méthode permettant une écriture arbitraire de fichier.
- [x] Rate limiting / validation (écritures et lectures capteurs).
- [x] Tests D-Bus (bus de session et bus isolé).
- [x] L'UI fonctionne sans accès root.

---

## 11. Packaging / exploitation

- [x] Installation FHS, aucun `/opt`.
- [x] Unit systemd système + utilisateur.
- [x] Veille/reprise via `PrepareForSleep` (plus de `tccd-sleep.service`).
- [x] D-Bus policy.
- [~] Polkit (actions installées et appelées par `tccd-admin` phase 1 ; setters `Manager1` pas encore couverts).
- [x] Udev webcam.
- [x] Desktop entry, icônes, metainfo.
- [x] deb, rpm, PKGBUILD Arch construits et validés en conteneurs.
- [x] Migration `/etc/tcc` → `/var/lib/tuxedo-rust-center` (premier démarrage, sans `.bak` de `/etc/tcc`).
- [x] Désinstallation propre (config conservée ou purgée selon package).
- [~] Documentation utilisateur (docs développeur/packaging complètes, guide utilisateur à écrire).
- [x] Journald, pas de `/var/log`.
- [x] Pas de runtime Node/Python.

---

## 12. Sécurité / qualité

- [x] Aucun `setuid`.
- [x] Daemon root minimal, GUI non privilégiée, agent session non privilégié.
- [x] Aucune commande shell interpolée (allowlist, arguments séparés).
- [-] Mots de passe LUKS : non applicable (fonction hors périmètre).
- [x] Validation stricte des entrées D-Bus/JSON.
- [~] `openat2`/`O_NOFOLLOW` pour les fichiers sensibles (`O_NOFOLLOW` + `fstat`, `openat2` non utilisé).
- [x] Écritures atomiques, permissions 0644/0755.
- [x] Hardening systemd (scores 3.1–3.6).
- [x] `cargo fmt`, `clippy -D warnings`, tests.
- [ ] `cargo audit`, `cargo deny` (configurés en CI, non exécutés localement).
- [~] Fuzzing des entrées critiques (property/mutation tests, pas de fuzzing continu).
- [x] Revue `unsafe` (`REVIEW_JALON1.md`, `REVIEW_HARDWARE.md`).
- [~] Budgets de performance mesurés : daemon 0,33 % CPU / 15,1 MiB RSS ; GUI Xvfb logicielle 127 MiB / 0,6 % CPU, démarrage ~300 ms ; backend GL/llvmpipe ~267 MiB à re-mesurer sur GPU réel.
- [~] Accessibilité et i18n vérifiées (i18n testée, a11y non automatisée).
