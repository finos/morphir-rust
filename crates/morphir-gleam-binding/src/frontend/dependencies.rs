//! Validation and conversion of frontend compile dependencies.

use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Distribution, FormatVersion, IRFile, PackageName, PackageSpecification,
};
use morphir_extension_sdk::CompileDependency;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DependencyError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) fn package_specifications(
    dependencies: &[CompileDependency],
    _supported_ir_version: &str,
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
            if crate::version::IrVersion::parse(&dependency.ir_version).is_err() {
                return Err(vec![DependencyError {
                    code: "UNSUPPORTED_DEPENDENCY_IR_VERSION",
                    message: format!(
                        "Dependency '{}' uses unsupported Morphir IR version '{}'; supported releases are 3 and 4",
                        dependency.package_name, dependency.ir_version
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
                && !matches_supported_format_version(&format_version, &dependency.ir_version)
            {
                return Err(vec![DependencyError {
                    code: "DEPENDENCY_IR_VERSION_MISMATCH",
                    message: format!(
                        "Dependency '{}' embeds Morphir IR format version {}; expected '{}'",
                        dependency.package_name,
                        display_format_version(&format_version),
                        dependency.ir_version
                    ),
                }]);
            }
            // A dependency is used through its public face, which a `Specs` states outright and a
            // `Library` or `Application` carries inside its definitions.
            let (distribution_package_name, specification) = match parsed.distribution {
                Distribution::Specs(content) => (content.package_name, content.spec),
                Distribution::Library(content) => {
                    (content.package_name, content.def.to_specification())
                }
                Distribution::Application(content) => {
                    (content.package_name, content.def.to_specification())
                }
            };
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
    if value == "morphir/SDK" {
        return Ok(PackageName::parse(value));
    }
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
    if crate::version::IrVersion::parse(&dependency.ir_version) == Ok(crate::version::IrVersion::V3)
    {
        let root = if dependency.distribution.is_array() {
            serde_json::json!({"formatVersion": 3, "distribution": dependency.distribution})
        } else {
            dependency.distribution.clone()
        };
        if let Some(written) = root.get("formatVersion") {
            let version =
                serde_json::from_value::<FormatVersion>(written.clone()).map_err(|e| {
                    vec![DependencyError {
                        code: "INVALID_DEPENDENCY_DISTRIBUTION",
                        message: e.to_string(),
                    }]
                })?;
            if !matches_supported_format_version(&version, &dependency.ir_version) {
                return Err(vec![DependencyError {
                    code: "DEPENDENCY_IR_VERSION_MISMATCH",
                    message: format!(
                        "Dependency '{}' embeds another IR release",
                        dependency.package_name
                    ),
                }]);
            }
        }
        let parsed = crate::version::read_ir(&root).map_err(|message| {
            vec![DependencyError {
                code: "INVALID_DEPENDENCY_DISTRIBUTION",
                message,
            }]
        })?;
        return Ok(ParsedDistribution {
            format_version: Some(FormatVersion::Integer(3)),
            package_name: parsed.distribution.package_name().to_string(),
            distribution: parsed.distribution,
        });
    }
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

fn matches_supported_format_version(
    format_version: &FormatVersion,
    supported_ir_version: &str,
) -> bool {
    let Ok(normalized) = format_version.normalize() else {
        return false;
    };
    let Ok(expected) = crate::version::IrVersion::parse(supported_ir_version) else {
        return false;
    };
    normalized.release.to_exact_string() == expected.release()
}

fn display_format_version(version: &FormatVersion) -> String {
    match version {
        FormatVersion::String(version) => format!("'{version}'"),
        FormatVersion::Integer(version) => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_package_name;

    #[test]
    fn explicit_sdk_dependency_uses_well_known_ir_identity() {
        assert_eq!(
            canonical_package_name("morphir/SDK").unwrap().to_string(),
            "morphir/SDK"
        );
        assert!(canonical_package_name("example/Upper").is_err());
    }
}
