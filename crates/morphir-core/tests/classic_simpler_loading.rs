use morphir_core::ir::classic::literal::Literal;
use morphir_core::ir::classic::naming::{Name, Path, FQName};
use morphir_core::ir::classic::types::Type;
use morphir_core::ir::classic::value::Value;
use morphir_core::ir::classic::pattern::Pattern;
use serde_json::json;

#[test]
fn test_simpler_name_loading() {
    let json = json!(["foo","bar"]);
    let name: Name = serde_json::from_value(json).expect("Failed to parse Name");
    assert_eq!(name.words, vec!["foo", "bar"]);
}

#[test]
fn test_simpler_literal_loading() {
    let json_bool = json!(["BoolLiteral", true]);
    let lit: Literal = serde_json::from_value(json_bool).expect("Failed to parse Bool");
    assert!(matches!(lit, Literal::Bool(true)));

    let json_int = json!(["WholeNumberLiteral", 42]);
    let lit: Literal = serde_json::from_value(json_int).expect("Failed to parse Int");
    assert!(matches!(lit, Literal::WholeNumber(42)));
}

#[test]
fn test_simpler_type_loading() {
    // Variable: ["Variable", attrs, name]
    let json_var = json!(["Variable", null, ["a"]]);
    let t: Type<()> = serde_json::from_value(json_var).expect("Failed to parse Variable");
    assert!(matches!(t, Type::Variable(_, _)));

    // Tuple: ["Tuple", attrs, elements]
    let json_tuple = json!(["Tuple", null, [["Variable", null, ["a"]], ["Variable", null, ["b"]]]]);
    let t: Type<()> = serde_json::from_value(json_tuple).expect("Failed to parse Tuple");
    assert!(matches!(t, Type::Tuple(_, _)));
}

#[test]
fn test_simpler_pattern_loading() {
    // Wildcard: ["WildcardPattern", attrs]
    let json_wild = json!(["WildcardPattern", null]);
    let p: Pattern<()> = serde_json::from_value(json_wild).expect("Failed to parse Wildcard");
    assert!(matches!(p, Pattern::Wildcard(_)));

    // AsPattern: ["AsPattern", attrs, pattern, name]
    let json_as = json!(["AsPattern", null, ["WildcardPattern", null], ["x"]]);
    let p: Pattern<()> = serde_json::from_value(json_as).expect("Failed to parse AsPattern");
    assert!(matches!(p, Pattern::As(_, _, _)));
}

#[test]
fn test_simpler_value_loading() {
    // Literal Value: ["Literal", attrs, literal]
    let json_lit = json!(["Literal", null, ["BoolLiteral", true]]);
    let v: Value<(), ()> = serde_json::from_value(json_lit).expect("Failed to parse Literal Value");
    assert!(matches!(v, Value::Literal(_, _)));

    // Apply: ["Apply", attrs, func, arg]
    let json_apply = json!(["Apply", null, ["Variable", null, ["f"]], ["Variable", null, ["x"]]]);
    let v: Value<(), ()> = serde_json::from_value(json_apply).expect("Failed to parse Apply");
    assert!(matches!(v, Value::Apply(_, _, _)));
}
