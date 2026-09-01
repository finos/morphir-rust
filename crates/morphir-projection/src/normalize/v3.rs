use morphir_core::ir::classic;

use crate::model::{
    Constructor, DistributionKind, NamedType, ProjectionDependency, ProjectionModule,
    ProjectionPackage, TypeDeclaration, TypeExpr, ValueSpecification,
};

pub(super) fn normalize(ir: classic::Distribution) -> ProjectionPackage {
    let classic::DistributionBody::Library(package_path, dependencies, package) = ir.distribution;
    let package_name = canonical_path(&package_path);
    let dependencies = normalize_dependencies(dependencies);
    let mut modules = package
        .modules
        .into_iter()
        .filter(|entry| matches!(entry.definition.access, classic::Access::Public))
        .map(|entry| normalize_module(&package_name, entry.path, entry.definition.value))
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.path.cmp(&right.path));

    ProjectionPackage {
        kind: DistributionKind::Library,
        package_name,
        dependencies,
        modules,
    }
}

fn normalize_module(
    package_name: &str,
    path: classic::Path,
    module: classic::ModuleDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
) -> ProjectionModule {
    let path = path.segments.iter().map(canonical_name).collect::<Vec<_>>();
    let mut types = module
        .types
        .into_iter()
        .filter(|(_, controlled)| matches!(controlled.access, classic::Access::Public))
        .map(|(name, controlled)| {
            normalize_type_declaration(
                package_name,
                &path,
                canonical_name(&name),
                controlled.value.doc,
                controlled.value.value,
            )
        })
        .collect::<Vec<_>>();
    types.sort_by(|left, right| left.source_name().cmp(right.source_name()));

    let mut values = module
        .values
        .into_iter()
        .filter(|(_, controlled)| matches!(controlled.access, classic::Access::Public))
        .map(|(name, controlled)| {
            let name = canonical_name(&name);
            let definition = controlled.value.value;
            let inputs = definition
                .input_types
                .into_iter()
                .map(|argument| NamedType {
                    name: canonical_name(&argument.name),
                    tpe: normalize_type(argument.ty),
                })
                .collect();
            let (inputs, output, value_kind) =
                super::normalize_signature(inputs, Some(normalize_type(definition.output_type)));
            ValueSpecification {
                source_name: super::canonical_fq_name(package_name, &path, &name),
                name,
                inputs,
                output,
                value_kind,
                entry_point: None,
                doc: Some(controlled.value.doc),
            }
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.source_name.cmp(&right.source_name));

    ProjectionModule {
        path,
        types,
        values,
        doc: module.doc,
    }
}

fn normalize_dependencies(
    dependencies: Vec<(classic::Path, classic::PackageSpecification<classic::Attrs>)>,
) -> Vec<ProjectionDependency> {
    let mut dependencies = dependencies
        .into_iter()
        .map(|(package_path, specification)| {
            let package_name = canonical_path(&package_path);
            ProjectionDependency {
                modules: normalize_specification_modules(&package_name, specification),
                package_name,
            }
        })
        .collect::<Vec<_>>();
    dependencies.sort_by(|left, right| left.package_name.cmp(&right.package_name));
    dependencies
}

fn normalize_specification_modules(
    package_name: &str,
    specification: classic::PackageSpecification<classic::Attrs>,
) -> Vec<ProjectionModule> {
    let mut modules = specification
        .modules
        .into_iter()
        .map(|entry| {
            let path = entry
                .path
                .segments
                .iter()
                .map(canonical_name)
                .collect::<Vec<_>>();
            let mut types = entry
                .specification
                .types
                .into_iter()
                .map(|(name, documented)| {
                    normalize_type_specification(
                        package_name,
                        &path,
                        canonical_name(&name),
                        documented.doc,
                        documented.value,
                    )
                })
                .collect::<Vec<_>>();
            types.sort_by(|left, right| left.source_name().cmp(right.source_name()));
            let mut values = entry
                .specification
                .values
                .into_iter()
                .map(|(name, documented)| {
                    let name = canonical_name(&name);
                    let inputs = documented
                        .value
                        .inputs
                        .into_iter()
                        .map(|input| NamedType {
                            name: canonical_name(&input.name),
                            tpe: normalize_type(input.ty),
                        })
                        .collect();
                    let (inputs, output, value_kind) = super::normalize_signature(
                        inputs,
                        Some(normalize_type(documented.value.output)),
                    );
                    ValueSpecification {
                        source_name: super::canonical_fq_name(package_name, &path, &name),
                        name,
                        inputs,
                        output,
                        value_kind,
                        entry_point: None,
                        doc: Some(documented.doc),
                    }
                })
                .collect::<Vec<_>>();
            values.sort_by(|left, right| left.source_name.cmp(&right.source_name));
            ProjectionModule {
                path,
                types,
                values,
                doc: entry.specification.doc,
            }
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.path.cmp(&right.path));
    modules
}

fn normalize_type_specification(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: String,
    specification: classic::TypeSpecification<classic::Attrs>,
) -> TypeDeclaration {
    let source_name = super::canonical_fq_name(package_name, module_path, &name);
    match specification {
        classic::TypeSpecification::Alias(type_params, value) => TypeDeclaration::Alias {
            source_name,
            name,
            type_params: type_params.iter().map(canonical_name).collect(),
            value: normalize_type(value),
            doc: Some(doc),
        },
        classic::TypeSpecification::Opaque(type_params) => TypeDeclaration::Opaque {
            source_name,
            name,
            type_params: type_params.iter().map(canonical_name).collect(),
            doc: Some(doc),
        },
        classic::TypeSpecification::Custom(type_params, constructors) => {
            let mut constructors = constructors
                .into_iter()
                .map(|constructor| normalize_constructor(package_name, module_path, constructor))
                .collect::<Vec<_>>();
            constructors.sort_by(|left, right| left.source_name.cmp(&right.source_name));
            TypeDeclaration::Custom {
                source_name,
                name,
                type_params: type_params.iter().map(canonical_name).collect(),
                constructors,
                doc: Some(doc),
            }
        }
    }
}

fn normalize_type_declaration(
    package_name: &str,
    module_path: &[String],
    name: String,
    doc: String,
    definition: classic::TypeDefinition<classic::Attrs>,
) -> TypeDeclaration {
    let source_name = super::canonical_fq_name(package_name, module_path, &name);
    match definition {
        classic::TypeDefinition::Alias(type_params, value) => TypeDeclaration::Alias {
            source_name,
            name,
            type_params: type_params.iter().map(canonical_name).collect(),
            value: normalize_type(value),
            doc: Some(doc),
        },
        classic::TypeDefinition::Custom(type_params, constructors) => {
            let type_params = type_params.iter().map(canonical_name).collect();
            if matches!(constructors.access, classic::Access::Private) {
                TypeDeclaration::Opaque {
                    source_name,
                    name,
                    type_params,
                    doc: Some(doc),
                }
            } else {
                let mut constructors = constructors
                    .value
                    .into_iter()
                    .map(|constructor| {
                        normalize_constructor(package_name, module_path, constructor)
                    })
                    .collect::<Vec<_>>();
                constructors.sort_by(|left, right| left.source_name.cmp(&right.source_name));
                TypeDeclaration::Custom {
                    source_name,
                    name,
                    type_params,
                    constructors,
                    doc: Some(doc),
                }
            }
        }
    }
}

fn normalize_constructor(
    package_name: &str,
    module_path: &[String],
    constructor: classic::Constructor<classic::Attrs>,
) -> Constructor {
    let name = canonical_name(&constructor.name);
    Constructor {
        source_name: super::canonical_fq_name(package_name, module_path, &name),
        name,
        arguments: constructor
            .args
            .into_iter()
            .map(|(name, tpe)| NamedType {
                name: canonical_name(&name),
                tpe: normalize_type(tpe),
            })
            .collect(),
    }
}

fn normalize_type(tpe: classic::Type<classic::Attrs>) -> TypeExpr {
    match tpe {
        classic::Type::Variable(_, name) => TypeExpr::Variable(canonical_name(&name)),
        classic::Type::Reference(_, name, arguments) => TypeExpr::Reference {
            source_name: canonical_fq_name(&name),
            arguments: arguments.into_iter().map(normalize_type).collect(),
        },
        classic::Type::Tuple(_, elements) => {
            TypeExpr::Tuple(elements.into_iter().map(normalize_type).collect())
        }
        classic::Type::Record(_, fields) => TypeExpr::Record(normalize_fields(fields)),
        classic::Type::ExtensibleRecord(_, variable, fields) => TypeExpr::ExtensibleRecord {
            variable: canonical_name(&variable),
            fields: normalize_fields(fields),
        },
        classic::Type::Function(_, input, output) => TypeExpr::Function {
            input: Box::new(normalize_type(*input)),
            output: Box::new(normalize_type(*output)),
        },
        classic::Type::Unit(_) => TypeExpr::Unit,
    }
}

fn normalize_fields(fields: Vec<classic::Field<classic::Attrs>>) -> Vec<NamedType> {
    let mut fields = fields
        .into_iter()
        .map(|field| NamedType {
            name: canonical_name(&field.name),
            tpe: normalize_type(field.ty),
        })
        .collect::<Vec<_>>();
    fields.sort_by(|left, right| left.name.cmp(&right.name));
    fields
}

fn canonical_fq_name(name: &classic::FQName) -> String {
    super::canonical_fq_name(
        &canonical_path(&name.package_path),
        &name
            .module_path
            .segments
            .iter()
            .map(canonical_name)
            .collect::<Vec<_>>(),
        &canonical_name(&name.local_name),
    )
}

fn canonical_path(path: &classic::Path) -> String {
    path.segments
        .iter()
        .map(canonical_name)
        .collect::<Vec<_>>()
        .join("/")
}

fn canonical_name(name: &classic::Name) -> String {
    morphir_core::naming::Name::from_words(
        name.words
            .iter()
            .map(|word| morphir_core::resolve(*word).to_owned()),
    )
    .to_canonical_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_initialisms_use_the_canonical_name_constructor() {
        assert_eq!(
            canonical_name(&classic::Name::from_str("APIResponse")),
            "API-response"
        );
    }
}
