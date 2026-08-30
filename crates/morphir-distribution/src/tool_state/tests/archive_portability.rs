use super::super::ToolPackageStore;
use super::support::{tar_gzip_release, write_tar_gzip, write_zip, zip_release};
use crate::{Channel, Selection, Sha256Digest};
use morphir_common::home::MorphirHome;
use std::fs;

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
