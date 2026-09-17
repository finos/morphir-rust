//! V4 IntegerLiteral tests
//!
//! Tests for IntegerLiteral (replacing WholeNumberLiteral) against the V4 specification at
//! https://morphir.finos.org/docs/spec/ir/schemas/v4/whats-new/

use morphir_core::ir::Literal;
use num_bigint::BigInt;

#[test]
fn test_integer_literal_serialize() {
    let lit = Literal::integer(42);

    let json = serde_json::to_string(&lit).unwrap();

    // V4 uses IntegerLiteral
    assert!(json.contains("IntegerLiteral"));
}

#[test]
fn test_integer_literal_deserialize_v4_format() {
    // V4 canonical format: a single-member wrapper carrying the value directly
    let json = r#"{ "IntegerLiteral": 42 }"#;

    let lit: Literal = serde_json::from_str(json).unwrap();

    match lit {
        Literal::Integer(n) => assert_eq!(n, BigInt::from(42)),
        _ => panic!("Expected Integer variant"),
    }
}

#[test]
fn test_integer_literal_deserialize_legacy_whole_number() {
    // The spelling IntegerLiteral replaced - still accepted on input, never written
    let json = r#"{ "WholeNumberLiteral": 99 }"#;

    let lit: Literal = serde_json::from_str(json).unwrap();

    match lit {
        Literal::Integer(n) => assert_eq!(n, BigInt::from(99)),
        _ => panic!("Expected Integer variant"),
    }
}

#[test]
fn test_negative_integer_literal() {
    // V4 correctly supports negative integers (unlike "whole number" which implies non-negative)
    let lit = Literal::integer(-100);

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    match parsed {
        Literal::Integer(n) => assert_eq!(n, BigInt::from(-100)),
        _ => panic!("Expected Integer variant"),
    }
}

#[test]
fn test_large_integer_literal() {
    let lit = Literal::integer(i64::MAX);

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    match parsed {
        Literal::Integer(n) => assert_eq!(n, BigInt::from(i64::MAX)),
        _ => panic!("Expected Integer variant"),
    }
}

#[test]
fn test_zero_integer_literal() {
    let lit = Literal::integer(0);

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    match parsed {
        Literal::Integer(n) => assert_eq!(n, BigInt::from(0)),
        _ => panic!("Expected Integer variant"),
    }
}

#[test]
fn test_bool_literal_round_trip() {
    let lit = Literal::bool(true);

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    assert_eq!(lit, parsed);
}

#[test]
fn test_string_literal_round_trip() {
    let lit = Literal::string("Hello, Morphir!");

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    assert_eq!(lit, parsed);
}

#[test]
fn test_float_literal_round_trip() {
    let lit = Literal::float(1.23456);

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    assert_eq!(lit, parsed);
}

#[test]
fn test_decimal_literal_round_trip() {
    let lit = Literal::decimal("123456789.987654321").unwrap();

    let json = serde_json::to_string(&lit).unwrap();
    let parsed: Literal = serde_json::from_str(&json).unwrap();

    assert_eq!(lit, parsed);
}
