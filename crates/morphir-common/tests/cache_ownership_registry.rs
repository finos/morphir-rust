use morphir_common::cache_maintenance::{
    CacheEntryState, CacheExecutionDisposition, CacheExecutionLimits, CacheInventoryLimits,
    CacheMaintenanceSession, CachePolicy, CleanupMode, load_cache_ownership_registry,
    plan_cache_cleanup, register_cache_ownership, unregister_cache_ownership,
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

    register_cache_ownership(&home, "downloads", "desktop/1.2.3.pkg", 10).unwrap();
    register_cache_ownership(&home, "downloads", "desktop/1.2.3.pkg", 20).unwrap();
    register_cache_ownership(&home, "indexes", "releases.json", 30).unwrap();

    let registry = load_cache_ownership_registry(&home).unwrap();
    assert_eq!(registry.len(), 2);
    let json = std::fs::read_to_string(home.cache_ownership_registry_file()).unwrap();
    assert!(json.contains("\"lastUsed\": 20"));

    assert!(unregister_cache_ownership(&home, "downloads", "desktop/1.2.3.pkg").unwrap());
    assert!(!unregister_cache_ownership(&home, "downloads", "desktop/1.2.3.pkg").unwrap());
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
    register_cache_ownership(&home, "downloads", "owned.pkg", 1).unwrap();

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
