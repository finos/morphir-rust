//! V4 Incompleteness serialization tests
//!
//! Tests for Incompleteness enum variants against the V4 specification at
//! https://morphir.finos.org/docs/spec/ir/schemas/v4/whats-new/

use morphir_core::ir::v4::{HoleReason, Incompleteness};

#[test]
fn test_incompleteness_draft_serialize() {
    let incomp = Incompleteness::Draft;

    let json = serde_json::to_string(&incomp).unwrap();

    assert!(json.contains("\"Draft\""));
    assert!(json.contains("{}"));
}

#[test]
fn test_incompleteness_draft_deserialize() {
    let json = r#"{"Draft": {}}"#;

    let incomp: Incompleteness = serde_json::from_str(json).unwrap();

    assert!(matches!(incomp, Incompleteness::Draft));
}

#[test]
fn test_incompleteness_hole_with_unresolved_reference_serialize() {
    let incomp = Incompleteness::Hole {
        reason: HoleReason::UnresolvedReference {
            target: "acme/finance:ledger#calculate-balance".to_string(),
        },
    };

    let json = serde_json::to_string(&incomp).unwrap();

    assert!(json.contains("\"Hole\""));
    assert!(json.contains("\"reason\""));
    assert!(json.contains("\"UnresolvedReference\""));
}

#[test]
fn test_incompleteness_hole_with_unresolved_reference_deserialize() {
    let json =
        r#"{"Hole": {"reason": {"UnresolvedReference": {"target": "acme/finance:ledger#calc"}}}}"#;

    let incomp: Incompleteness = serde_json::from_str(json).unwrap();

    match incomp {
        Incompleteness::Hole { reason } => match reason {
            HoleReason::UnresolvedReference { target } => {
                assert_eq!(target, "acme/finance:ledger#calc");
            }
            _ => panic!("Expected UnresolvedReference reason"),
        },
        _ => panic!("Expected Hole variant"),
    }
}

#[test]
fn test_incompleteness_hole_with_type_mismatch() {
    let incomp = Incompleteness::Hole {
        reason: HoleReason::TypeMismatch {
            expected: "Int".to_string(),
            found: "String".to_string(),
        },
    };

    let json = serde_json::to_string(&incomp).unwrap();
    let parsed: Incompleteness = serde_json::from_str(&json).unwrap();

    assert_eq!(incomp, parsed);
}

#[test]
fn test_incompleteness_draft_round_trip() {
    let original = Incompleteness::Draft;

    let json = serde_json::to_string(&original).unwrap();
    let parsed: Incompleteness = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}
