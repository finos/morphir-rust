use morphir_common::cache_maintenance::{
    AutomaticCacheCleanupDecision, AutomaticCacheMaintenanceTransaction, CacheCleanupCursor,
    CacheExecutionLimits, CacheInventoryLimits, CacheMaintenanceState, CacheNamespace, CachePolicy,
    CleanupMode, automatic_cache_cleanup_decision, inventory_cache_namespace,
    load_cache_maintenance_state, plan_cache_cleanup, save_cache_maintenance_state,
};
use morphir_common::home::MorphirHome;
use std::time::Duration;

fn a_morphir_home() -> (tempfile::TempDir, MorphirHome) {
    let directory = tempfile::TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(directory.path().as_os_str()), None).unwrap();
    (directory, home)
}

#[test]
fn missing_state_is_due_and_uses_the_empty_state() {
    let (_directory, home) = a_morphir_home();
    let state = load_cache_maintenance_state(&home).unwrap();

    assert_eq!(state, CacheMaintenanceState::default());
    assert_eq!(
        automatic_cache_cleanup_decision(&state, 1_000, Duration::from_secs(86_400)).unwrap(),
        AutomaticCacheCleanupDecision::Due
    );
}

#[test]
fn completed_runs_are_interval_gated_even_when_the_clock_moves_back() {
    let state = CacheMaintenanceState::default().completed(10_000);

    assert_eq!(
        automatic_cache_cleanup_decision(&state, 9_000, Duration::from_secs(100)).unwrap(),
        AutomaticCacheCleanupDecision::Deferred { next_run: 10_100 }
    );
    assert_eq!(
        automatic_cache_cleanup_decision(&state, 10_100, Duration::from_secs(100)).unwrap(),
        AutomaticCacheCleanupDecision::Due
    );
}

#[test]
fn continuation_state_round_trips_and_remains_due() {
    let (_directory, home) = a_morphir_home();
    let cursor = CacheCleanupCursor::new("downloads", "desktop/1.2.3").unwrap();
    let state = CacheMaintenanceState::default().continued(cursor.clone());

    save_cache_maintenance_state(&home, &state).unwrap();
    let loaded = load_cache_maintenance_state(&home).unwrap();

    assert_eq!(loaded, state);
    assert_eq!(loaded.continuation(), Some(&cursor));
    assert_eq!(
        automatic_cache_cleanup_decision(&loaded, 1, Duration::from_secs(86_400)).unwrap(),
        AutomaticCacheCleanupDecision::Due
    );

    let completed = loaded.completed(2_000);
    save_cache_maintenance_state(&home, &completed).unwrap();
    assert_eq!(
        load_cache_maintenance_state(&home).unwrap(),
        completed,
        "an existing state file must be atomically replaceable"
    );
}

#[test]
fn malformed_state_fails_closed_instead_of_resetting_the_schedule() {
    let (_directory, home) = a_morphir_home();
    let path = home.cache_maintenance_state_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, br#"{"schemaVersion":99}"#).unwrap();

    let error = load_cache_maintenance_state(&home).unwrap_err();

    assert!(error.to_string().contains("schema version"));
}

#[test]
fn link_like_data_directory_is_rejected_when_state_is_missing() {
    let (directory, home) = a_morphir_home();
    let target = directory.path().join("outside-data");
    std::fs::create_dir(&target).unwrap();
    if !create_directory_link(&target, &home.data_dir()) {
        return;
    }

    let error = load_cache_maintenance_state(&home).unwrap_err();

    assert!(error.to_string().contains("unsafe"));
}

#[test]
fn link_like_maintenance_directory_is_rejected_when_state_is_missing() {
    let (directory, home) = a_morphir_home();
    let target = directory.path().join("outside-maintenance");
    let maintenance = home.data_dir().join("maintenance");
    std::fs::create_dir(&target).unwrap();
    std::fs::create_dir(home.data_dir()).unwrap();
    if !create_directory_link(&target, &maintenance) {
        return;
    }

    let error = load_cache_maintenance_state(&home).unwrap_err();

    assert!(error.to_string().contains("unsafe"));
}

#[test]
fn oversized_state_is_rejected_at_the_read_boundary() {
    let (_directory, home) = a_morphir_home();
    let path = home.cache_maintenance_state_file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();

    let error = load_cache_maintenance_state(&home).unwrap_err();

    assert!(error.to_string().contains("65536-byte limit"));
}

#[test]
fn concurrent_first_saves_share_directory_initialization() {
    let (_directory, home) = a_morphir_home();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let saves = (0..16)
        .map(|timestamp| {
            let home = home.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                save_cache_maintenance_state(
                    &home,
                    &CacheMaintenanceState::default().completed(timestamp),
                )
            })
        })
        .collect::<Vec<_>>();

    for save in saves {
        save.join().unwrap().unwrap();
    }
    assert!(load_cache_maintenance_state(&home).is_ok());
}

#[test]
fn automatic_transaction_holds_coordination_across_load_execution_and_save() {
    let (_directory, home) = a_morphir_home();
    let cache_file = home.cache_dir().join("downloads/old.pkg");
    std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
    std::fs::write(&cache_file, b"old").unwrap();
    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("old.pkg", 1)
        .unwrap();
    let inventory =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();
    let plan = plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::from_secs(1), 0),
        10,
        CleanupMode::All,
    )
    .unwrap();
    let transaction = AutomaticCacheMaintenanceTransaction::begin(&home).unwrap();
    let completed = transaction.state().clone().completed(10);

    let report = transaction
        .execute_cleanup(
            &plan,
            std::slice::from_ref(&namespace),
            CacheInventoryLimits::default(),
            CacheExecutionLimits::new(1, 1024).unwrap(),
        )
        .unwrap();
    transaction.finish(completed).unwrap();

    assert_eq!(report.removed_bytes(), 3);
    assert!(!cache_file.exists());
    assert_eq!(
        load_cache_maintenance_state(&home)
            .unwrap()
            .last_successful_automatic_run(),
        Some(10)
    );
}

#[test]
fn zero_automatic_interval_is_rejected() {
    let error =
        automatic_cache_cleanup_decision(&CacheMaintenanceState::default(), 1, Duration::ZERO)
            .unwrap_err();

    assert!(error.to_string().contains("nonzero"));
}

#[test]
fn subsecond_automatic_interval_is_rejected() {
    let error = automatic_cache_cleanup_decision(
        &CacheMaintenanceState::default(),
        1,
        Duration::from_millis(500),
    )
    .unwrap_err();

    assert!(error.to_string().contains("nonzero"));
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
