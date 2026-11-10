#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Pipeline de release local : build Cargo verrouillé, arborescence FHS
# validée, puis paquets deb/rpm/Arch.
#
# Usage :
#   scripts/release/build.sh [options]
#
# Options :
#   --with-admin     construit/installe tccd-admin (phase 1, binaire requis)
#   --with-admin-hw  construit/installe tccd-admin-hw (TDP/ODM/cTGP, phase 2,
#                    binaire requis ; indépendant de --with-admin)
#   --source         produit l'archive source reproductible
#   --rpm            produit aussi le RPM (rpmbuild requis)
#   --arch           produit aussi le paquet Arch (makepkg requis, non root)
#   --deb            produit le .deb (défaut)
#   --no-deb         ne produit pas de .deb
#   --no-build       réutilise les binaires déjà présents
#   --no-validate    saute validate.sh
#
# Variables :
#   TCC_BUILD_DIR    répertoire de sortie (défaut: <repo>/build)
#   SOURCE_DATE_EPOCH horodatage reproductible
#   TCC_GPG_KEY      clé GPG pour signer build/packages/SHA256SUMS

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

WITH_ADMIN=1
WITH_ADMIN_HW=0
DO_DEB=1
DO_RPM=0
DO_ARCH=0
DO_SOURCE=0
DO_BUILD=1
DO_VALIDATE=1

while [ $# -gt 0 ]; do
    case "$1" in
        --with-admin) WITH_ADMIN=1 ;;
        --without-admin) WITH_ADMIN=0 ;;
        --with-admin-hw) WITH_ADMIN_HW=1 ;;
        --deb) DO_DEB=1 ;;
        --no-deb) DO_DEB=0 ;;
        --source) DO_SOURCE=1 ;;
        --rpm) DO_RPM=1 ;;
        --arch) DO_ARCH=1 ;;
        --no-build) DO_BUILD=0 ;;
        --no-validate) DO_VALIDATE=0 ;;
        -h|--help)
            awk 'NR==1 { next } /^set -euo pipefail/ { exit } { print }' "$0" \
                | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            tcc_die "argument inconnu: $1"
            ;;
    esac
    shift
done

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
BUILD_DIR=${TCC_BUILD_DIR:-"$REPO_ROOT/build"}
PKGROOT="$BUILD_DIR/pkgroot"
OUTDIR="$BUILD_DIR/packages"
VERSION=$(tcc_version "$REPO_ROOT")

install -d -m 0755 "$BUILD_DIR" "$OUTDIR"

if [ "$DO_BUILD" = "1" ]; then
    tcc_require_cmd cargo
    [ -f "$REPO_ROOT/Cargo.lock" ] || tcc_die "Cargo.lock absent (build non reproductible)"
    tcc_log "cargo build --release --locked (sans dev-overrides)"
    (
        cd "$REPO_ROOT"
        cargo build --release --locked
    )
else
    tcc_log "réutilisation des binaires existants"
fi

tcc_log "installation de l'arborescence FHS"
rm -rf "$PKGROOT"
install_args=("$PKGROOT")
if [ "$WITH_ADMIN" = "1" ]; then
    install_args+=(--with-admin)
else
    install_args+=(--without-admin)
fi
if [ "$WITH_ADMIN_HW" = "1" ]; then
    install_args+=(--with-admin-hw)
fi
TCC_WITH_ADMIN="$WITH_ADMIN" TCC_WITH_ADMIN_HW="$WITH_ADMIN_HW" \
    "$SCRIPT_DIR/install-tree.sh" "${install_args[@]}"

if [ "$DO_VALIDATE" = "1" ]; then
    tcc_log "validations statiques et d'arborescence"
    "$SCRIPT_DIR/validate.sh" --tree "$PKGROOT"
fi

if [ "$DO_SOURCE" = "1" ]; then
    "$SCRIPT_DIR/make-source-tarball.sh" "$BUILD_DIR"
fi

if [ "$DO_DEB" = "1" ]; then
    "$SCRIPT_DIR/make-deb.sh" "$PKGROOT" "$OUTDIR"
fi
if [ "$DO_RPM" = "1" ]; then
    TCC_WITH_ADMIN="$WITH_ADMIN" TCC_WITH_ADMIN_HW="$WITH_ADMIN_HW" \
        "$SCRIPT_DIR/make-rpm.sh" "$OUTDIR"
fi
if [ "$DO_ARCH" = "1" ]; then
    TCC_WITH_ADMIN="$WITH_ADMIN" TCC_WITH_ADMIN_HW="$WITH_ADMIN_HW" \
        "$SCRIPT_DIR/make-arch.sh" "$OUTDIR"
fi

# ---------------------------------------------------------------------------
# Sommes de contrôle et signature optionnelle
# ---------------------------------------------------------------------------
shopt -s nullglob
artifacts=("$OUTDIR"/*.deb "$OUTDIR"/*.rpm "$OUTDIR"/*.pkg.tar.* "$BUILD_DIR"/*.tar.gz)
shopt -u nullglob

if [ ${#artifacts[@]} -gt 0 ]; then
    sums="$OUTDIR/SHA256SUMS"
    rm -f "$sums"
    # Chaque artefact peut vivre dans $OUTDIR (paquets) ou $BUILD_DIR
    # (archive source) : hacher depuis SON répertoire, pas depuis $OUTDIR.
    : > "$sums"
    for artifact in "${artifacts[@]}"; do
        (
            cd "$(dirname -- "$artifact")"
            sha256sum -- "$(basename -- "$artifact")"
        ) >> "$sums"
    done
    chmod 0644 "$sums"
    tcc_log "sommes de contrôle : $sums"

    if [ -n "${TCC_GPG_KEY:-}" ]; then
        tcc_require_cmd gpg
        tcc_log "signature de SHA256SUMS avec la clé $TCC_GPG_KEY"
        gpg --batch --yes --armor --detach-sign \
            --local-user "$TCC_GPG_KEY" "$sums"
    fi
fi

printf '\n== Sortie\n'
printf '  version     : %s\n' "$VERSION"
printf '  arborescence: %s\n' "$PKGROOT"
printf '  paquets     : %s\n' "$OUTDIR"
