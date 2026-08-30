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

