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

