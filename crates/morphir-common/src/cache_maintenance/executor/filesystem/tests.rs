use super::open_removal_parent;
#[cfg(unix)]
use super::pin_object;
#[cfg(unix)]
use super::{
    CacheExecutionError, RemovalOutcome, create_trash_run, open_maintenance_trash,
    remove_revalidated_entry, remove_revalidated_entry_with_hook, sweep_existing_trash,
};
use crate::home::MorphirHome;
use tempfile::TempDir;

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

    assert!(open_removal_parent(&home, "downloads", "owned/keep").is_err());
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
    let (parent, leaf, source) = open_removal_parent(&home, "downloads", "owned.pkg").unwrap();
    let expected = pin_object(&parent, &leaf, &source).unwrap().handle;
    let trash = open_maintenance_trash(&home).unwrap();
    let trash_run = create_trash_run(&trash).unwrap();
    let staged = trash_run.path.join("00000000");

    let error = remove_revalidated_entry_with_hook(
        &home,
        "downloads",
        "owned.pkg",
        &expected,
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
    assert!(sweep_existing_trash(&trash).is_err());
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
    let (parent, leaf, source) = open_removal_parent(&home, "downloads", "owned.pkg").unwrap();
    let expected = pin_object(&parent, &leaf, &source).unwrap().handle;
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&target, b"current").unwrap();
    let trash = open_maintenance_trash(&home).unwrap();
    let trash_run = create_trash_run(&trash).unwrap();

    let outcome =
        remove_revalidated_entry(&home, "downloads", "owned.pkg", &expected, &trash_run, 0)
            .unwrap();

    assert_eq!(outcome, RemovalOutcome::Changed);
    assert_eq!(std::fs::read(&target).unwrap(), b"current");
    trash_run.finish().unwrap();
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
