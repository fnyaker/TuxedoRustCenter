// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests de `Control1.ReloadConfig` sur le bus de session.
//!
//! Le daemon de test tourne avec `--dev-session` (aucune écriture matérielle).
//! La vérification root est faite côté daemon : un appelant non root doit
//! recevoir `AccessDenied`. Le test root réel est hors CI (bus système et
//! privilèges).

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zbus::proxy;

#[proxy(
    interface = "com.tuxedocomputers.tccd.Control1",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Control1 {
    #[zbus(name = "ReloadConfig")]
    fn reload_config(&self) -> zbus::Result<String>;
}

#[proxy(
    interface = "com.tuxedocomputers.tccd",
    default_path = "/com/tuxedocomputers/tccd"
)]
trait Tccd {
    fn ping(&self) -> zbus::Result<bool>;
}

struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(|value| value.parse::<u32>().ok())
        })
        .is_some_and(|uid| uid == 0)
}

#[tokio::test]
async fn control1_reload_is_denied_to_non_root() {
    if running_as_root() {
        eprintln!("test non-root ignoré : processus root");
        return;
    }
    let Ok(connection) = zbus::Connection::session().await else {
        eprintln!("bus de session absent : test Control1 ignoré");
        return;
    };

    let service = format!("com.tuxedocomputers.tccd.ControlTest{}", std::process::id());
    let child = Command::new(env!("CARGO_BIN_EXE_tcc-daemon"))
        .args(["--dev-session", "--bus-name", &service])
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("démarrage de tccd --dev-session");
    let _guard = DaemonGuard(child);

    let legacy = TccdProxy::builder(&connection)
        .destination(service.clone())
        .expect("destination")
        .path("/com/tuxedocomputers/tccd")
        .expect("path")
        .build()
        .await
        .expect("proxy tccd");
    let started = Instant::now();
    loop {
        if legacy.ping().await.is_ok() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "tccd de test non démarré"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let control = Control1Proxy::builder(&connection)
        .destination(service)
        .expect("destination")
        .path("/com/tuxedocomputers/tccd")
        .expect("path")
        .build()
        .await
        .expect("proxy Control1");

    let error = control
        .reload_config()
        .await
        .expect_err("un appelant non root doit être refusé");
    match error {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.to_string(), "org.freedesktop.DBus.Error.AccessDenied");
        }
        other => panic!("erreur inattendue: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "nécessite root et le bus système ; appel de rechargement réel"]
async fn control1_reload_succeeds_for_root_on_system_bus() {
    // Procédure de validation manuelle : root sur le bus système doit obtenir
    // un JSON `AdminReloadReport` valide. Le test est ignoré pour ne jamais
    // toucher la configuration d'une machine de CI.
    let connection = zbus::Connection::system().await.expect("bus système");
    let control = Control1Proxy::builder(&connection)
        .destination("com.tuxedocomputers.tccd")
        .expect("destination")
        .path("/com/tuxedocomputers/tccd")
        .expect("path")
        .build()
        .await
        .expect("proxy Control1");
    let json = control.reload_config().await.expect("ReloadConfig root");
    let report =
        tcc_ipc::AdminReloadReport::from_json_bounded(&json, 1024 * 1024).expect("rapport valide");
    let _ = report;
}
