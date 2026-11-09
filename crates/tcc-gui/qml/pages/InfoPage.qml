// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// À propos : versions réelles, journal des modifications statique, licences.
// Aucune valeur produit de `DemoData` n'est utilisée ici : si le daemon ou le
// matériel n'expose pas une information, l'UI affiche « non disponible ».

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    /// Changelog livré avec l'application, marqué explicitement « statique ».
    readonly property var changelog: [
        { "version": "0.1.0", "date": "2026-11-06",
          "notes": "Design system Qt 6, dashboard, profils, réglages persistants, Admin.Hw et capacités matérielles réelles." },
        { "version": "0.0.1", "date": "2026-10-21",
          "notes": "Socle du workspace Rust, contrats IPC et analyse de l'application d'origine." }
    ]

    function unavailable() { return Strings.t("common.unavailable") }

    function textOr(value) {
        return typeof value === "string" && value !== "" ? value : page.unavailable();
    }

    readonly property string appVersion: Qt.application.version !== ""
                                         ? Qt.application.version : page.unavailable()
    readonly property string daemonVersion: LiveData.isLive
                                            && LiveData.daemonVersion !== ""
                                            ? LiveData.daemonVersion : page.unavailable()
    readonly property string contractVersion: LiveData.contractVersion > 0
                                              ? "v" + LiveData.contractVersion
                                              : page.unavailable()

    contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        title: Strings.t("info.title")
        subtitle: Strings.t("info.subtitle")
        iconName: "info"

        StatusBadge {
            kind: LiveData.isLive ? "ok" : "warn"
            label: LiveData.isLive ? Strings.t("app.live") : Strings.t("app.demo")
        }
    }

    AppCard {
        Layout.fillWidth: true

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s20

            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 148
                Layout.preferredHeight: 76
                radius: Theme.radiusMd
                color: "#ffffff"
                border.width: 1
                border.color: Theme.border

                AppIcon {
                    anchors.centerIn: parent
                    name: "brand/tcc_logo"
                    monochrome: false
                    size: 112
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2

                AppLabel {
                    variant: "h2"
                    text: Strings.t("app.name")
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    secondary: true
                    text: Strings.t("info.about")
                    wrapMode: Text.WordWrap
                }

                AppLabel {
                    Layout.topMargin: Theme.s4
                    variant: "caption"
                    secondary: true
                    text: Strings.t("info.realSources")
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.divider }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.version")
            value: page.appVersion
            iconName: "info"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.daemon")
            value: page.daemonVersion
            iconName: "gear"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.ipc")
            value: page.contractVersion
            iconName: "gear"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.kernel")
            value: page.textOr(HardwareData.device.kernelRelease)
            iconName: "gear"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.architecture")
            value: page.textOr(HardwareData.device.architecture)
            iconName: "cpu"
        }
    }

    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("info.device")
            hint: Strings.t("info.deviceHint")
            iconName: "monitor"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.vendor")
            value: page.textOr(HardwareData.device.vendor)
            iconName: "imprint"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.model")
            value: page.textOr(HardwareData.device.productName)
            iconName: "monitor"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.modelCode")
            value: page.textOr(HardwareData.device.model)
            iconName: "gear"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.sku")
            value: page.textOr(HardwareData.device.sku)
            iconName: "gear"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("info.driver")
            value: page.textOr(HardwareData.device.tuxedoDriverVersion)
            iconName: "gear"
        }

        AppLabel {
            Layout.fillWidth: true
            visible: LiveData.isLive && !HardwareData.device.isTuxedo
            variant: "caption"
            color: Theme.warning
            wrapMode: Text.WordWrap
            text: Strings.t("info.notTuxedo")
        }
    }

    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("info.changelog")
            hint: Strings.t("info.changelogHint")
            iconName: "logbook"

            StatusBadge {
                objectName: "infoStaticBadge"
                kind: "neutral"
                label: Strings.t("info.static")
            }
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("info.changelogStatic")
        }

        Repeater {
            model: page.changelog

            delegate: RowLayout {
                id: changelogRow
                required property var modelData
                Layout.fillWidth: true
                spacing: Theme.s16

                ColumnLayout {
                    Layout.preferredWidth: 110
                    spacing: 0

                    AppLabel {
                        variant: "title"
                        text: changelogRow.modelData.version
                    }

                    AppLabel {
                        variant: "caption"
                        secondary: true
                        text: changelogRow.modelData.date
                    }
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "small"
                    secondary: true
                    wrapMode: Text.WordWrap
                    text: changelogRow.modelData.notes
                }
            }
        }
    }

    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("info.licenses")
            hint: Strings.t("info.licensesHint")
            iconName: "imprint"
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("info.original")
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("info.icons")
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("info.font")
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "small"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("info.qtLicense")
        }
    }
}
