//! Serialization-independent merge rules for Morphir configuration values.
//!
//! Configuration sources are parsed into [`serde_json::Value`] trees and then
//! combined from lowest to highest precedence. The rules are:
//!
//! 1. **Overlay wins**: for a key present in both values, the overlay value
//!    takes precedence.
//! 2. **Maps merge recursively**: if both values for a key are objects, they are
//!    deep-merged.
//! 3. **Arrays replace**: an overlay array replaces the base array entirely.
//! 4. **Null overlay is ignored**: a `null` overlay value never overrides the
//!    base value.
//! 5. **Secret references replace atomically**: exact secret-reference objects
//!    never merge with another object.
//! 6. **No mutation**: inputs are left untouched and a new value is returned.

use super::is_secret_reference;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// A path to a value within a configuration tree.
pub type ValuePath = Vec<String>;

/// Maps configuration leaves to their source values.
pub type ProvenanceMap<T> = BTreeMap<ValuePath, T>;

/// Deep-merge `overlay` onto `base`, returning a new value.
///
/// ```
/// use morphir_common::config::merge::deep_merge;
/// use serde_json::json;
///
/// let base = json!({"ir": {"format_version": 3, "strict_mode": false}, "codegen": {"targets": ["go"]}});
/// let overlay = json!({"ir": {"strict_mode": true}, "codegen": {"targets": ["typescript"]}, "logging": null});
///
/// assert_eq!(
///     deep_merge(&base, &overlay),
///     json!({"ir": {"format_version": 3, "strict_mode": true}, "codegen": {"targets": ["typescript"]}})
/// );
/// ```
pub fn deep_merge(base: &Value, overlay: &Value) -> Value {
    merge_at_path(
        base,
        overlay,
        &mut Vec::new(),
        None::<&mut Provenance<'_, ()>>,
    )
}

/// Deep-merge configuration values while tracking the source of each winning leaf.
///
/// ```
/// use morphir_common::config::{ProvenanceMap, deep_merge_with_provenance};
/// use serde_json::json;
///
/// let base = json!({"registry": {"endpoint": "https://old", "token": "old"}});
/// let base_origins = ProvenanceMap::from([
///     (vec!["registry".into(), "endpoint".into()], "defaults"),
///     (vec!["registry".into(), "token".into()], "defaults"),
/// ]);
/// let overlay = json!({"registry": {"token": {"env": "REGISTRY_TOKEN"}}});
///
/// let (merged, origins) =
///     deep_merge_with_provenance(&base, &base_origins, &overlay, &"project");
///
/// assert_eq!(merged["registry"]["endpoint"], "https://old");
/// assert_eq!(merged["registry"]["token"], json!({"env": "REGISTRY_TOKEN"}));
/// assert_eq!(origins[&vec!["registry".into(), "endpoint".into()]], "defaults");
/// assert_eq!(origins[&vec!["registry".into(), "token".into()]], "project");
/// ```
pub fn deep_merge_with_provenance<T: Clone>(
    base: &Value,
    base_origins: &ProvenanceMap<T>,
    overlay: &Value,
    overlay_origin: &T,
) -> (Value, ProvenanceMap<T>) {
    let mut origins = base_origins.clone();
    let value = merge_at_path(
        base,
        overlay,
        &mut Vec::new(),
        Some(&mut Provenance {
            origins: &mut origins,
            overlay_origin,
        }),
    );

    (value, origins)
}

/// Merge configuration values in order; later values take precedence.
///
/// An empty iterator produces an empty object.
pub fn merge_all<'a>(values: impl IntoIterator<Item = &'a Value>) -> Value {
    values
        .into_iter()
        .fold(Value::Object(Map::new()), |merged, value| {
            deep_merge(&merged, value)
        })
}

struct Provenance<'a, T> {
    origins: &'a mut ProvenanceMap<T>,
    overlay_origin: &'a T,
}

fn merge_at_path<T: Clone>(
    base: &Value,
    overlay: &Value,
    path: &mut ValuePath,
    mut provenance: Option<&mut Provenance<'_, T>>,
) -> Value {
    if overlay.is_null() {
        return base.clone();
    }

    if !is_secret_reference(base)
        && !is_secret_reference(overlay)
        && let (Value::Object(base), Value::Object(overlay)) = (base, overlay)
    {
        let mut merged = base.clone();
        for (key, overlay_value) in overlay {
            if overlay_value.is_null() {
                continue;
            }

            path.push(key.clone());
            let value = match merged.get(key) {
                Some(base_value) => {
                    merge_at_path(base_value, overlay_value, path, provenance.as_deref_mut())
                }
                None => overlay_value.clone(),
            };
            if let Some(provenance) = provenance.as_deref_mut()
                && !merged.contains_key(key)
            {
                replace_provenance(
                    provenance.origins,
                    overlay_value,
                    path,
                    provenance.overlay_origin,
                );
            }
            merged.insert(key.clone(), value);
            path.pop();
        }
        return Value::Object(merged);
    }

    if let Some(provenance) = provenance {
        replace_provenance(provenance.origins, overlay, path, provenance.overlay_origin);
    }
    overlay.clone()
}

fn replace_provenance<T: Clone>(
    origins: &mut ProvenanceMap<T>,
    overlay: &Value,
    path: &ValuePath,
    overlay_origin: &T,
) {
    origins.retain(|existing_path, _| !existing_path.starts_with(path));

    let mut pending = vec![(path.clone(), overlay)];
    while let Some((path, value)) = pending.pop() {
        if is_secret_reference(value) || !value.is_object() {
            origins.insert(path, overlay_origin.clone());
            continue;
        }

        if let Some(object) = value.as_object() {
            for (key, child) in object {
                let mut child_path = path.clone();
                child_path.push(key.clone());
                pending.push((child_path, child));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn overlay_scalar_wins() {
        let merged = deep_merge(&json!({"a": 1, "b": "base"}), &json!({"b": "overlay"}));
        assert_eq!(merged, json!({"a": 1, "b": "overlay"}));
    }

    #[test]
    fn nested_maps_merge_recursively() {
        let base = json!({"ir": {"format_version": 3, "strict_mode": false}});
        let overlay = json!({"ir": {"strict_mode": true}, "ui": {"theme": "dark"}});

        assert_eq!(
            deep_merge(&base, &overlay),
            json!({"ir": {"format_version": 3, "strict_mode": true}, "ui": {"theme": "dark"}})
        );
    }

    #[test]
    fn arrays_replace_instead_of_concatenating() {
        let merged = deep_merge(
            &json!({"codegen": {"targets": ["go", "scala"]}}),
            &json!({"codegen": {"targets": ["typescript"]}}),
        );
        assert_eq!(merged, json!({"codegen": {"targets": ["typescript"]}}));
    }

    #[test]
    fn scalar_overlay_replaces_map() {
        let merged = deep_merge(&json!({"a": {"b": 1}}), &json!({"a": "flat"}));
        assert_eq!(merged, json!({"a": "flat"}));
    }

    #[test]
    fn null_overlay_is_ignored_at_every_level() {
        let base = json!({"a": {"b": 1}, "c": 2});
        assert_eq!(deep_merge(&base, &Value::Null), base);
        assert_eq!(deep_merge(&base, &json!({"a": null, "c": null})), base);
        assert_eq!(deep_merge(&base, &json!({"a": {"b": null}})), base);
    }

    #[test]
    fn null_overlay_does_not_introduce_keys() {
        let merged = deep_merge(
            &json!({}),
            &json!({"frontend": null, "ir": {"mode": "vfs"}}),
        );
        assert_eq!(merged, json!({"ir": {"mode": "vfs"}}));
    }

    #[test]
    fn inputs_are_not_mutated() {
        let base = json!({"a": {"b": [1, 2]}});
        let overlay = json!({"a": {"b": [3]}, "c": true});
        let base_before = base.clone();
        let overlay_before = overlay.clone();

        let merged = deep_merge(&base, &overlay);

        assert_eq!(base, base_before);
        assert_eq!(overlay, overlay_before);
        assert_eq!(merged, json!({"a": {"b": [3]}, "c": true}));
    }

    #[test]
    fn merge_all_applies_later_values_last() {
        let defaults = json!({"logging": {"level": "info", "format": "text"}});
        let global = json!({"logging": {"level": "debug"}});
        let project = json!({"logging": {"format": "json"}});
        let user = json!({"logging": {"level": "warn"}});

        assert_eq!(
            merge_all([&defaults, &global, &project, &user]),
            json!({"logging": {"level": "warn", "format": "json"}})
        );
        assert_eq!(merge_all([]), json!({}));
    }

    #[test]
    fn secret_references_are_indivisible_merge_leaves() {
        assert_eq!(
            deep_merge(
                &json!({"token": {"keyring": {"service": "github.com", "account": "alice"}}}),
                &json!({"token": {"env": "GITHUB_TOKEN"}}),
            ),
            json!({"token": {"env": "GITHUB_TOKEN"}}),
        );
        assert_eq!(
            deep_merge(
                &json!({"token": {"env": "OLD"}}),
                &json!({"token": {"description": "ordinary"}}),
            ),
            json!({"token": {"description": "ordinary"}}),
        );
    }

    #[test]
    fn provenance_follows_winning_leaves_and_preserves_untouched_children() {
        let base = json!({"registry": {"endpoint": "https://old", "token": {"env": "OLD"}}});
        let base_origins = BTreeMap::from([
            (vec!["registry".into(), "endpoint".into()], "base"),
            (vec!["registry".into(), "token".into()], "base"),
        ]);
        let overlay = json!({"registry": {"token": {"command": ["gh", "auth", "token"]}}});
        let (value, origins) = deep_merge_with_provenance(&base, &base_origins, &overlay, &"user");
        assert_eq!(value["registry"]["endpoint"], "https://old");
        assert_eq!(origins[&vec!["registry".into(), "endpoint".into()]], "base");
        assert_eq!(origins[&vec!["registry".into(), "token".into()]], "user");
    }

    #[test]
    fn arrays_are_leaves_with_overlay_provenance() {
        let base = json!({"registries": ["https://old"]});
        let base_origins = BTreeMap::from([(vec!["registries".into()], "base")]);
        let overlay = json!({"registries": ["https://new"]});

        let (value, origins) = deep_merge_with_provenance(&base, &base_origins, &overlay, &"user");

        assert_eq!(value, json!({"registries": ["https://new"]}));
        assert_eq!(
            origins,
            BTreeMap::from([(vec!["registries".into()], "user")])
        );
    }

    #[test]
    fn replacing_an_object_removes_stale_descendant_origins() {
        let base = json!({"token": {"env": "OLD"}});
        let base_origins = BTreeMap::from([(vec!["token".into(), "env".into()], "base")]);
        let overlay = json!({"token": "literal"});

        let (value, origins) = deep_merge_with_provenance(&base, &base_origins, &overlay, &"user");

        assert_eq!(value, json!({"token": "literal"}));
        assert_eq!(origins, BTreeMap::from([(vec!["token".into()], "user")]));
    }
}
