use morphir_core::ir::v4::{
    Documentation, TypeDefinition, ValueAttributes, ValueBody, ValueDefinition,
};

#[test]
fn incomplete_type_preserves_partial_expression() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/ir/v4/incomplete-type-definition-example.json"
    ))
    .unwrap();
    let value = fixture["examples"]["draftWithPartialBody"].clone();

    let decoded: TypeDefinition = serde_json::from_value(value).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();

    assert!(encoded["IncompleteTypeDefinition"]["partialTypeExp"].is_object());
}

#[test]
fn documentation_accepts_lines_and_normalizes_crlf() {
    let doc: Documentation = serde_json::from_str(r#"["line one\r","line two"]"#).unwrap();

    assert_eq!(doc.lines(), &["line one", "line two"]);
    assert_eq!(
        serde_json::to_string(&doc).unwrap(),
        r#"["line one","line two"]"#
    );
}

#[test]
fn documentation_keeps_single_line_string_form() {
    let doc: Documentation = serde_json::from_str(r#""one line""#).unwrap();

    assert_eq!(doc.lines(), &["one line"]);
    assert_eq!(serde_json::to_string(&doc).unwrap(), r#""one line""#);
}

#[test]
fn value_attributes_hold_a_concrete_inferred_type() {
    let attrs: ValueAttributes =
        serde_json::from_str(r#"{"inferredType":"morphir/(sdk):basics#int"}"#).unwrap();

    assert!(attrs.inferred_type.is_some());
}

#[test]
fn incomplete_value_preserves_optional_output_and_partial_body() {
    let json = serde_json::json!({
        "inputTypes": {},
        "body": {
            "IncompleteBody": {
                "incompleteness": { "Draft": {} },
                "partialBody": { "Unit": {} }
            }
        }
    });

    let decoded: ValueDefinition = serde_json::from_value(json).unwrap();

    assert!(decoded.output_type.is_none());
    assert!(matches!(
        decoded.body,
        ValueBody::Incomplete {
            partial_body: Some(_),
            ..
        }
    ));
}
