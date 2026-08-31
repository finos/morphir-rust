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
const MAX_TOOL_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TOOL_RELEASE_DESCRIPTOR_BYTES: u64 = 1024 * 1024;
const MAX_TOOL_RELEASE_DESCRIPTOR_COUNT: usize = 1024;
const MAX_TOOL_RELEASE_DESCRIPTORS_BYTES: u64 = 16 * 1024 * 1024;

/// A tool artifact selected from TUF-authenticated release metadata.
#[derive(Debug, Clone)]
pub struct ResolvedTrustedToolArtifact {
    release: ToolReleaseRecord,
    artifact: ToolArtifactRecord,
    selection: Selection,
    target: TargetName,
    digest: Sha256Digest,
    length: u64,
    snapshot_version: u64,
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

    /// Return the exact snapshot metadata version used for resolution.
    pub fn snapshot_version(&self) -> u64 {
        self.snapshot_version
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
            snapshot_version: 1,
        }
    }
}

/// A fully downloaded target whose length and digest were verified by TUF.
#[derive(Debug)]
pub struct DownloadedToolArtifact {
    path: PathBuf,
    _cleanup: Option<tempfile::TempDir>,
}

impl DownloadedToolArtifact {
    /// Return the verified downloaded target path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(path: PathBuf) -> Self {
        Self {
            path,
            _cleanup: None,
        }
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
    #[tracing::instrument(
        name = "morphir.tool.repository.load",
        skip(trusted_root),
        fields(
            metadata_directory = %metadata_directory.display(),
            targets_directory = %targets_directory.display(),
            datastore = %datastore.display()
        ),
        err
    )]
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
        tracing::info!(
            targets_version = repository.targets().signed.version.get(),
            snapshot_version = repository.snapshot().signed.version.get(),
            "authenticated tool repository loaded"
        );
        Ok(Self { repository })
    }

    /// Resolve an authenticated tool descriptor and its exact target metadata.
    #[tracing::instrument(
        name = "morphir.tool.resolve",
        skip(self),
        fields(
            tool_id = %tool_id,
            selection = %selection,
            platform = %platform,
            morphir_cli = %morphir_cli
        ),
        err
    )]
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
        validate_tool_artifact_length(target.raw(), target_metadata.length)?;
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
        let resolved = ResolvedTrustedToolArtifact {
            release: resolved.release().clone(),
            artifact: resolved.artifact().clone(),
            selection: resolved.selection().clone(),
            target,
            digest: Sha256Digest::from_bytes(digest_bytes),
            length: target_metadata.length,
            snapshot_version: self.repository.snapshot().signed.version.get(),
        };
        tracing::info!(
            version = %resolved.release.version(),
            target = %resolved.target.raw(),
            digest = %resolved.digest,
            "authenticated tool artifact resolved"
        );
        Ok(resolved)
    }

    /// Download and verify the selected target into an existing staging directory.
    #[tracing::instrument(
        name = "morphir.tool.download",
        skip(self),
        fields(
            tool_id = %resolved.release.tool_id(),
            version = %resolved.release.version(),
            target = %resolved.target.raw(),
            digest = %resolved.digest
        ),
        err
    )]
    pub async fn download(
        &self,
        resolved: &ResolvedTrustedToolArtifact,
        staging_directory: &Path,
    ) -> Result<DownloadedToolArtifact> {
        let target_metadata = self
            .repository
            .targets()
            .signed
            .find_target(&resolved.target, false)
            .map_err(|_| DistributionError::MissingToolTarget {
                target: resolved.target.raw().to_owned(),
            })?;
        validate_tool_artifact_length(resolved.target.raw(), target_metadata.length)?;
        let digest_bytes: [u8; 32] =
            target_metadata
                .hashes
                .sha256
                .as_ref()
                .try_into()
                .map_err(|_| {
                    invalid_metadata(resolved.target.raw(), "SHA-256 digest was not 32 bytes")
                })?;
        validate_resolved_target_metadata(
            resolved.target.raw(),
            resolved.length,
            &resolved.digest,
            target_metadata.length,
            &Sha256Digest::from_bytes(digest_bytes),
        )?;
        let (download_directory, path) =
            prepare_download_destination(staging_directory, Path::new(resolved.target.resolved()))?;
        self.repository
            .save_target(&resolved.target, download_directory.path(), Prefix::None)
            .await
            .map_err(repository_error)?;
        tracing::info!(path = %path.display(), "verified tool target downloaded");
        Ok(DownloadedToolArtifact {
            path,
            _cleanup: Some(download_directory),
        })
    }

    async fn release_descriptors(&self, tool_id: &ToolId) -> Result<Vec<AuthenticatedRelease>> {
        let mut targets = self
            .repository
            .all_targets()
            .map(|(name, target)| {
                release_custom(name.raw(), target.custom.get("morphir"), tool_id)
                    .map(|custom| custom.map(|custom| (name.clone(), custom, target.length)))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let aggregate_length =
            targets
                .iter()
                .try_fold(0_u64, |total, (target, _, length)| -> Result<u64> {
                    validate_release_descriptor_length(target.raw(), *length)?;
                    total.checked_add(*length).ok_or_else(|| {
                        invalid_metadata(
                            "release descriptor set",
                            "aggregate descriptor length overflow",
                        )
                    })
                })?;
        validate_release_descriptor_set(targets.len(), aggregate_length)?;
        targets.sort_by(|(left, _, _), (right, _, _)| left.raw().cmp(right.raw()));

        let mut releases = Vec::with_capacity(targets.len());
        for (target, custom, _) in targets {
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
            releases.push(authenticate_release(target.raw(), custom, record)?);
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

fn authenticate_release(
    target: &str,
    custom: ToolReleaseCustom,
    record: ToolReleaseRecord,
) -> Result<AuthenticatedRelease> {
    validate_release_custom(target, &custom, &record)?;
    Ok(AuthenticatedRelease {
        record: record.with_repository_state(custom.channels, custom.status),
    })
}

fn release_custom(
    target: &str,
    custom: Option<&serde_json::Value>,
    requested_tool_id: &ToolId,
) -> Result<Option<ToolReleaseCustom>> {
    let Some(custom) = custom else {
        return Ok(None);
    };
    if custom.get("kind").and_then(serde_json::Value::as_str) != Some("tool-release") {
        return Ok(None);
    }
    if custom
        .get("toolId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|tool_id| tool_id != requested_tool_id.as_str())
    {
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
        && (custom.status == ToolReleaseStatus::Active || custom.channels.is_empty())
        && (custom.status == ToolReleaseStatus::Revoked || !record.artifacts().is_empty())
        && custom.compatibility.morphir_cli == *record.morphir_cli_requirement()
        && custom_platforms == record_platforms
        && target == expected_target;
    if valid {
        Ok(())
    } else {
        Err(invalid_metadata(
            target,
            "descriptor identity or authenticated release state is invalid",
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

fn prepare_download_destination(
    staging_directory: &Path,
    target: &Path,
) -> Result<(tempfile::TempDir, PathBuf)> {
    fs::create_dir_all(staging_directory).map_err(|source| DistributionError::Io {
        path: staging_directory.to_path_buf(),
        source,
    })?;
    let canonical_staging =
        fs::canonicalize(staging_directory).map_err(|source| DistributionError::Io {
            path: staging_directory.to_path_buf(),
            source,
        })?;
    let download_directory = tempfile::Builder::new()
        .prefix(".morphir-download-")
        .tempdir_in(&canonical_staging)
        .map_err(|source| DistributionError::Io {
            path: canonical_staging,
            source,
        })?;
    let path = download_directory.path().join(target);
    Ok((download_directory, path))
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

fn validate_release_descriptor_length(target: &str, length: u64) -> Result<()> {
    if length <= MAX_TOOL_RELEASE_DESCRIPTOR_BYTES {
        Ok(())
    } else {
        Err(invalid_metadata(
            target,
            format!(
                "descriptor length {length} exceeds the {MAX_TOOL_RELEASE_DESCRIPTOR_BYTES}-byte limit"
            ),
        ))
    }
}

fn validate_tool_artifact_length(target: &str, length: u64) -> Result<()> {
    if length <= MAX_TOOL_ARTIFACT_BYTES {
        Ok(())
    } else {
        Err(invalid_metadata(
            target,
            format!("artifact length {length} exceeds the {MAX_TOOL_ARTIFACT_BYTES}-byte limit"),
        ))
    }
}

fn validate_resolved_target_metadata(
    target: &str,
    resolved_length: u64,
    resolved_digest: &Sha256Digest,
    current_length: u64,
    current_digest: &Sha256Digest,
) -> Result<()> {
    if current_length == resolved_length && current_digest == resolved_digest {
        return Ok(());
    }

    Err(invalid_metadata(
        target,
        format!(
            "current repository target does not match resolved artifact: expected {resolved_length} bytes with SHA-256 {resolved_digest}, found {current_length} bytes with SHA-256 {current_digest}"
        ),
    ))
}

fn validate_release_descriptor_set(count: usize, aggregate_length: u64) -> Result<()> {
    if count > MAX_TOOL_RELEASE_DESCRIPTOR_COUNT {
        return Err(invalid_metadata(
            "release descriptor set",
            format!(
                "descriptor count {count} exceeds the {MAX_TOOL_RELEASE_DESCRIPTOR_COUNT}-descriptor limit"
            ),
        ));
    }
    if aggregate_length > MAX_TOOL_RELEASE_DESCRIPTORS_BYTES {
        return Err(invalid_metadata(
            "release descriptor set",
            format!(
                "aggregate descriptor length {aggregate_length} exceeds the {MAX_TOOL_RELEASE_DESCRIPTORS_BYTES}-byte limit"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod descriptor_size_tests {
    use super::{
        MAX_TOOL_ARTIFACT_BYTES, MAX_TOOL_RELEASE_DESCRIPTOR_BYTES,
        MAX_TOOL_RELEASE_DESCRIPTOR_COUNT, MAX_TOOL_RELEASE_DESCRIPTORS_BYTES,
        validate_release_descriptor_length, validate_release_descriptor_set,
        validate_tool_artifact_length,
    };
    use crate::DistributionError;

    #[test]
    fn release_descriptors_are_bounded_before_their_bytes_are_read() {
        validate_release_descriptor_length(
            "releases/desktop/1.0.0.json",
            MAX_TOOL_RELEASE_DESCRIPTOR_BYTES,
        )
        .unwrap();

        let error = validate_release_descriptor_length(
            "releases/desktop/1.0.0.json",
            MAX_TOOL_RELEASE_DESCRIPTOR_BYTES + 1,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DistributionError::InvalidToolMetadata { .. }
        ));
        assert!(error.to_string().contains("exceeds the 1048576-byte limit"));
    }

    #[test]
    fn release_descriptor_sets_bound_count_and_aggregate_bytes_before_reads() {
        validate_release_descriptor_set(
            MAX_TOOL_RELEASE_DESCRIPTOR_COUNT,
            MAX_TOOL_RELEASE_DESCRIPTORS_BYTES,
        )
        .unwrap();

        for (count, bytes, expected) in [
            (
                MAX_TOOL_RELEASE_DESCRIPTOR_COUNT + 1,
                MAX_TOOL_RELEASE_DESCRIPTORS_BYTES,
                "descriptor count",
            ),
            (
                MAX_TOOL_RELEASE_DESCRIPTOR_COUNT,
                MAX_TOOL_RELEASE_DESCRIPTORS_BYTES + 1,
                "aggregate descriptor length",
            ),
        ] {
            let error = validate_release_descriptor_set(count, bytes).unwrap_err();
            assert!(matches!(
                error,
                DistributionError::InvalidToolMetadata { .. }
            ));
            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn authenticated_artifact_lengths_are_bounded_before_download() {
        validate_tool_artifact_length(
            "artifacts/desktop/1.0.0/desktop.zip",
            MAX_TOOL_ARTIFACT_BYTES,
        )
        .unwrap();

        let error = validate_tool_artifact_length(
            "artifacts/desktop/1.0.0/desktop.zip",
            MAX_TOOL_ARTIFACT_BYTES + 1,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DistributionError::InvalidToolMetadata { .. }
        ));
        assert!(error.to_string().contains("artifact length"));
    }
}

#[cfg(test)]
mod resolved_target_tests {
    use super::validate_resolved_target_metadata;
    use crate::{DistributionError, Sha256Digest};

    #[test]
    fn download_rejects_target_metadata_from_a_different_repository() {
        let resolved_digest = Sha256Digest::of_bytes(b"repository-a");
        let current_digest = Sha256Digest::of_bytes(b"repository-b");

        for (length, digest) in [(12, resolved_digest.clone()), (12_u64 - 1, current_digest)] {
            let error = validate_resolved_target_metadata(
                "artifacts/desktop/1.0.0/desktop.zip",
                12_u64 - 1,
                &resolved_digest,
                length,
                &digest,
            )
            .unwrap_err();

            assert!(matches!(
                error,
                DistributionError::InvalidToolMetadata { .. }
            ));
            assert!(
                error
                    .to_string()
                    .contains("does not match resolved artifact")
            );
        }
    }
}

#[cfg(test)]
mod descriptor_filter_tests {
    use super::release_custom;
    use crate::ToolId;

    #[test]
    fn unrelated_tool_metadata_is_filtered_before_strict_decoding() {
        let requested = ToolId::parse("desktop").unwrap();
        let unrelated = serde_json::json!({
            "kind": "tool-release",
            "toolId": "companion",
            "schemaVersion": "from-a-newer-profile"
        });

        assert!(
            release_custom(
                "releases/companion/2.0.0.json",
                Some(&unrelated),
                &requested
            )
            .unwrap()
            .is_none()
        );

        let matching = serde_json::json!({
            "kind": "tool-release",
            "toolId": "desktop",
            "schemaVersion": "invalid"
        });
        assert!(
            release_custom("releases/desktop/2.0.0.json", Some(&matching), &requested).is_err()
        );
    }
}

#[cfg(test)]
mod release_state_tests {
    use super::{ToolReleaseCustom, authenticate_release};
    use crate::{ToolReleaseRecord, ToolReleaseStatus};

    fn an_active_descriptor() -> ToolReleaseRecord {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "morphir-tool-release",
            "tool": { "id": "desktop", "name": "Morphir Desktop" },
            "version": "1.0.0",
            "channels": ["stable"],
            "status": "active",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "artifacts": [{
                "targetPath": "artifacts/desktop/1.0.0/desktop",
                "platform": { "os": "linux", "arch": "x86_64" },
                "archive": { "format": "raw", "entryPoint": "desktop" },
                "launch": { "kind": "executable", "path": "desktop", "args": [] }
            }]
        }))
        .unwrap()
    }

    fn yanked_targets(channels: &[&str]) -> ToolReleaseCustom {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "tool-release",
            "toolId": "desktop",
            "version": "1.0.0",
            "channels": channels,
            "status": "yanked",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "platforms": [{ "os": "linux", "arch": "x86_64" }]
        }))
        .unwrap()
    }

    fn a_revoked_descriptor_without_artifacts() -> ToolReleaseRecord {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "morphir-tool-release",
            "tool": { "id": "desktop", "name": "Morphir Desktop" },
            "version": "1.0.0",
            "channels": [],
            "status": "revoked",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "artifacts": []
        }))
        .unwrap()
    }

    #[test]
    fn authenticated_targets_override_the_descriptors_publication_state() {
        let descriptor = an_active_descriptor();
        let targets = yanked_targets(&[]);

        let authenticated =
            authenticate_release("releases/desktop/1.0.0.json", targets, descriptor).unwrap();

        assert_eq!(authenticated.record.status(), ToolReleaseStatus::Yanked);
        assert!(authenticated.record.channels().is_empty());
    }

    #[test]
    fn authenticated_targets_reject_channels_on_inactive_releases() {
        let error = authenticate_release(
            "releases/desktop/1.0.0.json",
            yanked_targets(&["stable"]),
            an_active_descriptor(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("authenticated release state"));
    }

    #[test]
    fn authenticated_targets_cannot_activate_an_artifactless_descriptor() {
        let targets: ToolReleaseCustom = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "tool-release",
            "toolId": "desktop",
            "version": "1.0.0",
            "channels": [],
            "status": "yanked",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "platforms": []
        }))
        .unwrap();

        let error = authenticate_release(
            "releases/desktop/1.0.0.json",
            targets,
            a_revoked_descriptor_without_artifacts(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("authenticated release state"));
    }
}

#[cfg(test)]
mod download_path_tests {
    use super::{DownloadedToolArtifact, prepare_download_destination};
    use std::fs;
    use std::path::Path;

    #[test]
    fn nested_downloads_are_isolated_from_a_reused_symlinked_parent() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("staging");
        let outside = root.path().join("outside");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&outside).unwrap();
        make_dir_symlink(&outside, &staging.join("artifacts"));

        let (download_directory, destination) =
            prepare_download_destination(&staging, Path::new("artifacts/desktop/desktop.zip"))
                .unwrap();

        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"verified target").unwrap();

        assert!(
            download_directory
                .path()
                .starts_with(fs::canonicalize(&staging).unwrap())
        );
        assert_ne!(destination, staging.join("artifacts/desktop/desktop.zip"));
        assert!(!outside.join("desktop/desktop.zip").exists());

        let isolated_root = download_directory.path().to_path_buf();
        let downloaded = DownloadedToolArtifact {
            path: destination,
            _cleanup: Some(download_directory),
        };
        drop(downloaded);
        assert!(!isolated_root.exists());
    }

    #[cfg(unix)]
    fn make_dir_symlink(source: &Path, destination: &Path) {
        std::os::unix::fs::symlink(source, destination).unwrap();
    }

    #[cfg(windows)]
    fn make_dir_symlink(source: &Path, destination: &Path) {
        std::os::windows::fs::symlink_dir(source, destination).unwrap();
    }
}
