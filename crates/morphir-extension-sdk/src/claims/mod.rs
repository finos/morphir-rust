//! Guest-authored capability claim sets, independent of a negotiated session.
//!
//! Capability objects retain unknown members when read and written. Only the
//! draft.1 and draft.2 formats are supported; critical paths must be understood explicitly.

mod agreement;
mod wire;
pub use agreement::{ClaimsAgreementError, SessionAgreementError};

use crate::{ExtensionCapabilities, ExtensionInfo};
use semver::{Comparator, Version, VersionReq};
use serde::{Deserialize, Serialize, de::Error as _};
use serde_json::{Map, Value};

/// The claim set format written by this SDK. Readers also accept draft.1.
pub const CLAIMS_VERSION: &str = "0.1.0-draft.2";

/// A guest's identity, supported protocols, and complete capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "Value")]
pub struct CapabilityClaimSet {
    claims_version: Version,
    /// Every MEP version the guest speaks.
    pub protocol_versions: Vec<String>,
    /// Guest identity and strictly recognized capability kinds.
    pub extension: ExtensionInfo,
    /// Capability members, including optional members unknown to this reader.
    pub capabilities: Map<String, Value>,
    /// Requirements imposed on the host, when declared by the guest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<ClaimsRequirements>,
    /// Member paths whose meaning readers must understand.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "read_critical"
    )]
    pub critical: Vec<String>,
}

impl CapabilityClaimSet {
    /// Build a claim set from a guest's existing discovery metadata.
    ///
    /// ```
    /// use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
    /// use morphir_extension_sdk::claims::CapabilityClaimSet;
    /// let claims = CapabilityClaimSet::from_metadata(
    ///     vec!["0.1".into()], ExtensionInfo::default(),
    ///     &ExtensionCapabilities::default(),
    /// ).unwrap();
    /// assert_eq!(claims.claims_version().to_string(), "0.1.0-draft.2");
    /// ```
    pub fn from_metadata(
        protocol_versions: Vec<String>,
        extension: ExtensionInfo,
        capabilities: &ExtensionCapabilities,
    ) -> Result<Self, serde_json::Error> {
        let capabilities = serde_json::from_value(serde_json::to_value(capabilities)?)?;
        Ok(Self::from_session(
            protocol_versions,
            extension,
            capabilities,
        ))
    }

    /// Build a claim set from session metadata without dropping unknown members.
    /// A fallback caller supplies only the negotiated protocol version. Sessions
    /// do not report requirements or critical paths, so both remain absent.
    pub fn from_session(
        protocol_versions: Vec<String>,
        extension: ExtensionInfo,
        capabilities: Map<String, Value>,
    ) -> Self {
        Self {
            claims_version: Version::parse(CLAIMS_VERSION)
                .expect("claims format constant is SemVer"),
            protocol_versions,
            extension,
            capabilities,
            requires: None,
            critical: vec![],
        }
    }

    /// The validated claims format version.
    pub fn claims_version(&self) -> &Version {
        &self.claims_version
    }

    /// Check all declared host comparators together, including prerelease rules.
    pub fn check_host(&self, host: &Version) -> Result<(), HostRequirementError> {
        let Some(requirements) = &self.requires else {
            return Ok(());
        };
        if requirements.host.is_empty() {
            return Ok(());
        }
        let range = VersionReq {
            comparators: requirements
                .host
                .iter()
                .map(|item| item.0.clone())
                .collect(),
        };
        if range.matches(host) {
            Ok(())
        } else {
            Err(HostRequirementError {
                host: host.clone(),
                range,
            })
        }
    }
}

/// Requirements stated by a guest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimsRequirements {
    /// Single SemVer comparators, all of which must hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host: Vec<HostComparator>,
}

/// A single SemVer comparator rather than a combined range string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HostComparator(Comparator);

impl TryFrom<String> for HostComparator {
    type Error = semver::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Comparator::parse(&value).map(Self)
    }
}

impl From<HostComparator> for String {
    fn from(value: HostComparator) -> Self {
        value.0.to_string()
    }
}

/// The host version lies outside the range declared by the guest.
#[derive(Debug, thiserror::Error)]
#[error("host {host} does not satisfy requires.host [{range}]")]
pub struct HostRequirementError {
    host: Version,
    range: VersionReq,
}

fn read_critical<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let paths = Vec::<String>::deserialize(deserializer)?;
    for path in &paths {
        if !understands_member(path) {
            return Err(D::Error::custom(format!(
                "unknown critical member '{path}'"
            )));
        }
    }
    Ok(paths)
}

/// Whether this SDK understands the semantics of a claim set member path.
pub fn understands_member(path: &str) -> bool {
    matches!(
        path,
        "claimsVersion"
            | "protocolVersions"
            | "extension"
            | "extension.id"
            | "extension.name"
            | "extension.version"
            | "extension.types"
            | "capabilities"
            | "capabilities.frontend"
            | "capabilities.backend"
            | "capabilities.workspace"
            | "capabilities.frontend.languages"
            | "capabilities.frontend.irVersions"
            | "capabilities.frontend.compile"
            | "capabilities.frontend.incremental"
            | "capabilities.frontend.fragments"
            | "capabilities.frontend.multiDocument"
            | "capabilities.backend.targets"
            | "capabilities.backend.irVersions"
            | "capabilities.backend.generate"
            | "capabilities.workspace.protocolVersions"
            | "capabilities.workspace.discover"
            | "capabilities.streaming"
            | "capabilities.incremental"
            | "capabilities.cancellation"
            | "capabilities.progress"
            | "requires"
            | "requires.host"
            | "critical"
    )
}
