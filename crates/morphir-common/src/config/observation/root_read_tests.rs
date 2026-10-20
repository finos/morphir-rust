//! Independent selected-root DATA-acquisition tests. No host/platform coverage inferred.
//! Unit child of config::observation; only phase hooks are private cfg(test).
use super::{
    Coverage, ReadClass, ReadCounts, ReadLimits, RootReadError, RootRelativeDirectory,
    RootRelativeFile, SourceReadScope,
};
use cap_std::fs::Dir;
use std::{io::Read as _, path::Path};

const PRIMARY: &[u8] = include_bytes!("../../../tests/source_observation_fixtures/primary.json");
const RECORD: &[u8] =
    include_bytes!("../../../tests/source_observation_fixtures/private-record.json");

fn held(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}
fn fixture() -> tempfile::TempDir {
    let parent = tempfile::tempdir().unwrap();
    let workspace = parent.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".morphir/compile.dest")).unwrap();
    std::fs::write(workspace.join(".morphir/compile.dest/model.json"), PRIMARY).unwrap();
    std::fs::write(workspace.join(".morphir/compile.json"), RECORD).unwrap();
    parent
}
fn scope(path: &Path) -> SourceReadScope {
    SourceReadScope::bind_workspace(
        &held(path),
        path,
        ReadLimits::new(64, 1024, 16_777_216, 16).unwrap(),
    )
    .unwrap()
}
fn output_route() -> RootRelativeDirectory {
    RootRelativeDirectory::descendant(Path::new(".morphir")).unwrap()
}
fn primary_path() -> RootRelativeFile {
    RootRelativeFile::parse(Path::new("compile.dest/model.json")).unwrap()
}
fn collect(mut file: impl std::io::Read) -> Vec<u8> {
    let mut result = Vec::new();
    let mut block = [0_u8; 65_536];
    loop {
        let count = file.read(&mut block).unwrap();
        if count == 0 {
            break;
        }
        result.extend_from_slice(&block[..count]);
    }
    result
}
fn counts(actual: ReadCounts, attempts: u64, calls: u64, bytes: u64) {
    assert_eq!(actual.content_attempts(), attempts);
    assert_eq!(actual.read_calls(), calls);
    assert_eq!(actual.returned_bytes(), bytes);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn genuine_descendant_primary_is_root_acquired_once_without_eager_read() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let witness = scope
        .register_file(&output, Path::new("compile.dest/model.json"))
        .unwrap();
    let route = output_route();
    let relative = primary_path();
    let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    counts(scope.observation().snapshot().total_counts(), 0, 0, 0);
    assert_eq!(
        scope.observation().snapshot().coverage(),
        Coverage::Unobserved
    );
    let file = scope.open_read_only_root(&plan).unwrap();
    assert!(file.metadata().unwrap().is_file());
    assert_eq!(file.metadata().unwrap().len(), 111);
    counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
    assert_eq!(collect(file), PRIMARY);
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    counts(snapshot.total_counts(), 1, 2, 111);
    counts(snapshot.counts(ReadClass::WorkspaceContent), 1, 2, 111);
    counts(snapshot.counts(ReadClass::AmbientContent), 0, 0, 0);
    counts(snapshot.counts_for(&witness).unwrap(), 1, 2, 111);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn selected_output_equal_to_workspace_uses_closed_selected_root_variant() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    std::fs::write(workspace.join("model.json"), PRIMARY).unwrap();
    let root = held(&workspace);
    let scope = scope(&workspace);
    let relative = RootRelativeFile::parse(Path::new("model.json")).unwrap();
    let plan = scope
        .prepare_root_read(&root, &RootRelativeDirectory::SelectedRoot, &relative)
        .unwrap();
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), PRIMARY);
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    counts(snapshot.total_counts(), 1, 2, 111);
    counts(snapshot.counts(ReadClass::WorkspaceContent), 1, 2, 111);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn genuine_private_record_is_exact237_and_one_data_open_two_reads() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let relative = RootRelativeFile::parse(Path::new("compile.json")).unwrap();
    let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), RECORD);
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    counts(snapshot.total_counts(), 1, 2, 237);
    counts(snapshot.counts(ReadClass::WorkspaceContent), 1, 2, 237);
    counts(snapshot.counts(ReadClass::AmbientContent), 0, 0, 0);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn moved_out_before_actual_root_open_cannot_fall_back_to_held_child() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let inside = workspace.join(".morphir");
    let outside = parent.path().join("outside");
    let scope = scope(&workspace);
    let output = held(&inside);
    let witness = scope
        .register_file(&output, Path::new("compile.dest/model.json"))
        .unwrap();
    let route = output_route();
    let relative = primary_path();
    let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let after_calls = AtomicUsize::new(0);
    let before = || std::fs::rename(&inside, &outside).unwrap();
    let after = || {
        after_calls.fetch_add(1, Ordering::SeqCst);
        std::fs::rename(&outside, &inside).unwrap();
    };
    plan.before_data_acquire = Some(&before);
    plan.after_data_acquire = Some(&after);
    assert!(scope.open_read_only_root(&plan).is_err());
    assert_eq!(after_calls.load(Ordering::SeqCst), 0);
    assert!(outside.join("compile.dest/model.json").is_file());
    counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
    counts(
        scope.observation().snapshot().counts_for(&witness).unwrap(),
        0,
        0,
        0,
    );
    drop(plan);
    assert!(scope.seal().require_complete().is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn replacement_prefix_same_bytes_is_denied_by_pinned_identity_before_read() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let original = workspace.join(".morphir");
    let moved = parent.path().join("moved");
    let replacement = parent.path().join("replacement");
    std::fs::create_dir_all(replacement.join("compile.dest")).unwrap();
    std::fs::write(replacement.join("compile.dest/model.json"), PRIMARY).unwrap();
    let scope = scope(&workspace);
    let output = held(&original);
    let expected = scope
        .register_file(&output, Path::new("compile.dest/model.json"))
        .unwrap();
    let observed = scope
        .register_file(&held(&replacement), Path::new("compile.dest/model.json"))
        .unwrap();
    let route = output_route();
    let relative = primary_path();
    let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let replace = || {
        std::fs::rename(&original, &moved).unwrap();
        std::fs::rename(&replacement, &original).unwrap();
    };
    plan.before_data_acquire = Some(&replace);
    assert!(matches!(
        scope.open_read_only_root(&plan),
        Err(RootReadError::IdentityMismatch)
    ));
    let snapshot = scope.observation().snapshot();
    counts(snapshot.total_counts(), 1, 0, 0);
    counts(snapshot.counts_for(&expected).unwrap(), 0, 0, 0);
    counts(snapshot.counts_for(&observed).unwrap(), 1, 0, 0);
    drop(plan);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn replacement_leaf_same_bytes_is_denied_and_actual_replacement_attempt_retained() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let leaf = workspace.join(".morphir/compile.dest/model.json");
    let original = workspace.join(".morphir/compile.dest/pinned-original");
    let replacement = workspace.join(".morphir/compile.dest/replacement");
    std::fs::write(&replacement, PRIMARY).unwrap();
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let expected = scope
        .register_file(&output, Path::new("compile.dest/model.json"))
        .unwrap();
    let observed = scope
        .register_file(&output, Path::new("compile.dest/replacement"))
        .unwrap();
    let route = output_route();
    let relative = primary_path();
    let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let replace = || {
        std::fs::rename(&leaf, &original).unwrap();
        std::fs::rename(&replacement, &leaf).unwrap();
    };
    plan.before_data_acquire = Some(&replace);
    assert!(matches!(
        scope.open_read_only_root(&plan),
        Err(RootReadError::IdentityMismatch)
    ));
    let snapshot = scope.observation().snapshot();
    counts(snapshot.total_counts(), 1, 0, 0);
    counts(snapshot.counts_for(&expected).unwrap(), 0, 0, 0);
    counts(snapshot.counts_for(&observed).unwrap(), 1, 0, 0);
    drop(plan);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn every_parent_symlink_is_refused_even_when_target_is_inside_workspace() {
    for outside in [false, true] {
        let parent = fixture();
        let workspace = parent.path().join("workspace");
        let dest = workspace.join(".morphir/compile.dest");
        let saved = workspace.join(".morphir/old-dest");
        let target = if outside {
            parent.path().join("target")
        } else {
            workspace.join("target")
        };
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("model.json"), PRIMARY).unwrap();
        let scope = scope(&workspace);
        let output = held(&workspace.join(".morphir"));
        let target_witness = scope
            .register_file(&held(&target), Path::new("model.json"))
            .unwrap();
        let route = output_route();
        let relative = primary_path();
        let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
        let replace = || {
            std::fs::rename(&dest, &saved).unwrap();
            std::os::unix::fs::symlink(&target, &dest).unwrap();
        };
        plan.before_data_acquire = Some(&replace);
        assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::ConfinementRefused)
        ));
        let snapshot = scope.observation().snapshot();
        counts(snapshot.total_counts(), 1, 0, 0);
        counts(snapshot.counts_for(&target_witness).unwrap(), 0, 0, 0);
        drop(plan);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn every_final_symlink_is_refused_even_when_target_is_inside_workspace() {
    for outside in [false, true] {
        let parent = fixture();
        let workspace = parent.path().join("workspace");
        let leaf = workspace.join(".morphir/compile.dest/model.json");
        let old = workspace.join(".morphir/compile.dest/old-model.json");
        let target = if outside {
            parent.path().join("target")
        } else {
            workspace.join("target")
        };
        std::fs::write(&target, PRIMARY).unwrap();
        let scope = scope(&workspace);
        let output = held(&workspace.join(".morphir"));
        let route = output_route();
        let relative = primary_path();
        let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
        let replace = || {
            std::fs::rename(&leaf, &old).unwrap();
            std::os::unix::fs::symlink(&target, &leaf).unwrap();
        };
        plan.before_data_acquire = Some(&replace);
        assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::ConfinementRefused)
        ));
        counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
        drop(plan);
    }
}

#[cfg(windows)]
#[test]
fn windows_parent_reparse_symlinks_are_refused_inside_and_outside_root() {
    for outside in [false, true] {
        let parent = fixture();
        let workspace = parent.path().join("workspace");
        let dest = workspace.join(".morphir/compile.dest");
        let saved = workspace.join(".morphir/old-dest");
        let retained_model = workspace.join(".morphir/retained-model.json");
        let target = if outside {
            parent.path().join("target")
        } else {
            workspace.join("target")
        };
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("model.json"), PRIMARY).unwrap();
        let scope = scope(&workspace);
        let output = held(&workspace.join(".morphir"));
        let route = output_route();
        let relative = primary_path();
        let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
        let replace = || {
            std::fs::rename(dest.join("model.json"), &retained_model).unwrap();
            std::fs::rename(&dest, &saved).unwrap();
            std::os::windows::fs::symlink_dir(&target, &dest).unwrap();
        };
        plan.before_data_acquire = Some(&replace);
        assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::ConfinementRefused)
        ));
        counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
        drop(plan);
    }
}

#[cfg(windows)]
#[test]
fn windows_final_reparse_symlinks_are_refused_inside_and_outside_root() {
    for outside in [false, true] {
        let parent = fixture();
        let workspace = parent.path().join("workspace");
        let leaf = workspace.join(".morphir/compile.dest/model.json");
        let old = workspace.join(".morphir/compile.dest/old-model.json");
        let target = if outside {
            parent.path().join("target")
        } else {
            workspace.join("target")
        };
        std::fs::write(&target, PRIMARY).unwrap();
        let scope = scope(&workspace);
        let output = held(&workspace.join(".morphir"));
        let route = output_route();
        let relative = primary_path();
        let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
        let replace = || {
            std::fs::rename(&leaf, &old).unwrap();
            std::os::windows::fs::symlink_file(&target, &leaf).unwrap();
        };
        plan.before_data_acquire = Some(&replace);
        assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::ConfinementRefused)
        ));
        counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
        drop(plan);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn acquired_directory_replacement_is_not_yielded_as_regular_read_target() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let leaf = workspace.join(".morphir/compile.dest/model.json");
    let old = workspace.join(".morphir/compile.dest/old-model.json");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let relative = primary_path();
    let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let replace = || {
        std::fs::rename(&leaf, &old).unwrap();
        std::fs::create_dir(&leaf).unwrap();
    };
    plan.before_data_acquire = Some(&replace);
    assert!(matches!(
        scope.open_read_only_root(&plan),
        Err(RootReadError::NonRegular)
    ));
    counts(scope.observation().snapshot().total_counts(), 1, 0, 0);
    drop(plan);
}

#[test]
fn root_directory_is_a_variant_and_files_never_use_empty_path_sentinels() {
    for path in [
        "",
        "/absolute",
        "../model",
        "./model",
        "a/../model",
        "a//model",
        "a/./model",
        "a\0b",
    ] {
        assert!(matches!(
            RootRelativeFile::parse(Path::new(path)),
            Err(RootReadError::InvalidRelativePath)
        ));
        assert!(matches!(
            RootRelativeDirectory::descendant(Path::new(path)),
            Err(RootReadError::InvalidRelativePath)
        ));
    }
    let _root = RootRelativeDirectory::SelectedRoot;
    RootRelativeDirectory::descendant(Path::new(".morphir")).unwrap();
    RootRelativeFile::parse(Path::new("compile.dest/model.json")).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_absolute_device_unc_and_stream_names_are_not_relative_file_operands() {
    for path in [
        "C:\\model.json",
        "\\\\server\\share\\model.json",
        "\\??\\C:\\model.json",
        "model.json:stream",
    ] {
        assert!(matches!(
            RootRelativeFile::parse(Path::new(path)),
            Err(RootReadError::InvalidRelativePath)
        ));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn foreign_plan_refuses_before_any_actual_data_attempt_despite_same_spelling() {
    let first = fixture();
    let second = fixture();
    let first_workspace = first.path().join("workspace");
    let second_workspace = second.path().join("workspace");
    let first_scope = scope(&first_workspace);
    let second_scope = scope(&second_workspace);
    let output = held(&first_workspace.join(".morphir"));
    let route = output_route();
    let relative = primary_path();
    let plan = first_scope
        .prepare_root_read(&output, &route, &relative)
        .unwrap();
    assert!(matches!(
        second_scope.open_read_only_root(&plan),
        Err(RootReadError::ForeignScope)
    ));
    counts(first_scope.observation().snapshot().total_counts(), 0, 0, 0);
    counts(
        second_scope.observation().snapshot().total_counts(),
        0,
        0,
        0,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn rename_after_actual_root_data_acquisition_preserves_the_granted_descriptor() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let inside = workspace.join(".morphir");
    let outside = parent.path().join("moved-after-acquisition");
    let scope = scope(&workspace);
    let output = held(&inside);
    let route = output_route();
    let relative = primary_path();
    let mut plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let after_calls = AtomicUsize::new(0);
    let after = || {
        assert_eq!(after_calls.fetch_add(1, Ordering::SeqCst), 0);
        std::fs::rename(&inside, &outside).unwrap();
    };
    plan.after_data_acquire = Some(&after);
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), PRIMARY);
    assert_eq!(after_calls.load(Ordering::SeqCst), 1);
    assert!(outside.join("compile.dest/model.json").is_file());
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    counts(snapshot.total_counts(), 1, 2, 111);
    // This component proves acquisition-time rooted provenance only. Actual
    // host Current/output/offer validation still determines Ready publication.
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn next_target_identity_after_record_read_does_not_issue_retrospective_zero_witness() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let record_path = RootRelativeFile::parse(Path::new("compile.json")).unwrap();
    let record_plan = scope
        .prepare_root_read(&output, &route, &record_path)
        .unwrap();
    assert_eq!(
        collect(scope.open_read_only_root(&record_plan).unwrap()),
        RECORD
    );
    drop(record_plan);
    assert!(
        scope
            .register_file(&output, Path::new("compile.dest/model.json"))
            .is_err()
    );
    let relative = primary_path();
    let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    counts(scope.observation().snapshot().total_counts(), 1, 2, 237);
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), PRIMARY);
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    counts(snapshot.total_counts(), 2, 4, 348);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn partial_read_returns_only_seven_bytes_without_eager_remainder() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let relative = primary_path();
    let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    let mut file = scope.open_read_only_root(&plan).unwrap();
    let mut prefix = [0_u8; 7];
    file.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"{\"forma");
    drop(file);
    drop(plan);
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(snapshot.total_counts().content_attempts(), 1);
    assert!(snapshot.total_counts().read_calls() >= 1);
    assert_eq!(snapshot.total_counts().returned_bytes(), 7);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn observer_overflow_does_not_refuse_second_business_root_open_or_read() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = SourceReadScope::bind_workspace(
        &held(&workspace),
        &workspace,
        ReadLimits::new(1, 1024, 16_777_216, 16).unwrap(),
    )
    .unwrap();
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let relative = primary_path();
    let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), PRIMARY);
    assert_eq!(collect(scope.open_read_only_root(&plan).unwrap()), PRIMARY);
    drop(plan);
    let snapshot = scope.seal();
    counts(snapshot.total_counts(), 1, 2, 111);
    assert_eq!(snapshot.coverage(), Coverage::Partial);
    assert!(snapshot.require_complete().is_err());
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[test]
fn unsupported_native_target_refuses_instead_of_generic_partial_success() {
    let parent = fixture();
    let workspace = parent.path().join("workspace");
    let scope = scope(&workspace);
    let output = held(&workspace.join(".morphir"));
    let route = output_route();
    let relative = primary_path();
    // Unsupported preparation may refuse before identity acquisition. If it can
    // prepare metadata, the actual data entry must still refuse explicitly.
    match scope.prepare_root_read(&output, &route, &relative) {
        Err(RootReadError::UnsupportedPlatform) => {}
        Ok(plan) => assert!(matches!(
            scope.open_read_only_root(&plan),
            Err(RootReadError::UnsupportedPlatform)
        )),
        Err(_) => panic!("unsupported target must retain the explicit capability refusal"),
    }
    counts(scope.observation().snapshot().total_counts(), 0, 0, 0);
}
#[cfg(windows)]
#[test]
fn windows_retained_expected_child_blocks_parent_rename_before_any_data_open() {
    const CHILD: &str = "MORPHIR_ROOT_PARENT_RENAME_CALIBRATION_CHILD";
    const COMPLETION: &str = "MORPHIR_ROOT_PARENT_RENAME_CALIBRATION_COMPLETION";
    if std::env::var_os(CHILD).is_some() {
        let parent = fixture();
        let workspace = parent.path().join("workspace");
        let dest = workspace.join(".morphir/compile.dest");
        let saved = workspace.join(".morphir/old-dest");
        let scope = scope(&workspace);
        let output = held(&workspace.join(".morphir"));
        let route = output_route();
        let relative = primary_path();
        let plan = scope.prepare_root_read(&output, &route, &relative).unwrap();
        // The genuine retained expected-file owner is the child whose live
        // directory entry prevents the parent rename. No DATA is requested.
        let error = std::fs::rename(&dest, &saved).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.raw_os_error(), Some(5));
        counts(scope.observation().snapshot().total_counts(), 0, 0, 0);
        assert_eq!(
            scope.observation().snapshot().coverage(),
            Coverage::Unobserved
        );
        assert!(dest.is_dir());
        assert!(!saved.exists());
        drop(plan);
        // The actual directory becomes renameable after that owner is gone.
        std::fs::rename(&dest, &saved).unwrap();
        counts(scope.observation().snapshot().total_counts(), 0, 0, 0);
        let snapshot = scope.seal();
        assert_eq!(snapshot.coverage(), Coverage::Unobserved);
        assert!(snapshot.require_complete().is_err());
        std::fs::write(
            std::env::var_os(COMPLETION).unwrap(),
            b"actual-plan-rename-calibration-completed",
        )
        .unwrap();
        return;
    }
    struct ChildOwner(Option<std::process::Child>);
    impl Drop for ChildOwner {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let completion = tempfile::tempdir().unwrap();
    let completed = completion.path().join("completed");
    let mut owner = ChildOwner(Some(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "config::observation::root_read_tests::windows_retained_expected_child_blocks_parent_rename_before_any_data_open",
                "--nocapture",
            ])
            .env(CHILD, "owned-calibration")
            .env(COMPLETION, &completed)
            .spawn()
            .unwrap(),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = owner.0.as_mut().unwrap().try_wait().unwrap() {
            owner.0.take(); // try_wait reaped; never kill a subsequently reused PID.
            assert!(
                status.success(),
                "actual parent rename calibration failed: {status}"
            );
            assert_eq!(
                std::fs::read(&completed).unwrap(),
                b"actual-plan-rename-calibration-completed"
            );
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "actual rename calibration exceeded5s; owned child killed and reaped"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
