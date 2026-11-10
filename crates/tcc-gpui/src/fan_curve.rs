// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Courbe de ventilation : tracé et édition directe à la souris.
//!
//! Comme dans l'interface Qt (`FanCurveChart.qml`) : glisser un point pour le
//! déplacer, double-clic pour en ajouter un, clic droit ou Suppr pour
//! l'enlever. Les températures restent strictement croissantes (un point ne
//! peut pas dépasser ses voisins) et il reste toujours au moins deux points.
//! Seule la table CPU est éditée ; la table GPU du profil est conservée.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    App as GpuiApp, Bounds, PathBuilder, Pixels, Point, canvas, div, point, prelude::*, px,
    relative, rems, size,
};
use gpui_omarchy::ActiveTheme;
use tcc_core::{FanTableEntry, MAX_FAN_TABLE_ENTRIES};

/// Bornes de l'axe des températures (°C) et des vitesses (%).
const LOW: f32 = 0.;
const HIGH: f32 = 100.;
/// Écart minimal entre deux points voisins (°C).
const GAP: f32 = 1.;
/// Rayon de capture d'un point (pixels).
const HIT_RADIUS: f32 = 14.;
/// Hauteur de la zone de tracé (pixels).
pub const PLOT_HEIGHT: f32 = 220.;

/// Zone de tracé à l'écran, relevée à chaque rendu pour convertir la position
/// de la souris en (température, vitesse).
pub type PlotBounds = Rc<Cell<Bounds<Pixels>>>;

/// Position de la souris → (température, vitesse), bornées et arrondies à
/// l'unité.
#[must_use]
pub fn value_at(bounds: Bounds<Pixels>, position: Point<Pixels>) -> (f32, f32) {
    let width = f32::from(bounds.size.width).max(1.);
    let height = f32::from(bounds.size.height).max(1.);
    let fx = (f32::from(position.x - bounds.origin.x) / width).clamp(0., 1.);
    let fy = (f32::from(position.y - bounds.origin.y) / height).clamp(0., 1.);
    (
        (LOW + fx * (HIGH - LOW)).round(),
        (HIGH - fy * (HIGH - LOW)).round(),
    )
}

fn screen(bounds: Bounds<Pixels>, entry: &FanTableEntry) -> Point<Pixels> {
    let fx = (entry.temp - LOW) / (HIGH - LOW);
    let fy = 1. - (entry.speed - LOW) / (HIGH - LOW);
    point(
        bounds.origin.x + bounds.size.width * fx,
        bounds.origin.y + bounds.size.height * fy,
    )
}

/// Point le plus proche de la souris, dans le rayon de capture.
#[must_use]
pub fn point_at(
    points: &[FanTableEntry],
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let at = screen(bounds, entry);
            let dx = f32::from(at.x - position.x);
            let dy = f32::from(at.y - position.y);
            (index, dx * dx + dy * dy)
        })
        .filter(|(_, distance)| *distance <= HIT_RADIUS * HIT_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// Déplace un point : la température reste entre ses voisins, la vitesse
/// entre 0 et 100. Renvoie `true` si le point a bougé.
pub fn move_point(points: &mut [FanTableEntry], index: usize, temp: f32, speed: f32) -> bool {
    if index >= points.len() {
        return false;
    }
    let low = index
        .checked_sub(1)
        .and_then(|i| points.get(i))
        .map_or(LOW, |p| p.temp + GAP);
    let high = points.get(index + 1).map_or(HIGH, |p| p.temp - GAP);
    if low > high {
        return false;
    }
    let temp = temp.clamp(low, high);
    let speed = speed.clamp(LOW, HIGH);
    let Some(entry) = points.get_mut(index) else {
        return false;
    };
    if entry.temp == temp && entry.speed == speed {
        return false;
    }
    entry.temp = temp;
    entry.speed = speed;
    true
}

/// Ajoute un point, en gardant les températures triées. Refusé s'il est trop
/// proche d'un point existant ou si la table est pleine. Renvoie son index.
pub fn insert_point(points: &mut Vec<FanTableEntry>, temp: f32, speed: f32) -> Option<usize> {
    if points.len() >= MAX_FAN_TABLE_ENTRIES
        || !(LOW..=HIGH).contains(&temp)
        || points.iter().any(|p| (p.temp - temp).abs() < GAP)
    {
        return None;
    }
    let index = points.partition_point(|p| p.temp < temp);
    points.insert(index, FanTableEntry::new(temp, speed.clamp(LOW, HIGH)));
    Some(index)
}

/// Ajoute un point au milieu du plus grand intervalle libre, sur la courbe.
pub fn add_point(points: &mut Vec<FanTableEntry>) -> Option<usize> {
    let (index, _) = points
        .windows(2)
        .enumerate()
        .map(|(index, pair)| (index, pair[1].temp - pair[0].temp))
        .filter(|(_, gap)| *gap >= 2. * GAP)
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let (a, b) = (points.get(index)?, points.get(index + 1)?);
    let temp = ((a.temp + b.temp) / 2.).round();
    let speed = ((a.speed + b.speed) / 2.).round();
    insert_point(points, temp, speed)
}

/// Supprime un point (il en reste toujours au moins deux).
pub fn remove_point(points: &mut Vec<FanTableEntry>, index: usize) -> bool {
    if points.len() <= 2 || index >= points.len() {
        return false;
    }
    points.remove(index);
    true
}

/// Tracé de la courbe : grille, aire, ligne et points. `plot` reçoit la zone
/// de tracé à l'écran.
pub fn plot(
    points: &[FanTableEntry],
    selected: Option<usize>,
    editable: bool,
    plot: PlotBounds,
    cx: &GpuiApp,
) -> impl IntoElement + use<> {
    let theme = cx.omarchy().clone();
    let points = points.to_vec();
    let grid = theme.border.opacity(0.5);
    let color = if editable {
        theme.accent
    } else {
        theme.secondary
    };
    canvas(
        move |bounds, _, _| plot.set(bounds),
        move |bounds, (), window, _| {
            for step in 0..=4 {
                let fraction = step as f32 / 4.;
                window.paint_quad(gpui_kit::fill(
                    Bounds::new(
                        point(
                            bounds.origin.x,
                            bounds.origin.y + bounds.size.height * fraction - px(0.5),
                        ),
                        size(bounds.size.width, px(1.)),
                    ),
                    grid,
                ));
            }
            for step in 0..=5 {
                let fraction = step as f32 / 5.;
                window.paint_quad(gpui_kit::fill(
                    Bounds::new(
                        point(
                            bounds.origin.x + bounds.size.width * fraction - px(0.5),
                            bounds.origin.y,
                        ),
                        size(px(1.), bounds.size.height),
                    ),
                    grid,
                ));
            }
            let (Some(first), Some(last)) = (points.first(), points.last()) else {
                return;
            };
            if points.len() >= 2 {
                let mut area = PathBuilder::fill();
                area.move_to(point(screen(bounds, first).x, bounds.bottom()));
                for entry in &points {
                    area.line_to(screen(bounds, entry));
                }
                area.line_to(point(screen(bounds, last).x, bounds.bottom()));
                area.close();
                if let Ok(path) = area.build() {
                    window.paint_path(path, color.opacity(0.12));
                }
                let mut line = PathBuilder::stroke(px(2.));
                for (index, entry) in points.iter().enumerate() {
                    if index == 0 {
                        line.move_to(screen(bounds, entry));
                    } else {
                        line.line_to(screen(bounds, entry));
                    }
                }
                if let Ok(path) = line.build() {
                    window.paint_path(path, color);
                }
            }
            for (index, entry) in points.iter().enumerate() {
                let active = editable && selected == Some(index);
                let half = px(if active { 6.5 } else { 4.5 });
                let center = screen(bounds, entry);
                let square = |half: Pixels| {
                    Bounds::new(
                        point(center.x - half, center.y - half),
                        size(half * 2., half * 2.),
                    )
                };
                window.paint_quad(gpui_kit::fill(square(half), color));
                if !active {
                    window.paint_quad(gpui_kit::fill(square(half - px(2.)), theme.background));
                }
            }
        },
    )
    .size_full()
}

/// Marge autour du tracé, à l'intérieur de la zone interactive : elle permet
/// d'atteindre 0 et 100 sans viser le dernier pixel.
pub const PAD: f32 = 10.;

/// Encadre la zone interactive (tracé + marge [`PAD`]) avec les graduations :
/// % à gauche, °C dessous.
pub fn framed(area: impl IntoElement, cx: &GpuiApp) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let label = |text: String| {
        div()
            .text_size(rems(0.625))
            .text_color(theme.secondary)
            .child(text)
    };
    let mut speeds = div()
        .relative()
        .w(rems(2.25))
        .h(px(PLOT_HEIGHT + 2. * PAD))
        .flex_shrink_0();
    for step in 0..=4 {
        let fraction = step as f32 / 4.;
        speeds = speeds.child(
            div()
                .absolute()
                .right(rems(0.125))
                .top(px(PAD + PLOT_HEIGHT * fraction - 6.))
                .child(label(format!("{} %", 100 - step * 25))),
        );
    }
    let mut temps = div().relative().h(rems(1.)).mx(px(PAD));
    for step in 0..=5 {
        let fraction = step as f32 / 5.;
        temps = temps.child(
            div()
                .absolute()
                .top_0()
                .left(relative(fraction))
                .ml(px(-8.))
                .child(label(format!("{}°", step * 20))),
        );
    }
    div().flex().child(speeds).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(area)
            .child(temps),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.), px(20.)), size(px(400.), px(200.)))
    }

    fn curve() -> Vec<FanTableEntry> {
        vec![
            FanTableEntry::new(20., 10.),
            FanTableEntry::new(50., 40.),
            FanTableEntry::new(90., 90.),
        ]
    }

    #[test]
    fn maps_the_pointer_to_values_and_back() {
        let bounds = bounds();
        assert_eq!(value_at(bounds, point(px(10.), px(20.))), (0., 100.));
        assert_eq!(value_at(bounds, point(px(410.), px(220.))), (100., 0.));
        assert_eq!(value_at(bounds, point(px(210.), px(120.))), (50., 50.));
        // Hors de la zone : borné.
        assert_eq!(value_at(bounds, point(px(-50.), px(900.))), (0., 0.));
        let points = curve();
        let at = screen(bounds, &points[1]);
        assert_eq!(point_at(&points, bounds, at), Some(1));
        assert_eq!(point_at(&points, bounds, point(px(300.), px(30.))), None);
    }

    #[test]
    fn a_dragged_point_stays_between_its_neighbours() {
        let mut points = curve();
        assert!(move_point(&mut points, 1, 95., 120.));
        assert_eq!((points[1].temp, points[1].speed), (89., 100.));
        assert!(move_point(&mut points, 1, 5., -3.));
        assert_eq!((points[1].temp, points[1].speed), (21., 0.));
        // Les extrémités vont jusqu'aux bornes de l'axe.
        assert!(move_point(&mut points, 0, -10., 10.));
        assert_eq!(points[0].temp, 0.);
        assert!(!move_point(&mut points, 0, 0., 10.));
        assert!(!move_point(&mut points, 9, 0., 0.));
    }

    #[test]
    fn inserts_sorted_and_refuses_duplicates() {
        let mut points = curve();
        assert_eq!(insert_point(&mut points, 70., 60.), Some(2));
        assert_eq!(insert_point(&mut points, 70.5, 60.), None);
        assert_eq!(insert_point(&mut points, 5., 0.), Some(0));
        let temps: Vec<f32> = points.iter().map(|p| p.temp).collect();
        assert_eq!(temps, [5., 20., 50., 70., 90.]);
    }

    #[test]
    fn adds_in_the_widest_gap_and_keeps_two_points() {
        let mut points = curve();
        // Plus grand intervalle : 50–90, milieu 70 °C, 65 %.
        assert_eq!(add_point(&mut points), Some(2));
        assert_eq!((points[2].temp, points[2].speed), (70., 65.));
        assert!(remove_point(&mut points, 2));
        assert!(remove_point(&mut points, 1));
        assert!(!remove_point(&mut points, 0));
        assert_eq!(points.len(), 2);
        let mut tight = vec![FanTableEntry::new(10., 0.), FanTableEntry::new(11., 5.)];
        assert_eq!(add_point(&mut tight), None);
    }
}
