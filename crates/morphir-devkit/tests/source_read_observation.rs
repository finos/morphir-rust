//! Independent real SDK entry routes. Complete here means the sealed SDK
//! prefix, never host/capture/whole-SDK coverage or all-work-exited.
use cap_std::fs::Dir;
use morphir_common::config::observation::{ReadClass, ReadLimits, SourceReadScope};
use morphir_devkit::{
    ConfigLoadOptions, SourceSelection, WorkspaceDiscoveryRoot, load_config_context_observed,
};
use std::path::Path;

fn scope(root: &Path) -> SourceReadScope {
    let held = Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
    SourceReadScope::bind_workspace(
        &held,
        root,
        ReadLimits::new(1_000_000, 1_000_000, 16_777_216, 16).unwrap(),
    )
    .unwrap()
}
fn config(root: &Path) {
    std::fs::write(
        root.join("morphir.toml"),
        include_bytes!("source_observation_fixtures/project.toml"),
    )
    .unwrap();
}

#[test]
fn confined_workspace_capture_measures_the_real_config_file_read() {
    let root = tempfile::tempdir().unwrap();
    config(root.path());
    let observation = scope(root.path());
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let discovery = WorkspaceDiscoveryRoot::from_directory(held, root.path()).unwrap();
    let captured = discovery
        .capture_observed(&ConfigLoadOptions::project_only(), &observation)
        .unwrap();
    assert!(captured.request().system_config.is_none());
    assert!(captured.request().morphir_home.is_none());
    let snapshot = observation.seal();
    snapshot.require_complete().unwrap();
    let actual = snapshot.counts(ReadClass::WorkspaceContent);
    assert_eq!(actual.content_attempts(), 1);
    assert!(actual.read_calls() >= 1);
    assert_eq!(
        actual.returned_bytes(),
        include_bytes!("source_observation_fixtures/project.toml").len() as u64
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        0
    );
}

#[test]
fn permitted_explicit_global_mount_is_still_an_actual_ambient_read() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    config(&workspace);
    let ambient = outer.path().join("global.toml");
    std::fs::write(
        &ambient,
        include_bytes!("source_observation_fixtures/ambient.toml"),
    )
    .unwrap();
    let observation = scope(&workspace);
    let mut options = ConfigLoadOptions::project_only();
    options.global = SourceSelection::Explicit(ambient);
    let held = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
    let discovery = WorkspaceDiscoveryRoot::from_directory(held, &workspace).unwrap();
    let captured = discovery.capture_observed(&options, &observation).unwrap();
    assert!(captured.request().morphir_home.is_some());
    let snapshot = observation.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        1
    );
    let actual = snapshot.counts(ReadClass::AmbientContent);
    assert_eq!(actual.content_attempts(), 1);
    assert_eq!(
        actual.returned_bytes(),
        include_bytes!("source_observation_fixtures/ambient.toml").len() as u64
    );
}

#[test]
fn ignored_enclosing_parent_configuration_read_is_not_lost_under_project_only() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    config(&workspace);
    // It is an ordinary project config, not a workspace; native enclosing
    // lookup reads it and then ignores its value. No output-derived oracle.
    std::fs::write(
        outer.path().join("morphir.toml"),
        include_bytes!("source_observation_fixtures/ambient.toml"),
    )
    .unwrap();
    let parent = Dir::open_ambient_dir(outer.path(), cap_std::ambient_authority()).unwrap();
    let observation = scope(&workspace);
    let parent_file = observation
        .register_file(&parent, Path::new("morphir.toml"))
        .unwrap();
    let actual = load_config_context_observed(
        &workspace.join("morphir.toml"),
        &ConfigLoadOptions::project_only(),
        &observation,
    )
    .unwrap();
    assert_eq!(actual.config.project.as_ref().unwrap().name, "acme/orders");
    let snapshot = observation.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot
            .counts_for(&parent_file)
            .unwrap()
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot.counts_for(&parent_file).unwrap().returned_bytes(),
        include_bytes!("source_observation_fixtures/ambient.toml").len() as u64
    );
    assert!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts()
            >= 1,
        "other actual ancestors are not assumed absent"
    );
}

#[test]
fn ignored_parent_parse_error_keeps_the_actual_read_attempt_and_prefix_coverage() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    config(&workspace);
    std::fs::write(
        outer.path().join("morphir.toml"),
        include_bytes!("source_observation_fixtures/invalid-ambient.toml"),
    )
    .unwrap();
    let parent = Dir::open_ambient_dir(outer.path(), cap_std::ambient_authority()).unwrap();
    let observation = scope(&workspace);
    let parent_file = observation
        .register_file(&parent, Path::new("morphir.toml"))
        .unwrap();
    let actual = load_config_context_observed(
        &workspace.join("morphir.toml"),
        &ConfigLoadOptions::project_only(),
        &observation,
    )
    .unwrap();
    assert_eq!(actual.config.project.as_ref().unwrap().name, "acme/orders");
    let snapshot = observation.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot
            .counts_for(&parent_file)
            .unwrap()
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot.counts_for(&parent_file).unwrap().returned_bytes(),
        include_bytes!("source_observation_fixtures/invalid-ambient.toml").len() as u64
    );
}
