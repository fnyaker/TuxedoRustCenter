// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Accès par chemin pointé (`display.brightness`, `fan.customFanCurve.cpu.0`)
//! dans un document JSON, pour `profile set` et `settings set`.

use anyhow::{Result, bail};
use serde_json::Value;

/// Interprète un argument de ligne de commande : JSON valide (`60`, `true`,
/// `null`, `{"a":1}`) sinon chaîne brute.
pub fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

fn segments(path: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = path.split('.').collect();
    if path.is_empty() || parts.iter().any(|part| part.is_empty()) {
        bail!("chemin invalide « {path} » (attendu : a.b.c)");
    }
    Ok(parts)
}

/// Lit la valeur à `path`.
pub fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for part in segments(path).ok()? {
        current = match current {
            Value::Object(map) => map.get(part)?,
            Value::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Écrit `value` à `path`. Les objets intermédiaires manquants sont créés ;
/// un index de tableau doit exister.
pub fn set_path(root: &mut Value, path: &str, value: Value) -> Result<()> {
    let parts = segments(path)?;
    let (last, parents) = parts.split_last().map_or((None, &[][..]), |(l, p)| (Some(*l), p));
    let Some(last) = last else {
        bail!("chemin vide");
    };
    let mut current = root;
    for part in parents {
        current = step(current, part, true)?;
    }
    match current {
        Value::Object(map) => {
            map.insert(last.to_owned(), value);
        }
        Value::Array(items) => {
            let index: usize = last
                .parse()
                .map_err(|_| anyhow::anyhow!("« {last} » n'est pas un index de tableau"))?;
            let Some(slot) = items.get_mut(index) else {
                bail!("index {index} hors du tableau ({} éléments)", items.len());
            };
            *slot = value;
        }
        _ => bail!("« {path} » : le parent n'est ni un objet ni un tableau"),
    }
    Ok(())
}

fn step<'a>(current: &'a mut Value, part: &str, create: bool) -> Result<&'a mut Value> {
    match current {
        Value::Object(map) => {
            if create && !map.contains_key(part) {
                map.insert(part.to_owned(), Value::Object(serde_json::Map::new()));
            }
            map.get_mut(part)
                .ok_or_else(|| anyhow::anyhow!("clé « {part} » introuvable"))
        }
        Value::Array(items) => {
            let index: usize = part
                .parse()
                .map_err(|_| anyhow::anyhow!("« {part} » n'est pas un index de tableau"))?;
            let len = items.len();
            items
                .get_mut(index)
                .ok_or_else(|| anyhow::anyhow!("index {index} hors du tableau ({len} éléments)"))
        }
        _ => bail!("« {part} » : le parent n'est ni un objet ni un tableau"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_json_or_string() {
        assert_eq!(parse_value("60"), json!(60));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("powersave"), json!("powersave"));
        assert_eq!(parse_value("{\"a\":1}"), json!({"a": 1}));
    }

    #[test]
    fn sets_nested_and_creates_objects() {
        let mut doc = json!({"display": {"brightness": 10}});
        set_path(&mut doc, "display.brightness", json!(60)).unwrap();
        set_path(&mut doc, "cpu.governor", json!("powersave")).unwrap();
        assert_eq!(doc["display"]["brightness"], json!(60));
        assert_eq!(doc["cpu"]["governor"], json!("powersave"));
    }

    #[test]
    fn sets_array_element_only_when_present() {
        let mut doc = json!({"tdp": [25, 35]});
        set_path(&mut doc, "tdp.1", json!(40)).unwrap();
        assert_eq!(doc["tdp"], json!([25, 40]));
        assert!(set_path(&mut doc, "tdp.5", json!(1)).is_err());
    }

    #[test]
    fn rejects_bad_paths() {
        let mut doc = json!({});
        assert!(set_path(&mut doc, "", json!(1)).is_err());
        assert!(set_path(&mut doc, "a..b", json!(1)).is_err());
        assert!(get_path(&doc, "a.b").is_none());
    }

    #[test]
    fn gets_values() {
        let doc = json!({"a": {"b": [1, 2]}});
        assert_eq!(get_path(&doc, "a.b.1"), Some(&json!(2)));
        assert_eq!(get_path(&doc, "a.c"), None);
    }
}
