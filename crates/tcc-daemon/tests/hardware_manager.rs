// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests d'intégration de l'interface `com.tuxedocomputers.tccd.Manager1`.
//!
//! Démarre un daemon `--dev-session` sur le bus de session : aucune écriture
//! matérielle n'est effectuée, mais les setters sont validés, limités et
//! audités comme en production. Sans bus de session, le test est ignoré.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use zbus::proxy;

#[proxy(
    interface = "com.tuxedocomputers.tccd.Manager1",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait HardwareManager {
    #[zbus(name = "GetContractVersion")]
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "GetHardwareSnapshotJSON")]
    fn get_hardware_snapshot_json(&self) -> zbus::Result<String>;
    #[zbus(name = "SetSensorDataCollectionStatus")]
    fn set_sensor_data_collection_status(&self, status: bool) -> zbus::Result<bool>;
    #[zbus(name = "GetSensorDataCollectionStatus")]
    fn get_sensor_data_collection_status(&self) -> zbus::Result<bool>;
    #[zbus(name = "SetChargingProfile")]
    fn set_charging_profile(&self, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetChargingPriority")]
    fn set_charging_priority(&self, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetChargeStartThreshold")]
    fn set_charge_start_threshold(&self, value: i32) -> zbus::Result<bool>;
    #[zbus(name = "SetChargeEndThreshold")]
    fn set_charge_end_threshold(&self, value: i32) -> zbus::Result<bool>;
    #[zbus(name = "SetChargeType")]
    fn set_charge_type(&self, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetKeyboardStatesJSON")]
    fn set_keyboard_states_json(&self, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetLightbarStatesJSON")]
    fn set_lightbar_states_json(&self, value: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetFnLockStatus")]
    fn set_fn_lock_status(&self, enabled: bool) -> zbus::Result<bool>;
}

#[proxy(
    interface = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait LegacyTccd {
    #[zbus(name = "SetTempProfile")]
    fn set_temp_profile(&self, id: &str) -> zbus::Result<bool>;
    #[zbus(name = "ClearTempProfile")]
    fn clear_temp_profile(&self) -> zbus::Result<bool>;
}

struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn build_manager<'a>(
    connection: &'a zbus::Connection,
    service: &str,
) -> HardwareManagerProxy<'a> {
    HardwareManagerProxy::builder(connection)
        .destination(service.to_owned())
        .expect("destination")
        .path("/com/tuxedocomputers/tccd")
        .expect("path")
        .build()
        .await
        .expect("proxy Manager1")
}

#[tokio::test]
async fn manager1_exposes_snapshot_sensors_and_safe_setters() {
    let Ok(connection) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test Manager1 ignoré");
        return;
    };

    let service = format!("com.tuxedocomputers.tccd.HwTest{}", std::process::id());
    let child = Command::new(env!("CARGO_BIN_EXE_tcc-daemon"))
        .args(["--dev-session", "--bus-name", &service])
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("démarrage de tccd --dev-session");
    let _guard = DaemonGuard(child);

    let manager = build_manager(&connection, &service).await;
    let started = Instant::now();
    loop {
        if manager.get_sensor_data_collection_status().await.is_ok() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "tccd de test non démarré"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Version de contrat via la méthode supportée (P-12), sans propriété.
    assert_eq!(
        manager.get_contract_version().await.expect("contrat"),
        tcc_ipc::IPC_CONTRACT_VERSION
    );

    // L'introspection reste inchangée : la propriété historique est toujours
    // déclarée, la nouvelle méthode aussi.
    let reply = connection
        .call_method(
            Some(service.as_str()),
            "/com/tuxedocomputers/tccd",
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await
        .expect("Introspect");
    let xml: String = reply.body().deserialize().expect("XML d'introspection");
    assert!(xml.contains("method name=\"GetContractVersion\""));
    assert!(xml.contains("property name=\"ContractVersion\""));

    // Snapshot : les domaines sont présents même sans matériel, et la collecte
    // de capteurs est désactivée par défaut.
    let snapshot_json = manager
        .get_hardware_snapshot_json()
        .await
        .expect("snapshot JSON");
    let snapshot: Value = serde_json::from_str(&snapshot_json).expect("snapshot valide");
    for key in [
        "charging", "lighting", "odm", "sensors", "nvidia", "webcam", "fnLock", "prime", "ycbcr",
    ] {
        assert!(
            snapshot.get(key).is_some(),
            "clé absente du snapshot: {key}"
        );
    }
    assert_eq!(
        snapshot["sensors"]["enabled"], false,
        "collecte désactivée par défaut"
    );

    // Capteurs à la demande.
    assert!(!manager.get_sensor_data_collection_status().await.unwrap());
    assert!(
        manager
            .set_sensor_data_collection_status(true)
            .await
            .unwrap()
    );
    assert!(manager.get_sensor_data_collection_status().await.unwrap());
    manager
        .set_sensor_data_collection_status(false)
        .await
        .unwrap();

    // Entrées invalides : refusées sans toucher au matériel.
    assert!(manager.set_charge_start_threshold(150).await.is_err());
    assert!(manager.set_charge_start_threshold(-1).await.is_err());
    assert!(manager.set_keyboard_states_json("not-json").await.is_err());
    assert!(
        manager
            .set_keyboard_states_json(r#"[{"brightness":10,"red":300,"green":0,"blue":0}]"#)
            .await
            .is_err()
    );
    assert!(
        manager
            .set_lightbar_states_json(r#"{"brightness":10,"red":0,"green":0,"blue":300}"#)
            .await
            .is_err()
    );
    assert!(manager.set_charge_type("NotAType").await.is_err());

    // Setters valides en mode dry-run : acceptés, non persistants. On utilise
    // une nouvelle connexion car les refus ci-dessus ont consommé le budget du
    // token bucket du premier nom unique.
    let valid_connection = zbus::Connection::session().await.expect("bus session");
    let valid_manager = build_manager(&valid_connection, &service).await;
    assert!(
        valid_manager
            .set_charging_profile("balanced")
            .await
            .unwrap()
    );
    assert!(
        valid_manager
            .set_charging_priority("charging")
            .await
            .unwrap()
    );
    assert!(valid_manager.set_charge_start_threshold(40).await.unwrap());
    assert!(valid_manager.set_charge_end_threshold(80).await.unwrap());
    assert!(valid_manager.set_charge_type("Custom").await.unwrap());
    assert!(
        valid_manager
            .set_keyboard_states_json(r#"[{"mode":0,"brightness":10,"red":1,"green":2,"blue":3}]"#)
            .await
            .unwrap()
    );
    assert!(
        valid_manager
            .set_lightbar_states_json("null")
            .await
            .unwrap()
    );
    assert!(valid_manager.set_fn_lock_status(true).await.unwrap());

    // Compatibilité legacy : SetTempProfile/ClearTempProfile existent toujours.
    let legacy = LegacyTccdProxy::builder(&connection)
        .destination(service.clone())
        .expect("destination legacy")
        .path("/com/tuxedocomputers/tccd")
        .expect("path legacy")
        .build()
        .await
        .expect("proxy legacy");
    assert!(legacy.set_temp_profile("__office__").await.unwrap());
    assert!(legacy.clear_temp_profile().await.unwrap());

    // Rate limit des lectures coûteuses : 5 snapshots puis rejet.
    let read_connection = zbus::Connection::session().await.expect("bus session");
    let read_manager = build_manager(&read_connection, &service).await;
    for index in 0..5 {
        assert!(
            read_manager.get_hardware_snapshot_json().await.is_ok(),
            "lecture {index} refusée dans le burst"
        );
    }
    assert!(
        read_manager.get_hardware_snapshot_json().await.is_err(),
        "la rafale de lectures n'a pas été limitée"
    );

    // Rate limit par nom unique ET UID : les appels précédents ont consommé
    // le bucket UID du processus de test. Attendre le refill (5/s, burst 10)
    // avant de vérifier une nouvelle rafale sur une connexion distincte.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rate_connection = zbus::Connection::session().await.expect("bus session");
    let rate_manager = build_manager(&rate_connection, &service).await;
    for index in 0..10 {
        assert!(
            rate_manager
                .set_sensor_data_collection_status(true)
                .await
                .is_ok(),
            "appel {index} du burst SetSensorDataCollectionStatus refusé"
        );
    }
    let mut limited = false;
    for _ in 0..10 {
        if rate_manager
            .set_sensor_data_collection_status(true)
            .await
            .is_err()
        {
            limited = true;
            break;
        }
    }
    assert!(
        limited,
        "la rafale de SetSensorDataCollectionStatus n'a pas été limitée"
    );
}
