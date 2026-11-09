// SPDX-License-Identifier: GPL-3.0-or-later
//! Politique de lecture des contrôles UVC.
//!
//! # Allowlist normative
//!
//! Seuls **15 CIDs** sont exposés, quelle que soit l'énumération du pilote.
//! Un contrôle énuméré mais absent de [`cid_allowed`] est ignoré (pas de
//! valeur lue, pas de menu, pas de JSON). C'est la barrière prévue par le
//! finding V-01 de `docs/security/REVIEW_V4L2.md`.
//!
//! # Normalisation des noms
//!
//! La conversion `name -> clé` est dérivée de `cameractrls.py`
//! (`TUXEDO Control Center`, MIT, copyright soyer) :
//! minuscules, espace et `-` → `_`, suppression de `, & ( ) . /`, `__` → `_`.
//! Elle est ensuite restreinte à `[a-z0-9_]` pour rester stable en JSON.
//!
//! # Attribution
//!
//! La normalisation reproduit une partie de `cameractrls.py`
//! (<https://github.com/soyersoyer/cameractrls>), distribué sous licence MIT.
//! Cette licence est compatible avec GPL-3.0-or-later ; l'attribution est
//! conservée ici et dans `docs/hardware/V4L2_DESIGN.md`.

#![forbid(unsafe_code)]

use super::abi;

/// CID `V4L2_CID_BRIGHTNESS`.
pub(crate) const CID_BRIGHTNESS: u32 = 0x0098_0900;
/// CID `V4L2_CID_CONTRAST`.
pub(crate) const CID_CONTRAST: u32 = 0x0098_0901;
/// CID `V4L2_CID_SATURATION`.
pub(crate) const CID_SATURATION: u32 = 0x0098_0902;
/// CID `V4L2_CID_HUE`.
pub(crate) const CID_HUE: u32 = 0x0098_0903;
/// CID `V4L2_CID_GAMMA`.
pub(crate) const CID_GAMMA: u32 = 0x0098_0910;
/// CID `V4L2_CID_EXPOSURE`.
pub(crate) const CID_EXPOSURE: u32 = 0x0098_0911;
/// CID `V4L2_CID_GAIN`.
pub(crate) const CID_GAIN: u32 = 0x0098_0913;
/// CID `V4L2_CID_POWER_LINE_FREQUENCY`.
pub(crate) const CID_POWER_LINE_FREQUENCY: u32 = 0x0098_0918;
/// CID `V4L2_CID_WHITE_BALANCE_TEMPERATURE`.
pub(crate) const CID_WHITE_BALANCE_TEMPERATURE: u32 = 0x0098_091a;
/// CID `V4L2_CID_SHARPNESS`.
pub(crate) const CID_SHARPNESS: u32 = 0x0098_091b;
/// CID `V4L2_CID_BACKLIGHT_COMPENSATION`.
pub(crate) const CID_BACKLIGHT_COMPENSATION: u32 = 0x0098_091c;
/// CID `V4L2_CID_EXPOSURE_ABSOLUTE`.
pub(crate) const CID_EXPOSURE_ABSOLUTE: u32 = 0x009a_0902;
/// CID `V4L2_CID_FOCUS_ABSOLUTE`.
pub(crate) const CID_FOCUS_ABSOLUTE: u32 = 0x009a_090a;
/// CID `V4L2_CID_FOCUS_AUTO`.
pub(crate) const CID_FOCUS_AUTO: u32 = 0x009a_090c;
/// CID `V4L2_CID_ZOOM_ABSOLUTE`.
pub(crate) const CID_ZOOM_ABSOLUTE: u32 = 0x009a_090d;

/// Liste normative des CIDs lisibles, dans l'ordre de la documentation.
///
/// Cette liste est **fermée** : toute extension passe par une revue
/// `security` et une mise à jour de `docs/hardware/V4L2_DESIGN.md`.
const ALLOWED_CIDS: [u32; 15] = [
    CID_BRIGHTNESS,
    CID_CONTRAST,
    CID_SATURATION,
    CID_HUE,
    CID_GAMMA,
    CID_WHITE_BALANCE_TEMPERATURE,
    CID_EXPOSURE_ABSOLUTE,
    CID_EXPOSURE,
    CID_GAIN,
    CID_SHARPNESS,
    CID_FOCUS_ABSOLUTE,
    CID_FOCUS_AUTO,
    CID_POWER_LINE_FREQUENCY,
    CID_BACKLIGHT_COMPENSATION,
    CID_ZOOM_ABSOLUTE,
];

/// Vrai si le CID appartient à l'allowlist normative.
///
/// Les CIDs privés (`>= 0x0800_0000`) sont toujours refusés, même s'ils
/// figurent un jour dans une table constructeur : cette fonction ne consulte
/// aucune donnée device.
#[must_use]
pub fn cid_allowed(cid: u32) -> bool {
    ALLOWED_CIDS.contains(&cid)
}

/// Liste des CIDs autorisés (tests et diagnostic).
#[must_use]
pub fn allowed_cids() -> &'static [u32] {
    &ALLOWED_CIDS
}

/// Type de contrôle UVC exposé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvcControlKind {
    /// Entier borné.
    Integer,
    /// Booléen.
    Boolean,
    /// Menu avec libellés.
    Menu,
    /// Menu avec valeurs entières 64 bits.
    IntegerMenu,
}

impl UvcControlKind {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Menu => "menu",
            Self::IntegerMenu => "integerMenu",
        }
    }
}

/// Catégorie d'affichage (reprise de `cameractrls.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvcCategory {
    /// Zoom, focus.
    Basic,
    /// Exposition, gain, compensation.
    Exposure,
    /// Balance des blancs, luminosité, contraste, couleur.
    Color,
    /// Fréquence de ligne, confidentialité, rotation.
    Advanced,
    /// Non classé.
    Other,
}

impl UvcCategory {
    /// Nom stable exposé à l'UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Exposure => "exposure",
            Self::Color => "color",
            Self::Advanced => "advanced",
            Self::Other => "other",
        }
    }
}

/// Entrée de menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcMenuEntry {
    /// Index V4L2 (valeur envoyée à `S_CTRL` dans une future étape).
    pub index: i64,
    /// Clé stable (libellé normalisé, ou valeur entière pour `INTEGER_MENU`).
    pub key: String,
    /// Libellé noyau sanitisé.
    pub label: String,
}

/// Valeur courante d'un contrôle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UvcValue {
    /// Entier.
    Integer(i64),
    /// Booléen.
    Boolean(bool),
    /// Menu : index V4L2, clé stable et libellé.
    Menu {
        /// Index du menu.
        index: i64,
        /// Clé stable.
        key: String,
        /// Libellé.
        label: String,
    },
}

/// Drapeaux d'un contrôle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UvcFlags {
    /// Contrôle inactif (grisé).
    pub inactive: bool,
    /// Contrôle en lecture seule.
    pub read_only: bool,
    /// Contrôle en écriture seule (valeur illisible).
    pub write_only: bool,
    /// Écriture qui met à jour d'autres contrôles.
    pub update: bool,
    /// Contrôle désactivé.
    pub disabled: bool,
    /// Contrôle verrouillé par une autre application.
    pub grabbed: bool,
}

impl UvcFlags {
    /// Décode les drapeaux noyau.
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        Self {
            inactive: raw & abi::V4L2_CTRL_FLAG_INACTIVE != 0,
            read_only: raw & abi::V4L2_CTRL_FLAG_READ_ONLY != 0,
            write_only: raw & abi::V4L2_CTRL_FLAG_WRITE_ONLY != 0,
            update: raw & abi::V4L2_CTRL_FLAG_UPDATE != 0,
            disabled: raw & abi::V4L2_CTRL_FLAG_DISABLED != 0,
            grabbed: raw & abi::V4L2_CTRL_FLAG_GRABBED != 0,
        }
    }

    /// Vrai si un contrôle peut être lu et affiché.
    #[must_use]
    pub fn is_readable(self) -> bool {
        !self.disabled && !self.grabbed && !self.write_only
    }

    /// Vrai si un contrôle peut être écrit sans ambiguïté.
    ///
    /// `inactive` est refusé : le mode courant (exposition automatique,
    /// balance des blancs automatique) ne garantit pas que l'écriture sera
    /// honorée. L'appelant doit d'abord désactiver le mode automatique via un
    /// contrôle dédié de l'allowlist si nécessaire.
    #[must_use]
    pub fn is_writable(self) -> bool {
        !self.disabled && !self.grabbed && !self.read_only && !self.write_only && !self.inactive
    }

    /// Vrai si le contrôle est une action implicite ou un contrôle composé.
    #[must_use]
    pub fn is_action_or_payload(raw: u32) -> bool {
        raw & (abi::V4L2_CTRL_FLAG_HAS_PAYLOAD | abi::V4L2_CTRL_FLAG_EXECUTE_ON_WRITE) != 0
    }
}

/// Contrôle exposé à l'UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcControlInfo {
    /// Clé stable (`brightness`, `focus_auto`, ...).
    pub key: String,
    /// Nom noyau sanitisé.
    pub name: String,
    /// Type exposé.
    pub kind: UvcControlKind,
    /// Borne basse.
    pub min: i64,
    /// Borne haute.
    pub max: i64,
    /// Pas.
    pub step: i64,
    /// Valeur par défaut.
    pub default: i64,
    /// Valeur courante, `None` si illisible.
    pub current: Option<UvcValue>,
    /// Entrées de menu (vide pour un entier).
    pub menu: Vec<UvcMenuEntry>,
    /// Drapeaux noyau.
    pub flags: UvcFlags,
    /// Catégorie d'affichage.
    pub category: UvcCategory,
    /// Visible en mode simple.
    pub simple: bool,
}

/// Convertit le type brut V4L2 en type exposé.
#[must_use]
pub fn kind_from_raw(raw: u32) -> Option<UvcControlKind> {
    match raw {
        abi::V4L2_CTRL_TYPE_INTEGER => Some(UvcControlKind::Integer),
        abi::V4L2_CTRL_TYPE_BOOLEAN => Some(UvcControlKind::Boolean),
        abi::V4L2_CTRL_TYPE_MENU => Some(UvcControlKind::Menu),
        abi::V4L2_CTRL_TYPE_INTEGER_MENU => Some(UvcControlKind::IntegerMenu),
        abi::V4L2_CTRL_TYPE_BUTTON => None,
        _ => None,
    }
}

/// Normalise un `INTEGER` 0/1/step 1 en booléen, comme l'original.
#[must_use]
pub fn normalize_kind(kind: UvcControlKind, min: i32, max: i32, step: i32) -> UvcControlKind {
    if kind == UvcControlKind::Integer && min == 0 && max == 1 && step == 1 {
        UvcControlKind::Boolean
    } else {
        kind
    }
}

/// Catégorie d'affichage d'un CID.
#[must_use]
pub fn category_for(cid: u32) -> UvcCategory {
    match cid {
        CID_ZOOM_ABSOLUTE | CID_FOCUS_ABSOLUTE | CID_FOCUS_AUTO => UvcCategory::Basic,
        CID_EXPOSURE | CID_EXPOSURE_ABSOLUTE | CID_GAIN | CID_BACKLIGHT_COMPENSATION => {
            UvcCategory::Exposure
        }
        CID_BRIGHTNESS
        | CID_CONTRAST
        | CID_SATURATION
        | CID_HUE
        | CID_GAMMA
        | CID_WHITE_BALANCE_TEMPERATURE
        | CID_SHARPNESS => UvcCategory::Color,
        CID_POWER_LINE_FREQUENCY => UvcCategory::Advanced,
        _ => UvcCategory::Other,
    }
}

/// Vrai si le contrôle doit apparaître dans le mode simple.
#[must_use]
pub fn is_simple(cid: u32) -> bool {
    matches!(cid, CID_BRIGHTNESS | CID_CONTRAST | CID_SATURATION)
}

/// Nettoie une chaîne issue du matériel : caractères de contrôle supprimés,
/// taille bornée en caractères, bords trimés.
#[must_use]
pub(crate) fn sanitize_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Convertit un nom noyau en clé stable.
///
/// Dérivé de `cameractrls.py` (MIT) : voir l'en-tête du module.
#[must_use]
pub(crate) fn to_key(name: &str, cid: u32) -> String {
    let sanitized = sanitize_text(name, 64);
    let mut key = String::with_capacity(sanitized.len());
    for character in sanitized.chars() {
        let lower = character.to_ascii_lowercase();
        match lower {
            ' ' | '-' => key.push('_'),
            ',' | '&' | '(' | ')' | '.' | '/' => {}
            'a'..='z' | '0'..='9' | '_' => key.push(lower),
            _ => {}
        }
    }
    while key.contains("__") {
        key = key.replace("__", "_");
    }
    let key = key.trim_matches('_');
    if key.is_empty() {
        // Repli interne, jamais exposé comme identifiant noyau brut.
        format!("control_{cid:x}")
    } else {
        key.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_is_exactly_the_documented_set() {
        assert_eq!(allowed_cids().len(), 15);
        for cid in allowed_cids() {
            assert!(cid_allowed(*cid), "CID 0x{cid:08x} absent");
        }
        // Quelques contrôles énumérables mais hors allowlist.
        assert!(!cid_allowed(0x009f_0903)); // V4L2_CID_TEST_PATTERN
        assert!(!cid_allowed(0x0098_0904)); // V4L2_CID_AUTO_WHITE_BALANCE
        assert!(!cid_allowed(0x009a_0901)); // V4L2_CID_EXPOSURE_AUTO
        assert!(!cid_allowed(0x0800_0000)); // privé
        assert!(!cid_allowed(0x0800_0001)); // privé
        assert!(!cid_allowed(0));
    }

    #[test]
    fn names_are_normalized_like_the_original() {
        assert_eq!(to_key("Brightness", CID_BRIGHTNESS), "brightness");
        assert_eq!(to_key("Auto Exposure", CID_EXPOSURE), "auto_exposure");
        assert_eq!(
            to_key(
                "White Balance Temperature, Auto",
                CID_WHITE_BALANCE_TEMPERATURE
            ),
            "white_balance_temperature_auto"
        );
        assert_eq!(
            to_key("Backlight Compensation", CID_BACKLIGHT_COMPENSATION),
            "backlight_compensation"
        );
        assert_eq!(
            to_key("Power Line Frequency", CID_POWER_LINE_FREQUENCY),
            "power_line_frequency"
        );
        assert_eq!(to_key("!!!", 0x1234), "control_1234");
        assert_eq!(to_key("A  B", 1), "a_b");
    }

    #[test]
    fn text_is_sanitized_and_bounded() {
        assert_eq!(sanitize_text("  Hi\nThere\t ", 32), "HiThere");
        assert_eq!(sanitize_text("abc\0def", 32), "abcdef");
        assert_eq!(sanitize_text(&"x".repeat(200), 32).chars().count(), 32);
        assert_eq!(sanitize_text("   ", 32), "");
    }

    #[test]
    fn kinds_and_flags_are_filtered() {
        assert_eq!(kind_from_raw(1), Some(UvcControlKind::Integer));
        assert_eq!(kind_from_raw(2), Some(UvcControlKind::Boolean));
        assert_eq!(kind_from_raw(3), Some(UvcControlKind::Menu));
        assert_eq!(kind_from_raw(9), Some(UvcControlKind::IntegerMenu));
        assert_eq!(kind_from_raw(4), None); // BUTTON
        assert_eq!(kind_from_raw(5), None); // INTEGER64
        assert_eq!(
            normalize_kind(UvcControlKind::Integer, 0, 1, 1),
            UvcControlKind::Boolean
        );
        assert_eq!(
            normalize_kind(UvcControlKind::Integer, 0, 2, 1),
            UvcControlKind::Integer
        );

        let readable = UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_READ_ONLY);
        assert!(readable.read_only);
        assert!(readable.is_readable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_DISABLED).is_readable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_GRABBED).is_readable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_WRITE_ONLY).is_readable());
        assert!(UvcFlags::default().is_writable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_READ_ONLY).is_writable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_WRITE_ONLY).is_writable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_INACTIVE).is_writable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_GRABBED).is_writable());
        assert!(!UvcFlags::from_raw(abi::V4L2_CTRL_FLAG_DISABLED).is_writable());
        assert!(UvcFlags::is_action_or_payload(
            abi::V4L2_CTRL_FLAG_EXECUTE_ON_WRITE
        ));
        assert!(UvcFlags::is_action_or_payload(
            abi::V4L2_CTRL_FLAG_HAS_PAYLOAD
        ));
    }

    #[test]
    fn categories_and_simple_match_the_design() {
        assert_eq!(category_for(CID_BRIGHTNESS), UvcCategory::Color);
        assert_eq!(category_for(CID_EXPOSURE_ABSOLUTE), UvcCategory::Exposure);
        assert_eq!(category_for(CID_FOCUS_AUTO), UvcCategory::Basic);
        assert_eq!(
            category_for(CID_POWER_LINE_FREQUENCY),
            UvcCategory::Advanced
        );
        assert!(is_simple(CID_BRIGHTNESS));
        assert!(is_simple(CID_CONTRAST));
        assert!(is_simple(CID_SATURATION));
        assert!(!is_simple(CID_GAMMA));
    }

    #[test]
    fn kind_labels_are_stable() {
        assert_eq!(UvcControlKind::Integer.as_str(), "integer");
        assert_eq!(UvcControlKind::Boolean.as_str(), "boolean");
        assert_eq!(UvcControlKind::Menu.as_str(), "menu");
        assert_eq!(UvcControlKind::IntegerMenu.as_str(), "integerMenu");
        assert_eq!(UvcCategory::Basic.as_str(), "basic");
    }
}
