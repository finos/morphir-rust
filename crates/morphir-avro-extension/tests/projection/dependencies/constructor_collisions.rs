use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn constructor_collisions_are_reported_at_each_constructor_fqname() {
    let bad = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#bad".to_owned(),
        name: "bad".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#foo-bar".to_owned(),
                name: "foo-bar".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#foo_bar".to_owned(),
                name: "foo_bar".to_owned(),
                arguments: Vec::new(),
            },
        ],
        doc: None,
    };
    let good_source = "acme/customer:customer#good";
    let input = package(vec![
        bad,
        alias(good_source, "good", TypeExpr::Record(Vec::new())),
    ]);
    let errors = project(&input, &AvroOptions::default())
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors
            .iter()
            .map(|error| error.source())
            .collect::<Vec<_>>(),
        [
            Some("acme/customer:customer#foo-bar"),
            Some("acme/customer:customer#foo_bar")
        ]
    );
    let partial = project(
        &input,
        &AvroOptions {
            unsupported: Unsupported::WarnAndSkip,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert!(partial.root(good_source).is_some());
    assert!(partial.root("acme/customer:customer#bad").is_none());
    assert_eq!(partial.diagnostics().len(), 2);
}
