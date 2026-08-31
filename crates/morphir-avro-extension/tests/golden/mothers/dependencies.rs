use super::super::*;

pub(crate) fn idl_linked_chain_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#chain-order",
        "chain-order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/chain:model#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/chain".to_owned(),
        modules: vec![
            ProjectionModule {
                path: vec!["identity".to_owned()],
                types: vec![alias(
                    "acme/chain:identity#identifier",
                    "identifier",
                    TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
                )],
                values: Vec::new(),
                doc: None,
            },
            ProjectionModule {
                path: vec!["model".to_owned()],
                types: vec![alias(
                    "acme/chain:model#customer",
                    "customer",
                    TypeExpr::Record(vec![field(
                        "identifier",
                        reference("acme/chain:identity#identifier", vec![]),
                    )]),
                )],
                values: Vec::new(),
                doc: None,
            },
        ],
    }];
    input
}

pub(crate) fn partial_package() -> ProjectionPackage {
    package(vec![
        alias(
            "acme/customer:customer#supported",
            "supported",
            reference(STRING, vec![]),
        ),
        alias(
            "acme/customer:customer#unsupported",
            "unsupported",
            TypeExpr::Function {
                input: Box::new(reference(STRING, vec![])),
                output: Box::new(reference(STRING, vec![])),
            },
        ),
    ])
}

pub(crate) fn linked_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#order",
        "order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/shared:types#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![alias(
                "acme/shared:types#customer",
                "customer",
                TypeExpr::Record(vec![field("id", reference(STRING, vec![]))]),
            )],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

pub(crate) fn linked_alias_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#customer-alias",
        "customer-alias",
        reference("acme/shared:types#customer", vec![]),
    )]);
    input.dependencies = linked_package().dependencies;
    input
}

pub(crate) fn linked_chain_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#order",
        "order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/shared:types#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![
                alias(
                    "acme/shared:types#customer",
                    "customer",
                    TypeExpr::Record(vec![field(
                        "identifier",
                        reference("acme/shared:types#identifier", vec![]),
                    )]),
                ),
                alias(
                    "acme/shared:types#identifier",
                    "identifier",
                    TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
                ),
            ],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

pub(crate) fn linked_cross_ownership_cycle_package() -> ProjectionPackage {
    let dependency = "acme/shared:types#dependency";
    let owned_shared = "acme/customer:customer#z-owned-shared";
    let mut input = package(vec![
        alias(
            "acme/customer:customer#a-root",
            "a-root",
            TypeExpr::Record(vec![field("dependency", reference(dependency, vec![]))]),
        ),
        alias(
            owned_shared,
            "z-owned-shared",
            TypeExpr::Record(vec![field("cycle", reference(dependency, vec![]))]),
        ),
    ]);
    input.dependencies = vec![ProjectionDependency {
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
    input
}

pub(crate) fn acyclic_protocol_package() -> ProjectionPackage {
    let identifier = alias(
        "acme/customer:customer#identifier",
        "identifier",
        TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
    );
    let customer = alias(
        "acme/customer:customer#customer",
        "customer",
        TypeExpr::Record(vec![field(
            "identifier",
            reference("acme/customer:customer#identifier", vec![]),
        )]),
    );
    protocol_package(
        vec![identifier, customer],
        vec![field(
            "customer",
            reference("acme/customer:customer#customer", vec![]),
        )],
        reference("acme/customer:customer#identifier", vec![]),
    )
}

pub(crate) fn mutually_recursive_protocol_package() -> ProjectionPackage {
    let a = alias(
        "acme/customer:customer#a",
        "a",
        TypeExpr::Record(vec![field(
            "b",
            reference("acme/customer:customer#b", vec![]),
        )]),
    );
    let b = alias(
        "acme/customer:customer#b",
        "b",
        TypeExpr::Record(vec![field(
            "a",
            reference("acme/customer:customer#a", vec![]),
        )]),
    );
    protocol_package(
        vec![a, b],
        vec![field(
            "input",
            reference("acme/customer:customer#a", vec![]),
        )],
        reference("acme/customer:customer#b", vec![]),
    )
}

pub(crate) fn mutually_recursive_schema_package() -> ProjectionPackage {
    package(vec![
        alias(
            "acme/customer:customer#a",
            "a",
            TypeExpr::Record(vec![field(
                "b",
                reference("acme/customer:customer#b", vec![]),
            )]),
        ),
        alias(
            "acme/customer:customer#b",
            "b",
            TypeExpr::Record(vec![field(
                "a",
                reference("acme/customer:customer#a", vec![]),
            )]),
        ),
    ])
}

pub(crate) fn linked_self_loop_package() -> ProjectionPackage {
    let node = "acme/shared:types#node";
    let mut input = package(vec![alias(
        "acme/customer:customer#root",
        "root",
        TypeExpr::Record(vec![field("node", reference(node, vec![]))]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![alias(
                node,
                "node",
                TypeExpr::Record(vec![field("next", reference(node, vec![]))]),
            )],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

pub(crate) fn protocol_package(
    types: Vec<TypeDeclaration>,
    inputs: Vec<morphir_avro_extension::NamedType>,
    output: TypeExpr,
) -> ProjectionPackage {
    let mut input = package(types);
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#exchange",
        "exchange",
        inputs,
        Some(output),
        ValueKind::Function,
        None,
    )];
    input
}

pub(crate) fn options(projection: Projection) -> AvroOptions {
    AvroOptions {
        projection,
        ..AvroOptions::default()
    }
}
