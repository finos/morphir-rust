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

use std::collections::HashMap;

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::OpenApiExtension;
use morphir_projection::testing::classic;
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

#[test]
fn declares_the_3_0_version() {
    let document = document(map([("version", json!("3.0"))]));

    assert_eq!(document["openapi"], "3.0.3");
}

#[test]
fn replaces_null_unions_with_the_nullable_keyword() {
    let document = document(map([("version", json!("3.0"))]));

    let mut offending = Vec::new();
    walk(&document, &mut |members| {
        if let Some(Value::Array(types)) = members.get("type") {
            offending.push(types.clone());
        }
        if members.contains_key("prefixItems") || members.contains_key("$defs") {
            offending.push(vec![json!("unsupported 2020-12 keyword")]);
        }
    });

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

/// Covers the downgrade over a document that has `paths`, not only
/// `components/schemas`: `v4_customer_application()`'s operations carry a
/// request body (`find-customer`), a non-body parameter (the `id` override
/// below), and a split `Result` response (`validate-customer`), so every
/// site the rewrite must reach outside `components/schemas` is exercised
/// here, on top of `components/schemas` itself which every operation's
/// request and response still refer back into.
#[test]
fn downgrades_operations_request_bodies_parameters_and_split_responses() {
    let result = OpenApiExtension
        .generate(GenerateRequest {
            ir: morphir_projection::testing::v4::v4_customer_application(),
            target: "openapi".into(),
            options: map([
                ("version", json!("3.0")),
                ("projection", json!("operations-public")),
                ("unsupported", json!("warn-and-skip")),
                ("result_responses", json!("split")),
                ("error_status", json!(422)),
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
            ])
            .into_iter()
            .collect(),
        })
        .expect("generation is a successful MEP call");
    assert!(result.success, "{:?}", result.diagnostics);
    let document: Value = serde_json::from_str(&result.artifacts[0].content).expect("valid JSON");

    assert_eq!(document["openapi"], "3.0.3");

    let find_customer = &document["paths"]["/customers/{id}"]["get"];
    assert_eq!(find_customer["parameters"][0]["name"], "id");
    assert_eq!(find_customer["parameters"][0]["in"], "path");

    let validate_customer = &document["paths"]["/domain/validateCustomer"]["post"];
    assert!(validate_customer["requestBody"].is_object());
    assert!(validate_customer["responses"]["200"].is_object());
    assert!(validate_customer["responses"]["422"].is_object());

    let mut offending = Vec::new();
    walk(&document, &mut |members| {
        if let Some(Value::Array(types)) = members.get("type") {
            offending.push(types.clone());
        }
        if members.contains_key("prefixItems")
            || members.contains_key("$defs")
            || members.contains_key("const")
        {
            offending.push(vec![json!("unsupported 2020-12 keyword")]);
        }
    });
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
