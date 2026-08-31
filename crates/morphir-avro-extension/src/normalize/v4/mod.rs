use std::collections::BTreeMap;

use morphir_core::ir::v4;

use crate::model::{
    DistributionKind, EntryPointMetadata, NamedType, ProjectionDependency, ProjectionModule,
    ProjectionPackage, TypeExpr, ValueSpecification,
};

use super::NormalizeError;

mod entry_points;
mod types;

use entry_points::validate_entry_points;
use types::{normalize_type, normalize_type_definition, normalize_type_specification};

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

fn canonical_path(path: &str) -> Vec<String> {
    path.split('/').map(ToOwned::to_owned).collect()
}

fn documentation(doc: Option<&v4::Documentation>) -> Option<String> {
    doc.map(|doc| doc.lines().join("\n"))
}
