use std::collections::BTreeMap;

use morphir_core::ir::v4;

use crate::model::{
    Constructor, DistributionKind, EntryPointKind, EntryPointMetadata, IncompletenessKind,
    NamedType, ProjectionDependency, ProjectionModule, ProjectionPackage, TypeDeclaration,
    TypeExpr, ValueSpecification,
};

use super::NormalizeError;

pub(super) fn normalize(ir: v4::IRFile) -> Result<ProjectionPackage, NormalizeError> {
    match ir.distribution {
        v4::Distribution::Library(content) => {
            let dependencies = normalize_dependencies(content.dependencies);
            Ok(normalize_definition_package(
                DistributionKind::Library,
                content.package_name.to_string(),
                dependencies,
                content.def,
                &BTreeMap::new(),
            ))
        }
        v4::Distribution::Specs(content) => {
            let dependencies = normalize_dependencies(content.dependencies);
            Ok(normalize_specification_package(
                content.package_name.to_string(),
                dependencies,
                content.spec,
            ))
        }
        v4::Distribution::Application(content) => {
            let entry_points = validate_entry_points(
                &content.package_name.to_string(),
                &content.def,
                content.entry_points,
            )?;
            let dependencies = normalize_dependencies(content.dependencies);
            Ok(normalize_definition_package(
                DistributionKind::Application,
                content.package_name.to_string(),
                dependencies,
                content.def,
                &entry_points,
            ))
        }
    }
}

fn validate_entry_points(
    package_name: &str,
    package: &v4::PackageDefinition,
    entry_points: v4::EntryPoints,
) -> Result<BTreeMap<String, EntryPointMetadata>, NormalizeError> {
    let mut entries = entry_points.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut by_target: BTreeMap<String, EntryPointMetadata> = BTreeMap::new();

    for (identifier, entry) in entries {
        let target =
            morphir_core::naming::FQName::from_canonical_string(&entry.target).map_err(|_| {
                NormalizeError::InvalidEntryPointTarget {
                    identifier: identifier.clone(),
                    target: entry.target.clone(),
                    reason: "invalid",
                }
            })?;
        if target.to_canonical_string() != entry.target {
            return Err(NormalizeError::InvalidEntryPointTarget {
                identifier,
                target: entry.target,
                reason: "noncanonical",
            });
        }
        let module_name = target.module_path.to_canonical_string();
        let value_name = target.local_name.to_canonical_string();
        let controlled_module = if target.package_path.to_canonical_string() == package_name {
            package.modules.get(&module_name)
        } else {
            None
        }
        .ok_or_else(|| NormalizeError::InvalidEntryPointTarget {
            identifier: identifier.clone(),
            target: entry.target.clone(),
            reason: "dangling",
        })?;
        if matches!(controlled_module.access, v4::Access::Private) {
            return Err(NormalizeError::InvalidEntryPointTarget {
                identifier,
                target: entry.target,
                reason: "private",
            });
        }
        let controlled_value =
            controlled_module
                .value
                .values
                .get(&value_name)
                .ok_or_else(|| NormalizeError::InvalidEntryPointTarget {
                    identifier: identifier.clone(),
                    target: entry.target.clone(),
                    reason: "dangling",
                })?;
        if matches!(controlled_value.access, v4::Access::Private) {
            return Err(NormalizeError::InvalidEntryPointTarget {
                identifier,
                target: entry.target,
                reason: "private",
            });
        }

        let metadata = EntryPointMetadata {
            identifier: identifier.clone(),
            kind: normalize_entry_point_kind(entry.kind),
            doc: entry.doc,
        };
        if let Some(existing) = by_target.get(&entry.target) {
            return Err(NormalizeError::DuplicateEntryPointTarget {
                target: entry.target,
                identifiers: vec![existing.identifier.clone(), identifier],
            });
        }
        by_target.insert(entry.target, metadata);
    }

    Ok(by_target)
}

fn normalize_definition_package(
    kind: DistributionKind,
    package_name: String,
    dependencies: Vec<ProjectionDependency>,
    package: v4::PackageDefinition,
    entry_points: &BTreeMap<String, EntryPointMetadata>,
) -> ProjectionPackage {
    let mut modules = package
        .modules
        .into_iter()
        .filter(|(_, controlled)| matches!(controlled.access, v4::Access::Public))
        .map(|(path, controlled)| {
            normalize_definition_module(&package_name, path, controlled.value, entry_points)
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.path.cmp(&right.path));
    ProjectionPackage {
        kind,
        package_name,
        dependencies,
        modules,
    }
}

fn normalize_definition_module(
    package_name: &str,
    path: String,
    module: v4::ModuleDefinition,
    entry_points: &BTreeMap<String, EntryPointMetadata>,
) -> ProjectionModule {
    let path = canonical_path(&path);
    let mut types = module
        .types
        .into_iter()
        .filter(|(_, controlled)| matches!(controlled.access, v4::Access::Public))
        .map(|(name, controlled)| {
            normalize_type_definition(
                package_name,
                &path,
                name,
                documentation(controlled.value.doc.as_ref()),
                controlled.value.value,
            )
        })
        .collect::<Vec<_>>();
    types.sort_by(|left, right| left.source_name().cmp(right.source_name()));

    let mut values = module
        .values
        .into_iter()
        .filter(|(_, controlled)| matches!(controlled.access, v4::Access::Public))
        .map(|(name, controlled)| {
            let definition = controlled.value.value;
            let inputs = definition
                .input_types
                .into_iter()
                .map(|(name, input)| NamedType {
                    name,
                    tpe: normalize_type(input.input_type),
                })
                .collect();
            normalize_value(
                package_name,
                &path,
                name,
                inputs,
                definition.output_type.map(normalize_type),
                documentation(controlled.value.doc.as_ref()),
                entry_points,
            )
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.source_name.cmp(&right.source_name));

    ProjectionModule {
        path,
        types,
        values,
        doc: documentation(module.doc.as_ref()),
    }
}

fn normalize_specification_package(
    package_name: String,
    dependencies: Vec<ProjectionDependency>,
    package: v4::PackageSpecification,
) -> ProjectionPackage {
    let modules = normalize_specification_modules(&package_name, package);
    ProjectionPackage {
        kind: DistributionKind::Specs,
        package_name,
        dependencies,
        modules,
    }
}

fn normalize_dependencies(dependencies: v4::Dependencies) -> Vec<ProjectionDependency> {
    let mut dependencies = dependencies
        .into_iter()
        .map(|(package_name, specification)| ProjectionDependency {
            modules: normalize_specification_modules(&package_name, specification),
            package_name,
        })
        .collect::<Vec<_>>();
    dependencies.sort_by(|left, right| left.package_name.cmp(&right.package_name));
    dependencies
}

fn normalize_specification_modules(
    package_name: &str,
    package: v4::PackageSpecification,
) -> Vec<ProjectionModule> {
    let mut modules = package
        .modules
        .into_iter()
        .map(|(path, module)| normalize_specification_module(package_name, path, module))
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.path.cmp(&right.path));
    modules
}

fn normalize_specification_module(
    package_name: &str,
    path: String,
    module: v4::ModuleSpecification,
) -> ProjectionModule {
    let path = canonical_path(&path);
    let mut types = module
        .types
        .into_iter()
        .map(|(name, documented)| {
            normalize_type_specification(
                package_name,
                &path,
                name,
                documentation(documented.doc.as_ref()),
                documented.value,
            )
        })
        .collect::<Vec<_>>();
    types.sort_by(|left, right| left.source_name().cmp(right.source_name()));

    let mut values = module
        .values
        .into_iter()
        .map(|(name, documented)| {
            let inputs = documented
                .value
                .inputs
                .into_iter()
                .map(|(name, tpe)| NamedType {
                    name,
                    tpe: normalize_type(tpe),
                })
                .collect();
            normalize_value(
                package_name,
                &path,
                name,
                inputs,
                Some(normalize_type(documented.value.output)),
                documentation(documented.doc.as_ref()),
                &BTreeMap::new(),
            )
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.source_name.cmp(&right.source_name));

    ProjectionModule {
        path,
        types,
        values,
        doc: documentation(module.doc.as_ref()),
    }
}

fn normalize_value(
    package_name: &str,
    module_path: &[String],
    name: String,
    inputs: Vec<NamedType>,
    output: Option<TypeExpr>,
    doc: Option<String>,
    entry_points: &BTreeMap<String, EntryPointMetadata>,
) -> ValueSpecification {
    let source_name = super::canonical_fq_name(package_name, module_path, &name);
    let (inputs, output, value_kind) = super::normalize_signature(inputs, output);
    ValueSpecification {
        entry_point: entry_points.get(&source_name).cloned(),
        source_name,
        name,
        inputs,
        output,
        value_kind,
        doc,
    }
}

fn normalize_type_definition(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: Option<String>,
    definition: v4::TypeDefinition,
) -> TypeDeclaration {
    let source_name = super::canonical_fq_name(package_name, module_path, &name);
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

fn normalize_type_specification(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: Option<String>,
    specification: v4::TypeSpecification,
) -> TypeDeclaration {
    let source_name = super::canonical_fq_name(package_name, module_path, &name);
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
                        source_name: super::canonical_fq_name(package_name, module_path, &name),
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
                source_name: super::canonical_fq_name(package_name, module_path, &name),
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

fn normalize_type(tpe: v4::Type) -> TypeExpr {
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

fn normalize_entry_point_kind(kind: v4::EntryPointKind) -> EntryPointKind {
    match kind {
        v4::EntryPointKind::Main => EntryPointKind::Main,
        v4::EntryPointKind::Command => EntryPointKind::Command,
        v4::EntryPointKind::Handler => EntryPointKind::Handler,
    }
}

fn canonical_path(path: &str) -> Vec<String> {
    path.split('/').map(ToOwned::to_owned).collect()
}

fn documentation(doc: Option<&v4::Documentation>) -> Option<String> {
    doc.map(|doc| doc.lines().join("\n"))
}
