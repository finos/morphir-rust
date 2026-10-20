//! Source route and actual file identity are separate observation facts.
//! Frozen before the first execution against SDK cohort1 c784af06.
#[cfg(unix)]
#[test]
fn outside_symlink_to_inside_file_is_ambient_while_inside_witness_matches() {
    use cap_std::fs::Dir;
    use morphir_common::config::load_config_value_observed;
    use morphir_common::config::observation::{ReadClass, ReadLimits, SourceReadScope};
    use std::path::Path;

    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let inside = workspace.join("morphir.json");
    std::fs::write(
        &inside,
        include_bytes!("source_observation_fixtures/valid-config.json"),
    )
    .unwrap();
    let outside = outer.path().join("outside-alias.json");
    std::os::unix::fs::symlink(&inside, &outside).unwrap();
    let held = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
    let limits = ReadLimits::new(1_000_000, 1_000_000, 16_777_216, 16).unwrap();
    let scope = SourceReadScope::bind_workspace(&held, &workspace, limits).unwrap();
    let actual_file = scope
        .register_file(&held, Path::new("morphir.json"))
        .unwrap();

    let value = load_config_value_observed(&outside, &scope).unwrap();
    assert_eq!(value["project"]["name"], "scope/fixtures");
    let snapshot = scope.seal();
    // Stable actual outside route plus known inside target has no unknown
    // classification in this fixture. File identity does not erase that route.
    snapshot.require_complete().unwrap();
    let witness = snapshot.counts_for(&actual_file).unwrap();
    assert_eq!(witness.content_attempts(), 1);
    assert!(witness.read_calls() >= 1);
    assert_eq!(witness.returned_bytes(), 111);
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        0
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .returned_bytes(),
        0
    );
    let ambient = snapshot.counts(ReadClass::AmbientContent);
    assert_eq!(ambient.content_attempts(), 1);
    assert!(ambient.read_calls() >= 1);
    assert_eq!(ambient.returned_bytes(), 111);
}
