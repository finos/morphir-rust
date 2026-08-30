//! Authenticated tool release descriptors and launch metadata.

use crate::{Channel, Platform, RelativeArtifactPath, ToolId};
use semver::{Version, VersionReq};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

/// Publication state of one exact tool release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolReleaseStatus {
    /// Eligible for channel and exact-version selection.
    Active,
    /// Removed from channels but still eligible for an explicit exact request.
    Yanked,
    /// Rejected for new installation and activation.
    Revoked,
}

/// Packaging format of a tool artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveFormat {
    /// ZIP archive.
    Zip,
    /// Gzip-compressed tar archive.
    TarGzip,
    /// Linux AppImage executable.
    Appimage,
    /// An unpackaged executable file.
    Raw,
}

/// Archive expansion contract for one platform artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolArchive {
    format: ArchiveFormat,
    entry_point: RelativeArtifactPath,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolArchiveWire {
    format: ArchiveFormat,
    entry_point: RelativeArtifactPath,
}

impl<'de> Deserialize<'de> for ToolArchive {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ToolArchiveWire::deserialize(deserializer)?;
        wire.entry_point
            .validate_declared()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            format: wire.format,
            entry_point: wire.entry_point,
        })
    }
}

impl ToolArchive {
    /// Return the declared packaging format.
    pub fn format(&self) -> ArchiveFormat {
        self.format
    }

    /// Return the expected entry point inside the expanded artifact.
    pub fn entry_point(&self) -> &RelativeArtifactPath {
        &self.entry_point
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ToolLaunchKind {
    Executable,
}

/// Direct process launch contract for an installed tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolLaunch {
    kind: ToolLaunchKind,
    path: RelativeArtifactPath,
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolLaunchWire {
    kind: ToolLaunchKind,
    path: RelativeArtifactPath,
    args: Vec<String>,
}

impl<'de> Deserialize<'de> for ToolLaunch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ToolLaunchWire::deserialize(deserializer)?;
        wire.path
            .validate_declared()
            .map_err(serde::de::Error::custom)?;
        if wire.args.iter().any(|argument| argument.contains('\0')) {
            return Err(serde::de::Error::custom(
                "tool launch arguments cannot contain NUL",
            ));
        }
        Ok(Self {
            kind: wire.kind,
            path: wire.path,
            args: wire.args,
        })
    }
}

impl ToolLaunch {
    /// Return the executable path relative to the installed release root.
    pub fn path(&self) -> &RelativeArtifactPath {
        &self.path
    }

    /// Return the fixed arguments prepended to user-supplied launch arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// One platform package declared by an authenticated tool release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolArtifactRecord {
    target_path: RelativeArtifactPath,
    platform: Platform,
    archive: ToolArchive,
    launch: ToolLaunch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolArtifactRecordWire {
    target_path: RelativeArtifactPath,
    platform: Platform,
    archive: ToolArchive,
    launch: ToolLaunch,
}

impl<'de> Deserialize<'de> for ToolArtifactRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ToolArtifactRecordWire::deserialize(deserializer)?;
        wire.target_path
            .validate_declared()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            target_path: wire.target_path,
            platform: wire.platform,
            archive: wire.archive,
            launch: wire.launch,
        })
    }
}

impl ToolArtifactRecord {
    /// Return the target path authenticated by repository metadata.
    pub fn target_path(&self) -> &RelativeArtifactPath {
        &self.target_path
    }

    /// Return the operating-system and architecture pair.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Return the archive expansion contract.
    pub fn archive(&self) -> &ToolArchive {
        &self.archive
    }

    /// Return the direct process launch contract.
    pub fn launch(&self) -> &ToolLaunch {
        &self.launch
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolIdentityWire {
    id: ToolId,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolCompatibilityWire {
    morphir_cli: VersionReq,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolReleaseWire {
    schema_version: u32,
    kind: String,
    tool: ToolIdentityWire,
    version: Version,
    channels: Vec<Channel>,
    status: ToolReleaseStatus,
    compatibility: ToolCompatibilityWire,
    artifacts: Vec<ToolArtifactRecord>,
}

/// One exact authenticated tool release descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReleaseRecord {
    schema_version: u32,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    channels: Vec<Channel>,
    status: ToolReleaseStatus,
    morphir_cli: VersionReq,
    artifacts: Vec<ToolArtifactRecord>,
}

impl ToolReleaseRecord {
    /// Return the release descriptor schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Return the tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the display name.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Return the exact semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the moving channels containing this release.
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Return the publication state.
    pub fn status(&self) -> ToolReleaseStatus {
        self.status
    }

    /// Return the supported Morphir CLI version requirement.
    pub fn morphir_cli_requirement(&self) -> &VersionReq {
        &self.morphir_cli
    }

    /// Return the platform artifacts.
    pub fn artifacts(&self) -> &[ToolArtifactRecord] {
        &self.artifacts
    }
}

impl<'de> Deserialize<'de> for ToolReleaseRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ToolReleaseWire::deserialize(deserializer)?;
        if wire.schema_version != 1 {
            return Err(serde::de::Error::custom(
                "tool release schemaVersion must be 1",
            ));
        }
        if wire.kind != "morphir-tool-release" {
            return Err(serde::de::Error::custom(
                "tool release kind must be morphir-tool-release",
            ));
        }
        if wire.tool.name.trim().is_empty() {
            return Err(serde::de::Error::custom("tool name cannot be blank"));
        }
        if wire.channels.iter().collect::<BTreeSet<_>>().len() != wire.channels.len() {
            return Err(serde::de::Error::custom(
                "tool release channels cannot contain duplicates",
            ));
        }
        if wire.status != ToolReleaseStatus::Active && !wire.channels.is_empty() {
            return Err(serde::de::Error::custom(
                "yanked and revoked releases cannot belong to channels",
            ));
        }
        if wire.status != ToolReleaseStatus::Revoked && wire.artifacts.is_empty() {
            return Err(serde::de::Error::custom(
                "non-revoked tool releases require at least one artifact",
            ));
        }
        if wire
            .artifacts
            .iter()
            .map(|artifact| &artifact.target_path)
            .collect::<BTreeSet<_>>()
            .len()
            != wire.artifacts.len()
        {
            return Err(serde::de::Error::custom(
                "tool release artifacts cannot contain duplicates",
            ));
        }
        if wire
            .artifacts
            .iter()
            .any(|artifact| artifact.archive.entry_point != artifact.launch.path)
        {
            return Err(serde::de::Error::custom(
                "tool archive entryPoint and launch path must match",
            ));
        }
        if wire.artifacts.iter().any(|artifact| {
            artifact.archive.format == ArchiveFormat::Appimage && artifact.platform.os() != "linux"
        }) {
            return Err(serde::de::Error::custom(
                "AppImage artifacts require Linux platforms",
            ));
        }
        if wire.artifacts.iter().any(|artifact| {
            matches!(
                artifact.archive.format,
                ArchiveFormat::Raw | ArchiveFormat::Appimage
            ) && artifact
                .target_path
                .as_path()
                .file_name()
                .and_then(|name| name.to_str())
                != Some(artifact.archive.entry_point.as_str())
        }) {
            return Err(serde::de::Error::custom(
                "raw and AppImage entryPoint must equal the targetPath filename",
            ));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            tool_id: wire.tool.id,
            tool_name: wire.tool.name,
            version: wire.version,
            channels: wire.channels,
            status: wire.status,
            morphir_cli: wire.compatibility.morphir_cli,
            artifacts: wire.artifacts,
        })
    }
}
