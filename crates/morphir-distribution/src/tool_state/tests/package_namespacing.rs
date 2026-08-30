use super::super::{ToolInstaller, ToolPackageStore, activate_installed_tool};
use super::support::write_zip;
use crate::{Channel, Selection, Sha256Digest, ToolId, ToolReleaseRecord};
use morphir_common::home::MorphirHome;
use std::fs;

fn zip_release(id: &str, name: &str, entry_point: &str) -> ToolReleaseRecord {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": id, "name": name },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": format!("artifacts/{id}/1.0.0/bundle.zip"),
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "zip", "entryPoint": entry_point },
            "launch": { "kind": "executable", "path": entry_point, "args": [] }
        }]
    }))
    .unwrap()
}

fn raw_release(id: &str, name: &str, filename: &str) -> ToolReleaseRecord {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": id, "name": name },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": format!("artifacts/{id}/1.0.0/{filename}"),
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "raw", "entryPoint": filename },
            "launch": { "kind": "executable", "path": filename, "args": [] }
        }]
    }))
    .unwrap()
}

#[test]
fn identical_archives_with_distinct_entry_points_have_distinct_packages() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let source = root.path().join("bundle.zip");
    write_zip(
        &source,
        &[("first.exe", b"first"), ("second.exe", b"second")],
    );
    let bytes = fs::read(&source).unwrap();
    let digest = Sha256Digest::of_bytes(&bytes);

    let prepare = |directory: &str, id: &str, name: &str, entry_point: &str| {
        let download = root.path().join(directory).join("bundle.zip");
        fs::create_dir_all(download.parent().unwrap()).unwrap();
        fs::copy(&source, &download).unwrap();
        ToolPackageStore::new(&home)
            .prepare(
                crate::ResolvedTrustedToolArtifact::test_fixture(
                    zip_release(id, name, entry_point),
                    Selection::Channel(Channel::Stable),
                    digest.clone(),
                    bytes.len() as u64,
                ),
                crate::DownloadedToolArtifact::test_fixture(download),
            )
            .unwrap()
    };
    let first = prepare("first", "first", "First Tool", "first.exe");
    let second = prepare("second", "second", "Second Tool", "second.exe");
    assert_ne!(first.package_root, second.package_root);

    ToolInstaller::new(&home).install(first).unwrap();
    ToolInstaller::new(&home).install(second).unwrap();

    let first = activate_installed_tool(&home, &ToolId::parse("first").unwrap()).unwrap();
    let second = activate_installed_tool(&home, &ToolId::parse("second").unwrap()).unwrap();
    assert_eq!(fs::read(first.program()).unwrap(), b"first");
    assert_eq!(fs::read(second.program()).unwrap(), b"second");
}

#[test]
fn identical_source_objects_with_raw_and_archive_semantics_coexist() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let source = root.path().join("bundle.zip");
    write_zip(&source, &[("desktop.exe", b"desktop")]);
    let bytes = fs::read(&source).unwrap();
    let digest = Sha256Digest::of_bytes(&bytes);

    let prepare = |directory: &str, release: ToolReleaseRecord| {
        let download = root.path().join(directory).join("bundle.zip");
        fs::create_dir_all(download.parent().unwrap()).unwrap();
        fs::copy(&source, &download).unwrap();
        ToolPackageStore::new(&home)
            .prepare(
                crate::ResolvedTrustedToolArtifact::test_fixture(
                    release,
                    Selection::Channel(Channel::Stable),
                    digest.clone(),
                    bytes.len() as u64,
                ),
                crate::DownloadedToolArtifact::test_fixture(download),
            )
            .unwrap()
    };
    let archive = prepare(
        "archive",
        zip_release("archive", "Archive Tool", "desktop.exe"),
    );
    let raw = prepare("raw", raw_release("raw", "Raw Tool", "bundle.zip"));

    ToolInstaller::new(&home).install(archive).unwrap();
    ToolInstaller::new(&home).install(raw).unwrap();
    assert_eq!(
        fs::read(
            activate_installed_tool(&home, &ToolId::parse("archive").unwrap())
                .unwrap()
                .program()
        )
        .unwrap(),
        b"desktop"
    );
    assert_eq!(
        fs::read(
            activate_installed_tool(&home, &ToolId::parse("raw").unwrap())
                .unwrap()
                .program()
        )
        .unwrap(),
        bytes
    );
}
