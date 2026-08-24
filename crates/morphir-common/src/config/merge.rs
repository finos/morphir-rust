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
//! 5. **No mutation**: inputs are left untouched and a new value is returned.

use serde_json::{Map, Value};

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
    match (base, overlay) {
        // Rule 4: a null overlay never overrides the base value.
        (base, Value::Null) => base.clone(),
        // Rule 2: objects merge key by key.
        (Value::Object(base), Value::Object(overlay)) => {
            let merged = overlay.iter().filter(|(_, value)| !value.is_null()).fold(
                base.clone(),
                |mut merged, (key, overlay_value)| {
                    let value = match merged.get(key) {
                        Some(base_value) => deep_merge(base_value, overlay_value),
                        None => overlay_value.clone(),
                    };
                    merged.insert(key.clone(), value);
                    merged
                },
            );
            Value::Object(merged)
        }
        // Rules 1 and 3: scalars and arrays in the overlay replace the base.
        (_, overlay) => overlay.clone(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
