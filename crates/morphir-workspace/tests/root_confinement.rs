use std::collections::BTreeMap;

use morphir_workspace::{
    DiagnosticSeverity, DiscoveryFailure, DiscoveryRequest, DiscoveryResponse, FileEntry, FileTree,
    ProjectSnapshot, ProjectState, RelativePath, RelativePathError, WORKSPACE_CONFIG_AMBIGUOUS,
    WORKSPACE_CONFIG_INVALID, WORKSPACE_CONFIG_MISSING, WORKSPACE_DISCOVERY_PROTOCOL,
    WORKSPACE_MEMBER_DUPLICATE_NAME, WORKSPACE_MEMBER_INVALID, WORKSPACE_PATH_NOT_CONFINED,
    WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceDiagnostic, WorkspaceSnapshot, WorkspaceState,
};
use serde_json::json;

#[test]
fn accepts_canonical_relative_paths() {
    assert_eq!(
        RelativePath::parse("packages/orders/src").unwrap().as_str(),
        "packages/orders/src"
    );
    assert_eq!(RelativePath::root().as_str(), ".");
}

#[test]
fn rejects_every_lexical_escape_shape() {
    for path in [
        "../outside",
        "packages/../../outside",
        "/etc/passwd",
        "C:/outside",
        r"packages\orders",
        "packages//orders",
    ] {
        assert!(matches!(
            RelativePath::parse(path),
            Err(RelativePathError::NotConfined { .. })
        ));
    }
}

#[test]
fn path_operations_preserve_canonical_confinement() {
    let packages = RelativePath::parse("packages").unwrap();
    let orders = packages.join("orders/src").unwrap();

    assert_eq!(orders.as_str(), "packages/orders/src");
    assert_eq!(orders.parent().as_str(), "packages/orders");
    assert_eq!(
        RelativePath::parse("packages").unwrap().parent().as_str(),
        "."
    );
    assert_eq!(RelativePath::root().parent(), RelativePath::root());
    assert_eq!(RelativePath::root().join("packages").unwrap(), packages);
    assert_eq!(packages.join(".").unwrap(), packages);

    for path in ["../outside", "orders/../outside", "/outside", "D:/outside"] {
        assert!(matches!(
            packages.join(path),
            Err(RelativePathError::NotConfined { .. })
        ));
    }
}

#[test]
fn deserialization_cannot_bypass_path_validation() {
    assert_eq!(
        serde_json::from_str::<RelativePath>(r#""packages/orders""#).unwrap(),
        RelativePath::parse("packages/orders").unwrap()
    );

    for json in [
        r#""../outside""#,
        r#""packages//orders""#,
        r#""C:/outside""#,
    ] {
        let error = serde_json::from_str::<RelativePath>(json).unwrap_err();
        assert!(error.to_string().contains(WORKSPACE_PATH_NOT_CONFINED));
    }
}

#[test]
fn file_entries_use_stable_kebab_case_tags() {
    assert_eq!(
        serde_json::to_value(FileEntry::Directory).unwrap(),
        json!({ "kind": "directory" })
    );
    assert_eq!(
        serde_json::to_value(FileEntry::File {
            text: "source".to_owned()
        })
        .unwrap(),
        json!({ "kind": "file", "text": "source" })
    );
    assert_eq!(
        serde_json::to_value(FileEntry::Symlink {
            target: RelativePath::parse("packages/orders").unwrap()
        })
        .unwrap(),
        json!({ "kind": "symlink", "target": "packages/orders" })
    );
}

#[test]
fn request_uses_camel_case_defaults_and_sorted_entries() {
    let request: DiscoveryRequest = serde_json::from_value(json!({
        "protocolVersion": WORKSPACE_DISCOVERY_PROTOCOL,
        "developmentRoot": {
            "entries": {
                "zeta": { "kind": "directory" },
                "alpha": { "kind": "file", "text": "first" }
            }
        },
        "morphirHome": null,
        "systemConfig": null
    }))
    .unwrap();

    assert!(request.environment.is_empty());
    assert_eq!(request.cli_overlay, serde_json::Value::Null);
    assert_eq!(
        serde_json::to_string(&request.development_root).unwrap(),
        r#"{"entries":{"alpha":{"kind":"file","text":"first"},"zeta":{"kind":"directory"}}}"#
    );
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["protocolVersion"], WORKSPACE_DISCOVERY_PROTOCOL);
    assert!(value.get("protocol_version").is_none());
    assert!(value.get("developmentRoot").is_some());
}

#[test]
fn invalid_file_tree_keys_are_rejected() {
    let error = serde_json::from_value::<FileTree>(json!({
        "entries": { "../outside": { "kind": "directory" } }
    }))
    .unwrap_err();

    assert!(error.to_string().contains(WORKSPACE_PATH_NOT_CONFINED));
}

#[test]
fn snapshot_and_response_wire_shapes_are_stable() {
    let diagnostic = WorkspaceDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: WORKSPACE_CONFIG_INVALID.to_owned(),
        message: "invalid workspace configuration".to_owned(),
        path: Some(RelativePath::parse("morphir.toml").unwrap()),
        project_path: Some(RelativePath::parse("packages/orders").unwrap()),
    };
    let snapshot = WorkspaceSnapshot {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        config_anchor: RelativePath::parse("morphir.toml").unwrap(),
        name: Some("shop".to_owned()),
        state: WorkspaceState::Open,
        projects: vec![ProjectSnapshot {
            name: "orders".to_owned(),
            version: Some("1.0.0".to_owned()),
            relative_path: RelativePath::parse("packages/orders").unwrap(),
            config_anchor: Some(RelativePath::parse("packages/orders/morphir.toml").unwrap()),
            source_directory: RelativePath::parse("packages/orders/src").unwrap(),
            state: ProjectState::Unloaded,
            diagnostics: vec![diagnostic.clone()],
        }],
        diagnostics: vec![diagnostic],
    };
    let response = DiscoveryResponse::Success {
        snapshot: snapshot.clone(),
    };

    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["status"], "success");
    assert_eq!(value["snapshot"]["protocolVersion"], 1);
    assert_eq!(value["snapshot"]["configAnchor"], "morphir.toml");
    assert_eq!(value["snapshot"]["state"], "open");
    assert_eq!(
        value["snapshot"]["projects"][0]["sourceDirectory"],
        "packages/orders/src"
    );
    assert_eq!(value["snapshot"]["diagnostics"][0]["severity"], "warning");
    assert_eq!(response.into_result().unwrap(), snapshot);
}

#[test]
fn failure_response_round_trips_and_converts_to_error() {
    let failure = DiscoveryFailure {
        code: WORKSPACE_PROTOCOL_UNSUPPORTED.to_owned(),
        message: "unsupported protocol".to_owned(),
        path: None,
    };
    let response = DiscoveryResponse::Failure {
        error: failure.clone(),
    };

    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        json,
        r#"{"status":"failure","error":{"code":"workspace.protocol.unsupported","message":"unsupported protocol","path":null}}"#
    );
    assert_eq!(response.into_result().unwrap_err(), failure);
}

#[test]
fn diagnostic_codes_are_exact_and_stable() {
    assert_eq!(WORKSPACE_CONFIG_MISSING, "workspace.config.missing");
    assert_eq!(WORKSPACE_CONFIG_AMBIGUOUS, "workspace.config.ambiguous");
    assert_eq!(WORKSPACE_CONFIG_INVALID, "workspace.config.invalid");
    assert_eq!(WORKSPACE_MEMBER_INVALID, "workspace.member.invalid");
    assert_eq!(
        WORKSPACE_MEMBER_DUPLICATE_NAME,
        "workspace.member.duplicate-name"
    );
    assert_eq!(WORKSPACE_PATH_NOT_CONFINED, "workspace.path.not-confined");
    assert_eq!(
        WORKSPACE_PROTOCOL_UNSUPPORTED,
        "workspace.protocol.unsupported"
    );
}

#[test]
fn file_tree_can_be_constructed_with_a_sorted_map() {
    let entries = BTreeMap::from([
        (RelativePath::parse("beta").unwrap(), FileEntry::Directory),
        (RelativePath::parse("alpha").unwrap(), FileEntry::Directory),
    ]);

    assert_eq!(
        FileTree { entries }
            .entries
            .keys()
            .map(RelativePath::as_str)
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
}
