#[test]
fn core_types_map_and_records_preserve_exact_source_identity() {
    let model = project(&package(vec![customer_record()]), &AvroOptions::default()).unwrap();
    let customer = model
        .named_schema("acme.customer.customer.Customer")
        .unwrap();

    assert_eq!(
        customer.property("morphir.fqname"),
        Some(&json!("Acme:Customer:Customer"))
    );
    assert_eq!(customer.field("active").unwrap().tpe(), &AvroType::Boolean);
    assert_eq!(customer.field("age").unwrap().tpe(), &AvroType::Long);
    assert_eq!(customer.field("name").unwrap().tpe(), &AvroType::String);
    let root = model.root("Acme:Customer:Customer").unwrap();
    assert_eq!(
        root.full_name().to_string(),
        "acme.customer.customer.Customer"
    );
    assert_eq!(root.tpe(), &AvroType::Named(customer.full_name().clone()));
    assert_eq!(
        root.referenced_named_declarations(),
        [customer.full_name().clone()]
    );
}

#[test]
fn every_public_alias_has_a_root_even_when_it_is_not_a_named_schema() {
    let primitive_source = "acme/customer:customer#count";
    let list_source = "acme/customer:customer#names";
    let mapped_source = "acme/customer:customer#lookup";
    let declarations = vec![
        alias(
            primitive_source,
            "count",
            reference("morphir/SDK:basics#int", vec![]),
        ),
        alias(
            list_source,
            "names",
            reference(
                "morphir/SDK:list#list",
                vec![reference("morphir/SDK:string#string", vec![])],
            ),
        ),
        alias(
            mapped_source,
            "lookup",
            reference(
                "morphir/SDK:dict#dict",
                vec![
                    reference("morphir/SDK:basics#int", vec![]),
                    reference("morphir/SDK:string#string", vec![]),
                ],
            ),
        ),
    ];
    let mut options = AvroOptions::default();
    options.type_mappings.insert(
        mapped_source.to_owned(),
        TypeMapping {
            physical_type: "bytes".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        },
    );

    let model = project(&package(declarations), &options).unwrap();
    assert_eq!(model.roots().len(), 3);
    assert_eq!(model.root(primitive_source).unwrap().tpe(), &AvroType::Long);
    assert_eq!(
        model.root(list_source).unwrap().tpe(),
        &AvroType::Array(Box::new(AvroType::String), Default::default())
    );
    let mapped = model.root(mapped_source).unwrap();
    let AvroType::Annotated {
        physical,
        properties,
    } = mapped.tpe()
    else {
        panic!("mapped root should be annotated")
    };
    assert_eq!(physical.as_ref(), &AvroType::Bytes);
    assert_eq!(
        properties.get("morphir.fqname"),
        Some(&json!(mapped_source))
    );
    assert_eq!(
        mapped.property("morphir.fqname"),
        Some(&json!(mapped_source))
    );
}

#[test]
fn unit_maybe_list_set_dict_char_and_tuple_use_the_approved_core_mappings() {
    let maybe_string = reference(
        "morphir/SDK:maybe#maybe",
        vec![reference("morphir/SDK:string#string", Vec::new())],
    );
    let list_int = reference(
        "morphir/SDK:list#list",
        vec![reference("morphir/SDK:basics#int", Vec::new())],
    );
    let set_char = reference(
        "morphir/SDK:set#set",
        vec![reference("morphir/SDK:char#char", Vec::new())],
    );
    let dict = reference(
        "morphir/SDK:dict#dict",
        vec![
            reference("morphir/SDK:string#string", Vec::new()),
            reference("morphir/SDK:basics#float", Vec::new()),
        ],
    );
    let container = alias(
        "acme/customer:customer#container",
        "container",
        TypeExpr::Record(vec![
            field("nothing", TypeExpr::Unit),
            field("optional", maybe_string),
            field("items", list_int),
            field("unique", set_char),
            field("index", dict),
            field(
                "pair",
                TypeExpr::Tuple(vec![
                    TypeExpr::Unit,
                    reference("morphir/SDK:string#string", vec![]),
                ]),
            ),
        ]),
    );

    let model = project(&package(vec![container]), &AvroOptions::default()).unwrap();
    let record = model
        .named_schema("acme.customer.customer.Container")
        .unwrap();
    assert_eq!(record.field("nothing").unwrap().tpe(), &AvroType::Null);
    assert_eq!(
        record.field("optional").unwrap().tpe(),
        &AvroType::Union(AvroUnion::new(vec![AvroType::Null, AvroType::String]).unwrap())
    );
    assert_eq!(
        record.field("items").unwrap().tpe(),
        &AvroType::Array(Box::new(AvroType::Long), Default::default())
    );
    let AvroType::Array(element, properties) = record.field("unique").unwrap().tpe() else {
        panic!("Set must project to an annotated array")
    };
    assert_eq!(
        properties.get("morphir.collection-kind"),
        Some(&json!("set"))
    );
    assert_eq!(
        element.properties().get("morphir.type"),
        Some(&json!("Char"))
    );
    assert_eq!(
        record.field("index").unwrap().tpe(),
        &AvroType::Map(Box::new(AvroType::Double), Default::default())
    );
    let AvroType::Named(tuple_name) = record.field("pair").unwrap().tpe() else {
        panic!("Tuple must project to a stable named record")
    };
    assert!(tuple_name.name().starts_with("Tuple_"));
    let tuple = model.named_schema(&tuple_name.to_string()).unwrap();
    assert_eq!(tuple.field("item1").unwrap().tpe(), &AvroType::Null);
    assert_eq!(tuple.field("item2").unwrap().tpe(), &AvroType::String);
}

#[test]
fn a_non_sdk_type_named_char_remains_a_named_reference() {
    let source = "acme/customer:domain#char";
    let wrapper = alias(
        "acme/customer:customer#wrapper",
        "wrapper",
        TypeExpr::Record(vec![field("character", reference(source, vec![]))]),
    );
    let mut input = package(vec![wrapper]);
    input.modules.push(ProjectionModule {
        path: vec!["domain".to_owned()],
        types: vec![alias(source, "char", TypeExpr::Record(Vec::new()))],
        values: Vec::new(),
        doc: None,
    });
    let model = project(&input, &AvroOptions::default()).unwrap();
    let wrapper = model
        .named_schema("acme.customer.customer.Wrapper")
        .unwrap();

    let AvroType::Named(character) = wrapper.field("character").unwrap().tpe() else {
        panic!("a non-SDK Char type must remain a named reference")
    };
    assert_eq!(character.to_string(), "acme.customer.domain.Char");
}

#[test]
fn nullary_custom_types_become_named_enums() {
    let status = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#status".to_owned(),
        name: "status".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#inactive".to_owned(),
                name: "inactive".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#active".to_owned(),
                name: "active".to_owned(),
                arguments: Vec::new(),
            },
        ],
        doc: None,
    };
    let model = project(&package(vec![status]), &AvroOptions::default()).unwrap();

    let NamedSchema::Enum(status) = model.named_schema("acme.customer.customer.Status").unwrap()
    else {
        panic!("nullary custom type must be an enum")
    };
    assert_eq!(status.symbols(), ["Active", "Inactive"]);
    assert_eq!(
        status.properties().get("morphir.fqname"),
        Some(&json!("acme/customer:customer#status"))
    );
}

#[test]
fn non_string_dict_keys_are_avro001_errors() {
    let bad = alias(
        "acme/customer:customer#bad-dict",
        "bad-dict",
        TypeExpr::Record(vec![field(
            "values",
            reference(
                "morphir/SDK:dict#dict",
                vec![
                    reference("morphir/SDK:basics#int", vec![]),
                    reference("morphir/SDK:string#string", vec![]),
                ],
            ),
        )]),
    );

    let error = project(&package(vec![bad]), &AvroOptions::default())
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(error[0].code(), "AVRO001");
}

#[test]
fn an_exact_root_mapping_bypasses_an_unsupported_non_string_dict_alias() {
    let mapped_source = "acme/customer:customer#bad-dict";
    let bad = alias(
        mapped_source,
        "bad-dict",
        reference(
            "morphir/SDK:dict#dict",
            vec![
                reference("morphir/SDK:basics#int", vec![]),
                reference("morphir/SDK:string#string", vec![]),
            ],
        ),
    );
    let envelope = alias(
        "acme/customer:customer#envelope",
        "envelope",
        TypeExpr::Record(vec![field("mapped", reference(mapped_source, vec![]))]),
    );
    let mut options = AvroOptions::default();
    options.type_mappings.insert(
        mapped_source.to_owned(),
        TypeMapping {
            physical_type: "string".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        },
    );

    let model = project(&package(vec![bad, envelope]), &options).unwrap();
    let envelope = model
        .named_schema("acme.customer.customer.Envelope")
        .unwrap();
    let AvroType::Annotated {
        physical,
        properties,
    } = envelope.field("mapped").unwrap().tpe()
    else {
        panic!("an explicitly mapped reference must retain source metadata")
    };
    assert_eq!(physical.as_ref(), &AvroType::String);
    assert_eq!(
        properties.get("morphir.fqname"),
        Some(&json!(mapped_source))
    );
}

#[test]
fn an_invalid_root_mapping_physical_type_is_avro004() {
    let source = "acme/customer:customer#bad-dict";
    let bad = alias(
        source,
        "bad-dict",
        reference(
            "morphir/SDK:dict#dict",
            vec![
                reference("morphir/SDK:basics#int", vec![]),
                reference("morphir/SDK:string#string", vec![]),
            ],
        ),
    );
    let mut options = AvroOptions::default();
    options.type_mappings.insert(
        source.to_owned(),
        TypeMapping {
            physical_type: "not-an-avro-type".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        },
    );

    let error = project(&package(vec![bad]), &options)
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(error[0].code(), "AVRO004");
}

#[test]
fn an_invalid_unused_mapping_physical_type_is_still_avro004() {
    let mut options = AvroOptions::default();
    options.type_mappings.insert(
        "acme/customer:customer#not-declared".to_owned(),
        TypeMapping {
            physical_type: "not-an-avro-type".to_owned(),
            logical_type: None,
            precision: None,
            scale: None,
        },
    );

    let error = project(&package(vec![customer_record()]), &options)
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(error[0].code(), "AVRO004");
    assert_eq!(
        error[0].source(),
        Some("acme/customer:customer#not-declared")
    );
}

#[test]
fn normalized_full_name_collisions_are_avro003_errors() {
    let first = alias(
        "acme/customer:customer#foo-bar",
        "foo-bar",
        TypeExpr::Record(Vec::new()),
    );
    let second = alias(
        "acme/customer:customer#foo_bar",
        "foo_bar",
        TypeExpr::Record(Vec::new()),
    );

    let error = project(&package(vec![first, second]), &AvroOptions::default())
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(error[0].code(), "AVRO003");
    assert!(error[0].message().contains("acme.customer.customer.FooBar"));
}

