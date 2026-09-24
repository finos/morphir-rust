//! Preserve supplied claim sets and keep converted declarations off legacy wires.

use morphir_extension_sdk::claims::CapabilityClaimSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether a host has checked the artifact's capability claims with a probe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimCheck {
    /// Metadata supplied by a publisher or converted from old flat keys.
    #[default]
    Unchecked,
    /// Metadata verified by a publication or installation probe.
    Probed,
}

/// The operation that supplied an installation probe's claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeSource {
    /// The guest answered the one-shot description request.
    Describe,
    /// The host reconstructed a claim set from a negotiated session.
    SessionFallback,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
struct PreservedClaims {
    wire: Value,
    #[serde(skip)]
    parsed: CapabilityClaimSet,
}

impl PartialEq for PreservedClaims {
    fn eq(&self, other: &Self) -> bool {
        self.wire == other.wire
    }
}
impl Eq for PreservedClaims {}

impl<'de> Deserialize<'de> for PreservedClaims {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut wire = Value::deserialize(deserializer)?;
        super::compatibility::normalize_document(&mut wire)?;
        let parsed: CapabilityClaimSet =
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

/// Why a `describe` answer does not agree with the declared claims.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DescribedAgreementError {
    /// A supplied claim set and the answer differ.
    #[error(transparent)]
    Claims(morphir_extension_sdk::claims::ClaimsAgreementError),
    /// A declaration converted from a version-1 record disagrees with the answer.
    #[error(transparent)]
    Session(morphir_extension_sdk::claims::SessionAgreementError),
    /// The answer claims no protocol version.
    #[error("describe answered with no protocol version")]
    NoProtocolVersion,
}

/// Claims fields embedded in an artifact or installed extension record.
/// A synthesized claim set is available to readers but omitted by legacy writers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "Value")]
pub struct ClaimsRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claims: Option<Box<PreservedClaims>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim_check: Option<ClaimCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe_source: Option<ProbeSource>,
    #[serde(skip)]
    legacy: Option<Box<PreservedClaims>>,
}

impl TryFrom<Value> for ClaimsRecord {
    type Error = String;

    fn try_from(mut value: Value) -> Result<Self, Self::Error> {
        super::normalize_record(&mut value)?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            claims: Option<Box<PreservedClaims>>,
            claim_check: Option<ClaimCheck>,
            probe_source: Option<ProbeSource>,
        }
        let wire: Wire = serde_json::from_value(value).map_err(|error| error.to_string())?;
        Ok(Self {
            claims: wire.claims,
            claim_check: wire.claim_check,
            probe_source: wire.probe_source,
            legacy: None,
        })
    }
}

impl ClaimsRecord {
    pub(crate) fn has_supplied_claims(&self) -> bool {
        self.claims.is_some()
    }

    pub(crate) fn as_declared(&self) -> Self {
        Self {
            claim_check: self.claims.as_ref().map(|_| ClaimCheck::Unchecked),
            probe_source: None,
            ..self.clone()
        }
    }

    /// Record a declaration without claiming local verification.
    pub fn declared(claims: CapabilityClaimSet) -> Self {
        Self {
            claims: Some(Box::new(PreservedClaims {
                wire: serde_json::to_value(&claims).expect("claim set serializes"),
                parsed: claims,
            })),
            claim_check: Some(ClaimCheck::Unchecked),
            probe_source: None,
            legacy: None,
        }
    }

    /// Mark the preserved declaration as verified. Legacy records keep their old wire shape.
    pub fn probed(&self, source: ProbeSource) -> Self {
        Self {
            claim_check: self.claims.as_ref().map(|_| ClaimCheck::Probed),
            probe_source: self.claims.as_ref().map(|_| source),
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
    ) -> Result<(), morphir_extension_sdk::claims::SessionAgreementError> {
        let declared = self.claims().expect("resolved record has a declaration");
        if self.claims.is_some() {
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

    /// Check a `morphir.extension.describe` answer against the declaration.
    ///
    /// A claim set the extension supplied must agree with its answer exactly. A declaration
    /// converted from a version-1 record holds only the members the flat format can express, and
    /// its defaults are the host's, not the extension's, so the answer is checked like a session
    /// instead: identity, the protocol, the capability kinds, and the flat members.
    pub fn check_described(
        &self,
        described: &CapabilityClaimSet,
    ) -> Result<(), DescribedAgreementError> {
        let declared = self.claims().expect("resolved record has a declaration");
        if self.claims.is_some() {
            return declared
                .check_claims(described)
                .map_err(DescribedAgreementError::Claims);
        }
        // A describe answer lists every version the extension serves, not a negotiated one, so
        // agreement uses a version both sides list. With none shared, the first answered version
        // is checked, and the session rule reports the mismatch.
        let protocol_version = described
            .protocol_versions
            .iter()
            .find(|version| declared.protocol_versions.contains(version))
            .or_else(|| described.protocol_versions.first())
            .ok_or(DescribedAgreementError::NoProtocolVersion)?;
        self.check_session(
            protocol_version,
            &described.extension,
            &described.capabilities,
        )
        .map_err(DescribedAgreementError::Session)
    }

    /// Return how a local probe obtained the claims, when recorded.
    pub fn probe_source(&self) -> Option<ProbeSource> {
        (self.claim_check() == ClaimCheck::Probed)
            .then_some(self.probe_source)
            .flatten()
    }

    /// Return the supplied or converted capability claim set, if available.
    pub fn claims(&self) -> Option<&CapabilityClaimSet> {
        self.claims
            .as_ref()
            .or(self.legacy.as_ref())
            .map(|value| &value.parsed)
    }

    /// Return the recorded check status, defaulting old declarations to `unchecked`.
    pub fn claim_check(&self) -> ClaimCheck {
        self.claim_check.unwrap_or_default()
    }

    pub(crate) fn supply_legacy(&mut self, claims: CapabilityClaimSet) {
        if self.claims.is_none() {
            self.legacy = Some(Box::new(PreservedClaims {
                wire: serde_json::to_value(&claims).expect("claim set serializes"),
                parsed: claims,
            }));
            self.claim_check = None;
            self.probe_source = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::claims::SessionAgreementError;
    use morphir_extension_sdk::protocol::MEP_VERSION;
    use serde_json::json;

    fn legacy_declaration() -> ClaimsRecord {
        let claims = serde_json::from_value(json!({
            "claimsVersion": "0.1.0-draft.2", "protocolVersions": [MEP_VERSION],
            "extension": {"id": "sample", "name": "Sample", "version": "1.0.0", "types": ["frontend", "backend"]},
            "capabilities": {
                "frontend": {"languages": [{"id": "elm", "fileExtensions": [".elm"]}], "irVersions": ["3"], "compile": true},
                "backend": {"targets": ["text"], "irVersions": ["3"], "generate": true}
            }
        })).unwrap();
        let mut record = ClaimsRecord::default();
        record.supply_legacy(claims);
        record
    }

    #[test]
    fn legacy_agreement_ignores_only_members_outside_flat_keys() {
        let record = legacy_declaration();
        let claims = record.claims().unwrap();
        let mut reported = claims.capabilities.clone();
        reported["frontend"]["incremental"] = true.into();
        reported["backend"]["future"] = json!({"extra": true});
        reported.insert("streaming".into(), true.into());
        assert!(
            record
                .check_session(MEP_VERSION, &claims.extension, &reported)
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
                record.check_session(MEP_VERSION, &claims.extension, &changed),
                Err(SessionAgreementError::CapabilityMember(format!(
                    "capabilities.{kind}.{member}"
                )))
            );
        }
        let mut extension = claims.extension.clone();
        extension.name = "Different".into();
        assert_eq!(
            record.check_session(MEP_VERSION, &extension, &reported),
            Err(SessionAgreementError::Identity("name"))
        );
        assert!(matches!(
            record.check_session("unsupported", &claims.extension, &reported),
            Err(SessionAgreementError::ProtocolVersion(_))
        ));
        let mut extension = claims.extension.clone();
        extension
            .types
            .push(morphir_extension_sdk::ExtensionType::Validator);
        assert!(matches!(
            record.check_session(MEP_VERSION, &extension, &reported),
            Err(SessionAgreementError::CapabilityKind(_))
        ));
        // Supplied claim sets retain the full protocol rule, including optional members.
        let supplied = ClaimsRecord::declared(claims.clone());
        assert!(
            supplied
                .check_session(MEP_VERSION, &claims.extension, &reported)
                .is_err()
        );
    }

    /// An extension that answers `describe` reports members a version-1 record cannot express,
    /// such as `incremental`, and the host's converted defaults need not match them.
    #[test]
    fn a_describe_answer_is_checked_against_converted_claims_like_a_session() {
        let record = legacy_declaration();
        let mut described = record.claims().unwrap().clone();
        described.capabilities["frontend"]["incremental"] = false.into();
        described.capabilities["frontend"]["multiDocument"] = false.into();
        described.capabilities.insert(
            "workspace".into(),
            json!({"protocolVersions": ["0.1.0-draft.1"], "discover": true}),
        );
        assert_eq!(record.check_described(&described), Ok(()));

        let mut changed = described.clone();
        changed.capabilities["frontend"]["irVersions"] = json!(["4"]);
        assert_eq!(
            record.check_described(&changed),
            Err(DescribedAgreementError::Session(
                SessionAgreementError::CapabilityMember("capabilities.frontend.irVersions".into())
            ))
        );
        let mut renamed = described.clone();
        renamed.extension.name = "Different".into();
        assert!(matches!(
            record.check_described(&renamed),
            Err(DescribedAgreementError::Session(
                SessionAgreementError::Identity("name")
            ))
        ));
        // The answer lists every version the extension serves; one shared version is enough.
        let mut multi = described.clone();
        multi.protocol_versions = vec!["0.2".into(), MEP_VERSION.into()];
        assert_eq!(record.check_described(&multi), Ok(()));
        let mut foreign = described.clone();
        foreign.protocol_versions = vec!["0.2".into()];
        assert!(matches!(
            record.check_described(&foreign),
            Err(DescribedAgreementError::Session(
                SessionAgreementError::ProtocolVersion(_)
            ))
        ));
        let mut silent = described.clone();
        silent.protocol_versions.clear();
        assert_eq!(
            record.check_described(&silent),
            Err(DescribedAgreementError::NoProtocolVersion)
        );
    }

    /// A claim set the extension supplied is its own statement, so its answer must match it exactly.
    #[test]
    fn a_describe_answer_must_match_supplied_claims_exactly() {
        let claims = legacy_declaration().claims().unwrap().clone();
        let supplied = ClaimsRecord::declared(claims.clone());
        assert_eq!(supplied.check_described(&claims), Ok(()));
        let mut described = claims.clone();
        described.capabilities["frontend"]["incremental"] = false.into();
        assert!(matches!(
            supplied.check_described(&described),
            Err(DescribedAgreementError::Claims(_))
        ));
    }

    #[test]
    fn provenance_never_promotes_a_legacy_record_to_a_supplied_claims() {
        let record = legacy_declaration();
        for transformed in [
            record.as_declared(),
            record.probed(ProbeSource::Describe),
            record.probed(ProbeSource::SessionFallback),
        ] {
            assert_eq!(serde_json::to_value(&transformed).unwrap(), json!({}));
            assert_eq!(
                serde_json::to_value(transformed.claims()).unwrap(),
                serde_json::to_value(record.claims()).unwrap()
            );
            assert_eq!(transformed.claim_check(), ClaimCheck::Unchecked);
            assert_eq!(transformed.probe_source(), None);
        }
    }
}
