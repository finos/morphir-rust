use super::super::ToolPackageStore;
use super::support::{tar_gzip_release, write_tar_gzip, write_zip, zip_release};
use crate::tool_archive::copy_zip_entry;
use crate::{Channel, DistributionError, Selection, Sha256Digest};
use morphir_common::home::MorphirHome;
use std::fs;
use std::io::Cursor;
use std::path::Path;

#[test]
fn zip_copy_stops_after_one_byte_beyond_an_understated_size() {
    let mut input = Cursor::new(b"understated");
    let mut output = Vec::new();

    assert!(matches!(
        copy_zip_entry(
            &mut input,
            &mut output,
            1,
            100,
            "desktop.exe",
            Path::new("desktop.exe")
        )
        .unwrap_err(),
        DistributionError::UnsafeToolArchive { .. }
    ));
    assert_eq!(output.len(), 2);
}

#[test]
fn zip_non_portable_components_are_rejected_before_extraction() {
    for entry in ["AUX.txt", "folder/foo.", "bin/a*b"] {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let download = root.path().join("desktop.zip");
        write_zip(
            &download,
            &[("desktop.exe", b"desktop"), (entry, b"invalid")],
        );
        let bytes = fs::read(&download).unwrap();
        let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
            zip_release("desktop.exe"),
            Selection::Channel(Channel::Stable),
            Sha256Digest::of_bytes(&bytes),
            bytes.len() as u64,
        );

        assert!(matches!(
            ToolPackageStore::new(&home)
                .prepare(
                    resolved,
                    crate::DownloadedToolArtifact::test_fixture(download)
                )
                .unwrap_err(),
            crate::DistributionError::UnsafeToolArchive { .. }
        ));
    }
}

#[test]
fn tar_gzip_non_portable_components_are_rejected_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.tar.gz");
    write_tar_gzip(
        &download,
        &[
            ("morphir-desktop", b"desktop"),
            ("resources/AUX.txt", b"reserved"),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        tar_gzip_release(),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );

    assert!(matches!(
        ToolPackageStore::new(&home)
            .prepare(
                resolved,
                crate::DownloadedToolArtifact::test_fixture(download)
            )
            .unwrap_err(),
        crate::DistributionError::UnsafeToolArchive { .. }
    ));
}

#[test]
fn zip_case_aliased_parent_components_are_rejected_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.zip");
    write_zip(
        &download,
        &[
            ("desktop.exe", b"desktop"),
            ("Bin/helper.dll", b"helper"),
            ("bin/config.json", b"config"),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        zip_release("desktop.exe"),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );

    assert!(matches!(
        ToolPackageStore::new(&home)
            .prepare(
                resolved,
                crate::DownloadedToolArtifact::test_fixture(download)
            )
            .unwrap_err(),
        DistributionError::UnsafeToolArchive { .. }
    ));
}

#[test]
fn tar_gzip_case_aliased_parent_components_are_rejected_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.tar.gz");
    write_tar_gzip(
        &download,
        &[
            ("morphir-desktop", b"desktop"),
            ("Bin/helper.so", b"helper"),
            ("bin/config.json", b"config"),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        tar_gzip_release(),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );

    assert!(matches!(
        ToolPackageStore::new(&home)
            .prepare(
                resolved,
                crate::DownloadedToolArtifact::test_fixture(download)
            )
            .unwrap_err(),
        DistributionError::UnsafeToolArchive { .. }
    ));
}
