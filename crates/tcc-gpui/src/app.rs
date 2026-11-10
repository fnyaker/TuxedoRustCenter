// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Vue racine : navigation, tableau de bord, profils, réglages, matériel.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::input::InputState;
use gpui_kit::{
    App as GpuiApp, ClickEvent, Context, Entity, FontWeight, Window, div, prelude::*, rems,
};
use gpui_omarchy::*;
use serde_json::Value;
use tcc_core::ProfileId;

use crate::flatten::{self, Leaf};
use crate::i18n::{t, tf};
use crate::worker::{Command, Handle, Snapshot};

/// Pages de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Profiles,
    Settings,
    Hardware,
}

impl Page {
    /// `--page dashboard|profiles|settings|hardware`.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "dashboard" => Some(Self::Dashboard),
            "profiles" => Some(Self::Profiles),
            "settings" => Some(Self::Settings),
            "hardware" => Some(Self::Hardware),
            _ => None,
        }
    }

    const ALL: [Self; 4] = [
        Self::Dashboard,
        Self::Profiles,
        Self::Settings,
        Self::Hardware,
    ];

    fn label(self) -> String {
        t(match self {
            Self::Dashboard => "nav.dashboard",
            Self::Profiles => "nav.profiles",
            Self::Settings => "nav.settings",
            Self::Hardware => "nav.hardware",
        })
    }
}

/// Application gpui.
pub struct App {
    handle: Handle,
    snap: Snapshot,
    page: Page,
    selected: Option<String>,
    inputs: HashMap<String, Entity<InputState>>,
    /// Puissance maximale de la jauge CPU (watts), relevée automatiquement.
    power_max: f64,
}

/// Adapte un écouteur `(valeur, fenêtre, app)` au callback `on_change`.
fn change<T>(
    listener: impl Fn(&T, &mut Window, &mut GpuiApp) + 'static,
) -> impl Fn(T, &ClickEvent, &mut Window, &mut GpuiApp) {
    move |value, _, window, cx| listener(&value, window, cx)
}

/// Identifiant d'élément unique par (portée, rôle, index).
fn eid(scope: &str, role: &str, index: usize) -> gpui_kit::SharedString {
    format!("{scope}/{role}/{index}").into()
}

fn fmt_opt(value: Option<f64>, unit: &str) -> String {
    value.map_or_else(|| "—".to_owned(), |v| format!("{v:.1} {unit}"))
}

fn str_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("id")
            .or_else(|| map.get("name"))
            .map(str_of)
            .unwrap_or_default(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl App {
    pub fn new(handle: Handle, page: Page, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Relevé périodique de l'état publié par le thread de fond.
        cx.spawn(async move |this, cx| {
            let mut seen = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    let snap = this.handle.snapshot();
                    if snap.revision != seen || snap.notice != this.snap.notice {
                        seen = snap.revision;
                        this.snap = snap;
                        this.track_power_max();
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            snap: handle.snapshot(),
            handle,
            page,
            selected: None,
            inputs: HashMap::new(),
            power_max: 15.,
        }
    }

    fn input_for(
        &mut self,
        key: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.inputs.get(key) {
            return state.clone();
        }
        let initial = initial.to_owned();
        let state = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        self.inputs.insert(key.to_owned(), state.clone());
        state
    }

    fn active_profile_id(&self) -> String {
        self.snap.state["activeProfile"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

fn row_with(
    label: impl Into<gpui_kit::SharedString>,
    value: impl IntoElement,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(1.))
        .child(div().text_color(cx.omarchy().secondary).child(label.into()))
        .child(value)
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let mut nav = div().flex().gap(rems(0.5));
        for (index, page) in Page::ALL.into_iter().enumerate() {
            let label = page.label();
            let variant = if page == self.page {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            nav = nav.child(
                button(("nav", index), label, variant, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    },
                )),
            );
        }
        let status = if self.snap.connected {
            badge(t("status.connected"), Status::Success, cx)
        } else {
            badge(t("status.offline"), Status::Error, cx)
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(1.))
            .px(rems(1.25))
            .py(rems(0.75))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(1.))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.accent)
                            .child(t("app.title")),
                    )
                    .child(nav),
            )
            .child(status);

        let body = match self.page {
            Page::Dashboard => self.dashboard(cx).into_any_element(),
            Page::Profiles => self.profiles(window, cx).into_any_element(),
            Page::Settings => self.settings(window, cx).into_any_element(),
            Page::Hardware => self.hardware(window, cx).into_any_element(),
        };

        let mut footer = div();
        if let Some(error) = self.snap.error.as_ref().filter(|_| !self.snap.connected) {
            footer = footer.child(alert(error.clone(), Status::Error, cx));
        } else if let Some((message, ok)) = self.snap.notice.clone() {
            let status = if ok { Status::Success } else { Status::Error };
            footer = footer.child(alert(message, status, cx));
        }

        focus_scope("tcc")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(rems(0.8125))
            .child(header)
            .child(
                div()
                    .id("content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(rems(1.25))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(1.))
                            .max_w(rems(60.))
                            .child(body),
                    ),
            )
            .child(footer)
    }
}

// ---------------------------------------------------------------------------
// Tableau de bord
// ---------------------------------------------------------------------------

/// Jauge en arc (270°, ouverte vers le bas) : `fraction` de 0 à 1, valeur au
/// centre avec sa légende, détail (`footer`) sous la jauge. La couleur passe à l'avertissement puis au danger.
fn gauge(
    fraction: Option<f64>,
    value: String,
    caption: String,
    footer: String,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    const SIZE: f32 = 132.;
    const STROKE: f32 = 10.;
    let theme = cx.omarchy().clone();
    let fraction = fraction.map_or(0., |f| f.clamp(0., 1.));
    let color = if fraction > 0.9 {
        theme.danger
    } else if fraction > 0.75 {
        theme.warning
    } else {
        theme.accent
    };
    let track = theme.border;
    let arc = move |bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
                    sweep: f64,
                    color: gpui_kit::Hsla,
                    window: &mut Window| {
        if sweep <= 0. {
            return;
        }
        let center = bounds.center();
        let radius =
            f64::from(bounds.size.width.min(bounds.size.height)) / 2. - f64::from(STROKE) / 2. - 1.;
        let steps = ((sweep / 270.) * 72.).ceil().max(2.) as usize;
        let mut builder = gpui_kit::PathBuilder::stroke(gpui_kit::px(STROKE));
        for step in 0..=steps {
            let angle = (135. + sweep * step as f64 / steps as f64).to_radians();
            let point = gpui_kit::point(
                center.x + gpui_kit::px((radius * angle.cos()) as f32),
                center.y + gpui_kit::px((radius * angle.sin()) as f32),
            );
            if step == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .relative()
                .size(gpui_kit::px(SIZE))
                .child(
                    gpui_kit::canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            arc(bounds, 270., track, window);
                            arc(bounds, 270. * fraction, color, window);
                        },
                    )
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(rems(1.25))
                                .font_weight(FontWeight::BOLD)
                                .child(value),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme.secondary)
                                .child(caption),
                        ),
                ),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme.secondary)
                .child(footer),
        )
}

/// Température affichée en °C ou °F selon le réglage.
fn fmt_temp(celsius: Option<f64>, fahrenheit: bool) -> String {
    match celsius {
        None => "—".to_owned(),
        Some(c) if fahrenheit => format!("{:.0} °F", c * 9. / 5. + 32.),
        Some(c) => format!("{c:.0} °C"),
    }
}

impl App {
    /// Puissance maximale de la jauge : au moins 15 W, au moins le TDP connu,
    /// puis relevée par paliers de 5 W dès qu'une mesure la dépasse.
    fn track_power_max(&mut self) {
        let tdp = self.snap.hardware["odm"]["tdp"]
            .as_array()
            .and_then(|values| values.iter().filter_map(Value::as_f64).reduce(f64::max))
            .unwrap_or(0.);
        let observed = self.snap.hardware["sensors"]["cpu"]["packagePowerW"]
            .as_f64()
            .unwrap_or(0.);
        let wanted = (observed / 5.).ceil() * 5.;
        self.power_max = self.power_max.max(15.).max(tdp).max(wanted);
    }

    fn dashboard(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let active = self.active_profile_id();
        let name = self.snap.state["activeProfile"]["name"]
            .as_str()
            .unwrap_or("—")
            .to_owned();
        let power = self.snap.state["powerSource"]
            .as_str()
            .unwrap_or("?")
            .to_owned();
        let ppd = self.snap.state["ppdProfile"]
            .as_str()
            .unwrap_or("—")
            .to_owned();
        let manual = self.snap.state["manualOverride"].as_bool().unwrap_or(false);
        let fahrenheit = self.snap.settings["fahrenheit"].as_bool().unwrap_or(false);

        let mut switcher = div().flex().flex_wrap().gap(rems(0.5));
        for (index, profile) in self.snap.profiles.iter().enumerate() {
            let id = profile["id"].as_str().unwrap_or_default().to_owned();
            let label = profile["name"].as_str().unwrap_or(&id).to_owned();
            let variant = if id == active {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            switcher = switcher.child(button(("switch", index), label, variant, cx).on_click(
                cx.listener(move |this, _, _, _| this.handle.send(Command::UseProfile(id.clone()))),
            ));
        }
        let auto = button("auto", t("dash.auto"), ButtonVariant::Secondary, cx)
            .disabled(!manual)
            .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::ClearTemp)));

        let cpu_sensors = &self.snap.hardware["sensors"]["cpu"];
        let usage = self.snap.cpu_usage;
        let cpu_power = cpu_sensors["packagePowerW"].as_f64();
        let cpu_temp = cpu_sensors["temperatureC"].as_f64();
        let battery = &self.snap.battery;
        let percent = battery["percent"].as_f64();

        let gauges = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(rems(1.5))
            .child(gauge(
                usage.map(|u| u / 100.),
                usage.map_or_else(|| "—".to_owned(), |u| format!("{u:.0} %")),
                t("dash.usage"),
                String::new(),
                cx,
            ))
            .child(gauge(
                cpu_power.map(|w| w / self.power_max),
                cpu_power.map_or_else(|| "—".to_owned(), |w| format!("{w:.1} W")),
                t("dash.power"),
                format!("max {:.0} W", self.power_max),
                cx,
            ))
            .child(gauge(
                cpu_temp.map(|c| c / 100.),
                fmt_temp(cpu_temp, fahrenheit),
                t("dash.temperature"),
                format!("max {}", fmt_temp(Some(100.), fahrenheit)),
                cx,
            ));

        let mut battery_panel = panel(t("dash.batteryCard"), cx);
        if battery["present"] == true {
            battery_panel = battery_panel
                .child(progress("battery", percent.unwrap_or(0.) as f32, cx))
                .child(row_with(
                    t("dash.charge"),
                    format!(
                        "{} % · {}",
                        percent.map_or_else(|| "—".to_owned(), |p| format!("{p:.0}")),
                        battery["status"].as_str().unwrap_or("?")
                    ),
                    cx,
                ))
                .child(row_with(
                    t("dash.power"),
                    fmt_opt(battery["powerW"].as_f64(), "W"),
                    cx,
                ));
        } else {
            battery_panel = battery_panel.child(t("dash.noBattery"));
        }

        let mut fans = panel(t("dash.fans"), cx);
        match self.snap.fans["fans"].as_array().filter(|f| !f.is_empty()) {
            Some(list) => {
                let mut row = div().flex().flex_wrap().justify_center().gap(rems(1.5));
                for fan in list {
                    let speed = fan["speed"].as_f64();
                    let label = fan["label"].as_str().unwrap_or("fan").to_owned();
                    let temp = fmt_temp(fan["temp"].as_f64(), fahrenheit);
                    row = row.child(gauge(
                        speed.map(|v| v / 100.),
                        speed.map_or_else(|| "—".to_owned(), |v| format!("{v:.0} %")),
                        t("dash.fanSpeed"),
                        format!("{label} · {temp}"),
                        cx,
                    ));
                }
                fans = fans.child(row);
            }
            None => fans = fans.child(t("dash.noFans")),
        }

        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(
                panel(t("dash.activeProfile"), cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.75))
                            .child(
                                div()
                                    .text_size(rems(1.25))
                                    .font_weight(FontWeight::BOLD)
                                    .child(name),
                            )
                            .child(badge(
                                if power == "ac" {
                                    t("dash.ac")
                                } else {
                                    t("dash.battery")
                                },
                                Status::Neutral,
                                cx,
                            ))
                            .child(badge(
                                tf("dash.ppd", &[("value", &ppd)]),
                                Status::Neutral,
                                cx,
                            ))
                            .when(manual, |row| {
                                row.child(badge(t("dash.manual"), Status::Warning, cx))
                            }),
                    )
                    .child(switcher)
                    .child(auto),
            )
            .child(panel(t("dash.cpu"), cx).child(gauges))
            .child(fans)
            .child(battery_panel)
    }
}

// ---------------------------------------------------------------------------
// Éditeur générique de champs
// ---------------------------------------------------------------------------

type MakeCommand = Rc<dyn Fn(String, String) -> Command>;

impl App {
    /// Une ligne par feuille : interrupteur pour les booléens, champ + bouton
    /// « Appliquer » pour le reste.
    fn editor(
        &mut self,
        scope: &str,
        leaves: Vec<Leaf>,
        editable: bool,
        make: MakeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let mut out = div().flex().flex_col().gap(rems(0.5));
        let mut current = String::new();
        for (index, leaf) in leaves.into_iter().enumerate() {
            let section = flatten::section(&leaf.path).to_owned();
            if section != current {
                out = out.child(
                    div()
                        .mt(rems(0.75))
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.omarchy().accent)
                        .child(section.clone()),
                );
                current = section;
            }
            let label = leaf.path.clone();
            let control = if let Value::Bool(checked) = leaf.value {
                let path = leaf.path.clone();
                let make = Rc::clone(&make);
                let handle = self.handle.clone();
                switch(eid(scope, "sw", index), "", checked, cx)
                    .disabled(!editable)
                    .on_change(change(move |value: &bool, _, _| {
                        handle.send(make(path.clone(), value.to_string()));
                    }))
                    .into_any_element()
            } else {
                let key = format!("{scope}:{}", leaf.path);
                let state = self.input_for(&key, &flatten::display(&leaf.value), window, cx);
                let field = input(eid(scope, "in", index), &state, window, cx);
                let path = leaf.path.clone();
                let make = Rc::clone(&make);
                let handle = self.handle.clone();
                let target = state.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(div().w(rems(16.)).child(field))
                    .child(
                        button(
                            eid(scope, "apply", index),
                            t("common.apply"),
                            ButtonVariant::Outline,
                            cx,
                        )
                        .disabled(!editable)
                        .on_click(move |_, _, cx| {
                            let raw = target.read(cx).value().to_string();
                            handle.send(make(path.clone(), raw));
                        }),
                    )
                    .into_any_element()
            };
            out = out.child(row_with(label, control, cx));
        }
        out
    }

    fn profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let active = self.active_profile_id();
        let selected = self
            .selected
            .clone()
            .filter(|id| self.snap.profiles.iter().any(|p| p["id"] == id.as_str()))
            .or_else(|| (!active.is_empty()).then(|| active.clone()))
            .or_else(|| {
                self.snap
                    .profiles
                    .first()
                    .and_then(|p| p["id"].as_str().map(str::to_owned))
            });

        let mut list = div().flex().flex_col().gap(rems(0.375)).w(rems(14.));
        for (index, profile) in self.snap.profiles.iter().enumerate() {
            let id = profile["id"].as_str().unwrap_or_default().to_owned();
            let label = profile["name"].as_str().unwrap_or(&id).to_owned();
            let variant = if Some(&id) == selected.as_ref() {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            list = list.child(
                button(("plist", index), label, variant, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    },
                )),
            );
        }
        let new_name = self.input_for("new-profile", "", window, cx);
        let create_target = new_name.clone();
        list = list
            .child(separator(cx))
            .child(input("new-profile-input", &new_name, window, cx))
            .child(
                button(
                    "create-profile",
                    t("common.create"),
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let name = create_target.read(cx).value().trim().to_owned();
                    if !name.is_empty() {
                        this.handle.send(Command::CreateProfile(name));
                    }
                })),
            );

        let mut detail = div().flex().flex_col().gap(rems(0.75)).flex_1().min_w_0();
        match selected
            .as_deref()
            .and_then(|id| self.snap.profiles.iter().find(|p| p["id"] == id).cloned())
        {
            None => {
                detail = detail.child(empty_state(t("profiles.none"), t("profiles.noneHint"), cx));
            }
            Some(profile) => {
                let id = profile["id"].as_str().unwrap_or_default().to_owned();
                let preset = ProfileId::new(id.clone()).is_builtin_preset();
                let name = profile["name"].as_str().unwrap_or(&id).to_owned();

                let use_id = id.clone();
                let copy_id = id.clone();
                let del_id = id.clone();
                let copy_name = tf("common.copySuffix", &[("name", &name)]);
                let mut actions = div()
                    .flex()
                    .gap(rems(0.5))
                    .child(
                        button(
                            "profile-use",
                            t("common.activate"),
                            ButtonVariant::Primary,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.handle.send(Command::UseProfile(use_id.clone()));
                        })),
                    )
                    .child(
                        button(
                            "profile-copy",
                            t("common.copy"),
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.handle.send(Command::CopyProfile {
                                from: copy_id.clone(),
                                name: copy_name.clone(),
                            });
                        })),
                    );
                if !preset {
                    actions = actions.child(
                        button(
                            "profile-delete",
                            t("common.delete"),
                            ButtonVariant::Danger,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.handle.send(Command::DeleteProfile(del_id.clone()));
                            this.selected = None;
                            cx.notify();
                        })),
                    );
                }
                detail = detail
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(0.75))
                            .child(
                                div()
                                    .text_size(rems(1.25))
                                    .font_weight(FontWeight::BOLD)
                                    .child(name),
                            )
                            .when(preset, |row| {
                                row.child(badge(t("profiles.preset"), Status::Neutral, cx))
                            })
                            .when(id == active, |row| {
                                row.child(badge(t("profiles.active"), Status::Success, cx))
                            }),
                    )
                    .child(actions);
                if preset {
                    detail = detail.child(alert(t("profiles.readOnly"), Status::Neutral, cx));
                }
                let leaves = flatten::leaves(&profile, &["id"]);
                let profile_id = id.clone();
                let make: MakeCommand = Rc::new(move |key, raw| Command::SetProfileField {
                    profile: profile_id.clone(),
                    key,
                    raw,
                });
                let scope = format!("profile-{id}");
                let rows = self.editor(&scope, leaves, !preset, make, window, cx);
                detail = detail.child(panel(t("profiles.parameters"), cx).child(rows));
            }
        }
        div().flex().gap(rems(1.25)).child(list).child(detail)
    }

    fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let leaves = flatten::leaves(&self.snap.settings, &["deletedProfileIds", "schemaVersion"]);
        let make: MakeCommand = Rc::new(|key, raw| Command::SetSetting { key, raw });
        let rows = self.editor("settings", leaves, true, make, window, cx);
        div().flex().flex_col().gap(rems(1.)).child(
            panel(t("settings.title"), cx)
                .child(
                    div()
                        .text_color(cx.omarchy().secondary)
                        .child(t("settings.hint")),
                )
                .child(rows),
        )
    }
}

// ---------------------------------------------------------------------------
// Matériel
// ---------------------------------------------------------------------------

impl App {
    fn choice_row(
        &mut self,
        id: &'static str,
        title: &str,
        options: &Value,
        current: &Value,
        method: &'static str,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let items: Vec<String> = options
            .as_array()?
            .iter()
            .map(str_of)
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            return None;
        }
        let current = str_of(current);
        let mut buttons = div().flex().flex_wrap().gap(rems(0.5));
        for (index, item) in items.into_iter().enumerate() {
            let variant = if item == current {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            let value = item.clone();
            buttons = buttons.child(button((id, index), item, variant, cx).on_click(cx.listener(
                move |this, _, _, _| {
                    this.handle.send(Command::ManagerString {
                        method,
                        value: value.clone(),
                    });
                },
            )));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.375))
                .child(
                    div()
                        .text_color(cx.omarchy().secondary)
                        .child(title.to_owned()),
                )
                .child(buttons),
        )
    }

    fn hardware(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let hw = self.snap.hardware.clone();
        let charging = &hw["charging"];
        let mut charge = panel(t("hw.charging"), cx);
        if charging["available"] == true {
            if let Some(row) = self.choice_row(
                "charge-profile",
                &t("hw.chargeProfile"),
                &charging["profiles"],
                &charging["profile"],
                "SetChargingProfile",
                cx,
            ) {
                charge = charge.child(row);
            }
            if let Some(row) = self.choice_row(
                "charge-priority",
                &t("hw.chargePriority"),
                &charging["priorities"],
                &charging["priority"],
                "SetChargingPriority",
                cx,
            ) {
                charge = charge.child(row);
            }
            if charging["thresholdsAvailable"] == true {
                let start = self.input_for(
                    "charge-start",
                    &str_of(&charging["startThreshold"]),
                    window,
                    cx,
                );
                let end =
                    self.input_for("charge-end", &str_of(&charging["endThreshold"]), window, cx);
                let (s, e) = (start.clone(), end.clone());
                let current_end = charging["endThreshold"].as_i64().unwrap_or(100);
                charge = charge.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.5))
                        .child(t("hw.start"))
                        .child(div().w(rems(6.)).child(input(
                            "charge-start-in",
                            &start,
                            window,
                            cx,
                        )))
                        .child(t("hw.end"))
                        .child(
                            div()
                                .w(rems(6.))
                                .child(input("charge-end-in", &end, window, cx)),
                        )
                        .child(
                            button(
                                "charge-apply",
                                t("common.apply"),
                                ButtonVariant::Outline,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let parse = |state: &Entity<InputState>, cx: &GpuiApp| {
                                        state.read(cx).value().trim().parse::<i32>().ok()
                                    };
                                    let (Some(start), Some(end)) = (parse(&s, cx), parse(&e, cx))
                                    else {
                                        return;
                                    };
                                    // Garder début < fin à chaque étape.
                                    let start_op = ("SetChargeStartThreshold", start);
                                    let end_op = ("SetChargeEndThreshold", end);
                                    let ops = if i64::from(end) < current_end {
                                        [start_op, end_op]
                                    } else {
                                        [end_op, start_op]
                                    };
                                    for (method, value) in ops {
                                        this.handle.send(Command::ManagerInt { method, value });
                                    }
                                },
                            )),
                        ),
                );
            }
        } else {
            charge = charge.child(t("hw.chargeUnavailable"));
        }

        let mut misc = panel(t("hw.misc"), cx);
        let fn_lock = &hw["fnLock"];
        if fn_lock["supported"] == true {
            let handle = self.handle.clone();
            misc = misc.child(
                switch("fn-lock", t("hw.fnLock"), fn_lock["enabled"] == true, cx).on_change(
                    change(move |value: &bool, _, _| handle.send(Command::FnLock(*value))),
                ),
            );
        }
        let odm = &hw["odm"];
        if odm["tdpAvailable"] == true {
            let current = odm["tdp"]
                .as_array()
                .map(|v| v.iter().map(str_of).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let state = self.input_for("tdp", &current, window, cx);
            let target = state.clone();
            misc = misc.child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(t("hw.tdp"))
                    .child(
                        div()
                            .w(rems(12.))
                            .child(input("tdp-in", &state, window, cx)),
                    )
                    .child(
                        button("tdp-apply", t("common.apply"), ButtonVariant::Outline, cx)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let watts: Vec<i32> = target
                                    .read(cx)
                                    .value()
                                    .split_whitespace()
                                    .filter_map(|w| w.parse().ok())
                                    .collect();
                                this.handle.send(Command::SetTdp(watts));
                            })),
                    ),
            );
        }
        if fn_lock["supported"] != true && odm["tdpAvailable"] != true {
            misc = misc.child(t("hw.nothing"));
        }

        let mut gpus = panel(t("hw.sensors"), cx);
        match hw["sensors"]["gpus"].as_array().filter(|g| !g.is_empty()) {
            Some(list) => {
                for gpu in list {
                    gpus = gpus.child(row_with(
                        gpu["name"].as_str().unwrap_or("GPU").to_owned(),
                        format!(
                            "{} · {}",
                            fmt_opt(gpu["temperatureC"].as_f64(), "°C"),
                            fmt_opt(gpu["powerW"].as_f64(), "W")
                        ),
                        cx,
                    ));
                }
            }
            None => gpus = gpus.child(t("hw.noGpu")),
        }
        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(charge)
            .child(misc)
            .child(gpus)
    }
}
