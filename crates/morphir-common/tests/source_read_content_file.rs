//! Real open/Read adapter prerequisites. No host completeness or role credit.
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsSyncExt as _};
use cap_std::fs::{Dir, OpenOptions};
use morphir_common::config::observation::{Coverage, ReadClass, ReadLimits, SourceReadScope};
use std::{io::Read as _, path::Path};

fn held(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}
fn bind(root: &Path) -> SourceReadScope {
    SourceReadScope::bind_workspace(
        &held(root),
        root,
        ReadLimits::new(64, 1024, 16_777_216, 16).unwrap(),
    )
    .unwrap()
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    options
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    std::fs::write(
        root.path().join("child/model.json"),
        include_bytes!("source_observation_fixtures/primary.json"),
    )
    .unwrap();
    root
}

#[test]
fn directory_identity_registration_is_metadata_and_cannot_certify_content_zero() {
    let root = fixture();
    let scope = bind(root.path());
    let child = held(&root.path().join("child"));
    let directory = scope.content_directory(&child, Some(&root.path().join("child")));
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.total_counts().content_attempts(), 0);
    assert_eq!(snapshot.total_counts().read_calls(), 0);
    assert_eq!(snapshot.total_counts().returned_bytes(), 0);
    assert!(snapshot.require_complete().is_err());
}

#[test]
fn actual_known_subdirectory_open_preserves_no_follow_and_the_existing_read_loop() {
    let root = fixture();
    let scope = bind(root.path());
    let child = held(&root.path().join("child"));
    let witness = scope
        .register_file(&child, Path::new("model.json"))
        .unwrap();
    let directory = scope.content_directory(&child, Some(&root.path().join("child")));
    let mut file = directory
        .open_content_file(Path::new("model.json"), &options())
        .unwrap();
    assert!(file.metadata().unwrap().is_file());
    assert_eq!(file.metadata().unwrap().len(), 111);
    let before = scope.observation().snapshot();
    assert_eq!(before.total_counts().content_attempts(), 1);
    assert_eq!(before.total_counts().read_calls(), 0);
    assert_eq!(before.total_counts().returned_bytes(), 0);
    assert!(
        before.require_complete().is_err(),
        "an owned file/route is not final seal"
    );
    // The same existing parent loop checks metadata, reads bytes then EOF.
    // This adapter must not read eagerly during open or metadata inspection.
    let mut block = [0u8; 65_536];
    let mut bytes = Vec::new();
    loop {
        let n = file.read(&mut block).unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&block[..n]);
    }
    assert_eq!(
        bytes,
        include_bytes!("source_observation_fixtures/primary.json")
    );
    drop(file);
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.coverage(), Coverage::Partial);
    assert!(snapshot.require_complete().is_err());
    assert_eq!(snapshot.total_counts().content_attempts(), 1);
    assert_eq!(snapshot.total_counts().read_calls(), 2);
    assert_eq!(snapshot.total_counts().returned_bytes(), 111);
    let counts = snapshot.counts_for(&witness).unwrap();
    assert_eq!(counts.content_attempts(), 1);
    assert_eq!(counts.read_calls(), 2);
    assert_eq!(counts.returned_bytes(), 111);
    // Unknown acquisition provenance retains real total/file counts above.
    // These unclassified attempts are not evidence of absent content access.
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        0
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        0
    );
}

#[cfg(unix)]
#[test]
fn no_follow_refusal_is_an_actual_attempt_with_no_content_or_target_witness_read() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("target"),
        include_bytes!("source_observation_fixtures/primary.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("target"),
        root.path().join("child/link"),
    )
    .unwrap();
    let scope = bind(root.path());
    let child = held(&root.path().join("child"));
    let target = scope
        .register_file(&held(outside.path()), Path::new("target"))
        .unwrap();
    let directory = scope.content_directory(&child, Some(&root.path().join("child")));
    assert!(
        directory
            .open_content_file(Path::new("link"), &options())
            .is_err()
    );
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.total_counts().content_attempts(), 1);
    assert_eq!(snapshot.total_counts().read_calls(), 0);
    assert_eq!(snapshot.total_counts().returned_bytes(), 0);
    assert_eq!(snapshot.counts_for(&target).unwrap().content_attempts(), 0);
    assert!(
        snapshot.require_complete().is_err(),
        "unopened target has no confident observed identity/class"
    );
}

#[test]
fn failed_actual_open_survives_as_attempt_and_cannot_certify_complete_zero() {
    let root = fixture();
    let scope = bind(root.path());
    let child = held(&root.path().join("child"));
    let directory = scope.content_directory(&child, Some(&root.path().join("child")));
    let error = directory
        .open_content_file(Path::new("missing"), &options())
        .err()
        .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.total_counts().content_attempts(), 1);
    assert_eq!(snapshot.total_counts().read_calls(), 0);
    assert_eq!(snapshot.total_counts().returned_bytes(), 0);
    assert!(snapshot.require_complete().is_err());
}

#[test]
fn missing_or_wrong_subdirectory_identity_provenance_does_not_change_read_or_certify_zero() {
    for wrong_label in [false, true] {
        let root = fixture();
        let scope = bind(root.path());
        let child = held(&root.path().join("child"));
        let label = if wrong_label { Some(root.path()) } else { None };
        let directory = scope.content_directory(&child, label);
        let mut file = directory
            .open_content_file(Path::new("model.json"), &options())
            .unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            include_bytes!("source_observation_fixtures/primary.json")
        );
        drop(file);
        drop(directory);
        let snapshot = scope.seal();
        assert_eq!(snapshot.total_counts().content_attempts(), 1);
        assert!(snapshot.total_counts().read_calls() >= 1);
        assert_eq!(snapshot.total_counts().returned_bytes(), 111);
        assert!(snapshot.require_complete().is_err());
    }
}

#[test]
fn explicit_outside_grant_is_ambient_despite_actual_file_identity_registration() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("model.json"),
        include_bytes!("source_observation_fixtures/primary.json"),
    )
    .unwrap();
    let scope = bind(root.path());
    let outside_dir = held(outside.path());
    let file_witness = scope
        .register_file(&outside_dir, Path::new("model.json"))
        .unwrap();
    let directory = scope.content_directory(&outside_dir, Some(outside.path()));
    let mut file = directory
        .open_content_file(Path::new("model.json"), &options())
        .unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        bytes,
        include_bytes!("source_observation_fixtures/primary.json")
    );
    drop(file);
    drop(directory);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot
            .counts_for(&file_witness)
            .unwrap()
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        0
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot.counts(ReadClass::AmbientContent).returned_bytes(),
        111
    );
}

#[test]
fn bounded_partial_read_accounts_only_returned_bytes_without_eager_buffering() {
    let root = fixture();
    let scope = bind(root.path());
    let child = held(&root.path().join("child"));
    let directory = scope.content_directory(&child, Some(&root.path().join("child")));
    let mut file = directory
        .open_content_file(Path::new("model.json"), &options())
        .unwrap();
    let mut prefix = [0u8; 7];
    file.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"{\"forma");
    drop(file);
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.coverage(), Coverage::Partial);
    assert!(snapshot.require_complete().is_err());
    assert_eq!(snapshot.total_counts().content_attempts(), 1);
    assert!(snapshot.total_counts().read_calls() >= 1);
    assert_eq!(snapshot.total_counts().returned_bytes(), 7);
}

#[test]
fn actual_selected_root_open_is_complete_with_exact_business_and_witness_counts() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("model.json"),
        include_bytes!("source_observation_fixtures/primary.json"),
    )
    .unwrap();
    let root_dir = held(root.path());
    let scope = SourceReadScope::bind_workspace(
        &root_dir,
        root.path(),
        ReadLimits::new(64, 1024, 16_777_216, 16).unwrap(),
    )
    .unwrap();
    let witness = scope
        .register_file(&root_dir, Path::new("model.json"))
        .unwrap();
    let directory = scope.content_directory(&root_dir, Some(root.path()));
    let mut file = directory
        .open_content_file(Path::new("model.json"), &options())
        .unwrap();
    assert!(file.metadata().unwrap().is_file());
    assert_eq!(file.metadata().unwrap().len(), 111);
    let before = scope.observation().snapshot();
    assert_eq!(before.total_counts().content_attempts(), 1);
    assert_eq!(before.total_counts().read_calls(), 0);
    assert_eq!(before.total_counts().returned_bytes(), 0);
    assert_eq!(before.coverage(), Coverage::Partial);
    let mut block = [0u8; 65_536];
    let mut bytes = Vec::new();
    loop {
        let count = file.read(&mut block).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&block[..count]);
    }
    assert_eq!(
        bytes,
        include_bytes!("source_observation_fixtures/primary.json")
    );
    drop(file);
    drop(directory);
    let snapshot = scope.seal();
    assert_eq!(snapshot.coverage(), Coverage::Complete);
    snapshot.require_complete().unwrap();
    let total = snapshot.total_counts();
    assert_eq!(total.content_attempts(), 1);
    assert_eq!(total.read_calls(), 2);
    assert_eq!(total.returned_bytes(), 111);
    let file_counts = snapshot.counts_for(&witness).unwrap();
    assert_eq!(file_counts.content_attempts(), 1);
    assert_eq!(file_counts.read_calls(), 2);
    assert_eq!(file_counts.returned_bytes(), 111);
    let workspace = snapshot.counts(ReadClass::WorkspaceContent);
    assert_eq!(workspace.content_attempts(), 1);
    assert_eq!(workspace.read_calls(), 2);
    assert_eq!(workspace.returned_bytes(), 111);
    let ambient = snapshot.counts(ReadClass::AmbientContent);
    assert_eq!(ambient.content_attempts(), 0);
    assert_eq!(ambient.read_calls(), 0);
    assert_eq!(ambient.returned_bytes(), 0);
}
