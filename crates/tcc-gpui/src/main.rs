// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Interface gpui de TuxedoRustCenter, au style Omarchy.
//!
//! Même périmètre que l'interface Qt : tableau de bord, profils et éditeur,
//! outils, réglages, à propos. Aucune logique privilégiée : tout passe par
//! `tcc-client` (D-Bus vers `tccd`, les services d'administration et l'agent
//! de session ; polkit tranche côté service). Les E/S sont dans un thread de
//! fond ([`worker`]) ; le rendu ne bloque jamais.
//!
//! Organisation : [`model`] (logique pure), [`worker`] (E/S), [`app`] (vue
//! racine et contrôles liés au modèle), [`widgets`] et [`fan_curve`]
//! (présentation), [`pages`] (une page par fichier), [`i18n`] / [`strings`]
//! (textes partagés avec Qt), [`prefs`] (préférences locales, autostart).

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod app;
mod fan_curve;
mod i18n;
mod model;
mod pages;
mod prefs;
mod strings;
mod widgets;
mod worker;

use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    let dev_session =
        std::env::var("TCC_DBUS_BUS").is_ok_and(|v| v.eq_ignore_ascii_case("session"));
    // `--page <nom>` ouvre directement une page, `--edit <profil>` l'éditeur
    // d'un profil ; `--zoom <pourcent>` règle la taille de l'interface (30 à
    // 300, tout est dimensionné en rem).
    let mut args = std::env::args().skip(1);
    let mut page = app::Page::Dashboard;
    let mut zoom = 100.;
    let mut edit = None;
    while let Some(arg) = args.next() {
        if arg == "--edit" {
            // Ouvre l'éditeur sur un profil (identifiant ou nom).
            edit = args.next();
            if edit.is_none() {
                eprintln!("tcc-gpui: --edit attend un identifiant ou un nom de profil");
                std::process::exit(2);
            }
        } else if arg == "--zoom" {
            match args.next().and_then(|raw| raw.parse::<f32>().ok()) {
                Some(percent) if (30.0..=300.0).contains(&percent) => zoom = percent,
                _ => {
                    eprintln!("tcc-gpui: --zoom attend un pourcentage entre 30 et 300");
                    std::process::exit(2);
                }
            }
        } else if arg == "--page" {
            match args.next().as_deref().and_then(app::Page::parse) {
                Some(parsed) => page = parsed,
                None => {
                    eprintln!(
                        "tcc-gpui: --page attend dashboard, profiles, tools, settings ou info"
                    );
                    std::process::exit(2);
                }
            }
        }
    }
    let prefs = prefs::Prefs::load();
    i18n::init(prefs.language.as_deref().and_then(i18n::Lang::parse));
    let handle = worker::spawn(dev_session);
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let bounds = Bounds::centered(None, size(px(1120.), px(800.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(i18n::t("app.name").into()),
                    ..TitlebarOptions::default()
                }),
                app_id: Some("tuxedo-rust-center-gpui".to_owned()),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                window.set_rem_size(px(16. * zoom / 100.));
                cx.new(|cx| {
                    app::App::new(
                        handle.clone(),
                        page,
                        prefs.clone(),
                        edit.clone(),
                        window,
                        cx,
                    )
                })
            });
            if let Err(error) = opened {
                eprintln!("tcc-gpui: impossible d'ouvrir la fenêtre : {error:#}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
