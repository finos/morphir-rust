#[path = "local_registry/tuf_backend.rs"]
mod backend;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod fixtures;

use async_trait::async_trait;
use fixtures::*;
use morphir_package::local_registry::tuf::{AdmissionError, MetadataLoadError, ProfileAdmission};
use package_tough::{
    FilesystemTransport, Transport, TransportError, TransportStream,
    experimental_storage::{MetadataRole, PackageLoadOutcome},
};
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::{Arc, Mutex},
};
use url::Url;

#[derive(Debug, Clone, Default)]
struct ObservedFiles(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl Transport for ObservedFiles {
    async fn fetch(&self, url: Url) -> Result<TransportStream, TransportError> {
        self.0
            .lock()
            .unwrap()
            .push(url.path().rsplit('/').next().unwrap().to_owned());
        FilesystemTransport.fetch(url).await
    }
}

fn package_targets() -> Vec<u8> {
    let release = json!({"packagePath":"example.com/finance/example", "version":"1.0.0"});
    let mut body = targets_body(1);
    let description = |bytes: &[u8], custom: Value| {
        let mut description = meta(bytes, 1);
        description.as_object_mut().unwrap().remove("version");
        description["custom"] = json!({"morphir":custom});
        description
    };
    // These tests authenticate the complete metadata inventory, not target content
    // or publisher authority. Each entry carries the package status discriminator.
    body["targets"] = json!({
        "records/example.json":description(b"record fixture", json!({
            "formatVersion":"0.1.0-draft.3", "kind":"LibraryRelease", "release":release, "status":"active"
        })),
        "statements/example.json":description(b"statement fixture", json!({
            "formatVersion":"0.1.0-draft.3", "kind":"LibraryReleaseStatement", "release":release
        }))
    });
    sign(body, &[(17, key(17))])
}

fn signed_body(bytes: &[u8]) -> Value {
    serde_json::from_slice::<Value>(bytes).unwrap()["signed"].clone()
}

fn write_view(directory: &std::path::Path, timestamp: &[u8], snapshot: &[u8], targets: &[u8]) {
    for (name, bytes) in [
        ("timestamp.json", timestamp),
        ("1.snapshot.json", snapshot),
        ("1.targets.json", targets),
    ] {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}

async fn load(
    guard: Arc<ProfileAdmission>,
    directory: &std::path::Path,
    observed: ObservedFiles,
) -> Result<PackageLoadOutcome, MetadataLoadError> {
    guard
        .load_metadata(
            Box::new(observed),
            Url::from_directory_path(directory).unwrap(),
        )
        .await
}

fn has_admission_error(
    error: &(dyn Error + 'static),
    predicate: impl Fn(&AdmissionError) -> bool,
) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error
            .downcast_ref::<AdmissionError>()
            .is_some_and(&predicate)
        {
            return true;
        }
        // The experimental port keeps the backend error in its public variant,
        // but its std::error::Error implementation does not expose source().
        if let Some(package_tough::experimental_storage::Error::Backend(inner)) =
            error.downcast_ref::<package_tough::experimental_storage::Error>()
        {
            return inner
                .downcast_ref::<AdmissionError>()
                .is_some_and(&predicate);
        }
        current = error.source();
    }
    false
}

#[tokio::test]
async fn exactly_32_rotations_finish_after_terminal_absence_and_load_complete_view() {
    let (backend, guard) = backend::setup();
    let directory = tempfile::tempdir().unwrap();
    for version in 2..=33 {
        std::fs::write(
            directory.path().join(format!("{version}.root.json")),
            root(version, 17),
        )
        .unwrap();
    }
    let targets = package_targets();
    let snapshot = snapshot(&targets, 17);
    let timestamp = timestamp(&snapshot, 17);
    write_view(directory.path(), &timestamp, &snapshot, &targets);
    let observed = ObservedFiles::default();
    let outcome = load(Arc::new(guard), directory.path(), observed.clone())
        .await
        .unwrap();
    assert!(matches!(outcome, PackageLoadOutcome::Updated(_)));
    let calls = observed.0.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|name| name.ends_with(".root.json"))
            .count(),
        33
    );
    assert_eq!(calls[32], "34.root.json");
    assert_eq!(calls[33], "timestamp.json");
    let state = backend.0.lock().unwrap();
    assert_eq!(state.state.current_root, root(33, 17));
    assert_eq!(state.root_chain.len(), 33);
    assert!(state.state.reset_baseline.is_none());
    assert_eq!(state.state.metadata.len(), 3);
    assert_eq!(state.state.accepted_time, None);
}

#[tokio::test]
async fn thirty_third_rotation_is_rejected_without_losing_accepted_root_33() {
    let (backend, guard) = backend::setup();
    let directory = tempfile::tempdir().unwrap();
    for version in 2..=34 {
        std::fs::write(
            directory.path().join(format!("{version}.root.json")),
            root(version, 17),
        )
        .unwrap();
    }
    let observed = ObservedFiles::default();
    let error = load(Arc::new(guard), directory.path(), observed.clone())
        .await
        .unwrap_err();
    assert!(has_admission_error(&error, |error| matches!(
        error,
        AdmissionError::Context
    )));
    assert_eq!(observed.0.lock().unwrap().last().unwrap(), "34.root.json");
    let state = backend.0.lock().unwrap();
    assert_eq!(state.state.current_root, root(33, 17));
    assert_eq!(state.root_chain.len(), 33);
    assert_eq!(state.state.reset_baseline, Some(root(1, 17)));
    assert!(state.state.metadata.is_empty());
    assert_eq!(state.state.accepted_time, None);
}

#[tokio::test]
async fn signed_sha512_and_exact_length_failures_prevent_child_retention() {
    for child in [MetadataRole::Snapshot, MetadataRole::Targets] {
        for fault in ["sha512", "length"] {
            let (backend, guard) = backend::setup();
            let directory = tempfile::tempdir().unwrap();
            let targets = package_targets();
            let mut snapshot = snapshot(&targets, 17);
            let mut timestamp = timestamp(&snapshot, 17);
            let (parent, name) = match child {
                MetadataRole::Snapshot => (&mut timestamp, "snapshot.json"),
                MetadataRole::Targets => (&mut snapshot, "targets.json"),
                _ => unreachable!(),
            };
            let mut body = signed_body(parent);
            let link = &mut body["meta"][name];
            match fault {
                "sha512" => link["hashes"]["sha512"] = json!("00".repeat(64)),
                "length" => link["length"] = json!(link["length"].as_u64().unwrap() + 1),
                _ => unreachable!(),
            }
            *parent = sign(body, &[(17, key(17))]);
            if child == MetadataRole::Targets {
                timestamp = fixtures::timestamp(&snapshot, 17);
            }
            write_view(directory.path(), &timestamp, &snapshot, &targets);
            let error = load(Arc::new(guard), directory.path(), ObservedFiles::default())
                .await
                .unwrap_err();
            assert!(
                has_admission_error(
                    &error,
                    |error| matches!(error, AdmissionError::Link(reason) if *reason == fault)
                ),
                "{child:?} {fault}: {error:?}"
            );
            let state = backend.0.lock().unwrap();
            assert!(!state.state.metadata.contains_key(&child));
            assert!(state.state.metadata.contains_key(&MetadataRole::Timestamp));
            assert_eq!(
                state.state.metadata.contains_key(&MetadataRole::Snapshot),
                child == MetadataRole::Targets
            );
            assert_eq!(state.state.accepted_time, None);
        }
    }
}

#[tokio::test]
async fn two_ids_for_one_raw_key_cannot_authorize_timestamp_through_loader() {
    let (backend, guard) = backend::setup();
    let directory = tempfile::tempdir().unwrap();
    let ordinary = key(17);
    let mut alias = ordinary.clone();
    alias["label"] = json!("same raw key, different key ID");
    assert_ne!(id(&ordinary), id(&alias));
    let signers = [(17, ordinary), (17, alias)];
    let mut next_root = root_body(2, &signers, 1);
    next_root["roles"]["timestamp"]["threshold"] = json!(2);
    let next_root = sign(next_root, &signers);
    std::fs::write(directory.path().join("2.root.json"), &next_root).unwrap();
    let targets = package_targets();
    let snapshot = snapshot(&targets, 17);
    let timestamp = sign(signed_body(&timestamp(&snapshot, 17)), &signers);
    write_view(directory.path(), &timestamp, &snapshot, &targets);
    let observed = ObservedFiles::default();
    let error = load(Arc::new(guard), directory.path(), observed.clone())
        .await
        .unwrap_err();
    assert!(
        has_admission_error(&error, |error| matches!(error, AdmissionError::Signature)),
        "{error:?}"
    );
    assert_eq!(
        *observed.0.lock().unwrap(),
        ["2.root.json", "3.root.json", "timestamp.json"]
    );
    let state = backend.0.lock().unwrap();
    assert_eq!(state.state.current_root, next_root);
    assert!(state.state.metadata.is_empty());
    assert_eq!(state.state.accepted_time, None);
}

#[tokio::test]
async fn equal_timestamp_discards_changed_link_even_when_candidate_has_expired() {
    for expired in [false, true] {
        let (backend, guard) = backend::setup();
        let guard = Arc::new(guard);
        let directory = tempfile::tempdir().unwrap();
        let targets = package_targets();
        let snapshot = snapshot(&targets, 17);
        let original_timestamp = timestamp(&snapshot, 17);
        write_view(directory.path(), &original_timestamp, &snapshot, &targets);
        assert!(matches!(
            load(guard.clone(), directory.path(), ObservedFiles::default())
                .await
                .unwrap(),
            PackageLoadOutcome::Updated(_)
        ));
        let before = backend.0.lock().unwrap().state.clone();
        let mut candidate = signed_body(&original_timestamp);
        candidate["meta"]["snapshot.json"] =
            json!({"version":99,"length":1,"hashes":{"sha256":"00".repeat(32)}});
        if expired {
            candidate["expires"] = json!("2020-01-01T00:00:00Z");
        }
        std::fs::write(
            directory.path().join("timestamp.json"),
            sign(candidate, &[(17, key(17))]),
        )
        .unwrap();
        std::fs::remove_file(directory.path().join("1.snapshot.json")).unwrap();
        std::fs::remove_file(directory.path().join("1.targets.json")).unwrap();
        let observed = ObservedFiles::default();
        assert!(matches!(
            load(guard, directory.path(), observed.clone())
                .await
                .unwrap(),
            PackageLoadOutcome::NoUpdate
        ));
        assert_eq!(
            *observed.0.lock().unwrap(),
            ["2.root.json", "timestamp.json"]
        );
        let state = backend.0.lock().unwrap();
        assert_eq!(state.state.revision, before.revision);
        for role in [
            MetadataRole::Timestamp,
            MetadataRole::Snapshot,
            MetadataRole::Targets,
        ] {
            assert_eq!(
                state.state.metadata[&role].bytes,
                before.metadata[&role].bytes
            );
            assert_eq!(
                state.state.metadata[&role].acceptance_root,
                before.metadata[&role].acceptance_root
            );
        }
        assert_eq!(state.state.accepted_time, None);
    }
}

fn write_versioned_view(directory: &std::path::Path, version: u64, snapshot_seed: u8) {
    let mut targets = signed_body(&package_targets());
    targets["version"] = json!(version);
    let targets = sign(targets, &[(17, key(17))]);
    let mut snapshot = signed_body(&fixtures::snapshot(&targets, snapshot_seed));
    snapshot["version"] = json!(version);
    snapshot["meta"]["targets.json"]["version"] = json!(version);
    let snapshot = sign(snapshot, &[(snapshot_seed, key(snapshot_seed))]);
    let mut timestamp = signed_body(&fixtures::timestamp(&snapshot, 17));
    timestamp["version"] = json!(version);
    timestamp["meta"]["snapshot.json"]["version"] = json!(version);
    let timestamp = sign(timestamp, &[(17, key(17))]);
    for (name, bytes) in [
        (format!("{version}.targets.json"), targets),
        (format!("{version}.snapshot.json"), snapshot),
        ("timestamp.json".into(), timestamp),
    ] {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}

#[tokio::test]
async fn snapshot_key_rotation_resets_both_floors_while_unchanged_keys_preserve_them() {
    for rotate in [true, false] {
        let (backend, guard) = backend::setup();
        let guard = Arc::new(guard);
        let directory = tempfile::tempdir().unwrap();
        write_versioned_view(directory.path(), 2, 17);
        assert!(matches!(
            load(guard.clone(), directory.path(), ObservedFiles::default())
                .await
                .unwrap(),
            PackageLoadOutcome::Updated(_)
        ));
        let original = backend.0.lock().unwrap().state.clone();
        let next_root = if rotate {
            let mut body = root_body(2, &[(17, key(17)), (18, key(18))], 1);
            for role in ["root", "timestamp", "targets"] {
                body["roles"][role]["keyids"] = json!([id(&key(17))]);
            }
            body["roles"]["snapshot"]["keyids"] = json!([id(&key(18))]);
            sign(body, &[(17, key(17))])
        } else {
            root(2, 17)
        };
        std::fs::write(directory.path().join("2.root.json"), &next_root).unwrap();
        // Stop after root-cycle persistence to inspect the exact atomic reset.
        std::fs::write(
            directory.path().join("timestamp.json"),
            b"invalid later metadata",
        )
        .unwrap();
        assert!(
            load(guard.clone(), directory.path(), ObservedFiles::default())
                .await
                .is_err()
        );
        {
            let state = backend.0.lock().unwrap();
            assert_eq!(state.state.current_root, next_root);
            assert!(state.state.reset_baseline.is_none());
            for role in [MetadataRole::Timestamp, MetadataRole::Snapshot] {
                assert_eq!(
                    state.state.metadata.contains_key(&role),
                    !rotate,
                    "{role:?} rotate={rotate}"
                );
            }
            assert_eq!(
                state.state.metadata[&MetadataRole::Targets].bytes,
                original.metadata[&MetadataRole::Targets].bytes
            );
        }
        write_versioned_view(directory.path(), 1, if rotate { 18 } else { 17 });
        let outcome = load(guard, directory.path(), ObservedFiles::default()).await;
        if rotate {
            assert!(matches!(outcome.unwrap(), PackageLoadOutcome::Updated(_)));
        } else {
            assert!(
                matches!(outcome,Err(MetadataLoadError::Update(error)) if matches!(*error,package_tough::error::Error::OlderMetadata{role:package_tough::schema::RoleType::Timestamp,..}))
            );
        }
        assert_eq!(backend.0.lock().unwrap().state.accepted_time, None);
    }
}
