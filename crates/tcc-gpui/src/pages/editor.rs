// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Éditeur de profil : général, affichage, processeur, webcam, ventilateurs
//! (courbe éditable à la souris) et ODM / TDP / cTGP.
//!
//! L'éditeur travaille sur un brouillon typé (`tcc_core::Profile`). Rien n'est
//! écrit avant « Enregistrer » ; l'écriture passe par `Admin1.SaveProfilesJSON`
//! et le daemon applique le profil s'il est actif. Les presets et l'absence du
//! service d'administration mettent l'éditeur en lecture seule.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, Window, div, prelude::*, px, rems,
};
use gpui_omarchy::*;
use tcc_core::{FanCurve, FrequencyLimit, Profile, ProfileId};

use crate::app::{App, Page};
use crate::fan_curve::{self, PlotBounds};
use crate::i18n::{t, tf};
use crate::model::{self, FieldErrors, Mode};
use crate::pages::is_preset;
use crate::widgets;
use crate::worker::Command;

/// Bornes des curseurs de fréquence (GHz). Le daemon valide la plage réelle.
const FREQ_RANGE: (f32, f32, f32) = (0.4, 6., 0.1);

/// Brouillon de profil en cours d'édition.
pub struct Editor {
    /// Identifiant du profil édité ; `None` pour un nouveau profil.
    pub id: Option<String>,
    /// Profil tel que lu du daemon.
    base: Profile,
    /// Profil modifié.
    pub draft: Profile,
    /// Champs TDP (texte, un par limite exposée).
    tdp: Vec<String>,
    /// Champ cTGP (texte).
    ctgp: String,
    /// Le profil a changé côté daemon pendant l'édition.
    conflict: bool,
    /// Point sélectionné de la courbe.
    point: Option<usize>,
    /// Un point est en cours de déplacement.
    dragging: bool,
    plot: PlotBounds,
    focus: FocusHandle,
}

fn number_text(value: f64) -> String {
    if value.fract() == 0. {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn parse_number(text: &str) -> Option<f64> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

impl Editor {
    fn new(id: Option<String>, mut profile: Profile, focus: FocusHandle) -> Self {
        if profile.fan.custom_fan_curve.table_cpu.len() < 2 {
            let default = FanCurve::default_custom();
            profile.fan.custom_fan_curve.table_cpu = default.table_cpu;
        }
        let mut editor = Self {
            id,
            base: profile.clone(),
            draft: profile,
            tdp: Vec::new(),
            ctgp: String::new(),
            conflict: false,
            point: None,
            dragging: false,
            plot: Rc::new(Cell::new(Bounds::default())),
            focus,
        };
        editor.load_texts();
        editor
    }

    fn load_texts(&mut self) {
        self.tdp = self
            .draft
            .odm_power_limits
            .tdp_values
            .iter()
            .map(|value| number_text(*value))
            .collect();
        self.ctgp = number_text(self.draft.nvidia_power_ctrl_profile.ctgp_offset);
    }

    /// Le brouillon diffère du profil lu.
    pub fn dirty(&self) -> bool {
        self.id.is_none() || self.draft != self.base
    }

    fn reset(&mut self) {
        self.draft = self.base.clone();
        self.conflict = false;
        self.point = None;
        self.load_texts();
    }

    /// Le profil a été enregistré : le brouillon devient la référence.
    pub fn saved(&mut self) {
        self.base = self.draft.clone();
        self.conflict = false;
    }

    /// Le profil a été créé sous l'identifiant `id`.
    pub fn created(&mut self, id: &str) {
        self.id = Some(id.to_owned());
        self.draft.id = ProfileId::new(id);
        self.saved();
    }
}

/// Choix d'affichage dérivés des modes exposés par l'agent.
struct DisplayChoices {
    resolutions: Vec<(u32, u32)>,
    rates: Vec<f64>,
    /// Résolution retenue pour lister les taux.
    active: Option<(u32, u32)>,
    /// La résolution enregistrée n'est plus exposée.
    resolution_missing: bool,
    /// Le taux enregistré n'existe pas pour la résolution retenue.
    rate_missing: bool,
}

fn display_choices(profile: &Profile, modes: &[Mode], usable: bool) -> DisplayChoices {
    let display = &profile.display;
    let resolutions = model::resolutions(modes);
    let stored = display.x_resolution.zip(display.y_resolution);
    let current = model::current_mode(modes).map(|mode| (mode.width, mode.height));
    let active = if display.use_resolution {
        stored
    } else {
        current.or(stored)
    };
    let rates = active
        .map(|res| model::rates(modes, res))
        .unwrap_or_default();
    let resolution_missing =
        usable && display.use_resolution && stored.is_none_or(|res| !resolutions.contains(&res));
    let rate_missing = usable
        && display.use_refresh_rate
        && display
            .refresh_rate
            .is_none_or(|hz| model::matching_rate(&rates, f64::from(hz)).is_none());
    DisplayChoices {
        resolutions,
        rates,
        active,
        resolution_missing,
        rate_missing,
    }
}

impl App {
    /// Ouvre l'éditeur sur un profil existant, ou sur un nouveau profil.
    pub(crate) fn open_editor(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let profile = match id {
            Some(id) => self
                .snap
                .profiles
                .iter()
                .find(|p| p.id.as_str() == id)
                .cloned(),
            None => Some(model::new_profile()),
        };
        let Some(profile) = profile else {
            self.notify(t("admin.error.notFound"), false);
            return;
        };
        self.editor = Some(Editor::new(
            id.map(str::to_owned),
            profile,
            cx.focus_handle(),
        ));
        self.navigate(Page::Editor, cx);
    }

    /// Suit les changements du profil côté daemon : rechargé si le brouillon
    /// est intact, signalé comme conflit sinon.
    pub(crate) fn sync_editor(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let Some(id) = editor.id.as_deref() else {
            return;
        };
        let Some(live) = self.snap.profiles.iter().find(|p| p.id.as_str() == id) else {
            return;
        };
        let mut live = live.clone();
        if live.fan.custom_fan_curve.table_cpu.len() < 2 {
            live.fan.custom_fan_curve.table_cpu = FanCurve::default_custom().table_cpu;
        }
        if live == editor.base {
            return;
        }
        if editor.dirty() {
            editor.conflict = true;
        } else {
            editor.base = live;
            editor.reset();
        }
    }

    fn edit(&mut self, change: impl FnOnce(&mut Profile)) {
        if let Some(editor) = self.editor.as_mut() {
            change(&mut editor.draft);
        }
    }

    fn editor_read_only(&self) -> bool {
        self.editor.as_ref().is_none_or(|editor| {
            !self.snap.admin || (editor.id.is_some() && is_preset(&editor.base))
        })
    }

    fn editor_errors(&self) -> FieldErrors {
        let Some(editor) = self.editor.as_ref() else {
            return FieldErrors::new();
        };
        let others: BTreeSet<String> = self
            .snap
            .profiles
            .iter()
            .filter(|p| Some(p.id.as_str()) != editor.id.as_deref())
            .map(|p| model::normalized_name(&p.name))
            .collect();
        let hw = &self.snap.hardware;
        let ctgp_max = hw["nvidia"]["maxCtgpOffset"].as_f64().unwrap_or(200.);
        let mut errors = model::validate_profile(&editor.draft, &others, ctgp_max);
        let choices = display_choices(&editor.draft, &self.display_modes(), self.agent_usable());
        if choices.resolution_missing {
            errors.push(("resolution", "editor.error.resolutionUnavailable"));
        }
        if choices.rate_missing {
            errors.push(("refreshRate", "editor.error.refreshUnavailable"));
        }
        if self.tdp_exposed() && editor.tdp.iter().any(|text| parse_number(text).is_none()) {
            errors.push(("tdpValues", "editor.error.tdpValues"));
        }
        if hw["nvidia"]["available"] == true && parse_number(&editor.ctgp).is_none() {
            errors.push(("ctgpOffset", "editor.error.ctgpOffset"));
        }
        errors
    }

    /// Des limites TDP sont réellement exposées par le matériel.
    pub(crate) fn tdp_exposed(&self) -> bool {
        let odm = &self.snap.hardware["odm"];
        odm["tdpAvailable"] == true && odm["tdp"].as_array().is_some_and(|tdp| !tdp.is_empty())
    }

    /// Choix ODM : `platform_profile` s'il est disponible, sinon `tuxedo-io`.
    pub(crate) fn odm_choices(&self) -> Vec<String> {
        let odm = &self.snap.hardware["odm"];
        let platform = model::strings(&odm["platformProfile"]["choices"]);
        if odm["platformProfile"]["available"] == true && !platform.is_empty() {
            platform
        } else {
            model::strings(&odm["io"]["choices"])
        }
    }

    fn save_editor(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let profile = Box::new(editor.draft.clone());
        self.handle.send(if editor.id.is_some() {
            Command::SaveProfile(profile)
        } else {
            Command::CreateProfile(profile)
        });
    }

    fn can_save(&self) -> bool {
        self.editor.as_ref().is_some_and(|editor| {
            editor.dirty() && !editor.conflict && !self.snap.busy && !self.editor_read_only()
        }) && self.editor_errors().is_empty()
    }

    /// Barre d'enregistrement, fixe en bas de l'éditeur.
    pub(crate) fn editor_bar(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let dirty = self.editor.as_ref().is_some_and(Editor::dirty);
        let invalid = dirty && !self.editor_errors().is_empty();
        let (message, color) = if self.snap.busy {
            (t("editor.saving"), theme.secondary)
        } else if self.editor_read_only() {
            (t("editor.readOnlyBadge"), theme.secondary)
        } else if invalid {
            (t("editor.invalidHint"), theme.danger)
        } else if dirty {
            (t("editor.unsaved"), theme.warning)
        } else {
            (t("editor.floatingClean"), theme.secondary)
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(0.75))
            .px(rems(1.25))
            .py(rems(0.625))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(div().text_color(color).child(message))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(
                        button(
                            "editor-reset",
                            t("common.cancel"),
                            ButtonVariant::Outline,
                            cx,
                        )
                        .disabled(!dirty || self.snap.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(editor) = this.editor.as_mut() {
                                if editor.id.is_some() {
                                    editor.reset();
                                } else {
                                    this.editor = None;
                                    this.navigate(Page::Profiles, cx);
                                }
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        button("editor-save", t("common.save"), ButtonVariant::Primary, cx)
                            .disabled(!self.can_save())
                            .on_click(cx.listener(|this, _, _, _| this.save_editor())),
                    ),
            )
    }

    /// Ligne « libellé — curseur — valeur ».
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slider_row(
        &mut self,
        key: &str,
        label: String,
        description: String,
        range: (f32, f32, f32),
        value: f32,
        text: String,
        disabled: bool,
        change: impl Fn(&mut App, f32, &mut Context<App>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let control = self.slider(key, range, value, disabled, change, None, window, cx);
        widgets::row(
            label,
            description,
            div()
                .flex()
                .items_center()
                .gap(rems(0.75))
                .child(control)
                .child(
                    div()
                        .w(rems(5.))
                        .flex()
                        .justify_end()
                        .text_color(cx.omarchy().bright)
                        .child(text),
                ),
            cx,
        )
    }

    pub(crate) fn editor_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let Some(editor) = self.editor.as_ref() else {
            return div().child(empty_state(
                t("profiles.empty.title"),
                t("profiles.empty.body"),
                cx,
            ));
        };
        let draft = editor.draft.clone();
        let is_new = editor.id.is_none();
        let preset = !is_new && is_preset(&editor.base);
        let conflict = editor.conflict;
        let dirty = editor.dirty();
        let read_only = self.editor_read_only();
        let errors = self.editor_errors();

        let mut badges = div().flex().items_center().gap(rems(0.5));
        if dirty && !read_only {
            badges = badges.child(badge(t("editor.unsaved"), Status::Warning, cx));
        }
        if read_only {
            badges = badges.child(badge(t("editor.readOnlyBadge"), Status::Neutral, cx));
        }
        badges = badges.child(
            button("editor-back", t("common.back"), ButtonVariant::Outline, cx)
                .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Profiles, cx))),
        );
        let title = if is_new {
            t("editor.newTitle")
        } else if draft.name.trim().is_empty() {
            t("editor.title")
        } else {
            draft.name.clone()
        };
        let mut page = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(widgets::page_header(
                &t("nav.profiles"),
                title,
                t("editor.subtitle"),
                badges,
                cx,
            ))
            .children(self.admin_notice(cx));
        if preset {
            page = page.child(widgets::notice(
                Status::Neutral,
                t("editor.presetReadOnly"),
                t("editor.presetReadOnlyHint"),
                None,
                cx,
            ));
        }
        if is_new {
            page = page.child(widgets::notice(
                Status::Neutral,
                t("editor.newTitle"),
                t("editor.newProfileHint"),
                None,
                cx,
            ));
        }
        if conflict {
            let reload = button(
                "editor-reload",
                t("editor.reload"),
                ButtonVariant::Outline,
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(editor) = this.editor.as_mut() {
                    editor.reset();
                }
                this.sync_editor();
                this.notify(t("editor.reloaded"), true);
                cx.notify();
            }))
            .into_any_element();
            page = page.child(widgets::notice(
                Status::Warning,
                t("editor.conflictTitle"),
                t("editor.conflictBody"),
                Some(reload),
                cx,
            ));
        }
        page.child(self.editor_general(&draft, read_only, &errors, window, cx))
            .child(self.editor_display(&draft, read_only, &errors, window, cx))
            .child(self.editor_cpu(&draft, read_only, &errors, window, cx))
            .children(self.editor_webcam(&draft, read_only, cx))
            .child(self.editor_fan(&draft, read_only, &errors, window, cx))
            .children(self.editor_odm(&draft, read_only, &errors, window, cx))
    }

    fn editor_general(
        &mut self,
        draft: &Profile,
        read_only: bool,
        errors: &FieldErrors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let name = self.text_input(
            "editor/name",
            &draft.name,
            &t("editor.namePlaceholder"),
            move |this, text, _| {
                if !this.editor_read_only() {
                    this.edit(|p| p.name = text.to_owned());
                }
            },
            window,
            cx,
        );
        let description = self.text_input(
            "editor/description",
            &draft.description,
            &t("editor.descriptionPlaceholder"),
            move |this, text, _| {
                if !this.editor_read_only() {
                    this.edit(|p| p.description = text.to_owned());
                }
            },
            window,
            cx,
        );
        let _ = read_only;
        widgets::card(t("editor.general"), t("editor.generalHint"), None, cx)
            .child(widgets::row(
                t("editor.name"),
                "",
                div().w(rems(24.)).child(name),
                cx,
            ))
            .children(
                model::first_error(errors, &["name"]).map(|key| widgets::error_text(t(key), cx)),
            )
            .child(widgets::row(
                t("editor.description"),
                "",
                div().w(rems(24.)).child(description),
                cx,
            ))
            .children(
                model::first_error(errors, &["description"])
                    .map(|key| widgets::error_text(t(key), cx)),
            )
            .child(widgets::muted(t("editor.assignmentMatrixHint"), cx))
    }

    fn editor_display(
        &mut self,
        draft: &Profile,
        read_only: bool,
        errors: &FieldErrors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let modes = self.display_modes();
        let usable = self.agent_usable();
        let available = self.agent_available();
        let choices = display_choices(draft, &modes, usable);
        let current = model::current_mode(&modes);
        let display = &draft.display;

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
            } else {
                Status::Warning
            },
            cx,
        );
        let mut card = widgets::card(
            t("editor.display"),
            t("editor.displayHint"),
            Some(state.into_any_element()),
            cx,
        )
        .children(self.agent_notice(cx))
        .children(
            model::first_error(errors, &["brightness", "resolution", "refreshRate"])
                .map(|key| widgets::error_text(t(key), cx)),
        );

        // Luminosité.
        let toggle = self.toggle(
            "ed-use-brightness",
            t("display.brightnessSet"),
            display.use_brightness,
            read_only,
            |this, on, _| this.edit(|p| p.display.use_brightness = on),
            cx,
        );
        card = card.child(widgets::row(t("display.brightnessSet"), "", toggle, cx));
        card = card.child(self.slider_row(
            "editor/brightness",
            t("display.brightness"),
            String::new(),
            (0., 100., 1.),
            f32::from(display.brightness),
            format!("{} %", display.brightness),
            read_only || !display.use_brightness,
            |this, value, _| this.edit(|p| p.display.brightness = value.round() as u8),
            window,
            cx,
        ));

        // Résolution : uniquement des modes réellement exposés par l'agent.
        let fallback = current.map(|mode| (mode.width, mode.height));
        let first_resolution = choices.resolutions.first().copied();
        let toggle = self.toggle(
            "ed-use-resolution",
            t("display.resolutionSet"),
            display.use_resolution,
            read_only || !usable,
            move |this, on, _| {
                this.edit(|p| {
                    p.display.use_resolution = on;
                    if on
                        && p.display.x_resolution.zip(p.display.y_resolution).is_none()
                        && let Some((width, height)) = fallback.or(first_resolution)
                    {
                        p.display.x_resolution = Some(width);
                        p.display.y_resolution = Some(height);
                    }
                });
            },
            cx,
        );
        card = card.child(widgets::row(
            t("display.resolutionSet"),
            t("display.resolutionHint"),
            toggle,
            cx,
        ));
        let resolution_items: Vec<(String, String)> = choices
            .resolutions
            .iter()
            .map(|(w, h)| {
                (
                    format!("{w}x{h}"),
                    tf(
                        "display.resolutionFormat",
                        &[&w.to_string(), &h.to_string()],
                    ),
                )
            })
            .collect();
        let selected = display
            .x_resolution
            .zip(display.y_resolution)
            .or(choices.active)
            .map(|(w, h)| format!("{w}x{h}"));
        let all_modes = modes.clone();
        let current_rate = current.map(|mode| mode.hz);
        let control = self.select(
            "editor/resolution",
            resolution_items,
            selected.as_deref(),
            read_only || !usable || !display.use_resolution,
            14.,
            move |this, value, _| {
                let Some((w, h)) = value
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                else {
                    return;
                };
                let rates = model::rates(&all_modes, (w, h));
                this.edit(|p| {
                    p.display.x_resolution = Some(w);
                    p.display.y_resolution = Some(h);
                    // Garder un taux qui existe pour la nouvelle résolution.
                    let keep = p
                        .display
                        .refresh_rate
                        .and_then(|hz| model::matching_rate(&rates, f64::from(hz)))
                        .or_else(|| current_rate.and_then(|hz| model::matching_rate(&rates, hz)))
                        .or_else(|| rates.first().copied());
                    if p.display.use_refresh_rate || p.display.refresh_rate.is_some() {
                        p.display.refresh_rate = keep.map(|hz| hz as f32);
                    }
                });
            },
            window,
            cx,
        );
        card = card.child(widgets::row(t("display.resolution"), "", control, cx));
        if choices.resolution_missing {
            card = card.child(widgets::warning_text(
                t("display.resolutionUnavailableHint"),
                cx,
            ));
        }

        // Taux de rafraîchissement : écran interne uniquement.
        let rates = choices.rates.clone();
        let default_rate = current_rate
            .and_then(|hz| model::matching_rate(&rates, hz))
            .or_else(|| rates.first().copied());
        let rate_available = usable && !rates.is_empty();
        let toggle = self.toggle(
            "ed-use-refresh",
            t("display.refreshSet"),
            display.use_refresh_rate,
            read_only || !rate_available,
            move |this, on, _| {
                this.edit(|p| {
                    p.display.use_refresh_rate = on;
                    if on && p.display.refresh_rate.is_none() {
                        p.display.refresh_rate = default_rate.map(|hz| hz as f32);
                    }
                });
            },
            cx,
        );
        card = card.child(widgets::row(
            t("display.refreshSet"),
            t("display.refreshHelp"),
            toggle,
            cx,
        ));
        let rate_items: Vec<(String, String)> = rates
            .iter()
            .map(|hz| (format!("{hz}"), model::fmt_hz(*hz)))
            .collect();
        let selected = display
            .refresh_rate
            .and_then(|hz| model::matching_rate(&rates, f64::from(hz)))
            .map(|hz| format!("{hz}"));
        let control = self.select(
            "editor/refresh",
            rate_items,
            selected.as_deref(),
            read_only || !rate_available || !display.use_refresh_rate,
            14.,
            |this, value, _| {
                if let Ok(hz) = value.parse::<f32>() {
                    this.edit(|p| p.display.refresh_rate = Some(hz));
                }
            },
            window,
            cx,
        );
        card = card.child(widgets::row(t("display.refreshRate"), "", control, cx));
        if choices.rate_missing {
            card = card.child(widgets::warning_text(
                t("display.refreshUnavailableHint"),
                cx,
            ));
        }
        if !usable {
            // Sans agent, les valeurs enregistrées restent visibles.
            if let Some((w, h)) = display.x_resolution.zip(display.y_resolution) {
                card = card.child(widgets::info_row(
                    t("display.resolution"),
                    tf(
                        "display.resolutionFormat",
                        &[&w.to_string(), &h.to_string()],
                    ),
                    None,
                    cx,
                ));
            }
            if let Some(hz) = display.refresh_rate {
                card = card.child(widgets::info_row(
                    t("display.refreshRate"),
                    model::fmt_hz(f64::from(hz)),
                    None,
                    cx,
                ));
            }
        }
        card.children(self.display_facts(cx))
    }

    /// Backend, écran interne et mode courant rapportés par l'agent.
    pub(crate) fn display_facts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let display = &self.snap.display;
        let mut rows = Vec::new();
        if display.is_null() {
            return rows;
        }
        let backend = &display["backend"];
        let name = model::text(&backend["name"]);
        let backend = if name.is_empty() {
            model::text(&backend["kind"])
        } else {
            name
        };
        if !backend.is_empty() {
            rows.push(
                widgets::info_row(t("gpui.displayBackend"), backend, None, cx).into_any_element(),
            );
        }
        let output = &display["internalOutput"];
        let name = model::text(&output["name"]);
        if !name.is_empty() {
            let label = if output["connected"] == false {
                format!("{name} ({})", t("display.disconnected"))
            } else {
                name
            };
            rows.push(widgets::info_row(t("display.output"), label, None, cx).into_any_element());
        }
        if let Some(mode) = model::current_mode(&self.display_modes()) {
            rows.push(
                widgets::info_row(t("gpui.displayCurrent"), model::mode_label(mode), None, cx)
                    .into_any_element(),
            );
        }
        rows
    }

    fn editor_cpu(
        &mut self,
        draft: &Profile,
        read_only: bool,
        errors: &FieldErrors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let caps = self.snap.hardware["cpu"].clone();
        let cpu = &draft.cpu;
        let total = caps["totalCores"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .unwrap_or_else(|| cpu.online_cores.unwrap_or(1).max(1));
        let governors = model::strings(&caps["availableGovernors"]);
        let perf_known = governors.is_empty() || governors.iter().any(|g| g == "performance");
        let mut epp_values = model::strings(&caps["availableEpp"]);
        let epp_known = !epp_values.is_empty();
        let epp = cpu.energy_performance_preference.clone();
        if !epp.is_empty() && !epp_values.contains(&epp) {
            epp_values.push(epp.clone());
        }

        let mut card = widgets::card(t("editor.cpu"), t("editor.cpuHint"), None, cx).children(
            model::first_error(errors, &["cores", "freqMin", "freqMax"])
                .map(|key| widgets::error_text(t(key), cx)),
        );
        card = card.child(
            self.slider_row(
                "editor/cores",
                t("cpu.cores"),
                String::new(),
                (1., total.max(2) as f32, 1.),
                cpu.online_cores.unwrap_or(total) as f32,
                cpu.online_cores
                    .map_or_else(|| t("common.auto"), |cores| format!("{cores} / {total}")),
                read_only,
                |this, value, _| {
                    this.edit(|p| p.cpu.online_cores = Some(value.round().max(1.) as u32))
                },
                window,
                cx,
            ),
        );
        let toggle = self.toggle(
            "ed-perf-gov",
            t("cpu.perfGov"),
            cpu.use_max_perf_gov,
            read_only || !perf_known,
            |this, on, _| this.edit(|p| p.cpu.use_max_perf_gov = on),
            cx,
        );
        card = card.child(widgets::row(
            t("cpu.perfGov"),
            if perf_known {
                t("cpu.perfGovHint")
            } else {
                t("cpu.perfGovUnavailable")
            },
            toggle,
            cx,
        ));
        let control = self.select(
            "editor/epp",
            epp_values.iter().map(|v| (v.clone(), v.clone())).collect(),
            Some(&epp),
            read_only || !epp_known,
            14.,
            |this, value, _| this.edit(|p| p.cpu.energy_performance_preference = value.to_owned()),
            window,
            cx,
        );
        card = card.child(widgets::row(
            t("cpu.epp"),
            if epp_known {
                t("cpu.eppHint")
            } else {
                t("cpu.eppUnavailable")
            },
            control,
            cx,
        ));
        let min = cpu.scaling_min_frequency.map(model::khz_to_ghz);
        card = card.child(self.slider_row(
            "editor/freq-min",
            t("cpu.minFreq"),
            String::new(),
            FREQ_RANGE,
            min.map_or(FREQ_RANGE.0, |ghz| ghz as f32),
            min.map_or_else(|| t("common.auto"), |ghz| format!("{ghz:.1} GHz")),
            read_only,
            |this, value, _| {
                this.edit(|p| {
                    p.cpu.scaling_min_frequency = Some(model::ghz_to_khz(f64::from(value)));
                });
            },
            window,
            cx,
        ));
        let (max_value, max_text) = match cpu.scaling_max_frequency {
            Some(FrequencyLimit::Hz(khz)) => {
                let ghz = model::khz_to_ghz(khz);
                (ghz as f32, format!("{ghz:.1} GHz"))
            }
            Some(FrequencyLimit::Reduced) => (FREQ_RANGE.0, t("cpu.reduced")),
            None => (FREQ_RANGE.1, t("common.auto")),
        };
        card = card.child(self.slider_row(
            "editor/freq-max",
            t("cpu.maxFreq"),
            String::new(),
            FREQ_RANGE,
            max_value,
            max_text,
            read_only,
            |this, value, _| {
                this.edit(|p| {
                    p.cpu.scaling_max_frequency =
                        Some(FrequencyLimit::Hz(model::ghz_to_khz(f64::from(value))));
                });
            },
            window,
            cx,
        ));
        let toggle = self.toggle(
            "ed-no-turbo",
            t("cpu.noTurbo"),
            cpu.no_turbo,
            read_only,
            |this, on, _| this.edit(|p| p.cpu.no_turbo = on),
            cx,
        );
        card.child(widgets::row(
            t("cpu.noTurbo"),
            t("cpu.noTurboHint"),
            toggle,
            cx,
        ))
    }

    fn editor_webcam(
        &mut self,
        draft: &Profile,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let webcam = &self.snap.hardware["webcam"];
        let killswitch = webcam["switchAvailable"] == true;
        let has_devices = webcam["devices"].as_array().is_some_and(|d| !d.is_empty());
        if !killswitch && !has_devices {
            return None;
        }
        let status = self.toggle(
            "ed-webcam-on",
            t("editor.webcamOn"),
            draft.webcam.status,
            read_only,
            |this, on, _| this.edit(|p| p.webcam.status = on),
            cx,
        );
        let apply = self.toggle(
            "ed-webcam-apply",
            t("editor.webcamApply"),
            draft.webcam.use_status,
            read_only,
            |this, on, _| this.edit(|p| p.webcam.use_status = on),
            cx,
        );
        let manage = button(
            "ed-webcam-tools",
            t("settings.manage"),
            ButtonVariant::Outline,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Tools, cx)));
        Some(
            widgets::card(t("editor.webcam"), t("editor.webcamHint"), None, cx)
                .child(widgets::row(t("editor.webcamOn"), "", status, cx))
                .child(widgets::row(
                    t("editor.webcamApply"),
                    t("editor.webcamApplyHint"),
                    apply,
                    cx,
                ))
                .when(killswitch, |card| {
                    card.child(widgets::info_row(
                        t("editor.webcamKillswitch"),
                        "",
                        Some((t("common.supported"), Status::Success)),
                        cx,
                    ))
                })
                .child(widgets::row(
                    t("editor.webcamPresets"),
                    t("editor.webcamPresetsHint"),
                    manage,
                    cx,
                )),
        )
    }

    fn editor_fan(
        &mut self,
        draft: &Profile,
        read_only: bool,
        errors: &FieldErrors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let fan = &draft.fan;
        let mode = model::fan_mode(draft);
        let backend = model::text(&self.snap.fans["backend"]);
        let trailing =
            (!backend.is_empty()).then(|| badge(backend, Status::Neutral, cx).into_any_element());
        let mut card = widgets::card(t("editor.fan"), t("editor.fanHint"), trailing, cx).children(
            model::first_error(errors, &["fanMin", "fanMax", "fanOffset", "fanCurve"])
                .map(|key| widgets::error_text(t(key), cx)),
        );
        let toggle = self.toggle(
            "ed-fan-control",
            t("fan.useControl"),
            fan.use_control,
            read_only,
            |this, on, _| this.edit(|p| p.fan.use_control = on),
            cx,
        );
        card = card.child(widgets::row(
            t("fan.useControl"),
            t("fan.useControlHint"),
            toggle,
            cx,
        ));
        let items = model::FAN_MODES
            .iter()
            .map(|key| ((*key).to_owned(), t(&format!("fan.mode.{key}"))))
            .collect();
        let control = self.select(
            "editor/fan-mode",
            items,
            Some(mode),
            read_only,
            14.,
            |this, value, _| {
                this.edit(|p| {
                    if value == "auto" {
                        p.fan.use_control = false;
                    } else {
                        p.fan.use_control = true;
                        p.fan.fan_profile = model::canonical_fan_profile(value).to_owned();
                    }
                });
            },
            window,
            cx,
        );
        card = card.child(widgets::row(t("fan.profile"), "", control, cx));
        card = card.child(self.slider_row(
            "editor/fan-min",
            t("fan.min"),
            String::new(),
            (0., 100., 1.),
            fan.minimum_fanspeed,
            format!("{:.0} %", fan.minimum_fanspeed),
            read_only,
            |this, value, _| this.edit(|p| p.fan.minimum_fanspeed = value.round()),
            window,
            cx,
        ));
        card = card.child(self.slider_row(
            "editor/fan-max",
            t("fan.max"),
            String::new(),
            (0., 100., 1.),
            fan.maximum_fanspeed,
            format!("{:.0} %", fan.maximum_fanspeed),
            read_only,
            |this, value, _| this.edit(|p| p.fan.maximum_fanspeed = value.round()),
            window,
            cx,
        ));
        card = card.child(self.slider_row(
            "editor/fan-offset",
            t("fan.offset"),
            String::new(),
            (-10., 10., 1.),
            fan.offset_fanspeed,
            format!("{:+.0} %", fan.offset_fanspeed),
            read_only,
            |this, value, _| this.edit(|p| p.fan.offset_fanspeed = value.round()),
            window,
            cx,
        ));
        card.child(separator(cx)).child(self.fan_curve_block(
            draft,
            !read_only && mode == "custom",
            window,
            cx,
        ))
    }

    /// Courbe de ventilation : éditable seulement avec le profil
    /// « Personnalisé », affichée en lecture seule sinon.
    fn fan_curve_block(
        &mut self,
        draft: &Profile,
        editable: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let points = &draft.fan.custom_fan_curve.table_cpu;
        let (selected, plot, focus) = match self.editor.as_ref() {
            Some(editor) => (
                editor.point.filter(|index| *index < points.len()),
                Rc::clone(&editor.plot),
                editor.focus.clone(),
            ),
            None => return div(),
        };
        let mut area = div()
            .id("fan-curve")
            .h(px(fan_curve::PLOT_HEIGHT + 2. * fan_curve::PAD))
            .p(px(fan_curve::PAD))
            .border_1()
            .border_color(if editable && focus.is_focused(window) {
                theme.accent
            } else {
                theme.border
            })
            .child(fan_curve::plot(points, selected, editable, plot, cx));
        if editable {
            area = area
                .track_focus(&focus)
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.curve_press(event, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.curve_edit(|points, bounds, _| {
                            let hit = fan_curve::point_at(points, bounds, event.position)?;
                            fan_curve::remove_point(points, hit).then_some(None)
                        });
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    let Some(editor) = this.editor.as_mut() else {
                        return;
                    };
                    if !editor.dragging {
                        return;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        editor.dragging = false;
                        return;
                    }
                    this.curve_edit(|points, bounds, selected| {
                        let (temp, speed) = fan_curve::value_at(bounds, event.position);
                        fan_curve::move_point(points, selected?, temp, speed).then_some(selected)
                    });
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.curve_release()),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.curve_release()),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    let (dt, ds, remove) = match event.keystroke.key.as_str() {
                        "left" => (-1., 0., false),
                        "right" => (1., 0., false),
                        "up" => (0., 1., false),
                        "down" => (0., -1., false),
                        "delete" | "backspace" => (0., 0., true),
                        _ => return,
                    };
                    this.curve_edit(|points, _, selected| {
                        let index = selected?;
                        if remove {
                            return fan_curve::remove_point(points, index).then_some(None);
                        }
                        let entry = points.get(index)?;
                        let (temp, speed) = (entry.temp + dt, entry.speed + ds);
                        fan_curve::move_point(points, index, temp, speed).then_some(selected)
                    });
                    cx.stop_propagation();
                    cx.notify();
                }));
        }
        let status = match selected.and_then(|index| points.get(index)) {
            Some(point) if editable => format!(
                "{} — {}",
                t("gpui.curveSelected"),
                tf(
                    "fanCurve.pointTooltip",
                    &[
                        &format!("{:.0}", point.temp),
                        &format!("{:.0}", point.speed)
                    ]
                )
            ),
            _ if editable => t("fanCurve.dragHint"),
            _ => t("fan.customOnly"),
        };
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(rems(0.75))
            .child(div().flex_1().min_w_0().child(widgets::muted(status, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.5))
                    .child(widgets::muted(
                        tf(
                            "fanCurve.count",
                            &[
                                &points.len().to_string(),
                                &tcc_core::MAX_FAN_TABLE_ENTRIES.to_string(),
                            ],
                        ),
                        cx,
                    ))
                    .child(
                        button("curve-add", t("fanCurve.add"), ButtonVariant::Outline, cx)
                            .disabled(!editable)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.curve_edit(|points, _, _| {
                                    fan_curve::add_point(points).map(Some)
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        button(
                            "curve-remove",
                            t("gpui.removePoint"),
                            ButtonVariant::Outline,
                            cx,
                        )
                        .disabled(!editable || selected.is_none() || points.len() <= 2)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.curve_edit(|points, _, selected| {
                                fan_curve::remove_point(points, selected?).then_some(None)
                            });
                            cx.notify();
                        })),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .gap(rems(0.5))
            .child(widgets::subheading(t("fan.curve"), cx))
            .child(widgets::muted(t("fan.curveHint"), cx))
            .child(fan_curve::framed(area, cx).when(!editable, |frame| frame.opacity(0.6)))
            .child(controls)
    }

    /// Applique une modification à la courbe. `change` reçoit les points, la
    /// zone de tracé et le point sélectionné ; il renvoie la nouvelle
    /// sélection, ou `None` s'il n'a rien changé.
    fn curve_edit(
        &mut self,
        change: impl FnOnce(
            &mut Vec<tcc_core::FanTableEntry>,
            Bounds<gpui_kit::Pixels>,
            Option<usize>,
        ) -> Option<Option<usize>>,
    ) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let bounds = editor.plot.get();
        let points = &mut editor.draft.fan.custom_fan_curve.table_cpu;
        if let Some(selection) = change(points, bounds, editor.point) {
            editor.point = selection;
        }
    }

    fn curve_press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.focus.focus(window, cx);
        let bounds = editor.plot.get();
        let points = &mut editor.draft.fan.custom_fan_curve.table_cpu;
        let hit = fan_curve::point_at(points, bounds, event.position);
        if hit.is_none() && event.click_count >= 2 {
            let (temp, speed) = fan_curve::value_at(bounds, event.position);
            editor.point = fan_curve::insert_point(points, temp, speed);
            editor.dragging = false;
        } else {
            editor.point = hit;
            editor.dragging = hit.is_some();
        }
        cx.notify();
    }

    fn curve_release(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.dragging = false;
        }
    }

    fn editor_odm(
        &mut self,
        draft: &Profile,
        read_only: bool,
        errors: &FieldErrors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let hw = self.snap.hardware.clone();
        let odm = &hw["odm"];
        let nvidia = hw["nvidia"]["available"] == true;
        let tdp_exposed = self.tdp_exposed();
        if odm["available"] != true && !tdp_exposed && !nvidia {
            return None;
        }
        let source = if odm["platformProfile"]["available"] == true {
            "platform_profile"
        } else if odm["io"]["available"] == true {
            "tuxedo-io"
        } else {
            ""
        };
        let trailing =
            (!source.is_empty()).then(|| badge(source, Status::Neutral, cx).into_any_element());
        let mut card = widgets::card(t("editor.odm"), t("editor.odmHint"), trailing, cx).children(
            model::first_error(errors, &["odmProfile", "tdpValues", "ctgpOffset"])
                .map(|key| widgets::error_text(t(key), cx)),
        );
        if odm["available"] == true {
            let mut choices = self.odm_choices();
            let stored = draft.odm_profile.name.clone().unwrap_or_default();
            if !stored.is_empty() && !choices.contains(&stored) {
                choices.push(stored.clone());
            }
            let mut items = vec![(String::new(), t("editor.odmUnchanged"))];
            items.extend(
                choices
                    .iter()
                    .map(|name| (name.clone(), model::odm_label(name))),
            );
            let control = self.select(
                "editor/odm",
                items,
                Some(&stored),
                read_only || choices.is_empty(),
                14.,
                |this, value, _| {
                    this.edit(|p| {
                        p.odm_profile.name = (!value.is_empty()).then(|| value.to_owned());
                    });
                },
                window,
                cx,
            );
            card = card.child(widgets::row(
                t("settings.odm"),
                if choices.is_empty() {
                    t("editor.odmNoChoice")
                } else if odm["platformProfile"]["available"] == true {
                    t("editor.odmPpdHint")
                } else {
                    String::new()
                },
                control,
                cx,
            ));
        }
        if tdp_exposed {
            card = card.child(widgets::subheading(t("settings.tdp"), cx));
            let limits = odm["tdp"].as_array().cloned().unwrap_or_default();
            for (index, limit) in limits.iter().enumerate() {
                let value = self
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.tdp.get(index).cloned())
                    .unwrap_or_default();
                let count = limits.len();
                let field = self.text_input(
                    &format!("editor/tdp/{index}"),
                    &value,
                    "W",
                    move |this, text, _| {
                        if this.editor_read_only() {
                            return;
                        }
                        let Some(editor) = this.editor.as_mut() else {
                            return;
                        };
                        editor
                            .tdp
                            .resize(count.max(editor.tdp.len()), String::new());
                        if let Some(slot) = editor.tdp.get_mut(index) {
                            *slot = text.to_owned();
                        }
                        let parsed: Option<Vec<f64>> =
                            editor.tdp.iter().map(|text| parse_number(text)).collect();
                        if let Some(values) = parsed {
                            editor.draft.odm_power_limits.tdp_values = values;
                        }
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
        }
        if nvidia {
            let max = hw["nvidia"]["maxCtgpOffset"].as_f64().unwrap_or(200.);
            let value = self
                .editor
                .as_ref()
                .map(|editor| editor.ctgp.clone())
                .unwrap_or_default();
            let field = self.text_input(
                "editor/ctgp",
                &value,
                "W",
                |this, text, _| {
                    if this.editor_read_only() {
                        return;
                    }
                    let Some(editor) = this.editor.as_mut() else {
                        return;
                    };
                    editor.ctgp = text.to_owned();
                    if let Some(watts) = parse_number(text) {
                        editor.draft.nvidia_power_ctrl_profile.ctgp_offset = watts;
                    }
                },
                window,
                cx,
            );
            card = card.child(widgets::row(
                t("settings.ctgp"),
                tf("settings.hwCtgpRange", &[&format!("{max:.0}")]),
                div().w(rems(8.)).child(field),
                cx,
            ));
        }
        Some(card.child(widgets::muted(t("editor.odmPersistedHint"), cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes() -> Vec<Mode> {
        let mode = |width, height, hz, current| Mode {
            width,
            height,
            hz,
            current,
        };
        vec![
            mode(2560, 1600, 240., true),
            mode(2560, 1600, 60., false),
            mode(1920, 1200, 60., false),
        ]
    }

    #[test]
    fn display_choices_follow_the_current_mode_by_default() {
        let profile = Profile::new("custom-a", "A");
        let choices = display_choices(&profile, &modes(), true);
        assert_eq!(choices.active, Some((2560, 1600)));
        assert_eq!(choices.rates, [240., 60.]);
        assert!(!choices.resolution_missing && !choices.rate_missing);
    }

    #[test]
    fn stale_display_settings_are_flagged_only_with_a_usable_agent() {
        let mut profile = Profile::new("custom-a", "A");
        profile.display.use_resolution = true;
        profile.display.x_resolution = Some(2880);
        profile.display.y_resolution = Some(1800);
        profile.display.use_refresh_rate = true;
        profile.display.refresh_rate = Some(120.);
        let choices = display_choices(&profile, &modes(), true);
        assert!(choices.resolution_missing && choices.rate_missing);
        // Sans agent, les valeurs enregistrées sont conservées telles quelles.
        let choices = display_choices(&profile, &[], false);
        assert!(!choices.resolution_missing && !choices.rate_missing);
    }

    #[test]
    fn parses_numbers_with_a_decimal_comma() {
        assert_eq!(parse_number(" 12,5 "), Some(12.5));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(number_text(45.), "45");
        assert_eq!(number_text(12.5), "12.5");
    }
}
