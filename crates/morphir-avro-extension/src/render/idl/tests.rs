use super::*;
use crate::{AvroRequest, AvroRoot, Protocol, RecordSchema};

#[test]
fn protocols_keep_non_named_public_roots_as_schema_artifacts() {
    let root_name = AvroFullName::new("example".to_owned(), "UserId".to_owned()).unwrap();
    let root = AvroRoot::new(
        "example:domain#user-id".to_owned(),
        root_name,
        AvroType::String,
        None,
    )
    .unwrap();
    let protocol = Protocol::new(
        AvroFullName::new("example".to_owned(), "Domain".to_owned()).unwrap(),
        Vec::new(),
        Vec::new(),
        Properties::new(),
    )
    .unwrap();
    let package = AvroPackage::new(
        vec![root],
        Vec::new(),
        Vec::new(),
        vec![protocol],
        Vec::new(),
    )
    .unwrap();

    let paths = render_idl(&package, Dependencies::SelfContained)
        .unwrap()
        .into_iter()
        .map(|artifact| artifact.path)
        .collect::<Vec<_>>();

    assert_eq!(paths, ["example/UserIdSchemas.avdl", "example/Domain.avdl"]);
}

#[test]
fn missing_linked_graph_node_is_an_internal_error_during_validation() {
    let package =
        AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
    let mut renderer = IdlRenderer::new(&package, Dependencies::Linked);
    renderer.linked_names.insert("example.Missing".to_owned());

    assert!(matches!(
        renderer.validate_linked_graph(),
        Err(AvroGenerationError::Internal(_))
    ));
}

#[test]
fn missing_linked_graph_node_is_an_internal_error_during_rendering() {
    let package =
        AvroPackage::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap();
    let renderer = IdlRenderer::new(&package, Dependencies::Linked);
    let schema = NamedSchema::Record(
        RecordSchema::new(
            AvroFullName::new("example".to_owned(), "Missing".to_owned()).unwrap(),
            Vec::new(),
            None,
            Properties::new(),
        )
        .unwrap(),
    );

    assert!(matches!(
        renderer.render_linked_declaration(&schema),
        Err(AvroGenerationError::Internal(_))
    ));
}

#[test]
fn identifiers_are_case_sensitive_in_protocol_type_field_and_message_contexts() {
    let cases = [
        (
            ["protocol", "record", "string", "error"],
            ["`protocol`", "`record`", "`string`", "`error`"],
        ),
        (
            [
                "time_micros",
                "timestamp_micros",
                "local_timestamp_micros",
                "BigDecimal",
            ],
            [
                "time_micros",
                "timestamp_micros",
                "local_timestamp_micros",
                "BigDecimal",
            ],
        ),
    ];
    for (identifiers, expected) in cases {
        let content = render_identifier_contexts(identifiers);
        let contexts = [
            ("protocol", format!("protocol {}", expected[0])),
            ("type", format!("record {}", expected[1])),
            ("field", format!("string {};", expected[2])),
            (
                "message",
                format!("example.{} {}();", expected[1], expected[3]),
            ),
        ];
        for (context, expected) in contexts {
            assert!(
                content.contains(&expected),
                "missing {context} context {expected:?} in {content}"
            );
        }
    }
}

fn render_identifier_contexts(identifiers: [&str; 4]) -> String {
    let [
        protocol_identifier,
        type_identifier,
        field_identifier,
        message_identifier,
    ] = identifiers;
    let record_name = AvroFullName::new("example".to_owned(), type_identifier.to_owned()).unwrap();
    let record = NamedSchema::Record(
        RecordSchema::new(
            record_name.clone(),
            vec![
                AvroField::new(
                    field_identifier.to_owned(),
                    AvroType::String,
                    Properties::new(),
                )
                .unwrap(),
            ],
            None,
            Properties::new(),
        )
        .unwrap(),
    );
    let message = AvroMessage::new(
        message_identifier.to_owned(),
        AvroRequest::new(Vec::new()).unwrap(),
        AvroType::Named(record_name.clone()),
        Vec::new(),
        Properties::from([(
            "morphir.value-kind".to_owned(),
            Value::String("function".to_owned()),
        )]),
    )
    .unwrap();
    let protocol = Protocol::new(
        AvroFullName::new("example".to_owned(), protocol_identifier.to_owned()).unwrap(),
        vec![message],
        vec![AvroType::Named(record_name)],
        Properties::new(),
    )
    .unwrap();
    let package = AvroPackage::new(
        Vec::new(),
        vec![record],
        Vec::new(),
        vec![protocol],
        Vec::new(),
    )
    .unwrap();

    let artifacts = render_idl(&package, Dependencies::SelfContained).unwrap();
    assert_eq!(artifacts.len(), 1);
    artifacts.into_iter().next().unwrap().content
}

#[test]
fn logical_shorthand_requires_the_canonical_physical_type_and_keeps_custom_properties() {
    let cases = [
        (
            AvroType::Int,
            "date",
            Properties::from([(
                "morphir.fqname".to_owned(),
                Value::String("example:types#date".to_owned()),
            )]),
            "@morphir.fqname(\"example:types#date\") date",
        ),
        (
            AvroType::String,
            "uuid",
            Properties::from([(
                "morphir.fqname".to_owned(),
                Value::String("example:types#identifier".to_owned()),
            )]),
            "@morphir.fqname(\"example:types#identifier\") uuid",
        ),
        (
            AvroType::Bytes,
            "decimal",
            Properties::from([
                (
                    "morphir.fqname".to_owned(),
                    Value::String("example:types#amount".to_owned()),
                ),
                ("precision".to_owned(), Value::from(20)),
                ("scale".to_owned(), Value::from(4)),
            ]),
            "@morphir.fqname(\"example:types#amount\") decimal(20, 4)",
        ),
    ];

    for (physical, logical, properties, expected) in cases {
        let actual = render_type(&AvroType::Logical {
            physical: Box::new(physical),
            name: logical.to_owned(),
            properties,
        })
        .unwrap();
        assert_eq!(actual, expected, "logical type {logical}");
    }
}

#[test]
fn noncanonical_logical_mappings_keep_the_configured_physical_type() {
    let cases = [
        (AvroType::Long, "date", "long"),
        (AvroType::Bytes, "uuid", "bytes"),
        (AvroType::String, "decimal", "string"),
    ];

    for (physical, logical, expected_physical) in cases {
        let actual = render_type(&AvroType::Logical {
            physical: Box::new(physical),
            name: logical.to_owned(),
            properties: Properties::from([(
                "morphir.fqname".to_owned(),
                Value::String(format!("example:types#{logical}")),
            )]),
        })
        .unwrap();
        assert_eq!(
            actual,
            format!(
                "@logicalType(\"{logical}\") @morphir.fqname(\"example:types#{logical}\") {expected_physical}"
            )
        );
    }
}

#[test]
fn affected_protocols_lead_with_the_javacc_compatibility_notice() {
    let content = render_identifier_contexts(["example", "response", "input", "find"]);
    assert!(content.starts_with(
            "// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"
        ));
}

#[test]
fn documentation_and_annotation_strings_escape_idl_terminators_and_controls() {
    let mut doc = String::new();
    render_doc(
        &mut doc,
        "  ",
        Some("first */ line\\path\nsecond\u{0001}line"),
    );
    assert_eq!(
        doc,
        "  /**\n   * first * / line\\path\n   * second\\u0001line\n   */\n"
    );
    assert_eq!(
        json_value(&Value::String("line\\path\ncontrol\u{0001}".to_owned())).unwrap(),
        "\"line\\\\path\\ncontrol\\u0001\""
    );
}
