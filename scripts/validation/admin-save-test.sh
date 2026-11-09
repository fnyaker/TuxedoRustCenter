#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test réel de Admin1.SaveSettingsJSON : bascule fahrenheit, vérifie le
# rechargement, puis restaure. À lancer en root (polkit root autorisé).
#
# Usage : sudo ./scripts/validation/admin-save-test.sh
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    printf 'ERREUR : root requis (sudo).\n' >&2
    exit 77
fi

service=com.tuxedocomputers.tccd
admin=com.tuxedocomputers.tccd.Admin
path=/com/tuxedocomputers/tccd/Admin

tmp=$(mktemp -d /tmp/tcc-admin-save.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

printf '== Lecture des réglages ==\n'
tcc-cli settings >"$tmp/original.json"

python3 - "$tmp/original.json" "$tmp/modified.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
data["fahrenheit"] = not bool(data.get("fahrenheit", False))
json.dump(data, open(sys.argv[2], "w"), ensure_ascii=False, indent=2)
print("fahrenheit ->", data["fahrenheit"])
PY

printf '== SaveSettingsJSON (activation admin + polkit root) ==\n'
busctl --system call "$admin" "$path" com.tuxedocomputers.tccd.Admin1 \
    SaveSettingsJSON s "$(cat "$tmp/modified.json")"

sleep 1
printf '== Vérification ==\n'
tcc-cli settings >"$tmp/after.json"
python3 - "$tmp/original.json" "$tmp/after.json" <<'PY'
import json, sys
before = json.load(open(sys.argv[1]))
after = json.load(open(sys.argv[2]))
if before.get("fahrenheit") == after.get("fahrenheit"):
    print("FAIL  fahrenheit n'a pas changé")
    raise SystemExit(1)
print(f"PASS  fahrenheit {before.get('fahrenheit')} -> {after.get('fahrenheit')}")
PY

printf '== Restauration ==\n'
busctl --system call "$admin" "$path" com.tuxedocomputers.tccd.Admin1 \
    SaveSettingsJSON s "$(cat "$tmp/original.json")"
sleep 1
tcc-cli settings >"$tmp/restored.json"
python3 - "$tmp/original.json" "$tmp/restored.json" <<'PY'
import json, sys
before = json.load(open(sys.argv[1]))
after = json.load(open(sys.argv[2]))
if before.get("fahrenheit") != after.get("fahrenheit"):
    print("FAIL  restauration incorrecte")
    raise SystemExit(1)
print("PASS  réglages restaurés")
PY

printf '== Backups ==\n'
ls -l /var/lib/tuxedo-rust-center/*.bak 2>/dev/null || true
printf '\nTest terminé.\n'
