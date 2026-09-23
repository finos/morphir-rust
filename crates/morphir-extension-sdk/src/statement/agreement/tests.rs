use super::*;
use serde_json::json;

fn a_statement() -> Value {
    json!({
        "statementVersion": "0.1.0-draft.1",
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
    let statement: CapabilityStatement = serde_json::from_value(a_statement()).unwrap();
    let mut extension = statement.extension.clone();
    extension.types.truncate(1);
    let less = json!({"frontend": {"compile": true}});
    assert!(
        statement
            .check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                less.as_object().unwrap()
            )
            .is_ok()
    );
    let changed = json!({"frontend": {"irVersions": []}});
    assert!(matches!(
        statement.check_session(
            crate::protocol::MEP_VERSION,
            &extension,
            changed.as_object().unwrap()
        ),
        Err(SessionAgreementError::CapabilityMember(_))
    ));
}

#[test]
fn agreement_refuses_the_first_failed_rule() {
    let statement: CapabilityStatement = serde_json::from_value(a_statement()).unwrap();
    let mut extension = statement.extension.clone();
    for field in ["id", "name", "version"] {
        let mut wire = serde_json::to_value(&extension).unwrap();
        wire[field] = json!("different");
        let changed = serde_json::from_value(wire).unwrap();
        assert!(matches!(
            statement.check_session("unsupported", &changed, &statement.capabilities),
            Err(SessionAgreementError::Identity(_))
        ));
    }
    assert!(matches!(
        statement.check_session("unsupported", &extension, &statement.capabilities),
        Err(SessionAgreementError::ProtocolVersion(_))
    ));
    extension.types.push(crate::ExtensionType::Backend);
    assert!(matches!(
        statement.check_session(
            crate::protocol::MEP_VERSION,
            &extension,
            &statement.capabilities
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
            statement.check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                reported.as_object().unwrap()
            ),
            Err(SessionAgreementError::CapabilityMember(_))
        ));
    }
    assert!(
        statement
            .check_session(
                crate::protocol::MEP_VERSION,
                &extension,
                &statement.capabilities
            )
            .is_ok()
    );
}

#[test]
fn direct_statements_must_match_in_both_directions() {
    let declared: CapabilityStatement = serde_json::from_value(a_statement()).unwrap();
    assert!(declared.check_statement(&declared).is_ok());
    for (member, replacement) in [
        ("protocolVersions", json!([crate::protocol::MEP_VERSION])),
        ("capabilities", json!({"frontend": {"compile": true}})),
        ("critical", json!(["capabilities.frontend.compile"])),
    ] {
        let mut wire = a_statement();
        wire[member] = replacement;
        let reported: CapabilityStatement = serde_json::from_value(wire).unwrap();
        assert!(
            declared
                .check_statement(&reported)
                .unwrap_err()
                .to_string()
                .contains(member)
        );
        assert!(reported.check_statement(&declared).is_err());
    }
    let mut reported = declared.clone();
    reported.capabilities["frontend"]["compile"] = false.into();
    assert!(
        declared
            .check_statement(&reported)
            .unwrap_err()
            .to_string()
            .contains("capabilities.frontend.compile")
    );
}
