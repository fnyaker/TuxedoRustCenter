// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Interface gpui de TuxedoRustCenter, au style Omarchy.
//!
//! Aucune logique privilégiée ni métier : tout passe par `tcc-client` (D-Bus
//! vers `tccd` et les services d'administration, polkit tranche côté
//! service). Les E/S sont dans un thread de fond ; le rendu ne bloque jamais.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod app;
mod flatten;
mod worker;

use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    let dev_session =
        std::env::var("TCC_DBUS_BUS").is_ok_and(|v| v.eq_ignore_ascii_case("session"));
    let handle = worker::spawn(dev_session);
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_omarchy::init(cx);
            let bounds = Bounds::centered(None, size(px(1000.), px(720.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("TUXEDO Control Center".into()),
                    ..TitlebarOptions::default()
                }),
                app_id: Some("tuxedo-rust-center-gpui".to_owned()),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                cx.new(|cx| app::App::new(handle.clone(), window, cx))
            });
            if let Err(error) = opened {
                eprintln!("tcc-gpui: impossible d'ouvrir la fenêtre : {error:#}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
