// SPDX-License-Identifier: GPL-3.0-or-later
//
// Robustesse des entrées legacy : mutations déterministes, troncatures et
// octets invalides. Aucune dépendance de fuzzing n'est ajoutée au manifeste ;
// ce test sert de filet minimal en attendant les cibles `cargo-fuzz` décrites
// dans `docs/qa/TEST_PLAN.md`.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use std::fs;

use tcc_core::{
    LegacyPaths, migrate_fan_tables, migrate_profiles, migrate_settings, read_legacy_config,
};

/// Générateur xorshift déterministe : aucune dépendance, même corpus à chaque
/// exécution.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next_u64() as usize) % len
    }
}

fn mutate_bytes(base: &str, iterations: usize, seed: u64, parse: impl Fn(&str)) {
    assert!(!base.is_empty());
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let mut bytes = base.as_bytes().to_vec();
        let index = rng.index(bytes.len());
        bytes[index] = (rng.next_u64() & 0xff) as u8;
        let text = String::from_utf8_lossy(&bytes);
        parse(&text);
    }
}

#[test]
fn mutated_legacy_profiles_never_panic() {
    let base = fs::read_to_string(fixtures::fixture_path("legacy/profiles")).unwrap();
    // 512 mutations suffisent à couvrir les champs structurants du fichier
    // sans transformer la CI en campagne de fuzzing.
    mutate_bytes(&base, 512, 0x5eed_0001, |text| {
        let _ = migrate_profiles(text);
    });

    // Troncature à toutes les frontières de caractères utiles.
    for cut in (0..=base.len()).step_by(17) {
        let mut end = cut;
        while !base.is_char_boundary(end) {
            end -= 1;
        }
        let _ = migrate_profiles(&base[..end]);
    }
}

#[test]
fn mutated_legacy_settings_and_fan_tables_never_panic() {
    let settings = fs::read_to_string(fixtures::fixture_path("legacy/settings")).unwrap();
    mutate_bytes(&settings, 256, 0x5eed_0002, |text| {
        let _ = migrate_settings(text);
    });

    let tables = fs::read_to_string(fixtures::fixture_path("legacy/fantables")).unwrap();
    mutate_bytes(&tables, 256, 0x5eed_0003, |text| {
        let _ = migrate_fan_tables(text);
    });
}

#[test]
fn invalid_utf8_and_oversized_legacy_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let paths = LegacyPaths::new(dir.path());

    fs::write(&paths.settings, [0xff, 0xfe, 0xfd, 0x00]).unwrap();
    let error = read_legacy_config(&paths).unwrap_err();
    assert!(
        error.to_string().contains("UTF-8"),
        "erreur inattendue: {error}"
    );

    let oversized = vec![b' '; tcc_core::DEFAULT_MAX_CONFIG_BYTES as usize + 1];
    fs::write(&paths.settings, oversized).unwrap();
    let error = read_legacy_config(&paths).unwrap_err();
    assert!(
        error.to_string().contains("limite"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn nul_and_control_characters_in_profile_ids_are_rejected() {
    let raw = r#"[
        {
            "id": "__office__\u0000suffix",
            "name": "Office",
            "display": {},
            "cpu": {},
            "webcam": {},
            "fan": {},
            "odmProfile": {},
            "odmPowerLimits": {},
            "nvidiaPowerCTRLProfile": {}
        }
    ]"#;
    let error = migrate_profiles(raw).unwrap_err();
    assert!(
        error.to_string().contains("id"),
        "erreur inattendue: {error}"
    );
}
