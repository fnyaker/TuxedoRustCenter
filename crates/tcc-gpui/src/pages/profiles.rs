// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Liste des profils : filtres, recherche, grille ou liste, activation,
//! duplication, suppression, import et export.

use std::rc::Rc;

use gpui_kit::{AnyElement, Context, FontWeight, PathPromptOptions, Window, div, prelude::*, rems};
use gpui_omarchy::*;
use tcc_core::Profile;

use crate::app::App;
use crate::i18n::{plural, t};
use crate::model;
use crate::pages::is_preset;
use crate::widgets;
use crate::worker::Command;

/// Dossier proposé pour l'export : Documents s'il existe, sinon le dossier
/// personnel.
fn export_directory() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| "/".into(), std::path::PathBuf::from);
    let documents = home.join("Documents");
    if documents.is_dir() { documents } else { home }
}

impl App {
    /// Les profils peuvent être modifiés (service d'administration présent,
    /// aucune opération en cours).
    pub(crate) fn can_mutate(&self) -> bool {
        self.snap.connected && self.snap.admin && !self.snap.busy
    }

    fn filtered_profiles(&self) -> Vec<Profile> {
        let query = self.search.trim().to_lowercase();
        let active = self.active_id();
        let settings = self.settings();
        self.snap
            .profiles
            .iter()
            .filter(|profile| match self.filter {
                1 => is_preset(profile),
                2 => !is_preset(profile),
                3 => {
                    let id = profile.id.as_str();
                    id == active
                        || settings.is_some_and(|s| {
                            model::assigned_to(s, id, "ac") || model::assigned_to(s, id, "battery")
                        })
                }
                _ => true,
            })
            .filter(|profile| {
                query.is_empty()
                    || profile.name.to_lowercase().contains(&query)
                    || profile.description.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    fn start_export(&mut self, cx: &mut Context<Self>) {
        if !self.snap.profiles.iter().any(|p| !is_preset(p)) {
            self.notify(t("admin.error.nothingToExport"), false);
            return;
        }
        let receiver = cx.prompt_for_new_path(&export_directory(), Some("TCC_Profiles.json"));
        cx.spawn(async move |this, cx| {
            let chosen = receiver.await.ok().and_then(Result::ok).flatten();
            let _ = this.update(cx, |this, cx| {
                match chosen {
                    Some(path) => this.handle.send(Command::Export(path)),
                    None => this.notify(t("profiles.exportCancelled"), true),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_import(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("profiles.importTitle").into()),
        });
        cx.spawn(async move |this, cx| {
            let chosen = receiver
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next());
            let _ = this.update(cx, |this, cx| {
                match chosen {
                    Some(path) => this.handle.send(Command::Import(path)),
                    None => this.notify(t("profiles.importCancelled"), true),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn profiles(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let snap = Rc::clone(&self.snap);
        let theme = cx.omarchy().clone();
        let can_mutate = self.can_mutate();
        let has_custom = snap.profiles.iter().any(|p| !is_preset(p));

        let actions = div()
            .flex()
            .items_center()
            .gap(rems(0.5))
            .child(
                button(
                    "profiles-import",
                    t("profiles.import"),
                    ButtonVariant::Outline,
                    cx,
                )
                .disabled(!can_mutate)
                .on_click(cx.listener(|this, _, _, cx| this.start_import(cx))),
            )
            .child(
                button(
                    "profiles-export",
                    t("profiles.export"),
                    ButtonVariant::Outline,
                    cx,
                )
                .disabled(snap.busy || !has_custom)
                .on_click(cx.listener(|this, _, _, cx| this.start_export(cx))),
            )
            .child(
                button(
                    "profiles-new",
                    t("profiles.new"),
                    ButtonVariant::Primary,
                    cx,
                )
                .disabled(!can_mutate)
                .on_click(cx.listener(|this, _, _, cx| this.open_editor(None, cx))),
            );
        let header =
            widgets::page_header("", t("profiles.title"), t("profiles.subtitle"), actions, cx);

        let shown = self.filtered_profiles();
        let filters = self.segmented(
            "profiles-filter",
            vec![
                t("profiles.filter.all"),
                t("profiles.filter.preset"),
                t("profiles.filter.custom"),
                t("profiles.filter.inUse"),
            ],
            self.filter,
            |this, index, _| this.filter = index,
            window,
            cx,
        );
        let search_value = self.search.clone();
        let search = self
            .text_input(
                "profiles/search",
                &search_value,
                &t("profiles.searchPlaceholder"),
                |this, text, _| this.search = text.to_owned(),
                window,
                cx,
            )
            .prefix(icon(IconName::Search).size(rems(0.875)));
        let list_mode = self.prefs.list_mode;
        let view = self.segmented(
            "profiles-view",
            vec![t("profiles.view.grid"), t("profiles.view.list")],
            usize::from(list_mode),
            |this, index, _| {
                this.prefs.list_mode = index == 1;
                this.prefs.save();
            },
            window,
            cx,
        );
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.75))
            .child(div().w(rems(24.)).child(filters))
            .child(div().text_color(theme.secondary).child(format!(
                "{} {}",
                shown.len(),
                plural(shown.len(), "profiles.unit", "profiles.unitPlural")
            )))
            .child(div().flex_1().min_w(rems(10.)).child(search))
            .child(div().w(rems(10.)).child(view));

        let mut page = div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(header)
            .children(self.admin_notice(cx))
            .child(toolbar);
        if shown.is_empty() {
            return page.child(
                empty_state(t("profiles.empty.title"), t("profiles.empty.body"), cx)
                    .border_1()
                    .border_color(theme.border),
            );
        }
        let cards = shown
            .iter()
            .enumerate()
            .map(|(index, profile)| self.profile_card(index, profile, list_mode, cx));
        page = page.child(if list_mode {
            div().flex().flex_col().gap(rems(0.5)).children(cards)
        } else {
            div().flex().flex_wrap().gap(rems(1.)).children(cards)
        });
        page
    }

    fn profile_card(
        &self,
        index: usize,
        profile: &Profile,
        list_mode: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.omarchy().clone();
        let snap = &self.snap;
        let id = profile.id.as_str().to_owned();
        let name = profile.name.clone();
        let preset = is_preset(profile);
        let active = id == self.active_id();
        let manual = snap.state["manualOverride"] == true;
        let can_mutate = self.can_mutate();

        let mut badges = div().flex().flex_wrap().items_center().gap(rems(0.375));
        if active {
            badges = badges.child(badge(t("profiles.active"), Status::Success, cx));
        }
        if preset {
            badges = badges.child(badge(t("profiles.preset"), Status::Neutral, cx));
        }
        if let Some(settings) = self.settings() {
            if model::assigned_to(settings, &id, "ac") {
                badges = badges.child(badge(t("profiles.assignedAc"), Status::Neutral, cx));
            }
            if model::assigned_to(settings, &id, "battery") {
                badges = badges.child(badge(t("profiles.assignedBat"), Status::Neutral, cx));
            }
        }

        let or_auto = |value: Option<String>| value.unwrap_or_else(|| t("common.auto"));
        let cpu = &profile.cpu;
        let metrics: Vec<(String, String)> = vec![
            (
                t("cpu.cores"),
                or_auto(cpu.online_cores.map(|cores| cores.to_string())),
            ),
            (
                t("cpu.maxFreq"),
                match cpu.scaling_max_frequency {
                    Some(tcc_core::FrequencyLimit::Reduced) => t("cpu.reduced"),
                    Some(tcc_core::FrequencyLimit::Hz(khz)) => {
                        format!("{:.1} GHz", model::khz_to_ghz(khz))
                    }
                    None => t("common.auto"),
                },
            ),
            (
                t("fan.profile"),
                t(&format!("fan.mode.{}", model::fan_mode(profile))),
            ),
            (
                t("display.brightness"),
                if profile.display.use_brightness {
                    format!("{} %", profile.display.brightness)
                } else {
                    t("display.unchanged")
                },
            ),
            (
                t("display.refreshRate"),
                profile
                    .display
                    .effective_refresh_rate()
                    .map_or_else(|| t("display.unchanged"), |hz| model::fmt_hz(f64::from(hz))),
            ),
        ];

        let (use_id, use_name) = (id.clone(), name.clone());
        let activate = if active && manual {
            button(
                ("profile-auto", index),
                t("profiles.deactivate"),
                ButtonVariant::Outline,
                cx,
            )
            .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::ClearTemp)))
        } else {
            button(
                ("profile-use", index),
                t("profiles.activate"),
                ButtonVariant::Primary,
                cx,
            )
            .disabled(active || !snap.connected)
            .on_click(cx.listener(move |this, _, _, _| {
                this.handle.send(Command::UseProfile {
                    id: use_id.clone(),
                    name: use_name.clone(),
                });
            }))
        };
        let edit_id = id.clone();
        let (copy_id, copy_name) = (id.clone(), name.clone());
        let (delete_id, delete_name) = (id.clone(), name.clone());
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(0.375))
            .child(activate)
            .child(
                button(
                    ("profile-edit", index),
                    t(if preset { "gpui.view" } else { "profiles.edit" }),
                    ButtonVariant::Outline,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_editor(Some(&edit_id), cx);
                })),
            )
            .child(
                button(
                    ("profile-copy", index),
                    t("profiles.clone"),
                    ButtonVariant::Outline,
                    cx,
                )
                .disabled(!can_mutate)
                .on_click(cx.listener(move |this, _, _, _| {
                    this.handle.send(Command::Duplicate {
                        id: copy_id.clone(),
                        name: copy_name.clone(),
                    });
                })),
            );
        if !preset {
            actions = actions.child(
                button(
                    ("profile-delete", index),
                    t("profiles.delete"),
                    ButtonVariant::Danger,
                    cx,
                )
                .disabled(!can_mutate)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ask_delete(delete_id.clone(), delete_name.clone(), window, cx);
                })),
            );
        }

        let title = div()
            .flex()
            .flex_col()
            .gap(rems(0.25))
            .min_w_0()
            .child(
                div()
                    .text_size(rems(0.9375))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.bright)
                    .child(name),
            )
            .child(badges)
            .when(!profile.description.is_empty(), |column| {
                column.child(widgets::muted(profile.description.clone(), cx))
            });
        let frame = div()
            .p(rems(0.875))
            .border_1()
            .border_color(if active { theme.accent } else { theme.border })
            .bg(theme.background);
        if list_mode {
            let summary = metrics
                .iter()
                .take(3)
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>()
                .join(" · ");
            return frame
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(rems(0.75))
                .child(title.flex_1())
                .child(div().text_color(theme.secondary).child(summary))
                .child(actions)
                .into_any_element();
        }
        let mut grid = div().flex().flex_col().gap(rems(0.125));
        for (label, value) in metrics {
            grid = grid.child(
                div()
                    .flex()
                    .justify_between()
                    .gap(rems(0.75))
                    .child(div().text_color(theme.secondary).child(label))
                    .child(value),
            );
        }
        frame
            .flex()
            .flex_col()
            .gap(rems(0.75))
            .w(rems(20.5))
            .flex_shrink_0()
            .child(title)
            .child(grid)
            .child(actions)
            .into_any_element()
    }
}
