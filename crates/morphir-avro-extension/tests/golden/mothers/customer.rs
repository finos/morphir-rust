use super::super::*;

pub(crate) fn customer_package() -> ProjectionPackage {
    let mut input = package(vec![documented_customer_record()]);
    input.kind = DistributionKind::Application;
    input.modules[0].doc = Some("Customer operations.".to_owned());
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#find-customer",
            "find-customer",
            vec![field("id", reference(STRING, vec![]))],
            Some(reference(CUSTOMER, vec![])),
            ValueKind::Function,
            Some(EntryPointMetadata {
                identifier: "customer-query".to_owned(),
                kind: EntryPointKind::Command,
                doc: Some("Query a customer by ID.".to_owned()),
            }),
        ),
        value_specification(
            "acme/customer:customer#schema-version",
            "schema-version",
            vec![],
            Some(reference(STRING, vec![])),
            ValueKind::Constant,
            None,
        ),
    ];
    input
}

pub(crate) fn documented_customer_record() -> TypeDeclaration {
    let mut declaration = customer_record();
    let TypeDeclaration::Alias { doc, .. } = &mut declaration else {
        unreachable!("customer fixture is an alias")
    };
    *doc = Some("A customer record.".to_owned());
    declaration
}

pub(crate) fn alias_wrapper_package() -> ProjectionPackage {
    package(vec![alias(
        "acme/customer:customer#customer-labels",
        "customer-labels",
        reference("morphir/SDK:list#list", vec![reference(STRING, vec![])]),
    )])
}

pub(crate) fn generic_result_package() -> ProjectionPackage {
    package(vec![alias(
        "acme/customer:customer#lookup-result",
        "lookup-result",
        reference(
            RESULT,
            vec![reference(STRING, vec![]), reference(STRING, vec![])],
        ),
    )])
}

pub(crate) fn logical_constants_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#logical-values",
        "logical-values",
        TypeExpr::Record(vec![
            field("as-of", reference(LOCAL_DATE, vec![])),
            field("amount", reference(DECIMAL, vec![])),
            field("tags", reference(SET, vec![reference(STRING, vec![])])),
        ]),
    )]);
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#logical-defaults",
        "logical-defaults",
        vec![],
        Some(reference("acme/customer:customer#logical-values", vec![])),
        ValueKind::Constant,
        None,
    )];
    input
}

pub(crate) fn idl_custom_types_package() -> ProjectionPackage {
    let custom_values = alias(
        "acme/customer:customer#custom-values",
        "custom-values",
        TypeExpr::Record(vec![
            field("amount", reference(DECIMAL, vec![])),
            field("as-of", reference(LOCAL_DATE, vec![])),
            field("identifier", reference(UUID, vec![])),
            field(
                "mapped-date",
                reference("acme/customer:customer#legacy-date", vec![]),
            ),
            field(
                "mapped-identifier",
                reference("acme/customer:customer#binary-id", vec![]),
            ),
            field(
                "mapped-amount",
                reference("acme/customer:customer#money", vec![]),
            ),
            field("initial", reference(CHAR, vec![])),
            field(
                "labels",
                reference(
                    DICT,
                    vec![reference(STRING, vec![]), reference(STRING, vec![])],
                ),
            ),
            field(
                "nickname",
                reference(MAYBE, vec![reference(STRING, vec![])]),
            ),
            field("observed-at", reference(INSTANT, vec![])),
            field("opens-at", reference(LOCAL_TIME, vec![])),
            field("record", reference(STRING, vec![])),
            field("tags", reference(SET, vec![reference(STRING, vec![])])),
        ]),
    );
    let status = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#status".to_owned(),
        name: "status".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#active".to_owned(),
                name: "active".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#inactive".to_owned(),
                name: "inactive".to_owned(),
                arguments: Vec::new(),
            },
        ],
        doc: Some("Customer status.".to_owned()),
    };
    let shape = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#shape".to_owned(),
        name: "shape".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#point".to_owned(),
                name: "point".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#circle".to_owned(),
                name: "circle".to_owned(),
                arguments: vec![field("radius", reference("morphir/SDK:basics#int", vec![]))],
            },
        ],
        doc: Some("A shape with payload constructors.".to_owned()),
    };
    let mapped = [
        ("acme/customer:customer#legacy-date", "legacy-date"),
        ("acme/customer:customer#binary-id", "binary-id"),
        ("acme/customer:customer#money", "money"),
    ]
    .map(|(source_name, name)| TypeDeclaration::Opaque {
        source_name: source_name.to_owned(),
        name: name.to_owned(),
        type_params: Vec::new(),
        doc: None,
    });
    let mut input = package(
        [custom_values, status, shape]
            .into_iter()
            .chain(mapped)
            .collect(),
    );
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#custom-defaults",
            "custom-defaults",
            vec![],
            Some(reference("acme/customer:customer#custom-values", vec![])),
            ValueKind::Constant,
            None,
        ),
        value_specification(
            "acme/customer:customer#error",
            "error",
            vec![],
            Some(reference(STRING, vec![])),
            ValueKind::Function,
            None,
        ),
    ];
    input
}

pub(crate) fn idl_custom_types_options() -> AvroOptions {
    let mut options = AvroOptions {
        representation: morphir_avro_extension::Representation::Idl,
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };
    for (source, physical_type, logical_type, precision, scale) in [
        (
            "acme/customer:customer#legacy-date",
            "long",
            "date",
            None,
            None,
        ),
        (
            "acme/customer:customer#binary-id",
            "bytes",
            "uuid",
            None,
            None,
        ),
        (
            "acme/customer:customer#money",
            "bytes",
            "decimal",
            Some(20),
            Some(4),
        ),
    ] {
        options.type_mappings.insert(
            source.to_owned(),
            TypeMapping {
                physical_type: physical_type.to_owned(),
                logical_type: Some(logical_type.to_owned()),
                precision,
                scale,
            },
        );
    }
    options
}

pub(crate) fn idl_escaping_package() -> ProjectionPackage {
    let mut input = customer_package();
    input.modules[0].doc = Some("Protocol */ docs\\path\ncontrol\u{0001}line".to_owned());
    let TypeDeclaration::Alias { doc, .. } = &mut input.modules[0].types[0] else {
        unreachable!("customer is an alias")
    };
    *doc = Some("Record */ docs\\path\ncontrol\u{0003}line".to_owned());
    input.modules[0].values[0].doc = Some("Message */ docs\\path\ncontrol\u{0004}line".to_owned());
    input.modules[0].values[0]
        .entry_point
        .as_mut()
        .expect("entry point")
        .doc = Some("Entry \\ docs\ncontrol\u{0002}".to_owned());
    input
}

pub(crate) fn idl_primitive_protocol_package() -> ProjectionPackage {
    let mut input = package(Vec::new());
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#primitive-response",
        "primitive-response",
        Vec::new(),
        Some(reference(STRING, vec![])),
        ValueKind::Function,
        None,
    )];
    input
}
