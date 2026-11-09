#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Vérifications de validation root de TuxedoRustCenter.
# Lecture seule : n'installe rien, ne redémarre rien, ne modifie aucun fichier.
#
# Usage : sudo ./scripts/validation/root-check.sh
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : ce script doit être lancé en root (sudo).\n' >&2
    exit 77
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

fail=0
pass=0
skip=0
ok() { printf 'PASS  %s\n' "$1"; pass=$((pass + 1)); }
ko() { printf 'FAIL  %s\n' "$1"; fail=$((fail + 1)); }
sk() { printf 'SKIP  %s\n' "$1"; skip=$((skip + 1)); }
section() { printf '\n== %s ==\n' "$1"; }

section "Identité"
printf 'INFO  uid=%s\n' "$(id -u)"

section "Policy D-Bus (bus système réel)"
if [ -x scripts/release/dbus-policy-test.sh ]; then
    if scripts/release/dbus-policy-test.sh; then
        ok "dbus-policy-test.sh"
    else
        ko "dbus-policy-test.sh"
    fi
else
    ko "scripts/release/dbus-policy-test.sh absent"
fi

section "Scores offline des unités du dépôt"
for unit in tccd tccd-user tccd-admin; do
    if systemd-analyze security --offline=true "packaging/systemd/$unit.service" >/tmp/sec-offline-"$unit".txt 2>&1; then
        score=$(grep 'Overall exposure level' /tmp/sec-offline-"$unit".txt || true)
        printf 'INFO  dépôt %s : %s\n' "$unit" "$score"
        ok "score offline $unit"
    else
        ko "score offline $unit"
    fi
done

section "Services installés"
old_tcc=0
if command -v systemctl >/dev/null 2>&1; then
    unit_path=$(systemctl show tccd.service -p FragmentPath --value 2>/dev/null || true)
    printf 'INFO  tccd.service -> %s\n' "${unit_path:-absent}"
    if printf '%s' "$unit_path" | grep -q '/opt/tuxedo-control-center'; then
        old_tcc=1
        sk "tccd.service est l'ancien TCC (/opt) : runtime de notre daemon non testable"
    fi
fi

for unit in tccd.service tccd-user.service tccd-admin.service; do
    if systemctl list-unit-files "$unit" >/dev/null 2>&1; then
        state=$(systemctl is-active "$unit" 2>/dev/null || true)
        enabled=$(systemctl is-enabled "$unit" 2>/dev/null || true)
        printf 'INFO  %s active=%s enabled=%s\n' "$unit" "$state" "$enabled"
        if [ "$unit" = "tccd.service" ] && [ "$old_tcc" -eq 1 ]; then
            sk "verify $unit (ancien TCC)"
        elif systemd-analyze verify "/usr/lib/systemd/system/$unit" >/dev/null 2>&1; then
            ok "verify $unit"
        else
            ko "verify $unit"
        fi
    else
        sk "$unit non installé"
    fi
done

section "Hardening runtime"
if [ "$old_tcc" -eq 1 ]; then
    sk "DeviceAllow runtime non testable : ancien TCC actif (utiliser runtime-system-check.sh)"
elif systemctl show tccd.service -p DeviceAllow >/tmp/device-allow.txt 2>/dev/null; then
    cat /tmp/device-allow.txt
    if grep -q '/dev/tuxedo_io' /tmp/device-allow.txt; then ok "DeviceAllow tuxedo_io"; else ko "DeviceAllow tuxedo_io"; fi
    if grep -q 'char-video4linux r' /tmp/device-allow.txt; then ok "DeviceAllow vidéo lecture"; else sk "DeviceAllow vidéo absent"; fi
else
    sk "tccd.service non chargé"
fi

section "D-Bus tccd (root)"
if [ "$old_tcc" -eq 1 ]; then
    sk "Manager1 non testable : ancien TCC actif"
elif busctl --system status com.tuxedocomputers.tccd >/dev/null 2>&1; then
    ok "nom com.tuxedocomputers.tccd possédé"
    if busctl --system call com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd \
        com.tuxedocomputers.tccd.Manager1 GetContractVersion >/dev/null 2>&1; then
        ok "Manager1.GetContractVersion"
    else
        ko "Manager1.GetContractVersion"
    fi
    if busctl --system call com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd \
        com.tuxedocomputers.tccd.Manager1 GetShutdownTimerJSON >/dev/null 2>&1; then
        ok "Manager1.GetShutdownTimerJSON"
    else
        ko "Manager1.GetShutdownTimerJSON"
    fi
else
    sk "tccd non actif"
fi

section "Control1 root-only"
if [ "$old_tcc" -eq 1 ]; then
    sk "Control1 non testable : ancien TCC actif"
elif command -v runuser >/dev/null 2>&1 && id nobody >/dev/null 2>&1; then
    if runuser -u nobody -- busctl --system call com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd \
        com.tuxedocomputers.tccd.Control1 ReloadConfig >/dev/null 2>&1; then
        ko "Control1 accepté pour nobody (attendu : refus)"
    else
        ok "Control1 refusé pour nobody"
    fi
else
    sk "runuser/nobody indisponible"
fi

section "Admin phase 1 (si installé)"
# Active le service à la demande (Ping), sans écriture.
if busctl --system call com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin \
    com.tuxedocomputers.tccd.Admin1 Ping >/dev/null 2>&1; then
    ok "nom admin possédé"
    for method in Ping GetVersion GetContractVersion; do
        if busctl --system call com.tuxedocomputers.tccd.Admin \
            /com/tuxedocomputers/tccd/Admin \
            com.tuxedocomputers.tccd.Admin1 "$method" >/dev/null 2>&1; then
            ok "Admin1.$method"
        else
            ko "Admin1.$method"
        fi
    done
    if busctl --system call com.tuxedocomputers.tccd.Admin \
        /com/tuxedocomputers/tccd/Admin \
        org.freedesktop.DBus.Properties.Get \
        string:com.tuxedocomputers.tccd.Admin1 string:ContractVersion >/dev/null 2>&1; then
        ko "Properties.Get accepté sur Admin (attendu : refus)"
    else
        ok "Properties.Get refusé sur Admin"
    fi
else
    sk "tccd-admin non installé (TCC_WITH_ADMIN=1 non activé)"
fi

section "Admin.Hw phase 2 (si installé)"
# Active le service à la demande (Ping), sans écriture.
if busctl --system call com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.AdminHw1 Ping >/dev/null 2>&1; then
    ok "nom Admin.Hw possédé"
    for method in Ping GetVersion GetContractVersion; do
        if busctl --system call com.tuxedocomputers.tccd.Admin.Hw \
            /com/tuxedocomputers/tccd/Admin/Hw \
            com.tuxedocomputers.tccd.AdminHw1 "$method" >/dev/null 2>&1; then
            ok "AdminHw1.$method"
        else
            ko "AdminHw1.$method"
        fi
    done
    busctl --system call com.tuxedocomputers.tccd.Admin.Hw \
        /com/tuxedocomputers/tccd/Admin/Hw \
        com.tuxedocomputers.tccd.AdminHw1 GetTdpValues >/dev/null 2>&1 \
        && ok "AdminHw1.GetTdpValues" || sk "GetTdpValues indisponible (matériel)"
    if busctl --system call com.tuxedocomputers.tccd.Admin.Hw \
        /com/tuxedocomputers/tccd/Admin/Hw \
        org.freedesktop.DBus.Properties.Get \
        string:com.tuxedocomputers.tccd.AdminHw1 string:ContractVersion >/dev/null 2>&1; then
        ko "Properties.Get accepté sur Admin.Hw (attendu : refus)"
    else
        ok "Properties.Get refusé sur Admin.Hw"
    fi
    if systemctl show tccd-admin-hw.service -p DeviceAllow >/tmp/device-allow-hw.txt 2>/dev/null; then
        grep -q '/dev/tuxedo_io' /tmp/device-allow-hw.txt && ok "DeviceAllow admin-hw" || sk "DeviceAllow admin-hw non chargé"
    fi
else
    sk "tccd-admin-hw non installé/actif (TCC_WITH_ADMIN_HW=1)"
fi

section "État persistant"
state_dir=/var/lib/tuxedo-rust-center
if [ -d "$state_dir" ]; then
    ls -la "$state_dir" || true
    if [ -f "$state_dir/profiles.json" ]; then
        mode=$(stat -c '%a %U:%G' "$state_dir/profiles.json")
        printf 'INFO  profiles.json %s\n' "$mode"
    fi
    for bak in "$state_dir"/*.bak; do
        [ -e "$bak" ] || continue
        mode=$(stat -c '%a %U:%G' "$bak")
        printf 'INFO  %s %s\n' "$(basename "$bak")" "$mode"
    done
else
    sk "$state_dir absent"
fi

section "Audit journald"
if command -v journalctl >/dev/null 2>&1; then
    journalctl -u tccd -u tccd-admin -n 40 --no-pager 2>/dev/null |
        grep -E 'caller_uid|result=|backup=|reload=' | tail -15 || true
fi

section "Bilan"
printf 'réussis=%d échecs=%d ignorés=%d\n' "$pass" "$fail" "$skip"
[ "$fail" -eq 0 ]
