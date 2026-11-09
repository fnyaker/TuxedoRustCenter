// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Outils : minuterie d'extinction, éclairage clavier, webcam UVC.
// LUKS est hors périmètre (décision utilisateur) : aucun formulaire, aucun
// secret dans la GUI. Les opérations matérielles passent par le daemon.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    // ── Minuterie d'extinction ──────────────────────────────────────────────
    // Brouillon local ; l'écriture passe par `Settings.shutdownTime` et
    // `Admin1.SaveSettingsJSON`. L'état réel vient de
    // `Manager1.GetShutdownTimerJSON` (`ShutdownTimerData`).
    property bool timerEnabled: false
    property int timerHour: 22
    property int timerMinute: 0
    property bool timerInitialEnabled: false
    property int timerInitialHour: 22
    property int timerInitialMinute: 0
    property bool timerSubmitting: false
    /// Vrai quand la dernière demande visait une annulation (toast).
    property bool timerClearing: false
    /// État d'erreur exposé par le daemon (`source=error` ou erreur logind).
    readonly property bool timerStatusError: ShutdownTimerData.errorState
    // Visibilité matérielle (snapshot sûr uniquement).
    readonly property bool lightingVisible: HardwareData.visibleIfAvailable(
        HardwareData.lighting.keyboard.available
        || HardwareData.lighting.lightbar.available)
    readonly property bool webcamVisible: HardwareData.visibleIfAvailable(
        HardwareData.webcam.switchAvailable
        || HardwareData.webcam.devices.length > 0)
    readonly property bool webcamNoControls: HardwareData.webcam.devices.length > 0
                                             && !HardwareData.webcam.controlsSupported
    readonly property string webcamControlsError: HardwareData.webcam.controlsError
    /// Écriture UVC indisponible : les contrôles restent entièrement lisibles.
    readonly property bool uvcWriteSupported: AgentData.uvcWriteSupported
    readonly property bool uvcPending: AgentData.uvcPending
    /// Dernier contrôle visé par une écriture (erreur inline ciblée).
    property string uvcLastDevice: ""
    property string uvcLastKey: ""
    /// Incrémenté quand une écriture échoue : les brouillons reprennent la
    /// valeur relue du snapshot (aucun état optimiste).
    property int uvcResetToken: 0

    // ── Éclairage : brouillon local, écriture par `Settings` ───────────────
    // Les valeurs initiales viennent de l'état réellement appliqué par le
    // daemon (`HardwareData.lighting.states`) ; aucune valeur n'est inventée.
    property bool lightingSubmitting: false
    property string lightingError: ""
    property real keyboardBrightness: 0
    property real keyboardRed: 0
    property real keyboardGreen: 0
    property real keyboardBlue: 0
    property string keyboardMode: "static"
    property real keyboardLastBrightness: 0
    property real keyboardInitialBrightness: 0
    property real keyboardInitialRed: 0
    property real keyboardInitialGreen: 0
    property real keyboardInitialBlue: 0
    property string keyboardInitialMode: "static"
    property real lightbarBrightness: 0
    property real lightbarRed: 0
    property real lightbarGreen: 0
    property real lightbarBlue: 0
    property real lightbarLastBrightness: 0
    property real lightbarInitialBrightness: 0
    property real lightbarInitialRed: 0
    property real lightbarInitialGreen: 0
    property real lightbarInitialBlue: 0

    readonly property bool keyboardAvailable: HardwareData.lighting.keyboard.available
    readonly property bool lightbarAvailable: HardwareData.lighting.lightbar.available
    readonly property bool keyboardRgb: HardwareData.lighting.keyboard.kind === "rgb"
            || HardwareData.lighting.keyboard.kind === "per-key"
    readonly property int keyboardMaxBrightness: {
        var max = HardwareData.lighting.keyboard.maxBrightness;
        return max > 0 ? Math.min(255, max) : 255;
    }
    readonly property int keyboardMaxRed: page.keyboardRgb
            ? (HardwareData.lighting.keyboard.maxRed > 0
               ? Math.min(255, HardwareData.lighting.keyboard.maxRed) : 255) : 0
    readonly property int keyboardMaxGreen: page.keyboardRgb
            ? (HardwareData.lighting.keyboard.maxGreen > 0
               ? Math.min(255, HardwareData.lighting.keyboard.maxGreen) : 255) : 0
    readonly property int keyboardMaxBlue: page.keyboardRgb
            ? (HardwareData.lighting.keyboard.maxBlue > 0
               ? Math.min(255, HardwareData.lighting.keyboard.maxBlue) : 255) : 0
    readonly property bool keyboardBreathing: HardwareData.lighting.keyboard.modes
                                               .indexOf("breathing") >= 0
    readonly property int lightbarMaxBrightness: {
        var max = HardwareData.lighting.lightbar.maxBrightness;
        return max > 0 ? Math.min(255, max) : 255;
    }
    readonly property int lightbarMaxRed: HardwareData.lighting.lightbar.maxRed > 0
            ? Math.min(255, HardwareData.lighting.lightbar.maxRed) : 255
    readonly property int lightbarMaxGreen: HardwareData.lighting.lightbar.maxGreen > 0
            ? Math.min(255, HardwareData.lighting.lightbar.maxGreen) : 255
    readonly property int lightbarMaxBlue: HardwareData.lighting.lightbar.maxBlue > 0
            ? Math.min(255, HardwareData.lighting.lightbar.maxBlue) : 255
    readonly property bool keyboardEditable: page.lightingEditable
            && HardwareData.lighting.keyboard.writable
            && HardwareData.lighting.keyboard.maxBrightness > 0
    readonly property bool lightbarEditable: page.lightingEditable
            && HardwareData.lighting.lightbar.writable
            && HardwareData.lighting.lightbar.maxBrightness > 0
    readonly property bool lightingEditable: AdminState.available
            && !AdminState.savingSettings && SettingsData.live
            && SettingsData.lightingEnabled
            && HardwareData.isLive && !HardwareData.stale

    function zoneValue(value) {
        return typeof value === "number" && isFinite(value) && value >= 0 ? value : 0;
    }

    function syncLighting() {
        var keyboardState = HardwareData.lighting.states.length > 0
                ? HardwareData.lighting.states[0] : null;
        var kbBrightness = keyboardState ? page.zoneValue(keyboardState.brightness) : 0;
        var kbRed = keyboardState ? page.zoneValue(keyboardState.red) : 0;
        var kbGreen = keyboardState ? page.zoneValue(keyboardState.green) : 0;
        var kbBlue = keyboardState ? page.zoneValue(keyboardState.blue) : 0;
        page.keyboardBrightness = kbBrightness;
        page.keyboardRed = kbRed;
        page.keyboardGreen = kbGreen;
        page.keyboardBlue = kbBlue;
        page.keyboardMode = SettingsData.keyboardMode === "breathing"
                && page.keyboardBreathing ? "breathing" : "static";
        page.keyboardLastBrightness = kbBrightness > 0 ? kbBrightness
                                                       : page.keyboardMaxBrightness;
        page.keyboardInitialBrightness = kbBrightness;
        page.keyboardInitialRed = kbRed;
        page.keyboardInitialGreen = kbGreen;
        page.keyboardInitialBlue = kbBlue;
        page.keyboardInitialMode = page.keyboardMode;

        var bar = HardwareData.lighting.lightbarState;
        var lbBrightness = bar ? page.zoneValue(bar.brightness) : 0;
        var lbRed = bar ? page.zoneValue(bar.red) : 0;
        var lbGreen = bar ? page.zoneValue(bar.green) : 0;
        var lbBlue = bar ? page.zoneValue(bar.blue) : 0;
        page.lightbarBrightness = lbBrightness;
        page.lightbarRed = lbRed;
        page.lightbarGreen = lbGreen;
        page.lightbarBlue = lbBlue;
        page.lightbarLastBrightness = lbBrightness > 0 ? lbBrightness
                                                        : page.lightbarMaxBrightness;
        page.lightbarInitialBrightness = lbBrightness;
        page.lightbarInitialRed = lbRed;
        page.lightbarInitialGreen = lbGreen;
        page.lightbarInitialBlue = lbBlue;
    }

    function lightingDirty() {
        if (page.keyboardAvailable
                && (page.keyboardBrightness !== page.keyboardInitialBrightness
                    || page.keyboardMode !== page.keyboardInitialMode
                    || (page.keyboardRgb
                        && (page.keyboardRed !== page.keyboardInitialRed
                            || page.keyboardGreen !== page.keyboardInitialGreen
                            || page.keyboardBlue !== page.keyboardInitialBlue))))
            return true;
        return page.lightbarAvailable
                && (page.lightbarBrightness !== page.lightbarInitialBrightness
                    || page.lightbarRed !== page.lightbarInitialRed
                    || page.lightbarGreen !== page.lightbarInitialGreen
                    || page.lightbarBlue !== page.lightbarInitialBlue);
    }

    function syncLightingIfClean() {
        if (!page.lightingDirty() && !page.lightingSubmitting
                && !AdminState.savingSettings)
            page.syncLighting();
    }

    function setKeyboardOn(on) {
        if (on) {
            if (page.keyboardBrightness === 0)
                page.keyboardBrightness = page.keyboardLastBrightness > 0
                        ? page.keyboardLastBrightness : page.keyboardMaxBrightness;
        } else {
            if (page.keyboardBrightness > 0)
                page.keyboardLastBrightness = page.keyboardBrightness;
            page.keyboardBrightness = 0;
        }
    }

    function setLightbarOn(on) {
        if (on) {
            if (page.lightbarBrightness === 0)
                page.lightbarBrightness = page.lightbarLastBrightness > 0
                        ? page.lightbarLastBrightness : page.lightbarMaxBrightness;
        } else {
            if (page.lightbarBrightness > 0)
                page.lightbarLastBrightness = page.lightbarBrightness;
            page.lightbarBrightness = 0;
        }
    }

    function lightingEdit() {
        var dto = {
            "fahrenheit": SettingsData.fahrenheit,
            "cpuSettingsEnabled": SettingsData.cpuEnabled,
            "fanControlEnabled": SettingsData.fanEnabled,
            "keyboardBacklightControlEnabled": SettingsData.lightingEnabled,
            "chargingType": null,
            "fnLockStatus": null
        };
        if (page.keyboardAvailable && HardwareData.lighting.keyboard.writable) {
            let zones = Math.max(1, Math.min(
                HardwareData.lighting.keyboard.zones > 0
                ? HardwareData.lighting.keyboard.zones : 1, 8));
            let states = [];
            for (var i = 0; i < zones; ++i) {
                states.push({
                    "mode": page.keyboardMode === "breathing" ? "breathing" : "static",
                    "brightness": Math.round(page.keyboardBrightness),
                    "red": page.keyboardRgb ? Math.round(page.keyboardRed) : -1,
                    "green": page.keyboardRgb ? Math.round(page.keyboardGreen) : -1,
                    "blue": page.keyboardRgb ? Math.round(page.keyboardBlue) : -1
                });
            }
            dto.keyboardStates = states;
        }
        if (page.lightbarAvailable && HardwareData.lighting.lightbar.writable) {
            dto.lightbarStates = {
                "brightness": Math.round(page.lightbarBrightness),
                "red": page.lightbarMaxRed > 0 ? Math.round(page.lightbarRed) : 0,
                "green": page.lightbarMaxGreen > 0 ? Math.round(page.lightbarGreen) : 0,
                "blue": page.lightbarMaxBlue > 0 ? Math.round(page.lightbarBlue) : 0
            };
        }
        return dto;
    }

    function submitLighting() {
        if (!page.lightingEditable || !page.lightingDirty())
            return;
        if (!page.keyboardEditable && !page.lightbarEditable)
            return;
        page.lightingError = "";
        page.lightingSubmitting = true;
        AppState.requestSettingsSave(JSON.stringify(page.lightingEdit()));
    }

    function resetLighting() {
        page.syncLighting();
        page.lightingSubmitting = false;
        page.lightingError = "";
    }

    function hourOptions() {
        var out = [];
        for (var h = 0; h < 24; ++h)
            out.push(("0" + h).slice(-2));
        return out;
    }

    function minuteOptions() {
        var out = [];
        for (var m = 0; m < 60; ++m)
            out.push(("0" + m).slice(-2));
        return out;
    }

    function timerTime() {
        return ("0" + page.timerHour).slice(-2) + ":"
                + ("0" + page.timerMinute).slice(-2);
    }

    function parseShutdownTime(value) {
        if (typeof value !== "string" || value.length !== 5 || value.charAt(2) !== ":")
            return null;
        var hours = parseInt(value.substring(0, 2), 10);
        var minutes = parseInt(value.substring(3, 5), 10);
        if (isNaN(hours) || isNaN(minutes) || hours < 0 || hours > 23
                || minutes < 0 || minutes > 59)
            return null;
        return { "hour": hours, "minute": minutes };
    }

    function syncTimer() {
        var parsed = page.parseShutdownTime(SettingsData.shutdownTime);
        var enabled = parsed !== null;
        var hour = parsed !== null ? parsed.hour : 22;
        var minute = parsed !== null ? parsed.minute : 0;
        page.timerEnabled = enabled;
        page.timerHour = hour;
        page.timerMinute = minute;
        page.timerInitialEnabled = enabled;
        page.timerInitialHour = hour;
        page.timerInitialMinute = minute;
    }

    function syncTimerIfClean() {
        if (!page.timerDirty() && !AdminState.savingSettings)
            page.syncTimer();
    }

    function timerDirty() {
        if (page.timerEnabled !== page.timerInitialEnabled)
            return true;
        return page.timerEnabled
                && (page.timerHour !== page.timerInitialHour
                    || page.timerMinute !== page.timerInitialMinute);
    }

    function timerEditable() {
        // Statut réel obligatoire : sans lui, impossible de savoir si une
        // planification existe déjà (logind peut être indisponible).
        return AdminState.available && !AdminState.savingSettings
                && SettingsData.live && ShutdownTimerData.available
                && !ShutdownTimerData.errorState;
    }

    function timerEdit(forceClear) {
        var clear = forceClear === true || !page.timerEnabled;
        return {
            "fahrenheit": SettingsData.fahrenheit,
            "cpuSettingsEnabled": SettingsData.cpuEnabled,
            "fanControlEnabled": SettingsData.fanEnabled,
            "keyboardBacklightControlEnabled": SettingsData.lightingEnabled,
            "chargingProfile": null,
            "chargingPriority": null,
            "chargingStartThreshold": null,
            "chargingEndThreshold": null,
            "stateMatrix": null,
            "shutdownTime": clear ? null : page.timerTime(),
            "clearShutdownTime": clear,
            "ppdOverride": null
        };
    }

    function submitTimer() {
        if (!page.timerEditable() || !page.timerDirty())
            return;
        page.timerClearing = !page.timerEnabled;
        page.timerSubmitting = true;
        AppState.requestSettingsSave(JSON.stringify(page.timerEdit(false)));
    }

    function cancelTimerSchedule() {
        if (!page.timerEditable())
            return;
        page.timerClearing = true;
        page.timerSubmitting = true;
        AppState.requestSettingsSave(JSON.stringify(page.timerEdit(true)));
    }

    Component.onCompleted: {
        page.syncTimer();
        page.syncLighting();
    }

    Connections {
        target: SettingsData

        function onShutdownTimeChanged() {
            page.syncTimerIfClean();
        }
    }

    Connections {
        target: HardwareData

        function onLightingChanged() {
            page.syncLightingIfClean();
        }
    }

    Connections {
        target: AdminState

        function onSaveSettingsErrorChanged() {
            // Le worker remet le champ à vide au début d'un enregistrement :
            // seul un code non vide est un refus.
            if (AdminState.saveSettingsError === "")
                return;
            // Refus : brouillon conservé, aucun état optimiste.
            page.timerSubmitting = false;
            page.timerClearing = false;
            if (page.lightingSubmitting) {
                page.lightingSubmitting = false;
                page.lightingError = AdminState.saveSettingsErrorText;
            }
        }

        function onSaveSettingsSucceededChanged() {
            if (!AdminState.saveSettingsSucceeded)
                return;
            if (page.timerSubmitting) {
                page.syncTimer();
                page.timerSubmitting = false;
                AppState.notify(Strings.t(page.timerClearing ? "toast.timerCancelled"
                                                             : "toast.timerSet"),
                                page.timerClearing ? "info" : "ok");
                page.timerClearing = false;
            }
            if (page.lightingSubmitting) {
                page.syncLighting();
                page.lightingSubmitting = false;
                page.lightingError = "";
                AppState.notify(Strings.t("tools.lighting.saved"), "ok");
            }
        }
    }

    Connections {
        target: AgentData

        // Fin d'écriture : les brouillons reprennent la valeur relue du
        // snapshot, que l'écriture ait réussi (valeur éventuellement ajustée
        // par la caméra) ou échoué.
        function onUvcPendingChanged() {
            if (!AgentData.uvcPending)
                page.uvcResetToken += 1;
        }
    }

    // ── Webcam ──────────────────────────────────────────────────────────────

    /// Un contrôle est éditable si l'agent expose l'écriture, si le device
    /// provient du snapshot et si ses drapeaux UVC autorisent l'écriture.
    function uvcEditable(control, devicePath) {
        if (!AgentData.uvcWriteSupported || !control || !control.current)
            return false;
        if (typeof devicePath !== "string" || devicePath === "")
            return false;
        var flags = control.flags;
        if (flags && (flags.readOnly === true || flags.disabled === true
                      || flags.grabbed === true || flags.inactive === true
                      || flags.writeOnly === true))
            return false;
        switch (control.kind) {
        case "integer":
            return control.current.kind === "integer"
                    && control.max > control.min;
        case "boolean":
            return control.current.kind === "boolean";
        case "menu":
        case "integerMenu":
            // Les tableaux issus du snapshot peuvent être des séquences QML :
            // on teste la forme (`length`) plutôt que `Array.isArray`.
            return control.menu !== undefined && control.menu !== null
                    && control.menu.length > 0;
        default:
            return false;
        }
    }

    function uvcValue(control) {
        if (!control || !control.current)
            return "—";
        var kind = control.current.kind;
        if (kind === "menu")
            return control.current.label !== "" ? control.current.label
                                                : (control.current.key !== ""
                                                   ? control.current.key : "—");
        if (kind === "boolean")
            return control.current.boolean === true ? Strings.t("common.yes")
                                                    : Strings.t("common.no");
        if (kind === "integer" || kind === "integerMenu")
            return Strings.formatNumber(control.current.value, 0);
        return "—";
    }

    function uvcDetails(control) {
        if (!control)
            return "";
        var parts = [
            Strings.tf("tools.webcam.details",
                       [Strings.formatNumber(control.min, 0),
                        Strings.formatNumber(control.max, 0),
                        Strings.formatNumber(control.step, 0),
                        Strings.formatNumber(control.default, 0)])
        ];
        if (control.category !== "")
            parts.push(Strings.tf("tools.webcam.category", [control.category]));
        return parts.join(" · ");
    }

    /// Valeur courante entière d'un contrôle (le snapshot fait foi).
    function uvcCurrentNumber(control) {
        if (!control || !control.current || typeof control.current.value !== "number"
                || !isFinite(control.current.value))
            return control ? control.min : 0;
        return control.current.value;
    }

    /// Valeur affichée à côté du curseur (brouillon pendant le drag).
    function uvcDraftLabel(control, draft) {
        var unit = control && typeof control.unit === "string" ? control.unit : "";
        var text = Strings.formatNumber(draft, 0);
        return unit !== "" ? text + " " + unit : text;
    }

    /// Position dans `menu` de la valeur courante (index UVC, clé ou libellé).
    function uvcMenuIndex(control) {
        if (!control || !control.current || !control.menu
                || typeof control.menu.length !== "number")
            return -1;
        var current = control.current;
        for (var i = 0; i < control.menu.length; ++i) {
            var entry = control.menu[i];
            if (entry.index === current.index)
                return i;
        }
        for (var j = 0; j < control.menu.length; ++j) {
            var candidate = control.menu[j];
            if (current.key !== "" && candidate.key === current.key)
                return j;
            if (current.label !== "" && candidate.label === current.label)
                return j;
        }
        return -1;
    }

    /// Émet la demande d'écriture ; `Main.qml` la traduit en appel `User1`.
    function commitUvc(devicePath, controlKey, value) {
        if (!AgentData.uvcWriteSupported || devicePath === "" || controlKey === "")
            return;
        if (AgentData.uvcPending)
            return;
        page.uvcLastDevice = devicePath;
        page.uvcLastKey = controlKey;
        AppState.requestUvcControl(devicePath, controlKey, Math.round(value));
    }

    function uvcRequestMatches(devicePath, controlKey) {
        return page.uvcLastDevice === devicePath && page.uvcLastKey === controlKey;
    }

    function uvcRequestFailed(devicePath, controlKey) {
        return AgentData.uvcError !== ""
                && page.uvcRequestMatches(devicePath, controlKey);
    }

    contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        title: Strings.t("tools.title")
        subtitle: Strings.t("tools.subtitle")
        iconName: "tools"

        StatusBadge {
            kind: HardwareData.isLive ? "ok" : "warn"
            label: HardwareData.isLive ? Strings.t("app.live") : Strings.t("app.demo")
        }
    }

    // ── Minuterie d'extinction ──────────────────────────────────────────────
    AppCard {
        objectName: "toolsTimerCard"
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("tools.timer.title")
            hint: Strings.t("tools.timer.desc")
            iconName: "stopwatch"

            StatusBadge {
                kind: ShutdownTimerData.statusKind
                label: ShutdownTimerData.statusText
            }
        }

        InlineNotice {
            Layout.fillWidth: true
            visible: !ShutdownTimerData.available
            kind: "warn"
            title: Strings.t("tools.timer.unavailable")
            body: ShutdownTimerData.error !== ""
                  ? ShutdownTimerData.error : Strings.t("tools.timer.unavailableHint")
        }

        InlineNotice {
            objectName: "timerErrorNotice"
            Layout.fillWidth: true
            visible: page.timerStatusError
            kind: "error"
            title: Strings.t("tools.timer.daemonError")
            body: ShutdownTimerData.errorText
        }

        InfoRow {
            Layout.fillWidth: true
            visible: ShutdownTimerData.available
            label: Strings.t("tools.timer.scheduledAt")
            value: ShutdownTimerData.enabled
                   ? (ShutdownTimerData.scheduledAt !== ""
                      ? ShutdownTimerData.scheduledAt
                      : Strings.t("common.yes"))
                   : Strings.t("common.none")
            iconName: "stopwatch"
        }

        InfoRow {
            Layout.fillWidth: true
            visible: ShutdownTimerData.available
            label: Strings.t("tools.timer.source")
            value: ShutdownTimerData.sourceText
            iconName: "gear"
        }

        ToggleRow {
            objectName: "timerEnableRow"
            Layout.fillWidth: true
            label: Strings.t("tools.timer.enable")
            description: Strings.t("tools.timer.enableHint")
            checked: page.timerEnabled
            supported: page.timerEditable()
            unsupportedReason: !AdminState.available ? Strings.t("settings.adminRequired")
                               : !ShutdownTimerData.available
                                 ? Strings.t("tools.timer.unavailable")
                                 : Strings.t("common.readOnly")
            onToggled: function(checked) { page.timerEnabled = checked; }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s12
            enabled: page.timerEditable() && page.timerEnabled

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                text: Strings.t("tools.timer.at")
                color: parent.enabled ? Theme.textPrimary : Theme.textMuted
            }

            AppSelect {
                id: hourSelect
                Layout.preferredWidth: 96
                model: page.hourOptions()
                Accessible.name: Strings.t("tools.timer.hour")
                onActivated: function(index) { page.timerHour = index; }
            }

            Binding {
                target: hourSelect
                property: "currentIndex"
                value: page.timerHour
            }

            AppLabel {
                text: ":"
                color: parent.enabled ? Theme.textSecondary : Theme.textMuted
            }

            AppSelect {
                id: minuteSelect
                Layout.preferredWidth: 96
                model: page.minuteOptions()
                Accessible.name: Strings.t("tools.timer.minute")
                onActivated: function(index) { page.timerMinute = index; }
            }

            Binding {
                target: minuteSelect
                property: "currentIndex"
                value: page.timerMinute
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: page.timerEditable() || page.timerDirty()

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: page.timerDirty() ? Strings.t("tools.timer.dirtyHint") : ""
            }

            AppButton {
                objectName: "timerCancelButton"
                text: Strings.t("tools.timer.cancel")
                variant: "secondary"
                enabled: page.timerEditable()
                         && (ShutdownTimerData.enabled
                             || SettingsData.shutdownTime !== "")
                onClicked: page.cancelTimerSchedule()
            }

            AppButton {
                objectName: "timerApplyButton"
                text: Strings.t("common.apply")
                enabled: page.timerDirty() && page.timerEditable()
                onClicked: page.submitTimer()
            }
        }
    }

    // ── Éclairage ───────────────────────────────────────────────────────────
    AppCard {
        objectName: "toolsLightingCard"
        Layout.fillWidth: true
        visible: page.lightingVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("tools.lighting.title")
            hint: Strings.t("tools.lighting.desc")
            iconName: "kb_backlight"

            StatusBadge {
                kind: page.lightingEditable ? "ok" : "neutral"
                label: page.lightingEditable ? Strings.t("tools.lighting.editable")
                                             : Strings.t("common.readOnly")
            }
        }

        InlineNotice {
            objectName: "lightingGlobalDisabled"
            Layout.fillWidth: true
            visible: !SettingsData.lightingEnabled
            kind: "warn"
            title: Strings.t("settings.lightingControl")
            body: Strings.t("tools.lighting.globalDisabled")
        }

        InlineNotice {
            objectName: "lightingAdminMissing"
            Layout.fillWidth: true
            visible: page.lightingVisible && !AdminState.available
            kind: "warn"
            title: Strings.t("admin.unavailable.missing")
            body: Strings.t("settings.adminRequired")
        }

        InlineNotice {
            objectName: "lightingSaving"
            Layout.fillWidth: true
            visible: page.lightingSubmitting
            kind: "info"
            title: Strings.t("settings.saving")
            body: Strings.t("tools.lighting.dirty")
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.keyboardAvailable
            variant: "title"
            text: Strings.t("tools.lighting.keyboard")
        }

        ToggleRow {
            objectName: "lightingKeyboardToggle"
            Layout.fillWidth: true
            visible: page.keyboardAvailable
            label: Strings.t("tools.lighting.keyboardOn")
            description: page.keyboardRgb
                         ? Strings.t("tools.lighting.zonesNote")
                           .arg(HardwareData.lighting.keyboard.zones)
                         : Strings.t("tools.lighting.whiteNote")
            checked: page.keyboardBrightness > 0
            supported: page.keyboardEditable
            unsupportedReason: Strings.t("tools.lighting.writeUnavailable")
            onToggled: function(checked) { page.setKeyboardOn(checked); }
        }

        SliderRow {
            objectName: "lightingKeyboardBrightness"
            Layout.fillWidth: true
            visible: page.keyboardAvailable
            label: Strings.t("display.brightness")
            from: 0
            to: page.keyboardMaxBrightness
            value: page.keyboardBrightness
            supported: page.keyboardEditable
            unsupportedReason: Strings.t("tools.lighting.writeUnavailable")
            onMoved: function(value) { page.keyboardBrightness = value; }
        }

        SliderRow {
            objectName: "lightingKeyboardRed"
            Layout.fillWidth: true
            visible: page.keyboardAvailable && page.keyboardRgb
            label: Strings.t("tools.lighting.red")
            from: 0
            to: page.keyboardMaxRed
            value: page.keyboardRed
            supported: page.keyboardEditable
            onMoved: function(value) { page.keyboardRed = value; }
        }

        SliderRow {
            objectName: "lightingKeyboardGreen"
            Layout.fillWidth: true
            visible: page.keyboardAvailable && page.keyboardRgb
            label: Strings.t("tools.lighting.green")
            from: 0
            to: page.keyboardMaxGreen
            value: page.keyboardGreen
            supported: page.keyboardEditable
            onMoved: function(value) { page.keyboardGreen = value; }
        }

        SliderRow {
            objectName: "lightingKeyboardBlue"
            Layout.fillWidth: true
            visible: page.keyboardAvailable && page.keyboardRgb
            label: Strings.t("tools.lighting.blue")
            from: 0
            to: page.keyboardMaxBlue
            value: page.keyboardBlue
            supported: page.keyboardEditable
            onMoved: function(value) { page.keyboardBlue = value; }
        }

        SelectRow {
            objectName: "lightingKeyboardMode"
            Layout.fillWidth: true
            visible: page.keyboardAvailable
            label: Strings.t("tools.lighting.mode")
            options: page.keyboardBreathing
                     ? [Strings.t("tools.lighting.modeStatic"),
                        Strings.t("tools.lighting.modeBreathing")]
                     : [Strings.t("tools.lighting.modeStatic")]
            currentIndex: page.keyboardMode === "breathing" ? 1 : 0
            supported: page.keyboardEditable && page.keyboardBreathing
            unsupportedReason: Strings.t("tools.lighting.writeUnavailable")
            onChanged: function(index) {
                page.keyboardMode = index === 1 ? "breathing" : "static";
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.divider
            visible: page.keyboardAvailable && page.lightbarAvailable
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            variant: "title"
            text: Strings.t("tools.lighting.lightbar")
        }

        ToggleRow {
            objectName: "lightingLightbarToggle"
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            label: Strings.t("tools.lighting.lightbarOn")
            checked: page.lightbarBrightness > 0
            supported: page.lightbarEditable
            unsupportedReason: Strings.t("tools.lighting.writeUnavailable")
            onToggled: function(checked) { page.setLightbarOn(checked); }
        }

        SliderRow {
            objectName: "lightingLightbarBrightness"
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            label: Strings.t("display.brightness")
            from: 0
            to: page.lightbarMaxBrightness
            value: page.lightbarBrightness
            supported: page.lightbarEditable
            onMoved: function(value) { page.lightbarBrightness = value; }
        }

        SliderRow {
            objectName: "lightingLightbarRed"
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            label: Strings.t("tools.lighting.red")
            from: 0
            to: page.lightbarMaxRed
            value: page.lightbarRed
            supported: page.lightbarEditable
            onMoved: function(value) { page.lightbarRed = value; }
        }

        SliderRow {
            objectName: "lightingLightbarGreen"
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            label: Strings.t("tools.lighting.green")
            from: 0
            to: page.lightbarMaxGreen
            value: page.lightbarGreen
            supported: page.lightbarEditable
            onMoved: function(value) { page.lightbarGreen = value; }
        }

        SliderRow {
            objectName: "lightingLightbarBlue"
            Layout.fillWidth: true
            visible: page.lightbarAvailable
            label: Strings.t("tools.lighting.blue")
            from: 0
            to: page.lightbarMaxBlue
            value: page.lightbarBlue
            supported: page.lightbarEditable
            onMoved: function(value) { page.lightbarBlue = value; }
        }

        InlineNotice {
            objectName: "lightingError"
            Layout.fillWidth: true
            visible: page.lightingError !== ""
            kind: "error"
            title: Strings.t("tools.lighting.writeError")
            body: page.lightingError
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: page.lightingEditable || page.lightingDirty()

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: page.lightingDirty() ? Strings.t("tools.lighting.dirty") : ""
            }

            AppButton {
                objectName: "lightingResetButton"
                text: Strings.t("tools.lighting.reset")
                variant: "secondary"
                enabled: page.lightingDirty() && !AdminState.savingSettings
                onClicked: page.resetLighting()
            }

            AppButton {
                objectName: "lightingApplyButton"
                text: page.lightingSubmitting ? Strings.t("settings.saving")
                                              : Strings.t("tools.lighting.apply")
                enabled: page.lightingDirty() && page.lightingEditable
                         && !page.lightingSubmitting
                onClicked: page.submitLighting()
            }
        }
    }

    // ── Webcam ──────────────────────────────────────────────────────────────
    AppCard {
        objectName: "toolsWebcamCard"
        Layout.fillWidth: true
        visible: page.webcamVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("tools.webcam.title")
            hint: Strings.t("tools.webcam.desc")
            iconName: "webcam"

            StatusBadge {
                kind: HardwareData.webcam.devices.length > 0 ? "ok" : "neutral"
                label: HardwareData.webcam.devices.length > 0
                       ? Strings.tf("tools.webcam.deviceCount",
                                    [HardwareData.webcam.devices.length])
                       : Strings.t("tools.webcam.noDevice")
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Theme.s12

            AppLabel {
                Layout.fillWidth: true
                visible: !AgentData.uvcWriteSupported
                         && HardwareData.webcam.devices.length > 0
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: Strings.t("tools.webcam.uvcWriteUnavailable")
            }

            // Sections par caméra : aucune ligne si le device est absent.
            Repeater {
                model: HardwareData.webcam.devices

                delegate: ColumnLayout {
                    id: webcamDevice
                    required property var modelData
                    required property int index

                    readonly property string devicePath: modelData.path !== undefined
                                                         ? modelData.path : ""

                    objectName: "webcamDevice_" + index
                    Layout.fillWidth: true
                    spacing: Theme.s4

                    InfoRow {
                        Layout.fillWidth: true
                        label: webcamDevice.modelData.name !== ""
                               ? webcamDevice.modelData.name
                               : Strings.t("tools.webcam.device")
                        value: webcamDevice.devicePath !== "" ? webcamDevice.devicePath : "—"
                        iconName: "webcam"
                        badgeLabel: Strings.tf("tools.webcam.controlCount",
                                               [webcamDevice.modelData.controls.length])
                        badgeKind: "neutral"
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        visible: webcamDevice.modelData.controlsError !== ""
                        variant: "small"
                        color: Theme.warning
                        wrapMode: Text.WordWrap
                        text: Strings.t("tools.webcam.controlsError") + " "
                              + webcamDevice.modelData.controlsError
                    }

                    AppLabel {
                        Layout.fillWidth: true
                        visible: HardwareData.webcam.controlsSupported
                                 && webcamDevice.modelData.controls.length === 0
                                 && webcamDevice.modelData.controlsError === ""
                        variant: "caption"
                        secondary: true
                        wrapMode: Text.WordWrap
                        text: Strings.t("tools.webcam.noControls")
                    }

                    Repeater {
                        model: webcamDevice.modelData.controls

                        delegate: ColumnLayout {
                            id: webcamControl
                            required property var modelData

                            readonly property var control: modelData
                            readonly property string controlKey: control.key
                            readonly property string controlLabel: control.name !== ""
                                    ? control.name : control.key
                            /// Métadonnées UVC : jamais éditables.
                            readonly property bool editable: !webcamDevice.modelData.metadata
                                    && page.uvcEditable(control, webcamDevice.devicePath)
                            readonly property bool showPending: AgentData.uvcPending
                                    && page.uvcRequestMatches(webcamDevice.devicePath,
                                                              controlKey)
                            readonly property bool failed: page.uvcRequestFailed(
                                    webcamDevice.devicePath, controlKey)
                            readonly property real snapshotValue:
                                    page.uvcCurrentNumber(control)
                            readonly property int menuIndex: page.uvcMenuIndex(control)
                            readonly property bool booleanValue: control.current
                                    && control.current.boolean === true

                            /// Brouillon entier : le snapshot fait foi hors drag.
                            property real draft: snapshotValue
                            property bool dragging: false
                            property int resetToken: page.uvcResetToken

                            objectName: "uvcControl_" + controlKey
                            Layout.fillWidth: true
                            spacing: 2

                            onSnapshotValueChanged: {
                                if (!AgentData.uvcPending)
                                    draft = snapshotValue;
                            }

                            onResetTokenChanged: draft = snapshotValue;

                            function commitDraft() {
                                var control = webcamControl.control;
                                var step = control.step > 0 ? control.step : 1;
                                var min = control.min;
                                var max = control.max;
                                // Aligne sur le pas UVC (le backend refuse une
                                // valeur non multiple) et borne à la plage.
                                var rounded = min + Math.round((draft - min) / step) * step;
                                rounded = Math.max(min, Math.min(max, rounded));
                                if (rounded === Math.round(snapshotValue))
                                    return;
                                page.commitUvc(webcamDevice.devicePath, controlKey, rounded);
                            }

                            function chooseMenu(position) {
                                if (!control.menu
                                        || typeof control.menu.length !== "number"
                                        || position < 0 || position >= control.menu.length)
                                    return;
                                page.commitUvc(webcamDevice.devicePath, controlKey,
                                               control.menu[position].index);
                            }

                            // ── Lecture seule (agent absent ou drapeaux UVC) ──
                            InfoRow {
                                objectName: "uvcValue_" + webcamControl.controlKey
                                Layout.fillWidth: true
                                visible: !webcamControl.editable
                                label: webcamControl.controlLabel
                                value: page.uvcValue(webcamControl.control)
                                iconName: "gear"
                                badgeLabel: Strings.t("common.readOnly")
                                badgeKind: "neutral"
                            }

                            // ── Éditeurs : chargés uniquement si autorisés ─────
                            Loader {
                                Layout.fillWidth: true
                                active: webcamControl.editable
                                sourceComponent: uvcEditorComponent
                            }

                            Component {
                                id: uvcEditorComponent

                                ColumnLayout {
                                    spacing: 2

                                    // ── En-tête éditable ───────────────────────
                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: Theme.s8

                                        AppLabel {
                                            Layout.fillWidth: true
                                            variant: "body"
                                            text: webcamControl.controlLabel
                                            elide: Text.ElideRight
                                        }

                                        StatusBadge {
                                            objectName: "uvcPending_" + webcamControl.controlKey
                                            visible: webcamControl.showPending
                                            kind: "info"
                                            label: Strings.t("tools.webcam.uvcApplying")
                                        }
                                    }

                                    // ── Entier : curseur, commit au relâchement
                                    RowLayout {
                                        Layout.fillWidth: true
                                        visible: webcamControl.control.kind === "integer"
                                        spacing: Theme.s12

                                        AppSlider {
                                            id: uvcSlider
                                            objectName: "uvcSlider_" + webcamControl.controlKey
                                            Layout.fillWidth: true
                                            from: webcamControl.control.min
                                            to: webcamControl.control.max
                                            stepSize: webcamControl.control.step > 0
                                                      ? webcamControl.control.step : 1
                                            snapMode: Slider.SnapAlways
                                            enabled: !AgentData.uvcPending
                                            Accessible.name: webcamControl.controlLabel
                                            Accessible.description: page.uvcDetails(
                                                                        webcamControl.control)

                                            onMoved: {
                                                webcamControl.draft = value;
                                                // Clavier/molette : aucun drag en
                                                // cours, la valeur est stable.
                                                if (!pressed)
                                                    webcamControl.commitDraft();
                                            }

                                            onPressedChanged: {
                                                if (pressed) {
                                                    webcamControl.dragging = true;
                                                } else if (webcamControl.dragging) {
                                                    webcamControl.dragging = false;
                                                    webcamControl.commitDraft();
                                                }
                                            }
                                        }

                                        AppLabel {
                                            objectName: "uvcDisplay_" + webcamControl.controlKey
                                            Layout.preferredWidth: 64
                                            horizontalAlignment: Text.AlignRight
                                            variant: "body"
                                            strong: true
                                            text: page.uvcDraftLabel(webcamControl.control,
                                                                     webcamControl.draft)
                                            Accessible.name: Strings.tf(
                                                "tools.webcam.uvcValueFormat",
                                                [webcamControl.controlLabel,
                                                 page.uvcDraftLabel(webcamControl.control,
                                                                    webcamControl.draft)])
                                        }
                                    }

                                    Binding {
                                        target: uvcSlider
                                        property: "value"
                                        value: webcamControl.draft
                                        restoreMode: Binding.RestoreNone
                                    }

                                    // ── Booléen : interrupteur, commit immédiat
                                    RowLayout {
                                        Layout.fillWidth: true
                                        visible: webcamControl.control.kind === "boolean"
                                        spacing: Theme.s8

                                        AppLabel {
                                            Layout.fillWidth: true
                                            variant: "small"
                                            secondary: true
                                            text: webcamControl.booleanValue
                                                  ? Strings.t("common.enabled")
                                                  : Strings.t("common.disabled")
                                        }

                                        AppSwitch {
                                            id: uvcSwitch
                                            objectName: "uvcSwitch_" + webcamControl.controlKey
                                            enabled: !AgentData.uvcPending
                                            Accessible.name: webcamControl.controlLabel
                                            Accessible.description: page.uvcDetails(
                                                                        webcamControl.control)
                                            // `toggle()`/programmation ne passe
                                            // pas par `toggled` : on ne commite
                                            // que si l'état diffère du snapshot.
                                            onCheckedChanged: {
                                                if (!AgentData.uvcPending
                                                        && checked
                                                           !== webcamControl.booleanValue)
                                                    page.commitUvc(webcamDevice.devicePath,
                                                                   webcamControl.controlKey,
                                                                   checked ? 1 : 0);
                                            }
                                        }
                                    }

                                    Binding {
                                        target: uvcSwitch
                                        property: "checked"
                                        value: webcamControl.booleanValue
                                        when: !AgentData.uvcPending
                                        restoreMode: Binding.RestoreNone
                                    }

                                    // ── Menu : libellés UVC, commit immédiat ───
                                    RowLayout {
                                        Layout.fillWidth: true
                                        visible: webcamControl.control.kind === "menu"
                                                 || webcamControl.control.kind === "integerMenu"
                                        spacing: Theme.s8

                                        AppSelect {
                                            id: uvcMenu
                                            objectName: "uvcMenu_" + webcamControl.controlKey
                                            Layout.preferredWidth: 240
                                            model: webcamControl.control.menu
                                            textRole: "label"
                                            enabled: !AgentData.uvcPending
                                            Accessible.name: webcamControl.controlLabel
                                            Accessible.description: page.uvcDetails(
                                                                        webcamControl.control)
                                            onActivated: function(index) {
                                                webcamControl.chooseMenu(index);
                                            }
                                        }
                                    }

                                    Binding {
                                        target: uvcMenu
                                        property: "currentIndex"
                                        value: webcamControl.menuIndex
                                        when: !AgentData.uvcPending
                                        restoreMode: Binding.RestoreNone
                                    }

                                    // ── Détails + erreur inline ────────────────
                                    AppLabel {
                                        Layout.fillWidth: true
                                        variant: "caption"
                                        secondary: true
                                        wrapMode: Text.WordWrap
                                        text: page.uvcDetails(webcamControl.control)
                                    }

                                    AppLabel {
                                        objectName: "uvcError_" + webcamControl.controlKey
                                        Layout.fillWidth: true
                                        visible: webcamControl.failed
                                        variant: "small"
                                        color: Theme.error
                                        wrapMode: Text.WordWrap
                                        text: AgentData.uvcErrorText
                                    }
                                }
                            }
                        }
                    }
                }
            }

            InlineNotice {
                objectName: "webcamControlsUnavailable"
                Layout.fillWidth: true
                visible: page.webcamNoControls
                kind: "info"
                title: Strings.t("tools.webcam.controls")
                body: Strings.t("tools.webcam.controlsUnavailable")
            }

            InlineNotice {
                Layout.fillWidth: true
                visible: HardwareData.webcam.controlsError !== ""
                kind: "warn"
                title: Strings.t("tools.webcam.controlsError")
                body: HardwareData.webcam.controlsError
            }

            AppLabel {
                Layout.fillWidth: true
                visible: HardwareData.webcam.controlsTruncated
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: Strings.t("tools.webcam.controlsTruncated")
            }

            InfoRow {
                Layout.fillWidth: true
                visible: HardwareData.webcam.switchAvailable
                label: Strings.t("tools.webcam.killswitch")
                value: HardwareData.webcam.enabled ? Strings.t("common.enabled")
                                                   : Strings.t("common.disabled")
                iconName: "webcam"
                badgeLabel: Strings.t("common.readOnly")
                badgeKind: "neutral"
            }
        }
    }
}
