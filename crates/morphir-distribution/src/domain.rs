//! Immutable values used by extension indexes and locks.

use crate::error::{Result, invalid_value};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

fn portable_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}

/// A lowercase portable extension identifier such as `morphir-elm`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

impl ExtensionId {
    /// Parse and validate an extension identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "extension id",
                value,
                "expected a lowercase portable token beginning with a letter",
            ))
        }
    }

    /// Return the portable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for ExtensionId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A validated segment in a named preview channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelSegment(String);

impl ChannelSegment {
    /// Parse a lowercase channel segment.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "preview channel segment",
                value,
                "expected a lowercase portable token",
            ))
        }
    }

    /// Return the channel segment.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A moving extension release channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    /// Production releases. Prerelease semantic versions never match.
    Stable,
    /// All preview releases, or one named preview stream.
    Preview(Option<ChannelSegment>),
    /// Alias for the whole preview family whose spelling is retained in locks.
    Insiders,
}

impl Channel {
    /// Parse one supported channel spelling.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "stable" => Ok(Self::Stable),
            "preview" => Ok(Self::Preview(None)),
            "insiders" => Ok(Self::Insiders),
            _ => value
                .strip_prefix("preview/")
                .map(ChannelSegment::parse)
                .transpose()?
                .map(|segment| Self::Preview(Some(segment)))
                .ok_or_else(|| {
                    invalid_value(
                        "channel",
                        value,
                        "expected stable, preview, insiders, or preview/<segment>",
                    )
                }),
        }
    }

    /// Return the exact channel spelling represented by this value.
    pub fn as_str(&self) -> Cow<'_, str> {
        match self {
            Self::Stable => Cow::Borrowed("stable"),
            Self::Preview(None) => Cow::Borrowed("preview"),
            Self::Preview(Some(segment)) => Cow::Owned(format!("preview/{}", segment.as_str())),
            Self::Insiders => Cow::Borrowed("insiders"),
        }
    }

    /// Return the complete serialized channel spelling.
    pub fn to_portable_string(&self) -> String {
        self.as_str().into_owned()
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_portable_string())
    }
}

impl Serialize for Channel {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A validated SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Parse a 64-digit hexadecimal SHA-256 digest.
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() != 64 {
            return Err(invalid_value(
                "SHA-256 digest",
                value,
                "expected exactly 64 hexadecimal digits",
            ));
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let text = std::str::from_utf8(pair).expect("hexadecimal is ASCII");
            bytes[index] = u8::from_str_radix(text, 16).map_err(|_| {
                invalid_value(
                    "SHA-256 digest",
                    value,
                    "expected exactly 64 hexadecimal digits",
                )
            })?;
        }
        Ok(Self(bytes))
    }

    /// Hash an in-memory byte slice.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Return the raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Sha256Digest {
    type Err = crate::DistributionError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A portable operating-system and CPU-architecture pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPlatform")]
pub struct Platform {
    os: String,
    arch: String,
}

#[derive(Deserialize)]
struct UncheckedPlatform {
    os: String,
    arch: String,
}

impl TryFrom<UncheckedPlatform> for Platform {
    type Error = crate::DistributionError;

    fn try_from(value: UncheckedPlatform) -> Result<Self> {
        Self::new(value.os, value.arch)
    }
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
        if !arch
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || arch.is_empty()
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

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.os, self.arch)
    }
}

/// One portable filename used in the content-addressed store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactFilename(String);

impl ArtifactFilename {
    /// Parse a filename that cannot select another directory.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let invalid = value.is_empty()
            || value == "."
            || value == ".."
            || value.contains('/')
            || value.contains('\\')
            || value.contains('\0');
        if invalid {
            Err(invalid_value(
                "artifact filename",
                value,
                "expected one non-special portable path component",
            ))
        } else {
            Ok(Self(value))
        }
    }

    /// Return the filename.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ArtifactFilename {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArtifactFilename {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A relative path declared by a local-file index source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativeArtifactPath(PathBuf);

impl RelativeArtifactPath {
    /// Parse a relative path without parent traversal.
    pub fn parse(value: impl Into<PathBuf>) -> Result<Self> {
        let value = value.into();
        let is_safe = !value.as_os_str().is_empty()
            && !value.is_absolute()
            && !value.to_string_lossy().contains('\\')
            && value
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
        if is_safe {
            Ok(Self(value))
        } else {
            Err(invalid_value(
                "local artifact path",
                value.to_string_lossy(),
                "expected a relative path without parent traversal",
            ))
        }
    }

    /// Return the relative path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Serialize for RelativeArtifactPath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string_lossy())
    }
}

impl<'de> Deserialize<'de> for RelativeArtifactPath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = PathBuf::from(String::deserialize(deserializer)?);
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Artifact runtime supported by this acquisition version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactRuntime {
    /// An executable that communicates through MEP standard streams.
    Process,
}

/// Artifact source supported by this acquisition version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ArtifactSource {
    /// A raw file below the controlled local index root.
    LocalFile {
        /// Relative path below the index root.
        path: RelativeArtifactPath,
    },
}

/// Extension operation advertised in the controlled index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// One platform-specific artifact declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    /// Runtime used to launch the artifact.
    pub runtime: ArtifactRuntime,
    /// Supported target platform.
    pub platform: Platform,
    /// Controlled source of the raw bytes.
    pub source: ArtifactSource,
    /// Expected digest of the raw bytes.
    pub sha256: Sha256Digest,
    /// Filename used in the content-addressed store.
    pub filename: ArtifactFilename,
    /// Arguments supplied every time the process starts.
    #[serde(default)]
    pub args: Vec<String>,
    /// Whether publication should add owner executable permission on Unix.
    #[serde(default)]
    pub executable: bool,
}

/// One exact extension release from a JSONL history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRecord {
    /// Index record schema version. Version 1 is currently supported.
    pub schema_version: u32,
    /// Stable portable extension identity.
    pub id: ExtensionId,
    /// Human-readable extension name used during MEP discovery validation.
    pub name: String,
    /// Exact semantic version.
    pub version: Version,
    /// Moving release channels that point at this version.
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Supported MEP protocol versions.
    pub mep_versions: Vec<String>,
    /// Advertised extension operations.
    pub capabilities: Vec<Capability>,
    /// Platform artifacts for this exact release.
    pub artifacts: Vec<ArtifactRecord>,
}

/// A mutually exclusive exact-version or moving-channel request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
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
