//! End-to-end safety properties of `extract_zip`, independent of the `zip` crate version.

use super::support::write_zip;
use crate::tool_archive::{ExtractedArchive, extract_zip};
use crate::{DistributionError, RelativeArtifactPath};
use std::fs;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

const ENTRY_POINT: &str = "tool.exe";
const CENTRAL_DIRECTORY_HEADER: &[u8] = b"PK\x01\x02";
const LOCAL_FILE_HEADER: &[u8] = b"PK\x03\x04";
const CENTRAL_NAME_OFFSET: usize = 46;
const CENTRAL_UNCOMPRESSED_SIZE_OFFSET: usize = 24;
const LOCAL_UNCOMPRESSED_SIZE_OFFSET: usize = 22;

fn extract(archive: &Path, destination: &Path) -> crate::Result<ExtractedArchive> {
    extract_zip(
        archive,
        destination,
        &RelativeArtifactPath::parse(ENTRY_POINT).unwrap(),
    )
}

fn assert_unsafe(result: crate::Result<ExtractedArchive>) {
    match result {
        Err(DistributionError::UnsafeToolArchive { .. }) => {}
        Err(other) => panic!("expected UnsafeToolArchive, got {other:?}"),
        Ok(_) => panic!("expected UnsafeToolArchive, but the archive was extracted"),
    }
}

fn header_offsets(bytes: &[u8], signature: &[u8]) -> Vec<usize> {
    bytes
        .windows(signature.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == signature).then_some(offset))
        .collect()
}

/// An archive with one entry whose local header says `local_name` and whose central directory
/// says `central_name`. The names must have the same length.
fn an_archive_whose_headers_disagree_on_the_name(
    path: &Path,
    local_name: &str,
    central_name: &str,
) {
    assert_eq!(local_name.len(), central_name.len());
    write_zip(path, &[(local_name, b"payload")]);
    let mut bytes = fs::read(path).unwrap();
    let central = header_offsets(&bytes, CENTRAL_DIRECTORY_HEADER)[0] + CENTRAL_NAME_OFFSET;
    bytes[central..central + central_name.len()].copy_from_slice(central_name.as_bytes());
    fs::write(path, bytes).unwrap();
}

/// An archive with two entries that the central directory gives the same name.
fn an_archive_with_a_duplicate_central_name(path: &Path) {
    write_zip(path, &[(ENTRY_POINT, b"first"), ("tool.bin", b"second")]);
    let mut bytes = fs::read(path).unwrap();
    let central = header_offsets(&bytes, CENTRAL_DIRECTORY_HEADER)[1] + CENTRAL_NAME_OFFSET;
    bytes[central..central + ENTRY_POINT.len()].copy_from_slice(ENTRY_POINT.as_bytes());
    fs::write(path, bytes).unwrap();
}

/// An archive whose headers both declare fewer uncompressed bytes than the entry expands to.
fn an_archive_that_understates_its_size(path: &Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    writer
        .start_file(
            ENTRY_POINT,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"understated payload").unwrap();
    writer.finish().unwrap();
    let mut bytes = fs::read(path).unwrap();
    let understated = 1_u32.to_le_bytes();
    let local = header_offsets(&bytes, LOCAL_FILE_HEADER)[0] + LOCAL_UNCOMPRESSED_SIZE_OFFSET;
    bytes[local..local + 4].copy_from_slice(&understated);
    let central =
        header_offsets(&bytes, CENTRAL_DIRECTORY_HEADER)[0] + CENTRAL_UNCOMPRESSED_SIZE_OFFSET;
    bytes[central..central + 4].copy_from_slice(&understated);
    fs::write(path, bytes).unwrap();
}

fn an_archive_with_a_symlink(path: &Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    writer
        .start_file(ENTRY_POINT, SimpleFileOptions::default())
        .unwrap();
    writer
        .add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();
}

fn an_archive_with_modes(path: &Path, entries: &[(&str, u32)]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, mode) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default().unix_permissions(*mode))
            .unwrap();
        writer.write_all(b"contents").unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn a_well_formed_archive_is_extracted() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("tool.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    write_zip(
        &archive,
        &[(ENTRY_POINT, b"tool"), ("resources/data.txt", b"data")],
    );

    let extracted = extract(&archive, &destination).unwrap();

    assert_eq!(extracted.files.len(), 2);
    assert_eq!(fs::read(destination.join(ENTRY_POINT)).unwrap(), b"tool");
    assert_eq!(
        fs::read(destination.join("resources/data.txt")).unwrap(),
        b"data"
    );
}

#[test]
fn an_archive_whose_headers_disagree_on_a_name_is_rejected() {
    // zip 8.6.0, and 9.2.1 without the header check, extract this under the central name.
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("cloaked.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    an_archive_whose_headers_disagree_on_the_name(&archive, "../a.exe", ENTRY_POINT);

    assert_unsafe(extract(&archive, &destination));
    assert!(!destination.join(ENTRY_POINT).exists());
    assert!(!root.path().join("a.exe").exists());
}

#[test]
fn a_duplicate_central_name_is_rejected() {
    // zip 8.6.0, and 9.2.1 without the header check, index entries by central name: the later
    // entry replaces the earlier one, and the extractor's collision check never sees the duplicate.
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("duplicate.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    an_archive_with_a_duplicate_central_name(&archive);

    assert_unsafe(extract(&archive, &destination));
    assert!(!destination.join(ENTRY_POINT).exists());
}

#[test]
fn an_entry_that_escapes_the_root_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("escape.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    write_zip(
        &archive,
        &[(ENTRY_POINT, b"tool"), ("../escape.exe", b"escape")],
    );

    assert_unsafe(extract(&archive, &destination));
    assert!(!root.path().join("escape.exe").exists());
}

#[test]
fn an_absolute_entry_name_stays_inside_the_root() {
    // `enclosed_name` drops a leading separator, so the entry lands under the destination.
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("absolute.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    let outside = root.path().join("outside.exe");
    let absolute = outside.to_str().unwrap();
    write_zip(&archive, &[(ENTRY_POINT, b"tool"), (absolute, b"outside")]);

    extract(&archive, &destination).unwrap();

    assert!(!outside.exists());
    assert_eq!(
        fs::read(destination.join(absolute.trim_start_matches('/'))).unwrap(),
        b"outside"
    );
}

#[test]
fn a_symlink_entry_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("symlink.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    an_archive_with_a_symlink(&archive);

    assert_unsafe(extract(&archive, &destination));
    assert!(fs::symlink_metadata(destination.join("link")).is_err());
}

#[test]
fn a_case_alias_of_another_entry_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("alias.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    write_zip(&archive, &[(ENTRY_POINT, b"tool"), ("TOOL.exe", b"alias")]);

    assert_unsafe(extract(&archive, &destination));
}

#[test]
fn an_understated_entry_size_stops_the_copy() {
    // zip 8.6.0 lets the extractor's own one-byte-over check see this. zip 9 fails the read first,
    // and the extractor must report that as the same unsafe archive.
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("understated.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    an_archive_that_understates_its_size(&archive);

    assert_unsafe(extract(&archive, &destination));
    assert!(fs::metadata(destination.join(ENTRY_POINT)).unwrap().len() <= 2);
}

#[test]
fn modes_decide_which_files_are_executable() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("modes.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    an_archive_with_modes(
        &archive,
        &[(ENTRY_POINT, 0o644), ("helper", 0o755), ("data.txt", 0o644)],
    );

    let extracted = extract(&archive, &destination).unwrap();

    let executable = |name: &str| {
        extracted
            .files
            .iter()
            .find(|file| file.path.as_str() == name)
            .unwrap()
            .executable
    };
    assert!(
        executable(ENTRY_POINT),
        "the entry point is always executable"
    );
    assert!(executable("helper"));
    assert!(!executable("data.txt"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |name: &str| {
            fs::metadata(destination.join(name))
                .unwrap()
                .permissions()
                .mode()
        };
        assert_ne!(mode("helper") & 0o100, 0);
        assert_eq!(mode("data.txt") & 0o111, 0);
    }
}
