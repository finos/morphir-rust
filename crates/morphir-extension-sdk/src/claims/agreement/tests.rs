use super::*;
use serde_json::json;

fn a_claims() -> Value {
    json!({
        "claimsVersion": "0.1.0-draft.2",
        "protocolVersions": [crate::protocol::MEP_VERSION, "0.2"],
        "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": ["frontend", "workspace"]},
        "capabilities": {
            "frontend": {"compile": true, "languages": [{"id": "elm", "fileExtensions": [".elm"]}], "irVersions": ["3"]},
            "workspace": {"discover": true, "protocolVersions": ["0.1.0-draft.1"]}
        }
    })
}

#[test]
fn sessions_can_omit_kinds_and_nested_members_but_not_change_arrays() {
    let claims: CapabilityClaimSet = serde_json::from_value(a_claims()).unwrap();
    let mut extension = claims.extension.clone();
    extension.types.truncate(1);
    let less = json!({"frontend": {"compile": true}});
    assert!(
        claims
            .check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                less.as_object().unwrap()
            )
            .is_ok()
    );
    let changed = json!({"frontend": {"irVersions": []}});
    assert!(matches!(
        claims.check_session(
            crate::protocol::MEP_VERSION,
            &extension,
            changed.as_object().unwrap()
        ),
        Err(SessionAgreementError::CapabilityMember(_))
    ));
}

#[test]
fn agreement_refuses_the_first_failed_rule() {
    let claims: CapabilityClaimSet = serde_json::from_value(a_claims()).unwrap();
    let mut extension = claims.extension.clone();
    for field in ["id", "name", "version"] {
        let mut wire = serde_json::to_value(&extension).unwrap();
        wire[field] = json!("different");
        let changed = serde_json::from_value(wire).unwrap();
        assert!(matches!(
            claims.check_session("unsupported", &changed, &claims.capabilities),
            Err(SessionAgreementError::Identity(_))
        ));
    }
    assert!(matches!(
        claims.check_session("unsupported", &extension, &claims.capabilities),
        Err(SessionAgreementError::ProtocolVersion(_))
    ));
    extension.types.push(crate::ExtensionType::Backend);
    assert!(matches!(
        claims.check_session(
            crate::protocol::MEP_VERSION,
            &extension,
            &claims.capabilities
        ),
        Err(SessionAgreementError::CapabilityKind(_))
    ));
    extension.types.pop();
    for reported in [
        json!({"frontend":{"compile":false}}),
        json!({"frontend":{"extra":true}}),
        json!({"backend":{}}),
    ] {
        assert!(matches!(
            claims.check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                reported.as_object().unwrap()
            ),
            Err(SessionAgreementError::CapabilityMember(_))
        ));
    }
    assert!(
        claims
            .check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                &claims.capabilities
            )
            .is_ok()
    );
}

#[test]
fn direct_claim_sets_must_match_in_both_directions() {
    let declared: CapabilityClaimSet = serde_json::from_value(a_claims()).unwrap();
    assert!(declared.check_claims(&declared).is_ok());
    for (member, replacement) in [
        ("protocolVersions", json!([crate::protocol::MEP_VERSION])),
        ("capabilities", json!({"frontend": {"compile": true}})),
        ("critical", json!(["capabilities.frontend.compile"])),
    ] {
        let mut wire = a_claims();
        wire[member] = replacement;
        let reported: CapabilityClaimSet = serde_json::from_value(wire).unwrap();
        assert!(
            declared
                .check_claims(&reported)
                .unwrap_err()
                .to_string()
                .contains(member)
        );
        assert!(reported.check_claims(&declared).is_err());
    }
    let mut reported = declared.clone();
    reported.capabilities["frontend"]["compile"] = false.into();
    assert!(
        declared
            .check_claims(&reported)
            .unwrap_err()
            .to_string()
            .contains("capabilities.frontend.compile")
    );
}

fn model_claims() -> CapabilityClaimSet {
    let mut wire = a_claims();
    wire["protocolVersions"] = json!(["0.1", "0.2.0-draft.1"]);
    wire["capabilities"]["modelTransport"] = json!({
        "tuples": [
            {"operation":"morphir.backend.generateModel", "profile":"morphir-model", "codec":"ion-binary", "envelopeVersion":"0.1.0-draft.1", "irVersion":"4.0.0"},
            {"operation":"morphir.frontend.compileModel", "profile":"morphir-ir", "codec":"morphir-json", "envelopeVersion":"0.1.0-draft.1", "irVersion":"4.0.0"}
        ],
        "limits":{"maxEncodedBytes":1024,"maxDecodedBytes":768,"maxDepth":32,"maxNodes":128},
        "guarantees":["urn:example:checked","urn:example:typed"]
    });
    serde_json::from_value(wire).unwrap()
}

#[test]
fn new_draft_session_can_narrow_only_model_transport() {
    let claims = model_claims();
    let mut reported = claims.capabilities.clone();
    reported["modelTransport"]["tuples"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    reported["modelTransport"]["guarantees"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    reported["modelTransport"]["limits"]["maxDepth"] = json!(16);
    assert!(
        claims
            .check_session("0.2.0-draft.1", &claims.extension, &reported)
            .is_ok()
    );
    assert!(
        claims
            .check_session("0.1", &claims.extension, &reported)
            .is_err()
    );
    reported["frontend"]["irVersions"] = json!([]);
    assert!(
        claims
            .check_session("0.2.0-draft.1", &claims.extension, &reported)
            .is_err()
    );
    let mut smaller = claims.clone();
    smaller.capabilities = reported;
    assert!(claims.check_claims(&smaller).is_err());
}

#[test]
fn model_transport_unknown_members_cannot_bypass_claim_agreement() {
    let mut claims = model_claims();
    claims.capabilities["modelTransport"]["future"] = json!({"enabled":true});
    claims.capabilities["modelTransport"]["tuples"][0]["futureTuple"] = json!("known");
    claims.capabilities["modelTransport"]["limits"]["futureLimit"] = json!(4);
    for (path, replacement) in [
        (vec!["future", "enabled"], json!(false)),
        (vec!["tuples", "0", "futureTuple"], json!("changed")),
        (vec!["limits", "futureLimit"], json!(3)),
    ] {
        let mut reported = claims.capabilities.clone();
        let mut value = &mut reported["modelTransport"];
        for key in path {
            value = if key == "0" {
                &mut value[0]
            } else {
                &mut value[key]
            };
        }
        *value = replacement;
        assert!(
            claims
                .check_session("0.2.0-draft.1", &claims.extension, &reported)
                .is_err()
        );
    }
    assert!(
        claims
            .check_session("0.2.0-draft.1", &claims.extension, &claims.capabilities)
            .is_ok()
    );
}

#[test]
fn session_refuses_raised_zero_duplicate_and_unclaimed_model_support() {
    let claims = model_claims();
    for replacement in [json!(0), json!(129), json!(2147483648_u64)] {
        let mut reported = claims.capabilities.clone();
        reported["modelTransport"]["limits"]["maxNodes"] = replacement;
        assert!(
            claims
                .check_session("0.2.0-draft.1", &claims.extension, &reported)
                .is_err()
        );
    }
    for duplicate in ["tuples", "guarantees"] {
        let mut reported = claims.capabilities.clone();
        let array = reported["modelTransport"][duplicate]
            .as_array_mut()
            .unwrap();
        array.push(array[0].clone());
        assert!(
            claims
                .check_session("0.2.0-draft.1", &claims.extension, &reported)
                .is_err()
        );
    }
    let mut reported = claims.capabilities.clone();
    reported["modelTransport"]["tuples"] = json!([]);
    reported["modelTransport"]["limits"]["maxNodes"] = json!(129);
    assert!(
        claims
            .check_session("0.2.0-draft.1", &claims.extension, &reported)
            .is_err()
    );
    let mut no_model_claim = claims.clone();
    no_model_claim.capabilities.remove("modelTransport");
    assert!(
        no_model_claim
            .check_session("0.2.0-draft.1", &claims.extension, &claims.capabilities)
            .is_err()
    );
    let mut reported = claims.capabilities.clone();
    reported["modelTransport"]["unclaimed"] = json!(true);
    assert!(
        claims
            .check_session("0.2.0-draft.1", &claims.extension, &reported)
            .is_err()
    );
}
