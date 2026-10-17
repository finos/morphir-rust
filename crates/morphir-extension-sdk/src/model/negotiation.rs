//! Pure bilateral admission for complete MorphirModel transport tuples.

use super::wire::{ModelTransportLimits, ModelTransportSupport};
use semver::Version;
use std::collections::HashSet;

/// Exact MEP draft authorizing the additive model methods.
pub const MODEL_MEP_VERSION: &str = "0.2.0-draft.1";
/// Independent exact version of the model envelope.
pub const MODEL_ENVELOPE_VERSION: &str = "0.1.0-draft.1";

/// A failed protocol, support validation, or bilateral selection rule.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelNegotiationError {
    /// Rich model transport requires the exact supported MEP draft.
    #[error("unsupported model protocol version")]
    UnsupportedProtocol,
    /// A rich selection cannot substitute for a live host offer.
    #[error("model selection requires a host offer")]
    MissingHostOffer,
    /// A support member is malformed, duplicated, or outside protocol bounds.
    #[error("invalid model transport support: {0}")]
    InvalidSupport(&'static str),
    /// Selection introduces a tuple absent from the host offer.
    #[error("model tuple was not offered")]
    UnofferedTuple,
    /// Selection raises one of the host's resource bounds.
    #[error("model selection exceeds offered limits")]
    RaisedLimit,
    /// Selection introduces a guarantee identifier absent from the host offer.
    #[error("model guarantee was not offered")]
    UnofferedGuarantee,
}

/// Validated model transport support admitted against both peers' offers.
///
/// This authorizes complete transport tuples, not evidence or model semantics.
/// Empty support grants no operation. The contained wire DTO is immutable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedModelTransport {
    support: ModelTransportSupport,
}

impl NegotiatedModelTransport {
    /// Intersect complete tuples and guarantee IDs, and take each lower bound.
    ///
    /// Tuple order and guarantee order follow the host offer. No independent
    /// field lists are combined into additional tuples.
    pub fn select(
        host: &ModelTransportSupport,
        guest: &ModelTransportSupport,
    ) -> Result<Self, ModelNegotiationError> {
        validate(host)?;
        validate(guest)?;
        Ok(Self {
            support: ModelTransportSupport {
                tuples: host
                    .tuples
                    .iter()
                    .filter(|tuple| guest.tuples.contains(tuple))
                    .cloned()
                    .collect(),
                limits: ModelTransportLimits {
                    max_encoded_bytes: host
                        .limits
                        .max_encoded_bytes
                        .min(guest.limits.max_encoded_bytes),
                    max_decoded_bytes: host
                        .limits
                        .max_decoded_bytes
                        .min(guest.limits.max_decoded_bytes),
                    max_depth: host.limits.max_depth.min(guest.limits.max_depth),
                    max_nodes: host.limits.max_nodes.min(guest.limits.max_nodes),
                },
                guarantees: host
                    .guarantees
                    .iter()
                    .filter(|id| guest.guarantees.contains(id))
                    .cloned()
                    .collect(),
            },
        })
    }

    /// Validate a guest's selected support against the live host offer.
    ///
    /// Legacy `0.1` never grants rich support. Under the exact new draft,
    /// omitted or empty support grants no model operation. Other protocol
    /// versions are refused, including other prereleases of the same version.
    ///
    /// ```
    /// use morphir_extension_sdk::model::{MODEL_MEP_VERSION, NegotiatedModelTransport};
    /// assert!(NegotiatedModelTransport::from_selection("0.1", None, None).unwrap().is_none());
    /// assert!(NegotiatedModelTransport::from_selection(MODEL_MEP_VERSION, None, None).unwrap().is_none());
    /// assert!(NegotiatedModelTransport::from_selection("0.2.0-draft.2", None, None).is_err());
    /// ```
    pub fn from_selection(
        protocol: &str,
        host: Option<&ModelTransportSupport>,
        selected: Option<&ModelTransportSupport>,
    ) -> Result<Option<Self>, ModelNegotiationError> {
        if protocol == "0.1" {
            return Ok(None);
        }
        if protocol != MODEL_MEP_VERSION {
            return Err(ModelNegotiationError::UnsupportedProtocol);
        }
        if let Some(host) = host {
            validate(host)?;
        }
        let Some(selected) = selected else {
            return Ok(None);
        };
        validate(selected)?;
        let Some(host) = host else {
            return if selected.tuples.is_empty() {
                Ok(None)
            } else {
                Err(ModelNegotiationError::MissingHostOffer)
            };
        };
        if selected
            .tuples
            .iter()
            .any(|tuple| !host.tuples.contains(tuple))
        {
            return Err(ModelNegotiationError::UnofferedTuple);
        }
        if !limits_within(&selected.limits, &host.limits) {
            return Err(ModelNegotiationError::RaisedLimit);
        }
        if selected
            .guarantees
            .iter()
            .any(|id| !host.guarantees.contains(id))
        {
            return Err(ModelNegotiationError::UnofferedGuarantee);
        }
        if selected.tuples.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Self {
                support: selected.clone(),
            }))
        }
    }

    /// Read the admitted immutable tuples, limits, and interpretable IDs.
    pub fn support(&self) -> &ModelTransportSupport {
        &self.support
    }
}

fn limits_within(selected: &ModelTransportLimits, offered: &ModelTransportLimits) -> bool {
    selected.max_encoded_bytes <= offered.max_encoded_bytes
        && selected.max_decoded_bytes <= offered.max_decoded_bytes
        && selected.max_depth <= offered.max_depth
        && selected.max_nodes <= offered.max_nodes
}

fn validate(support: &ModelTransportSupport) -> Result<(), ModelNegotiationError> {
    use ModelNegotiationError::InvalidSupport;
    let limits = &support.limits;
    if [
        limits.max_encoded_bytes,
        limits.max_decoded_bytes,
        limits.max_depth,
        limits.max_nodes,
    ]
    .iter()
    .any(|value| *value == 0 || *value > i32::MAX as u32)
    {
        return Err(InvalidSupport("limits"));
    }
    let mut tuples = HashSet::new();
    for tuple in &support.tuples {
        if tuple.profile.trim().is_empty() {
            return Err(InvalidSupport("profile"));
        }
        if tuple.envelope_version != MODEL_ENVELOPE_VERSION {
            return Err(InvalidSupport("envelopeVersion"));
        }
        // The IR release grammar is narrower than SemVer and is independent
        // of the protocol draft's maturity. A draft IR 4 still spells 4.0.0.
        let release = Version::parse(&tuple.ir_version).map_err(|_| InvalidSupport("irVersion"))?;
        if release.major < 3
            || release.major > u32::MAX.into()
            || release.minor > u32::MAX.into()
            || release.patch > u32::MAX.into()
            || !release.pre.is_empty()
            || !release.build.is_empty()
        {
            return Err(InvalidSupport("irVersion"));
        }
        if !tuples.insert(tuple) {
            return Err(InvalidSupport("duplicate tuple"));
        }
    }
    let mut guarantees = HashSet::new();
    for guarantee in &support.guarantees {
        if guarantee.trim().is_empty() || !guarantees.insert(guarantee) {
            return Err(InvalidSupport("guarantees"));
        }
    }
    Ok(())
}
