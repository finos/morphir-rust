//! Independent actual common-config I/O controls. SDK-prefix observations only.
use cap_std::fs::Dir;
use morphir_common::config::observation::{ReadClass, ReadLimits, SourceReadScope};
use morphir_common::config::{load_config_value, load_config_value_observed};
use std::path::Path;

fn limits() -> ReadLimits {
    ReadLimits::new(1_000_000, 1_000_000, 16_777_216, 16).unwrap()
}
fn bind(root: &Path) -> SourceReadScope {
    let held = Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
    SourceReadScope::bind_workspace(&held, root, limits()).unwrap()
}
fn primary(root: &Path) -> std::path::PathBuf {
    let path = root.join("model.json");
    std::fs::write(
        &path,
        include_bytes!("source_observation_fixtures/primary.json"),
    )
    .unwrap();
    path
}
fn project(root: &Path) -> std::path::PathBuf {
    let path = root.join("morphir.toml");
    std::fs::write(
        &path,
        include_bytes!("source_observation_fixtures/project.toml"),
    )
    .unwrap();
    path
}

#[test]
fn held_root_identity_must_match_label_before_binding() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let held = Dir::open_ambient_dir(a.path(), cap_std::ambient_authority()).unwrap();
    assert!(SourceReadScope::bind_workspace(&held, b.path(), limits()).is_err());
}

#[test]
fn actual111_byte_read_has_one_content_attempt_and_exact_returned_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = primary(root.path());
    let scope = bind(root.path());
    let view = scope.observation();
    assert!(
        view.snapshot().require_complete().is_err(),
        "unsealed view cannot certify zero"
    );
    // parse_config dispatches .json to LegacyProjectConfig, whose required
    // project name/sourceDirectory/exposedModules are absent in this fixed IR.
    assert!(load_config_value_observed(&path, &scope).is_err());
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    let reads = snapshot.counts(ReadClass::WorkspaceContent);
    assert_eq!(reads.content_attempts(), 1);
    assert!(reads.read_calls() >= 1);
    assert_eq!(reads.returned_bytes(), 111);
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        0
    );
}

#[test]
fn metadata_registration_does_not_manufacture_content_activity_or_complete_coverage() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("secret");
    std::fs::write(
        &path,
        include_bytes!("source_observation_fixtures/undeclared-sentinel.txt"),
    )
    .unwrap();
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = bind(root.path());
    let file = scope.register_file(&held, Path::new("secret")).unwrap();
    let snapshot = scope.seal();
    assert_eq!(snapshot.counts_for(&file).unwrap().content_attempts(), 0);
    assert!(
        snapshot.require_complete().is_err(),
        "identity metadata is not an observed content route"
    );
}

#[test]
fn forced_sentinel_read_is_nonzero_even_when_parsing_refuses_the_payload() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("secret"),
        include_bytes!("source_observation_fixtures/undeclared-sentinel.txt"),
    )
    .unwrap();
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = bind(root.path());
    let sentinel = scope.register_file(&held, Path::new("secret")).unwrap();
    assert!(load_config_value_observed(&root.path().join("secret"), &scope).is_err());
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    let actual = snapshot.counts_for(&sentinel).unwrap();
    assert_eq!(actual.content_attempts(), 1);
    assert!(actual.read_calls() >= 1);
    assert_eq!(
        actual.returned_bytes(),
        include_bytes!("source_observation_fixtures/undeclared-sentinel.txt").len() as u64
    );
}

#[test]
fn actual_file_identity_detects_a_hardlink_read_under_another_name() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("secret"),
        include_bytes!("source_observation_fixtures/undeclared-sentinel.txt"),
    )
    .unwrap();
    std::fs::hard_link(root.path().join("secret"), root.path().join("alias.json")).unwrap();
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = bind(root.path());
    let sentinel = scope.register_file(&held, Path::new("secret")).unwrap();
    assert!(load_config_value_observed(&root.path().join("alias.json"), &scope).is_err());
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot.counts_for(&sentinel).unwrap().content_attempts(),
        1,
        "identity, not name, matches sentinel"
    );
}

#[test]
fn ambient_is_outside_the_actual_workspace_not_the_native_read_api_name() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let inside = primary(&workspace);
    let outside = outer.path().join("ambient.toml");
    std::fs::write(
        &outside,
        include_bytes!("source_observation_fixtures/ambient.toml"),
    )
    .unwrap();
    let scope = bind(&workspace);
    assert!(load_config_value_observed(&inside, &scope).is_err());
    assert_eq!(
        load_config_value_observed(&outside, &scope).unwrap()["project"]["name"],
        "ambient/ignored"
    );
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        1
    );
    assert_eq!(
        snapshot.counts(ReadClass::AmbientContent).returned_bytes(),
        include_bytes!("source_observation_fixtures/ambient.toml").len() as u64
    );
}

#[test]
fn a_failed_utf8_business_read_preserves_attempt_and_returned_byte_observations() {
    let outer = tempfile::tempdir().unwrap();
    let workspace = outer.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let outside = outer.path().join("invalid.toml");
    std::fs::write(
        &outside,
        include_bytes!("source_observation_fixtures/invalid-utf8.toml"),
    )
    .unwrap();
    let scope = bind(&workspace);
    assert!(load_config_value_observed(&outside, &scope).is_err());
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    let counts = snapshot.counts(ReadClass::AmbientContent);
    assert_eq!(counts.content_attempts(), 1);
    assert!(counts.read_calls() >= 1);
    assert_eq!(counts.returned_bytes(), 2);
}

#[test]
fn an_uninstrumented_default_read_cannot_certify_a_zero_observation() {
    let root = tempfile::tempdir().unwrap();
    let path = primary(root.path());
    let scope = bind(root.path());
    assert!(load_config_value(&path).is_err());
    let snapshot = scope.seal();
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        0,
        "prefix counters contain no hook events"
    );
    assert!(
        snapshot.require_complete().is_err(),
        "unobserved is not measured zero"
    );
}

#[test]
fn observation_counter_overflow_does_not_change_the_business_read_result() {
    let root = tempfile::tempdir().unwrap();
    let path = project(root.path());
    let held = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let scope = SourceReadScope::bind_workspace(
        &held,
        root.path(),
        ReadLimits::new(1, 1_000_000, 16_777_216, 16).unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            load_config_value_observed(&path, &scope).unwrap(),
            serde_json::json!({"project":{"name":"acme/orders","version":"1.0.0"}})
        );
    }
    let snapshot = scope.seal();
    assert!(snapshot.require_complete().is_err());
    assert_eq!(
        snapshot
            .counts(ReadClass::WorkspaceContent)
            .content_attempts(),
        1,
        "bounded retained prefix, never a reset counter"
    );
}

#[test]
fn same_spelling_registered_files_do_not_cross_observation_scopes() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let pa = primary(a.path());
    let pb = primary(b.path());
    let da = Dir::open_ambient_dir(a.path(), cap_std::ambient_authority()).unwrap();
    let db = Dir::open_ambient_dir(b.path(), cap_std::ambient_authority()).unwrap();
    let sa = bind(a.path());
    let sb = bind(b.path());
    let wa = sa.register_file(&da, Path::new("model.json")).unwrap();
    let wb = sb.register_file(&db, Path::new("model.json")).unwrap();
    assert!(load_config_value_observed(&pa, &sa).is_err());
    assert!(load_config_value_observed(&pb, &sb).is_err());
    let va = sa.seal();
    let vb = sb.seal();
    va.require_complete().unwrap();
    vb.require_complete().unwrap();
    assert_eq!(vb.counts_for(&wb).unwrap().content_attempts(), 1);
    assert!(
        vb.counts_for(&wa).is_err(),
        "actual foreign file/scope is not authority from same spelling"
    );
}

#[test]
fn actual_valid_toml_business_success_has_exact_in_root_read_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = project(root.path());
    let scope = bind(root.path());
    assert_eq!(
        load_config_value_observed(&path, &scope).unwrap(),
        serde_json::json!({"project":{"name":"acme/orders","version":"1.0.0"}})
    );
    let snapshot = scope.seal();
    snapshot.require_complete().unwrap();
    let reads = snapshot.counts(ReadClass::WorkspaceContent);
    assert_eq!(reads.content_attempts(), 1);
    assert!(reads.read_calls() >= 1);
    assert_eq!(
        reads.returned_bytes(),
        include_bytes!("source_observation_fixtures/project.toml").len() as u64
    );
    assert_eq!(
        snapshot
            .counts(ReadClass::AmbientContent)
            .content_attempts(),
        0
    );
}
