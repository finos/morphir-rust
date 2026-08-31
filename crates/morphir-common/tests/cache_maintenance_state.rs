use morphir_common::cache_maintenance::{
    AutomaticCacheCleanupDecision, CacheCleanupCursor, CacheMaintenanceState,
    automatic_cache_cleanup_decision, load_cache_maintenance_state, save_cache_maintenance_state,
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
