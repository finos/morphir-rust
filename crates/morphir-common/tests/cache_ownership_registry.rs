use morphir_common::cache_maintenance::{
    CacheEntryState, CacheExecutionDisposition, CacheExecutionLimits, CacheInventoryLimits,
    CacheMaintenanceSession, CacheOwnershipMutationGuard, CachePolicy, CleanupMode,
    load_cache_ownership_registry, plan_cache_cleanup,
};
use morphir_common::home::MorphirHome;
use std::time::Duration;

fn a_morphir_home() -> (tempfile::TempDir, MorphirHome) {
    let directory = tempfile::TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(directory.path().as_os_str()), None).unwrap();
    (directory, home)
}

#[test]
fn producers_can_register_refresh_and_release_durable_ownership() {
    let (_directory, home) = a_morphir_home();
    assert!(load_cache_ownership_registry(&home).unwrap().is_empty());

    register(&home, "downloads", "desktop/1.2.3.pkg", 10);
    register(&home, "downloads", "desktop/1.2.3.pkg", 20);
    register(&home, "indexes", "releases.json", 30);

    let registry = load_cache_ownership_registry(&home).unwrap();
    assert_eq!(registry.len(), 2);
    let json = std::fs::read_to_string(home.cache_ownership_registry_file()).unwrap();
    assert!(json.contains("\"lastUsed\": 20"));

    assert!(release(&home, "downloads", "desktop/1.2.3.pkg"));
    assert!(!release(&home, "downloads", "desktop/1.2.3.pkg"));
    assert_eq!(load_cache_ownership_registry(&home).unwrap().len(), 1);
}

#[test]
fn mutation_begin_invalidates_prior_ownership_before_content_is_writable() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let artifact = home.downloads_cache_dir().join("interrupted.pkg");
    std::fs::write(&artifact, b"old").unwrap();
    register(&home, "downloads", "interrupted.pkg", 1);

    let mutation =
        CacheOwnershipMutationGuard::begin(&home, "downloads", "interrupted.pkg").unwrap();
    let json = std::fs::read_to_string(home.cache_ownership_registry_file()).unwrap();
    assert!(!json.contains("interrupted.pkg"));
    std::fs::write(&artifact, b"new but interrupted").unwrap();
    drop(mutation);

    let session = CacheMaintenanceSession::begin(&home).unwrap();
    let inventory = session
        .inventory(&["downloads"], CacheInventoryLimits::default())
        .unwrap();
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].state(), CacheEntryState::Unclassified);
}

#[test]
fn a_guarded_session_cleans_registered_content_and_preserves_unknown_files() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let owned = home.downloads_cache_dir().join("owned.pkg");
    let unknown = home.downloads_cache_dir().join("unknown.pkg");
    std::fs::write(&owned, b"owned").unwrap();
    std::fs::write(&unknown, b"unknown").unwrap();
    register(&home, "downloads", "owned.pkg", 1);

    let session = CacheMaintenanceSession::begin(&home).unwrap();
    let inventory = session
        .inventory(&["downloads"], CacheInventoryLimits::default())
        .unwrap();
    assert_eq!(inventory.len(), 2);
    assert!(inventory.iter().any(|entry| {
        entry.path() == "owned.pkg"
            && matches!(entry.state(), CacheEntryState::Disposable { last_used: 1 })
    }));
    assert!(inventory.iter().any(|entry| {
        entry.path() == "unknown.pkg" && matches!(entry.state(), CacheEntryState::Unclassified)
    }));

    let plan = plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::from_secs(1), 0),
        10,
        CleanupMode::All,
    )
    .unwrap();
    let report = session
        .execute_cleanup(
            &plan,
            CacheInventoryLimits::default(),
            CacheExecutionLimits::new(10, 1_024).unwrap(),
        )
        .unwrap();

    assert_eq!(report.removed_bytes(), 5);
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Removed
    );
    assert!(!owned.exists());
    assert!(unknown.exists());
}

#[test]
fn concurrent_mutation_handoffs_do_not_lose_registry_updates() {
    let (_directory, home) = a_morphir_home();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let registrations = (0..16)
        .map(|index| {
            let home = home.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                CacheOwnershipMutationGuard::begin(&home, "downloads", format!("{index}.pkg"))
                    .unwrap()
                    .finish(index)
            })
        })
        .collect::<Vec<_>>();

    for registration in registrations {
        registration.join().unwrap().unwrap();
    }
    assert_eq!(load_cache_ownership_registry(&home).unwrap().len(), 16);
}

#[test]
fn refresh_handoff_precedes_a_waiting_cleanup_session() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let artifact = home.downloads_cache_dir().join("recent.pkg");
    std::fs::write(&artifact, b"recent").unwrap();
    register(&home, "downloads", "recent.pkg", 1);

    let mutation = CacheOwnershipMutationGuard::begin(&home, "downloads", "recent.pkg").unwrap();
    let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
    let cleanup = {
        let home = home.clone();
        let ready = ready.clone();
        std::thread::spawn(move || {
            ready.wait();
            let session = CacheMaintenanceSession::begin(&home).unwrap();
            let inventory = session
                .inventory(&["downloads"], CacheInventoryLimits::default())
                .unwrap();
            let plan = plan_cache_cleanup(
                inventory,
                CachePolicy::new(Duration::from_secs(10), u64::MAX),
                100,
                CleanupMode::Policy,
            )
            .unwrap();
            session
                .execute_cleanup(
                    &plan,
                    CacheInventoryLimits::default(),
                    CacheExecutionLimits::new(10, 1024).unwrap(),
                )
                .unwrap()
        })
    };
    ready.wait();
    mutation.finish(100).unwrap();

    let report = cleanup.join().unwrap();
    assert!(report.items().is_empty());
    assert!(artifact.exists());
}

#[test]
fn failed_handoff_returns_a_lease_that_keeps_cleanup_excluded() {
    let (_directory, home) = a_morphir_home();
    let registry_path = home.cache_ownership_registry_file();
    let mutation = CacheOwnershipMutationGuard::begin(&home, "downloads", "recent.pkg").unwrap();
    std::fs::create_dir_all(&registry_path).unwrap();

    let failure = mutation.finish(100).unwrap_err();
    let (retained_mutation, source) = failure.into_parts();
    assert!(source.to_string().contains("unsafe"));

    let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let cleanup = {
        let home = home.clone();
        let ready = ready.clone();
        std::thread::spawn(move || {
            ready.wait();
            let failed_closed = CacheMaintenanceSession::begin(&home).is_err();
            finished_tx.send(failed_closed).unwrap();
        })
    };
    ready.wait();
    assert!(matches!(
        finished_rx.recv_timeout(Duration::from_millis(200)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    drop(retained_mutation);
    assert!(finished_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    cleanup.join().unwrap();
}

#[test]
fn selected_cleanup_is_not_blocked_by_an_unrelated_invalid_namespace() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let artifact = home.downloads_cache_dir().join("old.pkg");
    std::fs::write(&artifact, b"old").unwrap();
    register(&home, "downloads", "old.pkg", 1);
    register(&home, "indexes", "catalog.json", 1);
    std::fs::write(home.indexes_cache_dir(), b"not a namespace directory").unwrap();

    let session = CacheMaintenanceSession::begin(&home).unwrap();
    let inventory = session
        .inventory(&["downloads"], CacheInventoryLimits::default())
        .unwrap();
    let plan = plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::from_secs(1), 0),
        10,
        CleanupMode::All,
    )
    .unwrap();
    let report = session
        .execute_cleanup(
            &plan,
            CacheInventoryLimits::default(),
            CacheExecutionLimits::new(10, 1024).unwrap(),
        )
        .unwrap();

    assert_eq!(report.removed_bytes(), 3);
    assert!(!artifact.exists());
    assert!(home.indexes_cache_dir().is_file());
}

#[test]
fn cleanup_does_not_revalidate_a_namespace_beyond_the_removal_budget() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::create_dir_all(home.indexes_cache_dir()).unwrap();
    let first = home.downloads_cache_dir().join("first.pkg");
    let deferred = home.indexes_cache_dir().join("deferred.json");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&deferred, b"deferred").unwrap();
    register(&home, "downloads", "first.pkg", 1);
    register(&home, "indexes", "deferred.json", 1);

    let session = CacheMaintenanceSession::begin(&home).unwrap();
    let inventory = session
        .inventory(&["downloads", "indexes"], CacheInventoryLimits::default())
        .unwrap();
    let plan = plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::from_secs(1), 0),
        10,
        CleanupMode::All,
    )
    .unwrap();
    std::fs::remove_dir_all(home.indexes_cache_dir()).unwrap();
    std::fs::write(home.indexes_cache_dir(), b"invalid deferred namespace").unwrap();

    let report = session
        .execute_cleanup(
            &plan,
            CacheInventoryLimits::default(),
            CacheExecutionLimits::new(1, 1024).unwrap(),
        )
        .unwrap();

    assert_eq!(report.removed_bytes(), 5);
    assert_eq!(report.items().len(), 2);
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Removed
    );
    assert_eq!(
        report.items()[1].disposition(),
        CacheExecutionDisposition::DeferredLimit
    );
    assert!(!first.exists());
    assert!(home.indexes_cache_dir().is_file());
}

#[test]
fn cleanup_compacts_removed_and_missing_ownership_entries() {
    let (_directory, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let removable = home.downloads_cache_dir().join("removable.pkg");
    let missing = home.downloads_cache_dir().join("missing.pkg");
    std::fs::write(&removable, b"remove").unwrap();
    std::fs::write(&missing, b"disappear").unwrap();
    register(&home, "downloads", "removable.pkg", 1);
    register(&home, "downloads", "missing.pkg", 1);

    let session = CacheMaintenanceSession::begin(&home).unwrap();
    let inventory = session
        .inventory(&["downloads"], CacheInventoryLimits::default())
        .unwrap();
    let plan = plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::from_secs(1), 0),
        10,
        CleanupMode::All,
    )
    .unwrap();
    std::fs::remove_file(&missing).unwrap();

    let report = session
        .execute_cleanup(
            &plan,
            CacheInventoryLimits::default(),
            CacheExecutionLimits::new(10, 1024).unwrap(),
        )
        .unwrap();
    drop(session);

    assert!(report.items().iter().any(|item| {
        item.path() == "removable.pkg" && item.disposition() == CacheExecutionDisposition::Removed
    }));
    assert!(report.items().iter().any(|item| {
        item.path() == "missing.pkg" && item.disposition() == CacheExecutionDisposition::Missing
    }));
    assert!(load_cache_ownership_registry(&home).unwrap().is_empty());
}

#[test]
fn malformed_and_oversized_registries_fail_closed() {
    let (_directory, home) = a_morphir_home();
    let path = home.cache_ownership_registry_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, br#"{"schemaVersion":99,"entries":[]}"#).unwrap();
    assert!(
        load_cache_ownership_registry(&home)
            .unwrap_err()
            .to_string()
            .contains("schema version")
    );

    std::fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
    assert!(
        load_cache_ownership_registry(&home)
            .unwrap_err()
            .to_string()
            .contains("65536-byte limit")
    );
}

fn register(home: &MorphirHome, namespace: &str, path: &str, last_used: u64) {
    CacheOwnershipMutationGuard::begin(home, namespace, path)
        .unwrap()
        .finish(last_used)
        .unwrap();
}

fn release(home: &MorphirHome, namespace: &str, path: &str) -> bool {
    CacheOwnershipMutationGuard::begin(home, namespace, path)
        .unwrap()
        .finish_unowned()
}
