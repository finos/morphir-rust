//! V4 NativeHint serialization tests
//!
//! Tests for NativeHint variants against the V4 specification at
//! https://morphir.finos.org/docs/spec/ir/schemas/v4/whats-new/

use morphir_core::ir::v4::NativeHint;
use morphir_core::ir::{Diagnostic, DiagnosticCode};

/// The diagnostic a refused hint carries. A hint decoded on its own is entered at the root, so
/// the cursors below are relative to the hint itself.
fn refusal(json: &str) -> Diagnostic {
    let error =
        serde_json::from_str::<NativeHint>(json).expect_err("this spelling is not a native hint");
    Diagnostic::from_serde_error(&error).expect("a v4 refusal carries a diagnostic")
}

#[test]
fn test_native_hint_arithmetic_serialize() {
    let hint = NativeHint::Arithmetic;

    let json = serde_json::to_string(&hint).unwrap();

    assert!(json.contains("\"Arithmetic\""));
}

#[test]
fn test_native_hint_arithmetic_deserialize() {
    let json = r#"{"Arithmetic": {}}"#;

    let hint: NativeHint = serde_json::from_str(json).unwrap();

    assert!(matches!(hint, NativeHint::Arithmetic));
}

#[test]
fn test_native_hint_comparison_round_trip() {
    let original = NativeHint::Comparison;

    let json = serde_json::to_string(&original).unwrap();
    let parsed: NativeHint = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

#[test]
fn test_native_hint_string_op_round_trip() {
    let original = NativeHint::StringOp;

    let json = serde_json::to_string(&original).unwrap();
    let parsed: NativeHint = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

#[test]
fn test_native_hint_collection_op_round_trip() {
    let original = NativeHint::CollectionOp;

    let json = serde_json::to_string(&original).unwrap();
    let parsed: NativeHint = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

#[test]
fn test_native_hint_platform_specific_serialize() {
    let hint = NativeHint::PlatformSpecific {
        platform: "wasm".to_string(),
    };

    let json = serde_json::to_string(&hint).unwrap();

    assert!(json.contains("\"PlatformSpecific\""));
    assert!(json.contains("\"platform\""));
    assert!(json.contains("\"wasm\""));
}

#[test]
fn test_native_hint_platform_specific_deserialize() {
    let json = r#"{"PlatformSpecific": {"platform": "javascript"}}"#;

    let hint: NativeHint = serde_json::from_str(json).unwrap();

    match hint {
        NativeHint::PlatformSpecific { platform } => {
            assert_eq!(platform, "javascript");
        }
        _ => panic!("Expected PlatformSpecific variant"),
    }
}

#[test]
fn test_native_hint_platform_specific_round_trip() {
    let original = NativeHint::PlatformSpecific {
        platform: "native".to_string(),
    };

    let json = serde_json::to_string(&original).unwrap();
    let parsed: NativeHint = serde_json::from_str(&json).unwrap();

    assert_eq!(original, parsed);
}

// A hint is the wrapper object and nothing else: the bare tag `"Arithmetic"` is not one, and
// `PlatformSpecific` names its platform rather than having one invented for it
// (definitions-0009, 0030).
#[test]
fn test_native_hint_bare_tag_is_not_a_hint() {
    for bare in [r#""Arithmetic""#, r#""PlatformSpecific""#] {
        let refused = refusal(bare);
        assert_eq!(refused.code, DiagnosticCode::InvalidType);
        assert_eq!(refused.cursor, "");
    }
}

#[test]
fn test_native_hint_platform_specific_requires_a_platform() {
    let refused = refusal(r#"{"PlatformSpecific": {}}"#);
    assert_eq!(refused.code, DiagnosticCode::MissingMember);
    assert_eq!(refused.cursor, "/PlatformSpecific");
}
