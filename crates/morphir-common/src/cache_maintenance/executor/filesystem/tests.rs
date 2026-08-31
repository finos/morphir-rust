#[cfg(unix)]
use super::{
    CacheExecutionError, RemovalOutcome, RemovalTarget, create_trash_run, open_maintenance_trash,
    remove_revalidated_entry, remove_revalidated_entry_with_hook, sweep_existing_trash,
};
use super::{MaintenanceGuard, open_removal_parent};
#[cfg(unix)]
use super::{observe_tree, pin_object};
use crate::home::MorphirHome;
use tempfile::TempDir;

#[test]
fn durable_home_creation_syncs_new_entries_from_leaf_to_existing_ancestor() {
    let existing = TempDir::new().unwrap();
    let home = existing.path().join("suite").join("home");
    let mut synced = Vec::new();

    super::create_directory_tree_durably_with(&home, |parent| {
        synced.push(parent.to_path_buf());
        Ok(())
    })
    .unwrap();

    assert!(home.is_dir());
    assert_eq!(
        synced,
        vec![existing.path().join("suite"), existing.path().to_path_buf()]
    );
}

#[test]
fn relative_home_creation_stops_before_empty_path_and_syncs_current_directory() {
    let relative_home = std::path::Path::new("morphir-home");

    assert_eq!(super::nonempty_parent(relative_home), None);
    assert_eq!(
        super::directory_entry_parent(relative_home),
        Some(std::path::Path::new("."))
    );
}

#[test]
fn removal_refuses_a_link_like_source_ancestor() {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(outside.path().join("keep"), b"outside").unwrap();
    if create_directory_link(outside.path(), &home.downloads_cache_dir().join("owned")).is_err() {
        return;
    }

    let guard = MaintenanceGuard::acquire(&home).unwrap();
    assert!(open_removal_parent(&home, guard.home_dir(), "downloads", "owned/keep").is_err());
    assert_eq!(
        std::fs::read(outside.path().join("keep")).unwrap(),
        b"outside"
    );
}

#[cfg(unix)]
#[test]
fn removal_preserves_a_leaf_replaced_after_it_was_pinned() {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let target = home.downloads_cache_dir().join("owned.pkg");
    std::fs::write(&target, b"planned").unwrap();
    let guard = MaintenanceGuard::acquire(&home).unwrap();
    let (parent, leaf, source) =
        open_removal_parent(&home, guard.home_dir(), "downloads", "owned.pkg").unwrap();
    let expected = pin_object(&parent, &leaf, &source).unwrap().handle;
    let expected_fingerprint = observe_tree(&parent, leaf.as_ref(), &source)
        .unwrap()
        .fingerprint;
    let trash = open_maintenance_trash(&home, guard.home_dir()).unwrap();
    let trash_run = create_trash_run(&trash).unwrap();
    let staged = trash_run.path.join("00000000");

    let error = remove_revalidated_entry_with_hook(
        RemovalTarget {
            home: &home,
            home_dir: guard.home_dir(),
            namespace: "downloads",
            relative: "owned.pkg",
            expected: &expected,
            expected_bytes: 7,
            expected_fingerprint,
        },
        &trash_run,
        0,
        || {
            std::fs::remove_file(&target).unwrap();
            std::fs::write(&target, b"current").unwrap();
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CacheExecutionError::EntryChangedDuringRemoval { .. }
    ));
    assert_eq!(std::fs::read(&staged).unwrap(), b"current");
    drop(trash_run);
    assert!(sweep_existing_trash(&trash, super::CacheExecutionLimits::new(1, 1).unwrap()).is_err());
    assert_eq!(std::fs::read(staged).unwrap(), b"current");
}

#[cfg(unix)]
#[test]
fn removal_refuses_a_leaf_replaced_since_inventory() {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let target = home.downloads_cache_dir().join("owned.pkg");
    std::fs::write(&target, b"planned").unwrap();
    let guard = MaintenanceGuard::acquire(&home).unwrap();
    let (parent, leaf, source) =
        open_removal_parent(&home, guard.home_dir(), "downloads", "owned.pkg").unwrap();
    let expected = pin_object(&parent, &leaf, &source).unwrap().handle;
    let expected_fingerprint = observe_tree(&parent, leaf.as_ref(), &source)
        .unwrap()
        .fingerprint;
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&target, b"current").unwrap();
    let trash = open_maintenance_trash(&home, guard.home_dir()).unwrap();
    let trash_run = create_trash_run(&trash).unwrap();

    let outcome = remove_revalidated_entry(
        RemovalTarget {
            home: &home,
            home_dir: guard.home_dir(),
            namespace: "downloads",
            relative: "owned.pkg",
            expected: &expected,
            expected_bytes: 7,
            expected_fingerprint,
        },
        &trash_run,
        0,
    )
    .unwrap();

    assert_eq!(outcome, RemovalOutcome::Changed);
    assert_eq!(std::fs::read(&target).unwrap(), b"current");
    trash_run.finish().unwrap();
}

#[cfg(unix)]
#[test]
fn removal_preserves_content_mutated_in_place_after_inventory() {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    let target = home.downloads_cache_dir().join("owned.pkg");
    std::fs::write(&target, b"planned").unwrap();
    let guard = MaintenanceGuard::acquire(&home).unwrap();
    let (parent, leaf, source) =
        open_removal_parent(&home, guard.home_dir(), "downloads", "owned.pkg").unwrap();
    let expected = pin_object(&parent, &leaf, &source).unwrap().handle;
    let expected_fingerprint = observe_tree(&parent, leaf.as_ref(), &source)
        .unwrap()
        .fingerprint;
    let trash = open_maintenance_trash(&home, guard.home_dir()).unwrap();
    let trash_run = create_trash_run(&trash).unwrap();
    let staged = trash_run.path.join("00000000");

    let error = remove_revalidated_entry_with_hook(
        RemovalTarget {
            home: &home,
            home_dir: guard.home_dir(),
            namespace: "downloads",
            relative: "owned.pkg",
            expected: &expected,
            expected_bytes: 7,
            expected_fingerprint,
        },
        &trash_run,
        0,
        || std::fs::write(&target, b"changed").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CacheExecutionError::EntryChangedDuringRemoval { .. }
    ));
    assert_eq!(std::fs::read(staged).unwrap(), b"changed");
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
