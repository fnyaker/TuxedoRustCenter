// SPDX-License-Identifier: GPL-3.0-or-later
//
// Import / export de profils personnalisés.
//
// # Format d'échange
//
// Export (préféré, versionné) :
// ```json
// { "exportVersion": 1, "profiles": [ <tcc_core::Profile>, … ] }
// ```
//
// Import : l'enveloppe ci-dessus **ou** un tableau `ProfileCollection` nu
// (format historique de TCC). L'enveloppe refuse les clés inconnues
// (`deny_unknown_fields`) ; les profils conservent leurs champs inconnus dans
// `Profile::extra`, comme le reste du projet.
//
// # Sécurité
//
// - Le chemin vient exclusivement du `FileDialog` QML (absolu, sans `..`,
//   sans caractère de contrôle), jamais du contenu du fichier.
// - Taille bornée à 1 Mio **avant** tout parse, côté fichier puis côté buffer.
// - Aucun shell, aucune écriture privilégiée : `std::fs` depuis la GUI.
// - Les erreurs ne contiennent ni chemin ni contenu brut (noms de champ
//   uniquement).
// - Les presets sont filtrés à l'import et jamais exportés.
// - Un import invalide est rejeté **en bloc** (aucun ajout partiel).

use std::collections::BTreeSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use tcc_core::{
    MAX_PROFILE_NAME_LEN, MAX_PROFILES, Profile, ProfileCollection, ProfileId, validate_profile_id,
};

use crate::profile_edit::{PreparedProfiles, SaveError};

/// Version du format d'échange.
pub const EXPORT_VERSION: u32 = 1;
/// Taille maximale d'un fichier d'import ou d'un export (1 Mio).
pub const MAX_TRANSFER_BYTES: usize = 1024 * 1024;
/// Longueur maximale d'un chemin choisi par l'utilisateur.
pub const MAX_TRANSFER_PATH_LEN: usize = 4096;

/// Enveloppe d'export versionnée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportEnvelope {
    export_version: u32,
    profiles: Vec<Profile>,
}

/// Compteurs présentés à l'utilisateur après un import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Profils effectivement ajoutés.
    pub imported: usize,
    /// Profils renommés à cause d'un conflit de nom.
    pub renamed: usize,
    /// Presets ignorés (non importés).
    pub skipped: usize,
}

/// Exporte les profils personnalisés d'une collection.
///
/// # Errors
///
/// [`SaveError::NothingToExport`] si la collection ne contient que des presets,
/// [`SaveError::PayloadTooLarge`] au-delà de [`MAX_TRANSFER_BYTES`].
pub fn build_export(base: &ProfileCollection) -> Result<(String, usize), SaveError> {
    let profiles: Vec<Profile> = base
        .iter()
        .filter(|profile| !profile.is_preset())
        .cloned()
        .collect();
    if profiles.is_empty() {
        return Err(SaveError::NothingToExport);
    }
    let count = profiles.len();
    let envelope = ExportEnvelope {
        export_version: EXPORT_VERSION,
        profiles,
    };
    let json = serde_json::to_string_pretty(&envelope).map_err(|_| SaveError::InvalidPayload)?;
    if json.len() > MAX_TRANSFER_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    Ok((json, count))
}

/// Vérifie un chemin fourni par le `FileDialog`.
///
/// Refuse un chemin relatif, vide, trop long, contenant un caractère de
/// contrôle ou une composante `..`. Aucun chemin n'est déduit du contenu du
/// fichier.
///
/// # Errors
///
/// [`SaveError::PathInvalid`] si le chemin n'est pas un chemin absolu sûr.
pub fn validate_transfer_path(path: &str) -> Result<&Path, SaveError> {
    if path.is_empty()
        || path.len() > MAX_TRANSFER_PATH_LEN
        || !path.starts_with('/')
        || path.chars().any(char::is_control)
    {
        return Err(SaveError::PathInvalid);
    }
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(SaveError::PathInvalid);
    }
    Ok(candidate)
}

/// Écrit un export sur disque (1 Mio déjà garanti par [`build_export`]).
///
/// # Errors
///
/// [`SaveError::PathInvalid`] pour un chemin refusé ou un lien symbolique,
/// [`SaveError::FileWriteFailed`] si l'écriture échoue.
pub fn write_export_file(path: &str, json: &str) -> Result<(), SaveError> {
    if json.len() > MAX_TRANSFER_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    let target = validate_transfer_path(path)?;
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_dir() => {
            return Err(SaveError::PathInvalid);
        }
        Ok(_) => {}
        // Fichier absent : création autorisée.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SaveError::FileWriteFailed),
    }
    let mut file = std::fs::File::create(target).map_err(|_| SaveError::FileWriteFailed)?;
    use std::io::Write;
    file.write_all(json.as_bytes())
        .map_err(|_| SaveError::FileWriteFailed)?;
    let _ = file.sync_all();
    Ok(())
}

/// Lit et analyse un fichier d'import borné.
///
/// # Errors
///
/// [`SaveError::PathInvalid`], [`SaveError::PayloadTooLarge`],
/// [`SaveError::ReadFailed`] ou [`SaveError::InvalidImport`].
pub fn read_import_file(path: &str) -> Result<Vec<Profile>, SaveError> {
    let target = validate_transfer_path(path)?;
    let metadata = std::fs::symlink_metadata(target).map_err(|_| SaveError::ReadFailed)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SaveError::ReadFailed);
    }
    if metadata.len() > MAX_TRANSFER_BYTES as u64 {
        return Err(SaveError::PayloadTooLarge);
    }
    let data = std::fs::read(target).map_err(|_| SaveError::ReadFailed)?;
    if data.len() > MAX_TRANSFER_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    let text = std::str::from_utf8(&data).map_err(|_| SaveError::InvalidImport)?;
    parse_import(text)
}

/// Analyse un contenu d'import : enveloppe versionnée ou tableau nu (TCC).
///
/// # Errors
///
/// [`SaveError::InvalidImport`] si le JSON, l'enveloppe ou la version sont
/// invalides.
pub fn parse_import(text: &str) -> Result<Vec<Profile>, SaveError> {
    if text.len() > MAX_TRANSFER_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| SaveError::InvalidImport)?;
    if value.is_object() {
        let envelope: ExportEnvelope =
            serde_json::from_value(value).map_err(|_| SaveError::InvalidImport)?;
        if envelope.export_version != EXPORT_VERSION {
            return Err(SaveError::InvalidImport);
        }
        Ok(envelope.profiles)
    } else if value.is_array() {
        serde_json::from_value(value).map_err(|_| SaveError::InvalidImport)
    } else {
        Err(SaveError::InvalidImport)
    }
}

/// Prépare l'import : filtre les presets, résout les conflits, valide.
///
/// - collection modifiée depuis la lecture → [`SaveError::Conflict`] ;
/// - preset importé → ignoré (compté dans `skipped`) ;
/// - ID déjà présent → nouvel ID généré (jamais d'écrasement) ;
/// - nom déjà présent → suffixe `(importé)`, `(importé 2)`, … ;
/// - profil invalide ou limite `MAX_PROFILES` → import rejeté en bloc.
///
/// # Errors
///
/// [`SaveError::Conflict`], [`SaveError::TooManyProfiles`],
/// [`SaveError::Validation`], [`SaveError::PayloadTooLarge`].
pub fn prepare_import(
    base: &ProfileCollection,
    current: &ProfileCollection,
    imported: &[Profile],
) -> Result<(PreparedProfiles, ImportSummary), SaveError> {
    if base != current {
        return Err(SaveError::Conflict);
    }

    let mut summary = ImportSummary::default();
    let mut candidates: Vec<Profile> = Vec::new();
    for profile in imported {
        if profile.is_preset() {
            summary.skipped += 1;
            continue;
        }
        profile.validate().map_err(|error| SaveError::Validation {
            field: error
                .issues()
                .first()
                .map(|issue| issue.field.clone())
                .unwrap_or_else(|| "profiles".to_owned()),
        })?;
        candidates.push(profile.clone());
    }
    if base.len() + candidates.len() > MAX_PROFILES {
        return Err(SaveError::TooManyProfiles);
    }

    let mut updated = base.clone();
    let mut names: BTreeSet<String> = base
        .iter()
        .map(|profile| normalized_name(&profile.name))
        .collect();

    for mut profile in candidates {
        // Jamais d'écrasement : tout ID déjà pris (ou invalide) est remplacé.
        if profile.id.is_empty()
            || validate_profile_id(&profile.id).is_err()
            || updated.get(&profile.id).is_some()
        {
            let mut candidate = ProfileId::generate();
            while updated.get(&candidate).is_some() {
                candidate = ProfileId::generate();
            }
            profile.id = candidate;
        }

        if names.contains(&normalized_name(&profile.name)) {
            profile.name = unique_import_name(&mut names, &profile.name);
            summary.renamed += 1;
        } else {
            names.insert(normalized_name(&profile.name));
        }

        updated.upsert(profile);
        summary.imported += 1;
    }

    updated.validate().map_err(|error| SaveError::Validation {
        field: error
            .issues()
            .first()
            .map(|issue| issue.field.clone())
            .unwrap_or_else(|| "profiles".to_owned()),
    })?;

    let json = serde_json::to_string(&updated).map_err(|_| SaveError::InvalidPayload)?;
    if json.len() > crate::profile_edit::MAX_PROFILES_PAYLOAD_BYTES {
        return Err(SaveError::PayloadTooLarge);
    }
    Ok((
        PreparedProfiles {
            collection: updated,
            json,
            new_profile_id: None,
        },
        summary,
    ))
}

fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Nom unique pour un profil importé : `X (importé)`, `X (importé 2)`, …
fn unique_import_name(names: &mut BTreeSet<String>, source_name: &str) -> String {
    let source = source_name.trim();
    for counter in 1..=MAX_PROFILES + 1 {
        let suffix = if counter == 1 {
            " (importé)".to_owned()
        } else {
            format!(" (importé {counter})")
        };
        let keep = MAX_PROFILE_NAME_LEN.saturating_sub(suffix.chars().count());
        let candidate = format!(
            "{}{}",
            source.chars().take(keep).collect::<String>(),
            suffix
        );
        if names.insert(normalized_name(&candidate)) {
            return candidate;
        }
    }
    // Inatteignable avec `MAX_PROFILES` profils.
    format!(
        "{} (importé)",
        source
            .chars()
            .take(MAX_PROFILE_NAME_LEN - 10)
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EXPORT_VERSION, MAX_TRANSFER_BYTES, build_export, parse_import, prepare_import,
        read_import_file, validate_transfer_path, write_export_file,
    };
    use crate::profile_edit::SaveError;
    use tcc_core::{Profile, ProfileCollection, ProfileId};

    fn collection() -> ProfileCollection {
        ProfileCollection::new(vec![
            Profile::new("__office__", "Office"),
            Profile::new("custom-a", "Studio"),
            Profile::new("custom-b", "Gaming"),
        ])
    }

    fn imported(name: &str, id: &str) -> Profile {
        Profile::new(id, name)
    }

    #[test]
    fn export_contains_only_custom_profiles_with_version() {
        let (json, count) = build_export(&collection()).expect("export");
        assert_eq!(count, 2);
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["exportVersion"], EXPORT_VERSION);
        let profiles = value["profiles"].as_array().expect("profiles");
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|p| p["id"] != "__office__"));
        assert!(json.len() <= MAX_TRANSFER_BYTES);
    }

    #[test]
    fn export_refuses_preset_only_collection() {
        let presets = ProfileCollection::new(vec![Profile::new("__office__", "Office")]);
        assert_eq!(
            build_export(&presets).unwrap_err(),
            SaveError::NothingToExport
        );
    }

    #[test]
    fn envelope_and_legacy_array_are_accepted() {
        let envelope = r#"{"exportVersion":1,"profiles":[{"id":"x","name":"X"}]}"#.to_owned();
        let parsed = parse_import(&envelope).expect("enveloppe");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "X");

        let legacy = r#"[{"id":"y","name":"Y"}]"#;
        let parsed = parse_import(legacy).expect("tableau nu");
        assert_eq!(parsed[0].id.as_str(), "y");
    }

    #[test]
    fn invalid_envelopes_are_rejected() {
        for payload in [
            r#"{"exportVersion":2,"profiles":[]}"#,
            r#"{"profiles":[]}"#,
            r#"{"exportVersion":1,"profiles":[],"extra":true}"#,
            r#"{"exportVersion":1,"profiles":"nope"}"#,
            r#"{"exportVersion":1}"#,
            r#"42"#,
            r#""chaine""#,
            r#"{"exportVersion":1,"profiles":[{"id":"x","name":5}]}"#,
            "{ pas du json",
        ] {
            assert_eq!(
                parse_import(payload).unwrap_err(),
                SaveError::InvalidImport,
                "payload: {payload}"
            );
        }
    }

    #[test]
    fn oversized_payload_is_rejected_before_parse() {
        let text = "x".repeat(MAX_TRANSFER_BYTES + 1);
        assert_eq!(parse_import(&text).unwrap_err(), SaveError::PayloadTooLarge);
    }

    #[test]
    fn import_filters_presets_and_reports_skipped() {
        let profiles = vec![
            Profile::new("__office__", "Office"),
            imported("Studio", "custom-new"),
        ];
        let (prepared, summary) =
            prepare_import(&collection(), &collection(), &profiles).expect("import");
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.imported, 1);
        // « Studio » existe déjà dans la collection de test : l'import est
        // renommé, jamais fusionné.
        assert_eq!(summary.renamed, 1);
        assert_eq!(prepared.collection.len(), 4);
    }

    #[test]
    fn id_conflict_generates_a_new_id_without_overwrite() {
        let profiles = vec![imported("Studio bis", "custom-a")];
        let (prepared, summary) =
            prepare_import(&collection(), &collection(), &profiles).expect("import");
        let original = prepared.collection.get_by_id("custom-a").expect("source");
        assert_eq!(original.name, "Studio");
        let imported = prepared
            .collection
            .iter()
            .find(|p| p.name == "Studio bis")
            .expect("importé");
        assert_ne!(imported.id.as_str(), "custom-a");
        assert_eq!(summary.imported, 1);
    }

    #[test]
    fn name_conflict_is_suffixed() {
        let profiles = vec![
            Profile::new("custom-new", "Studio"),
            Profile::new("custom-new2", "Studio"),
        ];
        let (prepared, summary) =
            prepare_import(&collection(), &collection(), &profiles).expect("import");
        assert_eq!(summary.imported, 2);
        assert_eq!(summary.renamed, 2);
        let names: Vec<&str> = prepared
            .collection
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert!(names.contains(&"Studio (importé)"));
        assert!(names.contains(&"Studio (importé 2)"));
    }

    #[test]
    fn long_names_stay_within_the_limit() {
        let long = "x".repeat(tcc_core::MAX_PROFILE_NAME_LEN);
        let profiles = vec![Profile::new("custom-long", long.clone())];
        let (prepared, summary) =
            prepare_import(&collection(), &collection(), &profiles).expect("import");
        assert_eq!(summary.imported, 1);
        let imported = prepared
            .collection
            .iter()
            .find(|p| p.id.as_str() == "custom-long")
            .expect("importé");
        assert!(imported.name.chars().count() <= tcc_core::MAX_PROFILE_NAME_LEN);
    }

    #[test]
    fn invalid_profile_rejects_the_whole_import() {
        let mut invalid = Profile::new("custom-bad", "Valide");
        invalid.name.clear();
        let profiles = vec![Profile::new("custom-ok", "Bon"), invalid];
        let error = prepare_import(&collection(), &collection(), &profiles).unwrap_err();
        assert!(matches!(error, SaveError::Validation { .. }));
    }

    #[test]
    fn collection_limit_is_enforced_without_partial_import() {
        let mut base = ProfileCollection::new(Vec::new());
        for index in 0..tcc_core::MAX_PROFILES - 1 {
            base.upsert(Profile::new(
                format!("custom-{index}"),
                format!("Profil {index}"),
            ));
        }
        let profiles = vec![
            Profile::new("custom-new1", "Nouveau 1"),
            Profile::new("custom-new2", "Nouveau 2"),
        ];
        let error = prepare_import(&base, &base, &profiles).unwrap_err();
        assert_eq!(error, SaveError::TooManyProfiles);
        // Aucun ajout partiel : la base n'est pas modifiée.
        assert_eq!(base.len(), tcc_core::MAX_PROFILES - 1);
    }

    #[test]
    fn concurrent_change_is_a_conflict() {
        let mut current = collection();
        current.upsert(Profile::new("custom-other", "Autre"));
        let profiles = vec![imported("Nouveau", "custom-new")];
        assert_eq!(
            prepare_import(&collection(), &current, &profiles).unwrap_err(),
            SaveError::Conflict
        );
    }

    #[test]
    fn imported_names_are_unique_against_the_base() {
        let mut base = ProfileCollection::new(vec![
            Profile::new("custom-a", "Studio"),
            Profile::new("custom-a2", "Studio (importé)"),
        ]);
        base.upsert(Profile::new("custom-b", "Autre"));
        let profiles = vec![Profile::new("custom-c", "Studio")];
        let (prepared, summary) = prepare_import(&base, &base, &profiles).expect("import");
        assert_eq!(summary.renamed, 1);
        let names: Vec<&str> = prepared
            .collection
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert!(names.contains(&"Studio (importé 2)"));
    }

    #[test]
    fn paths_must_be_absolute_and_safe() {
        assert!(validate_transfer_path("/home/user/profiles.json").is_ok());
        for path in [
            "",
            "relative.json",
            "/home/../etc/passwd",
            "/home/user/a\u{7}.json",
            &format!("/{}", "x".repeat(MAX_TRANSFER_BYTES)),
        ] {
            assert_eq!(
                validate_transfer_path(path).unwrap_err(),
                SaveError::PathInvalid
            );
        }
    }

    #[test]
    fn export_import_roundtrip_on_disk() {
        let path = std::env::temp_dir().join(format!(
            "tcc-gui-transfer-{}-roundtrip.json",
            std::process::id()
        ));
        let path_string = path.to_string_lossy().to_string();
        let (json, count) = build_export(&collection()).expect("export");
        assert_eq!(count, 2);
        write_export_file(&path_string, &json).expect("écriture");
        let parsed = read_import_file(&path_string).expect("lecture");
        assert_eq!(parsed.len(), 2);
        // Les presets ne sont pas dans l'export.
        assert!(parsed.iter().all(|profile| !profile.is_preset()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_rejects_directories_and_missing_files() {
        let directory = std::env::temp_dir();
        let directory_string = directory.to_string_lossy().to_string();
        assert_eq!(
            read_import_file(&directory_string).unwrap_err(),
            SaveError::ReadFailed
        );
        let missing = std::env::temp_dir().join(format!(
            "tcc-gui-transfer-{}-missing.json",
            std::process::id()
        ));
        assert_eq!(
            read_import_file(&missing.to_string_lossy()).unwrap_err(),
            SaveError::ReadFailed
        );
    }

    #[test]
    fn import_keeps_unknown_profile_fields() {
        let payload = r#"[{"id":"custom-x","name":"X","futureField":{"kept":true}}]"#;
        let parsed = parse_import(payload).expect("parse");
        let (prepared, _) = prepare_import(&collection(), &collection(), &parsed).expect("import");
        let json = serde_json::to_string(&prepared.collection).expect("json");
        assert!(json.contains("futureField"));
    }

    #[test]
    fn imported_profile_ids_remain_valid() {
        let profiles = vec![imported("Nouveau", "custom-new")];
        let (prepared, _) =
            prepare_import(&collection(), &collection(), &profiles).expect("import");
        for profile in prepared.collection.iter() {
            tcc_core::validate_profile_id(&ProfileId::new(profile.id.as_str())).expect("ID valide");
        }
    }
}
