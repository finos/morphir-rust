//! Render a [`SchemaProjection`] as JSON Schema 2020-12 documents.
//!
//! One document is produced per public root type. `$defs` holds only the
//! transitive closure the root actually reaches, so a document is
//! self-contained: no unused definition, no dangling `$ref`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value, json};

use crate::schema::references;
use crate::{NamedSchema, Schema, SchemaProjection, SchemaVariant};

/// The only dialect this renderer speaks.
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Where a JSON Schema document's own `$ref`s point.
const REF_PREFIX: &str = "#/$defs/";

/// Render every public root of `projection` as a standalone JSON Schema
/// 2020-12 document.
pub fn render_json_schema(projection: &SchemaProjection) -> Vec<Artifact> {
    projection
        .roots
        .iter()
        .map(|root| render_document(root, &projection.definitions))
        .collect()
}

/// Render one root as a complete, self-contained document.
fn render_document(root: &NamedSchema, definitions: &BTreeMap<String, NamedSchema>) -> Artifact {
    let mut document = named_schema_object(root, REF_PREFIX);
    document.insert("$schema".to_owned(), json!(DIALECT));
    document.insert(
        "$id".to_owned(),
        json!(format!("{}.schema.json", root.name)),
    );
    document.insert("title".to_owned(), json!(root.name));

    let closure = transitive_closure(&root.schema, definitions);
    if !closure.is_empty() {
        let mut defs = Map::new();
        for name in closure {
            let named = definitions
                .get(&name)
                .unwrap_or_else(|| panic!("dangling reference to '{name}' inside a document"));
            defs.insert(name, Value::Object(named_schema_object(named, REF_PREFIX)));
        }
        document.insert("$defs".to_owned(), Value::Object(defs));
    }

    Artifact {
        path: artifact_path(root),
        content: format!(
            "{}\n",
            serde_json::to_string_pretty(&Value::Object(document))
                .expect("a schema document made of Value::Object and String always serializes")
        ),
        binary: false,
    }
}

/// The JSON object body of one named schema: its own keywords, plus the
/// `x-morphir-fqname` and optional `description` every named schema carries.
fn named_schema_object(named: &NamedSchema, ref_prefix: &str) -> Map<String, Value> {
    let mut object = schema_object(&named.schema, ref_prefix);
    object.insert("x-morphir-fqname".to_owned(), json!(named.source_name));
    if let Some(doc) = &named.doc {
        object.insert("description".to_owned(), json!(doc));
    }
    object
}

/// The JSON Schema keywords for one [`Schema`], with every `$ref` written
/// against `ref_prefix`.
///
/// This is the shared schema-body conversion: `ref_prefix` is the only place
/// reference-base knowledge lives, so a second renderer (an OpenAPI renderer
/// over `#/components/schemas/`) can reuse it unchanged.
fn schema_object(schema: &Schema, ref_prefix: &str) -> Map<String, Value> {
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
            object.insert(
                "items".to_owned(),
                Value::Object(schema_object(items, ref_prefix)),
            );
            if *unique {
                object.insert("uniqueItems".to_owned(), json!(true));
            }
        }
        Schema::Tuple(members) => {
            let prefix_items: Vec<Value> = members
                .iter()
                .map(|member| Value::Object(schema_object(member, ref_prefix)))
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
                Value::Object(schema_object(values, ref_prefix)),
            );
        }
        Schema::Object { fields, required } => {
            object.insert("type".to_owned(), json!("object"));
            let mut properties = Map::new();
            for field in fields {
                let mut property = schema_object(&field.schema, ref_prefix);
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
                .map(|variant| Value::Object(variant_object(variant, discriminator, ref_prefix)))
                .collect();
            object.insert("oneOf".to_owned(), Value::Array(variants));
        }
        // Always `anyOf`, even when every member is a simple type: a member
        // can be a `Schema::Reference`, and `anyOf` stays correct for that
        // case, so there is no separate "all members are simple" special case.
        Schema::Union(members) => {
            let members = members
                .iter()
                .map(|member| Value::Object(schema_object(member, ref_prefix)))
                .collect();
            object.insert("anyOf".to_owned(), Value::Array(members));
        }
        Schema::Reference(name) => {
            object.insert("$ref".to_owned(), json!(format!("{ref_prefix}{name}")));
        }
    }
    object
}

/// One [`Schema::OneOf`] variant, with its discriminator property fixed to
/// the constructor name by `const`.
fn variant_object(
    variant: &SchemaVariant,
    discriminator: &str,
    ref_prefix: &str,
) -> Map<String, Value> {
    let mut object = schema_object(&variant.schema, ref_prefix);
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

/// Every definition `root` reaches, directly or through another definition.
fn transitive_closure(
    root: &Schema,
    definitions: &BTreeMap<String, NamedSchema>,
) -> BTreeSet<String> {
    let mut closure = BTreeSet::new();
    let mut queue: VecDeque<String> = references(root).into_iter().map(str::to_owned).collect();
    while let Some(name) = queue.pop_front() {
        if !closure.insert(name.clone()) {
            continue;
        }
        if let Some(named) = definitions.get(&name) {
            queue.extend(references(&named.schema).into_iter().map(str::to_owned));
        }
    }
    closure
}

/// `<module path segments, lowercased and dot-joined>.<schema name>.schema.json`
///
/// The module path is read out of `named.source_name`
/// (`<package>:<module>#<local>`), so the artifact path never drifts from
/// the FQName recorded in `x-morphir-fqname`.
fn artifact_path(named: &NamedSchema) -> String {
    let after_package = named
        .source_name
        .split_once(':')
        .map_or(named.source_name.as_str(), |(_, rest)| rest);
    let module = after_package
        .split_once('#')
        .map_or(after_package, |(module, _)| module);
    let segments: Vec<String> = module
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_lowercase)
        .collect();
    if segments.is_empty() {
        format!("{}.schema.json", named.name)
    } else {
        format!("{}.{}.schema.json", segments.join("."), named.name)
    }
}
