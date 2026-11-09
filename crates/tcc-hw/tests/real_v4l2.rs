// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Test opt-in de lecture réelle des contrôles UVC.
//!
//! Il ouvre les nœuds `/dev/video*` de la machine et lit l'allowlist UVC via
//! le transport réel (`tcc-hw::v4l2::real`). Il est ignoré par défaut : la CI
//! et `cargo test` n'ont pas besoin de caméra.
//!
//! Exécution manuelle sur un laptop TUXEDO :
//!
//! ```text
//! cargo test -p tcc-hw --test real_v4l2 -- --ignored --nocapture
//! ```
//!
//! Si aucun nœud V4L2 n'est présent, le test se termine sans échec (matériel
//! absent). S'il y a une caméra, il exige au moins un nœud lisible et au moins
//! un contrôle de l'allowlist.

use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::v4l2::UvcReaderController;

#[test]
#[ignore = "ouvre /dev/video* : nécessite une caméra et un accès en lecture"]
fn reads_real_uvc_controls() {
    let controller = UvcReaderController::system(SysfsRoot::system());
    let devices = controller.devices();
    println!("devices: {:#?}", devices.devices);
    if devices.devices.is_empty() {
        println!("aucune caméra V4L2 détectée : test sans objet");
        return;
    }

    let report = controller.read_all();
    println!("errors: {:#?}", report.errors);
    for entry in &report.controls {
        println!(
            "{} available={} controls={} truncated={} error={:?}",
            entry.device.node,
            entry.device.available,
            entry.controls.len(),
            entry.truncated,
            entry.error
        );
        for control in &entry.controls {
            println!(
                "  {} ({}) {}..{} step {} default {} current {:?} menu {}",
                control.key,
                control.name,
                control.min,
                control.max,
                control.step,
                control.default,
                control.current,
                control.menu.len()
            );
        }
    }

    assert!(
        report.controls.iter().any(|entry| entry.device.available),
        "aucun nœud V4L2 lisible : {:?}",
        report.errors
    );
    assert!(
        report
            .controls
            .iter()
            .any(|entry| !entry.controls.is_empty()),
        "aucun contrôle UVC de l'allowlist lu : {:?}",
        report.errors
    );
}
