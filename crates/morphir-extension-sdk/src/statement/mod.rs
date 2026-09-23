//! Guest-authored capability statements, independent of a negotiated session.
//!
//! Capability objects retain unknown members when read and written. Only the
//! exact draft below is supported; critical paths must be understood explicitly.

mod agreement;
pub use agreement::SessionAgreementError;

use crate::{ExtensionCapabilities, ExtensionInfo};
use semver::{Comparator, Version, VersionReq};
use serde::{Deserialize, Serialize, de::Error as _};
use serde_json::{Map, Value};

/// The statement format written and read by this SDK.
pub const STATEMENT_VERSION: &str = "0.1.0-draft.1";

/// A guest's identity, supported protocols, and complete capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatement {
    #[serde(deserialize_with = "read_statement_version")]
    statement_version: Version,
    /// Every MEP version the guest speaks.
    pub protocol_versions: Vec<String>,
    /// Guest identity and strictly recognized capability kinds.
    pub extension: ExtensionInfo,
    /// Capability members, including optional members unknown to this reader.
    pub capabilities: Map<String, Value>,
    /// Requirements imposed on the host, when declared by the guest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<StatementRequirements>,
    /// Member paths whose meaning readers must understand.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "read_critical"
    )]
    pub critical: Vec<String>,
}

impl CapabilityStatement {
    /// Build a statement from a guest's existing discovery metadata.
    ///
    /// ```
    /// use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
    /// use morphir_extension_sdk::statement::CapabilityStatement;
    /// let statement = CapabilityStatement::from_metadata(
    ///     vec!["0.1".into()], ExtensionInfo::default(),
    ///     &ExtensionCapabilities::default(),
    /// ).unwrap();
    /// assert_eq!(statement.statement_version().to_string(), "0.1.0-draft.1");
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

    /// Build a statement from session metadata without dropping unknown members.
    /// A fallback caller supplies only the negotiated protocol version. Sessions
    /// do not report requirements or critical paths, so both remain absent.
    pub fn from_session(
        protocol_versions: Vec<String>,
        extension: ExtensionInfo,
        capabilities: Map<String, Value>,
    ) -> Self {
        Self {
            statement_version: Version::parse(STATEMENT_VERSION)
                .expect("statement format constant is SemVer"),
            protocol_versions,
            extension,
            capabilities,
            requires: None,
            critical: vec![],
        }
    }

    /// The validated statement format version.
    pub fn statement_version(&self) -> &Version {
        &self.statement_version
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
pub struct StatementRequirements {
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

fn read_statement_version<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Version, D::Error> {
    let version = Version::deserialize(deserializer)?;
    let supported = VersionReq::parse("=0.1.0-draft.1").expect("exact statement draft requirement");
    if supported.matches(&version) {
        Ok(version)
    } else {
        Err(D::Error::custom(format!(
            "unsupported statementVersion '{version}'"
        )))
    }
}

fn read_critical<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let paths = Vec::<String>::deserialize(deserializer)?;
    for path in &paths {
        if !understands(path) {
            return Err(D::Error::custom(format!(
                "unknown critical member '{path}'"
            )));
        }
    }
    Ok(paths)
}

fn understands(path: &str) -> bool {
    matches!(
        path,
        "statementVersion"
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
