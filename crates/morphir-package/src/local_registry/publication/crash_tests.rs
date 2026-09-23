use super::*;
use crate::authoring::{AuthoredLibrary, LocalSigningKey};
use serde_json::json;
use std::path::{Path, PathBuf};
fn library() -> AuthoredLibrary {
    library_version("1.0.0")
}
fn library_version(version: &str) -> AuthoredLibrary {
    AuthoredLibrary::create(&serde_json::to_vec(&json!({"packagePath":"example.com/greeting","version":version,"dependencies":{},"exports":{}})).unwrap(),
    br#"{"formatVersion":4,"distribution":{"Library":{"packageName":"example/greeting","dependencies":{},"def":{"modules":{}}}}}"#).unwrap()
}
fn setup() -> (tempfile::TempDir, PathBuf, Vec<u8>, LocalSigningKey) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let key = LocalSigningKey::from_seed([31; 32]);
    let id = key.tuf_key_id().unwrap();
    let role = json!({"keyids":[id.clone()],"threshold":1});
    let root=key.sign_tuf(&json!({"_type":"root","spec_version":"1.0.36","version":1,"expires":"2099-01-01T00:00:00Z","consistent_snapshot":true,
        "keys":{id:key.tuf_public_key()},"roles":{"root":role,"targets":role,"snapshot":role,"timestamp":role}})).unwrap();
    let hash = digest(&root);
    let policy=serde_json::to_vec(&json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy","repositories":[{"identity":hash,
        "bootstrapRoot":{"version":1,"digest":hash},"namespaces":["example.com"]}],"publisherRules":[{"namespace":"example.com","publicKeys":[key.public_key_hex()],"threshold":1}],"continuedUse":"fresh-metadata"})).unwrap();
    Registry::initialize(&base.join("registry"), &base.join("state"), &policy, &root).unwrap();
    std::fs::write(base.join("policy.json"), &policy).unwrap();
    {
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let library = library_version("0.9.0");
        let signed = library.sign(&key).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
    }
    (temp, base, policy, key)
}
fn crash(base: &Path, step: &str) {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "local_registry::publication::crash_tests::worker",
            "--nocapture",
        ])
        .env("MORPHIR_TEST_CRASH_BASE", base)
        .env("MORPHIR_TEST_CRASH_STEP", step)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn worker() {
    let Ok(base) = std::env::var("MORPHIR_TEST_CRASH_BASE") else {
        return;
    };
    let base = PathBuf::from(base);
    let policy = std::fs::read(base.join("policy.json")).unwrap();
    let key = LocalSigningKey::from_seed([31; 32]);
    let library = library();
    let signed = library.sign(&key).unwrap();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    let step = match std::env::var("MORPHIR_TEST_CRASH_STEP").unwrap().as_str() {
        "reserved" => FaultPoint::Reserved,
        "before-timestamp" => FaultPoint::BeforeTimestamp,
        "after-timestamp" => FaultPoint::AfterTimestamp,
        _ => panic!("unknown test point"),
    };
    FAULT.with(|fault| fault.set(Some((step, FaultAction::Crash))));
    registry
        .publish(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            draft.predecessor(),
            &draft.sign(&key, &key, &key).unwrap(),
        )
        .unwrap();
    panic!("fault checkpoint did not run");
}
#[test]
fn crash_before_timestamp_burns_reservations_and_keeps_prior_committed_view() {
    for step in ["reserved", "before-timestamp"] {
        let (_temp, base, policy, key) = setup();
        let previous = std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap();
        crash(&base, step);
        assert_eq!(
            std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap(),
            previous
        );
        let registry =
            Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
        let library = library();
        let signed = library.sign(&key).unwrap();
        let draft = registry
            .prepare(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                "2098-01-01T00:00:00Z",
            )
            .unwrap();
        assert_eq!(
            draft.predecessor(),
            &Predecessor::Timestamp {
                digest: digest(&previous)
            }
        );
        assert_eq!(draft.timestamp_version.to_string(), "3");
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                draft.predecessor(),
                &draft.sign(&key, &key, &key).unwrap(),
            )
            .unwrap();
    }
}
#[test]
fn crash_after_timestamp_observes_complete_visible_successor_without_power_loss_claim() {
    let (_temp, base, policy, key) = setup();
    crash(&base, "after-timestamp");
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let library = library();
    let signed = library.sign(&key).unwrap();
    let timestamp = std::fs::read(base.join("registry/metadata/timestamp.json")).unwrap();
    let invalid = Proposal {
        targets: vec![],
        snapshot: vec![],
        timestamp: vec![],
    };
    assert_eq!(
        registry
            .publish(
                &library,
                signed.record_bytes(),
                signed.envelope_bytes(),
                &Predecessor::Timestamp {
                    digest: digest(&timestamp)
                },
                &invalid
            )
            .unwrap()
            .outcome,
        Outcome::Idempotent
    );
}
#[test]
fn final_flush_failure_returns_uncertain_and_never_rolls_back() {
    let (_temp, base, policy, key) = setup();
    let registry = Registry::open(&base.join("registry"), &base.join("state"), &policy).unwrap();
    let library = library();
    let signed = library.sign(&key).unwrap();
    let draft = registry
        .prepare(
            &library,
            signed.record_bytes(),
            signed.envelope_bytes(),
            "2098-01-01T00:00:00Z",
        )
        .unwrap();
    FAULT.with(|fault| fault.set(Some((FaultPoint::BeforeFinalFlush, FaultAction::Fail))));
    let result = registry.publish(
        &library,
        signed.record_bytes(),
        signed.envelope_bytes(),
        draft.predecessor(),
        &draft.sign(&key, &key, &key).unwrap(),
    );
    FAULT.with(|fault| fault.set(None));
    assert!(matches!(result, Err(Error::CommitOutcomeUncertain)));
    assert!(base.join("registry/metadata/timestamp.json").exists());
}
