use super::ReleaseBundleDescriptor;
use serde_json::{Value, json};

fn a_legacy_descriptor() -> Value {
    json!({
        "schemaVersion": 1, "shortId": "elm", "extensionId": "morphir-elm",
        "package": "morphir-elm-extension", "version": "0.3.0",
        "mepVersions": ["0.1"], "runtime": "wasm", "irVersions": ["3"],
        "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
        "incremental": true, "workspaceDiscovery": true,
        "artifact": "morphir-elm.wasm", "sha256": "a".repeat(64)
    })
}

fn a_statement() -> Value {
    json!({
        "statementVersion": "0.1.0-draft.1", "protocolVersions": ["0.1"],
        "extension": {"id": "morphir-elm", "name": "Elm", "version": "0.3.0", "types": ["frontend", "workspace"]},
        "capabilities": {"frontend": {"compile": true}, "workspace": {"discover": true}}
    })
}

fn a_v2_descriptor() -> Value {
    json!({
        "schemaVersion": "2.0.0-draft.1", "extensionId": "morphir-elm",
        "shortId": "elm", "version": "0.3.0", "platformDifferences": "none",
        "artifacts": [{"platform": "aarch64-apple-darwin", "runtime": "process",
            "filename": "morphir-elm.tgz", "sha256": "a".repeat(64), "statement": a_statement()}]
    })
}

#[test]
fn descriptor_reads_v2_without_flat_v1_fields() {
    let descriptor = serde_json::from_value::<ReleaseBundleDescriptor>(a_v2_descriptor()).unwrap();
    assert_eq!(
        descriptor.artifacts()[0].platform(),
        Some("aarch64-apple-darwin")
    );
    assert_eq!(
        descriptor.artifacts()[0].runtime(),
        crate::ArtifactRuntime::Process
    );
}

#[test]
fn descriptor_accepts_missing_version_as_v1() {
    let mut value = a_legacy_descriptor();
    value.as_object_mut().unwrap().remove("schemaVersion");
    serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap();
}

#[test]
fn descriptor_ignores_unknown_optional_members() {
    let mut value = a_legacy_descriptor();
    value["futureMetadata"] = json!({"enabled": true});
    serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap();
}

#[test]
fn descriptor_rejects_unknown_critical_by_name() {
    let mut value = a_legacy_descriptor();
    value["futureMeaning"] = json!(true);
    value["critical"] = json!(["futureMeaning"]);
    let error = serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown critical member 'futureMeaning'"),
        "{error}"
    );
}

#[test]
fn descriptor_parses_host_requirements_without_comparing_versions() {
    let mut value = a_legacy_descriptor();
    value["requires"] = json!({"host": [">=999.0.0"]});
    value["critical"] = json!(["requires.host"]);
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_ok());
}

#[test]
fn descriptor_converts_flat_frontend_workspace_to_declared_statement() {
    let descriptor: ReleaseBundleDescriptor =
        serde_json::from_value(a_legacy_descriptor()).unwrap();
    let artifact = &descriptor.artifacts()[0];
    assert_eq!(
        artifact.statement_provenance(),
        crate::extension_format::StatementProvenance::Declared
    );
    let statement = artifact.statement();
    assert_eq!(statement.extension.id, "morphir-elm");
    assert_eq!(statement.protocol_versions, ["0.1"]);
    assert_eq!(
        statement.capabilities["frontend"]["languages"][0]["id"],
        "elm"
    );
    assert_eq!(statement.capabilities["frontend"]["incremental"], true);
    assert!(
        statement
            .extension
            .types
            .contains(&morphir_extension_sdk::ExtensionType::Workspace)
    );
}

#[test]
fn descriptor_preserves_supplied_statement_and_provenance() {
    for source in ["declared", "probed"] {
        for mut value in [a_legacy_descriptor(), a_v2_descriptor()] {
            let target = if value.get("artifacts").is_some() {
                &mut value["artifacts"][0]
            } else {
                &mut value
            };
            let mut statement = a_statement();
            statement["capabilities"]["futureCapability"] = json!({"enabled": true});
            target["statement"] = statement.clone();
            target["statementSource"] = json!(source);
            let descriptor: ReleaseBundleDescriptor = serde_json::from_value(value).unwrap();
            let artifact = &descriptor.artifacts()[0];
            assert_eq!(
                serde_json::to_value(artifact.statement()).unwrap(),
                statement
            );
            assert_eq!(
                serde_json::to_value(artifact.statement_provenance()).unwrap(),
                source
            );
        }
    }
}

#[test]
fn descriptor_accepts_supported_versions_only() {
    let mut v2 = a_v2_descriptor();
    v2["schemaVersion"] = json!("2.0.0-draft.1+build.123");
    serde_json::from_value::<ReleaseBundleDescriptor>(v2).unwrap();
    for version in [json!(1), json!("1.0"), json!("1.7.8")] {
        let mut value = a_legacy_descriptor();
        value["schemaVersion"] = version;
        serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap();
    }
    for version in [
        json!(0),
        json!(2),
        json!("2.0.0"),
        json!("3.0.0"),
        json!("2.0.0-draft.2"),
        json!("1.1.0-draft.1"),
    ] {
        let mut value = a_legacy_descriptor();
        value["schemaVersion"] = version;
        assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
    }
}

#[test]
fn descriptor_validates_host_comparators_without_comparing_versions() {
    let mut value = a_v2_descriptor();
    value["requires"] = json!({"host": [">=0.1.0", "<999.0.0"]});
    let error = serde_json::from_value::<ReleaseBundleDescriptor>(value.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires.host must be listed in critical"),
        "{error}"
    );
    value["critical"] = json!(["requires.host"]);
    serde_json::from_value::<ReleaseBundleDescriptor>(value.clone()).unwrap();
    value["requires"]["host"] = json!([">=0.1.0", "<0.2.0"]);
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_ok());
}

#[test]
fn descriptor_ignores_nested_optional_members_but_refuses_nested_critical() {
    let mut value = a_v2_descriptor();
    value["artifacts"][0]["futureArtifact"] = json!(true);
    serde_json::from_value::<ReleaseBundleDescriptor>(value.clone()).unwrap();
    value["artifacts"][0]["critical"] = json!(["futureArtifact"]);
    let error = serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap_err();
    assert!(error.to_string().contains("futureArtifact"));
}

#[test]
fn descriptor_requires_v2_statements_and_valid_artifact_declarations() {
    let mut value = a_v2_descriptor();
    value["artifacts"][0]
        .as_object_mut()
        .unwrap()
        .remove("statement");
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
    let mut value = a_v2_descriptor();
    value["artifacts"] = json!([]);
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
    let mut value = a_v2_descriptor();
    value["artifacts"][0]["filename"] = json!("../escape.tgz");
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
}

#[test]
fn descriptor_v2_is_not_a_publication_candidate() {
    let descriptor: ReleaseBundleDescriptor = serde_json::from_value(a_v2_descriptor()).unwrap();
    let error = descriptor
        .into_legacy(std::path::Path::new("bundle"))
        .unwrap_err();
    assert!(error.to_string().contains("version-1 WASM bundle"));
}

#[test]
fn descriptor_recognizes_critical_artifact_paths() {
    let mut value = a_v2_descriptor();
    value["critical"] = json!([
        "artifacts.runtime",
        "artifacts.statement.capabilities.workspace.discover"
    ]);
    serde_json::from_value::<ReleaseBundleDescriptor>(value).unwrap();
}

#[test]
fn descriptor_artifact_reader_requires_a_statement() {
    let mut artifact = a_v2_descriptor()["artifacts"][0].clone();
    artifact.as_object_mut().unwrap().remove("statement");
    assert!(serde_json::from_value::<super::BundleArtifactDescriptor>(artifact).is_err());
}

#[test]
fn descriptor_rejects_unknown_platform_differences_and_wasm_platforms() {
    let mut value = a_v2_descriptor();
    value["platformDifferences"] = json!("arbitrary");
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
    let mut value = a_v2_descriptor();
    value["artifacts"][0]["runtime"] = json!("wasm");
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value.clone()).is_err());
    value["artifacts"][0]["platform"] = Value::Null;
    assert!(serde_json::from_value::<ReleaseBundleDescriptor>(value).is_err());
}

#[test]
fn descriptor_round_trips_supplied_statements_and_legacy_wire_shape() {
    let mut absent_version = a_legacy_descriptor();
    absent_version
        .as_object_mut()
        .unwrap()
        .remove("schemaVersion");
    for mut value in [a_legacy_descriptor(), absent_version, a_v2_descriptor()] {
        value["futureMetadata"] = json!({"retained": true});
        value["requires"] = json!({"host": [">=0.1.0"]});
        value["critical"] = json!(["requires.host"]);
        let target = if value.get("artifacts").is_some() {
            &mut value["artifacts"][0]
        } else {
            &mut value
        };
        target["statement"] = a_statement();
        target["statement"]["futureStatementMember"] = json!({"retained": true});
        target["statementSource"] = json!("probed");
        let descriptor: ReleaseBundleDescriptor = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(descriptor).unwrap(), value);
    }
    let value = a_legacy_descriptor();
    let descriptor: ReleaseBundleDescriptor = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(descriptor).unwrap(), value);
}

#[test]
fn descriptor_retains_distinct_statements_for_each_artifact() {
    let mut value = a_v2_descriptor();
    value["platformDifferences"] = json!("declared");
    let mut wasm = value["artifacts"][0].clone();
    wasm.as_object_mut().unwrap().remove("platform");
    wasm["runtime"] = json!("wasm");
    wasm["filename"] = json!("morphir-elm.wasm");
    wasm["statement"]["capabilities"]["frontend"]["incremental"] = json!(false);
    value["artifacts"].as_array_mut().unwrap().push(wasm);
    let descriptor =
        ReleaseBundleDescriptor::parse_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        descriptor.platform_differences(),
        Some(super::PlatformDifferences::Declared)
    );
    assert_eq!(descriptor.artifacts().len(), 2);
    assert_eq!(
        descriptor.artifacts()[1].runtime(),
        crate::ArtifactRuntime::Wasm
    );
    assert!(descriptor.artifacts()[1].platform().is_none());
    assert_eq!(
        descriptor.artifacts()[1].statement().capabilities["frontend"]["incremental"],
        false
    );
    assert!(
        descriptor.artifacts()[0].statement().capabilities["frontend"]
            .get("incremental")
            .is_none()
    );
}
