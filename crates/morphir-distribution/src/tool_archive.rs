//! Safe extraction of portable tool archives.

use crate::store::{add_owner_executable, hash_file};
use crate::{ArtifactFilename, DistributionError, RelativeArtifactPath, Result, Sha256Digest};
use flate2::read::GzDecoder;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ExtractedFile {
    pub(crate) path: RelativeArtifactPath,
    pub(crate) digest: Sha256Digest,
    pub(crate) length: u64,
    pub(crate) executable: bool,
}

pub(crate) struct ExtractedArchive {
    pub(crate) files: Vec<ExtractedFile>,
    pub(crate) directories: Vec<RelativeArtifactPath>,
}

pub(crate) fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    entry_point: &RelativeArtifactPath,
) -> Result<ExtractedArchive> {
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

    let mut names = PortableArchivePaths::default();
    let mut unpacked = 0_u64;
    let mut files = Vec::new();
    let mut directories = Vec::new();
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
        let is_directory = entry.is_dir();
        let unix_mode = entry.unix_mode().unwrap_or(0);
        validate_zip_entry_kind(&raw_name, is_directory, unix_mode)?;
        if !names.insert(
            &relative,
            if is_directory {
                ArchivePathKind::Directory
            } else {
                ArchivePathKind::File
            },
        ) {
            return Err(unsafe_archive(
                &raw_name,
                "entry collides with another portable path",
            ));
        }
        let output = destination.join(relative.as_path());
        if is_directory {
            fs::create_dir_all(&output).map_err(|source| DistributionError::Io {
                path: output,
                source,
            })?;
            directories.push(relative);
            continue;
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
        let remaining = MAX_UNPACKED_BYTES - unpacked;
        let declared_size = entry.size();
        unpacked += copy_zip_entry(
            &mut entry,
            &mut output_file,
            declared_size,
            remaining,
            &raw_name,
            &output,
        )?;
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
        files.push(ExtractedFile {
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
    finish_extraction(files, directories, entry_point)
}

pub(crate) fn extract_tar_gzip(
    archive_path: &Path,
    destination: &Path,
    entry_point: &RelativeArtifactPath,
) -> Result<ExtractedArchive> {
    let file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|source| unsafe_archive("", format!("invalid tar.gz archive: {source}")))?;
    let mut names = PortableArchivePaths::default();
    let mut unpacked = 0_u64;
    let mut count = 0_usize;
    let mut files = Vec::new();
    let mut directories = Vec::new();

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
        let entry_type = entry.header().entry_type();
        let path_kind = if entry_type.is_dir() {
            ArchivePathKind::Directory
        } else if entry_type.is_file() {
            ArchivePathKind::File
        } else {
            return Err(unsafe_archive(
                &raw_name,
                "links, devices, and special files are not allowed",
            ));
        };
        if !names.insert(&relative, path_kind) {
            return Err(unsafe_archive(
                &raw_name,
                "entry collides with another portable path",
            ));
        }

        let output = destination.join(relative.as_path());
        if entry_type.is_dir() {
            fs::create_dir_all(&output).map_err(|source| DistributionError::Io {
                path: output,
                source,
            })?;
            directories.push(relative);
            continue;
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

    finish_extraction(files, directories, entry_point)
}

fn finish_extraction(
    mut files: Vec<ExtractedFile>,
    mut directories: Vec<RelativeArtifactPath>,
    entry_point: &RelativeArtifactPath,
) -> Result<ExtractedArchive> {
    if !files.iter().any(|file| &file.path == entry_point) {
        return Err(unsafe_archive(
            entry_point.as_str(),
            "declared launch entry point is missing",
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    directories.sort();
    Ok(ExtractedArchive { files, directories })
}

#[derive(Default)]
struct PortableArchivePaths {
    entries: BTreeSet<String>,
    components: BTreeMap<String, (String, ArchivePathKind)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchivePathKind {
    File,
    Directory,
}

impl PortableArchivePaths {
    fn insert(&mut self, path: &RelativeArtifactPath, kind: ArchivePathKind) -> bool {
        if !self.entries.insert(path.as_str().to_owned()) {
            return false;
        }

        let mut prefix = String::new();
        let mut components = path.as_str().split('/').peekable();
        while let Some(component) = components.next() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let component_kind = if components.peek().is_some() {
                ArchivePathKind::Directory
            } else {
                kind
            };
            let normalized = prefix.nfc().collect::<String>();
            let folded = normalized
                .as_str()
                .case_fold()
                .collect::<String>()
                .nfc()
                .collect::<String>();
            match self.components.get(&folded) {
                Some((existing, existing_kind))
                    if existing != &prefix || *existing_kind != component_kind =>
                {
                    return false;
                }
                Some(_) => {}
                None => {
                    self.components
                        .insert(folded, (prefix.clone(), component_kind));
                }
            }
        }
        true
    }
}

pub(crate) fn copy_zip_entry<R: Read, W: io::Write>(
    input: &mut R,
    output: &mut W,
    declared_size: u64,
    remaining_budget: u64,
    entry_name: &str,
    output_path: &Path,
) -> Result<u64> {
    if declared_size > remaining_budget {
        return Err(unsafe_archive(
            entry_name,
            format!("archive expands beyond {MAX_UNPACKED_BYTES} bytes"),
        ));
    }
    let actual = io::copy(&mut input.take(declared_size + 1), output).map_err(|source| {
        DistributionError::Io {
            path: output_path.to_path_buf(),
            source,
        }
    })?;
    if actual != declared_size {
        return Err(unsafe_archive(
            entry_name,
            format!("declared {declared_size} bytes but expanded to at least {actual}"),
        ));
    }
    Ok(actual)
}

pub(crate) fn portable_archive_path(path: &Path) -> Result<RelativeArtifactPath> {
    let relative = RelativeArtifactPath::from_native_path(path)?;
    for segment in relative.as_str().split('/') {
        ArtifactFilename::parse(segment)?;
    }
    Ok(relative)
}

fn validate_zip_entry_kind(entry: &str, is_directory: bool, unix_mode: u32) -> Result<()> {
    let mode_kind = unix_mode & 0o170000;
    if mode_kind != 0 && mode_kind != 0o040000 && mode_kind != 0o100000 {
        return Err(unsafe_archive(
            entry,
            "links, devices, and special files are not allowed",
        ));
    }
    if (mode_kind == 0o040000 && !is_directory) || (mode_kind == 0o100000 && is_directory) {
        return Err(unsafe_archive(
            entry,
            "entry name and Unix mode describe different entry kinds",
        ));
    }
    Ok(())
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

#[cfg(test)]
mod zip_entry_kind_tests {
    use super::validate_zip_entry_kind;
    use crate::DistributionError;

    #[test]
    fn zip_names_and_unix_modes_must_describe_the_same_entry_kind() {
        for (is_directory, unix_mode) in [(false, 0o040755), (true, 0o100644)] {
            assert!(matches!(
                validate_zip_entry_kind("runtime", is_directory, unix_mode).unwrap_err(),
                DistributionError::UnsafeToolArchive { .. }
            ));
        }

        validate_zip_entry_kind("runtime/", true, 0o040755).unwrap();
        validate_zip_entry_kind("runtime/app", false, 0o100755).unwrap();
    }
}
