// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Test d'intégration du limiteur d'écritures D-Bus (R-03/R-05).
//!
//! Le test démarre un `tccd --dev-session` sur le bus de session (aucune
//! écriture matérielle) avec un nom unique, puis effectue les appels sur **une
//! seule connexion** : c'est le cas où le token bucket par nom unique doit
//! rejeter la rafale. Sans bus de session, le test est ignoré silencieusement.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zbus::proxy;

#[proxy(
    interface = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Tccd {
    #[zbus(name = "SetTempProfile")]
    fn set_temp_profile(&self, id: &str) -> zbus::Result<bool>;

    #[zbus(name = "ClearTempProfile")]
    fn clear_temp_profile(&self) -> zbus::Result<bool>;

    #[zbus(name = "Ping")]
    fn ping(&self) -> zbus::Result<bool>;
}

struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn burst_is_limited_on_a_single_connection() {
    let Ok(connection) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test du limiteur ignoré");
        return;
    };

    let service = format!("com.tuxedocomputers.tccd.LimitsTest{}", std::process::id());
    let child = Command::new(env!("CARGO_BIN_EXE_tcc-daemon"))
        .args(["--dev-session", "--bus-name", &service])
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("démarrage de tccd --dev-session");
    let _guard = DaemonGuard(child);

    let proxy = TccdProxy::builder(&connection)
        .destination(service.clone())
        .expect("destination D-Bus")
        .path("/com/tuxedocomputers/tccd")
        .expect("objet D-Bus")
        .build()
        .await
        .expect("proxy D-Bus");

    let started = Instant::now();
    loop {
        if proxy.ping().await.is_ok() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "tccd de test non démarré"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Identifiant trop long : rejeté avant le limiteur et avant résolution.
    let too_long = "x".repeat(65);
    assert!(
        proxy.set_temp_profile(&too_long).await.is_err(),
        "un identifiant de 65 octets doit être rejeté"
    );

    // Rafale sur la même connexion : burst 10, puis LimitsExceeded.
    for index in 0..10 {
        assert!(
            proxy.set_temp_profile("__office__").await.is_ok(),
            "appel {index} dans le burst refusé"
        );
    }
    let mut limited = false;
    for _ in 0..30 {
        if proxy.set_temp_profile("__office__").await.is_err() {
            limited = true;
            break;
        }
    }
    assert!(limited, "la rafale n'a pas été limitée");

    // ClearTempProfile partage le même budget.
    assert!(
        proxy.clear_temp_profile().await.is_err(),
        "ClearTempProfile devrait être limité après la rafale"
    );
}
