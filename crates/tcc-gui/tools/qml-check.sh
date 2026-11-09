#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 TuxedoRustCenter contributors
#
# Lint QML + tests Qt Quick du shell. À lancer depuis la racine du dépôt.
# Qt 6 est cherché dans /usr/lib/qt6/bin (Arch) puis dans le PATH.

set -eu

QT_BIN="${QT_BIN:-}"
if [ -z "$QT_BIN" ]; then
    if [ -x /usr/lib/qt6/bin/qmllint ]; then
        QT_BIN=/usr/lib/qt6/bin
    else
        QT_BIN="$(dirname "$(command -v qmllint)")"
    fi
fi

ROOT="$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)"
QML="$ROOT/crates/tcc-gui/qml"
TESTS="$ROOT/crates/tcc-gui/tests/qml"

echo "== qmllint ($QT_BIN) =="
IMPORT_ARGS=""
if [ -d "$ROOT/target/cxxqt/qml_modules" ]; then
    IMPORT_ARGS="-I $ROOT/target/cxxqt/qml_modules"
fi
# shellcheck disable=SC2086
"$QT_BIN/qmllint" $IMPORT_ARGS \
    "$QML/Main.qml" \
    "$QML"/tray/*.qml \
    "$QML"/core/*.qml \
    "$QML"/components/*.qml \
    "$QML"/pages/*.qml \
    "$TESTS"/*.qml

echo "== qmltestrunner =="
QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-offscreen}" \
    "$QT_BIN/qmltestrunner" -input "$TESTS"
