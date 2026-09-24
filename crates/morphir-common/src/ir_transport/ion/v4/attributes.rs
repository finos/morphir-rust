//! Ion spelling of v4 attributes.
//!
//! A type's attributes are members of its expanded struct: `source`, `constraints` and
//! `extensions`. A value's or a pattern's attributes are one struct right after the S-expression
//! head: `source`, `inferredType` and `extensions`. `source` is
//! `{ startLine, startColumn, endLine, endColumn }`. `constraints` and `extensions` are JSON
//! objects. Empty members are omitted, and empty attributes are not written at all.

use std::collections::BTreeMap;

use ion_rs::Element;
use morphir_core::ir::v4::{SourceLocation, TypeAttributes, ValueAttributes};

use super::json::{object_from_ion, object_to_ion};
use super::member;
use super::types::{read_type, write_type};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{required_field, struct_fields};

const VALUE_MEMBERS: [&str; 3] = ["source", "inferredType", "extensions"];

pub(super) fn read_type_attributes(
    fields: &BTreeMap<&str, &Element>,
) -> Result<TypeAttributes, TransportDiagnostic> {
    Ok(TypeAttributes {
        source: fields.get("source").map(|e| read_source(e)).transpose()?,
        constraints: object(fields, "constraints")?,
        extensions: object(fields, "extensions")?,
    })
}

pub(super) fn with_type_attributes(
    mut builder: ion_rs::StructBuilder,
    attributes: &TypeAttributes,
) -> Result<ion_rs::StructBuilder, TransportDiagnostic> {
    if let Some(source) = &attributes.source {
        builder = builder.with_field("source", write_source(source));
    }
    if !attributes.constraints.is_empty() {
        builder = builder.with_field("constraints", object_to_ion(&attributes.constraints)?);
    }
    if !attributes.extensions.is_empty() {
        builder = builder.with_field("extensions", object_to_ion(&attributes.extensions)?);
    }
    Ok(builder)
}

pub(super) fn read_value_attributes(
    element: &Element,
) -> Result<ValueAttributes, TransportDiagnostic> {
    let fields = struct_fields(element, "attributes")?;
    if let Some(unknown) = fields.keys().find(|name| !VALUE_MEMBERS.contains(name)) {
        return Err(member(format!(
            "value attributes are source, inferredType and extensions, found {unknown}"
        )));
    }
    Ok(ValueAttributes {
        source: fields.get("source").map(|e| read_source(e)).transpose()?,
        inferred_type: fields
            .get("inferredType")
            .map(|e| read_type(e).map(Box::new))
            .transpose()?,
        extensions: object(&fields, "extensions")?,
    })
}

/// The struct that follows a head, or `None` when the attributes are empty.
pub(super) fn value_attributes(
    attributes: &ValueAttributes,
) -> Result<Option<Element>, TransportDiagnostic> {
    if *attributes == ValueAttributes::default() {
        return Ok(None);
    }
    let mut builder = ion_rs::Struct::builder();
    if let Some(source) = &attributes.source {
        builder = builder.with_field("source", write_source(source));
    }
    if let Some(inferred) = &attributes.inferred_type {
        builder = builder.with_field("inferredType", write_type(inferred)?);
    }
    if !attributes.extensions.is_empty() {
        builder = builder.with_field("extensions", object_to_ion(&attributes.extensions)?);
    }
    Ok(Some(Element::from(builder.build())))
}

fn object(
    fields: &BTreeMap<&str, &Element>,
    name: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, TransportDiagnostic> {
    fields
        .get(name)
        .map(|element| object_from_ion(element))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn read_source(element: &Element) -> Result<SourceLocation, TransportDiagnostic> {
    let fields = struct_fields(element, "source")?;
    let position = |name: &str| -> Result<u32, TransportDiagnostic> {
        required_field(&fields, name)?
            .as_i64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| member(format!("source {name} is a non-negative int")))
    };
    Ok(SourceLocation {
        start_line: position("startLine")?,
        start_column: position("startColumn")?,
        end_line: position("endLine")?,
        end_column: position("endColumn")?,
    })
}

fn write_source(source: &SourceLocation) -> Element {
    Element::from(
        ion_rs::Struct::builder()
            .with_field("startLine", i64::from(source.start_line))
            .with_field("startColumn", i64::from(source.start_column))
            .with_field("endLine", i64::from(source.end_line))
            .with_field("endColumn", i64::from(source.end_column))
            .build(),
    )
}
