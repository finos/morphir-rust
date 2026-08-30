use morphir_common::cache_maintenance::{
    CacheEntryState, CacheInventoryLimits, CacheNamespace, inventory_cache_namespace,
};
#[cfg(unix)]
use morphir_common::cache_maintenance::{CachePolicy, CleanupMode, plan_cache_cleanup};
use morphir_common::home::MorphirHome;
use std::ffi::OsStr;
#[cfg(unix)]
use std::time::Duration;
use tempfile::TempDir;

fn a_morphir_home() -> (TempDir, MorphirHome) {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    (root, home)
}

#[test]
fn inventory_measures_registered_entries_and_reports_unknown_siblings_without_following_them() {
    let (_root, home) = a_morphir_home();
    let namespace_root = home.downloads_cache_dir();
    std::fs::create_dir_all(namespace_root.join("packages/owned")).unwrap();
    std::fs::write(namespace_root.join("packages/owned/archive.bin"), b"owned").unwrap();
    std::fs::write(namespace_root.join("packages/unregistered.bin"), b"unknown").unwrap();
    std::fs::write(namespace_root.join("loose.bin"), b"loose").unwrap();

    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("packages/owned", 100)
        .unwrap();
    let entries =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();

    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.path(), entry.bytes(), entry.state()))
            .collect::<Vec<_>>(),
        vec![
            ("loose.bin", 5, CacheEntryState::Unclassified),
            (
                "packages/owned",
                5,
                CacheEntryState::Disposable { last_used: 100 }
            ),
            (
                "packages/unregistered.bin",
                7,
                CacheEntryState::Unclassified
            ),
        ]
    );
}

#[test]
fn inventory_preserves_active_lease_classification() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.desktop_cache_dir()).unwrap();
    std::fs::write(home.desktop_cache_dir().join("active-session"), b"leased").unwrap();

    let namespace = CacheNamespace::new("desktop")
        .unwrap()
        .with_lease("active-session", 42)
        .unwrap();
    let entries =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].bytes(), 6);
    assert_eq!(
        entries[0].state(),
        CacheEntryState::ActiveLease { last_used: 42 }
    );
}

#[test]
fn inventory_matches_portable_aliases_only_when_the_filesystem_resolves_them() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir().join("Packages/CAF\u{c9}")).unwrap();
    std::fs::write(
        home.downloads_cache_dir()
            .join("Packages/CAF\u{c9}/archive"),
        b"owned",
    )
    .unwrap();

    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("packages/cafe\u{301}", 100)
        .unwrap();
    let entries =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();

    let registered_parent_resolves = home.downloads_cache_dir().join("packages").exists();
    let registered_spelling_resolves = home
        .downloads_cache_dir()
        .join("packages/cafe\u{301}")
        .exists();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].path(),
        if registered_parent_resolves {
            "Packages/CAF\u{c9}"
        } else {
            "Packages"
        }
    );
    assert_eq!(entries[0].bytes(), 5);
    assert_eq!(
        entries[0].state(),
        if registered_spelling_resolves {
            CacheEntryState::Disposable { last_used: 100 }
        } else {
            CacheEntryState::Unclassified
        }
    );
}

#[cfg(unix)]
#[test]
fn a_lone_portable_alias_is_unclassified_when_its_registered_spelling_is_absent() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("ARTIFACT"), b"unregistered").unwrap();

    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("artifact", 100)
        .unwrap();
    let entries =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path(), "ARTIFACT");
    assert_eq!(entries[0].state(), CacheEntryState::Unclassified);
}

#[cfg(unix)]
#[test]
fn portable_aliases_keep_distinct_observed_paths() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("artifact"), b"lower").unwrap();
    std::fs::write(home.downloads_cache_dir().join("ARTIFACT"), b"upper").unwrap();

    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("artifact", 100)
        .unwrap();
    let entries =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();

    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.path(), entry.state()))
            .collect::<Vec<_>>(),
        [
            ("ARTIFACT", CacheEntryState::Unclassified),
            ("artifact", CacheEntryState::Disposable { last_used: 100 })
        ]
    );
    plan_cache_cleanup(
        entries,
        CachePolicy::new(Duration::from_secs(0), 0),
        200,
        CleanupMode::All,
    )
    .expect("portable aliases should remain distinct planner identities");
}

#[test]
fn inventory_limits_fail_closed_before_an_unbounded_walk() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.indexes_cache_dir()).unwrap();
    std::fs::write(home.indexes_cache_dir().join("first"), b"1").unwrap();
    std::fs::write(home.indexes_cache_dir().join("second"), b"2").unwrap();

    let namespace = CacheNamespace::new("indexes").unwrap();
    let error =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::new(1, 8).unwrap())
            .unwrap_err();

    assert!(error.to_string().contains("entry limit"));
}

#[test]
fn a_link_like_namespace_root_is_rejected_instead_of_followed() {
    let (_root, home) = a_morphir_home();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(home.cache_dir()).unwrap();

    if create_directory_link(outside.path(), &home.downloads_cache_dir()).is_err() {
        // Windows developer-mode or elevation may be unavailable. The production
        // reparse-point guard remains exercised by platform-independent unit tests.
        return;
    }

    let namespace = CacheNamespace::new("downloads").unwrap();
    let error =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap_err();
    assert!(error.to_string().contains("link-like namespace root"));
}

#[test]
fn a_link_like_cache_root_is_rejected_instead_of_followed() {
    let (_root, home) = a_morphir_home();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(outside.path().join("downloads")).unwrap();
    std::fs::write(outside.path().join("downloads/owned"), b"outside").unwrap();
    if create_directory_link(outside.path(), &home.cache_dir()).is_err() {
        return;
    }

    let namespace = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable("owned", 1)
        .unwrap();
    let error =
        inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap_err();

    assert!(error.to_string().contains("cache root"));
    assert_eq!(
        std::fs::read(outside.path().join("downloads/owned")).unwrap(),
        b"outside"
    );
}

#[test]
fn hostile_unknown_names_are_encoded_as_protected_identities() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("%raw"), b"unknown").unwrap();

    let entries = inventory_cache_namespace(
        &home,
        &CacheNamespace::new("downloads").unwrap(),
        CacheInventoryLimits::default(),
    )
    .unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state(), CacheEntryState::Unclassified);
    assert_eq!(entries[0].path(), "%25%72%61%77");
}

#[cfg(unix)]
#[test]
fn windows_reserved_unknown_names_are_encoded_on_unix() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("CON"), b"unknown").unwrap();

    let entries = inventory_cache_namespace(
        &home,
        &CacheNamespace::new("downloads").unwrap(),
        CacheInventoryLimits::default(),
    )
    .unwrap();

    assert_eq!(entries[0].state(), CacheEntryState::Unclassified);
    assert_eq!(entries[0].path(), "%43%4F%4E");
}

#[cfg(unix)]
#[test]
fn platform_specific_separators_in_unknown_names_are_encoded_on_unix() {
    let (_root, home) = a_morphir_home();
    std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
    std::fs::write(home.downloads_cache_dir().join("a:b"), b"colon").unwrap();
    std::fs::write(home.downloads_cache_dir().join(r"a\b"), b"backslash").unwrap();

    let entries = inventory_cache_namespace(
        &home,
        &CacheNamespace::new("downloads").unwrap(),
        CacheInventoryLimits::default(),
    )
    .unwrap();

    assert_eq!(
        entries.iter().map(|entry| entry.path()).collect::<Vec<_>>(),
        ["%61%3A%62", "%61%5C%62"]
    );
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[test]
fn namespace_registration_rejects_nonportable_owned_paths() {
    let error = CacheNamespace::new("downloads")
        .unwrap()
        .with_disposable(OsStr::new("../outside").to_string_lossy(), 1)
        .unwrap_err();
    assert!(error.to_string().contains("portable relative path"));
}
