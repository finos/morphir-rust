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
