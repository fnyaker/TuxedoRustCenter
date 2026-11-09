// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Vérifie le contrat public `Admin1` exposé par `tcc-ipc`.

use tcc_ipc::{
    ADMIN_CONTRACT_VERSION, ADMIN_ERROR_NAMESPACE, AdminAction, AdminErrorCode, AdminReloadReport,
    AdminVersionInfo, DBUS_ADMIN_INTERFACE, DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE,
    DBUS_CONTROL_INTERFACE, IpcError, ensure_admin_contract_compatible,
};

#[test]
fn admin_dbus_names_are_stable() {
    assert_eq!(DBUS_ADMIN_SERVICE, "com.tuxedocomputers.tccd.Admin");
    assert_eq!(DBUS_ADMIN_PATH, "/com/tuxedocomputers/tccd/Admin");
    assert_eq!(DBUS_ADMIN_INTERFACE, "com.tuxedocomputers.tccd.Admin1");
    assert_eq!(DBUS_CONTROL_INTERFACE, "com.tuxedocomputers.tccd.Control1");
    assert_eq!(ADMIN_CONTRACT_VERSION, 1);
    assert_eq!(
        ADMIN_ERROR_NAMESPACE,
        "org.tuxedocomputers.tccd.Admin1.Error"
    );
}

#[test]
fn reload_report_is_camel_case_and_omits_empty_warnings() {
    let report = AdminReloadReport::clean();
    let json = report.to_json().expect("sérialisation");
    assert_eq!(json, r#"{"ok":true}"#);

    let decoded = AdminReloadReport::from_json_bounded(
        r#"{"ok":false,"warnings":["réglages: ignoré"]}"#,
        1024,
    )
    .expect("décodage");
    assert!(!decoded.is_ok());
    assert_eq!(decoded.warnings, vec!["réglages: ignoré"]);
}

#[test]
fn reload_report_rejects_oversized_payload_before_parsing() {
    let error = AdminReloadReport::from_json_bounded("{}", 1).expect_err("taille");
    assert!(matches!(
        error,
        IpcError::PayloadTooLarge { size: 2, max: 1 }
    ));
}

#[test]
fn contract_version_is_strict() {
    assert!(ensure_admin_contract_compatible(ADMIN_CONTRACT_VERSION).is_ok());
    assert!(matches!(
        ensure_admin_contract_compatible(ADMIN_CONTRACT_VERSION + 1),
        Err(IpcError::AdminContractMismatch { .. })
    ));

    let info = AdminVersionInfo::for_current("0.1.0");
    assert!(info.validate().is_ok());
    assert_eq!(info.contract_version, ADMIN_CONTRACT_VERSION);
}

#[test]
fn error_codes_are_in_the_application_namespace() {
    for code in AdminErrorCode::ALL {
        let name = code.dbus_name();
        assert!(name.starts_with(ADMIN_ERROR_NAMESPACE));
        assert!(!name.starts_with("org.freedesktop.DBus.Error"));
    }
}

#[test]
fn actions_map_to_dbus_methods() {
    for action in AdminAction::ALL {
        assert!(
            action
                .method_name()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        );
        let key = serde_json::to_string(&action).expect("sérialisation");
        assert_eq!(key, format!("\"{}\"", action.as_str()));
    }
}
