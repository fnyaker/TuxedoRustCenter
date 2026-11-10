// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Pages de l'application et vues partagées sur l'état du daemon.

pub mod dashboard;
pub mod editor;
pub mod info;
pub mod profiles;
pub mod settings;
pub mod tools;

use gpui_kit::{AnyElement, Context, prelude::*};
use gpui_omarchy::*;
use tcc_core::Profile;

use crate::app::App;
use crate::i18n::t;
use crate::model::{self, Mode};
use crate::widgets;
use crate::worker::Command;

/// Vrai pour un profil fourni par TUXEDO (lecture seule).
pub(crate) fn is_preset(profile: &Profile) -> bool {
    profile.id.is_builtin_preset() || profile.is_preset()
}

impl App {
    /// L'agent de session répond.
    pub(crate) fn agent_available(&self) -> bool {
        !self.snap.display.is_null()
    }

    /// Modes de l'écran interne exposés par l'agent.
    pub(crate) fn display_modes(&self) -> Vec<Mode> {
        model::modes(&self.snap.display)
    }

    /// L'agent sait changer le mode de l'écran interne.
    pub(crate) fn agent_usable(&self) -> bool {
        let display = &self.snap.display;
        display["available"] == true
            && display["supported"] == true
            && !self.display_modes().is_empty()
    }

    /// Bandeau affiché quand l'agent de session est absent ou sans backend.
    pub(crate) fn agent_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.agent_usable() {
            return None;
        }
        let available = self.agent_available();
        let error = model::text(&self.snap.display["error"]);
        let body = if error.is_empty() {
            t("display.agentUsableHint")
        } else {
            error
        };
        let retry = button(
            "agent-refresh",
            t("display.refreshAgent"),
            ButtonVariant::Outline,
            cx,
        )
        .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::Refresh)))
        .into_any_element();
        Some(
            widgets::notice(
                if available {
                    Status::Warning
                } else {
                    Status::Neutral
                },
                t(if available {
                    "display.agentUnsupported"
                } else {
                    "display.agentUnavailable"
                }),
                body,
                Some(retry),
                cx,
            )
            .into_any_element(),
        )
    }

    /// Bandeau affiché quand le service d'administration ne répond pas : les
    /// profils et réglages restent en lecture seule.
    pub(crate) fn admin_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.snap.admin || !self.snap.connected {
            return None;
        }
        let retry = button("admin-retry", t("common.retry"), ButtonVariant::Outline, cx)
            .on_click(cx.listener(|this, _, _, _| this.handle.send(Command::Refresh)))
            .into_any_element();
        Some(
            widgets::notice(
                Status::Warning,
                t("admin.unavailable.title"),
                t("admin.unavailable.missing"),
                Some(retry),
                cx,
            )
            .into_any_element(),
        )
    }
}
