// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Éditeur de profil : sections Général, Affichage, CPU, Webcam, Ventilateurs,
// ODM/TDP/NVIDIA.
//
// Les champs éditables (nom, description, affichage, CPU, webcam,
// ventilateurs, ODM/TDP/cTGP) sont validés localement (`ProfileRules`) puis
// enregistrés via `Admin1.SaveProfilesJSON` par le pont Rust. Les presets
// restent en lecture seule. Tant que le service `tccd-admin` est absent, le
// formulaire est en lecture seule et un bandeau explique pourquoi.

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"
import "../core"

Item {
    id: page

    readonly property bool isNew: AppState.editingProfileId === "new"
    readonly property var source: isNew ? null : LiveData.profileById(AppState.editingProfileId)
    /// Profil preset : lecture seule, jamais envoyé comme personnalisé.
    readonly property bool isPreset: page.source !== null && page.source.preset === true
    /// Édition impossible (preset ou service admin absent). La création est
    /// autorisée avec `Admin1` : l'ID est généré côté Rust à l'enregistrement.
    readonly property bool readOnly: page.isPreset || !AdminState.available
    /// Tous les champs du profil sont modifiables (ODM/TDP/cTGP inclus).
    readonly property bool editable: !page.readOnly
    /// Profil live modifié par un autre client pendant l'édition.
    property bool conflict: false
    /// Signature de la source chargée (détection de conflit). Les champs
    /// d'assignation (`assignedAc`/`assignedBat`) sont exclus : la matrice
    /// peut changer sans que le profil lui-même soit modifié.
    property string loadedSignature: ""

    property bool dirty: false
    property string profileName: ""
    property string profileDescription: ""

    property bool useBrightness: false
    property real brightness: 100
    property bool useRefresh: false
    property int refreshIndex: 0
    property bool useResolution: false
    property int resolutionIndex: 0

    property int cores: 8
    property bool coresKnown: true
    /// Nombre de cœurs réel exposé par le daemon, sinon la valeur du profil
    /// (ou 1) : jamais une constante produit inventée.
    readonly property int cpuMaxCores: {
        if (HardwareData.cpuCaps.totalCores > 0)
            return HardwareData.cpuCaps.totalCores;
        return Math.max(1, page.cores);
    }
    /// Gouverneur `performance` réellement disponible. Liste vide = capacité
    /// non exposée, on laisse le toggle utilisable sans inventer de liste.
    readonly property bool performanceGovernorKnown: {
        var list = HardwareData.cpuCaps.availableGovernors;
        return list.length === 0 || list.indexOf("performance") >= 0;
    }
    property bool maxPerfGov: false
    property real minFreq: 0.8
    property bool freqMinKnown: true
    property real maxFreq: 3.5
    property bool freqMaxKnown: true
    property bool freqMaxReduced: false
    /// Valeur EPP sélectionnée pour ce profil (jamais un index partagé entre
    /// profils : c'est la valeur qui part dans le DTO).
    property string eppValue: "balance_performance"
    property bool noTurbo: false

    property bool webcamOn: true
    property bool useWebcam: true

    property int fanProfileIndex: 2
    property bool fanUseControl: true
    property real fanMin: 0
    property real fanMax: 100
    property real fanOffset: 0
    /// Courbe CPU du brouillon (points { temp, speed }). Le GPU n'est pas
    /// éditable ; le Rust préserve nom et `tableGPU` existants.
    property var fanCurve: []
    // Visibilité matérielle (absence confirmée seulement si snapshot sûr).
    readonly property bool webcamVisible: HardwareData.visibleIfAvailable(
        HardwareData.webcam.switchAvailable
        || HardwareData.webcam.devices.length > 0)
    readonly property bool odmCardVisible: HardwareData.visibleIfAvailable(
        HardwareData.odm.available || HardwareData.odm.tdpAvailable
        || HardwareData.nvidia.available)
    /// ODM/TDP/cTGP du profil : `None` tant que l'utilisateur ne modifie pas
    /// le champ ; la sauvegarde ne réécrit donc pas les valeurs existantes.
    property int odmProfileIndex: 0
    property int odmInitialIndex: 0
    property var tdpDraft: []
    property var tdpInitial: []
    property string ctgpDraft: "0"
    property string ctgpInitial: "0"
    readonly property bool tdpEditing: HardwareData.visibleIfAvailable(
        HardwareData.tdpExposed)
    readonly property bool ctgpEditing: HardwareData.visibleIfAvailable(
        HardwareData.nvidia.available)
    readonly property var odmEditChoices: {
        var list = HardwareData.odmChoices.slice();
        var stored = page.source && page.source.odm ? String(page.source.odm) : "";
        if (stored !== "" && list.indexOf(stored) < 0)
            list.push(stored);
        return list;
    }
    readonly property bool odmProfileDirty: page.odmProfileIndex
                                            !== page.odmInitialIndex

    /// Modes réellement exposés par l'agent de session (`User1`). Aucune
    /// combinaison absente n'est proposée.
    readonly property var resolutionOptions: AgentData.resolutions()
    readonly property bool agentUsable: AgentData.usable
    readonly property bool resolutionKnown: page.resolutionOptions.length > 0
    readonly property var selectedResolution: {
        if (!page.resolutionKnown)
            return null;
        var index = Math.max(0, Math.min(page.resolutionIndex,
                                         page.resolutionOptions.length - 1));
        return page.resolutionOptions[index];
    }
    /// Résolution utilisée pour filtrer les taux : celle demandée par le
    /// profil, sinon le mode courant de l'agent.
    readonly property var activeResolution: {
        if (page.useResolution && page.selectedResolution)
            return page.selectedResolution;
        if (AgentData.currentMode)
            return { "width": AgentData.currentMode.width,
                     "height": AgentData.currentMode.height };
        return page.selectedResolution;
    }
    readonly property var activeRates: page.activeResolution
            ? AgentData.refreshRates(page.activeResolution.width,
                                     page.activeResolution.height) : []
    readonly property real selectedRefreshRate: {
        if (page.activeRates.length === 0)
            return 0;
        var index = Math.max(0, Math.min(page.refreshIndex, page.activeRates.length - 1));
        return page.activeRates[index];
    }
    readonly property bool resolutionExists: page.selectedResolution !== null
            && AgentData.resolutionExists(page.selectedResolution.width,
                                          page.selectedResolution.height)
    readonly property bool refreshExists: page.activeResolution !== null
            && page.selectedRefreshRate > 0
            && AgentData.modeExists(page.activeResolution.width,
                                    page.activeResolution.height,
                                    page.selectedRefreshRate)
    readonly property bool refreshSupported: page.agentUsable
                                             && page.activeRates.length > 0
    property bool resolutionMissing: false
    property bool refreshMissing: false

    // Agent indisponible/non supporté : les contrôles sont désactivés et les
    // valeurs déjà enregistrées dans le profil sont **préservées** (aucun
    // effacement silencieux, aucune liste inventée).
    readonly property real storedRefreshRate: page.source
            && page.source.refreshRate > 0 ? page.source.refreshRate : 0
    readonly property int storedWidth: page.source && page.source.width > 0
            ? page.source.width : 0
    readonly property int storedHeight: page.source && page.source.height > 0
            ? page.source.height : 0
    readonly property real effectiveRefreshRate: page.agentUsable
            ? page.selectedRefreshRate : page.storedRefreshRate
    readonly property bool effectiveUseRefresh: page.agentUsable
            ? page.useRefresh : (page.source && page.source.useRefRate === true)
    readonly property bool effectiveUseResolution: page.agentUsable
            ? page.useResolution : (page.source && page.source.useResolution === true)
    readonly property int effectiveWidth: page.agentUsable
            ? (page.selectedResolution ? page.selectedResolution.width : 0)
            : page.storedWidth
    readonly property int effectiveHeight: page.agentUsable
            ? (page.selectedResolution ? page.selectedResolution.height : 0)
            : page.storedHeight
    readonly property bool effectiveResolutionExists: page.agentUsable
            ? page.resolutionExists : page.effectiveWidth > 0 && page.effectiveHeight > 0
    readonly property bool effectiveRefreshExists: page.agentUsable
            ? page.refreshExists : page.storedRefreshRate > 0

    /// Formulaire courant, validé localement avant tout envoi.
    readonly property var form: ({
        "name": page.profileName,
        "description": page.profileDescription,
        "brightness": page.brightness,
        "useBrightness": page.useBrightness,
        "refreshRate": page.effectiveRefreshRate,
        "useRefreshRate": page.effectiveUseRefresh,
        "resolutionWidth": page.effectiveWidth,
        "resolutionHeight": page.effectiveHeight,
        "useResolution": page.effectiveUseResolution,
        "resolutionExists": page.effectiveResolutionExists
                            && !page.resolutionMissing,
        "refreshExists": page.effectiveRefreshExists
                         && !page.refreshMissing,
        "odmProfileDirty": page.odmProfileDirty,
        "odmProfileName": page.odmProfileName(),
        "tdpDirty": page.tdpDirty(),
        "tdpValues": page.tdpValuesArray(),
        "ctgpDirty": page.ctgpDirty(),
        "ctgpOffset": page.ctgpNumber(),
        "ctgpMax": HardwareData.ctgpMax,
        "cores": page.cores,
        "coresKnown": page.coresKnown,
        "freqMin": page.minFreq,
        "freqMinKnown": page.freqMinKnown,
        "freqMax": page.maxFreq,
        "freqMaxKnown": page.freqMaxKnown,
        "freqMaxReduced": page.freqMaxReduced,
        "maxPerfGov": page.maxPerfGov,
        "epp": page.eppValue !== "" ? page.eppValue : "balance_performance",
        "noTurbo": page.noTurbo,
        "webcamOn": page.webcamOn,
        "useWebcam": page.useWebcam,
        "fanUseControl": page.fanUseControl,
        "fanProfile": page.fanProfileKeys[Math.max(0, page.fanProfileIndex)],
        "fanMin": page.fanMin,
        "fanMax": page.fanMax,
        "fanOffset": page.fanOffset,
        "fanCurve": page.fanCurve
    })
    readonly property var validation: ProfileRules.validate(page.form, page.otherNames())
    readonly property var errors: page.validation.errors
    readonly property bool valid: page.validation.ok

    readonly property bool nameInvalid: page.errors.name !== ""
    readonly property string displayError: ProfileRules.firstError(
        page.errors, ["brightness", "resolution", "refreshRate"])
    readonly property string cpuError: ProfileRules.firstError(
        page.errors, ["cores", "freqMin", "freqMax"])
    readonly property string fanError: ProfileRules.firstError(
        page.errors, ["fanMin", "fanMax", "fanOffset", "fanCurve"])
    readonly property string odmError: ProfileRules.firstError(
        page.errors, ["odmProfile", "tdpValues", "ctgpOffset"])

    /// Noms normalisés des autres profils (unicité).
    function otherNames() {
        var out = [];
        var list = LiveData.profiles;
        for (var i = 0; i < list.length; ++i) {
            if (list[i].id !== AppState.editingProfileId)
                out.push(ProfileRules.normalizedName(list[i].name));
        }
        return out;
    }

    function savePossible() {
        return page.dirty && page.valid && page.editable && AdminState.ready
                && !page.conflict;
    }

    function saveBlockedReason() {
        if (page.conflict)
            return Strings.t("editor.conflictBlocked");
        if (!AdminState.available)
            return Strings.t("admin.unavailable.missing");
        if (page.isPreset)
            return Strings.t("editor.presetReadOnlyHint");
        if (AdminState.saving)
            return Strings.t("editor.saving");
        if (!page.valid)
            return Strings.t("editor.invalidHint");
        return "";
    }
    readonly property var fanProfileKeys: ["silent", "quiet", "balanced", "performance", "custom", "auto"]
    /// Valeurs EPP réellement acceptées par le pilote, complétées par la
    /// valeur déjà enregistrée dans le profil si elle a disparu du matériel.
    /// Une liste vide → une seule entrée « Auto », sans invention.
    readonly property bool eppKnown: HardwareData.cpuCaps.availableEpp.length > 0
    readonly property var eppValues: page.eppValuesFor(page.eppValue)
    /// Index courant dérivé de la valeur : il ne peut pas désigner un autre
    /// profil quand la liste matérielle arrive après le chargement.
    readonly property int eppIndex: Math.max(0, page.eppValues.indexOf(page.eppValue))

    function eppValuesFor(value) {
        var list = HardwareData.cpuCaps.availableEpp.slice();
        var current = value === undefined || value === null ? "" : String(value);
        if (current !== "" && list.indexOf(current) < 0)
            list.push(current);
        // Aucune liste matérielle : une seule entrée qui conserve la valeur
        // par défaut de `tcc-core` (jamais une liste inventée).
        if (list.length === 0)
            list.push("balance_performance");
        return list;
    }

    /// Sélection utilisateur : met à jour la valeur et le brouillon.
    function selectEpp(index) {
        if (index < 0 || index >= page.eppValues.length)
            return;
        page.eppValue = String(page.eppValues[index]);
        page.touch();
    }

    Component.onCompleted: loadProfile()

    Connections {
        target: AppState
        function onEditingProfileIdChanged() {
            page.loadProfile();
        }
    }

    // Le payload live peut changer (reconnexion, autre client) : on recharge
    // seulement sans brouillon. Sinon on signale le conflit, sans jamais
    // écraser le formulaire.
    Connections {
        target: AgentData

        function onDisplayChanged() {
            if (page.dirty)
                return;
            page.resetDisplaySelection(page.source);
        }
    }

    Connections {
        target: LiveData

        function onProfilesChanged() {
            if (page.isNew)
                return;
            if (AdminState.saving)
                return; // la sauvegarde en cours va recharger explicitement
            if (!page.dirty) {
                page.loadProfile();
                return;
            }
            if (page.profileSignature(page.source) !== page.loadedSignature)
                page.conflict = true;
        }

        function onResolvedActiveIdChanged() {
            if (page.dirty && !AdminState.saving)
                page.conflict = true;
        }
    }

    // Résultat d'enregistrement : succès → rechargement ; échec → formulaire
    // conservé, message dans le bandeau.
    Connections {
        target: AdminState

        function onSaveSucceededChanged() {
            if (!AdminState.saveSucceeded)
                return;
            if (AdminState.saveAction === "create") {
                var createdName = page.profileName;
                var createdId = AdminState.createdProfileId;
                page.dirty = false;
                page.conflict = false;
                page.loadProfile();
                if (createdId !== "")
                    AppState.editProfile(createdId);
                AppState.notify(Strings.tf("profiles.created", [createdName]), "ok");
                return;
            }
            if (AdminState.saveAction !== "save")
                return;
            page.dirty = false;
            page.conflict = false;
            page.loadProfile();
            AppState.notify(Strings.t("toast.profileSaved"), "ok");
        }

        function onSaveErrorChanged() {
            // Le worker a relu la collection avant d'écrire et a détecté un
            // autre client : on impose « Recharger ».
            if (AdminState.saveError === "Conflict")
                page.conflict = true;
        }
    }

    function copyCurve(points) {
        var out = [];
        // Un tableau passé par signal `var` peut être une liste QML : on se
        // base sur `length`/index plutôt que sur `Array.isArray`.
        if (points === undefined || points === null || !points.length)
            return out;
        for (var i = 0; i < points.length; ++i) {
            out.push({ "temp": Number(points[i].temp),
                       "speed": Number(points[i].speed) });
        }
        return out;
    }

    function defaultCurve() {
        return page.copyCurve(DemoData.fanCurve);
    }

    function openFanCurveEditor() {
        if (page.fanProfileKeys[page.fanProfileIndex] !== "custom") {
            AppState.notify(Strings.t("fan.customOnly"), "info");
            return;
        }
        if (!page.editable)
            return;
        fanCurveDialog.openWith(page.fanCurve.length > 1
                                ? page.fanCurve : page.defaultCurve());
    }

    function loadProfile() {
        // Lecture directe de l'ID : ne dépend pas de l'ordre de mise à jour de
        // la propriété `source` lors d'un changement de profil.
        var p = page.isNew ? null : LiveData.profileById(AppState.editingProfileId);
        page.loadedSignature = page.profileSignature(p);
        page.conflict = false;
        if (!p) {
            // Brouillon de création : nom localisé, le reste aux valeurs par
            // défaut de `tcc_core::Profile` (ID généré à l'enregistrement).
            profileName = Strings.t("editor.newProfileName");
            profileDescription = "";
            // Valeurs par défaut de `tcc_core::Profile::default()`.
            useBrightness = false;
            brightness = 100;
            useRefresh = false;
            useResolution = false;
            page.resetDisplaySelection(null);
            // Cœurs et fréquences absents par défaut : `null` dans le DTO,
            // le daemon laisse le matériel décider.
            cores = 1;
            coresKnown = false;
            maxPerfGov = false;
            minFreq = 0.8;
            freqMinKnown = false;
            maxFreq = 0.4;
            freqMaxKnown = false;
            freqMaxReduced = false;
            eppValue = "balance_performance";
            noTurbo = false;
            webcamOn = true;
            useWebcam = true;
            fanProfileIndex = 2;
            fanUseControl = true;
            fanMin = 0;
            fanMax = 100;
            fanOffset = 0;
            fanCurve = page.defaultCurve();
            odmProfileIndex = 0;
            odmInitialIndex = 0;
            tdpDraft = [];
            tdpInitial = [];
            ctgpDraft = "0";
            ctgpInitial = "0";
        } else {
            profileName = p.name;
            profileDescription = p.description;
            useBrightness = p.useBrightness !== undefined ? p.useBrightness === true
                                                          : p.brightness > 0;
            brightness = p.brightness;
            useRefresh = p.useRefRate !== undefined ? p.useRefRate === true
                                                    : p.refreshRate > 0;
            useResolution = p.useResolution === true;
            page.resetDisplaySelection(p);
            cores = p.cores > 0 ? p.cores : 1;
            coresKnown = p.cores > 0;
            maxPerfGov = p.maxPerfGov === true || p.governor === "performance";
            // Un champ `Option` absent n'est ni inventé ni envoyé : il reste
            // « non disponible » et `null` dans le DTO.
            minFreq = p.freqMin > 0 ? p.freqMin : 0.8;
            freqMinKnown = p.freqMin > 0;
            maxFreq = p.freqMax > 0 ? p.freqMax : 0.4;
            freqMaxKnown = p.freqMax > 0 || p.freqMaxReduced === true;
            freqMaxReduced = p.freqMaxReduced === true;
            eppValue = typeof p.epp === "string" && p.epp !== ""
                    ? p.epp : "balance_performance";
            noTurbo = p.noTurbo === true;
            webcamOn = p.webcam === true;
            useWebcam = p.useWebcam !== undefined ? p.useWebcam === true : true;
            var fanIndex = page.fanProfileKeys.indexOf(p.fanMode);
            fanProfileIndex = fanIndex >= 0 ? fanIndex : 2;
            fanUseControl = p.useFanControl !== undefined ? p.useFanControl === true : true;
            fanMin = p.fanMin;
            fanMax = p.fanMax;
            fanOffset = p.fanOffset;
            var sourceCurve = page.copyCurve(p.fanCurve);
            fanCurve = sourceCurve.length > 1 ? sourceCurve : page.defaultCurve();

            var storedOdm = p.odm ? String(p.odm) : "";
            var odmIndex = storedOdm !== ""
                    ? page.odmEditChoices.indexOf(storedOdm) + 1 : 0;
            odmProfileIndex = odmIndex > 0 ? odmIndex : 0;
            odmInitialIndex = odmProfileIndex;

            var tdp = Array.isArray(p.tdpValues) ? p.tdpValues : [];
            var tdpCount = Math.max(HardwareData.odm.tdp.length, tdp.length);
            var tdpStrings = [];
            for (var t = 0; t < tdpCount; ++t) {
                tdpStrings.push(t < tdp.length ? String(tdp[t]) : "");
            }
            tdpDraft = tdpStrings;
            tdpInitial = tdpStrings.slice();
            ctgpDraft = String(p.ctgp !== undefined ? p.ctgp : 0);
            ctgpInitial = ctgpDraft;
        }
        dirty = false;
    }

    function profileSignature(profile) {
        if (!profile)
            return "";
        var copy = {};
        for (var key in profile) {
            if (key !== "assignedAc" && key !== "assignedBat")
                copy[key] = profile[key];
        }
        return JSON.stringify(copy);
    }

    function odmLabel(value) {
        var key = "odm.mode." + value;
        var label = Strings.t(key);
        return label === key ? String(value) : label;
    }

    function odmOptionLabels() {
        var out = [Strings.t("editor.odmUnchanged")];
        for (var i = 0; i < page.odmEditChoices.length; ++i)
            out.push(page.odmLabel(page.odmEditChoices[i]));
        return out;
    }

    function odmProfileName() {
        return page.odmProfileIndex > 0
                && page.odmProfileIndex <= page.odmEditChoices.length
                ? page.odmEditChoices[page.odmProfileIndex - 1] : "";
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
        page.touch();
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
        return !isFinite(value) || value < 1 || value > 1000;
    }

    function tdpValuesArray() {
        var out = [];
        for (var i = 0; i < page.tdpDraft.length; ++i)
            out.push(page.tdpNumber(i));
        return out;
    }

    function ctgpNumber() {
        var value = Number(String(page.ctgpDraft).replace(",", "."));
        return isFinite(value) ? value : NaN;
    }

    function ctgpDirty() {
        return page.ctgpDraft !== page.ctgpInitial;
    }

    function ctgpValid() {
        var value = page.ctgpNumber();
        return isFinite(value) && value >= 0 && value <= HardwareData.ctgpMax;
    }

    function touch() {
        if (!page.editable)
            return;
        dirty = true;
    }

    /// Envoie le DTO d'édition (JSON borné) au worker via `AppState`.
    /// En mode création, l'ID est généré côté Rust.
    function save() {
        if (!page.savePossible())
            return;
        var payload = JSON.stringify(ProfileRules.toEdit(page.form));
        if (page.isNew)
            AppState.requestProfileCreate(payload);
        else
            AppState.requestProfileSave(AppState.editingProfileId, payload);
    }

    function reloadFromLive() {
        page.loadProfile();
        AppState.notify(Strings.t("editor.reloaded"), "info");
    }

    /// Libellé du backend compositeur, vide si non exposé.
    readonly property string backendLabel: {
        if (!AgentData.available)
            return "";
        var backend = AgentData.display.backend;
        if (backend.name !== "")
            return backend.name;
        return backend.kind;
    }
    /// Connecteur interne réel et son état.
    readonly property string outputLabel: {
        if (!AgentData.available)
            return "";
        var output = AgentData.display.output;
        if (output.name === "")
            return "";
        return output.name
                + (output.connected ? "" : " (" + Strings.t("display.disconnected") + ")");
    }

    function resolutionOptionLabels() {
        var out = [];
        for (var i = 0; i < page.resolutionOptions.length; ++i)
            out.push(page.resolutionOptions[i].label);
        return out.length > 0 ? out : [Strings.t("common.unavailable")];
    }

    function refreshOptionLabels() {
        var out = [];
        for (var i = 0; i < page.activeRates.length; ++i)
            out.push(Strings.formatHz(page.activeRates[i]));
        return out.length > 0 ? out : [Strings.t("common.unavailable")];
    }

    function resolutionIndexFor(width, height) {
        return AgentData.resolutionIndex(width, height);
    }

    function refreshIndexFor(width, height, rate) {
        return AgentData.refreshIndex(width, height, rate);
    }

    /// Sélection d'une résolution réelle : le taux est réinitialisé sur le
    /// mode courant de cette résolution, sinon le premier taux disponible.
    function selectResolution(index) {
        page.resolutionIndex = index;
        page.resolutionMissing = false;
        page.refreshMissing = false;
        var resolution = page.selectedResolution;
        if (!resolution) {
            page.refreshIndex = 0;
            return;
        }
        var current = AgentData.refreshIndex(resolution.width, resolution.height,
                                             AgentData.currentRate);
        page.refreshIndex = current >= 0 ? current : 0;
        page.touch();
    }

    function resetDisplaySelection(profile) {
        var width = profile && profile.width > 0 ? profile.width : 0;
        var height = profile && profile.height > 0 ? profile.height : 0;
        var index = width > 0 && height > 0
                ? page.resolutionIndexFor(width, height) : -1;
        if (index < 0) {
            var fallback = AgentData.defaultResolution();
            index = fallback
                    ? page.resolutionIndexFor(fallback.width, fallback.height) : 0;
            page.resolutionMissing = page.agentUsable
                    && profile !== null && profile !== undefined
                    && profile.useResolution === true && width > 0 && height > 0;
        } else {
            page.resolutionMissing = false;
        }
        page.resolutionIndex = Math.max(0, index);

        var active = page.selectedResolution;
        if (!active) {
            page.refreshIndex = 0;
            page.refreshMissing = false;
            return;
        }
        var rate = profile && profile.refreshRate > 0 ? profile.refreshRate : 0;
        var rateIndex = rate > 0
                ? page.refreshIndexFor(active.width, active.height, rate) : -1;
        if (rateIndex < 0) {
            var current = page.refreshIndexFor(active.width, active.height,
                                               AgentData.currentRate);
            page.refreshIndex = current >= 0 ? current : 0;
            page.refreshMissing = page.agentUsable
                    && profile !== null && profile !== undefined
                    && profile.useRefRate === true && rate > 0;
        } else {
            page.refreshIndex = rateIndex;
            page.refreshMissing = false;
        }
    }

    function eppOptionLabels() {
        var out = [];
        for (var i = 0; i < page.eppValues.length; ++i) {
            var value = page.eppValues[i];
            if (!page.eppKnown || value === "")
                out.push(Strings.t("common.auto"));
            else
                out.push(value);
        }
        return out;
    }

    function fanOptionLabels() {
        var out = [];
        for (var i = 0; i < page.fanProfileKeys.length; ++i)
            out.push(Strings.t("fan.mode." + page.fanProfileKeys[i]));
        return out;
    }

    readonly property int floatingBarHeight: 68
    readonly property bool floatingBarVisible: page.editable

    PageScroll {
        id: scrollView
        anchors.fill: parent
        contentSpacing: Theme.s16

    PageHeader {
        Layout.fillWidth: true
        eyebrow: Strings.t("nav.profiles")
        title: page.isNew ? Strings.t("editor.newTitle")
                          : (page.profileName !== "" ? page.profileName : Strings.t("editor.title"))
        subtitle: Strings.t("editor.subtitle")
        iconName: "allprofiles"

        StatusBadge {
            visible: page.dirty
            kind: "warn"
            label: Strings.t("editor.unsaved")
        }

        StatusBadge {
            visible: page.readOnly
            kind: "warn"
            label: Strings.t("editor.readOnlyBadge")
        }

        StatusBadge {
            visible: AdminState.available
            kind: "ok"
            label: AdminState.version !== "" ? AdminState.version
                                             : Strings.t("common.supported")
        }

        AppButton {
            variant: "ghost"
            text: Strings.t("common.back")
            onClicked: AppState.navigate("profiles")
        }

        AppButton {
            variant: "secondary"
            text: Strings.t("common.reset")
            enabled: page.dirty && page.editable
            onClicked: page.loadProfile()
        }

        AppButton {
            variant: "ghost"
            text: AdminState.saving ? Strings.t("editor.saving") : Strings.t("common.save")
            iconName: "save"
            enabled: page.savePossible()
            Accessible.name: Strings.t("common.save")
            Accessible.description: page.saveBlockedReason()
            onClicked: page.save()
        }
    }

    // ── Bandeaux d'état ─────────────────────────────────────────────────────
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
        visible: page.isPreset
        kind: "info"
        title: Strings.t("editor.presetReadOnly")
        body: Strings.t("editor.presetReadOnlyHint")
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: page.isNew
        kind: "info"
        title: Strings.t("editor.newTitle")
        body: Strings.t("editor.newProfileHint")
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: page.conflict && !AdminState.saving
        kind: "warn"
        title: Strings.t("editor.conflictTitle")
        body: Strings.t("editor.conflictBody")
        actionText: Strings.t("editor.reload")
        onActionClicked: page.reloadFromLive()
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: AdminState.hasSaveError && !AdminState.saving
        kind: "error"
        title: AdminState.saveErrorText
        body: AdminState.saveErrorDetailText !== ""
              ? Strings.t("editor.errorField") + " " + AdminState.saveErrorDetailText
              : Strings.t("editor.errorKept")
    }

    InlineNotice {
        Layout.fillWidth: true
        visible: AdminState.saving
        kind: "info"
        title: Strings.t("editor.saving")
        body: Strings.t("editor.savingHint")
    }

    // ── Général ─────────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true
        enabled: page.editable

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.general")
            hint: Strings.t("editor.generalHint")
            iconName: "allprofiles"
        }

        TextFieldRow {
            Layout.fillWidth: true
            label: Strings.t("editor.name")
            placeholder: Strings.t("editor.namePlaceholder")
            text: page.profileName
            invalid: page.nameInvalid
            errorText: page.nameInvalid ? Strings.t(page.errors.name) : ""
            onEdited: function(value) {
                page.profileName = value;
                page.touch();
            }
        }

        TextFieldRow {
            Layout.fillWidth: true
            multiline: true
            label: Strings.t("editor.description")
            placeholder: Strings.t("editor.descriptionPlaceholder")
            text: page.profileDescription
            onEdited: function(value) {
                page.profileDescription = value;
                page.touch();
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.errors.description !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t(page.errors.description)
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.divider }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("editor.assignmentMatrixHint")
        }
    }

    // ── Affichage ───────────────────────────────────────────────────────────
    AppCard {
        objectName: "editorDisplayCard"
        Layout.fillWidth: true
        enabled: page.editable

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.display")
            hint: Strings.t("editor.displayHint")
            iconName: "displaylight"

            StatusBadge {
                kind: AgentData.usable ? "ok" : "warn"
                label: AgentData.usable ? Strings.t("display.supported")
                     : AgentData.available ? Strings.t("display.unsupported")
                     : Strings.t("display.agentUnavailable")
            }
        }

        InlineNotice {
            objectName: "editorAgentNotice"
            Layout.fillWidth: true
            visible: !AgentData.usable
            kind: "warn"
            title: AgentData.available ? Strings.t("display.agentUnsupported")
                                       : Strings.t("display.agentUnavailable")
            body: AgentData.errorText !== "" ? AgentData.errorText
                  : (AgentData.displayErrorText !== "" ? AgentData.displayErrorText
                     : Strings.t("display.agentUsableHint"))
            actionText: Strings.t("display.refreshAgent")
            onActionClicked: AppState.requestAgentRefresh()
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.displayError !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t(page.displayError)
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("display.brightnessSet")
            checked: page.useBrightness
            onToggled: function(checked) {
                page.useBrightness = checked;
                page.touch();
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("display.brightness")
            value: page.brightness
            supported: page.useBrightness
            unit: "%"
            onMoved: function(value) {
                page.brightness = value;
                page.touch();
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.divider }

        // Résolution réelle : uniquement les modes de l'écran interne.
        ToggleRow {
            objectName: "editorUseResolution"
            Layout.fillWidth: true
            label: Strings.t("display.resolutionSet")
            description: Strings.t("display.resolutionHint")
            checked: page.useResolution
            supported: page.agentUsable && page.resolutionKnown
            unsupportedReason: !page.agentUsable ? Strings.t("display.agentUnavailable")
                                                 : Strings.t("display.resolutionUnavailable")
            onToggled: function(checked) {
                page.useResolution = checked;
                page.resolutionMissing = false;
                page.refreshMissing = false;
                if (!checked && AgentData.currentMode) {
                    var index = AgentData.refreshIndex(AgentData.currentMode.width,
                                                       AgentData.currentMode.height,
                                                       AgentData.currentRate);
                    page.refreshIndex = index >= 0 ? index : 0;
                }
                page.touch();
            }
        }

        SelectRow {
            objectName: "editorResolutionSelect"
            Layout.fillWidth: true
            label: Strings.t("display.resolution")
            options: page.resolutionOptionLabels()
            currentIndex: page.resolutionOptions.length > 0
                          ? Math.max(0, Math.min(page.resolutionIndex,
                                                 page.resolutionOptions.length - 1))
                          : 0
            supported: page.agentUsable && page.resolutionKnown && page.useResolution
            statusLabel: page.resolutionExists ? Strings.t("display.supported")
                                               : Strings.t("common.unavailable")
            statusKind: page.resolutionExists ? "ok" : "warn"
            onChanged: function(index) { page.selectResolution(index); }
        }

        AppLabel {
            objectName: "editorResolutionWarning"
            Layout.fillWidth: true
            visible: page.resolutionMissing
                     || (page.useResolution && !page.resolutionExists)
            variant: "caption"
            color: Theme.warning
            wrapMode: Text.WordWrap
            text: Strings.t("display.resolutionUnavailableHint")
        }

        // Taux réellement disponibles pour la résolution choisie.
        ToggleRow {
            objectName: "editorUseRefresh"
            Layout.fillWidth: true
            label: Strings.t("display.refreshSet")
            description: Strings.t("display.refreshHelp")
            checked: page.useRefresh
            supported: page.refreshSupported
            unsupportedReason: !page.agentUsable
                               ? Strings.t("display.agentUnavailable")
                               : Strings.t("display.refreshUnavailable")
            onToggled: function(checked) {
                page.useRefresh = checked;
                page.touch();
            }
        }

        SelectRow {
            objectName: "editorRefreshSelect"
            Layout.fillWidth: true
            label: Strings.t("display.refreshRate")
            description: Strings.t("display.refreshHelp")
            options: page.refreshOptionLabels()
            currentIndex: page.activeRates.length > 0
                          ? Math.max(0, Math.min(page.refreshIndex,
                                                 page.activeRates.length - 1))
                          : 0
            supported: page.refreshSupported && page.useRefresh
            statusLabel: page.refreshSupported ? Strings.t("display.supported")
                                               : Strings.t("common.unavailable")
            statusKind: page.refreshSupported ? "ok" : "warn"
            onChanged: function(index) {
                page.refreshIndex = index;
                page.refreshMissing = false;
                page.touch();
            }
        }

        AppLabel {
            objectName: "editorRefreshWarning"
            Layout.fillWidth: true
            visible: page.refreshMissing
                     || (page.useRefresh && !page.refreshExists)
            variant: "caption"
            color: Theme.warning
            wrapMode: Text.WordWrap
            text: Strings.t("display.refreshUnavailableHint")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s24

            InfoRow {
                objectName: "editorBackendRow"
                Layout.fillWidth: true
                visible: page.backendLabel !== ""
                label: Strings.t("display.backend")
                value: page.backendLabel
                iconName: "gear"
            }

            InfoRow {
                objectName: "editorOutputRow"
                Layout.fillWidth: true
                visible: page.outputLabel !== ""
                label: Strings.t("display.output")
                value: page.outputLabel
                iconName: "monitor"
            }

            InfoRow {
                objectName: "editorCurrentModeRow"
                Layout.fillWidth: true
                visible: AgentData.currentModeLabel !== ""
                label: Strings.t("display.current")
                value: AgentData.currentModeLabel
                iconName: "speedometer"
            }
        }

    }

    // ── CPU ─────────────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true
        enabled: page.editable

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.cpu")
            hint: Strings.t("editor.cpuHint")
            iconName: "cpu"
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.cpuError !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t(page.cpuError)
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("cpu.cores")
            from: 1
            to: page.cpuMaxCores
            stepSize: 1
            value: page.cores
            supported: page.coresKnown
            unsupportedReason: Strings.t("common.auto")
            valueText: page.coresKnown
                       ? Strings.formatNumber(page.cores, 0) + " / " + page.cpuMaxCores
                       : Strings.t("common.auto")
            onMoved: function(value) {
                page.cores = value;
                page.touch();
            }
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("cpu.perfGov")
            description: Strings.t("cpu.perfGovHint")
            checked: page.maxPerfGov
            supported: page.performanceGovernorKnown
            unsupportedReason: Strings.t("cpu.perfGovUnavailable")
            onToggled: function(checked) {
                page.maxPerfGov = checked;
                page.touch();
            }
        }

        SelectRow {
            objectName: "editorEppRow"
            Layout.fillWidth: true
            label: Strings.t("cpu.epp")
            description: Strings.t("cpu.eppHint")
            tooltip: Strings.t("cpu.eppTooltip")
            options: page.eppOptionLabels()
            currentIndex: page.eppIndex
            supported: page.eppKnown
            unsupportedReason: Strings.t("cpu.eppUnavailable")
            onChanged: function(index) {
                page.selectEpp(index);
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("cpu.minFreq")
            from: 0.4
            to: 4.4
            stepSize: 0.1
            decimals: 1
            unit: "GHz"
            value: page.minFreq
            supported: page.freqMinKnown
            unsupportedReason: Strings.t("common.auto")
            valueText: page.freqMinKnown ? "" : Strings.t("common.auto")
            onMoved: function(value) {
                page.minFreq = value;
                page.freqMinKnown = true;
                page.touch();
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("cpu.maxFreq")
            from: 0.4
            to: 4.4
            stepSize: 0.1
            decimals: 1
            unit: "GHz"
            value: page.maxFreq
            supported: page.freqMaxKnown
            unsupportedReason: Strings.t("common.auto")
            valueText: page.freqMaxReduced ? Strings.t("cpu.reduced")
                       : (page.freqMaxKnown ? "" : Strings.t("common.auto"))
            onMoved: function(value) {
                page.maxFreq = value;
                page.freqMaxKnown = true;
                page.freqMaxReduced = false;
                page.touch();
            }
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("cpu.noTurbo")
            description: Strings.t("cpu.noTurboHint")
            checked: page.noTurbo
            onToggled: function(checked) {
                page.noTurbo = checked;
                page.touch();
            }
        }
    }

    // ── Webcam ──────────────────────────────────────────────────────────────
    AppCard {
        objectName: "editorWebcamCard"
        Layout.fillWidth: true
        enabled: page.editable
        visible: page.webcamVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.webcam")
            hint: Strings.t("editor.webcamHint")
            iconName: "webcam"
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("editor.webcamOn")
            checked: page.webcamOn
            onToggled: function(checked) {
                page.webcamOn = checked;
                page.touch();
            }
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("editor.webcamApply")
            description: Strings.t("editor.webcamApplyHint")
            checked: page.useWebcam
            onToggled: function(checked) {
                page.useWebcam = checked;
                page.touch();
            }
        }

        InfoRow {
            Layout.fillWidth: true
            visible: HardwareData.webcam.switchAvailable
            label: Strings.t("editor.webcamKillswitch")
            iconName: "webcam"
            badgeLabel: HardwareData.webcam.switchAvailable ? Strings.t("common.supported")
                                                            : Strings.t("editor.webcamKillswitchMissing")
            badgeKind: HardwareData.webcam.switchAvailable ? "ok" : "neutral"
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s12

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                AppLabel {
                    variant: "small"
                    text: Strings.t("editor.webcamPresets")
                }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: Strings.t("editor.webcamPresetsHint")
                }
            }

            AppButton {
                variant: "secondary"
                small: true
                text: Strings.t("settings.manage")
                onClicked: AppState.navigate("tools")
            }
        }
    }

    // ── Ventilateurs ────────────────────────────────────────────────────────
    AppCard {
        Layout.fillWidth: true
        enabled: page.editable

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.fan")
            hint: Strings.t("editor.fanHint")
            iconName: "fan"

            StatusBadge {
                kind: "neutral"
                label: HardwareData.fan.backend !== "" ? HardwareData.fan.backend : "tuxi"
            }
        }

        AppLabel {
            Layout.fillWidth: true
            visible: page.fanError !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t(page.fanError)
        }

        ToggleRow {
            Layout.fillWidth: true
            label: Strings.t("fan.useControl")
            description: Strings.t("fan.useControlHint")
            checked: page.fanUseControl
            onToggled: function(checked) {
                page.fanUseControl = checked;
                page.touch();
            }
        }

        SelectRow {
            Layout.fillWidth: true
            label: Strings.t("fan.profile")
            options: page.fanOptionLabels()
            currentIndex: page.fanProfileIndex
            onChanged: function(index) {
                page.fanProfileIndex = index;
                page.touch();
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("fan.min")
            value: page.fanMin
            unit: "%"
            onMoved: function(value) {
                page.fanMin = value;
                page.touch();
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("fan.max")
            value: page.fanMax
            unit: "%"
            onMoved: function(value) {
                page.fanMax = value;
                page.touch();
            }
        }

        SliderRow {
            Layout.fillWidth: true
            label: Strings.t("fan.offset")
            from: -10
            to: 10
            value: page.fanOffset
            unit: "%"
            onMoved: function(value) {
                page.fanOffset = value;
                page.touch();
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.divider }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s8

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                AppLabel {
                    variant: "body"
                    text: Strings.t("fan.curve")
                }

                AppLabel {
                    variant: "caption"
                    secondary: true
                    text: page.fanProfileKeys[page.fanProfileIndex] === "custom"
                          ? Strings.t("fan.curveHint")
                          : Strings.t("fan.customOnly")
                }
            }

            AppButton {
                objectName: "fanCurveButton"
                variant: "secondary"
                small: true
                text: Strings.t("fan.curveEdit")
                enabled: page.editable
                onClicked: page.openFanCurveEditor()
            }
        }

        FanCurveChart {
            objectName: "fanCurveChart"
            Layout.fillWidth: true
            Layout.preferredHeight: 200
            points: page.fanCurve.length > 1 ? page.fanCurve : page.defaultCurve()
            opacity: page.fanProfileKeys[page.fanProfileIndex] === "custom" ? 1 : 0.55
        }
    }

    // ── ODM / TDP / cTGP du profil ──────────────────────────────────────────
    AppCard {
        objectName: "editorOdmCard"
        Layout.fillWidth: true
        visible: page.odmCardVisible

        SectionHeader {
            Layout.fillWidth: true
            title: Strings.t("editor.odm")
            hint: Strings.t("editor.odmHint")
            iconName: "power"

            StatusBadge {
                visible: HardwareData.odmSourceKind !== ""
                kind: "neutral"
                label: HardwareData.odmSourceKind
            }
        }

        AppLabel {
            objectName: "editorOdmError"
            Layout.fillWidth: true
            visible: page.odmError !== ""
            variant: "caption"
            color: Theme.error
            wrapMode: Text.WordWrap
            text: Strings.t(page.odmError)
        }

        AppLabel {
            Layout.fillWidth: true
            visible: HardwareData.odm.platform.available
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("editor.odmPpdHint")
        }

        SelectRow {
            objectName: "editorOdmSelect"
            Layout.fillWidth: true
            visible: HardwareData.odm.available
            label: Strings.t("settings.odm")
            options: page.odmOptionLabels()
            currentIndex: page.odmProfileIndex
            supported: HardwareData.odmChoices.length > 0
            unsupportedReason: Strings.t("editor.odmNoChoice")
            onChanged: function(index) {
                page.odmProfileIndex = index;
                page.touch();
            }
        }

        // TDP PL1/PL2/PL4 : seulement si le firmware les expose réellement.
        AppLabel {
            Layout.fillWidth: true
            visible: page.tdpEditing
            variant: "body"
            strong: true
            text: Strings.t("settings.tdp")
        }

        Repeater {
            model: HardwareData.odm.tdp

            delegate: TextFieldRow {
                required property var modelData
                required property int index

                Layout.fillWidth: true
                visible: page.tdpEditing
                objectName: "editorTdpField_" + modelData.descriptor
                label: Strings.t("settings.tdp") + " " + modelData.descriptor
                description: Strings.formatNumber(modelData.min, 0) + "–"
                             + Strings.formatNumber(modelData.max, 0) + " W"
                text: page.tdpValue(index)
                inputMethodHints: Qt.ImhFormattedNumbersOnly
                invalid: page.tdpDirty() && page.tdpFieldInvalid(index)
                errorText: Strings.t("editor.error.tdpValues")
                onEdited: function(value) { page.setTdpDraft(index, value); }
            }
        }

        // cTGP NVIDIA : masqué si la dGPU/le contrôle n'est pas exposé.
        TextFieldRow {
            objectName: "editorCtgpField"
            Layout.fillWidth: true
            visible: page.ctgpEditing
            label: Strings.t("settings.ctgp")
            description: Strings.tf("settings.hwCtgpRange", [HardwareData.ctgpMax])
            text: page.ctgpDraft
            inputMethodHints: Qt.ImhFormattedNumbersOnly
            invalid: !page.ctgpValid()
            errorText: Strings.tf("settings.hwCtgpRange", [HardwareData.ctgpMax])
            onEdited: function(value) {
                page.ctgpDraft = value;
                page.touch();
            }
        }

        AppLabel {
            Layout.fillWidth: true
            variant: "caption"
            secondary: true
            wrapMode: Text.WordWrap
            text: Strings.t("editor.odmPersistedHint")
        }
    }

    FanCurveEditor {
        id: fanCurveDialog
        objectName: "fanCurveEditor"

        onApplied: function(points) {
            page.fanCurve = page.copyCurve(points);
            page.touch();
        }
    }

        // Réserve la hauteur de la barre flottante : le dernier champ ne doit
        // jamais être masqué par l'overlay.
        Item {
            objectName: "editorBottomSpacer"
            Layout.fillWidth: true
            implicitHeight: page.floatingBarHeight + Theme.s16
        }
    }

    // ── Barre d'action flottante ────────────────────────────────────────────
    Rectangle {
        id: floatingBar
        objectName: "editorFloatingBar"
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: page.floatingBarHeight
        visible: page.floatingBarVisible
        color: Theme.surface
        border.width: 1
        border.color: Theme.border
        z: 10

        RowLayout {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(parent.width - Theme.s24 * 2, Theme.contentMaxWidth)
            spacing: Theme.s12

            AppLabel {
                objectName: "floatingStatus"
                Layout.fillWidth: true
                variant: "caption"
                color: AdminState.hasSaveError || (page.dirty && !page.valid)
                       ? Theme.error
                       : (page.dirty ? Theme.textSecondary : Theme.textMuted)
                elide: Text.ElideRight
                text: AdminState.hasSaveError ? AdminState.saveErrorText
                      : (page.dirty && !page.valid
                         ? Strings.t("editor.invalidHint")
                         : (page.dirty ? Strings.t("editor.unsaved")
                                       : Strings.t("editor.floatingClean")))
            }

            LoadingState {
                objectName: "floatingSaving"
                visible: AdminState.saving
                size: 18
            }

            AppButton {
                objectName: "floatingCancelButton"
                variant: "secondary"
                text: Strings.t("common.cancel")
                enabled: page.dirty && !AdminState.saving
                onClicked: page.loadProfile()
            }

            AppButton {
                objectName: "floatingSaveButton"
                text: AdminState.saving ? Strings.t("editor.saving")
                                        : Strings.t("common.save")
                iconName: "save"
                enabled: page.savePossible()
                Accessible.name: Strings.t("common.save")
                Accessible.description: page.saveBlockedReason()
                onClicked: page.save()
            }
        }
    }
}
