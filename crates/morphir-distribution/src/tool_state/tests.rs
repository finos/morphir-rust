mod archive_portability;
mod package_namespacing;
mod rollback_history;
mod support;

use super::*;
use crate::{Channel, Selection, Sha256Digest, ToolId};
use morphir_common::home::MorphirHome;
use semver::Version;
use std::fs;
use support::*;

#[test]
fn verified_tool_install_activates_offline_and_retains_rollback_release() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let tool_id = ToolId::parse("desktop").unwrap();

    let first = package(&home, "1.0.0", b"desktop-v1");
    ToolInstaller::new(&home).install(first).unwrap();
    let launch = activate_installed_tool(&home, &tool_id).unwrap();
    assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v1");
    assert_eq!(launch.version(), &Version::parse("1.0.0").unwrap());

    let second = package(&home, "2.0.0", b"desktop-v2");
    ToolInstaller::new(&home).install(second).unwrap();
    let installed = list_installed_tools(&home).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(
        installed[0].active().version(),
        &Version::parse("2.0.0").unwrap()
    );
    assert_eq!(installed[0].rollback().len(), 1);
    assert_eq!(
        installed[0].rollback()[0].version(),
        &Version::parse("1.0.0").unwrap()
    );
    assert_eq!(
        installed[0].selection(),
        &Selection::Channel(Channel::Stable)
    );
}

#[test]
fn authenticated_raw_download_is_reverified_and_published_before_activation() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download_root = root.path().join("downloads");
    fs::create_dir_all(&download_root).unwrap();
    let download = download_root.join("desktop.exe");
    let bytes = b"signed-desktop";
    fs::write(&download, bytes).unwrap();
    let digest = Sha256Digest::of_bytes(bytes);
    let release: crate::ToolReleaseRecord = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/desktop.exe",
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "raw", "entryPoint": "desktop.exe" },
            "launch": {
                "kind": "executable",
                "path": "desktop.exe",
                "args": ["--morphir-home"]
            }
        }]
    }))
    .unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        release,
        Selection::Channel(Channel::Stable),
        digest.clone(),
        bytes.len() as u64,
    );
    let downloaded = crate::DownloadedToolArtifact::test_fixture(download);

    let package = ToolPackageStore::new(&home)
        .prepare(resolved, downloaded)
        .unwrap();
    let installed = ToolInstaller::new(&home).install(package).unwrap();
    assert!(installed.store_path().starts_with("store/tools/sha256"));
    assert_eq!(installed.digest(), &digest);
    assert_eq!(installed.snapshot_version(), 1);
    assert_eq!(
        activate_installed_tool(&home, &ToolId::parse("desktop").unwrap())
            .unwrap()
            .args(),
        ["--morphir-home"]
    );
}

#[test]
fn authenticated_zip_is_atomically_expanded_and_every_file_is_reverified_offline() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.zip");
    write_zip(
        &download,
        &[
            ("desktop.exe", b"signed-desktop"),
            ("resources/config.json", br#"{"ok":true}"#),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let digest = Sha256Digest::of_bytes(&bytes);
    let release = zip_release("desktop.exe");
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        release,
        Selection::Channel(Channel::Stable),
        digest,
        bytes.len() as u64,
    );
    let downloaded = crate::DownloadedToolArtifact::test_fixture(download);

    let package = ToolPackageStore::new(&home)
        .prepare(resolved, downloaded)
        .unwrap();
    ToolInstaller::new(&home).install(package).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    let launch = activate_installed_tool(&home, &id).unwrap();
    assert_eq!(fs::read(launch.program()).unwrap(), b"signed-desktop");

    let unexpected = launch.program().parent().unwrap().join("unexpected.dll");
    fs::write(&unexpected, b"unmanifested").unwrap();
    assert!(matches!(
        activate_installed_tool(&home, &id).unwrap_err(),
        crate::DistributionError::InvalidToolManifest { .. }
    ));
    fs::remove_file(unexpected).unwrap();

    let support_file = launch
        .program()
        .parent()
        .unwrap()
        .join("resources/config.json");
    fs::write(support_file, b"tampered").unwrap();
    assert!(matches!(
        activate_installed_tool(&home, &id).unwrap_err(),
        crate::DistributionError::DigestMismatch { .. }
    ));
}

#[test]
fn zip_traversal_is_rejected_without_publishing_or_activating_a_tool() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.zip");
    write_zip(&download, &[("../escape.exe", b"escape")]);
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        zip_release("desktop.exe"),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );

    let error = ToolPackageStore::new(&home)
        .prepare(
            resolved,
            crate::DownloadedToolArtifact::test_fixture(download),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        crate::DistributionError::UnsafeToolArchive { .. }
    ));
    assert!(!root.path().join("escape.exe").exists());
    assert!(!home.tools_catalog_file().exists());
}

#[test]
fn authenticated_tar_gzip_is_expanded_and_reverified_offline() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.tar.gz");
    write_tar_gzip(
        &download,
        &[
            ("morphir-desktop", b"signed-linux-desktop"),
            ("resources/config.json", br#"{"ok":true}"#),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        tar_gzip_release(),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );
    let package = ToolPackageStore::new(&home)
        .prepare(
            resolved,
            crate::DownloadedToolArtifact::test_fixture(download),
        )
        .unwrap();
    ToolInstaller::new(&home).install(package).unwrap();

    let launch = activate_installed_tool(&home, &ToolId::parse("desktop").unwrap()).unwrap();
    assert_eq!(fs::read(launch.program()).unwrap(), b"signed-linux-desktop");
}

#[test]
fn tar_gzip_preserves_safe_executable_bits_for_bundled_helpers() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.tar.gz");
    write_tar_gzip_with_modes(
        &download,
        &[
            ("morphir-desktop", b"desktop", 0o755),
            ("bin/helper", b"helper", 0o755),
        ],
    );
    let bytes = fs::read(&download).unwrap();
    let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
        tar_gzip_release(),
        Selection::Channel(Channel::Stable),
        Sha256Digest::of_bytes(&bytes),
        bytes.len() as u64,
    );

    let package = ToolPackageStore::new(&home)
        .prepare(
            resolved,
            crate::DownloadedToolArtifact::test_fixture(download),
        )
        .unwrap();

    assert!(
        package
            .files
            .iter()
            .any(|file| file.path.as_str().ends_with("bin/helper") && file.executable)
    );
}

#[test]
fn tar_gzip_links_are_rejected_without_catalog_activation() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let download = root.path().join("desktop.tar.gz");
    write_tar_gzip_link(&download, "morphir-desktop", "../escape");
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
    assert!(!home.tools_catalog_file().exists());
}

#[test]
fn failed_tool_catalog_commit_restores_the_previous_active_release() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let tool_id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();

    let writer = FailingCatalogWriter {
        catalog_path: home.tools_catalog_file(),
    };
    let error = ToolInstaller::new(&home)
        .install_with_writer(package(&home, "2.0.0", b"desktop-v2"), &writer)
        .unwrap_err();
    assert!(error.to_string().contains("injected tool catalog failure"));

    let launch = activate_installed_tool(&home, &tool_id).unwrap();
    assert_eq!(launch.version(), &Version::parse("1.0.0").unwrap());
    assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v1");
}

#[test]
fn tool_install_rejects_a_manifest_that_does_not_cover_the_launch_program() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let mut candidate = package(&home, "1.0.0", b"desktop");
    candidate.files.clear();

    assert!(matches!(
        ToolInstaller::new(&home).install(candidate).unwrap_err(),
        crate::DistributionError::InvalidToolManifest { .. }
    ));
    assert!(!home.tools_catalog_file().exists());
}

#[test]
fn explicit_tool_rollback_swaps_active_and_most_recent_retained_release() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    let mut update = package(&home, "2.0.0", b"desktop-v2");
    update.selection = Selection::Channel(Channel::Preview(None));
    ToolInstaller::new(&home).install(update).unwrap();

    let rolled_back = rollback_tool(&home, &id).unwrap();
    assert_eq!(rolled_back.version(), &Version::parse("1.0.0").unwrap());
    let snapshot = &list_installed_tools(&home).unwrap()[0];
    assert_eq!(snapshot.active().version(), rolled_back.version());
    assert_eq!(
        snapshot.rollback()[0].version(),
        &Version::parse("2.0.0").unwrap()
    );
    assert_eq!(snapshot.selection(), &Selection::Channel(Channel::Stable));
    assert_eq!(
        fs::read(activate_installed_tool(&home, &id).unwrap().program()).unwrap(),
        b"desktop-v1"
    );
}

#[test]
fn failed_tool_rollback_restores_the_previous_lock_and_catalog() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "2.0.0", b"desktop-v2"))
        .unwrap();
    let writer = FailingCatalogWriter {
        catalog_path: home.tools_catalog_file(),
    };

    rollback_with_writer(&home, &id, &writer).unwrap_err();

    let launch = activate_installed_tool(&home, &id).unwrap();
    assert_eq!(launch.version(), &Version::parse("2.0.0").unwrap());
    assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v2");
}

#[test]
fn exact_release_repair_replaces_corrupt_bytes_without_changing_selection() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    let launch = activate_installed_tool(&home, &id).unwrap();
    fs::write(launch.program(), b"corrupt").unwrap();
    let (resolved, downloaded) = raw_download(root.path(), "1.0.0", b"desktop-v1");

    let repaired = ToolRepairer::new(&home)
        .repair(&id, resolved, downloaded)
        .unwrap();

    assert_eq!(repaired.version(), &Version::parse("1.0.0").unwrap());
    let snapshot = &list_installed_tools(&home).unwrap()[0];
    assert_eq!(snapshot.selection(), &Selection::Channel(Channel::Stable));
    assert_eq!(
        fs::read(activate_installed_tool(&home, &id).unwrap().program()).unwrap(),
        b"desktop-v1"
    );
}

#[test]
fn repair_preserves_other_tools_that_share_the_digest_directory() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let desktop_id = ToolId::parse("desktop").unwrap();
    let companion_id = ToolId::parse("companion").unwrap();
    let bytes = b"shared-release";
    ToolInstaller::new(&home)
        .install(package_for(
            &home,
            "desktop",
            "Morphir Desktop",
            "desktop.exe",
            "1.0.0",
            bytes,
        ))
        .unwrap();
    ToolInstaller::new(&home)
        .install(package_for(
            &home,
            "companion",
            "Morphir Companion",
            "companion.exe",
            "1.0.0",
            bytes,
        ))
        .unwrap();
    fs::write(
        activate_installed_tool(&home, &desktop_id)
            .unwrap()
            .program(),
        b"corrupt",
    )
    .unwrap();
    let (resolved, downloaded) = raw_download(root.path(), "1.0.0", bytes);

    ToolRepairer::new(&home)
        .repair(&desktop_id, resolved, downloaded)
        .unwrap();

    assert_eq!(
        fs::read(
            activate_installed_tool(&home, &companion_id)
                .unwrap()
                .program()
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn mismatched_repair_candidate_restores_quarantined_active_bytes_and_state() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    let launch = activate_installed_tool(&home, &id).unwrap();
    fs::write(launch.program(), b"corrupt").unwrap();
    let catalog_before = fs::read(home.tools_catalog_file()).unwrap();
    let lock_before = fs::read(home.tools_locks_dir().join("desktop.json")).unwrap();
    let (resolved, downloaded) = raw_download(root.path(), "2.0.0", b"desktop-v2");

    assert!(matches!(
        ToolRepairer::new(&home)
            .repair(&id, resolved, downloaded)
            .unwrap_err(),
        crate::DistributionError::ToolRepairMismatch { .. }
    ));

    assert_eq!(fs::read(home.tools_catalog_file()).unwrap(), catalog_before);
    assert_eq!(
        fs::read(home.tools_locks_dir().join("desktop.json")).unwrap(),
        lock_before
    );
    assert_eq!(
        list_installed_tools(&home).unwrap()[0].active().version(),
        &Version::parse("1.0.0").unwrap()
    );
    assert!(matches!(
        activate_installed_tool(&home, &id).unwrap_err(),
        crate::DistributionError::DigestMismatch { .. }
    ));
}

#[test]
fn activation_recovers_an_interrupted_repair_quarantine() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    let installed = ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();

    super::recovery::simulate_interrupted_repair(&home, &installed).unwrap();

    let launch = activate_installed_tool(&home, &id).unwrap();
    assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v1");
    assert!(!super::repair_journal::repair_journal_path(&home, &id).exists());
}
