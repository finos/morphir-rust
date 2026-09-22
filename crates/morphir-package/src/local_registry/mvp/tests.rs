use super::{InitializeRequest, RestoreRequest, initialize};
use std::{fs, path::PathBuf};
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/mvp-fixture")
}
fn policy() -> Vec<u8> {
    fs::read_to_string(fixture().join("trust-policy.json"))
        .unwrap()
        .replace("previous-authorization", "fresh-metadata")
        .into_bytes()
}
#[tokio::test]
async fn restores_complete_graph_then_reauthenticates_exact_lock() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("trust");
    let policy = policy();
    let root = fs::read(fixture().join("registry/metadata/1.root.json")).unwrap();
    initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &state,
    })
    .unwrap();
    let lock = fs::read(fixture().join("morphir.lock")).unwrap();
    for name in ["first", "second"] {
        let output = dir.path().join(name);
        let report = restore(RestoreRequest {
            policy: &policy,
            lock: &lock,
            registry: &fixture().join("registry"),
            state: &state,
            output: &output,
        })
        .await
        .unwrap();
        assert_eq!(report.packages.len(), 2);
        for package in report.packages {
            assert!(output.join(package.directory).join("ir.json").is_file());
        }
    }
}
#[test]
fn initialization_never_replaces_established_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("trust");
    let policy = policy();
    let root = fs::read(fixture().join("registry/metadata/1.root.json")).unwrap();
    initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &state,
    })
    .unwrap();
    assert!(
        initialize(InitializeRequest {
            policy: &policy,
            root: &root,
            state: &state
        })
        .is_err()
    );
    fs::remove_file(state.join("trust.sqlite")).unwrap();
    assert!(
        initialize(InitializeRequest {
            policy: &policy,
            root: &root,
            state: &state
        })
        .is_err()
    );
}
fn initialized() -> (tempfile::TempDir, Vec<u8>, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let policy = policy();
    let root = fs::read(fixture().join("registry/metadata/1.root.json")).unwrap();
    initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &dir.path().join("trust"),
    })
    .unwrap();
    let lock = fs::read(fixture().join("morphir.lock")).unwrap();
    (dir, policy, lock)
}
#[tokio::test]
async fn missing_established_rollback_rows_refuse_after_restart() {
    let (dir, policy, lock) = initialized();
    let state = dir.path().join("trust");
    restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("first"),
    })
    .await
    .unwrap();
    let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
    db.execute("DELETE FROM metadata", []).unwrap();
    drop(db);
    let result = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("second"),
    })
    .await;
    assert!(
        result.is_err(),
        "deleted rollback floors must never reset trust"
    );
    assert!(!dir.path().join("second").exists());
}
#[tokio::test]
async fn concurrent_and_unresolved_operations_refuse_without_publication() {
    let (dir, policy, lock) = initialized();
    let state = dir.path().join("trust");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&held).unwrap();
    let error = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("out"),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("locked by another"));
    drop(held);
    fs::write(state.join("operation"), b"interrupted").unwrap();
    assert!(
        restore(RestoreRequest {
            policy: &policy,
            lock: &lock,
            registry: &fixture().join("registry"),
            state: &state,
            output: &dir.path().join("out")
        })
        .await
        .is_err()
    );
    assert!(!dir.path().join("out").exists());
}
#[tokio::test]
async fn authentication_failure_persists_refusal_after_restart() {
    let (dir, mut policy, lock) = initialized();
    let state = dir.path().join("trust");
    let mut value: serde_json::Value = serde_json::from_slice(&policy).unwrap();
    value["publisherRules"][0]["publicKeys"] = serde_json::json!(["00".repeat(32)]);
    policy = serde_json::to_vec(&value).unwrap();
    let error = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("out"),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("SignatureInvalid"));
    assert!(state.join("operation").is_file());
    assert!(!dir.path().join("out").exists());
    let policy = self::policy();
    assert!(
        restore(RestoreRequest {
            policy: &policy,
            lock: &lock,
            registry: &fixture().join("registry"),
            state: &state,
            output: &dir.path().join("out")
        })
        .await
        .is_err()
    );
}
fn copy_tree(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target)
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
#[tokio::test]
async fn unlisted_bundle_file_refuses_complete_publication() {
    let (dir, policy, lock) = initialized();
    let registry = dir.path().join("registry");
    copy_tree(&fixture().join("registry"), &registry);
    let bundle = fs::read_dir(registry.join("bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(bundle.join("unlisted.txt"), b"unlisted").unwrap();
    let result = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &registry,
        state: &dir.path().join("trust"),
        output: &dir.path().join("out"),
    })
    .await;
    assert!(result.is_err(), "unlisted source content cannot be ignored");
    assert!(!dir.path().join("out").exists());
}
#[tokio::test]
async fn historical_or_tampered_evidence_pins_are_explicitly_unsupported() {
    for field in ["digest", "path"] {
        let (dir, policy, lock) = initialized();
        let mut value: serde_json::Value = serde_json::from_slice(&lock).unwrap();
        let evidence = value["evidence"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["kind"] == "tuf-targets")
            .unwrap();
        evidence[field] = if field == "digest" {
            serde_json::json!(format!("sha256:{}", "ab".repeat(32)))
        } else {
            serde_json::json!("metadata/2.targets.json")
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        let error = restore(RestoreRequest {
            policy: &policy,
            lock: &bytes,
            registry: &fixture().join("registry"),
            state: &dir.path().join("trust"),
            output: &dir.path().join("out"),
        })
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("historical evidence unsupported by MVP"),
            "{error}"
        );
        assert!(!dir.path().join("out").exists());
    }
}
#[tokio::test]
async fn empty_extra_bundle_directory_refuses_publication() {
    let (dir, policy, lock) = initialized();
    let registry = dir.path().join("registry");
    copy_tree(&fixture().join("registry"), &registry);
    let bundle = fs::read_dir(registry.join("bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::create_dir(bundle.join("empty")).unwrap();
    assert!(
        restore(RestoreRequest {
            policy: &policy,
            lock: &lock,
            registry: &registry,
            state: &dir.path().join("trust"),
            output: &dir.path().join("out")
        })
        .await
        .is_err()
    );
}

// A trusted fixed test clock avoids expiry-dependent tests without a public clock override.
async fn restore(request: RestoreRequest<'_>) -> Result<super::RestoreReport, super::Error> {
    super::restore_at(request, "2027-01-01T00:00:00Z".parse().unwrap()).await
}

#[tokio::test]
async fn tampered_timestamp_and_expired_replay_refuse_with_specific_diagnostics() {
    let (dir, policy, lock) = initialized();
    let registry = dir.path().join("registry");
    copy_tree(&fixture().join("registry"), &registry);
    let timestamp = registry.join("metadata/timestamp.json");
    let original = fs::read(&timestamp).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["signatures"][0]["sig"] = serde_json::json!("00".repeat(64));
    fs::write(timestamp, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &registry,
        state: &dir.path().join("trust"),
        output: &dir.path().join("out"),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Signature threshold"), "{error}");
    let (dir, policy, lock) = initialized();
    let state = dir.path().join("trust");
    restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("first"),
    })
    .await
    .unwrap();
    let error = super::restore_at(
        RestoreRequest {
            policy: &policy,
            lock: &lock,
            registry: &fixture().join("registry"),
            state: &state,
            output: &dir.path().join("expired"),
        },
        "2029-01-01T00:00:00Z".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("expired"), "{error}");
    assert!(!dir.path().join("expired").exists());
}

#[tokio::test]
async fn failed_database_write_leaves_restart_refusal() {
    let (dir, policy, lock) = initialized();
    let state = dir.path().join("trust");
    let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_write BEFORE UPDATE ON state BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
    drop(db);
    let error = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("out"),
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("injected write failure"),
        "{error}"
    );
    assert!(state.join("operation").exists());
    assert!(!dir.path().join("out").exists());
    let db = rusqlite::Connection::open(state.join("trust.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER fail_write").unwrap();
    drop(db);
    let error = restore(RestoreRequest {
        policy: &policy,
        lock: &lock,
        registry: &fixture().join("registry"),
        state: &state,
        output: &dir.path().join("out"),
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("unresolved prior operation"),
        "{error}"
    );
}

#[test]
fn independently_started_operations_have_distinct_backend_identities() {
    let (a, policy, _) = initialized();
    let (b, _, _) = initialized();
    let policy = super::policy(&policy).unwrap();
    let now = "2027-01-01T00:00:00Z".parse().unwrap();
    let a =
        super::store::Backend::begin(&a.path().join("trust"), &policy.repositories()[0], now, 0)
            .unwrap();
    let b =
        super::store::Backend::begin(&b.path().join("trust"), &policy.repositories()[0], now, 0)
            .unwrap();
    assert_ne!(a.binding.id, b.binding.id);
}
