//! Wire DTOs and validated release manifest records.

mod capabilities;
mod release;

pub use capabilities::{BackendRecord, FrontendLanguageRecord, FrontendRecord};
pub use release::ReleaseRecord;

use super::identity::portable_token;
use super::{
    ArtifactFilename, Channel, ExtensionId, RelativeArtifactPath, SchemaVersion, Sha256Digest,
};
use crate::error::{Result, invalid_value};
use crate::extension_format::{
    CAPABILITY_PATHS, ExtensionSchemaVersion, StatementProvenance, StatementRecord,
    validate_members,
};
use morphir_extension_sdk::statement::CapabilityStatement;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// The earliest release manifest schema supported by this distribution build.
pub const MINIMUM_RELEASE_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1, 0);

/// The newest release manifest schema supported by this distribution build.
pub const CURRENT_RELEASE_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1, 0);

/// Return whether a release manifest schema version falls within the supported range.
pub(crate) fn supports_release_schema_version(candidate: SchemaVersion) -> bool {
    candidate >= MINIMUM_RELEASE_SCHEMA_VERSION
        && CURRENT_RELEASE_SCHEMA_VERSION.supports(candidate)
}

/// Distinguishes an omitted wire field from an explicit JSON `null`.
#[derive(Default)]
enum FieldPresence<T> {
    #[default]
    Missing,
    Present(Option<T>),
}

impl<'de, T> Deserialize<'de> for FieldPresence<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::Present(Option::<T>::deserialize(deserializer)?))
    }
}

impl<T> FieldPresence<T> {
    fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    fn has_value(&self) -> bool {
        matches!(self, Self::Present(Some(_)))
    }

    fn as_option(&self) -> Option<&T> {
        match self {
            Self::Present(value) => value.as_ref(),
            Self::Missing => None,
        }
    }

    fn into_option(self) -> Option<T> {
        match self {
            Self::Missing | Self::Present(None) => None,
            Self::Present(Some(value)) => Some(value),
        }
    }
}

/// A portable operating-system and CPU-architecture pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Platform {
    os: String,
    arch: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformWire {
    os: String,
    arch: String,
}

impl Platform {
    /// Construct a validated platform pair.
    pub fn new(os: impl Into<String>, arch: impl Into<String>) -> Result<Self> {
        let os = os.into();
        let arch = arch.into();
        if !portable_token(&os) {
            return Err(invalid_value(
                "platform operating system",
                os,
                "expected a lowercase portable token",
            ));
        }
        if arch.is_empty()
            || !arch
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(invalid_value(
                "platform architecture",
                arch,
                "expected a lowercase portable architecture token",
            ));
        }
        Ok(Self { os, arch })
    }

    /// Return the current Rust target platform.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }

    /// Return the operating-system token.
    pub fn os(&self) -> &str {
        &self.os
    }

    /// Return the CPU-architecture token.
    pub fn arch(&self) -> &str {
        &self.arch
    }
}

impl<'de> Deserialize<'de> for Platform {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PlatformWire::deserialize(deserializer)?;
        Self::new(wire.os, wire.arch).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.os, self.arch)
    }
}

/// Artifact runtime supported by this acquisition version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactRuntime {
    /// An executable that communicates through MEP standard streams.
    Process,
    /// A portable WebAssembly module.
    Wasm,
}

/// Artifact source supported by this acquisition version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ArtifactSource {
    /// A raw file below the controlled local index root.
    LocalFile {
        /// Normalized relative path below the index root.
        path: RelativeArtifactPath,
    },
}

/// Extension operation advertised in the controlled index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Source language frontend.
    Frontend,
    /// IR code-generation backend.
    Backend,
    /// IR-to-IR transform.
    Transform,
    /// IR validator.
    Validator,
    /// Morphir workspace discovery provider.
    Workspace,
}

/// One process-specific or portable artifact declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    runtime: ArtifactRuntime,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<Platform>,
    source: ArtifactSource,
    sha256: Sha256Digest,
    filename: ArtifactFilename,
    args: Vec<String>,
    executable: bool,
    #[serde(flatten)]
    statement: StatementRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    critical: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactRecordWire {
    runtime: ArtifactRuntime,
    #[serde(default)]
    platform: FieldPresence<ExtensionPlatform>,
    source: ArtifactSource,
    sha256: Sha256Digest,
    filename: ArtifactFilename,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    executable: bool,
    #[serde(flatten)]
    statement: StatementRecord,
    #[serde(default)]
    critical: Vec<String>,
}

impl ArtifactRecord {
    /// Return the supplied statement or the declaration converted from release metadata.
    pub fn statement(&self) -> Option<&CapabilityStatement> {
        self.statement.statement()
    }

    /// Return how the statement was obtained.
    pub fn statement_provenance(&self) -> StatementProvenance {
        self.statement.provenance()
    }

    pub(crate) fn statement_record(&self) -> &StatementRecord {
        &self.statement
    }

    /// Return the artifact runtime.
    pub fn runtime(&self) -> ArtifactRuntime {
        self.runtime
    }

    /// Return the process target platform, if this artifact has one.
    pub fn platform(&self) -> Option<&Platform> {
        self.platform.as_ref()
    }

    /// Return the controlled source declaration.
    pub fn source(&self) -> &ArtifactSource {
        &self.source
    }

    /// Return the declared SHA-256 digest.
    pub fn digest(&self) -> &Sha256Digest {
        &self.sha256
    }

    /// Return the portable store filename.
    pub fn filename(&self) -> &ArtifactFilename {
        &self.filename
    }

    /// Return immutable process arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return whether owner executable permission should be applied on Unix.
    pub fn executable(&self) -> bool {
        self.executable
    }
}

impl<'de> Deserialize<'de> for ArtifactRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        validate_members(&value, ARTIFACT_PATHS, false).map_err(serde::de::Error::custom)?;
        let wire: ArtifactRecordWire =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        match wire.runtime {
            ArtifactRuntime::Process if !wire.platform.has_value() => {
                return Err(serde::de::Error::custom(
                    "process artifacts require a platform",
                ));
            }
            ArtifactRuntime::Wasm
                if !wire.platform.is_missing() || !wire.args.is_empty() || wire.executable =>
            {
                return Err(serde::de::Error::custom(
                    "wasm artifacts must be portable, argument-free, and non-executable",
                ));
            }
            _ => {}
        }
        match &wire.source {
            ArtifactSource::LocalFile { path } => {
                path.validate_declared().map_err(serde::de::Error::custom)?
            }
        }
        if wire.args.iter().any(|argument| argument.contains('\0')) {
            return Err(serde::de::Error::custom(
                "process arguments cannot contain NUL",
            ));
        }
        Ok(Self {
            runtime: wire.runtime,
            platform: wire.platform.into_option().map(|platform| platform.0),
            source: wire.source,
            sha256: wire.sha256,
            filename: wire.filename,
            args: wire.args,
            executable: wire.executable,
            statement: wire.statement,
            critical: wire.critical,
        })
    }
}

/// A mutually exclusive exact-version or moving-channel request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum Selection {
    /// Resolve the highest compatible version in a moving channel.
    Channel(Channel),
    /// Resolve one exact semantic version independent of channel membership.
    Exact(Version),
}

impl fmt::Display for Selection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel(channel) => write!(formatter, "channel {channel}"),
            Self::Exact(version) => write!(formatter, "version {version}"),
        }
    }
}

#[derive(Deserialize)]
struct ExtensionPlatform(#[serde(deserialize_with = "required_extension_platform")] Platform);

fn required_extension_platform<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Platform, D::Error> {
    crate::extension_format::read_platform(deserializer)?
        .ok_or_else(|| serde::de::Error::custom("expected platform"))
}

const ARTIFACT_PATHS: &[&str] = &[
    "runtime",
    "platform",
    "platform.os",
    "platform.arch",
    "source",
    "source.kind",
    "source.path",
    "sha256",
    "filename",
    "args",
    "executable",
    "statement",
    "statementSource",
    "critical",
];
