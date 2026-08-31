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
    fn finite_nested_generic_specializations_project_each_named_schema() {
        let box_source = "acme/shared:types#box";
        let boxed = alias_with_params(
            box_source,
            "box",
            vec!["a"],
            TypeExpr::Record(vec![field("value", TypeExpr::Variable("a".to_owned()))]),
        );
        let nested = alias(
            "acme/customer:customer#nested-box",
            "nested-box",
            reference(
                box_source,
                vec![reference(
                    box_source,
                    vec![reference("morphir/SDK:basics#int", vec![])],
                )],
            ),
        );
        let mut input = package(vec![nested]);
        input.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![boxed],
                values: Vec::new(),
                doc: None,
            }],
        }];

        let model = project(&input, &AvroOptions::default()).unwrap();
        let boxes = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("Box"))
            .collect::<Vec<_>>();

        assert_eq!(boxes.len(), 2);
        assert!(boxes.iter().any(|schema| {
            matches!(
                schema.field("value").unwrap().tpe(),
                AvroType::Named(name) if name.name().starts_with("BoxInt_")
            )
        }));
    }

    #[test]
    fn equal_complexity_generic_specializations_converge_to_a_named_cycle() {
        let switch_source = "acme/customer:customer#switch";
        let switch = custom(
            switch_source,
            "switch",
            vec!["a"],
            vec![constructor(
                "next",
                vec![field(
                    "value",
                    reference(switch_source, vec![reference(INT, vec![])]),
                )],
            )],
        );
        let root = alias(
            "acme/customer:customer#string-switch",
            "string-switch",
            reference(switch_source, vec![reference(STRING, vec![])]),
        );

        let mut input = package(vec![root]);
        input.dependencies = vec![ProjectionDependency {
            package_name: "acme/customer".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["customer".to_owned()],
                types: vec![switch],
                values: Vec::new(),
                doc: None,
            }],
        }];
        let model = project(&input, &AvroOptions::default()).unwrap();
        let switches = model
            .schemas()
            .iter()
            .filter(|schema| schema.full_name().name().starts_with("Switch"))
            .collect::<Vec<_>>();

        assert!(
            switches
                .iter()
                .any(|schema| schema.full_name().name().starts_with("SwitchString_"))
        );
        let integer = switches
            .iter()
            .find(|schema| schema.full_name().name().starts_with("SwitchInt_"))
            .expect("convergent integer specialization should be projected");
        let payload_name = match integer.field("value").unwrap().tpe() {
            AvroType::Union(union) => union
                .branches()
                .iter()
                .find_map(|branch| match branch {
                    AvroType::Named(name) if name.name().ends_with("Next") => Some(name),
                    _ => None,
                })
                .expect("custom wrapper should reference its payload"),
            other => panic!("custom wrapper should be a union, got {other:?}"),
        };
        let payload = model.named_schema(&payload_name.to_string()).unwrap();
        assert!(matches!(
            payload.field("value").unwrap().tpe(),
            AvroType::Named(name) if name == integer.full_name()
        ));
    }
