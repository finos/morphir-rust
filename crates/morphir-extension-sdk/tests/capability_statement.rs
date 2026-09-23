use morphir_extension_sdk::statement::CapabilityStatement;
use serde_json::{Value, json};

fn a_statement() -> Value {
    json!({
        "statementVersion": "0.1.0-draft.1",
        "protocolVersions": ["0.1", "0.2"],
        "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": ["frontend", "workspace"]},
        "capabilities": {
            "frontend": {"compile": true, "languages": [{"id": "elm", "fileExtensions": [".elm"]}], "irVersions": ["3"]},
            "workspace": {"discover": true, "protocolVersions": ["0.1.0-draft.1"]}
        }
    })
}

#[test]
fn unknown_optional_members_are_ignored_and_capabilities_are_preserved() {
    let mut wire = a_statement();
    wire["future"] = json!({"anything": 42});
    wire["extension"]["future"] = json!(true);
    wire["capabilities"]["frontend"]["future"] = json!({"nested": [1, 2]});
    let statement: CapabilityStatement = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(statement).unwrap();
    assert_eq!(encoded["capabilities"], wire["capabilities"]);
    assert_eq!(encoded["statementVersion"], "0.1.0-draft.1");
}

#[test]
fn unknown_critical_paths_are_refused_by_name() {
    for path in ["future", "capabilities.frontend.future", "requires.future"] {
        let mut wire = a_statement();
        wire["critical"] = json!([path]);
        let error = serde_json::from_value::<CapabilityStatement>(wire).unwrap_err();
        assert!(error.to_string().contains(path), "{error}");
    }
}

#[test]
fn only_the_exact_statement_draft_is_accepted() {
    for version in ["0.1", "0.1.0", "0.1.0-draft.2", "0.2.0-draft.1", "1.0.0"] {
        let mut wire = a_statement();
        wire["statementVersion"] = json!(version);
        assert!(
            serde_json::from_value::<CapabilityStatement>(wire).is_err(),
            "{version}"
        );
    }
}

#[test]
fn unknown_capability_kinds_are_refused() {
    let mut wire = a_statement();
    wire["extension"]["types"] = json!(["future"]);
    assert!(serde_json::from_value::<CapabilityStatement>(wire).is_err());
}

#[test]
fn known_critical_members_and_single_host_comparators_are_supported() {
    let mut wire = a_statement();
    wire["critical"] = json!(["requires.host", "capabilities.frontend.compile"]);
    wire["requires"] = json!({"host": [">=0.4.0-alpha.7", "<0.5.0"]});
    let statement: CapabilityStatement = serde_json::from_value(wire.clone()).unwrap();
    assert!(
        statement
            .check_host(&"0.4.0-alpha.8".parse().unwrap())
            .is_ok()
    );
    assert!(
        statement
            .check_host(&"0.5.0".parse().unwrap())
            .unwrap_err()
            .to_string()
            .contains("<0.5.0")
    );
    wire["requires"]["host"] = json!([">=0.4.0, <0.5.0"]);
    assert!(serde_json::from_value::<CapabilityStatement>(wire).is_err());
}

#[test]
fn absent_host_comparators_do_not_restrict_prerelease_hosts() {
    let mut wire = a_statement();
    for requirements in [json!({}), json!({"host":[]}), json!({"future":true})] {
        wire["requires"] = requirements;
        let statement: CapabilityStatement = serde_json::from_value(wire.clone()).unwrap();
        assert!(
            statement
                .check_host(&"1.0.0-alpha.1".parse().unwrap())
                .is_ok()
        );
    }
}

#[test]
fn every_frontend_member_the_sdk_reads_can_be_critical() {
    for member in [
        "languages",
        "irVersions",
        "compile",
        "incremental",
        "fragments",
        "multiDocument",
    ] {
        let mut wire = a_statement();
        wire["critical"] = json!([format!("capabilities.frontend.{member}")]);
        assert!(
            serde_json::from_value::<CapabilityStatement>(wire).is_ok(),
            "{member}"
        );
    }
}
