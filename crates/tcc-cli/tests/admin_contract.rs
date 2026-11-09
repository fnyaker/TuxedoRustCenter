// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Test D-Bus de la commande `tcc-cli admin`.
//!
//! Un faux `Admin1` est servi sur le bus de session, **sans propriété
//! `ContractVersion`**. Le CLI doit réussir via `GetContractVersion()` et ne
//! jamais dépendre de `Properties.Get` (A1-11). Sans bus de session, le test
//! est ignoré.

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use tcc_ipc::{DBUS_ADMIN_PATH, DBUS_ADMIN_SERVICE};

struct FakeAdmin;

#[zbus::interface(name = "com.tuxedocomputers.tccd.Admin1")]
impl FakeAdmin {
    async fn ping(&self) -> bool {
        true
    }

    async fn get_version(&self) -> String {
        "fake".to_owned()
    }

    async fn get_contract_version(&self) -> u32 {
        tcc_ipc::ADMIN_CONTRACT_VERSION
    }
}

/// Démarre le faux service ; renvoie `None` si le bus de session ou le nom
/// n'est pas disponible.
fn start_fake_admin() -> Option<(
    tokio::sync::oneshot::Sender<()>,
    std::thread::JoinHandle<()>,
)> {
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    let handle = std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => {
                let _ = ready_tx.send(false);
                return;
            }
        };
        runtime.block_on(async move {
            let connection = match zbus::connection::Builder::session() {
                Ok(builder) => match builder.name(DBUS_ADMIN_SERVICE) {
                    Ok(builder) => builder,
                    Err(_) => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                },
                Err(_) => {
                    let _ = ready_tx.send(false);
                    return;
                }
            };
            let connection = match connection.serve_at(DBUS_ADMIN_PATH, FakeAdmin) {
                Ok(builder) => match builder.build().await {
                    Ok(connection) => connection,
                    Err(_) => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                },
                Err(_) => {
                    let _ = ready_tx.send(false);
                    return;
                }
            };
            let _ = ready_tx.send(true);
            let _ = stop_rx.await;
            drop(connection);
        });
    });

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => Some((stop_tx, handle)),
        Ok(false) | Err(_) => {
            let _ = stop_tx.send(());
            let _ = handle.join();
            None
        }
    }
}

#[test]
fn admin_uses_get_contract_version_without_properties() {
    let Some((stop, handle)) = start_fake_admin() else {
        eprintln!("bus de session absent ou nom admin occupé : test CLI admin ignoré");
        return;
    };

    let output = Command::new(env!("CARGO_BIN_EXE_tcc-cli"))
        .arg("admin")
        .env("TCC_DBUS_BUS", "session")
        .output()
        .expect("exécution de tcc-cli");
    let _ = stop.send(());
    let _ = handle.join();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("contrat=1"), "sortie inattendue: {stdout}");
}
