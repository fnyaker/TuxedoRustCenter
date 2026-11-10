#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Construit un RPM binaire à partir de l'archive source reproductible.
# Requiert rpmbuild (paquet rpm-build / rpmdevtools selon la distribution).
#
# Usage :
#   scripts/release/make-rpm.sh [OUTDIR]
#
# Variables :
#   TCC_BUILD_DIR   répertoire de build (défaut: <repo>/build)

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

tcc_require_cmd rpmbuild

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
VERSION=$(tcc_version "$REPO_ROOT")
[ -n "$VERSION" ] || tcc_die "version introuvable dans Cargo.toml"

BUILD_DIR=${TCC_BUILD_DIR:-"$REPO_ROOT/build"}
OUTDIR=${1:-"$BUILD_DIR/packages"}
mkdir -p "$OUTDIR"

tarball=$("$SCRIPT_DIR/make-source-tarball.sh" "$BUILD_DIR")

topdir="$BUILD_DIR/rpmbuild"
rm -rf "$topdir"
install -d -m 0755 \
    "$topdir/BUILD" \
    "$topdir/RPMS" \
    "$topdir/SOURCES" \
    "$topdir/SPECS" \
    "$topdir/SRPMS"
cp "$tarball" "$topdir/SOURCES/"

epoch=$(tcc_source_date_epoch "$REPO_ROOT")
export SOURCE_DATE_EPOCH="$epoch"

rpmbuild_args=(
    -bb
    --define "_topdir $topdir"
    --define "_sourcedir $topdir/SOURCES"
    --define "_buildhost tuxedo-rust-center"
    --define "use_source_date_epoch_as_buildtime 1"
    --define "clamp_mtime_to_source_date_epoch 1"
)
if [ "${TCC_WITH_ADMIN:-1}" = "0" ]; then
    tcc_log "RPM sans tccd-admin (--without admin)"
    rpmbuild_args+=(--without admin)
fi
# Phase 2 (AD-12 B2) : condition RPM indépendante. --with admin seul
# n'installe pas Admin.Hw.
if [ "${TCC_WITH_ADMIN_HW:-0}" = "1" ]; then
    tcc_log "RPM avec tccd-admin-hw (--with admin_hw)"
    rpmbuild_args+=(--with admin_hw)
fi

tcc_log "rpmbuild -bb (reproductible, sans --nodeps)"
rpmbuild "${rpmbuild_args[@]}" "$REPO_ROOT/distro/rpm/tuxedo-rust-center.spec"

shopt -s nullglob
rpms=("$topdir"/RPMS/*/*.rpm)
shopt -u nullglob
[ ${#rpms[@]} -gt 0 ] || tcc_die "rpmbuild n'a produit aucun RPM"

for rpm in "${rpms[@]}"; do
    cp "$rpm" "$OUTDIR/"
    tcc_log "paquet produit : $OUTDIR/$(basename -- "$rpm")"
    printf '%s\n' "$OUTDIR/$(basename -- "$rpm")"
done
