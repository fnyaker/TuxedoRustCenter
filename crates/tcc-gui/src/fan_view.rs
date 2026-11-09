// SPDX-License-Identifier: GPL-3.0-or-later
//
// Vue typée de `GetFanDataJSON` (interface legacy `com.tuxedocomputers.tccd`).
//
// Comme `hardware_view`, ce module est un **DTO d'affichage** et non le
// contrat : `FanData` vit dans `tcc-daemon` (`fan.rs`). La GUI ne garde que les
// champs affichés, tolère les champs absents (`default`) et supprime les clés
// inconnues à la re-sérialisation. Aucune écriture : le mode, la consigne et la
// vitesse sont publiés par le daemon, jamais commandés depuis la GUI.

#![allow(dead_code)] // Sous-ensemble : tous les champs ne sont pas affichés.

use serde::{Deserialize, Serialize};

/// Instantané des ventilateurs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanView {
    /// Vrai si un backend a été détecté.
    pub available: bool,
    /// Backend actif (`tuxi`, `pwm`, `tuxedo-io`).
    pub backend: Option<String>,
    /// Vrai si le backend accepte les écritures.
    pub writable: bool,
    /// Vrai si le profil et les réglages demandent le contrôle.
    pub control_enabled: bool,
    /// Mode effectif : `off`, `auto` ou `manual` (chaîne libre côté vue).
    pub mode: String,
    /// Détail par ventilateur.
    pub fans: Vec<FanReadingView>,
}

/// Un ventilateur exposé.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FanReadingView {
    /// Index 0-based.
    pub index: u8,
    /// Libellé matériel (`cpu`, `gpu1`, `fan1`, …).
    pub label: String,
    /// Rôle utilisé pour la courbe (`cpu` ou `gpu`).
    pub target: String,
    /// Température en °C, si le capteur répond.
    pub temp: Option<i32>,
    /// Vitesse mesurée en pourcentage, si elle est lisible.
    pub speed: Option<u8>,
    /// Dernière consigne appliquée par le daemon, en pourcentage.
    pub applied: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::{FanReadingView, FanView};

    const FAN_JSON: &str = r#"{
        "available": true,
        "backend": "tuxi",
        "writable": true,
        "controlEnabled": true,
        "mode": "manual",
        "fans": [
            { "index": 0, "label": "cpu", "target": "cpu",
              "temp": 57, "speed": 42, "applied": 45 },
            { "index": 1, "label": "gpu1", "target": "gpu",
              "temp": null, "speed": null, "applied": null }
        ],
        "futureField": { "ignored": true }
    }"#;

    #[test]
    fn view_deserializes_and_drops_unknown_keys() {
        let view: FanView = serde_json::from_str(FAN_JSON).unwrap();
        assert!(view.available);
        assert_eq!(view.backend.as_deref(), Some("tuxi"));
        assert!(view.writable);
        assert!(view.control_enabled);
        assert_eq!(view.mode, "manual");
        assert_eq!(view.fans.len(), 2);
        assert_eq!(view.fans[0].speed, Some(42));
        assert_eq!(view.fans[0].applied, Some(45));
        assert_eq!(view.fans[1].temp, None);

        let canonical = serde_json::to_string(&view).unwrap();
        assert!(!canonical.contains("futureField"));
        assert!(canonical.contains("\"controlEnabled\":true"));
        assert!(canonical.contains("\"applied\":45"));
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let view: FanView = serde_json::from_str("{}").unwrap();
        assert!(!view.available);
        assert!(view.fans.is_empty());
        assert_eq!(view.mode, "");
    }

    #[test]
    fn unavailable_snapshot_is_valid() {
        let json = r#"{"available": false, "backend": null, "writable": false,
                       "controlEnabled": false, "mode": "off", "fans": []}"#;
        let view: FanView = serde_json::from_str(json).unwrap();
        assert!(!view.available);
        assert!(view.fans.is_empty());
        assert_eq!(view.mode, "off");
        let reading: FanReadingView = FanReadingView::default();
        assert_eq!(reading.index, 0);
    }

    #[test]
    fn wrong_field_type_is_rejected() {
        assert!(serde_json::from_str::<FanView>(r#"{ "fans": 3 }"#).is_err());
        assert!(serde_json::from_str::<FanView>(r#"{ "available": "yes" }"#).is_err());
    }
}
