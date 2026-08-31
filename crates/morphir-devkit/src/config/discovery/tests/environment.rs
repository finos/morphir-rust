use super::*;

#[test]
fn native_request_keeps_only_configuration_environment_variables() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let options = ConfigLoadOptions {
        env: crate::EnvSelection::Explicit(vec![
            ("MORPHIR_PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
            ("MORPHIR_HOME".to_owned(), "/not/config".to_owned()),
            ("PATH".to_owned(), "/bin".to_owned()),
        ]),
        ..ConfigLoadOptions::project_only()
    };

    let request = build_workspace_discovery_request(root.path(), &options).unwrap();
    let snapshot = discover_workspace(root.path(), &options).unwrap();

    assert_eq!(
        request.environment,
        BTreeMap::from([("MORPHIR_PROJECT__VERSION".to_owned(), "2.0.0".to_owned())])
    );
    assert_eq!(snapshot.projects[0].version.as_deref(), Some("2.0.0"));
}

#[test]
fn custom_environment_prefix_preserves_reserved_looking_configuration_keys() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let options = ConfigLoadOptions {
        env: crate::EnvSelection::Explicit(vec![
            ("APP_HOME".to_owned(), "project-home".to_owned()),
            ("APP_LOG_DIR".to_owned(), "project-logs".to_owned()),
            ("APP_IR__STRICT_MODE".to_owned(), "true".to_owned()),
            ("APP_PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
            ("MORPHIR_HOME".to_owned(), "/operational-home".to_owned()),
            ("MORPHIR_LOG_DIR".to_owned(), "/operational-logs".to_owned()),
        ]),
        env_prefix: "APP".to_owned(),
        ..ConfigLoadOptions::project_only()
    };

    let request = build_workspace_discovery_request(root.path(), &options).unwrap();
    let snapshot = discover_workspace(root.path(), &options).unwrap();

    assert_eq!(
        request.environment,
        BTreeMap::from([
            ("MORPHIR__HOME".to_owned(), "project-home".to_owned()),
            ("MORPHIR__IR__STRICT_MODE".to_owned(), "true".to_owned()),
            ("MORPHIR__LOG_DIR".to_owned(), "project-logs".to_owned()),
            ("MORPHIR__PROJECT__VERSION".to_owned(), "2.0.0".to_owned()),
        ])
    );
    assert_eq!(snapshot.projects[0].version.as_deref(), Some("2.0.0"));
}

#[test]
fn missing_explicit_system_config_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\n",
    )
    .unwrap();
    let missing = root.path().join("missing-system.toml");
    let options = ConfigLoadOptions {
        system: SourceSelection::Explicit(missing.clone()),
        ..ConfigLoadOptions::project_only()
    };

    let error = build_workspace_discovery_request(root.path(), &options).unwrap_err();

    assert!(error.to_string().contains("explicit system config"));
    assert!(error.to_string().contains(&missing.display().to_string()));
}

#[test]
fn missing_explicit_global_config_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\n",
    )
    .unwrap();
    let missing = root.path().join("missing-global.yaml");
    let options = ConfigLoadOptions {
        global: SourceSelection::Explicit(missing.clone()),
        ..ConfigLoadOptions::project_only()
    };

    let error = build_workspace_discovery_request(root.path(), &options).unwrap_err();

    assert!(error.to_string().contains("explicit global user config"));
    assert!(error.to_string().contains(&missing.display().to_string()));
}

#[test]
fn native_tree_does_not_read_unrecognized_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\n",
    )
    .unwrap();
    std::fs::write(root.path().join("asset.bin"), [0xff, 0xfe, 0xfd]).unwrap();

    let request =
        build_workspace_discovery_request(root.path(), &ConfigLoadOptions::project_only()).unwrap();

    assert!(
        !request
            .development_root
            .entries
            .contains_key(&RelativePath::parse("asset.bin").unwrap())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn native_tree_ignores_unrecognized_non_utf8_file_names() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = \"acme/orders\"\n",
    )
    .unwrap();
    std::fs::write(root.path().join(OsString::from_vec(vec![0xff])), "ignored").unwrap();

    let snapshot = discover_workspace(root.path(), &ConfigLoadOptions::project_only()).unwrap();

    assert_eq!(snapshot.projects[0].name, "acme/orders");
}
