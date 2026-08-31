use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn normalized_duplicate_protocol_and_message_names_are_avro003() {
    let mut duplicate_modules = package(Vec::new());
    duplicate_modules.modules = vec![
        ProjectionModule {
            path: vec!["foo-bar".to_owned()],
            types: Vec::new(),
            values: Vec::new(),
            doc: None,
        },
        ProjectionModule {
            path: vec!["foo_bar".to_owned()],
            types: Vec::new(),
            values: Vec::new(),
            doc: None,
        },
    ];
    let options = AvroOptions {
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };
    assert_eq!(
        project(&duplicate_modules, &options)
            .unwrap_err()
            .into_diagnostics()
            .unwrap()[0]
            .code(),
        "AVRO003"
    );

    let mut duplicate_messages = package(Vec::new());
    duplicate_messages.modules[0].values = ["find-customer", "find_customer"]
        .into_iter()
        .map(|name| {
            value_specification(
                &format!("acme/customer:customer#{name}"),
                name,
                vec![],
                Some(TypeExpr::Unit),
                ValueKind::Function,
                None,
            )
        })
        .collect();
    assert_eq!(
        project(&duplicate_messages, &options)
            .unwrap_err()
            .into_diagnostics()
            .unwrap()[0]
            .code(),
        "AVRO003"
    );
}
