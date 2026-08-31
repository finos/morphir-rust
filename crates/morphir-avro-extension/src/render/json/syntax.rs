use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::Artifact;
use serde_json::{Map, Value};

use super::super::portable_artifact_path;
use crate::{AvroDiagnostic, AvroField, AvroFullName, AvroRoot, AvroType, Properties};

pub(super) fn render_field(
    field: &AvroField,
    render_type: &mut impl FnMut(&AvroType) -> Value,
) -> Value {
    let mut object = properties_object(field.properties());
    object.insert("name".to_owned(), Value::String(field.name().to_owned()));
    object.insert("type".to_owned(), render_type(field.tpe()));
    Value::Object(object)
}

pub(super) fn decorate_type(
    value: Value,
    properties: &Properties,
    logical_type: Option<&str>,
) -> Value {
    let mut object = properties_object(properties);
    if let Some(logical_type) = logical_type {
        object.insert(
            "logicalType".to_owned(),
            Value::String(logical_type.to_owned()),
        );
    }
    match value {
        Value::Object(inner) => object.extend(inner),
        other => {
            object.insert("type".to_owned(), other);
        }
    }
    Value::Object(object)
}

pub(super) fn decorate_root(value: Value, root: &AvroRoot, nested_named_target: bool) -> Value {
    let mut object = match (nested_named_target, value) {
        (true, nested) => {
            let mut object = Map::new();
            object.insert("type".to_owned(), nested);
            object
        }
        (false, Value::Object(inner)) => inner,
        (false, other) => {
            let mut object = Map::new();
            object.insert("type".to_owned(), other);
            object
        }
    };
    object.extend(properties_object(root.properties()));
    insert_doc(&mut object, root.doc());
    Value::Object(object)
}

pub(super) fn properties_object(properties: &Properties) -> Map<String, Value> {
    properties
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(super) fn insert_doc(object: &mut Map<String, Value>, doc: Option<&str>) {
    if let Some(doc) = doc {
        object.insert("doc".to_owned(), Value::String(doc.to_owned()));
    }
}

pub(super) fn insert_name(object: &mut Map<String, Value>, name: &AvroFullName) {
    object.insert("name".to_owned(), Value::String(name.name().to_owned()));
    if !name.namespace().is_empty() {
        object.insert(
            "namespace".to_owned(),
            Value::String(name.namespace().to_owned()),
        );
    }
}

pub(super) fn schema_path(name: &AvroFullName) -> String {
    artifact_path(name, "avsc")
}

pub(super) fn protocol_path(name: &AvroFullName) -> String {
    artifact_path(name, "avpr")
}

pub(super) fn artifact_path(name: &AvroFullName, extension: &str) -> String {
    portable_artifact_path(name, extension)
}

pub(super) fn text_artifact(path: String, value: Value) -> Result<Artifact, AvroDiagnostic> {
    let mut content = serde_json::to_string_pretty(&canonicalize(value))
        .map_err(|error| AvroDiagnostic::invalid_option(format!("render JSON: {error}")))?;
    content.push('\n');
    Ok(Artifact {
        path,
        content,
        binary: false,
    })
}

pub(super) fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}

pub(super) fn insert_artifact(
    artifacts: &mut Vec<Artifact>,
    paths: &mut BTreeSet<String>,
    artifact: Artifact,
) -> Result<(), AvroDiagnostic> {
    let path = artifact.path.clone();
    if !paths.insert(path.clone()) {
        return Err(AvroDiagnostic::name_collision(format!(
            "duplicate artifact path {path}"
        )));
    }
    artifacts.push(artifact);
    Ok(())
}
