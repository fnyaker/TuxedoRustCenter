#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sauvegarde complète du TCC officiel avant remplacement par TuxedoRustCenter.
# Lecture seule sur le système ; écrit uniquement dans /var/backups.
#
# Usage : sudo ./scripts/validation/backup-official.sh
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : root requis (sudo).\n' >&2
    exit 77
fi

pkg=tuxedo-control-center-bin
stamp=$(date +%Y%m%d-%H%M%S)
dest=/var/backups/tuxedo-official-$stamp

if ! pacman -Q "$pkg" >/dev/null 2>&1; then
    printf 'ERREUR : paquet %s non installé.\n' "$pkg" >&2
    exit 1
fi

version=$(pacman -Q "$pkg" | awk '{print $2}')
install -d -m 0700 "$dest"

printf '== Sauvegarde du TCC officiel (%s %s) ==\n' "$pkg" "$version"

# 1. /opt complet.
tar -C / -I zstd -cf "$dest/opt.tar.zst" opt/tuxedo-control-center

# 2. Fichiers du paquet hors /opt (unit, D-Bus, polkit, desktop, icônes, /etc/tcc…).
pacman -Ql "$pkg" | awk -v pkg="$pkg" '
    $1 == pkg { print $2 }
' | sed 's#^/##' | grep -v '^opt/tuxedo-control-center/' > "$dest/package-files.txt"

if [ -s "$dest/package-files.txt" ]; then
    tar -C / -I zstd -cf "$dest/system.tar.zst" -T "$dest/package-files.txt"
fi

# 2bis. Configuration runtime /etc/tcc (non possédée par le paquet).
if [ -d /etc/tcc ]; then
    tar -C / -I zstd -cf "$dest/etc-tcc.tar.zst" etc/tcc
fi

# 3. Base pacman locale (pour un rollback complet).
pacman_local=$(ls -d /var/lib/pacman/local/"$pkg"-* 2>/dev/null | head -1 || true)
if [ -n "$pacman_local" ]; then
    cp -a "$pacman_local" "$dest/pacman-local"
fi

# 4. Métadonnées.
pacman -Qi "$pkg" > "$dest/pacman-qi.txt"
pacman -Ql "$pkg" > "$dest/pacman-ql.txt"
(pacman -Qkk "$pkg" 2>&1 || true) > "$dest/pacman-qkk.txt"
systemctl cat tccd.service > "$dest/tccd.service.txt" 2>&1 || true

# 5. État éventuel de notre version (pour ne pas l'écraser au rollback).
if [ -d /var/lib/tuxedo-rust-center ]; then
    tar -C /var/lib -I zstd -cf "$dest/tuxedo-rust-center-state.tar.zst" tuxedo-rust-center
fi

cat > "$dest/RESTORE.md" <<EOF
# Restauration du TCC officiel

Sauvegarde : $dest
Paquet : $pkg $version

Rollback automatique :

    sudo ./scripts/validation/rollback-official.sh "$dest"

Le script supprime tuxedo-rust-center, restaure /opt, les fichiers système,
/etc/tcc et l'entrée pacman, puis réactive tccd.service.

Si la base pacman est incohérente, réinstaller le paquet :

    yay -S $pkg
EOF

printf '\nSauvegarde terminée : %s\n' "$dest"
du -sh "$dest"
printf 'Taille TCC officiel : '; du -sh /opt/tuxedo-control-center | cut -f1
