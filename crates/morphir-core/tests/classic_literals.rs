use morphir_core::ir::DecimalLiteral;
use morphir_core::ir::classic::literal::Literal;

#[test]
fn test_literal_bool() {
    let json = r#"["BoolLiteral", true]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    assert_eq!(lit, Literal::Bool(true));
}

#[test]
fn test_literal_string() {
    let json = r#"["StringLiteral", "hello"]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    assert_eq!(lit, Literal::String("hello".to_string()));
}

#[test]
fn test_literal_whole_number() {
    let json = r#"["WholeNumberLiteral", 123]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    assert_eq!(lit, Literal::WholeNumber(123));
}

#[test]
fn test_literal_int_legacy() {
    let json = r#"["IntLiteral", 123]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    assert_eq!(lit, Literal::WholeNumber(123));
}

#[test]
fn test_literal_float() {
    let json = r#"["FloatLiteral", 1.23]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    assert_eq!(lit, Literal::Float(1.23));
}

#[test]
fn a_v3_decimal_literal_keeps_its_text() {
    // MCK versions-0006: morphir-elm encodes DecimalLiteral as Decimal.toString.
    let decoded: Literal = serde_json::from_str(r#"["DecimalLiteral", "10.50"]"#).unwrap();
    assert_eq!(
        decoded,
        Literal::Decimal(DecimalLiteral::parse("10.50").unwrap())
    );
    assert_eq!(
        serde_json::to_string(&decoded).unwrap(),
        r#"["DecimalLiteral","10.50"]"#
    );
}

#[test]
fn a_v3_decimal_that_is_not_a_decimal_lexeme_is_refused() {
    assert!(serde_json::from_str::<Literal>(r#"["DecimalLiteral", "ten"]"#).is_err());
}
