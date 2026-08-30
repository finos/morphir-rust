//! Authenticated package materialization into the tool content-addressed store.

use super::catalog::tool_state_guard;
use super::package_key::extracted_package_path;
use super::verification::verify_one_file;
use crate::state_io::StateGuard;
use crate::tool_archive::{extract_tar_gzip, extract_zip};
use crate::{
    ArchiveFormat, ArtifactFilename, ArtifactStore, DistributionError, DownloadedToolArtifact,
    Platform, RelativeArtifactPath, ResolvedTrustedToolArtifact, Result, Selection, Sha256Digest,
    ToolId, ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Verified immutable bytes and authenticated metadata ready for catalog activation.
///
/// Packages returned by [`ToolPackageStore::prepare`] retain the tool-state guard until consumed
/// by [`super::ToolInstaller::install`]. This prevents repair from quarantining the prepared
/// content between publication and installation.
/// Fields are private so durable state cannot be built from an unchecked path.
#[derive(Debug)]
pub struct VerifiedToolPackage {
    pub(super) selection: Selection,
    pub(super) tool_id: ToolId,
    pub(super) tool_name: String,
    pub(super) version: Version,
    pub(super) status: ToolReleaseStatus,
    pub(super) platform: Platform,
    pub(super) digest: Sha256Digest,
    pub(super) length: u64,
    pub(super) snapshot_version: u64,
    pub(super) target_path: RelativeArtifactPath,
    pub(super) store_path: RelativeArtifactPath,
    pub(super) package_root: Option<RelativeArtifactPath>,
    pub(super) args: Vec<String>,
    pub(super) files: Vec<ToolPackageFile>,
    pub(super) directories: Vec<RelativeArtifactPath>,
    pub(super) state_guard: Option<PreparedPackageGuard>,
}

#[derive(Debug)]
pub(super) struct PreparedPackageGuard {
    home_root: PathBuf,
    state_guard: StateGuard,
}

impl VerifiedToolPackage {
    pub(super) fn take_state_guard(&mut self, home: &MorphirHome) -> Result<Option<StateGuard>> {
        let Some(guard) = self.state_guard.take() else {
            return Ok(None);
        };
        if guard.home_root != home.root() {
            return Err(DistributionError::InvalidToolManifest {
                reason: "prepared package belongs to a different Morphir Home".to_owned(),
            });
        }
        Ok(Some(guard.state_guard))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ToolPackageFile {
    pub(super) path: RelativeArtifactPath,
    pub(super) digest: Sha256Digest,
    pub(super) length: u64,
    pub(super) executable: bool,
}

/// Materialization boundary for authenticated raw executable tool targets.
#[derive(Debug)]
pub struct ToolPackageStore<'home> {
    home: &'home MorphirHome,
}

impl<'home> ToolPackageStore<'home> {
    /// Construct a tool package store for one Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Reverify and publish a raw executable, AppImage, ZIP, or tar.gz package into the tool CAS.
    ///
    /// The returned package retains state serialization and should be passed promptly to
    /// [`super::ToolInstaller::install`], which transfers and releases that guard after commit.
    #[tracing::instrument(
        name = "morphir.tool.package.prepare",
        skip(self, downloaded),
        fields(
            tool_id = %resolved.release().tool_id(),
            version = %resolved.release().version(),
            digest = %resolved.digest(),
            format = ?resolved.artifact().archive().format()
        ),
        err
    )]
    pub fn prepare(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<VerifiedToolPackage> {
        let state_guard = tool_state_guard(self.home)?;
        let mut package = self.prepare_unlocked(resolved, downloaded)?;
        package.state_guard = Some(PreparedPackageGuard {
            home_root: self.home.root().to_path_buf(),
            state_guard,
        });
        Ok(package)
    }

    #[tracing::instrument(
        name = "morphir.tool.package.materialize",
        skip(self, downloaded),
        fields(
            tool_id = %resolved.release().tool_id(),
            version = %resolved.release().version(),
            digest = %resolved.digest(),
            format = ?resolved.artifact().archive().format()
        ),
        err
    )]
    pub(super) fn prepare_unlocked(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<VerifiedToolPackage> {
        let format = resolved.artifact().archive().format();
        let package = match format {
            ArchiveFormat::Raw | ArchiveFormat::Appimage => {
                super::raw_package::prepare(self.home, resolved, downloaded)
            }
            ArchiveFormat::Zip => self.prepare_zip(resolved, downloaded),
            ArchiveFormat::TarGzip => self.prepare_tar_gzip(resolved, downloaded),
        }?;
        tracing::info!(
            program = %package.store_path.as_str(),
            file_count = package.files.len(),
            "verified tool package prepared"
        );
        Ok(package)
    }

    fn prepare_zip(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<VerifiedToolPackage> {
        let downloaded = downloaded.path();
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(downloaded)?;
        let filename = ArtifactFilename::parse(source_name)?;
        let source = RelativeArtifactPath::parse(source_name)?;
        let stored = ArtifactStore::for_tools(self.home).materialize_file(
            source_root,
            &source,
            resolved.digest(),
            &filename,
            false,
        )?;
        let actual_length = fs::metadata(stored.path())
            .map_err(|source| DistributionError::Io {
                path: stored.path().to_path_buf(),
                source,
            })?
            .len();
        if actual_length != resolved.length() {
            return Err(DistributionError::ToolLengthMismatch {
                path: stored.path().to_path_buf(),
                expected: resolved.length(),
                actual: actual_length,
            });
        }

        let digest_directory = self
            .home
            .tools_store_dir()
            .join(resolved.digest().to_string());
        let requested_destination = extracted_package_path(&digest_directory, resolved.artifact());
        let destination_name = requested_destination
            .file_name()
            .expect("tool package destination has a filename")
            .to_owned();
        let package_directory = verified_package_namespace(self.home, &digest_directory)?;
        let destination = package_directory.join(destination_name);
        let staging = tempfile::Builder::new()
            .prefix(".package-")
            .tempdir_in(&package_directory)
            .map_err(|source| DistributionError::Io {
                path: package_directory.clone(),
                source,
            })?;
        let staging_root = staging.path().join("root");
        fs::create_dir(&staging_root).map_err(|source| DistributionError::Io {
            path: staging_root.clone(),
            source,
        })?;
        let extracted = extract_zip(
            stored.path(),
            &staging_root,
            resolved.artifact().launch().path(),
        )?;
        let relative_files = extracted
            .files
            .into_iter()
            .map(|file| ToolPackageFile {
                path: file.path,
                digest: file.digest,
                length: file.length,
                executable: file.executable,
            })
            .collect::<Vec<_>>();
        let relative_directories = extracted.directories;
        if destination.exists() {
            verify_relative_package(&destination, &relative_files, &relative_directories)?;
        } else if let Err(source) = fs::rename(&staging_root, &destination) {
            if destination.exists() {
                verify_relative_package(&destination, &relative_files, &relative_directories)?;
            } else {
                return Err(DistributionError::Io {
                    path: destination,
                    source,
                });
            }
        }

        let program = destination.join(resolved.artifact().launch().path().as_path());
        let store_path = home_relative(self.home, &program)?;
        let package_root = home_relative(self.home, &destination)?;
        let files = relative_files
            .into_iter()
            .map(|file| {
                Ok(ToolPackageFile {
                    path: home_relative(self.home, &destination.join(file.path.as_path()))?,
                    digest: file.digest,
                    length: file.length,
                    executable: file.executable,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let directories = relative_directories
            .into_iter()
            .map(|directory| home_relative(self.home, &destination.join(directory.as_path())))
            .collect::<Result<Vec<_>>>()?;
        Ok(package_from_resolved(
            resolved,
            store_path,
            Some(package_root),
            files,
            directories,
        ))
    }

    fn prepare_tar_gzip(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<VerifiedToolPackage> {
        let downloaded = downloaded.path();
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(downloaded)?;
        let filename = ArtifactFilename::parse(source_name)?;
        let source = RelativeArtifactPath::parse(source_name)?;
        let stored = ArtifactStore::for_tools(self.home).materialize_file(
            source_root,
            &source,
            resolved.digest(),
            &filename,
            false,
        )?;
        let actual_length = fs::metadata(stored.path())
            .map_err(|source| DistributionError::Io {
                path: stored.path().to_path_buf(),
                source,
            })?
            .len();
        if actual_length != resolved.length() {
            return Err(DistributionError::ToolLengthMismatch {
                path: stored.path().to_path_buf(),
                expected: resolved.length(),
                actual: actual_length,
            });
        }

        let digest_directory = self
            .home
            .tools_store_dir()
            .join(resolved.digest().to_string());
        let requested_destination = extracted_package_path(&digest_directory, resolved.artifact());
        let destination_name = requested_destination
            .file_name()
            .expect("tool package destination has a filename")
            .to_owned();
        let package_directory = verified_package_namespace(self.home, &digest_directory)?;
        let destination = package_directory.join(destination_name);
        let staging = tempfile::Builder::new()
            .prefix(".package-")
            .tempdir_in(&package_directory)
            .map_err(|source| DistributionError::Io {
                path: package_directory.clone(),
                source,
            })?;
        let staging_root = staging.path().join("root");
        fs::create_dir(&staging_root).map_err(|source| DistributionError::Io {
            path: staging_root.clone(),
            source,
        })?;
        let extracted = extract_tar_gzip(
            stored.path(),
            &staging_root,
            resolved.artifact().launch().path(),
        )?;
        let relative_files = extracted
            .files
            .into_iter()
            .map(|file| ToolPackageFile {
                path: file.path,
                digest: file.digest,
                length: file.length,
                executable: file.executable,
            })
            .collect::<Vec<_>>();
        let relative_directories = extracted.directories;
        if destination.exists() {
            verify_relative_package(&destination, &relative_files, &relative_directories)?;
        } else if let Err(source) = fs::rename(&staging_root, &destination) {
            if destination.exists() {
                verify_relative_package(&destination, &relative_files, &relative_directories)?;
            } else {
                return Err(DistributionError::Io {
                    path: destination,
                    source,
                });
            }
        }

        let program = destination.join(resolved.artifact().launch().path().as_path());
        let store_path = home_relative(self.home, &program)?;
        let package_root = home_relative(self.home, &destination)?;
        let files = relative_files
            .into_iter()
            .map(|file| {
                Ok(ToolPackageFile {
                    path: home_relative(self.home, &destination.join(file.path.as_path()))?,
                    digest: file.digest,
                    length: file.length,
                    executable: file.executable,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let directories = relative_directories
            .into_iter()
            .map(|directory| home_relative(self.home, &destination.join(directory.as_path())))
            .collect::<Result<Vec<_>>>()?;
        Ok(package_from_resolved(
            resolved,
            store_path,
            Some(package_root),
            files,
            directories,
        ))
    }
}

pub(super) fn package_from_resolved(
    resolved: ResolvedTrustedToolArtifact,
    store_path: RelativeArtifactPath,
    package_root: Option<RelativeArtifactPath>,
    files: Vec<ToolPackageFile>,
    directories: Vec<RelativeArtifactPath>,
) -> VerifiedToolPackage {
    VerifiedToolPackage {
        selection: resolved.selection().clone(),
        tool_id: resolved.release().tool_id().clone(),
        tool_name: resolved.release().tool_name().to_owned(),
        version: resolved.release().version().clone(),
        status: resolved.release().status(),
        platform: resolved.artifact().platform().clone(),
        digest: resolved.digest().clone(),
        length: resolved.length(),
        snapshot_version: resolved.snapshot_version(),
        target_path: resolved.artifact().target_path().clone(),
        store_path,
        package_root,
        args: resolved.artifact().launch().args().to_vec(),
        files,
        directories,
        state_guard: None,
    }
}

pub(super) fn portable_filename(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DistributionError::InvalidValue {
            kind: "downloaded tool filename",
            value: path.to_string_lossy().into_owned(),
            reason: "expected one portable UTF-8 filename",
        })
}

pub(super) fn verified_package_namespace(
    home: &MorphirHome,
    digest_directory: &Path,
) -> Result<PathBuf> {
    let canonical_store =
        fs::canonicalize(home.tools_store_dir()).map_err(|source| DistributionError::Io {
            path: home.tools_store_dir(),
            source,
        })?;
    let canonical_digest =
        fs::canonicalize(digest_directory).map_err(|source| DistributionError::Io {
            path: digest_directory.to_path_buf(),
            source,
        })?;
    if !canonical_digest.starts_with(&canonical_store) {
        return Err(DistributionError::InstalledPathEscape {
            path: canonical_digest,
            root: canonical_store,
        });
    }

    let namespace = canonical_digest.join("packages");
    match fs::create_dir(&namespace) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(DistributionError::Io {
                path: namespace,
                source,
            });
        }
    }
    let metadata = fs::symlink_metadata(&namespace).map_err(|source| DistributionError::Io {
        path: namespace.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        let path = fs::canonicalize(&namespace).unwrap_or_else(|_| namespace.clone());
        return Err(DistributionError::InstalledPathEscape {
            path,
            root: canonical_digest,
        });
    }
    if !metadata.is_dir() {
        return Err(DistributionError::Io {
            path: namespace,
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tool package namespace is not a directory",
            ),
        });
    }

    let canonical_namespace =
        fs::canonicalize(&namespace).map_err(|source| DistributionError::Io {
            path: namespace,
            source,
        })?;
    if !canonical_namespace.starts_with(&canonical_digest) {
        return Err(DistributionError::InstalledPathEscape {
            path: canonical_namespace,
            root: canonical_digest,
        });
    }
    Ok(canonical_namespace)
}

pub(super) fn home_relative(home: &MorphirHome, path: &Path) -> Result<RelativeArtifactPath> {
    let canonical_home = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let canonical_path = fs::canonicalize(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let relative = canonical_path.strip_prefix(&canonical_home).map_err(|_| {
        DistributionError::InstalledPathEscape {
            path: canonical_path.clone(),
            root: canonical_home,
        }
    })?;
    RelativeArtifactPath::from_native_path(relative)
}

pub(super) fn verify_relative_package(
    root: &Path,
    files: &[ToolPackageFile],
    directories: &[RelativeArtifactPath],
) -> Result<()> {
    for file in files {
        let path = root.join(file.path.as_path());
        verify_one_file(&path, &file.digest, file.length, file.executable)?;
    }
    for directory in directories {
        let path = root.join(directory.as_path());
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(DistributionError::InvalidToolManifest {
                    reason: format!(
                        "declared package directory is missing: {}",
                        directory.as_str()
                    ),
                });
            }
            Err(source) => {
                return Err(DistributionError::Io {
                    path: path.clone(),
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(DistributionError::InvalidToolManifest {
                reason: format!(
                    "declared package directory is missing: {}",
                    directory.as_str()
                ),
            });
        }
    }
    Ok(())
}
