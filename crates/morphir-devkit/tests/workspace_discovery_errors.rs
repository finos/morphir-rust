use std::fs;

use morphir_devkit::{
    ConfigLoadOptions, NativeWorkspaceDiscoveryError, build_workspace_discovery_request,
    discover_workspace_detailed, discover_workspace_detailed_typed,
};
use morphir_workspace::{RelativePath, WORKSPACE_CONFIG_AMBIGUOUS, WORKSPACE_CONFIG_INVALID};

#[test]
fn direct_host_failures_remain_visible_in_the_standard_error_chain() {
    let error = NativeWorkspaceDiscoveryError::Host(anyhow::anyhow!("direct host failure"));

    let source = std::error::Error::source(&error).expect("host failure must be the source");

    assert_eq!(source.to_string(), "direct host failure");
}

#[test]
fn portable_failures_retain_their_typed_code_message_and_path() {
    for (name, files, expected_code, expected_message) in [
        (
            "ambiguous",
            vec![
                ("morphir.toml", "[project]\nname = 'typed/root'\n"),
                ("morphir.yaml", "project:\n  name: typed/root\n"),
            ],
            WORKSPACE_CONFIG_AMBIGUOUS,
            "multiple Morphir configurations found for workspace root: `morphir.toml`, `morphir.yaml`",
        ),
        (
            "invalid",
            vec![("morphir.toml", "[project\nname = 'typed/root'\n")],
            WORKSPACE_CONFIG_INVALID,
            "invalid Morphir configuration at `morphir.toml`: Failed to parse TOML config morphir.toml: TOML parse error at line 1, column 9\n  |\n1 | [project\n  |         ^\nunclosed table, expected `]`\n",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            fs::write(root.path().join(path), contents).unwrap();
        }

        let error =
            discover_workspace_detailed_typed(root.path(), &ConfigLoadOptions::project_only())
                .unwrap_err();
        let typed_display = error.to_string();
        let NativeWorkspaceDiscoveryError::Portable(failure) = error else {
            panic!("{name}: expected a portable discovery failure")
        };

        assert_eq!(failure.code, expected_code, "{name}");
        assert_eq!(failure.message, expected_message, "{name}");
        assert_eq!(
            failure.path,
            Some(RelativePath::parse("morphir.toml").unwrap()),
            "{name}"
        );
        let legacy = discover_workspace_detailed(root.path(), &ConfigLoadOptions::project_only())
            .unwrap_err();
        let expected_display = format!("{expected_code}: {expected_message} at `morphir.toml`");
        assert_eq!(typed_display, expected_display, "{name}");
        assert_eq!(legacy.to_string(), expected_display, "{name}");
    }
}

#[test]
fn full_config_decode_failures_remain_native_after_portable_success() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("morphir.toml"),
        "[project]\nname = 'typed/root'\n\n[ir]\nformat_version = 'invalid'\n",
    )
    .unwrap();
    let options = ConfigLoadOptions::project_only();
    let request = build_workspace_discovery_request(root.path(), &options).unwrap();

    morphir_workspace::discover(request).into_result().unwrap();

    let error = discover_workspace_detailed_typed(root.path(), &options).unwrap_err();
    let source = std::error::Error::source(&error).expect("host failure must retain its cause");
    assert_eq!(
        source.to_string(),
        "Failed to decode effective root Morphir configuration"
    );
    assert!(
        std::iter::successors(Some(source), |cause| cause.source())
            .any(|cause| cause.to_string().contains("invalid type: string"))
    );
    let NativeWorkspaceDiscoveryError::Host(host) = error else {
        panic!("expected a host full-config decoding failure")
    };
    assert_eq!(
        host.to_string(),
        "Failed to decode effective root Morphir configuration"
    );
    assert!(
        host.chain()
            .any(|cause| cause.to_string().contains("invalid type: string"))
    );

    let legacy = discover_workspace_detailed(root.path(), &options).unwrap_err();
    assert_eq!(legacy.to_string(), host.to_string());
}
