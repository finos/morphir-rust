use super::*;

fn a_frontend_record() -> Value {
    let mut entry = an_old_installed_record();
    entry.as_object_mut().unwrap().remove("backend");
    entry["capabilities"] = json!(["frontend"]);
    entry["frontend"] = json!({
        "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
        "irVersions": ["3"], "compile": true
    });
    entry
}

fn a_frontend_catalog(draft: &str, flags: Value) -> Value {
    let mut entry = a_frontend_record();
    let mut frontend = entry.as_object_mut().unwrap().remove("frontend").unwrap();
    frontend
        .as_object_mut()
        .unwrap()
        .extend(flags.as_object().unwrap().clone());
    let mut claims = a_claims();
    claims["extension"]["types"] = json!(["frontend"]);
    claims["capabilities"] = json!({"frontend": frontend});
    if draft == "1" {
        claims.as_object_mut().unwrap().remove("claimsVersion");
        claims["statementVersion"] = json!("0.1.0-draft.1");
        entry["statement"] = claims;
        entry["statementSource"] = json!("declared");
    } else {
        entry["claims"] = claims;
    }
    json!({"schemaVersion": format!("2.0.0-draft.{draft}"), "extensions": [entry]})
}

#[test]
fn installed_frontend_claims_keep_optional_flags_in_both_drafts() {
    for draft in ["1", "2"] {
        let catalog = read_catalog(a_frontend_catalog(
            draft,
            json!({
                "multiDocument": true, "fragments": true, "incremental": true,
                "futureFrontendMember": {"ignored": true}
            }),
        ))
        .unwrap();
        let frontend = catalog
            .entries()
            .next()
            .unwrap()
            .extension_capabilities()
            .frontend
            .unwrap();
        assert!(frontend.multi_document, "draft.{draft}");
        assert!(frontend.fragments, "draft.{draft}");
        assert!(frontend.incremental, "draft.{draft}");
    }
}

#[test]
fn installed_frontend_claims_default_optional_flags_to_false() {
    for draft in ["1", "2"] {
        for flags in [
            json!({}),
            json!({"multiDocument": false, "fragments": false, "incremental": false}),
        ] {
            let catalog = read_catalog(a_frontend_catalog(draft, flags)).unwrap();
            let frontend = catalog
                .entries()
                .next()
                .unwrap()
                .extension_capabilities()
                .frontend
                .unwrap();
            assert!(!frontend.multi_document);
            assert!(!frontend.fragments);
            assert!(!frontend.incremental);
        }
    }
}

#[test]
fn legacy_frontend_ignores_claim_only_flags_and_keeps_its_wire_shape() {
    let entry = a_frontend_record();
    let mut with_unknown = entry.clone();
    with_unknown["frontend"]["multiDocument"] = json!(true);
    with_unknown["frontend"]["fragments"] = json!(true);
    let catalog =
        read_catalog(json!({"schemaVersion": "1.0", "extensions": [with_unknown]})).unwrap();
    let installed = catalog.entries().next().unwrap();
    let frontend = installed.extension_capabilities().frontend.unwrap();
    assert!(!frontend.multi_document);
    assert!(!frontend.fragments);
    assert!(!frontend.incremental);
    assert_eq!(serde_json::to_value(installed).unwrap(), entry);
}
