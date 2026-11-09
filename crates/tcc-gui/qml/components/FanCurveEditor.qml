// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Éditeur de courbe de ventilation CPU (points température → vitesse).
// Aucune écriture ici : le dialogue produit une liste de points validée ;
// `ProfileEditorPage` l'intègre au brouillon et le DTO Rust la valide à nouveau.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../core"

Popup {
    id: editor

    parent: Overlay.overlay
    modal: true
    dim: true
    width: 620
    padding: 0
    closePolicy: Popup.CloseOnEscape
    Overlay.modal: Rectangle { color: Theme.overlay }

    readonly property int maxPoints: 256
    /// Points numériques valides pour l'aperçu.
    property var previewPoints: []
    /// Index de ligne source de chaque point de `previewPoints` : permet à un
    /// drag/ajout/suppression sur l'aperçu de mettre à jour la bonne ligne.
    property var previewRows: []
    /// Clé i18n de l'erreur courante (`""` si valide).
    property string errorKey: ""

    signal applied(var points)

    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)

    function parseNumber(value) {
        var text = String(value === undefined || value === null ? "" : value).trim();
        text = text.replace(",", ".");
        var number = Number(text);
        return isFinite(number) ? number : NaN;
    }

    function formatNumber(value) {
        var number = Number(value);
        return isFinite(number) ? String(number) : "";
    }

    /// Charge la courbe et ouvre le dialogue.
    function openWith(points) {
        rows.clear();
        var source = Array.isArray(points) ? points : [];
        for (var i = 0; i < source.length && rows.count < editor.maxPoints; ++i) {
            rows.append({
                "temp": editor.formatNumber(source[i].temp),
                "speed": editor.formatNumber(source[i].speed)
            });
        }
        if (rows.count < 2) {
            rows.clear();
            rows.append({ "temp": "30", "speed": "20" });
            rows.append({ "temp": "80", "speed": "80" });
        }
        editor.recompute();
        editor.open();
    }

    function addPoint() {
        if (rows.count >= editor.maxPoints)
            return;
        var last = rows.count > 0 ? rows.get(rows.count - 1) : { "temp": "30", "speed": "20" };
        var temp = editor.parseNumber(last.temp);
        var speed = editor.parseNumber(last.speed);
        var nextTemp = isFinite(temp) ? Math.min(100, Math.max(0, temp + 10)) : 30;
        var nextSpeed = isFinite(speed) ? Math.min(100, Math.max(0, speed)) : 20;
        rows.append({
            "temp": editor.formatNumber(nextTemp),
            "speed": editor.formatNumber(nextSpeed)
        });
        editor.recompute();
    }

    function removePoint(index) {
        if (rows.count <= 2 || index < 0 || index >= rows.count)
            return;
        rows.remove(index);
        editor.recompute();
    }

    function recompute() {
        var points = [];
        var indexes = [];
        var error = "";
        if (rows.count < 2) {
            error = "fanCurve.error.min";
        } else if (rows.count > editor.maxPoints) {
            error = "fanCurve.error.max";
        }
        var previous = -Infinity;
        for (var i = 0; i < rows.count; ++i) {
            var row = rows.get(i);
            var temp = editor.parseNumber(row.temp);
            var speed = editor.parseNumber(row.speed);
            if (isNaN(temp) || isNaN(speed)) {
                if (error === "")
                    error = "fanCurve.error.number";
                continue;
            }
            if (temp < 0 || temp > 100 || speed < 0 || speed > 100) {
                if (error === "")
                    error = "fanCurve.error.range";
                continue;
            }
            if (temp <= previous) {
                if (error === "")
                    error = "fanCurve.error.order";
            }
            previous = temp;
            points.push({ "temp": temp, "speed": speed });
            indexes.push(i);
        }
        editor.previewPoints = points;
        editor.previewRows = indexes;
        editor.errorKey = error;
    }

    /// Met à jour la ligne correspondant au point déplacé dans l'aperçu.
    function movePreviewPoint(index, temp, speed) {
        if (index < 0 || index >= editor.previewRows.length)
            return;
        var rowIndex = editor.previewRows[index];
        rows.setProperty(rowIndex, "temp", editor.formatNumber(temp));
        rows.setProperty(rowIndex, "speed", editor.formatNumber(speed));
        editor.recompute();
    }

    /// Insère un point créé au double-clic, en conservant l'ordre strict.
    function addPreviewPoint(temp, speed) {
        if (rows.count >= editor.maxPoints)
            return;
        var newTemp = Number(temp);
        var newSpeed = Number(speed);
        if (!isFinite(newTemp) || !isFinite(newSpeed))
            return;
        if (newTemp < 0 || newTemp > 100 || newSpeed < 0 || newSpeed > 100)
            return;

        var insertAt = 0;
        while (insertAt < rows.count) {
            var rowTemp = editor.parseNumber(rows.get(insertAt).temp);
            if (!isFinite(rowTemp) || newTemp < rowTemp)
                break;
            ++insertAt;
        }
        var previous = insertAt > 0 ? editor.parseNumber(rows.get(insertAt - 1).temp)
                                    : -Infinity;
        var next = insertAt < rows.count ? editor.parseNumber(rows.get(insertAt).temp)
                                         : Infinity;
        if (isFinite(previous) && newTemp - previous < 1)
            return;
        if (isFinite(next) && next - newTemp < 1)
            return;

        rows.insert(insertAt, {
            "temp": editor.formatNumber(newTemp),
            "speed": editor.formatNumber(newSpeed)
        });
        editor.recompute();
    }

    /// Supprime le point de l'aperçu en respectant le minimum de deux points.
    function removePreviewPoint(index) {
        if (index < 0 || index >= editor.previewRows.length || rows.count <= 2)
            return;
        editor.removePoint(editor.previewRows[index]);
    }

    function apply() {
        if (editor.errorKey !== "")
            return;
        var points = [];
        for (var i = 0; i < rows.count; ++i) {
            var row = rows.get(i);
            points.push({
                "temp": editor.parseNumber(row.temp),
                "speed": editor.parseNumber(row.speed)
            });
        }
        editor.applied(points);
        editor.close();
    }

    background: Rectangle {
        radius: Theme.radiusLg
        color: Theme.surface
        border.width: 1
        border.color: Theme.borderStrong
    }

    contentItem: ColumnLayout {
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.s24
            spacing: Theme.s10

            RowLayout {
                spacing: Theme.s10

                Rectangle {
                    Layout.preferredWidth: 32
                    Layout.preferredHeight: 32
                    radius: Theme.radiusSm
                    color: Theme.accentSoft

                    AppIcon {
                        anchors.centerIn: parent
                        name: "fan"
                        color: Theme.accentText
                        size: 16
                    }
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "h3"
                    text: Strings.t("fanCurve.title")
                }
            }

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: Strings.t("fanCurve.hint")
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s8

                AppLabel {
                    Layout.fillWidth: true
                    variant: "caption"
                    secondary: true
                    text: Strings.t("fanCurve.temp")
                }
                AppLabel {
                    Layout.fillWidth: true
                    variant: "caption"
                    secondary: true
                    text: Strings.t("fanCurve.speed")
                }
                Item { Layout.preferredWidth: Theme.hitTarget }
            }

            ListView {
                id: rowList
                Layout.fillWidth: true
                Layout.preferredHeight: 260
                clip: true
                spacing: Theme.s4
                model: ListModel { id: rows }
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

                delegate: RowLayout {
                    id: curveRow
                    required property int index
                    required property string temp
                    required property string speed

                    width: rowList.width
                    spacing: Theme.s8

                    AppTextField {
                        Layout.fillWidth: true
                        text: curveRow.temp
                        horizontalAlignment: TextInput.AlignRight
                        inputMethodHints: Qt.ImhFormattedNumbersOnly
                        validator: DoubleValidator {
                            bottom: 0
                            top: 100
                            decimals: 1
                            notation: DoubleValidator.StandardNotation
                        }
                        Accessible.name: Strings.tf("fanCurve.tempPoint", [curveRow.index + 1])
                        onTextEdited: {
                            rows.setProperty(curveRow.index, "temp", text);
                            editor.recompute();
                        }
                    }

                    AppTextField {
                        Layout.fillWidth: true
                        text: curveRow.speed
                        horizontalAlignment: TextInput.AlignRight
                        inputMethodHints: Qt.ImhFormattedNumbersOnly
                        validator: DoubleValidator {
                            bottom: 0
                            top: 100
                            decimals: 1
                            notation: DoubleValidator.StandardNotation
                        }
                        Accessible.name: Strings.tf("fanCurve.speedPoint", [curveRow.index + 1])
                        onTextEdited: {
                            rows.setProperty(curveRow.index, "speed", text);
                            editor.recompute();
                        }
                    }

                    IconButton {
                        iconName: "trash"
                        tooltip: Strings.t("fanCurve.remove")
                        enabled: rows.count > 2
                        onClicked: editor.removePoint(curveRow.index)
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s8

                AppButton {
                    variant: "secondary"
                    small: true
                    text: Strings.t("fanCurve.add")
                    enabled: rows.count < editor.maxPoints
                    onClicked: editor.addPoint()
                }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "caption"
                    secondary: true
                    text: Strings.tf("fanCurve.count", [rows.count, editor.maxPoints])
                }
            }

            FanCurveChart {
                id: previewChart
                objectName: "fanCurvePreview"
                Layout.fillWidth: true
                Layout.preferredHeight: 190
                points: editor.previewPoints
                editable: editor.errorKey === ""
                maxPoints: editor.maxPoints
                opacity: editor.errorKey === "" ? 1 : 0.55
                onPointMoved: function(index, temp, speed) {
                    editor.movePreviewPoint(index, temp, speed);
                }
                onPointAdded: function(temp, speed) {
                    editor.addPreviewPoint(temp, speed);
                }
                onPointRemoved: function(index) {
                    editor.removePreviewPoint(index);
                }
            }

            AppLabel {
                Layout.fillWidth: true
                visible: editor.errorKey === ""
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: Strings.t("fanCurve.dragHint")
            }

            AppLabel {
                Layout.fillWidth: true
                visible: editor.errorKey !== ""
                variant: "small"
                color: Theme.error
                wrapMode: Text.WordWrap
                text: Strings.t(editor.errorKey)
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.divider }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.s16
            spacing: Theme.s8

            Item { Layout.fillWidth: true }

            AppButton {
                text: Strings.t("common.cancel")
                variant: "secondary"
                onClicked: editor.close()
            }

            AppButton {
                objectName: "fanCurveApply"
                text: Strings.t("common.apply")
                enabled: editor.errorKey === ""
                onClicked: editor.apply()
            }
        }
    }
}
