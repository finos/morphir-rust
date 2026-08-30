use morphir_core::ir::v4::{
    Field, Pattern, RecordFieldEntry, Type, TypeAttributes, TypeEncoding, Value, ValueAttributes,
    with_type_encoding,
};
use morphir_core::naming::{FQName, Name};

fn reference() -> Type {
    Type::Reference(
        TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:basics#int").unwrap(),
        Vec::new(),
    )
}

#[test]
fn compact_encoding_uses_type_shorthand() {
    let encoded = with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_string(&reference()).unwrap()
    });
    assert_eq!(encoded, r#""morphir/SDK:basics#int""#);
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
        FQName::from_canonical_string("morphir/SDK:list#list").unwrap(),
        vec![Type::Variable(TypeAttributes::default(), Name::from("a"))],
    );
    let encoded = with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(&value).unwrap()
    });
    assert_eq!(
        encoded,
        serde_json::json!({"Reference": ["morphir/SDK:list#list", "a"]})
    );
    assert_eq!(serde_json::from_value::<Type>(encoded).unwrap(), value);
}

#[test]
fn checked_in_complete_v4_example_decodes_concretely() {
    serde_json::from_str::<morphir_core::ir::v4::IRFile>(include_str!(
        "fixtures/ir/v4/complete-example.json"
    ))
    .unwrap();
}

#[test]
fn canonical_acronym_names_round_trip_through_manual_v4_visitors() {
    let acronym = Name::new(&["u", "s", "d"]);
    let attrs = TypeAttributes::default();
    let types = [
        Type::Variable(attrs.clone(), acronym.clone()),
        Type::Record(
            attrs.clone(),
            vec![Field {
                name: acronym.clone(),
                tpe: Type::Unit(attrs.clone()),
            }],
        ),
        Type::ExtensibleRecord(
            attrs,
            acronym.clone(),
            vec![Field {
                name: acronym.clone(),
                tpe: Type::Unit(TypeAttributes::default()),
            }],
        ),
    ];
    for value in types {
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<Type>(encoded).unwrap(), value);
    }

    let value_attrs = ValueAttributes::default();
    let pattern = Pattern::AsPattern(
        value_attrs.clone(),
        Box::new(Pattern::WildcardPattern(value_attrs.clone())),
        acronym.clone(),
    );
    let encoded = serde_json::to_value(&pattern).unwrap();
    assert_eq!(serde_json::from_value::<Pattern>(encoded).unwrap(), pattern);

    let values = [
        Value::Variable(value_attrs.clone(), acronym.clone()),
        Value::Record(
            value_attrs.clone(),
            vec![RecordFieldEntry(
                acronym.clone(),
                Value::Unit(value_attrs.clone()),
            )],
        ),
        Value::Field(
            value_attrs.clone(),
            Box::new(Value::Unit(value_attrs.clone())),
            acronym.clone(),
        ),
        Value::FieldFunction(value_attrs, acronym),
    ];
    for value in values {
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(serde_json::from_value::<Value>(encoded).unwrap(), value);
    }
}
