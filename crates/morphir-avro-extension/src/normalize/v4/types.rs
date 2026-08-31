use morphir_core::ir::v4;

use crate::model::{Constructor, IncompletenessKind, NamedType, TypeDeclaration, TypeExpr};

pub(super) fn normalize_type_definition(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: Option<String>,
    definition: v4::TypeDefinition,
) -> TypeDeclaration {
    let source_name = super::super::canonical_fq_name(package_name, module_path, &name);
    match definition {
        v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => TypeDeclaration::Alias {
            source_name,
            name,
            type_params: normalize_names(type_params),
            value: normalize_type(type_expr),
            doc,
        },
        v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => {
            let type_params = normalize_names(type_params);
            if matches!(constructors.access, v4::Access::Private) {
                TypeDeclaration::Opaque {
                    source_name,
                    name,
                    type_params,
                    doc,
                }
            } else {
                TypeDeclaration::Custom {
                    source_name,
                    name,
                    type_params,
                    constructors: normalize_constructors(
                        package_name,
                        module_path,
                        constructors.value,
                    ),
                    doc,
                }
            }
        }
        v4::TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
            partial_type_expr,
        } => TypeDeclaration::Incomplete {
            source_name,
            name,
            type_params: normalize_names(type_params),
            incompleteness: normalize_incompleteness(incompleteness),
            partial_type: partial_type_expr.map(normalize_type),
            doc,
        },
    }
}

pub(super) fn normalize_type_specification(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: Option<String>,
    specification: v4::TypeSpecification,
) -> TypeDeclaration {
    let source_name = super::super::canonical_fq_name(package_name, module_path, &name);
    match specification {
        v4::TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
        } => TypeDeclaration::Alias {
            source_name,
            name,
            type_params: normalize_names(type_params),
            value: normalize_type(type_expr),
            doc,
        },
        v4::TypeSpecification::OpaqueTypeSpecification { type_params } => TypeDeclaration::Opaque {
            source_name,
            name,
            type_params: normalize_names(type_params),
            doc,
        },
        v4::TypeSpecification::CustomTypeSpecification {
            type_params,
            constructors,
        } => {
            let mut constructors = constructors
                .into_iter()
                .map(|constructor| {
                    let name = constructor.name.to_canonical_string();
                    Constructor {
                        source_name: super::super::canonical_fq_name(
                            package_name,
                            module_path,
                            &name,
                        ),
                        name,
                        arguments: constructor
                            .args
                            .into_iter()
                            .map(|argument| NamedType {
                                name: argument.name.to_canonical_string(),
                                tpe: normalize_type(argument.arg_type),
                            })
                            .collect(),
                    }
                })
                .collect::<Vec<_>>();
            constructors.sort_by(|left, right| left.source_name.cmp(&right.source_name));
            TypeDeclaration::Custom {
                source_name,
                name,
                type_params: normalize_names(type_params),
                constructors,
                doc,
            }
        }
    }
}

fn normalize_constructors(
    package_name: &str,
    module_path: &[String],
    constructors: Vec<v4::ConstructorDefinition>,
) -> Vec<Constructor> {
    let mut constructors = constructors
        .into_iter()
        .map(|constructor| {
            let name = constructor.name.to_canonical_string();
            Constructor {
                source_name: super::super::canonical_fq_name(package_name, module_path, &name),
                name,
                arguments: constructor
                    .args
                    .into_iter()
                    .map(|argument| NamedType {
                        name: argument.name.to_canonical_string(),
                        tpe: normalize_type(argument.arg_type),
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    constructors.sort_by(|left, right| left.source_name.cmp(&right.source_name));
    constructors
}

pub(super) fn normalize_type(tpe: v4::Type) -> TypeExpr {
    match tpe {
        v4::Type::Variable(_, name) => TypeExpr::Variable(name.to_canonical_string()),
        v4::Type::Reference(_, name, arguments) => TypeExpr::Reference {
            source_name: name.to_canonical_string(),
            arguments: arguments.into_iter().map(normalize_type).collect(),
        },
        v4::Type::Tuple(_, elements) => {
            TypeExpr::Tuple(elements.into_iter().map(normalize_type).collect())
        }
        v4::Type::Record(_, fields) => TypeExpr::Record(normalize_fields(fields)),
        v4::Type::ExtensibleRecord(_, variable, fields) => TypeExpr::ExtensibleRecord {
            variable: variable.to_canonical_string(),
            fields: normalize_fields(fields),
        },
        v4::Type::Function(_, input, output) => TypeExpr::Function {
            input: Box::new(normalize_type(*input)),
            output: Box::new(normalize_type(*output)),
        },
        v4::Type::Unit(_) => TypeExpr::Unit,
    }
}

fn normalize_fields(fields: Vec<v4::Field>) -> Vec<NamedType> {
    let mut fields = fields
        .into_iter()
        .map(|field| NamedType {
            name: field.name.to_canonical_string(),
            tpe: normalize_type(field.tpe),
        })
        .collect::<Vec<_>>();
    fields.sort_by(|left, right| left.name.cmp(&right.name));
    fields
}

fn normalize_names(names: Vec<v4::Name>) -> Vec<String> {
    names
        .into_iter()
        .map(|name| name.to_canonical_string())
        .collect()
}

fn normalize_incompleteness(incompleteness: v4::Incompleteness) -> IncompletenessKind {
    match incompleteness {
        v4::Incompleteness::Draft => IncompletenessKind::Draft,
        v4::Incompleteness::Hole(_) => IncompletenessKind::Hole,
    }
}
