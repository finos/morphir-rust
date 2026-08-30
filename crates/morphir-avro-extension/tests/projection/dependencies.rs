#[test]
fn linked_projection_reports_a_missing_declaration_as_avro006() {
    let missing = alias(
        "acme/customer:customer#customer",
        "customer",
        TypeExpr::Record(vec![field(
            "missing",
            reference("acme/missing:types#identifier", vec![]),
        )]),
    );
    let error = project(
        &package(vec![missing]),
        &AvroOptions {
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap_err()
    .into_diagnostics()
    .unwrap();
    assert_eq!(error[0].code(), "AVRO006");
    assert_eq!(error[0].source(), Some("acme/customer:customer#customer"));
}

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

#[test]
fn protocol_and_message_collisions_quarantine_all_counterparts_but_keep_independent_messages() {
    fn input(reverse: bool) -> morphir_avro_extension::ProjectionPackage {
        let mut modules = vec![
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
    assert_eq!(first.diagnostics().len(), 4);
    assert!(
        first
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code() == "AVRO003")
    );
}

#[test]
fn linked_schema_ownership_follows_declarations_across_cross_edges_and_cycles() {
    let owned_root = "acme/customer:customer#a-root";
    let owned_shared = "acme/customer:customer#z-owned-shared";
    let dependency = "acme/shared:types#dependency";
    let input = {
        let mut package = package(vec![
            alias(
                owned_root,
                "a-root",
                TypeExpr::Record(vec![field("dependency", reference(dependency, vec![]))]),
            ),
            alias(
                owned_shared,
                "z-owned-shared",
                TypeExpr::Record(vec![field("cycle", reference(dependency, vec![]))]),
            ),
        ]);
        package.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![alias(
                    dependency,
                    "dependency",
                    TypeExpr::Record(vec![field("owned", reference(owned_shared, vec![]))]),
                )],
                values: Vec::new(),
                doc: None,
            }],
        }];
        package
    };
    let linked = project(
        &input,
        &AvroOptions {
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert!(
        linked
            .named_schema("acme.customer.customer.ARoot")
            .is_some()
    );
    assert!(
        linked
            .named_schema("acme.customer.customer.ZOwnedShared")
            .is_some()
    );
    assert!(
        linked
            .linked_schema("acme.shared.types.Dependency")
            .is_some()
    );
    assert_eq!(linked.linked_schemas().len(), 1);

    let self_contained = project(&input, &AvroOptions::default()).unwrap();
    assert!(self_contained.linked_schemas().is_empty());
    assert!(
        self_contained
            .named_schema("acme.shared.types.Dependency")
            .is_some()
    );
}

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

