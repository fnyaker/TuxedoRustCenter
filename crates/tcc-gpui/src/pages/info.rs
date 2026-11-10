// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! À propos : versions, appareil, licences et contact du support.
//!
//! Toutes les valeurs viennent du daemon ou du binaire ; rien n'est inventé.

use gpui_kit::{Context, div, prelude::*, rems};
use gpui_omarchy::*;

use crate::app::App;
use crate::i18n::t;
use crate::model;
use crate::widgets;

/// Site du support TUXEDO (ouvert dans le navigateur par défaut).
const SUPPORT_URL: &str = "https://www.tuxedocomputers.com/support";
/// Téléphone du support TUXEDO.
const SUPPORT_PHONE: &str = "+49 4192 99117 0";

impl App {
    pub(crate) fn info(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let snap = &self.snap;
        let device = &snap.hardware["device"];
        let unavailable = || t("common.unavailable");
        let status = if snap.connected {
            badge(t("app.live"), Status::Success, cx)
        } else {
            badge(t("state.disconnected"), Status::Warning, cx)
        };

        let versions = widgets::card(t("app.name"), t("info.realSources"), None, cx)
            .child(widgets::info_row(
                t("info.version"),
                env!("CARGO_PKG_VERSION"),
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("info.daemon"),
                if snap.daemon_version.is_empty() {
                    unavailable()
                } else {
                    snap.daemon_version.clone()
                },
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("info.ipc"),
                if snap.contract > 0 {
                    format!("v{}", snap.contract)
                } else {
                    unavailable()
                },
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("dash.agent"),
                if snap.agent_version.is_empty() {
                    unavailable()
                } else {
                    snap.agent_version.clone()
                },
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("info.kernel"),
                model::text_or_unavailable(&device["kernelRelease"]),
                None,
                cx,
            ))
            .child(widgets::info_row(
                t("info.architecture"),
                model::text_or_unavailable(&device["architecture"]),
                None,
                cx,
            ));

        let mut hardware = widgets::card(t("info.device"), t("info.deviceHint"), None, cx);
        for (label, field) in [
            ("info.vendor", "vendor"),
            ("info.model", "productName"),
            ("info.modelCode", "model"),
            ("info.sku", "sku"),
            ("info.driver", "tuxedoDriverVersion"),
        ] {
            hardware = hardware.child(widgets::info_row(
                t(label),
                model::text_or_unavailable(&device[field]),
                None,
                cx,
            ));
        }
        if snap.connected && device["isTuxedo"] == false {
            hardware = hardware.child(widgets::warning_text(t("info.notTuxedo"), cx));
        }

        let open = button(
            "support-open",
            t("support.contact.openSite"),
            ButtonVariant::Outline,
            cx,
        )
        .on_click(|_, _, cx| cx.open_url(SUPPORT_URL));
        let support = widgets::card(
            t("support.contact.title"),
            t("support.contact.desc"),
            Some(open.into_any_element()),
            cx,
        )
        .child(widgets::info_row(
            t("support.contact.phone"),
            SUPPORT_PHONE,
            None,
            cx,
        ))
        .child(widgets::info_row(
            t("support.contact.online"),
            "www.tuxedocomputers.com/support",
            None,
            cx,
        ))
        .child(widgets::muted(t("support.contact.privacy"), cx));

        let licenses = widgets::card(t("info.licenses"), t("info.licensesHint"), None, cx)
            .child(widgets::muted(t("info.original"), cx))
            .child(widgets::muted(
                "TuxedoRustCenter — GPL-3.0-or-later. GPUI, gpui-kit, gpui-omarchy — Apache-2.0 ; gpui-base — MIT.",
                cx,
            ));

        div()
            .flex()
            .flex_col()
            .gap(rems(1.))
            .child(widgets::page_header(
                "",
                t("info.title"),
                t("info.subtitle"),
                status,
                cx,
            ))
            .child(versions)
            .child(hardware)
            .child(support)
            .child(licenses)
    }
}
