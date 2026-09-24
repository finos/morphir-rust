//! Ion spelling of v4 type expressions, type definitions and type specifications.
//!
//! A canonical fully qualified name is a reference with no arguments, a canonical local name is a
//! variable, and a bare list is a tuple. Every other type is an annotated struct: `variable::`,
//! `reference::`, `tuple::`, `record::`, `extensibleRecord::`, `function::` or `unit::`.

use std::collections::BTreeMap;

use ion_rs::Element;
use morphir_core::ir::v4::{self, Access, TypeAttributes};
use morphir_core::naming::Name;

use super::{
    access_of, access_symbol, fq_name, list, local_name, member, name_elements, name_list,
    optional_doc, refuse_annotations, unwritten,
};
use crate::ir_transport::TransportDiagnostic;
use crate::ir_transport::ion::{annotation_names, required_field, required_string, struct_fields};

type TypeDef = v4::AccessControlled<v4::Documented<v4::TypeDefinition>>;
type TypeSpec = v4::Documented<v4::TypeSpecification>;

// =============================================================================
// Type expressions
// =============================================================================

pub(super) fn read_type(element: &Element) -> Result<v4::Type, TransportDiagnostic> {
    let names = annotation_names(element)?;
    if names.is_empty() {
        if let Some(text) = element.as_string() {
            return string_type(text);
        }
        if let Some(items) = element.as_list() {
            return Ok(v4::Type::Tuple(
                TypeAttributes::default(),
                read_types(items)?,
            ));
        }
        return Err(member(
            "a type is a canonical name, a list, or an annotated struct",
        ));
    }
    match names.as_slice() {
        ["tuple"] => {
            let items = element
                .as_list()
                .ok_or_else(|| member("tuple:: is a list"))?;
            Ok(v4::Type::Tuple(
                TypeAttributes::default(),
                read_types(items)?,
            ))
        }
        ["variable"] => {
            let fields = struct_fields(element, "variable")?;
            Ok(v4::Type::Variable(
                TypeAttributes::default(),
                local_name(required_string(&fields, "name")?)?,
            ))
        }
        ["reference"] => {
            let fields = struct_fields(element, "reference")?;
            let arguments = match fields.get("arguments") {
                Some(list) => read_types(
                    list.as_list()
                        .ok_or_else(|| member("arguments is a list"))?,
                )?,
                None => Vec::new(),
            };
            Ok(v4::Type::Reference(
                TypeAttributes::default(),
                fq_name(required_string(&fields, "name")?)?,
                arguments,
            ))
        }
        ["record"] => {
            let fields = struct_fields(element, "record")?;
            Ok(v4::Type::Record(
                TypeAttributes::default(),
                read_record_fields(fields.get("fields").copied())?,
            ))
        }
        ["extensibleRecord"] => {
            let fields = struct_fields(element, "extensibleRecord")?;
            Ok(v4::Type::ExtensibleRecord(
                TypeAttributes::default(),
                local_name(required_string(&fields, "variable")?)?,
                read_record_fields(fields.get("fields").copied())?,
            ))
        }
        ["function"] => {
            let fields = struct_fields(element, "function")?;
            Ok(v4::Type::Function(
                TypeAttributes::default(),
                Box::new(read_type(required_field(&fields, "parameterType")?)?),
                Box::new(read_type(required_field(&fields, "returnType")?)?),
            ))
        }
        ["unit"] => Ok(v4::Type::Unit(TypeAttributes::default())),
        names => Err(member(format!("unknown v4 type {}", names.join("::")))),
    }
}

/// A bare string: a reference when it is a fully qualified name, a variable otherwise.
fn string_type(text: &str) -> Result<v4::Type, TransportDiagnostic> {
    if text.contains(':') || text.contains('#') {
        return Ok(v4::Type::Reference(
            TypeAttributes::default(),
            fq_name(text)?,
            Vec::new(),
        ));
    }
    Ok(v4::Type::Variable(
        TypeAttributes::default(),
        local_name(text)?,
    ))
}

fn read_types(items: &ion_rs::Sequence) -> Result<Vec<v4::Type>, TransportDiagnostic> {
    items.iter().map(read_type).collect()
}

pub(super) fn write_type(ty: &v4::Type) -> Result<Element, TransportDiagnostic> {
    if *ty.attributes() != TypeAttributes::default() {
        return Err(unwritten("v4 type attributes"));
    }
    Ok(match ty {
        v4::Type::Variable(_, name) => Element::string(name.to_canonical_string()),
        v4::Type::Reference(_, name, arguments) if arguments.is_empty() => {
            Element::string(name.to_canonical_string())
        }
        v4::Type::Reference(_, name, arguments) => Element::from(
            ion_rs::Struct::builder()
                .with_field("name", name.to_canonical_string())
                .with_field("arguments", list(write_types(arguments)?))
                .build(),
        )
        .with_annotations(["reference"]),
        v4::Type::Tuple(_, elements) => {
            Element::from(list(write_types(elements)?)).with_annotations(["tuple"])
        }
        v4::Type::Record(_, fields) => Element::from(
            ion_rs::Struct::builder()
                .with_field("fields", list(write_record_fields(fields)?))
                .build(),
        )
        .with_annotations(["record"]),
        v4::Type::ExtensibleRecord(_, variable, fields) => Element::from(
            ion_rs::Struct::builder()
                .with_field("variable", variable.to_canonical_string())
                .with_field("fields", list(write_record_fields(fields)?))
                .build(),
        )
        .with_annotations(["extensibleRecord"]),
        v4::Type::Function(_, parameter, return_type) => Element::from(
            ion_rs::Struct::builder()
                .with_field("parameterType", write_type(parameter)?)
                .with_field("returnType", write_type(return_type)?)
                .build(),
        )
        .with_annotations(["function"]),
        v4::Type::Unit(_) => {
            Element::from(ion_rs::Struct::builder().build()).with_annotations(["unit"])
        }
    })
}

fn write_types(types: &[v4::Type]) -> Result<Vec<Element>, TransportDiagnostic> {
    types.iter().map(write_type).collect()
}

fn read_record_fields(element: Option<&Element>) -> Result<Vec<v4::Field>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(Vec::new());
    };
    let items = element
        .as_list()
        .ok_or_else(|| member("fields is a list"))?;
    let mut fields: Vec<v4::Field> = Vec::new();
    for item in items.iter() {
        let members = struct_fields(item, "field")?;
        let name = local_name(required_string(&members, "name")?)?;
        if fields.iter().any(|field| field.name == name) {
            return Err(member(format!(
                "field '{}' is listed twice",
                name.to_canonical_string()
            )));
        }
        fields.push(v4::Field {
            name,
            tpe: read_type(required_field(&members, "type")?)?,
        });
    }
    Ok(fields)
}

fn write_record_fields(fields: &[v4::Field]) -> Result<Vec<Element>, TransportDiagnostic> {
    fields
        .iter()
        .map(|field| {
            Ok(Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", field.name.to_canonical_string())
                    .with_field("type", write_type(&field.tpe)?)
                    .build(),
            ))
        })
        .collect()
}

// =============================================================================
// Type definitions
// =============================================================================

pub(super) fn read_type_def(element: &Element) -> Result<(String, TypeDef), TransportDiagnostic> {
    let names = annotation_names(element)?;
    let access = access_of(&names)?;
    let fields = struct_fields(element, "type")?;
    let name = required_string(&fields, "name")?.to_owned();
    let type_params = name_list(&fields, "typeParams")?;
    let defined = match names.as_slice() {
        [_, "def", "alias", "type"] => v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr: read_type(required_field(&fields, "typeExp")?)?,
        },
        [_, "def", "custom", "type"] => v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors: v4::AccessControlled {
                access: constructor_access(&fields)?,
                value: read_constructors(fields.get("constructors").copied())?
                    .into_iter()
                    .map(|(name, args)| v4::ConstructorDefinition {
                        name,
                        args: args
                            .into_iter()
                            .map(|(name, arg_type)| v4::ConstructorArg { name, arg_type })
                            .collect(),
                    })
                    .collect(),
            },
        },
        [_, "def", "incomplete", "type"] => v4::TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness: read_incompleteness(required_field(&fields, "incompleteness")?)?,
            partial_type_expr: fields
                .get("partialTypeExp")
                .map(|e| read_type(e))
                .transpose()?,
        },
        _ => {
            return Err(member(format!(
                "unknown v4 type definition {}",
                names.join("::")
            )));
        }
    };
    Ok((
        name,
        v4::AccessControlled {
            access,
            value: v4::Documented::new(optional_doc(&fields)?, defined),
        },
    ))
}

pub(super) fn write_type_def(
    name: &str,
    defined: &TypeDef,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    let kind = match &defined.value.value {
        v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => {
            builder = with_type_params(builder, type_params)
                .with_field("typeExp", write_type(type_expr)?);
            "alias"
        }
        v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => {
            builder = with_type_params(builder, type_params).with_field(
                "access",
                Element::symbol(access_symbol(&constructors.access)),
            );
            let written = constructors
                .value
                .iter()
                .map(|constructor| {
                    write_constructor(
                        &constructor.name,
                        constructor
                            .args
                            .iter()
                            .map(|arg| (&arg.name, &arg.arg_type)),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !written.is_empty() {
                builder = builder.with_field("constructors", list(written));
            }
            "custom"
        }
        v4::TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
            partial_type_expr,
        } => {
            builder = with_type_params(builder, type_params)
                .with_field("incompleteness", write_incompleteness(incompleteness)?);
            if let Some(partial) = partial_type_expr {
                builder = builder.with_field("partialTypeExp", write_type(partial)?);
            }
            "incomplete"
        }
    };
    if let Some(doc) = &defined.value.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations([
        access_symbol(&defined.access),
        "def",
        kind,
        "type",
    ]))
}

// =============================================================================
// Type specifications
// =============================================================================

pub(super) fn read_type_spec(element: &Element) -> Result<(String, TypeSpec), TransportDiagnostic> {
    let names = annotation_names(element)?;
    let fields = struct_fields(element, "type spec")?;
    refuse_annotations(&fields)?;
    let name = required_string(&fields, "name")?.to_owned();
    let type_params = name_list(&fields, "typeParams")?;
    let spec = match names.as_slice() {
        ["public", "spec", "opaque", "type"] => v4::TypeSpecification::OpaqueTypeSpecification {
            annotations: Vec::new(),
            type_params,
        },
        ["public", "spec", "alias", "type"] => v4::TypeSpecification::TypeAliasSpecification {
            annotations: Vec::new(),
            type_params,
            type_expr: read_type(required_field(&fields, "typeExp")?)?,
        },
        ["public", "spec", "custom", "type"] => v4::TypeSpecification::CustomTypeSpecification {
            annotations: Vec::new(),
            type_params,
            constructors: read_constructors(fields.get("constructors").copied())?
                .into_iter()
                .map(|(name, args)| v4::ConstructorSpecification {
                    name,
                    args: args
                        .into_iter()
                        .map(|(name, arg_type)| v4::ConstructorArgSpec { name, arg_type })
                        .collect(),
                })
                .collect(),
        },
        ["public", "spec", "derived", "type"] => v4::TypeSpecification::DerivedTypeSpecification {
            annotations: Vec::new(),
            type_params,
            base_type: read_type(required_field(&fields, "baseType")?)?,
            from_base_type: fq_name(required_string(&fields, "fromBaseType")?)?,
            to_base_type: fq_name(required_string(&fields, "toBaseType")?)?,
        },
        _ => {
            return Err(member(format!(
                "unknown v4 type specification {}",
                names.join("::")
            )));
        }
    };
    Ok((name, v4::Documented::new(optional_doc(&fields)?, spec)))
}

pub(super) fn write_type_spec(name: &str, spec: &TypeSpec) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    let (kind, annotations) = match &spec.value {
        v4::TypeSpecification::OpaqueTypeSpecification {
            annotations,
            type_params,
        } => {
            builder = with_type_params(builder, type_params);
            ("opaque", annotations)
        }
        v4::TypeSpecification::TypeAliasSpecification {
            annotations,
            type_params,
            type_expr,
        } => {
            builder = with_type_params(builder, type_params)
                .with_field("typeExp", write_type(type_expr)?);
            ("alias", annotations)
        }
        v4::TypeSpecification::CustomTypeSpecification {
            annotations,
            type_params,
            constructors,
        } => {
            builder = with_type_params(builder, type_params);
            let written = constructors
                .iter()
                .map(|constructor| {
                    write_constructor(
                        &constructor.name,
                        constructor
                            .args
                            .iter()
                            .map(|arg| (&arg.name, &arg.arg_type)),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !written.is_empty() {
                builder = builder.with_field("constructors", list(written));
            }
            ("custom", annotations)
        }
        v4::TypeSpecification::DerivedTypeSpecification {
            annotations,
            type_params,
            base_type,
            from_base_type,
            to_base_type,
        } => {
            builder = with_type_params(builder, type_params)
                .with_field("baseType", write_type(base_type)?)
                .with_field("fromBaseType", from_base_type.to_canonical_string())
                .with_field("toBaseType", to_base_type.to_canonical_string());
            ("derived", annotations)
        }
    };
    if !annotations.is_empty() {
        return Err(unwritten("Morphir annotations"));
    }
    if let Some(doc) = &spec.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations(["public", "spec", kind, "type"]))
}

// =============================================================================
// Shared pieces
// =============================================================================

type Constructor = (Name, Vec<(Name, v4::Type)>);

/// Constructors as `[ { name, args: [ { name, type } ] } ]`. Empty `args` are omitted.
fn read_constructors(element: Option<&Element>) -> Result<Vec<Constructor>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(Vec::new());
    };
    let items = element
        .as_list()
        .ok_or_else(|| member("constructors is a list"))?;
    let mut constructors: Vec<Constructor> = Vec::new();
    for item in items.iter() {
        let fields = struct_fields(item, "constructor")?;
        let name = local_name(required_string(&fields, "name")?)?;
        if constructors.iter().any(|(existing, _)| *existing == name) {
            return Err(member(format!(
                "constructor '{}' is listed twice",
                name.to_canonical_string()
            )));
        }
        let mut args = Vec::new();
        if let Some(list) = fields.get("args") {
            for arg in list
                .as_list()
                .ok_or_else(|| member("args is a list"))?
                .iter()
            {
                let arg_fields = struct_fields(arg, "constructor argument")?;
                args.push((
                    local_name(required_string(&arg_fields, "name")?)?,
                    read_type(required_field(&arg_fields, "type")?)?,
                ));
            }
        }
        constructors.push((name, args));
    }
    Ok(constructors)
}

fn write_constructor<'a>(
    name: &Name,
    args: impl Iterator<Item = (&'a Name, &'a v4::Type)>,
) -> Result<Element, TransportDiagnostic> {
    let args = args
        .map(|(name, ty)| {
            Ok(Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", name.to_canonical_string())
                    .with_field("type", write_type(ty)?)
                    .build(),
            ))
        })
        .collect::<Result<Vec<_>, TransportDiagnostic>>()?;
    let mut builder = ion_rs::Struct::builder().with_field("name", name.to_canonical_string());
    if !args.is_empty() {
        builder = builder.with_field("args", list(args));
    }
    Ok(Element::from(builder.build()))
}

fn constructor_access(fields: &BTreeMap<&str, &Element>) -> Result<Access, TransportDiagnostic> {
    match crate::ir_transport::ion::required_text(fields, "access")? {
        "public" => Ok(Access::Public),
        "private" => Ok(Access::Private),
        other => Err(member(format!(
            "constructor access is public or private, found {other}"
        ))),
    }
}

fn with_type_params(builder: ion_rs::StructBuilder, params: &[Name]) -> ion_rs::StructBuilder {
    if params.is_empty() {
        builder
    } else {
        builder.with_field("typeParams", name_elements(params))
    }
}

/// `draft`, or `hole::{ reason, partialBody }`.
pub(super) fn read_incompleteness(
    element: &Element,
) -> Result<v4::Incompleteness, TransportDiagnostic> {
    if element.as_symbol().and_then(|symbol| symbol.text()) == Some("draft") {
        return Ok(v4::Incompleteness::Draft);
    }
    if annotation_names(element)?.as_slice() != ["hole"] {
        return Err(member("incompleteness is draft or hole::{ reason }"));
    }
    let fields = struct_fields(element, "hole")?;
    Ok(v4::Incompleteness::Hole {
        reason: read_hole_reason(required_field(&fields, "reason")?)?,
        partial_body: fields
            .get("partialBody")
            .map(|e| read_type(e))
            .transpose()?,
    })
}

pub(super) fn write_incompleteness(
    incompleteness: &v4::Incompleteness,
) -> Result<Element, TransportDiagnostic> {
    match incompleteness {
        v4::Incompleteness::Draft => Ok(Element::symbol("draft")),
        v4::Incompleteness::Hole {
            reason,
            partial_body,
        } => {
            let mut builder =
                ion_rs::Struct::builder().with_field("reason", write_hole_reason(reason)?);
            if let Some(partial) = partial_body {
                builder = builder.with_field("partialBody", write_type(partial)?);
            }
            Ok(Element::from(builder.build()).with_annotations(["hole"]))
        }
    }
}

pub(super) fn read_hole_reason(element: &Element) -> Result<v4::HoleReason, TransportDiagnostic> {
    match annotation_names(element)?.as_slice() {
        ["unresolvedReference"] => {
            let fields = struct_fields(element, "unresolvedReference")?;
            Ok(v4::HoleReason::UnresolvedReference {
                target: fq_name(required_string(&fields, "target")?)?,
            })
        }
        ["deletedDuringRefactor"] => {
            let fields = struct_fields(element, "deletedDuringRefactor")?;
            Ok(v4::HoleReason::DeletedDuringRefactor {
                tx_id: required_string(&fields, "txId")?.to_owned(),
            })
        }
        ["typeMismatch"] => {
            let fields = struct_fields(element, "typeMismatch")?;
            Ok(v4::HoleReason::TypeMismatch {
                expected: required_string(&fields, "expected")?.to_owned(),
                found: required_string(&fields, "found")?.to_owned(),
            })
        }
        names => Err(member(format!("unknown hole reason {}", names.join("::")))),
    }
}

pub(super) fn write_hole_reason(reason: &v4::HoleReason) -> Result<Element, TransportDiagnostic> {
    let (annotation, fields) = match reason {
        v4::HoleReason::UnresolvedReference { target } => (
            "unresolvedReference",
            ion_rs::Struct::builder().with_field("target", target.to_canonical_string()),
        ),
        v4::HoleReason::DeletedDuringRefactor { tx_id } => (
            "deletedDuringRefactor",
            ion_rs::Struct::builder().with_field("txId", tx_id.as_str()),
        ),
        v4::HoleReason::TypeMismatch { expected, found } => (
            "typeMismatch",
            ion_rs::Struct::builder()
                .with_field("expected", expected.as_str())
                .with_field("found", found.as_str()),
        ),
    };
    Ok(Element::from(fields.build()).with_annotations([annotation]))
}
