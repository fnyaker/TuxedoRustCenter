#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fonctions partagées des scripts de release TuxedoRustCenter.
# Ce fichier est sourcé, jamais exécuté directement.
#
# Règles :
#   - aucune commande shell construite par interpolation puis exécutée ;
#   - toutes les variables sont citées ;
#   - les chemins viennent du dépôt, jamais d'une entrée utilisateur.

# Répertoire racine du dépôt, à partir du répertoire du script appelant.
tcc_repo_root() {
    local script_dir=$1
    (CDPATH='' cd -- "$script_dir/../.." && pwd -P)
}

# Version du workspace, lue dans [workspace.package] de Cargo.toml.
tcc_version() {
    local repo_root=$1
    awk '
        /^\[workspace\.package\]/ { in_section = 1; next }
        /^\[/ { in_section = 0 }
        in_section && $1 == "version" {
            value = $3
            gsub(/"/, "", value)
            print value
            exit
        }
    ' "$repo_root/Cargo.toml"
}

# Horodatage reproductible. Priorité :
#   1. SOURCE_DATE_EPOCH fourni par l'environnement ;
#   2. date du dernier commit git ;
#   3. constante alignée sur la première entrée du changelog (2026-11-06).
tcc_source_date_epoch() {
    local repo_root=$1
    if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
        printf '%s\n' "$SOURCE_DATE_EPOCH"
        return 0
    fi
    if [ -d "$repo_root/.git" ] && command -v git >/dev/null 2>&1; then
        git -C "$repo_root" log -1 --format=%ct 2>/dev/null && return 0
    fi
    printf '%s\n' "1793923200"
}

tcc_log() {
    printf '==> %s\n' "$*" >&2
}

tcc_warn() {
    printf 'avertissement: %s\n' "$*" >&2
}

tcc_die() {
    printf 'erreur: %s\n' "$*" >&2
    exit 1
}

tcc_require_cmd() {
    local cmd=$1
    command -v "$cmd" >/dev/null 2>&1 || tcc_die "commande requise absente: $cmd"
}

# Vérifie qu'un répertoire de destination est absolu et sûr.
tcc_check_destdir() {
    local destdir=$1
    case "$destdir" in
        /*) ;;
        *) tcc_die "DESTDIR doit être un chemin absolu: $destdir" ;;
    esac
    case "$destdir" in
        *..*) tcc_die "DESTDIR ne doit pas contenir '..': $destdir" ;;
        /) tcc_die "refus d'installer directement dans / : utiliser un DESTDIR" ;;
    esac
}
