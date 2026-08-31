use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn protocol_and_message_collisions_quarantine_all_counterparts_but_keep_independent_messages() {
    fn input(reverse: bool) -> morphir_avro_extension::ProjectionPackage {
        let mut modules = vec![
            ProjectionModule {
                path: vec!["foo-bar".to_owned()],
                types: vec![alias(
                    "acme/customer:foo-bar#left-record",
                    "left-record",
                    TypeExpr::Record(Vec::new()),
                )],
                values: Vec::new(),
                doc: None,
            },
            ProjectionModule {
                path: vec!["foo_bar".to_owned()],
                types: vec![alias(
                    "acme/customer:foo_bar#right-record",
                    "right-record",
                    TypeExpr::Record(Vec::new()),
                )],
                values: Vec::new(),
                doc: None,
            },
            ProjectionModule {
                path: vec!["valid".to_owned()],
                types: Vec::new(),
                values: vec![
                    value_specification(
                        "acme/customer:valid#find-customer",
                        "find-customer",
                        vec![],
                        Some(TypeExpr::Unit),
                        ValueKind::Function,
                        None,
                    ),
                    value_specification(
                        "acme/customer:valid#find_customer",
                        "find_customer",
                        vec![],
                        Some(TypeExpr::Unit),
                        ValueKind::Function,
                        None,
                    ),
                    value_specification(
                        "acme/customer:valid#healthy",
                        "healthy",
                        vec![],
                        Some(TypeExpr::Unit),
                        ValueKind::Function,
                        None,
                    ),
                ],
                doc: None,
            },
        ];
        if reverse {
            modules.reverse();
            modules
                .iter_mut()
                .for_each(|module| module.values.reverse());
        }
        morphir_avro_extension::ProjectionPackage {
            kind: DistributionKind::Library,
            package_name: "acme/customer".to_owned(),
            dependencies: Vec::new(),
            modules,
        }
    }
    let options = AvroOptions {
        projection: Projection::ProtocolPublic,
        unsupported: Unsupported::WarnAndSkip,
        ..AvroOptions::default()
    };
    let strict = project(
        &input(false),
        &AvroOptions {
            unsupported: Unsupported::Error,
            ..options.clone()
        },
    )
    .unwrap_err()
    .into_diagnostics()
    .unwrap();
    assert_eq!(strict.len(), 4);
    let first = project(&input(false), &options).unwrap();
    let second = project(&input(true), &options).unwrap();
    assert_eq!(first, second);
    assert!(first.protocol("acme.customer.FooBar").is_none());
    let valid = first.protocol("acme.customer.Valid").unwrap();
    assert_eq!(valid.messages().len(), 1);
    assert!(valid.message("healthy").is_some());
    let json = render_json(&first, Dependencies::SelfContained).unwrap();
    assert_eq!(
        json.iter()
            .filter(|artifact| artifact.path.ends_with(".avsc"))
            .count(),
        2,
        "named roots from quarantined protocols must remain as JSON schemas"
    );
    assert_eq!(
        render_idl(&first, Dependencies::SelfContained)
            .unwrap()
            .len(),
        3,
        "named roots from quarantined protocols need independent IDL wrappers"
    );
    assert_eq!(first.diagnostics().len(), 4);
    assert!(
        first
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code() == "AVRO003")
    );
}
