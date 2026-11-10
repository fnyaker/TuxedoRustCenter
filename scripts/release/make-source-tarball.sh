#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Crée une archive source reproductible pour les builds RPM et Arch.
# Sortie sur stdout : chemin du .tar.gz (les logs vont sur stderr).
#
# Usage :
#   scripts/release/make-source-tarball.sh [OUTDIR]
#
# Variables :
#   TCC_BUILD_DIR   répertoire de build (défaut: <repo>/build)

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

tcc_require_cmd tar
tcc_require_cmd gzip

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
VERSION=$(tcc_version "$REPO_ROOT")
[ -n "$VERSION" ] || tcc_die "version introuvable dans Cargo.toml"

BUILD_DIR=${TCC_BUILD_DIR:-"$REPO_ROOT/build"}
OUTDIR=${1:-"$BUILD_DIR"}
mkdir -p "$OUTDIR"

name="tuxedo-rust-center-$VERSION"
epoch=$(tcc_source_date_epoch "$REPO_ROOT")
work=$(mktemp -d "$BUILD_DIR/.tar.XXXXXX")
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT

install -d -m 0755 "$work/$name"

tcc_log "copie de l'arbre source (hors target, build, référence TCC)"
tar -C "$REPO_ROOT" -cf - \
    --exclude=./target \
    --exclude=./crates/tcc-gpui/target \
    --exclude=./build \
    --exclude=./dist \
    --exclude=./.git \
    --exclude=./tuxedo-control-center-master \
    --exclude=./opencode.json \
    --exclude=./docs/ui/captures \
    --exclude='*.bak' \
    --exclude='*.swp' \
    . | tar -C "$work/$name" -xf -

# Vérification : les fichiers indispensables au build sont présents.
for required in \
    Cargo.toml \
    Cargo.lock \
    LICENSE \
    scripts/release/install-tree.sh \
    scripts/release/lib.sh \
    packaging/systemd/tccd.service \
    distro/deb/copyright \
    crates/tcc-gui/qml/Main.qml; do
    [ -e "$work/$name/$required" ] || tcc_die "fichier absent de l'archive source: $required"
done

output="$OUTDIR/$name.tar.gz"
rm -f "$output"
tcc_log "création de l'archive source reproductible"
tar -C "$work" -cf - \
    --sort=name \
    --mtime="@$epoch" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    "$name" | gzip -9n > "$output"

tcc_log "archive source : $output"
printf '%s\n' "$output"
