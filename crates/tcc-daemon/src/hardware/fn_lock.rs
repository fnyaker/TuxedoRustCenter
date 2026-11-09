// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker Fn-Lock.
//!
//! `FnLockController` de l'original n'est pas un worker : la GUI lit et écrit
//! `fn_lock` à la demande. Ici un worker de lecture publie l'état (utile pour
//! la tray et l'UI), et `linux-platform` écrit via
//! [`tcc_hw::fn_lock::FnLock`] depuis son setter D-Bus.
//!
//! Le Fn-Lock n'est pas un champ de profil : il est global, comme dans
//! l'original (`SetFnLockStatus`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::fn_lock::FnLock;
use tcc_hw::sysfs::SysfsRoot;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de relecture.
pub const FN_LOCK_TICK: Duration = Duration::from_secs(10);

/// État Fn-Lock exposé à l'UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FnLockData {
    /// Attribut `fn_lock` présent.
    pub supported: bool,
    /// État courant, `None` si non supporté ou illisible.
    pub enabled: Option<bool>,
    /// Le matériel accepte une écriture. L'écriture D-Bus reste soumise à sa
    /// politique d'autorisation séparée (`linux-platform`) : ce drapeau ne
    /// préjuge pas de l'autorisation de l'appelant.
    pub writable: bool,
}

/// Runtime Fn-Lock.
#[derive(Debug)]
pub struct FnLockRuntime {
    fn_lock: FnLock,
    dry_run: bool,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
    error_logged: bool,
    unsupported_logged: bool,
}

impl FnLockRuntime {
    /// Construit le runtime.
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            fn_lock: FnLock::new(root),
            dry_run,
            last_revision: None,
            applied_revision: None,
            error_logged: false,
            unsupported_logged: false,
        }
    }

    /// Lit l'état et applique `settings.fn_lock_status` au changement de
    /// révision.
    pub fn tick(&mut self, input: &HardwareInputs) -> FnLockData {
        let supported = self.fn_lock.is_available();
        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }

        let already_applied =
            self.applied_revision == self.last_revision && self.last_revision.is_some();
        if !self.dry_run && !already_applied {
            self.apply(input, supported);
            // Une tentative par révision quand le matériel est présent : un
            // échec n'est pas rejoué à chaque tick, ce qui évite une boucle
            // d'écriture. Si l'attribut apparaît tardivement, la détection est
            // retentée au tick suivant sans attendre une révision.
            if supported {
                self.applied_revision = self.last_revision;
            }
        }

        let enabled = match self.fn_lock.status() {
            Ok(status) => status,
            Err(error) => {
                tracing::debug!(%error, "état Fn-Lock illisible");
                None
            }
        };
        FnLockData {
            supported,
            enabled,
            writable: supported && !self.dry_run,
        }
    }

    /// Applique l'état demandé, sans erreur fatale si le matériel est absent.
    fn apply(&mut self, input: &HardwareInputs, supported: bool) {
        let Some(desired) = input.settings.fn_lock_status else {
            return;
        };
        if !supported {
            if !self.unsupported_logged {
                self.unsupported_logged = true;
                tracing::warn!("Fn-Lock demandé mais attribut absent : aucune écriture");
            }
            return;
        }
        self.unsupported_logged = false;
        match self.fn_lock.status() {
            Ok(Some(current)) if current == desired => {
                self.error_logged = false;
            }
            Ok(_) => match self.fn_lock.set(desired) {
                Ok(()) => {
                    self.error_logged = false;
                }
                Err(error) => {
                    if !self.error_logged {
                        self.error_logged = true;
                        tracing::warn!(%error, desired, "écriture Fn-Lock impossible");
                    }
                }
            },
            Err(error) => {
                if !self.error_logged {
                    self.error_logged = true;
                    tracing::warn!(%error, "état Fn-Lock illisible avant écriture");
                }
            }
        }
    }
}

/// Démarre le worker Fn-Lock.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = FnLockRuntime::new(root, dry_run);
    spawn_worker(
        "fn-lock",
        FN_LOCK_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_fn_lock,
        FnLockRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, contents: &str) {
        let path = root.join(tcc_hw::fn_lock::FN_LOCK_PATH);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn reads_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1\n");
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.supported);
        assert_eq!(data.enabled, Some(true));
        assert!(data.writable);
    }

    #[test]
    fn dry_run_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1\n");
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.supported);
        assert_eq!(data.enabled, Some(true));
        assert!(!data.writable);
    }

    fn input(settings: tcc_core::Settings, revision: u64) -> HardwareInputs {
        HardwareInputs::new(settings, None, false, revision)
    }

    #[test]
    fn applies_requested_state_and_reflects_it() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "0\n");
        let settings = tcc_core::Settings {
            fn_lock_status: Some(true),
            ..tcc_core::Settings::default()
        };
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(settings.clone(), 1));
        assert!(data.supported);
        assert!(data.writable);
        assert_eq!(data.enabled, Some(true));
        assert_eq!(
            fs::read_to_string(dir.path().join(tcc_hw::fn_lock::FN_LOCK_PATH)).unwrap(),
            "1"
        );

        // Même révision : une modification externe n'est pas écrasée.
        write(dir.path(), "0\n");
        let data = runtime.tick(&input(settings.clone(), 1));
        assert_eq!(data.enabled, Some(false));

        // Nouvelle révision : l'état demandé est réappliqué.
        let data = runtime.tick(&input(settings, 2));
        assert_eq!(data.enabled, Some(true));
    }

    #[test]
    fn applies_false_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1\n");
        let settings = tcc_core::Settings {
            fn_lock_status: Some(false),
            ..tcc_core::Settings::default()
        };
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(settings, 1));
        assert_eq!(data.enabled, Some(false));
        assert_eq!(
            fs::read_to_string(dir.path().join(tcc_hw::fn_lock::FN_LOCK_PATH)).unwrap(),
            "0"
        );
    }

    #[test]
    fn unsupported_or_dry_run_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let settings = tcc_core::Settings {
            fn_lock_status: Some(true),
            ..tcc_core::Settings::default()
        };
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&input(settings.clone(), 1));
        assert!(!data.supported);
        assert!(!data.writable);
        assert_eq!(data.enabled, None);

        write(dir.path(), "0\n");
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&input(settings, 1));
        assert!(data.supported);
        assert!(!data.writable);
        assert_eq!(data.enabled, Some(false));
        assert_eq!(
            fs::read_to_string(dir.path().join(tcc_hw::fn_lock::FN_LOCK_PATH))
                .unwrap()
                .trim(),
            "0"
        );
    }

    #[test]
    fn absent_is_not_supported() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(!data.supported);
        assert_eq!(data.enabled, None);
        assert!(!data.writable);
    }

    #[test]
    fn invalid_content_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "maybe\n");
        let mut runtime = FnLockRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::default());
        assert!(data.supported);
        assert_eq!(data.enabled, None);
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_string(&FnLockData {
            supported: true,
            enabled: Some(false),
            writable: true,
        })
        .expect("json");
        assert!(json.contains("\"supported\":true"));
        assert!(json.contains("\"enabled\":false"));
        assert!(json.contains("\"writable\":true"));
    }
}
