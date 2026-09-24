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
use morphir_core::ir::v4::{Annotation, AnnotationArgument};

use super::values::{read_value, write_value};
use super::{fq_name, list, local_name, member};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{annotation_names, required_field, required_string, struct_fields};

pub(super) fn read_annotations(
    fields: &BTreeMap<&str, &Element>,
) -> Result<Vec<Annotation>, TransportDiagnostic> {
    let Some(element) = fields.get("annotations") else {
        return Ok(Vec::new());
    };
    element
        .as_list()
        .ok_or_else(|| member("annotations is a list"))?
        .iter()
        .map(read_annotation)
        .collect()
}

pub(super) fn with_annotations(
    builder: ion_rs::StructBuilder,
    annotations: &[Annotation],
) -> Result<ion_rs::StructBuilder, TransportDiagnostic> {
    if annotations.is_empty() {
        return Ok(builder);
    }
    let written = annotations
        .iter()
        .map(write_annotation)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(builder.with_field("annotations", list(written)))
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

fn read_annotation(element: &Element) -> Result<Annotation, TransportDiagnostic> {
    if let Some(text) = element.as_string() {
        let split = text
            .find('#')
            .and_then(|hash| text[hash + 1..].find(':').map(|colon| hash + 1 + colon));
        let (name, free_text) = match split {
            Some(at) => (&text[..at], Some(text[at + 1..].to_owned())),
            None => (text, None),
        };
        return Ok(Annotation::Compact {
            name: fq_name(name)?,
            text: free_text,
        });
    }
    if !annotation_names(element)?.is_empty() || element.as_struct().is_none() {
        return Err(member("an annotation is a string or an unannotated struct"));
    }
    let fields = struct_fields(element, "annotation")?;
    if let Some(unknown) = fields
        .keys()
        .find(|name| !["name", "arguments"].contains(name))
    {
        return Err(member(format!(
            "an annotation has name and arguments, found {unknown}"
        )));
    }
    let args = match fields.get("arguments") {
        None => Vec::new(),
        Some(arguments) => arguments
            .as_list()
            .ok_or_else(|| member("arguments is a list"))?
            .iter()
            .map(read_argument)
            .collect::<Result<_, _>>()?,
    };
    Ok(Annotation::Structured {
        name: fq_name(required_string(&fields, "name")?)?,
        args,
    })
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
