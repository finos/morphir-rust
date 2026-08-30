//! Authenticated package materialization into the tool content-addressed store.

use super::verification::verify_one_file;
use crate::store::{add_owner_executable, hash_file};
use crate::tool_archive::{extract_tar_gzip, portable_archive_path, unsafe_archive};
use crate::{
    ArchiveFormat, ArtifactFilename, ArtifactStore, DistributionError, DownloadedToolArtifact,
    Platform, RelativeArtifactPath, ResolvedTrustedToolArtifact, Result, Selection, Sha256Digest,
    ToolId, ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Verified immutable bytes and authenticated metadata ready for catalog activation.
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
    pub(super) targets_version: u64,
    pub(super) target_path: RelativeArtifactPath,
    pub(super) store_path: RelativeArtifactPath,
    pub(super) package_root: Option<RelativeArtifactPath>,
    pub(super) args: Vec<String>,
    pub(super) files: Vec<ToolPackageFile>,
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

    /// Reverify and publish a raw executable, AppImage, or ZIP package into the tool CAS.
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
        let format = resolved.artifact().archive().format();
        let package = match format {
            ArchiveFormat::Raw | ArchiveFormat::Appimage => {
                self.prepare_raw(resolved, downloaded.into_path())
            }
            ArchiveFormat::Zip => self.prepare_zip(resolved, downloaded.into_path()),
            ArchiveFormat::TarGzip => self.prepare_tar_gzip(resolved, downloaded.into_path()),
        }?;
        tracing::info!(
            program = %package.store_path.as_str(),
            file_count = package.files.len(),
            "verified tool package prepared"
        );
        Ok(package)
    }

    fn prepare_raw(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: PathBuf,
    ) -> Result<VerifiedToolPackage> {
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(&downloaded)?;
        let filename = ArtifactFilename::parse(source_name)?;
        let source = RelativeArtifactPath::parse(source_name)?;
        let entry_point = resolved.artifact().launch().path();
        if entry_point.as_str() != source_name {
            return Err(DistributionError::ToolEntryPointMismatch {
                target: source_name.to_owned(),
                entry_point: entry_point.as_str().to_owned(),
            });
        }
        let stored = ArtifactStore::for_tools(self.home).materialize_file(
            source_root,
            &source,
            resolved.digest(),
            &filename,
            true,
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
        let store_path = RelativeArtifactPath::from_native_path(stored.store_path())?;
        let files = vec![ToolPackageFile {
            path: store_path.clone(),
            digest: resolved.digest().clone(),
            length: resolved.length(),
            executable: true,
        }];
        Ok(package_from_resolved(resolved, store_path, None, files))
    }

    fn prepare_zip(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: PathBuf,
    ) -> Result<VerifiedToolPackage> {
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(&downloaded)?;
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

        let digest_directory = stored
            .path()
            .parent()
            .expect("CAS artifact has a digest directory");
        let destination = digest_directory.join("package");
        let staging = tempfile::Builder::new()
            .prefix(".package-")
            .tempdir_in(digest_directory)
            .map_err(|source| DistributionError::Io {
                path: digest_directory.to_path_buf(),
                source,
            })?;
        let staging_root = staging.path().join("root");
        fs::create_dir(&staging_root).map_err(|source| DistributionError::Io {
            path: staging_root.clone(),
            source,
        })?;
        let relative_files = extract_zip(
            stored.path(),
            &staging_root,
            resolved.artifact().launch().path(),
        )?;
        if destination.exists() {
            verify_relative_files(&destination, &relative_files)?;
        } else if let Err(source) = fs::rename(&staging_root, &destination) {
            if destination.exists() {
                verify_relative_files(&destination, &relative_files)?;
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
        Ok(package_from_resolved(
            resolved,
            store_path,
            Some(package_root),
            files,
        ))
    }

    fn prepare_tar_gzip(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: PathBuf,
    ) -> Result<VerifiedToolPackage> {
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(&downloaded)?;
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

        let digest_directory = stored
            .path()
            .parent()
            .expect("CAS artifact has a digest directory");
        let destination = digest_directory.join("package");
        let staging = tempfile::Builder::new()
            .prefix(".package-")
            .tempdir_in(digest_directory)
            .map_err(|source| DistributionError::Io {
                path: digest_directory.to_path_buf(),
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
            .into_iter()
            .map(|file| ToolPackageFile {
                path: file.path,
                digest: file.digest,
                length: file.length,
                executable: file.executable,
            })
            .collect::<Vec<_>>();
        if destination.exists() {
            verify_relative_files(&destination, &relative_files)?;
        } else if let Err(source) = fs::rename(&staging_root, &destination) {
            if destination.exists() {
                verify_relative_files(&destination, &relative_files)?;
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
        Ok(package_from_resolved(
            resolved,
            store_path,
            Some(package_root),
            files,
        ))
    }
}

fn package_from_resolved(
    resolved: ResolvedTrustedToolArtifact,
    store_path: RelativeArtifactPath,
    package_root: Option<RelativeArtifactPath>,
    files: Vec<ToolPackageFile>,
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
        targets_version: resolved.targets_version(),
        target_path: resolved.artifact().target_path().clone(),
        store_path,
        package_root,
        args: resolved.artifact().launch().args().to_vec(),
        files,
    }
}

fn portable_filename(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DistributionError::InvalidValue {
            kind: "downloaded tool filename",
            value: path.to_string_lossy().into_owned(),
            reason: "expected one portable UTF-8 filename",
        })
}

fn home_relative(home: &MorphirHome, path: &Path) -> Result<RelativeArtifactPath> {
    let canonical_home = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let relative =
        path.strip_prefix(&canonical_home)
            .map_err(|_| DistributionError::InstalledPathEscape {
                path: path.to_path_buf(),
                root: canonical_home,
            })?;
    RelativeArtifactPath::from_native_path(relative)
}

fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    entry_point: &RelativeArtifactPath,
) -> Result<Vec<ToolPackageFile>> {
    let file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|source| unsafe_archive("", format!("invalid ZIP archive: {source}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(unsafe_archive(
            "",
            format!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries"),
        ));
    }

    let mut names = BTreeSet::new();
    let mut unpacked = 0_u64;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| unsafe_archive("", format!("invalid ZIP entry: {source}")))?;
        let raw_name = entry.name().to_owned();
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            unsafe_archive(&raw_name, "entry escapes the package root".to_owned())
        })?;
        let relative = portable_archive_path(&enclosed)
            .map_err(|error| unsafe_archive(&raw_name, error.to_string()))?;
        let collision_key = relative.as_str().to_lowercase();
        if !names.insert(collision_key) {
            return Err(unsafe_archive(
                &raw_name,
                "entry collides with another portable path".to_owned(),
            ));
        }
        let unix_mode = entry.unix_mode().unwrap_or(0);
        let mode_kind = unix_mode & 0o170000;
        if mode_kind != 0 && mode_kind != 0o040000 && mode_kind != 0o100000 {
            return Err(unsafe_archive(
                &raw_name,
                "links, devices, and special files are not allowed".to_owned(),
            ));
        }
        let output = destination.join(relative.as_path());
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|source| DistributionError::Io {
                path: output,
                source,
            })?;
            continue;
        }
        unpacked = unpacked
            .checked_add(entry.size())
            .ok_or_else(|| unsafe_archive(&raw_name, "unpacked size overflow".to_owned()))?;
        if unpacked > MAX_UNPACKED_BYTES {
            return Err(unsafe_archive(
                &raw_name,
                format!("archive expands beyond {MAX_UNPACKED_BYTES} bytes"),
            ));
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut output_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        io::copy(&mut entry, &mut output_file).map_err(|source| DistributionError::Io {
            path: output.clone(),
            source,
        })?;
        output_file
            .sync_all()
            .map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        let executable = &relative == entry_point || unix_mode & 0o111 != 0;
        if executable {
            add_owner_executable(&output)?;
        }
        files.push(ToolPackageFile {
            path: relative,
            digest: hash_file(&output)?,
            length: fs::metadata(&output)
                .map_err(|source| DistributionError::Io {
                    path: output.clone(),
                    source,
                })?
                .len(),
            executable,
        });
    }
    if !files.iter().any(|file| &file.path == entry_point) {
        return Err(unsafe_archive(
            entry_point.as_str(),
            "declared launch entry point is missing".to_owned(),
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn verify_relative_files(root: &Path, files: &[ToolPackageFile]) -> Result<()> {
    for file in files {
        let path = root.join(file.path.as_path());
        verify_one_file(&path, &file.digest, file.length, file.executable)?;
    }
    Ok(())
}
