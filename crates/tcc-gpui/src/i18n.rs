// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Textes de l'interface : anglais (repli), français et allemand.
//!
//! Le dictionnaire principal ([`crate::strings`]) est celui de l'interface
//! Qt : mêmes clés, mêmes textes, paramètres `%1`..`%9`. [`EXTRA`] ne contient
//! que les textes propres à l'interface gpui.
//!
//! La langue vient du réglage enregistré, sinon de `LC_ALL`, `LC_MESSAGES`,
//! `LANGUAGE` puis `LANG`. Elle se change à chaud avec [`set_lang`].

use std::sync::atomic::{AtomicU8, Ordering};

use crate::strings::STRINGS;

/// Langue de l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
    De,
}

impl Lang {
    /// Langues proposées, dans l'ordre du sélecteur.
    pub const ALL: [Self; 3] = [Self::Fr, Self::En, Self::De];

    /// Code ISO (`en`, `fr`, `de`).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
            Self::De => "de",
        }
    }

    /// Nom de la langue, dans cette langue.
    #[must_use]
    pub fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Fr => "Français",
            Self::De => "Deutsch",
        }
    }

    /// Langue d'un code ISO ou d'une locale (`fr`, `fr_FR.UTF-8`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.to_ascii_lowercase();
        if raw.starts_with("fr") {
            Some(Self::Fr)
        } else if raw.starts_with("de") {
            Some(Self::De)
        } else if raw.starts_with("en") {
            Some(Self::En)
        } else {
            None
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::En => 0,
            Self::Fr => 1,
            Self::De => 2,
        }
    }
}

/// Textes propres à l'interface gpui : `(clé, en, fr, de)`, trié par clé.
const EXTRA: &[(&str, &str, &str, &str)] = &[
    (
        "gpui.autostartSaved",
        "Autostart updated",
        "Démarrage automatique mis à jour",
        "Autostart aktualisiert",
    ),
    ("gpui.battery", "Battery", "Batterie", "Akku"),
    ("gpui.batteryCharge", "Charge", "Charge", "Ladestand"),
    (
        "gpui.curveSelected",
        "Selected point",
        "Point sélectionné",
        "Ausgewählter Punkt",
    ),
    (
        "gpui.displayBackend",
        "Display backend",
        "Backend d'affichage",
        "Anzeige-Backend",
    ),
    (
        "gpui.displayCurrent",
        "Current mode",
        "Mode courant",
        "Aktueller Modus",
    ),
    (
        "gpui.displayModes",
        "Available modes",
        "Modes disponibles",
        "Verfügbare Modi",
    ),
    ("gpui.fanNumber", "Fan %1", "Ventilateur %1", "Lüfter %1"),
    (
        "gpui.noBattery",
        "No battery detected",
        "Aucune batterie détectée",
        "Kein Akku erkannt",
    ),
    (
        "gpui.quickSwitch",
        "Switch profile",
        "Changer de profil",
        "Profil wechseln",
    ),
    (
        "gpui.removePoint",
        "Remove point",
        "Supprimer le point",
        "Punkt entfernen",
    ),
    (
        "gpui.themeFollowsSystem",
        "Colors follow the current Omarchy theme.",
        "Les couleurs suivent le thème Omarchy courant.",
        "Die Farben folgen dem aktuellen Omarchy-Design.",
    ),
    ("gpui.view", "View", "Voir", "Ansehen"),
];

static LANG: AtomicU8 = AtomicU8::new(0);

fn detect(vars: &[Option<String>]) -> Lang {
    for value in vars.iter().flatten() {
        let first = value.split(':').next().unwrap_or("");
        if first.is_empty()
            || first.eq_ignore_ascii_case("c")
            || first.eq_ignore_ascii_case("posix")
        {
            continue;
        }
        return Lang::parse(first).unwrap_or(Lang::En);
    }
    Lang::En
}

/// Initialise la langue : `saved` (réglage enregistré) prime sur
/// l'environnement.
pub fn init(saved: Option<Lang>) {
    let lang = saved.unwrap_or_else(|| {
        detect(&[
            std::env::var("LC_ALL").ok(),
            std::env::var("LC_MESSAGES").ok(),
            std::env::var("LANGUAGE").ok(),
            std::env::var("LANG").ok(),
        ])
    });
    set_lang(lang);
}

/// Change la langue de l'interface.
pub fn set_lang(lang: Lang) {
    LANG.store(lang.index(), Ordering::Relaxed);
}

/// Langue courante.
#[must_use]
pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Fr,
        2 => Lang::De,
        _ => Lang::En,
    }
}

fn find(table: &'static [(&str, &str, &str, &str)], lang: Lang, key: &str) -> Option<&'static str> {
    let index = table.binary_search_by(|row| row.0.cmp(key)).ok()?;
    let row = table.get(index)?;
    Some(match lang {
        Lang::En => row.1,
        Lang::Fr => row.2,
        Lang::De => row.3,
    })
}

fn lookup(lang: Lang, key: &str) -> Option<&'static str> {
    find(STRINGS, lang, key).or_else(|| find(EXTRA, lang, key))
}

/// Vrai si `key` existe (utile pour les libellés dérivés d'une valeur).
#[must_use]
pub fn has(key: &str) -> bool {
    lookup(Lang::En, key).is_some()
}

/// Texte de `key` dans la langue courante (la clé elle-même si inconnue).
#[must_use]
pub fn t(key: &str) -> String {
    lookup(lang(), key).unwrap_or(key).to_owned()
}

/// Comme [`t`], en remplaçant `%1`, `%2`… par les arguments.
#[must_use]
pub fn tf(key: &str, args: &[&str]) -> String {
    let mut text = t(key);
    for (index, value) in args.iter().enumerate() {
        text = text.replacen(&format!("%{}", index + 1), value, 1);
    }
    text
}

/// `n profil` / `n profils`.
#[must_use]
pub fn plural(n: usize, one: &str, many: &str) -> String {
    t(if n > 1 { many } else { one })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn placeholders(text: &str) -> BTreeSet<char> {
        let mut out = BTreeSet::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(digit) = chars.peek().filter(|d| d.is_ascii_digit())
            {
                out.insert(*digit);
            }
        }
        out
    }

    #[test]
    fn tables_are_sorted_complete_and_consistent() {
        for table in [STRINGS, EXTRA] {
            for pair in table.windows(2) {
                assert!(pair[0].0 < pair[1].0, "ordre ou doublon : {}", pair[1].0);
            }
            for (key, en, fr, de) in table {
                for text in [en, fr, de] {
                    assert!(!text.is_empty(), "{key} : traduction vide");
                }
                assert_eq!(placeholders(en), placeholders(fr), "{key} : paramètres fr");
                assert_eq!(placeholders(en), placeholders(de), "{key} : paramètres de");
            }
        }
        for (key, ..) in EXTRA {
            assert!(find(STRINGS, Lang::En, key).is_none(), "{key} déjà partagé");
        }
    }

    #[test]
    fn every_key_used_in_the_code_exists() {
        let sources = [
            include_str!("app.rs"),
            include_str!("worker.rs"),
            include_str!("model.rs"),
            include_str!("widgets.rs"),
            include_str!("fan_curve.rs"),
            include_str!("pages/dashboard.rs"),
            include_str!("pages/profiles.rs"),
            include_str!("pages/editor.rs"),
            include_str!("pages/settings.rs"),
            include_str!("pages/tools.rs"),
            include_str!("pages/info.rs"),
        ];
        let mut checked = 0;
        for source in sources {
            for marker in ["t(\"", "tf(\"", "tr(\"", "has(\""] {
                for piece in source.split(marker).skip(1) {
                    let key = piece.split('"').next().unwrap_or("");
                    // Les préfixes dynamiques se terminent par un point.
                    if key.contains('.') && !key.ends_with('.') && !key.contains(' ') {
                        assert!(lookup(Lang::En, key).is_some(), "clé inconnue : {key}");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 200, "trop peu de clés trouvées : {checked}");
    }

    /// Le dictionnaire partagé doit rester identique à celui de l'interface
    /// Qt. Sans les sources Qt (archive partielle), le test ne vérifie rien.
    #[test]
    fn shared_strings_match_the_qt_table() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tcc-gui/qml/core/Strings.qml"
        );
        let Ok(source) = std::fs::read_to_string(path) else {
            return;
        };
        let mut qt: BTreeMap<String, [String; 3]> = BTreeMap::new();
        let mut slot = None;
        for line in source.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("readonly property var ") {
                slot = match rest.split(':').next() {
                    Some("en") => Some(0),
                    Some("fr") => Some(1),
                    Some("de") => Some(2),
                    _ => None,
                };
                continue;
            }
            let (Some(slot), Some(rest)) = (slot, line.strip_prefix('"')) else {
                continue;
            };
            let Some((key, value)) = rest.split_once("\": ") else {
                continue;
            };
            let value = value.trim_end_matches(',');
            let Ok(text) = serde_json::from_str::<String>(value) else {
                continue;
            };
            qt.entry(key.to_owned()).or_default()[slot] = text;
        }
        assert_eq!(qt.len(), STRINGS.len(), "nombre de clés");
        for (key, en, fr, de) in STRINGS {
            let row = qt.get(*key).unwrap_or_else(|| panic!("{key} absent de Qt"));
            assert_eq!(
                [row[0].as_str(), row[1].as_str(), row[2].as_str()],
                [*en, *fr, *de]
            );
        }
    }

    #[test]
    fn detects_the_language() {
        let v = |s: &str| Some(s.to_owned());
        assert_eq!(detect(&[v("fr_FR.UTF-8")]), Lang::Fr);
        assert_eq!(detect(&[None, v("de_DE.UTF-8")]), Lang::De);
        assert_eq!(detect(&[v("C"), v("de")]), Lang::De);
        assert_eq!(detect(&[v("ja_JP.UTF-8")]), Lang::En);
        assert_eq!(detect(&[None, None]), Lang::En);
        assert_eq!(detect(&[v("fr:en")]), Lang::Fr);
        assert_eq!(Lang::parse("xx"), None);
    }

    #[test]
    fn substitutes_positional_parameters() {
        assert_eq!(lookup(Lang::En, "fanCurve.count"), Some("%1 / %2 points"));
        let text = lookup(Lang::En, "fanCurve.count")
            .map(|raw| raw.replacen("%1", "3", 1).replacen("%2", "256", 1));
        assert_eq!(text.as_deref(), Some("3 / 256 points"));
        assert!(has("common.apply"));
        assert!(!has("nope.nope"));
    }
}
