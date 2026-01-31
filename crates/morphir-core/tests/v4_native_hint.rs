//! V4 NativeHint serialization tests
//!
//! Tests for NativeHint variants against the V4 specification at
//! https://morphir.finos.org/docs/spec/ir/schemas/v4/whats-new/

use morphir_core::ir::v4::NativeHint;

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

// Backward compatibility - accept legacy string format
#[test]
fn test_native_hint_legacy_string_arithmetic() {
    let json = r#""Arithmetic""#;

    let hint: NativeHint = serde_json::from_str(json).unwrap();

    assert!(matches!(hint, NativeHint::Arithmetic));
}

#[test]
fn test_native_hint_legacy_string_platform_specific() {
    // Legacy string format should provide default platform
    let json = r#""PlatformSpecific""#;

    let hint: NativeHint = serde_json::from_str(json).unwrap();

    match hint {
        NativeHint::PlatformSpecific { platform } => {
            assert_eq!(platform, "unknown");
        }
        _ => panic!("Expected PlatformSpecific variant"),
    }
}
