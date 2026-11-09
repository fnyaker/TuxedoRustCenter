// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Support : uniquement les canaux réellement disponibles (contact et site
// officiels). Aucun faux ticket, aucun outil d'installation simulé, aucune
// carte « à venir ». Les liens fixes passent par `Qt.openUrlExternally`,
// jamais par un shell.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    /// URL fixe du support TUXEDO (aucune valeur fournie par l'utilisateur).
    readonly property string supportUrl: "https://www.tuxedocomputers.com/support"

    function openSupport() {
        if (!Qt.openUrlExternally(page.supportUrl))
            AppState.notify(Strings.t("support.linkFailed"), "error");
    }

    contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        title: Strings.t("support.title")
        iconName: "support"

        AppButton {
            objectName: "supportOnlineButton"
            text: Strings.t("support.contact.online")
            variant: "secondary"
            iconName: "online"
            onClicked: page.openSupport()
        }
    }

    AppCard {
        objectName: "supportContactCard"
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("support.contact.title")
            hint: Strings.t("support.contact.desc")
            iconName: "support"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("support.contact.phone")
            value: "+49 4192 99117 0"
            iconName: "support"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("support.contact.online")
            value: "www.tuxedocomputers.com/support"
            iconName: "online"
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8

            Item { Layout.fillWidth: true }

            AppButton {
                objectName: "supportOpenSiteButton"
                text: Strings.t("support.contact.openSite")
                iconName: "online"
                onClicked: page.openSupport()
            }
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("support.contact.privacy")
        }
    }
}
