use serde_json::{Map, json};

use super::*;
use crate::{
    AvroUnion, Properties,
    avro::{EnumSchema, FixedSchema},
};

#[test]
fn renders_every_type_expression_form() {
    let package =
        AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
    let renderer = JsonRenderer::new(&package, Dependencies::SelfContained);
    let named = AvroFullName::new("example.types".to_owned(), "Named".to_owned()).unwrap();
    let cases = [
        (AvroType::Null, json!("null")),
        (AvroType::Boolean, json!("boolean")),
        (AvroType::Int, json!("int")),
        (AvroType::Long, json!("long")),
        (AvroType::Float, json!("float")),
        (AvroType::Double, json!("double")),
        (AvroType::Bytes, json!("bytes")),
        (AvroType::String, json!("string")),
        (
            AvroType::Array(Box::new(AvroType::String), Properties::new()),
            json!({"type": "array", "items": "string"}),
        ),
        (
            AvroType::Map(Box::new(AvroType::Long), Properties::new()),
            json!({"type": "map", "values": "long"}),
        ),
        (
            AvroType::Union(AvroUnion::new(vec![AvroType::Null, AvroType::String]).unwrap()),
            json!(["null", "string"]),
        ),
        (AvroType::Named(named), json!("example.types.Named")),
        (
            AvroType::Logical {
                physical: Box::new(AvroType::Bytes),
                name: "decimal".to_owned(),
                properties: Properties::from([
                    ("precision".to_owned(), json!(12)),
                    ("scale".to_owned(), json!(2)),
                ]),
            },
            json!({
                "type": "bytes",
                "logicalType": "decimal",
                "precision": 12,
                "scale": 2
            }),
        ),
        (
            AvroType::Annotated {
                physical: Box::new(AvroType::String),
                properties: Properties::from([("morphir.type-name".to_owned(), json!("Char"))]),
            },
            json!({"type": "string", "morphir.type-name": "Char"}),
        ),
    ];

    for (tpe, expected) in cases {
        assert_eq!(renderer.render_type_reference_only(&tpe), expected);
    }
}

#[test]
fn renders_enum_and_fixed_standard_members_with_custom_properties() {
    let package =
        AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
    let renderer = JsonRenderer::new(&package, Dependencies::SelfContained);
    let enum_schema = NamedSchema::Enum(
        EnumSchema::new(
            AvroFullName::new("example.types".to_owned(), "Status".to_owned()).unwrap(),
            vec!["Pending".to_owned(), "Active".to_owned()],
            Some("Lifecycle status.".to_owned()),
            Properties::from([("morphir.source-kind".to_owned(), json!("custom"))]),
        )
        .unwrap(),
    );
    assert_eq!(
        renderer.render_named_reference_only(&enum_schema),
        json!({
            "type": "enum",
            "name": "Status",
            "namespace": "example.types",
            "symbols": ["Active", "Pending"],
            "doc": "Lifecycle status.",
            "morphir.source-kind": "custom"
        })
    );

    let fixed_schema = NamedSchema::Fixed(
        FixedSchema::new(
            AvroFullName::new("example.types".to_owned(), "Hash".to_owned()).unwrap(),
            32,
            Some("SHA-256 bytes.".to_owned()),
            Properties::from([("morphir.format".to_owned(), json!("sha-256"))]),
        )
        .unwrap(),
    );
    assert_eq!(
        renderer.render_named_reference_only(&fixed_schema),
        json!({
            "type": "fixed",
            "name": "Hash",
            "namespace": "example.types",
            "size": 32,
            "doc": "SHA-256 bytes.",
            "morphir.format": "sha-256"
        })
    );
}

#[test]
fn canonicalization_sorts_nested_object_keys_without_reordering_arrays() {
    let value = Value::Object(Map::from_iter([
        (
            "z".to_owned(),
            Value::Object(Map::from_iter([
                ("second".to_owned(), json!(2)),
                ("first".to_owned(), json!(1)),
            ])),
        ),
        ("a".to_owned(), json!([{"z": 1, "a": 2}, "last"])),
    ]));
    let rendered = serde_json::to_string(&canonicalize(value)).unwrap();
    assert_eq!(
        rendered,
        r#"{"a":[{"a":2,"z":1},"last"],"z":{"first":1,"second":2}}"#
    );
}
