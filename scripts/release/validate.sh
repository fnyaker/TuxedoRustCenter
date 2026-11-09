#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Validations statiques et d'arborescence du packaging TuxedoRustCenter.
#
# Usage :
#   scripts/release/validate.sh                 # fichiers d'intégration du dépôt
#   scripts/release/validate.sh --tree DIR      # + arborescence installée
#   scripts/release/validate.sh --repo
#
# Variables :
#   TCC_VALIDATE_STRICT=1   échoue si un outil de validation est absent
#                           (défaut : avertit et continue)
#
# Toutes les commandes sont exécutées sans shell interpolé sur des données
# externes ; les chemins viennent du dépôt ou d'un argument vérifié.

set -uo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
STRICT=${TCC_VALIDATE_STRICT:-0}
TREE=""
MODE=repo

while [ $# -gt 0 ]; do
    case "$1" in
        --repo)
            MODE=repo
            shift
            ;;
        --tree)
            [ $# -ge 2 ] || tcc_die "--tree attend un répertoire"
            TREE=$2
            MODE=tree
            shift 2
            ;;
        -h|--help)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            tcc_die "argument inconnu: $1"
            ;;
    esac
done

FAILURES=0
SKIPPED=()
CHECKS=0

fail() {
    FAILURES=$((FAILURES + 1))
    printf 'ÉCHEC: %s\n' "$*" >&2
}

pass() {
    CHECKS=$((CHECKS + 1))
}

skip() {
    SKIPPED+=("$1")
    printf 'IGNORÉ: %s\n' "$1" >&2
}

need_tool() {
    if command -v "$1" >/dev/null 2>&1; then
        return 0
    fi
    if [ "$STRICT" = "1" ]; then
        fail "outil requis absent: $1"
        return 1
    fi
    skip "outil absent: $1"
    return 1
}

section() {
    printf '\n== %s\n' "$*"
}

# Lignes actives d'un fichier systemd/udev/tmpfiles : commentaires `#` et
# lignes vides exclus. Évite les faux positifs sur la documentation intégrée.
active_lines() {
    awk '!/^[[:space:]]*#/ && NF' "$1"
}

# Éléments XML actifs : commentaires <!-- ... --> exclus, y compris
# multi-lignes (les commentaires expliquent souvent ce qui est interdit).
strip_xml_comments() {
    awk '
        BEGIN { in_comment = 0 }
        {
            line = $0
            result = ""
            while (1) {
                if (in_comment) {
                    pos = index(line, "-->")
                    if (pos == 0) { line = ""; break }
                    line = substr(line, pos + 3)
                    in_comment = 0
                } else {
                    pos = index(line, "<!--")
                    if (pos == 0) { result = result line; break }
                    result = result substr(line, 1, pos - 1)
                    line = substr(line, pos + 4)
                    in_comment = 1
                }
            }
            if (length(result) > 0) print result
        }
    ' "$1"
}

# Un tag XML par ligne, commentaires retirés : permet de vérifier qu'un
# élément multi-lignes contient bien send_interface/send_member.
xml_tags() {
    strip_xml_comments "$1" | tr '\n' ' ' | sed 's/>[[:space:]]*</>\n</g'
}

# Bloc <policy context="default"> uniquement : la politique root est
# volontairement permissive (root est déjà root).
default_policy() {
    xml_tags "$1" | awk '/<policy context="default">/{f=1} f{print} f&&/<\/policy>/{exit}'
}

# Bloc <policy user="root"> uniquement (contrôle Control1 root-only).
root_policy() {
    xml_tags "$1" | awk '/<policy user="root">/{f=1} f{print} f&&/<\/policy>/{exit}'
}

# ---------------------------------------------------------------------------
section "Fichiers d'intégration"

UNITS=(
    "$REPO_ROOT/packaging/systemd/tccd.service"
    "$REPO_ROOT/packaging/systemd/tccd-user.service"
    "$REPO_ROOT/packaging/systemd/tccd-admin.service"
    "$REPO_ROOT/packaging/systemd/tccd-admin-hw.service"
)

for unit in "${UNITS[@]}"; do
    [ -f "$unit" ] || fail "unité absente: $unit"
done

# tccd-sleep.service n'est pas un livrable (R-06) : le fichier de référence
# doit rester explicitement marqué `.example` et non installé.
if [ -f "$REPO_ROOT/packaging/systemd/tccd-sleep.service" ]; then
    fail "tccd-sleep.service ne doit pas exister comme unité livrable (voir .example)"
else
    pass
fi
if grep -qxF "# RÉFÉRENCE UNIQUEMENT — CE FICHIER N'EST PAS INSTALLÉ." \
    "$REPO_ROOT/packaging/systemd/tccd-sleep.service.example"; then
    pass
else
    fail "tccd-sleep.service.example doit porter la mention NON INSTALLÉ"
fi
if active_lines "$REPO_ROOT/scripts/release/install-tree.sh" \
    | grep -E 'systemd/tccd-sleep\.service"' >/dev/null; then
    fail "install-tree.sh installe encore tccd-sleep.service"
else
    pass
fi

if need_tool systemd-analyze; then
    verify_output=$(systemd-analyze verify "${UNITS[@]}" 2>&1)
    # Les binaires ne sont pas installés dans l'arbre hôte : « is not
    # executable » est attendu. Toute autre ligne concernant nos units est une
    # erreur.
    unit_errors=$(printf '%s\n' "$verify_output" \
        | grep -E '^(tccd|tccd-admin|tccd-admin-hw|tccd-user)\.service:' \
        | grep -v 'is not executable' || true)
    if [ -n "$unit_errors" ]; then
        fail "systemd-analyze verify:
$unit_errors"
    else
        pass
    fi
fi

if need_tool xmllint; then
    if xmllint --noout \
        "$REPO_ROOT/packaging/polkit/com.tuxedocomputers.tccd.policy" \
        "$REPO_ROOT/packaging/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml" \
        "$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.conf" 2>/dev/null; then
        pass
    else
        fail "XML invalide (polkit, metainfo ou politique D-Bus)"
    fi
fi

if need_tool desktop-file-validate; then
    if desktop-file-validate "$REPO_ROOT/packaging/desktop/tuxedo-rust-center.desktop" >/dev/null 2>&1; then
        pass
    else
        fail "desktop-file-validate a échoué"
    fi
fi

if need_tool appstreamcli; then
    if appstreamcli validate --no-net \
        "$REPO_ROOT/packaging/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml" >/dev/null 2>&1; then
        pass
    else
        fail "appstreamcli validate a échoué"
    fi
fi

if need_tool udevadm; then
    if udevadm verify "$REPO_ROOT/packaging/udev/99-tuxedo-rust-center-webcam.rules" >/dev/null 2>&1; then
        pass
    else
        fail "udevadm verify a échoué"
    fi
fi

# ---------------------------------------------------------------------------
section "Hardening systemd (D-règles, D1, D2)"

TCCD="$REPO_ROOT/packaging/systemd/tccd.service"
TCCD_USER="$REPO_ROOT/packaging/systemd/tccd-user.service"
TCCD_ADMIN="$REPO_ROOT/packaging/systemd/tccd-admin.service"
TCCD_ADMIN_HW="$REPO_ROOT/packaging/systemd/tccd-admin-hw.service"

require_line() {
    local file=$1 line=$2
    if grep -qxF -- "$line" "$file"; then
        pass
    else
        fail "$(basename -- "$file"): ligne manquante « $line »"
    fi
}

forbid_pattern() {
    local file=$1 pattern=$2
    if active_lines "$file" | grep -E -- "$pattern" >/dev/null; then
        fail "$(basename -- "$file"): motif interdit présent « $pattern »"
    else
        pass
    fi
}

forbid_xml_pattern() {
    local file=$1 pattern=$2
    if strip_xml_comments "$file" | grep -E -- "$pattern" >/dev/null; then
        fail "$(basename -- "$file"): motif XML interdit présent « $pattern »"
    else
        pass
    fi
}

require_line "$TCCD" "Type=dbus"
require_line "$TCCD" "BusName=com.tuxedocomputers.tccd"
# PPD est lui-même After=multi-user.target ; tccd étant WantedBy=multi-user,
# un After=PPD crée un cycle et systemd supprime le démarrage de tccd au boot.
require_line "$TCCD" "Wants=power-profiles-daemon.service"
forbid_pattern "$TCCD" '^After=.*power-profiles-daemon\.service'
require_line "$TCCD" "NoNewPrivileges=yes"
require_line "$TCCD" "ProtectSystem=strict"
require_line "$TCCD" "ProtectKernelTunables=no"
require_line "$TCCD" "ProtectHome=yes"
require_line "$TCCD" "PrivateTmp=yes"
require_line "$TCCD" "PrivateDevices=no"
require_line "$TCCD" "DevicePolicy=closed"
require_line "$TCCD" "DeviceAllow=/dev/tuxedo_io rw"
# V-17 : lecture V4L2 uniquement, jamais rw.
require_line "$TCCD" "DeviceAllow=char-video4linux r"
require_line "$TCCD" "ReadWritePaths=/var/lib/tuxedo-rust-center /run/tuxedo-rust-center"
require_line "$TCCD" "RestrictAddressFamilies=AF_UNIX AF_NETLINK"
require_line "$TCCD" "MemoryDenyWriteExecute=yes"
require_line "$TCCD" "SystemCallArchitectures=native"
require_line "$TCCD" "SecureBits=noroot-locked"
require_line "$TCCD" "LimitCORE=0"
require_line "$TCCD" "UMask=0077"
require_line "$TCCD" "CapabilityBoundingSet="
require_line "$TCCD" "AmbientCapabilities="
require_line "$TCCD" "MemoryMax=256M"
require_line "$TCCD" "TasksMax=64"
require_line "$TCCD" "CPUQuota=25%"
require_line "$TCCD" "StartLimitIntervalSec=60"
require_line "$TCCD" "StartLimitBurst=5"
require_line "$TCCD" "ProtectKernelLogs=yes"
require_line "$TCCD" "KeyringMode=private"
require_line "$TCCD" "Wants=modprobe@tuxedo_io.service"
require_line "$TCCD" "After=systemd-modules-load.service modprobe@tuxedo_io.service"
require_line "$TCCD" "User=root"
forbid_pattern "$TCCD" '^ReadWritePaths=.*/etc'
forbid_pattern "$TCCD" '^ProtectKernelTunables=yes'
forbid_pattern "$TCCD" '^EnvironmentFile='
forbid_pattern "$TCCD" 'TCC_SYSFS_ROOT'
forbid_pattern "$TCCD" 'TCC_CONFIG_DIR'
forbid_pattern "$TCCD" '^DynamicUser='
forbid_pattern "$TCCD" '^DeviceAllow=char-video4linux rw'
forbid_pattern "$TCCD" '^DeviceAllow=/dev/video[0-9]'
# Anti-pattern R-06 : aucun systemctl dans une unité livrée.
forbid_pattern "$TCCD" '/usr/bin/systemctl'
forbid_pattern "$TCCD_USER" '/usr/bin/systemctl'
forbid_pattern "$TCCD_ADMIN" '/usr/bin/systemctl'

require_line "$TCCD_USER" "PartOf=graphical-session.target"
require_line "$TCCD_USER" "After=graphical-session.target"
require_line "$TCCD_USER" "NoNewPrivileges=yes"
require_line "$TCCD_USER" "ProtectSystem=strict"
require_line "$TCCD_USER" "RestrictAddressFamilies=AF_UNIX"
require_line "$TCCD_USER" "MemoryDenyWriteExecute=yes"
require_line "$TCCD_USER" "LimitCORE=0"
require_line "$TCCD_USER" "UMask=0077"
require_line "$TCCD_USER" "ProtectKernelLogs=yes"
require_line "$TCCD_USER" "MemoryMax=128M"
require_line "$TCCD_USER" "TasksMax=32"
forbid_pattern "$TCCD_USER" '^PrivateTmp=yes'
forbid_pattern "$TCCD_USER" '^PrivateDevices=yes'
forbid_pattern "$TCCD_USER" '^DeviceAllow='
forbid_pattern "$TCCD_USER" '^DevicePolicy='

# tccd-admin PHASE 1 : configuration uniquement, aucun périphérique/capacité.
require_line "$TCCD_ADMIN" "Type=dbus"
require_line "$TCCD_ADMIN" "BusName=com.tuxedocomputers.tccd.Admin"
require_line "$TCCD_ADMIN" "ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin"
require_line "$TCCD_ADMIN" "Restart=no"
require_line "$TCCD_ADMIN" "RuntimeMaxSec=5min"
require_line "$TCCD_ADMIN" "NoNewPrivileges=yes"
require_line "$TCCD_ADMIN" "ProtectSystem=strict"
require_line "$TCCD_ADMIN" "ReadWritePaths=/var/lib/tuxedo-rust-center"
require_line "$TCCD_ADMIN" "ProtectHome=yes"
require_line "$TCCD_ADMIN" "PrivateTmp=yes"
require_line "$TCCD_ADMIN" "PrivateDevices=yes"
require_line "$TCCD_ADMIN" "DevicePolicy=closed"
require_line "$TCCD_ADMIN" "ProtectKernelTunables=yes"
require_line "$TCCD_ADMIN" "ProtectKernelModules=yes"
require_line "$TCCD_ADMIN" "ProtectKernelLogs=yes"
require_line "$TCCD_ADMIN" "ProtectControlGroups=yes"
require_line "$TCCD_ADMIN" "RestrictAddressFamilies=AF_UNIX"
require_line "$TCCD_ADMIN" "RestrictNamespaces=yes"
require_line "$TCCD_ADMIN" "RestrictRealtime=yes"
require_line "$TCCD_ADMIN" "RestrictSUIDSGID=yes"
require_line "$TCCD_ADMIN" "LockPersonality=yes"
require_line "$TCCD_ADMIN" "MemoryDenyWriteExecute=yes"
require_line "$TCCD_ADMIN" "SystemCallFilter=@system-service"
require_line "$TCCD_ADMIN" "LimitCORE=0"
require_line "$TCCD_ADMIN" "UMask=0077"
require_line "$TCCD_ADMIN" "MemoryMax=192M"
require_line "$TCCD_ADMIN" "TasksMax=16"
require_line "$TCCD_ADMIN" "StartLimitIntervalSec=60"
require_line "$TCCD_ADMIN" "StartLimitBurst=5"
require_line "$TCCD_ADMIN" "CapabilityBoundingSet="
require_line "$TCCD_ADMIN" "AmbientCapabilities="
require_line "$TCCD_ADMIN" "User=root"
forbid_pattern "$TCCD_ADMIN" '^\[Install\]'
forbid_pattern "$TCCD_ADMIN" '^WantedBy='
forbid_pattern "$TCCD_ADMIN" '^StateDirectory='
forbid_pattern "$TCCD_ADMIN" '^ConfigurationDirectory='
forbid_pattern "$TCCD_ADMIN" '^CapabilityBoundingSet=CAP_'
forbid_pattern "$TCCD_ADMIN" '^DeviceAllow='
forbid_pattern "$TCCD_ADMIN" '^PrivateDevices=no'
forbid_pattern "$TCCD_ADMIN" '^ProtectKernelTunables=no'
forbid_pattern "$TCCD_ADMIN" '^ProtectSystem=full'

# tccd-admin-hw PHASE 2 (AD-12 B2) : TDP/ODM/cTGP. Unité distincte de la
# phase 1 ; elle ne doit jamais être installée par `--with-admin` seul.
require_line "$TCCD_ADMIN_HW" "Type=dbus"
require_line "$TCCD_ADMIN_HW" "BusName=com.tuxedocomputers.tccd.Admin.Hw"
require_line "$TCCD_ADMIN_HW" "ExecStart=/usr/libexec/tuxedo-rust-center/tccd-admin-hw"
require_line "$TCCD_ADMIN_HW" "Restart=no"
require_line "$TCCD_ADMIN_HW" "RuntimeMaxSec=5min"
require_line "$TCCD_ADMIN_HW" "NoNewPrivileges=yes"
require_line "$TCCD_ADMIN_HW" "ProtectSystem=strict"
require_line "$TCCD_ADMIN_HW" "ProtectHome=yes"
require_line "$TCCD_ADMIN_HW" "PrivateTmp=yes"
require_line "$TCCD_ADMIN_HW" "PrivateDevices=no"
require_line "$TCCD_ADMIN_HW" "DevicePolicy=closed"
require_line "$TCCD_ADMIN_HW" "DeviceAllow=/dev/tuxedo_io rw"
require_line "$TCCD_ADMIN_HW" "ProtectKernelTunables=no"
require_line "$TCCD_ADMIN_HW" "ReadOnlyPaths=/sys/kernel /sys/module"
require_line "$TCCD_ADMIN_HW" "ReadOnlyPaths=/sys/devices/system/cpu"
require_line "$TCCD_ADMIN_HW" "ReadOnlyPaths=-/sys/class/leds -/sys/class/backlight"
require_line "$TCCD_ADMIN_HW" "ReadOnlyPaths=-/sys/class/power_supply -/sys/class/hwmon -/sys/class/thermal -/sys/class/drm"
require_line "$TCCD_ADMIN_HW" "ProtectKernelModules=yes"
require_line "$TCCD_ADMIN_HW" "ProtectKernelLogs=yes"
require_line "$TCCD_ADMIN_HW" "ProtectControlGroups=yes"
require_line "$TCCD_ADMIN_HW" "ProtectClock=yes"
require_line "$TCCD_ADMIN_HW" "ProtectHostname=yes"
require_line "$TCCD_ADMIN_HW" "KeyringMode=private"
require_line "$TCCD_ADMIN_HW" "RemoveIPC=yes"
require_line "$TCCD_ADMIN_HW" "RestrictAddressFamilies=AF_UNIX"
require_line "$TCCD_ADMIN_HW" "RestrictNamespaces=yes"
require_line "$TCCD_ADMIN_HW" "RestrictRealtime=yes"
require_line "$TCCD_ADMIN_HW" "RestrictSUIDSGID=yes"
require_line "$TCCD_ADMIN_HW" "LockPersonality=yes"
require_line "$TCCD_ADMIN_HW" "MemoryDenyWriteExecute=yes"
require_line "$TCCD_ADMIN_HW" "SystemCallArchitectures=native"
require_line "$TCCD_ADMIN_HW" "SystemCallFilter=@system-service"
require_line "$TCCD_ADMIN_HW" "SystemCallErrorNumber=EPERM"
require_line "$TCCD_ADMIN_HW" "SecureBits=noroot-locked"
require_line "$TCCD_ADMIN_HW" "LimitCORE=0"
require_line "$TCCD_ADMIN_HW" "MemoryMax=256M"
require_line "$TCCD_ADMIN_HW" "TasksMax=16"
require_line "$TCCD_ADMIN_HW" "StartLimitIntervalSec=60"
require_line "$TCCD_ADMIN_HW" "StartLimitBurst=20"
require_line "$TCCD_ADMIN_HW" "CapabilityBoundingSet="
require_line "$TCCD_ADMIN_HW" "AmbientCapabilities="
require_line "$TCCD_ADMIN_HW" "User=root"
require_line "$TCCD_ADMIN_HW" "Wants=modprobe@tuxedo_io.service"
require_line "$TCCD_ADMIN_HW" "After=dbus.service systemd-modules-load.service modprobe@tuxedo_io.service"
forbid_pattern "$TCCD_ADMIN_HW" '^\[Install\]'
forbid_pattern "$TCCD_ADMIN_HW" '^WantedBy='
forbid_pattern "$TCCD_ADMIN_HW" '^StateDirectory='
forbid_pattern "$TCCD_ADMIN_HW" '^ConfigurationDirectory='
require_line "$TCCD_ADMIN_HW" "ReadOnlyPaths=/etc/tuxedo-rust-center /var/lib/tuxedo-rust-center"
# D2/design §4.4 : Admin.Hw ne persiste rien. Aucun ReadWritePaths.
# ProcSubset=pid masque /proc/sys : un ReadOnlyPaths=/proc/sys échoue au
# démarrage (226/NAMESPACE). Interdire le retour de ce motif.
if active_lines "$TCCD_ADMIN_HW" | grep -E '^ReadOnlyPaths=.*/proc/sys( |$)' >/dev/null; then
    fail "Admin.Hw: ReadOnlyPaths=/proc/sys interdit (masqué par ProcSubset=pid, 226/NAMESPACE)"
else
    pass
fi
forbid_pattern "$TCCD_ADMIN_HW" '^ReadWritePaths='
forbid_pattern "$TCCD_ADMIN_HW" '^CapabilityBoundingSet=CAP_'
forbid_pattern "$TCCD_ADMIN_HW" '^PrivateDevices=yes'
forbid_pattern "$TCCD_ADMIN_HW" '^ProtectKernelTunables=yes'
forbid_pattern "$TCCD_ADMIN_HW" '^ProtectSystem=full'
# Un seul nœud autorisé : /dev/tuxedo_io rw. Tout autre DeviceAllow est une
# extension de privilège qui doit être relue par security.
hw_extra_devices=$(active_lines "$TCCD_ADMIN_HW" | grep '^DeviceAllow=' \
    | grep -vxF 'DeviceAllow=/dev/tuxedo_io rw' || true)
if [ -n "$hw_extra_devices" ]; then
    fail "Admin.Hw: DeviceAllow non prévu:
$hw_extra_devices"
else
    pass
fi
forbid_pattern "$TCCD_ADMIN_HW" '^DeviceAllow=char-video4linux'
forbid_pattern "$TCCD_ADMIN_HW" '^DeviceAllow=/dev/video'
forbid_pattern "$TCCD_ADMIN_HW" '^EnvironmentFile='
forbid_pattern "$TCCD_ADMIN_HW" 'TCC_SYSFS_ROOT'
forbid_pattern "$TCCD_ADMIN_HW" 'TCC_CONFIG_DIR'
forbid_pattern "$TCCD_ADMIN_HW" '^DynamicUser='
forbid_pattern "$TCCD_ADMIN_HW" '/usr/bin/systemctl'

# ---------------------------------------------------------------------------
section "Politique D-Bus et polkit"

DBUS_CONF="$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.conf"
POLKIT="$REPO_ROOT/packaging/polkit/com.tuxedocomputers.tccd.policy"

for required in \
    '<type>system</type>' \
    'deny own="com.tuxedocomputers.tccd"' \
    'deny own="com.tuxedocomputers.tccd.Admin"' \
    'deny own="com.tuxedocomputers.tccd.Admin.Hw"' \
    'deny send_destination="com.tuxedocomputers.tccd"' \
    'deny send_destination="com.tuxedocomputers.tccd.Admin"' \
    'deny send_destination="com.tuxedocomputers.tccd.Admin.Hw"' \
    'send_member="GetStateJSON"' \
    'send_member="GetFanDataJSON"' \
    'send_member="SetTempProfile"' \
    'send_member="ClearTempProfile"' \
    'send_interface="com.tuxedocomputers.tccd.Manager1"'; do
    if strip_xml_comments "$DBUS_CONF" | grep -F -- "$required" >/dev/null; then
        pass
    else
        fail "politique D-Bus: motif attendu absent « $required »"
    fi
done
forbid_xml_pattern "$DBUS_CONF" 'own="\*"'
forbid_xml_pattern "$DBUS_CONF" 'allow send_destination="\*"'
forbid_xml_pattern "$DBUS_CONF" 'allow own="\*"'
forbid_xml_pattern "$DBUS_CONF" 'Properties" send_member="Set"'

# Manager1 : aucun allow d'interface sans membre (REVIEW_HARDWARE H-10).
if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_interface="com\.tuxedocomputers\.tccd\.Manager1"' \
    | grep -v 'send_member=' >/dev/null; then
    fail "politique D-Bus: allow Manager1 global sans membre (H-10)"
else
    pass
fi

# Manager1 : lectures/bail autorisés ; les 8 setters sont en deny temporaire
# en attendant M1-01/M1-02 (bucket UID/DialogGate, lighting respectant
# keyboard_backlight_control_enabled). Ne pas rouvrir avant revue security.
manager_allowed=(GetContractVersion GetHardwareSnapshotJSON GetSensorDataCollectionStatus GetShutdownTimerJSON SetSensorDataCollectionStatus)
manager_denied=(
    SetChargingProfile SetChargingPriority SetChargeStartThreshold SetChargeEndThreshold
    SetChargeType SetKeyboardStatesJSON SetLightbarStatesJSON SetFnLockStatus
)
for member in "${manager_allowed[@]}"; do
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*Manager1[^>]*send_member=\"$member\"" >/dev/null; then
        pass
    else
        fail "politique D-Bus: Manager1.$member devrait être autorisé"
    fi
    if default_policy "$DBUS_CONF" | grep -E "<deny[^>]*Manager1[^>]*send_member=\"$member\"" >/dev/null; then
        fail "politique D-Bus: Manager1.$member est explicitement refusé"
    else
        pass
    fi
done
for member in "${manager_denied[@]}"; do
    if default_policy "$DBUS_CONF" | grep -E "<deny[^>]*Manager1[^>]*send_member=\"$member\"" >/dev/null; then
        pass
    else
        fail "politique D-Bus: Manager1.$member devrait être refusé (M1-01/M1-02)"
    fi
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*Manager1[^>]*send_member=\"$member\"" >/dev/null; then
        fail "politique D-Bus: Manager1.$member ne doit pas être autorisé (M1-01/M1-02)"
    else
        pass
    fi
done

# Le blocage temporaire doit rester documenté et auditable.
if grep -q 'M1-01/M1-02' "$DBUS_CONF"; then
    pass
else
    fail "politique D-Bus: marqueur M1-01/M1-02 absent du bloc Manager1"
fi

# Admin1 phase 1 : aucun allow d'interface, 3 membres autorisés, le reste
# refusé explicitement ; aucun Properties (Set absent par construction).
ADMIN_DBUS="$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.Admin.service"
require_line "$ADMIN_DBUS" "Name=com.tuxedocomputers.tccd.Admin"
require_line "$ADMIN_DBUS" "Exec=/usr/libexec/tuxedo-rust-center/tccd-admin"
require_line "$ADMIN_DBUS" "User=root"
require_line "$ADMIN_DBUS" "SystemdService=tccd-admin.service"

if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_interface="com\.tuxedocomputers\.tccd\.Admin1"' \
    | grep -v 'send_member=' >/dev/null; then
    fail "politique D-Bus: allow Admin1 global sans membre (AD-2)"
else
    pass
fi

admin_allowed=(Ping GetVersion GetContractVersion SaveProfilesJSON SaveSettingsJSON ReloadConfig)
admin_denied=(
    SetTdpValues SetOdmProfile SetNvidiaCtgpOffset SetPrimeMode
    TomteRun PackagesApply
)
for member in "${admin_allowed[@]}"; do
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*Admin1[^>]*send_member=\"$member\"" >/dev/null; then
        pass
    else
        fail "politique D-Bus: Admin1.$member devrait être autorisé (phase 1)"
    fi
    if default_policy "$DBUS_CONF" | grep -E "<deny[^>]*Admin1[^>]*send_member=\"$member\"" >/dev/null; then
        fail "politique D-Bus: Admin1.$member est refusé"
    else
        pass
    fi
done
for member in "${admin_denied[@]}"; do
    if default_policy "$DBUS_CONF" | grep -E "<deny[^>]*Admin1[^>]*send_member=\"$member\"" >/dev/null; then
        pass
    else
        fail "politique D-Bus: Admin1.$member devrait être refusé (phase 1)"
    fi
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*Admin1[^>]*send_member=\"$member\"" >/dev/null; then
        fail "politique D-Bus: Admin1.$member ne doit pas être autorisé"
    else
        pass
    fi
done

# Admin : AUCUN allow Properties (A1-11). La version passe par
# Admin1.GetContractVersion ; une propriété future ne doit pas être exposée
# automatiquement. Properties.Set reste absent globalement.
if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_destination="com\.tuxedocomputers\.tccd\.Admin"[^>]*Properties' >/dev/null; then
    fail "politique D-Bus: Properties autorisé sur Admin (A1-11)"
else
    pass
fi

# Admin.Hw phase 2 (AD-12 B2) : activation D-Bus et policy membre par membre.
ADMIN_HW_DBUS="$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.Admin.Hw.service"
require_line "$ADMIN_HW_DBUS" "Name=com.tuxedocomputers.tccd.Admin.Hw"
require_line "$ADMIN_HW_DBUS" "Exec=/usr/libexec/tuxedo-rust-center/tccd-admin-hw"
require_line "$ADMIN_HW_DBUS" "User=root"
require_line "$ADMIN_HW_DBUS" "SystemdService=tccd-admin-hw.service"

if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_interface="com\.tuxedocomputers\.tccd\.AdminHw1"' \
    | grep -v 'send_member=' >/dev/null; then
    fail "politique D-Bus: allow AdminHw1 global sans membre (AD-2)"
else
    pass
fi

# Refus explicite de l'interface entière, avant les allow par membre : une
# méthode future non listée reste injoignable.
if default_policy "$DBUS_CONF" \
    | grep -E '<deny[^>]*send_destination="com\.tuxedocomputers\.tccd\.Admin\.Hw"[^>]*send_interface="com\.tuxedocomputers\.tccd\.AdminHw1"' >/dev/null; then
    pass
else
    fail "politique D-Bus: deny d'interface AdminHw1 absent (reste non listé)"
fi

admin_hw_allowed=(
    Ping GetVersion GetContractVersion GetTdpValues
    SetTdpValues SetOdmProfile SetOdmProfileOverride SetNvidiaCtgpOffset
)
for member in "${admin_hw_allowed[@]}"; do
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*AdminHw1[^>]*send_member=\"$member\"" >/dev/null; then
        pass
    else
        fail "politique D-Bus: AdminHw1.$member devrait être autorisé"
    fi
done
# Phase 1 et phase 3 ne doivent jamais être joignables sur .Admin.Hw.
for member in SetPrimeMode TomteRun PackagesApply; do
    if default_policy "$DBUS_CONF" | grep -E "<allow[^>]*AdminHw1[^>]*send_member=\"$member\"" >/dev/null; then
        fail "politique D-Bus: AdminHw1.$member ne doit pas être autorisé"
    else
        pass
    fi
done

# Properties interdit sur .Admin.Hw (A1-11), comme sur .Admin et tccd.
if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_destination="com\.tuxedocomputers\.tccd\.Admin\.Hw"[^>]*Properties' >/dev/null; then
    fail "politique D-Bus: Properties autorisé sur Admin.Hw (A1-11)"
else
    pass
fi
if default_policy "$DBUS_CONF" \
    | grep -E '<deny[^>]*send_destination="com\.tuxedocomputers\.tccd\.Admin\.Hw"[^>]*send_interface="org\.freedesktop\.DBus\.Properties"' >/dev/null; then
    pass
else
    fail "politique D-Bus: deny Properties absent sur Admin.Hw (A1-11)"
fi

# Root possède le nom et peut parler aux deux services admin.
if root_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*own="com\.tuxedocomputers\.tccd\.Admin\.Hw"' >/dev/null; then
    pass
else
    fail "politique D-Bus: root ne peut pas posséder Admin.Hw"
fi

# P-12 : aucun allow Properties (Get/GetAll/Set) sur tccd ni Admin.
# La version passe par Manager1.GetContractVersion / Admin1.GetContractVersion.
if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_interface="org\.freedesktop\.DBus\.Properties"' >/dev/null; then
    fail "politique D-Bus: Properties encore autorisé (Get/GetAll/Set) — P-12"
else
    pass
fi

# Control1.ReloadConfig : root uniquement.
if root_policy "$DBUS_CONF" | grep -E '<allow[^>]*Control1[^>]*send_member="ReloadConfig"' >/dev/null; then
    pass
else
    fail "politique D-Bus: Control1.ReloadConfig absent du bloc root"
fi
if default_policy "$DBUS_CONF" | grep -E '<deny[^>]*Control1[^>]*send_member="ReloadConfig"' >/dev/null; then
    pass
else
    fail "politique D-Bus: Control1.ReloadConfig non refusé par défaut"
fi
if default_policy "$DBUS_CONF" | grep -E '<allow[^>]*Control1' >/dev/null; then
    fail "politique D-Bus: Control1 autorisé aux non-root"
else
    pass
fi

# Fail-closed : tout allow send_destination doit nommer une interface ou un
# membre, et les deny send_destination doivent précéder les allow.
if default_policy "$DBUS_CONF" \
    | grep -E '<allow[^>]*send_destination=' \
    | grep -vE 'send_interface=|send_member=' >/dev/null; then
    fail "politique D-Bus: allow send_destination sans interface ni membre"
else
    pass
fi
first_deny=$(default_policy "$DBUS_CONF" | grep -n 'deny send_destination=' | head -1 | cut -d: -f1)
first_allow=$(default_policy "$DBUS_CONF" | grep -n '<allow send_destination=' | head -1 | cut -d: -f1)
if [ -n "$first_deny" ] && [ -n "$first_allow" ] && [ "$first_deny" -lt "$first_allow" ]; then
    pass
else
    fail "politique D-Bus: le deny send_destination global doit précéder les allows"
fi

# Règle bloquante docs/security/CHECKLIST.md §3.2 : pas d'action pkexec.
forbid_xml_pattern "$POLKIT" 'policykit\.exec\.path'
forbid_xml_pattern "$POLKIT" 'policykit\.exec\.argv1'
forbid_xml_pattern "$POLKIT" 'policykit\.exec\.allow_gui'
for action in \
    profiles settings power-limits charging display prime-select tomte-run packages; do
    if strip_xml_comments "$POLKIT" | grep -F "id=\"com.tuxedocomputers.tccd1.$action\"" >/dev/null; then
        pass
    else
        fail "polkit: action com.tuxedocomputers.tccd1.$action absente"
    fi
done
if strip_xml_comments "$POLKIT" | grep -F 'id="com.tuxedocomputers.tccd.tomte"' >/dev/null; then
    fail "polkit: ancienne action tomte encore présente"
else
    pass
fi
if strip_xml_comments "$POLKIT" | grep -F 'id="com.tuxedocomputers.tccd1.assign-profile"' >/dev/null; then
    fail "polkit: assign-profile doit être supprimée (ADMIN_DESIGN §3.2)"
else
    pass
fi

# allow_any / allow_inactive doivent être `no` partout (R-09).
bad_defaults=$(strip_xml_comments "$POLKIT" \
    | grep -E '<allow_(any|inactive)>' \
    | grep -vE '<allow_(any|inactive)>no</allow_(any|inactive)>' || true)
if [ -n "$bad_defaults" ]; then
    fail "polkit: allow_any/allow_inactive doivent être 'no' : $bad_defaults"
else
    pass
fi

# Aucun cache auth_admin_keep : toutes les actions sont sans _keep
# (ADMIN_DESIGN §3.2, R-09).
if strip_xml_comments "$POLKIT" | grep -F 'auth_admin_keep' >/dev/null; then
    fail "polkit: auth_admin_keep ne doit plus être utilisé (ADMIN_DESIGN §3.2)"
else
    pass
fi

# profiles : auth_admin sans _keep (écriture persistante des profils).
profiles_block=$(strip_xml_comments "$POLKIT" \
    | awk '/id="com\.tuxedocomputers\.tccd1\.profiles"/{f=1} f{print} f&&/<\/action>/{exit}')
if printf '%s\n' "$profiles_block" | grep -F '<allow_active>auth_admin</allow_active>' >/dev/null; then
    pass
else
    fail "polkit: profiles doit utiliser auth_admin (sans _keep)"
fi

# power-limits : action utilisée par tccd-admin-hw (TDP/ODM/cTGP), sans
# cache (auth_admin, pas de _keep).
power_limits_block=$(strip_xml_comments "$POLKIT" \
    | awk '/id="com\.tuxedocomputers\.tccd1\.power-limits"/{f=1} f{print} f&&/<\/action>/{exit}')
if printf '%s\n' "$power_limits_block" | grep -F '<allow_active>auth_admin</allow_active>' >/dev/null; then
    pass
else
    fail "polkit: power-limits doit utiliser auth_admin (sans _keep)"
fi

# UVC de session : aucun polkit dédié. User1.SetUvcControl vit sur le bus de
# session ; l'accès est borné par l'ACL du nœud caméra et l'allowlist.
if strip_xml_comments "$POLKIT" | grep -qiE 'uvc|webcam'; then
    fail "polkit: action UVC/webcam présente alors que l'écriture est de session"
else
    pass
fi

# LUKS est hors périmètre : aucune action, aucune méthode, aucune référence.
if strip_xml_comments "$POLKIT" | grep -E 'id="[^"]*luks' >/dev/null; then
    fail "polkit: action LUKS présente alors que LUKS est hors périmètre"
else
    pass
fi
if strip_xml_comments "$DBUS_CONF" | grep -E 'Luks' >/dev/null; then
    fail "politique D-Bus: méthode LUKS présente alors que LUKS est hors périmètre"
else
    pass
fi
# Absence de toute référence de procédure LUKS dans le packaging et les docs
# packaging/scripts (validate.sh exclu pour éviter l'auto-détection du motif).
if grep -rInE --exclude=validate.sh 'luks-change-key|LuksChangePassphrase|cryptsetup' \
    "$REPO_ROOT/packaging" "$REPO_ROOT/distro" "$REPO_ROOT/docs/packaging" \
    "$REPO_ROOT/scripts/release" >/dev/null; then
    fail "référence LUKS (action/procédure) trouvée dans le packaging"
else
    pass
fi

# Validation réelle du fichier par dbus-daemon (parse + démarrage d'un bus de
# test isolé). Un échec de parse est bloquant ; l'absence de dbus-daemon est
# signalée.
if need_tool dbus-daemon; then
    # Le fichier livré est un fragment de politique (pas un bus complet) :
    # on le charge via un bus de test isolé avec un <listen> temporaire.
    dbus_tmp=$(mktemp -d)
    {
        printf '%s\n' '<busconfig>'
        printf '%s\n' '  <type>system</type>'
        printf '  <listen>unix:tmpdir=%s</listen>\n' "$dbus_tmp"
        printf '  <pidfile>%s/pid</pidfile>\n' "$dbus_tmp"
        printf '  <include>%s</include>\n' "$DBUS_CONF"
        printf '%s\n' '</busconfig>'
    } > "$dbus_tmp/bus.conf"
    dbus_out=$(timeout 3 dbus-daemon --config-file="$dbus_tmp/bus.conf" --nofork --print-address 2>&1)
    dbus_rc=$?
    rm -rf "$dbus_tmp"
    if [ "$dbus_rc" -eq 124 ] || [ "$dbus_rc" -eq 0 ]; then
        pass
    else
        fail "dbus-daemon refuse la politique : $dbus_out"
    fi
fi

# ---------------------------------------------------------------------------
# Valide un fichier tmpfiles avec systemd-tmpfiles quand c'est possible.
# `--dry-run` n'existe qu'à partir de systemd 256 (absent d'Ubuntu 24.04) ;
# sans lui, `--create --root` change les propriétaires et exige root.
check_tmpfiles() {
    tmpfiles_file=$1
    tmpfiles_label=$2
    if ! command -v systemd-tmpfiles >/dev/null 2>&1; then
        skip "$tmpfiles_label : systemd-tmpfiles absent"
        return 0
    fi
    tmpfiles_help=$(systemd-tmpfiles --help 2>&1 || true)
    tmpfiles_root=$(mktemp -d)
    case "$tmpfiles_help" in
        *--dry-run*)
            if systemd-tmpfiles --create --dry-run --root="$tmpfiles_root" \
                "$tmpfiles_file" >/dev/null 2>&1; then
                pass
            else
                fail "$tmpfiles_label"
            fi
            ;;
        *)
            if [ "$(id -u)" -eq 0 ]; then
                if systemd-tmpfiles --create --root="$tmpfiles_root" \
                    "$tmpfiles_file" >/dev/null 2>&1; then
                    pass
                else
                    fail "$tmpfiles_label"
                fi
            else
                skip "$tmpfiles_label : --dry-run absent et non-root (format vérifié)"
            fi
            ;;
    esac
    rm -rf "$tmpfiles_root"
}

section "udev et tmpfiles"

UDEV_RULE="$REPO_ROOT/packaging/udev/99-tuxedo-rust-center-webcam.rules"
forbid_pattern "$UDEV_RULE" 'RUN\+='
forbid_pattern "$UDEV_RULE" 'IMPORT\{program\}'
forbid_pattern "$UDEV_RULE" 'PROGRAM='
forbid_pattern "$UDEV_RULE" 'python|node|/bin/sh|/bin/bash'
# shellcheck disable=SC2016  # motif literal, pas une expansion
forbid_pattern "$UDEV_RULE" '\$env\{'

TMPFILES="$REPO_ROOT/packaging/tmpfiles/tuxedo-rust-center.conf"
for line in \
    '^d[[:space:]]+/run/tuxedo-rust-center[[:space:]]+0755[[:space:]]+root[[:space:]]+root[[:space:]]+-$' \
    '^d[[:space:]]+/var/lib/tuxedo-rust-center[[:space:]]+0755[[:space:]]+root[[:space:]]+root[[:space:]]+-$' \
    '^d[[:space:]]+/etc/tuxedo-rust-center[[:space:]]+0755[[:space:]]+root[[:space:]]+root[[:space:]]+-$'; do
    if active_lines "$TMPFILES" | grep -E -- "$line" >/dev/null; then
        pass
    else
        fail "tmpfiles: ligne absente « $line »"
    fi
done

check_tmpfiles "$TMPFILES" "systemd-tmpfiles refuse le fichier tmpfiles"

# ---------------------------------------------------------------------------
section "Bureau et AppStream"

DESKTOP="$REPO_ROOT/packaging/desktop/tuxedo-rust-center.desktop"
METAINFO="$REPO_ROOT/packaging/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml"

for required in \
    'Type=Application' \
    'Exec=tuxedo-rust-center' \
    'Icon=tuxedo-rust-center' \
    'Terminal=false' \
    'Categories=Settings;HardwareSettings;'; do
    if grep -qxF -- "$required" "$DESKTOP"; then
        pass
    else
        fail "desktop: ligne absente « $required »"
    fi
done
forbid_pattern "$DESKTOP" '/opt'
forbid_pattern "$DESKTOP" 'sh -c'

for required in \
    '<id>com.tuxedocomputers.TuxedoRustCenter</id>' \
    '<metadata_license>CC0-1.0</metadata_license>' \
    '<project_license>GPL-3.0-or-later</project_license>' \
    '<launchable type="desktop-id">tuxedo-rust-center.desktop</launchable>'; do
    if grep -qF -- "$required" "$METAINFO"; then
        pass
    else
        fail "metainfo: motif absent « $required »"
    fi
done

# ---------------------------------------------------------------------------
section "Dépendances QML (GUI)"

qml_debian_package() {
    case "$1" in
        QtCore) echo "qml6-module-qtcore" ;;
        Qt.labs.platform) echo "qml6-module-qt-labs-platform" ;;
        Qt.labs.settings) echo "qml6-module-qt-labs-settings" ;;
        QtQml.Models) echo "qml6-module-qtqml-models" ;;
        QtQuick) echo "qml6-module-qtquick" ;;
        QtQuick.Controls) echo "qml6-module-qtquick-controls" ;;
        QtQuick.Dialogs) echo "qml6-module-qtquick-dialogs" ;;
        Qt5Compat.GraphicalEffects) echo "qml6-module-qt5compat-graphicaleffects" ;;
        QtQuick.Layouts) echo "qml6-module-qtquick-layouts" ;;
        QtQuick.Shapes) echo "qml6-module-qtquick-shapes" ;;
        QtQuick.Window) echo "qml6-module-qtquick-window" ;;
        Tcc) echo "" ;;  # module compilé dans le binaire
        *) echo "UNKNOWN:$1" ;;
    esac
}

qml_modules=$(grep -rhE '^[[:space:]]*import[[:space:]]+[A-Za-z]' \
    "$REPO_ROOT/crates/tcc-gui/qml" | awk '{print $2}' | LC_ALL=C sort -u)
for module in $qml_modules; do
    package=$(qml_debian_package "$module")
    case "$package" in
        UNKNOWN:*)
            fail "import QML non mappé dans validate.sh: $module"
            ;;
        "")
            pass
            ;;
        *)
            if grep -q -- "$package" "$REPO_ROOT/distro/deb/control.in"; then
                pass
            else
                fail "dépendance Debian manquante pour $module: $package"
            fi
            ;;
    esac
done

# Qt Widgets : requis par la glue QApplication du tray (Qt.labs.platform).
if grep -q 'libqt6widgets6' "$REPO_ROOT/distro/deb/control.in"; then
    pass
else
    fail "Debian: libqt6widgets6 manquant (QApplication/tray)"
fi
if grep -qE '^Requires:[[:space:]]+qt6-qtbase-gui' \
    "$REPO_ROOT/distro/rpm/tuxedo-rust-center.spec"; then
    pass
else
    fail "RPM: qt6-qtbase-gui manquant (Qt6Widgets/tray)"
fi
if grep -q "'qt6-base'" "$REPO_ROOT/distro/arch/PKGBUILD"; then
    pass
else
    fail "Arch: qt6-base manquant (Qt6Widgets/tray)"
fi

if grep -qE '^Requires:[[:space:]]+qt6-qtdeclarative' \
    "$REPO_ROOT/distro/rpm/tuxedo-rust-center.spec"; then
    pass
else
    fail "RPM: qt6-qtdeclarative manquant (Qt.labs.platform/QtQuick.Dialogs)"
fi
if grep -q "'qt6-declarative'" "$REPO_ROOT/distro/arch/PKGBUILD"; then
    pass
else
    fail "Arch: qt6-declarative manquant (Qt.labs.platform/QtQuick.Dialogs)"
fi

# ---------------------------------------------------------------------------
section "Installation conditionnelle Admin.Hw (phase 2)"

INSTALL_TREE="$REPO_ROOT/scripts/release/install-tree.sh"

for token in \
    'WITH_ADMIN_HW=${TCC_WITH_ADMIN_HW:-0}' \
    '--with-admin-hw) WITH_ADMIN_HW=1' \
    'usr/libexec/tuxedo-rust-center/tccd-admin-hw' \
    'usr/lib/systemd/system/tccd-admin-hw.service' \
    'usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service'; do
    if grep -qF -- "$token" "$INSTALL_TREE"; then
        pass
    else
        fail "install-tree.sh: motif Admin.Hw absent « $token »"
    fi
done

# Indépendance des deux conditions : le bloc WITH_ADMIN ne doit jamais
# installer Admin.Hw, et le bloc WITH_ADMIN_HW ne doit jamais installer la
# phase 1. (`--with-admin` seul ne doit pas relâcher PrivateDevices.)
admin_block=$(awk '/^if \[ "\$WITH_ADMIN" = "1" \]; then$/{f=1} f{print} f&&/^fi$/{f=0}' "$INSTALL_TREE")
if printf '%s\n' "$admin_block" | grep -q 'ADMIN_HW\|tccd-admin-hw'; then
    fail "install-tree.sh: Admin.Hw installé par TCC_WITH_ADMIN (doit être indépendant)"
else
    pass
fi
admin_hw_block=$(awk '/^if \[ "\$WITH_ADMIN_HW" = "1" \]; then$/{f=1} f{print} f&&/^fi$/{f=0}' "$INSTALL_TREE")
if printf '%s\n' "$admin_hw_block" | grep -qE '\$ADMIN_BIN([^_]|$)|tccd-admin\.service'; then
    fail "install-tree.sh: phase 1 installée par TCC_WITH_ADMIN_HW"
else
    pass
fi

SPEC="$REPO_ROOT/distro/rpm/tuxedo-rust-center.spec"
PKGBUILD="$REPO_ROOT/distro/arch/PKGBUILD"
for pair in \
    "$SPEC:%bcond_with admin_hw" \
    "$SPEC:TCC_WITH_ADMIN_HW=1" \
    "$SPEC:%{_libexecdir}/tuxedo-rust-center/tccd-admin-hw" \
    "$SPEC:%{_unitdir}/tccd-admin-hw.service" \
    "$SPEC:%{_datadir}/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service" \
    "$REPO_ROOT/scripts/release/make-rpm.sh:--with admin_hw" \
    "$PKGBUILD:_with_admin_hw=0" \
    "$PKGBUILD:TCC_WITH_ADMIN_HW=\"\$_with_admin_hw\"" \
    "$REPO_ROOT/scripts/release/make-arch.sh:_with_admin_hw=1" \
    "$REPO_ROOT/scripts/release/build.sh:--with-admin-hw" \
    "$REPO_ROOT/distro/deb/prerm:tccd-admin-hw.service" \
    "$REPO_ROOT/distro/arch/tuxedo-rust-center.install:tccd-admin-hw.service"; do
    file=${pair%%:*}
    token=${pair#*:}
    if grep -qF -- "$token" "$file"; then
        pass
    else
        fail "$(basename -- "$file"): motif Admin.Hw absent « $token »"
    fi
done

# Score de durcissement cible (design §4.4 : ≤ 3.0 ; phase 1 = 1.6).
if need_tool systemd-analyze; then
    hw_security_line=$(systemd-analyze security --offline=true --no-pager \
        "$TCCD_ADMIN_HW" 2>/dev/null \
        | grep 'Overall exposure level' | tail -1 || true)
    printf '  %s\n' "${hw_security_line:-systemd-analyze security: pas de sortie}"
    hw_score=$(printf '%s\n' "$hw_security_line" | sed -n 's/.*: \([0-9][0-9.]*\).*/\1/p')
    if [ -z "$hw_score" ]; then
        skip "score systemd-analyze security Admin.Hw non mesuré"
    elif awk -v s="$hw_score" 'BEGIN { exit !(s > 3.0) }'; then
        fail "Admin.Hw: score systemd-analyze security > 3.0 ($hw_score)"
    else
        pass
    fi
fi

# ---------------------------------------------------------------------------
section "Absence de /opt, de setuid et de runtime Node/Python (dépôt)"

if grep -rInE '(ExecStart|ExecStop|Exec)=/opt/' \
    "$REPO_ROOT/packaging" "$REPO_ROOT/distro" >/dev/null 2>&1; then
    fail "un fichier d'intégration référence un exécutable dans /opt"
else
    pass
fi

if grep -rInE '(RUN\+=|Exec(Start|Stop)?=).*(python|python3|node|/bin/sh|/bin/bash)' \
    "$REPO_ROOT/packaging" "$REPO_ROOT/distro" >/dev/null 2>&1; then
    fail "un fichier d'intégration exécute un interpréteur (python/node/shell)"
else
    pass
fi

# ---------------------------------------------------------------------------
section "Cohérence des versions"

cargo_version=$(tcc_version "$REPO_ROOT")
pkgbuild_version=$(sed -n 's/^pkgver=//p' "$REPO_ROOT/distro/arch/PKGBUILD" | head -1)
spec_version=$(sed -n 's/^Version:[[:space:]]*//p' "$REPO_ROOT/distro/rpm/tuxedo-rust-center.spec" | head -1)
changelog_version=$(sed -n '1s/^[^(]*(\([^)-]*\).*/\1/p' "$REPO_ROOT/packaging/changelog")

[ -n "$cargo_version" ] || fail "version Cargo.toml introuvable"
for pair in \
    "PKGBUILD:$pkgbuild_version" \
    "spec:$spec_version" \
    "changelog:$changelog_version"; do
    name=${pair%%:*}
    value=${pair#*:}
    if [ "$value" = "$cargo_version" ]; then
        pass
    else
        fail "version incohérente ($name=$value, Cargo.toml=$cargo_version)"
    fi
done

# ---------------------------------------------------------------------------
if [ "$MODE" = "tree" ]; then
    section "Arborescence installée: $TREE"
    [ -d "$TREE" ] || tcc_die "arborescence introuvable: $TREE"

    require_path() {
        local path=$1 mode=$2
        if [ ! -e "$TREE/$path" ]; then
            fail "chemin installé absent: $path"
            return
        fi
        local actual
        actual=$(stat -c '%a' "$TREE/$path")
        if [ "$actual" = "$mode" ]; then
            pass
        else
            fail "$path: mode $actual, attendu $mode"
        fi
    }

    require_path usr/bin/tuxedo-rust-center 755
    require_path usr/bin/tcc-cli 755
    require_path usr/libexec/tuxedo-rust-center/tccd 755
    require_path usr/libexec/tuxedo-rust-center/tccd-user 755
    require_path usr/lib/systemd/system/tccd.service 644
    require_path usr/lib/systemd/user/tccd-user.service 644
    require_path usr/share/dbus-1/system.d/com.tuxedocomputers.tccd.conf 644
    require_path usr/share/polkit-1/actions/com.tuxedocomputers.tccd.policy 644
    require_path usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules 644
    require_path usr/lib/tmpfiles.d/tuxedo-rust-center.conf 644
    require_path usr/share/applications/tuxedo-rust-center.desktop 644
    require_path usr/share/icons/hicolor/scalable/apps/tuxedo-rust-center.svg 644
    for size in 256 128 64 48 32; do
        require_path "usr/share/icons/hicolor/${size}x${size}/apps/tuxedo-rust-center.png" 644
    done
    require_path usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml 644
    require_path usr/share/licenses/tuxedo-rust-center/LICENSE 644
    require_path usr/share/doc/tuxedo-rust-center/copyright 644
    require_path usr/share/doc/tuxedo-rust-center/changelog.gz 644
    require_path etc/tuxedo-rust-center/README 644
    require_path var/lib/tuxedo-rust-center 755
    require_path usr/share/tuxedo-rust-center/qml/Main.qml 644
    require_path usr/share/tuxedo-rust-center/assets/fonts/TitilliumWeb-Regular.ttf 644
    require_path usr/share/tuxedo-rust-center/assets/icons/icon_dashboard.svg 644

    # tccd-sleep : jamais installé (R-06).
    if [ -e "$TREE/usr/lib/systemd/system/tccd-sleep.service" ]; then
        fail "tccd-sleep.service est installé alors qu'il est retiré (R-06)"
    else
        pass
    fi

    # Le service et l'activation admin ne sont jamais installés sans binaire.
    if [ -e "$TREE/usr/libexec/tuxedo-rust-center/tccd-admin" ]; then
        require_path usr/lib/systemd/system/tccd-admin.service 644
        require_path usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service 644
    else
        if [ -e "$TREE/usr/lib/systemd/system/tccd-admin.service" ] \
            || [ -e "$TREE/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service" ]; then
            fail "tccd-admin absent mais unité ou activation D-Bus installée"
        else
            pass
        fi
    fi

    # Admin.Hw phase 2 : le binaire, l'unité et l'activation vont ensemble.
    # Jamais de demi-installation (surtout pas d'activation sans binaire).
    if [ -e "$TREE/usr/libexec/tuxedo-rust-center/tccd-admin-hw" ]; then
        require_path usr/libexec/tuxedo-rust-center/tccd-admin-hw 755
        require_path usr/lib/systemd/system/tccd-admin-hw.service 644
        require_path usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service 644
    else
        if [ -e "$TREE/usr/lib/systemd/system/tccd-admin-hw.service" ] \
            || [ -e "$TREE/usr/share/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service" ]; then
            fail "tccd-admin-hw absent mais unité ou activation D-Bus installée"
        else
            pass
        fi
    fi

    # Activation utilisateur : lien relatif valide.
    wants="$TREE/usr/lib/systemd/user/graphical-session.target.wants/tccd-user.service"
    if [ -L "$wants" ] && [ -f "$TREE/usr/lib/systemd/user/tccd-user.service" ]; then
        target=$(readlink "$wants")
        if [ "$target" = "../tccd-user.service" ]; then
            pass
        else
            fail "lien .wants inattendu: $target"
        fi
    else
        fail "lien graphical-session.target.wants/tccd-user.service absent"
    fi

    # Lien de licence documentaire.
    doc_license="$TREE/usr/share/doc/tuxedo-rust-center/LICENSE"
    if [ -L "$doc_license" ] && [ -e "$doc_license" ]; then
        pass
    else
        fail "lien /usr/share/doc/tuxedo-rust-center/LICENSE cassé"
    fi

    # Aucun fichier dans /opt.
    if [ -e "$TREE/opt" ]; then
        fail "arborescence /opt présente dans le paquet"
    else
        pass
    fi

    # Aucun setuid/setgid.
    privileged=$(find "$TREE" -perm /6000 -print 2>/dev/null)
    if [ -n "$privileged" ]; then
        fail "fichiers setuid/setgid:
$privileged"
    else
        pass
    fi

    # Aucun world-writable.
    world_writable=$(find "$TREE" -perm -0002 ! -type l -print 2>/dev/null)
    if [ -n "$world_writable" ]; then
        fail "fichiers world-writable:
$world_writable"
    else
        pass
    fi

    # Aucune capability de fichier.
    if command -v getcap >/dev/null 2>&1; then
        caps=$(getcap -r "$TREE" 2>/dev/null || true)
        if [ -n "$caps" ]; then
            fail "capabilities de fichier:
$caps"
        else
            pass
        fi
    else
        skip "getcap absent"
    fi

    # Aucun script interprété installé (Node/Python/shell).
    interpreted=""
    while IFS= read -r -d '' candidate; do
        first_line=$(head -c 80 "$candidate" 2>/dev/null | tr -d '\000' || true)
        case "$first_line" in
            '#!'*python*|'#!'*node*|'#!'*/bin/sh*|'#!'*/bin/bash*|'#!'*env\ *)
                interpreted="$interpreted
$candidate"
                ;;
        esac
    done < <(find "$TREE" -type f -perm -u+x -print0 2>/dev/null)
    if [ -n "$interpreted" ]; then
        fail "scripts interprétés installés:$interpreted"
    else
        pass
    fi

    # Revalidation des fichiers installés.
    if need_tool desktop-file-validate; then
        if desktop-file-validate "$TREE/usr/share/applications/tuxedo-rust-center.desktop" >/dev/null 2>&1; then
            pass
        else
            fail "desktop-file-validate sur l'arbre installé"
        fi
    fi
    if need_tool appstreamcli; then
        if appstreamcli validate --no-net \
            "$TREE/usr/share/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml" >/dev/null 2>&1; then
            pass
        else
            fail "appstreamcli validate sur l'arbre installé"
        fi
    fi
    if need_tool udevadm; then
        if udevadm verify "$TREE/usr/lib/udev/rules.d/99-tuxedo-rust-center-webcam.rules" >/dev/null 2>&1; then
            pass
        else
            fail "udevadm verify sur l'arbre installé"
        fi
    fi
    check_tmpfiles "$TREE/usr/lib/tmpfiles.d/tuxedo-rust-center.conf" \
        "systemd-tmpfiles refuse le fichier installé"

    # Lecture seule pour le daemon : aucun exécutable dans /etc.
    etc_exec=$(find "$TREE/etc" -type f -perm /111 -print 2>/dev/null)
    if [ -n "$etc_exec" ]; then
        fail "fichiers exécutables dans /etc:
$etc_exec"
    else
        pass
    fi

    # Score de durcissement documenté (informatif, avertissement au-delà de 4).
    if command -v systemd-analyze >/dev/null 2>&1; then
        security_line=$(systemd-analyze security --offline=true --no-pager \
            "$TREE/usr/lib/systemd/system/tccd.service" 2>/dev/null \
            | grep 'Overall exposure level' | tail -1 || true)
        printf '  %s\n' "${security_line:-systemd-analyze security: pas de sortie}"
        score=$(printf '%s\n' "$security_line" | sed -n 's/.*: \([0-9][0-9.]*\).*/\1/p')
        if [ -n "$score" ] && awk -v s="$score" 'BEGIN { exit !(s > 4.0) }'; then
            tcc_warn "score systemd-analyze security > 4.0 : $score"
        fi
    fi
fi

# ---------------------------------------------------------------------------
printf '\n== Bilan\n'
printf '  vérifications réussies : %d\n' "$CHECKS"
printf '  échecs                 : %d\n' "$FAILURES"
if [ ${#SKIPPED[@]} -gt 0 ]; then
    printf '  vérifications ignorées :\n'
    for item in "${SKIPPED[@]}"; do
        printf '    - %s\n' "$item"
    done
fi

if [ "$FAILURES" -gt 0 ]; then
    exit 1
fi
exit 0
