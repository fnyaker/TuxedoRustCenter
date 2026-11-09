// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests d'intégration de la lecture UVC V4L2, sans caméra.
//!
//! Le transport réel n'est jamais instancié : une fabrique renvoie un
//! [`FakeReader`] et une racine sysfs factice fournit les nœuds `videoN`.
//! Aucun `/dev`, aucun privilège, aucun réseau.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tcc_hw::sysfs::SysfsRoot;
use tcc_hw::v4l2::fake::{FakeControl, FakeReader};
use tcc_hw::v4l2::{DeviceCapabilities, UvcReaderController, UvcValue};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn fixture(root: &Path, count: usize) {
    for index in 0..count {
        let node = format!("video{index}");
        write(
            root,
            &format!("class/video4linux/{node}/name"),
            "Integrated Camera\n",
        );
        write(root, &format!("class/video4linux/{node}/index"), "0\n");
        write(
            root,
            &format!("class/video4linux/{node}/dev"),
            &format!("81:{index}\n"),
        );
        write(
            root,
            &format!("class/video4linux/{node}/device/uevent"),
            "PRODUCT=1bcf/2c99/7200\n",
        );
    }
}

fn capabilities() -> DeviceCapabilities {
    DeviceCapabilities {
        driver: "uvcvideo".to_owned(),
        card: "Integrated Camera".to_owned(),
        bus_info: "usb-0000:00:14.0-1".to_owned(),
        capabilities: 0x8000_0001,
        device_caps: 0x0000_0001,
    }
}

fn controller(root: &Path, reader: FakeReader) -> (UvcReaderController, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let reader = Arc::new(reader);
    let controller = UvcReaderController::with_factory(
        SysfsRoot::new(root),
        Arc::new(move |_device| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeReader::clone_state(&reader)))
        }),
    );
    (controller, calls)
}

#[test]
fn enumerates_capture_devices_and_reads_allowlisted_controls() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    let reader = FakeReader::new(capabilities())
        .with_control(
            FakeControl::new(0x0098_0900, "Brightness", 1)
                .range(0, 255, 1)
                .default(128)
                .value(42),
        )
        // Énuméré par le pilote mais hors allowlist : jamais exposé.
        .with_control(FakeControl::new(0x009f_0903, "Test Pattern", 3).range(0, 1, 1))
        // CID privé : jamais exposé.
        .with_control(FakeControl::new(0x0800_0001, "Private", 1).range(0, 10, 1));
    let (controller, _) = controller(dir.path(), reader);

    let devices = controller.devices();
    assert_eq!(devices.devices.len(), 1);
    assert!(devices.devices[0].available);
    assert_eq!(devices.devices[0].identity.vendor.as_deref(), Some("1bcf"));

    let report = controller.read_all();
    assert_eq!(report.controls.len(), 1);
    let controls = &report.controls[0].controls;
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].key, "brightness");
    assert_eq!(controls[0].current, Some(UvcValue::Integer(42)));
    assert_eq!(controls[0].category.as_str(), "color");
    assert!(controls[0].simple);
}

#[test]
fn metadata_nodes_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    // Nœud metadata UVC (index 1) : ne doit pas être listé.
    write(
        dir.path(),
        "class/video4linux/video9/name",
        "Integrated Camera Metadata\n",
    );
    write(dir.path(), "class/video4linux/video9/index", "1\n");
    write(dir.path(), "class/video4linux/video9/dev", "81:9\n");
    let (controller, _) = controller(dir.path(), FakeReader::new(capabilities()));
    let devices = controller.devices();
    assert_eq!(devices.devices.len(), 1);
    assert_eq!(devices.devices[0].node, "video0");
}

#[test]
fn menu_is_read_and_mapped() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    let reader = FakeReader::new(capabilities()).with_control(
        FakeControl::new(0x0098_0918, "Power Line Frequency", 3)
            .range(0, 2, 1)
            .value(1)
            .menu(&[(0, "Disabled"), (1, "50 Hz"), (2, "60 Hz")]),
    );
    let (controller, _) = controller(dir.path(), reader);
    let report = controller.read_all();
    let control = &report.controls[0].controls[0];
    assert_eq!(control.key, "power_line_frequency");
    assert_eq!(control.menu.len(), 3);
    assert_eq!(
        control.current,
        Some(UvcValue::Menu {
            index: 1,
            key: "50_hz".to_owned(),
            label: "50 Hz".to_owned(),
        })
    );
}

#[test]
fn eintr_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    let reader = FakeReader::new(capabilities())
        .with_control(FakeControl::new(0x0098_0900, "Brightness", 1).value(10))
        .with_eintr_once();
    let (controller, _) = controller(dir.path(), reader);
    let report = controller.read_all();
    assert_eq!(
        report.controls[0].controls[0].current,
        Some(UvcValue::Integer(10))
    );
}

#[test]
fn meta_capture_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    let mut caps = capabilities();
    caps.device_caps = 0x0080_0000; // V4L2_CAP_META_CAPTURE
    let (controller, _) = controller(dir.path(), FakeReader::new(caps));
    let report = controller.devices();
    assert!(!report.devices[0].available);
    assert!(report.errors.iter().any(|error| error.contains("meta")));
}

#[test]
fn busy_and_absent_are_reported_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 1);
    let reader = FakeReader::new(capabilities()).fail_capabilities(std::io::ErrorKind::WouldBlock);
    let (busy_controller, _) = controller(dir.path(), reader);
    let report = busy_controller.devices();
    assert!(!report.devices[0].available);
    assert!(report.errors.iter().any(|error| error.contains("occupé")));

    let empty = tempfile::tempdir().unwrap();
    let (empty_controller, calls) = controller(empty.path(), FakeReader::new(capabilities()));
    let report = empty_controller.read_all();
    assert!(report.controls.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn budget_truncates_and_bounds_devices() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), 20);
    let (controller, _) = controller(
        dir.path(),
        FakeReader::new(capabilities())
            .with_control(FakeControl::new(0x0098_0900, "Brightness", 1).value(1)),
    );
    let report = controller.devices();
    assert_eq!(report.devices.len(), tcc_hw::v4l2::MAX_DEVICES);
    assert!(report.truncated);
}

#[test]
fn no_device_never_calls_the_factory() {
    let dir = tempfile::tempdir().unwrap();
    let (controller, calls) = controller(dir.path(), FakeReader::new(capabilities()));
    let report = controller.read_all();
    assert!(report.controls.is_empty());
    assert!(!report.truncated);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
