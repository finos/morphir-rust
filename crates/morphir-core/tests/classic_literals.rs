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
fn test_literal_decimal() {
    let json = r#"["DecimalLiteral", "1.23"]"#;
    let lit: Literal = serde_json::from_str(json).unwrap();
    if let Literal::Float(f) = lit {
        assert!((f - 1.23).abs() < 1e-6);
    } else {
        panic!("Expected Float for DecimalLiteral");
    }
}
