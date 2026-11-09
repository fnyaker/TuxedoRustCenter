# Politique de sécurité

## Signaler une vulnérabilité

Ne pas ouvrir d'issue publique pour une vulnérabilité exploitable. Contacter les
mainteneurs en privé avec :

- la version ou le commit concerné ;
- une description du problème et de l'impact ;
- les étapes de reproduction ;
- une proposition de correctif si possible.

## Périmètre

Sont concernés :

- le daemon `tccd` et le service `tccd-admin` ;
- l'agent de session `tccd-user` ;
- la GUI `tcc-gui` et son client D-Bus ;
- les unit systemd, politiques D-Bus/polkit, règles udev et scripts de packaging ;
- les dépendances Cargo et la chaîne de build.

## Principes

Le modèle de menace, les décisions D1–D7 et la checklist de revue sont dans :

- [`docs/security/THREAT_MODEL.md`](docs/security/THREAT_MODEL.md)
- [`docs/security/DECISIONS.md`](docs/security/DECISIONS.md)
- [`docs/security/CHECKLIST.md`](docs/security/CHECKLIST.md)

Aucun secret LUKS ne doit transiter par argv, l'environnement, une chaîne D-Bus ou
un log. Aucune commande shell interpolée. Aucun téléchargement exécuté en root.
