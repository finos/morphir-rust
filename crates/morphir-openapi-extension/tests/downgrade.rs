//! Behavior of the OpenAPI 3.0 downgrade pass, applied through the full
//! `openapi` target.
//!
//! The plan brief's own illustrative tests name
//! `mothers::classic::customer_library()`; the real fixture is
//! `morphir_projection::testing::classic::classic_schema_library()`, which
//! is what `tests/golden.rs` already builds its reviewed goldens from. It
//! covers the widest set of shapes the downgrade rewrites reach: an
//! optional (`Maybe`) field that renders as a 2020-12 null union, a tuple
//! (`metrics.extent`) that renders with `prefixItems`, and a discriminated
//! union (`shape`) whose variants carry a `const` discriminator — exactly
//! the forms `render::downgrade` rewrites for OpenAPI 3.0.

use std::collections::{BTreeMap, HashMap};

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::{
    HttpMethod, OpenApiExtension, OpenApiVersion, OperationOverride, ParameterBinding, Projection,
    ResultResponses, SchemaOptions, project, project_operations, render_openapi,
};
use morphir_projection::testing::classic;
use morphir_projection::{
    DistributionKind, NamedType, ProjectionModule, ProjectionPackage, TypeExpr, ValueKind,
    ValueSpecification,
};
use serde_json::{Value, json};

fn document(options: HashMap<String, Value>) -> Value {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir: classic::classic_schema_library(),
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

fn walk(value: &Value, visit: &mut impl FnMut(&serde_json::Map<String, Value>)) {
    match value {
        Value::Object(members) => {
            visit(members);
            members.values().for_each(|member| walk(member, visit));
        }
        Value::Array(members) => members.iter().for_each(|member| walk(member, visit)),
        _ => {}
    }
}

/// Every 2020-12-only form a 3.0 document must never contain, anywhere:
/// `type` as an array, the scalar `type: "null"` (3.0 has no `null` type at
/// all — only `nullable` alongside another type), `prefixItems`, and
/// `$defs`.
fn offending_2020_12_forms(document: &Value) -> Vec<Value> {
    let mut offending = Vec::new();
    walk(document, &mut |members| {
        match members.get("type") {
            Some(Value::Array(types)) => offending.push(json!(types)),
            Some(Value::String(scalar)) if scalar == "null" => {
                offending.push(json!("type: \"null\""));
            }
            _ => {}
        }
        if members.contains_key("prefixItems") || members.contains_key("$defs") {
            offending.push(json!("unsupported 2020-12 keyword"));
        }
    });
    offending
}

#[test]
fn declares_the_3_0_version() {
    let document = document(map([("version", json!("3.0"))]));

    assert_eq!(document["openapi"], "3.0.3");
}

#[test]
fn replaces_null_unions_with_the_nullable_keyword() {
    let document = document(map([("version", json!("3.0"))]));

    let offending = offending_2020_12_forms(&document);
    assert!(
        offending.is_empty(),
        "3.0 forbids these forms: {offending:?}"
    );

    let mut nullable_seen = false;
    walk(&document, &mut |members| {
        if members.get("nullable") == Some(&json!(true)) {
            nullable_seen = true;
        }
    });
    assert!(nullable_seen, "an optional field becomes nullable in 3.0");
}

#[test]
fn keeps_the_3_1_document_unchanged_by_default() {
    let document = document(HashMap::new());

    assert_eq!(document["openapi"], "3.1.0");
    let mut nullable_seen = false;
    walk(&document, &mut |members| {
        if members.contains_key("nullable") {
            nullable_seen = true;
        }
    });
    assert!(
        !nullable_seen,
        "3.1 uses type unions, not the nullable keyword"
    );
}

/// `const` only appears inside the `shape` discriminated union's variants
/// (the `kind` discriminator property). Confirms it survives as a
/// single-value `enum` rather than being dropped, and that no `const`
/// keyword — 2020-12-only, unsupported by 3.0's Draft 4-based dialect —
/// remains anywhere in the document.
#[test]
fn replaces_const_discriminators_with_a_single_value_enum() {
    let document = document(map([("version", json!("3.0"))]));

    let shape = &document["components"]["schemas"]["Shape"];
    let variants = shape["oneOf"].as_array().expect("Shape is a oneOf");
    assert!(!variants.is_empty());
    for variant in variants {
        let discriminator = &variant["properties"]["kind"];
        assert!(
            discriminator["enum"].is_array(),
            "the discriminator becomes a single-value enum: {discriminator}"
        );
        assert!(discriminator.get("const").is_none());
    }

    let mut const_seen = false;
    walk(&document, &mut |members| {
        if members.contains_key("const") {
            const_seen = true;
        }
    });
    assert!(!const_seen, "3.0 forbids the const keyword");
}

/// `Metrics.nothing` (`Schema::Null`, Morphir's `()`) is a required field
/// whose 3.1 shape is `{"type": "null"}` — not part of any union, so it is
/// not covered by `replaces_null_unions_with_the_nullable_keyword`. Confirms
/// it downgrades to `nullable` plus a single-value `enum` of `null`, per
/// OAS 3.0.3 §4.4's "no null type" rule, rather than surviving as the
/// unsupported scalar `type: "null"`.
#[test]
fn replaces_a_bare_null_schema_with_a_nullable_single_value_enum() {
    let document = document(map([("version", json!("3.0"))]));

    let nothing = &document["components"]["schemas"]["Metrics"]["properties"]["nothing"];
    assert_eq!(nothing, &json!({"nullable": true, "enum": [null]}));
}

/// Every `$ref` in the 3.0 document still resolves inside
/// `components/schemas`, whether bare or wrapped in `allOf` by the
/// `$ref`-with-siblings rewrite: the rewrite changes a reference's shape,
/// never what it points to or whether it resolves.
#[test]
fn every_reference_still_resolves_inside_components_schemas() {
    let document = document(map([("version", json!("3.0"))]));

    let schemas = document["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut references = Vec::new();
    walk(&document, &mut |members| {
        if let Some(Value::String(reference)) = members.get("$ref") {
            references.push(reference.clone());
        }
    });
    assert!(
        !references.is_empty(),
        "the fixture's Customer references Status"
    );
    for reference in references {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("non-local reference {reference}"));
        assert!(schemas.contains_key(name), "dangling reference {reference}");
    }
}

// --- Coverage over a document that has `paths`, not only
// `components/schemas`.
//
// Neither `v4_customer_application()` nor `classic_schema_library()`
// declares a value specification whose own input or output carries a
// `Maybe` or a tuple: every v4 value is plain strings, `$ref`s, and a
// nullary enum, so a downgrade test built on it would pass even if the
// rewrite pass were scoped to `components/schemas` alone and never touched
// `paths` at all. This constructs a `ProjectionPackage` directly — the same
// way `tests/operations.rs`'s "Unsupported-form handling" section builds
// one for a case neither fixture's IR can express — with a value whose
// input carries a `Maybe` (nullable) and whose successful `Result` member
// is a tuple (`prefixItems`), so the rewrite has something real to do
// inside `requestBody`, `parameters`, and a split response.

const SDK_STRING: &str = "morphir/SDK:string#string";
const SDK_INT: &str = "morphir/SDK:basics#int";
const SDK_MAYBE: &str = "morphir/SDK:maybe#maybe";
const SDK_RESULT: &str = "morphir/SDK:result#result";

fn string_ref() -> TypeExpr {
    TypeExpr::Reference {
        source_name: SDK_STRING.to_owned(),
        arguments: Vec::new(),
    }
}

fn int_ref() -> TypeExpr {
    TypeExpr::Reference {
        source_name: SDK_INT.to_owned(),
        arguments: Vec::new(),
    }
}

fn maybe_ref(inner: TypeExpr) -> TypeExpr {
    TypeExpr::Reference {
        source_name: SDK_MAYBE.to_owned(),
        arguments: vec![inner],
    }
}

fn result_ref(error: TypeExpr, value: TypeExpr) -> TypeExpr {
    TypeExpr::Reference {
        source_name: SDK_RESULT.to_owned(),
        arguments: vec![error, value],
    }
}

/// A package with one value, `annotate`: inputs `id: String` and
/// `nickname: Maybe String`, output `Result String (Int, Int)`. Under
/// `ResultResponses::Split` its `Ok` member (the `(Int, Int)` tuple) becomes
/// the `200` response and its `Err` member (`String`) the error response;
/// an override binds `id` to a `Path` parameter, leaving `nickname` — the
/// `Maybe` — as the one remaining `requestBody` property.
fn package_with_a_maybe_and_a_tuple() -> ProjectionPackage {
    ProjectionPackage {
        kind: DistributionKind::Application,
        package_name: "acme/customer".to_owned(),
        dependencies: Vec::new(),
        modules: vec![ProjectionModule {
            path: vec!["domain".to_owned()],
            types: Vec::new(),
            values: vec![ValueSpecification {
                source_name: "acme/customer:domain#annotate".to_owned(),
                name: "annotate".to_owned(),
                inputs: vec![
                    NamedType {
                        name: "id".to_owned(),
                        tpe: string_ref(),
                    },
                    NamedType {
                        name: "nickname".to_owned(),
                        tpe: maybe_ref(string_ref()),
                    },
                ],
                output: Some(result_ref(
                    string_ref(),
                    TypeExpr::Tuple(vec![int_ref(), int_ref()]),
                )),
                value_kind: ValueKind::Function,
                entry_point: None,
                doc: None,
            }],
            doc: None,
        }],
    }
}

fn downgrade_options() -> SchemaOptions {
    let mut operations = BTreeMap::new();
    operations.insert(
        "acme/customer:domain#annotate".to_owned(),
        OperationOverride {
            method: Some(HttpMethod::Get),
            path: Some("/customers/{id}".to_owned()),
            parameters: [("id".to_owned(), ParameterBinding::Path)]
                .into_iter()
                .collect(),
        },
    );
    SchemaOptions {
        version: OpenApiVersion::V30,
        projection: Projection::OperationsPublic,
        result_responses: ResultResponses::Split,
        error_status: 422,
        operations,
        ..SchemaOptions::default()
    }
}

#[test]
fn downgrades_operations_request_bodies_parameters_and_split_responses() {
    let package = package_with_a_maybe_and_a_tuple();
    let options = downgrade_options();

    let mut projection = project(&package, &options).expect("no types to skip");
    projection.operations = project_operations(&package, &mut projection, &options)
        .expect("no unsupported operation forms");
    let artifacts = render_openapi(&projection, &options).expect("no unsupported forms");
    let document: Value = serde_json::from_str(&artifacts[0].content).expect("valid JSON");

    assert_eq!(document["openapi"], "3.0.3");

    let annotate = &document["paths"]["/customers/{id}"]["get"];
    assert!(annotate.is_object(), "the override selects GET: {document}");
    assert_eq!(annotate["parameters"][0]["name"], "id");
    assert_eq!(annotate["parameters"][0]["in"], "path");

    // The `Maybe`-typed `nickname` is the one field left in the request
    // body once `id` moved to a `Path` parameter: it must come out
    // `nullable`, not as a 2020-12 `anyOf`-with-null union.
    let nickname =
        &annotate["requestBody"]["content"]["application/json"]["schema"]["properties"]["nickname"];
    assert_eq!(nickname, &json!({"type": "string", "nullable": true}));

    // The `(Int, Int)` tuple is the `Ok` member of the split `Result`
    // response: it must come out as a bounded `anyOf` array, not
    // `prefixItems`.
    let ok_schema = &annotate["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(
        ok_schema,
        &json!({
            "type": "array",
            "items": {"anyOf": [
                {"type": "integer", "format": "int64"},
                {"type": "integer", "format": "int64"}
            ]},
            "minItems": 2,
            "maxItems": 2
        })
    );
    assert!(annotate["responses"]["422"].is_object());

    let offending = offending_2020_12_forms(&document);
    assert!(
        offending.is_empty(),
        "3.0 forbids these forms: {offending:?}"
    );

    let schemas = document["components"]["schemas"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut references = Vec::new();
    walk(&document, &mut |members| {
        if let Some(Value::String(reference)) = members.get("$ref") {
            references.push(reference.clone());
        }
    });
    for reference in references {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("non-local reference {reference}"));
        assert!(schemas.contains_key(name), "dangling reference {reference}");
    }
}
