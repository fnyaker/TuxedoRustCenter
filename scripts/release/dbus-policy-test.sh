#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test fonctionnel de la politique D-Bus dans un bus système isolé.
#
# Vérifie, avec un vrai dbus-daemon :
#   - le fichier est syntaxiquement valide et démarre ;
#   - un utilisateur non privilégié peut appeler les méthodes listées ;
#   - une méthode non listée est refusée par le bus (AccessDenied) ;
#   - Properties.Set est refusé ;
#   - un utilisateur non privilégié ne peut pas posséder le nom du service.
#
# Requiert root (le service de test doit posséder le nom, réservé à root) et
# dbus-daemon, dbus-send, dbus-test-tool, useradd, runuser.
#
# Usage :
#   sudo scripts/release/dbus-policy-test.sh [POLICY]
#
# Sortie : 0 si tous les cas passent, 77 si l'environnement ne permet pas le
# test (non root ou outil absent), 1 sinon.

set -euo pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

REPO_ROOT=$(tcc_repo_root "$SCRIPT_DIR")
POLICY=${1:-"$REPO_ROOT/packaging/dbus/com.tuxedocomputers.tccd.conf"}

[ -f "$POLICY" ] || tcc_die "politique introuvable: $POLICY"
# dbus-daemon résout <include> relativement au fichier de configuration du bus
# de test (créé dans $tmp). Un chemin relatif ferait donc chercher
# « $tmp/packaging/... ». Canonicaliser AVANT d'écrire bus.conf.
POLICY_DIR=$(CDPATH='' cd -- "$(dirname -- "$POLICY")" && pwd -P)
POLICY="$POLICY_DIR/$(basename -- "$POLICY")"

if [ "$(id -u)" -ne 0 ]; then
    printf 'SKIP: le test D-Bus nécessite root (service de test root, client non privilégié)\n' >&2
    exit 77
fi

for tool in dbus-daemon dbus-send dbus-test-tool useradd runuser timeout; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'SKIP: outil absent: %s\n' "$tool" >&2
        exit 77
    fi
done

TEST_USER=tcc-policy-test
if ! id "$TEST_USER" >/dev/null 2>&1; then
    useradd -M -s /bin/sh "$TEST_USER"
fi

tmp=$(mktemp -d)
# Le socket du bus est créé dans $tmp : le client non privilégié doit pouvoir
# traverser le répertoire (le fichier socket lui-même est protégé par le bus).
chmod 0755 "$tmp"
bus_pid=""
echo_pid=""
admin_pid=""
hw_pid=""
cleanup() {
    [ -n "$hw_pid" ] && kill "$hw_pid" >/dev/null 2>&1 || true
    [ -n "$admin_pid" ] && kill "$admin_pid" >/dev/null 2>&1 || true
    [ -n "$echo_pid" ] && kill "$echo_pid" >/dev/null 2>&1 || true
    [ -n "$bus_pid" ] && kill "$bus_pid" >/dev/null 2>&1 || true
    rm -rf "$tmp"
    userdel "$TEST_USER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Bus de test : règles de base (connexion, réponses) AVANT l'include, pour que
# les deny/allow de la politique livrée soient les dernières règles (dernière
# règle gagnante, dbus-daemon(1)).
{
    printf '%s\n' '<busconfig>'
    printf '%s\n' '  <type>system</type>'
    printf '  <listen>unix:tmpdir=%s</listen>\n' "$tmp"
    printf '  <pidfile>%s/pid</pidfile>\n' "$tmp"
    printf '%s\n' '  <policy context="default">'
    printf '%s\n' '    <allow user="*"/>'
    printf '%s\n' '    <allow send_destination="org.freedesktop.DBus" send_interface="org.freedesktop.DBus"/>'
    printf '%s\n' '    <allow send_destination="org.freedesktop.DBus" send_interface="org.freedesktop.DBus.Introspectable"/>'
    printf '%s\n' '    <allow send_destination="org.freedesktop.DBus" send_interface="org.freedesktop.DBus.Properties"/>'
    printf '%s\n' '    <allow send_type="signal"/>'
    printf '%s\n' '    <allow send_requested_reply="true" send_type="method_return"/>'
    printf '%s\n' '    <allow send_requested_reply="true" send_type="error"/>'
    printf '%s\n' '    <allow receive_type="method_call"/>'
    printf '%s\n' '    <allow receive_type="method_return"/>'
    printf '%s\n' '    <allow receive_type="error"/>'
    printf '%s\n' '    <allow receive_type="signal"/>'
    printf '%s\n' '  </policy>'
    printf '  <include>%s</include>\n' "$POLICY"
    printf '%s\n' '</busconfig>'
} > "$tmp/bus.conf"

dbus-daemon --config-file="$tmp/bus.conf" --fork --print-address --print-pid > "$tmp/bus.info"
address=$(sed -n '1p' "$tmp/bus.info")
bus_pid=$(sed -n '2p' "$tmp/bus.info")
export DBUS_SYSTEM_BUS_ADDRESS="$address"

# Services de test root : possèdent les noms (autorisés uniquement à root).
dbus-test-tool echo --name=com.tuxedocomputers.tccd --system >/dev/null 2>&1 &
echo_pid=$!
dbus-test-tool echo --name=com.tuxedocomputers.tccd.Admin --system >/dev/null 2>&1 &
admin_pid=$!
dbus-test-tool echo --name=com.tuxedocomputers.tccd.Admin.Hw --system >/dev/null 2>&1 &
hw_pid=$!
sleep 1

FAILURES=0
check_ok() {
    local label=$1
    shift
    if "$@" >/dev/null 2>&1; then
        printf 'OK    %s\n' "$label"
    else
        printf 'ÉCHEC %s\n' "$label" >&2
        FAILURES=$((FAILURES + 1))
    fi
}

check_denied() {
    local label=$1
    shift
    local out
    if out=$("$@" 2>&1); then
        printf 'ÉCHEC %s (appel accepté)\n' "$label" >&2
        FAILURES=$((FAILURES + 1))
        return
    fi
    if printf '%s' "$out" | grep -qE 'AccessDenied|Access denied|Rejected send message|failed to take bus name'; then
        printf 'OK    %s (refusé par le bus)\n' "$label"
    else
        printf 'ÉCHEC %s (refus inattendu: %s)\n' "$label" "$out" >&2
        FAILURES=$((FAILURES + 1))
    fi
}

client() {
    runuser -u "$TEST_USER" -- env DBUS_SYSTEM_BUS_ADDRESS="$address" "$@"
}

# Appel root (le script tourne déjà en root) : utilisé pour Control1.
root_call() {
    env DBUS_SYSTEM_BUS_ADDRESS="$address" "$@"
}

check_ok "GetStateJSON autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.GetStateJSON

check_ok "Introspect autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd org.freedesktop.DBus.Introspectable.Introspect

check_ok "SetTempProfile autorisé (risque documenté)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.SetTempProfile string:__office__

# Manager1 : lecture et bail capteurs autorisés (aucun setter matériel).
for member in GetHardwareSnapshotJSON GetSensorDataCollectionStatus; do
    check_ok "Manager1.$member autorisé" \
        client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd "com.tuxedocomputers.tccd.Manager1.$member"
done

check_ok "Manager1.SetSensorDataCollectionStatus autorisé (bail)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Manager1.SetSensorDataCollectionStatus \
    boolean:true

check_ok "Manager1.GetShutdownTimerJSON autorisé (lecture)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Manager1.GetShutdownTimerJSON

# Manager1 : setters BLOQUÉS au bus en attente M1-01 (bucket UID + DialogGate)
# et M1-02 (lighting respectant keyboard_backlight_control_enabled).
# À réactiver en check_ok quand security confirme la levée des deux findings
# et que la policy est rouverte membre par membre.
for member in \
    SetChargingProfile SetChargingPriority SetChargeStartThreshold SetChargeEndThreshold \
    SetChargeType SetKeyboardStatesJSON SetLightbarStatesJSON SetFnLockStatus; do
    check_denied "Manager1.$member refusé (M1-01/M1-02)" \
        client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
        /com/tuxedocomputers/tccd "com.tuxedocomputers.tccd.Manager1.$member"
done

# P-12 : la version du contrat passe par la méthode Manager1, pas par
# Properties. tccd n'autorise plus Properties.Get/GetAll.
check_ok "Manager1.GetContractVersion autorisé (P-12)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Manager1.GetContractVersion

# ---------------------------------------------------------------------------
# Admin1 phase 1 : 3 méthodes d'écriture autorisées, le reste refusé au bus.
# ---------------------------------------------------------------------------
check_ok "Admin Introspect autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin org.freedesktop.DBus.Introspectable.Introspect

check_ok "Admin Peer.Ping autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin org.freedesktop.DBus.Peer.Ping

check_ok "Admin1.Ping autorisé (diagnostic)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.Ping

check_ok "Admin1.GetVersion autorisé (négociation)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.GetVersion

check_ok "Admin1.GetContractVersion autorisé (négociation A1-11)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.GetContractVersion

check_ok "Admin1.SaveProfilesJSON autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.SaveProfilesJSON string:'[]'

check_ok "Admin1.SaveSettingsJSON autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.SaveSettingsJSON string:'{}'

check_ok "Admin1.ReloadConfig autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin com.tuxedocomputers.tccd.Admin1.ReloadConfig

for member in SetTdpValues SetOdmProfile SetNvidiaCtgpOffset SetPrimeMode \
    TomteRun PackagesApply; do
    check_denied "Admin1.$member refusé (phase 1)" \
        client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
        /com/tuxedocomputers/tccd/Admin "com.tuxedocomputers.tccd.Admin1.$member"
done

check_denied "Admin Properties.Get refusé (A1-11)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin org.freedesktop.DBus.Properties.Get \
    string:com.tuxedocomputers.tccd.Admin1 string:ContractVersion

check_denied "Admin Properties.GetAll refusé (A1-11)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin org.freedesktop.DBus.Properties.GetAll \
    string:com.tuxedocomputers.tccd.Admin1

check_denied "Admin Properties.Set refusé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin org.freedesktop.DBus.Properties.Set \
    string:com.tuxedocomputers.tccd.Admin1 string:ContractVersion variant:uint32:2

check_denied "Revendication du nom Admin refusée à un non-root" \
    client timeout 2 dbus-test-tool echo --name=com.tuxedocomputers.tccd.Admin --system

# ---------------------------------------------------------------------------
# Admin.Hw phase 2 (AD-12 B2) : TDP/ODM/cTGP, membres listés un par un.
# Le fichier de policy est toujours installé ; l'activation D-Bus ne l'est que
# si TCC_WITH_ADMIN_HW=1. Le bus autorise, polkit tranche dans le service.
# ---------------------------------------------------------------------------
check_ok "Admin.Hw Introspect autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw org.freedesktop.DBus.Introspectable.Introspect

check_ok "Admin.Hw Peer.Ping autorisé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw org.freedesktop.DBus.Peer.Ping

for member in Ping GetVersion GetContractVersion GetTdpValues; do
    check_ok "AdminHw1.$member autorisé (lecture)" \
        client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
        /com/tuxedocomputers/tccd/Admin/Hw \
        "com.tuxedocomputers.tccd.AdminHw1.$member"
done

check_ok "AdminHw1.SetTdpValues autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.AdminHw1.SetTdpValues array:int32:35

check_ok "AdminHw1.SetOdmProfile autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.AdminHw1.SetOdmProfile string:balanced

check_ok "AdminHw1.SetOdmProfileOverride autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.AdminHw1.SetOdmProfileOverride string:balanced

check_ok "AdminHw1.SetNvidiaCtgpOffset autorisé (bus ; polkit dans le service)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.AdminHw1.SetNvidiaCtgpOffset int32:10

# Méthodes non listées / interfaces étrangères : refusées par la policy.
for member in SetPrimeMode TomteRun PackagesApply Evil; do
    check_denied "AdminHw1.$member refusé (non listé)" \
        client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
        /com/tuxedocomputers/tccd/Admin/Hw \
        "com.tuxedocomputers.tccd.AdminHw1.$member"
done

check_denied "Admin1.SaveProfilesJSON refusé sur Admin.Hw" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw \
    com.tuxedocomputers.tccd.Admin1.SaveProfilesJSON string:'[]'

check_denied "AdminHw1.SetTdpValues refusé sur Admin (phase 1)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin \
    /com/tuxedocomputers/tccd/Admin \
    com.tuxedocomputers.tccd.AdminHw1.SetTdpValues array:int32:35

check_denied "Admin.Hw Properties.Get refusé (A1-11)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw org.freedesktop.DBus.Properties.Get \
    string:com.tuxedocomputers.tccd.AdminHw1 string:ContractVersion

check_denied "Admin.Hw Properties.GetAll refusé (A1-11)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw org.freedesktop.DBus.Properties.GetAll \
    string:com.tuxedocomputers.tccd.AdminHw1

check_denied "Admin.Hw Properties.Set refusé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd.Admin.Hw \
    /com/tuxedocomputers/tccd/Admin/Hw org.freedesktop.DBus.Properties.Set \
    string:com.tuxedocomputers.tccd.AdminHw1 string:ContractVersion variant:uint32:2

check_denied "Revendication du nom Admin.Hw refusée à un non-root" \
    client timeout 2 dbus-test-tool echo --name=com.tuxedocomputers.tccd.Admin.Hw --system

# ---------------------------------------------------------------------------
# Control1.ReloadConfig : root uniquement (AD-6).
# ---------------------------------------------------------------------------
check_denied "Control1.ReloadConfig refusé à un non-root" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Control1.ReloadConfig

check_ok "Control1.ReloadConfig autorisé à root" \
    root_call dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Control1.ReloadConfig

check_denied "méthode non listée refusée" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd com.tuxedocomputers.tccd.Evil

check_denied "tccd Properties.Get refusé (P-12)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd org.freedesktop.DBus.Properties.Get \
    string:com.tuxedocomputers.tccd string:ContractVersion

check_denied "tccd Properties.GetAll refusé (P-12)" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd org.freedesktop.DBus.Properties.GetAll \
    string:com.tuxedocomputers.tccd

check_denied "Properties.Set refusé" \
    client dbus-send --system --print-reply --dest=com.tuxedocomputers.tccd \
    /com/tuxedocomputers/tccd org.freedesktop.DBus.Properties.Set \
    string:com.tuxedocomputers.tccd string:ContractVersion variant:uint32:2

check_denied "Revendication du nom refusée à un non-root" \
    client timeout 2 dbus-test-tool echo --name=com.tuxedocomputers.tccd --system

if [ "$FAILURES" -gt 0 ]; then
    printf '\n%d test(s) D-Bus en échec\n' "$FAILURES" >&2
    exit 1
fi
printf '\nPolitique D-Bus: tous les cas passent\n'
