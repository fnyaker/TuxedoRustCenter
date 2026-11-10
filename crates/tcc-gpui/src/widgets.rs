// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Éléments de présentation partagés par les pages : en-têtes, cartes,
//! lignes de réglage, bandeaux, jauges et courbes d'historique.
//!
//! Aucun état ici : chaque fonction rend ce qu'on lui donne.

use gpui_kit::{
    AnyElement, App as GpuiApp, Bounds, ClickEvent, FontWeight, Hsla, PathBuilder, Pixels,
    SharedString, Window, canvas, div, point, prelude::*, px, rems,
};
use gpui_omarchy::*;

/// Adapte un écouteur `(valeur, fenêtre, app)` au callback `on_change`.
pub fn change<T>(
    listener: impl Fn(&T, &mut Window, &mut GpuiApp) + 'static,
) -> impl Fn(T, &ClickEvent, &mut Window, &mut GpuiApp) {
    move |value, _, window, cx| listener(&value, window, cx)
}

/// Texte secondaire.
pub fn muted(text: impl Into<SharedString>, cx: &GpuiApp) -> gpui_kit::Div {
    div()
        .text_color(cx.omarchy().secondary)
        .text_size(rems(0.6875))
        .child(text.into())
}

/// Message d'erreur sous un champ.
pub fn error_text(text: impl Into<SharedString>, cx: &GpuiApp) -> gpui_kit::Div {
    div()
        .text_color(cx.omarchy().danger)
        .text_size(rems(0.6875))
        .child(text.into())
}

/// Avertissement sous un champ.
pub fn warning_text(text: impl Into<SharedString>, cx: &GpuiApp) -> gpui_kit::Div {
    div()
        .text_color(cx.omarchy().warning)
        .text_size(rems(0.6875))
        .child(text.into())
}

/// En-tête de page : surtitre facultatif, titre, sous-titre et actions à
/// droite.
pub fn page_header(
    eyebrow: &str,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: impl IntoElement,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let subtitle = subtitle.into();
    div()
        .flex()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(rems(1.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .min_w_0()
                .when(!eyebrow.is_empty(), |column| {
                    column.child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme.accent)
                            .child(eyebrow.to_owned()),
                    )
                })
                .child(
                    div()
                        .text_size(rems(1.25))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.bright)
                        .child(title.into()),
                )
                .when(!subtitle.is_empty(), |column| {
                    column.child(div().text_color(theme.secondary).child(subtitle))
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(rems(0.5))
                .child(actions),
        )
}

/// Carte : titre, indication et élément à droite du titre (badge, bouton).
pub fn card(
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    trailing: Option<AnyElement>,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let hint = hint.into();
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(rems(0.75))
        .p(rems(1.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(
            div()
                .flex()
                .items_start()
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
                                .text_size(rems(0.875))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.bright)
                                .child(title.into()),
                        )
                        .when(!hint.is_empty(), |column| column.child(muted(hint, cx))),
                )
                .children(trailing),
        )
}

/// Sous-titre à l'intérieur d'une carte.
pub fn subheading(text: impl Into<SharedString>, cx: &GpuiApp) -> gpui_kit::Div {
    div()
        .mt(rems(0.25))
        .font_weight(FontWeight::BOLD)
        .text_color(cx.omarchy().bright)
        .child(text.into())
}

/// Ligne de réglage : libellé et description à gauche, contrôle à droite.
pub fn row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let description = description.into();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(1.))
        .min_h(rems(2.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .flex_1()
                .min_w_0()
                .child(div().child(label.into()))
                .when(!description.is_empty(), |column| {
                    column.child(muted(description, cx))
                }),
        )
        .child(div().flex().items_center().flex_shrink_0().child(control))
}

/// Ligne d'information en lecture seule, avec badge facultatif.
pub fn info_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    badge_label: Option<(String, Status)>,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let mut right = div()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .child(div().text_color(theme.bright).child(value.into()));
    if let Some((label, status)) = badge_label.filter(|(label, _)| !label.is_empty()) {
        right = right.child(badge(label, status, cx));
    }
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(1.))
        .min_h(rems(1.5))
        .child(div().text_color(theme.secondary).child(label.into()))
        .child(right)
}

/// Bandeau : titre, détail et action facultative.
pub fn notice(
    status: Status,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let color = match status {
        Status::Neutral => theme.accent,
        other => other.color(cx),
    };
    let body = body.into();
    div()
        .flex()
        .items_center()
        .gap(rems(0.75))
        .p(rems(0.625))
        .border_1()
        .border_color(color.opacity(0.35))
        .bg(color.opacity(0.06))
        .child(
            icon(match status {
                Status::Success => IconName::Check,
                Status::Neutral => IconName::Info,
                Status::Warning | Status::Error => IconName::TriangleAlert,
            })
            .size(rems(0.875))
            .flex_shrink_0()
            .text_color(color),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(0.125))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(title.into()),
                )
                .when(!body.is_empty(), |column| {
                    column.child(div().text_color(theme.foreground).child(body))
                }),
        )
        .children(action)
}

/// Jauge en arc.
pub struct Gauge {
    /// Valeur, ou `None` si le capteur n'est pas exposé.
    pub value: Option<f64>,
    /// Valeur pleine échelle.
    pub max: f64,
    /// Valeur affichée au centre (déjà formatée, sans unité).
    pub text: String,
    /// Unité affichée sous la valeur.
    pub unit: String,
    /// Légende sous la jauge.
    pub label: String,
    /// Fraction (0–1) à partir de laquelle la jauge passe à l'avertissement.
    pub warn: f64,
}

/// Jauge en arc de 270°, ouverte vers le bas. La couleur passe à
/// l'avertissement au-delà de `warn`, puis au danger près du maximum. Un
/// capteur absent affiche un tiret, jamais un zéro inventé.
pub fn gauge(spec: Gauge, cx: &GpuiApp) -> gpui_kit::Div {
    /// Diamètre en rem et épaisseur de l'arc en fraction du diamètre : la
    /// jauge suit le zoom de l'interface.
    const SIZE: f32 = 7.75;
    const STROKE: f32 = 0.075;
    let theme = cx.omarchy().clone();
    let available = spec.value.is_some();
    let fraction = spec.value.map_or(0., |value| {
        (value / spec.max.max(f64::EPSILON)).clamp(0., 1.)
    });
    let color = if fraction >= (spec.warn + 1.) / 2. {
        theme.danger
    } else if fraction >= spec.warn {
        theme.warning
    } else {
        theme.accent
    };
    let track = theme.border;
    let arc = move |bounds: Bounds<Pixels>, sweep: f64, color: Hsla, window: &mut Window| {
        if sweep <= 0. {
            return;
        }
        let center = bounds.center();
        let diameter = f32::from(bounds.size.width.min(bounds.size.height));
        let stroke = diameter * STROKE;
        let radius = f64::from(diameter - stroke) / 2. - 1.;
        let steps = ((sweep / 270.) * 72.).ceil().max(2.) as usize;
        let mut builder = PathBuilder::stroke(px(stroke));
        for step in 0..=steps {
            let angle = (135. + sweep * step as f64 / steps as f64).to_radians();
            let at = point(
                center.x + px((radius * angle.cos()) as f32),
                center.y + px((radius * angle.sin()) as f32),
            );
            if step == 0 {
                builder.move_to(at);
            } else {
                builder.line_to(at);
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
        .gap(rems(0.25))
        .child(
            div()
                .relative()
                .size(rems(SIZE))
                .child(
                    canvas(
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
                                .text_color(if available {
                                    theme.bright
                                } else {
                                    theme.secondary
                                })
                                .child(if available {
                                    spec.text
                                } else {
                                    crate::model::DASH.to_owned()
                                }),
                        )
                        .child(
                            div()
                                .text_size(rems(0.6875))
                                .text_color(theme.secondary)
                                .child(spec.unit),
                        ),
                ),
        )
        .child(div().text_color(theme.secondary).child(spec.label))
}

/// Rangée de jauges centrée.
pub fn gauge_row() -> gpui_kit::Div {
    div()
        .flex()
        .flex_wrap()
        .justify_center()
        .gap(rems(1.75))
        .py(rems(0.25))
}

/// Courbe d'historique : légende, dernière valeur et tracé à échelle
/// automatique. Une série vide laisse la zone libre plutôt que d'inventer des
/// points.
pub fn history_chart(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    values: &[f32],
    color: Hsla,
    height_rems: f32,
    cx: &GpuiApp,
) -> gpui_kit::Div {
    let theme = cx.omarchy().clone();
    let values = values.to_vec();
    let grid = theme.border.opacity(0.5);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(rems(10.))
        .gap(rems(0.25))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(rems(0.5))
                .text_size(rems(0.6875))
                .text_color(theme.secondary)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(0.375))
                        .child(div().size(px(8.)).bg(color))
                        .child(label.into()),
                )
                .child(value.into()),
        )
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    for step in 0..=2 {
                        let y = bounds.origin.y + bounds.size.height * (step as f32 / 2.);
                        window.paint_quad(gpui_kit::fill(
                            Bounds::new(
                                point(bounds.origin.x, y - px(0.5)),
                                gpui_kit::size(bounds.size.width, px(1.)),
                            ),
                            grid,
                        ));
                    }
                    if values.len() < 2 {
                        return;
                    }
                    let low = values.iter().copied().fold(f32::INFINITY, f32::min);
                    let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let span = (high - low).max(1.);
                    let pad = px(3.);
                    let width = bounds.size.width;
                    let usable = bounds.size.height - pad * 2.;
                    let last = (values.len() - 1) as f32;
                    let at = |index: usize, value: f32| {
                        point(
                            bounds.origin.x + width * (index as f32 / last),
                            bounds.origin.y + pad + usable * (1. - (value - low) / span),
                        )
                    };
                    let mut area = PathBuilder::fill();
                    area.move_to(point(bounds.origin.x, bounds.bottom()));
                    for (index, value) in values.iter().enumerate() {
                        area.line_to(at(index, *value));
                    }
                    area.line_to(point(bounds.right(), bounds.bottom()));
                    area.close();
                    if let Ok(path) = area.build() {
                        window.paint_path(path, color.opacity(0.12));
                    }
                    let mut line = PathBuilder::stroke(px(1.75));
                    for (index, value) in values.iter().enumerate() {
                        if index == 0 {
                            line.move_to(at(index, *value));
                        } else {
                            line.line_to(at(index, *value));
                        }
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .w_full()
            .h(rems(height_rems)),
        )
}
