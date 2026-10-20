//! Independent scope adversaries for the frozen cohort-1 public API.
//! These controls qualify only observed common-config prefixes, never a host role.
use cap_std::fs::Dir;
use morphir_common::config::load_config_value_observed;
use morphir_common::config::observation::{ReadClass, ReadLimits, SourceReadScope};
use std::path::{Path, PathBuf};

// This valid legacy config is separate from the preserved 111-byte IR primary.
const VALID_CONFIG: &[u8] = include_bytes!("source_observation_fixtures/valid-config.json");

fn generous_limits() -> ReadLimits {
    ReadLimits::new(1_000_000, 1_000_000, 16_777_216, 16).unwrap()
}

fn held(root: &Path) -> Dir {
    Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap()
}

fn write_config(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, VALID_CONFIG).unwrap();
    path
}

fn assert_config(path: &Path, scope: &SourceReadScope) {
    let actual = load_config_value_observed(path, scope).unwrap();
    assert_eq!(actual["project"]["name"], "scope/fixtures");
    assert_eq!(actual["project"]["source_directory"], "src");
    assert_eq!(actual["project"]["version"], "0.1.0");
}

#[cfg(unix)]
#[test]
fn in_root_symlink_to_outside_content_cannot_certify_complete_ambient_zero() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let outside = write_config(outer.path(), "outside.json");
    let link = workspace.join("linked.json");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let root = held(&workspace);
    let outside_directory = held(outer.path());
    let scope = SourceReadScope::bind_workspace(&root, &workspace, generous_limits()).unwrap();
    let witness = scope
        .register_file(&outside_directory, Path::new("outside.json"))
        .unwrap();
    assert_config(&link, &scope);
    let snapshot = scope.seal();
    let actual_file = snapshot.counts_for(&witness).unwrap();
    assert_eq!(actual_file.content_attempts(), 1);
    assert_eq!(actual_file.returned_bytes(), 111);
    if snapshot.require_complete().is_ok() {
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
}

#[test]
fn outside_hardlink_is_ambient_even_when_it_matches_an_in_root_file_witness() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let inside = write_config(&workspace, "model.json");
    let outside = outer.path().join("outside-alias.json");
    std::fs::hard_link(&inside, &outside).unwrap();
    let root = held(&workspace);
    let scope = SourceReadScope::bind_workspace(&root, &workspace, generous_limits()).unwrap();
    let witness = scope.register_file(&root, Path::new("model.json")).unwrap();
    assert_config(&outside, &scope);
    let snapshot = scope.seal();
    let actual_file = snapshot.counts_for(&witness).unwrap();
    assert_eq!(
        actual_file.content_attempts(),
        1,
        "file identity is independent of route class"
    );
    assert_eq!(actual_file.returned_bytes(), 111);
    if snapshot.require_complete().is_ok() {
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
}

#[cfg(unix)]
#[test]
fn replacement_at_the_bound_root_label_does_not_become_the_original_workspace_grant() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    let moved = outer.path().join("moved-original-workspace");
    std::fs::create_dir(&workspace).unwrap();
    write_config(&workspace, "model.json");
    let root = held(&workspace);
    let scope = SourceReadScope::bind_workspace(&root, &workspace, generous_limits()).unwrap();
    let original = scope.register_file(&root, Path::new("model.json")).unwrap();
    std::fs::rename(&workspace, &moved).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    // Same spelling and bytes, but a new directory and a different actual file.
    let replacement = write_config(&workspace, "model.json");
    assert_config(&replacement, &scope);
    let snapshot = scope.seal();
    assert_eq!(
        snapshot.counts_for(&original).unwrap().content_attempts(),
        0
    );
    if snapshot.require_complete().is_ok() {
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
    // The genuine held directory and original file remain alive until after the read.
    assert_eq!(root.read("model.json").unwrap(), VALID_CONFIG);
}

#[test]
fn failed_outside_open_cannot_certify_a_complete_zero_ambient_attempt_count() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let missing = outer.path().join("absent.json");
    let root = held(&workspace);
    let scope = SourceReadScope::bind_workspace(&root, &workspace, generous_limits()).unwrap();
    assert!(load_config_value_observed(&missing, &scope).is_err());
    let snapshot = scope.seal();
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .returned_bytes(),
        0
    );
    assert_eq!(
        snapshot.counts(ReadClass::AmbientContent).returned_bytes(),
        0
    );
    if snapshot.require_complete().is_ok() {
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
    }
}

#[test]
fn attempt_limit_is_aggregate_across_inside_and_outside_in_both_orders() {
    for outside_first in [false, true] {
        let outer = tempfile::tempdir().unwrap();
        let workspace = outer.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let inside = write_config(&workspace, "inside.json");
        let outside = write_config(outer.path(), "outside.json");
        let root = held(&workspace);
        let limits = ReadLimits::new(1, 1_000_000, 16_777_216, 16).unwrap();
        let scope = SourceReadScope::bind_workspace(&root, &workspace, limits).unwrap();
        let (first, second, first_class, second_class) = if outside_first {
            (
                &outside,
                &inside,
                ReadClass::AmbientContent,
                ReadClass::WorkspaceContent,
            )
        } else {
            (
                &inside,
                &outside,
                ReadClass::WorkspaceContent,
                ReadClass::AmbientContent,
            )
        };
        assert_config(first, &scope);
        assert_config(second, &scope);
        let snapshot = scope.seal();
        assert!(
            snapshot.require_complete().is_err(),
            "observation overflow cannot change the two successful business reads"
        );
        assert_eq!(snapshot.counts(first_class).content_attempts(), 1);
        assert_eq!(
            snapshot.counts(second_class).content_attempts(),
            0,
            "the second class does not receive a fresh attempt allowance"
        );
    }
}

#[test]
fn read_call_limit_is_aggregate_across_inside_and_outside_in_both_orders() {
    for outside_first in [false, true] {
        let outer = tempfile::tempdir().unwrap();
        let workspace = outer.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let inside = write_config(&workspace, "inside.json");
        let outside = write_config(outer.path(), "outside.json");
        let root = held(&workspace);
        let limits = ReadLimits::new(1_000_000, 1, 16_777_216, 16).unwrap();
        let scope = SourceReadScope::bind_workspace(&root, &workspace, limits).unwrap();
        let (first, second, first_class, second_class) = if outside_first {
            (
                &outside,
                &inside,
                ReadClass::AmbientContent,
                ReadClass::WorkspaceContent,
            )
        } else {
            (
                &inside,
                &outside,
                ReadClass::WorkspaceContent,
                ReadClass::AmbientContent,
            )
        };
        assert_config(first, &scope);
        assert_config(second, &scope);
        let snapshot = scope.seal();
        // Each nonempty successful physical file read needs at least one read
        // dispatch. A possible first-file EOF call may exhaust this cap earlier;
        // neither case permits a fresh allowance for the second route class.
        assert!(snapshot.require_complete().is_err());
        assert_eq!(snapshot.counts(first_class).read_calls(), 1);
        assert_eq!(snapshot.counts(second_class).read_calls(), 0);
    }
}

#[test]
fn returned_byte_limit_is_aggregate_across_inside_and_outside_in_both_orders() {
    for outside_first in [false, true] {
        let outer = tempfile::tempdir().unwrap();
        let workspace = outer.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let inside = write_config(&workspace, "inside.json");
        let outside = write_config(outer.path(), "outside.json");
        let root = held(&workspace);
        let limits = ReadLimits::new(1_000_000, 1_000_000, 111, 16).unwrap();
        let scope = SourceReadScope::bind_workspace(&root, &workspace, limits).unwrap();
        let (first, second, first_class, second_class) = if outside_first {
            (
                &outside,
                &inside,
                ReadClass::AmbientContent,
                ReadClass::WorkspaceContent,
            )
        } else {
            (
                &inside,
                &outside,
                ReadClass::WorkspaceContent,
                ReadClass::AmbientContent,
            )
        };
        assert_config(first, &scope);
        assert_config(second, &scope);
        let snapshot = scope.seal();
        assert!(snapshot.require_complete().is_err());
        assert_eq!(snapshot.counts(first_class).returned_bytes(), 111);
        assert_eq!(
            snapshot.counts(second_class).returned_bytes(),
            0,
            "the second class does not receive a fresh byte allowance"
        );
    }
}
