use morphir_core::ir::classic::literal::Literal;
use morphir_core::ir::classic::naming::Name;
use morphir_core::ir::classic::value::Value;

#[test]
fn test_value_variable() {
    let json = r#"["Variable", null, ["x"]]"#;
    let v: Value<(), ()> = serde_json::from_str(json).expect("Failed to parse Variable");
    match v {
        Value::Variable(_, name) => assert_eq!(name, Name::from_str("x")),
        _ => panic!("Expected Variable"),
    }
}

#[test]
fn test_value_literal() {
    let json = r#"["Literal", null, ["WholeNumberLiteral", 42]]"#;
    let v: Value<(), ()> = serde_json::from_str(json).expect("Failed to parse Literal");
    match v {
        Value::Literal(_, lit) => assert_eq!(lit, Literal::WholeNumber(42)),
        _ => panic!("Expected Literal"),
    }
}

#[test]
fn test_value_reference() {
    // Reference in Value position
    // ["Reference", attributes, FQName]  <-- Note: Value::Reference has different structure than Type::Reference (no args)

    let json = r#"["Reference", null, [[["p"]],[["m"]],["v"]]]"#;
    let v: Value<(), ()> = serde_json::from_str(json).expect("Failed to parse Reference");
    match v {
        Value::Reference(_, fqname) => {
            assert_eq!(fqname.local_name, Name::from_str("v"));
        }
        _ => panic!("Expected Reference"),
    }
}

#[test]
fn test_value_apply() {
    // Apply(VA, func, arg)
    let json = r#"["Apply", null, ["Variable", null, ["f"]], ["Variable", null, ["x"]]]"#;
    let v: Value<(), ()> = serde_json::from_str(json).expect("Failed to parse Apply");
    match v {
        Value::Apply(_, func, arg) => {
            match *func {
                Value::Variable(_, name) => assert_eq!(name, Name::from_str("f")),
                _ => panic!("Expected Variable func"),
            }
            match *arg {
                Value::Variable(_, name) => assert_eq!(name, Name::from_str("x")),
                _ => panic!("Expected Variable arg"),
            }
        }
        _ => panic!("Expected Apply"),
    }
}

#[test]
fn test_value_constructor() {
    let json = r#"["Constructor", null, [[["p"]],[["m"]],["C"]]]"#;
    let v: Value<(), ()> = serde_json::from_str(json).expect("Failed to parse Constructor");
    match v {
        Value::Constructor(_, fqname) => {
            assert_eq!(fqname.local_name, Name::new(vec!["C".to_string()]));
        }
        _ => panic!("Expected Constructor"),
    }
}
