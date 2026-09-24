//! Ion spelling of a classic type expression.

use std::collections::BTreeMap;

use ion_rs::Element;
use morphir_core::ir::classic;

use super::{
    IonCodec, annotation_names, canonical_name, canonical_package, classic_name_from, classic_path,
    classic_path_from, required_field, struct_fields,
};
use crate::ir_transport::{Stage, TransportDiagnostic};

pub(super) fn read_type(
    element: &Element,
) -> Result<classic::Type<classic::Attrs>, TransportDiagnostic> {
    if let Some(text) = element.as_string() {
        return read_compact_string(text);
    }
    if annotation_names(element)?.is_empty()
        && let Some(list) = element.as_list()
    {
        return read_tuple_elements(list);
    }
    match annotation_names(element)?.as_slice() {
        ["unit"] => Ok(classic::Type::Unit(classic::Attrs::None)),
        ["variable"] => {
            let fields = struct_fields(element, "variable")?;
            Ok(classic::Type::Variable(
                classic::Attrs::None,
                local_name(required_string_field(&fields, "name")?)?,
            ))
        }
        ["reference"] => read_reference(element),
        ["tuple"] => {
            let Some(list) = element.as_list() else {
                return Err(type_error("tuple:: is a list of types"));
            };
            read_tuple_elements(list)
        }
        ["record"] => {
            let fields = struct_fields(element, "record")?;
            Ok(classic::Type::Record(
                classic::Attrs::None,
                read_fields(fields.get("fields").copied())?,
            ))
        }
        ["extensibleRecord"] => {
            let fields = struct_fields(element, "extensibleRecord")?;
            Ok(classic::Type::ExtensibleRecord(
                classic::Attrs::None,
                local_name(required_string_field(&fields, "variable")?)?,
                read_fields(fields.get("fields").copied())?,
            ))
        }
        ["function"] => {
            let fields = struct_fields(element, "function")?;
            Ok(classic::Type::Function(
                classic::Attrs::None,
                Box::new(read_type(required_field(&fields, "parameterType")?)?),
                Box::new(read_type(required_field(&fields, "returnType")?)?),
            ))
        }
        names => Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Normalization,
            format!(
                "expected a type, found {}",
                if names.is_empty() {
                    element.ion_type().to_string()
                } else {
                    names.join("::")
                }
            ),
        )),
    }
}

pub(super) fn write_type(ty: &classic::Type<classic::Attrs>) -> Element {
    match ty {
        classic::Type::Unit(_) => empty_struct("unit"),
        classic::Type::Variable(_, name) => Element::string(canonical_name(name)),
        classic::Type::Reference(_, name, arguments) if arguments.is_empty() => {
            Element::string(canonical_fq(name))
        }
        classic::Type::Reference(_, name, arguments) => {
            let arguments = arguments
                .iter()
                .map(write_type)
                .fold(ion_rs::Sequence::builder(), |builder, element| {
                    builder.push(element)
                });
            Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", canonical_fq(name))
                    .with_field("arguments", arguments.build_list())
                    .build(),
            )
            .with_annotations(["reference"])
        }
        classic::Type::Tuple(_, elements) => {
            annotate_list("tuple", elements.iter().map(write_type))
        }
        classic::Type::Record(_, fields) => Element::from(
            ion_rs::Struct::builder()
                .with_field("fields", write_fields(fields))
                .build(),
        )
        .with_annotations(["record"]),
        classic::Type::ExtensibleRecord(_, variable, fields) => Element::from(
            ion_rs::Struct::builder()
                .with_field("variable", canonical_name(variable))
                .with_field("fields", write_fields(fields))
                .build(),
        )
        .with_annotations(["extensibleRecord"]),
        classic::Type::Function(_, parameter, return_type) => Element::from(
            ion_rs::Struct::builder()
                .with_field("parameterType", write_type(parameter))
                .with_field("returnType", write_type(return_type))
                .build(),
        )
        .with_annotations(["function"]),
    }
}

pub(super) fn read_constructors(
    element: &Element,
) -> Result<Vec<classic::Constructor<classic::Attrs>>, TransportDiagnostic> {
    let Some(list) = element.as_list() else {
        return Err(type_error("constructors is a list"));
    };
    let mut constructors = Vec::new();
    for item in list.iter() {
        let fields = struct_fields(item, "constructor")?;
        let name = local_name(required_string_field(&fields, "name")?)?;
        let mut args = Vec::new();
        if let Some(raw_args) = fields.get("args") {
            let Some(arg_list) = raw_args.as_list() else {
                return Err(type_error("args is a list"));
            };
            for arg in arg_list.iter() {
                let arg_fields = struct_fields(arg, "argument")?;
                args.push((
                    local_name(required_string_field(&arg_fields, "name")?)?,
                    read_type(required_field(&arg_fields, "type")?)?,
                ));
            }
        }
        constructors.push(classic::Constructor { name, args });
    }
    Ok(constructors)
}

pub(super) fn write_constructors(
    constructors: &[classic::Constructor<classic::Attrs>],
) -> ion_rs::List {
    constructors
        .iter()
        .map(|constructor| {
            let mut args = ion_rs::Sequence::builder();
            for (name, ty) in &constructor.args {
                args = args.push(Element::from(
                    ion_rs::Struct::builder()
                        .with_field("name", canonical_name(name))
                        .with_field("type", write_type(ty))
                        .build(),
                ));
            }
            Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", canonical_name(&constructor.name))
                    .with_field("args", args.build_list())
                    .build(),
            )
        })
        .fold(ion_rs::Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

fn read_reference(element: &Element) -> Result<classic::Type<classic::Attrs>, TransportDiagnostic> {
    let fields = struct_fields(element, "reference")?;
    let name = canonical_fq_name(required_string_field(&fields, "name")?)?;
    let mut arguments = Vec::new();
    if let Some(raw) = fields.get("arguments") {
        let Some(list) = raw.as_list() else {
            return Err(type_error("arguments is a list"));
        };
        for argument in list.iter() {
            arguments.push(read_type(argument)?);
        }
    }
    Ok(classic::Type::Reference(
        classic::Attrs::None,
        name,
        arguments,
    ))
}

fn read_tuple_elements(
    list: &ion_rs::Sequence,
) -> Result<classic::Type<classic::Attrs>, TransportDiagnostic> {
    let mut elements = Vec::new();
    for element in list.iter() {
        elements.push(read_type(element)?);
    }
    Ok(classic::Type::Tuple(classic::Attrs::None, elements))
}

fn read_fields(
    element: Option<&Element>,
) -> Result<Vec<classic::Field<classic::Attrs>>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(Vec::new());
    };
    let Some(list) = element.as_list() else {
        return Err(type_error("fields is a list"));
    };
    let mut fields = Vec::new();
    for item in list.iter() {
        let members = struct_fields(item, "field")?;
        fields.push(classic::Field {
            name: local_name(required_string_field(&members, "name")?)?,
            ty: read_type(required_field(&members, "type")?)?,
        });
    }
    Ok(fields)
}

fn write_fields(fields: &[classic::Field<classic::Attrs>]) -> ion_rs::List {
    fields
        .iter()
        .map(|field| {
            Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", canonical_name(&field.name))
                    .with_field("type", write_type(&field.ty))
                    .build(),
            )
        })
        .fold(ion_rs::Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

fn read_compact_string(text: &str) -> Result<classic::Type<classic::Attrs>, TransportDiagnostic> {
    if text.contains('#') || text.contains(':') {
        return Ok(classic::Type::Reference(
            classic::Attrs::None,
            canonical_fq_name(text)?,
            Vec::new(),
        ));
    }
    Ok(classic::Type::Variable(
        classic::Attrs::None,
        local_name(text)?,
    ))
}

pub(super) fn local_name(text: &str) -> Result<classic::Name, TransportDiagnostic> {
    let path = classic_path(text)?;
    if path.segments.len() != 1 {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            format!("'{text}' is one canonical name"),
        ));
    }
    Ok(path.segments[0].clone())
}

pub(super) fn canonical_fq_name(text: &str) -> Result<classic::FQName, TransportDiagnostic> {
    let name = morphir_core::naming::FQName::from_canonical_string(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            error,
        )
    })?;
    Ok(classic::FQName::new(
        classic_path_from(&name.package_path),
        classic_path_from(&name.module_path),
        classic_name_from(&name.local_name),
    ))
}

pub(super) fn canonical_fq(name: &classic::FQName) -> String {
    format!(
        "{}:{}#{}",
        canonical_package(&name.package_path),
        canonical_package(&name.module_path),
        canonical_name(&name.local_name)
    )
}

fn required_string_field<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    element.as_string().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a string"),
        )
    })
}

fn annotate_list(annotation: &str, elements: impl Iterator<Item = Element>) -> Element {
    Element::from(
        elements
            .fold(ion_rs::Sequence::builder(), |builder, element| {
                builder.push(element)
            })
            .build_list(),
    )
    .with_annotations([annotation])
}

fn empty_struct(annotation: &str) -> Element {
    Element::from(ion_rs::Struct::builder().build()).with_annotations([annotation])
}

fn type_error(message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        message,
    )
}
