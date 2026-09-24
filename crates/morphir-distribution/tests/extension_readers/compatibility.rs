use super::*;

// Compatibility fixtures intentionally retain the released draft.1 vocabulary.
fn a_draft_one_document() -> Value {
    let mut value = a_claims();
    value.as_object_mut().unwrap().remove("claimsVersion");
    value["statementVersion"] = json!("0.1.0-draft.1");
    value["futureDocumentMember"] = json!({"preserved": true});
    value
}

fn a_draft_one_artifact() -> Value {
    let mut value = an_old_release()["artifacts"][0].clone();
    value["statement"] = a_draft_one_document();
    value["statementSource"] = json!("declared");
    value["critical"] = json!(["statement.capabilities.backend.generate"]);
    value
}

fn a_draft_one_catalog() -> Value {
    let mut entry = an_old_installed_record();
    entry["statement"] = a_draft_one_document();
    entry["statementSource"] = json!("declared");
    entry["critical"] = json!(["statement.capabilities.backend.generate"]);
    json!({"schemaVersion": "2.0.0-draft.1", "extensions": [entry],
        "critical": ["extensions.statement.capabilities.backend.generate"]})
}

fn assert_current_record(written: &Value) {
    assert_eq!(written["claims"]["claimsVersion"], "0.1.0-draft.2");
    assert_eq!(written["claimCheck"], "unchecked");
    assert_eq!(written["claims"]["futureDocumentMember"]["preserved"], true);
    assert_eq!(
        written["critical"],
        json!(["claims.capabilities.backend.generate"])
    );
    assert!(written.get("statement").is_none());
    assert!(written.get("statementSource").is_none());
    assert!(written["claims"].get("statementVersion").is_none());
}

#[test]
fn draft_one_catalog_loads_unchecked_claims_and_converts_critical_paths() {
    let catalog = read_catalog(a_draft_one_catalog()).unwrap();
    let entry = catalog.entries().next().unwrap();
    assert_eq!(
        entry.claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    assert_eq!(entry.claims().extension.id, "sample");
    let written = serde_json::to_value(entry).unwrap();
    assert_current_record(&written);
    let reread: InstalledExtension = serde_json::from_value(written.clone()).unwrap();
    assert_eq!(serde_json::to_value(reread).unwrap(), written);
}

#[test]
fn draft_one_index_loads_unchecked_claims_and_writes_draft_two() {
    let mut value = an_old_release();
    value["schemaVersion"] = json!("2.0.0-draft.1");
    value["artifacts"][0] = a_draft_one_artifact();
    value["critical"] = json!(["artifacts.statement.capabilities.backend.generate"]);
    let record: ReleaseRecord = serde_json::from_value(value).unwrap();
    assert_eq!(
        record.artifacts()[0].claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    assert_eq!(
        record.artifacts()[0].claims().unwrap().extension.id,
        "sample"
    );
    let written = serde_json::to_value(record).unwrap();
    assert_current_record(&written["artifacts"][0]);
    assert_eq!(written["schemaVersion"], "2.0.0-draft.2");
    assert_eq!(
        written["critical"],
        json!(["artifacts.claims.capabilities.backend.generate"])
    );
    let reread: ReleaseRecord = serde_json::from_value(written.clone()).unwrap();
    assert_eq!(serde_json::to_value(reread).unwrap(), written);
}

#[test]
fn draft_one_bundle_loads_unchecked_claims_and_writes_draft_two() {
    let value = json!({"schemaVersion":"2.0.0-draft.1", "extensionId":"sample",
        "shortId":"sample", "version":"1.0.0", "artifacts":[a_draft_one_artifact()],
        "critical":["artifacts.statement.capabilities.backend.generate"]});
    let descriptor: morphir_distribution::ReleaseBundleDescriptor =
        serde_json::from_value(value).unwrap();
    assert_eq!(
        descriptor.artifacts()[0].claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    assert_eq!(descriptor.artifacts()[0].claims().extension.id, "sample");
    let written = serde_json::to_value(descriptor).unwrap();
    assert_current_record(&written["artifacts"][0]);
    assert_eq!(written["schemaVersion"], "2.0.0-draft.2");
    assert_eq!(
        written["critical"],
        json!(["artifacts.claims.capabilities.backend.generate"])
    );
    let reread: morphir_distribution::ReleaseBundleDescriptor =
        serde_json::from_value(written.clone()).unwrap();
    assert_eq!(serde_json::to_value(reread).unwrap(), written);
}

#[test]
fn all_envelopes_refuse_mixed_draft_members() {
    for schema in ["2.0.0-draft.1", "2.0.0-draft.2"] {
        for mixed in [false, true] {
            let mut artifact = a_draft_one_artifact();
            let mut catalog = a_draft_one_catalog();
            if schema == "2.0.0-draft.1" || mixed {
                for record in [&mut artifact, &mut catalog["extensions"][0]] {
                    record["claims"] = a_claims();
                    record["claimCheck"] = json!("unchecked");
                    if !mixed {
                        record.as_object_mut().unwrap().remove("statement");
                        record.as_object_mut().unwrap().remove("statementSource");
                    }
                }
            }
            catalog["schemaVersion"] = json!(schema);
            let mut index = an_old_release();
            index["schemaVersion"] = json!(schema);
            index["artifacts"][0] = artifact.clone();
            let bundle = json!({"schemaVersion":schema,"extensionId":"sample","shortId":"sample",
                "version":"1.0.0","artifacts":[artifact]});
            for error in [
                read_catalog(catalog).unwrap_err().to_string(),
                serde_json::from_value::<ReleaseRecord>(index)
                    .unwrap_err()
                    .to_string(),
                serde_json::from_value::<morphir_distribution::ReleaseBundleDescriptor>(bundle)
                    .unwrap_err()
                    .to_string(),
            ] {
                assert!(error.contains("mixed draft record"), "{error}");
            }
        }
    }
}

#[test]
fn draft_one_record_refuses_draft_two_critical_version_member() {
    let mut old = a_draft_one_catalog();
    old["extensions"][0]["critical"] = json!(["statement.claimsVersion"]);
    let error = read_catalog(old).unwrap_err();
    assert!(error.to_string().contains("critical"), "{error}");
}

#[test]
fn draft_one_probe_route_and_check_survive_conversion() {
    for route in ["describe", "session-fallback"] {
        let mut old = a_draft_one_catalog();
        old["extensions"][0]["statementSource"] = json!("probed");
        old["extensions"][0]["probeSource"] = json!(route);
        let catalog = read_catalog(old).unwrap();
        let entry = catalog.entries().next().unwrap();
        assert_eq!(
            entry.claim_check(),
            morphir_distribution::ClaimCheck::Probed
        );
        assert_eq!(serde_json::to_value(entry.probe_source()).unwrap(), route);
        let written = serde_json::to_value(entry).unwrap();
        assert_eq!(written["claimCheck"], "probed");
        assert_eq!(written["probeSource"], route);
    }
}
