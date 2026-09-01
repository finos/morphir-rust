use std::collections::HashMap;
use std::path::PathBuf;

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::OpenApiExtension;
use morphir_projection::testing::{classic, v4};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn golden(name: &str, actual: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).expect("golden file is writable");
    }
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden file {name}"))
}

fn generate(ir: Value) -> morphir_extension_sdk::GenerateResult {
    OpenApiExtension
        .generate(GenerateRequest {
            ir,
            target: "json-schema".into(),
            options: HashMap::new(),
        })
        .expect("generation is a successful MEP call")
}

#[test]
fn renders_one_document_per_public_root_type() {
    let result = generate(classic::classic_schema_library());

    assert!(result.success, "{:?}", result.diagnostics);
    assert!(!result.artifacts.is_empty());
    for artifact in &result.artifacts {
        assert!(artifact.path.ends_with(".schema.json"), "{}", artifact.path);
        assert!(!artifact.binary);
        assert!(artifact.content.ends_with('\n'));
        assert!(!artifact.content.ends_with("\n\n"));
    }
}

/// Every root the fixture declares is pinned byte-exactly, not just the
/// customer root: a golden that only pins one of four documents leaves the
/// other three checked structurally only, so a wrong `format`, a dropped
/// `uniqueItems`, or a swapped `const` discriminator would still pass.
#[test]
fn matches_the_reviewed_golden_documents() {
    let result = generate(classic::classic_schema_library());

    let expected_paths = [
        "customer.Customer.schema.json",
        "customer.Metrics.schema.json",
        "customer.Shape.schema.json",
        "customer.Status.schema.json",
    ];
    assert_eq!(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        expected_paths.into_iter().collect(),
        "the fixture's public roots changed; update expected_paths and the golden files together"
    );

    for path in expected_paths {
        let artifact = result
            .artifacts
            .iter()
            .find(|artifact| artifact.path == path)
            .unwrap_or_else(|| panic!("{path} is generated"));

        assert_eq!(artifact.content, golden(path, &artifact.content), "{path}");
    }
}

#[test]
fn every_document_is_a_valid_2020_12_schema() {
    let result = generate(classic::classic_schema_library());

    for artifact in &result.artifacts {
        let document: Value = serde_json::from_str(&artifact.content)
            .unwrap_or_else(|error| panic!("{}: {error}", artifact.path));
        assert_eq!(
            document["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        jsonschema::validator_for(&document)
            .unwrap_or_else(|error| panic!("{} is not a valid schema: {error}", artifact.path));
    }
}

#[test]
fn local_references_resolve_inside_the_document() {
    let result = generate(classic::classic_schema_library());

    for artifact in &result.artifacts {
        let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
        let definitions = document
            .get("$defs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for reference in references(&document) {
            let name = reference
                .strip_prefix("#/$defs/")
                .unwrap_or_else(|| panic!("{}: non-local reference {reference}", artifact.path));
            assert!(
                definitions.contains_key(name),
                "{}: dangling reference {reference}",
                artifact.path
            );
        }
    }
}

fn generate_openapi(
    ir: Value,
    options: HashMap<String, Value>,
) -> morphir_extension_sdk::GenerateResult {
    OpenApiExtension
        .generate(GenerateRequest {
            ir,
            target: "openapi".into(),
            options,
        })
        .expect("generation is a successful MEP call")
}

#[test]
fn renders_one_openapi_document_per_package() {
    let result = generate_openapi(classic::classic_schema_library(), HashMap::new());

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    let artifact = &result.artifacts[0];
    assert_eq!(artifact.path, "openapi.json");
    assert!(artifact.content.ends_with('\n'));
    assert!(!artifact.content.ends_with("\n\n"));

    let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
    assert_eq!(document["openapi"], "3.1.0");
    assert!(document["components"]["schemas"].is_object());
    assert!(document["paths"].is_object());
    assert_eq!(
        artifact.content,
        golden("customer.openapi-3.1.json", &artifact.content)
    );
}

/// Pins the OpenAPI 3.0 downgrade of the same `schemas`-mode document
/// `renders_one_openapi_document_per_package` pins as 3.1: same fixture,
/// same projection, `version: "3.0"` instead of the default. A wrong
/// `nullable` placement, a missed `prefixItems` rewrite, or a dropped
/// `allOf` wrap around a `$ref` with siblings would still pass a purely
/// structural check.
#[test]
fn matches_the_reviewed_3_0_golden_document() {
    let result = generate_openapi(
        classic::classic_schema_library(),
        [("version".to_owned(), json!("3.0"))].into_iter().collect(),
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    let artifact = &result.artifacts[0];
    assert_eq!(artifact.path, "openapi.json");
    assert!(artifact.content.ends_with('\n'));
    assert!(!artifact.content.ends_with("\n\n"));

    let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
    assert_eq!(document["openapi"], "3.0.3");
    assert!(document["components"]["schemas"].is_object());
    assert!(document["paths"].is_object());
    assert_eq!(
        artifact.content,
        golden("customer.openapi-3.0.json", &artifact.content)
    );
}

/// Pins the `operations-entry-points` mode document byte-exactly, the same
/// way `matches_the_reviewed_golden_documents` pins the `schemas`-mode
/// documents: a wrong default path, a missing `x-morphir-entry-point-kind`,
/// or a dropped `requestBody` would still pass a purely structural check.
///
/// `v4_customer_application()` also declares `acme/customer:domain#complex`,
/// a generic alias with an unbound type parameter unrelated to operations;
/// `unsupported: "warn-and-skip"` lets the rest of the package project while
/// that one declaration is skipped with a warning.
#[test]
fn matches_the_reviewed_entry_points_golden_document() {
    let result = generate_openapi(
        v4::v4_customer_application(),
        [
            ("projection".to_owned(), json!("operations-entry-points")),
            ("unsupported".to_owned(), json!("warn-and-skip")),
        ]
        .into_iter()
        .collect(),
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    let artifact = &result.artifacts[0];
    assert_eq!(artifact.path, "openapi.json");

    let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
    let paths = document["paths"].as_object().expect("paths is an object");
    assert_eq!(
        paths.len(),
        2,
        "the fixture declares two entry points: 'customer-query' and 'unfinished'"
    );

    assert_eq!(
        artifact.content,
        golden("customer.openapi-3.1-entry-points.json", &artifact.content)
    );
}

fn references(value: &Value) -> Vec<String> {
    match value {
        Value::Object(members) => members
            .iter()
            .flat_map(|(key, member)| {
                if key == "$ref" {
                    member.as_str().map(str::to_owned).into_iter().collect()
                } else {
                    references(member)
                }
            })
            .collect(),
        Value::Array(members) => members.iter().flat_map(references).collect(),
        _ => Vec::new(),
    }
}
