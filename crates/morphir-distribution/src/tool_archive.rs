//! Safe extraction of portable tool archives.

use crate::store::{add_owner_executable, hash_file};
use crate::{ArtifactFilename, DistributionError, RelativeArtifactPath, Result, Sha256Digest};
use flate2::read::GzDecoder;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TAR_EXTENSION_BYTES: u64 = 1024 * 1024;
const MAX_TAR_STREAM_BYTES: u64 = MAX_UNPACKED_BYTES + (MAX_ARCHIVE_ENTRIES as u64 * 1024);
const MAX_ZIP_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;
const ZIP_EOCD_MINIMUM_BYTES: usize = 22;
const ZIP_EOCD_SEARCH_BYTES: u64 = ZIP_EOCD_MINIMUM_BYTES as u64 + u16::MAX as u64;

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
    let mut file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    preflight_zip(&mut file)?;
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
    preflight_tar_gzip(archive_path)?;
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
        let declared_length = entry.size();
        let path_kind = if entry_type.is_dir() {
            validate_tar_directory_entry(&raw_name, declared_length)?;
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

fn preflight_zip(file: &mut fs::File) -> Result<()> {
    let file_length = file
        .seek(SeekFrom::End(0))
        .map_err(|source| unsafe_archive("", format!("cannot inspect ZIP footer: {source}")))?;
    let tail_length = file_length.min(ZIP_EOCD_SEARCH_BYTES) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(|source| unsafe_archive("", format!("cannot inspect ZIP footer: {source}")))?;
    let mut tail = vec![0_u8; tail_length];
    file.read_exact(&mut tail)
        .map_err(|source| unsafe_archive("", format!("cannot inspect ZIP footer: {source}")))?;

    let mut found = false;
    for position in 0..tail.len().saturating_sub(ZIP_EOCD_MINIMUM_BYTES - 1) {
        if tail[position..].starts_with(b"PK\x05\x06") {
            let comment_length = u16::from_le_bytes([tail[position + 20], tail[position + 21]]);
            if position + ZIP_EOCD_MINIMUM_BYTES + usize::from(comment_length) != tail.len() {
                continue;
            }
            found = true;
            let entries = u16::from_le_bytes([tail[position + 10], tail[position + 11]]);
            if usize::from(entries) > MAX_ARCHIVE_ENTRIES {
                return Err(unsafe_archive(
                    "",
                    format!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries"),
                ));
            }
            let directory_bytes = u32::from_le_bytes([
                tail[position + 12],
                tail[position + 13],
                tail[position + 14],
                tail[position + 15],
            ]);
            if u64::from(directory_bytes) > MAX_ZIP_CENTRAL_DIRECTORY_BYTES {
                return Err(unsafe_archive(
                    "",
                    format!(
                        "ZIP central directory exceeds {MAX_ZIP_CENTRAL_DIRECTORY_BYTES} bytes"
                    ),
                ));
            }
        }
    }
    if !found {
        return Err(unsafe_archive("", "invalid ZIP end-of-directory record"));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| unsafe_archive("", format!("cannot rewind ZIP archive: {source}")))?;
    Ok(())
}

fn preflight_tar_gzip(archive_path: &Path) -> Result<()> {
    let file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let mut decoder = GzDecoder::new(file);
    let mut count = 0_usize;
    let mut stream_bytes = 0_u64;
    let mut pending_pax_size = None;
    let mut block = [0_u8; 512];
    loop {
        if !read_tar_block(&mut decoder, &mut block)? || block.iter().all(|byte| *byte == 0) {
            return Ok(());
        }
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(unsafe_archive(
                "",
                format!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries"),
            ));
        }

        let header = tar::Header::from_byte_slice(&block);
        let size = header
            .size()
            .map_err(|source| unsafe_archive("", format!("invalid tar header: {source}")))?;
        let entry_type = header.entry_type();
        if entry_type.is_gnu_sparse() {
            return Err(unsafe_archive("", "GNU sparse tar entries are not allowed"));
        }
        if is_tar_extension(entry_type) && size > MAX_TAR_EXTENSION_BYTES {
            return Err(unsafe_archive(
                "",
                format!("tar extension exceeds {MAX_TAR_EXTENSION_BYTES} bytes"),
            ));
        }
        let payload_size = if is_tar_extension(entry_type) {
            size
        } else {
            pending_pax_size.take().unwrap_or(size)
        };
        let padded_size = payload_size
            .checked_add(511)
            .map(|bytes| bytes / 512 * 512)
            .ok_or_else(|| unsafe_archive("", "tar entry size overflow"))?;
        stream_bytes = stream_bytes
            .checked_add(512)
            .and_then(|bytes| bytes.checked_add(padded_size))
            .ok_or_else(|| unsafe_archive("", "tar stream size overflow"))?;
        if stream_bytes > MAX_TAR_STREAM_BYTES {
            return Err(unsafe_archive(
                "",
                format!("tar stream expands beyond {MAX_TAR_STREAM_BYTES} bytes"),
            ));
        }
        if entry_type.is_pax_local_extensions() {
            let payload = read_tar_payload(&mut decoder, size)?;
            pending_pax_size = pax_size_override(&payload)?;
            skip_tar_payload(&mut decoder, padded_size - size)?;
        } else {
            skip_tar_payload(&mut decoder, padded_size)?;
        }
    }
}

fn read_tar_block(reader: &mut impl Read, block: &mut [u8; 512]) -> Result<bool> {
    let mut read = 0;
    while read < block.len() {
        match reader.read(&mut block[read..]) {
            Ok(0) if read == 0 => return Ok(false),
            Ok(0) => return Err(unsafe_archive("", "truncated tar header")),
            Ok(bytes) => read += bytes,
            Err(source) => {
                return Err(unsafe_archive(
                    "",
                    format!("cannot decompress tar header: {source}"),
                ));
            }
        }
    }
    Ok(true)
}

fn skip_tar_payload(reader: &mut impl Read, size: u64) -> Result<()> {
    let copied = io::copy(&mut reader.take(size), &mut io::sink())
        .map_err(|source| unsafe_archive("", format!("cannot decompress tar payload: {source}")))?;
    if copied != size {
        return Err(unsafe_archive("", "truncated tar payload"));
    }
    Ok(())
}

fn read_tar_payload(reader: &mut impl Read, size: u64) -> Result<Vec<u8>> {
    let mut payload = vec![0_u8; size as usize];
    reader
        .read_exact(&mut payload)
        .map_err(|source| unsafe_archive("", format!("cannot decompress tar payload: {source}")))?;
    Ok(payload)
}

fn pax_size_override(payload: &[u8]) -> Result<Option<u64>> {
    let mut size = None;
    for extension in tar::PaxExtensions::new(payload) {
        let extension = extension
            .map_err(|source| unsafe_archive("", format!("invalid PAX extension: {source}")))?;
        if extension.key_bytes() == b"size" {
            let value = std::str::from_utf8(extension.value_bytes())
                .map_err(|source| unsafe_archive("", format!("invalid PAX size: {source}")))?;
            size = Some(
                value
                    .parse()
                    .map_err(|source| unsafe_archive("", format!("invalid PAX size: {source}")))?,
            );
        }
    }
    Ok(size)
}

fn is_tar_extension(entry_type: tar::EntryType) -> bool {
    entry_type.is_gnu_longname()
        || entry_type.is_gnu_longlink()
        || entry_type.is_pax_global_extensions()
        || entry_type.is_pax_local_extensions()
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
    relative.validate_declared()?;
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

fn validate_tar_directory_entry(entry: &str, declared_length: u64) -> Result<()> {
    if declared_length == 0 {
        Ok(())
    } else {
        Err(unsafe_archive(
            entry,
            "directory entries must not carry payload bytes",
        ))
    }
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
    use super::{
        MAX_ARCHIVE_ENTRIES, MAX_TAR_EXTENSION_BYTES, MAX_ZIP_CENTRAL_DIRECTORY_BYTES,
        preflight_tar_gzip, preflight_zip, validate_tar_directory_entry, validate_zip_entry_kind,
    };
    use crate::DistributionError;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs;
    use std::io::{self, Read, Write};

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

    #[test]
    fn tar_directories_must_not_carry_payload_bytes() {
        validate_tar_directory_entry("runtime/", 0).unwrap();
        assert!(matches!(
            validate_tar_directory_entry("runtime/", 1).unwrap_err(),
            DistributionError::UnsafeToolArchive { .. }
        ));
    }

    #[test]
    fn zip_entry_count_is_rejected_before_archive_indexing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("too-many.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for index in 0..=MAX_ARCHIVE_ENTRIES {
            writer
                .start_file(
                    format!("{index}.txt"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
        }
        writer.finish().unwrap();

        let mut file = fs::File::open(path).unwrap();
        assert!(matches!(
            preflight_zip(&mut file).unwrap_err(),
            DistributionError::UnsafeToolArchive { .. }
        ));
    }

    #[test]
    fn zip_central_directory_size_is_rejected_before_archive_indexing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("oversized-directory.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        writer
            .start_file("desktop.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let footer = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .unwrap();
        bytes[footer + 12..footer + 16]
            .copy_from_slice(&((MAX_ZIP_CENTRAL_DIRECTORY_BYTES + 1) as u32).to_le_bytes());
        fs::write(&path, bytes).unwrap();

        let mut file = fs::File::open(path).unwrap();
        assert!(matches!(
            preflight_zip(&mut file).unwrap_err(),
            DistributionError::UnsafeToolArchive { .. }
        ));
    }

    #[test]
    fn tar_preflight_counts_hidden_extension_records() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("too-many-extensions.tar.gz");
        let mut encoder = GzEncoder::new(fs::File::create(&path).unwrap(), Compression::fast());
        for _ in 0..=MAX_ARCHIVE_ENTRIES {
            write_tar_extension(&mut encoder, 0);
        }
        encoder.finish().unwrap();

        assert!(matches!(
            preflight_tar_gzip(&path).unwrap_err(),
            DistributionError::UnsafeToolArchive { .. }
        ));
    }

    #[test]
    fn tar_preflight_bounds_extension_payload_before_tar_buffers_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("oversized-extension.tar.gz");
        let mut encoder = GzEncoder::new(fs::File::create(&path).unwrap(), Compression::fast());
        write_tar_extension(&mut encoder, MAX_TAR_EXTENSION_BYTES + 1);
        encoder.finish().unwrap();

        assert!(matches!(
            preflight_tar_gzip(&path).unwrap_err(),
            DistributionError::UnsafeToolArchive { .. }
        ));
    }

    fn write_tar_extension(writer: &mut impl Write, size: u64) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::GNULongName);
        header.set_size(size);
        header.set_cksum();
        writer.write_all(header.as_bytes()).unwrap();
        io::copy(&mut io::repeat(0).take(size), writer).unwrap();
        let padding = (512 - size % 512) % 512;
        io::copy(&mut io::repeat(0).take(padding), writer).unwrap();
    }
}
