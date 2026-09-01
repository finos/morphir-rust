//! Proves the shared-core property the whole design rests on: a type
//! projects to the same schema whether it is rendered by the JSON Schema
//! target or the OpenAPI target, apart from the base a `$ref` is written
//! against.
//!
//! The comparison is driven off [`morphir_openapi_extension::project`]
//! directly, not off whatever a JSON Schema document's `$defs` happens to
//! reach: a root that nothing else references (`Customer`, `Metrics`,
//! `Shape` in the fixture below) never appears inside anyone's `$defs`, only
//! as its own document's top-level body. Comparing only `$defs` entries
//! would silently exercise just the fixture's one referenced-and-shared
//! type (`Status`) and never touch `Object`, `OneOf`, `Array`, `Tuple`, or
//! `Map`.

use std::collections::HashMap;

use morphir_extension_sdk::{Artifact, Backend, GenerateRequest};
use morphir_openapi_extension::{OpenApiExtension, SchemaOptions, project};
use morphir_projection::{normalize, testing::classic};
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

/// Every schema name the JSON Schema target's documents carry a body for,
/// mapped to that body.
///
/// Two sources feed one map, because a name can appear in either place and
/// never both at once: a document's own top-level body (its root's schema,
/// with the document-only keys `$schema`, `$id`, `title`, and `$defs`
/// stripped so what remains is comparable to a `components/schemas` entry),
/// and any document's `$defs`, for a name some root's closure reached but
/// that is not itself a root document.
fn json_schema_bodies(artifacts: &[Artifact]) -> HashMap<String, Value> {
    let mut bodies = HashMap::new();
    for artifact in artifacts {
        let document: Value = serde_json::from_str(&artifact.content).expect("valid JSON");

        let title = document["title"]
            .as_str()
            .expect("a JSON Schema document has a title")
            .to_owned();
        let mut root_body = document.clone();
        if let Value::Object(object) = &mut root_body {
            for document_only_key in ["$schema", "$id", "title", "$defs"] {
                object.remove(document_only_key);
            }
        }
        bodies.entry(title).or_insert(root_body);

        for (name, definition) in document["$defs"].as_object().into_iter().flatten() {
            bodies
                .entry(name.clone())
                .or_insert_with(|| definition.clone());
        }
    }
    bodies
}

#[test]
fn a_type_has_the_same_schema_in_both_targets() {
    let package = normalize(&classic::classic_schema_library()).expect("the fixture normalizes");
    let projection = project(&package, &SchemaOptions::default()).expect("the fixture projects");
    assert!(
        !projection.definitions.is_empty(),
        "the fixture produced no definitions to compare"
    );

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
    assert_eq!(
        components.len(),
        projection.definitions.len(),
        "components/schemas must hold every projected definition, not a subset"
    );

    let bodies = json_schema_bodies(&schema_result.artifacts);
    assert_eq!(
        bodies.len(),
        projection.definitions.len(),
        "every projected definition must yield a JSON Schema body to compare"
    );

    let mut compared = 0;
    for name in projection.definitions.keys() {
        let body = bodies
            .get(name)
            .unwrap_or_else(|| panic!("{name} has no JSON Schema body to compare"));
        let component = components
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing from components/schemas"));
        assert_eq!(
            rebase(body, "#/$defs/", "#/components/schemas/"),
            *component,
            "{name} differs between targets"
        );
        compared += 1;
    }
    // Pinned to the fixture's real definition count (4: Customer, Metrics,
    // Shape, Status) so this test cannot silently shrink back down to
    // comparing only the one type every other type happens to reference.
    assert_eq!(
        compared, 4,
        "the fixture's definition count changed; update this assertion with it"
    );
}
