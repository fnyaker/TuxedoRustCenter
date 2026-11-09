# Guide utilisateur — TuxedoRustCenter

Ce guide décrit l'utilisation courante de TuxedoRustCenter (TCC réécrit en
Rust). Il suppose une installation par paquet (deb, rpm ou Arch) et un laptop
TUXEDO compatible. Les écarts et limites sont détaillés en fin de document.

Documents liés :

- installation et migration : `docs/packaging/README.md`, `docs/packaging/MIGRATION.md` ;
- API D-Bus : `docs/DBUS_API.md` ;
- état du projet et limites : `docs/STATUS.md`, `docs/FEATURE_PARITY.md`.

---

## 1. Démarrage

Après installation :

- `tccd.service` démarre au boot et applique le profil correspondant à
  l'état courant (`power-profiles-daemon` × secteur/batterie) ;
- `tccd-user.service` démarre avec la session graphique et applique le
  refresh rate du profil actif ;
- l'application se lance depuis le menu ou avec `tuxedo-rust-center` ;
- `tcc-cli` permet de diagnostiquer sans lancer la GUI.

Si le daemon est absent, la GUI affiche un bandeau « Démo » et des données de
démonstration : aucun réglage n'est alors appliqué.

## 2. Tableau de bord

- Profil actif, source d'alimentation (secteur/batterie), profil PPD
  (`power-saver`, `balanced`, `performance`).
- CPU, iGPU, dGPU : température, fréquence, puissance.
- Ventilateurs : vitesse mesurée, consigne, mode (`auto`, `manual`, `off`),
  backend (`tuxi`, `pwm`, `tuxedo-io`).
- Collecte des capteurs : désactivée par défaut. Le bouton active un **bail de
  10 s**, renouvelé tant que le tableau de bord est visible et que la collecte
  est demandée ; elle expire automatiquement après 10 s hors page ou 5 min de
  session. Aucun capteur n'est lu sans cela.
- Cliquer sur le profil ouvre l'éditeur ; cliquer sur le mode graphique ouvre
  les réglages PRIME.

Les capteurs absents ou non supportés affichent « non disponible » avec une
infobulle ; aucune valeur n'est inventée.

## 3. Profils

- Les presets TUXEDO sont fournis par le daemon (Max Energy Save, Quiet,
  Office, High Performance) et ne sont pas modifiables.
- Activation manuelle : un clic sur « Activer » applique le profil jusqu'au
  prochain changement de PPD ou d'alimentation (override temporaire).
- Édition : nom, description, luminosité, refresh rate, CPU, webcam,
  ventilateurs (contrôle, profil, min/max/offset).
- « Enregistrer » valide localement, puis demande l'autorisation polkit et
  passe par `tccd-admin` ; le daemon recharge la configuration et réapplique
  le profil. Le formulaire est conservé si l'enregistrement échoue.
- Si un autre client modifie les profils pendant l'édition, un bandeau
  « Recharger » apparaît ; l'enregistrement est bloqué tant que le conflit
  n'est pas résolu.
- Création, duplication, suppression et import/export ne sont pas encore
  disponibles (phase 1).

## 4. Refresh rate

Le profil peut demander un taux de rafraîchissement de l'**écran interne**
uniquement (`eDP-*`, `LVDS-*`, `DSI-*`) ; les écrans externes ne sont jamais
modifiés.

Backends disponibles :

| Session | Backend |
|---|---|
| X11 | XRandR natif |
| Hyprland | socket IPC natif |
| Sway | IPC Sway |
| KDE | `kscreen-doctor` (fallback) |
| GNOME/Mutter | **non supporté** |

Si le taux demandé n'est pas disponible à la résolution courante, le changement
est refusé avec un message clair ; la résolution n'est pas modifiée
implicitement. Si aucun backend n'est disponible, l'UI indique « non supporté »
et le profil reste appliqué pour le reste.

## 5. Réglages

- température °C/°F ;
- contrôle CPU activé/désactivé ;
- contrôle des ventilateurs activé/désactivé ;
- contrôle de l'éclairage activé/désactivé.

La sauvegarde de ces réglages passe par le même service admin et demande une
autorisation polkit. Les autres réglages (charge, matrice d'état, PRIME, ODM,
TDP, cTGP) sont affichés en **lecture seule** pour l'instant.

## 6. Outils et matériel

- **Minuterie d'extinction** : choisir une heure locale (HH:MM), appliquer ou
  annuler. La planification passe par logind ; l'heure est persistée dans les
  réglages. Une heure invalide est signalée sans être appliquée.
- Éclairage : type de clavier (blanc, zones RGB, per-key), lightbar, modes.
  La couche matérielle existe ; l'écriture depuis l'UI attend l'autorisation
  des setters.
- Webcam : kill switch matériel et liste des périphériques. Les contrôles UVC
  (luminosité, contraste, etc.) sont exposés par l'agent de session
  (`User1.SetUvcControl`) quand la caméra est libre ; les curseurs de
  l'interface sont en cours d'ajout. Une caméra déjà utilisée par une autre
  application n'est pas forcée : TCC signale qu'elle est occupée.
- Charge : profil, priorité, seuils et type affichés si le matériel les expose.
- Fn-Lock : état affiché.
- ODM/plateforme : PPD reste propriétaire de `platform_profile` et de l'EPP
  par défaut ; TCC ne les écrase pas.
- TDP, NVIDIA cTGP et PRIME : lecture seule (phase 2 pour l'écriture).

## 7. Autorisation et service admin

L'enregistrement des profils/réglages nécessite :

1. le paquet construit avec `TCC_WITH_ADMIN=1` (non installé par défaut) ;
2. `tccd-admin.service`, activé à la demande par D-Bus ;
3. polkit : une fenêtre d'authentification administrateur peut apparaître.

En cas de refus, d'annulation ou de service absent :

- « Service d'administration non installé/activé » : le paquet admin n'est pas
  installé ;
- « Autorisation refusée » : polkit a refusé ou la session a été annulée ;
- « Une autre opération est en cours » : réessayer ;
- « Enregistré, mais le daemon n'a pas rechargé » : la sauvegarde a été
  restaurée si possible ; vérifier `journalctl -u tccd`.

`tccd-admin` n'exécute jamais de commande shell et n'accepte ni chemin ni
paramètre libre.

## 8. Diagnostic

```sh
systemctl status tccd.service
systemctl --user status tccd-user.service
journalctl -u tccd.service -e
tcc-cli status
tcc-cli profiles
tcc-cli hardware
tcc-cli sensors
tcc-cli admin          # version du service admin s'il est installé
```

Mode développement sans root, sur le bus de session :

```sh
./scripts/dev-session.sh
```

Cohabitation avec l'ancien TCC : arrêter et désactiver `tccd.service` original,
vérifier que le nom D-Bus `com.tuxedocomputers.tccd` est libre, puis installer.
Le postinst refuse d'activer le nouveau service si `/opt/tuxedo-control-center`
subsiste. `/etc/tcc` n'est jamais supprimé.

## 9. Barre système (tray)

- Icône native Qt (`Qt.labs.platform`) avec menu : ouvrir, profils temporaires,
  autostart, power save blocker, Fn-Lock, mode graphique, version, quitter.
- Fermer la fenêtre la masque si le tray est disponible ; « Quitter » termine
  l'application.
- Sur les bureaux sans StatusNotifierItem (GNOME sans extension), l'icône peut
  ne pas apparaître ; la fenêtre reste utilisable normalement.

## 10. Limites connues

- GNOME/Mutter non supporté pour le refresh rate.
- Création/suppression/import-export de profils non disponibles.
- Écriture des réglages globaux partielle (4 interrupteurs).
- Setters charge/éclairage/Fn-Lock refusés par la policy D-Bus tant que
  l'autorisation interne de `tccd` n'est pas faite.
- Contrôles UVC webcam : service de session livré, curseurs GUI en cours ;
  écriture réelle sur caméra non validée.
- TDP/ODM/cTGP/PRIME en lecture seule (phase 2).
- Changement de mot de passe LUKS : **hors périmètre**, non implémenté.
- Tomte, WebFAI, Aquaris : pas encore disponibles.
- Tray : dépend du support StatusNotifierItem du bureau (GNOME nécessite une
  extension ; KDE/Hyprland le supportent généralement).
- Validation sur matériel TUXEDO réel encore partielle (Clevo/Uniwill, EC,
  veille).
- `tccd-admin` doit être validé avec le vrai polkit sur bus système avant
  activation en production.

## 11. Sécurité et vie privée

- La GUI et `tccd-user` ne sont pas privilégiés.
- `tccd` est root uniquement pour les écritures sysfs/ioctl et la persistance
  dans `/var/lib/tuxedo-rust-center`.
- Aucun secret n'est journalisé ; les payloads D-Bus ne contiennent pas de
  chemin ni de contenu de fichier.
- Les capteurs ne sont lus que sur demande explicite.
- Les logs vont dans journald, jamais dans `/var/log`.
