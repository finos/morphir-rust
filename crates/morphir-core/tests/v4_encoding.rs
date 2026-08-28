use morphir_core::ir::v4::{Type, TypeAttributes, TypeEncoding, with_type_encoding};
use morphir_core::naming::{FQName, Name};

fn reference() -> Type {
    Type::Reference(
        TypeAttributes::default(),
        FQName::from_canonical_string("morphir/(sdk):basics#int").unwrap(),
        Vec::new(),
    )
}

#[test]
fn compact_encoding_uses_type_shorthand() {
    let encoded = with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_string(&reference()).unwrap()
    });
    assert_eq!(encoded, r#""morphir/(sdk):basics#int""#);
}

#[test]
fn expanded_encoding_preserves_explicit_type_nodes() {
    let encoded = with_type_encoding(TypeEncoding::Expanded, || {
        serde_json::to_value(reference()).unwrap()
    });
    assert!(encoded.get("Reference").unwrap().is_object());
}

#[test]
fn compact_parameterized_references_round_trip() {
    let value = Type::Reference(
        TypeAttributes::default(),
        FQName::from_canonical_string("morphir/(sdk):list#list").unwrap(),
        vec![Type::Variable(TypeAttributes::default(), Name::from("a"))],
    );
    let encoded = with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(&value).unwrap()
    });
    assert_eq!(
        encoded,
        serde_json::json!({"Reference": ["morphir/(sdk):list#list", "a"]})
    );
    assert_eq!(serde_json::from_value::<Type>(encoded).unwrap(), value);
}

#[test]
fn checked_in_complete_v4_example_decodes_concretely() {
    serde_json::from_str::<morphir_core::ir::v4::IRFile>(include_str!(
        "../../../../../website/static/ir/examples/v4/complete-example.json"
    ))
    .unwrap();
}
