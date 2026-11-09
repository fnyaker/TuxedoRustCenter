#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Remplace le TCC officiel par TuxedoRustCenter (paquet Arch local), après
# sauvegarde obligatoire.
#
# Usage :
#   sudo ./scripts/validation/install-replacement.sh <backup-dir> <paquet.pkg.tar.zst> --yes
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : root requis (sudo).\n' >&2
    exit 77
fi

backup=${1:-}
package=${2:-}
confirm=${3:-}

if [ -z "$backup" ] || [ -z "$package" ] || [ "$confirm" != "--yes" ]; then
    printf 'Usage : sudo %s <backup-dir> <paquet.pkg.tar.zst> --yes\n' "$0" >&2
    exit 2
fi

if [ ! -f "$backup/opt.tar.zst" ] || [ ! -f "$backup/system.tar.zst" ]; then
    printf 'ERREUR : sauvegarde incomplète dans %s.\n' "$backup" >&2
    exit 1
fi
if [ ! -f "$package" ]; then
    printf 'ERREUR : paquet introuvable : %s.\n' "$package" >&2
    exit 1
fi

pacman -Q tuxedo-control-center-bin >/dev/null 2>&1 || {
    printf 'ERREUR : tuxedo-control-center-bin non installé ; utiliser rollback/reinstall.\n' >&2
    exit 1
}

printf '== Arrêt du TCC officiel ==\n'
systemctl stop tccd.service 2>/dev/null || true
systemctl disable tccd.service 2>/dev/null || true

# Ferme la GUI/tray officiels qui gardent /opt ouvert.
if pgrep -f '/opt/tuxedo-control-center' >/dev/null 2>&1; then
    printf 'INFO  fermeture des processus TCC officiels\n'
    pkill -TERM -f '/opt/tuxedo-control-center' 2>/dev/null || true
    sleep 1
    pkill -KILL -f '/opt/tuxedo-control-center' 2>/dev/null || true
fi

printf '== Suppression du paquet officiel (sauvegarde : %s) ==\n' "$backup"
pacman -R --noconfirm tuxedo-control-center-bin

printf '== Restauration de /etc/tcc pour la migration ==\n'
if [ -f "$backup/etc-tcc.tar.zst" ]; then
    tar -C / -I zstd -xf "$backup/etc-tcc.tar.zst"
else
    tar -C / -I zstd -xf "$backup/system.tar.zst" etc/tcc 2>/dev/null || true
fi

printf '== Installation de TuxedoRustCenter ==\n'
pacman -U --noconfirm "$package"

printf '== Activation ==\n'
systemctl daemon-reload
systemctl enable --now tccd.service
systemctl --user start tccd-user.service 2>/dev/null || true

printf '== État ==\n'
systemctl --no-pager --full status tccd.service || true
busctl --system call com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd.Manager1 GetContractVersion || true
systemctl --no-pager --full status tccd-admin.service || true

printf '\nInstallation terminée.\n'
printf 'Rollback : sudo ./scripts/validation/rollback-official.sh %s\n' "$backup"
