// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Chargement de la configuration du daemon.
//!
//! Le daemon utilise [`ConfigStore::system`] : état mutable dans
//! `/var/lib/tuxedo-rust-center`, configuration administrateur dans
//! `/etc/tuxedo-rust-center` en lecture seule (D2). En développement, la
//! feature `dev-overrides` permet d'injecter une racine d'état non root (D7).

use std::path::Path;

use anyhow::{Context, Result};
use tcc_core::{
    ConfigStore, FanTableSet, Loaded, ProfileCollection, Settings,
    config::DEFAULT_MAX_CONFIG_BYTES, merge_with_default_profiles, migration,
    validate_settings_references,
};

/// Configuration chargée et validée au démarrage.
pub struct LoadedConfig {
    /// Magasin de persistance (état mutable + admin).
    pub store: ConfigStore,
    /// Réglages globaux.
    pub settings: Settings,
    /// Profils fusionnés avec les presets.
    pub profiles: ProfileCollection,
    /// Courbes de ventilation.
    pub fan_tables: FanTableSet,
    /// Avertissements non bloquants, avec chemin (journal uniquement).
    pub warnings: Vec<String>,
    /// Avertissements **sanitizés** destinés à `Control1.ReloadConfig`.
    ///
    /// Ils ne contiennent ni chemin absolu, ni contenu de fichier, ni nom de
    /// profil : seulement la catégorie du problème. Un seul avertissement
    /// suffit à rendre le rapport `ok = false` (AD-6).
    pub report_warnings: Vec<String>,
}

/// Construit le magasin de production ou de développement.
fn build_store() -> Result<ConfigStore> {
    #[cfg(feature = "dev-overrides")]
    {
        if let Some(state_root) =
            tcc_core::config::dev_state_root().context("lecture de TCC_CONFIG_DIR")?
        {
            tracing::warn!(
                state_root = %state_root.display(),
                "override de développement actif : état hors /var/lib"
            );
            return Ok(ConfigStore::with_admin_roots(
                state_root,
                tcc_core::config::ADMIN_CONFIG_DIR,
            ));
        }
    }

    Ok(ConfigStore::system())
}

/// Charge les fichiers, migre si nécessaire et applique les valeurs par défaut.
///
/// # Errors
///
/// Renvoie une erreur si un fichier existant est illisible ou si la
/// configuration ne peut pas être réparée sans intervention.
pub fn load() -> Result<LoadedConfig> {
    let store = build_store()?;
    let mut warnings = Vec::new();
    let mut report_warnings = Vec::new();

    // Migration opportuniste depuis `/etc/tcc` si aucun état n'existe encore.
    // Un processus non root ne doit jamais tenter d'écrire dans `/var/lib` :
    // `--check` se contente de lire et d'afficher l'état.
    if crate::is_root()
        && !store.paths().settings.exists()
        && Path::new("/etc/tcc/settings").exists()
    {
        match migration::read_legacy_config(&migration::LegacyPaths::default_system()) {
            Ok(legacy) if !legacy.is_empty() => {
                warnings.push("migration depuis /etc/tcc détectée".to_owned());
                for note in &legacy.notes {
                    tracing::info!(note = %note, "migration legacy");
                }
                if let Some(settings) = legacy.settings {
                    store
                        .save_settings(&settings.value)
                        .context("écriture des settings migrés")?;
                }
                if let Some(profiles) = legacy.profiles {
                    store
                        .save_profiles(&profiles.value)
                        .context("écriture des profils migrés")?;
                }
                if let Some(fan_tables) = legacy.fan_tables {
                    store
                        .save_fan_tables(&fan_tables.value)
                        .context("écriture des courbes migrées")?;
                }
            }
            Ok(_) => {}
            Err(error) => warnings.push(format!("migration /etc/tcc impossible: {error}")),
        }
    }

    let settings_loaded = store.load_settings().context("chargement des settings")?;
    let profiles_loaded = store.load_profiles().context("chargement des profils")?;
    let fan_tables_loaded = store
        .load_fan_tables()
        .context("chargement des courbes de ventilation")?;

    collect_warning(
        "settings",
        &settings_loaded,
        &mut warnings,
        &mut report_warnings,
    );
    collect_warning(
        "profils",
        &profiles_loaded,
        &mut warnings,
        &mut report_warnings,
    );
    collect_warning(
        "courbes de ventilation",
        &fan_tables_loaded,
        &mut warnings,
        &mut report_warnings,
    );

    let mut settings = settings_loaded.value;
    let profiles = merge_with_default_profiles(profiles_loaded.value);
    let fan_tables = fan_tables_loaded.value;

    // Première installation : matrice PPD proposée par défaut.
    if settings.state_matrix.is_empty() {
        settings.with_suggested_matrix();
        if settings.schema_version != tcc_core::SETTINGS_SCHEMA_VERSION {
            settings.schema_version = tcc_core::SETTINGS_SCHEMA_VERSION;
        }
    }

    if let Err(error) = validate_settings_references(&settings, &profiles) {
        warnings.push(format!("références de profils invalides: {error}"));
        // Aucun nom de profil ni identifiant : seulement la catégorie.
        report_warnings.push("références de profils invalides".to_owned());
    }

    // Écrit les valeurs par défaut uniquement si les fichiers d'état manquent.
    // En `--check` non root, l'écriture échouerait : l'appelant décide.
    if !store.paths().settings.exists() {
        tracing::info!("settings absents, valeurs par défaut utilisées");
    }
    if !store.paths().profiles.exists() {
        tracing::info!("profils absents, presets utilisés");
    }

    Ok(LoadedConfig {
        store,
        settings,
        profiles,
        fan_tables,
        warnings,
        report_warnings,
    })
}

fn collect_warning<T>(
    label: &str,
    loaded: &Loaded<T>,
    warnings: &mut Vec<String>,
    report_warnings: &mut Vec<String>,
) {
    if let Some(warning) = loaded.warning() {
        // Journal : chemin et détail utiles au diagnostic root.
        warnings.push(format!(
            "{}: {} ({})",
            warning.path.display(),
            warning.kind.as_str(),
            warning.message
        ));
        // Rapport D-Bus : catégorie seulement, jamais de chemin ni de
        // contenu (AD-2/AD-6).
        report_warnings.push(format!("{label}: {}", warning.kind.as_str()));
    }
}

/// Persiste les valeurs par défaut si nécessaire et si le processus est root.
///
/// # Errors
///
/// Renvoie une erreur d'écriture réelle. En mode non root, l'écriture est
/// ignorée avec un avertissement.
pub fn persist_defaults_if_needed(config: &LoadedConfig) -> Result<()> {
    if !crate::is_root() {
        return Ok(());
    }
    let store = &config.store;
    if !store.paths().settings.exists() {
        store
            .save_settings(&config.settings)
            .context("écriture des settings par défaut")?;
    }
    if !store.paths().profiles.exists() {
        store
            .save_profiles(&config.profiles)
            .context("écriture des profils par défaut")?;
    }
    if !store.paths().fan_tables.exists() {
        store
            .save_fan_tables(&config.fan_tables)
            .context("écriture des courbes par défaut")?;
    }
    Ok(())
}

/// Vérifie qu'un fichier legacy reste lisible sans dépasser la limite.
#[allow(dead_code)]
fn legacy_size_ok(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() <= DEFAULT_MAX_CONFIG_BYTES)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tcc_core::{LoadWarning, LoadWarningKind, Loaded, Settings};

    use super::collect_warning;

    #[test]
    fn reload_report_omits_paths_and_details() {
        let loaded = Loaded {
            value: Settings::default(),
            warning: Some(LoadWarning {
                path: PathBuf::from("/var/lib/tuxedo-rust-center/settings.json"),
                kind: LoadWarningKind::InvalidJson,
                message: "ligne 1: contenu sensible".to_owned(),
                backup_path: Some(PathBuf::from("/var/lib/tuxedo-rust-center/x.bak")),
            }),
        };
        let mut logs = Vec::new();
        let mut report = Vec::new();
        collect_warning("settings", &loaded, &mut logs, &mut report);

        assert!(logs[0].contains("/var/lib/tuxedo-rust-center"));
        assert_eq!(report, vec!["settings: JSON invalide".to_owned()]);
        assert!(!report[0].contains('/'));
        assert!(!report[0].contains("sensible"));
        assert!(!report[0].contains("x.bak"));
    }

    #[test]
    fn clean_load_produces_no_report_warning() {
        let loaded: Loaded<Settings> = Loaded::clean(Settings::default());
        let mut logs = Vec::new();
        let mut report = Vec::new();
        collect_warning("settings", &loaded, &mut logs, &mut report);
        assert!(logs.is_empty());
        assert!(report.is_empty());
    }
}
