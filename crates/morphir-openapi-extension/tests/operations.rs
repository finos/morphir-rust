//! Behavior of `paths` synthesis from Morphir entry points and public values.
//!
//! The plan's brief names `mothers::v4::customer_application()` and
//! `mothers::classic::customer_library()`; the real fixtures are
//! `morphir_projection::testing::v4::{v4_customer_application, v4_customer_library,
//! v4_customer_specs}`. Verified by reading
//! `crates/morphir-projection/src/testing/v4.rs`: `v4_customer_application()`
//! declares two entry points on `acme/customer:domain` — `customer-query`
//! (command, targeting `find-customer`, a function of one input) and
//! `unfinished` (handler, targeting a zero-input constant). `v4_customer_library()`
//! is a Library distribution and `v4_customer_specs()` is a Specs
//! distribution over the same package; `normalize()` only ever attaches
//! entry-point metadata to a value when normalizing an `Application`
//! distribution (`crates/morphir-projection/src/normalize/v4/mod.rs`), so
//! both prove "no declared entry points" from the distribution kind itself,
//! not merely because a fixture happens to omit one.
//!
//! Every v4 fixture that shares `v4_library_content()` also declares
//! `acme/customer:domain#complex` (a generic alias whose body uses its own
//! unbound type parameter) and `acme/customer:domain#secret` (a custom type
//! with private constructors, normalized to `TypeDeclaration::Opaque`).
//! Both are genuine `JSC003`s, unrelated to operations. Every test below
//! sets `unsupported: "warn-and-skip"` so those two declarations are
//! skipped with a warning while the rest of the package still projects.

use std::collections::HashMap;

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::{
    OpenApiExtension, Projection, SchemaOptions, Unsupported, project, project_operations,
};
use morphir_projection::testing::v4;
use morphir_projection::{
    DistributionKind, EntryPointKind, EntryPointMetadata, NamedType, ProjectionDependency,
    ProjectionModule, ProjectionPackage, TypeDeclaration, TypeExpr, ValueKind, ValueSpecification,
};
use serde_json::{Value, json};

fn document(ir: Value, options: HashMap<String, Value>) -> Value {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir,
            target: "openapi".into(),
            options,
        })
        .expect("generation is a successful MEP call");
    assert!(result.success, "{:?}", result.diagnostics);
    serde_json::from_str(&result.artifacts[0].content).expect("valid JSON")
}

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[test]
fn schemas_mode_emits_no_paths() {
    let document = document(
        v4::v4_customer_application(),
        map([("unsupported", json!("warn-and-skip"))]),
    );

    assert_eq!(document["paths"], json!({}));
}

#[test]
fn entry_point_mode_posts_to_a_module_scoped_path() {
    let document = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    let paths = document["paths"].as_object().expect("paths is an object");
    assert!(!paths.is_empty(), "declared entry points become paths");
    let (path, item) = paths.iter().next().expect("at least one path");
    assert!(path.starts_with('/'), "{path}");
    let operation = &item["post"];
    assert!(operation.is_object(), "the default method is POST: {item}");
    assert!(
        operation["requestBody"]["content"]["application/json"]["schema"]["properties"].is_object(),
        "arguments become a request body object"
    );
    assert!(operation["responses"]["200"].is_object());
    assert_eq!(operation["x-morphir-entry-point"], true);
}

#[test]
fn a_library_has_no_declared_entry_points() {
    let document = document(
        v4::v4_customer_library(),
        map([
            ("projection", json!("operations-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    assert_eq!(document["paths"], json!({}));
    assert!(
        document["components"]["schemas"]
            .as_object()
            .is_some_and(|schemas| !schemas.is_empty())
    );
}

#[test]
fn a_specs_distribution_has_no_declared_entry_points() {
    let document = document(
        v4::v4_customer_specs(),
        map([
            ("projection", json!("operations-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    assert_eq!(document["paths"], json!({}));
    assert!(
        document["components"]["schemas"]
            .as_object()
            .is_some_and(|schemas| !schemas.is_empty())
    );
}

#[test]
fn a_constant_entry_point_takes_no_request_body() {
    let document = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    let has_constant = document["paths"]
        .as_object()
        .expect("paths is an object")
        .values()
        .any(|item| item["post"]["x-morphir-value-kind"] == "constant");
    assert!(
        has_constant,
        "the fixture's 'unfinished' handler is a zero-input constant"
    );
    let constant = document["paths"]
        .as_object()
        .unwrap()
        .values()
        .find(|item| item["post"]["x-morphir-value-kind"] == "constant")
        .unwrap();
    assert!(constant["post"]["requestBody"].is_null());
}

// --- Unsupported-form handling for operations, exercised directly on hand-built
// `ProjectionPackage`s the way `tests/projection.rs` builds unsupported *type*
// declarations. Neither v4 nor Classic fixture declares a value specification
// whose own input or output type is directly unsupported (an unbound type
// parameter with no concrete type to substitute) without normalization
// rejecting or reshaping it first, so this constructs the projection model
// directly rather than round-tripping through IR JSON.

const SDK_BOOL: &str = "morphir/SDK:basics#bool";

fn source(local: &str) -> String {
    format!("acme/customer:domain#{local}")
}

fn package_with(values: Vec<ValueSpecification>) -> ProjectionPackage {
    ProjectionPackage {
        kind: DistributionKind::Application,
        package_name: "acme/customer".to_owned(),
        dependencies: Vec::new(),
        modules: vec![ProjectionModule {
            path: vec!["domain".to_owned()],
            types: Vec::new(),
            values,
            doc: None,
        }],
    }
}

fn entry_point(identifier: &str) -> Option<EntryPointMetadata> {
    Some(EntryPointMetadata {
        identifier: identifier.to_owned(),
        kind: EntryPointKind::Command,
        doc: None,
    })
}

/// A declared entry point whose one input is an unbound type parameter: no
/// concrete type to substitute, so it has no schema, the same as `every_unsupported_morphir_form_is_reported_as_jsc003`'s
/// "unbound type parameter" row in `tests/projection.rs`.
fn unsupported_entry_point(local: &str) -> ValueSpecification {
    ValueSpecification {
        source_name: source(local),
        name: local.to_owned(),
        inputs: vec![NamedType {
            name: "value".to_owned(),
            tpe: TypeExpr::Variable("a".to_owned()),
        }],
        output: Some(TypeExpr::Reference {
            source_name: SDK_BOOL.to_owned(),
            arguments: Vec::new(),
        }),
        value_kind: ValueKind::Function,
        entry_point: entry_point(&format!("{local}-id")),
        doc: None,
    }
}

fn supported_entry_point(local: &str) -> ValueSpecification {
    ValueSpecification {
        source_name: source(local),
        name: local.to_owned(),
        inputs: Vec::new(),
        output: Some(TypeExpr::Reference {
            source_name: SDK_BOOL.to_owned(),
            arguments: Vec::new(),
        }),
        value_kind: ValueKind::Constant,
        entry_point: entry_point(&format!("{local}-id")),
        doc: None,
    }
}

#[test]
fn strict_mode_fails_the_whole_generation_on_an_unsupported_operation_signature() {
    let package = package_with(vec![unsupported_entry_point("broken")]);
    let mut projection = project(&package, &SchemaOptions::default()).expect("no types to skip");
    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        ..SchemaOptions::default()
    };

    let error = project_operations(&package, &mut projection, &options)
        .expect_err("an unbound type parameter has no schema");

    assert_eq!(error.code(), "JSC003");
}

#[test]
fn warn_and_skip_omits_the_unsupported_operation_and_keeps_the_rest() {
    let package = package_with(vec![
        unsupported_entry_point("broken"),
        supported_entry_point("fine"),
    ]);
    let mut projection = project(&package, &SchemaOptions::default()).expect("no types to skip");
    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        unsupported: Unsupported::WarnAndSkip,
        ..SchemaOptions::default()
    };

    let operations = project_operations(&package, &mut projection, &options)
        .expect("skipping keeps operation projection successful");

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("fine").as_str()],
        "the unsupported operation is omitted, the supported one still renders"
    );
    assert!(
        projection.diagnostics.iter().any(|(diagnostic, warning)| {
            *warning
                && diagnostic.code() == "JSC003"
                && diagnostic.source() == Some(source("broken").as_str())
        }),
        "the skipped operation is warned about by its own FQName: {:?}",
        projection.diagnostics
    );
}

/// The `extend_definitions` closure operations run after `project` must
/// reuse `project`'s own collision detection: a projected schema name is
/// keyed on the local name only (`schema_name`), so a dependency type an
/// operation reaches that happens to share a local name with an
/// already-registered definition from a different Morphir source is a
/// `JSC004` collision, not a silent alias of one type's `$ref` onto
/// another's schema.
#[test]
fn a_dependency_type_that_collides_with_an_existing_definition_is_jsc004_not_a_silent_alias() {
    let mut package = package_with(vec![ValueSpecification {
        source_name: source("look-up"),
        name: "look-up".to_owned(),
        inputs: Vec::new(),
        output: Some(TypeExpr::Reference {
            source_name: "shared/vault:api#token".to_owned(),
            arguments: Vec::new(),
        }),
        value_kind: ValueKind::Constant,
        entry_point: entry_point("look-up-id"),
        doc: None,
    }]);
    package.modules[0].types = vec![alias(&source("token"), "token", string_ref())];
    package.dependencies = vec![ProjectionDependency {
        package_name: "shared/vault".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["api".to_owned()],
            types: vec![alias("shared/vault:api#token", "token", string_ref())],
            values: Vec::new(),
            doc: None,
        }],
    }];

    let mut projection =
        project(&package, &SchemaOptions::default()).expect("the own 'token' alias projects");
    assert!(
        projection.definitions.contains_key("Token"),
        "the package's own 'token' alias is already a registered root before operations run"
    );

    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        ..SchemaOptions::default()
    };
    let error = project_operations(&package, &mut projection, &options).expect_err(
        "the dependency's 'shared/vault:api#token' collides with the already-registered 'Token'",
    );

    assert_eq!(error.code(), "JSC004");
}

/// A multi-hop case: the unsupported type is reached only through
/// `extend_definitions`'s own closure over an already-registered
/// definition's references, not directly by the operation's own top-level
/// signature (which is a plain reference to `wrapper` and projects fine on
/// its own). `close_definitions` must run its dangling cleanup for this
/// second pass too, or `wrapper` — inserted into `projection.definitions`
/// before its own field turned out to reference a skipped declaration —
/// survives with a `$ref` to a `components/schemas` entry that does not
/// exist.
#[test]
fn warn_and_skip_drops_a_definition_reached_two_hops_from_an_operation() {
    let mut package = package_with(vec![ValueSpecification {
        source_name: source("look-up"),
        name: "look-up".to_owned(),
        inputs: Vec::new(),
        output: Some(TypeExpr::Reference {
            source_name: "shared/vault:support#wrapper".to_owned(),
            arguments: Vec::new(),
        }),
        value_kind: ValueKind::Constant,
        entry_point: entry_point("look-up-id"),
        doc: None,
    }]);
    package.dependencies = vec![ProjectionDependency {
        package_name: "shared/vault".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["support".to_owned()],
            types: vec![
                TypeDeclaration::Alias {
                    source_name: "shared/vault:support#wrapper".to_owned(),
                    name: "wrapper".to_owned(),
                    type_params: Vec::new(),
                    value: TypeExpr::Record(vec![NamedType {
                        name: "payload".to_owned(),
                        tpe: TypeExpr::Reference {
                            source_name: "shared/vault:support#broken".to_owned(),
                            arguments: Vec::new(),
                        },
                    }]),
                    doc: None,
                },
                alias(
                    "shared/vault:support#broken",
                    "broken",
                    TypeExpr::Variable("a".to_owned()),
                ),
            ],
            values: Vec::new(),
            doc: None,
        }],
    }];

    let mut projection = project(&package, &SchemaOptions::default())
        .expect("neither 'wrapper' nor 'broken' is a public type root of this package");
    assert!(
        projection.definitions.is_empty(),
        "'wrapper' and 'broken' are dependency types reached only through the operation below"
    );

    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        unsupported: Unsupported::WarnAndSkip,
        ..SchemaOptions::default()
    };
    let operations = project_operations(&package, &mut projection, &options).expect(
        "the operation's own top-level signature is a plain reference; only its second-hop \
         dependency ('broken') is unsupported",
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("look-up").as_str()]
    );
    assert!(
        !projection.definitions.contains_key("Wrapper"),
        "'Wrapper' refers to the skipped 'Broken' and must not survive with a dangling $ref: {:?}",
        projection.definitions.keys().collect::<Vec<_>>()
    );
    assert!(!projection.definitions.contains_key("Broken"));

    let dropped_wrapper = projection.diagnostics.iter().any(|(diagnostic, warning)| {
        *warning
            && diagnostic.code() == "JSC003"
            && diagnostic.source() == Some("shared/vault:support#wrapper")
    });
    assert!(
        dropped_wrapper,
        "the referring declaration 'wrapper' is warned about by its own FQName: {:?}",
        projection.diagnostics
    );
}

fn alias(source_name: &str, name: &str, value: TypeExpr) -> TypeDeclaration {
    TypeDeclaration::Alias {
        source_name: source_name.to_owned(),
        name: name.to_owned(),
        type_params: Vec::new(),
        value,
        doc: None,
    }
}

fn string_ref() -> TypeExpr {
    TypeExpr::Reference {
        source_name: "morphir/SDK:string#string".to_owned(),
        arguments: Vec::new(),
    }
}
