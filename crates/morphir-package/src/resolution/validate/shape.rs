use super::super::model::*;
use super::{LocatedCatalog, LocatedRecord, LocatedTarget, Mode, OuterInput, normalize};
use crate::resolution::wire::join_pointer;
use serde_json::{Map, Value};

pub(crate) fn outer_shape(value: &Value) -> Result<OuterInput, Vec<Violation>> {
    let mut violations = Vec::new();
    let Some(object) = expect_object(value, "", &mut violations) else {
        return Err(normalize(violations));
    };

    validate_literal(
        object,
        "formatVersion",
        "",
        "0.1.0-draft.2",
        &mut violations,
    );
    validate_literal(object, "capability", "", "flat-library", &mut violations);
    let mode = parse_mode(object, &mut violations);
    let root = object
        .get("root")
        .map(|value| release_record(value, "/root", &mut violations))
        .unwrap_or_else(|| {
            missing("/root", &mut violations);
            None
        });

    let mut catalogs = Vec::new();
    let mut releases = Vec::new();
    let mut targets = Vec::new();
    let allowed: &[&str] = match mode {
        Some(Mode::Initial) => {
            catalogs = parse_catalogs(object, &mut violations);
            &["formatVersion", "capability", "root", "mode", "catalogs"]
        }
        Some(Mode::Update) => {
            catalogs = parse_catalogs(object, &mut violations);
            targets = parse_targets(object, &mut violations);
            &[
                "formatVersion",
                "capability",
                "root",
                "mode",
                "catalogs",
                "lock",
                "targets",
            ]
        }
        Some(Mode::Replay) => {
            releases = parse_releases(object, &mut violations);
            &[
                "formatVersion",
                "capability",
                "root",
                "mode",
                "releases",
                "lock",
            ]
        }
        None => &[
            "formatVersion",
            "capability",
            "root",
            "mode",
            "catalogs",
            "releases",
            "lock",
            "targets",
        ],
    };
    unknown_fields(object, "", allowed, &mut violations);

    if !violations.is_empty() {
        return Err(normalize(violations));
    }
    Ok(OuterInput {
        mode: mode.expect("valid shape has a recognized mode"),
        root: root.expect("valid shape has a root"),
        catalogs,
        releases,
        targets,
        raw_lock: object.get("lock").cloned(),
    })
}

fn parse_mode(object: &Map<String, Value>, violations: &mut Vec<Violation>) -> Option<Mode> {
    let Some(value) = object.get("mode") else {
        missing("/mode", violations);
        return None;
    };
    let Some(value) = value.as_str() else {
        violation("/mode", ViolationRule::InvalidType, violations);
        return None;
    };
    match value {
        "initial" => Some(Mode::Initial),
        "update" => Some(Mode::Update),
        "replay" => Some(Mode::Replay),
        _ => {
            violation("/mode", ViolationRule::InvalidValue, violations);
            None
        }
    }
}

fn parse_catalogs(
    object: &Map<String, Value>,
    violations: &mut Vec<Violation>,
) -> Vec<LocatedCatalog> {
    let Some(value) = object.get("catalogs") else {
        missing("/catalogs", violations);
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        violation("/catalogs", ViolationRule::InvalidType, violations);
        return Vec::new();
    };
    array
        .iter()
        .enumerate()
        .filter_map(|(index, value)| catalog(value, &format!("/catalogs/{index}"), violations))
        .collect()
}

fn parse_releases(
    object: &Map<String, Value>,
    violations: &mut Vec<Violation>,
) -> Vec<LocatedRecord> {
    let Some(value) = object.get("releases") else {
        missing("/releases", violations);
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        violation("/releases", ViolationRule::InvalidType, violations);
        return Vec::new();
    };
    array
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            release_record(value, &format!("/releases/{index}"), violations)
        })
        .collect()
}

fn parse_targets(
    object: &Map<String, Value>,
    violations: &mut Vec<Violation>,
) -> Vec<LocatedTarget> {
    let Some(value) = object.get("targets") else {
        missing("/targets", violations);
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        violation("/targets", ViolationRule::InvalidType, violations);
        return Vec::new();
    };
    if array.is_empty() {
        violation("/targets", ViolationRule::InvalidValue, violations);
    }
    array
        .iter()
        .enumerate()
        .filter_map(|(index, value)| target(value, &format!("/targets/{index}"), violations))
        .collect()
}

fn catalog(
    value: &Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<LocatedCatalog> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(object, pointer, &["packagePath", "releases"], violations);
    let package_path = package_path_field(object, "packagePath", pointer, violations);
    let records = match object.get("releases") {
        None => {
            missing(&format!("{pointer}/releases"), violations);
            Vec::new()
        }
        Some(value) => match value.as_array() {
            None => {
                violation(
                    format!("{pointer}/releases"),
                    ViolationRule::InvalidType,
                    violations,
                );
                Vec::new()
            }
            Some(values) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    release_record(value, &format!("{pointer}/releases/{index}"), violations)
                })
                .collect(),
        },
    };
    package_path.map(|package_path| LocatedCatalog {
        value: Catalog {
            package_path,
            releases: records.iter().map(|record| record.value.clone()).collect(),
        },
        records,
        pointer: pointer.to_owned(),
        suppressed: false,
    })
}

fn release_record(
    value: &Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<LocatedRecord> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(
        object,
        pointer,
        &[
            "release",
            "irPackageName",
            "manifestDigest",
            "contentDigest",
            "dependencies",
        ],
        violations,
    );
    let release = object
        .get("release")
        .map(|value| release_id(value, &format!("{pointer}/release"), violations))
        .unwrap_or_else(|| {
            missing(&format!("{pointer}/release"), violations);
            None
        });
    let ir_package_name = ir_name_field(object, "irPackageName", pointer, violations);
    let manifest_digest = digest_field(object, "manifestDigest", pointer, violations);
    let content_digest = digest_field(object, "contentDigest", pointer, violations);
    let dependencies = match object.get("dependencies") {
        None => {
            missing(&format!("{pointer}/dependencies"), violations);
            Vec::new()
        }
        Some(value) => match value.as_array() {
            None => {
                violation(
                    format!("{pointer}/dependencies"),
                    ViolationRule::InvalidType,
                    violations,
                );
                Vec::new()
            }
            Some(values) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    requirement(
                        value,
                        &format!("{pointer}/dependencies/{index}"),
                        violations,
                    )
                })
                .collect(),
        },
    };
    Some(LocatedRecord {
        value: ReleaseRecord {
            release: release?,
            ir_package_name: ir_package_name?,
            manifest_digest: manifest_digest?,
            content_digest: content_digest?,
            dependencies,
        },
        pointer: pointer.to_owned(),
        suppressed: false,
    })
}

fn release_id(value: &Value, pointer: &str, violations: &mut Vec<Violation>) -> Option<ReleaseId> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(object, pointer, &["packagePath", "version"], violations);
    let package_path = package_path_field(object, "packagePath", pointer, violations);
    let version = version_field(object, "version", pointer, violations);
    Some(ReleaseId {
        package_path: package_path?,
        version: version?,
    })
}

fn requirement(
    value: &Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<Requirement> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(
        object,
        pointer,
        &["irPackageName", "packagePath", "versionRange"],
        violations,
    );
    let ir_package_name = ir_name_field(object, "irPackageName", pointer, violations);
    let package_path = package_path_field(object, "packagePath", pointer, violations);
    let version_range = object
        .get("versionRange")
        .map(|value| version_range(value, &format!("{pointer}/versionRange"), violations))
        .unwrap_or_else(|| {
            missing(&format!("{pointer}/versionRange"), violations);
            None
        });
    Some(Requirement {
        ir_package_name: ir_package_name?,
        package_path: package_path?,
        version_range: version_range?,
    })
}

fn version_range(
    value: &Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<VersionRange> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(
        object,
        pointer,
        &["minimumInclusive", "maximumExclusive"],
        violations,
    );
    let minimum_inclusive = version_field(object, "minimumInclusive", pointer, violations);
    let maximum_exclusive = version_field(object, "maximumExclusive", pointer, violations);
    Some(VersionRange {
        minimum_inclusive: minimum_inclusive?,
        maximum_exclusive: maximum_exclusive?,
    })
}

fn target(value: &Value, pointer: &str, violations: &mut Vec<Violation>) -> Option<LocatedTarget> {
    let object = expect_object(value, pointer, violations)?;
    let kind = string_field(object, "kind", pointer, violations);
    let package_path = package_path_field(object, "packagePath", pointer, violations);
    let parsed = match kind.as_deref() {
        Some("eligible") => {
            unknown_fields(object, pointer, &["kind", "packagePath"], violations);
            package_path.map(|package_path| UpdateTarget::Eligible { package_path })
        }
        Some("exact") => {
            unknown_fields(
                object,
                pointer,
                &["kind", "packagePath", "version"],
                violations,
            );
            match (
                package_path,
                version_field(object, "version", pointer, violations),
            ) {
                (Some(package_path), Some(version)) => Some(UpdateTarget::Exact {
                    package_path,
                    version,
                }),
                _ => None,
            }
        }
        Some(_) => {
            unknown_fields(
                object,
                pointer,
                &["kind", "packagePath", "version"],
                violations,
            );
            violation(
                format!("{pointer}/kind"),
                ViolationRule::InvalidValue,
                violations,
            );
            None
        }
        None => {
            unknown_fields(
                object,
                pointer,
                &["kind", "packagePath", "version"],
                violations,
            );
            None
        }
    };
    parsed.map(|value| LocatedTarget {
        value,
        pointer: pointer.to_owned(),
    })
}

fn validate_literal(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    expected: &str,
    violations: &mut Vec<Violation>,
) {
    let pointer = join_pointer(parent, field);
    let Some(value) = object.get(field) else {
        missing(&pointer, violations);
        return;
    };
    let Some(value) = value.as_str() else {
        violation(pointer, ViolationRule::InvalidType, violations);
        return;
    };
    if value != expected {
        violation(pointer, ViolationRule::InvalidValue, violations);
    }
}

fn package_path_field(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    violations: &mut Vec<Violation>,
) -> Option<PackagePath> {
    validated_string_field(object, field, parent, PackagePath::parse, violations)
}

fn ir_name_field(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    violations: &mut Vec<Violation>,
) -> Option<IrPackageName> {
    validated_string_field(object, field, parent, IrPackageName::parse, violations)
}

fn version_field(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    violations: &mut Vec<Violation>,
) -> Option<StableVersion> {
    validated_string_field(object, field, parent, StableVersion::parse, violations)
}

fn digest_field(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    violations: &mut Vec<Violation>,
) -> Option<ResolutionDigest> {
    validated_string_field(object, field, parent, ResolutionDigest::parse, violations)
}

fn validated_string_field<T>(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    parse: impl FnOnce(&str) -> Result<T, ResolutionValueError>,
    violations: &mut Vec<Violation>,
) -> Option<T> {
    let pointer = join_pointer(parent, field);
    let value = string_field(object, field, parent, violations)?;
    match parse(&value) {
        Ok(value) => Some(value),
        Err(error) => {
            violation(pointer, value_rule(error), violations);
            None
        }
    }
}

fn value_rule(error: ResolutionValueError) -> ViolationRule {
    match error {
        ResolutionValueError::PackagePath | ResolutionValueError::IrPackageName => {
            ViolationRule::InvalidName
        }
        ResolutionValueError::StableVersion => ViolationRule::InvalidVersion,
        ResolutionValueError::ResolutionDigest => ViolationRule::InvalidDigest,
    }
}

fn string_field(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    violations: &mut Vec<Violation>,
) -> Option<String> {
    let pointer = join_pointer(parent, field);
    let Some(value) = object.get(field) else {
        missing(&pointer, violations);
        return None;
    };
    let Some(value) = value.as_str() else {
        violation(pointer, ViolationRule::InvalidType, violations);
        return None;
    };
    Some(value.to_owned())
}

fn expect_object<'a>(
    value: &'a Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<&'a Map<String, Value>> {
    match value.as_object() {
        Some(object) => Some(object),
        None => {
            violation(pointer, ViolationRule::InvalidType, violations);
            None
        }
    }
}

fn unknown_fields(
    object: &Map<String, Value>,
    pointer: &str,
    allowed: &[&str],
    violations: &mut Vec<Violation>,
) {
    for key in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
        violation(
            join_pointer(pointer, key),
            ViolationRule::UnknownField,
            violations,
        );
    }
}

fn missing(pointer: &str, violations: &mut Vec<Violation>) {
    violation(pointer, ViolationRule::MissingField, violations);
}

fn violation(pointer: impl Into<String>, rule: ViolationRule, violations: &mut Vec<Violation>) {
    violations.push(Violation {
        pointer: pointer.into(),
        rule,
    });
}
