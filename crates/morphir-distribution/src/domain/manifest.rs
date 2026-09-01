//! Strict wire DTOs and validated release manifest records.

use super::identity::portable_token;
use super::{
    ArtifactFilename, Channel, ExtensionId, RelativeArtifactPath, SchemaVersion, Sha256Digest,
};
use crate::error::{Result, invalid_value};
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
    /// Morphir workspace discovery provider.
    Workspace,
}

/// One source language accepted by a schema `"1.0"` frontend extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendLanguageRecord {
    id: String,
    file_extensions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrontendLanguageRecordWire {
    id: String,
    file_extensions: Vec<String>,
}

impl<'de> Deserialize<'de> for FrontendLanguageRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FrontendLanguageRecordWire::deserialize(deserializer)?;
        let id = wire.id.trim();
        if id.is_empty() || id != wire.id {
            return Err(serde::de::Error::custom(
                "frontend languages must have non-empty trimmed IDs",
            ));
        }
        if !valid_frontend_file_extensions(&wire.file_extensions) {
            return Err(serde::de::Error::custom(
                "frontend file extensions must be non-empty, dot-prefixed, trimmed, and unique",
            ));
        }
        Ok(Self {
            id: wire.id,
            file_extensions: wire.file_extensions,
        })
    }
}

impl FrontendLanguageRecord {
    /// Return the stable source-language identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the non-empty unique file extensions recognized for this language.
    pub fn file_extensions(&self) -> &[String] {
        &self.file_extensions
    }
}

/// Frontend-specific metadata carried by schema `"1.0"` release records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendRecord {
    languages: Vec<FrontendLanguageRecord>,
    ir_versions: Vec<String>,
    #[serde(default = "default_frontend_compile")]
    compile: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrontendRecordWire {
    languages: Vec<FrontendLanguageRecord>,
    ir_versions: Vec<String>,
    #[serde(default = "default_frontend_compile")]
    compile: bool,
}

fn default_frontend_compile() -> bool {
    true
}

fn valid_frontend_file_extensions(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed == value && trimmed.starts_with('.')
        })
        && values
            .iter()
            .map(|value| value.trim())
            .collect::<BTreeSet<_>>()
            .len()
            == values.len()
}

impl<'de> Deserialize<'de> for FrontendRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FrontendRecordWire::deserialize(deserializer)?;
        if wire.languages.is_empty()
            || wire
                .languages
                .iter()
                .map(|language| language.id().trim())
                .collect::<BTreeSet<_>>()
                .len()
                != wire.languages.len()
        {
            return Err(serde::de::Error::custom(
                "frontend languages must be non-empty and have unique IDs",
            ));
        }
        if !valid_backend_identifiers(&wire.ir_versions) {
            return Err(serde::de::Error::custom(
                "frontend IR versions must be non-empty and unique",
            ));
        }
        Ok(Self {
            languages: wire.languages,
            ir_versions: wire.ir_versions,
            compile: wire.compile,
        })
    }
}

impl FrontendRecord {
    /// Return the non-empty set of source languages accepted by the frontend.
    pub fn languages(&self) -> &[FrontendLanguageRecord] {
        &self.languages
    }

    /// Return the non-empty unique Morphir IR versions produced by the frontend.
    pub fn ir_versions(&self) -> &[String] {
        &self.ir_versions
    }

    /// Return whether this frontend accepts compile requests.
    pub fn compile(&self) -> bool {
        self.compile
    }
}

/// Backend-specific metadata carried by schema `"1.0"` release records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRecord {
    targets: Vec<String>,
    ir_versions: Vec<String>,
    generate: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackendRecordWire {
    targets: Vec<String>,
    ir_versions: Vec<String>,
    #[serde(default = "default_backend_generate")]
    generate: bool,
}

fn default_backend_generate() -> bool {
    true
}

fn valid_backend_identifiers(values: &[String]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty() && trimmed == value
        })
        && values
            .iter()
            .map(|value| value.trim())
            .collect::<BTreeSet<_>>()
            .len()
            == values.len()
}

impl<'de> Deserialize<'de> for BackendRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BackendRecordWire::deserialize(deserializer)?;
        if !valid_backend_identifiers(&wire.targets) {
            return Err(serde::de::Error::custom(
                "backend targets must be non-empty and unique",
            ));
        }
        if !valid_backend_identifiers(&wire.ir_versions) {
            return Err(serde::de::Error::custom(
                "backend IR versions must be non-empty and unique",
            ));
        }
        Ok(Self {
            targets: wire.targets,
            ir_versions: wire.ir_versions,
            generate: wire.generate,
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

    /// Return whether this backend accepts generate requests.
    pub fn generate(&self) -> bool {
        self.generate
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
    schema_version: SchemaVersion,
    id: ExtensionId,
    name: String,
    version: Version,
    channels: Vec<Channel>,
    mep_versions: Vec<String>,
    capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontend: Option<FrontendRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseRecordWire {
    schema_version: SchemaVersion,
    id: ExtensionId,
    name: String,
    version: Version,
    #[serde(default)]
    channels: Vec<Channel>,
    mep_versions: Vec<String>,
    capabilities: Vec<Capability>,
    #[serde(default)]
    frontend: FieldPresence<FrontendRecord>,
    #[serde(default)]
    backend: FieldPresence<BackendRecord>,
    artifacts: Vec<ArtifactRecord>,
}

impl ReleaseRecord {
    /// Return the index record schema version.
    pub fn schema_version(&self) -> SchemaVersion {
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

    /// Return frontend-specific metadata when declared by a schema `"1.0"` record.
    pub fn frontend(&self) -> Option<&FrontendRecord> {
        self.frontend.as_ref()
    }

    /// Return backend-specific metadata when declared by a schema `"1.0"` record.
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
        if !supports_release_schema_version(wire.schema_version) {
            return Err(serde::de::Error::custom(format!(
                "unsupported extension index schema version {}; supported range is {} through {}",
                wire.schema_version, MINIMUM_RELEASE_SCHEMA_VERSION, CURRENT_RELEASE_SCHEMA_VERSION
            )));
        }
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
        let declares_frontend = wire.capabilities.contains(&Capability::Frontend);
        match (declares_frontend, wire.frontend.has_value()) {
            (true, false) => {
                return Err(serde::de::Error::custom(
                    "frontend metadata is required when frontend capability is declared",
                ));
            }
            (false, _) if !wire.frontend.is_missing() => {
                return Err(serde::de::Error::custom(
                    "frontend metadata requires the frontend capability",
                ));
            }
            _ => {}
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
            frontend: wire.frontend.into_option(),
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
