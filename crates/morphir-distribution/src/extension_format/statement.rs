//! Preserve supplied statements and keep converted declarations off legacy wires.

use morphir_extension_sdk::statement::CapabilityStatement;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How an artifact's capability statement was obtained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatementProvenance {
    /// Metadata supplied by a publisher or converted from old flat keys.
    #[default]
    Declared,
    /// Metadata verified by an installation probe.
    Probed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
struct PreservedStatement {
    wire: Value,
    #[serde(skip)]
    parsed: CapabilityStatement,
}

impl PartialEq for PreservedStatement {
    fn eq(&self, other: &Self) -> bool {
        self.wire == other.wire
    }
}
impl Eq for PreservedStatement {}

impl<'de> Deserialize<'de> for PreservedStatement {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = Value::deserialize(deserializer)?;
        let parsed: CapabilityStatement =
            serde_json::from_value(wire.clone()).map_err(serde::de::Error::custom)?;
        if wire
            .get("requires")
            .and_then(|requires| requires.get("host"))
            .is_some()
            && !parsed.critical.iter().any(|path| path == "requires.host")
        {
            return Err(serde::de::Error::custom(
                "requires.host must be listed in critical",
            ));
        }
        parsed
            .check_host(
                &semver::Version::parse(env!("CARGO_PKG_VERSION"))
                    .expect("crate version is SemVer"),
            )
            .map_err(serde::de::Error::custom)?;
        Ok(Self { wire, parsed })
    }
}

/// Statement fields embedded in an artifact or installed extension record.
/// A synthesized statement is available to readers but omitted by legacy writers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    statement: Option<PreservedStatement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    statement_source: Option<StatementProvenance>,
    #[serde(skip)]
    legacy: Option<PreservedStatement>,
}

impl StatementRecord {
    /// Return the supplied or converted capability statement, if available.
    pub fn statement(&self) -> Option<&CapabilityStatement> {
        self.statement
            .as_ref()
            .or(self.legacy.as_ref())
            .map(|value| &value.parsed)
    }

    /// Return the recorded provenance, defaulting old declarations to `declared`.
    pub fn provenance(&self) -> StatementProvenance {
        self.statement_source.unwrap_or_default()
    }

    pub(crate) fn supply_legacy(&mut self, statement: CapabilityStatement) {
        if self.statement.is_none() {
            self.legacy = Some(PreservedStatement {
                wire: serde_json::to_value(&statement).expect("statement serializes"),
                parsed: statement,
            });
            self.statement_source = None;
        }
    }
}
