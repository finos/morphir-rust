//! Renderers that turn a dialect-neutral [`crate::SchemaProjection`] into a
//! dialect-specific document.
//!
//! [`json_schema`] renders JSON Schema 2020-12 documents and [`openapi`]
//! renders an OpenAPI 3.1 document. Both share the schema-body conversion
//! defined in this module, differing only in the base a `$ref` is written
//! against.

mod downgrade;
pub mod json_schema;
pub mod openapi;

pub use json_schema::render_json_schema;
pub use openapi::render_openapi;

use serde_json::{Map, Value, json};

use crate::{NamedSchema, Schema, SchemaVariant};

/// The JSON object body of one named schema: its own keywords, plus the
/// `x-morphir-fqname` and optional `description` every named schema carries.
///
/// Shared by both renderers: `reference_base` is the only place
/// reference-base knowledge lives, so the JSON Schema renderer (`#/$defs/`)
/// and the OpenAPI renderer (`#/components/schemas/`) produce the same body
/// for the same [`NamedSchema`], apart from where a `$ref` points.
pub(crate) fn named_schema_body(named: &NamedSchema, reference_base: &str) -> Map<String, Value> {
    let mut object = as_object(schema_body(&named.schema, reference_base));
    object.insert("x-morphir-fqname".to_owned(), json!(named.source_name));
    if let Some(doc) = &named.doc {
        object.insert("description".to_owned(), json!(doc));
    }
    object
}

/// The schema keywords for one [`Schema`], with every `$ref` written against
/// `reference_base`.
///
/// This is the shared schema-body conversion: `reference_base` is the only
/// place reference-base knowledge lives, so the JSON Schema renderer
/// (`#/$defs/`) and the OpenAPI renderer (`#/components/schemas/`) reuse it
/// unchanged, apart from where a `$ref` points.
pub(crate) fn schema_body(schema: &Schema, reference_base: &str) -> Value {
    let mut object = Map::new();
    match schema {
        Schema::Boolean => {
            object.insert("type".to_owned(), json!("boolean"));
        }
        Schema::Integer { format } => {
            object.insert("type".to_owned(), json!("integer"));
            if let Some(format) = format {
                object.insert("format".to_owned(), json!(format));
            }
        }
        Schema::Number { format } => {
            object.insert("type".to_owned(), json!("number"));
            if let Some(format) = format {
                object.insert("format".to_owned(), json!(format));
            }
        }
        Schema::Text { max_length } => {
            object.insert("type".to_owned(), json!("string"));
            if let Some(max_length) = max_length {
                object.insert("maxLength".to_owned(), json!(max_length));
            }
        }
        Schema::Null => {
            object.insert("type".to_owned(), json!("null"));
        }
        Schema::Array { items, unique } => {
            object.insert("type".to_owned(), json!("array"));
            object.insert("items".to_owned(), schema_body(items, reference_base));
            if *unique {
                object.insert("uniqueItems".to_owned(), json!(true));
            }
        }
        Schema::Tuple(members) => {
            let prefix_items: Vec<Value> = members
                .iter()
                .map(|member| schema_body(member, reference_base))
                .collect();
            let count = prefix_items.len();
            object.insert("type".to_owned(), json!("array"));
            object.insert("prefixItems".to_owned(), Value::Array(prefix_items));
            object.insert("items".to_owned(), json!(false));
            object.insert("minItems".to_owned(), json!(count));
            object.insert("maxItems".to_owned(), json!(count));
        }
        Schema::Map { values } => {
            object.insert("type".to_owned(), json!("object"));
            object.insert(
                "additionalProperties".to_owned(),
                schema_body(values, reference_base),
            );
        }
        Schema::Object { fields, required } => {
            object.insert("type".to_owned(), json!("object"));
            let mut properties = Map::new();
            for field in fields {
                let mut property = as_object(schema_body(&field.schema, reference_base));
                if let Some(doc) = &field.doc {
                    property.insert("description".to_owned(), json!(doc));
                }
                properties.insert(field.name.clone(), Value::Object(property));
            }
            object.insert("properties".to_owned(), Value::Object(properties));
            if !required.is_empty() {
                object.insert("required".to_owned(), json!(required));
            }
        }
        Schema::Enumeration(values) => {
            object.insert("type".to_owned(), json!("string"));
            object.insert("enum".to_owned(), json!(values));
        }
        Schema::OneOf {
            discriminator,
            variants,
        } => {
            let variants = variants
                .iter()
                .map(|variant| Value::Object(variant_body(variant, discriminator, reference_base)))
                .collect();
            object.insert("oneOf".to_owned(), Value::Array(variants));
        }
        // Always `anyOf`, even when every member is a simple type: a member
        // can be a `Schema::Reference`, and `anyOf` stays correct for that
        // case, so there is no separate "all members are simple" special case.
        Schema::Union(members) => {
            let members = members
                .iter()
                .map(|member| schema_body(member, reference_base))
                .collect();
            object.insert("anyOf".to_owned(), Value::Array(members));
        }
        Schema::Reference(name) => {
            object.insert("$ref".to_owned(), json!(format!("{reference_base}{name}")));
        }
    }
    Value::Object(object)
}

/// One [`Schema::OneOf`] variant, with its discriminator property fixed to
/// the constructor name by `const`.
fn variant_body(
    variant: &SchemaVariant,
    discriminator: &str,
    reference_base: &str,
) -> Map<String, Value> {
    let mut object = as_object(schema_body(&variant.schema, reference_base));
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        properties.insert(discriminator.to_owned(), json!({ "const": variant.name }));
    }
    let mut required: Vec<Value> = object
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    required.push(json!(discriminator));
    object.insert("required".to_owned(), Value::Array(required));
    object
}

/// Unwrap a [`schema_body`] result back into its object, for a caller that
/// needs to add more keys before serializing.
///
/// `schema_body` always returns `Value::Object`: every [`Schema`] variant
/// builds a JSON object, never a bare array or scalar.
fn as_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => unreachable!("schema_body always returns an object, got {other:?}"),
    }
}
