// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests d'intégration des workers matériels.
//!
//! Ce fichier inclut le module `src/hardware/` **sans** dépendre de `main.rs`
//! (que `tuxedo-hardware` ne possède pas). Il garantit que la matrice
//! matérielle compile et que ses tests unitaires tournent en CI, avant même
//! que `linux-platform` n'ajoute `mod hardware;` au binaire.
//!
//! Aucun laptop TUXEDO n'est requis : tout passe par une racine sysfs factice
//! ou par l'absence propre de matériel.

#[path = "../src/hardware/mod.rs"]
mod hardware;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hardware::{HardwareInputs, HardwareStatus, HardwareWorkers};
use tcc_hw::sysfs::SysfsRoot;

#[tokio::test]
async fn workers_start_and_stop_without_hardware() {
    let inputs = Arc::new(tokio::sync::RwLock::new(HardwareInputs::default()));
    let status = Arc::new(HardwareStatus::default());
    let workers = HardwareWorkers::start(
        inputs,
        status.clone(),
        SysfsRoot::new("/nonexistent-tcc-hardware"),
        true,
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    workers.notify();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let snapshot = status.snapshot();
    // Aucun domaine ne doit paniquer ni écrire : tout est « non disponible ».
    assert!(!snapshot.charging.available);
    assert!(!snapshot.lighting.available);
    assert!(!snapshot.odm.available);
    assert!(!snapshot.sensors.enabled);
    assert!(!snapshot.nvidia.available);
    assert!(!snapshot.prime.supported);
    assert!(snapshot.fn_lock.enabled.is_none());
    let _ = tokio::time::timeout(Duration::from_secs(5), workers.shutdown()).await;
}

#[tokio::test]
async fn sensors_are_disabled_until_requested() {
    let inputs = Arc::new(tokio::sync::RwLock::new(HardwareInputs::default()));
    let status = Arc::new(HardwareStatus::default());
    let enabled = Arc::new(AtomicBool::new(false));
    let workers = HardwareWorkers::start(
        inputs,
        status.clone(),
        SysfsRoot::new("/nonexistent-tcc-hardware"),
        true,
        enabled.clone(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!status.sensors().enabled);
    enabled.store(true, std::sync::atomic::Ordering::Relaxed);
    workers.notify();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // La collecte est active, mais aucun capteur n'existe : `available: false`.
    assert!(status.sensors().enabled);
    assert!(!status.sensors().available);
    let _ = tokio::time::timeout(Duration::from_secs(5), workers.shutdown()).await;
}

#[test]
fn snapshot_serialization_is_stable() {
    let snapshot = HardwareStatus::default().snapshot();
    let json = serde_json::to_string(&snapshot).expect("snapshot JSON");
    for key in [
        "charging", "lighting", "odm", "sensors", "nvidia", "webcam", "fnLock", "prime", "ycbcr",
    ] {
        assert!(json.contains(&format!("\"{key}\"")), "clé absente: {key}");
    }
}
