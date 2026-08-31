use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn warn_and_skip_indexes_collisions_transactionally_and_is_input_order_independent() {
    fn colliding_package(reverse: bool) -> morphir_avro_extension::ProjectionPackage {
        let normalized_left = "acme/customer:customer#foo-bar";
        let normalized_right = "acme/customer:customer#foo_bar";
        let duplicate = "acme/customer:other#duplicate";
        let dependent = "acme/customer:customer#dependent";
        let good = "acme/customer:customer#good";
        let mut customer_types = vec![
            alias(normalized_left, "foo-bar", TypeExpr::Record(Vec::new())),
            alias(normalized_right, "foo_bar", TypeExpr::Record(Vec::new())),
            alias(
                dependent,
                "dependent",
                TypeExpr::Record(vec![field("conflict", reference(normalized_left, vec![]))]),
            ),
            alias(good, "good", TypeExpr::Record(Vec::new())),
            alias(duplicate, "duplicate-one", TypeExpr::Record(Vec::new())),
        ];
        let mut other_types = vec![alias(
            duplicate,
            "duplicate-two",
            TypeExpr::Record(Vec::new()),
        )];
        if reverse {
            customer_types.reverse();
            other_types.reverse();
        }
        let mut modules = vec![
            ProjectionModule {
                path: vec!["customer".to_owned()],
                types: customer_types,
                values: vec![
                    value_specification(
                        "acme/customer:customer#bad-message",
                        "bad-message",
                        vec![],
                        Some(reference(normalized_right, vec![])),
                        ValueKind::Function,
                        None,
                    ),
                    value_specification(
                        "acme/customer:customer#good-message",
                        "good-message",
                        vec![],
                        Some(reference(good, vec![])),
                        ValueKind::Function,
                        None,
                    ),
                ],
                doc: None,
            },
            ProjectionModule {
                path: vec!["other".to_owned()],
                types: other_types,
                values: Vec::new(),
                doc: None,
            },
        ];
        if reverse {
            modules.reverse();
        }
        morphir_avro_extension::ProjectionPackage {
            kind: DistributionKind::Library,
            package_name: "acme/customer".to_owned(),
            dependencies: Vec::new(),
            modules,
        }
    }

    let mut strict_options = AvroOptions {
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };
    strict_options.type_mappings.insert(
        "acme/customer:customer#foo-bar".to_owned(),
        TypeMapping {
            physical_type: "string".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        },
    );
    let first_errors = project(&colliding_package(false), &strict_options)
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    let second_errors = project(&colliding_package(true), &strict_options)
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    let signature = |errors: &[morphir_avro_extension::AvroDiagnostic]| {
        errors
            .iter()
            .map(|error| {
                (
                    error.source().map(str::to_owned),
                    error.code(),
                    error.message().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(signature(&first_errors), signature(&second_errors));
    assert_eq!(first_errors.len(), 6);
    assert!(
        first_errors
            .iter()
            .all(|diagnostic| diagnostic.code() == "AVRO003")
    );

    let partial_options = AvroOptions {
        unsupported: Unsupported::WarnAndSkip,
        ..strict_options.clone()
    };
    let first = project(&colliding_package(false), &partial_options).unwrap();
    let second = project(&colliding_package(true), &partial_options).unwrap();
    assert_eq!(first, second);
    assert!(first.root("acme/customer:customer#good").is_some());
    assert!(first.root("acme/customer:customer#dependent").is_none());
    assert!(first.root("acme/customer:customer#foo-bar").is_none());
    assert!(first.root("acme/customer:customer#foo_bar").is_none());
    assert!(first.root("acme/customer:other#duplicate").is_none());
    let customer = first.protocol("acme.customer.Customer").unwrap();
    assert!(customer.message("goodMessage").is_some());
    assert!(customer.message("badMessage").is_none());
    assert_eq!(first.diagnostics().len(), 6);
    assert!(first.diagnostics().iter().all(|diagnostic| {
        diagnostic.code() == "AVRO003"
            && diagnostic.severity() == morphir_extension_sdk::DiagnosticSeverity::Warning
    }));
}
