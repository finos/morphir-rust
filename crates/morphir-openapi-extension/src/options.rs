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
    /// OpenAPI document version to render.
    pub version: OpenApiVersion,
    /// Public-model surface projected into the OpenAPI document.
    pub projection: Projection,
    /// How a value operation's result is shaped into responses.
    pub result_responses: ResultResponses,
    /// HTTP status code used for the error response.
    pub error_status: u16,
    /// Per-operation overrides, keyed by canonical Morphir FQName.
    pub operations: BTreeMap<String, OperationOverride>,
}

impl Default for SchemaOptions {
    fn default() -> Self {
        Self {
            unsupported: Unsupported::Error,
            version: OpenApiVersion::V31,
            projection: Projection::Schemas,
            result_responses: ResultResponses::Data,
            error_status: 400,
            operations: BTreeMap::new(),
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
        let decoded: Self = serde_json::from_value(value)
            .map_err(|error| SchemaDiagnostic::invalid_option(error.to_string()))?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validate ranges and shapes after decoding or direct construction.
    ///
    /// Projection entry points must call this when their options did not come
    /// from [`Self::from_map`].
    pub fn validate(&self) -> Result<(), SchemaDiagnostic> {
        if !(400..=599).contains(&self.error_status) {
            return Err(SchemaDiagnostic::invalid_option(format!(
                "error_status ({}) must be in the range 400 through 599",
                self.error_status
            )));
        }
        for (source_name, operation) in &self.operations {
            if let Some(path) = &operation.path
                && !path.starts_with('/')
            {
                return Err(SchemaDiagnostic::invalid_option(format!(
                    "operation path for {source_name} must start with '/': {path}"
                )));
            }
        }
        Ok(())
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

/// The OpenAPI document version to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum OpenApiVersion {
    /// OpenAPI 3.1.
    #[default]
    #[serde(rename = "3.1")]
    V31,
    /// OpenAPI 3.0.
    #[serde(rename = "3.0")]
    V30,
}

/// The subset of the Morphir package projected into the OpenAPI document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Projection {
    /// Project public type roots and schemas only.
    #[default]
    Schemas,
    /// Project schemas plus declared application entry points as operations.
    OperationsEntryPoints,
    /// Project schemas plus all public value specifications as operations.
    OperationsPublic,
}

/// How a value operation's result is shaped into HTTP responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResultResponses {
    /// A single success response carrying the whole result value.
    #[default]
    Data,
    /// Separate success and error responses split from the result value.
    Split,
}

/// A per-operation override, keyed by canonical Morphir FQName.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OperationOverride {
    /// HTTP method override.
    pub method: Option<HttpMethod>,
    /// Path template override; must start with `/` when present.
    pub path: Option<String>,
    /// Parameter binding overrides, keyed by parameter name.
    pub parameters: BTreeMap<String, ParameterBinding>,
}

/// An HTTP method usable in an operation override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    /// HTTP GET.
    Get,
    /// HTTP PUT.
    Put,
    /// HTTP POST.
    Post,
    /// HTTP DELETE.
    Delete,
    /// HTTP PATCH.
    Patch,
}

/// Where an operation parameter is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterBinding {
    /// Bound to a path segment.
    Path,
    /// Bound to a query parameter.
    Query,
    /// Bound to an HTTP header.
    Header,
    /// Bound to the request body.
    Body,
}
