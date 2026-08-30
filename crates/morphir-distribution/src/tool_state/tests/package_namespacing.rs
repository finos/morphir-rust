use super::super::package_key::package_path;
use super::super::{ToolInstaller, ToolPackageStore, activate_installed_tool};
use super::support::write_zip;
use crate::{
    ArchiveFormat, Channel, RelativeArtifactPath, Selection, Sha256Digest, ToolId,
    ToolReleaseRecord,
};
use morphir_common::home::MorphirHome;
use std::fs;
use std::io::Write;

fn zip_release(id: &str, name: &str, entry_point: &str) -> ToolReleaseRecord {
    zip_release_named(id, name, entry_point, "bundle.zip")
}

fn zip_release_named(id: &str, name: &str, entry_point: &str, filename: &str) -> ToolReleaseRecord {
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
            "archive": { "format": "zip", "entryPoint": entry_point },
            "launch": { "kind": "executable", "path": entry_point, "args": [] }
        }]
    }))
    .unwrap()
}

#[test]
fn archive_source_filename_cannot_collide_with_its_package_directory() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let entry_point = RelativeArtifactPath::parse("desktop.exe").unwrap();
    let colliding_name = package_path(
        std::path::Path::new("digest"),
        ArchiveFormat::Zip,
        &entry_point,
    )
    .file_name()
    .unwrap()
    .to_str()
    .unwrap()
    .to_owned();
    let download = root.path().join(&colliding_name);
    write_zip(&download, &[("desktop.exe", b"desktop")]);
    let bytes = fs::read(&download).unwrap();
    let package = ToolPackageStore::new(&home)
        .prepare(
            crate::ResolvedTrustedToolArtifact::test_fixture(
                zip_release_named("desktop", "Morphir Desktop", "desktop.exe", &colliding_name),
                Selection::Channel(Channel::Stable),
                Sha256Digest::of_bytes(&bytes),
                bytes.len() as u64,
            ),
            crate::DownloadedToolArtifact::test_fixture(download),
        )
        .unwrap();

    ToolInstaller::new(&home).install(package).unwrap();
    let launch = activate_installed_tool(&home, &ToolId::parse("desktop").unwrap()).unwrap();
    assert_eq!(fs::read(launch.program()).unwrap(), b"desktop");
}

#[test]
fn cached_archive_reuse_rejects_a_missing_declared_directory() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let source = root.path().join("source.zip");
    let file = fs::File::create(&source).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    archive
        .add_directory("runtime/", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive
        .start_file("desktop.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"desktop").unwrap();
    archive.finish().unwrap();
    let bytes = fs::read(&source).unwrap();
    let digest = Sha256Digest::of_bytes(&bytes);
    let prepare = |directory: &str| {
        let download = root.path().join(directory).join("bundle.zip");
        fs::create_dir_all(download.parent().unwrap()).unwrap();
        fs::copy(&source, &download).unwrap();
        ToolPackageStore::new(&home).prepare(
            crate::ResolvedTrustedToolArtifact::test_fixture(
                zip_release("desktop", "Morphir Desktop", "desktop.exe"),
                Selection::Channel(Channel::Stable),
                digest.clone(),
                bytes.len() as u64,
            ),
            crate::DownloadedToolArtifact::test_fixture(download),
        )
    };
    let first = prepare("first").unwrap();
    let runtime_directory = home.root().join(first.directories[0].as_path());
    drop(first);
    fs::remove_dir(&runtime_directory).unwrap();

    assert!(matches!(
        prepare("second").unwrap_err(),
        crate::DistributionError::InvalidToolManifest { .. }
    ));
}

#[test]
fn symlinked_package_namespace_is_rejected_before_staging() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("bundle.zip");
    write_zip(&download, &[("desktop.exe", b"desktop")]);
    let bytes = fs::read(&download).unwrap();
    let digest = Sha256Digest::of_bytes(&bytes);
    let digest_directory = home.tools_store_dir().join(digest.to_string());
    let outside = root.path().join("outside");
    fs::create_dir_all(&digest_directory).unwrap();
    fs::create_dir(&outside).unwrap();
    if !symlink_directory(&outside, &digest_directory.join("packages")) {
        return;
    }

    let error = ToolPackageStore::new(&home)
        .prepare(
            crate::ResolvedTrustedToolArtifact::test_fixture(
                zip_release("desktop", "Morphir Desktop", "desktop.exe"),
                Selection::Channel(Channel::Stable),
                digest,
                bytes.len() as u64,
            ),
            crate::DownloadedToolArtifact::test_fixture(download),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        crate::DistributionError::InstalledPathEscape { .. }
    ));
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(unix)]
fn symlink_directory(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).unwrap();
    true
}

#[cfg(windows)]
fn symlink_directory(target: &std::path::Path, link: &std::path::Path) -> bool {
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(error) => panic!("failed to create package-namespace symlink: {error}"),
    }
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
    let first_package_root = first.package_root.clone();
    ToolInstaller::new(&home).install(first).unwrap();
    let second = prepare("second", "second", "Second Tool", "second.exe");
    assert_ne!(first_package_root, second.package_root);

    ToolInstaller::new(&home).install(second).unwrap();

    let first = activate_installed_tool(&home, &ToolId::parse("first").unwrap()).unwrap();
    assert_eq!(fs::read(first.program()).unwrap(), b"first");
    drop(first);
    let second = activate_installed_tool(&home, &ToolId::parse("second").unwrap()).unwrap();
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
    ToolInstaller::new(&home).install(archive).unwrap();
    let raw = prepare("raw", raw_release("raw", "Raw Tool", "bundle.zip"));

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
