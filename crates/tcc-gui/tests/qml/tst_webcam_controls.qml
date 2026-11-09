// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Contrôles UVC (Outils) : lecture seule complète quand l'agent n'expose pas
// l'écriture, éditeurs bornés sinon (entier au relâchement, booléen et menu
// immédiats), erreurs inline sans valeur optimiste. Aucun aperçu vidéo.

import QtQuick
import QtQuick.Window
import QtTest
import "../../qml/core"
import "../../qml/pages"

TestCase {
    id: testCase
    name: "WebcamControls"
    when: windowShown

    /// Fenêtre dédiée : `mousePress`/`mouseRelease` exigent un `Window`
    /// (un `TestCase` nu n'en fournit pas).
    Window {
        id: testWindow
        width: 1000
        height: 900
        visible: true
    }

    Component { id: toolsComponent; ToolsPage { } }

    SignalSpy {
        id: uvcSpy
        target: AppState
        signalName: "uvcControlRequested"
    }

    function writableFlags() {
        return {
            "inactive": false,
            "readOnly": false,
            "writeOnly": false,
            "update": false,
            "disabled": false,
            "grabbed": false
        };
    }

    function payload(webcam) {
        return JSON.stringify({
            "sensors": {},
            "charging": {},
            "webcam": webcam
        });
    }

    function setWebcam(webcam) {
        HardwareData.connected = true;
        HardwareData.hardwareError = "";
        HardwareData.hardwareJson = payload(webcam);
    }

    function camera(controls, supported, metadata) {
        return {
            "switchAvailable": true,
            "enabled": true,
            "controlsSupported": supported === undefined ? true : supported,
            "controlsTruncated": false,
            "errors": [],
            "devices": [{
                "name": "Integrated Camera",
                "devicePath": metadata === true ? "" : "/dev/video0",
                "metadata": metadata === true,
                "controlsError": "",
                "controls": controls
            }]
        };
    }

    function integerControl(key, value, flags) {
        return {
            "key": key,
            "name": key === "brightness" ? "Brightness" : key,
            "kind": "integer",
            "min": 0, "max": 255, "step": 1, "default": 128,
            "current": { "kind": "integer", "value": value },
            "menu": [],
            "flags": flags === undefined ? writableFlags() : flags,
            "category": "basic",
            "simple": true
        };
    }

    function booleanControl(key, value, flags) {
        return {
            "key": key,
            "name": "Focus, Auto",
            "kind": "boolean",
            "min": 0, "max": 1, "step": 1, "default": 0,
            "current": { "kind": "boolean", "value": value ? 1 : 0,
                         "boolean": value === true },
            "menu": [],
            "flags": flags === undefined ? writableFlags() : flags,
            "category": "focus",
            "simple": true
        };
    }

    function menuControl(key, index, flags) {
        return {
            "key": key,
            "name": "Exposure, Auto",
            "kind": "menu",
            "min": 0, "max": 1, "step": 1, "default": 0,
            "current": { "kind": "menu", "index": index,
                         "key": index === 1 ? "manual" : "auto",
                         "label": index === 1 ? "Manual Mode" : "Auto" },
            "menu": [
                { "index": 0, "key": "auto", "label": "Auto" },
                { "index": 1, "key": "manual", "label": "Manual Mode" }
            ],
            "flags": flags === undefined ? writableFlags() : flags,
            "category": "exposure",
            "simple": false
        };
    }

    function readOnlyFlags() {
        var flags = writableFlags();
        flags.readOnly = true;
        return flags;
    }

    function setUvcWritable(value) {
        AgentData.uvcWriteSupported = value;
        AgentData.uvcPending = false;
        AgentData.uvcError = "";
        AgentData.uvcSucceeded = false;
    }

    function reset() {
        HardwareData.connected = false;
        HardwareData.hardwareJson = "{}";
        HardwareData.hardwareError = "";
        setUvcWritable(false);
        uvcSpy.clear();
    }

    function init() { reset(); }
    function cleanup() { reset(); }

    function pageWith(webcam) {
        setWebcam(webcam);
        var page = createTemporaryObject(toolsComponent, testWindow.contentItem);
        verify(page !== null);
        page.width = testWindow.width;
        page.height = testWindow.height;
        return page;
    }

    function test_readOnlyWhenAgentDoesNotExposeWriting() {
        var page = pageWith(camera([
            integerControl("brightness", 100),
            booleanControl("focus_auto", false),
            menuControl("exposure_auto", 1)
        ]));
        compare(page.uvcWriteSupported, false);
        compare(page.webcamVisible, true);

        var brightness = findChild(page, "uvcValue_brightness");
        verify(brightness !== null);
        compare(brightness.value, Strings.formatNumber(100, 0));
        compare(brightness.badgeLabel, Strings.t("common.readOnly"));

        verify(findChild(page, "uvcSlider_brightness") === null);
        verify(findChild(page, "uvcSwitch_focus_auto") === null);
        verify(findChild(page, "uvcMenu_exposure_auto") === null);
        // Aucun aperçu.
        compare(typeof page.webcamPreview, "undefined");
    }

    function test_integerControlCommitsOnReleaseWithoutDragSpam() {
        setUvcWritable(true);
        var page = pageWith(camera([integerControl("brightness", 100)]));

        var editor = findChild(page, "uvcControl_brightness");
        verify(editor !== null);
        var slider = findChild(page, "uvcSlider_brightness");
        verify(slider !== null);
        compare(slider.from, 0);
        compare(slider.to, 255);
        compare(slider.stepSize, 1);
        verify(slider.enabled);

        // Le contrôle peut être sous la ligne de flottaison : on l'amène à
        // l'écran avant les événements souris.
        page.scrollToItem(editor);
        waitForRendering(page);

        mousePress(slider, slider.width * 0.30, slider.height / 2);
        mouseMove(slider, slider.width * 0.70, slider.height / 2);
        // Aucun commit pendant le déplacement : le curseur ne parle qu'au
        // relâchement.
        compare(uvcSpy.count, 0);
        mouseRelease(slider, slider.width * 0.70, slider.height / 2);

        compare(uvcSpy.count, 1);
        compare(uvcSpy.signalArguments[0][0], "/dev/video0");
        compare(uvcSpy.signalArguments[0][1], "brightness");
        verify(uvcSpy.signalArguments[0][2] > 100);
        verify(uvcSpy.signalArguments[0][2] <= 255);
        verify(editor.draft === uvcSpy.signalArguments[0][2]);
    }

    function test_failureShowsInlineErrorAndRestoresSnapshot() {
        setUvcWritable(true);
        var page = pageWith(camera([integerControl("brightness", 100)]));

        var editor = findChild(page, "uvcControl_brightness");
        var slider = findChild(page, "uvcSlider_brightness");
        verify(editor !== null && slider !== null);

        slider.value = 150;
        editor.draft = 150;
        editor.commitDraft();
        compare(uvcSpy.count, 1);

        // Refus : pending passe à faux avec un code stable.
        AgentData.uvcPending = true;
        AgentData.uvcError = "Busy";
        AgentData.uvcPending = false;

        var error = findChild(page, "uvcError_brightness");
        verify(error !== null);
        verify(editor.failed);
        compare(error.text, Strings.t("tools.webcam.uvcBusy"));
        compare(slider.value, 100);
        compare(editor.draft, 100);
    }

    function test_idempotentResultIsNotAnError() {
        setUvcWritable(true);
        var page = pageWith(camera([integerControl("brightness", 100)]));

        var editor = findChild(page, "uvcControl_brightness");
        var slider = findChild(page, "uvcSlider_brightness");
        slider.value = 120;
        editor.draft = 120;
        editor.commitDraft();
        compare(uvcSpy.count, 1);

        // `false` côté D-Bus = valeur déjà courante : succès sans erreur.
        AgentData.uvcPending = true;
        AgentData.uvcError = "";
        AgentData.uvcSucceeded = true;
        AgentData.uvcPending = false;

        var error = findChild(page, "uvcError_brightness");
        verify(error !== null);
        compare(editor.failed, false);
        compare(AgentData.uvcSucceeded, true);
    }

    function test_booleanControlTogglesImmediately() {
        setUvcWritable(true);
        var page = pageWith(camera([booleanControl("focus_auto", false)]));

        var control = findChild(page, "uvcSwitch_focus_auto");
        verify(control !== null);
        compare(control.checked, false);
        control.toggle();

        compare(uvcSpy.count, 1);
        compare(uvcSpy.signalArguments[0][0], "/dev/video0");
        compare(uvcSpy.signalArguments[0][1], "focus_auto");
        compare(uvcSpy.signalArguments[0][2], 1);

        // Échec : l'interrupteur revient à la valeur du snapshot.
        AgentData.uvcPending = true;
        AgentData.uvcError = "Busy";
        AgentData.uvcPending = false;
        compare(control.checked, false);
    }

    function test_menuControlUsesUvcIndexes() {
        setUvcWritable(true);
        var page = pageWith(camera([menuControl("exposure_auto", 1)]));

        var combo = findChild(page, "uvcMenu_exposure_auto");
        verify(combo !== null);
        compare(combo.count, 2);
        compare(combo.currentIndex, 1);
        compare(combo.currentText, "Manual Mode");

        var editor = findChild(page, "uvcControl_exposure_auto");
        verify(editor !== null);
        editor.chooseMenu(0);

        compare(uvcSpy.count, 1);
        compare(uvcSpy.signalArguments[0][1], "exposure_auto");
        // La valeur transmise est l'index UVC, pas la position du modèle.
        compare(uvcSpy.signalArguments[0][2], 0);
    }

    function test_readOnlyOrDisabledFlagsHaveNoEditor() {
        setUvcWritable(true);
        var disabled = writableFlags();
        disabled.disabled = true;
        var grabbed = writableFlags();
        grabbed.grabbed = true;
        var inactive = writableFlags();
        inactive.inactive = true;
        var writeOnly = writableFlags();
        writeOnly.writeOnly = true;

        var page = pageWith(camera([
            integerControl("brightness", 100, readOnlyFlags()),
            booleanControl("focus_auto", false, disabled),
            menuControl("exposure_auto", 1, grabbed),
            menuControl("white_balance_auto", 0, inactive),
            integerControl("contrast", 10, writeOnly)
        ]));

        verify(findChild(page, "uvcSlider_brightness") === null);
        verify(findChild(page, "uvcSwitch_focus_auto") === null);
        verify(findChild(page, "uvcMenu_exposure_auto") === null);
        verify(findChild(page, "uvcMenu_white_balance_auto") === null);
        verify(findChild(page, "uvcSlider_contrast") === null);
        verify(findChild(page, "uvcValue_brightness") !== null);
        verify(findChild(page, "uvcValue_contrast") !== null);
    }

    function test_pendingDisablesEditors() {
        setUvcWritable(true);
        var page = pageWith(camera([
            integerControl("brightness", 100),
            booleanControl("focus_auto", false),
            menuControl("exposure_auto", 1)
        ]));
        AgentData.uvcPending = true;

        compare(findChild(page, "uvcSlider_brightness").enabled, false);
        compare(findChild(page, "uvcSwitch_focus_auto").enabled, false);
        compare(findChild(page, "uvcMenu_exposure_auto").enabled, false);
    }

    function test_metadataDeviceIsNeverWritable() {
        setUvcWritable(true);
        var page = pageWith(camera([integerControl("brightness", 100)], true, true));
        // Le nœud metadata n'a pas de chemin de capture : jamais d'écriture.
        verify(findChild(page, "uvcSlider_brightness") === null);
        verify(findChild(page, "uvcValue_brightness") !== null);
    }

    function test_cameraWithoutControlsShowsInfo() {
        var page = pageWith(camera([], false));
        compare(page.webcamVisible, true);
        compare(page.webcamNoControls, true);
        verify(findChild(page, "uvcControl_brightness") === null);
    }

    function test_noCameraHidesCard() {
        var page = pageWith({ "switchAvailable": false, "devices": [] });
        compare(page.webcamVisible, false);
        compare(page.webcamNoControls, false);
    }

    function test_controlsErrorIsExposed() {
        var webcam = camera([], false);
        webcam.devices[0].controlsError = "EACCES";
        var page = pageWith(webcam);
        compare(page.webcamControlsError, "EACCES");
    }
}
