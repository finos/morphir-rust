use morphir_common::cache_maintenance::{
    CacheEntryState, CacheExecutionDisposition, CacheExecutionLimits, CacheInventoryLimits,
    CacheMaintenanceSession, CacheMutationGuard, CachePolicy, CleanupMode,
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
                let mutation = CacheMutationGuard::acquire(&home).unwrap();
                barrier.wait();
                mutation.finish_with_ownership("downloads", format!("{index}.pkg"), index)
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

    let mutation = CacheMutationGuard::acquire(&home).unwrap();
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
    mutation
        .finish_with_ownership("downloads", "recent.pkg", 100)
        .unwrap();

    let report = cleanup.join().unwrap();
    assert!(report.items().is_empty());
    assert!(artifact.exists());
}

#[test]
fn failed_handoff_returns_a_lease_that_keeps_cleanup_excluded() {
    let (_directory, home) = a_morphir_home();
    let registry_path = home.cache_ownership_registry_file();
    std::fs::create_dir_all(&registry_path).unwrap();
    let mutation = CacheMutationGuard::acquire(&home).unwrap();

    let failure = mutation
        .finish_with_ownership("downloads", "recent.pkg", 100)
        .unwrap_err();
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
    CacheMutationGuard::acquire(home)
        .unwrap()
        .finish_with_ownership(namespace, path, last_used)
        .unwrap();
}

fn release(home: &MorphirHome, namespace: &str, path: &str) -> bool {
    CacheMutationGuard::acquire(home)
        .unwrap()
        .finish_releasing_ownership(namespace, path)
        .unwrap()
}
