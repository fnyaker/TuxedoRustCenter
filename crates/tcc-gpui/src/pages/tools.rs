// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Outils : minuterie d'extinction, éclairage du clavier et contrôles UVC de
//! la webcam.
//!
//! Minuterie et éclairage sont des réglages persistants
//! (`Admin1.SaveSettingsJSON`). Les contrôles UVC sont appliqués tout de
//! suite par l'agent de session (`User1.SetUvcControl`), sans privilège.

use std::rc::Rc;

use gpui_kit::{Context, Window, div, prelude::*, rems};
use gpui_omarchy::*;
use serde_json::Value;
use tcc_core::{KeyboardColorMode, KeyboardState, LightbarState};

use crate::app::App;
use crate::i18n::{t, tf};
use crate::model::{self, DASH};
use crate::widgets;
use crate::worker::{Command, SettingsTag};

/// Éclairage en cours de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct LightDraft {
    pub keyboard: f32,
    pub keyboard_rgb: [f32; 3],
    pub breathing: bool,
    pub lightbar: f32,
    pub lightbar_rgb: [f32; 3],
}

/// Capacités d'une zone d'éclairage.
struct Zone {
    available: bool,
    writable: bool,
    rgb: bool,
    max: f32,
    max_rgb: [f32; 3],
}

fn channel_max(value: &Value) -> f32 {
    value
        .as_f64()
        .filter(|max| *max > 0.)
        .map_or(255., |max| max.min(255.) as f32)
}

fn zone(value: &Value, rgb: bool) -> Zone {
    Zone {
        available: value["available"] == true,
        writable: value["writable"] == true && value["maxBrightness"].as_f64().unwrap_or(0.) > 0.,
        rgb,
        max: channel_max(&value["maxBrightness"]),
        max_rgb: [
            channel_max(&value["maxRed"]),
            channel_max(&value["maxGreen"]),
            channel_max(&value["maxBlue"]),
        ],
    }
}

fn level(value: &Value) -> f32 {
    value.as_f64().filter(|v| *v >= 0.).unwrap_or(0.) as f32
}

fn two_digits() -> impl Fn(u32) -> (String, String) {
    |value| (value.to_string(), format!("{value:02}"))
}

/// Un contrôle UVC est-il réglable depuis l'interface ?
fn uvc_editable(control: &Value) -> bool {
    let flags = &control["flags"];
    if ["readOnly", "disabled", "grabbed", "inactive", "writeOnly"]
        .iter()
        .any(|flag| flags[*flag] == true)
    {
        return false;
    }
    let current = &control["current"];
    match control["kind"].as_str() {
        Some("integer") => {
            current["kind"] == "integer"
                && control["max"]
                    .as_i64()
                    .zip(control["min"].as_i64())
                    .is_some_and(|(max, min)| max > min)
        }
        Some("boolean") => current["kind"] == "boolean",
        Some("menu" | "integerMenu") => control["menu"].as_array().is_some_and(|m| !m.is_empty()),
        _ => false,
    }
}

/// Valeur lisible d'un contrôle UVC.
fn uvc_value(control: &Value) -> String {
    let current = &control["current"];
    match current["kind"].as_str() {
        Some("menu") => {
            let label = model::text(&current["label"]);
            if label.is_empty() {
                model::text(&current["key"])
            } else {
                label
            }
        }
        Some("boolean") => t(if current["boolean"] == true {
            "common.yes"
        } else {
            "common.no"
        }),
        Some("integer" | "integerMenu") => current["value"]
            .as_i64()
            .map_or_else(|| DASH.to_owned(), |v| v.to_string()),
        _ => DASH.to_owned(),
    }
}

impl App {
    pub(crate) fn tools(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let hw = &snap.hardware;
        let status = if snap.connected {
            badge(t("app.live"), Status::Success, cx)
        } else {
            badge(t("state.disconnected"), Status::Warning, cx)
        };
        let mut page = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(widgets::page_header(
                "",
                t("tools.title"),
                t("tools.subtitle"),
                status,
                cx,
            ))
            .children(self.admin_notice(cx))
            .child(self.tools_timer(window, cx));
        let lighting = &hw["lighting"];
        if lighting["keyboard"]["available"] == true || lighting["lightbar"]["available"] == true {
            page = page.child(self.tools_lighting(window, cx));
        }
        let webcam = &hw["webcam"];
        if webcam["switchAvailable"] == true
            || webcam["devices"].as_array().is_some_and(|d| !d.is_empty())
        {
            page = page.child(self.tools_webcam(window, cx));
        }
        page
    }

    fn tools_timer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let timer = &snap.timer;
        let available = timer.is_object();
        let error = model::text(&timer["error"]);
        let scheduled = timer["enabled"] == true;
        let at = model::text(&timer["scheduledAt"]);
        let persisted = self
            .settings()
            .and_then(|s| s.shutdown_time.clone())
            .unwrap_or_default();
        let live = model::parse_shutdown_time(&persisted)
            .map_or((false, 22, 0), |(hour, minute)| (true, hour, minute));
        let (enabled, hour, minute) = self.timer.unwrap_or(live);
        let editable = self.settings_editable() && available && error.is_empty();
        let dirty = self.timer.is_some();

        let state = if !error.is_empty() {
            badge(t("tools.timer.statusError"), Status::Error, cx)
        } else if scheduled {
            badge(
                tf(
                    "tools.timer.statusScheduled",
                    &[if at.is_empty() { &persisted } else { &at }],
                ),
                Status::Success,
                cx,
            )
        } else {
            badge(t("tools.timer.statusNone"), Status::Neutral, cx)
        };
        let mut card = widgets::card(
            t("tools.timer.title"),
            t("tools.timer.desc"),
            Some(state.into_any_element()),
            cx,
        );
        if !available {
            card = card.child(widgets::notice(
                Status::Warning,
                t("tools.timer.unavailable"),
                t("tools.timer.unavailableHint"),
                None,
                cx,
            ));
        } else {
            if !error.is_empty() {
                card = card.child(widgets::notice(
                    Status::Error,
                    t("tools.timer.daemonError"),
                    error.clone(),
                    None,
                    cx,
                ));
            }
            let source = match timer["source"].as_str() {
                Some("logind") => "tools.timer.sourceLogind",
                Some("settings") => "tools.timer.sourceSettings",
                Some("none") => "tools.timer.sourceNone",
                Some("error") => "tools.timer.sourceError",
                _ => "tools.timer.sourceUnknown",
            };
            card = card
                .child(widgets::info_row(
                    t("tools.timer.scheduledAt"),
                    if !scheduled {
                        t("common.none")
                    } else if at.is_empty() {
                        t("common.yes")
                    } else {
                        at.clone()
                    },
                    None,
                    cx,
                ))
                .child(widgets::info_row(
                    t("tools.timer.source"),
                    t(source),
                    None,
                    cx,
                ));
        }
        // Tout changement repart de la valeur affichée ; revenir à la valeur
        // du daemon abandonne le brouillon.
        let set = move |this: &mut App, next: (bool, u32, u32)| {
            this.timer = (next != live).then_some(next);
        };
        let toggle = self.toggle(
            "timer-enable",
            t("tools.timer.enable"),
            enabled,
            !editable,
            move |this, on, _| set(this, (on, hour, minute)),
            cx,
        );
        card = card.child(widgets::row(
            t("tools.timer.enable"),
            t("tools.timer.enableHint"),
            toggle,
            cx,
        ));
        let hours = self.select(
            "timer/hour",
            (0..24).map(two_digits()).collect(),
            Some(&hour.to_string()),
            !editable || !enabled,
            5.,
            move |this, value, _| {
                if let Ok(value) = value.parse() {
                    set(this, (enabled, value, minute));
                }
            },
            window,
            cx,
        );
        let minutes = self.select(
            "timer/minute",
            (0..60).map(two_digits()).collect(),
            Some(&minute.to_string()),
            !editable || !enabled,
            5.,
            move |this, value, _| {
                if let Ok(value) = value.parse() {
                    set(this, (enabled, hour, value));
                }
            },
            window,
            cx,
        );
        card = card.child(widgets::row(
            t("tools.timer.at"),
            "",
            div()
                .flex()
                .items_center()
                .gap(rems(0.375))
                .child(hours)
                .child(":")
                .child(minutes),
            cx,
        ));
        let cancel = button(
            "timer-cancel",
            t("tools.timer.cancel"),
            ButtonVariant::Outline,
            cx,
        )
        .disabled(!editable || !(scheduled || !persisted.is_empty()))
        .on_click(cx.listener(|this, _, _, _| {
            this.timer = None;
            this.patch_settings(SettingsTag::Timer(true), |s| s.shutdown_time = None);
        }));
        let apply = button("timer-apply", t("common.apply"), ButtonVariant::Primary, cx)
            .disabled(!editable || !dirty)
            .on_click(cx.listener(move |this, _, _, _| {
                let time = enabled.then(|| format!("{hour:02}:{minute:02}"));
                this.handle.send(Command::Settings {
                    tag: SettingsTag::Timer(!enabled),
                    patch: Box::new(move |s| s.shutdown_time = time),
                });
            }));
        card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(rems(0.75))
                .child(widgets::muted(
                    if dirty {
                        t("common.unsavedChanges")
                    } else {
                        String::new()
                    },
                    cx,
                ))
                .child(div().flex().gap(rems(0.5)).child(cancel).child(apply)),
        )
    }

    fn light_live(&self) -> LightDraft {
        let lighting = &self.snap.hardware["lighting"];
        let keyboard = &lighting["states"][0];
        let bar = &lighting["lightbarState"];
        let breathing = self
            .settings()
            .and_then(|s| s.keyboard_states.first())
            .is_some_and(|state| state.mode == KeyboardColorMode::Breathing)
            && model::strings(&lighting["keyboard"]["modes"])
                .iter()
                .any(|mode| mode == "breathing");
        LightDraft {
            keyboard: level(&keyboard["brightness"]),
            keyboard_rgb: [
                level(&keyboard["red"]),
                level(&keyboard["green"]),
                level(&keyboard["blue"]),
            ],
            breathing,
            lightbar: level(&bar["brightness"]),
            lightbar_rgb: [
                level(&bar["red"]),
                level(&bar["green"]),
                level(&bar["blue"]),
            ],
        }
    }

    /// Modifie le brouillon d'éclairage à partir de la valeur affichée.
    fn light_edit(&mut self, change: impl FnOnce(&mut LightDraft)) {
        let live = self.light_live();
        let mut next = self.light.clone().unwrap_or_else(|| live.clone());
        change(&mut next);
        self.light = (next != live).then_some(next);
    }

    fn tools_lighting(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let lighting = &snap.hardware["lighting"];
        let kind = model::text(&lighting["keyboard"]["kind"]);
        let keyboard = zone(&lighting["keyboard"], kind == "rgb" || kind == "per-key");
        let lightbar = zone(&lighting["lightbar"], true);
        let zones = lighting["keyboard"]["zones"]
            .as_u64()
            .map_or(1, |zones| zones.clamp(1, 8) as usize);
        let breathing_known = model::strings(&lighting["keyboard"]["modes"])
            .iter()
            .any(|mode| mode == "breathing");
        let enabled = self
            .settings()
            .is_some_and(|s| s.keyboard_backlight_control_enabled);
        let editable = self.settings_editable() && enabled;
        let draft = self.light.clone().unwrap_or_else(|| self.light_live());
        let dirty = self.light.is_some();

        let state = badge(
            t(if editable {
                "tools.lighting.editable"
            } else {
                "common.readOnly"
            }),
            if editable {
                Status::Success
            } else {
                Status::Neutral
            },
            cx,
        );
        let mut card = widgets::card(
            t("tools.lighting.title"),
            t("tools.lighting.desc"),
            Some(state.into_any_element()),
            cx,
        );
        if !enabled && self.settings().is_some() {
            card = card.child(widgets::notice(
                Status::Warning,
                t("settings.lightingControl"),
                t("tools.lighting.globalDisabled"),
                None,
                cx,
            ));
        }
        let channels = [
            "tools.lighting.red",
            "tools.lighting.green",
            "tools.lighting.blue",
        ];
        if keyboard.available {
            let writable = editable && keyboard.writable;
            let max = keyboard.max;
            let on = self.toggle(
                "kb-on",
                t("tools.lighting.keyboardOn"),
                draft.keyboard > 0.,
                !writable,
                move |this, on, _| {
                    this.light_edit(|light| light.keyboard = if on { max } else { 0. });
                },
                cx,
            );
            card = card
                .child(widgets::subheading(t("tools.lighting.keyboard"), cx))
                .child(widgets::row(
                    t("tools.lighting.keyboardOn"),
                    if !keyboard.writable {
                        t("tools.lighting.writeUnavailable")
                    } else if keyboard.rgb {
                        tf("tools.lighting.zonesNote", &[&zones.to_string()])
                    } else {
                        t("tools.lighting.whiteNote")
                    },
                    on,
                    cx,
                ))
                .child(self.slider_row(
                    "light/kb",
                    t("display.brightness"),
                    String::new(),
                    (0., max, 1.),
                    draft.keyboard,
                    format!("{:.0} / {max:.0}", draft.keyboard),
                    !writable,
                    |this, value, _| this.light_edit(|light| light.keyboard = value.round()),
                    window,
                    cx,
                ));
            if keyboard.rgb {
                for (index, label) in channels.into_iter().enumerate() {
                    card = card.child(self.slider_row(
                        &format!("light/kb/{index}"),
                        t(label),
                        String::new(),
                        (0., keyboard.max_rgb[index], 1.),
                        draft.keyboard_rgb[index],
                        format!("{:.0}", draft.keyboard_rgb[index]),
                        !writable,
                        move |this, value, _| {
                            this.light_edit(|light| light.keyboard_rgb[index] = value.round());
                        },
                        window,
                        cx,
                    ));
                }
            }
            let mut modes = vec![("static".to_owned(), t("tools.lighting.modeStatic"))];
            if breathing_known {
                modes.push(("breathing".to_owned(), t("tools.lighting.modeBreathing")));
            }
            let mode = self.select(
                "light/mode",
                modes,
                Some(if draft.breathing {
                    "breathing"
                } else {
                    "static"
                }),
                !writable || !breathing_known,
                14.,
                |this, value, _| this.light_edit(|light| light.breathing = value == "breathing"),
                window,
                cx,
            );
            card = card.child(widgets::row(t("tools.lighting.mode"), "", mode, cx));
        }
        if lightbar.available {
            let writable = editable && lightbar.writable;
            let max = lightbar.max;
            let on = self.toggle(
                "lb-on",
                t("tools.lighting.lightbarOn"),
                draft.lightbar > 0.,
                !writable,
                move |this, on, _| {
                    this.light_edit(|light| light.lightbar = if on { max } else { 0. });
                },
                cx,
            );
            if keyboard.available {
                card = card.child(separator(cx));
            }
            card = card
                .child(widgets::subheading(t("tools.lighting.lightbar"), cx))
                .child(widgets::row(t("tools.lighting.lightbarOn"), "", on, cx))
                .child(self.slider_row(
                    "light/lb",
                    t("display.brightness"),
                    String::new(),
                    (0., max, 1.),
                    draft.lightbar,
                    format!("{:.0} / {max:.0}", draft.lightbar),
                    !writable,
                    |this, value, _| this.light_edit(|light| light.lightbar = value.round()),
                    window,
                    cx,
                ));
            for (index, label) in channels.into_iter().enumerate() {
                card = card.child(self.slider_row(
                    &format!("light/lb/{index}"),
                    t(label),
                    String::new(),
                    (0., lightbar.max_rgb[index], 1.),
                    draft.lightbar_rgb[index],
                    format!("{:.0}", draft.lightbar_rgb[index]),
                    !writable,
                    move |this, value, _| {
                        this.light_edit(|light| light.lightbar_rgb[index] = value.round());
                    },
                    window,
                    cx,
                ));
            }
        }
        let keyboard_write = keyboard.available && keyboard.writable;
        let keyboard_rgb = keyboard.rgb;
        let lightbar_write = lightbar.available && lightbar.writable;
        card.child(self.apply_bar(
            "lighting",
            if dirty {
                t("tools.lighting.dirty")
            } else {
                String::new()
            },
            dirty,
            dirty && editable && (keyboard_write || lightbar_write),
            |this| this.light = None,
            move |this| {
                let Some(light) = this.light.clone() else {
                    return;
                };
                this.handle.send(Command::Settings {
                    tag: SettingsTag::Lighting,
                    patch: Box::new(move |settings| {
                        let byte = |value: f32| value.round().clamp(0., 255.) as u8;
                        let channel = |value: f32| i16::from(byte(value));
                        if keyboard_write {
                            let [red, green, blue] = if keyboard_rgb {
                                light.keyboard_rgb.map(channel)
                            } else {
                                [-1; 3]
                            };
                            let mode = if light.breathing {
                                KeyboardColorMode::Breathing
                            } else {
                                KeyboardColorMode::Static
                            };
                            settings.keyboard_states = vec![
                                KeyboardState::new(
                                    mode,
                                    byte(light.keyboard),
                                    red,
                                    green,
                                    blue
                                );
                                zones
                            ];
                        }
                        if lightbar_write {
                            let [red, green, blue] = light.lightbar_rgb.map(channel);
                            settings.lightbar_states = Some(LightbarState {
                                brightness: byte(light.lightbar),
                                red,
                                green,
                                blue,
                            });
                        }
                    }),
                });
            },
            cx,
        ))
    }

    fn tools_webcam(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let webcam = &snap.hardware["webcam"];
        let devices: Vec<Value> = webcam["devices"]
            .as_array()
            .map(|devices| {
                devices
                    .iter()
                    // Les nœuds de métadonnées sans contrôle ne sont pas des caméras.
                    .filter(|device| {
                        device["metadata"] != true
                            || device["controls"].as_array().is_some_and(|c| !c.is_empty())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let state = badge(
            if devices.is_empty() {
                t("tools.webcam.noDevice")
            } else {
                tf("tools.webcam.deviceCount", &[&devices.len().to_string()])
            },
            if devices.is_empty() {
                Status::Neutral
            } else {
                Status::Success
            },
            cx,
        );
        let mut card = widgets::card(
            t("tools.webcam.title"),
            t("tools.webcam.desc"),
            Some(state.into_any_element()),
            cx,
        );
        if !snap.uvc_write && !devices.is_empty() {
            card = card.child(widgets::muted(t("tools.webcam.uvcWriteUnavailable"), cx));
        }
        for (device_index, device) in devices.iter().enumerate() {
            let path = model::text(&device["devicePath"]);
            let name = {
                let name = model::text(&device["deviceName"]);
                if name.is_empty() {
                    t("tools.webcam.device")
                } else {
                    name
                }
            };
            let controls = device["controls"].as_array().cloned().unwrap_or_default();
            if device_index > 0 {
                card = card.child(separator(cx));
            }
            card = card.child(widgets::info_row(
                name,
                if path.is_empty() {
                    DASH.to_owned()
                } else {
                    path.clone()
                },
                Some((
                    tf("tools.webcam.controlCount", &[&controls.len().to_string()]),
                    Status::Neutral,
                )),
                cx,
            ));
            let error = model::text(&device["controlsError"]);
            if !error.is_empty() {
                card = card.child(widgets::warning_text(
                    format!("{} {error}", t("tools.webcam.controlsError")),
                    cx,
                ));
            } else if controls.is_empty() && webcam["controlsSupported"] == true {
                card = card.child(widgets::muted(t("tools.webcam.noControls"), cx));
            }
            let writable =
                snap.uvc_write && !snap.busy && device["metadata"] != true && !path.is_empty();
            for control in &controls {
                card = card.child(self.uvc_row(&path, control, writable, window, cx));
            }
        }
        if !devices.is_empty() && webcam["controlsSupported"] != true {
            card = card.child(widgets::notice(
                Status::Neutral,
                t("tools.webcam.controls"),
                t("tools.webcam.controlsUnavailable"),
                None,
                cx,
            ));
        }
        if webcam["controlsTruncated"] == true {
            card = card.child(widgets::warning_text(
                t("tools.webcam.controlsTruncated"),
                cx,
            ));
        }
        if webcam["switchAvailable"] == true {
            card = card.child(widgets::info_row(
                t("tools.webcam.killswitch"),
                t(if webcam["enabled"] == true {
                    "common.enabled"
                } else {
                    "common.disabled"
                }),
                Some((t("common.readOnly"), Status::Neutral)),
                cx,
            ));
        }
        card
    }

    /// Ligne d'un contrôle UVC : curseur, interrupteur ou liste selon son
    /// type, ligne en lecture seule sinon.
    fn uvc_row(
        &mut self,
        device: &str,
        control: &Value,
        writable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let key = model::text(&control["key"]);
        let label = {
            let name = model::text(&control["name"]);
            if name.is_empty() { key.clone() } else { name }
        };
        if !writable || !uvc_editable(control) {
            return widgets::info_row(
                label,
                uvc_value(control),
                Some((t("common.readOnly"), Status::Neutral)),
                cx,
            );
        }
        let slot = (device.to_owned(), key.clone());
        let id = format!("uvc/{device}/{key}");
        let (min, max) = (
            control["min"].as_i64().unwrap_or(0),
            control["max"].as_i64().unwrap_or(0),
        );
        let step = control["step"]
            .as_i64()
            .filter(|step| *step > 0)
            .unwrap_or(1);
        let details = {
            let mut text = tf(
                "tools.webcam.details",
                &[
                    &min.to_string(),
                    &max.to_string(),
                    &step.to_string(),
                    &control["default"].as_i64().unwrap_or(0).to_string(),
                ],
            );
            let category = model::text(&control["category"]);
            if !category.is_empty() {
                text = format!("{text} · {}", tf("tools.webcam.category", &[&category]));
            }
            text
        };
        let send = {
            let slot = slot.clone();
            move |this: &mut App, value: i32| {
                this.uvc.insert(slot.clone(), value);
                this.handle.send(Command::Uvc {
                    device: slot.0.clone(),
                    key: slot.1.clone(),
                    value,
                });
            }
        };
        match control["kind"].as_str() {
            Some("integer") => {
                let current = control["current"]["value"].as_i64().unwrap_or(min) as i32;
                let value = self.uvc.get(&slot).copied().unwrap_or(current);
                let drag = slot.clone();
                let release: crate::app::NumberChange = Rc::new(move |this, value, _| {
                    let value = value.round() as i32;
                    if value != current {
                        send(this, value);
                    }
                });
                let slider = self.slider(
                    &id,
                    (min as f32, max as f32, step as f32),
                    value as f32,
                    false,
                    move |this, value, _| {
                        this.uvc.insert(drag.clone(), value.round() as i32);
                    },
                    Some(release),
                    window,
                    cx,
                );
                widgets::row(
                    label,
                    details,
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.75))
                        .child(slider)
                        .child(
                            div()
                                .w(rems(3.))
                                .flex()
                                .justify_end()
                                .text_color(cx.omarchy().bright)
                                .child(value.to_string()),
                        ),
                    cx,
                )
            }
            Some("boolean") => {
                let current = control["current"]["boolean"] == true;
                let checked = self.uvc.get(&slot).map_or(current, |value| *value != 0);
                let entity = cx.entity();
                let toggle = switch(gpui_kit::SharedString::from(id), "", checked, cx)
                    .accessibility_label(label.clone())
                    .on_change(widgets::change(move |on: &bool, _, cx| {
                        entity.update(cx, |this, cx| {
                            send(this, i32::from(*on));
                            cx.notify();
                        });
                    }));
                widgets::row(label, "", toggle, cx)
            }
            _ => {
                let menu = control["menu"].as_array().cloned().unwrap_or_default();
                let items = menu
                    .iter()
                    .filter_map(|entry| {
                        Some((
                            entry["index"].as_i64()?.to_string(),
                            model::text(&entry["label"]),
                        ))
                    })
                    .collect();
                let current = self
                    .uvc
                    .get(&slot)
                    .map(|value| i64::from(*value))
                    .or_else(|| control["current"]["index"].as_i64())
                    .map(|index| index.to_string());
                let control = self.select(
                    &id,
                    items,
                    current.as_deref(),
                    false,
                    14.,
                    move |this, value, _| {
                        if let Ok(value) = value.parse() {
                            send(this, value);
                        }
                    },
                    window,
                    cx,
                );
                widgets::row(label, "", control, cx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uvc_controls_are_editable_only_when_writable_and_typed() {
        let integer = json!({
            "kind": "integer", "min": 0, "max": 100,
            "current": {"kind": "integer", "value": 50},
            "flags": {"readOnly": false}
        });
        assert!(uvc_editable(&integer));
        assert_eq!(uvc_value(&integer), "50");
        let mut locked = integer.clone();
        locked["flags"]["inactive"] = json!(true);
        assert!(!uvc_editable(&locked));
        let flat = json!({"kind": "integer", "min": 5, "max": 5,
            "current": {"kind": "integer", "value": 5}});
        assert!(!uvc_editable(&flat));
        let menu = json!({"kind": "menu", "menu": [{"index": 1, "label": "50 Hz"}],
            "current": {"kind": "menu", "index": 1, "label": "50 Hz"}});
        assert!(uvc_editable(&menu));
        assert_eq!(uvc_value(&menu), "50 Hz");
        assert!(!uvc_editable(&json!({"kind": "button"})));
    }

    #[test]
    fn lighting_zones_read_their_limits() {
        let white = zone(
            &json!({"available": true, "writable": true, "maxBrightness": 4, "maxRed": null}),
            false,
        );
        assert!(white.available && white.writable && !white.rgb);
        assert_eq!(white.max, 4.);
        assert_eq!(white.max_rgb, [255.; 3]);
        let none = zone(&json!({"available": false, "maxBrightness": 0}), true);
        assert!(!none.writable);
        assert_eq!(level(&json!(-1)), 0.);
    }
}
