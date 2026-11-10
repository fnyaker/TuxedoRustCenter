// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Opérations de haut niveau (profils, réglages) au-dessus de [`Bus`].
//!
//! Toute écriture persistante passe par `tccd-admin` ; polkit tranche côté
//! service. Les identifiants de profils sont immuables et toujours générés.

use anyhow::{Context, Result, bail};
use tcc_core::{Profile, ProfileCollection, ProfileId, Settings};

use crate::bus::{Bus, Target};
use crate::ops::{apply_path, editable, resolve_profile};

/// Client du daemon et des services d'administration.
pub struct Client {
    /// Connexion D-Bus sous-jacente (pour les appels bruts).
    pub bus: Bus,
}

impl Client {
    /// `dev_session` : daemon de développement sur le bus de session.
    #[must_use]
    pub fn new(dev_session: bool) -> Self {
        Self {
            bus: Bus::new(dev_session),
        }
    }

    /// Appel sans argument renvoyant du JSON brut.
    ///
    /// # Errors
    ///
    /// Erreur D-Bus (daemon absent, appel refusé).
    pub async fn json(&mut self, target: Target, method: &str) -> Result<String> {
        self.bus.call(target, method, &()).await
    }

    /// Charge la collection de profils (presets inclus).
    ///
    /// # Errors
    ///
    /// Erreur D-Bus ou JSON invalide.
    pub async fn load_profiles(&mut self) -> Result<ProfileCollection> {
        let raw = self.json(Target::Daemon, "GetProfilesJSON").await?;
        serde_json::from_str(&raw).context("collection de profils invalide")
    }

    /// Charge les réglages globaux.
    ///
    /// # Errors
    ///
    /// Erreur D-Bus ou JSON invalide.
    pub async fn load_settings(&mut self) -> Result<Settings> {
        let raw = self.json(Target::Daemon, "GetSettingsJSON").await?;
        serde_json::from_str(&raw).context("réglages invalides")
    }

    /// Valide puis persiste la collection via `tccd-admin`.
    ///
    /// # Errors
    ///
    /// Validation, refus polkit ou erreur du service.
    pub async fn save_profiles(&mut self, collection: &ProfileCollection) -> Result<()> {
        collection
            .validate()
            .context("collection de profils invalide")?;
        let payload = serde_json::to_string(collection)?;
        let ok: bool = self
            .bus
            .call(Target::Admin, "SaveProfilesJSON", &payload.as_str())
            .await?;
        if !ok {
            bail!("SaveProfilesJSON a refusé l'écriture");
        }
        Ok(())
    }

    /// Valide puis persiste les réglages via `tccd-admin`.
    ///
    /// # Errors
    ///
    /// Validation, refus polkit ou erreur du service.
    pub async fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        settings.validate().context("réglages invalides")?;
        let payload = serde_json::to_string(settings)?;
        let ok: bool = self
            .bus
            .call(Target::Admin, "SaveSettingsJSON", &payload.as_str())
            .await?;
        if !ok {
            bail!("SaveSettingsJSON a refusé l'écriture");
        }
        Ok(())
    }

    /// Active un profil (id ou nom) jusqu'au prochain changement d'état.
    ///
    /// # Errors
    ///
    /// Profil inconnu ou appel refusé.
    pub async fn use_profile(&mut self, query: &str) -> Result<String> {
        let collection = self.load_profiles().await?;
        let id = resolve_profile(&collection, query)?.id.as_str().to_owned();
        let ok: bool = self
            .bus
            .call(Target::Daemon, "SetTempProfile", &id.as_str())
            .await?;
        if !ok {
            bail!("SetTempProfile a refusé le profil {id}");
        }
        Ok(id)
    }

    /// Supprime l'override temporaire.
    ///
    /// # Errors
    ///
    /// Appel refusé.
    pub async fn clear_temp(&mut self) -> Result<()> {
        let _: bool = self
            .bus
            .call(Target::Daemon, "ClearTempProfile", &())
            .await?;
        Ok(())
    }

    /// Crée un profil par défaut ; renvoie son id.
    ///
    /// # Errors
    ///
    /// Validation ou écriture refusée.
    pub async fn create_profile(&mut self, name: String) -> Result<String> {
        let mut collection = self.load_profiles().await?;
        let profile = Profile::new(ProfileId::generate(), name);
        let id = profile.id.as_str().to_owned();
        collection.upsert(profile);
        self.save_profiles(&collection).await?;
        Ok(id)
    }

    /// Copie un profil (preset inclus) sous un nouvel id ; renvoie l'id.
    ///
    /// # Errors
    ///
    /// Profil inconnu ou écriture refusée.
    pub async fn copy_profile(&mut self, from: &str, name: String) -> Result<String> {
        let mut collection = self.load_profiles().await?;
        let mut copy = resolve_profile(&collection, from)?.clone();
        copy.id = ProfileId::generate();
        copy.name = name;
        let id = copy.id.as_str().to_owned();
        collection.upsert(copy);
        self.save_profiles(&collection).await?;
        Ok(id)
    }

    /// Importe un profil avec un nouvel id ; renvoie l'id.
    ///
    /// # Errors
    ///
    /// Profil invalide ou écriture refusée.
    pub async fn import_profile(&mut self, mut profile: Profile) -> Result<String> {
        // Toujours un nouvel identifiant : pas de collision ni de tombstone.
        profile.id = ProfileId::generate();
        profile.validate().context("profil importé invalide")?;
        let id = profile.id.as_str().to_owned();
        let mut collection = self.load_profiles().await?;
        collection.upsert(profile);
        self.save_profiles(&collection).await?;
        Ok(id)
    }

    /// Supprime un profil personnalisé ; renvoie son id.
    ///
    /// # Errors
    ///
    /// Preset, profil inconnu ou écriture refusée.
    pub async fn delete_profile(&mut self, query: &str) -> Result<String> {
        let mut collection = self.load_profiles().await?;
        let id = editable(&mut collection, query)?.id.clone();
        collection.remove(&id);
        self.save_profiles(&collection).await?;
        Ok(id.as_str().to_owned())
    }

    /// Renomme un profil personnalisé.
    ///
    /// # Errors
    ///
    /// Preset, profil inconnu ou écriture refusée.
    pub async fn rename_profile(&mut self, query: &str, name: String) -> Result<()> {
        let mut collection = self.load_profiles().await?;
        editable(&mut collection, query)?.name = name;
        self.save_profiles(&collection).await
    }

    /// Modifie un champ d'un profil personnalisé (chemin pointé, valeur JSON
    /// ou texte).
    ///
    /// # Errors
    ///
    /// Clé inconnue, valeur rejetée, preset ou écriture refusée.
    pub async fn set_profile_field(&mut self, query: &str, key: &str, raw: &str) -> Result<()> {
        if key == "id" || key.starts_with("id.") {
            bail!("l'identifiant d'un profil est immuable");
        }
        let mut collection = self.load_profiles().await?;
        let target = editable(&mut collection, query)?;
        let updated = apply_path(target, key, raw, |doc| {
            serde_json::from_value::<Profile>(doc).context("profil invalide")
        })?;
        updated.validate().context("profil invalide")?;
        if updated.extra.len() > target.extra.len() {
            bail!("clé inconnue : {key}");
        }
        *target = updated;
        self.save_profiles(&collection).await
    }

    /// Modifie un réglage global (chemin pointé, valeur JSON ou texte).
    ///
    /// # Errors
    ///
    /// Clé inconnue, valeur rejetée ou écriture refusée.
    pub async fn set_setting(&mut self, key: &str, raw: &str) -> Result<()> {
        if key.starts_with("deletedProfileIds") {
            bail!("les tombstones de profils ne se modifient pas directement");
        }
        let settings = self.load_settings().await?;
        let updated = apply_path(&settings, key, raw, |doc| {
            serde_json::from_value::<Settings>(doc).context("réglages invalides")
        })?;
        if updated.extra.len() > settings.extra.len() {
            bail!("clé inconnue : {key}");
        }
        self.save_settings(&updated).await
    }
}
