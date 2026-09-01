//! Behavior of `paths` synthesis from Morphir entry points and public values.
//!
//! The plan's brief names `mothers::v4::customer_application()` and
//! `mothers::classic::customer_library()`; the real fixtures are
//! `morphir_projection::testing::v4::v4_customer_application` and
//! `morphir_projection::testing::classic::classic_customer_library`. Both
//! declare exactly the shape each test below needs, verified by reading
//! `crates/morphir-projection/src/testing/{v4,classic}.rs`:
//! `v4_customer_application()` declares two entry points on
//! `acme/customer:domain` — `customer-query` (command, targeting
//! `find-customer`, a function of one input) and `unfinished` (handler,
//! targeting a zero-input constant) — and `classic_customer_library()` is a
//! Classic Library, whose format has no entry-point concept at all, so every
//! one of its values normalizes with `entry_point: None`.
//!
//! Both v4 fixtures also declare `acme/customer:domain#complex`, a generic
//! alias whose body uses its own unbound type parameter — unrelated to
//! operations, but a `JSC003` under the default `Unsupported::Error`. Every
//! test below sets `unsupported: "warn-and-skip"` so that declaration is
//! skipped with a warning rather than failing the whole generation.

use std::collections::HashMap;

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::OpenApiExtension;
use morphir_projection::testing::{classic, v4};
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
        classic::classic_customer_library(),
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
