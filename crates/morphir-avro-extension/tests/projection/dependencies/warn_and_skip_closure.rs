use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn warn_and_skip_removes_unsupported_reverse_dependents_and_keeps_closed_artifacts() {
    let bad_source = "acme/customer:customer#bad";
    let dependent_source = "acme/customer:customer#dependent";
    let good_source = "acme/customer:customer#good";
    let mut input = package(vec![
        TypeDeclaration::Opaque {
            source_name: bad_source.to_owned(),
            name: "bad".to_owned(),
            type_params: Vec::new(),
            doc: None,
        },
        alias(
            dependent_source,
            "dependent",
            TypeExpr::Record(vec![field("bad", reference(bad_source, vec![]))]),
        ),
        alias(good_source, "good", TypeExpr::Record(Vec::new())),
    ]);
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#bad-message",
            "bad-message",
            vec![],
            Some(reference(dependent_source, vec![])),
            ValueKind::Function,
            None,
        ),
        value_specification(
            "acme/customer:customer#good-message",
            "good-message",
            vec![],
            Some(reference(good_source, vec![])),
            ValueKind::Function,
            None,
        ),
    ];

    let strict = project(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            ..AvroOptions::default()
        },
    )
    .unwrap_err()
    .into_diagnostics()
    .unwrap();
    assert_eq!(
        strict
            .iter()
            .map(|error| error.source())
            .collect::<Vec<_>>(),
        [
            Some("acme/customer:customer#bad"),
            Some("acme/customer:customer#bad-message"),
            Some("acme/customer:customer#dependent"),
        ]
    );

    let partial = project(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            unsupported: Unsupported::WarnAndSkip,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert!(partial.root(bad_source).is_none());
    assert!(partial.root(dependent_source).is_none());
    assert!(partial.root(good_source).is_some());
    let protocol = partial.only_protocol().unwrap();
    assert!(protocol.message("badMessage").is_none());
    assert!(protocol.message("goodMessage").is_some());
    assert_eq!(
        protocol
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["acme.customer.customer.Good"]
    );
    assert_eq!(
        partial
            .diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.source(),
                diagnostic.code(),
                diagnostic.severity()
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some("acme/customer:customer#bad"),
                "AVRO001",
                morphir_extension_sdk::DiagnosticSeverity::Warning,
            ),
            (
                Some("acme/customer:customer#bad-message"),
                "AVRO001",
                morphir_extension_sdk::DiagnosticSeverity::Warning,
            ),
            (
                Some("acme/customer:customer#dependent"),
                "AVRO001",
                morphir_extension_sdk::DiagnosticSeverity::Warning,
            ),
        ]
    );
    let mep_warning = partial.diagnostics()[0].clone().into_diagnostic();
    assert_eq!(
        mep_warning.severity,
        morphir_extension_sdk::DiagnosticSeverity::Warning
    );
    assert_eq!(mep_warning.code.as_deref(), Some("AVRO001"));
    assert_eq!(
        mep_warning.location.unwrap().uri,
        "morphir-fqname:acme/customer:customer#bad"
    );
}
