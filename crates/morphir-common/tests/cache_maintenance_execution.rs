use morphir_common::cache_maintenance::{
    CacheEntry, CacheExecutionDisposition, CacheExecutionLimits, CacheInventoryLimits,
    CacheNamespace, CachePolicy, CleanupMode, execute_cache_cleanup, plan_cache_cleanup,
};
use morphir_common::home::MorphirHome;
use std::time::Duration;
use tempfile::TempDir;

fn a_morphir_home() -> (TempDir, MorphirHome) {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    (root, home)
}

fn a_policy_plan(entries: Vec<CacheEntry>) -> morphir_common::cache_maintenance::CleanupPlan {
    plan_cache_cleanup(
        entries,
        CachePolicy::new(Duration::from_secs(5), u64::MAX),
        10,
        CleanupMode::Policy,
    )
    .unwrap()
}

#[test]
fn execution_removes_only_planner_selected_owned_entries() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("old.pkg"), b"old").unwrap();
    std::fs::write(home.downloads_cache_dir().join("recent.pkg"), b"recent").unwrap();
    std::fs::write(home.downloads_cache_dir().join("leased.pkg"), b"leased").unwrap();
    std::fs::write(home.downloads_cache_dir().join("unknown.pkg"), b"unknown").unwrap();

    let plan = a_policy_plan(vec![
        CacheEntry::disposable("downloads", "old.pkg", 3, 0).unwrap(),
        CacheEntry::disposable("downloads", "recent.pkg", 6, 10).unwrap(),
        CacheEntry::leased("downloads", "leased.pkg", 6, 0).unwrap(),
        CacheEntry::unclassified("downloads", "unknown.pkg", 7).unwrap(),
    ]);
    let ownership = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("old.pkg", 0)
        .unwrap();
    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[ownership],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(!home.downloads_cache_dir().join("old.pkg").exists());
    for retained in ["recent.pkg", "leased.pkg", "unknown.pkg"] {
        assert!(home.downloads_cache_dir().join(retained).exists());
    }
    assert_eq!(report.removed_bytes(), 3);
    assert_eq!(report.items().len(), 1);
    assert_eq!(report.items()[0].path(), "old.pkg");
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Removed
    );
}

#[test]
fn execution_refuses_an_entry_that_changed_after_planning() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.indexes_cache_dir()).unwrap();
    let target = home.indexes_cache_dir().join("old.index");
    std::fs::write(&target, b"old").unwrap();
    let plan = a_policy_plan(vec![
        CacheEntry::disposable("indexes", "old.index", 3, 0).unwrap(),
    ]);
    std::fs::write(&target, b"changed").unwrap();

    let ownership = CacheNamespace::new("indexes")
        .unwrap()
        .with_disposable("old.index", 0)
        .unwrap();
    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[ownership],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(target.exists());
    assert_eq!(report.removed_bytes(), 0);
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Stale
    );
}

#[test]
fn execution_stops_at_a_deterministic_entry_budget() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.desktop_cache_dir()).unwrap();
    std::fs::write(home.desktop_cache_dir().join("a.cache"), b"a").unwrap();
    std::fs::write(home.desktop_cache_dir().join("b.cache"), b"b").unwrap();
    let plan = a_policy_plan(vec![
        CacheEntry::disposable("desktop", "b.cache", 1, 0).unwrap(),
        CacheEntry::disposable("desktop", "a.cache", 1, 0).unwrap(),
    ]);

    let ownership = CacheNamespace::new("desktop")
        .unwrap()
        .with_disposable("a.cache", 0)
        .unwrap()
        .with_disposable("b.cache", 0)
        .unwrap();
    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[ownership],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(1, 1_024).unwrap(),
    )
    .unwrap();

    assert!(!home.desktop_cache_dir().join("a.cache").exists());
    assert!(home.desktop_cache_dir().join("b.cache").exists());
    assert_eq!(report.items()[0].path(), "a.cache");
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Removed
    );
    assert_eq!(report.items()[1].path(), "b.cache");
    assert_eq!(
        report.items()[1].disposition(),
        CacheExecutionDisposition::DeferredLimit
    );
}

#[test]
fn execution_does_not_follow_a_link_added_beneath_an_owned_directory() {
    let (_root, home) = a_morphir_home();
    let outside = TempDir::new().unwrap();
    let owned = home.downloads_cache_dir().join("owned");
    std::fs::create_dir_all(&owned).unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
    if create_file_link(&outside.path().join("keep.txt"), &owned.join("link")).is_err() {
        return;
    }
    let plan = a_policy_plan(vec![
        CacheEntry::disposable("downloads", "owned", 4, 0).unwrap(),
    ]);

    let ownership = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("owned", 0)
        .unwrap();
    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[ownership],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(owned.exists());
    assert_eq!(
        std::fs::read(outside.path().join("keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Unclassified
    );
}

#[test]
fn execution_honors_a_lease_acquired_after_the_plan_was_created() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let target = home.downloads_cache_dir().join("became-leased.pkg");
    std::fs::write(&target, b"leased").unwrap();
    let plan = a_policy_plan(vec![
        CacheEntry::disposable("downloads", "became-leased.pkg", 6, 0).unwrap(),
    ]);
    let ownership = CacheNamespace::new("downloads")
        .unwrap()
        .with_lease("became-leased.pkg", 10)
        .unwrap();

    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[ownership],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(target.exists());
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::ActiveLease
    );
}

#[test]
fn execution_refuses_a_selected_entry_without_current_ownership() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.indexes_cache_dir()).unwrap();
    let target = home.indexes_cache_dir().join("unregistered.index");
    std::fs::write(&target, b"index").unwrap();
    let plan = a_policy_plan(vec![
        CacheEntry::disposable("indexes", "unregistered.index", 5, 0).unwrap(),
    ]);

    let report = execute_cache_cleanup(
        &home,
        &plan,
        &[],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(target.exists());
    assert_eq!(
        report.items()[0].disposition(),
        CacheExecutionDisposition::Unregistered
    );
}

#[test]
fn execution_refuses_a_link_like_maintenance_lock() {
    let (_root, home) = a_morphir_home();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(home.locks_dir()).unwrap();
    let outside_lock = outside.path().join("outside.lock");
    std::fs::write(&outside_lock, b"outside").unwrap();
    if create_file_link(&outside_lock, &home.maintenance_lock_file()).is_err() {
        return;
    }

    let error = execute_cache_cleanup(
        &home,
        &a_policy_plan(Vec::new()),
        &[],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("unsafe maintenance path"));
}

#[test]
fn execution_recovers_content_from_an_interrupted_trash_run() {
    let (_root, home) = a_morphir_home();
    let stranded_run = home
        .maintenance_trash_dir()
        .join("0123456789abcdef0123456789abcdef");
    std::fs::create_dir_all(stranded_run.join("nested")).unwrap();
    std::fs::write(stranded_run.join("nested/entry"), b"stranded").unwrap();

    execute_cache_cleanup(
        &home,
        &a_policy_plan(Vec::new()),
        &[],
        CacheInventoryLimits::default(),
        CacheExecutionLimits::new(10, 1_024).unwrap(),
    )
    .unwrap();

    assert!(!stranded_run.exists());
}

#[cfg(unix)]
fn create_file_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
