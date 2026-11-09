// SPDX-License-Identifier: GPL-3.0-or-later
//! Fn-Lock : `/sys/devices/platform/tuxedo_keyboard/fn_lock`.
//!
//! Source : `FnLockController` de l'original. Le fichier est un booléen sysfs
//! (`0`/`1`). Son absence signifie « non supporté », jamais une erreur.
//!
//! Aucun autre chemin n'est utilisé : contrairement à certaines
//! implémentations, le module ne tente pas de déduire l'état depuis le
//! contrôleur clavier ACPI ou `hid-generic`.

use crate::error::Result;
use crate::sysfs::SysfsRoot;

/// Chemin de l'attribut Fn-Lock.
pub const FN_LOCK_PATH: &str = "devices/platform/tuxedo_keyboard/fn_lock";

/// Contrôleur Fn-Lock.
#[derive(Debug, Clone)]
pub struct FnLock {
    root: SysfsRoot,
}

impl FnLock {
    /// Crée le contrôleur (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot) -> Self {
        Self { root }
    }

    /// Vrai si le matériel expose `fn_lock`.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.root.is_file(FN_LOCK_PATH)
    }

    /// Lit l'état courant.
    ///
    /// Retourne `Ok(None)` si l'attribut est absent (non supporté) : le daemon
    /// expose alors `supported: false` sans erreur.
    ///
    /// # Errors
    ///
    /// Erreur d'E/S réelle ou contenu non booléen.
    pub fn status(&self) -> Result<Option<bool>> {
        if !self.is_available() {
            return Ok(None);
        }
        self.root.read_bool(FN_LOCK_PATH).map(Some)
    }

    /// Active ou désactive le Fn-Lock.
    ///
    /// # Errors
    ///
    /// - [`crate::error::HwError::NotSupported`] si l'attribut est absent ;
    /// - erreur d'E/S en cas d'échec d'écriture.
    pub fn set(&self, enabled: bool) -> Result<()> {
        if !self.is_available() {
            return Err(crate::error::HwError::NotSupported("fn_lock"));
        }
        self.root.write_bool(FN_LOCK_PATH, enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(root: &Path, contents: &str) {
        let path = root.join(FN_LOCK_PATH);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn reads_and_writes_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1\n");
        let fn_lock = FnLock::new(SysfsRoot::new(dir.path()));
        assert!(fn_lock.is_available());
        assert_eq!(fn_lock.status().unwrap(), Some(true));
        fn_lock.set(false).unwrap();
        assert_eq!(fn_lock.status().unwrap(), Some(false));
        assert_eq!(
            fs::read_to_string(dir.path().join(FN_LOCK_PATH)).unwrap(),
            "0"
        );
    }

    #[test]
    fn absent_attribute_is_not_supported() {
        let dir = tempfile::tempdir().unwrap();
        let fn_lock = FnLock::new(SysfsRoot::new(dir.path()));
        assert!(!fn_lock.is_available());
        assert_eq!(fn_lock.status().unwrap(), None);
        assert!(matches!(
            fn_lock.set(true),
            Err(crate::error::HwError::NotSupported(_))
        ));
    }

    #[test]
    fn invalid_content_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "maybe\n");
        let fn_lock = FnLock::new(SysfsRoot::new(dir.path()));
        assert!(matches!(
            fn_lock.status(),
            Err(crate::error::HwError::InvalidValue { .. })
        ));
    }
}
