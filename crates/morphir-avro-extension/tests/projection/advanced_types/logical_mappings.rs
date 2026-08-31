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

