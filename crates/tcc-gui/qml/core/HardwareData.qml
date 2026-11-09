// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Adaptateur d'affichage des données matérielles (`Manager1` →
// `GetHardwareSnapshotJSON`). Même contrat que `LiveData` :
//
//   - `JSON.parse` uniquement si le daemon est connecté et si le payload a
//     changé (mémo `connected + "|" + hardwareJson`) ;
//   - validation de forme minimale (objet, sections `sensors`/`charging`) ;
//   - normalisation des unités (MHz → GHz) et repli intégral sur `DemoData` ;
//   - aucune écriture : charge, éclairage, Fn-Lock, TDP/ODM, cTGP, PRIME et
//     YCbCr sont exposés en lecture seule.
//
// Le contrat matériel n'est pas encore dans `tcc-ipc` : la GUI désérialise une
// vue typée côté Rust (`src/hardware_view.rs`) qui rejette déjà les clés
// inconnues. Ce fichier ne fait que du formatage d'affichage.

pragma Singleton

import QtQuick

QtObject {
    id: root

    // ── Entrées, poussées par Main.qml ──────────────────────────────────────
    property bool connected: false
    property string hardwareJson: "{}"
    /// Dernière erreur du bridge matériel (payload conservé mais périmé).
    property string hardwareError: ""
    /// Instantané `GetFanDataJSON` (lecture seule).
    property string fanJson: "{}"
    /// Erreur du dernier cycle ventilateurs (payload conservé).
    property string fanError: ""
    /// Collecte automatique (`DaemonClient.sensorsEnabled`). Le bail n'est
    /// renouvelé que lorsque le Dashboard est affiché ; aucun interrupteur.
    property bool sensorsRequested: true
    /// Vrai quand la page Dashboard est visible : condition du renouvellement
    /// du bail capteurs. Hors Dashboard, le bail expire (10 s).
    property bool dashboardVisible: false

    // ── Sorties ─────────────────────────────────────────────────────────────
    property bool usingDemo: true
    property bool isLive: false
    property string parseError: ""
    /// Payload normalisé (live) — remplacé intégralement à chaque changement.
    property var livePayload: null

    property string _lastInput: ""

    /// Données effectivement affichées : live validé ou démo (vivante).
    readonly property var payload: root.usingDemo ? root._demo() : root.livePayload

    readonly property var sensors: root.payload.sensors
    readonly property var cpu: root.sensors.cpu
    readonly property var igpu: root.sensors.igpu
    readonly property var dgpu: root.sensors.dgpu
    readonly property var fan: root.payload.fan
    readonly property var fans: root.fan.fans
    readonly property bool fanStale: root.fanError !== ""

    readonly property var device: root.payload.device
    readonly property var cpuCaps: root.payload.cpuCaps
    readonly property var displayInfo: root.payload.display
    readonly property var capabilities: root.payload.capabilities

    /// Libellé d'appareil réel (produit, sinon modèle, sinon carte). Vide hors
    /// live : aucune valeur de démonstration n'est présentée comme un modèle.
    readonly property string deviceLabel: {
        if (!root.isLive)
            return "";
        var d = root.device;
        if (d.productName !== "")
            return d.productName;
        if (d.model !== "")
            return d.model;
        if (d.boardName !== "")
            return d.boardName;
        return d.vendor;
    }
    readonly property string deviceVendor: root.isLive ? root.device.vendor : ""
    /// Cœurs en ligne / total, vide si le daemon n'expose pas la topologie.
    readonly property string cpuCoresText: {
        if (root.cpuCaps.totalCores <= 0)
            return "";
        return root.cpuCaps.onlineCores + " / " + root.cpuCaps.totalCores;
    }
    /// Borne cTGP effective : dynamique si exposée, sinon 0–200 W statique.
    readonly property int ctgpMax: root.nvidia.maxCtgpOffset >= 0
                                   ? root.nvidia.maxCtgpOffset : 200
    readonly property bool hasInternalDisplay: root.isLive
                                               && root.displayInfo.hasInternalDisplay
    readonly property string internalDisplayLabel: {
        if (!root.isLive || root.displayInfo.internalConnector === "")
            return "";
        return root.displayInfo.internalConnector
                + (root.displayInfo.connected ? "" : " (" + Strings.t("display.disconnected") + ")");
    }

    readonly property var charging: root.payload.charging
    readonly property var lighting: root.payload.lighting
    readonly property var fnLock: root.payload.fnLock
    readonly property var odm: root.payload.odm

    /// Choix ODM : `platform_profile` prime, sinon fallback `tuxedo-io`.
    readonly property var odmChoices: {
        if (root.odm.platform.available && root.odm.platform.choices.length > 0)
            return root.odm.platform.choices;
        return root.odm.io.choices;
    }
    /// Profil courant : seule la source platform expose un état courant.
    readonly property string odmCurrentProfile: root.odm.platform.available
            ? root.odm.platform.current : ""
    /// Défaut : platform si disponible, sinon calculé par le firmware io.
    readonly property string odmDefaultProfile: {
        if (root.odm.platform.available && root.odm.platform.defaultProfile !== "")
            return root.odm.platform.defaultProfile;
        return root.odm.io.defaultProfile;
    }
    readonly property bool odmWritable: root.odm.odmWritable === true
    readonly property bool tdpWritable: root.odm.tdpWritable === true
    /// TDP PL réellement exposé : via `platform_profile` ou `io.tdpAvailable`.
    /// Sur les modèles où le TDP officiel passe par le profil ODM
    /// (`io.tdpAvailable=false`, platform absent), les curseurs sont masqués.
    readonly property bool tdpExposed: root.odm.tdpAvailable
            && root.odm.tdp.length > 0
            && (root.odm.io.tdpAvailable || root.odm.platform.available)
    /// `platform_profile` ou `tuxedo-io` (badge technique).
    readonly property string odmSourceKind: root.odm.platform.available
            ? "platform_profile"
            : (root.odm.io.available ? "tuxedo-io" : "")
    readonly property var nvidia: root.payload.nvidia
    readonly property var prime: root.payload.prime
    readonly property var ycbcr: root.payload.ycbcr
    readonly property var webcam: root.payload.webcam

    /// Vrai quand les capteurs sont réellement collectés côté daemon.
    readonly property bool sensorsActive: root.isLive && root.sensors.enabled === true
    /// Vrai quand l'utilisateur a demandé la collecte mais que le bail daemon
    /// est retombé (expiration 10 s, daemon redémarré, suspension).
    readonly property bool leaseExpired: root.isLive && root.sensorsRequested
                                         && !root.sensorsActive && root.dashboardVisible
    /// Collecte demandée mais en attente : le Dashboard n'est pas visible, le
    /// bail est laissé expirer volontairement (aucun `false` envoyé).
    readonly property bool sensorsPending: root.isLive && root.sensorsRequested
                                           && !root.sensorsActive && !root.dashboardVisible
    readonly property bool sensorsAvailable: root.isLive && root.sensors.available === true
    /// Vrai si le dernier payload matériel est conservé alors que le bridge a
    /// remonté une erreur (valeurs potentiellement périmées).
    readonly property bool stale: root.hardwareError !== ""

    /// Snapshot live fiable : les règles de masquage ne s'appliquent qu'ici.
    /// Pendant le chargement, la démo ou un payload périmé, les blocs restent
    /// visibles pour éviter le flicker et les faux « absent ».
    readonly property bool liveKnown: root.isLive && !root.stale

    /// Vrai si un bloc `available` doit être affiché.
    function visibleIfAvailable(available) {
        return !root.liveKnown || available === true;
    }

    /// Vrai si un bloc structurel (GPU) doit être affiché.
    function visibleIfPresent(present) {
        return !root.liveKnown || present === true;
    }

    Component.onCompleted: root.rebuild()
    onConnectedChanged: root.rebuild()
    onHardwareJsonChanged: root.rebuild()
    onFanJsonChanged: root.rebuild()

    // ── Construction ────────────────────────────────────────────────────────
    function rebuild() {
        var key = (root.connected ? "1" : "0") + "|" + root.hardwareJson
                + "|" + root.fanJson;
        if (key === root._lastInput)
            return;
        root._lastInput = key;

        if (!root.connected) {
            root._applyDemo();
            return;
        }

        var parsed;
        try {
            parsed = JSON.parse(root.hardwareJson);
        } catch (error) {
            root._fail("hardwareJson: JSON invalide");
            return;
        }
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)
                || typeof parsed.sensors !== "object" || typeof parsed.charging !== "object") {
            root._fail("hardwareJson: forme inattendue");
            return;
        }

        // Ordre inverse : `livePayload` est prêt avant que `usingDemo` ne
        // bascule, donc `payload` n'est jamais nul.
        var fanPayload;
        try {
            fanPayload = JSON.parse(root.fanJson);
        } catch (error) {
            root._fail("fanJson: JSON invalide");
            return;
        }
        if (!fanPayload || typeof fanPayload !== "object" || Array.isArray(fanPayload)) {
            root._fail("fanJson: forme inattendue");
            return;
        }

        root.livePayload = root._normalize(parsed, fanPayload);
        root.usingDemo = false;
        root.isLive = true;
        root.parseError = "";
    }

    function _applyDemo() {
        // `usingDemo` d'abord : `payload` ne doit jamais être évalué avec
        // `usingDemo === false` et `livePayload === null`.
        root.usingDemo = true;
        root.isLive = false;
        root.livePayload = null;
        root.parseError = "";
    }

    function _fail(reason) {
        root._applyDemo();
        root.parseError = reason;
    }

    // ── Normalisation d'un payload `HardwareSnapshot` ───────────────────────
    function _normalize(data, fan) {
        var sensors = data.sensors || {};
        var device = data.device || {};
        var cpuCaps = data.cpu || {};
        var display = data.display || {};
        var caps = data.capabilities || {};
        var charging = data.charging || {};
        var lighting = data.lighting || {};
        var keyData = lighting.keyboard || {};
        var barData = lighting.lightbar || {};
        var odmData = data.odm || {};
        var platform = odmData.platformProfile || {};
        var nvidia = data.nvidia || {};
        var prime = data.prime || {};
        var ycbcr = data.ycbcr || {};
        var webcam = data.webcam || {};
        var fnLock = data.fnLock || {};

        return {
            "live": true,
            "device": {
                "available": device.available === true,
                "vendor": root._text(device.vendor, 128),
                "productName": root._text(device.productName, 128),
                "boardName": root._text(device.boardName, 128),
                "boardVendor": root._text(device.boardVendor, 128),
                "chassisVendor": root._text(device.chassisVendor, 128),
                "sku": root._text(device.sku, 64),
                "isTuxedo": device.isTuxedo === true,
                "model": root._text(device.model, 64),
                "kernelRelease": root._text(device.kernelRelease, 128),
                "architecture": root._text(device.architecture, 32),
                "tuxedoDriverVersion": root._text(device.tuxedoDriverVersion, 64),
                "errors": root._strings(device.errors)
            },
            "cpuCaps": {
                "totalCores": root._boundedInt(cpuCaps.totalCores, 0, 4096),
                "onlineCores": root._boundedInt(cpuCaps.onlineCores, 0, 4096),
                "availableGovernors": root._strings(cpuCaps.availableGovernors),
                "availableEpp": root._strings(cpuCaps.availableEpp),
                "hasControl": cpuCaps.hasCpuControl === true,
                "writable": cpuCaps.writable === true,
                "scalingDriver": root._text(cpuCaps.scalingDriver, 64),
                "errors": root._strings(cpuCaps.errors)
            },
            "display": {
                "available": display.available === true,
                "hasInternalDisplay": display.hasInternalDisplay === true,
                "internalConnector": root._text(display.internalConnector, 64),
                "connected": display.connected === true,
                "source": root._text(display.source, 32),
                "errors": root._strings(display.errors)
            },
            "capabilities": {
                "hasBattery": caps.hasBattery === true,
                "hasAc": caps.hasAc === true,
                "hasIgpu": caps.hasIgpu === true,
                "hasDedicatedGpu": caps.hasDedicatedGpu === true,
                "errors": root._strings(caps.errors)
            },
            "fan": root._fanSection(fan),
            "sensors": {
                "enabled": sensors.enabled === true,
                "available": sensors.available === true,
                "cpu": root._cpu(sensors.cpu),
                "igpu": root._gpu(sensors.igpu),
                "dgpu": root._gpu(sensors.dgpu),
                "fans": root._fans(sensors.fans)
            },
            "charging": {
                "available": charging.available === true,
                "writeSupported": charging.writeSupported === true,
                "profileWritable": charging.profileWritable === true,
                "priorityWritable": charging.priorityWritable === true,
                "thresholdsWritable": charging.thresholdsWritable === true,
                "chargeTypeAvailable": charging.chargeTypeAvailable === true,
                "chargeTypeWritable": charging.chargeTypeWritable === true,
                "chargeTypes": root._chargingList(charging.chargeTypes),
                "profiles": root._chargingList(charging.profiles),
                "profile": charging.profile || "",
                "priorities": root._chargingList(charging.priorities),
                "priority": charging.priority || "",
                "start": root._number(charging.startThreshold, 0),
                "end": root._number(charging.endThreshold, 0),
                "startThresholds": root._thresholdList(charging.startThresholds),
                "endThresholds": root._thresholdList(charging.endThresholds),
                "chargeType": charging.chargeType || "",
                "thresholdsAvailable": charging.thresholdsAvailable === true,
                "applied": charging.applied === true,
                "errors": root._strings(charging.errors)
            },
            "lighting": {
                "available": lighting.available === true,
                "enabled": lighting.enabled === true,
                "unsupported": lighting.unsupported === true,
                "keyboard": {
                    "available": keyData.available === true,
                    "writable": keyData.writable === true,
                    "modes": root._strings(keyData.modes),
                    "kind": keyData.kind || "none",
                    "zones": root._number(keyData.zones, 0),
                    "maxBrightness": root._number(keyData.maxBrightness, 0),
                    "maxRed": root._number(keyData.maxRed, 0),
                    "maxGreen": root._number(keyData.maxGreen, 0),
                    "maxBlue": root._number(keyData.maxBlue, 0),
                    "bufferInput": keyData.bufferInput === true
                },
                "lightbar": {
                    "available": barData.available === true,
                    "writable": barData.writable === true,
                    "modes": root._strings(barData.modes),
                    "maxBrightness": root._number(barData.maxBrightness, 0),
                    "maxRed": root._number(barData.maxRed, 0),
                    "maxGreen": root._number(barData.maxGreen, 0),
                    "maxBlue": root._number(barData.maxBlue, 0)
                },
                "states": root._zones(lighting.states),
                "lightbarState": root._zone(lighting.lightbarState),
                "applied": lighting.applied === true,
                "errors": root._strings(lighting.errors)
            },
            "fnLock": {
                "supported": fnLock.supported === true,
                "writable": fnLock.writable === true,
                "enabledKnown": typeof fnLock.enabled === "boolean",
                "enabled": fnLock.enabled === true
            },
            "odm": {
                "available": odmData.available === true
                             || platform.available === true
                             || (odmData.io !== undefined && odmData.io !== null
                                 && (odmData.io.odmAvailable === true
                                     || odmData.io.tdpAvailable === true)),
                "odmWritable": odmData.odmWritable === true,
                "tdpWritable": odmData.tdpWritable === true,
                "platform": {
                    "available": platform.available === true,
                    "writable": platform.writable === true,
                    "source": platform.source || "",
                    "current": platform.current || "",
                    "choices": root._strings(platform.choices),
                    "defaultProfile": platform.defaultProfile || "",
                    "owner": platform.owner || "",
                    "applied": platform.applied || ""
                },
                "io": {
                    "available": (odmData.io && odmData.io.available === true),
                    "interface": odmData.io && odmData.io.interface
                                 ? odmData.io.interface : "",
                    "moduleVersion": odmData.io && odmData.io.moduleVersion
                                     ? odmData.io.moduleVersion : "",
                    "odmAvailable": (odmData.io && odmData.io.odmAvailable === true),
                    "choices": root._strings(odmData.io ? odmData.io.choices : []),
                    "defaultProfile": odmData.io && odmData.io.defaultProfile
                                      ? odmData.io.defaultProfile : "",
                    "tdpAvailable": (odmData.io && odmData.io.tdpAvailable === true)
                },
                "tdpAvailable": odmData.tdpAvailable === true
                                || (odmData.io !== undefined && odmData.io !== null
                                    && odmData.io.tdpAvailable === true),
                "tdp": root._tdp(odmData.tdp),
                "interface": odmData.interface || "",
                "moduleVersion": odmData.moduleVersion || "",
                "applied": odmData.applied === true,
                "errors": root._strings(odmData.errors)
            },
            "nvidia": {
                "available": nvidia.available === true,
                "attributeAvailable": nvidia.attributeAvailable === true,
                "smiAvailable": nvidia.nvidiaSmiAvailable === true,
                "ctgpOffset": root._number(nvidia.ctgpOffset, 0),
                "defaultPowerLimit": root._number(nvidia.defaultPowerLimit, 0),
                "maxPowerLimit": root._number(nvidia.maxPowerLimit, 0),
                // Borne dynamique cTGP si le contrat l'expose ; -1 = absente
                // (le client retombe alors sur 0–200 W statique).
                "maxCtgpOffset": typeof nvidia.maxCtgpOffset === "number"
                                 && isFinite(nvidia.maxCtgpOffset)
                                 ? Math.max(0, Math.min(200,
                                       Math.round(nvidia.maxCtgpOffset))) : -1,
                "appliedOffset": root._number(nvidia.appliedOffset, 0),
                "errors": root._strings(nvidia.errors)
            },
            "prime": {
                "available": prime.available === true,
                "supported": prime.supported === true,
                "mode": prime.mode || "unknown",
                "internalDisplayOnNvidia": prime.internalDisplayOnNvidia === true,
                "requiresOffloading": prime.requiresOffloading === true
            },
            "ycbcr": {
                "available": ycbcr.available === true,
                "ports": root._ports(ycbcr.ports),
                "pendingModeReapply": ycbcr.pendingModeReapply === true
            },
            "webcam": {
                "switchAvailable": webcam.switchAvailable === true,
                "enabled": webcam.enabled === true,
                "controlsSupported": webcam.controlsSupported === true,
                "controlsWritable": false,
                "controlsTruncated": webcam.controlsTruncated === true,
                "devices": root._devices(webcam.devices),
                "controlsError": root._controlsError(webcam)
            }
        };
    }

    function _cpu(cpu) {
        var c = cpu || {};
        return {
            "present": cpu !== null && cpu !== undefined,
            "temp": root._number(c.temperatureC, 0),
            "freqGhz": root._ghz(c.frequencyMhz),
            "powerW": root._number(c.powerW, 0),
            // `!= null` couvre `null` et `undefined` (champ absent).
            "available": c.temperatureC != null || c.frequencyMhz != null
                         || c.powerW != null
        };
    }

    function _gpu(gpu) {
        if (!gpu) {
            return { "present": false, "name": "", "vendor": "", "role": "",
                     "temp": 0, "freqGhz": 0, "maxGhz": 0, "powerW": 0,
                     "available": false };
        }
        return {
            "present": true,
            "name": gpu.name || "",
            "vendor": gpu.vendor || "",
            "role": gpu.role || "",
            "temp": root._number(gpu.temperatureC, 0),
            "freqGhz": root._ghz(gpu.frequencyMhz),
            "maxGhz": root._ghz(gpu.maxFrequencyMhz),
            "powerW": root._number(gpu.powerW, 0),
            // Disponibilité par métrique : ne jamais tracer un 0 inventé
            // quand seule une partie des capteurs est exposée.
            "tempAvailable": gpu.temperatureC != null,
            "freqAvailable": gpu.frequencyMhz != null,
            "powerAvailable": gpu.powerW != null,
            "available": gpu.temperatureC != null || gpu.frequencyMhz != null
                         || gpu.powerW != null
        };
    }

    function _fans(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length; ++i) {
            var fan = list[i];
            if (!fan || typeof fan !== "object")
                continue;
            out.push({
                "name": fan.name || ("Fan " + (i + 1)),
                "temp": root._number(fan.temp, 0),
                "speed": root._number(fan.speed, 0),
                "applied": fan.applied !== false
            });
        }
        return out;
    }

    /// Section ventilateurs issue de `GetFanDataJSON`.
    function _fanSection(fan) {
        var data = fan || {};
        var mode = typeof data.mode === "string" ? data.mode : "";
        if (mode !== "off" && mode !== "auto" && mode !== "manual")
            mode = "unknown";
        return {
            "available": data.available === true,
            "backend": data.backend || "",
            "writable": data.writable === true,
            "controlEnabled": data.controlEnabled === true,
            "mode": mode,
            "fans": root._fanReadings(data.fans)
        };
    }

    function _fanReadings(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length; ++i) {
            var fan = list[i];
            if (!fan || typeof fan !== "object")
                continue;
            out.push({
                "index": root._number(fan.index, i),
                "label": fan.label || ("fan" + i),
                "target": fan.target || "",
                "temp": fan.temp == null ? -1 : fan.temp,
                "speed": fan.speed == null ? -1 : fan.speed,
                "applied": fan.applied == null ? -1 : fan.applied
            });
        }
        return out;
    }

    function _zone(zone) {
        if (!zone || typeof zone !== "object")
            return { "brightness": 0, "red": 0, "green": 0, "blue": 0 };
        return {
            "brightness": root._number(zone.brightness, 0),
            "red": root._number(zone.red, 0),
            "green": root._number(zone.green, 0),
            "blue": root._number(zone.blue, 0)
        };
    }

    function _zones(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length; ++i)
            out.push(root._zone(list[i]));
        return out;
    }

    function _tdp(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length; ++i) {
            var tdp = list[i];
            if (!tdp || typeof tdp !== "object")
                continue;
            out.push({
                "descriptor": tdp.descriptor || "",
                "current": root._number(tdp.current, 0),
                "min": root._number(tdp.min, 0),
                "max": root._number(tdp.max, 0)
            });
        }
        return out;
    }

    function _ports(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length; ++i) {
            var port = list[i];
            if (!port || typeof port !== "object")
                continue;
            out.push({
                "card": root._number(port.card, 0),
                "port": port.port || "",
                "forced": port.forced === true
            });
        }
        return out;
    }

    function _devices(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && i < 16; ++i) {
            var device = list[i];
            if (!device || typeof device !== "object")
                continue;
            out.push({
                "name": device.name || "",
                "path": device.devicePath || "",
                "metadata": device.metadata === true,
                "controlsError": device.controlsError || "",
                "controls": root._uvcControls(device.controls)
            });
        }
        return out;
    }

    function _controlsError(webcam) {
        if (!webcam || typeof webcam !== "object")
            return "";
        var errors = root._strings(webcam.errors);
        if (errors.length > 0)
            return errors[0];
        if (Array.isArray(webcam.devices)) {
            for (var i = 0; i < webcam.devices.length; ++i) {
                var error = webcam.devices[i] && webcam.devices[i].controlsError;
                if (typeof error === "string" && error !== "")
                    return error;
            }
        }
        return "";
    }

    function _uvcControls(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && i < 64; ++i) {
            var control = list[i];
            if (!control || typeof control !== "object")
                continue;
            var menu = [];
            if (Array.isArray(control.menu)) {
                for (var j = 0; j < control.menu.length && j < 128; ++j) {
                    var entry = control.menu[j];
                    if (!entry || typeof entry !== "object")
                        continue;
                    menu.push({
                        "index": root._number(entry.index, 0),
                        "key": entry.key || "",
                        "label": entry.label || ""
                    });
                }
            }
            out.push({
                "key": control.key || "",
                "name": control.name || control.key || "",
                "kind": control.kind || "unknown",
                "min": root._number(control.min, 0),
                "max": root._number(control.max, 0),
                "step": root._number(control.step, 0),
                "default": root._number(control.default, 0),
                "current": root._uvcValue(control.current),
                "menu": menu,
                "flags": {
                    "inactive": control.flags && control.flags.inactive === true,
                    "readOnly": control.flags && control.flags.readOnly === true,
                    "writeOnly": control.flags && control.flags.writeOnly === true,
                    "update": control.flags && control.flags.update === true,
                    "disabled": control.flags && control.flags.disabled === true,
                    "grabbed": control.flags && control.flags.grabbed === true
                },
                "category": control.category || "other",
                "simple": control.simple === true
            });
        }
        return out;
    }

    function _uvcValue(value) {
        if (!value || typeof value !== "object")
            return { "kind": "none", "value": 0, "index": 0, "key": "", "label": "" };
        var raw = value.value;
        var numeric = typeof raw === "number" && isFinite(raw) ? raw : null;
        var boolValue = typeof raw === "boolean" ? raw : null;
        return {
            "kind": value.kind || "none",
            "value": numeric !== null ? numeric : (boolValue !== null ? (boolValue ? 1 : 0) : 0),
            "boolean": boolValue,
            "index": root._number(value.index, 0),
            "key": value.key || "",
            "label": value.label || ""
        };
    }

    /// Liste de valeurs de charge (profils/priorités) : chaînes non vides,
    /// bornée à 32 entrées pour rester un menu raisonnable.
    function _chargingList(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && i < 32; ++i) {
            if (typeof list[i] === "string" && list[i] !== "")
                out.push(list[i]);
        }
        return out;
    }

    /// Seuils disponibles : entiers 0..100, triés et dédupliqués.
    function _thresholdList(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && i < 101; ++i) {
            var value = list[i];
            if (typeof value === "number" && isFinite(value) && value >= 0 && value <= 100)
                out.push(Math.round(value));
        }
        out.sort(function(a, b) { return a - b; });
        var unique = [];
        for (var j = 0; j < out.length; ++j) {
            if (j === 0 || out[j] !== out[j - 1])
                unique.push(out[j]);
        }
        return unique;
    }

    function _strings(list) {
        var out = [];
        if (!Array.isArray(list))
            return out;
        for (var i = 0; i < list.length && i < 8; ++i) {
            if (typeof list[i] === "string")
                out.push(list[i]);
        }
        return out;
    }

    function _number(value, fallback) {
        return typeof value === "number" && isFinite(value) ? value : fallback;
    }

    /// Chaîne bornée, sans caractère de contrôle (valeurs DMI non fiables).
    function _text(value, max) {
        if (typeof value !== "string")
            return "";
        var out = "";
        for (var i = 0; i < value.length && out.length < max; ++i) {
            var code = value.charCodeAt(i);
            if (code >= 32 && code !== 127)
                out += value.charAt(i);
        }
        return out.trim();
    }

    function _boundedInt(value, min, max) {
        if (typeof value !== "number" || !isFinite(value))
            return min;
        return Math.max(min, Math.min(max, Math.round(value)));
    }

    // MHz → GHz, arrondi 2 décimales (3200 MHz → 3,2 GHz).
    function _ghz(mhz) {
        if (typeof mhz !== "number" || !isFinite(mhz) || mhz <= 0)
            return 0;
        return Math.round(mhz / 10) / 100;
    }

    // ── Repli de démonstration ──────────────────────────────────────────────
    function _demo() {
        var profile = DemoData.profileById(DemoData.activeProfileId) || {};
        return {
            "live": false,
            // Aucune valeur produit inventée en démonstration : l'Info et le
            // Dashboard affichent « non disponible ».
            "device": {
                "available": false, "vendor": "", "productName": "",
                "boardName": "", "boardVendor": "", "chassisVendor": "",
                "sku": "", "isTuxedo": false, "model": "",
                "kernelRelease": "", "architecture": "",
                "tuxedoDriverVersion": "", "errors": []
            },
            "cpuCaps": {
                "totalCores": 0, "onlineCores": 0,
                "availableGovernors": [], "availableEpp": [],
                "hasControl": false, "writable": false,
                "scalingDriver": "", "errors": []
            },
            "display": {
                "available": false, "hasInternalDisplay": false,
                "internalConnector": "", "connected": false,
                "source": "", "errors": []
            },
            "capabilities": {
                "hasBattery": false, "hasAc": false, "hasIgpu": false,
                "hasDedicatedGpu": false, "errors": []
            },
            "fan": {
                "available": true,
                "backend": "tuxi",
                "writable": true,
                "controlEnabled": true,
                "mode": "auto",
                "fans": [{ "index": 0, "label": "cpu", "target": "cpu",
                           "temp": DemoData.cpuTemp, "speed": DemoData.cpuFan,
                           "applied": DemoData.cpuFan }]
            },
            "sensors": {
                "enabled": false,
                "available": true,
                "cpu": { "present": true, "temp": DemoData.cpuTemp,
                         "freqGhz": DemoData.cpuFreq, "powerW": DemoData.cpuPower,
                         "available": true },
                "igpu": { "present": true, "name": Strings.t("dash.igpu"),
                          "vendor": "intel", "role": "integrated",
                          "temp": DemoData.igpuTemp, "freqGhz": DemoData.igpuFreq,
                          "maxGhz": 0, "powerW": 0,
                          "tempAvailable": true, "freqAvailable": true,
                          "powerAvailable": false, "available": true },
                "dgpu": { "present": DemoData.dgpuPresent, "name": "NVIDIA",
                          "vendor": "nvidia", "role": "discrete",
                          "temp": DemoData.dgpuTemp, "freqGhz": DemoData.dgpuFreq,
                          "maxGhz": 0, "powerW": DemoData.dgpuPower,
                          "tempAvailable": DemoData.dgpuPresent,
                          "freqAvailable": DemoData.dgpuPresent,
                          "powerAvailable": DemoData.dgpuPresent,
                          "available": DemoData.dgpuPresent }
            },
            "charging": {
                "available": DemoData.chargingAvailable,
                "writeSupported": true,
                "profileWritable": true,
                "priorityWritable": true,
                "thresholdsWritable": true,
                "chargeTypeAvailable": true,
                "chargeTypeWritable": true,
                "chargeTypes": ["Trickle", "Fast", "Standard", "Adaptive",
                                "Custom", "LongLife", "Bypass"],
                "profiles": ["balanced", "high-capacity"],
                "profile": "balanced",
                "priorities": ["balanced", "charging"],
                "priority": "balanced",
                "start": 80,
                "end": 90,
                "startThresholds": [40, 50, 60, 70, 80],
                "endThresholds": [50, 60, 70, 80, 90, 100],
                "chargeType": "Custom",
                "thresholdsAvailable": DemoData.chargingAvailable,
                "applied": true,
                "errors": []
            },
            "lighting": {
                "available": true,
                "enabled": true,
                "unsupported": false,
                "keyboard": { "available": true, "writable": true,
                              "modes": ["static"], "kind": "rgb", "zones": 3,
                              "maxBrightness": 100, "maxRed": 255,
                              "maxGreen": 255, "maxBlue": 255,
                              "bufferInput": true },
                "lightbar": { "available": true, "writable": true,
                              "modes": ["static"], "maxBrightness": 100,
                              "maxRed": 255, "maxGreen": 255, "maxBlue": 255 },
                "states": [],
                "lightbarState": { "brightness": 0, "red": 0, "green": 0, "blue": 0 },
                "applied": true,
                "errors": []
            },
            "fnLock": { "supported": true, "writable": true,
                        "enabledKnown": true, "enabled": false },
            "odm": {
                "available": DemoData.odmAvailable,
                "odmWritable": true,
                "tdpWritable": true,
                "platform": {
                    "available": DemoData.odmAvailable,
                    "writable": true,
                    "source": "tuxedo_platform_profile",
                    "current": profile.odm || "balanced",
                    "choices": ["power_save", "quiet", "balanced", "performance",
                                "overboost"],
                    "defaultProfile": "balanced",
                    "owner": "ppd",
                    "applied": profile.odm || "balanced"
                },
                "io": {
                    "available": true,
                    "interface": "uniwill",
                    "moduleVersion": "demo",
                    "odmAvailable": true,
                    "choices": ["power_save", "enthusiast", "overboost"],
                    "defaultProfile": "enthusiast",
                    "tdpAvailable": DemoData.odmAvailable
                },
                "tdpAvailable": DemoData.odmAvailable,
                "tdp": [
                    { "descriptor": "pl1", "current": profile.tdp1 || 0, "min": 15, "max": 75 },
                    { "descriptor": "pl2", "current": profile.tdp2 || 0, "min": 15, "max": 95 }
                ],
                "interface": "uniwill",
                "moduleVersion": "demo",
                "applied": true,
                "errors": []
            },
            "nvidia": {
                "available": DemoData.nvidiaAvailable,
                "attributeAvailable": DemoData.nvidiaAvailable,
                "smiAvailable": DemoData.nvidiaAvailable,
                "ctgpOffset": profile.ctgp || 0,
                "defaultPowerLimit": 60,
                "maxPowerLimit": 90,
                "maxCtgpOffset": -1,
                "appliedOffset": profile.ctgp || 0,
                "errors": []
            },
            "prime": {
                "available": false,
                "supported": false,
                "mode": "unknown",
                "internalDisplayOnNvidia": false,
                "requiresOffloading": false
            },
            "ycbcr": { "available": false, "ports": [], "pendingModeReapply": false },
            "webcam": {
                "switchAvailable": DemoData.killswitchAvailable,
                "enabled": true,
                "controlsSupported": false,
                "controlsWritable": false,
                "controlsTruncated": false,
                "controlsError": "",
                "devices": []
            }
        };
    }
}
