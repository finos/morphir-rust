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

// ---------------------------------------------------------------------------
// OpenAPI metaschema validation
// ---------------------------------------------------------------------------
//
// Every rendered OpenAPI document is checked against the published OpenAPI
// metaschema for the version it claims, so a golden is validated by a real
// parser rather than only against its own previous bytes. The metaschemas are
// vendored under `tests/metaschema/` because the test suite must run offline;
// each file keeps its published `$id`/`id`, which is the URI it was retrieved
// from on 2026-08-31:
//
// - `oas-3.1-schema-base-2022-10-07.json`
//   https://spec.openapis.org/oas/3.1/schema-base/2022-10-07
// - `oas-3.1-schema-2022-10-07.json`
//   https://spec.openapis.org/oas/3.1/schema/2022-10-07
// - `oas-3.1-dialect-base.json`
//   https://spec.openapis.org/oas/3.1/dialect/base
// - `oas-3.1-meta-base.json`
//   https://spec.openapis.org/oas/3.1/meta/base
// - `oas-3.0-schema-2021-09-28.json`
//   https://spec.openapis.org/oas/3.0/schema/2021-09-28
//
// The 3.1 entry point is `schema-base`, not the bare `schema`: `schema-base`
// binds the `meta` dynamic anchor to the OAS 3.1 dialect, so every Schema
// Object in the document is validated as a JSON Schema too. The bare `schema`
// leaves Schema Objects unconstrained, which would let a malformed
// `components/schemas` entry through.
//
// One byte-level edit was made to `oas-3.1-schema-base-2022-10-07.json`: its
// two `"$ref": "#/$defs/dialect"` occurrences were rewritten to the
// equivalent absolute form
// `"https://spec.openapis.org/oas/3.1/schema-base/2022-10-07#/$defs/dialect"`.
// Neither occurrence sits under an intervening `$id`, so the published
// relative form resolves against exactly that URI and the two are the same
// reference; `jsonschema` 0.26 loses the base URI once resolution crosses
// into the dialect resource and reports `PointerToNowhere` for the relative
// form. `the_vendored_metaschemas_reject_an_invalid_document` keeps the
// rewrite honest by proving both validators still reject bad input.

fn metaschema(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/metaschema")
        .join(name);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("vendored metaschema {name} is readable: {error}"));
    serde_json::from_str(&contents)
        .unwrap_or_else(|error| panic!("vendored metaschema {name} is valid JSON: {error}"))
}

fn resource(name: &str) -> jsonschema::Resource {
    jsonschema::Resource::from_contents(metaschema(name))
        .unwrap_or_else(|error| panic!("vendored metaschema {name} is a schema resource: {error}"))
}

fn openapi_31_validator() -> jsonschema::Validator {
    jsonschema::options()
        .with_resources(
            [
                (
                    "https://spec.openapis.org/oas/3.1/schema/2022-10-07",
                    resource("oas-3.1-schema-2022-10-07.json"),
                ),
                (
                    "https://spec.openapis.org/oas/3.1/dialect/base",
                    resource("oas-3.1-dialect-base.json"),
                ),
                (
                    "https://spec.openapis.org/oas/3.1/meta/base",
                    resource("oas-3.1-meta-base.json"),
                ),
            ]
            .into_iter(),
        )
        .build(&metaschema("oas-3.1-schema-base-2022-10-07.json"))
        .expect("the vendored OpenAPI 3.1 metaschema compiles")
}

fn openapi_30_validator() -> jsonschema::Validator {
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .build(&metaschema("oas-3.0-schema-2021-09-28.json"))
        .expect("the vendored OpenAPI 3.0 metaschema compiles")
}

fn assert_valid(validator: &jsonschema::Validator, label: &str, document: &Value) {
    let errors: Vec<String> = validator
        .iter_errors(document)
        .map(|error| format!("  at {}: {error}", error.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "{label} does not satisfy the OpenAPI metaschema:\n{}",
        errors.join("\n")
    );
}

/// Every option combination the suite renders, as `(label, options)`. Both
/// metaschema tests walk the same matrix so a mode that is valid under 3.1
/// but invalid once downgraded — or the reverse — cannot hide in a gap
/// between the two.
fn rendered_option_matrix() -> Vec<(&'static str, Value, HashMap<String, Value>)> {
    let warn = || ("unsupported".to_owned(), json!("warn-and-skip"));
    vec![
        (
            "schemas mode",
            classic::classic_schema_library(),
            HashMap::new(),
        ),
        (
            "operations-entry-points mode",
            v4::v4_customer_application(),
            [
                ("projection".to_owned(), json!("operations-entry-points")),
                warn(),
            ]
            .into_iter()
            .collect(),
        ),
        (
            "operations-public mode",
            v4::v4_customer_application(),
            [
                ("projection".to_owned(), json!("operations-public")),
                warn(),
            ]
            .into_iter()
            .collect(),
        ),
        (
            "operations-public mode with split result responses",
            v4::v4_customer_application(),
            [
                ("projection".to_owned(), json!("operations-public")),
                warn(),
                ("result_responses".to_owned(), json!("split")),
                ("error_status".to_owned(), json!(422)),
            ]
            .into_iter()
            .collect(),
        ),
        (
            "operations-public mode with path, query and header overrides",
            v4::v4_customer_application(),
            [
                ("projection".to_owned(), json!("operations-public")),
                warn(),
                (
                    "operations".to_owned(),
                    json!({
                        "acme/customer:domain#find-customer": {
                            "method": "get",
                            "path": "/customers/{id}",
                            "parameters": {"id": "path"}
                        }
                    }),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    ]
}

#[test]
fn every_openapi_3_1_document_satisfies_the_openapi_metaschema() {
    let validator = openapi_31_validator();
    for (label, ir, options) in rendered_option_matrix() {
        let result = generate_openapi(ir, options);
        assert!(result.success, "{label}: {:?}", result.diagnostics);
        for artifact in &result.artifacts {
            let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
            assert_eq!(document["openapi"], "3.1.0", "{label}");
            assert_valid(&validator, label, &document);
        }
    }
}

#[test]
fn every_openapi_3_0_document_satisfies_the_openapi_metaschema() {
    let validator = openapi_30_validator();
    for (label, ir, mut options) in rendered_option_matrix() {
        options.insert("version".to_owned(), json!("3.0"));
        let result = generate_openapi(ir, options);
        assert!(result.success, "{label}: {:?}", result.diagnostics);
        for artifact in &result.artifacts {
            let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
            assert_eq!(document["openapi"], "3.0.3", "{label}");
            assert_valid(&validator, label, &document);
        }
    }
}

/// The vendored metaschemas are only worth having if they actually reject
/// something: a mis-registered resource, a `$ref` that silently resolves to
/// `true`, or a validator built over the wrong draft would all leave the two
/// tests above passing on any input at all. Both defects Finding 1 produces —
/// a duplicated `required` entry, which JSON Schema 2020-12 and the OAS 3.0
/// metaschema both forbid — are pinned here as the canary.
#[test]
fn the_vendored_metaschemas_reject_an_invalid_document() {
    let duplicated_required = |version: &str, discriminator: Value| {
        json!({
            "openapi": version,
            "info": {"title": "canary", "version": "0"},
            "paths": {},
            "components": {
                "schemas": {
                    "Shape": {
                        "type": "object",
                        "properties": {"kind": discriminator},
                        "required": ["kind", "radius", "kind"]
                    }
                }
            }
        })
    };

    assert!(
        openapi_31_validator()
            .iter_errors(&duplicated_required("3.1.0", json!({"const": "Circle"})))
            .next()
            .is_some(),
        "the 3.1 metaschema must reject a duplicated `required` entry"
    );
    assert!(
        openapi_30_validator()
            .iter_errors(&duplicated_required("3.0.3", json!({"enum": ["Circle"]})))
            .next()
            .is_some(),
        "the 3.0 metaschema must reject a duplicated `required` entry"
    );
}
