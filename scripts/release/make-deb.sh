#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Construit un paquet Debian (.deb) à partir de l'arborescence déjà installée
# par scripts/release/install-tree.sh. Aucun cargo-deb, aucun dh, aucun /opt.
#
# Usage :
#   scripts/release/make-deb.sh [PKGROOT] [OUTDIR]
#
# Variables :
#   TCC_ARCH            architecture Debian (défaut: dpkg --print-architecture)
#   TCC_DEB_REVISION    révision Debian (défaut: 1)
#   TCC_DEB_DISTRIBUTION distribution du changelog (défaut: noble, Ubuntu 24.04)
#   TCC_BUILD_DIR       répertoire de build (défaut: <repo>/build)

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

tcc_require_cmd dpkg-deb
tcc_require_cmd md5sum
tcc_require_cmd gzip
tcc_require_cmd install
tcc_require_cmd find

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
VERSION=$(tcc_version "$REPO_ROOT")
[ -n "$VERSION" ] || tcc_die "version introuvable dans Cargo.toml"

BUILD_DIR=${TCC_BUILD_DIR:-"$REPO_ROOT/build"}
PKGROOT=${1:-"$BUILD_DIR/pkgroot"}
OUTDIR=${2:-"$BUILD_DIR/packages"}
ARCH=${TCC_ARCH:-$(dpkg --print-architecture)}
REVISION=${TCC_DEB_REVISION:-1}
# Distribution visée par le paquet de release : Ubuntu 24.04 LTS (noble).
# Le changelog embarqué est normalisé sur cette valeur pour que lintian et
# `apt` ne voient jamais une distribution Debian inattendue.
DEB_DISTRIBUTION=${TCC_DEB_DISTRIBUTION:-noble}
case "$DEB_DISTRIBUTION" in
    ''|*[!a-z0-9._-]*) tcc_die "distribution Debian invalide: $DEB_DISTRIBUTION" ;;
esac
DEB_VERSION="$VERSION-$REVISION"

[ -d "$PKGROOT" ] || tcc_die "arborescence absente: $PKGROOT (lancer install-tree.sh)"
mkdir -p "$OUTDIR"

work=$(mktemp -d "$BUILD_DIR/.deb.XXXXXX")
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT

root="$work/root"
install -d -m 0755 "$root"
cp -a "$PKGROOT/." "$root/"

# Taille installée en KiB, calculée avant DEBIAN/ (exclu du paquet).
installed_size=$(du -sk "$root" | cut -f1)

deb_control="$root/DEBIAN"
install -d -m 0755 "$deb_control"

sed \
    -e "s|@VERSION@|$DEB_VERSION|g" \
    -e "s|@ARCH@|$ARCH|g" \
    -e "s|@INSTALLED_SIZE@|$installed_size|g" \
    "$REPO_ROOT/distro/deb/control.in" > "$deb_control/control"
chmod 0644 "$deb_control/control"

# Conffile unique : la note de configuration administrateur. Les fichiers de
# /var/lib sont de l'état, pas de la configuration conffile.
printf '/etc/tuxedo-rust-center/README\n' > "$deb_control/conffiles"
chmod 0644 "$deb_control/conffiles"

# Debian exige `changelog.Debian.gz` (lintian E: debian-changelog-file-missing-
# or-wrong-name). Le staging commun garde `changelog.gz` pour rpm/Arch ; le
# .deb le renomme pour éviter `duplicate-changelog-files`.
mv "$root/usr/share/doc/tuxedo-rust-center/changelog.gz" \
    "$root/usr/share/doc/tuxedo-rust-center/changelog.Debian.gz"

# Normalisation de la distribution du changelog sur la cible du paquet
# (noble = Ubuntu 24.04 LTS). Le fichier source est partagé avec rpm/Arch ;
# seule la copie embarquée dans le .deb est réécrite. `gzip -9n` conserve la
# reproductibilité (aucun horodatage dans l'archive).
changelog_deb="$root/usr/share/doc/tuxedo-rust-center/changelog.Debian.gz"
changelog_plain="$work/changelog.Debian"
gzip -dc "$changelog_deb" > "$changelog_plain"
sed -E -i \
    -e "1s/^([^()]*)\(([^()]*)\)[[:space:]]+[^;]+;/\1(\2) $DEB_DISTRIBUTION;/" \
    "$changelog_plain"
first_line=$(sed -n '1p' "$changelog_plain")
if ! printf '%s\n' "$first_line" | grep -qF ") $DEB_DISTRIBUTION;"; then
    tcc_die "changelog Debian non normalisé en $DEB_DISTRIBUTION: $first_line"
fi
gzip -9n -c "$changelog_plain" > "$changelog_deb"

install -m 0755 "$REPO_ROOT/distro/deb/postinst" "$deb_control/postinst"
install -m 0755 "$REPO_ROOT/distro/deb/prerm" "$deb_control/prerm"
install -m 0755 "$REPO_ROOT/distro/deb/postrm" "$deb_control/postrm"

# md5sums triés, chemins relatifs sans './', DEBIAN exclu.
(
    cd "$root"
    LC_ALL=C find . -path './DEBIAN' -prune -o -type f -printf '%P\0' \
        | LC_ALL=C sort -z \
        | while IFS= read -r -d '' file; do
              md5sum -- "$file"
          done
) > "$deb_control/md5sums"
chmod 0644 "$deb_control/md5sums"

# Reproductibilité : tous les mtimes (y compris DEBIAN généré) ramenés à
# SOURCE_DATE_EPOCH avant l'archivage.
epoch=$(tcc_source_date_epoch "$REPO_ROOT")
find "$root" -exec touch -h -d "@$epoch" {} +

output="$OUTDIR/tuxedo-rust-center_${DEB_VERSION}_${ARCH}.deb"
rm -f "$output"
tcc_log "construction du .deb (xz -9) : $output"
dpkg-deb --root-owner-group -Zxz -z9 --build "$root" "$output" >/dev/null

# Contrôle immédiat : aucun /opt, aucun setuid.
contents=$(dpkg-deb --contents "$output")
if printf '%s\n' "$contents" | grep -E '(^| )\./opt/' >/dev/null; then
    tcc_die "le .deb contient un fichier sous /opt"
fi
if printf '%s\n' "$contents" | grep -E '^-rws|^-r-xr-s|^-rwsr' >/dev/null; then
    tcc_die "le .deb contient un fichier setuid/setgid"
fi

size=$(stat -c '%s' "$output")
tcc_log "paquet produit : $output ($size octets)"
printf '%s\n' "$output"
