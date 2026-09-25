//! Conversion of V3 and V4 declarations into closed validation shapes.

use super::{DataValueError, Definition, Shape, unsupported};
use crate::ir::{classic, v4};
use crate::naming::Name;
use std::collections::HashMap;

pub(super) fn v4_shape(ty: &v4::Type) -> Shape {
    match ty {
        v4::Type::Unit(_) => Shape::Unit,
        v4::Type::Variable(_, name) => Shape::Variable(name.to_canonical_string()),
        v4::Type::Reference(_, name, args) => Shape::Reference(
            name.to_canonical_string(),
            args.iter().map(v4_shape).collect(),
        ),
        v4::Type::Record(_, fields) => Shape::Record(
            fields
                .iter()
                .map(|field| (field.name.to_camel_case(), v4_shape(&field.tpe)))
                .collect(),
        ),
        v4::Type::Tuple(_, elements) => Shape::Tuple(elements.iter().map(v4_shape).collect()),
        v4::Type::ExtensibleRecord(_, _, _) => Shape::Unsupported("extensible record"),
        v4::Type::Function(_, _, _) => Shape::Unsupported("function"),
    }
}

pub(super) fn v3_shape(ty: &classic::Type<classic::Attrs>) -> Shape {
    match ty {
        classic::Type::Unit(_) => Shape::Unit,
        classic::Type::Variable(_, name) => Shape::Variable(name.to_string()),
        classic::Type::Reference(_, name, args) => {
            Shape::Reference(classic_fq(name), args.iter().map(v3_shape).collect())
        }
        classic::Type::Record(_, fields) => Shape::Record(
            fields
                .iter()
                .map(|field| (camel(&field.name.to_string()), v3_shape(&field.ty)))
                .collect(),
        ),
        classic::Type::Tuple(_, elements) => Shape::Tuple(elements.iter().map(v3_shape).collect()),
        classic::Type::ExtensibleRecord(_, _, _) => Shape::Unsupported("extensible record"),
        classic::Type::Function(_, _, _) => Shape::Unsupported("function"),
    }
}

fn camel(canonical: &str) -> String {
    Name::from_canonical_string(canonical)
        .expect("classic name normalized")
        .to_camel_case()
}

pub(super) fn classic_path(path: &classic::Path) -> String {
    path.segments
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn classic_fq(name: &classic::FQName) -> String {
    format!(
        "{}:{}#{}",
        classic_path(&name.package_path),
        classic_path(&name.module_path),
        name.local_name
    )
}

fn v4_definition(definition: &v4::TypeDefinition) -> Definition {
    match definition {
        v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => Definition::Alias {
            params: type_params.iter().map(Name::to_canonical_string).collect(),
            body: v4_shape(type_expr),
        },
        v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => Definition::Custom {
            params: type_params.iter().map(Name::to_canonical_string).collect(),
            constructors: constructors
                .value
                .iter()
                .map(|constructor| {
                    (
                        constructor.name.to_camel_case(),
                        constructor
                            .args
                            .iter()
                            .map(|arg| v4_shape(&arg.arg_type))
                            .collect(),
                    )
                })
                .collect(),
        },
        v4::TypeDefinition::IncompleteTypeDefinition { .. } => {
            Definition::Unsupported("incomplete type")
        }
    }
}

fn v4_specification(specification: &v4::TypeSpecification) -> Definition {
    match specification {
        v4::TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
            ..
        } => Definition::Alias {
            params: type_params.iter().map(Name::to_canonical_string).collect(),
            body: v4_shape(type_expr),
        },
        v4::TypeSpecification::CustomTypeSpecification {
            type_params,
            constructors,
            ..
        } => Definition::Custom {
            params: type_params.iter().map(Name::to_canonical_string).collect(),
            constructors: constructors
                .iter()
                .map(|constructor| {
                    (
                        constructor.name.to_camel_case(),
                        constructor
                            .args
                            .iter()
                            .map(|arg| v4_shape(&arg.arg_type))
                            .collect(),
                    )
                })
                .collect(),
        },
        v4::TypeSpecification::OpaqueTypeSpecification { .. } => {
            Definition::Unsupported("opaque type")
        }
        v4::TypeSpecification::DerivedTypeSpecification { .. } => {
            Definition::Unsupported("derived type")
        }
    }
}

fn v3_definition(definition: &classic::TypeDefinition<classic::Attrs>) -> Definition {
    match definition {
        classic::TypeDefinition::Alias(params, body) => Definition::Alias {
            params: params.iter().map(ToString::to_string).collect(),
            body: v3_shape(body),
        },
        classic::TypeDefinition::Custom(params, constructors) => Definition::Custom {
            params: params.iter().map(ToString::to_string).collect(),
            constructors: constructors
                .value
                .iter()
                .map(|constructor| {
                    (
                        camel(&constructor.name.to_string()),
                        constructor
                            .args
                            .iter()
                            .map(|(_, ty)| v3_shape(ty))
                            .collect(),
                    )
                })
                .collect(),
        },
    }
}

fn v3_specification(specification: &classic::TypeSpecification<classic::Attrs>) -> Definition {
    match specification {
        classic::TypeSpecification::Alias(params, body) => Definition::Alias {
            params: params.iter().map(ToString::to_string).collect(),
            body: v3_shape(body),
        },
        classic::TypeSpecification::Custom(params, constructors) => Definition::Custom {
            params: params.iter().map(ToString::to_string).collect(),
            constructors: constructors
                .iter()
                .map(|constructor| {
                    (
                        camel(&constructor.name.to_string()),
                        constructor
                            .args
                            .iter()
                            .map(|(_, ty)| v3_shape(ty))
                            .collect(),
                    )
                })
                .collect(),
        },
        classic::TypeSpecification::Opaque(_) => Definition::Unsupported("opaque type"),
        classic::TypeSpecification::Derived(_, _) => Definition::Unsupported("derived type"),
    }
}

fn insert_definition(
    output: &mut HashMap<String, Definition>,
    key: String,
    definition: Definition,
) -> Result<(), DataValueError> {
    if output.insert(key.clone(), definition).is_some() {
        return Err(unsupported("$", format!("duplicate data type {key}")));
    }
    Ok(())
}

pub(super) fn collect_v4_definitions(
    output: &mut HashMap<String, Definition>,
    package: &str,
    definition: &v4::PackageDefinition,
) -> Result<(), DataValueError> {
    for (module, controlled) in &definition.modules {
        for (name, ty) in &controlled.value.types {
            insert_definition(
                output,
                format!("{package}:{module}#{name}"),
                v4_definition(&ty.value.value),
            )?;
        }
    }
    Ok(())
}

pub(super) fn collect_v4_specifications(
    output: &mut HashMap<String, Definition>,
    package: &str,
    specification: &v4::PackageSpecification,
) -> Result<(), DataValueError> {
    for (module, spec) in &specification.modules {
        for (name, ty) in &spec.types {
            insert_definition(
                output,
                format!("{package}:{module}#{name}"),
                v4_specification(&ty.value),
            )?;
        }
    }
    Ok(())
}

pub(super) fn collect_v3_definitions(
    output: &mut HashMap<String, Definition>,
    package: &str,
    definition: &classic::PackageDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
) -> Result<(), DataValueError> {
    for entry in &definition.modules {
        let module = classic_path(&entry.path);
        for (name, controlled) in &entry.definition.value.types {
            insert_definition(
                output,
                format!("{package}:{module}#{name}"),
                v3_definition(&controlled.value.value),
            )?;
        }
    }
    Ok(())
}

pub(super) fn collect_v3_specifications(
    output: &mut HashMap<String, Definition>,
    package: &str,
    specification: &classic::PackageSpecification<classic::Attrs>,
) -> Result<(), DataValueError> {
    for entry in &specification.modules {
        let module = classic_path(&entry.path);
        for (name, ty) in &entry.specification.types {
            insert_definition(
                output,
                format!("{package}:{module}#{name}"),
                v3_specification(&ty.value),
            )?;
        }
    }
    Ok(())
}
