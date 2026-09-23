use super::CapabilityStatement;
use crate::{ExtensionInfo, ExtensionType};
use serde_json::{Map, Value};

/// The first failed rule of session agreement, in protocol order.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionAgreementError {
    /// Rule 1: an identity field changed.
    #[error("session agreement rule 1: extension.{0} differs")]
    Identity(&'static str),
    /// Rule 2: the negotiated protocol was not stated.
    #[error("session agreement rule 2: protocolVersion '{0}' was not stated")]
    ProtocolVersion(String),
    /// Rule 3: the session reports an unstated capability kind.
    #[error("session agreement rule 3: capability kind {0:?} was not stated")]
    CapabilityKind(ExtensionType),
    /// Rule 4: a reported member is absent or has a different value.
    #[error("session agreement rule 4: {0} is absent or differs")]
    CapabilityMember(String),
}

impl CapabilityStatement {
    /// Check a session against this statement without I/O or normalization.
    ///
    /// Pass capability members as received on the wire: typed capability defaults
    /// can add members the session did not report. Objects may omit members;
    /// arrays and scalar values must match exactly.
    pub fn check_session(
        &self,
        protocol_version: &str,
        extension: &ExtensionInfo,
        capabilities: &Map<String, Value>,
    ) -> Result<(), SessionAgreementError> {
        for (field, stated, reported) in [
            ("id", &self.extension.id, &extension.id),
            ("name", &self.extension.name, &extension.name),
            ("version", &self.extension.version, &extension.version),
        ] {
            if stated != reported {
                return Err(SessionAgreementError::Identity(field));
            }
        }
        if !self
            .protocol_versions
            .iter()
            .any(|version| version == protocol_version)
        {
            return Err(SessionAgreementError::ProtocolVersion(
                protocol_version.into(),
            ));
        }
        for kind in &extension.types {
            if !self.extension.types.contains(kind) {
                return Err(SessionAgreementError::CapabilityKind(*kind));
            }
        }
        compare_members(&self.capabilities, capabilities, "capabilities")
    }
}

fn compare_members(
    stated: &Map<String, Value>,
    reported: &Map<String, Value>,
    path: &str,
) -> Result<(), SessionAgreementError> {
    for (key, value) in reported {
        let path = format!("{path}.{key}");
        match (stated.get(key), value) {
            (Some(Value::Object(stated)), Value::Object(reported)) => {
                compare_members(stated, reported, &path)?;
            }
            (Some(stated), reported) if stated == reported => {}
            _ => return Err(SessionAgreementError::CapabilityMember(path)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
