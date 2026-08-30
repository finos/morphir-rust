mod fixtures;

use fixtures::fixture_request;
use morphir_workspace::{
    DiscoveryRequest, DiscoveryResponse, ProjectSnapshot, ProjectState, WORKSPACE_CONFIG_AMBIGUOUS,
    WORKSPACE_CONFIG_INVALID, WORKSPACE_MEMBER_INVALID, WORKSPACE_PATH_NOT_CONFINED,
    WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceSnapshot, discover, discover_with_details,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct CorpusCase {
    name: String,
    request: DiscoveryRequest,
    expected: DiscoveryResponse,
}

#[test]
fn discovers_members_excludes_paths_keeps_root_and_isolates_failures() {
    let request = fixture_request("valid-monorepo");
    let snapshot = discover(request).into_result().unwrap();
    let paths = snapshot
        .projects
        .iter()
        .map(|project| project.relative_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        [
            ".",
            "packages/broken",
            "packages/duplicate",
            "packages/orders",
            "packages/risk"
        ]
    );
    assert_eq!(snapshot.projects[0].name, "acme/root");
    let orders = project(&snapshot, "packages/orders");
    assert_eq!(
        (
            orders.relative_path.as_str(),
            orders.source_directory.as_str()
        ),
        ("packages/orders", "elm")
    );
    assert_eq!(
        project(&snapshot, "packages/broken").state,
        ProjectState::Error
    );
    assert!(
        snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "workspace.member.duplicate-name")
    );
}

#[test]
fn discovery_details_reuse_the_exact_pass_and_expose_effective_configs() {
    let request = fixture_request("valid-monorepo");
    let response = discover(request.clone());
    let details = discover_with_details(request).unwrap();

    assert_eq!(
        response,
        DiscoveryResponse::Success {
            snapshot: details.snapshot.clone()
        }
    );
    assert_eq!(details.root_effective["ir"]["format_version"], 4);
    assert_eq!(
        details.project_effective
            [&morphir_workspace::RelativePath::parse("packages/orders").unwrap()]["codegen"]["output_format"],
        "compact"
    );
    assert_eq!(
        details.project_effective
            [&morphir_workspace::RelativePath::parse("packages/risk").unwrap()]["codegen"]["output_format"],
        "risk-compact"
    );
    assert!(
        !details
            .project_effective
            .contains_key(&morphir_workspace::RelativePath::parse("packages/broken").unwrap())
    );
}

#[test]
fn shared_discovery_corpus_matches_structurally() {
    let corpus: Vec<CorpusCase> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/workspace-discovery/corpus.json"
    )))
    .unwrap();

    for case in corpus {
        assert_eq!(
            discover(case.request),
            case.expected,
            "case `{}`",
            case.name
        );
    }
}

#[test]
fn missing_config_in_an_explicit_optional_mount_is_ignored() {
    let response = discover(request_with_home(Some(json!({
        "notes.txt": { "kind": "file", "text": "not configuration" }
    }))));

    assert_eq!(
        response.into_result().unwrap().projects[0].name,
        "optional/root"
    );
}

#[test]
fn ambiguous_config_in_an_explicit_optional_mount_is_fatal() {
    let response = discover(request_with_home(Some(json!({
        "morphir.toml": { "kind": "file", "text": "[ir]\nmode = \"toml\"\n" },
        "morphir.yaml": { "kind": "file", "text": "ir:\n  mode: yaml\n" }
    }))));
    let error = response.into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_CONFIG_AMBIGUOUS);
    assert!(error.message.contains("morphir.toml"));
    assert!(error.message.contains("morphir.yaml"));
}

#[test]
fn invalid_config_in_an_explicit_optional_mount_is_fatal() {
    let response = discover(request_with_home(Some(json!({
        "morphir.toml": { "kind": "file", "text": "[ir\ninvalid" }
    }))));

    assert_eq!(
        response.into_result().unwrap_err().code,
        WORKSPACE_CONFIG_INVALID
    );
}

#[test]
fn unsupported_protocol_is_rejected_before_discovery() {
    let mut request = request_with_home(None);
    request.protocol_version += 1;

    assert_eq!(
        discover(request).into_result().unwrap_err().code,
        WORKSPACE_PROTOCOL_UNSUPPORTED
    );
}

#[test]
fn escaping_default_member_is_fatal() {
    let response = discover(request_with_root_config(
        "[workspace]\ndefault_member = \"../outside\"\n",
    ));

    assert_eq!(
        response.into_result().unwrap_err().code,
        WORKSPACE_PATH_NOT_CONFINED
    );
}

#[test]
fn escaping_source_directory_is_fatal() {
    let response = discover(request_with_root_config(
        "[project]\nname = \"escape/root\"\nsource_directory = \"../outside\"\n",
    ));

    assert_eq!(
        response.into_result().unwrap_err().code,
        WORKSPACE_PATH_NOT_CONFINED
    );
}

#[test]
fn escaping_exclude_takes_priority_over_invalid_member_glob() {
    let response = discover(request_with_root_config(
        "[workspace]\nmembers = [\"[\"]\nexclude = [\"../outside/*\"]\n",
    ));

    assert_eq!(
        response.into_result().unwrap_err().code,
        WORKSPACE_PATH_NOT_CONFINED
    );
}

#[test]
fn confined_invalid_member_glob_has_stable_member_invalid_code() {
    let response = discover(request_with_root_config(
        "[workspace]\nmembers = [\"[\"]\nexclude = []\n",
    ));

    let error = response.into_result().unwrap_err();
    assert_eq!(error.code, WORKSPACE_MEMBER_INVALID);
    assert_eq!(error.path.unwrap().as_str(), "morphir.toml");
}

fn request_with_home(home_entries: Option<Value>) -> DiscoveryRequest {
    serde_json::from_value(json!({
        "protocolVersion": 1,
        "developmentRoot": { "entries": {
            "morphir.toml": {
                "kind": "file",
                "text": "[project]\nname = \"optional/root\"\nsource_directory = \"src\"\n"
            }
        } },
        "morphirHome": home_entries.map(|entries| json!({ "entries": entries })),
        "systemConfig": null,
        "environment": {},
        "cliOverlay": {}
    }))
    .unwrap()
}

fn request_with_root_config(text: &str) -> DiscoveryRequest {
    serde_json::from_value(json!({
        "protocolVersion": 1,
        "developmentRoot": { "entries": {
            "morphir.toml": { "kind": "file", "text": text }
        } },
        "morphirHome": null,
        "systemConfig": null,
        "environment": {},
        "cliOverlay": {}
    }))
    .unwrap()
}

fn project<'a>(snapshot: &'a WorkspaceSnapshot, path: &str) -> &'a ProjectSnapshot {
    snapshot
        .projects
        .iter()
        .find(|project| project.relative_path.as_str() == path)
        .unwrap()
}
