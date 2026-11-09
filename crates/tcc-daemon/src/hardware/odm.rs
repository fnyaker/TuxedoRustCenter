// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Worker ODM (`platform_profile`) et TDP.
//!
//! Reprend `ODMProfileWorker` et `ODMPowerLimitWorker` :
//!
//! 1. ordre de préférence `tuxedo_platform_profile` > ACPI
//!    `platform_profile` > fallback `tuxedo-io` ;
//! 2. quirk « pas d'ACPI platform_profile » sur IBP G10 AMD, IBM15A10 et
//!    IBM16A10 ;
//! 3. TDP PL1/PL2/PL4 sur Uniwill, valeurs vides ⇒ maxima du firmware ;
//! 4. application au démarrage et à chaque changement de profil/réglages, pas
//!    à chaque tick (un tick ne fait que rafraîchir l'état).
//!
//! # PPD (D4)
//!
//! Quand `ppd_active` est vrai et que l'override explicite
//! (`HardwareInputs.ppd_override`) est faux, `platform_profile` n'est **jamais**
//! écrit : l'état expose `owner: "ppd"` et un log unique explique la décision.
//! C'est une différence documentée avec l'original, qui ne connaissait pas PPD.
//!
//! # Fallback `tuxedo-io`
//!
//! Si aucun `platform_profile` n'est exploitable, le worker applique le profil
//! ODM via les ioctls Clevo/Uniwill. Le profil demandé doit appartenir à la
//! liste exposée : un nom absent est refusé sans écriture (erreur visible).
//! Un profil sans ODM (`name = None`) ne touche pas à l'état firmware. Le
//! profil par défaut n'est **pas** appliqué implicitement ; il reste exposé
//! dans `io.defaultProfile` pour l'UI.
//!
//! Ce fallback reste piloté par TCC même quand PPD est actif : PPD ne connaît
//! que `platform_profile` et l'EPP, pas les profils ODM du module
//! `tuxedo_io`. Il n'y a donc pas de conflit d'écriture.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tcc_hw::device::TuxedoDevice;
use tcc_hw::error::HwError;
use tcc_hw::platform_profile::PlatformProfile;
use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::tdp::{TdpControl, TdpLimit};
use tcc_hw::tuxedo_io::TuxedoIo;

use super::{HardwareInputs, HardwareStatus, WorkerContext, spawn_worker};

/// Période de rafraîchissement (le TDP était relu toutes les 5 s dans
/// l'original).
pub const ODM_TICK: Duration = Duration::from_secs(5);
/// Nombre de ticks entre deux tentatives d'ouverture de `/dev/tuxedo_io`.
const IO_RETRY_TICKS: u32 = 6;

/// État du profil de plateforme.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlatformProfileData {
    pub available: bool,
    /// TCC peut écrire ce profil (matériel présent, `owner == "tcc"`, hors
    /// `dry_run`). Faux quand PPD est propriétaire (D4).
    pub writable: bool,
    /// `tuxedo_platform_profile` ou `acpi_platform_profile`.
    pub source: Option<String>,
    pub current: Option<String>,
    pub choices: Vec<String>,
    pub default_profile: Option<String>,
    /// Vrai si l'ACPI a été ignoré à cause d'un quirk de modèle.
    pub acpi_quirk_blocked: bool,
    /// `ppd` ou `tcc`.
    pub owner: String,
    /// Profil appliqué par la dernière réconciliation.
    pub applied: Option<String>,
}

/// Une limite TDP exposée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TdpData {
    /// `pl1`, `pl2`, `pl4`.
    pub descriptor: String,
    pub current: i32,
    pub min: i32,
    pub max: i32,
}

impl From<&TdpLimit> for TdpData {
    fn from(limit: &TdpLimit) -> Self {
        Self {
            descriptor: limit.descriptor.to_owned(),
            current: limit.current,
            min: limit.min,
            max: limit.max,
        }
    }
}

/// Capacités du chemin `tuxedo-io` (fallback quand `platform_profile` est
/// absent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IoData {
    /// `/dev/tuxedo_io` ouvert et interface reconnue.
    pub available: bool,
    /// Interface détectée (`clevo`, `uniwill`).
    pub interface: Option<String>,
    /// Version du module noyau.
    pub module_version: Option<String>,
    /// Profils ODM réellement exposés par l'ioctl.
    pub odm_available: bool,
    /// Profils ODM (`power_save`, `enthusiast`, `overboost`, ...).
    pub choices: Vec<String>,
    /// Profil ODM par défaut calculé par le firmware.
    pub default_profile: Option<String>,
    /// Au moins une limite TDP (PL1/PL2/PL4) est exposée.
    pub tdp_available: bool,
}

/// Instantané ODM/TDP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OdmData {
    /// Un réglage ODM ou TDP est exposé, que ce soit via `platform_profile`
    /// **ou** via le chemin `tuxedo-io` (ODM/TDP).
    pub available: bool,
    /// Un profil ODM est pilotable (platform_profile TCC ou fallback
    /// `tuxedo-io`), hors `dry_run`.
    pub odm_writable: bool,
    /// Au moins une limite TDP est pilotable, hors `dry_run`.
    pub tdp_writable: bool,
    pub platform_profile: PlatformProfileData,
    /// Capacités du chemin `tuxedo-io` (source de vérité pour ODM/TDP quand
    /// `platform_profile` est absent).
    pub io: IoData,
    /// Vrai si le firmware expose au moins une limite TDP.
    pub tdp_available: bool,
    pub tdp: Vec<TdpData>,
    /// Interface `tuxedo-io` détectée (`clevo`, `uniwill`).
    pub interface: Option<String>,
    /// Version du module noyau.
    pub module_version: Option<String>,
    /// Vrai si la dernière réconciliation a réussi.
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Runtime ODM/TDP.
#[derive(Debug)]
pub struct OdmRuntime {
    root: SysfsRoot,
    dry_run: bool,
    io: Option<Arc<TuxedoIo>>,
    device: Option<TuxedoDevice>,
    io_retry: u32,
    last_revision: Option<u64>,
    applied_revision: Option<u64>,
    ppd_skip_logged: bool,
    odm_applied: Option<(u64, String)>,
    platform_applied: Option<(u64, String)>,
    tdp_applied_revision: Option<u64>,
    odm_warned: Option<String>,
    platform_warned: Option<(u64, String)>,
    tdp_warned: Option<(u64, String)>,
}

impl OdmRuntime {
    /// Construit le runtime (aucun accès disque).
    #[must_use]
    pub fn new(root: SysfsRoot, dry_run: bool) -> Self {
        Self {
            root,
            dry_run,
            io: None,
            device: None,
            io_retry: 0,
            last_revision: None,
            applied_revision: None,
            ppd_skip_logged: false,
            odm_applied: None,
            platform_applied: None,
            tdp_applied_revision: None,
            odm_warned: None,
            platform_warned: None,
            tdp_warned: None,
        }
    }

    /// Exécute un tick.
    pub fn tick(&mut self, input: &HardwareInputs) -> OdmData {
        self.ensure_io();
        let device = self.ensure_device();
        let platform = PlatformProfile::detect(&self.root, device);
        let mut errors = Vec::new();

        let platform_profile = match platform {
            Ok(profile) => {
                let owner = if input.platform_profile_owner()
                    == tcc_hw::platform_profile::ProfileOwnership::PpdOwned
                {
                    "ppd".to_owned()
                } else {
                    "tcc".to_owned()
                };
                let data = PlatformProfileData {
                    available: profile.is_available(),
                    writable: profile.is_available() && owner == "tcc" && !self.dry_run,
                    source: profile.source().map(|source| source.as_str().to_owned()),
                    current: profile.current().map(str::to_owned),
                    choices: profile.choices().to_vec(),
                    default_profile: profile.default_profile().map(str::to_owned),
                    acpi_quirk_blocked: profile.acpi_quirk_blocked(),
                    owner,
                    applied: None,
                };
                if input.ppd_active && !input.ppd_override && !self.ppd_skip_logged {
                    self.ppd_skip_logged = true;
                    tracing::info!(
                        "power-profiles-daemon est propriétaire de platform_profile (D4) ; \
                         écriture TCC désactivée"
                    );
                }
                Some((profile, data))
            }
            Err(error) => {
                errors.push(format!("platform_profile: {error}"));
                None
            }
        };

        let mut data = OdmData {
            platform_profile: platform_profile
                .as_ref()
                .map(|(_, data)| data.clone())
                .unwrap_or_default(),
            errors: errors.clone(),
            ..OdmData::default()
        };

        if let Some(io) = &self.io {
            data.io.available = true;
            data.io.interface = Some(io.interface().as_str().to_owned());
            data.io.module_version = io.module_version().map(str::to_owned);
            data.interface = data.io.interface.clone();
            data.module_version = data.io.module_version.clone();

            match io.odm_profiles() {
                Ok(choices) => {
                    data.io.odm_available = !choices.is_empty();
                    if data.io.odm_available {
                        data.io.default_profile = io.default_odm_profile().ok().flatten();
                    }
                    data.io.choices = choices;
                }
                Err(error) if error.is_unavailable() => {}
                Err(error) => errors.push(format!("ODM tuxedo-io: {error}")),
            }

            let tdp = TdpControl::new(Arc::clone(io));
            match tdp.limits() {
                Ok(limits) => {
                    data.tdp_available = !limits.is_empty();
                    data.io.tdp_available = data.tdp_available;
                    data.tdp = limits.iter().map(TdpData::from).collect();
                    data.tdp_writable = !self.dry_run && data.tdp_available;
                }
                Err(error) if error.is_unavailable() => {}
                Err(error) => errors.push(format!("TDP: {error}")),
            }
        }
        // `available` doit refléter les deux chemins : platform_profile ou
        // tuxedo-io (ODM/TDP). La GUI ne doit pas masquer ODM quand seul
        // `tuxedo-io` expose des profils (InfinityBook AMD Gen9, par ex.).
        data.available =
            data.platform_profile.available || data.io.odm_available || data.io.tdp_available;
        // Le fallback tuxedo-io n'est utilisé que si aucun platform_profile
        // exploitable n'existe (même logique que ODMProfileWorker).
        let io_fallback_writable = !data.platform_profile.available && data.io.odm_available;
        data.odm_writable =
            !self.dry_run && (data.platform_profile.writable || io_fallback_writable);

        let revision_changed = self.last_revision != Some(input.revision);
        if revision_changed {
            self.last_revision = Some(input.revision);
            self.applied_revision = None;
        }
        if self.dry_run
            || (self.applied_revision == self.last_revision && self.last_revision.is_some())
        {
            data.applied = errors.is_empty();
            data.errors = errors;
            return data;
        }

        if let Some((profile, platform_data)) = &platform_profile {
            data.platform_profile.applied =
                self.apply_platform(profile, platform_data.available, input, &mut errors);
        }
        if let Some(io) = self.io.clone() {
            self.apply_odm_fallback(&io, input, data.platform_profile.available, &mut errors);
            self.apply_tdp(&io, input, &mut errors);
        }
        // Relit l'état réel après écriture : le profil courant et les limites
        // TDP peuvent avoir été normalisés par le firmware.
        if let Ok(refreshed) = PlatformProfile::detect(&self.root, device) {
            data.platform_profile.current = refreshed.current().map(str::to_owned);
            data.platform_profile.choices = refreshed.choices().to_vec();
            data.platform_profile.source =
                refreshed.source().map(|source| source.as_str().to_owned());
        }
        if let Some(io) = &self.io
            && let Ok(limits) = TdpControl::new(Arc::clone(io)).limits()
        {
            data.tdp = limits.iter().map(TdpData::from).collect();
            data.tdp_available = !data.tdp.is_empty();
            data.tdp_writable = !self.dry_run && data.tdp_available;
        }
        data.errors = errors;
        data.applied = data.errors.is_empty();
        if data.applied {
            self.applied_revision = self.last_revision;
        }
        data
    }

    fn apply_platform(
        &mut self,
        profile: &PlatformProfile,
        available: bool,
        input: &HardwareInputs,
        errors: &mut Vec<String>,
    ) -> Option<String> {
        if !available {
            return None;
        }
        let name = input
            .profile
            .as_ref()
            .and_then(|profile| profile.odm_profile.name.clone())?;
        if self
            .platform_applied
            .as_ref()
            .is_some_and(|(revision, applied)| *revision == input.revision && applied == &name)
        {
            tracing::debug!(
                profile = %name,
                source = "platform_profile",
                "profil ODM déjà appliqué pour cette révision"
            );
            return Some(name);
        }
        match profile.apply(&self.root, &name, input.platform_profile_owner()) {
            Ok(()) => {
                self.platform_applied = Some((input.revision, name.clone()));
                self.platform_warned = None;
                tracing::info!(
                    profile = %name,
                    source = "platform_profile",
                    "profil ODM appliqué"
                );
                Some(name)
            }
            Err(HwError::PolicyDenied { .. }) => {
                // PPD propriétaire : comportement attendu, pas une erreur.
                tracing::debug!(profile = %name, "profil ODM non écrit : PPD propriétaire");
                None
            }
            Err(HwError::NotSupported(_)) => {
                tracing::debug!(profile = %name, "profil ODM non supporté par platform_profile");
                None
            }
            Err(error) => {
                errors.push(format!("profil ODM '{name}': {error}"));
                let key = (input.revision, name.clone());
                if self.platform_warned.as_ref() != Some(&key) {
                    self.platform_warned = Some(key);
                    tracing::warn!(
                        profile = %name,
                        %error,
                        source = "platform_profile",
                        "échec d'application du profil ODM"
                    );
                }
                None
            }
        }
    }

    fn apply_odm_fallback(
        &mut self,
        io: &Arc<TuxedoIo>,
        input: &HardwareInputs,
        platform_available: bool,
        errors: &mut Vec<String>,
    ) {
        if platform_available {
            return;
        }
        let available = match io.odm_profiles() {
            Ok(profiles) => profiles,
            Err(error) => {
                if !error.is_unavailable() {
                    errors.push(format!("profils ODM: {error}"));
                }
                return;
            }
        };
        if available.is_empty() {
            return;
        }
        let requested = input
            .profile
            .as_ref()
            .and_then(|profile| profile.odm_profile.name.clone());
        let chosen = match requested {
            Some(name) => {
                if !available.contains(&name) {
                    if self.odm_warned.as_deref() != Some(name.as_str()) {
                        self.odm_warned = Some(name.clone());
                        tracing::warn!(
                            profile = %name,
                            choices = ?available,
                            source = "tuxedo-io",
                            "profil ODM demandé absent de l'allowlist : aucune écriture"
                        );
                    }
                    errors.push(format!(
                        "profil ODM '{name}': non supporté (choix: {available:?})"
                    ));
                    return;
                }
                name
            }
            None => return,
        };
        // `tcc-hw` ne lit pas le profil ODM courant : un no-op firmware n'est
        // pas distinguable d'une écriture. Le suivi par révision évite les
        // doublons lors d'un retry après échec partiel (TDP, par exemple).
        if self
            .odm_applied
            .as_ref()
            .is_some_and(|(revision, applied)| *revision == input.revision && applied == &chosen)
        {
            tracing::debug!(
                profile = %chosen,
                source = "tuxedo-io",
                "profil ODM déjà appliqué pour cette révision"
            );
            return;
        }
        match io.set_odm_profile(&chosen) {
            Ok(()) => {
                self.odm_applied = Some((input.revision, chosen.clone()));
                self.odm_warned = None;
                tracing::info!(
                    profile = %chosen,
                    source = "tuxedo-io",
                    "profil ODM appliqué"
                );
            }
            Err(error) => {
                errors.push(format!("profil ODM '{chosen}': {error}"));
                if self.odm_warned.as_deref() != Some(chosen.as_str()) {
                    self.odm_warned = Some(chosen.clone());
                    tracing::warn!(
                        profile = %chosen,
                        %error,
                        source = "tuxedo-io",
                        "échec d'application du profil ODM"
                    );
                }
            }
        }
    }

    fn apply_tdp(&mut self, io: &Arc<TuxedoIo>, input: &HardwareInputs, errors: &mut Vec<String>) {
        let values = input
            .profile
            .as_ref()
            .map(|profile| profile.odm_power_limits.tdp_values.clone())
            .unwrap_or_default();
        // Dans le worker de profil, une liste vide/absente signifie « ne pas
        // changer » (pas « maxima firmware »). Les maxima restent disponibles
        // via l'action explicite Admin.Hw.SetTdpValues([]).
        if values.is_empty() || self.tdp_applied_revision == Some(input.revision) {
            return;
        }
        let control = TdpControl::new(Arc::clone(io));
        match control.set_values(&values) {
            Ok(_) => {
                self.tdp_applied_revision = Some(input.revision);
                self.tdp_warned = None;
                tracing::info!(values = ?values, "limites TDP appliquées");
            }
            Err(HwError::NotSupported(_)) => {
                tracing::debug!(values = ?values, "TDP non supporté par le firmware");
            }
            Err(error) => {
                errors.push(format!("TDP: {error}"));
                let key = (input.revision, format!("{values:?}"));
                if self.tdp_warned.as_ref() != Some(&key) {
                    self.tdp_warned = Some(key);
                    tracing::warn!(values = ?values, %error, "échec d'application des limites TDP");
                }
            }
        }
    }

    fn ensure_io(&mut self) {
        if self.io.is_some() {
            return;
        }
        if self.io_retry > 0 {
            self.io_retry -= 1;
            return;
        }
        self.io_retry = IO_RETRY_TICKS;
        match TuxedoIo::open() {
            Ok(io) => {
                tracing::info!(
                    interface = io.interface().as_str(),
                    version = io.module_version().unwrap_or("inconnue"),
                    "tuxedo-io ouvert pour ODM/TDP"
                );
                self.io = Some(Arc::new(io));
            }
            Err(error) => {
                tracing::debug!(%error, "tuxedo-io indisponible pour ODM/TDP");
            }
        }
    }

    fn ensure_device(&mut self) -> TuxedoDevice {
        if let Some(device) = self.device {
            return device;
        }
        let model_id = self.io.as_ref().and_then(|io| io.model_id());
        let device = TuxedoDevice::detect(&self.root, model_id);
        self.device = Some(device);
        device
    }
}

/// Démarre le worker ODM/TDP.
pub(crate) fn spawn(
    ctx: WorkerContext,
    root: SysfsRoot,
    dry_run: bool,
) -> tokio::task::JoinHandle<()> {
    let runtime = OdmRuntime::new(root, dry_run);
    spawn_worker(
        "odm",
        ODM_TICK,
        ctx,
        runtime,
        None,
        HardwareStatus::set_odm,
        OdmRuntime::tick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tcc_core::presets::office_profile;
    use tcc_hw::tuxedo_io::fake::FakeTransport;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn profile_with_odm(name: &str) -> tcc_core::Profile {
        let mut profile = office_profile();
        profile.odm_profile.name = Some(name.to_owned());
        profile
    }

    fn runtime_with_io(root: &Path, dry_run: bool, transport: FakeTransport) -> OdmRuntime {
        runtime_with_io_observer(root, dry_run, transport).0
    }

    fn runtime_with_io_observer(
        root: &Path,
        dry_run: bool,
        transport: FakeTransport,
    ) -> (OdmRuntime, FakeTransport) {
        let observer = transport.clone();
        let io = TuxedoIo::with_transport(Box::new(transport)).expect("fake tuxedo_io");
        let mut runtime = OdmRuntime::new(SysfsRoot::new(root), dry_run);
        runtime.io = Some(Arc::new(io));
        (runtime, observer)
    }

    #[test]
    fn applies_platform_profile_when_tcc_owns_it() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let input = HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            false,
            1,
        );
        let data = runtime.tick(&input);
        assert!(data.available);
        assert!(data.platform_profile.available);
        assert_eq!(data.platform_profile.owner, "tcc");
        assert!(data.platform_profile.writable);
        assert!(data.odm_writable);
        assert_eq!(
            data.platform_profile.applied.as_deref(),
            Some("performance")
        );
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "performance"
        );
    }

    #[test]
    fn ppd_ownership_skips_write_and_reports_owner() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let input = HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            true,
            1,
        );
        let data = runtime.tick(&input);
        assert_eq!(data.platform_profile.owner, "ppd");
        assert!(!data.platform_profile.writable);
        assert!(!data.odm_writable);
        assert_eq!(data.platform_profile.applied, None);
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "balanced"
        );
        assert!(data.applied);
    }

    #[test]
    fn explicit_override_writes_under_ppd() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let mut input = HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            true,
            1,
        );
        input.ppd_override = true;
        let data = runtime.tick(&input);
        assert_eq!(data.platform_profile.owner, "tcc");
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "performance"
        );
    }

    #[test]
    fn acpi_is_skipped_for_quirk_models() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "class/dmi/id/product_sku", "IBM16A10\n");
        write(
            dir.path(),
            "firmware/acpi/platform_profile_choices",
            "low-power balanced performance\n",
        );
        write(dir.path(), "firmware/acpi/platform_profile", "balanced\n");
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let input = HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            false,
            1,
        );
        let data = runtime.tick(&input);
        assert!(!data.platform_profile.available);
        assert!(data.platform_profile.acpi_quirk_blocked);
        assert_eq!(
            read(dir.path(), "firmware/acpi/platform_profile"),
            "balanced"
        );
    }

    #[test]
    fn io_only_exposes_odm_choices_and_tdp() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = runtime_with_io(dir.path(), true, FakeTransport::uniwill());
        let data = runtime.tick(&HardwareInputs::default());

        assert!(data.available);
        assert!(!data.platform_profile.available);
        assert!(data.io.available);
        assert_eq!(data.io.interface.as_deref(), Some("uniwill"));
        assert_eq!(data.io.module_version.as_deref(), Some("0.2.6"));
        assert!(data.io.odm_available);
        assert_eq!(
            data.io.choices,
            vec!["power_save", "enthusiast", "overboost"]
        );
        assert_eq!(data.io.default_profile.as_deref(), Some("overboost"));
        assert!(data.io.tdp_available);
        assert!(data.tdp_available);
        assert_eq!(data.tdp.len(), 3);
        assert_eq!(data.tdp[0].descriptor, "pl1");
        assert_eq!(data.tdp[0].current, 35);
        assert_eq!(data.tdp[0].min, 10);
        assert_eq!(data.tdp[0].max, 80);
        assert_eq!(data.tdp[2].descriptor, "pl4");
        assert!(!data.odm_writable, "dry_run ne doit pas écrire");
        assert!(!data.tdp_writable, "dry_run ne doit pas écrire");
        assert!(data.applied);
        assert!(data.errors.is_empty());

        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"io\":{"));
        assert!(json.contains("\"odmAvailable\":true"));
        assert!(json.contains("\"choices\":[\"power_save\",\"enthusiast\",\"overboost\"]"));
        assert!(json.contains("\"defaultProfile\":\"overboost\""));
        assert!(json.contains("\"tdpAvailable\":true"));
    }

    #[test]
    fn io_odm_without_tdp_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let transport = FakeTransport::uniwill()
            .with_version("0.3.9")
            .with_tdp_limits([-1, -1, -1], [0, 0, 0]);
        let mut runtime = runtime_with_io(dir.path(), true, transport);
        let data = runtime.tick(&HardwareInputs::default());

        assert!(data.available, "ODM via tuxedo-io doit suffire");
        assert!(!data.platform_profile.available);
        assert!(data.io.available);
        assert_eq!(data.io.module_version.as_deref(), Some("0.3.9"));
        assert!(data.io.odm_available);
        assert_eq!(
            data.io.choices,
            vec!["power_save", "enthusiast", "overboost"]
        );
        assert!(!data.io.tdp_available);
        assert!(!data.tdp_available);
        assert!(data.tdp.is_empty());
    }

    #[test]
    fn io_only_is_writable_outside_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = runtime_with_io(dir.path(), false, FakeTransport::uniwill());
        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("enthusiast")),
            false,
            1,
        ));
        assert!(data.available);
        assert!(data.io.odm_available);
        assert!(data.odm_writable);
        assert!(data.tdp_available);
        assert!(data.tdp_writable);
        assert!(data.applied, "{data:?}");
    }

    #[test]
    fn ppd_owned_platform_does_not_use_io_odm_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = runtime_with_io(dir.path(), false, FakeTransport::uniwill());
        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            true,
            1,
        ));
        assert!(data.platform_profile.available);
        assert!(!data.platform_profile.writable);
        assert!(data.io.odm_available);
        assert!(
            !data.odm_writable,
            "le fallback tuxedo-io ne s'applique pas quand platform_profile existe"
        );
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "balanced"
        );
    }

    #[test]
    fn io_without_odm_or_tdp_is_not_a_capability() {
        let dir = tempfile::tempdir().unwrap();
        let transport = FakeTransport::uniwill()
            .with_profiles(0)
            .with_tdp_limits([-1, -1, -1], [0, 0, 0]);
        let mut runtime = runtime_with_io(dir.path(), true, transport);
        let data = runtime.tick(&HardwareInputs::default());

        assert!(!data.available);
        assert!(data.io.available, "l'interface reste ouverte");
        assert!(!data.io.odm_available);
        assert!(data.io.choices.is_empty());
        assert_eq!(data.io.default_profile, None);
        assert!(!data.io.tdp_available);
        assert!(!data.odm_writable);
        assert!(!data.tdp_writable);
        assert!(data.errors.is_empty());
    }

    #[test]
    fn invalid_odm_name_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let input = HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("turbo")),
            false,
            1,
        );
        let data = runtime.tick(&input);
        assert!(!data.applied);
        assert_eq!(data.errors.len(), 1);
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "balanced"
        );
    }

    #[test]
    fn unknown_io_odm_name_is_rejected_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut runtime, observer) =
            runtime_with_io_observer(dir.path(), false, FakeTransport::uniwill());
        observer.set_odm_profile_value(0x01);
        let mut profile = office_profile();
        profile.odm_profile.name = Some("balanced".to_owned());

        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            1,
        ));
        assert!(!data.applied);
        assert!(
            data.errors
                .iter()
                .any(|error| error.contains("non supporté"))
        );
        assert_eq!(
            observer.odm_profile_value(),
            0x01,
            "aucune écriture pour un nom absent de l'allowlist"
        );
    }

    #[test]
    fn profile_without_odm_does_not_touch_firmware() {
        let dir = tempfile::tempdir().unwrap();
        let (mut runtime, observer) =
            runtime_with_io_observer(dir.path(), false, FakeTransport::uniwill());
        observer.set_odm_profile_value(0x01);
        let mut profile = office_profile();
        profile.odm_profile.name = None;

        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            1,
        ));
        assert!(data.applied, "{data:?}");
        assert!(data.errors.is_empty());
        assert_eq!(observer.odm_profile_value(), 0x01);
    }

    #[test]
    fn valid_io_odm_name_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let (mut runtime, observer) =
            runtime_with_io_observer(dir.path(), false, FakeTransport::uniwill());
        observer.set_odm_profile_value(0x01);
        let mut profile = office_profile();
        profile.odm_profile.name = Some("enthusiast".to_owned());

        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            1,
        ));
        assert!(data.applied, "{data:?}");
        assert!(data.errors.is_empty());
        assert_eq!(observer.odm_profile_value(), 0x02);
    }

    #[test]
    fn empty_profile_tdp_does_not_write_maxima() {
        let dir = tempfile::tempdir().unwrap();
        let (mut runtime, observer) =
            runtime_with_io_observer(dir.path(), false, FakeTransport::uniwill());
        let mut profile = office_profile();
        profile.odm_profile.name = Some("enthusiast".to_owned());
        profile.odm_power_limits.tdp_values = Vec::new();

        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            1,
        ));
        assert!(data.applied, "{data:?}");
        assert!(data.errors.is_empty());
        assert_eq!(observer.tdp_current(0), Some(35));
        assert_eq!(observer.tdp_current(1), Some(45));
        assert_eq!(observer.tdp_current(2), Some(60));
    }

    #[test]
    fn non_empty_profile_tdp_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let (mut runtime, observer) =
            runtime_with_io_observer(dir.path(), false, FakeTransport::uniwill());
        let mut profile = office_profile();
        profile.odm_profile.name = Some("enthusiast".to_owned());
        profile.odm_power_limits.tdp_values = vec![50.0, 60.0, 70.0];

        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile),
            false,
            1,
        ));
        assert!(data.applied, "{data:?}");
        assert_eq!(observer.tdp_current(0), Some(50));
        assert_eq!(observer.tdp_current(1), Some(60));
        assert_eq!(observer.tdp_current(2), Some(70));
    }

    #[test]
    fn no_hardware_is_available_false() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), false);
        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            false,
            1,
        ));
        assert!(!data.available);
        assert!(!data.platform_profile.available);
        assert!(!data.io.available);
        assert!(data.io.choices.is_empty());
        assert!(!data.tdp_available);
        assert!(!data.odm_writable);
        assert!(data.applied);
        assert!(data.errors.is_empty());
        assert_eq!(data.platform_profile.owner, "tcc");
    }

    #[test]
    fn dry_run_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile_choices",
            "power_save balanced performance\n",
        );
        write(
            dir.path(),
            "bus/platform/devices/tuxedo_platform_profile/platform_profile",
            "balanced\n",
        );
        let mut runtime = OdmRuntime::new(SysfsRoot::new(dir.path()), true);
        let data = runtime.tick(&HardwareInputs::new(
            tcc_core::Settings::default(),
            Some(profile_with_odm("performance")),
            false,
            1,
        ));
        assert!(data.applied);
        assert_eq!(
            read(
                dir.path(),
                "bus/platform/devices/tuxedo_platform_profile/platform_profile"
            ),
            "balanced"
        );
    }

    #[test]
    fn tdp_data_conversion() {
        let limit = TdpLimit {
            descriptor: "pl1",
            current: 35,
            min: 10,
            max: 80,
        };
        let data = TdpData::from(&limit);
        assert_eq!(data.descriptor, "pl1");
        assert_eq!(data.current, 35);
        let json = serde_json::to_string(&data).expect("json");
        assert!(json.contains("\"descriptor\":\"pl1\""));
    }
}
