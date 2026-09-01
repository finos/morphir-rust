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

use std::collections::{BTreeMap, HashMap};

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::{
    OpenApiExtension, OperationOverride, ParameterBinding, Projection, ResultResponses,
    SchemaOptions, Unsupported, project, project_operations, render_openapi,
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

// --- `operations-public`, `Result` splitting, and per-operation overrides.
//
// `v4_customer_application()` declares `find-customer` (targeted by the
// `customer-query` entry point) and, since this plan step, `validate-customer`
// — a public value the plan brief's own fixture description did not have
// reason to mention because it predates this task; both return in module
// `domain`, so their canonical FQNames are `acme/customer:domain#find-customer`
// and `acme/customer:domain#validate-customer`. `validate-customer` returns
// `morphir/SDK:result#result` applied to `String` (the error) and `Customer`
// (the value) — the one value in every v4 fixture whose output is
// `Result`-shaped, added to `morphir-projection`'s testing module rather than
// asserted through a hand-built `ProjectionPackage`, so `ResultResponses`
// splitting is exercised through the same normalize -> project -> render
// pipeline every other test in this file uses.

#[test]
fn public_mode_covers_values_that_are_not_entry_points() {
    let entry_points = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-entry-points")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );
    let public = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-public")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    let entry_point_count = entry_points["paths"].as_object().unwrap().len();
    let public_count = public["paths"].as_object().unwrap().len();
    assert!(
        public_count > entry_point_count,
        "public mode covers more values: {public_count} vs {entry_point_count}"
    );
}

#[test]
fn a_result_stays_data_in_the_200_response_by_default() {
    let document = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-public")),
            ("unsupported", json!("warn-and-skip")),
        ]),
    );

    for item in document["paths"].as_object().unwrap().values() {
        for operation in item.as_object().unwrap().values() {
            let responses = operation["responses"].as_object().unwrap();
            assert_eq!(
                responses.keys().collect::<Vec<_>>(),
                vec!["200"],
                "the default emits only a 200 response: {operation}"
            );
        }
    }
}

#[test]
fn split_mode_moves_the_error_branch_to_the_configured_status() {
    let document = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-public")),
            ("unsupported", json!("warn-and-skip")),
            ("result_responses", json!("split")),
            ("error_status", json!(422)),
        ]),
    );

    let validate_customer = &document["paths"]["/domain/validateCustomer"]["post"];
    assert!(
        validate_customer.is_object(),
        "'validate-customer' keeps its default path: {document}"
    );
    assert_eq!(
        validate_customer["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Customer",
        "the 200 response is the Ok member's own schema, not the whole Result: {validate_customer}"
    );
    let error_response = &validate_customer["responses"]["422"];
    assert!(
        error_response.is_object(),
        "a Result-returning value gains a 422 response: {validate_customer}"
    );
    assert_eq!(
        error_response["content"]["application/json"]["schema"]["type"], "string",
        "the 422 response is the Err member's own schema: {error_response}"
    );

    let has_error_response = document["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|item| item.as_object().unwrap().values())
        .any(|operation| operation["responses"]["422"].is_object());
    assert!(
        has_error_response,
        "a Result-returning value gains a 422 response"
    );
}

#[test]
fn an_override_replaces_the_method_and_the_path() {
    let document = document(
        v4::v4_customer_application(),
        map([
            ("projection", json!("operations-public")),
            ("unsupported", json!("warn-and-skip")),
            (
                "operations",
                json!({
                    "acme/customer:domain#find-customer": {
                        "method": "get",
                        "path": "/customers/{id}",
                        "parameters": {"id": "path"}
                    }
                }),
            ),
        ]),
    );

    let item = &document["paths"]["/customers/{id}"];
    assert!(
        item["get"].is_object(),
        "the override selects GET: {document}"
    );
    assert!(
        item["get"]["requestBody"].is_null(),
        "a path parameter is not a body"
    );
    assert_eq!(item["get"]["x-morphir-value-kind"], "function");
    let parameter = &item["get"]["parameters"][0];
    assert_eq!(parameter["name"], "id");
    assert_eq!(parameter["in"], "path");
    assert_eq!(parameter["required"], true);
}

#[test]
fn a_path_bound_parameter_without_a_placeholder_in_the_override_path_is_an_error() {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir: v4::v4_customer_application(),
            target: "openapi".into(),
            options: map([
                ("projection", json!("operations-public")),
                ("unsupported", json!("warn-and-skip")),
                (
                    "operations",
                    json!({
                        "acme/customer:domain#find-customer": {
                            "path": "/customers",
                            "parameters": {"id": "path"}
                        }
                    }),
                ),
            ]),
        })
        .expect("generation is a successful MEP call");

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("OAS002"));
}

/// Reproduces the exact failure mode Important-finding review flagged: the
/// plan brief's own illustrative override names its path parameter
/// `customerId` against `/customers/{customerId}`, while the real fixture's
/// input is named `id`. Adjusting only the FQName (as the brief instructed)
/// and copying the parameter name and path verbatim lands squarely on a
/// `Path` binding whose name matches no request field — the placeholder is
/// present, so the check above alone would miss this and silently leave a
/// `{customerId}` path template with no matching Parameter Object, a
/// structurally invalid document generated without complaint.
#[test]
fn an_override_path_parameter_naming_no_request_field_is_an_error() {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir: v4::v4_customer_application(),
            target: "openapi".into(),
            options: map([
                ("projection", json!("operations-public")),
                ("unsupported", json!("warn-and-skip")),
                (
                    "operations",
                    json!({
                        "acme/customer:domain#find-customer": {
                            "method": "get",
                            "path": "/customers/{customerId}",
                            "parameters": {"customerId": "path"}
                        }
                    }),
                ),
            ]),
        })
        .expect("generation is a successful MEP call");

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("OAS002"));
}

#[test]
fn an_override_naming_an_unknown_value_is_an_error() {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir: v4::v4_customer_application(),
            target: "openapi".into(),
            options: map([
                ("projection", json!("operations-public")),
                ("unsupported", json!("warn-and-skip")),
                (
                    "operations",
                    json!({"acme/customer:domain#no-such-value": {"method": "get"}}),
                ),
            ]),
        })
        .expect("generation is a successful MEP call");

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("OAS002"));
}

// --- Unsupported-form handling for operations, exercised directly on hand-built
// `ProjectionPackage`s the way `tests/projection.rs` builds unsupported *type*
// declarations. Neither v4 nor Classic fixture declares a value specification
// whose own input or output type is directly unsupported (an unbound type
// parameter with no concrete type to substitute) without normalization
// rejecting or reshaping it first, so this constructs the projection model
// directly rather than round-tripping through IR JSON.

const SDK_BOOL: &str = "morphir/SDK:basics#bool";
const SDK_RESULT: &str = "morphir/SDK:result#result";

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
/// exist. One layer further out, `look-up` itself — whose own top-level
/// signature is a plain, otherwise-fine reference to `wrapper` — must be
/// dropped too, once `wrapper` is gone, by `drop_dangling_operations`; see
/// `a_dangling_reference_from_an_operation_is_dropped_from_the_rendered_document`
/// for the document-level assertion of that.
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
        Vec::<&str>::new(),
        "'look-up' refers to 'wrapper', which the sweep below removes, so it must be dropped too"
    );
    assert!(
        !projection.definitions.contains_key("Wrapper"),
        "'Wrapper' refers to the skipped 'Broken' and must not survive with a dangling $ref: {:?}",
        projection.definitions.keys().collect::<Vec<_>>()
    );
    assert!(!projection.definitions.contains_key("Broken"));

    let warned_about = |source_name: &str| {
        projection.diagnostics.iter().any(|(diagnostic, warning)| {
            *warning && diagnostic.code() == "JSC003" && diagnostic.source() == Some(source_name)
        })
    };
    assert!(
        warned_about("shared/vault:support#wrapper"),
        "the referring declaration 'wrapper' is warned about by its own FQName: {:?}",
        projection.diagnostics
    );
    assert!(
        warned_about(&source("look-up")),
        "the dropped operation is warned about by its own FQName: {:?}",
        projection.diagnostics
    );
}

/// The whole-document invariant round 3 protects: no `$ref` anywhere in a
/// rendered OpenAPI document — inside `paths` or inside
/// `components/schemas` itself — may point at a `components/schemas` entry
/// that does not exist. `close_definitions`'s dangling sweep only ever
/// walked `definitions`; an operation is a second, independent source of
/// references into that same namespace, so `look-up`'s response (a plain
/// reference to `wrapper`, which the sweep removes because its own field
/// refers to the unsupported `broken`) must drop `look-up` out of `paths`
/// too, not leave it behind with a `$ref` to nothing. `fine`, an unrelated
/// operation with no dependency on any of this, proves the rest of the
/// document still renders — the assertion below is not vacuously true from
/// an empty `paths`.
#[test]
fn a_dangling_reference_from_an_operation_is_dropped_from_the_rendered_document() {
    let mut package = package_with(vec![
        ValueSpecification {
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
        },
        supported_entry_point("fine"),
    ]);
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
        .expect("neither dependency type is a public type root of this package");
    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        unsupported: Unsupported::WarnAndSkip,
        ..SchemaOptions::default()
    };
    projection.operations = project_operations(&package, &mut projection, &options)
        .expect("skipping keeps the rest of the document rendering");

    assert_eq!(
        projection
            .operations
            .iter()
            .map(|operation| operation.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("fine").as_str()],
        "'look-up' is dropped for its dangling response; 'fine' is unaffected"
    );

    let artifacts = render_openapi(&projection, &options);
    assert_eq!(artifacts.len(), 1);
    let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");

    let schemas = document["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for reference in collect_refs(&document) {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("non-local reference {reference}"));
        assert!(
            schemas.contains_key(name),
            "dangling $ref to '{name}' with no components/schemas entry anywhere \
             in the document: {document}"
        );
    }

    let paths = document["paths"].as_object().expect("paths is an object");
    assert!(
        !paths.contains_key("/domain/lookUp"),
        "'look-up' must not appear in paths: {paths:?}"
    );
    assert_eq!(paths.len(), 1, "only 'fine' survives: {paths:?}");
    assert!(paths.contains_key("/domain/fine"));
}

/// The same whole-document invariant as
/// `a_dangling_reference_from_an_operation_is_dropped_from_the_rendered_document`,
/// but reached through `operation.parameters` rather than `operation.response`:
/// `operation_references` was extended to walk moved-out parameters
/// alongside the request body, and this is the only test that would fail if
/// that branch were missing or wrong. `look-up-by-token`'s only reference to
/// the dangling `wrapper` type is through its `token` input, which an
/// override moves out of the request body and into a `Query` parameter
/// before the dangling sweep runs; `fine` again proves the rest of the
/// document survives.
#[test]
fn a_dangling_reference_reached_only_through_an_override_parameter_is_dropped() {
    let mut package = package_with(vec![
        ValueSpecification {
            source_name: source("look-up-by-token"),
            name: "look-up-by-token".to_owned(),
            inputs: vec![NamedType {
                name: "token".to_owned(),
                tpe: TypeExpr::Reference {
                    source_name: "shared/vault:support#wrapper".to_owned(),
                    arguments: Vec::new(),
                },
            }],
            output: Some(TypeExpr::Reference {
                source_name: SDK_BOOL.to_owned(),
                arguments: Vec::new(),
            }),
            value_kind: ValueKind::Function,
            entry_point: entry_point("look-up-by-token-id"),
            doc: None,
        },
        supported_entry_point("fine"),
    ]);
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
        .expect("neither dependency type is a public type root of this package");
    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        unsupported: Unsupported::WarnAndSkip,
        operations: BTreeMap::from([(
            source("look-up-by-token"),
            OperationOverride {
                method: None,
                path: None,
                parameters: BTreeMap::from([("token".to_owned(), ParameterBinding::Query)]),
            },
        )]),
        ..SchemaOptions::default()
    };
    projection.operations = project_operations(&package, &mut projection, &options)
        .expect("skipping keeps the rest of the document rendering");

    assert_eq!(
        projection
            .operations
            .iter()
            .map(|operation| operation.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("fine").as_str()],
        "'look-up-by-token' is dropped for its dangling Query parameter; 'fine' is unaffected"
    );

    let artifacts = render_openapi(&projection, &options);
    assert_eq!(artifacts.len(), 1);
    let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");

    let schemas = document["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for reference in collect_refs(&document) {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("non-local reference {reference}"));
        assert!(
            schemas.contains_key(name),
            "dangling $ref to '{name}' with no components/schemas entry anywhere              in the document: {document}"
        );
    }

    let paths = document["paths"].as_object().expect("paths is an object");
    assert!(
        !paths.contains_key("/domain/lookUpByToken"),
        "'look-up-by-token' must not appear in paths: {paths:?}"
    );
    assert_eq!(paths.len(), 1, "only 'fine' survives: {paths:?}");
    assert!(paths.contains_key("/domain/fine"));
}

/// The same whole-document invariant again, this time reached through
/// `operation.error_response` rather than `operation.response`:
/// `operation_references` was also extended to walk the split error
/// response, and this is the only test that would fail if that branch were
/// missing or wrong. `look-up-with-result` returns
/// `Result wrapper Bool`; under `ResultResponses::Split` its `Err` member
/// (`wrapper`, the dangling type) lands only in `error_response`, never in
/// `response`, so only the new branch can catch it.
#[test]
fn a_dangling_reference_reached_only_through_a_split_error_response_is_dropped() {
    let mut package = package_with(vec![
        ValueSpecification {
            source_name: source("look-up-with-result"),
            name: "look-up-with-result".to_owned(),
            inputs: Vec::new(),
            output: Some(TypeExpr::Reference {
                source_name: SDK_RESULT.to_owned(),
                arguments: vec![
                    TypeExpr::Reference {
                        source_name: "shared/vault:support#wrapper".to_owned(),
                        arguments: Vec::new(),
                    },
                    TypeExpr::Reference {
                        source_name: SDK_BOOL.to_owned(),
                        arguments: Vec::new(),
                    },
                ],
            }),
            value_kind: ValueKind::Constant,
            entry_point: entry_point("look-up-with-result-id"),
            doc: None,
        },
        supported_entry_point("fine"),
    ]);
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
        .expect("neither dependency type is a public type root of this package");
    let options = SchemaOptions {
        projection: Projection::OperationsEntryPoints,
        unsupported: Unsupported::WarnAndSkip,
        result_responses: ResultResponses::Split,
        ..SchemaOptions::default()
    };
    projection.operations = project_operations(&package, &mut projection, &options)
        .expect("skipping keeps the rest of the document rendering");

    assert_eq!(
        projection
            .operations
            .iter()
            .map(|operation| operation.source_name.as_str())
            .collect::<Vec<_>>(),
        vec![source("fine").as_str()],
        "'look-up-with-result' is dropped for its dangling split error response;          'fine' is unaffected"
    );

    let artifacts = render_openapi(&projection, &options);
    assert_eq!(artifacts.len(), 1);
    let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");

    let schemas = document["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for reference in collect_refs(&document) {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("non-local reference {reference}"));
        assert!(
            schemas.contains_key(name),
            "dangling $ref to '{name}' with no components/schemas entry anywhere              in the document: {document}"
        );
    }

    let paths = document["paths"].as_object().expect("paths is an object");
    assert!(
        !paths.contains_key("/domain/lookUpWithResult"),
        "'look-up-with-result' must not appear in paths: {paths:?}"
    );
    assert_eq!(paths.len(), 1, "only 'fine' survives: {paths:?}");
    assert!(paths.contains_key("/domain/fine"));
}

/// Every `$ref` target anywhere in a JSON value, walked recursively so both
/// `paths` and `components/schemas` are covered by one call.
fn collect_refs(value: &Value) -> Vec<String> {
    match value {
        Value::Object(members) => members
            .iter()
            .flat_map(|(key, member)| {
                if key == "$ref" {
                    member.as_str().map(str::to_owned).into_iter().collect()
                } else {
                    collect_refs(member)
                }
            })
            .collect(),
        Value::Array(members) => members.iter().flat_map(collect_refs).collect(),
        _ => Vec::new(),
    }
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
