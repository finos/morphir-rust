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
