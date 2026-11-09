// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Tests D-Bus `Admin.Hw` pilotés par les fixtures versionnées.
//!
//! Complète `admin_hw_dbus.rs` (contrat, TDP, taille de corps) avec les
//! arguments exacts des fixtures `tests/fixtures/dbus/admin_hw_*.json` :
//! TDP, ODM, cTGP, `PolicyDenied`, `PartialApply`, `NotSupported` et les
//! entrées invalides. Aucun root, aucun polkit, aucun matériel : les backends
//! sont des fakes en mémoire.
//!
//! `PartialApply` est reproduit avec un backend TDP dont le rollback échoue
//! (même scénario que le test unitaire `hw.rs::tests::...partial_apply`),
//! afin de vérifier aussi le **nom D-Bus** de l'erreur.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tcc_admin::{
    Activity, AdminHwService, Authorizer, AuthzDecision, BoundedAdminHwInterface, CtgpBackend,
    OdmBackend, OdmPlatform, OverrideConsent, PpdOwnership, PpdProbe, TdpBackend,
};
use tcc_hw::tdp::{TDP_DESCRIPTORS, TdpLimit};
use tcc_ipc::AdminHwErrorCode;
use zbus::proxy;

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

#[proxy(
    interface = "com.tuxedocomputers.tccd.AdminHw1",
    default_path = "/com/tuxedocomputers/tccd/Admin/Hw"
)]
trait AdminHw1 {
    fn ping(&self) -> zbus::Result<bool>;
    #[zbus(name = "GetTdpValues")]
    fn get_tdp_values(&self) -> zbus::Result<Vec<i32>>;
    #[zbus(name = "SetTdpValues")]
    fn set_tdp_values(&self, values: &[i32]) -> zbus::Result<bool>;
    #[zbus(name = "SetOdmProfile")]
    fn set_odm_profile(&self, name: &str) -> zbus::Result<bool>;
    #[zbus(name = "SetNvidiaCtgpOffset")]
    fn set_nvidia_ctgp_offset(&self, watts: i32) -> zbus::Result<bool>;
}

// ── Fixtures ───────────────────────────────────────────────────────────────

fn read_json(name: &str) -> serde_json::Value {
    let raw = fixtures::read_fixture(name);
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{name}: fixture illisible: {error}"))
}

fn tdp_values(name: &str) -> Vec<i32> {
    let fixture = read_json(name);
    fixture["arguments"]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("{name}: arguments.values absent"))
        .iter()
        .map(|value| {
            value
                .as_i64()
                .unwrap_or_else(|| panic!("{name}: valeur TDP non entière"))
                .try_into()
                .unwrap_or_else(|_| panic!("{name}: valeur TDP hors i32"))
        })
        .collect()
}

fn odm_name(name: &str) -> String {
    let fixture = read_json(name);
    fixture["arguments"]["name"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: arguments.name absent"))
        .to_owned()
}

fn ctgp_watts(name: &str) -> i32 {
    let fixture = read_json(name);
    fixture["arguments"]["watts"]
        .as_i64()
        .unwrap_or_else(|| panic!("{name}: arguments.watts absent"))
        .try_into()
        .unwrap_or_else(|_| panic!("{name}: watts hors i32"))
}

fn expected_error_name(code: AdminHwErrorCode) -> String {
    let fixture = read_json("dbus/admin_hw_errors.json");
    fixture["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("fixture d'erreurs sans tableau cases"))
        .iter()
        .find(|case| case["code"].as_str() == Some(code.as_str()))
        .and_then(|case| case["dbusName"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("cas d'erreur absent de la fixture: {code:?}"))
}

// ── Fakes matériels ────────────────────────────────────────────────────────

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
    writes: Mutex<usize>,
    fail_after: Option<usize>,
    rollback_fails: bool,
}

impl FakeTdp {
    fn with_limits(values: &[(i32, i32, i32)]) -> Self {
        Self {
            limits: Mutex::new(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, (current, min, max))| TdpLimit {
                        descriptor: TDP_DESCRIPTORS[index],
                        current: *current,
                        min: *min,
                        max: *max,
                    })
                    .collect(),
            ),
            writes: Mutex::new(0),
            fail_after: None,
            rollback_fails: false,
        }
    }

    fn with_partial_rollback() -> Self {
        let mut fake = Self::with_limits(&[(35, 5, 100), (45, 5, 100)]);
        fake.fail_after = Some(1);
        fake.rollback_fails = true;
        fake
    }

    fn currents(&self) -> Vec<i32> {
        self.limits
            .lock()
            .map(|guard| guard.iter().map(|limit| limit.current).collect())
            .unwrap_or_default()
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
        let attempt = self.writes.lock().map(|guard| *guard).unwrap_or(0);
        if let Ok(mut writes) = self.writes.lock() {
            *writes += 1;
        }
        if attempt > 0 && self.rollback_fails {
            return Err(tcc_hw::HwError::Io {
                path: std::path::PathBuf::from("/dev/test-tdp"),
                source: std::io::Error::other("rollback refusé"),
            });
        }
        let fail_after = if attempt == 0 { self.fail_after } else { None };
        let mut limits = self
            .limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if values.is_empty() {
            for (index, limit) in limits.iter_mut().enumerate() {
                if fail_after == Some(index) {
                    return Err(tcc_hw::HwError::Io {
                        path: std::path::PathBuf::from("/dev/test-tdp"),
                        source: std::io::Error::other("écriture partielle"),
                    });
                }
                limit.current = limit.max;
            }
            return Ok(limits.clone());
        }
        for (index, value) in values.iter().enumerate() {
            if fail_after == Some(index) {
                return Err(tcc_hw::HwError::Io {
                    path: std::path::PathBuf::from("/dev/test-tdp"),
                    source: std::io::Error::other("écriture partielle"),
                });
            }
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
    fail_apply: bool,
}

impl FakeOdm {
    fn available(choices: &[&str]) -> Self {
        Self {
            platform: Mutex::new(Some(OdmPlatform {
                available: true,
                current: Some("balanced".to_owned()),
                choices: choices.iter().map(|value| (*value).to_owned()).collect(),
            })),
            io: Vec::new(),
            fail_apply: false,
        }
    }

    fn failing() -> Self {
        let mut fake = Self::available(&["power_save", "balanced", "performance"]);
        fake.fail_apply = true;
        fake
    }
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
        if self.fail_apply {
            return Err(tcc_hw::HwError::NotSupported("platform_profile"));
        }
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
    max: Option<i32>,
}

impl FakeCtgp {
    fn new(offset: Option<i32>, max: Option<i32>) -> Self {
        Self {
            offset: Mutex::new(offset),
            max,
        }
    }

    fn current(&self) -> Option<i32> {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl CtgpBackend for FakeCtgp {
    fn offset(&self) -> tcc_hw::Result<Option<i32>> {
        Ok(self.current())
    }

    fn set_offset(&self, watts: i32) -> tcc_hw::Result<()> {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(watts);
        Ok(())
    }

    fn max_offset(&self) -> tcc_hw::Result<Option<i32>> {
        Ok(self.max)
    }
}

fn service(
    tdp: Arc<FakeTdp>,
    odm: Arc<FakeOdm>,
    ctgp: Arc<FakeCtgp>,
    ppd: PpdOwnership,
) -> AdminHwService {
    AdminHwService::new(
        tdp,
        odm,
        ctgp,
        Arc::new(FakePpd(ppd)),
        Arc::new(FakeConsent(false)),
        Arc::new(AllowAll),
        Arc::new(Activity::new()),
    )
}

async fn serve(service: AdminHwService) -> Option<zbus::Connection> {
    let connection = zbus::Connection::session().await.ok()?;
    connection
        .object_server()
        .at(
            tcc_ipc::DBUS_ADMIN_HW_PATH,
            BoundedAdminHwInterface::new(service),
        )
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
        .path(tcc_ipc::DBUS_ADMIN_HW_PATH)
        .ok()?
        .build()
        .await
        .ok()
}

fn error_name(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => panic!("erreur D-Bus inattendue: {other:?}"),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn valid_fixtures_are_applied_over_dbus() {
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[
            (35, 5, 100),
            (45, 5, 100),
            (65, 5, 120),
        ])),
        Arc::new(FakeOdm::available(&[
            "power_save",
            "balanced",
            "performance",
        ])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    let tdp = tdp_values("dbus/admin_hw_tdp.json");
    assert!(proxy.ping().await.expect("Ping"));
    assert!(proxy.set_tdp_values(&tdp).await.expect("SetTdpValues"));
    assert_eq!(proxy.get_tdp_values().await.expect("GetTdpValues"), tdp);

    let odm = odm_name("dbus/admin_hw_odm.json");
    assert!(proxy.set_odm_profile(&odm).await.expect("SetOdmProfile"));

    let ctgp = ctgp_watts("dbus/admin_hw_ctgp.json");
    assert!(
        proxy
            .set_nvidia_ctgp_offset(ctgp)
            .await
            .expect("SetNvidiaCtgpOffset")
    );
}

#[tokio::test]
async fn invalid_tdp_fixtures_return_the_published_codes() {
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[
            (35, 5, 100),
            (45, 5, 100),
            (65, 5, 120),
        ])),
        Arc::new(FakeOdm::available(&["balanced"])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    for (name, expected) in [
        (
            "dbus/invalid/admin_hw_tdp_too_many.json",
            AdminHwErrorCode::InvalidArgs,
        ),
        (
            "dbus/invalid/admin_hw_tdp_out_of_range.json",
            AdminHwErrorCode::Validation,
        ),
    ] {
        let fixture = tdp_values(name);
        let error = proxy.set_tdp_values(&fixture).await.expect_err(name);
        assert_eq!(error_name(&error), expected.dbus_name(), "{name}");
    }
}

#[tokio::test]
async fn empty_tdp_fixture_applies_firmware_maxima() {
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[
            (35, 5, 100),
            (45, 5, 100),
            (65, 5, 120),
        ])),
        Arc::new(FakeOdm::available(&["balanced"])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    let empty = tdp_values("dbus/admin_hw_tdp_empty.json");
    assert!(empty.is_empty());
    assert!(proxy.set_tdp_values(&empty).await.expect("maxima"));
    assert_eq!(
        proxy.get_tdp_values().await.expect("TDP"),
        vec![100, 100, 120]
    );
}

#[tokio::test]
async fn invalid_odm_and_ctgp_fixtures_return_the_published_codes() {
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[(35, 5, 100)])),
        Arc::new(FakeOdm::available(&["balanced", "performance"])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    for name in [
        "dbus/invalid/admin_hw_odm_too_long.json",
        "dbus/invalid/admin_hw_odm_backslash.json",
    ] {
        let fixture = odm_name(name);
        let error = proxy.set_odm_profile(&fixture).await.expect_err(name);
        assert_eq!(
            error_name(&error),
            AdminHwErrorCode::InvalidArgs.dbus_name(),
            "{name}"
        );
    }

    let fixture = ctgp_watts("dbus/invalid/admin_hw_ctgp_out_of_range.json");
    let error = proxy
        .set_nvidia_ctgp_offset(fixture)
        .await
        .expect_err("cTGP hors bornes");
    assert_eq!(error_name(&error), AdminHwErrorCode::Validation.dbus_name());
}

#[tokio::test]
async fn ppd_owned_odm_returns_policy_denied() {
    // PPD possède `platform_profile` : l'écriture est refusée sans bascule
    // silencieuse vers `tuxedo_io`.
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[(35, 5, 100)])),
        Arc::new(FakeOdm::available(&[
            "power_save",
            "balanced",
            "performance",
        ])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Owned,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    let odm = odm_name("dbus/admin_hw_odm.json");
    let error = proxy
        .set_odm_profile(&odm)
        .await
        .expect_err("PPD propriétaire");
    assert_eq!(
        error_name(&error),
        expected_error_name(AdminHwErrorCode::PolicyDenied)
    );
}

#[tokio::test]
async fn absent_odm_backend_returns_not_supported() {
    let Some(server) = serve(service(
        Arc::new(FakeTdp::with_limits(&[(35, 5, 100)])),
        Arc::new(FakeOdm::failing()),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    let odm = odm_name("dbus/admin_hw_odm.json");
    let error = proxy
        .set_odm_profile(&odm)
        .await
        .expect_err("backend absent");
    assert_eq!(
        error_name(&error),
        expected_error_name(AdminHwErrorCode::NotSupported)
    );
}

#[tokio::test]
async fn failed_rollback_returns_partial_apply() {
    let tdp = Arc::new(FakeTdp::with_partial_rollback());
    let Some(server) = serve(service(
        Arc::clone(&tdp),
        Arc::new(FakeOdm::available(&["balanced"])),
        Arc::new(FakeCtgp::new(Some(10), Some(200))),
        PpdOwnership::Absent,
    ))
    .await
    else {
        eprintln!("bus de session absent : test Admin.Hw fixtures ignoré");
        return;
    };
    let client = zbus::Connection::session().await.expect("client");
    let Some(proxy) = proxy(&client, &server).await else {
        panic!("proxy AdminHw1");
    };

    let error = proxy
        .set_tdp_values(&[50, 60])
        .await
        .expect_err("rollback impossible");
    assert_eq!(
        error_name(&error),
        expected_error_name(AdminHwErrorCode::PartialApply)
    );
    // Le premier réglage est resté appliqué, le second n'a pas été écrit.
    assert_eq!(tdp.currents(), vec![50, 45]);
}
