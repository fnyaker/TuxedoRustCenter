#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Vérifications de validation non-root de TuxedoRustCenter.
# Aucune écriture système, aucun root, aucune modification de session.
#
# Usage : ./scripts/validation/nonroot-check.sh
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

fail=0
pass=0
skip=0

ok() { printf 'PASS  %s\n' "$1"; pass=$((pass + 1)); }
ko() { printf 'FAIL  %s\n' "$1"; fail=$((fail + 1)); }
sk() { printf 'SKIP  %s\n' "$1"; skip=$((skip + 1)); }

section() { printf '\n== %s ==\n' "$1"; }

section "Outils"
for tool in cargo xmllint systemd-analyze; do
    if command -v "$tool" >/dev/null 2>&1; then ok "$tool présent"; else ko "$tool absent"; fi
done

section "Format, lints, tests"
if cargo fmt --all -- --check >/dev/null 2>&1; then ok "cargo fmt"; else ko "cargo fmt"; fi
if cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; then ok "clippy -D warnings"; else ko "clippy"; fi
if cargo test --workspace --quiet >/dev/null 2>&1; then ok "cargo test --workspace"; else ko "cargo test"; fi

section "Packaging statique"
if ./scripts/release/validate.sh --repo >/dev/null 2>&1; then ok "validate.sh --repo"; else ko "validate.sh --repo"; fi
for xml in packaging/dbus/com.tuxedocomputers.tccd.conf packaging/polkit/com.tuxedocomputers.tccd.policy; do
    if xmllint --noout "$xml" >/dev/null 2>&1; then ok "xmllint $(basename "$xml")"; else ko "xmllint $xml"; fi
done
for unit in tccd tccd-user tccd-admin; do
    if systemd-analyze verify "packaging/systemd/$unit.service" >/dev/null 2>&1; then
        ok "systemd-analyze verify $unit"
    else
        sk "systemd-analyze verify $unit (ExecStart non installé)"
    fi
done

section "Session et matériel"
session_type=${XDG_SESSION_TYPE:-inconnu}
desktop=${XDG_CURRENT_DESKTOP:-inconnu}
printf 'INFO  session=%s desktop=%s wayland=%s display=%s\n' \
    "$session_type" "$desktop" "${WAYLAND_DISPLAY:-none}" "${DISPLAY:-none}"

if command -v busctl >/dev/null 2>&1; then
    if busctl --system get-property net.hadess.PowerProfiles \
        /net/hadess/PowerProfiles net.hadess.PowerProfiles ActiveProfile >/dev/null 2>&1; then
        ok "power-profiles-daemon joignable"
    else
        ko "power-profiles-daemon absent"
    fi
fi

if [ -e /dev/tuxedo_io ]; then ok "/dev/tuxedo_io présent"; else sk "/dev/tuxedo_io absent"; fi
if compgen -G "/dev/video*" >/dev/null; then ok "nœuds /dev/video* présents"; else sk "aucune caméra V4L2"; fi

section "Binaires (dev)"
cargo build -q -p tcc-daemon -p tcc-user -p tcc-cli >/dev/null 2>&1 || ko "build dev"
if ./target/debug/tcc-daemon --check >/tmp/tcc-check.json 2>/dev/null; then
    ok "tccd --check"
    python3 - <<'PY' || true
import json
text = open('/tmp/tcc-check.json', encoding='utf-8', errors='replace').read()
start = text.find('{')
data = json.loads(text[start:]) if start >= 0 else {}
print(f"INFO  ppd={data.get('ppdProfile')} power={data.get('powerSource')} profil={data.get('activeProfile', {}).get('name')}")
PY
else
    ko "tccd --check"
fi

if ./target/debug/tcc-user --list >/tmp/tcc-user.json 2>/dev/null; then
    ok "tccd-user --list"
    python3 - <<'PY' || true
import json
with open('/tmp/tcc-user.json') as handle:
    data = json.load(handle)
print(f"INFO  backend={data.get('backend', {}).get('name')} interne={data.get('internal_output', {}).get('name')} activé={data.get('internal_output', {}).get('enabled')}")
PY
else
    ko "tccd-user --list"
fi

if ./target/debug/tcc-user --dry-run --set-refresh 60 >/tmp/tcc-dry.txt 2>&1; then
    ok "tccd-user --dry-run 60 Hz"
    sed -n '1,3p' /tmp/tcc-dry.txt
else
    ko "tccd-user --dry-run"
fi

section "Caméra réelle (opt-in)"
if compgen -G "/dev/video*" >/dev/null; then
    if cargo test -p tcc-hw --test real_v4l2 -- --ignored --nocapture >/tmp/tcc-v4l2.txt 2>&1; then
        ok "lecture UVC réelle"
        grep -E 'available=|controls=' /tmp/tcc-v4l2.txt | head -5 || true
    else
        ko "lecture UVC réelle"
    fi
else
    sk "aucune caméra"
fi

section "Bilan"
printf 'réussis=%d échecs=%d ignorés=%d\n' "$pass" "$fail" "$skip"
[ "$fail" -eq 0 ]
