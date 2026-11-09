#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Construit un paquet Arch Linux avec makepkg à partir de l'archive source
# reproductible. makepkg refuse de tourner en root : ce script doit être lancé
# par un utilisateur non privilégié (ou via `su builder -c ...` en CI).
#
# Usage :
#   scripts/release/make-arch.sh [OUTDIR]
#
# Variables :
#   TCC_BUILD_DIR   répertoire de build (défaut: <repo>/build)

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

tcc_require_cmd makepkg
tcc_require_cmd sha256sum

if [ "$(id -u)" -eq 0 ]; then
    tcc_die "makepkg refuse de tourner en root ; utiliser un utilisateur de build"
fi

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
VERSION=$(tcc_version "$REPO_ROOT")
[ -n "$VERSION" ] || tcc_die "version introuvable dans Cargo.toml"

BUILD_DIR=${TCC_BUILD_DIR:-"$REPO_ROOT/build"}
OUTDIR=${1:-"$BUILD_DIR/packages"}
mkdir -p "$OUTDIR"

tarball=$("$SCRIPT_DIR/make-source-tarball.sh" "$BUILD_DIR")

work="$BUILD_DIR/arch"
rm -rf "$work"
install -d -m 0755 "$work"
cp "$REPO_ROOT/distro/arch/PKGBUILD" "$work/PKGBUILD"
cp "$REPO_ROOT/distro/arch/tuxedo-rust-center.install" "$work/tuxedo-rust-center.install"
cp "$tarball" "$work/"

if [ "${TCC_WITH_ADMIN:-0}" = "1" ]; then
    tcc_log "paquet Arch avec tccd-admin (_with_admin=1)"
    sed -i 's/^_with_admin=0$/_with_admin=1/' "$work/PKGBUILD"
    grep -q '^_with_admin=1$' "$work/PKGBUILD" \
        || tcc_die "impossible d'activer _with_admin dans le PKGBUILD"
fi
# Phase 2 (AD-12 B2) : condition indépendante de _with_admin.
if [ "${TCC_WITH_ADMIN_HW:-0}" = "1" ]; then
    tcc_log "paquet Arch avec tccd-admin-hw (_with_admin_hw=1)"
    sed -i 's/^_with_admin_hw=0$/_with_admin_hw=1/' "$work/PKGBUILD"
    grep -q '^_with_admin_hw=1$' "$work/PKGBUILD" \
        || tcc_die "impossible d'activer _with_admin_hw dans le PKGBUILD"
fi

hash=$(sha256sum "$tarball" | cut -d' ' -f1)
# Le PKGBUILD du dépôt utilise SKIP (build local sans source publiée). Ici la
# somme est calculée sur l'archive réellement construite.
sed -i "s|sha256sums=('SKIP')|sha256sums=('$hash')|" "$work/PKGBUILD"
grep -q "sha256sums=('$hash')" "$work/PKGBUILD" \
    || tcc_die "impossible d'injecter la somme sha256 dans le PKGBUILD"

tcc_log "makepkg (utilisateur $(id -un))"
(
    cd "$work"
    PKGDEST="$OUTDIR" \
    SRCDEST="$work" \
    BUILDDIR="$work/build" \
    PKGEXT='.pkg.tar.zst' \
    makepkg --noconfirm --nocheck --force
)

shopt -s nullglob
packages=("$OUTDIR"/tuxedo-rust-center-"$VERSION"-*.pkg.tar.zst)
shopt -u nullglob
[ ${#packages[@]} -gt 0 ] || tcc_die "makepkg n'a produit aucun paquet"

for package in "${packages[@]}"; do
    tcc_log "paquet produit : $package"
    printf '%s\n' "$package"
done
