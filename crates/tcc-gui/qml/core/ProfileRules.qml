// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 TuxedoRustCenter contributors
//
// Règles d'édition et de validation locale des profils (phase 1).
//
// Ces règles reflètent les bornes de `tcc-core` lorsque la GUI les connaît
// (longueur de nom, description, fréquences, cœurs, ventilateurs). Elles ne
// remplacent jamais la validation du daemon : elles empêchent seulement
// d'envoyer un formulaire manifestement invalide et fournissent un message
// par champ. Aucune correction silencieuse.
//
// Le DTO produit par `toEdit()` est identique à `ProfileEdit` côté Rust
// (`src/profile_edit.rs`, `deny_unknown_fields`) : toute évolution doit être
// faite des deux côtés.

pragma Singleton

import QtQuick

QtObject {
    id: root

    /// Longueur maximale d'un nom (`MAX_PROFILE_NAME_LEN`).
    readonly property int maxNameLength: 128
    /// Longueur maximale d'une description (`MAX_DESCRIPTION_LEN`).
    readonly property int maxDescriptionLength: 4096
    /// Bornes de fréquence CPU en GHz (`MIN/MAX_CPU_FREQUENCY_KHZ`).
    readonly property real minGhz: 0.1
    readonly property real maxGhz: 10.0
    /// Borne de cœurs en ligne (`MAX_CPU_ONLINE_CORES`).
    readonly property int maxCores: 1024
    /// Points de courbe (`MAX_FAN_TABLE_ENTRIES`).
    readonly property int maxFanCurvePoints: 256

    function normalizedName(name) {
        return String(name === undefined || name === null ? "" : name).trim().toLowerCase();
    }

    /// Valide le formulaire. `otherNames` contient les noms normalisés des
    /// autres profils (unicité insensible à la casse).
    ///
    /// Renvoie `{ ok: bool, errors: { champ: cléI18n } }`.
    function validate(form, otherNames) {
        var errors = {
            "name": "",
            "description": "",
            "brightness": "",
            "refreshRate": "",
            "resolution": "",
            "odmProfile": "",
            "tdpValues": "",
            "ctgpOffset": "",
            "cores": "",
            "freqMin": "",
            "freqMax": "",
            "fanMin": "",
            "fanMax": "",
            "fanOffset": "",
            "fanCurve": ""
        };
        var name = String(form.name === undefined ? "" : form.name).trim();
        if (name === "") {
            errors.name = "editor.error.nameEmpty";
        } else if (name.length > root.maxNameLength) {
            errors.name = "editor.error.nameTooLong";
        } else if (Array.isArray(otherNames)
                   && otherNames.indexOf(root.normalizedName(name)) >= 0) {
            errors.name = "editor.error.nameDuplicate";
        }
        if (String(form.description === undefined ? "" : form.description).length
                > root.maxDescriptionLength) {
            errors.description = "editor.error.descriptionTooLong";
        }
        if (!(form.brightness >= 0 && form.brightness <= 100))
            errors.brightness = "editor.error.brightnessRange";
        if (form.useRefreshRate) {
            var rate = Number(form.refreshRate);
            if (form.refreshExists === false) {
                errors.refreshRate = "editor.error.refreshUnavailable";
            } else if (!isFinite(rate) || rate <= 0 || rate > 1000) {
                errors.refreshRate = "editor.error.refreshRange";
            }
        }
        if (form.odmProfileDirty === true) {
            var odm = String(form.odmProfileName === undefined ? "" : form.odmProfileName).trim();
            if (odm.length > 64
                    || (odm !== "" && !/^[A-Za-z0-9_-]+$/.test(odm))) {
                errors.odmProfile = "editor.error.odmProfile";
            }
        }
        if (form.tdpDirty === true && Array.isArray(form.tdpValues)
                && form.tdpValues.length > 0) {
            if (form.tdpValues.length > 3) {
                errors.tdpValues = "editor.error.tdpValues";
            } else {
                var previous = -Infinity;
                for (var t = 0; t < form.tdpValues.length; ++t) {
                    var tdp = Number(form.tdpValues[t]);
                    if (!isFinite(tdp) || tdp < 1 || tdp > 1000 || tdp <= previous) {
                        errors.tdpValues = "editor.error.tdpValues";
                        break;
                    }
                    previous = tdp;
                }
            }
        } else if (form.tdpDirty === true && !Array.isArray(form.tdpValues)) {
            errors.tdpValues = "editor.error.tdpValues";
        }
        if (form.ctgpDirty === true) {
            var ctgp = Number(form.ctgpOffset);
            var ctgpMax = form.ctgpMax !== undefined ? Number(form.ctgpMax) : 200;
            if (!isFinite(ctgp) || ctgp < 0 || !isFinite(ctgpMax)
                    || ctgp > ctgpMax)
                errors.ctgpOffset = "editor.error.ctgpOffset";
        }
        if (form.useResolution) {
            var width = Number(form.resolutionWidth);
            var height = Number(form.resolutionHeight);
            if (form.resolutionExists === false) {
                errors.resolution = "editor.error.resolutionUnavailable";
            } else if (!isFinite(width) || !isFinite(height)
                       || width < 1 || width > 16384
                       || height < 1 || height > 16384) {
                errors.resolution = "editor.error.resolutionRange";
            }
        }
        // Les champs `Option` de `tcc-core` ne sont validés que s'ils sont
        // connus : un champ absent reste absent (aucune valeur inventée).
        if (form.coresKnown !== false
                && !(form.cores >= 1 && form.cores <= root.maxCores)) {
            errors.cores = "editor.error.coresRange";
        }
        if (form.freqMinKnown !== false
                && !(form.freqMin >= root.minGhz && form.freqMin <= root.maxGhz)) {
            errors.freqMin = "editor.error.freqRange";
        }
        if (form.freqMaxKnown !== false && form.freqMaxReduced !== true
                && !(form.freqMax >= root.minGhz && form.freqMax <= root.maxGhz)) {
            errors.freqMax = "editor.error.freqRange";
        } else if (form.freqMaxKnown !== false && form.freqMaxReduced !== true
                   && form.freqMinKnown !== false && form.freqMin > form.freqMax) {
            errors.freqMin = "editor.error.freqOrder";
        }
        if (!(form.fanMin >= 0 && form.fanMin <= 100))
            errors.fanMin = "editor.error.fanRange";
        if (!(form.fanMax >= 0 && form.fanMax <= 100))
            errors.fanMax = "editor.error.fanRange";
        if (form.fanMin > form.fanMax)
            errors.fanMin = "editor.error.fanOrder";
        if (!(form.fanOffset >= -100 && form.fanOffset <= 100))
            errors.fanOffset = "editor.error.fanOffsetRange";

        // Courbe personnalisée : validée uniquement en mode custom (c'est le
        // seul mode où le DTO l'envoie).
        if (form.fanProfile === "custom") {
            var curve = form.fanCurve;
            if (!Array.isArray(curve) || curve.length < 2) {
                errors.fanCurve = "fanCurve.error.min";
            } else if (curve.length > root.maxFanCurvePoints) {
                errors.fanCurve = "fanCurve.error.max";
            } else {
                var previous = -Infinity;
                for (var i = 0; i < curve.length; ++i) {
                    var point = curve[i];
                    if (!point || typeof point !== "object") {
                        errors.fanCurve = "fanCurve.error.number";
                        break;
                    }
                    var temp = Number(point.temp);
                    var speed = Number(point.speed);
                    if (!isFinite(temp) || !isFinite(speed)) {
                        errors.fanCurve = "fanCurve.error.number";
                        break;
                    }
                    if (temp < 0 || temp > 100 || speed < 0 || speed > 100) {
                        errors.fanCurve = "fanCurve.error.range";
                        break;
                    }
                    if (temp <= previous) {
                        errors.fanCurve = "fanCurve.error.order";
                        break;
                    }
                    previous = temp;
                }
            }
        }

        var ok = true;
        for (var key in errors) {
            if (errors[key] !== "")
                ok = false;
        }
        return { "ok": ok, "errors": errors };
    }

    /// Première erreur (pour un bandeau de section), ou `""`.
    function firstError(errors, fields) {
        for (var i = 0; i < fields.length; ++i) {
            if (errors[fields[i]] !== "")
                return errors[fields[i]];
        }
        return "";
    }

    /// Convertit le formulaire en DTO `ProfileEdit` (camelCase).
    function toEdit(form) {
        var rate = Number(form.refreshRate);
        return {
            "name": String(form.name).trim(),
            "description": String(form.description === undefined ? "" : form.description),
            "brightness": Math.round(form.brightness),
            "useBrightness": form.useBrightness === true,
            "refreshRate": form.useRefreshRate === true && form.refreshExists !== false
                           && isFinite(rate) && rate > 0 ? rate : null,
            "useRefreshRate": form.useRefreshRate === true && form.refreshExists !== false,
            "useResolution": form.useResolution === true
                             && form.resolutionExists !== false
                             && Number(form.resolutionWidth) > 0
                             && Number(form.resolutionHeight) > 0,
            "xResolution": form.useResolution === true
                           && form.resolutionExists !== false
                           ? Number(form.resolutionWidth) : null,
            "yResolution": form.useResolution === true
                           && form.resolutionExists !== false
                           ? Number(form.resolutionHeight) : null,
            "odmProfile": form.odmProfileDirty === true
                          && String(form.odmProfileName === undefined
                                    ? "" : form.odmProfileName).trim() !== ""
                          ? String(form.odmProfileName).trim() : null,
            "tdpValues": form.tdpDirty === true
                         ? (Array.isArray(form.tdpValues)
                            ? form.tdpValues.map(function(value) {
                                  return Number(value);
                              })
                            : null)
                         : null,
            "ctgpOffset": form.ctgpDirty === true
                          ? Number(form.ctgpOffset) : null,
            // `null` = ne pas modifier (champ absent du profil) ; `-1` =
            // marqueur « réduit » côté `tcc-core`.
            "onlineCores": form.coresKnown === false ? null : Math.round(form.cores),
            "useMaxPerfGov": form.maxPerfGov === true,
            "scalingMinKhz": form.freqMinKnown === false
                             ? null : Math.round(form.freqMin * 1000000),
            "scalingMaxKhz": form.freqMaxReduced === true
                             ? -1
                             : (form.freqMaxKnown === false
                                ? null : Math.round(form.freqMax * 1000000)),
            "energyPerformancePreference": String(form.epp),
            "noTurbo": form.noTurbo === true,
            "webcamStatus": form.webcamOn === true,
            "useWebcamStatus": form.useWebcam === true,
            "fanUseControl": form.fanUseControl === true,
            "fanProfile": String(form.fanProfile),
            "fanMin": Number(form.fanMin),
            "fanMax": Number(form.fanMax),
            "fanOffset": Number(form.fanOffset),
            // `null` = ne pas modifier la courbe existante (nom/GPU préservés
            // côté Rust). La table CPU seule est éditée par la GUI.
            "customFanCurve": form.fanProfile === "custom"
                ? { "tableCPU": (Array.isArray(form.fanCurve) ? form.fanCurve : [])
                    .map(function(point) {
                        return { "temp": Number(point.temp),
                                 "speed": Number(point.speed) };
                    }) }
                : null
        };
    }
}
