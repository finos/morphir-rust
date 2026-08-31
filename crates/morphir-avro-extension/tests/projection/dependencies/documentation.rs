use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn declaration_docs_are_structural_on_roots_owned_linked_and_specialized_schemas() {
    let owned_record_source = "acme/customer:customer#documented-record";
    let owned_enum_source = "acme/customer:customer#documented-enum";
    let dependency_source = "acme/shared:types#box";
    let plain_dependency_source = "acme/shared:types#plain";
    let mut owned_record = alias(
        owned_record_source,
        "documented-record",
        TypeExpr::Record(vec![field(
            "plain",
            reference(plain_dependency_source, vec![]),
        )]),
    );
    let TypeDeclaration::Alias { doc, .. } = &mut owned_record else {
        unreachable!()
    };
    *doc = Some("Owned record documentation.".to_owned());
    let owned_enum = TypeDeclaration::Custom {
        source_name: owned_enum_source.to_owned(),
        name: "documented-enum".to_owned(),
        type_params: Vec::new(),
        constructors: vec![Constructor {
            source_name: "acme/customer:customer#only".to_owned(),
            name: "only".to_owned(),
            arguments: Vec::new(),
        }],
        doc: Some("Owned enum documentation.".to_owned()),
    };
    let specialized = alias(
        "acme/customer:customer#boxed",
        "boxed",
        reference(
            dependency_source,
            vec![reference("morphir/SDK:string#string", vec![])],
        ),
    );
    let mut input = package(vec![owned_record, owned_enum, specialized]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![
                TypeDeclaration::Alias {
                    source_name: dependency_source.to_owned(),
                    name: "box".to_owned(),
                    type_params: vec!["a".to_owned()],
                    value: TypeExpr::Record(vec![field(
                        "value",
                        TypeExpr::Variable("a".to_owned()),
                    )]),
                    doc: Some("Linked generic documentation.".to_owned()),
                },
                TypeDeclaration::Alias {
                    source_name: plain_dependency_source.to_owned(),
                    name: "plain".to_owned(),
                    type_params: Vec::new(),
                    value: TypeExpr::Record(Vec::new()),
                    doc: Some("Linked declaration documentation.".to_owned()),
                },
            ],
            values: Vec::new(),
            doc: None,
        }],
    }];
    let model = project(
        &input,
        &AvroOptions {
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        model.root(owned_record_source).unwrap().doc(),
        Some("Owned record documentation.")
    );
    assert_eq!(
        model
            .named_schema("acme.customer.customer.DocumentedRecord")
            .unwrap()
            .doc(),
        Some("Owned record documentation.")
    );
    assert_eq!(
        model
            .named_schema("acme.customer.customer.DocumentedEnum")
            .unwrap()
            .doc(),
        Some("Owned enum documentation.")
    );
    assert_eq!(
        model.root(owned_enum_source).unwrap().doc(),
        Some("Owned enum documentation.")
    );
    assert_eq!(
        model
            .linked_schema("acme.shared.types.Plain")
            .unwrap()
            .doc(),
        Some("Linked declaration documentation.")
    );
    let linked = model
        .linked_schemas()
        .iter()
        .find(|schema| schema.full_name().name().starts_with("BoxString_"))
        .unwrap();
    assert_eq!(linked.doc(), Some("Linked generic documentation."));
}
