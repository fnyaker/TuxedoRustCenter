// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! État runtime du daemon : profils chargés, entrées PPD/alimentation et
//! résolution du profil actif.

use std::time::Duration;

use tcc_core::{
    DEFAULT_CUSTOM_PROFILE_ID, FanTableSet, ObserveOutcome, PowerSource, PpdProfile, Profile,
    ProfileCollection, ProfileId, ResolvedState, Settings, StateInput, StateResolver,
};
use tcc_hw::power_supply::{PowerState, PowerSupplies};
use tcc_hw::sysfs::SysfsRoot;
use tcc_ipc::{ActiveProfile, StateSnapshot};

use crate::config::LoadedConfig;

/// Entrées courantes du moteur de bascule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// Profil actif de power-profiles-daemon, si disponible.
    pub ppd: Option<PpdProfile>,
    /// Source d'alimentation.
    pub power: PowerSource,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            ppd: None,
            power: PowerSource::Unknown,
        }
    }
}

impl Inputs {
    /// Lit uniquement l'alimentation depuis sysfs.
    #[must_use]
    pub fn from_sysfs(root: &SysfsRoot) -> Self {
        let power = match PowerSupplies::read(root) {
            Ok(supplies) => power_from_hw(supplies.power_state()),
            Err(error) => {
                tracing::debug!(%error, "lecture de l'alimentation sysfs impossible");
                PowerSource::Unknown
            }
        };
        Self { ppd: None, power }
    }

    /// Met à jour le profil PPD.
    pub fn set_ppd(&mut self, value: Option<&str>) -> bool {
        let ppd = PpdProfile::from_optional_dbus(value);
        if self.ppd == ppd {
            false
        } else {
            self.ppd = ppd;
            true
        }
    }

    /// Met à jour la source d'alimentation.
    pub fn set_power(&mut self, power: PowerSource) -> bool {
        if self.power == power {
            false
        } else {
            self.power = power;
            true
        }
    }
}

/// Conversion depuis la couche matérielle.
#[must_use]
pub fn power_from_hw(power: PowerState) -> PowerSource {
    match power {
        PowerState::Ac => PowerSource::Ac,
        PowerState::Battery => PowerSource::Battery,
        PowerState::Unknown => PowerSource::Unknown,
    }
}

/// État partagé par le service D-Bus et les watchers.
pub struct RuntimeState {
    /// Réglages globaux.
    pub settings: Settings,
    /// Profils disponibles (presets + custom).
    pub profiles: ProfileCollection,
    /// Courbes de ventilation.
    pub fan_tables: FanTableSet,
    /// Moteur de bascule.
    pub resolver: StateResolver,
    /// Dernier profil PPD observé.
    pub ppd: Option<PpdProfile>,
    /// Dernière source d'alimentation observée.
    pub power: PowerSource,
    /// Compteur monotone de révision.
    pub revision: u64,
}

impl RuntimeState {
    /// Construit l'état à partir de la configuration chargée.
    #[must_use]
    pub fn new(config: &LoadedConfig) -> Self {
        let profiles = config.profiles.clone();
        let mut settings = config.settings.clone();
        if settings.state_matrix.is_empty() {
            settings.with_suggested_matrix();
        }
        let matrix = settings.state_matrix.clone();
        let default_profile = default_profile_id(&profiles);
        let mut resolver = StateResolver::new(matrix, settings.state_map.clone(), default_profile);
        resolver.set_known_profiles(Some(profiles.id_set()));
        Self {
            settings,
            profiles,
            fan_tables: config.fan_tables.clone(),
            resolver,
            ppd: None,
            power: PowerSource::Unknown,
            revision: 0,
        }
    }

    /// Remplace la configuration après un rechargement.
    pub fn reload(&mut self, config: &LoadedConfig) {
        self.settings = config.settings.clone();
        if self.settings.state_matrix.is_empty() {
            self.settings.with_suggested_matrix();
        }
        self.profiles = config.profiles.clone();
        self.fan_tables = config.fan_tables.clone();

        let matrix = self.settings.state_matrix.clone();
        self.resolver.set_matrix(matrix);
        self.resolver
            .set_legacy_state_map(self.settings.state_map.clone());
        self.resolver
            .set_default_profile(default_profile_id(&self.profiles));
        self.resolver
            .set_known_profiles(Some(self.profiles.id_set()));
        self.resolver.force_reapply();
    }

    /// Observe les entrées et renvoie le nouvel état s'il faut appliquer un
    /// profil. Le debounce est géré par [`StateResolver`] ; l'appelant doit
    /// appeler cette méthode régulièrement pendant une transition.
    pub fn observe(&mut self, inputs: &Inputs, now: Duration) -> Option<ResolvedState> {
        self.ppd.clone_from(&inputs.ppd);
        self.power = inputs.power;
        match self
            .resolver
            .observe(StateInput::new(inputs.ppd.clone(), inputs.power), now)
        {
            ObserveOutcome::Changed { current, .. } => {
                self.revision = self.revision.saturating_add(1);
                Some(current)
            }
            ObserveOutcome::Unchanged(_) | ObserveOutcome::Pending { .. } => None,
        }
    }

    /// Profil actuellement résolu.
    #[must_use]
    pub fn active_profile(&self) -> Option<&Profile> {
        let resolved = self.resolver.current()?;
        self.profiles.get(&resolved.profile_id)
    }

    /// Snapshot exposé aux clients D-Bus.
    #[must_use]
    pub fn snapshot(&self) -> StateSnapshot {
        let active = self.resolver.current().and_then(|resolved| {
            self.profiles
                .get(&resolved.profile_id)
                .map(|profile| ActiveProfile::from_profile(profile, resolved.source))
        });
        StateSnapshot::new(self.revision, self.ppd.clone(), self.power, active)
    }

    /// Active un profil temporaire pour la clé courante.
    ///
    /// # Errors
    ///
    /// Renvoie une erreur si le profil n'existe pas ou si aucun état n'a encore
    /// été observé.
    pub fn set_temp_profile(&mut self, id: &str) -> Result<Profile, String> {
        let profile_id = ProfileId::new(id);
        let profile = self
            .profiles
            .get(&profile_id)
            .cloned()
            .ok_or_else(|| format!("profil inconnu: {id}"))?;
        if !self.resolver.set_manual_override(profile_id) {
            return Err("aucun état actif : impossible de définir un profil temporaire".to_owned());
        }
        // Sans cela, `observe` verrait la même clé et renverrait `Unchanged` :
        // le profil manuel ne serait pas appliqué aux domaines CPU/affichage.
        self.resolver.force_reapply();
        self.revision = self.revision.saturating_add(1);
        Ok(profile)
    }

    /// Supprime l'override temporaire.
    pub fn clear_temp_profile(&mut self) -> bool {
        if self.resolver.clear_manual_override().is_some() {
            self.resolver.force_reapply();
            self.revision = self.revision.saturating_add(1);
            true
        } else {
            false
        }
    }
}

fn default_profile_id(profiles: &ProfileCollection) -> ProfileId {
    let default = ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID);
    if profiles.contains(&default) {
        return default;
    }
    profiles
        .iter()
        .next()
        .map(|profile| profile.id.clone())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_core::{FanTableSet, Settings, default_profiles};

    fn loaded() -> LoadedConfig {
        LoadedConfig {
            store: tcc_core::ConfigStore::with_admin_roots("/tmp/state", "/tmp/admin"),
            settings: Settings::default(),
            profiles: default_profiles(),
            fan_tables: FanTableSet::default(),
            warnings: Vec::new(),
            report_warnings: Vec::new(),
        }
    }

    #[test]
    fn resolves_suggested_matrix() {
        let config = loaded();
        let mut state = RuntimeState::new(&config);
        let inputs = Inputs {
            ppd: Some(PpdProfile::Balanced),
            power: PowerSource::Ac,
        };
        let changed = state.observe(&inputs, Duration::ZERO).unwrap();
        assert_eq!(changed.profile_id.as_str(), "__office__");
        assert_eq!(state.revision, 1);
        let snapshot = state.snapshot();
        assert_eq!(snapshot.contract_version, tcc_ipc::IPC_CONTRACT_VERSION);
        assert_eq!(
            snapshot.active_profile.as_ref().map(|p| p.id.as_str()),
            Some("__office__")
        );
    }

    #[test]
    fn manual_override_survives_until_state_change() {
        let config = loaded();
        let mut state = RuntimeState::new(&config);
        let inputs = Inputs {
            ppd: Some(PpdProfile::Balanced),
            power: PowerSource::Ac,
        };
        state.observe(&inputs, Duration::ZERO).unwrap();
        state.set_temp_profile("__profile_silent__").unwrap();
        assert_eq!(
            state.active_profile().map(|p| p.id.as_str()),
            Some("__profile_silent__")
        );

        // Le profil manuel est appliqué au prochain `observe` (force_reapply).
        let changed = state
            .observe(&inputs, Duration::from_secs(1))
            .expect("override à appliquer");
        assert_eq!(changed.profile_id.as_str(), "__profile_silent__");
        assert!(changed.is_manual_override());

        // Même état ensuite : l'override reste sans nouvelle application.
        assert!(state.observe(&inputs, Duration::from_secs(2)).is_none());

        // Changement d'alimentation : nouvel état après debounce.
        let battery = Inputs {
            ppd: Some(PpdProfile::Balanced),
            power: PowerSource::Battery,
        };
        assert!(state.observe(&battery, Duration::from_secs(3)).is_none());
        let changed = state
            .observe(&battery, Duration::from_secs(4))
            .expect("transition après debounce");
        assert_eq!(changed.profile_id.as_str(), "__profile_silent__");
        assert!(!changed.is_manual_override());
    }

    #[test]
    fn unknown_ppd_falls_back_to_power_map() {
        let config = loaded();
        let mut state = RuntimeState::new(&config);
        let inputs = Inputs {
            ppd: Some(PpdProfile::Unknown("oem".into())),
            power: PowerSource::Battery,
        };
        let changed = state.observe(&inputs, Duration::ZERO).unwrap();
        assert_eq!(changed.profile_id.as_str(), "__default_custom_profile__");
    }
}
