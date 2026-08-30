//! TUF-authenticated tool release discovery and target download.

use crate::{
    Channel, DistributionError, Platform, Result, Selection, Sha256Digest, ToolArtifactRecord,
    ToolId, ToolReleaseRecord, ToolReleaseStatus, resolve_tool,
};
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tough::{FilesystemTransport, IntoVec, Limits, Prefix, RepositoryLoader, TargetName};
use url::Url;

const MAX_ROOT_UPDATES_PER_REFRESH: u64 = 32;

/// A tool artifact selected from TUF-authenticated release metadata.
#[derive(Debug, Clone)]
pub struct ResolvedTrustedToolArtifact {
    release: ToolReleaseRecord,
    artifact: ToolArtifactRecord,
    selection: Selection,
    target: TargetName,
    digest: Sha256Digest,
    length: u64,
    targets_version: u64,
}

impl ResolvedTrustedToolArtifact {
    /// Return the selected exact release.
    pub fn release(&self) -> &ToolReleaseRecord {
        &self.release
    }

    /// Return the selected platform artifact contract.
    pub fn artifact(&self) -> &ToolArtifactRecord {
        &self.artifact
    }

    /// Return the original channel or exact-version request.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the artifact digest authenticated by TUF targets metadata.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Return the artifact length authenticated by TUF targets metadata.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Return the exact targets metadata version used for resolution.
    pub fn targets_version(&self) -> u64 {
        self.targets_version
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(
        release: ToolReleaseRecord,
        selection: Selection,
        digest: Sha256Digest,
        length: u64,
    ) -> Self {
        let artifact = release.artifacts()[0].clone();
        let target = TargetName::new(artifact.target_path().as_str()).unwrap();
        Self {
            release,
            artifact,
            selection,
            target,
            digest,
            length,
            targets_version: 1,
        }
    }
}

/// A fully downloaded target whose length and digest were verified by TUF.
#[derive(Debug)]
pub struct DownloadedToolArtifact {
    path: PathBuf,
}

impl DownloadedToolArtifact {
    /// Return the verified downloaded target path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn into_path(self) -> PathBuf {
        self.path
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(path: PathBuf) -> Self {
        Self { path }
    }
}

/// A loaded TUF repository restricted to Morphir tool metadata.
#[derive(Debug)]
pub struct TrustedToolRepository {
    repository: tough::Repository,
}

impl TrustedToolRepository {
    /// Load and authenticate a filesystem-backed repository from an out-of-band root.
    ///
    /// The datastore persists rollback protection metadata. Expiration enforcement
    /// remains in TUF's safe default mode, and one refresh accepts at most 32
    /// sequential root updates as required by the Morphir tool repository profile.
    pub async fn load_filesystem(
        trusted_root: &[u8],
        metadata_directory: &Path,
        targets_directory: &Path,
        datastore: &Path,
    ) -> Result<Self> {
        fs::create_dir_all(datastore).map_err(|source| DistributionError::Io {
            path: datastore.to_path_buf(),
            source,
        })?;
        let metadata_url = directory_url(metadata_directory, "metadata directory")?;
        let targets_url = directory_url(targets_directory, "targets directory")?;
        let repository = RepositoryLoader::new(&trusted_root, metadata_url, targets_url)
            .transport(FilesystemTransport)
            .limits(Limits {
                max_root_updates: MAX_ROOT_UPDATES_PER_REFRESH,
                ..Limits::default()
            })
            .datastore(datastore)
            .load()
            .await
            .map_err(repository_error)?;
        Ok(Self { repository })
    }

    /// Resolve an authenticated tool descriptor and its exact target metadata.
    pub async fn resolve(
        &self,
        tool_id: &ToolId,
        selection: &Selection,
        platform: &Platform,
        morphir_cli: &Version,
    ) -> Result<ResolvedTrustedToolArtifact> {
        let authenticated = self.release_descriptors(tool_id).await?;
        let releases = authenticated
            .iter()
            .map(|release| release.record.clone())
            .collect::<Vec<_>>();
        let resolved = resolve_tool(&releases, selection, platform, morphir_cli)?;
        let descriptor = authenticated
            .iter()
            .find(|candidate| candidate.record.version() == resolved.release().version())
            .expect("resolved release came from authenticated descriptors");
        let target = TargetName::new(resolved.artifact().target_path().as_str())
            .map_err(repository_error)?;
        let target_metadata = self
            .repository
            .targets()
            .signed
            .find_target(&target, false)
            .map_err(|_| DistributionError::MissingToolTarget {
                target: target.raw().to_owned(),
            })?;
        validate_artifact_custom(
            target.raw(),
            target_metadata.custom.get("morphir"),
            descriptor.record.tool_id(),
            descriptor.record.version(),
            resolved.artifact().platform(),
        )?;
        let digest_bytes: [u8; 32] = target_metadata
            .hashes
            .sha256
            .as_ref()
            .try_into()
            .map_err(|_| invalid_metadata(target.raw(), "SHA-256 digest was not 32 bytes"))?;
        Ok(ResolvedTrustedToolArtifact {
            release: resolved.release().clone(),
            artifact: resolved.artifact().clone(),
            selection: resolved.selection().clone(),
            target,
            digest: Sha256Digest::from_bytes(digest_bytes),
            length: target_metadata.length,
            targets_version: self.repository.targets().signed.version.get(),
        })
    }

    /// Download and verify the selected target into an existing staging directory.
    pub async fn download(
        &self,
        resolved: &ResolvedTrustedToolArtifact,
        staging_directory: &Path,
    ) -> Result<DownloadedToolArtifact> {
        fs::create_dir_all(staging_directory).map_err(|source| DistributionError::Io {
            path: staging_directory.to_path_buf(),
            source,
        })?;
        self.repository
            .save_target(&resolved.target, staging_directory, Prefix::None)
            .await
            .map_err(repository_error)?;
        Ok(DownloadedToolArtifact {
            path: staging_directory.join(resolved.target.resolved()),
        })
    }

    async fn release_descriptors(&self, tool_id: &ToolId) -> Result<Vec<AuthenticatedRelease>> {
        let mut targets = self
            .repository
            .all_targets()
            .map(|(name, target)| {
                release_custom(name.raw(), target.custom.get("morphir"))
                    .map(|custom| custom.map(|custom| (name.clone(), custom)))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter(|(_, custom)| &custom.tool_id == tool_id)
            .collect::<Vec<_>>();
        targets.sort_by(|(left, _), (right, _)| left.raw().cmp(right.raw()));

        let mut releases = Vec::with_capacity(targets.len());
        for (target, custom) in targets {
            let bytes = self
                .repository
                .read_target(&target)
                .await
                .map_err(repository_error)?
                .ok_or_else(|| DistributionError::MissingToolTarget {
                    target: target.raw().to_owned(),
                })?
                .into_vec()
                .await
                .map_err(repository_error)?;
            let record: ToolReleaseRecord = serde_json::from_slice(&bytes).map_err(|source| {
                invalid_metadata(
                    target.raw(),
                    format!("descriptor decoding failed: {source}"),
                )
            })?;
            validate_release_custom(target.raw(), &custom, &record)?;
            releases.push(AuthenticatedRelease { record });
        }
        Ok(releases)
    }
}

#[derive(Debug)]
struct AuthenticatedRelease {
    record: ToolReleaseRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolReleaseCustom {
    schema_version: u32,
    kind: String,
    tool_id: ToolId,
    version: Version,
    channels: Vec<Channel>,
    status: ToolReleaseStatus,
    compatibility: ToolCompatibilityCustom,
    platforms: Vec<Platform>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolCompatibilityCustom {
    morphir_cli: VersionReq,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolArtifactCustom {
    schema_version: u32,
    kind: String,
    tool_id: ToolId,
    version: Version,
    platform: Platform,
}

fn release_custom(
    target: &str,
    custom: Option<&serde_json::Value>,
) -> Result<Option<ToolReleaseCustom>> {
    let Some(custom) = custom else {
        return Ok(None);
    };
    if custom.get("kind").and_then(serde_json::Value::as_str) != Some("tool-release") {
        return Ok(None);
    }
    serde_json::from_value(custom.clone())
        .map(Some)
        .map_err(|source| {
            invalid_metadata(target, format!("custom metadata decoding failed: {source}"))
        })
}

fn validate_release_custom(
    target: &str,
    custom: &ToolReleaseCustom,
    record: &ToolReleaseRecord,
) -> Result<()> {
    let expected_target = format!("releases/{}/{}.json", record.tool_id(), record.version());
    let record_platforms = record
        .artifacts()
        .iter()
        .map(|artifact| artifact.platform().to_string())
        .collect::<BTreeSet<_>>();
    let custom_platforms = custom
        .platforms
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let valid = custom.schema_version == 1
        && custom.kind == "tool-release"
        && custom.tool_id == *record.tool_id()
        && custom.version == *record.version()
        && custom.channels == record.channels()
        && custom.status == record.status()
        && custom.compatibility.morphir_cli == *record.morphir_cli_requirement()
        && custom_platforms == record_platforms
        && target == expected_target;
    if valid {
        Ok(())
    } else {
        Err(invalid_metadata(
            target,
            "descriptor disagrees with authenticated target metadata",
        ))
    }
}

fn validate_artifact_custom(
    target: &str,
    custom: Option<&serde_json::Value>,
    tool_id: &ToolId,
    version: &Version,
    platform: &Platform,
) -> Result<()> {
    let custom: ToolArtifactCustom = serde_json::from_value(
        custom
            .cloned()
            .ok_or_else(|| invalid_metadata(target, "missing custom.morphir metadata"))?,
    )
    .map_err(|source| {
        invalid_metadata(target, format!("custom metadata decoding failed: {source}"))
    })?;
    if custom.schema_version == 1
        && custom.kind == "tool-artifact"
        && custom.tool_id == *tool_id
        && custom.version == *version
        && custom.platform == *platform
    {
        Ok(())
    } else {
        Err(invalid_metadata(
            target,
            "artifact disagrees with authenticated target metadata",
        ))
    }
}

fn directory_url(path: &Path, kind: &'static str) -> Result<Url> {
    Url::from_directory_path(path).map_err(|()| DistributionError::InvalidValue {
        kind,
        value: path.to_string_lossy().into_owned(),
        reason: "expected an absolute filesystem directory",
    })
}

fn repository_error(source: tough::error::Error) -> DistributionError {
    DistributionError::ToolRepository {
        source: Box::new(source),
    }
}

fn invalid_metadata(target: impl Into<String>, reason: impl Into<String>) -> DistributionError {
    DistributionError::InvalidToolMetadata {
        target: target.into(),
        reason: reason.into(),
    }
}
