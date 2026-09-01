use std::collections::HashMap;

use morphir_openapi_extension::{SchemaOptions, Unsupported};
use serde_json::{Value, json};

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[test]
fn defaults_to_strict_unsupported_handling() {
    let options = SchemaOptions::from_map(&HashMap::new()).expect("an empty map decodes");

    assert_eq!(options.unsupported, Unsupported::Error);
    assert_eq!(options, SchemaOptions::default());
}

#[test]
fn decodes_the_documented_enum_spelling() {
    let options = SchemaOptions::from_map(&map([("unsupported", json!("warn-and-skip"))]))
        .expect("the documented spelling decodes");

    assert_eq!(options.unsupported, Unsupported::WarnAndSkip);
}

#[test]
fn rejects_an_unknown_option_key() {
    let error = SchemaOptions::from_map(&map([("representation", json!("idl"))]))
        .expect_err("an unknown key is an invalid option");

    assert_eq!(error.code(), "JSC002");
}

#[test]
fn rejects_a_wrong_json_type() {
    let error = SchemaOptions::from_map(&map([("unsupported", json!(true))]))
        .expect_err("a boolean is not an unsupported policy");

    assert_eq!(error.code(), "JSC002");
}

#[test]
fn rejects_an_invalid_enum_value() {
    let error = SchemaOptions::from_map(&map([("unsupported", json!("ignore"))]))
        .expect_err("only the documented values decode");

    assert_eq!(error.code(), "JSC002");
}

use morphir_openapi_extension::{
    HttpMethod, OpenApiVersion, ParameterBinding, Projection, ResultResponses,
};

#[test]
fn defaults_to_openapi_3_1_schemas_and_data_results() {
    let options = SchemaOptions::default();

    assert_eq!(options.version, OpenApiVersion::V31);
    assert_eq!(options.projection, Projection::Schemas);
    assert_eq!(options.result_responses, ResultResponses::Data);
    assert_eq!(options.error_status, 400);
    assert!(options.operations.is_empty());
}

#[test]
fn decodes_the_documented_option_spellings() {
    let options = SchemaOptions::from_map(&map([
        ("version", json!("3.0")),
        ("projection", json!("operations-entry-points")),
        ("result_responses", json!("split")),
        ("error_status", json!(422)),
    ]))
    .expect("the documented spellings decode");

    assert_eq!(options.version, OpenApiVersion::V30);
    assert_eq!(options.projection, Projection::OperationsEntryPoints);
    assert_eq!(options.result_responses, ResultResponses::Split);
    assert_eq!(options.error_status, 422);
}

#[test]
fn decodes_a_per_operation_override() {
    let options = SchemaOptions::from_map(&map([(
        "operations",
        json!({
            "acme/customer:customer#find-customer": {
                "method": "get",
                "path": "/customers/{customerId}",
                "parameters": {"customerId": "path"}
            }
        }),
    )]))
    .expect("an override table decodes");

    let override_entry = options
        .operations
        .get("acme/customer:customer#find-customer")
        .expect("the override is keyed by canonical FQName");
    assert_eq!(override_entry.method, Some(HttpMethod::Get));
    assert_eq!(
        override_entry.path.as_deref(),
        Some("/customers/{customerId}")
    );
    assert_eq!(
        override_entry.parameters.get("customerId"),
        Some(&ParameterBinding::Path)
    );
}

#[test]
fn rejects_an_error_status_outside_the_error_range() {
    let error = SchemaOptions::from_map(&map([("error_status", json!(200))]))
        .expect_err("200 is not an error status");

    assert_eq!(error.code(), "JSC002");
}

#[test]
fn rejects_an_unknown_openapi_version() {
    let error = SchemaOptions::from_map(&map([("version", json!("2.0"))]))
        .expect_err("only 3.1 and 3.0 decode");

    assert_eq!(error.code(), "JSC002");
}

#[test]
fn rejects_an_override_path_without_a_leading_slash() {
    let error = SchemaOptions::from_map(&map([(
        "operations",
        json!({"acme/customer:customer#find-customer": {"path": "customers"}}),
    )]))
    .expect_err("a path template starts with a slash");

    assert_eq!(error.code(), "JSC002");
}
