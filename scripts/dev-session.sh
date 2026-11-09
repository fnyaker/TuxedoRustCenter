#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Stack de développement TuxedoRustCenter :
#   - daemon Rust en dry-run sur le bus de session ;
#   - agent de session (refresh rate) ;
#   - GUI Qt 6 native.
#
# Aucun root, aucune écriture /sys ou /etc, aucun conflit avec le TCC original.

set -eu

ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
SERVICE="${TCC_DBUS_SERVICE:-com.tuxedocomputers.tccd.Dev}"
export TCC_DBUS_BUS=session
export TCC_DBUS_SERVICE="$SERVICE"

# La GUI doit être construite avec `dev-overrides` pour honorer
# TCC_DBUS_BUS/TCC_DBUS_SERVICE (D7/R-14) : ces variables sont ignorées par un
# binaire sans la feature, qui resterait sur le bus système.
cargo build -p tcc-daemon -p tcc-user
cargo build -p tcc-gui --features dev-overrides

"$ROOT/target/debug/tcc-daemon" --dev-session --bus-name "$SERVICE" &
DAEMON_PID=$!
AGENT_PID=

cleanup() {
    if [ -n "${AGENT_PID:-}" ]; then
        kill "$AGENT_PID" 2>/dev/null || true
    fi
    kill "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Laisse le temps au nom D-Bus d'être publié sur le bus de session.
attempt=0
while [ "$attempt" -lt 20 ]; do
    if busctl --user status "$SERVICE" >/dev/null 2>&1; then
        break
    fi
    attempt=$((attempt + 1))
    sleep 0.2
done

"$ROOT/target/debug/tcc-user" &
AGENT_PID=$!

"$ROOT/target/debug/tcc-gui"
