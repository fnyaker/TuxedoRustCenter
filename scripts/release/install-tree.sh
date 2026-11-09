#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Installe l'arborescence FHS de TuxedoRustCenter dans un DESTDIR.
#
# Source de vérité UNIQUE des chemins installés : les paquets deb, rpm et Arch
# appellent tous ce script, ce qui évite la dérive entre manifests.
#
# Usage :
#   scripts/release/install-tree.sh DESTDIR [--with-admin] [--with-admin-hw]
#
# Variables :
#   TCC_TARGET_DIR     répertoire target Cargo (défaut: <repo>/target)
#   TCC_WITH_ADMIN     1 pour installer tccd-admin phase 1 (binaire + unité +
#                      activation D-Bus). Refusé si le binaire est absent. Le
#                      binaire Cargo peut s'appeler `tccd-admin` ou
#                      `tcc-admin` ; nom installé stable `tccd-admin`.
#   TCC_WITH_ADMIN_HW  1 pour installer tccd-admin-hw (TDP/ODM/cTGP, phase 2
#                      AD-12 B2) : binaire + unité + activation D-Bus.
#                      INDÉPENDANT de TCC_WITH_ADMIN : `--with-admin` seul
#                      n'installe pas Admin.Hw. Refusé si le binaire est
#                      absent. Binaire accepté : `tccd-admin-hw` ou
#                      `tcc-admin-hw` ; nom installé stable `tccd-admin-hw`.
#   TCC_NO_MTIME_CLAMP=1 pour ne pas normaliser les mtimes (débogage).
#
# Aucun fichier n'est installé dans /opt. Aucun setuid. Aucun runtime
# Node/Python.

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
VERSION=$(tcc_version "$REPO_ROOT")
[ -n "$VERSION" ] || tcc_die "version introuvable dans Cargo.toml"

DESTDIR=${1:-}
if [ -z "$DESTDIR" ] || [ "$DESTDIR" = "-h" ] || [ "$DESTDIR" = "--help" ]; then
    awk 'NR==1 { next } /^set -euo pipefail/ { exit } { print }' "$0" \
        | sed 's/^# \{0,1\}//'
    [ -n "$DESTDIR" ] && exit 0 || exit 2
fi
shift || true

WITH_ADMIN=${TCC_WITH_ADMIN:-0}
WITH_ADMIN_HW=${TCC_WITH_ADMIN_HW:-0}
for arg in "$@"; do
    case "$arg" in
        --with-admin) WITH_ADMIN=1 ;;
        --with-admin-hw) WITH_ADMIN_HW=1 ;;
        *) tcc_die "argument inconnu: $arg" ;;
    esac
done

DESTDIR=${DESTDIR%/}
tcc_check_destdir "$DESTDIR"

TARGET_DIR=${TCC_TARGET_DIR:-"$REPO_ROOT/target"}
BIN_DIR="$TARGET_DIR/release"

DAEMON_BIN="$BIN_DIR/tcc-daemon"
USER_BIN="$BIN_DIR/tcc-user"
GUI_BIN="$BIN_DIR/tcc-gui"
CLI_BIN="$BIN_DIR/tcc-cli"
# Le crate s'appelle `tcc-admin` ; le binaire peut être nommé `tccd-admin`
# (recommandé) ou `tcc-admin` (nom par défaut Cargo). Le nom installé est
# toujours `tccd-admin` (contrat systemd/D-Bus).
ADMIN_BIN=""
# Phase 2 (AD-12 B2) : binaire distinct, jamais fourni par le crate phase 1.
ADMIN_HW_BIN=""

for binary in "$DAEMON_BIN" "$USER_BIN" "$GUI_BIN" "$CLI_BIN"; do
    [ -f "$binary" ] || tcc_die "binaire manquant: $binary
Construire d'abord : cargo build --release --locked (sans dev-overrides)"
done

if [ "$WITH_ADMIN" = "1" ]; then
    for candidate in tccd-admin tcc-admin; do
        if [ -f "$BIN_DIR/$candidate" ]; then
            ADMIN_BIN="$BIN_DIR/$candidate"
            break
        fi
    done
    [ -n "$ADMIN_BIN" ] || tcc_die "TCC_WITH_ADMIN=1 mais aucun binaire admin
n'a été trouvé dans $BIN_DIR (attendu: tccd-admin ou tcc-admin).
Le paquet ne doit pas dépendre d'un exécutable inexistant : construire le
crate tcc-admin ou laisser TCC_WITH_ADMIN=0."
    tcc_log "tccd-admin installé depuis $(basename -- "$ADMIN_BIN") (phase 1, activation D-Bus)"
else
    tcc_log "tccd-admin non installé (TCC_WITH_ADMIN=0)"
fi

if [ "$WITH_ADMIN_HW" = "1" ]; then
    for candidate in tccd-admin-hw tcc-admin-hw; do
        if [ -f "$BIN_DIR/$candidate" ]; then
            ADMIN_HW_BIN="$BIN_DIR/$candidate"
            break
        fi
    done
    [ -n "$ADMIN_HW_BIN" ] || tcc_die "TCC_WITH_ADMIN_HW=1 mais aucun binaire
Admin.Hw n'a été trouvé dans $BIN_DIR (attendu: tccd-admin-hw ou
tcc-admin-hw).
Le service TDP/ODM/cTGP n'a pas de binaire : le paquet ne doit pas installer
une activation D-Bus qui échoue au premier appel. Construire le crate
tcc-admin (bin tccd-admin-hw) ou laisser TCC_WITH_ADMIN_HW=0."
    tcc_log "tccd-admin-hw installé depuis $(basename -- "$ADMIN_HW_BIN") (phase 2, activation D-Bus)"
else
    tcc_log "tccd-admin-hw non installé (TCC_WITH_ADMIN_HW=0)"
fi

# ---------------------------------------------------------------------------
# Répertoires (modes explicites, jamais hérités de l'umask)
# ---------------------------------------------------------------------------
directories=(
    usr/bin
    usr/libexec/tuxedo-rust-center
    usr/lib/systemd/system
    usr/lib/systemd/user
    usr/lib/systemd/user/graphical-session.target.wants
    usr/lib/udev/rules.d
    usr/lib/tmpfiles.d
    usr/share/applications
    usr/share/dbus-1/system.d
    usr/share/icons/hicolor/scalable/apps
    usr/share/icons/hicolor/256x256/apps
    usr/share/icons/hicolor/128x128/apps
    usr/share/icons/hicolor/64x64/apps
    usr/share/icons/hicolor/48x48/apps
    usr/share/icons/hicolor/32x32/apps
    usr/share/licenses/tuxedo-rust-center
    usr/share/metainfo
    usr/share/polkit-1/actions
    usr/share/doc/tuxedo-rust-center
    usr/share/tuxedo-rust-center
    etc/tuxedo-rust-center
    var/lib/tuxedo-rust-center
)
for dir in "${directories[@]}"; do
    install -d -m 0755 "$DESTDIR/$dir"
done

install_file() {
    local mode=$1 src=$2 dest=$3
    [ -f "$src" ] || tcc_die "fichier source absent: $src"
    install -D -m "$mode" "$src" "$DESTDIR/$dest"
}

# ---------------------------------------------------------------------------
# Binaires
# ---------------------------------------------------------------------------
install_file 0755 "$GUI_BIN" usr/bin/tuxedo-rust-center
install_file 0755 "$CLI_BIN" usr/bin/tcc-cli
install_file 0755 "$DAEMON_BIN" usr/libexec/tuxedo-rust-center/tccd
install_file 0755 "$USER_BIN" usr/libexec/tuxedo-rust-center/tccd-user
if [ "$WITH_ADMIN" = "1" ]; then
    install_file 0755 "$ADMIN_BIN" usr/libexec/tuxedo-rust-center/tccd-admin
fi
if [ "$WITH_ADMIN_HW" = "1" ]; then
    install_file 0755 "$ADMIN_HW_BIN" usr/libexec/tuxedo-rust-center/tccd-admin-hw
fi

# ---------------------------------------------------------------------------
# Intégration système
# ---------------------------------------------------------------------------
install_file 0644 "$REPO_ROOT/packaging/systemd/tccd.service" \
    usr/lib/systemd/system/tccd.service
# tccd-sleep.service n'est PAS installé (REVIEW_JALON1 R-06) : la veille sera
# gérée par org.freedesktop.login1.PrepareForSleep dans tccd. Le fichier
# packaging/systemd/tccd-sleep.service.example est une référence, pas un livrable.
install_file 0644 "$REPO_ROOT/packaging/systemd/tccd-user.service" \
    usr/lib/systemd/user/tccd-user.service

# Activation par session graphique, sans `systemctl --user enable` par
# utilisateur. Le lien est relatif, donc valable quel que soit le préfixe.
ln -sfn ../tccd-user.service \
    "$DESTDIR/usr/lib/systemd/user/graphical-session.target.wants/tccd-user.service"

install_file 0644 "$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.conf" \
    usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf
install_file 0644 "$REPO_ROOT/packaging/polkit/com.tuxedocomputers.tccd.policy" \
    usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy
install_file 0644 "$REPO_ROOT/packaging/udev/99-tuxedo-rust-center-webcam.rules" \
    usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules
install_file 0644 "$REPO_ROOT/packaging/tmpfiles/tuxedo-rust-center.conf" \
    usr/lib/tmpfiles.d/tuxedo-rust-center.conf

if [ "$WITH_ADMIN" = "1" ]; then
    install_file 0644 "$REPO_ROOT/packaging/systemd/tccd-admin.service" \
        usr/lib/systemd/system/tccd-admin.service
    install_file 0644 "$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.Admin.service" \
        usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service
fi
# Phase 2 : condition propre. `--with-admin` seul ne doit JAMAIS installer
# Admin.Hw (l'unité relâche PrivateDevices/ProtectKernelTunables).
if [ "$WITH_ADMIN_HW" = "1" ]; then
    install_file 0644 "$REPO_ROOT/packaging/systemd/tccd-admin-hw.service" \
        usr/lib/systemd/system/tccd-admin-hw.service
    install_file 0644 "$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.Admin.Hw.service" \
        usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service
fi

# ---------------------------------------------------------------------------
# Bureau, AppStream, icônes
# ---------------------------------------------------------------------------
install_file 0644 "$REPO_ROOT/packaging/desktop/tuxedo-rust-center.desktop" \
    usr/share/applications/tuxedo-rust-center.desktop
install_file 0644 "$REPO_ROOT/packaging/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml" \
    usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml
install_file 0644 "$REPO_ROOT/packaging/icons/hicolor/scalable/apps/tuxedo-rust-center.svg" \
    usr/share/icons/hicolor/scalable/apps/tuxedo-rust-center.svg
for size in 256 128 64 48 32; do
    install_file 0644 "$REPO_ROOT/packaging/icons/hicolor/${size}x${size}/apps/tuxedo-rust-center.png" \
        "usr/share/icons/hicolor/${size}x${size}/apps/tuxedo-rust-center.png"
done

# ---------------------------------------------------------------------------
# Ressources Qt : /usr/share/tuxedo-rust-center/{qml,assets}
# Les QML résolvent ../../assets/... depuis qml/ : la disposition relative est
# donc un contrat (crates/tcc-gui/qml/components/AppIcon.qml).
# ---------------------------------------------------------------------------
copy_tree() {
    local src_root=$1 dest_root=$2
    [ -d "$src_root" ] || tcc_die "arborescence source absente: $src_root"
    local path
    while IFS= read -r -d '' path; do
        install -d -m 0755 "$dest_root/${path#./}"
    done < <(cd "$src_root" && find . -mindepth 1 -type d -print0)
    while IFS= read -r -d '' path; do
        install -D -m 0644 "$src_root/${path#./}" "$dest_root/${path#./}"
    done < <(cd "$src_root" && find . -type f -print0)
}

copy_tree "$REPO_ROOT/crates/tcc-gui/qml" \
    "$DESTDIR/usr/share/tuxedo-rust-center/qml"
copy_tree "$REPO_ROOT/crates/tcc-gui/assets" \
    "$DESTDIR/usr/share/tuxedo-rust-center/assets"

# ---------------------------------------------------------------------------
# Configuration administrateur et état (D2)
# ---------------------------------------------------------------------------
install_file 0644 "$REPO_ROOT/packaging/etc/README" etc/tuxedo-rust-center/README

# ---------------------------------------------------------------------------
# Licences et documentation
# ---------------------------------------------------------------------------
install_file 0644 "$REPO_ROOT/LICENSE" \
    usr/share/licenses/tuxedo-rust-center/LICENSE
install_file 0644 "$REPO_ROOT/distro/deb/copyright" \
    usr/share/doc/tuxedo-rust-center/copyright

ln -sfn ../../licenses/tuxedo-rust-center/LICENSE \
    "$DESTDIR/usr/share/doc/tuxedo-rust-center/LICENSE"

changelog_tmp=$(mktemp)
trap 'rm -f "$changelog_tmp"' EXIT
gzip -9n -c "$REPO_ROOT/packaging/changelog" > "$changelog_tmp"
install -D -m 0644 "$changelog_tmp" \
    "$DESTDIR/usr/share/doc/tuxedo-rust-center/changelog.gz"

documentation=(
    "docs/linux/PLATFORM_DESIGN.md"
    "docs/linux/TCC_USER_DESIGN.md"
    "docs/DBUS_API.md"
    "docs/packaging/MIGRATION.md"
    "docs/packaging/SECURITY.md"
    "crates/tcc-gui/assets/ATTRIBUTION.md"
)
for doc in "${documentation[@]}"; do
    install_file 0644 "$REPO_ROOT/$doc" \
        "usr/share/doc/tuxedo-rust-center/$(basename -- "$doc")"
done

# ---------------------------------------------------------------------------
# Reproductibilité : mtimes normalisés
# ---------------------------------------------------------------------------
if [ "${TCC_NO_MTIME_CLAMP:-0}" != "1" ]; then
    epoch=$(tcc_source_date_epoch "$REPO_ROOT")
    find "$DESTDIR" -exec touch -h -d "@$epoch" {} +
fi

tcc_log "arborescence installée dans $DESTDIR (version $VERSION)"
