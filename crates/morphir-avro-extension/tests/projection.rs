mod support;

use morphir_avro_extension::{
    Aliases, AvroOptions, AvroType, AvroUnion, Constructor, Dependencies, DistributionKind,
    EntryPointKind, EntryPointMetadata, IncompletenessKind, NamedSchema, Projection,
    ProjectionDependency, ProjectionModule, TypeDeclaration, TypeExpr, TypeMapping, UnionError,
    Unsupported, ValueKind, ValueSpecification, escape_idl_identifier, project,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use support::projection::{alias, customer_record, field, package, reference, value_specification};

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

#[test]
fn names_are_canonical_keyword_safe_and_output_is_source_order_independent() {
    let keyword = alias(
        "acme/customer:customer#order-item",
        "order-item",
        TypeExpr::Record(vec![field(
            "record",
            reference("morphir/SDK:string#string", vec![]),
        )]),
    );
    let customer = customer_record();
    let forward = project(
        &package(vec![keyword.clone(), customer.clone()]),
        &AvroOptions::default(),
    )
    .unwrap();
    let reverse = project(&package(vec![customer, keyword]), &AvroOptions::default()).unwrap();

    assert_eq!(forward, reverse);
    assert_eq!(
        forward
            .schemas()
            .iter()
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>(),
        [
            "acme.customer.customer.Customer",
            "acme.customer.customer.OrderItem"
        ]
    );
    assert!(forward.schemas()[1].field("record").is_some());
    assert_eq!(escape_idl_identifier("record"), "`record`");
    assert_eq!(escape_idl_identifier("namespace"), "`namespace`");
    assert_eq!(escape_idl_identifier("customer"), "customer");
    assert_eq!(escape_idl_identifier("Record"), "Record");
}

#[test]
fn idl_identifier_escaping_matches_the_avro_1_12_2_parser_keyword_union() {
    let reserved = [
        "array",
        "big_decimal",
        "boolean",
        "bytes",
        "date",
        "decimal",
        "double",
        "enum",
        "error",
        "false",
        "fixed",
        "float",
        "idl",
        "import",
        "int",
        "local_timestamp_ms",
        "long",
        "map",
        "namespace",
        "null",
        "oneway",
        "protocol",
        "record",
        "schema",
        "string",
        "throws",
        "time_ms",
        "timestamp_ms",
        "true",
        "union",
        "uuid",
        "void",
    ];
    for keyword in reserved {
        assert_eq!(
            escape_idl_identifier(keyword),
            format!("`{keyword}`"),
            "reserved Avro 1.12.2 identifier {keyword}"
        );
    }

    let raw = [
        "time_micros",
        "timestamp_micros",
        "local_timestamp_micros",
        "BigDecimal",
        "BIG_DECIMAL",
        "customer",
    ];
    for identifier in raw {
        assert_eq!(
            escape_idl_identifier(identifier),
            identifier,
            "case-sensitive non-keyword {identifier}"
        );
    }
}

#[test]
fn synthetic_tuple_names_depend_on_canonical_type_not_traversal_order() {
    let tuple = TypeExpr::Tuple(vec![
        reference("morphir/SDK:basics#bool", vec![]),
        reference("morphir/SDK:basics#int", vec![]),
    ]);
    let first = alias(
        "acme/customer:customer#first",
        "first",
        TypeExpr::Record(vec![field("tuple", tuple.clone())]),
    );
    let second = alias(
        "acme/customer:customer#second",
        "second",
        TypeExpr::Record(vec![field("tuple", tuple)]),
    );
    let forward = project(
        &package(vec![first.clone(), second.clone()]),
        &AvroOptions::default(),
    )
    .unwrap();
    let reverse = project(&package(vec![second, first]), &AvroOptions::default()).unwrap();

    let tuple_names = |package: &morphir_avro_extension::AvroPackage| {
        package
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("Tuple_"))
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(tuple_names(&forward), tuple_names(&reverse));
    assert_eq!(tuple_names(&forward).len(), 1);
    assert_eq!(
        tuple_names(&forward)[0],
        "acme.customer.customer.Tuple_3126ea1e1667"
    );
}

#[test]
fn unions_reject_duplicate_and_nested_branches_at_construction() {
    assert_eq!(AvroUnion::new(Vec::new()), Err(UnionError::Empty));
    assert_eq!(
        AvroUnion::new(vec![AvroType::String, AvroType::String]),
        Err(UnionError::DuplicateBranch("string".to_owned()))
    );
    let valid = AvroUnion::new(vec![AvroType::Null, AvroType::String]).unwrap();
    assert_eq!(
        AvroUnion::new(vec![AvroType::Boolean, AvroType::Union(valid)]),
        Err(UnionError::NestedUnion)
    );
    assert_eq!(
        AvroUnion::new(vec![
            AvroType::String,
            AvroType::Annotated {
                physical: Box::new(AvroType::String),
                properties: Default::default(),
            },
        ]),
        Err(UnionError::DuplicateBranch("string".to_owned()))
    );
    assert_eq!(
        AvroUnion::new(vec![
            AvroType::Long,
            AvroType::Logical {
                physical: Box::new(AvroType::Long),
                name: "timestamp-micros".to_owned(),
                properties: Default::default(),
            },
        ]),
        Err(UnionError::DuplicateBranch("long".to_owned()))
    );
}

#[test]
fn project_validates_directly_constructed_options() {
    let options = AvroOptions {
        decimal_precision: 0,
        ..AvroOptions::default()
    };

    let error = project(&package(vec![customer_record()]), &options)
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
    assert_eq!(error[0].code(), "AVRO004");
}

mod advanced_types {
    use super::*;
    use pretty_assertions::assert_eq;

    const STRING: &str = "morphir/SDK:string#string";
    const INT: &str = "morphir/SDK:basics#int";
    const RESULT: &str = "morphir/SDK:result#result";
    const LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
    const LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
    const INSTANT: &str = "morphir/SDK:instant#instant";
    const DATE_TIME: &str = "morphir/SDK:date-time#date-time";
    const UUID: &str = "morphir/SDK:uuid#uuid";
    const DECIMAL: &str = "morphir/SDK:decimal#decimal";

    fn custom(
        source_name: &str,
        name: &str,
        type_params: Vec<&str>,
        constructors: Vec<Constructor>,
    ) -> TypeDeclaration {
        TypeDeclaration::Custom {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            type_params: type_params.into_iter().map(str::to_owned).collect(),
            constructors,
            doc: None,
        }
    }

    fn constructor(name: &str, arguments: Vec<morphir_avro_extension::NamedType>) -> Constructor {
        Constructor {
            source_name: format!("acme/customer:customer#{name}"),
            name: name.to_owned(),
            arguments,
        }
    }

    fn alias_with_params(
        source_name: &str,
        name: &str,
        type_params: Vec<&str>,
        value: TypeExpr,
    ) -> TypeDeclaration {
        TypeDeclaration::Alias {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            type_params: type_params.into_iter().map(str::to_owned).collect(),
            value,
            doc: None,
        }
    }

    fn logical(
        physical: AvroType,
        name: &str,
        properties: &[(&str, serde_json::Value)],
    ) -> AvroType {
        AvroType::Logical {
            physical: Box::new(physical),
            name: name.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        }
    }

    #[test]
    fn payload_constructors_are_named_records_inside_a_wrapper_union() {
        let shape = custom(
            "acme/customer:customer#shape",
            "shape",
            vec![],
            vec![
                constructor("point", vec![]),
                constructor("circle", vec![field("radius", reference(INT, vec![]))]),
            ],
        );

        let model = project(&package(vec![shape]), &AvroOptions::default()).unwrap();
        let wrapper = model.named_schema("acme.customer.customer.Shape").unwrap();
        let AvroType::Union(union) = wrapper.field("value").unwrap().tpe() else {
            panic!("payload custom type must use a wrapper union")
        };
        assert_eq!(
            union
                .branches()
                .iter()
                .map(|branch| match branch {
                    AvroType::Named(name) => name.to_string(),
                    other => panic!("constructor branch must be named, got {other:?}"),
                })
                .collect::<Vec<_>>(),
            [
                "acme.customer.customer.Shape.Circle",
                "acme.customer.customer.Shape.Point"
            ]
        );
        assert_eq!(
            model
                .named_schema("acme.customer.customer.Shape.Circle")
                .unwrap()
                .field("radius")
                .unwrap()
                .tpe(),
            &AvroType::Long
        );
        assert!(
            model
                .named_schema("acme.customer.customer.Shape.Point")
                .unwrap()
                .field("radius")
                .is_none()
        );
    }

    #[test]
    fn result_is_data_not_an_rpc_error_channel() {
        let customer = customer_record();
        let result = alias(
            "acme/customer:customer#lookup-result",
            "lookup-result",
            reference(
                RESULT,
                vec![
                    reference(STRING, vec![]),
                    reference("Acme:Customer:Customer", vec![]),
                ],
            ),
        );

        let model = project(&package(vec![customer, result]), &AvroOptions::default()).unwrap();
        let root = model.root("acme/customer:customer#lookup-result").unwrap();
        let AvroType::Named(result_name) = root.tpe() else {
            panic!("Result must project to a named wrapper")
        };
        assert!(result_name.name().starts_with("ResultStringCustomer_"));
        assert_eq!(result_name.name().len(), "ResultStringCustomer_".len() + 12);
        let wrapper = model.named_schema(&result_name.to_string()).unwrap();
        let AvroType::Union(union) = wrapper.field("value").unwrap().tpe() else {
            panic!("Result wrapper value must be a union")
        };
        assert_eq!(
            union
                .branches()
                .iter()
                .map(|branch| match branch {
                    AvroType::Named(name) => name.name(),
                    other => panic!("Result branch must be named, got {other:?}"),
                })
                .collect::<Vec<_>>(),
            ["Err", "Ok"]
        );
        assert!(model.protocols().is_empty());
    }

    #[test]
    fn concrete_generic_applications_substitute_before_deriving_names() {
        let boxed = alias_with_params(
            "acme/shared:customer#box",
            "box",
            vec!["a"],
            TypeExpr::Record(vec![field("value", TypeExpr::Variable("a".to_owned()))]),
        );
        let boxes = alias(
            "acme/customer:customer#boxes",
            "boxes",
            TypeExpr::Record(vec![
                field(
                    "text",
                    reference("acme/shared:customer#box", vec![reference(STRING, vec![])]),
                ),
                field(
                    "number",
                    reference("acme/shared:customer#box", vec![reference(INT, vec![])]),
                ),
            ]),
        );

        let mut input = package(vec![boxes]);
        input.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["customer".to_owned()],
                types: vec![boxed],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let model = project(&input, &AvroOptions::default()).unwrap();
        let boxes = model.named_schema("acme.customer.customer.Boxes").unwrap();
        for (field_name, schema_prefix, value_type) in [
            ("text", "BoxString_", AvroType::String),
            ("number", "BoxInt_", AvroType::Long),
        ] {
            let AvroType::Named(actual_name) = boxes.field(field_name).unwrap().tpe() else {
                panic!("generic application must be named")
            };
            assert!(actual_name.name().starts_with(schema_prefix));
            assert_eq!(actual_name.name().len(), schema_prefix.len() + 12);
            assert_eq!(
                model
                    .named_schema(&actual_name.to_string())
                    .unwrap()
                    .field("value")
                    .unwrap()
                    .tpe(),
                &value_type
            );
        }

        let linked = project(
            &input,
            &AvroOptions {
                dependencies: Dependencies::Linked,
                ..AvroOptions::default()
            },
        )
        .unwrap();
        assert!(
            !linked
                .schemas()
                .iter()
                .any(|schema| schema.full_name().name().starts_with("BoxString_"))
        );
        assert!(
            !linked
                .schemas()
                .iter()
                .any(|schema| schema.full_name().name().starts_with("BoxInt_"))
        );
        assert_eq!(
            linked
                .linked_schemas()
                .iter()
                .filter(|schema| {
                    schema.full_name().name().starts_with("BoxString_")
                        || schema.full_name().name().starts_with("BoxInt_")
                })
                .count(),
            2
        );
        assert!(linked.root("acme/shared:customer#box").is_none());
        assert!(model.root("acme/shared:customer#box").is_none());
    }

    #[test]
    fn unbound_generic_at_an_artifact_root_is_avro002() {
        let source = "acme/customer:customer#box";
        let exposed = alias_with_params(
            source,
            "box",
            vec!["a", "z"],
            TypeExpr::Record(vec![field("value", TypeExpr::Variable("a".to_owned()))]),
        );

        let error = project(&package(vec![exposed]), &AvroOptions::default())
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO002");
        assert!(error[0].message().contains('a'));
        assert!(error[0].message().contains(source));
        assert_eq!(error[0].source(), Some(source));
        for severity in [
            morphir_extension_sdk::DiagnosticSeverity::Error,
            morphir_extension_sdk::DiagnosticSeverity::Warning,
        ] {
            let diagnostic = error[0].clone().into_diagnostic(severity);
            assert_eq!(diagnostic.severity, severity);
            assert_eq!(diagnostic.code.as_deref(), Some("AVRO002"));
            assert_eq!(diagnostic.message, error[0].message());
            let location = diagnostic.location.unwrap();
            assert_eq!(location.uri, format!("morphir-fqname:{source}"));
            assert_eq!(location.range.start.line, 0);
            assert_eq!(location.range.start.character, 0);
            assert_eq!(location.range.end.line, 0);
            assert_eq!(location.range.end.character, 0);
        }
    }

    #[test]
    fn aliases_inline_arbitrary_targets_or_wrap_them_while_records_stay_named() {
        let names_source = "acme/customer:customer#names";
        let label_source = "acme/customer:customer#label";
        let record_source = "acme/customer:customer#identity";
        let declarations = vec![
            alias(
                names_source,
                "names",
                reference("morphir/SDK:list#list", vec![reference(STRING, vec![])]),
            ),
            alias(label_source, "label", reference(names_source, vec![])),
            alias(
                record_source,
                "identity",
                TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
            ),
        ];

        let inline = project(&package(declarations.clone()), &AvroOptions::default()).unwrap();
        let expected = AvroType::Array(Box::new(AvroType::String), Default::default());
        assert_eq!(inline.root(names_source).unwrap().tpe(), &expected);
        assert_eq!(inline.root(label_source).unwrap().tpe(), &expected);
        assert!(matches!(
            inline.root(record_source).unwrap().tpe(),
            AvroType::Named(_)
        ));

        let wrapped = project(
            &package(declarations),
            &AvroOptions {
                aliases: Aliases::WrapperRecord,
                ..AvroOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            wrapped
                .named_schema("acme.customer.customer.Names")
                .unwrap()
                .field("value")
                .unwrap()
                .tpe(),
            &expected
        );
        assert!(matches!(
            wrapped
                .named_schema("acme.customer.customer.Label")
                .unwrap()
                .field("value")
                .unwrap()
                .tpe(),
            AvroType::Named(name) if name.to_string() == "acme.customer.customer.Names"
        ));
    }

    #[test]
    fn exact_sdk_logical_types_use_the_approved_physical_pairs() {
        let logicals = alias(
            "acme/customer:customer#logical-values",
            "logical-values",
            TypeExpr::Record(vec![
                field("date", reference(LOCAL_DATE, vec![])),
                field("time", reference(LOCAL_TIME, vec![])),
                field("instant", reference(INSTANT, vec![])),
                field("date-time", reference(DATE_TIME, vec![])),
                field("uuid", reference(UUID, vec![])),
                field("decimal", reference(DECIMAL, vec![])),
            ]),
        );
        let model = project(&package(vec![logicals]), &AvroOptions::default()).unwrap();
        let record = model
            .named_schema("acme.customer.customer.LogicalValues")
            .unwrap();

        assert_eq!(
            record.field("date").unwrap().tpe(),
            &logical(AvroType::Int, "date", &[])
        );
        assert_eq!(
            record.field("time").unwrap().tpe(),
            &logical(AvroType::Long, "time-micros", &[])
        );
        assert_eq!(
            record.field("instant").unwrap().tpe(),
            &logical(AvroType::Long, "timestamp-micros", &[])
        );
        assert_eq!(
            record.field("dateTime").unwrap().tpe(),
            &logical(AvroType::Long, "timestamp-micros", &[])
        );
        assert_eq!(
            record.field("uuid").unwrap().tpe(),
            &logical(AvroType::String, "uuid", &[])
        );
        assert_eq!(
            record.field("decimal").unwrap().tpe(),
            &logical(
                AvroType::Bytes,
                "decimal",
                &[("precision", json!(38)), ("scale", json!(10))],
            )
        );

        let physical = project(
            &package(vec![alias(
                "acme/customer:customer#date",
                "date",
                reference(LOCAL_DATE, vec![]),
            )]),
            &AvroOptions {
                logical_types: false,
                ..AvroOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            physical.root("acme/customer:customer#date").unwrap().tpe(),
            &AvroType::Int
        );
    }

    #[test]
    fn decimal_defaults_and_exact_mapping_overrides_are_applied() {
        let money_source = "acme/customer:customer#money";
        let money = alias(money_source, "money", reference(DECIMAL, vec![]));
        let envelope = alias(
            "acme/customer:customer#payment",
            "payment",
            TypeExpr::Record(vec![field("amount", reference(money_source, vec![]))]),
        );
        let mut options = AvroOptions::default();
        options.type_mappings.insert(
            money_source.to_owned(),
            TypeMapping {
                physical_type: "bytes".to_owned(),
                logical_type: Some("decimal".to_owned()),
                precision: Some(20),
                scale: Some(4),
            },
        );

        let model = project(&package(vec![money, envelope]), &options).unwrap();
        let payment = model
            .named_schema("acme.customer.customer.Payment")
            .unwrap();
        assert_eq!(
            payment.field("amount").unwrap().tpe(),
            &logical(
                AvroType::Bytes,
                "decimal",
                &[
                    ("morphir.fqname", json!(money_source)),
                    ("precision", json!(20)),
                    ("scale", json!(4)),
                ],
            )
        );

        options.type_mappings.insert(
            UUID.to_owned(),
            TypeMapping {
                physical_type: "bytes".to_owned(),
                logical_type: None,
                precision: None,
                scale: None,
            },
        );
        let uuid = alias(
            "acme/customer:customer#identifier",
            "identifier",
            reference(UUID, vec![]),
        );
        let overridden = project(&package(vec![uuid]), &options).unwrap();
        assert_eq!(
            overridden
                .root("acme/customer:customer#identifier")
                .unwrap()
                .tpe(),
            &AvroType::Annotated {
                physical: Box::new(AvroType::Bytes),
                properties: [("morphir.fqname".to_owned(), json!(UUID))].into(),
            }
        );
    }

    #[test]
    fn decimal_properties_require_the_decimal_logical_type() {
        let mut options = AvroOptions::default();
        options.type_mappings.insert(
            "acme/customer:customer#money".to_owned(),
            TypeMapping {
                physical_type: "bytes".to_owned(),
                logical_type: Some("uuid".to_owned()),
                precision: Some(20),
                scale: None,
            },
        );

        let error = project(&package(vec![]), &options)
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO004");
        assert!(error[0].message().contains("decimal"));
    }

    #[test]
    fn opaque_types_require_an_exact_mapping() {
        let source = "acme/customer:customer#token";
        let token = TypeDeclaration::Opaque {
            source_name: source.to_owned(),
            name: "token".to_owned(),
            type_params: Vec::new(),
            doc: None,
        };
        let error = project(&package(vec![token.clone()]), &AvroOptions::default())
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO001");
        assert!(error[0].message().contains(source));

        let mut options = AvroOptions::default();
        options.type_mappings.insert(
            source.to_owned(),
            TypeMapping {
                physical_type: "string".to_owned(),
                logical_type: None,
                precision: None,
                scale: None,
            },
        );
        let model = project(&package(vec![token]), &options).unwrap();
        assert!(matches!(
            model.root(source).unwrap().tpe(),
            AvroType::Annotated { physical, properties }
                if physical.as_ref() == &AvroType::String
                    && properties.get("morphir.fqname") == Some(&json!(source))
        ));
    }

    #[test]
    fn open_records_functions_and_incomplete_types_are_avro001() {
        let unsupported = [
            alias(
                "acme/customer:customer#open",
                "open",
                TypeExpr::Record(vec![field(
                    "nested",
                    TypeExpr::ExtensibleRecord {
                        variable: "row".to_owned(),
                        fields: vec![field("name", reference(STRING, vec![]))],
                    },
                )]),
            ),
            alias(
                "acme/customer:customer#callback",
                "callback",
                TypeExpr::Record(vec![field(
                    "nested",
                    TypeExpr::Function {
                        input: Box::new(reference(STRING, vec![])),
                        output: Box::new(reference(INT, vec![])),
                    },
                )]),
            ),
        ];
        for declaration in unsupported {
            let source = declaration.source_name().to_owned();
            let error = project(&package(vec![declaration]), &AvroOptions::default())
                .unwrap_err()
                .into_diagnostics()
                .unwrap();
            assert_eq!(error[0].code(), "AVRO001");
            assert!(error[0].message().contains(&source));
            assert_eq!(error[0].source(), Some(source.as_str()));
        }

        let source = "acme/customer:customer#unfinished";
        let incomplete = TypeDeclaration::Incomplete {
            source_name: source.to_owned(),
            name: "unfinished".to_owned(),
            type_params: Vec::new(),
            incompleteness: IncompletenessKind::Hole,
            partial_type: Some(reference(STRING, vec![])),
            doc: None,
        };
        let error = project(&package(vec![incomplete]), &AvroOptions::default())
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO001");
        assert!(error[0].message().contains(source));
    }

    #[test]
    fn named_record_recursion_is_safe_but_inline_alias_cycles_are_avro005() {
        let node_source = "acme/customer:customer#node";
        let node = alias(
            node_source,
            "node",
            TypeExpr::Record(vec![field(
                "next",
                reference(
                    "morphir/SDK:maybe#maybe",
                    vec![reference(node_source, vec![])],
                ),
            )]),
        );
        let model = project(&package(vec![node]), &AvroOptions::default()).unwrap();
        let node = model.named_schema("acme.customer.customer.Node").unwrap();
        assert!(matches!(
            node.field("next").unwrap().tpe(),
            AvroType::Union(union)
                if matches!(&union.branches()[1], AvroType::Named(name) if name.to_string() == "acme.customer.customer.Node")
        ));

        let loop_source = "acme/customer:customer#forever";
        let cycle = alias(loop_source, "forever", reference(loop_source, vec![]));
        let error = project(&package(vec![cycle]), &AvroOptions::default())
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO005");
        assert!(error[0].message().contains(loop_source));
    }

    #[test]
    fn recursive_generic_specializations_terminate_and_only_same_named_type_is_safe() {
        let tree_source = "acme/shared:types#tree";
        let tree = alias_with_params(
            tree_source,
            "tree",
            vec!["a"],
            TypeExpr::Record(vec![
                field("value", TypeExpr::Variable("a".to_owned())),
                field(
                    "children",
                    reference(
                        "morphir/SDK:list#list",
                        vec![reference(
                            tree_source,
                            vec![TypeExpr::Variable("a".to_owned())],
                        )],
                    ),
                ),
            ]),
        );
        let forest = alias(
            "acme/customer:customer#forest",
            "forest",
            reference(tree_source, vec![reference(STRING, vec![])]),
        );
        let options = AvroOptions::default();
        let mut safe_input = package(vec![forest]);
        safe_input.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![tree],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let model = project(&safe_input, &options).unwrap();
        let tree = model
            .schemas()
            .iter()
            .find(|schema| schema.full_name().name().starts_with("TreeString_"))
            .unwrap();
        let tree_name = tree.full_name().to_string();
        assert!(matches!(
            tree.field("children").unwrap().tpe(),
            AvroType::Array(element, _)
                if matches!(element.as_ref(), AvroType::Named(name) if name.to_string() == tree_name)
        ));

        let grow_source = "acme/shared:types#grow";
        let grow = alias_with_params(
            grow_source,
            "grow",
            vec!["a"],
            TypeExpr::Record(vec![field(
                "next",
                reference(
                    grow_source,
                    vec![reference(
                        "morphir/SDK:list#list",
                        vec![TypeExpr::Variable("a".to_owned())],
                    )],
                ),
            )]),
        );
        let dangerous = alias(
            "acme/customer:customer#dangerous",
            "dangerous",
            reference(grow_source, vec![reference(STRING, vec![])]),
        );
        let mut unsafe_input = package(vec![dangerous]);
        unsafe_input.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![grow],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let error = project(&unsafe_input, &options)
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error[0].code(), "AVRO005");
        assert!(error[0].message().contains(grow_source));
        assert_eq!(error[0].source(), Some("acme/customer:customer#dangerous"));
    }

    #[test]
    fn indexing_rejects_cross_package_name_collisions_and_duplicate_sources() {
        let owned = alias(
            "acme/foo-bar:domain#customer",
            "customer",
            TypeExpr::Record(Vec::new()),
        );
        let dependency = alias(
            "acme/foo_bar:domain#customer",
            "customer",
            TypeExpr::Record(Vec::new()),
        );
        let mut collision = package(vec![owned]);
        collision.package_name = "acme/foo-bar".to_owned();
        collision.modules[0].path = vec!["domain".to_owned()];
        collision.dependencies = vec![ProjectionDependency {
            package_name: "acme/foo_bar".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["domain".to_owned()],
                types: vec![dependency],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let error = project(
            &collision,
            &AvroOptions {
                dependencies: Dependencies::Linked,
                ..AvroOptions::default()
            },
        )
        .unwrap_err()
        .into_diagnostics()
        .unwrap();
        assert_eq!(error.len(), 2);
        assert!(
            error
                .iter()
                .all(|diagnostic| diagnostic.code() == "AVRO003")
        );
        assert_eq!(
            error
                .iter()
                .map(|diagnostic| diagnostic.source())
                .collect::<Vec<_>>(),
            [
                Some("acme/foo-bar:domain#customer"),
                Some("acme/foo_bar:domain#customer")
            ]
        );

        let source = "acme/customer:customer#duplicate";
        let mut duplicate = package(vec![alias(
            source,
            "duplicate",
            TypeExpr::Record(Vec::new()),
        )]);
        duplicate.dependencies = vec![ProjectionDependency {
            package_name: "other/package".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["other".to_owned()],
                types: vec![alias(source, "other-name", TypeExpr::Record(Vec::new()))],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let error = project(&duplicate, &AvroOptions::default())
            .unwrap_err()
            .into_diagnostics()
            .unwrap();
        assert_eq!(error.len(), 2);
        assert!(error.iter().all(|diagnostic| {
            diagnostic.code() == "AVRO003" && diagnostic.source() == Some(source)
        }));
    }

    #[test]
    fn generic_custom_and_result_names_digest_complete_canonical_types() {
        let envelope_source = "acme/shared:types#envelope";
        let envelope = custom(
            envelope_source,
            "envelope",
            vec!["a"],
            vec![constructor(
                "envelope",
                vec![field("value", TypeExpr::Variable("a".to_owned()))],
            )],
        );
        let pair_source = "acme/shared:types#pair";
        let pair = custom(
            pair_source,
            "pair",
            vec!["a", "b"],
            vec![constructor(
                "pair",
                vec![
                    field("left", TypeExpr::Variable("a".to_owned())),
                    field("right", TypeExpr::Variable("b".to_owned())),
                ],
            )],
        );
        let customers = [
            ("acme/one:domain#customer", "acme/one"),
            ("acme/two:domain#customer", "acme/two"),
        ];
        let roots = customers
            .iter()
            .enumerate()
            .map(|(index, (source, _))| {
                alias(
                    &format!("acme/customer:customer#wrapped-{index}"),
                    &format!("wrapped-{index}"),
                    reference(envelope_source, vec![reference(source, vec![])]),
                )
            })
            .chain(customers.iter().enumerate().map(|(index, (source, _))| {
                alias(
                    &format!("acme/customer:customer#result-{index}"),
                    &format!("result-{index}"),
                    reference(
                        RESULT,
                        vec![reference(STRING, vec![]), reference(source, vec![])],
                    ),
                )
            }))
            .chain(
                [
                    ("acme/types:domain#a-b", "acme/types:domain#c"),
                    ("acme/types:domain#a", "acme/types:domain#b-c"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (left, right))| {
                    alias(
                        &format!("acme/customer:customer#pair-{index}"),
                        &format!("pair-{index}"),
                        reference(
                            pair_source,
                            vec![reference(left, vec![]), reference(right, vec![])],
                        ),
                    )
                }),
            )
            .collect::<Vec<_>>();
        let mut input = package(roots);
        input.dependencies = std::iter::once(ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![envelope, pair],
                values: Vec::new(),
                doc: None,
            }],
        })
        .chain(
            customers
                .iter()
                .map(|(source, package_name)| ProjectionDependency {
                    package_name: (*package_name).to_owned(),
                    modules: vec![ProjectionModule {
                        path: vec!["domain".to_owned()],
                        types: vec![alias(source, "customer", TypeExpr::Record(Vec::new()))],
                        values: Vec::new(),
                        doc: None,
                    }],
                }),
        )
        .chain(std::iter::once(ProjectionDependency {
            package_name: "acme/types".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["domain".to_owned()],
                types: ["a-b", "c", "a", "b-c"]
                    .into_iter()
                    .map(|name| {
                        alias(
                            &format!("acme/types:domain#{name}"),
                            name,
                            TypeExpr::Record(Vec::new()),
                        )
                    })
                    .collect(),
                values: Vec::new(),
                doc: None,
            }],
        }))
        .collect();

        let model = project(&input, &AvroOptions::default()).unwrap();
        let envelope_names = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("EnvelopeCustomer_"))
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>();
        let result_names = model
            .schemas()
            .iter()
            .filter(|schema| {
                schema
                    .full_name()
                    .name()
                    .starts_with("ResultStringCustomer_")
            })
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>();
        let pair_names = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("PairABC_"))
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>();
        assert_eq!(envelope_names.len(), 2);
        assert_ne!(envelope_names[0], envelope_names[1]);
        assert_eq!(result_names.len(), 2);
        assert_ne!(result_names[0], result_names[1]);
        assert_eq!(pair_names.len(), 2);
        assert_ne!(pair_names[0], pair_names[1]);

        let repeat = project(&input, &AvroOptions::default()).unwrap();
        assert_eq!(model, repeat);
    }

    #[test]
    fn generic_record_alias_names_digest_full_fqnames_and_argument_boundaries() {
        let box_source = "acme/shared:types#box";
        let pair_source = "acme/shared:types#alias-pair";
        let generic_types = vec![
            alias_with_params(
                box_source,
                "box",
                vec!["a"],
                TypeExpr::Record(vec![field("value", TypeExpr::Variable("a".to_owned()))]),
            ),
            alias_with_params(
                pair_source,
                "alias-pair",
                vec!["a", "b"],
                TypeExpr::Record(vec![
                    field("left", TypeExpr::Variable("a".to_owned())),
                    field("right", TypeExpr::Variable("b".to_owned())),
                ]),
            ),
        ];
        let customer_sources = ["acme/one:domain#customer", "acme/two:domain#customer"];
        let ambiguous_arguments = [
            ("acme/types:domain#a-b", "acme/types:domain#c"),
            ("acme/types:domain#a", "acme/types:domain#b-c"),
        ];
        let roots = customer_sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                alias(
                    &format!("acme/customer:customer#box-{index}"),
                    &format!("box-{index}"),
                    reference(box_source, vec![reference(source, vec![])]),
                )
            })
            .chain(
                ambiguous_arguments
                    .iter()
                    .enumerate()
                    .map(|(index, (left, right))| {
                        alias(
                            &format!("acme/customer:customer#alias-pair-{index}"),
                            &format!("alias-pair-{index}"),
                            reference(
                                pair_source,
                                vec![reference(left, vec![]), reference(right, vec![])],
                            ),
                        )
                    }),
            )
            .collect();
        let mut input = package(roots);
        input.dependencies = vec![
            ProjectionDependency {
                package_name: "acme/shared".to_owned(),
                modules: vec![ProjectionModule {
                    path: vec!["types".to_owned()],
                    types: generic_types,
                    values: Vec::new(),
                    doc: None,
                }],
            },
            ProjectionDependency {
                package_name: "acme/one".to_owned(),
                modules: vec![ProjectionModule {
                    path: vec!["domain".to_owned()],
                    types: vec![alias(
                        customer_sources[0],
                        "customer",
                        TypeExpr::Record(Vec::new()),
                    )],
                    values: Vec::new(),
                    doc: None,
                }],
            },
            ProjectionDependency {
                package_name: "acme/two".to_owned(),
                modules: vec![ProjectionModule {
                    path: vec!["domain".to_owned()],
                    types: vec![alias(
                        customer_sources[1],
                        "customer",
                        TypeExpr::Record(Vec::new()),
                    )],
                    values: Vec::new(),
                    doc: None,
                }],
            },
            ProjectionDependency {
                package_name: "acme/types".to_owned(),
                modules: vec![ProjectionModule {
                    path: vec!["domain".to_owned()],
                    types: ["a-b", "c", "a", "b-c"]
                        .into_iter()
                        .map(|name| {
                            alias(
                                &format!("acme/types:domain#{name}"),
                                name,
                                TypeExpr::Record(Vec::new()),
                            )
                        })
                        .collect(),
                    values: Vec::new(),
                    doc: None,
                }],
            },
        ];

        let model = project(&input, &AvroOptions::default()).unwrap();
        let box_names = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("BoxCustomer_"))
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>();
        let pair_names = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("AliasPairABC_"))
            .map(|schema| schema.full_name().to_string())
            .collect::<Vec<_>>();
        assert_eq!(box_names.len(), 2);
        assert_ne!(box_names[0], box_names[1]);
        let AvroType::Named(first_box) = model.root("acme/customer:customer#box-0").unwrap().tpe()
        else {
            panic!("generic record alias root must reference its specialization")
        };
        assert_eq!(
            first_box.to_string(),
            "acme.shared.types.BoxCustomer_8d04f4548cd8"
        );
        assert_eq!(pair_names.len(), 2);
        assert_ne!(pair_names[0], pair_names[1]);
        assert_eq!(model, project(&input, &AvroOptions::default()).unwrap());
    }

    #[test]
    fn decimal_logical_mappings_reject_non_bytes_physical_types_even_when_unused() {
        for physical_type in ["string", "int", "long"] {
            let mut options = AvroOptions::default();
            options.type_mappings.insert(
                "acme/customer:customer#unused-money".to_owned(),
                TypeMapping {
                    physical_type: physical_type.to_owned(),
                    logical_type: Some("decimal".to_owned()),
                    precision: Some(20),
                    scale: Some(4),
                },
            );
            let error = project(&package(Vec::new()), &options)
                .unwrap_err()
                .into_diagnostics()
                .unwrap();
            assert_eq!(error[0].code(), "AVRO004");
            assert_eq!(
                error[0].source(),
                Some("acme/customer:customer#unused-money")
            );
        }
    }
}
