//! Validation and conversion of frontend compile dependencies.

use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access, ConstructorArgSpec, ConstructorSpecification, Distribution, FormatVersion, IRFile,
    ModuleDefinition, ModuleSpecification, PackageDefinition, PackageName, PackageSpecification,
    TypeDefinition, TypeSpecification, ValueDefinition, ValueSpecification,
};
use morphir_extension_sdk::CompileDependency;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DependencyError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) fn package_specifications(
    dependencies: &[CompileDependency],
    supported_ir_version: &str,
) -> Result<IndexMap<String, PackageSpecification>, Vec<DependencyError>> {
    dependencies
        .iter()
        .try_fold(IndexMap::new(), |mut resolved, dependency| {
            if let Err(message) = canonical_package_name(&dependency.package_name) {
                return Err(vec![DependencyError {
                    code: "INVALID_DEPENDENCY_PACKAGE_NAME",
                    message,
                }]);
            }
            if resolved.contains_key(&dependency.package_name) {
                return Err(vec![DependencyError {
                    code: "DUPLICATE_DEPENDENCY",
                    message: format!(
                        "Dependency '{}' was supplied more than once",
                        dependency.package_name
                    ),
                }]);
            }
            if dependency.ir_version != supported_ir_version {
                return Err(vec![DependencyError {
                    code: "UNSUPPORTED_DEPENDENCY_IR_VERSION",
                    message: format!(
                        "Dependency '{}' uses Morphir IR version '{}'; expected '{}'",
                        dependency.package_name, dependency.ir_version, supported_ir_version
                    ),
                }]);
            }
            let parsed = parse_distribution(dependency)?;
            if parsed.package_name != dependency.package_name {
                return Err(vec![DependencyError {
                    code: "DEPENDENCY_PACKAGE_MISMATCH",
                    message: format!(
                        "Dependency '{}' contains package '{}'",
                        dependency.package_name, parsed.package_name
                    ),
                }]);
            }
            if let Some(format_version) = parsed.format_version
                && format_version != FormatVersion::String(supported_ir_version.into())
            {
                return Err(vec![DependencyError {
                    code: "DEPENDENCY_IR_VERSION_MISMATCH",
                    message: format!(
                        "Dependency '{}' embeds Morphir IR format version {}; expected '{}'",
                        dependency.package_name,
                        display_format_version(&format_version),
                        supported_ir_version
                    ),
                }]);
            }
            let (distribution_package_name, specification) = match parsed.distribution {
                Distribution::Specs(content) => Ok((content.package_name, content.spec)),
                Distribution::Library(content) => definition_to_specification(content.def)
                    .map(|specification| (content.package_name, specification)),
                Distribution::Application(content) => definition_to_specification(content.def)
                    .map(|specification| (content.package_name, specification)),
            }
            .map_err(|message| {
                vec![DependencyError {
                    code: "INCOMPATIBLE_DEPENDENCY_DISTRIBUTION",
                    message: format!("Dependency '{}': {message}", dependency.package_name),
                }]
            })?;
            if distribution_package_name.to_string() != dependency.package_name {
                return Err(vec![DependencyError {
                    code: "DEPENDENCY_PACKAGE_MISMATCH",
                    message: format!(
                        "Dependency '{}' contains package '{}'",
                        dependency.package_name, distribution_package_name
                    ),
                }]);
            }
            resolved.insert(dependency.package_name.clone(), specification);
            Ok(resolved)
        })
}

pub(crate) fn canonical_package_name(value: &str) -> Result<PackageName, String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.as_bytes().get(1) == Some(&b':')
    {
        return Err(format!("Invalid Morphir package name '{value}'"));
    }
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || *segment == "."
            || *segment == ".."
            || !segment
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }) {
        return Err(format!("Invalid Morphir package name '{value}'"));
    }
    let package_name = PackageName::parse(value);
    if package_name.to_string() != value {
        return Err(format!("Package name '{value}' is not canonical"));
    }
    Ok(package_name)
}

struct ParsedDistribution {
    format_version: Option<FormatVersion>,
    package_name: String,
    distribution: Distribution,
}

fn parse_distribution(
    dependency: &CompileDependency,
) -> Result<ParsedDistribution, Vec<DependencyError>> {
    let package_name = serialized_package_name(&dependency.distribution).ok_or_else(|| {
        vec![DependencyError {
            code: "INVALID_DEPENDENCY_DISTRIBUTION",
            message: format!(
                "Dependency '{}' does not contain a V4 packageName",
                dependency.package_name
            ),
        }]
    })?;
    let parsed = if dependency
        .distribution
        .as_object()
        .is_some_and(|object| object.contains_key("distribution"))
    {
        serde_json::from_value::<IRFile>(dependency.distribution.clone()).map(|file| {
            ParsedDistribution {
                format_version: Some(file.format_version),
                package_name: package_name.clone(),
                distribution: file.distribution,
            }
        })
    } else {
        serde_json::from_value::<Distribution>(dependency.distribution.clone()).map(
            |distribution| ParsedDistribution {
                format_version: None,
                package_name,
                distribution,
            },
        )
    };
    parsed.map_err(|error| {
        vec![DependencyError {
            code: "INVALID_DEPENDENCY_DISTRIBUTION",
            message: format!(
                "Dependency '{}' is not a valid V4 distribution: {error}",
                dependency.package_name
            ),
        }]
    })
}

fn serialized_package_name(value: &serde_json::Value) -> Option<String> {
    let distribution = value.get("distribution").unwrap_or(value).as_object()?;
    let content = ["Library", "Specs", "Application"]
        .into_iter()
        .find_map(|variant| distribution.get(variant))?;
    content.get("packageName")?.as_str().map(str::to_owned)
}

fn display_format_version(version: &FormatVersion) -> String {
    match version {
        FormatVersion::String(version) => format!("'{version}'"),
        FormatVersion::Integer(version) => version.to_string(),
    }
}

fn definition_to_specification(
    definition: PackageDefinition,
) -> Result<PackageSpecification, String> {
    definition
        .modules
        .into_iter()
        .try_fold(IndexMap::new(), |mut modules, (name, controlled)| {
            if controlled.access == Access::Public {
                modules.insert(
                    name.clone(),
                    module_to_specification(controlled.value, &name)?,
                );
            }
            Ok(modules)
        })
        .map(|modules| PackageSpecification { modules })
}

fn module_to_specification(
    definition: ModuleDefinition,
    module_name: &str,
) -> Result<ModuleSpecification, String> {
    let types = definition.types.into_iter().try_fold(
        IndexMap::new(),
        |mut types, (name, controlled)| {
            if controlled.access == Access::Public {
                types.insert(
                    name.clone(),
                    type_to_specification(controlled.value).map_err(|message| {
                        format!("module '{module_name}', type '{name}': {message}")
                    })?,
                );
            }
            Ok::<_, String>(types)
        },
    )?;
    Ok(ModuleSpecification {
        types,
        values: definition
            .values
            .into_iter()
            .filter_map(|(name, controlled)| {
                (controlled.access == Access::Public)
                    .then(|| (name, value_to_specification(controlled.value)))
            })
            .collect(),
        doc: definition.doc,
    })
}

fn value_to_specification(definition: ValueDefinition) -> ValueSpecification {
    ValueSpecification {
        inputs: definition
            .input_types
            .into_iter()
            .map(|(name, input)| (name, input.input_type))
            .collect(),
        output: definition.output_type,
    }
}

fn type_to_specification(definition: TypeDefinition) -> Result<TypeSpecification, String> {
    match definition {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => Ok(TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
        }),
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => Ok(match constructors.access {
            Access::Public => TypeSpecification::CustomTypeSpecification {
                type_params,
                constructors: constructors
                    .value
                    .into_iter()
                    .map(|constructor| ConstructorSpecification {
                        name: constructor.name,
                        args: constructor
                            .args
                            .into_iter()
                            .map(|argument| ConstructorArgSpec {
                                name: argument.name,
                                arg_type: argument.arg_type,
                            })
                            .collect(),
                    })
                    .collect(),
            },
            Access::Private => TypeSpecification::OpaqueTypeSpecification { type_params },
        }),
        TypeDefinition::IncompleteTypeDefinition { .. } => {
            Err("incomplete type definitions cannot be dependency specifications".into())
        }
    }
}
