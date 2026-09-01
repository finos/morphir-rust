//! Proves the shared-core property the whole design rests on: a type
//! projects to the same schema whether it is rendered by the JSON Schema
//! target or the OpenAPI target, apart from the base a `$ref` is written
//! against.

use std::collections::HashMap;

use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_openapi_extension::OpenApiExtension;
use morphir_projection::testing::classic;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn generate(target: &str) -> morphir_extension_sdk::GenerateResult {
    OpenApiExtension
        .generate(GenerateRequest {
            ir: classic::classic_schema_library(),
            target: target.into(),
            options: HashMap::new(),
        })
        .expect("generation is a successful MEP call")
}

fn rebase(value: &Value, from: &str, to: &str) -> Value {
    match value {
        Value::Object(members) => members
            .iter()
            .map(|(key, member)| {
                if key == "$ref" {
                    let reference = member.as_str().unwrap_or_default().replace(from, to);
                    (key.clone(), Value::String(reference))
                } else {
                    (key.clone(), rebase(member, from, to))
                }
            })
            .collect::<serde_json::Map<_, _>>()
            .into(),
        Value::Array(members) => members.iter().map(|m| rebase(m, from, to)).collect(),
        other => other.clone(),
    }
}

#[test]
fn a_type_has_the_same_schema_in_both_targets() {
    let schema_result = generate("json-schema");
    let openapi_result = generate("openapi");

    let openapi: Value = serde_json::from_str(
        &openapi_result
            .artifacts
            .iter()
            .find(|artifact| artifact.path == "openapi.json")
            .expect("the openapi document is generated")
            .content,
    )
    .expect("valid JSON");
    let components = openapi["components"]["schemas"]
        .as_object()
        .expect("components/schemas is an object");

    let mut compared = 0;
    for artifact in &schema_result.artifacts {
        let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");
        for (name, definition) in document["$defs"].as_object().into_iter().flatten() {
            let component = components
                .get(name)
                .unwrap_or_else(|| panic!("{name} is missing from components/schemas"));
            assert_eq!(
                rebase(definition, "#/$defs/", "#/components/schemas/"),
                *component,
                "{name} differs between targets"
            );
            compared += 1;
        }
    }
    assert!(compared > 0, "the fixture produced no shared definitions");
}
