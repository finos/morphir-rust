use super::super::{ToolInstaller, rollback_tool};
use super::support::package;
use crate::{Selection, ToolId};
use morphir_common::home::MorphirHome;
use semver::Version;
use std::fs;

#[test]
fn reinstalling_the_active_release_does_not_add_it_to_rollback_history() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "2.0.0", b"desktop-v2"))
        .unwrap();
    let mut reselected = package(&home, "2.0.0", b"desktop-v2");
    reselected.selection = Selection::Exact(Version::parse("2.0.0").unwrap());
    ToolInstaller::new(&home).install(reselected).unwrap();

    let restored = rollback_tool(&home, &id).unwrap();

    assert_eq!(restored.version(), &Version::parse("1.0.0").unwrap());
}

#[test]
fn rollback_rejects_catalog_metadata_not_retained_in_the_exact_lock() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ToolId::parse("desktop").unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "1.0.0", b"desktop-v1"))
        .unwrap();
    ToolInstaller::new(&home)
        .install(package(&home, "2.0.0", b"desktop-v2"))
        .unwrap();
    let lock_path = home.tools_locks_dir().join("desktop.json");
    let lock_before = fs::read(&lock_path).unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(home.tools_catalog_file()).unwrap()).unwrap();
    catalog["tools"][0]["rollback"][0]["version"] = serde_json::json!("9.9.9");
    fs::write(
        home.tools_catalog_file(),
        serde_json::to_vec_pretty(&catalog).unwrap(),
    )
    .unwrap();

    assert!(matches!(
        rollback_tool(&home, &id).unwrap_err(),
        crate::DistributionError::ToolStateMismatch { .. }
    ));
    assert_eq!(fs::read(lock_path).unwrap(), lock_before);
}
