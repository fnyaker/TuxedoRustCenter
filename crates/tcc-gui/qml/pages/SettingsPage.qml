// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Réglages globaux : apparence, contrôle matériel, bascule PPD × alimentation,
// charge, affichage/agent, PRIME, capacités et services. Les préférences
// purement locales (thème, langue, unités, animations) sont persistées via
// QSettings ; les réglages matériels passent par `Admin1`/`Admin.Hw`, sans
// aucune écriture privilégiée dans QML.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

PageScroll {
    id: page

    readonly property var languageCodes: ["en", "fr", "de"]
    readonly property var matrixPpdKeys: ["power-saver", "balanced", "performance"]
    readonly property var matrixPowerKeys: ["ac", "battery"]


    function ppdLabel() {
        if (LiveData.ppd === "power-saver")
            return Strings.t("settings.ppdSaver");
        if (LiveData.ppd === "performance")
            return Strings.t("settings.ppdPerformance");
        if (LiveData.ppd === "balanced")
            return Strings.t("settings.ppdBalanced");
        return Strings.t("settings.ppdUnknown");
    }

    function powerLabel() {
        if (LiveData.power === "ac")
            return Strings.t("dash.ac");
        if (LiveData.power === "battery")
            return Strings.t("dash.battery");
        return Strings.t("state.unavailable");
    }

    function chargeLabel(value) {
        return value && value !== "" ? value : "—";
    }

    function thresholdLabel(value) {
        return value > 0 ? Strings.formatPercent(value) : "—";
    }

    readonly property string agentBackendLabel: {
        if (!AgentData.available)
            return "";
        var backend = AgentData.display.backend;
        return backend.name !== "" ? backend.name : backend.kind;
    }
    readonly property string agentOutputLabel: {
        if (!AgentData.available)
            return "";
        var output = AgentData.display.output;
        if (output.name === "")
            return "";
        return output.name
                + (output.connected ? "" : " (" + Strings.t("display.disconnected") + ")");
    }

    function primeLabel(mode) {
        if (mode === "iGPU" || mode === "igpu")
            return Strings.t("settings.graphics.igpu");
        if (mode === "dGPU" || mode === "dgpu")
            return Strings.t("settings.graphics.dgpu");
        if (mode === "on-demand" || mode === "ondemand")
            return Strings.t("settings.graphics.ondemand");
        if (mode === "off")
            return Strings.t("settings.graphics.off");
        return Strings.t("settings.ppdUnknown");
    }

    function manualProfileName() {
        var profile = LiveData.profileById(AppState.manualProfileId);
        return profile ? profile.name : Strings.t("common.none");
    }

    /// Défilement ciblé demandé par le Dashboard (mode graphique).
    function applyRequestedSection() {
        if (AppState.settingsSection !== "prime")
            return;
        page.scrollToItem(primeCard);
        AppState.settingsSection = "";
    }

    Connections {
        target: AppState

        function onSettingsSectionChanged() {
            if (AppState.page === "settings")
                Qt.callLater(page.applyRequestedSection);
        }

        function onPageChanged() {
            if (AppState.page === "settings")
                Qt.callLater(page.applyRequestedSection);
        }
    }

    // ── Réglages globaux modifiables ────────────────────────────────────────
    // Source : `GetSettingsJSON` → `tcc_core::Settings` (pont Rust). Les
    // interrupteurs portent l'intention locale ; toute relecture ou erreur les
    // resynchronise sur l'état réel.
    property bool fahrenheit: false
    property bool cpuEnabled: true
    property bool fanEnabled: true
    property bool lightingEnabled: true
    property bool settingsConflict: false

    // ── Charge (brouillon local ; écriture via Admin1.SaveSettingsJSON) ─────
    // Les listes autorisées viennent de `HardwareData.charging` (dernier
    // instantané matériel) ; les valeurs persistées servent de repli.
    property string chargeProfile: ""
    property string chargePriority: ""
    property int chargeStart: -1
    property int chargeEnd: -1
    property string chargeInitialProfile: ""
    property string chargeInitialPriority: ""
    property int chargeInitialStart: -1
    property int chargeInitialEnd: -1
    /// Vrai quand la dernière demande d'enregistrement incluait la charge.
    property bool chargeSubmitting: false
    property string chargeTypeDraft: ""
    property string chargeInitialType: ""
    property bool fnLockDraft: false
    property bool fnLockSubmitting: false

    /// Intention locale de l'inhibiteur de veille (révertée sur erreur).
    property bool powerBlockerDraft: false
    /// Intention locale de l'autostart (révertée si l'écriture échoue).
    property bool autostartDraft: false

    // ── Admin.Hw : ODM/TDP/cTGP (écritures transitoires) ───────────────────
    property int odmDraftIndex: -1
    property int odmInitialIndex: -1
    property var tdpDraft: []
    property var tdpInitial: []
    property string ctgpDraft: ""
    property string ctgpInitial: ""
    property bool ppdOverrideDraft: false
    /// Erreur locale sûre (clé de traduction), jamais la valeur saisie.
    property string odmError: ""
    property string tdpError: ""

    readonly property bool hwEditable: AdminHwData.available && !AdminHwData.pending
    readonly property bool odmPpdOwned: HardwareData.odm.platform.owner === "ppd"
    readonly property bool odmEditing: AdminHwData.available
                                        && HardwareData.visibleIfAvailable(HardwareData.odm.available)
    readonly property bool tdpEditing: AdminHwData.available
                                       && HardwareData.visibleIfAvailable(
                                           HardwareData.tdpExposed)
    readonly property bool ctgpEditing: AdminHwData.available
                                        && HardwareData.visibleIfAvailable(
                                            HardwareData.nvidia.available)
    /// Matériel ODM/TDP/cTGP présent, indépendamment de l'état du service.
    readonly property bool hwPresent: HardwareData.visibleIfAvailable(
        HardwareData.odm.available || HardwareData.odm.tdpAvailable
        || HardwareData.nvidia.available)
    readonly property bool hwServiceMissing: page.hwPresent
                                             && !AdminHwData.available
    /// Le tray a été chargé mais n'est pas disponible sur ce bureau.
    readonly property bool trayUnavailable: AppState.trayLoaded
                                            && !AppState.trayAvailable

    // Visibilité matérielle (absence confirmée seulement si snapshot sûr).
    readonly property bool chargingVisible: HardwareData.visibleIfAvailable(
        HardwareData.charging.available)
    readonly property bool primeVisible: HardwareData.visibleIfAvailable(
        HardwareData.prime.supported && HardwareData.nvidia.available)
    // La carte reste visible si le matériel existe même sans `Admin.Hw` :
    // la notice de service manquant doit être accessible. Masquage uniquement
    // quand le matériel lui-même est absent (YCbCr mis à part).
    readonly property bool capabilitiesVisible: page.hwPresent
            || HardwareData.visibleIfAvailable(HardwareData.ycbcr.available)
    readonly property bool lightingVisible: HardwareData.visibleIfAvailable(
        HardwareData.lighting.keyboard.available
        || HardwareData.lighting.lightbar.available)
    readonly property bool fnLockVisible: HardwareData.visibleIfAvailable(
        HardwareData.fnLock.supported)
    readonly property bool ycbcrVisible: HardwareData.visibleIfAvailable(
        HardwareData.ycbcr.available)

    // ── Matrice PPD × alimentation (brouillon local) ────────────────────────
    /// Clés `ppd/power` → `ProfileId`. Une clé absente = fallback AC/BAT.
    property var matrixDraft: ({})
    property var matrixInitial: ({})
    property bool matrixSubmitting: false

    function syncSettings() {
        page.fahrenheit = SettingsData.fahrenheit;
        page.cpuEnabled = SettingsData.cpuEnabled;
        page.fanEnabled = SettingsData.fanEnabled;
        page.lightingEnabled = SettingsData.lightingEnabled;
        // Ne jamais écraser un brouillon de charge/matrice en cours.
        page.syncChargeIfClean();
        page.syncMatrixIfClean();
    }

    function syncHwDrafts() {
        var choices = HardwareData.odmChoices;
        var index = choices.indexOf(HardwareData.odmCurrentProfile);
        if (index < 0)
            index = choices.indexOf(HardwareData.odmDefaultProfile);
        if (index < 0)
            index = 0;
        page.odmDraftIndex = index;
        page.odmInitialIndex = page.odmDraftIndex;
        var tdp = HardwareData.odm.tdp;
        var values = [];
        for (var i = 0; i < tdp.length; ++i)
            values.push(String(tdp[i].current));
        page.tdpDraft = values;
        page.tdpInitial = values.slice();
        page.ctgpDraft = String(HardwareData.nvidia.ctgpOffset);
        page.ctgpInitial = page.ctgpDraft;
        page.ppdOverrideDraft = SettingsData.ppdOverride;
        page.odmError = "";
        page.tdpError = "";
    }

    function syncHwDraftsIfClean() {
        if (page.odmDirty() || page.tdpDirty() || page.ctgpDirty())
            return;
        page.syncHwDrafts();
    }

    function odmChoices() {
        return HardwareData.odmChoices;
    }

    /// Libellé localisé d'un profil ODM, repli sur l'identifiant matériel.
    function odmLabel(value) {
        var key = "odm.mode." + value;
        var label = Strings.t(key);
        return label === key ? String(value) : label;
    }

    function odmOptions() {
        var choices = page.odmChoices();
        var out = [];
        for (var i = 0; i < choices.length; ++i)
            out.push({ "value": choices[i], "name": page.odmLabel(choices[i]) });
        return out;
    }

    function odmDirty() {
        return page.odmDraftIndex !== page.odmInitialIndex;
    }

    /// Mêmes règles que `tccd-admin::validate_odm_name` : non vide, ≤ 64
    /// caractères ASCII graphiques, sans antislash.
    function odmNameValid(value) {
        var text = String(value);
        if (text.length === 0 || text.length > 64)
            return false;
        for (var i = 0; i < text.length; ++i) {
            var code = text.charCodeAt(i);
            if (code < 33 || code > 126 || code === 92)
                return false;
        }
        return true;
    }

    function tdpValue(index) {
        return index >= 0 && index < page.tdpDraft.length
               ? page.tdpDraft[index] : "";
    }

    function setTdpDraft(index, value) {
        var next = page.tdpDraft.slice();
        while (next.length <= index)
            next.push("");
        next[index] = value;
        page.tdpDraft = next;
        page.tdpError = "";
    }

    function tdpDirty() {
        if (page.tdpDraft.length !== page.tdpInitial.length)
            return true;
        for (var i = 0; i < page.tdpDraft.length; ++i) {
            if (page.tdpDraft[i] !== page.tdpInitial[i])
                return true;
        }
        return false;
    }

    function tdpNumber(index) {
        var value = Number(String(page.tdpValue(index)).replace(",", "."));
        return isFinite(value) ? value : NaN;
    }

    function tdpFieldInvalid(index) {
        var value = page.tdpNumber(index);
        var limits = HardwareData.odm.tdp[index];
        if (!limits || !isFinite(value))
            return true;
        return value < limits.min || value > limits.max;
    }

    /// Descripteurs attendus par le serveur, dans l'ordre PL1/PL2/PL4.
    readonly property var tdpExpectedDescriptors: ["pl1", "pl2", "pl4"]

    function tdpCountValid() {
        return HardwareData.odm.tdp.length > 0 && HardwareData.odm.tdp.length <= 3;
    }

    /// Ordre et unicité des descripteurs exposés par le daemon : le contrat ne
    /// transporte que les valeurs, on refuse donc localement une liste
    /// incohérente (4 entrées, doublon ou ordre inattendu).
    function tdpDescriptorsValid() {
        var list = HardwareData.odm.tdp;
        if (!page.tdpCountValid() || list.length !== page.tdpDraft.length)
            return false;
        var seen = {};
        for (var i = 0; i < list.length; ++i) {
            var descriptor = String(list[i].descriptor).toLowerCase();
            if (descriptor !== page.tdpExpectedDescriptors[i] || seen[descriptor])
                return false;
            seen[descriptor] = true;
        }
        return true;
    }

    /// PL1 ≤ PL2 ≤ PL4 : la monotonie évite une configuration refusée par le
    /// firmware. Les champs non numériques sont déjà signalés séparément.
    function tdpMonotonicValid() {
        var previous = -Infinity;
        for (var i = 0; i < page.tdpDraft.length; ++i) {
            var value = page.tdpNumber(i);
            if (!isFinite(value))
                return true;
            if (value < previous)
                return false;
            previous = value;
        }
        return true;
    }

    readonly property bool tdpMonotonicInvalid: page.tdpDraft.length > 1
            && !page.tdpMonotonicValid()

    function tdpValid() {
        if (page.tdpDraft.length === 0 || !page.tdpDescriptorsValid())
            return false;
        if (!page.tdpMonotonicValid())
            return false;
        for (var i = 0; i < page.tdpDraft.length; ++i) {
            if (page.tdpFieldInvalid(i))
                return false;
        }
        return true;
    }

    function ctgpValue() {
        var value = Number(String(page.ctgpDraft).replace(",", "."));
        return isFinite(value) ? Math.round(value) : NaN;
    }

    function ctgpDirty() {
        return page.ctgpDraft !== page.ctgpInitial;
    }

    function ctgpValid() {
        var value = page.ctgpValue();
        return isFinite(value) && value >= 0 && value <= HardwareData.ctgpMax;
    }

    function submitOdm(force) {
        if (!page.hwEditable || !HardwareData.odmWritable)
            return;
        // « Forcer » écrit `platform_profile` malgré PPD : la GUI n'envoie rien
        // sans le consentement persistant (le service refuserait de toute façon).
        var forcing = force === true;
        if (forcing && (!page.odmPpdOwned || !page.ppdOverrideDraft
                        || AdminState.savingSettings))
            return;
        var choices = page.odmChoices();
        if (page.odmDraftIndex < 0 || page.odmDraftIndex >= choices.length)
            return;
        var name = String(choices[page.odmDraftIndex]);
        if (!page.odmNameValid(name)) {
            AdminHwData.succeeded = false;
            page.odmError = "adminHw.error.invalidArgs";
            return;
        }
        page.odmError = "";
        AppState.requestOdmApply(name, forcing);
    }

    function submitTdp() {
        if (!page.hwEditable)
            return;
        if (!page.tdpDescriptorsValid()) {
            AdminHwData.succeeded = false;
            page.tdpError = "settings.tdpDescriptorError";
            return;
        }
        if (!page.tdpMonotonicValid()) {
            AdminHwData.succeeded = false;
            page.tdpError = "settings.tdpMonotonicError";
            return;
        }
        if (!page.tdpValid()) {
            AdminHwData.succeeded = false;
            page.tdpError = "settings.tdpRange";
            return;
        }
        page.tdpError = "";
        var values = [];
        for (var i = 0; i < page.tdpDraft.length; ++i)
            values.push(page.tdpNumber(i));
        AppState.requestTdpApply(JSON.stringify(values));
    }

    function submitCtgp() {
        if (!page.hwEditable)
            return;
        if (!page.ctgpValid()) {
            AdminHwData.succeeded = false;
            return;
        }
        AppState.requestCtgpApply(page.ctgpValue());
    }

    function submitPpdOverride(value) {
        if (!page.settingsEditable())
            return;
        page.ppdOverrideDraft = value;
        var dto = page.settingsEdit();
        dto.ppdOverride = value;
        AppState.requestSettingsSave(JSON.stringify(dto));
    }

    /// DTO commun : les booléens sont toujours envoyés, les champs de charge
    /// seulement s'ils ont été modifiés (`null` = ne pas changer).
    function settingsEdit() {
        return {
            "fahrenheit": page.fahrenheit,
            "cpuSettingsEnabled": page.cpuEnabled,
            "fanControlEnabled": page.fanEnabled,
            "keyboardBacklightControlEnabled": page.lightingEnabled,
            "chargingProfile": null,
            "chargingPriority": null,
            "chargingStartThreshold": null,
            "chargingEndThreshold": null,
            "stateMatrix": null,
            "shutdownTime": null,
            "clearShutdownTime": false,
            "ppdOverride": null,
            "chargingType": null,
            "fnLockStatus": null
        };
    }

    function settingsEditable() {
        // Des réglages invalides (schéma faux, référence absente) rendent la
        // base d'édition inutilisable : l'UI reste en lecture seule.
        return AdminState.available && !AdminState.savingSettings
                && SettingsData.live;
    }

    // ── Charge : sources, brouillon et validation ───────────────────────────
    function chargeEditable() {
        // Instantané matériel réellement décodé, non périmé, et capacités de
        // charge exposées : sinon les listes de valeurs seraient fictives.
        return page.settingsEditable() && HardwareData.isLive
                && !HardwareData.stale && HardwareData.charging.available;
    }

    readonly property bool chargeTypeSelectable: HardwareData.charging.chargeTypeWritable

    function chargeThresholdsEditable() {
        return page.chargeEditable()
                && HardwareData.charging.thresholdsAvailable
                && HardwareData.charging.startThresholds.length > 0
                && HardwareData.charging.endThresholds.length > 0;
    }

    function chargeProfileIndex() {
        return HardwareData.charging.profiles.indexOf(page.chargeProfile);
    }

    function chargePriorityIndex() {
        return HardwareData.charging.priorities.indexOf(page.chargePriority);
    }

    function chargeStartIndex() {
        return HardwareData.charging.startThresholds.indexOf(page.chargeStart);
    }

    function chargeEndIndex() {
        return HardwareData.charging.endThresholds.indexOf(page.chargeEnd);
    }

    function thresholdLabels(list) {
        var out = [];
        for (var i = 0; i < list.length; ++i)
            out.push(Strings.formatPercent(list[i]));
        return out;
    }

    // La valeur persistée fait foi après enregistrement (le worker matériel
    // peut appliquer la charge avec un léger décalage) ; le matériel sert de
    // repli quand le réglage n'est pas encore défini.
    function chargeSourceProfile() {
        if (SettingsData.chargingProfile !== "")
            return SettingsData.chargingProfile;
        return HardwareData.charging.profile;
    }

    function chargeSourcePriority() {
        if (SettingsData.chargingPriority !== "")
            return SettingsData.chargingPriority;
        return HardwareData.charging.priority;
    }

    function chargeSourceStart() {
        if (SettingsData.chargingStart >= 0)
            return SettingsData.chargingStart;
        return HardwareData.charging.thresholdsAvailable
                ? HardwareData.charging.start : -1;
    }

    function chargeSourceEnd() {
        if (SettingsData.chargingEnd >= 0)
            return SettingsData.chargingEnd;
        return HardwareData.charging.thresholdsAvailable
                ? HardwareData.charging.end : -1;
    }

    readonly property var standardChargeTypes: [
        "Trickle", "Fast", "Standard", "Adaptive", "Custom",
        "LongLife", "Bypass"
    ]

    /// Types de charge proposés : liste backend si non vide, sinon ABI
    /// standard. La valeur persistée est ajoutée pour ne pas la masquer.
    function chargeTypeValues() {
        var list = HardwareData.charging.chargeTypes.slice();
        if (list.length === 0)
            list = page.standardChargeTypes.slice();
        var persisted = SettingsData.chargingType;
        if (persisted !== "" && list.indexOf(persisted) < 0)
            list.push(persisted);
        return list;
    }

    function chargeTypeOptions() {
        return [""].concat(page.chargeTypeValues());
    }

    function chargeTypeLabels() {
        var values = page.chargeTypeOptions();
        var out = [];
        for (var i = 0; i < values.length; ++i) {
            out.push(values[i] === "" ? Strings.t("settings.chargeTypeUnchanged")
                                      : values[i]);
        }
        return out;
    }

    function chargeTypeIndex() {
        return page.chargeTypeOptions().indexOf(page.chargeTypeDraft);
    }

    function syncCharge() {
        var profile = page.chargeSourceProfile();
        var priority = page.chargeSourcePriority();
        var start = page.chargeSourceStart();
        var end = page.chargeSourceEnd();
        page.chargeProfile = profile;
        page.chargePriority = priority;
        page.chargeStart = start;
        page.chargeEnd = end;
        page.chargeInitialProfile = profile;
        page.chargeInitialPriority = priority;
        page.chargeInitialStart = start;
        page.chargeInitialEnd = end;
        var type = SettingsData.chargingType;
        page.chargeTypeDraft = type;
        page.chargeInitialType = type;
    }

    function chargeDirty() {
        return page.chargeProfile !== page.chargeInitialProfile
                || page.chargePriority !== page.chargeInitialPriority
                || page.chargeStart !== page.chargeInitialStart
                || page.chargeEnd !== page.chargeInitialEnd
                || page.chargeTypeDraft !== page.chargeInitialType;
    }

    /// Ne resynchronise que si l'utilisateur n'a pas de modification en cours.
    function syncChargeIfClean() {
        if (!page.chargeDirty() && !AdminState.savingSettings)
            page.syncCharge();
    }

    readonly property bool chargeProfileInvalid: page.chargeProfile
            !== page.chargeInitialProfile
            && HardwareData.charging.profiles.indexOf(page.chargeProfile) < 0
    readonly property bool chargePriorityInvalid: page.chargePriority
            !== page.chargeInitialPriority
            && HardwareData.charging.priorities.indexOf(page.chargePriority) < 0
    readonly property bool chargeStartInvalid: page.chargeStart
            !== page.chargeInitialStart
            && HardwareData.charging.startThresholds.indexOf(page.chargeStart) < 0
    readonly property bool chargeEndInvalid: page.chargeEnd
            !== page.chargeInitialEnd
            && HardwareData.charging.endThresholds.indexOf(page.chargeEnd) < 0

    readonly property bool chargeThresholdsDirty: page.chargeStart
            !== page.chargeInitialStart || page.chargeEnd !== page.chargeInitialEnd
    readonly property bool chargeRangeInvalid: page.chargeThresholdsDirty
            && (page.chargeStart < 0 || page.chargeEnd < 0
                || page.chargeStart >= page.chargeEnd)
    readonly property bool chargeTypeInvalid: page.chargeTypeDraft
            !== page.chargeInitialType
            && (!HardwareData.charging.chargeTypeWritable
                || (page.chargeTypeDraft !== ""
                    && (HardwareData.charging.chargeTypes.length > 0
                        ? HardwareData.charging.chargeTypes
                        : page.standardChargeTypes).indexOf(
                            page.chargeTypeDraft) < 0))

    function chargeValid() {
        return !page.chargeProfileInvalid && !page.chargePriorityInvalid
                && !page.chargeStartInvalid && !page.chargeEndInvalid
                && !page.chargeRangeInvalid && !page.chargeTypeInvalid;
    }

    function chargeEdit() {
        return {
            "fahrenheit": page.fahrenheit,
            "cpuSettingsEnabled": page.cpuEnabled,
            "fanControlEnabled": page.fanEnabled,
            "keyboardBacklightControlEnabled": page.lightingEnabled,
            "chargingProfile": page.chargeProfile !== page.chargeInitialProfile
                               && HardwareData.charging.profiles.length > 0
                               ? page.chargeProfile : null,
            "chargingPriority": page.chargePriority !== page.chargeInitialPriority
                                && HardwareData.charging.priorities.length > 0
                                ? page.chargePriority : null,
            "chargingStartThreshold": page.chargeStart !== page.chargeInitialStart
                                      && page.chargeThresholdsEditable()
                                      ? page.chargeStart : null,
            "chargingEndThreshold": page.chargeEnd !== page.chargeInitialEnd
                                    && page.chargeThresholdsEditable()
                                    ? page.chargeEnd : null,
            "chargingType": page.chargeTypeDraft !== page.chargeInitialType
                            && page.chargeTypeDraft !== ""
                            && HardwareData.charging.chargeTypeWritable
                            ? page.chargeTypeDraft : null,
            "fnLockStatus": null,
            "stateMatrix": null,
            "shutdownTime": null,
            "clearShutdownTime": false,
            "ppdOverride": null
        };
    }

    function submitCharge() {
        if (!page.chargeEditable() || !page.chargeDirty() || !page.chargeValid())
            return;
        page.chargeSubmitting = true;
        page.matrixSubmitting = false;
        AppState.requestSettingsSave(JSON.stringify(page.chargeEdit()));
    }

    function resetCharge() {
        page.syncCharge();
        page.chargeSubmitting = false;
    }

    /// Le switch Fn-Lock représente l'état réel du matériel ; après une
    /// écriture, la relecture du worker fait foi.
    function syncFnLock() {
        if (HardwareData.fnLock.enabledKnown) {
            page.fnLockDraft = HardwareData.fnLock.enabled;
        }
    }

    function submitFnLock(value) {
        if (!page.settingsEditable() || !HardwareData.fnLock.writable
                || !HardwareData.fnLock.enabledKnown)
            return;
        page.fnLockDraft = value;
        page.fnLockSubmitting = true;
        var dto = page.settingsEdit();
        dto.fnLockStatus = value;
        AppState.requestSettingsSave(JSON.stringify(dto));
    }

    // ── Matrice PPD × alimentation ──────────────────────────────────────────
    function matrixKey(ppd, power) {
        return ppd + "/" + power;
    }

    function matrixEditable() {
        return page.settingsEditable() && LiveData.isLive
                && LiveData.profiles.length > 0;
    }

    /// Valeur persistée d'une cellule (`""` = fallback AC/BAT).
    function settingsMatrixCell(ppd, power) {
        switch (ppd + "/" + power) {
        case "power-saver/ac":
            return SettingsData.matrixPowerSaverAc;
        case "power-saver/battery":
            return SettingsData.matrixPowerSaverBat;
        case "balanced/ac":
            return SettingsData.matrixBalancedAc;
        case "balanced/battery":
            return SettingsData.matrixBalancedBat;
        case "performance/ac":
            return SettingsData.matrixPerformanceAc;
        case "performance/battery":
            return SettingsData.matrixPerformanceBat;
        default:
            return "";
        }
    }

    function matrixCellFrom(source, ppd, power) {
        if (!source)
            return "";
        var value = source[page.matrixKey(ppd, power)];
        return value === undefined || value === null ? "" : value;
    }

    function matrixCell(ppd, power) {
        return page.matrixCellFrom(page.matrixDraft, ppd, power);
    }

    function matrixInitialCell(ppd, power) {
        return page.matrixCellFrom(page.matrixInitial, ppd, power);
    }

    function matrixCellInvalid(ppd, power) {
        var id = page.matrixCell(ppd, power);
        return id !== "" && LiveData.profileById(id) === null;
    }

    function matrixDirty() {
        for (var i = 0; i < page.matrixPpdKeys.length; ++i) {
            for (var j = 0; j < page.matrixPowerKeys.length; ++j) {
                var ppd = page.matrixPpdKeys[i];
                var power = page.matrixPowerKeys[j];
                if (page.matrixCell(ppd, power) !== page.matrixInitialCell(ppd, power))
                    return true;
            }
        }
        return false;
    }

    function matrixValid() {
        for (var i = 0; i < page.matrixPpdKeys.length; ++i) {
            for (var j = 0; j < page.matrixPowerKeys.length; ++j) {
                if (page.matrixCellInvalid(page.matrixPpdKeys[i], page.matrixPowerKeys[j]))
                    return false;
            }
        }
        return true;
    }

    function matrixOptionLabels() {
        var out = [Strings.t("settings.matrixFallback")];
        var list = LiveData.profiles;
        for (var i = 0; i < list.length; ++i)
            out.push(list[i].name);
        return out;
    }

    function matrixOptionIndex(ppd, power) {
        var id = page.matrixCell(ppd, power);
        if (id === "")
            return 0;
        var list = LiveData.profiles;
        for (var i = 0; i < list.length; ++i) {
            if (list[i].id === id)
                return i + 1;
        }
        // Profil disparu : la cellule est signalée invalide, la liste montre
        // le fallback pour forcer un choix explicite.
        return 0;
    }

    function setMatrixCell(ppd, power, profileId) {
        var draft = {};
        for (var key in page.matrixDraft)
            draft[key] = page.matrixDraft[key];
        var cellKey = page.matrixKey(ppd, power);
        if (profileId === "")
            delete draft[cellKey];
        else
            draft[cellKey] = profileId;
        page.matrixDraft = draft;
    }

    function syncMatrix() {
        var draft = {};
        var initial = {};
        for (var i = 0; i < page.matrixPpdKeys.length; ++i) {
            for (var j = 0; j < page.matrixPowerKeys.length; ++j) {
                var ppd = page.matrixPpdKeys[i];
                var power = page.matrixPowerKeys[j];
                var id = page.settingsMatrixCell(ppd, power);
                if (id !== "") {
                    draft[page.matrixKey(ppd, power)] = id;
                    initial[page.matrixKey(ppd, power)] = id;
                }
            }
        }
        page.matrixDraft = draft;
        page.matrixInitial = initial;
    }

    function syncMatrixIfClean() {
        if (!page.matrixDirty() && !AdminState.savingSettings)
            page.syncMatrix();
    }

    function matrixEdit() {
        var matrix = [];
        for (var i = 0; i < page.matrixPpdKeys.length; ++i) {
            for (var j = 0; j < page.matrixPowerKeys.length; ++j) {
                var ppd = page.matrixPpdKeys[i];
                var power = page.matrixPowerKeys[j];
                var id = page.matrixCell(ppd, power);
                if (id !== "") {
                    matrix.push({
                        "ppd": ppd,
                        "power": power,
                        "profileId": id
                    });
                }
            }
        }
        return {
            "fahrenheit": page.fahrenheit,
            "cpuSettingsEnabled": page.cpuEnabled,
            "fanControlEnabled": page.fanEnabled,
            "keyboardBacklightControlEnabled": page.lightingEnabled,
            "chargingProfile": null,
            "chargingPriority": null,
            "chargingStartThreshold": null,
            "chargingEndThreshold": null,
            "stateMatrix": matrix,
            "shutdownTime": null,
            "clearShutdownTime": false,
            "ppdOverride": null
        };
    }

    function submitMatrix() {
        if (!page.matrixEditable() || !page.matrixDirty() || !page.matrixValid())
            return;
        page.matrixSubmitting = true;
        AppState.requestSettingsSave(JSON.stringify(page.matrixEdit()));
    }

    function resetMatrix() {
        page.syncMatrix();
        page.matrixSubmitting = false;
    }

    function submitSettings() {
        if (!page.settingsEditable())
            return;
        page.chargeSubmitting = false;
        page.matrixSubmitting = false;
        AppState.requestSettingsSave(JSON.stringify(page.settingsEdit()));
    }

    function setFahrenheit(value) {
        if (!page.settingsEditable() || page.fahrenheit === value)
            return;
        page.fahrenheit = value;
        page.submitSettings();
    }

    function reloadSettings() {
        page.syncSettings();
        page.syncCharge();
        page.syncMatrix();
        page.chargeSubmitting = false;
        page.matrixSubmitting = false;
        page.settingsConflict = false;
        AppState.notify(Strings.t("settings.reloaded"), "info");
    }

    Component.onCompleted: {
        page.syncSettings();
        page.syncCharge();
        page.syncMatrix();
        page.syncFnLock();
        page.powerBlockerDraft = PowerBlockerData.blocked;
        page.autostartDraft = AutostartData.enabled;
        page.syncHwDrafts();
        // Première visite après « Dashboard → PRIME » : la page vient d'être
        // créée par le Loader, le signal de navigation a pu la précéder.
        Qt.callLater(page.applyRequestedSection);
    }

    Connections {
        target: SettingsData

        function onFahrenheitChanged() {
            if (!AdminState.savingSettings)
                page.fahrenheit = SettingsData.fahrenheit;
        }

        function onCpuEnabledChanged() {
            if (!AdminState.savingSettings)
                page.cpuEnabled = SettingsData.cpuEnabled;
        }

        function onFanEnabledChanged() {
            if (!AdminState.savingSettings)
                page.fanEnabled = SettingsData.fanEnabled;
        }

        function onLightingEnabledChanged() {
            if (!AdminState.savingSettings)
                page.lightingEnabled = SettingsData.lightingEnabled;
        }

        function onChargingProfileChanged() {
            page.syncChargeIfClean();
        }

        function onChargingTypeChanged() {
            page.syncChargeIfClean();
        }

        function onChargingPriorityChanged() {
            page.syncChargeIfClean();
        }

        function onChargingStartChanged() {
            page.syncChargeIfClean();
        }

        function onChargingEndChanged() {
            page.syncChargeIfClean();
        }

        function onMatrixPowerSaverAcChanged() { page.syncMatrixIfClean(); }
        function onMatrixPowerSaverBatChanged() { page.syncMatrixIfClean(); }
        function onMatrixBalancedAcChanged() { page.syncMatrixIfClean(); }
        function onMatrixBalancedBatChanged() { page.syncMatrixIfClean(); }
        function onMatrixPerformanceAcChanged() { page.syncMatrixIfClean(); }
        function onMatrixPerformanceBatChanged() { page.syncMatrixIfClean(); }

        function onPpdOverrideChanged() {
            if (!AdminState.savingSettings)
                page.ppdOverrideDraft = SettingsData.ppdOverride;
        }
    }

    Connections {
        target: HardwareData

        function onChargingChanged() {
            page.syncChargeIfClean();
        }

        function onFnLockChanged() {
            if (!page.fnLockSubmitting)
                page.syncFnLock();
        }
    }

    Connections {
        target: AdminState

        function onSaveSettingsErrorChanged() {
            // Le worker remet le champ à vide au début d'un enregistrement :
            // seul un code non vide est un refus.
            if (AdminState.saveSettingsError === "")
                return;
            // Refus : les interrupteurs reviennent aux valeurs lues, mais un
            // brouillon de charge est conservé pour permettre de corriger.
            page.syncSettings();
            // Ne pas révoquer le consentement optimiste pendant l'envoi : la
            // valeur relue ne revient qu'une fois l'écriture terminée.
            if (!AdminState.savingSettings)
                page.ppdOverrideDraft = SettingsData.ppdOverride;
            if (!AdminState.savingSettings)
                page.syncFnLock();
            page.fnLockSubmitting = false;
            page.chargeSubmitting = false;
            page.matrixSubmitting = false;
            if (AdminState.saveSettingsError === "Conflict")
                page.settingsConflict = true;
        }

        function onSaveSettingsSucceededChanged() {
            if (!AdminState.saveSettingsSucceeded)
                return;
            page.syncSettings();
            // Après un enregistrement de charge, on relit la vérité du daemon.
            if (page.chargeSubmitting)
                page.syncCharge();
            if (page.matrixSubmitting)
                page.syncMatrix();
            page.ppdOverrideDraft = SettingsData.ppdOverride;
            page.syncFnLock();
            page.fnLockSubmitting = false;
            page.chargeSubmitting = false;
            page.matrixSubmitting = false;
            page.settingsConflict = false;
            AppState.notify(Strings.t("toast.settingsSaved"), "ok");
        }
    }

    contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        title: Strings.t("settings.title")
        subtitle: Strings.t("settings.subtitle")
        iconName: "settings"

        StatusBadge {
            kind: LiveData.connected ? "ok" : "warn"
            label: LiveData.connected ? Strings.t("state.connected")
                                      : Strings.t("app.demo")
        }
    }

    // ── Bandeaux des réglages persistants ───────────────────────────────────
    InlineNotice {
        Layout.fillWidth: true
        visible: !AdminState.available
        kind: "warn"
        title: Strings.t("admin.unavailable.title")
        body: AdminState.availabilityText
        actionText: Strings.t("common.retry")
        onActionClicked: AppState.requestAdminRefresh()
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: SettingsData.connected && SettingsData.error !== ""
        kind: "error"
        title: Strings.t("settings.readError")
        body: Strings.t("settings.readErrorHint") + " " + SettingsData.error
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: page.settingsConflict
        kind: "warn"
        title: Strings.t("settings.conflictTitle")
        body: Strings.t("settings.conflictBody")
        actionText: Strings.t("editor.reload")
        onActionClicked: page.reloadSettings()
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: AdminState.savingSettings
        kind: "info"
        title: Strings.t("settings.saving")
        body: Strings.t("settings.savingHint")
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: AdminState.saveSettingsErrorText !== ""
                 && !AdminState.savingSettings
        kind: "error"
        title: AdminState.saveSettingsErrorText
        body: AdminState.saveSettingsErrorDetailText !== ""
              ? Strings.t("editor.errorField") + " " + AdminState.saveSettingsErrorDetailText
              : Strings.t("editor.errorKept")
    }

    // ── Daemon et agent ─────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.daemon")
            hint: Strings.t("settings.daemonHint")
            iconName: "gear"

            StatusBadge {
                kind: LiveData.connected ? "ok" : "warn"
                label: LiveData.connected ? Strings.t("state.connected")
                                          : Strings.t("state.disconnected")
            }
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("settings.dataSource")
            value: LiveData.isLive ? Strings.t("app.live") : Strings.t("app.demo")
            iconName: "allprofiles"
            badgeLabel: LiveData.usingDemo ? Strings.t("common.demoTag") : ""
            badgeKind: "warn"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("settings.daemonProfiles")
            value: Strings.formatNumber(LiveData.profiles.length, 0)
            iconName: "allprofiles"
            badgeLabel: LiveData.parseError !== "" ? Strings.t("profiles.sourceError") : ""
            badgeKind: "error"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("dash.ppd")
            value: page.ppdLabel()
            iconName: "power"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("dash.power")
            value: page.powerLabel()
            iconName: LiveData.power === "ac" ? "pluggedin" : "batterymode"
        }

        InfoRow {
            Layout.fillWidth: true
            visible: LiveData.lastError !== ""
            label: Strings.t("settings.daemonError")
            value: LiveData.lastError
            iconName: "help"
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s12

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: Strings.t("settings.daemonRetryHint")
            }

            AppButton {
                text: Strings.t("common.retry")
                variant: "secondary"
                small: true
                enabled: !LiveData.commandBusy
                onClicked: AppState.requestRefresh()
            }
        }
    }

    // ── Apparence ───────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.appearance")
            hint: Strings.t("settings.appearanceHint")
            iconName: "theme"
        }

        Item {
            Layout.fillWidth: true
            implicitHeight: 40

            RowLayout {
                anchors.fill: parent
                spacing: Theme.s16

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: Strings.t("settings.theme")
                }

                SegmentedControl {
                    options: [
                        { "label": Strings.t("theme.system") },
                        { "label": Strings.t("theme.light") },
                        { "label": Strings.t("theme.dark") }
                    ]
                    currentIndex: Math.max(0, ["system", "light", "dark"].indexOf(AppState.themeMode))
                    onActivated: function(index) {
                        AppState.setThemeMode(["system", "light", "dark"][index]);
                    }
                }
            }
        }

        Item {
            Layout.fillWidth: true
            implicitHeight: 40

            RowLayout {
                anchors.fill: parent
                spacing: Theme.s16

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: Strings.t("settings.language")
                }

                SegmentedControl {
                    options: [
                        { "label": "Français" },
                        { "label": "English" },
                        { "label": "Deutsch" }
                    ]
                    currentIndex: Math.max(0, page.languageCodes.indexOf(AppState.language))
                    onActivated: function(index) {
                        AppState.setLanguage(page.languageCodes[index]);
                    }
                }
            }
        }

        Item {
            Layout.fillWidth: true
            implicitHeight: 40

            RowLayout {
                anchors.fill: parent
                spacing: Theme.s16

                AppLabel {
                    Layout.fillWidth: true
                    variant: "body"
                    text: Strings.t("settings.units")
                }

                SegmentedControl {
                    id: unitControl
                    compact: true
                    enabled: page.settingsEditable()
                    options: [
                        { "label": "°C" },
                        { "label": "°F" }
                    ]
                    currentIndex: page.fahrenheit ? 1 : 0
                    onActivated: function(index) {
                        page.setFahrenheit(index === 1);
                    }
                }
            }
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("settings.reduceMotion")
            description: Strings.t("settings.reduceMotionHint")
            checked: AppState.reducedMotion
            onToggled: function(checked) { AppState.setReducedMotion(checked); }
        }
    }

    // ── Contrôle matériel ───────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.hardware")
            hint: Strings.t("settings.hardwareHint")
            iconName: "gear"
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("settings.cpuControl")
            description: Strings.t("settings.cpuControlHint")
            supported: AdminState.available
            unsupportedReason: Strings.t("settings.adminRequired")
            enabled: !AdminState.savingSettings
            checked: page.cpuEnabled
            onToggled: function(checked) {
                page.cpuEnabled = checked;
                page.submitSettings();
            }
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("settings.fanControl")
            description: Strings.t("settings.fanControlHint")
            supported: AdminState.available
            unsupportedReason: Strings.t("settings.adminRequired")
            enabled: !AdminState.savingSettings
            checked: page.fanEnabled
            onToggled: function(checked) {
                page.fanEnabled = checked;
                page.submitSettings();
            }
        }

        ToggleRow {
            objectName: "settingsLightingToggle"
            Layout.fillWidth: true
            visible: page.lightingVisible
            label: Strings.t("settings.lightingControl")
            description: HardwareData.lighting.available
                         ? Strings.t("settings.lightingKind") + " "
                           + (HardwareData.lighting.keyboard.kind !== "none"
                              ? HardwareData.lighting.keyboard.kind
                              : Strings.t("common.unavailable"))
                         : Strings.t("settings.lightingUnavailable")
            supported: AdminState.available
            unsupportedReason: Strings.t("settings.adminRequired")
            enabled: !AdminState.savingSettings
            checked: page.lightingEnabled
            onToggled: function(checked) {
                page.lightingEnabled = checked;
                page.submitSettings();
            }
        }

        ToggleRow {
            objectName: "settingsFnLockToggle"
            Layout.fillWidth: true
            visible: page.fnLockVisible && HardwareData.fnLock.writable
                     && HardwareData.fnLock.enabledKnown
            label: Strings.t("settings.fnLock")
            description: Strings.t("settings.fnLockHint")
            checked: page.fnLockDraft
            supported: page.settingsEditable() && HardwareData.fnLock.enabledKnown
            unsupportedReason: !HardwareData.fnLock.enabledKnown
                               ? Strings.t("settings.fnLockUnknown")
                               : Strings.t("settings.adminRequired")
            onToggled: function(checked) { page.submitFnLock(checked); }
        }

        InfoRow {
            objectName: "settingsFnLockRow"
            Layout.fillWidth: true
            visible: page.fnLockVisible
                     && (!HardwareData.fnLock.writable || !HardwareData.fnLock.enabledKnown)
            label: Strings.t("settings.fnLock")
            value: HardwareData.fnLock.enabledKnown
                   ? (HardwareData.fnLock.enabled ? Strings.t("common.enabled")
                                                  : Strings.t("common.disabled"))
                   : Strings.t("common.unavailable")
            iconName: "gear"
            badgeLabel: HardwareData.fnLock.writable ? "" : Strings.t("common.readOnly")
            badgeKind: "neutral"
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("settings.hardwareNote")
        }
    }

    // ── Bascule automatique ─────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.switching")
            hint: Strings.t("settings.switchingHint")
            iconName: "power"

            StatusBadge {
                id: matrixStateBadge
                kind: page.matrixEditable() ? "ok" : "neutral"
                label: page.matrixEditable() ? Strings.t("common.editable")
                                             : Strings.t("common.readOnly")

                // L'explication PPD reste accessible, sans paragraphe permanent.
                HoverHandler { id: matrixStateHover }
                AppToolTip {
                    visible: matrixStateHover.hovered
                    text: Strings.t("settings.switchingNote")
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Theme.s8

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s12

                Item { Layout.preferredWidth: 130 }

                AppLabel {
                    Layout.fillWidth: true
                    variant: "caption"
                    secondary: true
                    text: Strings.t("dash.ac")
                }
                AppLabel {
                    Layout.fillWidth: true
                    variant: "caption"
                    secondary: true
                    text: Strings.t("dash.battery")
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s12

                AppLabel {
                    Layout.preferredWidth: 130
                    variant: "small"
                    text: Strings.t("settings.ppdSaver")
                }

                MatrixSelectCell {
                    objectName: "matrixPowerSaverAc"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdSaver") + " · " + Strings.t("dash.ac")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("power-saver", "ac")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("power-saver", "ac")
                    onSelected: function(index) {
                        page.setMatrixCell("power-saver", "ac",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }

                MatrixSelectCell {
                    objectName: "matrixPowerSaverBat"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdSaver") + " · " + Strings.t("dash.battery")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("power-saver", "battery")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("power-saver", "battery")
                    onSelected: function(index) {
                        page.setMatrixCell("power-saver", "battery",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s12

                AppLabel {
                    Layout.preferredWidth: 130
                    variant: "small"
                    text: Strings.t("settings.ppdBalanced")
                }

                MatrixSelectCell {
                    objectName: "matrixBalancedAc"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdBalanced") + " · " + Strings.t("dash.ac")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("balanced", "ac")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("balanced", "ac")
                    onSelected: function(index) {
                        page.setMatrixCell("balanced", "ac",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }

                MatrixSelectCell {
                    objectName: "matrixBalancedBat"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdBalanced") + " · " + Strings.t("dash.battery")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("balanced", "battery")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("balanced", "battery")
                    onSelected: function(index) {
                        page.setMatrixCell("balanced", "battery",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.s12

                AppLabel {
                    Layout.preferredWidth: 130
                    variant: "small"
                    text: Strings.t("settings.ppdPerformance")
                }

                MatrixSelectCell {
                    objectName: "matrixPerformanceAc"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdPerformance") + " · " + Strings.t("dash.ac")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("performance", "ac")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("performance", "ac")
                    onSelected: function(index) {
                        page.setMatrixCell("performance", "ac",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }

                MatrixSelectCell {
                    objectName: "matrixPerformanceBat"
                    Layout.fillWidth: true
                    label: Strings.t("settings.ppdPerformance") + " · " + Strings.t("dash.battery")
                    options: page.matrixOptionLabels()
                    selectedIndex: page.matrixOptionIndex("performance", "battery")
                    editable: page.matrixEditable()
                    invalid: page.matrixCellInvalid("performance", "battery")
                    onSelected: function(index) {
                        page.setMatrixCell("performance", "battery",
                                           index === 0 ? "" : LiveData.profiles[index - 1].id);
                    }
                }
            }
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("settings.matrixFallbackHint")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: page.matrixEditable() || page.matrixDirty()

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: page.matrixDirty() && !page.matrixValid()
                      ? Strings.t("settings.matrixFixHint")
                      : (page.matrixDirty() ? Strings.t("settings.matrixDirtyHint") : "")
                color: page.matrixDirty() && !page.matrixValid()
                       ? Theme.error : Theme.textSecondary
            }

            AppButton {
                objectName: "matrixResetButton"
                text: Strings.t("common.reset")
                variant: "secondary"
                enabled: page.matrixDirty() && !AdminState.savingSettings
                onClicked: page.resetMatrix()
            }

            AppButton {
                objectName: "matrixApplyButton"
                text: Strings.t("common.apply")
                enabled: page.matrixDirty() && page.matrixValid()
                         && page.matrixEditable()
                onClicked: page.submitMatrix()
            }
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("settings.switchingManual")
            value: LiveData.isLive
                   ? (LiveData.manualOverride && LiveData.activeName !== ""
                      ? LiveData.activeName : Strings.t("common.none"))
                   : (AppState.manualProfileId !== ""
                      ? page.manualProfileName() : Strings.t("common.none"))
            iconName: "power"
        }
    }

    // ── Charge ──────────────────────────────────────────────────────────────
    AppCard {
        objectName: "chargingCard"
        Layout.fillWidth: true
        visible: page.chargingVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.charging")
            hint: Strings.t("settings.chargingHint")
            iconName: "batterymode"

            StatusBadge {
                kind: HardwareData.charging.available ? "ok" : "neutral"
                label: HardwareData.charging.available
                       ? (page.chargeEditable() ? Strings.t("settings.chargeEditable")
                                                : Strings.t("common.readOnly"))
                       : Strings.t("common.unavailable")
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: !HardwareData.charging.available
            variant: "small"
            secondary: true
            text: Strings.t("settings.chargeUnavailable")
        }

        // Profil de charge.
        ChargeSelectRow {
            objectName: "chargeProfileRow"
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.profiles.length > 0
            label: Strings.t("settings.chargeProfile")
            options: HardwareData.charging.profiles
            currentIndex: page.chargeProfileIndex()
            editable: page.chargeEditable()
            invalid: page.chargeProfileInvalid
            errorText: Strings.t("settings.chargeInvalidValue")
            onSelected: function(index) {
                page.chargeProfile = HardwareData.charging.profiles[index];
            }
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.profiles.length === 0
            label: Strings.t("settings.chargeProfile")
            value: page.chargeLabel(HardwareData.charging.profile)
            iconName: "batterymode"
        }

        // Priorité de charge.
        ChargeSelectRow {
            objectName: "chargePriorityRow"
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.priorities.length > 0
            label: Strings.t("settings.chargePriority")
            options: HardwareData.charging.priorities
            currentIndex: page.chargePriorityIndex()
            editable: page.chargeEditable()
            invalid: page.chargePriorityInvalid
            errorText: Strings.t("settings.chargeInvalidValue")
            onSelected: function(index) {
                page.chargePriority = HardwareData.charging.priorities[index];
            }
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.priorities.length === 0
            label: Strings.t("settings.chargePriority")
            value: page.chargeLabel(HardwareData.charging.priority)
            iconName: "power"
        }

        // Seuils début/fin (éditables si le matériel expose les listes).
        ChargeSelectRow {
            objectName: "chargeStartRow"
            Layout.fillWidth: true
            visible: page.chargeThresholdsEditable()
            label: Strings.t("settings.chargeStart")
            options: page.thresholdLabels(HardwareData.charging.startThresholds)
            currentIndex: page.chargeStartIndex()
            editable: page.chargeEditable()
            invalid: page.chargeStartInvalid || page.chargeRangeInvalid
            errorText: page.chargeRangeInvalid
                       ? Strings.t("settings.chargeRangeError")
                       : Strings.t("settings.chargeInvalidValue")
            onSelected: function(index) {
                page.chargeStart = HardwareData.charging.startThresholds[index];
            }
        }

        ChargeSelectRow {
            objectName: "chargeEndRow"
            Layout.fillWidth: true
            visible: page.chargeThresholdsEditable()
            label: Strings.t("settings.chargeEnd")
            options: page.thresholdLabels(HardwareData.charging.endThresholds)
            currentIndex: page.chargeEndIndex()
            editable: page.chargeEditable()
            invalid: page.chargeEndInvalid || page.chargeRangeInvalid
            errorText: page.chargeRangeInvalid
                       ? Strings.t("settings.chargeRangeError")
                       : Strings.t("settings.chargeInvalidValue")
            onSelected: function(index) {
                page.chargeEnd = HardwareData.charging.endThresholds[index];
            }
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.thresholdsAvailable
                     && !page.chargeThresholdsEditable()
            label: Strings.t("settings.chargeStart")
            value: page.thresholdLabel(HardwareData.charging.start)
            iconName: "batterymode"
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.thresholdsAvailable
                     && !page.chargeThresholdsEditable()
            label: Strings.t("settings.chargeEnd")
            value: page.thresholdLabel(HardwareData.charging.end)
            iconName: "batterymode"
        }

        ChargeSelectRow {
            objectName: "chargeTypeRow"
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.chargeTypeWritable
            label: Strings.t("settings.chargeType")
            options: page.chargeTypeLabels()
            currentIndex: page.chargeTypeIndex()
            editable: page.chargeEditable()
            invalid: page.chargeTypeInvalid
            errorText: Strings.t("settings.chargeInvalidValue")
            onSelected: function(index) {
                page.chargeTypeDraft = page.chargeTypeOptions()[index];
            }
        }

        InfoRow {
            objectName: "chargeTypeCurrentRow"
            Layout.fillWidth: true
            visible: HardwareData.charging.available
                     && HardwareData.charging.chargeType !== ""
            label: Strings.t("settings.chargeTypeCurrent")
            value: HardwareData.charging.chargeType
            iconName: "gear"
            badgeLabel: HardwareData.charging.chargeTypeWritable
                        ? "" : Strings.t("common.readOnly")
            badgeKind: "neutral"
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: page.chargeEditable() || page.chargeDirty()

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                secondary: true
                wrapMode: Text.WordWrap
                text: page.chargeDirty() && !page.chargeValid()
                      ? Strings.t("settings.chargeFixHint")
                      : (page.chargeDirty() ? Strings.t("settings.chargeDirtyHint") : "")
                color: page.chargeDirty() && !page.chargeValid()
                       ? Theme.error : Theme.textSecondary
            }

            AppButton {
                objectName: "chargeResetButton"
                text: Strings.t("common.reset")
                variant: "secondary"
                enabled: page.chargeDirty() && !AdminState.savingSettings
                onClicked: page.resetCharge()
            }

            AppButton {
                objectName: "chargeApplyButton"
                text: Strings.t("common.apply")
                enabled: page.chargeDirty() && page.chargeValid()
                         && page.chargeEditable()
                onClicked: page.submitCharge()
            }
        }
    }

    // ── Affichage / agent ───────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.display")
            hint: Strings.t("settings.displayHint")
            iconName: "monitor"

            StatusBadge {
                kind: AgentData.usable ? "ok"
                      : (AgentData.available ? "warn" : "neutral")
                label: AgentData.usable ? Strings.t("display.supported")
                     : AgentData.available ? Strings.t("display.unsupported")
                     : Strings.t("display.agentUnavailable")
            }
        }

        InfoRow {
            objectName: "settingsAgentStateRow"
            Layout.fillWidth: true
            label: Strings.t("settings.agentState")
            value: AgentData.available
                   ? (AgentData.dormant ? Strings.t("display.dormant")
                      : (AgentData.version !== "" ? AgentData.version
                                                 : Strings.t("display.agentActive")))
                   : Strings.t("display.agentUnavailable")
            iconName: "monitor"
            badgeLabel: AgentData.usable ? Strings.t("display.supported") : ""
            badgeKind: AgentData.usable ? "ok" : "warn"
        }

        InfoRow {
            objectName: "settingsAgentBackendRow"
            Layout.fillWidth: true
            visible: page.agentBackendLabel !== ""
            label: Strings.t("display.backend")
            value: page.agentBackendLabel
            iconName: "gear"
        }

        InfoRow {
            objectName: "settingsAgentOutputRow"
            Layout.fillWidth: true
            visible: page.agentOutputLabel !== ""
            label: Strings.t("display.output")
            value: page.agentOutputLabel
            iconName: "monitor"
        }

        InfoRow {
            objectName: "settingsAgentCurrentRow"
            Layout.fillWidth: true
            visible: AgentData.currentModeLabel !== ""
            label: Strings.t("display.current")
            value: AgentData.currentModeLabel
            iconName: "speedometer"
        }

        InfoRow {
            objectName: "settingsAgentModesRow"
            Layout.fillWidth: true
            visible: AgentData.modes.length > 0
            label: Strings.t("display.available")
            value: Strings.tf("display.modeCount", [AgentData.modes.length])
            iconName: "monitor"
        }

        // Repli DRM quand l'agent ne fournit pas de sortie : information
        // réelle du noyau, jamais un mode inventé.
        InfoRow {
            objectName: "settingsInternalDisplayRow"
            Layout.fillWidth: true
            visible: page.agentOutputLabel === ""
                     && HardwareData.internalDisplayLabel !== ""
            label: Strings.t("display.output")
            value: HardwareData.internalDisplayLabel
            iconName: "monitor"
            badgeLabel: HardwareData.displayInfo.source !== ""
                        ? HardwareData.displayInfo.source : ""
            badgeKind: "neutral"
        }

        InlineNotice {
            objectName: "settingsAgentNotice"
            Layout.fillWidth: true
            visible: !AgentData.usable
            kind: AgentData.available ? "warn" : "info"
            title: AgentData.available ? Strings.t("display.agentUnsupported")
                                       : Strings.t("display.agentUnavailable")
            body: AgentData.errorText !== "" ? AgentData.errorText
                  : (AgentData.displayErrorText !== "" ? AgentData.displayErrorText
                     : Strings.t("display.agentUsableHint"))
            actionText: Strings.t("display.refreshAgent")
            onActionClicked: AppState.requestAgentRefresh()
        }

        InlineNotice {
            objectName: "trayUnavailableNotice"
            Layout.fillWidth: true
            visible: page.trayUnavailable
            kind: "warn"
            title: Strings.t("tray.unavailable")
            body: Strings.t("tray.unavailableHint")
        }

        ToggleRow {
            id: powerBlockerRow
            objectName: "powerBlockerRow"
            Layout.fillWidth: true
            label: Strings.t("settings.powerBlocker")
            description: Strings.t("settings.powerBlockerHint")
            checked: page.powerBlockerDraft
            supported: PowerBlockerData.editable
            unsupportedReason: PowerBlockerData.pending
                               ? Strings.t("settings.powerBlockerPending")
                               : Strings.t("settings.powerBlockerNotGraphical")
            onToggled: function(checked) {
                page.powerBlockerDraft = checked;
                AppState.requestPowerBlocker(checked);
            }

            // Durée de vie de l'inhibiteur : en infobulle, pas en paragraphe.
            HoverHandler { id: powerBlockerHover }
            AppToolTip {
                visible: powerBlockerHover.hovered
                text: Strings.t("settings.powerBlockerNote")
            }
        }

        InlineNotice {
            objectName: "powerBlockerError"
            Layout.fillWidth: true
            visible: PowerBlockerData.hasError
            kind: "error"
            title: Strings.t("settings.powerBlockerErrorTitle")
            body: PowerBlockerData.errorText
        }
    }

    // ── PRIME ───────────────────────────────────────────────────────────────
    AppCard {
        id: primeCard
        objectName: "primeCard"
        Layout.fillWidth: true
        visible: page.primeVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.graphics")
            hint: Strings.t("settings.graphicsHint")
            iconName: "gpu"
        }

        InfoRow {
            Layout.fillWidth: true
            label: Strings.t("settings.graphicsMode")
            value: page.primeLabel(HardwareData.prime.mode)
            iconName: "gpu"
            badgeLabel: HardwareData.prime.supported ? Strings.t("common.readOnly")
                                                     : Strings.t("common.unavailable")
            badgeKind: HardwareData.prime.supported ? "neutral" : "warn"
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.prime.requiresOffloading
            label: Strings.t("settings.graphicsOffload")
            value: Strings.t("common.yes")
            iconName: "gear"
        }
    }

    // ── ODM / TDP / cTGP (Admin.Hw, transitoire) + YCbCr (lecture) ──────────
    AppCard {
        objectName: "capabilitiesCard"
        Layout.fillWidth: true
        visible: page.capabilitiesVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.capabilities")
            hint: Strings.t("settings.capabilitiesHint")
            iconName: "power"

            StatusBadge {
                kind: AdminHwData.available ? "warn" : "neutral"
                label: AdminHwData.available ? Strings.t("settings.hwTransient")
                                             : Strings.t("common.readOnly")
            }
        }

        InlineNotice {
            objectName: "hwServiceMissingNotice"
            Layout.fillWidth: true
            visible: page.hwServiceMissing
            kind: "warn"
            title: Strings.t("settings.hwServiceMissing")
            body: AdminHwData.availabilityText
            actionText: Strings.t("common.retry")
            onActionClicked: AppState.requestAdminRefresh()
        }

        // ODM
        RowLayout {
            Layout.fillWidth: true
            visible: page.odmEditing

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                text: Strings.t("settings.odm")
            }

            AppSelect {
                id: odmSelect
                Layout.preferredWidth: 250
                model: page.odmOptions()
                textRole: "name"
                enabled: page.hwEditable && HardwareData.odmWritable
                Accessible.name: Strings.t("settings.odm")
                onActivated: function(index) {
                    page.odmDraftIndex = index;
                    page.odmError = "";
                }
            }

            Binding {
                target: odmSelect
                property: "currentIndex"
                value: page.odmDraftIndex
            }

            StatusBadge {
                visible: HardwareData.odmSourceKind !== ""
                kind: "neutral"
                label: HardwareData.odmSourceKind
            }

            StatusBadge {
                visible: HardwareData.odm.platform.owner !== ""
                kind: page.odmPpdOwned ? "warn" : "neutral"
                label: page.odmPpdOwned ? Strings.t("settings.hwPpdOwner")
                                        : HardwareData.odm.platform.owner
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.odmEditing && page.odmPpdOwned
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("settings.hwPpdOwnedHint")
        }

        AppLabel {
            objectName: "odmError"
            Layout.fillWidth: true
            visible: page.odmError !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: page.odmError !== "" ? Strings.t(page.odmError) : ""
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8
            visible: page.odmEditing

            Item { Layout.fillWidth: true }

            AppButton {
                objectName: "odmApply"
                text: Strings.t("common.apply")
                enabled: page.hwEditable && HardwareData.odmWritable
                         && page.odmDirty() && !page.odmPpdOwned
                onClicked: page.submitOdm(false)
            }

            AppButton {
                objectName: "odmForce"
                visible: page.odmPpdOwned
                text: Strings.t("settings.hwForce")
                variant: "secondary"
                enabled: page.hwEditable && HardwareData.odmWritable
                         && page.odmDirty() && page.ppdOverrideDraft
                         && !AdminState.savingSettings
                onClicked: page.submitOdm(true)
            }
        }

        ToggleRow {
            objectName: "ppdOverrideToggle"
            Layout.fillWidth: true
            visible: page.odmEditing && HardwareData.odm.platform.available
            label: Strings.t("settings.ppdOverride")
            description: Strings.t("settings.ppdOverrideHint")
            supported: page.settingsEditable()
            unsupportedReason: Strings.t("settings.adminRequired")
            checked: page.ppdOverrideDraft
            onToggled: function(checked) { page.submitPpdOverride(checked); }
        }

        InlineNotice {
            Layout.fillWidth: true
            visible: page.odmEditing && HardwareData.odm.platform.available
                     && page.ppdOverrideDraft
            kind: "warn"
            title: Strings.t("settings.ppdOverride")
            body: Strings.t("settings.ppdOverrideWarning")
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.divider
            visible: page.odmEditing && (page.tdpEditing || page.ctgpEditing)
        }

        // TDP
        AppLabel {
            Layout.fillWidth: true
            visible: page.tdpEditing
            variant: "body"
            strong: true
            text: Strings.t("settings.tdp")
        }

        AppLabel {
            objectName: "tdpCountError"
            Layout.fillWidth: true
            visible: page.tdpEditing && !page.tdpCountValid()
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t("settings.tdpDescriptorError")
        }

        Repeater {
            model: HardwareData.odm.tdp

            delegate: TextFieldRow {
                required property var modelData
                required property int index

                Layout.fillWidth: true
                visible: page.tdpEditing
                objectName: "tdpField_" + modelData.descriptor
                label: Strings.t("settings.tdp") + " " + modelData.descriptor
                description: Strings.formatNumber(modelData.min, 0) + "–"
                             + Strings.formatNumber(modelData.max, 0) + " W"
                text: page.tdpValue(index)
                inputMethodHints: Qt.ImhFormattedNumbersOnly
                invalid: page.tdpFieldInvalid(index)
                errorText: Strings.t("settings.tdpRange")
                onEdited: function(value) { page.setTdpDraft(index, value); }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: page.tdpEditing
            spacing: Theme.s8

            AppLabel {
                Layout.fillWidth: true
                variant: "caption"
                color: page.tdpValid() ? Theme.textSecondary : Theme.error
                wrapMode: Text.WordWrap
                text: page.tdpValid() ? ""
                      : (page.tdpMonotonicInvalid
                         ? Strings.t("settings.tdpMonotonicError")
                         : Strings.t(page.tdpError !== ""
                                     ? page.tdpError : "settings.tdpRange"))
            }

            AppButton {
                objectName: "tdpApply"
                text: Strings.t("common.apply")
                enabled: page.hwEditable && page.tdpDirty() && page.tdpValid()
                onClicked: page.submitTdp()
            }
        }

        // cTGP
        RowLayout {
            Layout.fillWidth: true
            visible: page.ctgpEditing
            spacing: Theme.s8

            TextFieldRow {
                Layout.fillWidth: true
                objectName: "ctgpField"
                label: Strings.t("settings.ctgpOffset")
                description: Strings.tf("settings.hwCtgpRange", [HardwareData.ctgpMax])
                text: page.ctgpDraft
                inputMethodHints: Qt.ImhFormattedNumbersOnly
                invalid: !page.ctgpValid()
                errorText: Strings.tf("settings.hwCtgpRange", [HardwareData.ctgpMax])
                onEdited: function(value) { page.ctgpDraft = value; }
            }

            AppButton {
                objectName: "ctgpApply"
                text: Strings.t("common.apply")
                enabled: page.hwEditable && page.ctgpDirty() && page.ctgpValid()
                onClicked: page.submitCtgp()
            }
        }

        // YCbCr reste une capacité en lecture seule.
        InfoRow {
            objectName: "settingsYcbcrRow"
            Layout.fillWidth: true
            visible: page.ycbcrVisible
            label: Strings.t("settings.ycbcr")
            value: HardwareData.ycbcr.available
                   ? Strings.tf("settings.ycbcrPorts", [HardwareData.ycbcr.ports.length])
                   : Strings.t("common.unavailable")
            iconName: "monitor"
            badgeLabel: HardwareData.ycbcr.pendingModeReapply
                        ? Strings.t("settings.ycbcrPending") : ""
            badgeKind: "warn"
        }

        InlineNotice {
            Layout.fillWidth: true
            visible: AdminHwData.pending
            kind: "info"
            title: Strings.t("settings.hwApplying")
            body: Strings.t("settings.hwApplyingHint")
        }

        InlineNotice {
            objectName: "adminHwError"
            Layout.fillWidth: true
            visible: AdminHwData.hasSaveError && !AdminHwData.pending
            kind: "error"
            title: AdminHwData.saveErrorText
            body: AdminHwData.saveErrorDetailText !== ""
                  ? Strings.t("editor.errorField") + " " + AdminHwData.saveErrorDetailText
                  : Strings.t("settings.hwErrorKept")
        }
    }

    // ── Services ────────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("settings.services")
            hint: Strings.t("settings.servicesHint")
            iconName: "gear"
        }

        ToggleRow {
            objectName: "autostartRow"
            Layout.fillWidth: true
            label: Strings.t("settings.trayAutostart")
            description: Strings.t("settings.trayAutostartHint")
            checked: page.autostartDraft
            supported: AutostartData.editable
            // « Répertoire utilisateur introuvable » est réservé au code
            // `NoHome` : un simple état non résolu affiche « Indisponible ».
            unsupportedReason: AutostartData.hasError
                               ? AutostartData.errorText
                               : Strings.t("common.unavailable")
            onToggled: function(checked) {
                page.autostartDraft = checked;
                AppState.requestAutostart(checked);
                // Écriture synchrone côté Rust : en cas d'échec, l'état réel
                // n'a pas changé et l'interrupteur revient immédiatement.
                page.autostartDraft = AutostartData.enabled;
            }
        }

        InlineNotice {
            objectName: "autostartError"
            Layout.fillWidth: true
            visible: AutostartData.hasError
            kind: "error"
            title: Strings.t("settings.autostartErrorTitle")
            body: AutostartData.errorText
        }
    }

    Connections {
        target: AutostartData

        function onEnabledChanged() {
            page.autostartDraft = AutostartData.enabled;
        }
    }

    Connections {
        target: PowerBlockerData

        function onBlockedChanged() {
            if (!PowerBlockerData.pending)
                page.powerBlockerDraft = PowerBlockerData.blocked;
        }

        function onPendingChanged() {
            // Fin de commande : l'état réel (fd) fait foi, y compris après un
            // refus (l'interrupteur revient à l'état antérieur).
            if (!PowerBlockerData.pending)
                page.powerBlockerDraft = PowerBlockerData.blocked;
        }
    }

    Connections {
        target: HardwareData

        function onOdmChanged() { page.syncHwDraftsIfClean(); }
        function onNvidiaChanged() { page.syncHwDraftsIfClean(); }
    }

    Connections {
        target: AdminHwData

        function onAvailableChanged() { page.syncHwDraftsIfClean(); }

        function onSucceededChanged() {
            if (!AdminHwData.succeeded)
                return;
            page.syncHwDrafts();
            AppState.requestHardwareRefresh();
            AppState.notify(Strings.t("toast.hwApplied"), "ok");
        }
    }

    // Ligne de sélection de charge : `Binding` réapplique la valeur après une
    // synchronisation (un `currentIndex` lié serait rompu par l'interaction).
    component ChargeSelectRow: RowLayout {
        id: chargeRow

        property string label: ""
        property var options: []
        property int currentIndex: -1
        property bool editable: true
        property bool invalid: false
        property string errorText: ""

        signal selected(int index)

        spacing: Theme.s16

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            AppLabel {
                Layout.fillWidth: true
                variant: "body"
                text: chargeRow.label
                color: chargeRow.editable ? Theme.textPrimary : Theme.textMuted
            }

            AppLabel {
                Layout.fillWidth: true
                visible: chargeRow.invalid && chargeRow.errorText !== ""
                variant: "small"
                text: chargeRow.errorText
                color: Theme.error
                wrapMode: Text.WordWrap
            }
        }

        AppSelect {
            id: chargeSelect
            Layout.preferredWidth: 220
            model: chargeRow.options
            enabled: chargeRow.editable
            Accessible.name: chargeRow.label
            onActivated: function(index) { chargeRow.selected(index); }
        }

        Binding {
            target: chargeSelect
            property: "currentIndex"
            value: chargeRow.currentIndex
        }
    }

    // Cellule de matrice : `Binding` réapplique la sélection après une
    // resynchronisation (un `currentIndex` lié serait rompu par l'interaction).
    component MatrixSelectCell: ColumnLayout {
        id: matrixCell

        property string label: ""
        property var options: []
        property int selectedIndex: 0
        property bool editable: true
        property bool invalid: false

        signal selected(int index)

        spacing: 2

        AppSelect {
            id: matrixSelect
            Layout.fillWidth: true
            model: matrixCell.options
            enabled: matrixCell.editable
            Accessible.name: matrixCell.label
            onActivated: function(index) { matrixCell.selected(index); }
        }

        Binding {
            target: matrixSelect
            property: "currentIndex"
            value: matrixCell.selectedIndex
        }

        AppLabel {
            Layout.fillWidth: true
            visible: matrixCell.invalid
            variant: "small"
            text: Strings.t("settings.matrixInvalidProfile")
            color: Theme.error
            wrapMode: Text.WordWrap
        }
    }
}
