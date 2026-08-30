//! Safe extraction of portable tool archives.

use crate::store::{add_owner_executable, hash_file};
use crate::{ArtifactFilename, DistributionError, RelativeArtifactPath, Result, Sha256Digest};
use flate2::read::GzDecoder;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ExtractedFile {
    pub(crate) path: RelativeArtifactPath,
    pub(crate) digest: Sha256Digest,
    pub(crate) length: u64,
    pub(crate) executable: bool,
}

pub(crate) fn extract_tar_gzip(
    archive_path: &Path,
    destination: &Path,
    entry_point: &RelativeArtifactPath,
) -> Result<Vec<ExtractedFile>> {
    let file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|source| unsafe_archive("", format!("invalid tar.gz archive: {source}")))?;
    let mut names = BTreeSet::new();
    let mut unpacked = 0_u64;
    let mut count = 0_usize;
    let mut files = Vec::new();

    for entry in entries {
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(unsafe_archive(
                "",
                format!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries"),
            ));
        }
        let mut entry =
            entry.map_err(|source| unsafe_archive("", format!("invalid tar entry: {source}")))?;
        let raw_path = entry
            .path()
            .map_err(|source| unsafe_archive("", format!("invalid tar path: {source}")))?
            .into_owned();
        let raw_name = raw_path.to_string_lossy().into_owned();
        let relative = portable_archive_path(&raw_path)
            .map_err(|error| unsafe_archive(&raw_name, error.to_string()))?;
        if !names.insert(relative.as_str().to_lowercase()) {
            return Err(unsafe_archive(
                &raw_name,
                "entry collides with another portable path",
            ));
        }

        let output = destination.join(relative.as_path());
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            fs::create_dir_all(&output).map_err(|source| DistributionError::Io {
                path: output,
                source,
            })?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(unsafe_archive(
                &raw_name,
                "links, devices, and special files are not allowed",
            ));
        }

        let declared_length = entry.size();
        unpacked = unpacked
            .checked_add(declared_length)
            .ok_or_else(|| unsafe_archive(&raw_name, "unpacked size overflow"))?;
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
        let copied =
            io::copy(&mut entry, &mut output_file).map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        if copied != declared_length {
            return Err(unsafe_archive(
                &raw_name,
                format!("declared {declared_length} bytes but extracted {copied}"),
            ));
        }
        output_file
            .sync_all()
            .map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        let declared_executable = entry
            .header()
            .mode()
            .map(|mode| mode & 0o111 != 0)
            .unwrap_or(false);
        let executable = &relative == entry_point || declared_executable;
        if executable {
            add_owner_executable(&output)?;
        }
        files.push(ExtractedFile {
            path: relative,
            digest: hash_file(&output)?,
            length: copied,
            executable,
        });
    }

    if !files.iter().any(|file| &file.path == entry_point) {
        return Err(unsafe_archive(
            entry_point.as_str(),
            "declared launch entry point is missing",
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(crate) fn portable_archive_path(path: &Path) -> Result<RelativeArtifactPath> {
    let relative = RelativeArtifactPath::from_native_path(path)?;
    for segment in relative.as_str().split('/') {
        ArtifactFilename::parse(segment)?;
    }
    Ok(relative)
}

pub(crate) fn unsafe_archive(
    entry: impl Into<String>,
    reason: impl Into<String>,
) -> DistributionError {
    DistributionError::UnsafeToolArchive {
        entry: entry.into(),
        reason: reason.into(),
    }
}
