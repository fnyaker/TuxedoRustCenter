// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Vérifie le passage des tombstones d'IDs de profils par le contrat JSON
//! `Admin1.SaveSettingsJSON`.
//!
//! Aucun type D-Bus n'est modifié : les tombstones sont un champ additif de
//! [`tcc_core::Settings`], sérialisé `deletedProfileIds`. Un ancien lecteur
//! les conserve dans `extra` ; un nouveau lecteur absent les interprète comme
//! une liste vide.

use tcc_core::{MAX_DELETED_PROFILE_IDS, ProfileId, Settings};

#[test]
fn settings_tombstones_roundtrip_over_the_admin_json_contract() {
    let mut settings = Settings::default();
    settings
        .mark_profile_deleted(&ProfileId::new("custom-old"))
        .unwrap();

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("\"deletedProfileIds\":[\"custom-old\"]"));
    assert!(!json.contains("deleted_profile_ids"));

    let parsed: Settings = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_profile_id_deleted(&ProfileId::new("custom-old")));
    assert!(!parsed.is_profile_id_deleted(&ProfileId::new("custom-new")));
    parsed.validate().unwrap();
}

#[test]
fn unknown_fields_survive_with_tombstones() {
    let raw = r#"{
        "deletedProfileIds": ["custom-old"],
        "futureSetting": 42
    }"#;
    let settings: Settings = serde_json::from_str(raw).unwrap();
    let json = serde_json::to_value(&settings).unwrap();

    assert_eq!(
        json.get("deletedProfileIds"),
        Some(&serde_json::json!(["custom-old"]))
    );
    assert_eq!(json.get("futureSetting"), Some(&serde_json::json!(42)));
}

#[test]
fn oversized_tombstone_payload_is_rejected_by_validation() {
    let ids = (0..=MAX_DELETED_PROFILE_IDS)
        .map(|index| format!("\"custom-{index}\""))
        .collect::<Vec<_>>()
        .join(",");
    let json = format!("{{\"deletedProfileIds\":[{ids}]}}");
    assert!(json.len() < tcc_core::DEFAULT_MAX_CONFIG_BYTES as usize);

    let settings: Settings = serde_json::from_str(&json).unwrap();
    assert!(settings.validate().is_err());
}
