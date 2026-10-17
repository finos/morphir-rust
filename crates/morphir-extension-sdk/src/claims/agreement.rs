use super::CapabilityClaimSet;
use crate::model::{MODEL_MEP_VERSION, ModelTransportSupport, NegotiatedModelTransport};
use crate::{ExtensionInfo, ExtensionType};
use serde_json::{Map, Value};

/// The first member that differs between two complete capability claim sets.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("claims member {0} differs")]
pub struct ClaimsAgreementError(pub String);

/// The first failed rule of session agreement, in protocol order.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionAgreementError {
    /// Rule 1: an identity field changed.
    #[error("session agreement rule 1: extension.{0} differs")]
    Identity(&'static str),
    /// Rule 2: the negotiated protocol was not claimed.
    #[error("session agreement rule 2: protocolVersion '{0}' was not claimed")]
    ProtocolVersion(String),
    /// Rule 3: the session reports an unstated capability kind.
    #[error("session agreement rule 3: capability kind {0:?} was not claimed")]
    CapabilityKind(ExtensionType),
    /// Rule 4: a reported member is absent or has a different value.
    #[error("session agreement rule 4: {0} is absent or differs")]
    CapabilityMember(String),
}

impl CapabilityClaimSet {
    /// Compare complete claim sets, including protocol sets and requirements.
    /// Unlike session agreement, neither claim set may omit a reported member.
    pub fn check_claims(&self, reported: &Self) -> Result<(), ClaimsAgreementError> {
        fn compare(left: &Value, right: &Value, path: &str) -> Result<(), ClaimsAgreementError> {
            if let (Value::Object(left), Value::Object(right)) = (left, right) {
                for key in left
                    .keys()
                    .chain(right.keys().filter(|key| !left.contains_key(*key)))
                {
                    let path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    match (left.get(key), right.get(key)) {
                        (Some(left), Some(right)) => compare(left, right, &path)?,
                        _ => return Err(ClaimsAgreementError(path)),
                    }
                }
                Ok(())
            } else if left == right {
                Ok(())
            } else {
                Err(ClaimsAgreementError(path.into()))
            }
        }
        compare(
            &serde_json::to_value(self).expect("claim set serializes"),
            &serde_json::to_value(reported).expect("claim set serializes"),
            "",
        )
    }

    /// Check a session against this claims without I/O or normalization.
    ///
    /// Pass capability members as received on the wire: typed capability defaults
    /// can add members the session did not report. Objects may omit members;
    /// arrays and scalar values must match exactly. Only under the exact model
    /// draft, `modelTransport` may narrow complete tuples, guarantee IDs and
    /// positive resource bounds. Unknown members retain the ordinary rules.
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
        for (key, reported) in capabilities {
            if protocol_version == MODEL_MEP_VERSION && key == "modelTransport" {
                let stated = self.capabilities.get(key).ok_or_else(|| {
                    SessionAgreementError::CapabilityMember("capabilities.modelTransport".into())
                })?;
                compare_model_transport(stated, reported)?;
            } else {
                compare_members(
                    &self.capabilities,
                    &Map::from_iter([(key.clone(), reported.clone())]),
                    "capabilities",
                )?;
            }
        }
        Ok(())
    }
}

// Validate known negotiation fields, then compare every unknown raw field under
// the unchanged agreement rules. Decoding DTOs alone would discard unknown
// members and could incorrectly admit an unclaimed extension capability.
fn compare_model_transport(stated: &Value, reported: &Value) -> Result<(), SessionAgreementError> {
    const PATH: &str = "capabilities.modelTransport";
    let failed = || SessionAgreementError::CapabilityMember(PATH.into());
    let stated_support: ModelTransportSupport =
        serde_json::from_value(stated.clone()).map_err(|_| failed())?;
    let reported_support: ModelTransportSupport =
        serde_json::from_value(reported.clone()).map_err(|_| failed())?;
    NegotiatedModelTransport::from_selection(
        MODEL_MEP_VERSION,
        Some(&stated_support),
        Some(&reported_support),
    )
    .map_err(|_| failed())?;
    compare_unknown(stated, reported, &["tuples", "limits", "guarantees"], PATH)?;
    compare_unknown(
        &stated["limits"],
        &reported["limits"],
        &["maxEncodedBytes", "maxDecodedBytes", "maxDepth", "maxNodes"],
        "capabilities.modelTransport.limits",
    )?;
    for (index, tuple) in reported_support.tuples.iter().enumerate() {
        let stated_index = stated_support
            .tuples
            .iter()
            .position(|stated| stated == tuple)
            .ok_or_else(failed)?;
        compare_unknown(
            &stated["tuples"][stated_index],
            &reported["tuples"][index],
            &[
                "operation",
                "profile",
                "codec",
                "envelopeVersion",
                "irVersion",
            ],
            &format!("{PATH}.tuples.{index}"),
        )?;
    }
    Ok(())
}

fn compare_unknown(
    stated: &Value,
    reported: &Value,
    known: &[&str],
    path: &str,
) -> Result<(), SessionAgreementError> {
    let Value::Object(stated) = stated else {
        return Err(SessionAgreementError::CapabilityMember(path.into()));
    };
    let Value::Object(reported) = reported else {
        return Err(SessionAgreementError::CapabilityMember(path.into()));
    };
    let unknown = reported
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    compare_members(stated, &unknown, path)
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
