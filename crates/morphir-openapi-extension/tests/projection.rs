use morphir_openapi_extension::{
    Schema, SchemaDiagnostic, SchemaField, SchemaOptions, SchemaProjection, Unsupported, project,
};
use morphir_projection::{
    Constructor, DistributionKind, IncompletenessKind, NamedType, ProjectionModule,
    ProjectionPackage, TypeDeclaration, TypeExpr, normalize, testing::classic,
};

fn projection(ir: serde_json::Value) -> SchemaProjection {
    let package = normalize(&ir).expect("the fixture normalizes");
    project(&package, &SchemaOptions::default()).expect("the fixture projects")
}

fn root<'a>(projection: &'a SchemaProjection, source_name: &str) -> &'a Schema {
    &projection
        .roots
        .iter()
        .find(|root| root.source_name == source_name)
        .unwrap_or_else(|| panic!("no root for {source_name}"))
        .schema
}

#[test]
fn projects_a_record_alias_as_an_object_with_required_fields() {
    let projection = projection(classic::classic_schema_library());

    let Schema::Object { fields, required } = root(&projection, "acme/customer:customer#customer")
    else {
        panic!("a record alias projects as an object");
    };
    assert!(fields.iter().any(|field| field.name == "customerId"));
    assert!(required.contains(&"customerId".to_owned()));
}

#[test]
fn projects_maybe_as_a_union_with_null() {
    let projection = projection(classic::classic_schema_library());

    let optional = projection
        .definitions
        .values()
        .flat_map(|named| match &named.schema {
            Schema::Object { fields, .. } => fields.clone(),
            _ => Vec::new(),
        })
        .find(|field| matches!(field.schema, Schema::Union(_)))
        .expect("the fixture has an optional field");

    let Schema::Union(members) = optional.schema else {
        unreachable!("filtered above");
    };
    assert!(members.iter().any(|member| matches!(member, Schema::Null)));
}

#[test]
fn projects_a_nullary_custom_type_as_an_enumeration() {
    let projection = projection(classic::classic_schema_library());

    let enumeration = projection
        .definitions
        .values()
        .find(|named| matches!(named.schema, Schema::Enumeration(_)))
        .expect("the fixture has a nullary custom type");

    let Schema::Enumeration(values) = &enumeration.schema else {
        unreachable!("filtered above");
    };
    assert!(!values.is_empty());
    assert_eq!(values.clone(), {
        let mut sorted = values.clone();
        sorted.sort();
        sorted
    });
}

#[test]
fn a_name_collision_is_an_error_rather_than_a_rename() {
    let package =
        normalize(&classic::classic_colliding_names_library()).expect("the fixture normalizes");

    let error =
        project(&package, &SchemaOptions::default()).expect_err("a collision fails projection");

    assert_eq!(error.code(), "JSC004");
}

#[test]
fn strict_mode_fails_on_a_function_used_as_data() {
    let package =
        normalize(&classic::classic_function_field_library()).expect("the fixture normalizes");

    let error =
        project(&package, &SchemaOptions::default()).expect_err("a function field has no schema");

    assert_eq!(error.code(), "JSC003");
}

#[test]
fn warn_and_skip_omits_the_form_and_keeps_the_rest() {
    let package =
        normalize(&classic::classic_function_field_library()).expect("the fixture normalizes");
    let options = SchemaOptions {
        unsupported: Unsupported::WarnAndSkip,
        ..SchemaOptions::default()
    };

    let projection = project(&package, &options).expect("skipping keeps projection successful");

    assert!(
        projection
            .diagnostics
            .iter()
            .any(|(diagnostic, warning)| { *warning && diagnostic.code() == "JSC003" })
    );
    assert!(!projection.roots.is_empty());
    assert!(
        !projection
            .roots
            .iter()
            .any(|root| root.source_name == "acme/customer:customer#handler")
    );
}

/// Every scalar and collection row of the type-mapping table, asserted through
/// the real IR path so a misspelled SDK FQName constant cannot pass silently.
#[test]
fn projects_every_scalar_and_collection_form() {
    let projection = projection(classic::classic_schema_library());

    let Schema::Object { fields, .. } = root(&projection, "acme/customer:customer#metrics") else {
        panic!("a record alias projects as an object");
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("no field {name}"))
            .schema
            .clone()
    };
    let text = Schema::Text { max_length: None };

    assert_eq!(field("active"), Schema::Boolean);
    assert_eq!(
        field("count"),
        Schema::Integer {
            format: Some("int64")
        }
    );
    assert_eq!(
        field("ratio"),
        Schema::Number {
            format: Some("double")
        }
    );
    assert_eq!(
        field("grade"),
        Schema::Text {
            max_length: Some(1)
        }
    );
    assert_eq!(field("nothing"), Schema::Null);
    assert_eq!(
        field("tags"),
        Schema::Array {
            items: Box::new(text.clone()),
            unique: false
        }
    );
    assert_eq!(
        field("labels"),
        Schema::Array {
            items: Box::new(text),
            unique: true
        }
    );
    assert_eq!(
        field("scores"),
        Schema::Map {
            values: Box::new(Schema::Number {
                format: Some("double")
            })
        }
    );
    assert_eq!(
        field("extent"),
        Schema::Tuple(vec![
            Schema::Integer {
                format: Some("int64")
            },
            Schema::Integer {
                format: Some("int64")
            }
        ])
    );
}

#[test]
fn projects_a_reference_to_a_declared_type_as_a_reference_to_its_schema_name() {
    let projection = projection(classic::classic_schema_library());

    let Schema::Object { fields, .. } = root(&projection, "acme/customer:customer#customer") else {
        panic!("a record alias projects as an object");
    };
    let status = fields
        .iter()
        .find(|field| field.name == "status")
        .expect("the fixture has a field referring to a sibling declaration");

    assert_eq!(status.schema, Schema::Reference("Status".to_owned()));
    assert!(projection.definitions.contains_key("Status"));
}

#[test]
fn projects_a_custom_type_with_payloads_as_a_tagged_choice() {
    let projection = projection(classic::classic_schema_library());

    let Schema::OneOf {
        discriminator,
        variants,
    } = root(&projection, "acme/customer:customer#shape")
    else {
        panic!("a custom type with payloads projects as a tagged choice");
    };

    assert_eq!(discriminator, "kind");
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Circle", "RoundedBox"]
    );
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "acme/customer:customer#circle",
            "acme/customer:customer#rounded-box"
        ]
    );
    assert_eq!(
        variants[0].schema,
        Schema::Object {
            fields: vec![SchemaField {
                name: "radius".to_owned(),
                schema: Schema::Number {
                    format: Some("double")
                },
                required: true,
                doc: None,
            }],
            required: vec!["radius".to_owned()],
        }
    );
    let Schema::Object { fields, .. } = &variants[1].schema else {
        panic!("a variant payload projects as an object");
    };
    // Constructor arguments are positional, so they keep their declared order
    // rather than the alphabetical order normalization gives record fields.
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["width", "height"]
    );
}

const SDK_STRING: &str = "morphir/SDK:string#string";
const SDK_INT: &str = "morphir/SDK:basics#int";
const SDK_BOOL: &str = "morphir/SDK:basics#bool";
const SDK_DICT: &str = "morphir/SDK:dict#dict";

/// The canonical FQName a fixture declaration is given.
fn source(local: &str) -> String {
    format!("acme/customer:customer#{local}")
}

fn package(types: Vec<TypeDeclaration>) -> ProjectionPackage {
    ProjectionPackage {
        kind: DistributionKind::Library,
        package_name: "acme/customer".to_owned(),
        dependencies: Vec::new(),
        modules: vec![ProjectionModule {
            path: vec!["customer".to_owned()],
            types,
            values: Vec::new(),
            doc: None,
        }],
    }
}

fn alias(local: &str, value: TypeExpr) -> TypeDeclaration {
    TypeDeclaration::Alias {
        source_name: source(local),
        name: local.to_owned(),
        type_params: Vec::new(),
        value,
        doc: None,
    }
}

fn record(fields: Vec<(&str, TypeExpr)>) -> TypeExpr {
    TypeExpr::Record(
        fields
            .into_iter()
            .map(|(name, tpe)| NamedType {
                name: name.to_owned(),
                tpe,
            })
            .collect(),
    )
}

fn reference(source_name: &str, arguments: Vec<TypeExpr>) -> TypeExpr {
    TypeExpr::Reference {
        source_name: source_name.to_owned(),
        arguments,
    }
}

/// A record alias holding a function, which no data schema can represent.
fn unsupported_alias(local: &str) -> TypeDeclaration {
    alias(
        local,
        record(vec![(
            "run",
            TypeExpr::Function {
                input: Box::new(reference(SDK_STRING, Vec::new())),
                output: Box::new(reference(SDK_BOOL, Vec::new())),
            },
        )]),
    )
}

fn strict_failure(types: Vec<TypeDeclaration>) -> SchemaDiagnostic {
    project(&package(types), &SchemaOptions::default())
        .expect_err("the declaration has no schema projection")
}

fn skipping(types: Vec<TypeDeclaration>) -> SchemaProjection {
    let options = SchemaOptions {
        unsupported: Unsupported::WarnAndSkip,
        ..SchemaOptions::default()
    };
    project(&package(types), &options).expect("skipping keeps projection successful")
}

/// Every rejection row of the type-mapping table reports the same stable code.
#[test]
fn every_unsupported_morphir_form_is_reported_as_jsc003() {
    let cases: Vec<(&str, TypeDeclaration)> = vec![
        (
            "extensible record",
            alias(
                "open",
                TypeExpr::ExtensibleRecord {
                    variable: "row".to_owned(),
                    fields: Vec::new(),
                },
            ),
        ),
        (
            "opaque declaration",
            TypeDeclaration::Opaque {
                source_name: source("token"),
                name: "token".to_owned(),
                type_params: Vec::new(),
                doc: None,
            },
        ),
        (
            "incomplete declaration",
            TypeDeclaration::Incomplete {
                source_name: source("draft"),
                name: "draft".to_owned(),
                type_params: Vec::new(),
                incompleteness: IncompletenessKind::Hole,
                partial_type: None,
                doc: None,
            },
        ),
        (
            "unbound type parameter",
            TypeDeclaration::Alias {
                source_name: source("box"),
                name: "box".to_owned(),
                type_params: vec!["a".to_owned()],
                value: record(vec![("value", TypeExpr::Variable("a".to_owned()))]),
                doc: None,
            },
        ),
        (
            "Dict with a non-String key",
            alias(
                "index",
                reference(
                    SDK_DICT,
                    vec![
                        reference(SDK_INT, Vec::new()),
                        reference(SDK_STRING, Vec::new()),
                    ],
                ),
            ),
        ),
        ("function used as data", unsupported_alias("handler")),
    ];

    for (label, declaration) in cases {
        let error = strict_failure(vec![declaration]);
        assert_eq!(error.code(), "JSC003", "{label} must report JSC003");
    }
}

#[test]
fn warn_and_skip_drops_a_declaration_that_refers_to_a_skipped_one() {
    let projection = skipping(vec![
        alias(
            "customer",
            record(vec![("customer-id", reference(SDK_STRING, Vec::new()))]),
        ),
        unsupported_alias("handler"),
        alias(
            "registry",
            record(vec![("handler", reference(&source("handler"), Vec::new()))]),
        ),
    ]);

    assert_eq!(
        projection.definitions.keys().collect::<Vec<_>>(),
        vec!["Customer"]
    );
    assert_eq!(
        projection
            .roots
            .iter()
            .map(|root| root.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("customer")]
    );

    let warned = projection
        .diagnostics
        .iter()
        .filter(|(diagnostic, warning)| *warning && diagnostic.code() == "JSC003")
        .filter_map(|(diagnostic, _)| diagnostic.source())
        .collect::<Vec<_>>();
    assert!(
        warned.contains(&source("handler").as_str()),
        "the unsupported declaration is warned about: {warned:?}"
    );
    assert!(
        warned.contains(&source("registry").as_str()),
        "the referrer that had to go with it is warned about: {warned:?}"
    );
}

#[test]
fn dropping_cascades_through_referrers_and_leaves_a_reference_cycle_intact() {
    let projection = skipping(vec![
        // A mutually recursive pair that projects cleanly and must survive.
        alias(
            "node-a",
            record(vec![("next", reference(&source("node-b"), Vec::new()))]),
        ),
        alias(
            "node-b",
            record(vec![("previous", reference(&source("node-a"), Vec::new()))]),
        ),
        // A chain that must be dropped one link at a time.
        unsupported_alias("handler"),
        alias(
            "middle",
            record(vec![("handler", reference(&source("handler"), Vec::new()))]),
        ),
        alias(
            "outer",
            record(vec![("middle", reference(&source("middle"), Vec::new()))]),
        ),
    ]);

    assert_eq!(
        projection.definitions.keys().collect::<Vec<_>>(),
        vec!["NodeA", "NodeB"]
    );
    assert_eq!(
        projection
            .roots
            .iter()
            .map(|root| root.name.as_str())
            .collect::<Vec<_>>(),
        vec!["NodeA", "NodeB"]
    );

    let warned = projection
        .diagnostics
        .iter()
        .filter(|(diagnostic, warning)| *warning && diagnostic.code() == "JSC003")
        .filter_map(|(diagnostic, _)| diagnostic.source())
        .collect::<Vec<_>>();
    for local in ["handler", "middle", "outer"] {
        assert!(
            warned.contains(&source(local).as_str()),
            "{local} is warned about: {warned:?}"
        );
    }
}

/// A package whose only declaration is unsupported, and so gets skipped
/// under `WarnAndSkip`, still carries its own name: `package_name` comes
/// straight from `ProjectionPackage::package_name`, not reconstructed from a
/// root, so a package with zero roots is not a package with no name.
#[test]
fn names_itself_from_the_package_even_when_every_declaration_is_skipped() {
    let projection = skipping(vec![unsupported_alias("handler")]);

    assert!(projection.roots.is_empty());
    assert!(projection.definitions.is_empty());
    assert_eq!(projection.package_name, "acme/customer");
}

/// A custom type declaration with the given constructors, each argument list
/// written as `(argument name, type)` pairs.
fn custom(local: &str, constructors: Vec<(&str, Vec<(&str, TypeExpr)>)>) -> TypeDeclaration {
    TypeDeclaration::Custom {
        source_name: source(local),
        name: local.to_owned(),
        type_params: Vec::new(),
        constructors: constructors
            .into_iter()
            .map(|(name, arguments)| Constructor {
                source_name: source(&name.to_lowercase()),
                name: name.to_owned(),
                arguments: arguments
                    .into_iter()
                    .map(|(name, tpe)| NamedType {
                        name: name.to_owned(),
                        tpe,
                    })
                    .collect(),
            })
            .collect(),
        doc: None,
    }
}

/// `Shape = Circle { kind: Int, radius: String } | Square { side: String }`.
/// `kind` is an ordinary Morphir identifier, so nothing stops a package from
/// declaring it — and it is exactly the property the discriminator claims.
fn shape_shadowing_the_discriminator() -> TypeDeclaration {
    custom(
        "shape",
        vec![
            (
                "Circle",
                vec![
                    ("kind", reference(SDK_INT, Vec::new())),
                    ("radius", reference(SDK_STRING, Vec::new())),
                ],
            ),
            ("Square", vec![("side", reference(SDK_STRING, Vec::new()))]),
        ],
    )
}

/// Without this rule the discriminator overwrites the constructor's own
/// `kind` property and appends a second `"kind"` to `required` — a document
/// JSON Schema 2020-12 and the OpenAPI 3.0 metaschema both reject, generated
/// with no diagnostic at all. The collision is a `JSC003` instead.
#[test]
fn a_constructor_argument_shadowing_the_discriminator_is_an_error() {
    let diagnostic = strict_failure(vec![shape_shadowing_the_discriminator()]);

    assert_eq!(diagnostic.code(), "JSC003");
    assert_eq!(diagnostic.source(), Some(source("shape").as_str()));
    assert!(
        diagnostic.message().contains("Circle") && diagnostic.message().contains("'kind'"),
        "the diagnostic names the constructor and the colliding property: {}",
        diagnostic.message()
    );
}

/// The collision is an unsupported Morphir form like any other, so
/// `unsupported: "warn-and-skip"` omits just this declaration and keeps the
/// rest of the package, rather than failing the whole generation.
#[test]
fn warn_and_skip_omits_a_constructor_that_shadows_the_discriminator() {
    let projection = skipping(vec![
        shape_shadowing_the_discriminator(),
        alias("label", reference(SDK_STRING, Vec::new())),
    ]);

    assert_eq!(
        projection
            .roots
            .iter()
            .map(|root| root.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("label")],
        "the colliding declaration is dropped and the rest still projects"
    );
    assert_eq!(
        projection
            .diagnostics
            .iter()
            .map(|(diagnostic, warning)| (diagnostic.code(), *warning))
            .collect::<Vec<_>>(),
        vec![("JSC003", true)]
    );
}

/// A single-constructor custom type still carries a discriminator, so the
/// same collision applies to it: the rule is about the discriminator, not
/// about there being more than one variant to tell apart.
#[test]
fn the_rule_applies_to_a_single_constructor_custom_type() {
    let diagnostic = strict_failure(vec![custom(
        "wrapper",
        vec![("Wrap", vec![("kind", reference(SDK_STRING, Vec::new()))])],
    )]);

    assert_eq!(diagnostic.code(), "JSC003");
}

/// A nullary custom type projects as a string enumeration with no
/// discriminator property at all, so a constructor *named* `Kind` is fine.
/// The rule is about argument names, and only where a discriminator is
/// actually written.
#[test]
fn a_constructor_named_kind_with_no_arguments_still_projects() {
    let projection = project(
        &package(vec![custom(
            "flavor",
            vec![("Kind", Vec::new()), ("Other", Vec::new())],
        )]),
        &SchemaOptions::default(),
    )
    .expect("a nullary custom type has no discriminator to collide with");

    assert_eq!(
        root(&projection, &source("flavor")),
        &Schema::Enumeration(vec!["Kind".to_owned(), "Other".to_owned()])
    );
}
