//! Environment-variable configuration source.
//!
//! Variables that start with the configured prefix (default `MORPHIR_`) are
//! converted into a nested configuration value:
//!
//! - The prefix and any underscores that immediately follow it are removed.
//! - A double underscore (`__`) separates nesting levels:
//!   `MORPHIR_CODEGEN__OUTPUT_FORMAT=compact` → `codegen.output_format = "compact"`.
//! - Single underscores stay part of the key at that level and every segment is
//!   lower-cased: `MORPHIR_IR_FORMAT_VERSION=3` → `ir_format_version = 3`.
//! - Values are typed mechanically: `true`/`false` (any case) become booleans,
//!   integers become numbers, values that start with `[` or `{` and parse as
//!   JSON become arrays or objects, and anything else stays a string.
//!
//! The mapping is deliberately mechanical and does not guess dotted paths.

use serde_json::{Map, Value};

/// Default environment-variable prefix, without the trailing underscore.
pub const DEFAULT_ENV_PREFIX: &str = "MORPHIR";

/// Operational variables that control Morphir itself and are therefore never
/// interpreted as configuration keys, even though they carry the prefix.
pub const RESERVED_ENV_VARS: &[&str] = &["MORPHIR_HOME", "MORPHIR_LOG_DIR"];

/// Convert prefixed environment variables into a configuration value.
///
/// Entries are processed in sorted key order so the result does not depend on
/// the iteration order of the input. When a scalar and a nested key conflict
/// (for example `MORPHIR_IR=x` and `MORPHIR_IR__STRICT_MODE=true`), the shorter
/// path wins and the nested entry is dropped.
///
/// ```
/// use morphir_config::env::env_config_value;
/// use serde_json::json;
///
/// let value = env_config_value(
///     "MORPHIR",
///     [
///         ("MORPHIR_IR__STRICT_MODE", "true"),
///         ("MORPHIR_CODEGEN__TARGETS", r#"["go", "typescript"]"#),
///         ("MORPHIR_LOGGING__LEVEL", "debug"),
///         ("PATH", "/usr/bin"),
///     ],
/// );
///
/// assert_eq!(
///     value,
///     json!({
///         "ir": {"strict_mode": true},
///         "codegen": {"targets": ["go", "typescript"]},
///         "logging": {"level": "debug"}
///     })
/// );
/// ```
pub fn env_config_value<K, V>(prefix: &str, vars: impl IntoIterator<Item = (K, V)>) -> Value
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    let prefix = format!("{}_", prefix.trim_end_matches('_').to_ascii_uppercase());

    let mut entries = vars
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.as_ref().to_ascii_uppercase();
            if RESERVED_ENV_VARS.contains(&key.as_str()) {
                return None;
            }
            let path = env_key_to_path(key.strip_prefix(&prefix)?);
            (!path.is_empty()).then(|| (path, parse_env_value(value.as_ref())))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let root = entries
        .into_iter()
        .fold(Map::new(), |mut root, (path, value)| {
            insert_nested(&mut root, &path, value);
            root
        });
    Value::Object(root)
}

/// Read the current process environment into a configuration value.
pub fn process_env_config_value(prefix: &str) -> Value {
    env_config_value(
        prefix,
        std::env::vars_os().map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        }),
    )
}

/// Split an un-prefixed environment key into lower-case path segments.
///
/// ```
/// use morphir_config::env::env_key_to_path;
///
/// assert_eq!(env_key_to_path("IR__FORMAT_VERSION"), vec!["ir", "format_version"]);
/// assert_eq!(env_key_to_path("_CODEGEN__GO__PACKAGE"), vec!["codegen", "go", "package"]);
/// assert_eq!(env_key_to_path("IR_FORMAT_VERSION"), vec!["ir_format_version"]);
/// ```
pub fn env_key_to_path(key: &str) -> Vec<String> {
    key.trim_start_matches('_')
        .split("__")
        .filter(|segment| !segment.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Interpret an environment-variable value as a configuration scalar.
///
/// ```
/// use morphir_config::env::parse_env_value;
/// use serde_json::json;
///
/// assert_eq!(parse_env_value("TRUE"), json!(true));
/// assert_eq!(parse_env_value("42"), json!(42));
/// assert_eq!(parse_env_value("1.0"), json!("1.0"));
/// assert_eq!(parse_env_value("[\"go\"]"), json!(["go"]));
/// assert_eq!(parse_env_value("[not json"), json!("[not json"));
/// ```
pub fn parse_env_value(raw: &str) -> Value {
    let trimmed = raw.trim();

    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return Value::from(integer);
    }
    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
    {
        return value;
    }

    Value::String(raw.to_string())
}

fn insert_nested(map: &mut Map<String, Value>, path: &[String], value: Value) {
    match path {
        [] => {}
        [leaf] => {
            map.entry(leaf.clone()).or_insert(value);
        }
        [head, rest @ ..] => {
            let child = map
                .entry(head.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(child) = child {
                insert_nested(child, rest, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ignores_variables_without_prefix() {
        let value = env_config_value("MORPHIR", [("HOME", "/home/alice"), ("MORPHIRX", "1")]);
        assert_eq!(value, json!({}));
    }

    #[test]
    fn reserved_operational_variables_are_not_configuration() {
        let value = env_config_value(
            "MORPHIR",
            [
                ("MORPHIR_HOME", "/sandbox/mh"),
                ("MORPHIR_LOG_DIR", "/tmp/logs"),
                ("MORPHIR_LOGGING__LEVEL", "debug"),
            ],
        );
        assert_eq!(value, json!({"logging": {"level": "debug"}}));
    }

    #[test]
    fn prefix_match_is_case_insensitive_and_tolerates_trailing_underscore() {
        let value = env_config_value("morphir_", [("morphir_ui__theme", "dark")]);
        assert_eq!(value, json!({"ui": {"theme": "dark"}}));
    }

    #[test]
    fn single_underscores_are_not_split() {
        let value = env_config_value("MORPHIR", [("MORPHIR_IR_FORMAT_VERSION", "3")]);
        assert_eq!(value, json!({"ir_format_version": 3}));
    }

    #[test]
    fn leading_double_underscore_is_tolerated() {
        let value = env_config_value("MORPHIR", [("MORPHIR__IR__FORMAT_VERSION", "4")]);
        assert_eq!(value, json!({"ir": {"format_version": 4}}));
    }

    #[test]
    fn deeper_nesting_creates_intermediate_maps() {
        let value = env_config_value(
            "MORPHIR",
            [
                ("MORPHIR_CODEGEN__GO__PACKAGE", "foo"),
                ("MORPHIR_CODEGEN__OUTPUT_FORMAT", "compact"),
            ],
        );
        assert_eq!(
            value,
            json!({"codegen": {"go": {"package": "foo"}, "output_format": "compact"}})
        );
    }

    #[test]
    fn shorter_path_wins_on_conflict_regardless_of_input_order() {
        let expected = json!({"ir": "flat"});
        let first = env_config_value(
            "MORPHIR",
            [("MORPHIR_IR", "flat"), ("MORPHIR_IR__STRICT_MODE", "true")],
        );
        let second = env_config_value(
            "MORPHIR",
            [("MORPHIR_IR__STRICT_MODE", "true"), ("MORPHIR_IR", "flat")],
        );
        assert_eq!(first, expected);
        assert_eq!(second, expected);
    }

    #[test]
    fn values_are_typed_mechanically() {
        assert_eq!(parse_env_value("false"), json!(false));
        assert_eq!(parse_env_value(" 7 "), json!(7));
        assert_eq!(parse_env_value("-7"), json!(-7));
        assert_eq!(parse_env_value("5m"), json!("5m"));
        assert_eq!(parse_env_value("{\"a\": 1}"), json!({"a": 1}));
        assert_eq!(parse_env_value(""), json!(""));
    }

    #[test]
    fn process_environment_is_read_with_prefix() {
        // The result is environment dependent; only the shape is asserted.
        assert!(process_env_config_value("MORPHIR").is_object());
    }
}
