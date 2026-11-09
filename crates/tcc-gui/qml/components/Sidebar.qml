// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Barre latérale : identité, navigation principale, profils utilisés par état,
// navigation secondaire et utilitaires (langue, thème, tray).
// Se replie en mode icônes sous 1120 px de large (voir Main.qml).

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Rectangle {
    id: root

    property bool compact: false

    implicitWidth: compact ? Theme.sidebarCompactWidth : Theme.sidebarWidth
    color: Theme.sidebar

    // Séparateur vertical
    Rectangle {
        anchors.right: parent.right
        width: 1
        height: parent.height
        color: Theme.border
    }

    function profileForAc() {
        return LiveData.assignedAc;
    }

    function profileForBat() {
        return LiveData.assignedBat;
    }

    ColumnLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.margins: Theme.s12
        spacing: Theme.s4

        // ── Identité ────────────────────────────────────────────────────────
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 56

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: root.compact ? 0 : Theme.s4
                anchors.rightMargin: Theme.s4
                spacing: Theme.s10

                Rectangle {
                    Layout.alignment: Qt.AlignVCenter
                    Layout.preferredWidth: 34
                    Layout.preferredHeight: 34
                    radius: Theme.radiusSm
                    color: "#ffffff"
                    border.width: 1
                    border.color: Theme.border

                    Image {
                        anchors.centerIn: parent
                        source: "../../assets/brand/taskbar_icon_free.svg"
                        sourceSize.width: 52
                        width: 26
                        height: 26
                        fillMode: Image.PreserveAspectFit
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    visible: !root.compact
                    spacing: 0

                    AppLabel {
                        variant: "small"
                        strong: true
                        text: "TUXEDO"
                        font.letterSpacing: 1.4
                        font.pixelSize: 13
                    }

                    AppLabel {
                        variant: "caption"
                        secondary: true
                        text: Strings.t("app.short")
                    }
                }

                StatusBadge {
                    Layout.alignment: Qt.AlignVCenter
                    visible: !root.compact
                    kind: LiveData.connected ? "ok" : "warn"
                    label: LiveData.connected
                           ? Strings.t("state.connected")
                           : Strings.t("common.demoTag")
                    AppToolTip {
                        visible: demoHover.hovered
                        text: LiveData.connected
                              ? Strings.t("state.connected")
                              : (LiveData.lastError !== ""
                                 ? LiveData.lastError : Strings.t("app.demoHint"))
                    }
                    HoverHandler { id: demoHover }
                }

                IconButton {
                    id: retryButton
                    visible: LiveData.lastError !== ""
                    enabled: !LiveData.commandBusy
                    iconName: "update"
                    tooltip: Strings.t("common.retry")
                    Accessible.name: Strings.t("common.retry")
                    onClicked: AppState.requestRefresh()
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            Layout.bottomMargin: Theme.s4
            Layout.preferredHeight: 1
            color: Theme.divider
        }

        // ── Navigation principale ───────────────────────────────────────────
        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "dashboard"
            label: Strings.t("nav.dashboard")
            selected: AppState.section === "dashboard"
            onClicked: AppState.navigate("dashboard")
        }

        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "allprofiles"
            label: Strings.t("nav.profiles")
            selected: AppState.section === "profiles"
            onClicked: AppState.navigate("profiles")
        }

        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "tools"
            label: Strings.t("nav.tools")
            selected: AppState.section === "tools"
            onClicked: AppState.navigate("tools")
        }

        // ── Profils utilisés ────────────────────────────────────────────────
        AppLabel {
            Layout.leftMargin: Theme.s4
            Layout.topMargin: Theme.s16
            Layout.bottomMargin: Theme.s4
            visible: !root.compact
            variant: "label"
            secondary: true
            text: Strings.t("nav.usedProfiles")
        }

        UsedProfileRow {
            Layout.fillWidth: true
            compact: root.compact
            stateIcon: "pluggedin"
            stateLabel: Strings.t("dash.ac")
            profileName: {
                var p = root.profileForAc();
                return p ? p.name : Strings.t("common.none");
            }
            onClicked: {
                var p = root.profileForAc();
                if (p)
                    AppState.editProfile(p.id);
            }
        }

        UsedProfileRow {
            Layout.fillWidth: true
            compact: root.compact
            stateIcon: "batterymode"
            stateLabel: Strings.t("dash.battery")
            profileName: {
                var p = root.profileForBat();
                return p ? p.name : Strings.t("common.none");
            }
            onClicked: {
                var p = root.profileForBat();
                if (p)
                    AppState.editProfile(p.id);
            }
        }

        Item { Layout.fillHeight: true }

        // ── Navigation secondaire ───────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.bottomMargin: Theme.s4
            Layout.preferredHeight: 1
            color: Theme.divider
        }

        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "settings"
            label: Strings.t("nav.settings")
            selected: AppState.section === "settings"
            onClicked: AppState.navigate("settings")
        }

        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "support"
            label: Strings.t("nav.support")
            selected: AppState.section === "support"
            onClicked: AppState.navigate("support")
        }

        NavItem {
            Layout.fillWidth: true
            compact: root.compact
            iconName: "info"
            label: Strings.t("nav.info")
            selected: AppState.section === "info"
            onClicked: AppState.navigate("info")
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: Theme.s4
            Layout.bottomMargin: Theme.s4
            Layout.preferredHeight: 1
            color: Theme.divider
        }

        // ── Utilitaires ─────────────────────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignHCenter
            spacing: Theme.s4

            IconButton {
                id: languageButton
                visible: !root.compact
                iconName: "language"
                tooltip: Strings.t("nav.language")
                onClicked: languageMenu.showAt(languageButton)
            }

            IconButton {
                id: themeButton
                visible: !root.compact
                iconName: "theme"
                tooltip: Strings.t("nav.theme")
                onClicked: root.cycleTheme()
            }

            IconButton {
                id: trayButton
                iconName: "monitor"
                tooltip: Strings.t("nav.tray")
                onClicked: trayMenu.openAt(trayButton)
            }
        }
    }

    AppContextMenu {
        id: languageMenu
        items: [
            { "label": "Français", "checked": AppState.language === "fr" },
            { "label": "English", "checked": AppState.language === "en" },
            { "label": "Deutsch", "checked": AppState.language === "de" }
        ]
        onTriggered: function(index) {
            var codes = ["fr", "en", "de"];
            AppState.setLanguage(codes[index]);
        }
    }

    TrayMenu {
        id: trayMenu

        function openAt(sourceItem) {
            var p = sourceItem.mapToItem(null, 0, 0);
            trayMenu.x = Math.max(Theme.s8, p.x + sourceItem.width + Theme.s8);
            trayMenu.y = Math.max(Theme.s8, trayMenu.parent.height - trayMenu.implicitHeight - Theme.s12);
            trayMenu.open();
        }
    }

    function cycleTheme() {
        var order = ["system", "light", "dark"];
        var next = order[(order.indexOf(AppState.themeMode) + 1) % order.length];
        AppState.setThemeMode(next);
        var key = next === "dark" ? "theme.dark" : next === "light" ? "theme.light" : "theme.system";
        AppState.notify(Strings.t("toast.theme") + " · " + Strings.t(key), "info");
    }
}
