// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Réglages : daemon, apparence, domaines pilotés, matrice de bascule,
//! charge de la batterie, affichage, PRIME, capacités matérielles (ODM, TDP,
//! cTGP) et services.
//!
//! Les réglages persistants passent par `Admin1.SaveSettingsJSON` ; ODM, TDP
//! et cTGP immédiats passent par `AdminHw1` et ne sont pas persistés (le
//! profil actif les réapplique). Les setters `Manager1` ne sont pas utilisés.

use std::rc::Rc;

use gpui_kit::{AnyElement, Context, Window, div, prelude::*, rems};
use gpui_omarchy::*;
use serde_json::Value;

use crate::app::App;
use crate::i18n::{self, Lang, t, tf};
use crate::model::{self, DASH};
use crate::widgets;
use crate::worker::{Command, SettingsTag};

/// Types de charge standards du noyau, proposés quand le firmware n'en
/// annonce aucun.
const STANDARD_CHARGE_TYPES: [&str; 7] = [
    "Trickle", "Fast", "Standard", "Adaptive", "Custom", "LongLife", "Bypass",
];

/// Modifications de charge pas encore appliquées (`None` = valeur du daemon).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChargeDraft {
    pub profile: Option<String>,
    pub priority: Option<String>,
    pub start: Option<u8>,
    pub end: Option<u8>,
    /// Type de charge ; chaîne vide = ne pas changer.
    pub kind: Option<String>,
}

/// Valeurs de charge de référence : le réglage persisté prime, sinon l'état
/// matériel.
struct ChargeLive {
    profile: String,
    priority: String,
    start: Option<u8>,
    end: Option<u8>,
}

fn percent(value: &Value) -> Option<u8> {
    value.as_u64().and_then(|v| u8::try_from(v).ok())
}

fn parse_watts(text: &str) -> Option<f64> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Erreur des limites TDP saisies (clé i18n), ou `None`.
fn tdp_error(limits: &[Value], texts: &[String]) -> Option<&'static str> {
    const DESCRIPTORS: [&str; 3] = ["pl1", "pl2", "pl4"];
    if limits.is_empty() || limits.len() > 3 || limits.len() != texts.len() {
        return Some("settings.tdpDescriptorError");
    }
    let mut previous = f64::NEG_INFINITY;
    for (index, (limit, text)) in limits.iter().zip(texts).enumerate() {
        if !model::text(&limit["descriptor"]).eq_ignore_ascii_case(DESCRIPTORS[index]) {
            return Some("settings.tdpDescriptorError");
        }
        let (Some(value), Some(min), Some(max)) = (
            parse_watts(text),
            limit["min"].as_f64(),
            limit["max"].as_f64(),
        ) else {
            return Some("settings.tdpRange");
        };
        if value < min || value > max {
            return Some("settings.tdpRange");
        }
        if value < previous {
            return Some("settings.tdpMonotonicError");
        }
        previous = value;
    }
    None
}

impl App {
    pub(crate) fn settings_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let hw = &snap.hardware;
        let status = if snap.connected {
            badge(t("state.connected"), Status::Success, cx)
        } else {
            badge(t("state.disconnected"), Status::Warning, cx)
        };
        let mut page = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(widgets::page_header(
                "",
                t("settings.title"),
                t("settings.subtitle"),
                status,
                cx,
            ))
            .children(self.admin_notice(cx));
        if snap.connected && snap.settings.is_none() {
            page = page.child(widgets::notice(
                Status::Error,
                t("settings.readError"),
                t("settings.readErrorHint"),
                None,
                cx,
            ));
        }
        if snap.busy {
            page = page.child(widgets::notice(
                Status::Neutral,
                t("settings.saving"),
                t("editor.savingHint"),
                None,
                cx,
            ));
        }
        page = page
            .child(self.settings_daemon(cx))
            .child(self.settings_appearance(window, cx))
            .child(self.settings_hardware(cx))
            .child(self.settings_matrix(window, cx));
        if hw["charging"]["available"] == true {
            page = page.child(self.settings_charging(window, cx));
        }
        page = page.child(self.settings_display(cx));
        if hw["prime"]["supported"] == true && hw["nvidia"]["available"] == true {
            page = page.child(self.settings_prime(cx));
        }
        page.children(self.settings_capabilities(window, cx))
            .child(self.settings_services(cx))
    }

    fn settings_daemon(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = &self.snap;
        let state = &snap.state;
        let badge_state = badge(
            t(if snap.connected {
                "state.connected"
            } else {
                "state.disconnected"
            }),
            if snap.connected {
                Status::Success
            } else {
                Status::Warning
            },
            cx,
        );
        let mut card = widgets::card(
            t("settings.daemon"),
            t("settings.daemonHint"),
            Some(badge_state.into_any_element()),
            cx,
        )
        .child(widgets::info_row(
            t("settings.daemonProfiles"),
            snap.profiles.len().to_string(),
            None,
            cx,
        ))
        .child(widgets::info_row(
            t("dash.ppd"),
            model::ppd_label(&model::text(&state["ppdProfile"])),
            None,
            cx,
        ))
        .child(widgets::info_row(
            t("dash.power"),
            model::power_label(&model::text(&state["powerSource"])),
            None,
            cx,
        ));
        if let Some(error) = snap.error.as_ref() {
            card = card.child(widgets::info_row(
                t("settings.daemonError"),
                error.clone(),
                None,
                cx,
            ));
        }
        let retry = button(
            "settings-retry",
            t("common.retry"),
            ButtonVariant::Outline,
            cx,
        )
        .disabled(snap.busy)
        .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::Refresh)));
        card.child(widgets::row(
            t(if snap.connected {
                "app.live"
            } else {
                "state.disconnected"
            }),
            t(if snap.connected {
                "app.liveHint"
            } else {
                "settings.daemonRetryHint"
            }),
            retry,
            cx,
        ))
    }

    fn settings_appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let current = Lang::ALL
            .iter()
            .position(|lang| *lang == i18n::lang())
            .unwrap_or(0);
        let language = self.segmented(
            "settings-language",
            Lang::ALL
                .iter()
                .map(|lang| lang.native_name().to_owned())
                .collect(),
            current,
            |this, index, _| {
                let Some(lang) = Lang::ALL.get(index) else {
                    return;
                };
                i18n::set_lang(*lang);
                this.prefs.language = Some(lang.code().to_owned());
                this.prefs.save();
                this.notify(t("toast.language"), true);
            },
            window,
            cx,
        );
        let fahrenheit = self.fahrenheit();
        let editable = self.settings_editable();
        let units: AnyElement = if editable {
            self.segmented(
                "settings-units",
                vec!["°C".to_owned(), "°F".to_owned()],
                usize::from(fahrenheit),
                |this, index, _| {
                    let value = index == 1;
                    this.patch_settings(SettingsTag::General, move |s| s.fahrenheit = value);
                },
                window,
                cx,
            )
        } else {
            badge(model::temp_unit(fahrenheit), Status::Neutral, cx).into_any_element()
        };
        widgets::card(
            t("settings.appearance"),
            t("settings.appearanceHint"),
            None,
            cx,
        )
        .child(widgets::row(
            t("settings.theme"),
            t("gpui.themeFollowsSystem"),
            badge(cx.omarchy().name.clone(), Status::Neutral, cx),
            cx,
        ))
        .child(widgets::row(
            t("settings.language"),
            "",
            div().w(rems(20.)).child(language),
            cx,
        ))
        .child(widgets::row(
            t("settings.units"),
            "",
            div().w(rems(8.)).child(units),
            cx,
        ))
    }

    fn settings_hardware(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let hw = &snap.hardware;
        let editable = self.settings_editable();
        let settings = snap.settings.clone().unwrap_or_default();
        let hint = |text: String| {
            if snap.admin {
                text
            } else {
                t("settings.adminRequired")
            }
        };
        let cpu = self.toggle(
            "set-cpu",
            t("settings.cpuControl"),
            settings.cpu_settings_enabled,
            !editable,
            |this, on, _| {
                this.patch_settings(SettingsTag::General, move |s| s.cpu_settings_enabled = on);
            },
            cx,
        );
        let fan = self.toggle(
            "set-fan",
            t("settings.fanControl"),
            settings.fan_control_enabled,
            !editable,
            |this, on, _| {
                this.patch_settings(SettingsTag::General, move |s| s.fan_control_enabled = on);
            },
            cx,
        );
        let mut card = widgets::card(t("settings.hardware"), t("settings.hardwareHint"), None, cx)
            .child(widgets::row(
                t("settings.cpuControl"),
                hint(t("settings.cpuControlHint")),
                cpu,
                cx,
            ))
            .child(widgets::row(
                t("settings.fanControl"),
                hint(t("settings.fanControlHint")),
                fan,
                cx,
            ));
        let lighting = &hw["lighting"];
        if lighting["keyboard"]["available"] == true || lighting["lightbar"]["available"] == true {
            let kind = model::text(&lighting["keyboard"]["kind"]);
            let toggle = self.toggle(
                "set-lighting",
                t("settings.lightingControl"),
                settings.keyboard_backlight_control_enabled,
                !editable,
                |this, on, _| {
                    this.patch_settings(SettingsTag::General, move |s| {
                        s.keyboard_backlight_control_enabled = on;
                    });
                },
                cx,
            );
            card = card.child(widgets::row(
                t("settings.lightingControl"),
                hint(format!(
                    "{} {}",
                    t("settings.lightingKind"),
                    if kind.is_empty() || kind == "none" {
                        t("common.unavailable")
                    } else {
                        kind
                    }
                )),
                toggle,
                cx,
            ));
        }
        let fn_lock = &hw["fnLock"];
        if fn_lock["supported"] == true {
            let enabled = fn_lock["enabled"].as_bool();
            match enabled.filter(|_| fn_lock["writable"] == true) {
                Some(enabled) => {
                    let toggle = self.toggle(
                        "set-fn-lock",
                        t("settings.fnLock"),
                        settings.fn_lock_status.unwrap_or(enabled),
                        !editable,
                        |this, on, _| {
                            this.patch_settings(SettingsTag::FnLock, move |s| {
                                s.fn_lock_status = Some(on);
                            });
                        },
                        cx,
                    );
                    card = card.child(widgets::row(
                        t("settings.fnLock"),
                        hint(t("settings.fnLockHint")),
                        toggle,
                        cx,
                    ));
                }
                None => {
                    card = card.child(widgets::info_row(
                        t("settings.fnLock"),
                        match enabled {
                            Some(true) => t("common.enabled"),
                            Some(false) => t("common.disabled"),
                            None => t("settings.fnLockUnknown"),
                        },
                        Some((t("common.readOnly"), Status::Neutral)),
                        cx,
                    ));
                }
            }
        }
        card.child(widgets::muted(t("settings.hardwareNote"), cx))
    }

    fn settings_matrix(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let theme = cx.omarchy().clone();
        let editable = self.settings_editable() && !snap.profiles.is_empty();
        let settings = snap.settings.clone().unwrap_or_default();
        let mut items = vec![(String::new(), t("settings.matrixFallback"))];
        items.extend(
            snap.profiles
                .iter()
                .map(|p| (p.id.as_str().to_owned(), p.name.clone())),
        );
        let state = badge(
            t(if editable {
                "common.editable"
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
            t("settings.switching"),
            t("settings.switchingHint"),
            Some(state.into_any_element()),
            cx,
        );
        let column = |label: String| div().w(rems(15.)).text_color(theme.secondary).child(label);
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .child(div().flex_1())
                .child(column(t("dash.ac")))
                .child(column(t("dash.battery"))),
        );
        let mut invalid = false;
        for ppd in model::MATRIX_PPD {
            let mut line = div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .child(div().flex_1().child(model::ppd_label(ppd)));
            for power in model::MATRIX_POWER {
                let cell = format!("{ppd}/{power}");
                let live = model::matrix_cell(&settings, &cell).unwrap_or_default();
                let value = self
                    .matrix
                    .get(&cell)
                    .map_or_else(|| live.clone(), |draft| draft.clone().unwrap_or_default());
                let unknown = !value.is_empty() && !items.iter().any(|(id, _)| *id == value);
                invalid |= unknown;
                let key = cell.clone();
                let control = self.select(
                    &format!("matrix/{cell}"),
                    items.clone(),
                    Some(&value),
                    !editable,
                    15.,
                    move |this, chosen, _| {
                        if chosen == live {
                            this.matrix.remove(&key);
                        } else {
                            this.matrix.insert(
                                key.clone(),
                                (!chosen.is_empty()).then(|| chosen.to_owned()),
                            );
                        }
                    },
                    window,
                    cx,
                );
                line = line.child(control);
            }
            card = card.child(line);
        }
        if invalid {
            card = card.child(widgets::error_text(t("settings.matrixInvalidProfile"), cx));
        }
        card = card.child(widgets::muted(t("settings.matrixFallbackHint"), cx));
        let dirty = !self.matrix.is_empty();
        card = card.child(self.apply_bar(
            "matrix",
            if dirty {
                t("settings.matrixDirtyHint")
            } else {
                String::new()
            },
            dirty,
            dirty && editable,
            |this| this.matrix.clear(),
            |this| {
                let draft = this.matrix.clone();
                this.handle.send(Command::Settings {
                    tag: SettingsTag::Matrix,
                    patch: Box::new(move |settings| {
                        for (cell, profile) in &draft {
                            model::set_matrix_cell(settings, cell, profile.as_deref());
                        }
                    }),
                });
            },
            cx,
        ));
        let manual = snap.state["manualOverride"] == true;
        card.child(widgets::info_row(
            t("settings.switchingManual"),
            if manual {
                model::text(&snap.state["activeProfile"]["name"])
            } else {
                t("common.none")
            },
            None,
            cx,
        ))
    }

    /// Ligne « message — Réinitialiser — Appliquer » sous un brouillon.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_bar(
        &self,
        id: &'static str,
        message: String,
        can_reset: bool,
        can_apply: bool,
        reset: impl Fn(&mut App) + 'static,
        apply: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(0.75))
            .child(div().flex_1().min_w_0().child(widgets::muted(message, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        button((id, 0usize), t("common.reset"), ButtonVariant::Outline, cx)
                            .disabled(!can_reset || self.snap.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                reset(this);
                                cx.notify();
                            })),
                    )
                    .child(
                        button((id, 1usize), t("common.apply"), ButtonVariant::Primary, cx)
                            .disabled(!can_apply || self.snap.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                apply(this);
                                cx.notify();
                            })),
                    ),
            )
    }

    fn charge_live(&self) -> ChargeLive {
        let charging = &self.snap.hardware["charging"];
        let settings = self.settings();
        let thresholds = charging["thresholdsAvailable"] == true;
        ChargeLive {
            profile: settings
                .and_then(|s| s.charging_profile.clone())
                .unwrap_or_else(|| model::text(&charging["profile"])),
            priority: settings
                .and_then(|s| s.charging_priority.clone())
                .unwrap_or_else(|| model::text(&charging["priority"])),
            start: settings
                .and_then(|s| s.charging_start_threshold)
                .or_else(|| percent(&charging["startThreshold"]).filter(|_| thresholds)),
            end: settings
                .and_then(|s| s.charging_end_threshold)
                .or_else(|| percent(&charging["endThreshold"]).filter(|_| thresholds)),
        }
    }

    fn settings_charging(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let charging = &snap.hardware["charging"];
        let editable = self.settings_editable();
        let live = self.charge_live();
        let draft = self.charge.clone();
        let profiles = model::strings(&charging["profiles"]);
        let priorities = model::strings(&charging["priorities"]);
        let starts = model::ints(&charging["startThresholds"]);
        let ends = model::ints(&charging["endThresholds"]);
        let thresholds = editable
            && charging["thresholdsAvailable"] == true
            && !starts.is_empty()
            && !ends.is_empty();
        let state = badge(
            t(if editable {
                "settings.chargeEditable"
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
            t("settings.charging"),
            t("settings.chargingHint"),
            Some(state.into_any_element()),
            cx,
        );
        let plain = |values: &[String]| -> Vec<(String, String)> {
            values.iter().map(|v| (v.clone(), v.clone())).collect()
        };
        let percents = |values: &[i64]| -> Vec<(String, String)> {
            values
                .iter()
                .map(|v| (v.to_string(), format!("{v} %")))
                .collect()
        };

        if profiles.is_empty() {
            card = card.child(widgets::info_row(
                t("settings.chargeProfile"),
                if live.profile.is_empty() {
                    DASH.to_owned()
                } else {
                    live.profile.clone()
                },
                None,
                cx,
            ));
        } else {
            let value = draft
                .profile
                .clone()
                .unwrap_or_else(|| live.profile.clone());
            let reference = live.profile.clone();
            let control = self.select(
                "charge/profile",
                plain(&profiles),
                Some(&value),
                !editable,
                14.,
                move |this, chosen, _| {
                    this.charge.profile = (chosen != reference).then(|| chosen.to_owned());
                },
                window,
                cx,
            );
            card = card.child(widgets::row(t("settings.chargeProfile"), "", control, cx));
        }
        if priorities.is_empty() {
            if !live.priority.is_empty() {
                card = card.child(widgets::info_row(
                    t("settings.chargePriority"),
                    live.priority.clone(),
                    None,
                    cx,
                ));
            }
        } else {
            let value = draft
                .priority
                .clone()
                .unwrap_or_else(|| live.priority.clone());
            let reference = live.priority.clone();
            let control = self.select(
                "charge/priority",
                plain(&priorities),
                Some(&value),
                !editable,
                14.,
                move |this, chosen, _| {
                    this.charge.priority = (chosen != reference).then(|| chosen.to_owned());
                },
                window,
                cx,
            );
            card = card.child(widgets::row(t("settings.chargePriority"), "", control, cx));
        }
        let start = draft.start.or(live.start);
        let end = draft.end.or(live.end);
        let range_invalid = (draft.start.is_some() || draft.end.is_some())
            && !matches!((start, end), (Some(start), Some(end)) if start < end);
        if thresholds {
            let reference = live.start;
            let control = self.select(
                "charge/start",
                percents(&starts),
                start.map(|v| v.to_string()).as_deref(),
                false,
                14.,
                move |this, chosen, _| {
                    let chosen = chosen.parse::<u8>().ok();
                    this.charge.start = chosen.filter(|_| chosen != reference);
                },
                window,
                cx,
            );
            card = card.child(widgets::row(t("settings.chargeStart"), "", control, cx));
            let reference = live.end;
            let control = self.select(
                "charge/end",
                percents(&ends),
                end.map(|v| v.to_string()).as_deref(),
                false,
                14.,
                move |this, chosen, _| {
                    let chosen = chosen.parse::<u8>().ok();
                    this.charge.end = chosen.filter(|_| chosen != reference);
                },
                window,
                cx,
            );
            card = card.child(widgets::row(t("settings.chargeEnd"), "", control, cx));
            if range_invalid {
                card = card.child(widgets::error_text(t("settings.chargeRangeError"), cx));
            }
        } else if charging["thresholdsAvailable"] == true {
            let label =
                |value: Option<u8>| value.map_or_else(|| DASH.to_owned(), |v| format!("{v} %"));
            card = card
                .child(widgets::info_row(
                    t("settings.chargeStart"),
                    label(live.start),
                    None,
                    cx,
                ))
                .child(widgets::info_row(
                    t("settings.chargeEnd"),
                    label(live.end),
                    None,
                    cx,
                ));
        }
        let type_writable = charging["chargeTypeWritable"] == true;
        if type_writable {
            let persisted = self
                .settings()
                .and_then(|s| s.charging_type.clone())
                .unwrap_or_default();
            let mut types = model::strings(&charging["chargeTypes"]);
            if types.is_empty() {
                types = STANDARD_CHARGE_TYPES
                    .iter()
                    .map(|v| (*v).to_owned())
                    .collect();
            }
            if !persisted.is_empty() && !types.contains(&persisted) {
                types.push(persisted.clone());
            }
            let mut items = vec![(String::new(), t("settings.chargeTypeUnchanged"))];
            items.extend(plain(&types));
            let value = draft.kind.clone().unwrap_or_else(|| persisted.clone());
            let control = self.select(
                "charge/type",
                items,
                Some(&value),
                !editable,
                14.,
                move |this, chosen, _| {
                    this.charge.kind = (chosen != persisted).then(|| chosen.to_owned());
                },
                window,
                cx,
            );
            card = card.child(widgets::row(t("settings.chargeType"), "", control, cx));
        }
        let current_type = model::text(&charging["chargeType"]);
        if !current_type.is_empty() {
            card = card.child(widgets::info_row(
                t("settings.chargeTypeCurrent"),
                current_type,
                (!type_writable).then(|| (t("common.readOnly"), Status::Neutral)),
                cx,
            ));
        }
        let dirty = draft != ChargeDraft::default();
        card.child(self.apply_bar(
            "charge",
            if dirty && range_invalid {
                t("settings.chargeFixHint")
            } else if dirty {
                t("settings.chargeDirtyHint")
            } else {
                String::new()
            },
            dirty,
            dirty && editable && !range_invalid,
            |this| this.charge = ChargeDraft::default(),
            |this| {
                let draft = this.charge.clone();
                this.handle.send(Command::Settings {
                    tag: SettingsTag::Charging,
                    patch: Box::new(move |settings| {
                        if let Some(profile) = draft.profile {
                            settings.charging_profile = Some(profile);
                        }
                        if let Some(priority) = draft.priority {
                            settings.charging_priority = Some(priority);
                        }
                        if let Some(start) = draft.start {
                            settings.charging_start_threshold = Some(start);
                        }
                        if let Some(end) = draft.end {
                            settings.charging_end_threshold = Some(end);
                        }
                        if let Some(kind) = draft.kind.filter(|kind| !kind.is_empty()) {
                            settings.charging_type = Some(kind);
                        }
                    }),
                });
            },
            cx,
        ))
    }

    fn settings_display(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let usable = self.agent_usable();
        let available = self.agent_available();
        let state = badge(
            t(if usable {
                "display.supported"
            } else if available {
                "display.unsupported"
            } else {
                "display.agentUnavailable"
            }),
            if usable {
                Status::Success
            } else if available {
                Status::Warning
            } else {
                Status::Neutral
            },
            cx,
        );
        let mut card = widgets::card(
            t("settings.display"),
            t("settings.displayHint"),
            Some(state.into_any_element()),
            cx,
        )
        .child(widgets::info_row(
            t("settings.agentState"),
            if !available {
                t("display.agentUnavailable")
            } else if snap.agent_version.is_empty() {
                t("display.agentActive")
            } else {
                snap.agent_version.clone()
            },
            None,
            cx,
        ))
        .children(self.display_facts(cx));
        let modes = self.display_modes();
        if !modes.is_empty() {
            card = card.child(widgets::info_row(
                t("gpui.displayModes"),
                tf("display.modeCount", &[&modes.len().to_string()]),
                None,
                cx,
            ));
        }
        if !available {
            let connector = model::text(&snap.hardware["display"]["internalConnector"]);
            if !connector.is_empty() {
                card = card.child(widgets::info_row(
                    t("display.output"),
                    connector,
                    Some((
                        model::text(&snap.hardware["display"]["source"]),
                        Status::Neutral,
                    )),
                    cx,
                ));
            }
        }
        card = card.children(self.agent_notice(cx));
        let blocker = self.toggle(
            "set-power-blocker",
            t("settings.powerBlocker"),
            snap.power_blocked,
            snap.busy,
            |this, on, _| this.handle.send(Command::PowerBlocker(on)),
            cx,
        );
        card.child(widgets::row(
            t("settings.powerBlocker"),
            format!(
                "{} {}",
                t("settings.powerBlockerHint"),
                t("settings.powerBlockerNote")
            ),
            blocker,
            cx,
        ))
    }

    fn settings_prime(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let prime = &self.snap.hardware["prime"];
        let mut card = widgets::card(t("settings.graphics"), t("settings.graphicsHint"), None, cx)
            .child(widgets::info_row(
                t("settings.graphicsMode"),
                model::prime_label(&model::text(&prime["mode"])),
                Some((t("common.readOnly"), Status::Neutral)),
                cx,
            ));
        if prime["requiresOffloading"] == true {
            card = card.child(widgets::info_row(
                t("settings.graphicsOffload"),
                t("common.yes"),
                None,
                cx,
            ));
        }
        card
    }

    fn settings_capabilities(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let snap = Rc::clone(&self.snap);
        let hw = &snap.hardware;
        let odm = &hw["odm"];
        let nvidia = &hw["nvidia"];
        let ycbcr = &hw["ycbcr"];
        let tdp_exposed = self.tdp_exposed();
        let hw_present = odm["available"] == true || tdp_exposed || nvidia["available"] == true;
        if !hw_present && ycbcr["available"] != true {
            return None;
        }
        let service = snap.admin_hw;
        let editable = service && !snap.busy;
        let state = badge(
            t(if service {
                "settings.hwTransient"
            } else {
                "common.readOnly"
            }),
            if service {
                Status::Warning
            } else {
                Status::Neutral
            },
            cx,
        );
        let mut card = widgets::card(
            t("settings.capabilities"),
            t("settings.capabilitiesHint"),
            Some(state.into_any_element()),
            cx,
        );
        if hw_present && !service {
            let retry = button("hw-retry", t("common.retry"), ButtonVariant::Outline, cx)
                .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::Refresh)))
                .into_any_element();
            card = card.child(widgets::notice(
                Status::Warning,
                t("settings.hwServiceMissing"),
                t("adminHw.unavailable.missing"),
                Some(retry),
                cx,
            ));
        }

        // Profil ODM / platform_profile, appliqué immédiatement.
        let platform = &odm["platformProfile"];
        if service && odm["available"] == true {
            let choices = self.odm_choices();
            let ppd_owned = model::text(&platform["owner"]) == "ppd";
            let writable = odm["odmWritable"] == true;
            let live = [
                model::text(&platform["current"]),
                model::text(&platform["defaultProfile"]),
                model::text(&odm["io"]["defaultProfile"]),
            ]
            .into_iter()
            .find(|name| choices.contains(name))
            .or_else(|| choices.first().cloned())
            .unwrap_or_default();
            let value = self.odm.clone().unwrap_or_else(|| live.clone());
            let reference = live.clone();
            let control = self.select(
                "hw/odm",
                choices
                    .iter()
                    .map(|name| (name.clone(), model::odm_label(name)))
                    .collect(),
                Some(&value),
                !editable || !writable,
                14.,
                move |this, chosen, _| {
                    this.odm = (chosen != reference).then(|| chosen.to_owned());
                },
                window,
                cx,
            );
            let source = if platform["available"] == true {
                "platform_profile"
            } else {
                "tuxedo-io"
            };
            card = card.child(widgets::row(
                t("settings.odm"),
                if ppd_owned {
                    t("settings.hwPpdOwnedHint")
                } else {
                    String::new()
                },
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(badge(source, Status::Neutral, cx))
                    .when(ppd_owned, |row| {
                        row.child(badge(t("settings.hwPpdOwner"), Status::Warning, cx))
                    })
                    .child(control),
                cx,
            ));
            let dirty = self.odm.is_some();
            let consent = self.settings().is_some_and(|s| s.ppd_override);
            let name_ok = model::odm_name_valid(&value);
            let apply = button("odm-apply", t("common.apply"), ButtonVariant::Primary, cx)
                .disabled(!editable || !writable || !dirty || ppd_owned || !name_ok)
                .on_click(cx.listener(|this, _, _, _| {
                    if let Some(name) = this.odm.clone() {
                        this.handle.send(Command::Odm { name, force: false });
                    }
                }));
            let mut buttons = div().flex().justify_end().gap(rems(0.5)).child(apply);
            if ppd_owned {
                buttons = buttons.child(
                    button(
                        "odm-force",
                        t("settings.hwForce"),
                        ButtonVariant::Danger,
                        cx,
                    )
                    .disabled(!editable || !writable || !dirty || !consent || !name_ok)
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(name) = this.odm.clone() {
                            this.handle.send(Command::Odm { name, force: true });
                        }
                    })),
                );
            }
            card = card.child(buttons);
            if platform["available"] == true {
                let toggle = self.toggle(
                    "set-ppd-override",
                    t("settings.ppdOverride"),
                    consent,
                    !self.settings_editable(),
                    |this, on, _| {
                        this.patch_settings(SettingsTag::General, move |s| s.ppd_override = on);
                    },
                    cx,
                );
                card = card.child(widgets::row(
                    t("settings.ppdOverride"),
                    t("settings.ppdOverrideHint"),
                    toggle,
                    cx,
                ));
                if consent {
                    card = card.child(widgets::notice(
                        Status::Warning,
                        t("settings.ppdOverride"),
                        t("settings.ppdOverrideWarning"),
                        None,
                        cx,
                    ));
                }
            }
        }

        // Limites TDP, appliquées immédiatement.
        if service && tdp_exposed {
            let limits = odm["tdp"].as_array().cloned().unwrap_or_default();
            let live: Vec<String> = limits
                .iter()
                .map(|limit| {
                    limit["current"]
                        .as_f64()
                        .map_or_else(String::new, |w| format!("{w:.0}"))
                })
                .collect();
            let texts = self.tdp.clone().unwrap_or_else(|| live.clone());
            card = card
                .child(separator(cx))
                .child(widgets::subheading(t("settings.tdp"), cx));
            for (index, limit) in limits.iter().enumerate() {
                let reference = live.clone();
                let field = self.text_input(
                    &format!("hw/tdp/{index}"),
                    texts.get(index).map_or("", String::as_str),
                    "W",
                    move |this, text, _| {
                        let mut draft = this.tdp.clone().unwrap_or_else(|| reference.clone());
                        if let Some(slot) = draft.get_mut(index) {
                            *slot = text.to_owned();
                        }
                        this.tdp = (draft != reference).then_some(draft);
                    },
                    window,
                    cx,
                );
                card = card.child(widgets::row(
                    format!(
                        "{} {}",
                        t("settings.tdp"),
                        model::text(&limit["descriptor"]).to_uppercase()
                    ),
                    format!(
                        "{:.0}–{:.0} W",
                        limit["min"].as_f64().unwrap_or(0.),
                        limit["max"].as_f64().unwrap_or(0.)
                    ),
                    div().w(rems(8.)).child(field),
                    cx,
                ));
            }
            let error = tdp_error(&limits, &texts);
            let dirty = self.tdp.is_some();
            card = card.child(self.apply_bar(
                "tdp",
                error.map(t).unwrap_or_default(),
                dirty,
                dirty && editable && error.is_none(),
                |this| this.tdp = None,
                |this| {
                    let watts: Option<Vec<i32>> = this.tdp.as_ref().map(|texts| {
                        texts
                            .iter()
                            .filter_map(|text| parse_watts(text))
                            .map(|w| w.round() as i32)
                            .collect()
                    });
                    if let Some(watts) = watts {
                        this.handle.send(Command::Tdp(watts));
                    }
                },
                cx,
            ));
        }

        // Offset cTGP NVIDIA, appliqué immédiatement.
        if service && nvidia["available"] == true {
            let max = nvidia["maxCtgpOffset"].as_f64().unwrap_or(200.);
            let live = nvidia["ctgpOffset"]
                .as_f64()
                .map_or_else(|| "0".to_owned(), |w| format!("{w:.0}"));
            let text = self.ctgp.clone().unwrap_or_else(|| live.clone());
            let valid = parse_watts(&text).is_some_and(|w| (0.0..=max).contains(&w));
            let reference = live.clone();
            let field = self.text_input(
                "hw/ctgp",
                &text,
                "W",
                move |this, text, _| {
                    this.ctgp = (text != reference).then(|| text.to_owned());
                },
                window,
                cx,
            );
            let range = tf("settings.hwCtgpRange", &[&format!("{max:.0}")]);
            card = card.child(separator(cx)).child(widgets::row(
                t("settings.ctgp"),
                range.clone(),
                div().w(rems(8.)).child(field),
                cx,
            ));
            let dirty = self.ctgp.is_some();
            card = card.child(self.apply_bar(
                "ctgp",
                if valid { String::new() } else { range },
                dirty,
                dirty && editable && valid,
                |this| this.ctgp = None,
                |this| {
                    if let Some(watts) = this.ctgp.as_deref().and_then(parse_watts) {
                        this.handle.send(Command::Ctgp(watts.round() as i32));
                    }
                },
                cx,
            ));
        }

        if ycbcr["available"] == true {
            let ports = ycbcr["ports"].as_array().map_or(0, Vec::len);
            card = card.child(widgets::info_row(
                t("settings.ycbcr"),
                tf("settings.ycbcrPorts", &[&ports.to_string()]),
                (ycbcr["pendingModeReapply"] == true)
                    .then(|| (t("settings.ycbcrPending"), Status::Warning)),
                cx,
            ));
        }
        Some(card)
    }

    fn settings_services(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let toggle = self.toggle(
            "set-autostart",
            t("settings.trayAutostart"),
            self.autostart,
            false,
            |this, on, _| match crate::prefs::set_autostart(on) {
                Ok(()) => {
                    this.autostart = on;
                    this.notify(t("gpui.autostartSaved"), true);
                }
                Err(_) => {
                    this.autostart = crate::prefs::autostart_enabled();
                    this.notify(t("settings.autostartWriteFailed"), false);
                }
            },
            cx,
        );
        widgets::card(t("settings.services"), t("settings.servicesHint"), None, cx).child(
            widgets::row(
                t("settings.trayAutostart"),
                t("settings.trayAutostartHint"),
                toggle,
                cx,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn limits() -> Vec<Value> {
        vec![
            json!({"descriptor": "pl1", "min": 10, "max": 45, "current": 35}),
            json!({"descriptor": "pl2", "min": 10, "max": 65, "current": 45}),
        ]
    }

    fn texts(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn tdp_limits_are_bounded_and_ordered() {
        assert_eq!(tdp_error(&limits(), &texts(&["35", "45"])), None);
        assert_eq!(
            tdp_error(&limits(), &texts(&["50", "60"])),
            Some("settings.tdpRange")
        );
        assert_eq!(
            tdp_error(&limits(), &texts(&["40", "30"])),
            Some("settings.tdpMonotonicError")
        );
        assert_eq!(
            tdp_error(&limits(), &texts(&["x", "30"])),
            Some("settings.tdpRange")
        );
        assert_eq!(
            tdp_error(&limits(), &texts(&["35"])),
            Some("settings.tdpDescriptorError")
        );
        let odd = vec![json!({"descriptor": "pl4", "min": 1, "max": 9})];
        assert_eq!(
            tdp_error(&odd, &texts(&["5"])),
            Some("settings.tdpDescriptorError")
        );
    }

    #[test]
    fn parses_watts_with_a_decimal_comma() {
        assert_eq!(parse_watts("12,5"), Some(12.5));
        assert_eq!(parse_watts(""), None);
    }
}
