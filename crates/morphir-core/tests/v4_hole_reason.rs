//! V4 HoleReason serialization tests
//!
//! Tests for HoleReason variants against the V4 specification at
//! https://morphir.finos.org/docs/spec/ir/schemas/v4/whats-new/

use morphir_core::ir::v4::HoleReason;
use morphir_core::naming::FQName;

#[test]
fn test_hole_reason_unresolved_reference_serialize() {
    let target = FQName::from_canonical_string("my-org/my-package:domain/users#get-user").unwrap();
    let reason = HoleReason::UnresolvedReference { target };

    let json = serde_json::to_string(&reason).unwrap();

    // V4 format uses wrapper object with canonical FQName format (# separator)
    assert!(json.contains("\"UnresolvedReference\""));
    assert!(json.contains("\"target\""));
    assert!(json.contains("my-org/my-package:domain/users#get-user"));
}

#[test]
fn test_hole_reason_unresolved_reference_deserialize() {
    let json = r#"{"UnresolvedReference": {"target": "my-org/my-package:domain/users#get-user"}}"#;

    let reason: HoleReason = serde_json::from_str(json).unwrap();

    match reason {
        HoleReason::UnresolvedReference { target } => {
            assert_eq!(
                target.to_canonical_string(),
                "my-org/my-package:domain/users#get-user"
            );
        }
        _ => panic!("Expected UnresolvedReference variant"),
    }
}

#[test]
fn test_hole_reason_deleted_during_refactor_serialize() {
    let reason = HoleReason::DeletedDuringRefactor {
        tx_id: "refactor-2026-01-30-001".to_string(),
    };

    let json = serde_json::to_string(&reason).unwrap();

    assert!(json.contains("\"DeletedDuringRefactor\""));
    assert!(json.contains("\"tx-id\"")); // V4 uses kebab-case in JSON
    assert!(json.contains("refactor-2026-01-30-001"));
}

#[test]
fn test_hole_reason_deleted_during_refactor_deserialize() {
    let json = r#"{"DeletedDuringRefactor": {"tx-id": "refactor-2026-01-30-001"}}"#;

    let reason: HoleReason = serde_json::from_str(json).unwrap();

    match reason {
        HoleReason::DeletedDuringRefactor { tx_id } => {
            assert_eq!(tx_id, "refactor-2026-01-30-001");
        }
        _ => panic!("Expected DeletedDuringRefactor variant"),
    }
}

#[test]
fn test_hole_reason_type_mismatch_serialize() {
    let reason = HoleReason::TypeMismatch {
        expected: "morphir/sdk:basics#int".to_string(),
        found: "morphir/sdk:string#string".to_string(),
    };

    let json = serde_json::to_string(&reason).unwrap();

    assert!(json.contains("\"TypeMismatch\""));
    assert!(json.contains("\"expected\""));
    assert!(json.contains("\"found\""));
    assert!(json.contains("morphir/sdk:basics#int"));
    assert!(json.contains("morphir/sdk:string#string"));
}

#[test]
fn test_hole_reason_type_mismatch_deserialize() {
    let json = r#"{"TypeMismatch": {"expected": "Int", "found": "String"}}"#;

    let reason: HoleReason = serde_json::from_str(json).unwrap();

    match reason {
        HoleReason::TypeMismatch { expected, found } => {
            assert_eq!(expected, "Int");
            assert_eq!(found, "String");
        }
        _ => panic!("Expected TypeMismatch variant"),
    }
}

#[test]
fn test_hole_reason_draft_serialize() {
    let reason = HoleReason::Draft;

    let json = serde_json::to_string(&reason).unwrap();

    assert!(json.contains("\"Draft\""));
    assert!(json.contains("{}"));
}

#[test]
fn test_hole_reason_draft_deserialize() {
    let json = r#"{"Draft": {}}"#;

    let reason: HoleReason = serde_json::from_str(json).unwrap();

    assert!(matches!(reason, HoleReason::Draft));
}

#[test]
fn test_hole_reason_type_mismatch_round_trip() {
    let original = HoleReason::TypeMismatch {
        expected: "Int".to_string(),
        found: "String".to_string(),
    };

    let json = serde_json::to_string(&original).unwrap();
    let parsed: HoleReason = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

#[test]
fn test_hole_reason_unresolved_reference_round_trip() {
    let target = FQName::from_canonical_string("acme/finance:ledger#balance").unwrap();
    let original = HoleReason::UnresolvedReference { target };

    let json = serde_json::to_string(&original).unwrap();
    let parsed: HoleReason = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

#[test]
fn test_hole_reason_deleted_during_refactor_round_trip() {
    let original = HoleReason::DeletedDuringRefactor {
        tx_id: "tx-12345".to_string(),
    };

    let json = serde_json::to_string(&original).unwrap();
    let parsed: HoleReason = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}
