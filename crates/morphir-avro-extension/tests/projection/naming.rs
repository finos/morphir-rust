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

