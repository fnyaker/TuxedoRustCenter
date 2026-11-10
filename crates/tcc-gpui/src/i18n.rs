// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Textes de l'interface : anglais (repli), français et allemand.
//!
//! La langue vient de `LC_ALL`, `LC_MESSAGES`, `LANGUAGE` puis `LANG`. Les
//! paramètres sont écrits `{nom}` et remplacés par [`tf`]. Les tests vérifient
//! que chaque clé existe dans les trois langues, avec les mêmes paramètres, et
//! que chaque clé utilisée dans le code existe.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Fr,
    De,
}

/// `(clé, en, fr, de)`.
const STRINGS: &[(&str, &str, &str, &str)] = &[
    (
        "app.title",
        "TUXEDO Control Center",
        "TUXEDO Control Center",
        "TUXEDO Control Center",
    ),
    ("nav.dashboard", "Dashboard", "Tableau de bord", "Übersicht"),
    ("nav.profiles", "Profiles", "Profils", "Profile"),
    ("nav.settings", "Settings", "Réglages", "Einstellungen"),
    ("nav.hardware", "Hardware", "Matériel", "Hardware"),
    (
        "status.connected",
        "tccd connected",
        "tccd connecté",
        "tccd verbunden",
    ),
    (
        "status.offline",
        "tccd unreachable",
        "tccd injoignable",
        "tccd nicht erreichbar",
    ),
    (
        "dash.activeProfile",
        "Active profile",
        "Profil actif",
        "Aktives Profil",
    ),
    ("dash.ac", "On AC", "Secteur", "Netzbetrieb"),
    ("dash.battery", "On battery", "Batterie", "Akkubetrieb"),
    ("dash.ppd", "PPD {value}", "PPD {value}", "PPD {value}"),
    ("dash.manual", "Manual", "Manuel", "Manuell"),
    ("dash.auto", "Automatic", "Automatique", "Automatisch"),
    ("dash.cpu", "Processor", "Processeur", "Prozessor"),
    ("dash.usage", "Usage", "Charge", "Auslastung"),
    ("dash.power", "Power", "Puissance", "Leistung"),
    (
        "dash.temperature",
        "Temperature",
        "Température",
        "Temperatur",
    ),
    ("dash.batteryCard", "Battery", "Batterie", "Akku"),
    ("dash.charge", "Charge", "Charge", "Ladestand"),
    (
        "dash.noBattery",
        "No battery detected",
        "Aucune batterie détectée",
        "Kein Akku erkannt",
    ),
    ("dash.fans", "Fans", "Ventilateurs", "Lüfter"),
    (
        "dash.noFans",
        "No controllable fan",
        "Aucun ventilateur contrôlable",
        "Kein steuerbarer Lüfter",
    ),
    ("dash.fanSpeed", "Fan speed", "Vitesse", "Drehzahl"),
    ("common.apply", "Apply", "Appliquer", "Anwenden"),
    ("common.create", "Create", "Créer", "Erstellen"),
    ("common.copy", "Copy", "Copier", "Kopieren"),
    ("common.delete", "Delete", "Supprimer", "Löschen"),
    ("common.activate", "Activate", "Activer", "Aktivieren"),
    (
        "common.copySuffix",
        "{name} (copy)",
        "{name} (copie)",
        "{name} (Kopie)",
    ),
    ("profiles.none", "No profile", "Aucun profil", "Kein Profil"),
    (
        "profiles.noneHint",
        "The daemon returned no profile.",
        "Le daemon n'a renvoyé aucun profil.",
        "Der Daemon hat kein Profil geliefert.",
    ),
    ("profiles.preset", "Preset", "Preset", "Voreinstellung"),
    ("profiles.active", "Active", "Actif", "Aktiv"),
    (
        "profiles.readOnly",
        "Read-only preset: copy it to edit it.",
        "Preset en lecture seule : copiez-le pour le modifier.",
        "Schreibgeschützte Voreinstellung: zum Bearbeiten kopieren.",
    ),
    (
        "profiles.parameters",
        "Parameters",
        "Paramètres",
        "Parameter",
    ),
    ("settings.title", "Settings", "Réglages", "Einstellungen"),
    (
        "settings.hint",
        "Every change is saved through tccd-admin (polkit).",
        "Chaque modification est enregistrée via tccd-admin (polkit).",
        "Jede Änderung wird über tccd-admin gespeichert (polkit).",
    ),
    (
        "hw.charging",
        "Battery charging",
        "Charge de la batterie",
        "Akkuladung",
    ),
    (
        "hw.chargeProfile",
        "Charging profile",
        "Profil de charge",
        "Ladeprofil",
    ),
    ("hw.chargePriority", "Priority", "Priorité", "Priorität"),
    ("hw.start", "Start", "Début", "Start"),
    ("hw.end", "End", "Fin", "Ende"),
    (
        "hw.chargeUnavailable",
        "Charge control is not available on this machine",
        "Contrôle de charge non disponible sur cette machine",
        "Ladesteuerung auf diesem Gerät nicht verfügbar",
    ),
    ("hw.misc", "Other", "Divers", "Sonstiges"),
    ("hw.fnLock", "Fn lock", "Verrou Fn", "Fn-Sperre"),
    ("hw.tdp", "TDP (W)", "TDP (W)", "TDP (W)"),
    (
        "hw.nothing",
        "No additional feature available",
        "Aucune fonction supplémentaire disponible",
        "Keine weiteren Funktionen verfügbar",
    ),
    ("hw.sensors", "Sensors", "Capteurs", "Sensoren"),
    (
        "hw.noGpu",
        "No GPU detected or sensors inactive",
        "Aucun GPU détecté ou capteurs inactifs",
        "Keine GPU erkannt oder Sensoren inaktiv",
    ),
    (
        "msg.profileActive",
        "Active profile: {id}",
        "Profil actif : {id}",
        "Aktives Profil: {id}",
    ),
    (
        "msg.backToAuto",
        "Back to automatic switching",
        "Retour à la bascule automatique",
        "Zurück zur automatischen Umschaltung",
    ),
    (
        "msg.created",
        "Profile created",
        "Profil créé",
        "Profil erstellt",
    ),
    (
        "msg.copied",
        "Profile copied",
        "Profil copié",
        "Profil kopiert",
    ),
    (
        "msg.deleted",
        "Profile deleted",
        "Profil supprimé",
        "Profil gelöscht",
    ),
    (
        "msg.saved",
        "{key} saved",
        "{key} enregistré",
        "{key} gespeichert",
    ),
    (
        "msg.applied",
        "{call} applied",
        "{call} appliqué",
        "{call} angewendet",
    ),
    (
        "msg.fnLock",
        "Fn lock applied",
        "Verrou Fn appliqué",
        "Fn-Sperre angewendet",
    ),
    (
        "msg.tdp",
        "TDP limits applied",
        "Limites TDP appliquées",
        "TDP-Grenzen angewendet",
    ),
];

fn detect(vars: &[Option<String>]) -> Lang {
    for value in vars.iter().flatten() {
        let first = value.split(':').next().unwrap_or("").to_ascii_lowercase();
        if first.is_empty() || first == "c" || first == "posix" {
            continue;
        }
        return if first.starts_with("fr") {
            Lang::Fr
        } else if first.starts_with("de") {
            Lang::De
        } else {
            Lang::En
        };
    }
    Lang::En
}

fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        detect(&[
            std::env::var("LC_ALL").ok(),
            std::env::var("LC_MESSAGES").ok(),
            std::env::var("LANGUAGE").ok(),
            std::env::var("LANG").ok(),
        ])
    })
}

fn lookup(lang: Lang, key: &str) -> Option<&'static str> {
    STRINGS
        .iter()
        .find(|row| row.0 == key)
        .map(|row| match lang {
            Lang::En => row.1,
            Lang::Fr => row.2,
            Lang::De => row.3,
        })
}

/// Texte de `key` dans la langue courante (la clé elle-même si inconnue).
#[must_use]
pub fn t(key: &str) -> String {
    lookup(lang(), key).unwrap_or(key).to_owned()
}

/// Comme [`t`], en remplaçant chaque `{nom}` par sa valeur.
#[must_use]
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut text = t(key);
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<String> {
        text.split('{')
            .skip(1)
            .filter_map(|rest| rest.split('}').next())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_key_is_complete_unique_and_consistent() {
        let mut seen = BTreeSet::new();
        for (key, en, fr, de) in STRINGS {
            assert!(seen.insert(*key), "clé dupliquée : {key}");
            for text in [en, fr, de] {
                assert!(!text.is_empty(), "{key} : traduction vide");
            }
            assert_eq!(placeholders(en), placeholders(fr), "{key} : paramètres fr");
            assert_eq!(placeholders(en), placeholders(de), "{key} : paramètres de");
        }
    }

    #[test]
    fn every_key_used_in_the_code_exists() {
        let mut checked = 0;
        for source in [include_str!("app.rs"), include_str!("worker.rs")] {
            for piece in source
                .split("t(\"")
                .skip(1)
                .chain(source.split("tf(\"").skip(1))
            {
                let key = piece.split('"').next().unwrap_or("");
                if key.contains('.') {
                    assert!(lookup(Lang::En, key).is_some(), "clé inconnue : {key}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 30, "trop peu de clés trouvées : {checked}");
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
    }

    #[test]
    fn substitutes_parameters() {
        assert_eq!(
            tf("msg.saved", &[("key", "fahrenheit")]).replace("fahrenheit", "K"),
            t("msg.saved").replace("{key}", "K")
        );
    }
}
