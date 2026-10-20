//! Independent controls for witness issuance before the first actual read attempt.
//! Frozen against SDK cohort1 c784af06 before any registration-order correction.
use cap_std::fs::Dir;
use morphir_common::config::load_config_value_observed;
use morphir_common::config::observation::{ReadClass, ReadLimits, SourceReadScope};
use std::path::Path;

const VALID_CONFIG: &[u8] = include_bytes!("source_observation_fixtures/valid-config.json");

fn limits() -> ReadLimits {
    ReadLimits::new(1_000_000, 1_000_000, 16_777_216, 16).unwrap()
}

#[test]
fn successful_actual_read_closes_late_witness_issuance_for_that_same_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("morphir.json");
    std::fs::write(&path, VALID_CONFIG).unwrap();
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = SourceReadScope::bind_workspace(&held, root.path(), limits()).unwrap();
    let value = load_config_value_observed(&path, &scope).unwrap();
    assert_eq!(value["project"]["name"], "scope/fixtures");
    assert!(
        scope
            .register_file(&held, Path::new("morphir.json"))
            .is_err(),
        "a witness issued after this actual read would otherwise falsely count zero"
    );
    let snapshot = scope.seal();
    // Rejecting late issuance may also mark coverage incomplete. Either way,
    // the actual successful read counters and business result are unchanged.
    let actual = snapshot.counts(ReadClass::WorkspaceContent);
    assert_eq!(actual.content_attempts(), 1);
    assert!(actual.read_calls() >= 1);
    assert_eq!(actual.returned_bytes(), 111);
}

#[test]
fn failed_actual_outside_open_also_closes_late_witness_issuance() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("morphir.json"), VALID_CONFIG).unwrap();
    let held = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
    let scope = SourceReadScope::bind_workspace(&held, &workspace, limits()).unwrap();
    assert!(load_config_value_observed(&outer.path().join("missing.json"), &scope).is_err());
    assert!(
        scope
            .register_file(&held, Path::new("morphir.json"))
            .is_err(),
        "closure is caused by the first actual attempt, not successful open/classification"
    );
    let snapshot = scope.seal();
    assert!(snapshot.require_complete().is_err());
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
}

#[test]
fn preregistered_unread_file_has_zero_only_with_a_genuine_completed_other_read_prefix() {
    let root = tempfile::tempdir().unwrap();
    let read_path = root.path().join("morphir.json");
    std::fs::write(&read_path, VALID_CONFIG).unwrap();
    // Distinct actual file identity, even though the contents match exactly.
    std::fs::write(root.path().join("unread.json"), VALID_CONFIG).unwrap();
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = SourceReadScope::bind_workspace(&held, root.path(), limits()).unwrap();
    let unread = scope
        .register_file(&held, Path::new("unread.json"))
        .unwrap();
    let before = scope.observation().snapshot();
    assert!(
        before.require_complete().is_err(),
        "metadata alone is not a completed prefix"
    );
    assert_eq!(before.counts_for(&unread).unwrap().content_attempts(), 0);
    let value = load_config_value_observed(&read_path, &scope).unwrap();
    assert_eq!(value["project"]["name"], "scope/fixtures");
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    let counts = snapshot.counts_for(&unread).unwrap();
    assert_eq!(counts.content_attempts(), 0);
    assert_eq!(counts.read_calls(), 0);
    assert_eq!(counts.returned_bytes(), 0);
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .returned_bytes(),
        111
    );
}
