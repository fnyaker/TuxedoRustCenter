// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Aplatit un document JSON en champs éditables (chemin pointé → feuille).

use serde_json::Value;

/// Feuille éditable.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    /// Chemin pointé (`display.brightness`).
    pub path: String,
    /// Valeur courante.
    pub value: Value,
}

/// Liste les feuilles scalaires (et `null`) d'un document, dans l'ordre du
/// document. Les tableaux sont parcourus par index ; `skip` exclut des
/// préfixes (par exemple `id`, `deletedProfileIds`).
#[must_use]
pub fn leaves(doc: &Value, skip: &[&str]) -> Vec<Leaf> {
    let mut out = Vec::new();
    walk(doc, String::new(), skip, &mut out);
    out
}

fn walk(value: &Value, path: String, skip: &[&str], out: &mut Vec<Leaf>) {
    if skip.iter().any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    }) {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                walk(child, next, skip, out);
            }
        }
        Value::Array(items) if items.len() <= 64 => {
            for (index, child) in items.iter().enumerate() {
                walk(child, format!("{path}.{index}"), skip, out);
            }
        }
        Value::Array(_) => {}
        scalar => out.push(Leaf {
            path,
            value: scalar.clone(),
        }),
    }
}

/// Texte affiché dans le champ pour une valeur.
#[must_use]
pub fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

/// Section (premier segment du chemin).
#[must_use]
pub fn section(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_objects_and_arrays() {
        let doc = json!({"name": "A", "display": {"brightness": 60, "on": true}, "tdp": [25, 35]});
        // L'ordre des clés dépend de la feature `preserve_order` : on le trie.
        let mut paths: Vec<_> = leaves(&doc, &[]).into_iter().map(|l| l.path).collect();
        paths.sort();
        assert_eq!(
            paths,
            ["display.brightness", "display.on", "name", "tdp.0", "tdp.1"]
        );
    }

    #[test]
    fn skips_prefixes_only_at_segment_boundaries() {
        let doc = json!({"id": "x", "idle": 1, "deletedProfileIds": ["a"]});
        let paths: Vec<_> = leaves(&doc, &["id", "deletedProfileIds"])
            .into_iter()
            .map(|l| l.path)
            .collect();
        assert_eq!(paths, ["idle"]);
    }

    #[test]
    fn formats_values() {
        assert_eq!(display(&json!("hi")), "hi");
        assert_eq!(display(&json!(3)), "3");
        assert_eq!(display(&json!(null)), "null");
        assert_eq!(display(&json!(true)), "true");
        assert_eq!(section("a.b.c"), "a");
    }
}
