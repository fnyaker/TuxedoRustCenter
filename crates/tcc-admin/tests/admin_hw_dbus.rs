// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests D-Bus du service `Admin.Hw` sur le bus de session.
//!
//! Le service réel est monté avec des backends factices : aucun root, aucun
//! polkit, aucun matériel TUXEDO. Sans bus de session, les tests sont ignorés.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tcc_admin::{
    Activity, AdminHwService, Authorizer, AuthzDecision, BoundedAdminHwInterface, CtgpBackend,
    OdmBackend, OdmPlatform, OverrideConsent, PpdOwnership, PpdProbe, TdpBackend,
};
use tcc_hw::tdp::{TDP_DESCRIPTORS, TdpLimit};
use tcc_ipc::{ADMIN_HW_CONTRACT_VERSION, DBUS_ADMIN_HW_PATH};
use zbus::proxy;

#[proxy(
    interface = "com.tuxedocomputers.tccd.AdminHw1",
    default_path = "/com/tuxedocomputers/tccd/Admin/Hw"
)]
trait AdminHw1 {
    fn ping(&self) -> zbus::Result<bool>;
    fn get_version(&self) -> zbus::Result<String>;
    fn get_contract_version(&self) -> zbus::Result<u32>;
    #[zbus(name = "GetTdpValues")]
    fn get_tdp_values(&self) -> zbus::Result<Vec<i32>>;
    #[zbus(name = "SetTdpValues")]
    fn set_tdp_values(&self, values: &[i32]) -> zbus::Result<bool>;
    #[zbus(name = "SetOdmProfile")]
    fn set_odm_profile(&self, name: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetNvidiaCtgpOffset")]
    fn set_nvidia_ctgp_offset(&self, watts: i32) -> zbus::Result<bool>;
    #[zbus(property)]
    fn contract_version(&self) -> zbus::Result<u32>;
}

struct AllowAll;
#[async_trait]
impl Authorizer for AllowAll {
    async fn authorize(&self, _caller: &str, _uid: u32, _action: &str) -> AuthzDecision {
        AuthzDecision::Authorized
    }
}

struct FakePpd(PpdOwnership);
#[async_trait]
impl PpdProbe for FakePpd {
    async fn ownership(&self) -> PpdOwnership {
        self.0
    }
}

struct FakeConsent(bool);
impl OverrideConsent for FakeConsent {
    fn enabled(&self) -> bool {
        self.0
    }
}

#[derive(Debug)]
struct FakeTdp {
    limits: Mutex<Vec<TdpLimit>>,
}

impl FakeTdp {
    fn new() -> Self {
        Self {
            limits: Mutex::new(vec![
                TdpLimit {
                    descriptor: TDP_DESCRIPTORS[0],
                    current: 35,
                    min: 5,
                    max: 100,
                },
                TdpLimit {
                    descriptor: TDP_DESCRIPTORS[1],
                    current: 45,
                    min: 5,
                    max: 100,
                },
            ]),
        }
    }
}

impl TdpBackend for FakeTdp {
    fn limits(&self) -> tcc_hw::Result<Vec<TdpLimit>> {
        Ok(self
            .limits
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default())
    }

    fn set_values(&self, values: &[f64]) -> tcc_hw::Result<Vec<TdpLimit>> {
        let mut limits = self
            .limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if values.is_empty() {
            for limit in &mut *limits {
                limit.current = limit.max;
            }
            return Ok(limits.clone());
        }
        for (index, value) in values.iter().enumerate() {
            if let Some(limit) = limits.get_mut(index) {
                limit.current = *value as i32;
            }
        }
        Ok(limits.clone())
    }
}

#[derive(Debug)]
struct FakeOdm {
    platform: Mutex<Option<OdmPlatform>>,
    io: Vec<String>,
}

impl OdmBackend for FakeOdm {
    fn platform(&self) -> tcc_hw::Result<Option<OdmPlatform>> {
        Ok(self
            .platform
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default())
    }

    fn io_profiles(&self) -> tcc_hw::Result<Vec<String>> {
        Ok(self.io.clone())
    }

    fn apply_platform(&self, name: &str) -> tcc_hw::Result<()> {
        if let Ok(mut platform) = self.platform.lock()
            && let Some(platform) = platform.as_mut()
        {
            platform.current = Some(name.to_owned());
        }
        Ok(())
    }

    fn apply_io(&self, _name: &str) -> tcc_hw::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct FakeCtgp {
    offset: Mutex<Option<i32>>,
}

impl CtgpBackend for FakeCtgp {
    fn offset(&self) -> tcc_hw::Result<Option<i32>> {
        Ok(*self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    fn set_offset(&self, watts: i32) -> tcc_hw::Result<()> {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(watts);
        Ok(())
    }

    fn max_offset(&self) -> tcc_hw::Result<Option<i32>> {
        Ok(Some(200))
    }
}

fn service() -> AdminHwService {
    AdminHwService::new(
        Arc::new(FakeTdp::new()),
        Arc::new(FakeOdm {
            platform: Mutex::new(Some(OdmPlatform {
                available: true,
                current: Some("balanced".to_owned()),
                choices: vec!["power_save".to_owned(), "balanced".to_owned()],
            })),
            io: vec!["enthusiast".to_owned()],
        }),
        Arc::new(FakeCtgp {
            offset: Mutex::new(Some(10)),
        }),
        Arc::new(FakePpd(PpdOwnership::Absent)),
        Arc::new(FakeConsent(false)),
        Arc::new(AllowAll),
        Arc::new(Activity::new()),
    )
}

fn error_name(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => panic!("erreur D-Bus inattendue: {other:?}"),
    }
}

async fn serve() -> Option<zbus::Connection> {
    let connection = zbus::Connection::session().await.ok()?;
    connection
        .object_server()
        .at(DBUS_ADMIN_HW_PATH, BoundedAdminHwInterface::new(service()))
        .await
        .ok()?;
    Some(connection)
}

async fn proxy<'a>(
    client: &'a zbus::Connection,
    service: &zbus::Connection,
) -> Option<AdminHw1Proxy<'a>> {
    let destination = service.unique_name()?.to_string();
    AdminHw1Proxy::builder(client)
        .destination(destination)
        .ok()?
        .path(DBUS_ADMIN_HW_PATH)
        .ok()?
        .build()
        .await
        .ok()
}

#[tokio::test]
async fn contract_and_reads_are_available() {
    let Some(service) = serve().await else {
        eprintln!("bus de session absent : test Admin.Hw ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &service).await else {
        panic!("proxy AdminHw1");
    };

    assert!(proxy.ping().await.expect("Ping"));
    assert!(!proxy.get_version().await.expect("GetVersion").is_empty());
    assert_eq!(
        proxy
            .get_contract_version()
            .await
            .expect("GetContractVersion"),
        ADMIN_HW_CONTRACT_VERSION
    );
    assert_eq!(
        proxy.contract_version().await.expect("ContractVersion"),
        ADMIN_HW_CONTRACT_VERSION
    );
    assert_eq!(
        proxy.get_tdp_values().await.expect("GetTdpValues"),
        vec![35, 45]
    );
    assert!(proxy.set_tdp_values(&[40, 50]).await.expect("SetTdpValues"));
    assert_eq!(
        proxy.get_tdp_values().await.expect("GetTdpValues"),
        vec![40, 50]
    );
}

#[tokio::test]
async fn invalid_tdp_is_a_typed_error() {
    let Some(service) = serve().await else {
        eprintln!("bus de session absent : test Admin.Hw ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &service).await else {
        panic!("proxy AdminHw1");
    };

    let error = proxy
        .set_tdp_values(&[50, 60, 70, 80])
        .await
        .expect_err("trop de valeurs");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.AdminHw1.Error.InvalidArgs"
    );
}

#[tokio::test]
async fn empty_tdp_applies_firmware_maxima() {
    let Some(service) = serve().await else {
        eprintln!("bus de session absent : test Admin.Hw ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &service).await else {
        panic!("proxy AdminHw1");
    };
    assert!(proxy.set_tdp_values(&[]).await.expect("maxima firmware"));
    assert_eq!(proxy.get_tdp_values().await.expect("TDP"), vec![100, 100]);
}

#[tokio::test]
async fn oversized_argument_is_rejected_without_killing_service() {
    let Some(service) = serve().await else {
        eprintln!("bus de session absent : test Admin.Hw ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &service).await else {
        panic!("proxy AdminHw1");
    };

    // 65 Kio > MAX_HW_MESSAGE_BODY_BYTES (64 Kio) : le garde doit refuser
    // avant la désérialisation de la chaîne et le service rester vivant.
    let oversized = "x".repeat(65 * 1024);
    let error = proxy
        .set_odm_profile(&oversized)
        .await
        .expect_err("corps D-Bus surdimensionné");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.AdminHw1.Error.InvalidArgs"
    );
    // Même garde pour un `ai` volumineux (~80 Kio).
    let oversized_values = vec![50i32; 20_000];
    let error = proxy
        .set_tdp_values(&oversized_values)
        .await
        .expect_err("tableau TDP surdimensionné");
    assert_eq!(
        error_name(&error),
        "org.tuxedocomputers.tccd.AdminHw1.Error.InvalidArgs"
    );
    assert!(
        proxy.ping().await.expect("Ping après rejet"),
        "le service doit rester vivant après un message surdimensionné"
    );
}

#[tokio::test]
async fn introspection_lists_phase2_methods_only() {
    let Some(service) = serve().await else {
        eprintln!("bus de session absent : test Admin.Hw ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let destination = service.unique_name().expect("nom unique").to_string();
    let introspectable = zbus::Proxy::new(
        &client,
        destination,
        DBUS_ADMIN_HW_PATH,
        "org.freedesktop.DBus.Introspectable",
    )
    .await
    .expect("proxy introspection");
    let xml: String = introspectable
        .call("Introspect", &())
        .await
        .expect("Introspect");
    for method in [
        "Ping",
        "GetVersion",
        "GetContractVersion",
        "GetTdpValues",
        "SetTdpValues",
        "SetOdmProfile",
        "SetOdmProfileOverride",
        "SetNvidiaCtgpOffset",
    ] {
        assert!(xml.contains(method), "méthode absente: {method}");
    }
    for forbidden in ["SaveProfilesJSON", "SaveSettingsJSON", "ReloadConfig"] {
        assert!(
            !xml.contains(forbidden),
            "méthode phase 1 exposée: {forbidden}"
        );
    }
}
