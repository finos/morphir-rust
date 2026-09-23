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
    /// Metadata verified by a publication or installation probe.
    Probed,
}

/// The operation that supplied an installation probe's statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeSource {
    /// The guest answered the one-shot description request.
    Describe,
    /// The host reconstructed a statement from a negotiated session.
    SessionFallback,
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
        Ok(Self { wire, parsed })
    }
}

/// Statement fields embedded in an artifact or installed extension record.
/// A synthesized statement is available to readers but omitted by legacy writers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    statement: Option<Box<PreservedStatement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    statement_source: Option<StatementProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe_source: Option<ProbeSource>,
    #[serde(skip)]
    legacy: Option<Box<PreservedStatement>>,
}

impl StatementRecord {
    pub(crate) fn as_declared(&self) -> Self {
        Self {
            statement_source: self
                .statement
                .as_ref()
                .map(|_| StatementProvenance::Declared),
            probe_source: None,
            ..self.clone()
        }
    }

    /// Record a declaration without claiming local verification.
    pub fn declared(statement: CapabilityStatement) -> Self {
        Self {
            statement: Some(Box::new(PreservedStatement {
                wire: serde_json::to_value(&statement).expect("statement serializes"),
                parsed: statement,
            })),
            statement_source: Some(StatementProvenance::Declared),
            probe_source: None,
            legacy: None,
        }
    }

    /// Mark the preserved declaration as verified. Legacy records keep their old wire shape.
    pub fn probed(&self, source: ProbeSource) -> Self {
        Self {
            statement_source: self.statement.as_ref().map(|_| StatementProvenance::Probed),
            probe_source: self.statement.as_ref().map(|_| source),
            ..self.clone()
        }
    }

    /// Check session agreement, restricting legacy declarations to expressible flat members.
    /// Identity, protocol, and capability kinds still use the SDK's agreement rules.
    pub fn check_session(
        &self,
        protocol_version: &str,
        extension: &morphir_extension_sdk::ExtensionInfo,
        capabilities: &serde_json::Map<String, Value>,
    ) -> Result<(), morphir_extension_sdk::statement::SessionAgreementError> {
        let declared = self.statement().expect("resolved record has a declaration");
        if self.statement.is_some() {
            return declared.check_session(protocol_version, extension, capabilities);
        }
        let representable = capabilities
            .iter()
            .filter_map(|(kind, value)| {
                let members: &[&str] = match kind.as_str() {
                    "frontend" => &["languages", "irVersions", "compile"],
                    "backend" => &["targets", "irVersions", "generate"],
                    _ => return None,
                };
                let value = match value.as_object() {
                    Some(object) => Value::Object(
                        object
                            .iter()
                            .filter(|(key, _)| members.contains(&key.as_str()))
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect(),
                    ),
                    None => value.clone(),
                };
                Some((kind.clone(), value))
            })
            .collect();
        declared.check_session(protocol_version, extension, &representable)
    }

    /// Return how a local probe obtained the statement, when recorded.
    pub fn probe_source(&self) -> Option<ProbeSource> {
        (self.provenance() == StatementProvenance::Probed)
            .then_some(self.probe_source)
            .flatten()
    }

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
            self.legacy = Some(Box::new(PreservedStatement {
                wire: serde_json::to_value(&statement).expect("statement serializes"),
                parsed: statement,
            }));
            self.statement_source = None;
            self.probe_source = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::protocol::MEP_VERSION;
    use morphir_extension_sdk::statement::SessionAgreementError;
    use serde_json::json;

    fn legacy_declaration() -> StatementRecord {
        let statement = serde_json::from_value(json!({
            "statementVersion": "0.1.0-draft.1", "protocolVersions": [MEP_VERSION],
            "extension": {"id": "sample", "name": "Sample", "version": "1.0.0", "types": ["frontend", "backend"]},
            "capabilities": {
                "frontend": {"languages": [{"id": "elm", "fileExtensions": [".elm"]}], "irVersions": ["3"], "compile": true},
                "backend": {"targets": ["text"], "irVersions": ["3"], "generate": true}
            }
        })).unwrap();
        let mut record = StatementRecord::default();
        record.supply_legacy(statement);
        record
    }

    #[test]
    fn legacy_agreement_ignores_only_members_outside_flat_keys() {
        let record = legacy_declaration();
        let statement = record.statement().unwrap();
        let mut reported = statement.capabilities.clone();
        reported["frontend"]["incremental"] = true.into();
        reported["backend"]["future"] = json!({"extra": true});
        reported.insert("streaming".into(), true.into());
        assert!(
            record
                .check_session(MEP_VERSION, &statement.extension, &reported)
                .is_ok()
        );
        for (kind, member, replacement) in [
            ("frontend", "languages", json!([])),
            ("frontend", "irVersions", json!(["4"])),
            ("frontend", "compile", json!(false)),
            ("backend", "targets", json!(["other"])),
            ("backend", "irVersions", json!(["4"])),
            ("backend", "generate", json!(false)),
        ] {
            let mut changed = reported.clone();
            changed[kind][member] = replacement;
            assert_eq!(
                record.check_session(MEP_VERSION, &statement.extension, &changed),
                Err(SessionAgreementError::CapabilityMember(format!(
                    "capabilities.{kind}.{member}"
                )))
            );
        }
        let mut extension = statement.extension.clone();
        extension.name = "Different".into();
        assert_eq!(
            record.check_session(MEP_VERSION, &extension, &reported),
            Err(SessionAgreementError::Identity("name"))
        );
        assert!(matches!(
            record.check_session("unsupported", &statement.extension, &reported),
            Err(SessionAgreementError::ProtocolVersion(_))
        ));
        let mut extension = statement.extension.clone();
        extension
            .types
            .push(morphir_extension_sdk::ExtensionType::Validator);
        assert!(matches!(
            record.check_session(MEP_VERSION, &extension, &reported),
            Err(SessionAgreementError::CapabilityKind(_))
        ));
        // Supplied statements retain the full protocol rule, including optional members.
        let supplied = StatementRecord::declared(statement.clone());
        assert!(
            supplied
                .check_session(MEP_VERSION, &statement.extension, &reported)
                .is_err()
        );
    }

    #[test]
    fn provenance_never_promotes_a_legacy_record_to_a_supplied_statement() {
        let record = legacy_declaration();
        for transformed in [
            record.as_declared(),
            record.probed(ProbeSource::Describe),
            record.probed(ProbeSource::SessionFallback),
        ] {
            assert_eq!(serde_json::to_value(&transformed).unwrap(), json!({}));
            assert_eq!(
                serde_json::to_value(transformed.statement()).unwrap(),
                serde_json::to_value(record.statement()).unwrap()
            );
            assert_eq!(transformed.provenance(), StatementProvenance::Declared);
            assert_eq!(transformed.probe_source(), None);
        }
    }
}
