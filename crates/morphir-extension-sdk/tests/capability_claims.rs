use morphir_extension_sdk::claims::CapabilityClaimSet;
use serde_json::{Value, json};

fn a_claims() -> Value {
    json!({
        "claimsVersion": "0.1.0-draft.2",
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
    let mut wire = a_claims();
    wire["future"] = json!({"anything": 42});
    wire["extension"]["future"] = json!(true);
    wire["capabilities"]["frontend"]["future"] = json!({"nested": [1, 2]});
    let claims: CapabilityClaimSet = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(claims).unwrap();
    assert_eq!(encoded["capabilities"], wire["capabilities"]);
    assert_eq!(encoded["claimsVersion"], "0.1.0-draft.2");
}

#[test]
fn unknown_critical_paths_are_refused_by_name() {
    for path in ["future", "capabilities.frontend.future", "requires.future"] {
        let mut wire = a_claims();
        wire["critical"] = json!([path]);
        let error = serde_json::from_value::<CapabilityClaimSet>(wire).unwrap_err();
        assert!(error.to_string().contains(path), "{error}");
    }
}

#[test]
fn only_the_exact_claims_draft_is_accepted() {
    for version in ["0.1", "0.1.0", "0.1.0-draft.3", "0.2.0-draft.1", "1.0.0"] {
        let mut wire = a_claims();
        wire["claimsVersion"] = json!(version);
        assert!(
            serde_json::from_value::<CapabilityClaimSet>(wire).is_err(),
            "{version}"
        );
    }
}

#[test]
fn unknown_capability_kinds_are_refused() {
    let mut wire = a_claims();
    wire["extension"]["types"] = json!(["future"]);
    assert!(serde_json::from_value::<CapabilityClaimSet>(wire).is_err());
}

#[test]
fn known_critical_members_and_single_host_comparators_are_supported() {
    let mut wire = a_claims();
    wire["critical"] = json!(["requires.host", "capabilities.frontend.compile"]);
    wire["requires"] = json!({"host": [">=0.4.0-alpha.7", "<0.5.0"]});
    let claims: CapabilityClaimSet = serde_json::from_value(wire.clone()).unwrap();
    assert!(claims.check_host(&"0.4.0-alpha.8".parse().unwrap()).is_ok());
    assert!(
        claims
            .check_host(&"0.5.0".parse().unwrap())
            .unwrap_err()
            .to_string()
            .contains("<0.5.0")
    );
    wire["requires"]["host"] = json!([">=0.4.0, <0.5.0"]);
    assert!(serde_json::from_value::<CapabilityClaimSet>(wire).is_err());
}

#[test]
fn absent_host_comparators_do_not_restrict_prerelease_hosts() {
    let mut wire = a_claims();
    for requirements in [json!({}), json!({"host":[]}), json!({"future":true})] {
        wire["requires"] = requirements;
        let claims: CapabilityClaimSet = serde_json::from_value(wire.clone()).unwrap();
        assert!(claims.check_host(&"1.0.0-alpha.1".parse().unwrap()).is_ok());
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
        let mut wire = a_claims();
        wire["critical"] = json!([format!("capabilities.frontend.{member}")]);
        assert!(
            serde_json::from_value::<CapabilityClaimSet>(wire).is_ok(),
            "{member}"
        );
    }
}

#[test]
fn draft_one_document_is_rewritten_as_draft_two() {
    let mut old = a_claims();
    old.as_object_mut().unwrap().remove("claimsVersion");
    old["statementVersion"] = json!("0.1.0-draft.1");
    old["critical"] = json!(["statementVersion"]);
    let parsed: CapabilityClaimSet = serde_json::from_value(old).unwrap();
    let written = serde_json::to_value(parsed).unwrap();
    assert_eq!(written["claimsVersion"], "0.1.0-draft.2");
    assert_eq!(written["critical"], json!(["claimsVersion"]));
    assert!(written.get("statementVersion").is_none());
}

#[test]
fn both_exact_drafts_accept_build_metadata_and_write_current_version() {
    for (member, version) in [
        ("statementVersion", "0.1.0-draft.1+guest.42"),
        ("claimsVersion", "0.1.0-draft.2+guest.42"),
    ] {
        let mut value = a_claims();
        value.as_object_mut().unwrap().remove("claimsVersion");
        value[member] = json!(version);
        let parsed: CapabilityClaimSet = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.claims_version().to_string(), "0.1.0-draft.2");
    }
}

#[test]
fn mixed_document_versions_and_members_are_refused() {
    for (member, version) in [
        ("statementVersion", "0.1.0-draft.2"),
        ("claimsVersion", "0.1.0-draft.1"),
        ("statementVersion", "0.1.0-draft.3"),
    ] {
        let mut value = a_claims();
        value.as_object_mut().unwrap().remove("claimsVersion");
        value[member] = json!(version);
        let error = serde_json::from_value::<CapabilityClaimSet>(value).unwrap_err();
        assert!(error.to_string().contains("claimsVersion"), "{error}");
    }
    let mut value = a_claims();
    value["statementVersion"] = json!("0.1.0-draft.1");
    assert!(
        serde_json::from_value::<CapabilityClaimSet>(value)
            .unwrap_err()
            .to_string()
            .contains("mixed")
    );
}

#[test]
fn draft_one_document_refuses_draft_two_critical_vocabulary() {
    let mut old = a_claims();
    old.as_object_mut().unwrap().remove("claimsVersion");
    old["statementVersion"] = json!("0.1.0-draft.1");
    old["critical"] = json!(["claimsVersion"]);
    let error = serde_json::from_value::<CapabilityClaimSet>(old).unwrap_err();
    assert!(error.to_string().contains("critical"), "{error}");
}
