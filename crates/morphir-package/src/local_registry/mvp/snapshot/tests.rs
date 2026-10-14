use super::*;
use crate::local_registry::mvp::{InitializeRequest, initialize};
use std::fs;

#[test]
fn failed_finalization_removes_already_promoted_output() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture");
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("trust");
    let policy_bytes = fs::read_to_string(fixture.join("trust-policy.json"))
        .unwrap()
        .replace("previous-authorization", "fresh-metadata")
        .into_bytes();
    initialize(InitializeRequest {
        policy: &policy_bytes,
        root: &fs::read(fixture.join("registry/metadata/1.root.json")).unwrap(),
        state: &state,
    })
    .unwrap();
    let policy = policy(&policy_bytes).unwrap();
    let backend = store::Backend::begin(
        &state,
        &policy.repositories()[0],
        "2027-01-01T00:00:00Z".parse().unwrap(),
        0,
    )
    .unwrap();
    let stage = tempfile::tempdir_in(directory.path()).unwrap();
    fs::write(stage.path().join("ir.json"), b"staged bytes").unwrap();
    fs::write(state.join("operation"), b"changed marker").unwrap();
    let output = directory.path().join("out");
    assert!(publish(&backend, stage.path(), &output).is_err());
    assert!(
        !output.exists(),
        "a failed finalization cannot leave a published result"
    );
    assert_eq!(
        fs::read(state.join("operation")).unwrap(),
        b"changed marker"
    );
}
