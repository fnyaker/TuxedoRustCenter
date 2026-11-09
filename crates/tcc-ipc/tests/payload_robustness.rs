// SPDX-License-Identifier: GPL-3.0-or-later
//
// Robustesse des payloads IPC : mutations déterministes des fixtures, aucun
// panic attendu. Ce test complète les cibles `cargo-fuzz` proposées dans
// `docs/qa/TEST_PLAN.md` sans ajouter de dépendance au manifeste.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use tcc_core::ProfileCollection;
use tcc_ipc::{ActiveProfile, StateSnapshot};

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

fn mutate(base: &str, iterations: usize, seed: u64, mut check: impl FnMut(&str)) {
    let mut rng = Rng::new(seed);
    for _ in 0..iterations {
        let mut bytes = base.as_bytes().to_vec();
        let index = rng.index(bytes.len());
        bytes[index] = (rng.next_u64() & 0xff) as u8;
        let text = String::from_utf8_lossy(&bytes);
        check(&text);
    }
}

#[test]
fn mutated_state_snapshots_never_panic() {
    let base = fixtures::read_fixture("dbus/state_snapshot_manual_override.json");
    mutate(&base, 512, 0x1cc0_0001, |text| {
        if let Ok(snapshot) = serde_json::from_str::<StateSnapshot>(text) {
            let _ = snapshot.validate();
        }
    });
}

#[test]
fn mutated_active_profiles_never_panic() {
    let base = fixtures::read_fixture("dbus/active_profile.json");
    mutate(&base, 256, 0x1cc0_0002, |text| {
        if let Ok(profile) = serde_json::from_str::<ActiveProfile>(text) {
            let _ = profile.validate();
        }
    });
}

#[test]
fn mutated_profile_collections_never_panic() {
    let base = fixtures::read_fixture("dbus/profile_collection.json");
    mutate(&base, 512, 0x1cc0_0003, |text| {
        if let Ok(profiles) = serde_json::from_str::<ProfileCollection>(text) {
            let _ = profiles.validate();
            let _ = serde_json::to_string(&profiles);
        }
    });

    // Troncatures : un payload coupé ne doit jamais faire paniquer serde.
    for cut in (0..=base.len()).step_by(31) {
        let mut end = cut;
        while !base.is_char_boundary(end) {
            end -= 1;
        }
        let _ = serde_json::from_str::<ProfileCollection>(&base[..end]);
    }
}

#[test]
fn nul_bytes_and_empty_payloads_are_rejected_cleanly() {
    for payload in ["", "\0", "\u{feff}{}", "null", "{}"] {
        assert!(serde_json::from_str::<StateSnapshot>(payload).is_err());
        assert!(serde_json::from_str::<ProfileCollection>(payload).is_err());
    }

    // `[]` est une collection vide valide : ce n'est pas une erreur de parsing.
    let empty: ProfileCollection = serde_json::from_str("[]").unwrap();
    assert!(empty.is_empty());
    empty.validate().unwrap();
}
