//! Ion spelling of Morphir annotations.
//!
//! A module, type or value specification may carry `annotations`, a list. A compact entry is a
//! string: `pkg:mod#local`, or `pkg:mod#local:free text`, where the text starts after the first
//! colon that follows the `#`. A structured entry is `{ name, arguments }`. A positional argument
//! is a value. A named argument is `{ name, value }`; no value is an unannotated struct, so the
//! shape decides. A definition carries no annotations, and neither does a package specification
//! until finos/morphir#944 decides.

use std::collections::BTreeMap;

use ion_rs::Element;
use morphir_core::ir::v4::{Annotation, AnnotationArgument, Annotations};

use super::values::{read_value, write_value};
use super::{list, local_name, member};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{annotation_names, required_field, required_string, struct_fields};

pub(super) fn read_annotations(
    fields: &BTreeMap<&str, &Element>,
) -> Result<Annotations, TransportDiagnostic> {
    let Some(element) = fields.get("annotations") else {
        return Ok(Annotations::default());
    };
    if let Some(items) = element.as_list() {
        let entries = items
            .iter()
            .map(entry_json)
            .collect::<Result<Vec<_>, _>>()?;
        return Annotations::parse_unresolved(&serde_json::Value::Array(entries)).map_err(member);
    }
    let envelope = struct_fields(element, "annotations")?;
    let mut object = serde_json::Map::new();
    for (name, value) in envelope {
        let value = if name == "entries" {
            let entries = value
                .as_list()
                .ok_or_else(|| member("annotation entries are a list"))?;
            serde_json::Value::Array(entries.iter().map(entry_json).collect::<Result<_, _>>()?)
        } else {
            super::json::from_ion(value)?
        };
        object.insert(name.to_owned(), value);
    }
    Annotations::parse_unresolved(&serde_json::Value::Object(object)).map_err(member)
}

pub(super) fn with_annotations(
    builder: ion_rs::StructBuilder,
    annotations: &Annotations,
) -> Result<ion_rs::StructBuilder, TransportDiagnostic> {
    if annotations.is_empty() {
        return Ok(builder);
    }
    let written = annotations
        .iter()
        .map(write_annotation)
        .collect::<Result<Vec<_>, _>>()?;
    let linked_entries = annotations.entries.iter().any(|entry| {
        matches!(
            entry,
            Annotation::LinkedCompact { .. }
                | Annotation::LinkedStructured { .. }
                | Annotation::PendingCompact { .. }
                | Annotation::PendingStructured { .. }
        )
    });
    if annotations.metadata.is_none() && !linked_entries {
        return Ok(builder.with_field("annotations", list(written)));
    }
    let mut envelope = ion_rs::Struct::builder();
    if let Some(metadata) = &annotations.metadata
        && let Some(context) = &metadata.context
    {
        envelope = envelope.with_field("@context", super::json::to_ion(context.authored())?);
    }
    if !annotations.entries.is_empty() {
        envelope = envelope.with_field("entries", list(written));
    }
    if let Some(metadata) = &annotations.metadata
        && !metadata.facts.is_empty()
    {
        let value =
            serde_json::to_value(&metadata.facts).map_err(|error| member(error.to_string()))?;
        envelope = envelope.with_field("facts", super::json::to_ion(&value)?);
    }
    Ok(builder.with_field("annotations", Element::from(envelope.build())))
}

fn entry_json(element: &Element) -> Result<serde_json::Value, TransportDiagnostic> {
    if let Some(text) = element.as_string() {
        return Ok(serde_json::Value::String(text.to_owned()));
    }
    if !annotation_names(element)?.is_empty() {
        return Err(member("an annotation entry is an unannotated struct"));
    }
    let fields = struct_fields(element, "annotation")?;
    if let Some(unknown) = fields
        .keys()
        .find(|name| !["name", "arguments"].contains(name))
    {
        return Err(member(format!(
            "an annotation entry has name and arguments, found {unknown}"
        )));
    }
    let mut object = serde_json::Map::new();
    object.insert(
        "name".to_owned(),
        serde_json::Value::String(required_string(&fields, "name")?.to_owned()),
    );
    if let Some(arguments) = fields.get("arguments") {
        let arguments = arguments
            .as_list()
            .ok_or_else(|| member("annotation arguments are a list"))?;
        let values = arguments
            .iter()
            .map(|arg| {
                let parsed = read_argument(arg)?;
                serde_json::to_value(parsed).map_err(|error| member(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        object.insert("arguments".to_owned(), serde_json::Value::Array(values));
    }
    Ok(serde_json::Value::Object(object))
}

/// A definition carries no annotations.
pub(super) fn refuse_on_definition(
    fields: &BTreeMap<&str, &Element>,
) -> Result<(), TransportDiagnostic> {
    if fields.contains_key("annotations") {
        return Err(member(
            "a definition carries no Morphir annotations; a specification does",
        ));
    }
    Ok(())
}

fn write_annotation(annotation: &Annotation) -> Result<Element, TransportDiagnostic> {
    Ok(match annotation {
        Annotation::Compact { name, text: None } => Element::string(name.to_canonical_string()),
        Annotation::Compact {
            name,
            text: Some(text),
        } => Element::string(format!("{}:{text}", name.to_canonical_string())),
        Annotation::Structured { name, args } => {
            let mut builder =
                ion_rs::Struct::builder().with_field("name", name.to_canonical_string());
            if !args.is_empty() {
                let written = args
                    .iter()
                    .map(write_argument)
                    .collect::<Result<Vec<_>, _>>()?;
                builder = builder.with_field("arguments", list(written));
            }
            Element::from(builder.build())
        }
        Annotation::LinkedCompact { authored_name, .. }
        | Annotation::PendingCompact { authored_name } => Element::string(authored_name.as_str()),
        Annotation::LinkedStructured {
            authored_name,
            args,
            ..
        }
        | Annotation::PendingStructured {
            authored_name,
            args,
        } => {
            let mut builder = ion_rs::Struct::builder().with_field("name", authored_name.as_str());
            if !args.is_empty() {
                let written = args
                    .iter()
                    .map(write_argument)
                    .collect::<Result<Vec<_>, _>>()?;
                builder = builder.with_field("arguments", list(written));
            }
            Element::from(builder.build())
        }
    })
}

fn read_argument(element: &Element) -> Result<AnnotationArgument, TransportDiagnostic> {
    if element.as_struct().is_some() && annotation_names(element)?.is_empty() {
        let fields = struct_fields(element, "named argument")?;
        if fields.len() != 2 || !fields.contains_key("name") || !fields.contains_key("value") {
            return Err(member("a named argument is { name, value }"));
        }
        return Ok(AnnotationArgument::Named {
            name: local_name(required_string(&fields, "name")?)?,
            value: read_value(required_field(&fields, "value")?)?,
        });
    }
    Ok(AnnotationArgument::Positional(read_value(element)?))
}

fn write_argument(argument: &AnnotationArgument) -> Result<Element, TransportDiagnostic> {
    Ok(match argument {
        AnnotationArgument::Positional(value) => write_value(value)?,
        AnnotationArgument::Named { name, value } => Element::from(
            ion_rs::Struct::builder()
                .with_field("name", name.to_canonical_string())
                .with_field("value", write_value(value)?)
                .build(),
        ),
    })
}
