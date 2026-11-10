// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tableau de bord : profil actif, bascule rapide, capteurs CPU, ventilateurs,
//! iGPU, dGPU et batterie.
//!
//! Aucune écriture matérielle ici. La collecte des capteurs n'est demandée au
//! daemon que tant que cette page est affichée. Une métrique absente affiche
//! un tiret : jamais de zéro inventé.

use std::rc::Rc;

use gpui_kit::{Context, FontWeight, Window, div, prelude::*, rems};
use gpui_omarchy::*;
use serde_json::Value;

use crate::app::App;
use crate::i18n::{t, tf};
use crate::model::{self, DASH};
use crate::widgets::{self, Gauge};
use crate::worker::Command;

impl App {
    pub(crate) fn dashboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let hw = &snap.hardware;
        let sensors = &hw["sensors"];
        let fahrenheit = self.fahrenheit();
        let active = self.active_id();

        let live = if snap.connected {
            badge(t("app.live"), Status::Success, cx)
        } else {
            badge(t("state.disconnected"), Status::Warning, cx)
        };
        let edit_id = active.clone();
        let actions = div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .child(live)
            .child(
                button(
                    "dash-edit",
                    t("dash.editProfile"),
                    ButtonVariant::Outline,
                    cx,
                )
                .disabled(active.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_editor(Some(&edit_id), cx);
                })),
            );
        let header = widgets::page_header(
            &model::text(&hw["device"]["productName"]),
            t("dash.title"),
            t("dash.subtitle"),
            actions,
            cx,
        );

        let mut page = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(header)
            .child(self.active_profile_card(window, cx))
            .child(self.cpu_card(sensors, fahrenheit, cx))
            .child(self.fans_card(fahrenheit, cx));
        if sensors["igpu"].is_object() || hw["capabilities"]["hasIgpu"] == true {
            page = page.child(self.igpu_card(&sensors["igpu"], fahrenheit, cx));
        }
        if sensors["dgpu"].is_object() || hw["capabilities"]["hasDedicatedGpu"] == true {
            page = page.child(self.dgpu_card(&sensors["dgpu"], fahrenheit, cx));
        }
        page = page.child(self.battery_card(cx));
        page.children(self.agent_notice(cx))
    }

    fn active_profile_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let theme = cx.omarchy().clone();
        let state = &snap.state;
        let active = self.active_id();
        let profile = snap.profiles.iter().find(|p| p.id.as_str() == active);
        let name = profile.map_or_else(
            || model::text(&state["activeProfile"]["name"]),
            |p| p.name.clone(),
        );
        let description = profile.map(|p| p.description.clone()).unwrap_or_default();
        let ppd = model::text(&state["ppdProfile"]);
        let power = model::text(&state["powerSource"]);
        let manual = state["manualOverride"] == true;

        let mut badges = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.375))
            .child(badge(
                format!("{} · {}", t("dash.ppd"), model::ppd_label(&ppd)),
                Status::Neutral,
                cx,
            ))
            .child(badge(
                model::power_label(&power),
                if power == "ac" {
                    Status::Success
                } else {
                    Status::Neutral
                },
                cx,
            ));
        if manual {
            badges = badges.child(badge(t("profiles.manualOverride"), Status::Warning, cx));
        }
        let prime = &snap.hardware["prime"];
        if prime["available"] == true {
            badges = badges.child(badge(
                model::prime_label(&model::text(&prime["mode"])),
                Status::Neutral,
                cx,
            ));
        }
        if let Some(mode) = model::current_mode(&self.display_modes()) {
            badges = badges.child(badge(model::mode_label(mode), Status::Neutral, cx));
        }

        let items: Vec<(String, String)> = snap
            .profiles
            .iter()
            .map(|p| (p.id.as_str().to_owned(), p.name.clone()))
            .collect();
        let switcher = self.select(
            "dash/profile",
            items,
            Some(&active),
            !snap.connected || snap.profiles.is_empty(),
            16.,
            |this, id, _| {
                let name = this
                    .snap
                    .profiles
                    .iter()
                    .find(|p| p.id.as_str() == id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                this.handle.send(Command::UseProfile {
                    id: id.to_owned(),
                    name,
                });
            },
            window,
            cx,
        );
        let auto = button("dash-auto", t("tray.auto"), ButtonVariant::Outline, cx)
            .disabled(!manual)
            .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::ClearTemp)));

        let mut card = widgets::card(t("dash.activeProfile"), "", None, cx)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(rems(0.75))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(0.125))
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(rems(1.375))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.bright)
                                    .child(if name.is_empty() {
                                        t("dash.noData")
                                    } else {
                                        name
                                    }),
                            )
                            .when(!description.is_empty(), |column| {
                                column.child(widgets::muted(description, cx))
                            }),
                    )
                    .child(badges),
            )
            .child(separator(cx))
            .child(widgets::row(
                t("gpui.quickSwitch"),
                t("settings.switchingManualHint"),
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(switcher)
                    .child(auto),
                cx,
            ));

        // Profil power-profiles-daemon : PPD reste propriétaire, on passe par
        // son API publique, puis `tccd` suit sa matrice d'états.
        let ppd_profiles: Vec<String> = ["power-saver", "balanced", "performance"]
            .into_iter()
            .filter(|name| snap.ppd_profiles.iter().any(|p| p == name))
            .map(str::to_owned)
            .collect();
        if !ppd_profiles.is_empty() {
            let labels = ppd_profiles.iter().map(|p| model::ppd_label(p)).collect();
            let selected = ppd_profiles.iter().position(|p| *p == ppd).unwrap_or(0);
            let control = self.segmented(
                "dash-ppd",
                labels,
                selected,
                move |this, index, _| {
                    if let Some(name) = ppd_profiles.get(index) {
                        this.handle.send(Command::Ppd(name.clone()));
                    }
                },
                window,
                cx,
            );
            card = card.child(widgets::row(
                t("dash.ppd"),
                t("settings.switchingHint"),
                div().w(rems(24.)).child(control),
                cx,
            ));
        }
        card
    }

    fn cpu_card(
        &mut self,
        sensors: &Value,
        fahrenheit: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let cpu = &sensors["cpu"];
        let caps = &self.snap.hardware["cpu"];
        let temp = cpu["temperatureC"].as_f64();
        let freq = cpu["frequencyMhz"].as_f64().map(|mhz| mhz / 1000.);
        let power = cpu["powerW"].as_f64();
        let usage = self.snap.cpu_usage;
        let power_max = self.history.power_max.max(15.);
        let hint = match (caps["onlineCores"].as_u64(), caps["totalCores"].as_u64()) {
            (Some(online), Some(total)) => {
                format!("{online} / {total} {}", t("dash.cores").to_lowercase())
            }
            _ => String::new(),
        };
        let gauges = widgets::gauge_row()
            .child(widgets::gauge(
                Gauge {
                    value: usage,
                    max: 100.,
                    text: usage.map(|v| model::fmt_num(v, 0)).unwrap_or_default(),
                    unit: "%".to_owned(),
                    label: t("dash.cpuLoad"),
                    warn: 0.8,
                },
                cx,
            ))
            .child(widgets::gauge(
                Gauge {
                    value: temp,
                    max: 100.,
                    text: temp
                        .map(|v| model::fmt_num(model::temp_value(v, fahrenheit), 0))
                        .unwrap_or_default(),
                    unit: model::temp_unit(fahrenheit).to_owned(),
                    label: t("dash.temp"),
                    warn: 0.8,
                },
                cx,
            ))
            .child(widgets::gauge(
                Gauge {
                    value: freq,
                    max: 5.5,
                    text: freq.map(|v| model::fmt_num(v, 2)).unwrap_or_default(),
                    unit: "GHz".to_owned(),
                    label: t("dash.freq"),
                    warn: 2.,
                },
                cx,
            ))
            .child(widgets::gauge(
                Gauge {
                    value: power,
                    max: power_max,
                    text: power.map(|v| model::fmt_num(v, 1)).unwrap_or_default(),
                    unit: format!("W / {power_max:.0}"),
                    label: t("dash.powerDraw"),
                    warn: 0.8,
                },
                cx,
            ));
        let mut card = widgets::card(t("dash.cpu"), hint, None, cx).child(gauges);
        if sensors["available"] == false {
            card = card.child(widgets::muted(t("dash.missingSensor"), cx));
        }
        card.child(widgets::history_chart(
            t("dash.temp"),
            model::fmt_temp(temp, fahrenheit),
            self.history.cpu_temp.values(),
            theme.accent,
            5.25,
            cx,
        ))
    }

    fn fans_card(&mut self, fahrenheit: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let fans = &self.snap.fans;
        let backend = model::text(&fans["backend"]);
        let trailing =
            (!backend.is_empty()).then(|| badge(backend, Status::Neutral, cx).into_any_element());
        let mut card = widgets::card(t("editor.fan"), t("fans.readOnlyHint"), trailing, cx);
        let list = fans["fans"].as_array().filter(|list| !list.is_empty());
        let Some(list) = list else {
            return card.child(widgets::muted(t("dash.missingSensor"), cx));
        };
        let mut gauges = widgets::gauge_row();
        for (index, fan) in list.iter().enumerate() {
            let speed = fan["speed"].as_f64().filter(|v| *v >= 0.);
            let target = model::text(&fan["target"]);
            let label = if target.eq_ignore_ascii_case("gpu") {
                t("dash.fanGpu")
            } else {
                tf("gpui.fanNumber", &[&(index + 1).to_string()])
            };
            let applied = fan["applied"]
                .as_f64()
                .filter(|v| *v >= 0.)
                .map_or_else(|| DASH.to_owned(), |v| format!("{v:.0} %"));
            gauges = gauges.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(rems(0.125))
                    .child(widgets::gauge(
                        Gauge {
                            value: speed,
                            max: 100.,
                            text: speed.map(|v| model::fmt_num(v, 0)).unwrap_or_default(),
                            unit: "%".to_owned(),
                            label,
                            warn: 0.85,
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme.secondary)
                            .child(format!(
                                "{} · {} {applied}",
                                model::fmt_temp(fan["temp"].as_f64(), fahrenheit),
                                t("fans.consigne")
                            )),
                    ),
            );
        }
        card = card.child(gauges);
        let mode = model::text(&fans["mode"]);
        let mode_key = format!("fans.mode.{mode}");
        let mode_label = if crate::i18n::has(&mode_key) {
            t(&mode_key)
        } else {
            t("fans.mode.unknown")
        };
        card.child(widgets::info_row(t("fans.mode"), mode_label, None, cx))
    }

    fn igpu_card(
        &mut self,
        igpu: &Value,
        fahrenheit: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let present = igpu.is_object();
        let temp = igpu["temperatureC"].as_f64();
        let freq = igpu["frequencyMhz"].as_f64().map(|mhz| mhz / 1000.);
        let freq_max = igpu["maxFrequencyMhz"]
            .as_f64()
            .map_or(2.5, |mhz| (mhz / 1000.).max(0.5));
        let power = igpu["powerW"].as_f64();
        let vendor = model::text(&igpu["vendor"]).to_uppercase();
        let trailing = badge(
            if !present {
                t("common.unavailable")
            } else if vendor.is_empty() {
                t("common.supported")
            } else {
                vendor
            },
            if present {
                Status::Success
            } else {
                Status::Neutral
            },
            cx,
        );
        let gauges = widgets::gauge_row()
            .child(temp_gauge(temp, fahrenheit, cx))
            .child(widgets::gauge(
                Gauge {
                    value: freq,
                    max: freq_max,
                    text: freq.map(|v| model::fmt_num(v, 2)).unwrap_or_default(),
                    unit: "GHz".to_owned(),
                    label: t("dash.freq"),
                    warn: 2.,
                },
                cx,
            ))
            .child(widgets::gauge(
                Gauge {
                    value: power,
                    max: 40.,
                    text: power.map(|v| model::fmt_num(v, 1)).unwrap_or_default(),
                    unit: "W".to_owned(),
                    label: t("dash.powerDraw"),
                    warn: 0.8,
                },
                cx,
            ));
        let charts = div()
            .flex()
            .flex_wrap()
            .gap(rems(1.25))
            .child(widgets::history_chart(
                t("dash.temp"),
                model::fmt_temp(temp, fahrenheit),
                self.history.igpu_temp.values(),
                theme.accent,
                4.,
                cx,
            ))
            .child(widgets::history_chart(
                t("dash.freq"),
                freq.map_or_else(|| DASH.to_owned(), |v| format!("{v:.2} GHz")),
                self.history.igpu_freq.values(),
                theme.chart[1],
                4.,
                cx,
            ))
            .child(widgets::history_chart(
                t("dash.powerDraw"),
                power.map_or_else(|| DASH.to_owned(), |v| format!("{v:.1} W")),
                self.history.igpu_power.values(),
                theme.chart[2],
                4.,
                cx,
            ));
        widgets::card(
            t("dash.igpu"),
            model::text(&igpu["name"]),
            Some(trailing.into_any_element()),
            cx,
        )
        .child(gauges)
        .child(charts)
    }

    fn dgpu_card(
        &mut self,
        dgpu: &Value,
        fahrenheit: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let present = dgpu.is_object();
        let temp = dgpu["temperatureC"].as_f64();
        let freq = dgpu["frequencyMhz"].as_f64().map(|mhz| mhz / 1000.);
        let freq_max = dgpu["maxFrequencyMhz"]
            .as_f64()
            .map_or(2.5, |mhz| (mhz / 1000.).max(0.5));
        let power = dgpu["powerW"].as_f64();
        let power_max = self.snap.hardware["nvidia"]["maxPowerLimit"]
            .as_f64()
            .filter(|w| *w > 0.)
            .unwrap_or(90.);
        let vendor = model::text(&dgpu["vendor"]).to_uppercase();
        let trailing = badge(
            if !present {
                t("common.unavailable")
            } else if vendor.is_empty() {
                t("common.supported")
            } else {
                vendor
            },
            if present {
                Status::Success
            } else {
                Status::Neutral
            },
            cx,
        );
        let gauges = widgets::gauge_row()
            .child(temp_gauge(temp, fahrenheit, cx))
            .child(widgets::gauge(
                Gauge {
                    value: freq,
                    max: freq_max,
                    text: freq.map(|v| model::fmt_num(v, 2)).unwrap_or_default(),
                    unit: "GHz".to_owned(),
                    label: t("dash.freq"),
                    warn: 2.,
                },
                cx,
            ))
            .child(widgets::gauge(
                Gauge {
                    value: power,
                    max: power_max,
                    text: power.map(|v| model::fmt_num(v, 0)).unwrap_or_default(),
                    unit: "W".to_owned(),
                    label: t("dash.powerDraw"),
                    warn: 0.85,
                },
                cx,
            ));
        widgets::card(
            t("dash.dgpu"),
            model::text(&dgpu["name"]),
            Some(trailing.into_any_element()),
            cx,
        )
        .child(gauges)
        .child(widgets::history_chart(
            t("dash.temp"),
            model::fmt_temp(temp, fahrenheit),
            self.history.dgpu_temp.values(),
            theme.chart[1],
            5.25,
            cx,
        ))
    }

    fn battery_card(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let battery = &self.snap.battery;
        let card = widgets::card(t("gpui.battery"), "", None, cx);
        if battery["present"] != true {
            return card.child(widgets::muted(t("gpui.noBattery"), cx));
        }
        let percent = battery["percent"].as_f64();
        let status = model::text(&battery["status"]);
        card.child(progress("battery", percent.unwrap_or(0.) as f32, cx))
            .child(widgets::info_row(
                t("gpui.batteryCharge"),
                format!(
                    "{} · {status}",
                    percent.map_or_else(|| DASH.to_owned(), |p| format!("{p:.0} %")),
                ),
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("dash.powerDraw"),
                battery["powerW"]
                    .as_f64()
                    .map_or_else(|| DASH.to_owned(), |w| format!("{w:.1} W")),
                None,
                cx,
            ))
    }
}

fn temp_gauge(celsius: Option<f64>, fahrenheit: bool, cx: &gpui_kit::App) -> gpui_kit::Div {
    widgets::gauge(
        Gauge {
            value: celsius,
            max: 100.,
            text: celsius
                .map(|v| model::fmt_num(model::temp_value(v, fahrenheit), 0))
                .unwrap_or_default(),
            unit: model::temp_unit(fahrenheit).to_owned(),
            label: t("dash.temp"),
            warn: 0.85,
        },
        cx,
    )
}
