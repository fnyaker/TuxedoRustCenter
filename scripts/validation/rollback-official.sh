#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Rollback : supprime TuxedoRustCenter et restaure le TCC officiel sauvegardé.
#
# Usage : sudo ./scripts/validation/rollback-official.sh <backup-dir>
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : root requis (sudo).\n' >&2
    exit 77
fi

backup=${1:-}
if [ -z "$backup" ] || [ ! -f "$backup/opt.tar.zst" ] || [ ! -f "$backup/system.tar.zst" ]; then
    printf 'Usage : sudo %s <backup-dir>\n' "$0" >&2
    exit 2
fi

printf '== Arrêt de TuxedoRustCenter ==\n'
systemctl stop tccd.service 2>/dev/null || true
systemctl stop tccd-admin.service 2>/dev/null || true
systemctl --user stop tccd-user.service 2>/dev/null || true

if pacman -Q tuxedo-rust-center >/dev/null 2>&1; then
    printf '== Suppression du paquet tuxedo-rust-center ==\n'
    pacman -R --noconfirm tuxedo-rust-center
else
    printf 'INFO  tuxedo-rust-center non installé\n'
fi

printf '== Restauration /opt/tuxedo-control-center ==\n'
tar -C / -I zstd -xf "$backup/opt.tar.zst"

printf '== Restauration des fichiers système, /etc/tcc, unités, D-Bus, polkit ==\n'
tar -C / -I zstd -xf "$backup/system.tar.zst"
if [ -f "$backup/etc-tcc.tar.zst" ]; then
    tar -C / -I zstd -xf "$backup/etc-tcc.tar.zst"
fi

printf '== Restauration de la base pacman ==\n'
if [ -d "$backup/pacman-local" ]; then
    name=$(basename "$backup/pacman-local")
    rm -rf "/var/lib/pacman/local/$name"
    cp -a "$backup/pacman-local" "/var/lib/pacman/local/$name"
    chown -R root:root "/var/lib/pacman/local/$name"
fi

printf '== Réactivation du TCC officiel ==\n'
systemctl daemon-reload
systemctl enable --now tccd.service
systemctl --no-pager --full status tccd.service || true

printf '\n== Vérification pacman ==\n'
if pacman -Q tuxedo-control-center-bin; then
    printf 'OK    paquet officiel restauré\n'
else
    printf 'ATTENTION : entrée pacman absente ou invalide.\n'
    printf 'Réinstaller avec : yay -S tuxedo-control-center-bin\n'
fi

printf '\nRollback terminé. Redémarrage recommandé pour repartir proprement.\n'
