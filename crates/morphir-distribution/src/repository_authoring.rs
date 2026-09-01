//! Local extension repository initialization and verified publication.

use crate::domain::portable_token;
use crate::local::ensure_contained;
use crate::state_io::{StateGuard, atomic_write_bytes, create_dir_all_durable};
use crate::{
    ArtifactFilename, ArtifactRuntime, DistributionError, ExtensionHistory, ExtensionId,
    ReleaseRecord, Result, Sha256Digest,
};
use semver::Version;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const RELEASE_BUNDLE_SCHEMA_VERSION: u32 = 1;
const REPOSITORY_DIRECTORIES: [&str; 2] = ["artifacts", "extensions"];

/// Whether publication added a release or found the exact release already present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStatus {
    /// The repository gained a new release record.
    Published,
    /// The repository already contained the exact release and artifact.
    AlreadyPresent,
}

/// One verified release published into a local extension repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPublication {
    status: PublicationStatus,
    release: ReleaseRecord,
    artifact_path: PathBuf,
}

impl RepositoryPublication {
    /// Return whether this call added a release or confirmed an identical one.
    pub fn status(&self) -> PublicationStatus {
        self.status
    }

    /// Return the exact repository release record.
    pub fn release(&self) -> &ReleaseRecord {
        &self.release
    }

    /// Return the canonical artifact path below the repository root.
    pub fn artifact_path(&self) -> &Path {
        &self.artifact_path
    }
}

/// An initialized local-directory extension repository used for authoring.
///
/// ```no_run
/// use morphir_distribution::{LocalExtensionRepository, PublicationStatus};
///
/// # fn author() -> Result<(), Box<dyn std::error::Error>> {
/// let repository = LocalExtensionRepository::init("./local-extensions")?;
/// let publication = repository.publish("./release-bundle")?;
/// assert!(matches!(
///     publication.status(),
///     PublicationStatus::Published | PublicationStatus::AlreadyPresent
/// ));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExtensionRepository {
    root: PathBuf,
}

impl LocalExtensionRepository {
    /// Create the repository directory contract, or open it when it already exists.
    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let requested = root.as_ref();
        create_dir_all_durable(requested)?;
        for directory in REPOSITORY_DIRECTORIES {
            create_dir_all_durable(&requested.join(directory))?;
        }
        let repository = Self::open(requested)?;
        tracing::info!(
            event_name = "extension.repository.init",
            path = %repository.root.display(),
            "local extension repository initialized"
        );
        Ok(repository)
    }

    /// Open an initialized repository and reject redirected authoring directories.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let requested = root.as_ref();
        let root = canonical_directory(requested, "local repository root is not a directory")?;
        for directory in REPOSITORY_DIRECTORIES {
            let path = root.join(directory);
            let canonical =
                canonical_directory(&path, "local repository authoring path is not a directory")?;
            ensure_contained(&root, &canonical)?;
        }
        Ok(Self { root })
    }

    /// Return the canonical repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verify and publish one deterministic extension release bundle atomically.
    pub fn publish(&self, bundle: impl AsRef<Path>) -> Result<RepositoryPublication> {
        let bundle = VerifiedReleaseBundle::load(bundle.as_ref())?;
        let _guard = StateGuard::acquire(&self.root.join(".publish.lock"))?;
        let history_path = self
            .root
            .join("extensions")
            .join(format!("{}.jsonl", bundle.release.extension_id()));
        let previous = read_optional(&history_path)?;
        let status = publication_status(previous.as_deref(), &bundle.release)?;
        let artifact_path = self.publish_artifact(&bundle)?;

        if status == PublicationStatus::Published {
            let next = append_release(previous.as_deref(), &bundle.release)?;
            atomic_write_bytes(&history_path, &next)?;
        }

        tracing::info!(
            event_name = "extension.repository.publish",
            extension_id = %bundle.release.extension_id(),
            version = %bundle.release.version(),
            status = match status {
                PublicationStatus::Published => "published",
                PublicationStatus::AlreadyPresent => "already-present",
            },
            "extension release bundle published"
        );
        Ok(RepositoryPublication {
            status,
            release: bundle.release,
            artifact_path,
        })
    }

    fn publish_artifact(&self, bundle: &VerifiedReleaseBundle) -> Result<PathBuf> {
        let destination = self.root.join("artifacts").join(bundle.artifact.as_str());
        if destination.exists() {
            let metadata =
                fs::symlink_metadata(&destination).map_err(|source| DistributionError::Io {
                    path: destination.clone(),
                    source,
                })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(invalid_bundle(
                    &destination,
                    "repository artifact destination is not a regular file",
                ));
            }
            let canonical =
                fs::canonicalize(&destination).map_err(|source| DistributionError::Io {
                    path: destination.clone(),
                    source,
                })?;
            ensure_contained(&self.root, &canonical)?;
            let bytes = fs::read(&canonical).map_err(|source| DistributionError::Io {
                path: canonical.clone(),
                source,
            })?;
            verify_digest(&canonical, &bytes, &bundle.digest)?;
            return Ok(canonical);
        }

        atomic_write_bytes(&destination, &bundle.artifact_bytes)?;
        let canonical = fs::canonicalize(&destination).map_err(|source| DistributionError::Io {
            path: destination,
            source,
        })?;
        ensure_contained(&self.root, &canonical)?;
        Ok(canonical)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseBundleDescriptor {
    schema_version: u32,
    short_id: String,
    extension_id: ExtensionId,
    package: String,
    version: Version,
    mep_versions: Vec<String>,
    runtime: ArtifactRuntime,
    targets: Vec<String>,
    ir_versions: Vec<String>,
    artifact: ArtifactFilename,
    sha256: Sha256Digest,
    #[serde(default)]
    git_commit: Option<String>,
}

struct VerifiedReleaseBundle {
    release: ReleaseRecord,
    artifact: ArtifactFilename,
    artifact_bytes: Vec<u8>,
    digest: Sha256Digest,
}

impl VerifiedReleaseBundle {
    fn load(requested: &Path) -> Result<Self> {
        let root = canonical_directory(requested, "release bundle is not a directory")?;
        let entries = regular_bundle_entries(&root)?;
        let descriptor_bytes = entries
            .get("release.json")
            .ok_or_else(|| invalid_bundle(&root, "release bundle has no release.json"))?;
        let descriptor: ReleaseBundleDescriptor = serde_json::from_slice(descriptor_bytes)
            .map_err(|error| invalid_bundle(root.join("release.json"), error.to_string()))?;
        descriptor.validate(&root)?;

        let artifact_name = descriptor.artifact.as_str();
        let checksum_name = format!("{artifact_name}.sha256");
        let expected_names = BTreeSet::from([
            "release.json".to_owned(),
            artifact_name.to_owned(),
            checksum_name.clone(),
        ]);
        if entries.keys().cloned().collect::<BTreeSet<_>>() != expected_names {
            return Err(invalid_bundle(
                &root,
                "release bundle files do not match release.json",
            ));
        }
        let artifact_bytes = entries
            .get(artifact_name)
            .expect("validated bundle contains its artifact")
            .clone();
        verify_digest(
            &root.join(artifact_name),
            &artifact_bytes,
            &descriptor.sha256,
        )?;
        let expected_checksum = format!("{}  {artifact_name}\n", descriptor.sha256);
        if entries
            .get(&checksum_name)
            .expect("validated bundle contains its checksum")
            != expected_checksum.as_bytes()
        {
            return Err(invalid_bundle(
                root.join(checksum_name),
                "release bundle checksum file does not match its artifact",
            ));
        }

        let release = descriptor.release_record(&root)?;
        Ok(Self {
            release,
            artifact: descriptor.artifact,
            artifact_bytes,
            digest: descriptor.sha256,
        })
    }
}

impl ReleaseBundleDescriptor {
    fn validate(&self, root: &Path) -> Result<()> {
        if self.schema_version != RELEASE_BUNDLE_SCHEMA_VERSION {
            return Err(invalid_bundle(
                root.join("release.json"),
                format!(
                    "unsupported release bundle schema version {}",
                    self.schema_version
                ),
            ));
        }
        if !portable_token(&self.short_id) || !portable_token(&self.package) {
            return Err(invalid_bundle(
                root.join("release.json"),
                "release bundle shortId and package must be portable tokens",
            ));
        }
        if self.runtime != ArtifactRuntime::Wasm {
            return Err(invalid_bundle(
                root.join("release.json"),
                "local repository publication currently requires a WASM bundle",
            ));
        }
        for (kind, values) in [
            ("MEP versions", &self.mep_versions),
            ("backend targets", &self.targets),
            ("Morphir IR versions", &self.ir_versions),
        ] {
            if values.is_empty()
                || values
                    .iter()
                    .any(|value| value.trim() != value || value.is_empty())
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return Err(invalid_bundle(
                    root.join("release.json"),
                    format!("release bundle {kind} must be non-empty and unique"),
                ));
            }
        }
        if self
            .git_commit
            .as_ref()
            .is_some_and(|commit| commit.trim() != commit || commit.is_empty())
        {
            return Err(invalid_bundle(
                root.join("release.json"),
                "release bundle gitCommit must be non-empty when present",
            ));
        }
        Ok(())
    }

    fn release_record(&self, root: &Path) -> Result<ReleaseRecord> {
        let artifact_path = format!("artifacts/{}", self.artifact.as_str());
        let channel = if self.version.pre.is_empty() {
            "stable"
        } else {
            "preview"
        };
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "id": self.extension_id,
            "name": display_name(&self.extension_id),
            "version": self.version,
            "channels": [channel],
            "mepVersions": self.mep_versions,
            "capabilities": ["backend"],
            "backend": {
                "targets": self.targets,
                "irVersions": self.ir_versions,
                "generate": true
            },
            "artifacts": [{
                "runtime": "wasm",
                "source": { "kind": "local-file", "path": artifact_path },
                "sha256": self.sha256,
                "filename": self.artifact
            }]
        }))
        .map_err(|error| invalid_bundle(root.join("release.json"), error.to_string()))
    }
}

fn canonical_directory(path: &Path, reason: &'static str) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !canonical.is_dir() {
        return Err(DistributionError::Io {
            path: canonical,
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, reason),
        });
    }
    Ok(canonical)
}

fn regular_bundle_entries(root: &Path) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut entries = std::collections::BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|source| DistributionError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DistributionError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| DistributionError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(invalid_bundle(
                &path,
                "release bundle entries must be regular files",
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid_bundle(&path, "release bundle filenames must be UTF-8"))?;
        let canonical = fs::canonicalize(&path).map_err(|source| DistributionError::Io {
            path: path.clone(),
            source,
        })?;
        ensure_contained(root, &canonical)?;
        let bytes = fs::read(&canonical).map_err(|source| DistributionError::Io {
            path: canonical,
            source,
        })?;
        entries.insert(name, bytes);
    }
    Ok(entries)
}

fn verify_digest(path: &Path, bytes: &[u8], expected: &Sha256Digest) -> Result<()> {
    let actual = Sha256Digest::of_bytes(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(DistributionError::DigestMismatch {
            path: path.to_path_buf(),
            expected: expected.clone(),
            actual,
        })
    }
}

fn publication_status(
    previous: Option<&[u8]>,
    release: &ReleaseRecord,
) -> Result<PublicationStatus> {
    let Some(previous) = previous else {
        return Ok(PublicationStatus::Published);
    };
    let history = ExtensionHistory::parse_jsonl(previous)?;
    for existing in history.releases() {
        if existing.version().cmp_precedence(release.version()).is_eq() {
            return if existing == release {
                Ok(PublicationStatus::AlreadyPresent)
            } else {
                Err(DistributionError::RepositoryReleaseConflict {
                    id: release.extension_id().clone(),
                    version: release.version().clone(),
                })
            };
        }
    }
    Ok(PublicationStatus::Published)
}

fn append_release(previous: Option<&[u8]>, release: &ReleaseRecord) -> Result<Vec<u8>> {
    let mut next = previous.unwrap_or_default().to_vec();
    if !next.is_empty() && !next.ends_with(b"\n") {
        next.push(b'\n');
    }
    next.extend(serde_json::to_vec(release).map_err(DistributionError::StateEncoding)?);
    next.push(b'\n');
    ExtensionHistory::parse_jsonl(&next)?;
    Ok(next)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DistributionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn display_name(extension_id: &ExtensionId) -> String {
    extension_id
        .as_str()
        .split('-')
        .map(|segment| {
            let mut characters = segment.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn invalid_bundle(path: impl AsRef<Path>, reason: impl Into<String>) -> DistributionError {
    DistributionError::InvalidReleaseBundle {
        path: path.as_ref().to_path_buf(),
        reason: reason.into(),
    }
}
