// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// États dégradés de l'interface : chargement, vide et erreur de connexion.
// Ces tests ne dépendent ni du daemon ni d'un compositeur.
//
//   /usr/lib/qt6/bin/qmltestrunner -input crates/tcc-gui/tests/qml

import QtQuick
import QtTest
import "../../qml/core"
import "../../qml/components"

TestCase {
    id: testCase
    name: "UiStates"
    when: windowShown

    Component {
        id: loadingComponent
        LoadingState { label: "Chargement"; size: 24 }
    }

    Component {
        id: emptyComponent
        Item {
            width: 400
            height: 300
            property alias emptyState: emptyStateItem
            property alias actionSpy: actionSpy

            EmptyState {
                id: emptyStateItem
                anchors.fill: parent
                title: "Aucun profil"
                body: "Créez un profil pour commencer."
                actionText: "Nouveau profil"
            }

            SignalSpy {
                id: actionSpy
                target: emptyStateItem
                signalName: "actionClicked"
            }
        }
    }

    Component {
        id: bannerComponent
        Item {
            width: 600
            height: 48
            property alias banner: banner
            property alias retrySpy: retrySpy

            ConnectionBanner {
                id: banner
                anchors.fill: parent
                detail: "Bus système indisponible"
            }

            SignalSpy {
                id: retrySpy
                target: banner
                signalName: "retryRequested"
            }
        }
    }

    function test_loadingStateExposesLabelAndAccessibility() {
        var loading = createTemporaryObject(loadingComponent, testCase);
        verify(loading !== null);
        compare(loading.label, "Chargement");
        compare(loading.size, 24);
        compare(loading.Accessible.name, "Chargement");

        var unlabelled = createTemporaryObject(loadingComponent, testCase, { "label": "" });
        compare(unlabelled.Accessible.name, Strings.t("common.loading"));
        verify(unlabelled.implicitWidth > 0);
        verify(unlabelled.implicitHeight > 0);
    }

    function test_emptyStateCarriesTitleBodyAndAction() {
        var wrapper = createTemporaryObject(emptyComponent, testCase);
        verify(wrapper !== null);
        compare(wrapper.emptyState.title, "Aucun profil");
        compare(wrapper.emptyState.body, "Créez un profil pour commencer.");
        compare(wrapper.emptyState.actionText, "Nouveau profil");

        compare(wrapper.actionSpy.count, 0);
        wrapper.emptyState.actionClicked();
        compare(wrapper.actionSpy.count, 1);
    }

    function test_connectionBannerDefaultsAndRetrySignal() {
        var wrapper = createTemporaryObject(bannerComponent, testCase);
        verify(wrapper !== null);
        compare(wrapper.banner.title, Strings.t("connection.title"));
        compare(wrapper.banner.detail, "Bus système indisponible");

        compare(wrapper.retrySpy.count, 0);
        wrapper.banner.retryRequested();
        compare(wrapper.retrySpy.count, 1);
    }
}
