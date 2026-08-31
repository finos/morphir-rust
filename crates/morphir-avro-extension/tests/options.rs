use std::collections::HashMap;

use morphir_avro_extension::{
    Aliases, AvroOptions, Dependencies, Projection, Representation, TypeMapping, Unsupported,
};
use morphir_extension_sdk::DiagnosticSeverity;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn options(entries: impl IntoIterator<Item = (&'static str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[test]
fn an_empty_option_map_uses_the_documented_defaults() {
    let actual = AvroOptions::from_map(&HashMap::new()).expect("empty options should decode");

    assert_eq!(actual.representation, Representation::Json);
    assert_eq!(actual.projection, Projection::Schemas);
    assert_eq!(actual.dependencies, Dependencies::SelfContained);
    assert_eq!(actual.aliases, Aliases::Inline);
    assert_eq!(actual.unsupported, Unsupported::Error);
    assert!(actual.logical_types);
    assert_eq!(actual.decimal_precision, 38);
    assert_eq!(actual.decimal_scale, 10);
    assert!(actual.type_mappings.is_empty());
}

#[test]
fn accepted_enum_spellings_decode() {
    #[derive(Debug)]
    enum Expected {
        Representation(Representation),
        Projection(Projection),
        Dependencies(Dependencies),
        Aliases(Aliases),
        Unsupported(Unsupported),
    }

    let cases = [
        (
            "representation",
            json!("json"),
            Expected::Representation(Representation::Json),
        ),
        (
            "representation",
            json!("idl"),
            Expected::Representation(Representation::Idl),
        ),
        (
            "projection",
            json!("schemas"),
            Expected::Projection(Projection::Schemas),
        ),
        (
            "projection",
            json!("protocol-entry-points"),
            Expected::Projection(Projection::ProtocolEntryPoints),
        ),
        (
            "projection",
            json!("protocol-public"),
            Expected::Projection(Projection::ProtocolPublic),
        ),
        (
            "dependencies",
            json!("self-contained"),
            Expected::Dependencies(Dependencies::SelfContained),
        ),
        (
            "dependencies",
            json!("linked"),
            Expected::Dependencies(Dependencies::Linked),
        ),
        (
            "aliases",
            json!("inline"),
            Expected::Aliases(Aliases::Inline),
        ),
        (
            "aliases",
            json!("wrapper-record"),
            Expected::Aliases(Aliases::WrapperRecord),
        ),
        (
            "unsupported",
            json!("error"),
            Expected::Unsupported(Unsupported::Error),
        ),
        (
            "unsupported",
            json!("warn-and-skip"),
            Expected::Unsupported(Unsupported::WarnAndSkip),
        ),
    ];

    for (key, value, expected) in cases {
        let actual = AvroOptions::from_map(&options([(key, value)]))
            .unwrap_or_else(|error| panic!("{key} should accept its spelling: {error}"));
        match expected {
            Expected::Representation(expected) => assert_eq!(actual.representation, expected),
            Expected::Projection(expected) => assert_eq!(actual.projection, expected),
            Expected::Dependencies(expected) => assert_eq!(actual.dependencies, expected),
            Expected::Aliases(expected) => assert_eq!(actual.aliases, expected),
            Expected::Unsupported(expected) => assert_eq!(actual.unsupported, expected),
        }
    }
}

#[test]
fn json_values_preserve_their_types_when_decoded_from_a_map() {
    let actual = AvroOptions::from_map(&options([
        ("logical_types", json!(false)),
        ("decimal_precision", json!(18)),
        (
            "type_mappings",
            json!({
                "Example.Amount": {
                    "type": "bytes",
                    "logical_type": "decimal",
                    "precision": 18,
                    "scale": 4
                }
            }),
        ),
    ]))
    .expect("JSON values should retain their native types");

    assert!(!actual.logical_types);
    assert_eq!(actual.decimal_precision, 18);
    assert_eq!(
        actual.type_mappings.get("Example.Amount"),
        Some(&TypeMapping {
            physical_type: "bytes".to_owned(),
            logical_type: Some("decimal".to_owned()),
            precision: Some(18),
            scale: Some(4),
        })
    );
}

#[test]
fn nested_type_mappings_are_keyed_by_morphir_fqn() {
    let actual = AvroOptions::from_map(&options([(
        "type_mappings",
        json!({
            "Acme.Payments.Money": { "type": "string" }
        }),
    )]))
    .expect("mapping should decode");

    assert_eq!(
        actual.type_mappings["Acme.Payments.Money"],
        TypeMapping {
            physical_type: "string".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        }
    );
}

#[test]
fn unknown_top_level_option_is_an_avro004_error() {
    let error = AvroOptions::from_map(&options([("typo", json!(true))])).unwrap_err();

    assert_eq!(error.code(), "AVRO004");
    assert!(error.message().contains("typo"));
}

#[test]
fn top_level_unknown_option_errors_are_deterministic_across_map_insertion_order() {
    let mut first = HashMap::new();
    first.insert("zebra".to_owned(), json!(true));
    first.insert("alpha".to_owned(), json!(true));
    let mut second = HashMap::new();
    second.insert("alpha".to_owned(), json!(true));
    second.insert("zebra".to_owned(), json!(true));

    let first = AvroOptions::from_map(&first).unwrap_err();
    let second = AvroOptions::from_map(&second).unwrap_err();

    assert_eq!(first.code(), "AVRO004");
    assert_eq!(first.message(), second.message());
    assert!(first.message().contains("alpha"));
}

#[test]
fn unknown_nested_mapping_option_is_an_avro004_error() {
    let error = AvroOptions::from_map(&options([(
        "type_mappings",
        json!({ "Acme.Money": { "type": "bytes", "typo": true } }),
    )]))
    .unwrap_err();

    assert_eq!(error.code(), "AVRO004");
    assert!(error.message().contains("typo"));
}

#[test]
fn nested_unknown_option_errors_are_deterministic_across_map_insertion_order() {
    fn mapping_with_unknowns(first: &str, second: &str) -> HashMap<String, Value> {
        let mut mapping = serde_json::Map::new();
        mapping.insert("type".to_owned(), json!("bytes"));
        mapping.insert(first.to_owned(), json!(true));
        mapping.insert(second.to_owned(), json!(true));
        HashMap::from([(
            "type_mappings".to_owned(),
            json!({ "Acme.Money": Value::Object(mapping) }),
        )])
    }

    let first = AvroOptions::from_map(&mapping_with_unknowns("zebra", "alpha")).unwrap_err();
    let second = AvroOptions::from_map(&mapping_with_unknowns("alpha", "zebra")).unwrap_err();

    assert_eq!(first.code(), "AVRO004");
    assert_eq!(first.message(), second.message());
    assert!(first.message().contains("alpha"));
}

#[test]
fn invalid_global_decimal_ranges_are_an_avro004_error() {
    for values in [
        options([("decimal_precision", json!(0))]),
        options([("decimal_precision", json!(3)), ("decimal_scale", json!(4))]),
    ] {
        let error = AvroOptions::from_map(&values).unwrap_err();
        assert_eq!(error.code(), "AVRO004");
    }
}

#[test]
fn invalid_mapping_decimal_ranges_use_global_defaults_for_omitted_values() {
    for mapping in [
        json!({ "type": "bytes", "precision": 0 }),
        json!({ "type": "bytes", "precision": 3 }),
        json!({ "type": "bytes", "precision": 10 }),
        json!({ "type": "bytes", "scale": 39 }),
    ] {
        let defaults = if mapping["precision"] == json!(10) {
            vec![("decimal_scale", json!(11))]
        } else {
            Vec::new()
        };
        let values = defaults
            .into_iter()
            .chain([("type_mappings", json!({ "Acme.Money": mapping }))]);
        let error = AvroOptions::from_map(&options(values)).unwrap_err();
        assert_eq!(error.code(), "AVRO004");
    }
}

#[test]
fn validate_rejects_invalid_public_options_constructed_without_from_map() {
    let options = AvroOptions {
        decimal_precision: 0,
        ..AvroOptions::default()
    };

    let error = options.validate().unwrap_err();

    assert_eq!(error.code(), "AVRO004");
    assert!(error.message().contains("decimal_precision"));
}

#[test]
fn diagnostic_conversion_uses_the_requested_severity_without_context() {
    let error = AvroOptions::from_map(&options([("typo", json!(true))])).unwrap_err();

    for severity in [DiagnosticSeverity::Error, DiagnosticSeverity::Warning] {
        let diagnostic = error.clone().into_diagnostic(severity);
        assert_eq!(diagnostic.severity, severity);
        assert_eq!(diagnostic.code.as_deref(), Some("AVRO004"));
        assert_eq!(diagnostic.message, error.message());
        assert!(diagnostic.location.is_none());
        assert!(diagnostic.related.is_empty());
    }
    assert!(error.source().is_none());
}

#[test]
fn malformed_values_and_enum_spellings_are_an_avro004_error() {
    for values in [
        options([("logical_types", json!("true"))]),
        options([("decimal_precision", json!("38"))]),
        options([("representation", json!("avsc"))]),
        options([("projection", json!("all"))]),
    ] {
        let error = AvroOptions::from_map(&values).unwrap_err();
        assert_eq!(error.code(), "AVRO004");
        assert!(!error.message().is_empty());
    }
}
