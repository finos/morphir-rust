#[path = "local_registry/tuf_backend.rs"]
mod backend;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod fixtures;
use backend::setup;
use fixtures::*;
use morphir_package::local_registry::{tuf::*, *};
use package_tough::experimental_storage::{
    Admission, MetadataRole, RetainedMetadata, Revision, Storage, Transition,
};
use std::sync::Arc;

#[tokio::test]
async fn candidate_requires_durable_exact_evidence_and_current_predecessor() {
    let (backend, guard) = setup();
    let predecessor = guard.snapshot().await.unwrap();
    let candidate = RetainedMetadata {
        bytes: targets(1, 17),
        acceptance_root: root(1, 17),
    };
    let transition = Transition::Retain {
        role: MetadataRole::Targets,
        metadata: candidate.clone(),
    };
    assert!(guard.transition(&predecessor, &transition).await.is_err());
    guard
        .record(TufRole::Targets, &candidate.bytes)
        .await
        .unwrap();
    // Targets also require their authenticated snapshot link at admission.
    assert!(guard.transition(&predecessor, &transition).await.is_err());
    backend.0.lock().unwrap().state.metadata.insert(
        MetadataRole::Snapshot,
        RetainedMetadata {
            bytes: snapshot(&candidate.bytes, 17),
            acceptance_root: root(1, 17),
        },
    );
    let predecessor = guard.snapshot().await.unwrap();
    guard.transition(&predecessor, &transition).await.unwrap();
    backend.0.lock().unwrap().state.revision = Revision(2);
    assert!(guard.transition(&predecessor, &transition).await.is_err());
}
#[tokio::test]
async fn unrelated_marker_clock_or_acceptance_root_cannot_enter_tough() {
    let (backend, guard) = setup();
    let original = backend.0.lock().unwrap().clone();
    backend.0.lock().unwrap().binding.id = OperationId::new([24; 32]);
    assert!(guard.snapshot().await.is_err());
    *backend.0.lock().unwrap() = original.clone();
    backend.0.lock().unwrap().binding.fixed_time = "2030-01-02T00:00:00Z".parse().unwrap();
    assert!(guard.snapshot().await.is_err());
    *backend.0.lock().unwrap() = original;
    backend.0.lock().unwrap().state.metadata.insert(
        MetadataRole::Targets,
        RetainedMetadata {
            bytes: targets(1, 18),
            acceptance_root: root(1, 18),
        },
    );
    assert!(guard.snapshot().await.is_err());
}
#[tokio::test]
async fn roots_persist_before_later_candidate_failure_and_rotate_at_most_32_times() {
    let (backend, guard) = setup();
    for version in 2..=33 {
        let bytes = root(version, 17);
        guard.record(TufRole::Root, &bytes).await.unwrap();
        let predecessor = guard.snapshot().await.unwrap();
        guard
            .commit(
                predecessor.revision,
                &Transition::AdvanceRoot {
                    root: bytes,
                    baseline: root(1, 17),
                },
            )
            .await
            .unwrap();
    }
    let bytes = root(34, 17);
    guard.record(TufRole::Root, &bytes).await.unwrap();
    let before = guard.snapshot().await.unwrap();
    assert!(
        guard
            .commit(
                before.revision,
                &Transition::AdvanceRoot {
                    root: bytes,
                    baseline: root(1, 17)
                }
            )
            .await
            .is_err()
    );
    assert!(
        guard
            .record(TufRole::Timestamp, b"invalid later candidate")
            .await
            .is_err()
    );
    assert_eq!(backend.0.lock().unwrap().state.current_root, root(33, 17));
}

#[tokio::test]
async fn composed_loader_admits_complete_signed_view_and_preserves_no_update() {
    let (backend, guard) = setup();
    let guard = Arc::new(guard);
    let directory = tempfile::tempdir().unwrap();
    let targets = targets(1, 17);
    let snapshot = snapshot(&targets, 17);
    let timestamp = timestamp(&snapshot, 17);
    for (name, bytes) in [
        ("1.targets.json", &targets),
        ("1.snapshot.json", &snapshot),
        ("timestamp.json", &timestamp),
    ] {
        std::fs::write(directory.path().join(name), bytes).unwrap();
    }
    let base = url::Url::from_directory_path(directory.path()).unwrap();
    let outcome = guard
        .clone()
        .load_metadata(Box::new(package_tough::FilesystemTransport), base.clone())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        package_tough::experimental_storage::PackageLoadOutcome::Updated(_)
    ));
    let before = backend.0.lock().unwrap().state.clone();
    assert_eq!(before.metadata.len(), 3);
    assert_eq!(before.accepted_time, None);
    // Equality must terminate before fetching a now-missing snapshot or targets.
    std::fs::remove_file(directory.path().join("1.snapshot.json")).unwrap();
    std::fs::remove_file(directory.path().join("1.targets.json")).unwrap();
    let outcome = guard
        .load_metadata(Box::new(package_tough::FilesystemTransport), base)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        package_tough::experimental_storage::PackageLoadOutcome::NoUpdate
    ));
    let after = &backend.0.lock().unwrap().state;
    // FinishRootCycle may commit the upstream no-reset step, but equality never
    // overwrites retained timestamp/complete-view bytes or authorization time.
    assert_eq!(
        before.metadata[&MetadataRole::Timestamp].bytes,
        after.metadata[&MetadataRole::Timestamp].bytes
    );
    assert_eq!(after.accepted_time, None);
}
#[tokio::test]
async fn composed_loader_keeps_accepted_roots_after_invalid_later_timestamp() {
    let (backend, guard) = setup();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("2.root.json"), root(2, 17)).unwrap();
    std::fs::write(directory.path().join("timestamp.json"), b"invalid").unwrap();
    let base = url::Url::from_directory_path(directory.path()).unwrap();
    assert!(
        Arc::new(guard)
            .load_metadata(Box::new(package_tough::FilesystemTransport), base)
            .await
            .is_err()
    );
    assert_eq!(backend.0.lock().unwrap().state.current_root, root(2, 17));
    assert_eq!(backend.0.lock().unwrap().state.accepted_time, None);
}
#[tokio::test]
async fn backend_error_invalidates_session_even_when_rows_are_visible() {
    for fault in [1, 2] {
        let (backend, guard) = setup();
        let bytes = root(2, 17);
        if fault == 2 {
            guard.record(TufRole::Root, &bytes).await.unwrap();
        }
        backend.1.store(fault, std::sync::atomic::Ordering::SeqCst);
        if fault == 1 {
            assert!(guard.record(TufRole::Root, &bytes).await.is_err());
        } else {
            assert!(
                guard
                    .commit(
                        Revision(1),
                        &Transition::AdvanceRoot {
                            root: bytes.clone(),
                            baseline: root(1, 17)
                        }
                    )
                    .await
                    .is_err()
            );
        }
        // Like a final SQLite sync failure: the new rows are visible, but that
        // does not authorize this failed operation to resume when I/O recovers.
        backend.1.store(0, std::sync::atomic::Ordering::SeqCst);
        assert!(!backend.0.lock().unwrap().evidence.is_empty());
        assert!(guard.snapshot().await.is_err());
        assert!(guard.record(TufRole::Root, &bytes).await.is_err());
    }
}

#[tokio::test]
async fn shared_operation_budget_includes_other_metadata_and_allows_exact_retry() {
    use futures::TryStreamExt;
    use package_tough::Transport;
    for extra in [0, 1] {
        let (backend, guard) = setup();
        let bytes = targets(1, 17);
        backend.0.lock().unwrap().other_metadata_bytes =
            268_435_456 - root(1, 17).len() - bytes.len() + extra;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("1.targets.json"), &bytes).unwrap();
        let base = url::Url::from_directory_path(directory.path()).unwrap();
        let transport = ProfileTransport::new(
            Box::new(package_tough::FilesystemTransport),
            Arc::new(guard),
            base.clone(),
        )
        .unwrap();
        for _ in 0..2 {
            let result = transport
                .fetch(base.join("1.targets.json").unwrap())
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await;
            assert_eq!(result.is_ok(), extra == 0);
        }
        assert_eq!(backend.0.lock().unwrap().evidence.is_empty(), extra == 1);
    }
}
