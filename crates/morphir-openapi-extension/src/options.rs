use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

use crate::SchemaDiagnostic;

/// Configuration accepted by the OpenAPI and JSON Schema backend.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchemaOptions {
    /// Unsupported-form handling policy.
    pub unsupported: Unsupported,
}

impl Default for SchemaOptions {
    fn default() -> Self {
        Self {
            unsupported: Unsupported::Error,
        }
    }
}

impl SchemaOptions {
    /// Decode backend options without coercing the JSON values supplied by the host.
    pub fn from_map(options: &HashMap<String, Value>) -> Result<Self, SchemaDiagnostic> {
        let options = options
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        let value = serde_json::to_value(options)
            .map_err(|error| SchemaDiagnostic::invalid_option(error.to_string()))?;
        serde_json::from_value(value)
            .map_err(|error| SchemaDiagnostic::invalid_option(error.to_string()))
    }
}

/// How the backend reacts to a Morphir form it cannot project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unsupported {
    /// Fail the whole generation and emit no artifacts.
    #[default]
    Error,
    /// Skip the form, warn at its Morphir FQName, and keep valid artifacts.
    WarnAndSkip,
}
