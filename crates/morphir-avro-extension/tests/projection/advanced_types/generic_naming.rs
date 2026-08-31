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