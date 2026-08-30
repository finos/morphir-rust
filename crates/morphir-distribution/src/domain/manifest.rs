//! Strict wire DTOs and validated release manifest records.

use super::identity::portable_token;
use super::{ArtifactFilename, Channel, ExtensionId, RelativeArtifactPath, Sha256Digest};
use crate::error::{Result, invalid_value};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt;

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
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
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
}

/// Backend-specific metadata carried by schema-v2 release records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRecord {
    targets: Vec<String>,
    ir_versions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackendRecordWire {
    targets: Vec<String>,
    ir_versions: Vec<String>,
}

impl<'de> Deserialize<'de> for BackendRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BackendRecordWire::deserialize(deserializer)?;
        if wire.targets.is_empty()
            || wire.targets.iter().any(|target| target.trim().is_empty())
            || wire.targets.iter().collect::<BTreeSet<_>>().len() != wire.targets.len()
        {
            return Err(serde::de::Error::custom(
                "backend targets must be non-empty and unique",
            ));
        }
        if wire.ir_versions.is_empty()
            || wire
                .ir_versions
                .iter()
                .any(|ir_version| ir_version.trim().is_empty())
            || wire.ir_versions.iter().collect::<BTreeSet<_>>().len() != wire.ir_versions.len()
        {
            return Err(serde::de::Error::custom(
                "backend IR versions must be non-empty and unique",
            ));
        }
        Ok(Self {
            targets: wire.targets,
            ir_versions: wire.ir_versions,
        })
    }
}

impl BackendRecord {
    /// Return the non-empty unique backend target names.
    pub fn targets(&self) -> &[String] {
        &self.targets
    }

    /// Return the non-empty unique Morphir IR versions supported by the backend.
    pub fn ir_versions(&self) -> &[String] {
        &self.ir_versions
    }
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactRecordWire {
    runtime: ArtifactRuntime,
    #[serde(default)]
    platform: FieldPresence<Platform>,
    source: ArtifactSource,
    sha256: Sha256Digest,
    filename: ArtifactFilename,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    executable: bool,
}

impl ArtifactRecord {
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
        let wire = ArtifactRecordWire::deserialize(deserializer)?;
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
        if wire.args.iter().any(|argument| argument.contains('\0')) {
            return Err(serde::de::Error::custom(
                "process arguments cannot contain NUL",
            ));
        }
        Ok(Self {
            runtime: wire.runtime,
            platform: wire.platform.into_option(),
            source: wire.source,
            sha256: wire.sha256,
            filename: wire.filename,
            args: wire.args,
            executable: wire.executable,
        })
    }
}

/// One exact extension release from a JSONL history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRecord {
    schema_version: u32,
    id: ExtensionId,
    name: String,
    version: Version,
    channels: Vec<Channel>,
    mep_versions: Vec<String>,
    capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseRecordWire {
    schema_version: u32,
    id: ExtensionId,
    name: String,
    version: Version,
    #[serde(default)]
    channels: Vec<Channel>,
    mep_versions: Vec<String>,
    capabilities: Vec<Capability>,
    #[serde(default)]
    backend: FieldPresence<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
}

impl ReleaseRecord {
    /// Return the index record schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Return the stable portable identity.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.id
    }

    /// Return the non-empty human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return moving channels that point at this release.
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Return non-empty supported MEP version spellings.
    pub fn mep_versions(&self) -> &[String] {
        &self.mep_versions
    }

    /// Return the non-empty set of advertised operations.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Return backend-specific metadata when declared by a schema-v2 record.
    pub fn backend(&self) -> Option<&BackendRecord> {
        self.backend.as_ref()
    }

    /// Return the non-empty platform artifact set.
    pub fn artifacts(&self) -> &[ArtifactRecord] {
        &self.artifacts
    }
}

impl<'de> Deserialize<'de> for ReleaseRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ReleaseRecordWire::deserialize(deserializer)?;
        if wire.name.trim().is_empty() {
            return Err(serde::de::Error::custom("extension name cannot be empty"));
        }
        if wire.mep_versions.is_empty()
            || wire
                .mep_versions
                .iter()
                .any(|version| version.trim().is_empty())
        {
            return Err(serde::de::Error::custom(
                "MEP versions must contain non-empty values",
            ));
        }
        if wire.capabilities.is_empty() {
            return Err(serde::de::Error::custom(
                "extension capabilities cannot be empty",
            ));
        }
        if wire.capabilities.iter().collect::<BTreeSet<_>>().len() != wire.capabilities.len() {
            return Err(serde::de::Error::custom(
                "extension capabilities cannot contain duplicates",
            ));
        }
        match wire.schema_version {
            1 => {
                if !wire.backend.is_missing() {
                    return Err(serde::de::Error::custom(
                        "schema-v1 records cannot declare backend metadata",
                    ));
                }
                if wire
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.runtime() != ArtifactRuntime::Process)
                {
                    return Err(serde::de::Error::custom(
                        "schema-v1 records require process artifacts",
                    ));
                }
            }
            2 => {
                let declares_backend = wire.capabilities.contains(&Capability::Backend);
                match (declares_backend, wire.backend.has_value()) {
                    (true, false) => {
                        return Err(serde::de::Error::custom(
                            "backend metadata is required when backend capability is declared",
                        ));
                    }
                    (false, _) if !wire.backend.is_missing() => {
                        return Err(serde::de::Error::custom(
                            "backend metadata requires the backend capability",
                        ));
                    }
                    _ => {}
                }
            }
            version => {
                return Err(serde::de::Error::custom(format!(
                    "unsupported extension index schema version {version}"
                )));
            }
        }
        if wire.artifacts.is_empty() {
            return Err(serde::de::Error::custom(
                "release artifacts cannot be empty",
            ));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            id: wire.id,
            name: wire.name,
            version: wire.version,
            channels: wire.channels,
            mep_versions: wire.mep_versions,
            capabilities: wire.capabilities,
            backend: wire.backend.into_option(),
            artifacts: wire.artifacts,
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
