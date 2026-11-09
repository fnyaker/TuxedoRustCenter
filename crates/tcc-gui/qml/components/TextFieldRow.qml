// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Ligne de saisie : libellé + description à gauche, champ à droite.
// `text` est piloté par le formulaire ; les frappes remontent via `edited`.
// Les Binding réappliquent la valeur du modèle après un reset, sans casser la
// saisie en cours.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "../core"

Item {
    id: root

    property string label: ""
    property string description: ""
    property string text: ""
    property string placeholder: ""
    property bool password: false
    property bool invalid: false
    property string errorText: ""
    property int inputMethodHints: Qt.ImhNone
    property int fieldWidth: 320
    property bool multiline: false

    signal edited(string text)

    implicitHeight: multiline ? Math.max(row.implicitHeight, 100) : Math.max(row.implicitHeight, 44)

    RowLayout {
        id: row
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: root.multiline ? Theme.s4 : 0
        spacing: Theme.s16

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop
            spacing: 2

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                text: root.label
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.description !== ""
                variant: "small"
                secondary: true
                text: root.description
            }

            AppLabel {
                Layout.fillWidth: true
                visible: root.invalid && root.errorText !== ""
                variant: "small"
                text: root.errorText
                color: Theme.error
            }
        }

        AppTextField {
            id: singleField
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: root.fieldWidth
            visible: !root.multiline
            placeholderText: root.placeholder
            echoMode: root.password ? TextInput.Password : TextInput.Normal
            inputMethodHints: root.inputMethodHints
            invalid: root.invalid
            errorText: root.errorText
            Accessible.name: root.label
            onTextEdited: root.edited(text)
        }

        AppTextArea {
            id: multiField
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: root.fieldWidth
            visible: root.multiline
            placeholderText: root.placeholder
            invalid: root.invalid
            errorText: root.errorText
            Accessible.name: root.label
            onTextChanged: if (activeFocus) root.edited(text)
        }
    }

    Binding {
        target: singleField
        property: "text"
        value: root.text
        when: !root.multiline
    }

    Binding {
        target: multiField
        property: "text"
        value: root.text
        when: root.multiline
    }
}
