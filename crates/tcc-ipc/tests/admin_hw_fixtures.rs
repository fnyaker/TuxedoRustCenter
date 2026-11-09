// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Contrat statique `Admin.Hw` (`AdminHw1`) et fixtures de payloads.
//!
//! Les méthodes `Admin.Hw` utilisent des types D-Bus natifs (`ai`, `s`, `i`),
//! pas du JSON. Les fixtures `tests/fixtures/dbus/admin_hw_*.json` décrivent
//! donc les arguments, les réponses attendues et les erreurs du contrat ; ce
//! test les confronte aux constantes publiques de `tcc-ipc` et aux bornes
//! publiées dans `docs/DBUS_API.md` (§ « Service Admin.Hw »).
//!
//! Les bornes par valeur TDP (`1..=1000`) ne sont pas exportées par
//! `tcc-admin` (constantes privées) : elles sont recopiées ici depuis le
//! contrat public, pas depuis l'implémentation.

#[path = "../../../tests/support/fixtures.rs"]
mod fixtures;

use serde::Deserialize;

use tcc_ipc::{
    ADMIN_HW_CONTRACT_VERSION, ADMIN_HW_ERROR_NAMESPACE, AdminAction, AdminHwErrorCode,
    DBUS_ADMIN_HW_INTERFACE, DBUS_ADMIN_HW_PATH, DBUS_ADMIN_HW_SERVICE,
    MAX_CTGP_STATIC_OFFSET_WATTS, MAX_HW_STRING_CHARS, MAX_TDP_VALUES,
};

/// Bornes de syntaxe TDP publiées par `docs/DBUS_API.md` (§ TDP).
const TDP_MIN_WATTS: i64 = 1;
const TDP_MAX_WATTS: i64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum AdminHwMethod {
    #[serde(rename = "SetTdpValues")]
    TdpValues,
    #[serde(rename = "SetOdmProfile")]
    OdmProfile,
    #[serde(rename = "SetNvidiaCtgpOffset")]
    NvidiaCtgpOffset,
}

impl AdminHwMethod {
    fn as_action(self) -> AdminAction {
        match self {
            Self::TdpValues => AdminAction::SetTdpValues,
            Self::OdmProfile => AdminAction::SetOdmProfile,
            Self::NvidiaCtgpOffset => AdminAction::SetNvidiaCtgpOffset,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TdpValues => "SetTdpValues",
            Self::OdmProfile => "SetOdmProfile",
            Self::NvidiaCtgpOffset => "SetNvidiaCtgpOffset",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminHwFixture {
    method: AdminHwMethod,
    arguments: serde_json::Value,
    #[serde(default)]
    expected: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AdminHwErrorFixture {
    namespace: String,
    cases: Vec<AdminHwErrorCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminHwErrorCase {
    code: AdminHwErrorCode,
    method: AdminHwMethod,
    reason: String,
    dbus_name: String,
}

/// Valide les arguments d'une fixture comme le ferait la couche publique.
///
/// Cette validation est volontairement indépendante de `tcc-admin` : elle
/// vérifie le **contrat publié**, pas une copie de l'implémentation.
fn validate_fixture(fixture: &AdminHwFixture) -> Result<(), String> {
    match fixture.method {
        AdminHwMethod::TdpValues => {
            let values = fixture
                .arguments
                .get("values")
                .ok_or_else(|| "values absent".to_owned())?
                .as_array()
                .ok_or_else(|| "values doit être un tableau".to_owned())?;
            if values.len() > MAX_TDP_VALUES {
                return Err(format!(
                    "values contient {} éléments > {MAX_TDP_VALUES}",
                    values.len()
                ));
            }
            let mut previous = 0;
            for value in values {
                let value = value
                    .as_i64()
                    .ok_or_else(|| "chaque valeur doit être un entier".to_owned())?;
                if !(TDP_MIN_WATTS..=TDP_MAX_WATTS).contains(&value) {
                    return Err(format!(
                        "TDP {value} hors de {TDP_MIN_WATTS}..={TDP_MAX_WATTS}"
                    ));
                }
                if value < previous {
                    return Err("la séquence TDP doit être croissante".to_owned());
                }
                previous = value;
            }
        }
        AdminHwMethod::OdmProfile => {
            let name = fixture
                .arguments
                .get("name")
                .ok_or_else(|| "name absent".to_owned())?
                .as_str()
                .ok_or_else(|| "name doit être une chaîne".to_owned())?;
            if name.is_empty() {
                return Err("name ne doit pas être vide".to_owned());
            }
            if name.chars().count() > MAX_HW_STRING_CHARS {
                return Err(format!("name dépasse {MAX_HW_STRING_CHARS} caractères"));
            }
            if !name
                .chars()
                .all(|character| character.is_ascii_graphic() && character != '\\')
            {
                return Err("name doit être ASCII graphique sans barre oblique inverse".to_owned());
            }
        }
        AdminHwMethod::NvidiaCtgpOffset => {
            let watts = fixture
                .arguments
                .get("watts")
                .ok_or_else(|| "watts absent".to_owned())?
                .as_i64()
                .ok_or_else(|| "watts doit être un entier".to_owned())?;
            if !(0..=i64::from(MAX_CTGP_STATIC_OFFSET_WATTS)).contains(&watts) {
                return Err(format!(
                    "cTGP {watts} hors de 0..={MAX_CTGP_STATIC_OFFSET_WATTS}"
                ));
            }
        }
    }

    if let Some(expected) = &fixture.expected
        && expected
            .get("accepted")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err("expected.accepted doit valoir true pour une fixture valide".to_owned());
    }
    Ok(())
}

fn read_fixture(name: &str) -> AdminHwFixture {
    let raw = fixtures::read_fixture(name);
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{name}: fixture Admin.Hw invalide: {error}"))
}

#[test]
fn service_identifiers_are_stable() {
    assert_eq!(DBUS_ADMIN_HW_SERVICE, "com.tuxedocomputers.tccd.Admin.Hw");
    assert_eq!(DBUS_ADMIN_HW_PATH, "/com/tuxedocomputers/tccd/Admin/Hw");
    assert_eq!(DBUS_ADMIN_HW_INTERFACE, "com.tuxedocomputers.tccd.AdminHw1");
    assert_eq!(ADMIN_HW_CONTRACT_VERSION, 1);
    assert_eq!(
        ADMIN_HW_ERROR_NAMESPACE,
        "org.tuxedocomputers.tccd.AdminHw1.Error"
    );
    assert_eq!(MAX_TDP_VALUES, 3);
    assert_eq!(MAX_HW_STRING_CHARS, 64);
    assert_eq!(MAX_CTGP_STATIC_OFFSET_WATTS, 200);
}

#[test]
fn valid_admin_hw_fixtures_match_the_contract() {
    for (name, expected_action) in [
        ("dbus/admin_hw_tdp.json", AdminAction::SetTdpValues),
        ("dbus/admin_hw_tdp_empty.json", AdminAction::SetTdpValues),
        ("dbus/admin_hw_odm.json", AdminAction::SetOdmProfile),
        ("dbus/admin_hw_ctgp.json", AdminAction::SetNvidiaCtgpOffset),
    ] {
        let fixture = read_fixture(name);
        validate_fixture(&fixture).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(fixture.method.as_action(), expected_action, "{name}");
        assert_eq!(
            fixture.method.as_str(),
            expected_action.method_name(),
            "{name}"
        );
    }
}

#[test]
fn invalid_admin_hw_fixtures_are_rejected_by_the_published_bounds() {
    for name in [
        "dbus/invalid/admin_hw_tdp_too_many.json",
        "dbus/invalid/admin_hw_tdp_out_of_range.json",
        "dbus/invalid/admin_hw_odm_too_long.json",
        "dbus/invalid/admin_hw_odm_backslash.json",
        "dbus/invalid/admin_hw_ctgp_out_of_range.json",
        "dbus/invalid/admin_hw_wrong_argument_type.json",
    ] {
        let fixture = read_fixture(name);
        assert!(
            validate_fixture(&fixture).is_err(),
            "{name}: la fixture invalide a été acceptée"
        );
    }
}

#[test]
fn error_fixture_names_match_the_public_enum() {
    let raw = fixtures::read_fixture("dbus/admin_hw_errors.json");
    let fixture: AdminHwErrorFixture = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("fixture d'erreurs Admin.Hw invalide: {error}"));
    assert_eq!(fixture.namespace, ADMIN_HW_ERROR_NAMESPACE);
    assert!(!fixture.cases.is_empty());

    let mut seen = Vec::new();
    for case in &fixture.cases {
        assert_eq!(case.code.dbus_name(), case.dbus_name);
        assert!(
            case.dbus_name
                .starts_with(&format!("{}.", fixture.namespace)),
            "namespace incohérent: {}",
            case.dbus_name
        );
        assert!(!case.reason.is_empty());
        assert!(
            !case.reason.chars().any(char::is_whitespace),
            "le motif d'erreur doit être un code stable: {}",
            case.reason
        );
        assert_eq!(case.method.as_action().method_name(), case.method.as_str());
        seen.push(case.code);
    }
    for expected in [
        AdminHwErrorCode::PolicyDenied,
        AdminHwErrorCode::PartialApply,
        AdminHwErrorCode::NotSupported,
    ] {
        assert!(
            seen.contains(&expected),
            "cas d'erreur absent: {expected:?}"
        );
    }
}

#[test]
fn unknown_admin_hw_error_is_rejected() {
    let raw = fixtures::read_fixture("dbus/invalid/admin_hw_unknown_error.json");
    let error = serde_json::from_str::<AdminHwErrorFixture>(&raw)
        .expect_err("un code d'erreur inconnu doit être refusé");
    assert!(
        error.to_string().contains("unknown variant"),
        "erreur inattendue: {error}"
    );
}

#[test]
fn all_error_codes_stay_in_the_admin_hw_namespace() {
    for code in AdminHwErrorCode::ALL {
        let name = code.dbus_name();
        assert!(name.starts_with(ADMIN_HW_ERROR_NAMESPACE), "{name}");
        assert!(
            !name.starts_with("org.freedesktop.DBus.Error"),
            "{name} ne doit pas utiliser le namespace D-Bus générique"
        );
    }
}
