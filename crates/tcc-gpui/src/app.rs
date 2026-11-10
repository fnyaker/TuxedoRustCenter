// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Vue racine : navigation, état partagé des pages, contrôles liés au
//! modèle, notifications et dialogues.
//!
//! Les pages (`pages/*`) sont des blocs `impl App`. Elles ne gardent que des
//! brouillons : la valeur affichée est le brouillon s'il existe, sinon la
//! dernière valeur lue du daemon. Rien n'est écrit sans action explicite.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, SharedString, Subscription,
    Window, div, prelude::*, relative, rems,
};
use gpui_omarchy::*;
use tcc_core::Settings;

use crate::i18n::{t, tf};
use crate::model::{self, History};
use crate::pages::editor::Editor;
use crate::pages::settings::ChargeDraft;
use crate::pages::tools::LightDraft;
use crate::prefs::Prefs;
use crate::widgets;
use crate::worker::{Command, Handle, Op, Outcome, SettingsTag, Snapshot};

/// Pages de l'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Profiles,
    /// Éditeur de profil (sous-page de `Profiles`).
    Editor,
    Tools,
    Settings,
    Info,
}

impl Page {
    /// Pages de la barre de navigation.
    const NAV: [Self; 5] = [
        Self::Dashboard,
        Self::Profiles,
        Self::Tools,
        Self::Settings,
        Self::Info,
    ];

    /// `--page dashboard|profiles|tools|settings|info`.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "dashboard" => Some(Self::Dashboard),
            "profiles" => Some(Self::Profiles),
            "tools" | "hardware" => Some(Self::Tools),
            "settings" => Some(Self::Settings),
            "info" | "about" => Some(Self::Info),
            _ => None,
        }
    }

    fn label(self) -> String {
        t(match self {
            Self::Dashboard => "nav.dashboard",
            Self::Profiles | Self::Editor => "nav.profiles",
            Self::Tools => "nav.tools",
            Self::Settings => "nav.settings",
            Self::Info => "nav.info",
        })
    }

    /// Onglet de navigation correspondant.
    fn section(self) -> Self {
        if self == Self::Editor {
            Self::Profiles
        } else {
            self
        }
    }
}

type TextChange = Rc<dyn Fn(&mut App, &str, &mut Context<App>)>;
pub(crate) type NumberChange = Rc<dyn Fn(&mut App, f32, &mut Context<App>)>;

struct SelectSlot {
    state: Entity<ChoiceState>,
    items: Vec<(String, String)>,
    disabled: bool,
    value: Option<String>,
    change: TextChange,
    _watch: Subscription,
}

struct SliderSlot {
    state: Entity<SliderState>,
    range: (f32, f32, f32),
    value: f32,
    change: NumberChange,
    release: Option<NumberChange>,
    _watch: Subscription,
}

struct InputSlot {
    state: Entity<InputState>,
    value: String,
    change: TextChange,
    _watch: Subscription,
}

struct Toast {
    id: usize,
    message: String,
    ok: bool,
    until: Instant,
}

/// Historiques locaux des capteurs (le contrat n'expose pas de série).
#[derive(Default)]
pub struct Histories {
    pub cpu_temp: History,
    pub igpu_temp: History,
    pub igpu_freq: History,
    pub igpu_power: History,
    pub dgpu_temp: History,
    /// Pleine échelle de la jauge de puissance CPU (watts) : au moins 15 W,
    /// au moins le TDP connu, relevée par paliers de 5 W.
    pub power_max: f64,
}

/// Application gpui.
pub struct App {
    pub(crate) handle: Handle,
    pub(crate) snap: Rc<Snapshot>,
    pub(crate) page: Page,
    pub(crate) prefs: Prefs,
    selects: HashMap<String, SelectSlot>,
    sliders: HashMap<String, SliderSlot>,
    inputs: HashMap<String, InputSlot>,
    toasts: Vec<Toast>,
    toast_seq: usize,
    dialog_focus: FocusHandle,
    /// Profil dont la suppression attend une confirmation : `(id, nom)`.
    pub(crate) confirm_delete: Option<(String, String)>,

    // Tableau de bord.
    pub(crate) history: Histories,
    last_sample: Option<Instant>,

    // Profils.
    pub(crate) filter: usize,
    pub(crate) search: String,
    pub(crate) editor: Option<Editor>,

    // Réglages : brouillons non appliqués.
    pub(crate) matrix: BTreeMap<String, Option<String>>,
    pub(crate) charge: ChargeDraft,
    pub(crate) odm: Option<String>,
    pub(crate) tdp: Option<Vec<String>>,
    pub(crate) ctgp: Option<String>,
    pub(crate) autostart: bool,

    // Outils : brouillons non appliqués.
    pub(crate) timer: Option<(bool, u32, u32)>,
    pub(crate) light: Option<LightDraft>,
    /// Valeurs UVC en cours de réglage : `(périphérique, contrôle)` → valeur.
    pub(crate) uvc: HashMap<(String, String), i32>,
    /// Une écriture UVC vient de se terminer : les valeurs en cours de
    /// réglage sont abandonnées à la prochaine lecture du matériel.
    uvc_settle: bool,
    /// Profil (identifiant ou nom) à ouvrir dans l'éditeur dès que la liste
    /// est chargée (`--edit`).
    edit: Option<String>,
}

fn single(value: SliderValue) -> f32 {
    match value {
        SliderValue::Single(value) | SliderValue::Range(_, value) => value,
    }
}

impl App {
    pub fn new(
        handle: Handle,
        page: Page,
        prefs: Prefs,
        edit: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Relevé périodique de l'état publié par le thread de fond.
        cx.spawn(async move |this, cx| {
            let mut seen = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let alive = this.update(cx, |this, cx| this.poll(&mut seen, cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        handle.set_sensors_wanted(page == Page::Dashboard);
        Self {
            snap: Rc::new(handle.snapshot()),
            handle,
            page,
            prefs,
            selects: HashMap::new(),
            sliders: HashMap::new(),
            inputs: HashMap::new(),
            toasts: Vec::new(),
            toast_seq: 0,
            dialog_focus: cx.focus_handle(),
            confirm_delete: None,
            history: Histories::default(),
            last_sample: None,
            filter: 0,
            search: String::new(),
            editor: None,
            matrix: BTreeMap::new(),
            charge: ChargeDraft::default(),
            odm: None,
            tdp: None,
            ctgp: None,
            autostart: crate::prefs::autostart_enabled(),
            timer: None,
            light: None,
            uvc: HashMap::new(),
            uvc_settle: false,
            edit,
        }
    }

    fn poll(&mut self, seen: &mut u64, cx: &mut Context<Self>) {
        let mut changed = false;
        let revision = self.handle.revision();
        if revision != *seen {
            *seen = revision;
            self.snap = Rc::new(self.handle.snapshot());
            if std::mem::take(&mut self.uvc_settle) {
                self.uvc.clear();
            }
            self.sample_sensors();
            self.sync_editor();
            self.open_requested(cx);
            changed = true;
        }
        for outcome in self.handle.take_outcomes() {
            self.finish(outcome);
            changed = true;
        }
        let now = Instant::now();
        let before = self.toasts.len();
        self.toasts.retain(|toast| toast.until > now);
        if changed || self.toasts.len() != before {
            cx.notify();
        }
    }

    /// Ajoute un point aux historiques, au rythme de publication des
    /// capteurs (2 s) et seulement quand la collecte est active.
    fn sample_sensors(&mut self) {
        let sensors = &self.snap.hardware["sensors"];
        let tdp = self.snap.hardware["odm"]["tdp"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|limit| limit["max"].as_f64())
            .fold(0., f64::max);
        let observed = sensors["cpu"]["powerW"].as_f64().unwrap_or(0.);
        self.history.power_max = self
            .history
            .power_max
            .max(15.)
            .max(tdp)
            .max((observed / 5.).ceil() * 5.);
        if self.page != Page::Dashboard || sensors["enabled"] != true {
            return;
        }
        let now = Instant::now();
        if self
            .last_sample
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(1900))
        {
            return;
        }
        self.last_sample = Some(now);
        self.history
            .cpu_temp
            .push(sensors["cpu"]["temperatureC"].as_f64());
        let igpu = &sensors["igpu"];
        if igpu.is_object() {
            self.history.igpu_temp.push(igpu["temperatureC"].as_f64());
            self.history
                .igpu_freq
                .push(igpu["frequencyMhz"].as_f64().map(|mhz| mhz / 1000.));
            self.history.igpu_power.push(igpu["powerW"].as_f64());
        } else {
            self.history.igpu_temp.clear();
            self.history.igpu_freq.clear();
            self.history.igpu_power.clear();
        }
        let dgpu = &sensors["dgpu"];
        if dgpu.is_object() {
            self.history.dgpu_temp.push(dgpu["temperatureC"].as_f64());
        } else {
            self.history.dgpu_temp.clear();
        }
    }

    /// Réagit au résultat d'une commande : message, puis abandon du brouillon
    /// correspondant en cas de succès (la valeur du daemon reprend la main).
    fn finish(&mut self, outcome: Outcome) {
        if outcome.ok {
            match &outcome.op {
                Op::Settings(SettingsTag::Matrix) => self.matrix.clear(),
                Op::Settings(SettingsTag::Charging) => self.charge = ChargeDraft::default(),
                Op::Settings(SettingsTag::Timer(_)) => self.timer = None,
                Op::Settings(SettingsTag::Lighting) => self.light = None,
                Op::Hardware => {
                    self.odm = None;
                    self.tdp = None;
                    self.ctgp = None;
                }
                Op::SaveProfile => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.saved();
                    }
                }
                Op::CreateProfile(id) => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.created(id);
                    }
                }
                _ => {}
            }
        }
        if outcome.op == Op::Uvc {
            self.uvc_settle = true;
        }
        if !outcome.message.is_empty() {
            self.notify(outcome.message, outcome.ok);
        }
    }

    /// Ouvre l'éditeur demandé en ligne de commande, une fois les profils lus.
    fn open_requested(&mut self, cx: &mut Context<Self>) {
        if self.snap.profiles.is_empty() {
            return;
        }
        let Some(query) = self.edit.take() else {
            return;
        };
        let found = self
            .snap
            .profiles
            .iter()
            .find(|p| p.id.as_str() == query || p.name.eq_ignore_ascii_case(&query))
            .map(|p| p.id.as_str().to_owned());
        match found {
            Some(id) => self.open_editor(Some(&id), cx),
            None => self.notify(t("admin.error.notFound"), false),
        }
    }

    /// Affiche une notification temporaire.
    pub(crate) fn notify(&mut self, message: impl Into<String>, ok: bool) {
        self.toast_seq += 1;
        let seconds = if ok { 4 } else { 10 };
        self.toasts.push(Toast {
            id: self.toast_seq,
            message: message.into(),
            ok,
            until: Instant::now() + Duration::from_secs(seconds),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Change de page.
    pub(crate) fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.handle.set_sensors_wanted(page == Page::Dashboard);
        cx.notify();
    }

    /// Réglages lus du daemon, s'ils sont disponibles.
    pub(crate) fn settings(&self) -> Option<&Settings> {
        self.snap.settings.as_ref()
    }

    /// Températures en °F.
    pub(crate) fn fahrenheit(&self) -> bool {
        self.settings().is_some_and(|settings| settings.fahrenheit)
    }

    /// Identifiant du profil actif.
    pub(crate) fn active_id(&self) -> String {
        model::text(&self.snap.state["activeProfile"]["id"])
    }

    /// Les réglages persistants sont modifiables.
    pub(crate) fn settings_editable(&self) -> bool {
        self.snap.connected && self.snap.admin && !self.snap.busy && self.snap.settings.is_some()
    }

    /// Modifie les réglages persistants : appliqué tout de suite à la copie
    /// locale (l'interface répond sans attendre), puis envoyé au service
    /// d'administration. En cas de refus, la relecture rétablit la valeur.
    pub(crate) fn patch_settings(
        &mut self,
        tag: SettingsTag,
        patch: impl Fn(&mut Settings) + Send + 'static,
    ) {
        if let Some(settings) = Rc::make_mut(&mut self.snap).settings.as_mut() {
            patch(settings);
        }
        self.handle.send(Command::Settings {
            tag,
            patch: Box::new(patch),
        });
    }

    // -----------------------------------------------------------------------
    // Contrôles liés au modèle
    // -----------------------------------------------------------------------

    /// Liste déroulante. `items` : `(valeur, libellé)` ; `current` : valeur du
    /// modèle ; `change` est appelé quand l'utilisateur choisit une valeur.
    /// L'état de la liste est conservé entre les rendus et resynchronisé si
    /// le modèle change par ailleurs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn select(
        &mut self,
        key: &str,
        items: Vec<(String, String)>,
        current: Option<&str>,
        disabled: bool,
        width: f32,
        change: impl Fn(&mut App, &str, &mut Context<App>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = current.map(str::to_owned);
        let index = current
            .as_ref()
            .and_then(|value| items.iter().position(|(item, _)| item == value));
        let change: TextChange = Rc::new(change);
        let fresh = self
            .selects
            .get(key)
            .is_none_or(|slot| slot.items != items || slot.disabled != disabled);
        if fresh {
            let choices = items
                .iter()
                .map(|(value, label)| ChoiceItem::new(value.clone(), label.clone()))
                .collect();
            let state = cx.new(|cx| {
                let state = ChoiceState::new(choices, window, cx)
                    .placeholder(model::DASH)
                    .disabled(disabled);
                match index {
                    Some(index) => state.default_selected(index),
                    None => state,
                }
            });
            let watched = key.to_owned();
            let watch = cx.observe(&state, move |this, state, cx| {
                let selected = state.read(cx).selected().map(|item| item.value.to_string());
                let Some(slot) = this.selects.get_mut(&watched) else {
                    return;
                };
                let Some(value) = selected.filter(|value| slot.value.as_ref() != Some(value))
                else {
                    return;
                };
                slot.value = Some(value.clone());
                let change = Rc::clone(&slot.change);
                change(this, &value, cx);
                cx.notify();
            });
            self.selects.insert(
                key.to_owned(),
                SelectSlot {
                    state,
                    items,
                    disabled,
                    value: index.and(current),
                    change,
                    _watch: watch,
                },
            );
        } else if let Some(slot) = self.selects.get_mut(key) {
            slot.change = change;
            let wanted = index.and(current);
            if slot.value != wanted {
                slot.value = wanted;
                slot.state
                    .update(cx, |state, cx| state.set_selected(index, cx));
            }
        }
        let Some(slot) = self.selects.get(key) else {
            return div().into_any_element();
        };
        let state = slot.state.clone();
        div()
            .w(rems(width))
            .child(select(
                ElementId::from(SharedString::from(key.to_owned())),
                &state,
                window,
                cx,
            ))
            .into_any_element()
    }

    /// Curseur. `range` : `(min, max, pas)`. `change` est appelé pendant le
    /// réglage, `release` (facultatif) au relâchement.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slider(
        &mut self,
        key: &str,
        range: (f32, f32, f32),
        value: f32,
        disabled: bool,
        change: impl Fn(&mut App, f32, &mut Context<App>) + 'static,
        release: Option<NumberChange>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (min, max, step) = range;
        let max = max.max(min + step);
        let value = value.clamp(min, max);
        let change: NumberChange = Rc::new(change);
        let fresh = self
            .sliders
            .get(key)
            .is_none_or(|slot| slot.range != (min, max, step));
        if fresh {
            let state = cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(step)
                    .default_value(value)
            });
            let watched = key.to_owned();
            let watch = cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
                let Some(slot) = this.sliders.get_mut(&watched) else {
                    return;
                };
                match event {
                    SliderEvent::Change(value) => {
                        let value = single(*value);
                        slot.value = value;
                        let change = Rc::clone(&slot.change);
                        change(this, value, cx);
                    }
                    SliderEvent::Release(value) => {
                        if let Some(release) = slot.release.clone() {
                            release(this, single(*value), cx);
                        }
                    }
                }
                cx.notify();
            });
            self.sliders.insert(
                key.to_owned(),
                SliderSlot {
                    state,
                    range: (min, max, step),
                    value,
                    change,
                    release,
                    _watch: watch,
                },
            );
        } else if let Some(slot) = self.sliders.get_mut(key) {
            slot.change = change;
            slot.release = release;
            if (slot.value - value).abs() > f32::EPSILON {
                slot.value = value;
                slot.state
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
        let Some(slot) = self.sliders.get(key) else {
            return div().into_any_element();
        };
        let state = slot.state.clone();
        div()
            .w(rems(12.))
            .child(slider(&state, disabled, window, cx))
            .into_any_element()
    }

    /// Champ de texte lié à `value`. `change` est appelé à chaque frappe.
    pub(crate) fn text_input(
        &mut self,
        key: &str,
        value: &str,
        placeholder: &str,
        change: impl Fn(&mut App, &str, &mut Context<App>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Input {
        let change: TextChange = Rc::new(change);
        if let Some(slot) = self.inputs.get_mut(key) {
            slot.change = change;
            if slot.value != value {
                slot.value = value.to_owned();
                let text = value.to_owned();
                slot.state
                    .update(cx, |state, cx| state.set_value(text, window, cx));
            }
        } else {
            let initial = value.to_owned();
            let placeholder = placeholder.to_owned();
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(initial)
            });
            let watched = key.to_owned();
            let watch = cx.subscribe(&state, move |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                let Some(slot) = this.inputs.get_mut(&watched) else {
                    return;
                };
                if slot.value == text {
                    return;
                }
                slot.value.clone_from(&text);
                let change = Rc::clone(&slot.change);
                change(this, &text, cx);
                cx.notify();
            });
            self.inputs.insert(
                key.to_owned(),
                InputSlot {
                    state,
                    value: value.to_owned(),
                    change,
                    _watch: watch,
                },
            );
        }
        let state = self
            .inputs
            .get(key)
            .map(|slot| slot.state.clone())
            .unwrap_or_else(|| cx.new(|cx| InputState::new(window, cx)));
        input(
            ElementId::from(SharedString::from(key.to_owned())),
            &state,
            window,
            cx,
        )
    }

    /// Interrupteur. `label` sert aux technologies d'assistance : le libellé
    /// visible est celui de la ligne.
    pub(crate) fn toggle(
        &self,
        id: &'static str,
        label: String,
        checked: bool,
        disabled: bool,
        change: impl Fn(&mut App, bool, &mut Context<App>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        switch(id, "", checked, cx)
            .accessibility_label(label)
            .disabled(disabled)
            .on_change(widgets::change(move |value: &bool, _, cx| {
                entity.update(cx, |this, cx| {
                    change(this, *value, cx);
                    cx.notify();
                });
            }))
            .into_any_element()
    }

    /// Choix exclusif en segments.
    pub(crate) fn segmented(
        &self,
        id: &'static str,
        labels: Vec<String>,
        selected: usize,
        change: impl Fn(&mut App, usize, &mut Context<App>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let items = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| ChoiceItem::new(index.to_string(), label))
            .collect();
        button_group(
            id,
            items,
            Some(selected),
            move |index, _, cx| {
                entity.update(cx, |this, cx| {
                    change(this, index, cx);
                    cx.notify();
                });
            },
            window,
            cx,
        )
        .into_any_element()
    }

    // -----------------------------------------------------------------------
    // Dialogues et notifications
    // -----------------------------------------------------------------------

    /// Demande confirmation avant de supprimer un profil.
    pub(crate) fn ask_delete(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_delete = Some((id, name));
        self.dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn delete_dialog(&self, name: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        use gpui_kit::base::actions::{Cancel, Confirm};
        let body = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(dialog_title(t("profiles.deleteTitle"), cx))
            .child(dialog_description(tf("profiles.deleteBody", &[name]), cx))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(rems(0.5))
                    .child(
                        dialog_button(
                            "delete-cancel",
                            t("common.cancel"),
                            ButtonVariant::Secondary,
                            cx,
                        )
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
                    )
                    .child(
                        dialog_button(
                            "delete-confirm",
                            t("common.delete"),
                            ButtonVariant::Danger,
                            cx,
                        )
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
                        }),
                    ),
            );
        let popup = div()
            .id("delete-surface")
            .occlude()
            .max_w(relative(0.9))
            .child(dialog_popup(cx).child(body));
        let close = cx.listener(|this, confirmed: &bool, _, cx| {
            if let Some((id, name)) = this.confirm_delete.take()
                && *confirmed
            {
                this.handle.send(Command::Delete { id, name });
            }
            cx.notify();
        });
        alert_dialog(&self.dialog_focus, cx)
            .popup(popup)
            .request_close(move |confirmed, window, cx| close(&confirmed, window, cx))
    }

    fn toast_stack(&self, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let theme = cx.omarchy().clone();
        div()
            .id("toasts")
            .absolute()
            .right(rems(1.))
            .bottom(rems(1.))
            .w(rems(22.))
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .occlude()
            .children(self.toasts.iter().map(|entry| {
                let id = entry.id;
                let color = if entry.ok {
                    theme.success
                } else {
                    theme.danger
                };
                toast(("toast", id), cx).border_color(color).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.625))
                        .child(
                            icon(if entry.ok {
                                IconName::Check
                            } else {
                                IconName::TriangleAlert
                            })
                            .size(rems(1.))
                            .text_color(color),
                        )
                        .child(div().flex_1().min_w_0().child(entry.message.clone()))
                        .child(
                            icon_button(
                                ("toast-close", id),
                                IconName::Close,
                                t("common.close"),
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.toasts.retain(|toast| toast.id != id);
                                    cx.notify();
                                },
                            )),
                        ),
                )
            }))
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let section = self.page.section();
        let entity = cx.entity();
        let nav = tab_list(
            "nav",
            Page::NAV
                .iter()
                .map(|page| ChoiceItem::new(page.label(), page.label()))
                .collect(),
            Page::NAV.iter().position(|page| *page == section),
            false,
            move |index, _, cx| {
                entity.update(cx, |this, cx| {
                    if let Some(page) = Page::NAV.get(index) {
                        this.navigate(*page, cx);
                    }
                });
            },
            window,
            cx,
        );
        let status = if self.snap.connected {
            badge(t("state.connected"), Status::Success, cx)
        } else {
            badge(t("state.disconnected"), Status::Error, cx)
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(1.))
            .px(rems(1.25))
            .py(rems(0.625))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(1.25))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.accent)
                            .child(t("app.name")),
                    )
                    .child(div().w(rems(30.)).child(nav)),
            )
            .child(status);

        let body = match self.page {
            Page::Dashboard => self.dashboard(window, cx),
            Page::Profiles => self.profiles(window, cx),
            Page::Editor => self.editor_page(window, cx),
            Page::Tools => self.tools(window, cx),
            Page::Settings => self.settings_page(window, cx),
            Page::Info => self.info(cx),
        };
        let offline = (!self.snap.connected).then(|| {
            widgets::notice(
                Status::Error,
                t("connection.title"),
                self.snap.error.clone().unwrap_or_default(),
                Some(
                    button("reconnect", t("common.retry"), ButtonVariant::Outline, cx)
                        .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::Refresh)))
                        .into_any_element(),
                ),
                cx,
            )
        });
        let save_bar = (self.page == Page::Editor).then(|| self.editor_bar(cx));
        let dialog = self
            .confirm_delete
            .clone()
            .map(|(_, name)| self.delete_dialog(&name, cx));

        focus_scope("tcc")
            .relative()
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
                    .id(("content", self.page as usize))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(rems(1.25))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(1.))
                            .w_full()
                            .max_w(rems(64.))
                            .mx_auto()
                            .children(offline)
                            .child(body),
                    ),
            )
            .children(save_bar)
            .when(!self.toasts.is_empty(), |root| {
                root.child(self.toast_stack(cx))
            })
            .children(dialog)
    }
}
