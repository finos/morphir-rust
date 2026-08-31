#[test]
fn public_protocol_includes_functions_constants_and_entry_metadata() {
    let customer_source = "acme/customer:customer#customer";
    let mut input = package(vec![alias(
        customer_source,
        "customer",
        TypeExpr::Record(vec![field(
            "id",
            reference("morphir/SDK:string#string", vec![]),
        )]),
    )]);
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#find-customer",
            "find-customer",
            vec![field(
                "customer-id",
                reference("morphir/SDK:string#string", vec![]),
            )],
            Some(reference(customer_source, vec![])),
            ValueKind::Function,
            Some(EntryPointMetadata {
                identifier: "customer-query".to_owned(),
                kind: EntryPointKind::Command,
                doc: Some("Application command.".to_owned()),
            }),
        ),
        value_specification(
            "acme/customer:customer#schema-version",
            "schema-version",
            vec![],
            Some(reference("morphir/SDK:string#string", vec![])),
            ValueKind::Constant,
            None,
        ),
    ];
    let options = AvroOptions {
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };

    let model = project(&input, &options).unwrap();
    let protocol = model.protocol("acme.customer.Customer").unwrap();
    let find = protocol.message("findCustomer").unwrap();
    assert_eq!(find.request().fields()[0].name(), "customerId");
    assert_eq!(
        find.property("morphir.value-kind"),
        Some(&json!("function"))
    );
    assert_eq!(find.property("morphir.entry-point"), Some(&json!(true)));
    assert_eq!(
        find.property("morphir.entry-point-kind"),
        Some(&json!("command"))
    );
    assert_eq!(
        find.property("morphir.entry-point-id"),
        Some(&json!("customer-query"))
    );
    assert!(find.errors().is_empty());
    let constant = protocol.message("schemaVersion").unwrap();
    assert!(constant.request().fields().is_empty());
    assert_eq!(
        constant.property("morphir.value-kind"),
        Some(&json!("constant"))
    );
    assert_eq!(
        constant.property("morphir.constant-value"),
        None,
        "projection must not inspect or evaluate a constant body"
    );
}

#[test]
fn protocol_requests_preserve_function_parameter_order() {
    let mut input = package(Vec::new());
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#ordered",
        "ordered",
        vec![
            field("z-input", TypeExpr::Unit),
            field("a-input", TypeExpr::Unit),
        ],
        Some(TypeExpr::Unit),
        ValueKind::Function,
        None,
    )];
    let options = AvroOptions {
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };

    let model = project(&input, &options).unwrap();
    let request = model
        .protocol("acme.customer.Customer")
        .unwrap()
        .message("ordered")
        .unwrap()
        .request();

    assert_eq!(
        request
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["zInput", "aInput"]
    );
}

#[test]
fn projection_modes_select_messages_without_inventing_library_entry_points() {
    let mut application = package(Vec::new());
    application.kind = DistributionKind::Application;
    application.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#main",
            "main",
            vec![],
            Some(TypeExpr::Unit),
            ValueKind::Function,
            Some(EntryPointMetadata {
                identifier: "main".to_owned(),
                kind: EntryPointKind::Main,
                doc: None,
            }),
        ),
        value_specification(
            "acme/customer:customer#handle",
            "handle",
            vec![],
            Some(TypeExpr::Unit),
            ValueKind::Function,
            Some(EntryPointMetadata {
                identifier: "handler".to_owned(),
                kind: EntryPointKind::Handler,
                doc: None,
            }),
        ),
        value_specification(
            "acme/customer:customer#helper",
            "helper",
            vec![],
            Some(TypeExpr::Unit),
            ValueKind::Function,
            None,
        ),
    ];

    let schemas = project(&application, &AvroOptions::default()).unwrap();
    assert!(schemas.protocols().is_empty());

    let entry_options = AvroOptions {
        projection: Projection::ProtocolEntryPoints,
        ..AvroOptions::default()
    };
    let entry = project(&application, &entry_options).unwrap();
    let protocol = entry.only_protocol().unwrap();
    assert_eq!(
        protocol
            .messages()
            .iter()
            .map(|message| message.name())
            .collect::<Vec<_>>(),
        ["handle", "main"]
    );
    assert_eq!(
        protocol
            .message("main")
            .unwrap()
            .property("morphir.entry-point-kind"),
        Some(&json!("main"))
    );
    assert_eq!(
        protocol
            .message("handle")
            .unwrap()
            .property("morphir.entry-point-kind"),
        Some(&json!("handler"))
    );

    for kind in [DistributionKind::Library, DistributionKind::Specs] {
        let mut type_only = application.clone();
        type_only.kind = kind;
        let model = project(&type_only, &entry_options).unwrap();
        assert!(
            model.only_protocol().unwrap().messages().is_empty(),
            "only an Application distribution may expose declared entry points"
        );

        let public = project(
            &type_only,
            &AvroOptions {
                projection: Projection::ProtocolPublic,
                ..AvroOptions::default()
            },
        )
        .unwrap();
        assert_eq!(public.only_protocol().unwrap().messages().len(), 3);
    }
}

#[test]
fn result_responses_remain_data_and_incomplete_values_follow_the_unsupported_policy() {
    let result_type = reference(
        "morphir/SDK:result#result",
        vec![
            reference("morphir/SDK:string#string", vec![]),
            reference("morphir/SDK:basics#int", vec![]),
        ],
    );
    let mut input = package(Vec::new());
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#lookup",
            "lookup",
            vec![],
            Some(result_type),
            ValueKind::Function,
            None,
        ),
        value_specification(
            "acme/customer:customer#unfinished",
            "unfinished",
            vec![],
            None,
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
    assert_eq!(strict.len(), 1);
    assert_eq!(strict[0].code(), "AVRO001");
    assert_eq!(
        strict[0].source(),
        Some("acme/customer:customer#unfinished")
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
    let protocol = partial.only_protocol().unwrap();
    let lookup = protocol.message("lookup").unwrap();
    assert!(lookup.errors().is_empty());
    assert!(matches!(lookup.response(), AvroType::Named(_)));
    assert!(protocol.message("unfinished").is_none());
}

#[test]
fn a_constant_with_inputs_is_rejected_instead_of_becoming_a_function_shape() {
    let mut input = package(Vec::new());
    input.modules[0].values = vec![ValueSpecification {
        source_name: "acme/customer:customer#invalid-constant".to_owned(),
        name: "invalid-constant".to_owned(),
        inputs: vec![field("argument", TypeExpr::Unit)],
        output: Some(TypeExpr::Unit),
        value_kind: ValueKind::Constant,
        entry_point: None,
        doc: None,
    }];

    let errors = project(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            ..AvroOptions::default()
        },
    )
    .unwrap_err()
    .into_diagnostics()
    .unwrap();
    assert_eq!(errors[0].code(), "AVRO001");
    assert_eq!(
        errors[0].source(),
        Some("acme/customer:customer#invalid-constant")
    );
}

#[test]
fn protocol_and_root_closures_are_sorted_and_linked_dependencies_are_not_emitted() {
    let dependency_source = "acme/shared:types#identifier";
    let owned_source = "acme/customer:customer#customer";
    let dependency = alias(
        dependency_source,
        "identifier",
        TypeExpr::Record(vec![field(
            "value",
            reference("morphir/SDK:string#string", vec![]),
        )]),
    );
    let owned = alias(
        owned_source,
        "customer",
        TypeExpr::Record(vec![field("id", reference(dependency_source, vec![]))]),
    );
    let mut input = package(vec![owned]);
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#find",
        "find",
        vec![],
        Some(reference(owned_source, vec![])),
        ValueKind::Function,
        None,
    )];
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![dependency],
            values: Vec::new(),
            doc: None,
        }],
    }];

    let self_contained = project(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        self_contained
            .root(owned_source)
            .unwrap()
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "acme.customer.customer.Customer",
            "acme.shared.types.Identifier"
        ]
    );
    assert_eq!(
        self_contained
            .only_protocol()
            .unwrap()
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "acme.customer.customer.Customer",
            "acme.shared.types.Identifier"
        ]
    );

    let linked = project(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert!(
        linked
            .named_schema("acme.shared.types.Identifier")
            .is_none()
    );
    assert_eq!(
        linked
            .linked_schemas()
            .iter()
            .filter(|schema| schema.full_name().to_string() == "acme.shared.types.Identifier")
            .count(),
        1,
        "a reachable linked declaration must be emitted exactly once"
    );
    assert!(linked.root(dependency_source).is_none());
    assert_eq!(
        linked
            .only_protocol()
            .unwrap()
            .referenced_named_declarations()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "acme.customer.customer.Customer",
            "acme.shared.types.Identifier"
        ]
    );
    for reference in linked
        .root(owned_source)
        .unwrap()
        .referenced_named_declarations()
    {
        assert!(
            linked.named_schema(&reference.to_string()).is_some()
                || linked.linked_schema(&reference.to_string()).is_some(),
            "every root reference must resolve to an owned or linked declaration"
        );
    }
}

