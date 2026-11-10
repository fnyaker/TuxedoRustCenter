// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Opérations sur profils partagées entre le CLI et les interfaces.

use anyhow::{Result, bail};
use serde_json::Value;
use tcc_core::{Profile, ProfileCollection};

use crate::jsonpath;

/// Identifiant ou nom (unique, insensible à la casse) → identifiant.
pub fn resolve_profile<'a>(collection: &'a ProfileCollection, query: &str) -> Result<&'a Profile> {
    if let Some(profile) = collection.get_by_id(query) {
        return Ok(profile);
    }
    let wanted = query.to_lowercase();
    let mut matches = collection
        .iter()
        .filter(|profile| profile.name.to_lowercase() == wanted);
    match (matches.next(), matches.next()) {
        (Some(profile), None) => Ok(profile),
        (Some(_), Some(_)) => bail!("« {query} » correspond à plusieurs profils : utilisez l'id"),
        _ => bail!("profil introuvable : {query}"),
    }
}

pub fn editable<'a>(collection: &'a mut ProfileCollection, query: &str) -> Result<&'a mut Profile> {
    let id = resolve_profile(collection, query)?.id.clone();
    if id.is_builtin_preset() || collection.get(&id).is_some_and(Profile::is_preset) {
        bail!("« {query} » est un preset : copiez-le d'abord (profile copy)");
    }
    collection
        .get_mut(&id)
        .ok_or_else(|| anyhow::anyhow!("profil introuvable : {query}"))
}

/// Modifie `value[path]` puis relit le résultat via `parse` : une clé inconnue
/// (ignorée par serde) ou une valeur rejetée est une erreur, pas un no-op.
pub fn apply_path<T>(
    current: &T,
    path: &str,
    raw: &str,
    parse: impl Fn(Value) -> Result<T>,
) -> Result<T>
where
    T: serde::Serialize,
{
    let mut document = serde_json::to_value(current)?;
    let value = jsonpath::parse_value(raw);
    jsonpath::set_path(&mut document, path, value.clone())?;
    let updated = parse(document)?;
    let reread = serde_json::to_value(&updated)?;
    if jsonpath::get_path(&reread, path).is_none() {
        bail!("clé inconnue ou valeur rejetée : {path}");
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_core::Profile;

    fn collection() -> ProfileCollection {
        ProfileCollection::new(vec![
            Profile::new("custom-a", "Travail"),
            Profile::new("custom-b", "Jeu"),
        ])
    }

    #[test]
    fn resolves_by_id_and_by_name() {
        let collection = collection();
        assert_eq!(
            resolve_profile(&collection, "custom-a").unwrap().name,
            "Travail"
        );
        assert_eq!(
            resolve_profile(&collection, "jeu").unwrap().id.as_str(),
            "custom-b"
        );
        assert!(resolve_profile(&collection, "nope").is_err());
    }

    #[test]
    fn ambiguous_name_is_rejected() {
        let collection =
            ProfileCollection::new(vec![Profile::new("a", "Même"), Profile::new("b", "même")]);
        assert!(
            resolve_profile(&collection, "MÊME").is_err()
                || resolve_profile(&collection, "même").is_err()
        );
    }

    #[test]
    fn apply_path_updates_and_rejects_unknown_keys() {
        let profile = Profile::new("custom-a", "Travail");
        let parse = |doc: Value| serde_json::from_value::<Profile>(doc).map_err(Into::into);
        let updated = apply_path(&profile, "name", "Bureau", parse).unwrap();
        assert_eq!(updated.name, "Bureau");
        // Une clé imbriquée inconnue est perdue au décodage : erreur.
        assert!(apply_path(&profile, "display.nonexistent", "1", parse).is_err());
    }

    #[test]
    fn presets_are_not_editable() {
        let mut collection = ProfileCollection::new(vec![Profile::new("__office__", "Office")]);
        assert!(editable(&mut collection, "__office__").is_err());
    }
}
