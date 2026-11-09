#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Validation runtime du daemon TuxedoRustCenter sur le bus système, en
# dry-run, sans désinstaller le TCC officiel.
#
# Le script :
#   1. arrête temporairement tccd.service (TCC officiel) s'il est actif ;
#   2. lance notre target/release/tcc-daemon --dry-run --start ;
#   3. vérifie Manager1/Control1 et le refus des setters non-root ;
#   4. restaure automatiquement le service officiel (trap).
#
# Usage : sudo ./scripts/validation/runtime-system-check.sh
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : root requis (sudo).\n' >&2
    exit 77
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

daemon="$repo_root/target/release/tcc-daemon"
log=/tmp/tccd-runtime-dry-run.log
pid=""
official_was_active=0

pass=0
fail=0
ok() { printf 'PASS  %s\n' "$1"; pass=$((pass + 1)); }
ko() { printf 'FAIL  %s\n' "$1"; fail=$((fail + 1)); }
sk() { printf 'SKIP  %s\n' "$1"; }

cleanup() {
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        kill -TERM "$pid" 2>/dev/null || true
        for _ in $(seq 1 30); do
            kill -0 "$pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    if [ "$official_was_active" -eq 1 ]; then
        systemctl start tccd.service 2>/dev/null || true
        printf 'INFO  TCC officiel redémarré\n'
    fi
}
trap cleanup EXIT INT TERM

if [ ! -x "$daemon" ]; then
    printf 'ERREUR : %s absent ; lancer cargo build --release --workspace.\n' "$daemon" >&2
    exit 1
fi

printf '== Préparation ==\n'
if systemctl is-active --quiet tccd.service 2>/dev/null; then
    official_was_active=1
    printf 'INFO  arrêt temporaire du TCC officiel\n'
    systemctl stop tccd.service
fi

install -d -m 0755 /var/lib/tuxedo-rust-center /run/tuxedo-rust-center

printf '== Démarrage du daemon en dry-run ==\n'
rm -f "$log"
"$daemon" --dry-run --start >"$log" 2>&1 &
pid=$!

ready=0
for _ in $(seq 1 100); do
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    if busctl --system status com.tuxedocomputers.tccd >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.1
done

if [ "$ready" -eq 1 ]; then
    ok "daemon démarré sur le bus système (dry-run)"
else
    ko "daemon non démarré"
    printf '%s\n' '--- log ---'
    tail -30 "$log" || true
    exit 1
fi

printf '== Lectures D-Bus ==\n'
if busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd.Manager1 GetContractVersion >/dev/null 2>&1; then
    ok "Manager1.GetContractVersion"
else
    ko "Manager1.GetContractVersion"
fi
if busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd.Manager1 GetShutdownTimerJSON >/dev/null 2>&1; then
    ok "Manager1.GetShutdownTimerJSON"
else
    ko "Manager1.GetShutdownTimerJSON"
fi
if busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd.Manager1 GetHardwareSnapshotJSON >/dev/null 2>&1; then
    ok "Manager1.GetHardwareSnapshotJSON"
else
    ko "Manager1.GetHardwareSnapshotJSON"
fi
if busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd GetStateJSON >/dev/null 2>&1; then
    ok "GetStateJSON"
else
    ko "GetStateJSON"
fi

printf '== Policy (non-root) ==\n'
if command -v runuser >/dev/null 2>&1 && id nobody >/dev/null 2>&1; then
    if runuser -u nobody -- busctl --system call com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd \
        com.tuxedocomputers.tccd.Manager1 SetChargingProfile s balanced >/dev/null 2>&1; then
        ko "setter Manager1 accepté pour nobody (attendu : refus policy)"
    else
        ok "setter Manager1 refusé pour nobody"
    fi
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

printf '== Control1 root ==\n'
if busctl --system call com.tuxedocomputers.tccd /com/tuxedocomputers/tccd \
    com.tuxedocomputers.tccd.Control1 ReloadConfig >/dev/null 2>&1; then
    ok "Control1.ReloadConfig root"
else
    ko "Control1.ReloadConfig root"
fi

printf '== Admin (si installé) ==\n'
if busctl --system status com.tuxedocomputers.tccd.Admin >/dev/null 2>&1; then
    if busctl --system call com.tuxedocomputers.tccd.Admin \
        /com/tuxedocomputers/tccd/Admin \
        com.tuxedocomputers.tccd.Admin1 Ping >/dev/null 2>&1; then
        ok "Admin1.Ping"
    else
        ko "Admin1.Ping"
    fi
else
    sk "tccd-admin non installé/actif (TCC_WITH_ADMIN=1)"
fi

printf '== Logs dry-run ==\n'
grep -iE 'dry-run|aucune écriture' "$log" | tail -5 || true

printf '== Bilan ==\n'
printf 'réussis=%d échecs=%d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
