use super::super::{ToolInstaller, rollback_tool};
use super::support::package;
use crate::{Selection, ToolId};
use morphir_common::home::MorphirHome;
use semver::Version;

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
