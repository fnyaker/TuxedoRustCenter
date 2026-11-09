// SPDX-License-Identifier: GPL-3.0-or-later
//! Backends non implémentés : Wayland générique.
//!
//! Ces stubs retournent [`tcc_hw::HwError::NotSupported`] avec un message
//! explicite. L'UI/le daemon peuvent ainsi exposer un état « non supporté »
//! sans crash ni fallback dangereux (pas de XRandR sous Wayland, pas de
//! commande shell).

use tcc_hw::display::{
    ApplyOutcome, DisplayBackendKind, DisplayMode, DisplayModeBackend, InternalOutput, OutputInfo,
};
use tcc_hw::error::{HwError, Result as HwResult};

/// Backend inerte documenté.
#[derive(Debug, Clone)]
pub struct StubBackend {
    kind: DisplayBackendKind,
    name: &'static str,
    reason: &'static str,
}

impl StubBackend {
    /// Wayland générique (`zwlr_output_manager_v1`) non implémenté.
    pub fn wayland_generic() -> Self {
        Self {
            kind: DisplayBackendKind::Wlroots,
            name: "wayland-generic",
            reason: "Wayland générique non supporté (compositeur non reconnu)",
        }
    }

    /// Aucun backend d'affichage (TTY, session inconnue).
    pub fn unsupported() -> Self {
        Self {
            kind: DisplayBackendKind::Unsupported,
            name: "unsupported",
            reason: "aucun backend d'affichage disponible dans cette session",
        }
    }

    /// Raison exposée à l'UI.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl DisplayModeBackend for StubBackend {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    fn kind(&self) -> DisplayBackendKind {
        self.kind
    }

    fn list_outputs(&mut self) -> HwResult<Vec<OutputInfo>> {
        Err(HwError::NotSupported(self.reason))
    }

    fn apply_refresh_rate(
        &mut self,
        _output: &InternalOutput,
        _mode: &DisplayMode,
    ) -> HwResult<ApplyOutcome> {
        Err(HwError::NotSupported(self.reason))
    }

    fn is_available(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_are_not_supported() {
        for mut stub in [StubBackend::wayland_generic(), StubBackend::unsupported()] {
            assert!(!stub.is_available());
            assert!(matches!(stub.list_outputs(), Err(HwError::NotSupported(_))));
            assert!(!stub.reason().is_empty());
        }
    }

    #[test]
    fn unsupported_stub_preserves_kind() {
        assert_eq!(
            StubBackend::unsupported().kind(),
            DisplayBackendKind::Unsupported
        );
        assert_eq!(
            StubBackend::wayland_generic().kind(),
            DisplayBackendKind::Wlroots
        );
    }
}
