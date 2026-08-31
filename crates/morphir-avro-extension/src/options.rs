use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;

use crate::AvroDiagnostic;

/// Configuration accepted by the Avro backend extension.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AvroOptions {
    /// Artifact syntax to render.
    pub representation: Representation,
    /// Public-model surface to project.
    pub projection: Projection,
    /// Dependency inclusion policy.
    pub dependencies: Dependencies,
    /// Alias representation policy.
    pub aliases: Aliases,
    /// Unsupported-form handling policy.
    pub unsupported: Unsupported,
    /// Whether recognized Morphir concepts receive Avro logical types.
    pub logical_types: bool,
    /// Default decimal precision.
    pub decimal_precision: u32,
    /// Default decimal scale.
    pub decimal_scale: u32,
    /// Exact Morphir FQName to Avro type overrides.
    pub type_mappings: BTreeMap<String, TypeMapping>,
}

impl Default for AvroOptions {
    fn default() -> Self {
        Self {
            representation: Representation::Json,
            projection: Projection::Schemas,
            dependencies: Dependencies::SelfContained,
            aliases: Aliases::Inline,
            unsupported: Unsupported::Error,
            logical_types: true,
            decimal_precision: 38,
            decimal_scale: 10,
            type_mappings: BTreeMap::new(),
        }
    }
}

impl AvroOptions {
    /// Decode backend options without coercing the JSON values supplied by the host.
    pub fn from_map(options: &HashMap<String, Value>) -> Result<Self, AvroDiagnostic> {
        let options = options
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        let value = serde_json::to_value(options)
            .map_err(|error| AvroDiagnostic::invalid_option(error.to_string()))?;
        let decoded: Self = serde_json::from_value(value)
            .map_err(|error| AvroDiagnostic::invalid_option(error.to_string()))?;
        decoded.validate()?;
        Ok(decoded)
    }

    /// Validate decimal ranges after decoding or constructing options directly.
    ///
    /// Projection and generation entry points must call this when their options
    /// did not originate from [`Self::from_map`].
    pub fn validate(&self) -> Result<(), AvroDiagnostic> {
        if self.decimal_precision == 0 {
            return Err(AvroDiagnostic::invalid_option(
                "decimal_precision must be greater than zero",
            ));
        }
        if self.decimal_scale > self.decimal_precision {
            return Err(AvroDiagnostic::invalid_option(format!(
                "decimal_scale ({}) must not exceed decimal_precision ({})",
                self.decimal_scale, self.decimal_precision
            )));
        }

        for (name, mapping) in &self.type_mappings {
            if (mapping.precision.is_some() || mapping.scale.is_some())
                && mapping.logical_type.as_deref() != Some("decimal")
            {
                return Err(AvroDiagnostic::invalid_option(format!(
                    "type_mappings.{name}.precision and scale require logical_type = \"decimal\""
                ))
                .with_source(name));
            }
            if mapping.logical_type.as_deref() != Some("decimal") {
                continue;
            }
            if mapping.physical_type != "bytes" {
                return Err(AvroDiagnostic::invalid_option(format!(
                    "type_mappings.{name}.logical_type = \"decimal\" requires type = \"bytes\""
                ))
                .with_source(name));
            }
            let precision = mapping.precision.unwrap_or(self.decimal_precision);
            let scale = mapping.scale.unwrap_or(self.decimal_scale);
            if precision == 0 {
                return Err(AvroDiagnostic::invalid_option(format!(
                    "type_mappings.{name}.precision must be greater than zero"
                ))
                .with_source(name));
            }
            if scale > precision {
                return Err(AvroDiagnostic::invalid_option(format!(
                    "type_mappings.{name}.scale ({scale}) must not exceed effective precision ({precision})"
                ))
                .with_source(name));
            }
        }
        Ok(())
    }
}

/// The Avro artifact representation to produce.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Representation {
    /// Avro JSON schema or protocol syntax.
    #[default]
    Json,
    /// Avro IDL syntax.
    Idl,
}

/// The subset of the Morphir package projected into Avro artifacts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Projection {
    /// Project public type roots and schemas only.
    #[default]
    Schemas,
    /// Project schemas plus declared application entry points.
    ProtocolEntryPoints,
    /// Project schemas plus all public value specifications.
    ProtocolPublic,
}

/// How generated artifacts reference dependency types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dependencies {
    /// Include dependency schemas in generated artifacts.
    #[default]
    SelfContained,
    /// Refer to dependency schemas without embedding them.
    Linked,
}

/// How Morphir aliases are represented in Avro.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Aliases {
    /// Inline aliases whose targets are not named records.
    #[default]
    Inline,
    /// Preserve aliases through generated wrapper records.
    WrapperRecord,
}

/// How the backend handles unsupported Morphir constructs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unsupported {
    /// Fail generation on an unsupported public form.
    #[default]
    Error,
    /// Emit a warning and omit the unsupported form.
    WarnAndSkip,
}

/// Per-type overrides for Avro's physical and logical type representation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeMapping {
    #[serde(rename = "type")]
    /// Avro physical type name.
    pub physical_type: String,
    /// Optional Avro logical type name.
    pub logical_type: Option<String>,
    /// Optional decimal precision override.
    pub precision: Option<u32>,
    /// Optional decimal scale override.
    pub scale: Option<u32>,
}
