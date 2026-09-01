//! Render a [`SchemaProjection`] as JSON Schema 2020-12 documents.
//!
//! One document is produced per public root type. `$defs` holds only the
//! transitive closure the root actually reaches, so a document is
//! self-contained: no unused definition, no dangling `$ref`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value, json};

use crate::render::named_schema_body;
use crate::schema::references;
use crate::{NamedSchema, Schema, SchemaProjection};

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
    let mut document = named_schema_body(root, REF_PREFIX);
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
            defs.insert(name, Value::Object(named_schema_body(named, REF_PREFIX)));
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
