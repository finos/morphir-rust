//! Content-based parsing for supported Morphir configuration formats.

use crate::legacy::{LegacyProjectConfig, normalize_legacy_config};
use serde_json::Value;
use serde_saphyr::budget::BudgetBreach;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
use serde_saphyr::options::{DuplicateKeyPolicy, MergeKeyPolicy};
use serde_saphyr::{Error as YamlError, alias_limits, budget, options};
use thiserror::Error;

const MAX_YAML_INPUT_BYTES: usize = 512 * 1024 * 1024;

/// A failure to parse or normalize configuration content.
#[derive(Debug, Error)]
pub enum ConfigParseError {
    /// The configuration name does not identify a supported serialization.
    #[error("Unsupported Morphir config format for {name} (expected .toml, .yaml, .yml, or .json)")]
    UnsupportedFormat {
        /// Name used to select the serialization.
        name: String,
    },
    /// TOML parsing failed.
    #[error("Failed to parse TOML config {name}: {source}")]
    Toml {
        /// Configuration name.
        name: String,
        /// Parser error.
        #[source]
        source: toml::de::Error,
    },
    /// TOML could not be normalized into JSON.
    #[error("Failed to normalize TOML config {name}: {source}")]
    NormalizeToml {
        /// Configuration name.
        name: String,
        /// Normalization error.
        #[source]
        source: serde_json::Error,
    },
    /// Strict YAML parsing or validation failed.
    #[error("Failed to parse YAML config {name}: {message}")]
    Yaml {
        /// Configuration name.
        name: String,
        /// Human-readable reason.
        message: String,
    },
    /// Legacy JSON parsing failed.
    #[error("Failed to parse legacy JSON config {name}: {source}")]
    LegacyJson {
        /// Configuration name.
        name: String,
        /// Parser error.
        #[source]
        source: serde_json::Error,
    },
}

/// Parse configuration content based on its name without accessing a filesystem.
///
/// TOML and YAML inputs are normalized into JSON values. JSON inputs use the
/// legacy `morphir.json` shape and are normalized to the current configuration
/// structure.
pub fn parse_config(name: &str, content: &str) -> Result<Value, ConfigParseError> {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());

    match extension.as_deref() {
        Some("toml") => parse_toml(name, content),
        Some("yaml" | "yml") => parse_yaml(name, content),
        Some("json") => parse_legacy_json(name, content),
        _ => Err(ConfigParseError::UnsupportedFormat {
            name: name.to_owned(),
        }),
    }
}

fn parse_toml(name: &str, content: &str) -> Result<Value, ConfigParseError> {
    let value =
        toml::from_str::<toml::Value>(content).map_err(|source| ConfigParseError::Toml {
            name: name.to_owned(),
            source,
        })?;
    serde_json::to_value(value).map_err(|source| ConfigParseError::NormalizeToml {
        name: name.to_owned(),
        source,
    })
}

fn parse_yaml(name: &str, content: &str) -> Result<Value, ConfigParseError> {
    validate_yaml_profile(name, content)?;
    let value = serde_saphyr::from_str_with_options(content, yaml_options())
        .map_err(|error| yaml_parse_error(name, &error))?;
    validate_yaml_value(name, &value, true)?;
    Ok(value)
}

fn parse_legacy_json(name: &str, content: &str) -> Result<Value, ConfigParseError> {
    let legacy = serde_json::from_str::<LegacyProjectConfig>(content).map_err(|source| {
        ConfigParseError::LegacyJson {
            name: name.to_owned(),
            source,
        }
    })?;
    Ok(normalize_legacy_config(legacy))
}

fn yaml_options() -> serde_saphyr::Options {
    options! {
        budget: budget! {
            max_reader_input_bytes: Some(MAX_YAML_INPUT_BYTES),
            max_events: 50_000_000,
            max_aliases: 0,
            max_anchors: 0,
            max_depth: 512,
            max_documents: 1,
            max_nodes: 20_000_000,
            max_total_scalar_bytes: MAX_YAML_INPUT_BYTES,
            max_merge_keys: 0,
        },
        duplicate_keys: DuplicateKeyPolicy::Error,
        merge_keys: MergeKeyPolicy::Error,
        alias_limits: alias_limits! {
            max_total_replayed_events: 0,
            max_replay_stack_depth: 0,
            max_alias_expansions_per_anchor: 0,
        },
        emit_comments: false,
        strict_booleans: true,
        legacy_octal_numbers: false,
        reject_non_finite_typeless_float: true,
        with_snippet: false,
    }
}

fn yaml_parse_error(name: &str, error: &YamlError) -> ConfigParseError {
    let message = match error.without_snippet() {
        YamlError::DuplicateMappingKey { .. } => "YAML config must not contain duplicate keys",
        YamlError::MultipleDocuments { .. }
        | YamlError::Budget {
            breach: BudgetBreach::Documents { .. },
            ..
        } => "YAML config must contain exactly one document",
        YamlError::MergeKeyNotAllowed { .. } => "YAML config must not use merge keys",
        YamlError::TaggedScalarCannotDeserializeIntoString { .. }
        | YamlError::TaggedEnumMismatch { .. } => "YAML config must not contain custom tags",
        YamlError::AliasReplayCounterOverflow { .. }
        | YamlError::AliasReplayLimitExceeded { .. }
        | YamlError::AliasExpansionLimitExceeded { .. }
        | YamlError::AliasReplayStackDepthExceeded { .. }
        | YamlError::AliasError { .. }
        | YamlError::Budget {
            breach: BudgetBreach::Aliases { .. } | BudgetBreach::Anchors { .. },
            ..
        } => "YAML config must not contain anchors or aliases",
        _ => return yaml_error(name, error.to_string()),
    };
    match error.location() {
        Some(location) => yaml_error(
            name,
            format!(
                "{message} at line {}, column {}",
                location.line(),
                location.column()
            ),
        ),
        None => yaml_error(name, message),
    }
}

fn validate_yaml_profile(name: &str, content: &str) -> Result<(), ConfigParseError> {
    let parser_options = serde_saphyr::granit_parser::options! {
        emit_comments: false,
    };
    let mut document_count = 0;
    for parsed in Parser::new_from_str_with_options(content, parser_options) {
        let (event, span) = parsed.map_err(|error| yaml_error(name, error.to_string()))?;
        if matches!(event, Event::DocumentStart(..)) {
            document_count += 1;
            if document_count > 1 {
                return Err(yaml_profile_error(
                    name,
                    "YAML config must contain exactly one document",
                    span.start.line(),
                    span.start.col(),
                ));
            }
        }
        if event.anchor_id().is_some() || event.alias_id().is_some() {
            return Err(yaml_profile_error(
                name,
                "YAML config must not contain anchors or aliases",
                span.start.line(),
                span.start.col(),
            ));
        }
        if event.tag().is_some() {
            let marker = span.tag_start.unwrap_or(span.start);
            return Err(yaml_profile_error(
                name,
                "YAML config must not contain custom tags",
                marker.line(),
                marker.col(),
            ));
        }
        if event.scalar().is_some_and(|(value, style)| {
            style == ScalarStyle::Plain && looks_like_timestamp(value)
        }) {
            return Err(yaml_profile_error(
                name,
                "YAML config must quote timestamp-like scalars",
                span.start.line(),
                span.start.col(),
            ));
        }
    }
    Ok(())
}

fn validate_yaml_value(name: &str, value: &Value, at_root: bool) -> Result<(), ConfigParseError> {
    match value {
        Value::Null => Err(yaml_error(name, "YAML config must not contain null values")),
        Value::Object(mapping) => mapping
            .values()
            .try_for_each(|value| validate_yaml_value(name, value, false)),
        Value::Array(values) if at_root => {
            let _ = values;
            Err(yaml_error(name, "YAML config root must be a mapping"))
        }
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_yaml_value(name, value, false)),
        _ if at_root => Err(yaml_error(name, "YAML config root must be a mapping")),
        _ => Ok(()),
    }
}

fn yaml_error(name: &str, message: impl Into<String>) -> ConfigParseError {
    ConfigParseError::Yaml {
        name: name.to_owned(),
        message: message.into(),
    }
}

fn yaml_profile_error(name: &str, message: &str, line: usize, column: usize) -> ConfigParseError {
    yaml_error(
        name,
        format!("{message} at line {line}, column {}", column + 1),
    )
}

fn looks_like_timestamp(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_yaml_and_legacy_json_without_a_filesystem() {
        let toml =
            parse_config("morphir.toml", "[project]\nname='acme/orders'\nversion='1'").unwrap();
        let yaml = parse_config(
            "morphir.yaml",
            "project:\n  name: acme/orders\n  version: '1'\n",
        )
        .unwrap();
        let legacy = parse_config(
            "morphir.json",
            r#"{"name":"acme/orders","sourceDirectory":"src","exposedModules":[]}"#,
        )
        .unwrap();

        assert_eq!(toml["project"]["name"], "acme/orders");
        assert_eq!(yaml["project"]["name"], "acme/orders");
        assert_eq!(legacy["project"]["version"], "0.1.0");
    }

    #[test]
    fn rejects_yaml_aliases_tags_duplicate_keys_nulls_and_multiple_documents() {
        for source in [
            "defaults: &d { language: elm }\nfrontend: *d\n",
            "frontend: !custom elm\n",
            "project: {}\nproject: {}\n",
            "project: null\n",
            "project: {}\n---\nproject: {}\n",
        ] {
            assert!(
                parse_config("morphir.yaml", source).is_err(),
                "accepted {source:?}"
            );
        }
    }

    #[test]
    fn normalized_yaml_errors_preserve_parser_locations() {
        for (source, expected_reason) in [
            (
                "project:\n  name: first\n  name: second\n",
                "YAML config must not contain duplicate keys",
            ),
            (
                "project:\n  name: orders\n  <<: {version: '1'}\n",
                "YAML config must not use merge keys",
            ),
        ] {
            let error = parse_config("located.yaml", source).expect_err("invalid YAML");
            let diagnostic = error.to_string();

            assert!(diagnostic.contains(expected_reason), "{diagnostic}");
            assert!(
                diagnostic.contains("line 3, column 3"),
                "missing parser location in {diagnostic}"
            );
        }
    }

    #[test]
    fn rejects_actual_yaml_properties_and_ambiguous_complete_scalars() {
        for source in [
            "value: !custom text\n",
            "value: !!str text\n",
            "value: &anchor text\n",
            "value: *alias\n",
            "value:\n  <<: {nested: true}\n",
            "value: .inf\n",
            "value: 2026-08-30\n",
        ] {
            assert!(
                parse_config("strict.yaml", source).is_err(),
                "accepted {source:?}"
            );
        }
    }

    #[test]
    fn allows_forbidden_token_text_in_quoted_and_block_scalars() {
        let config = parse_config(
            "quoted.yaml",
            "project:\n  name: 'acme/!*&'\n  version: '2026-08-30'\n  description: |\n    Literal * alias, ! tag, & anchor, and 2026-08-30 text.\n",
        )
        .unwrap();

        assert_eq!(config["project"]["name"], "acme/!*&");
    }

    #[test]
    fn allows_forbidden_tokens_after_doubled_single_quote_escapes() {
        for (source, expected) in [
            ("description: 'it''s ! text'\n", "it's ! text"),
            ("description: 'it''s * text'\n", "it's * text"),
        ] {
            let config = parse_config("quoted.yaml", source).unwrap();
            assert_eq!(config["description"], expected);
        }
    }

    #[test]
    fn allows_yaml_indicator_characters_and_dates_inside_plain_scalars() {
        for (source, expected) in [
            ("description: hello ! world\n", "hello ! world"),
            ("description: rock & roll\n", "rock & roll"),
            ("description: a * character\n", "a * character"),
            (
                "description: released 2026-08-30 successfully\n",
                "released 2026-08-30 successfully",
            ),
        ] {
            let config = parse_config("plain.yaml", source).unwrap();
            assert_eq!(config["description"], expected);
        }
    }

    #[test]
    fn parses_a_long_plain_scalar_without_line_sized_scanner_collections() {
        let description = format!(
            "{} ! & * released 2026-08-30 successfully",
            "a".repeat(1024 * 1024)
        );
        let source = format!("description: {description}\n");

        let config = parse_config("long-line.yaml", &source).unwrap();

        assert_eq!(
            config["description"].as_str().map(str::len),
            Some(description.len())
        );
        assert!(
            config["description"]
                .as_str()
                .is_some_and(|value| value.ends_with("2026-08-30 successfully"))
        );
    }

    #[test]
    fn every_parse_diagnostic_names_its_input() {
        for (name, source) in [
            ("broken.toml", "[project"),
            ("broken.yaml", "project: null\n"),
            ("broken.json", "{"),
            ("broken.txt", "project = {}"),
        ] {
            let error = parse_config(name, source).expect_err("invalid configuration");
            assert!(error.to_string().contains(name), "{error}");
        }
    }
}
