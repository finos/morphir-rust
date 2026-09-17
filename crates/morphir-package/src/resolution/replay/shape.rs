use super::super::model::*;
use super::super::validate::OuterInput;
use super::{LocatedLock, LocatedNode, invalid_lock, violation};
use serde_json::{Map, Value};

pub(super) fn lock_shape(input: &OuterInput) -> Result<LocatedLock, Box<ResolutionDiagnostic>> {
    let Some(value) = input.raw_lock.as_ref() else {
        return Err(Box::new(invalid_lock(vec![violation(
            "/lock",
            ViolationRule::MissingField,
        )])));
    };
    let mut violations = Vec::new();
    let Some(object) = expect_object(value, "/lock", &mut violations) else {
        return Err(Box::new(invalid_lock(violations)));
    };
    unknown_fields(object, "/lock", &["root", "nodes"], &mut violations);
    let root = object
        .get("root")
        .map(|value| release_id(value, "/lock/root", &mut violations))
        .unwrap_or_else(|| {
            violations.push(violation("/lock/root", ViolationRule::MissingField));
            None
        });
    let nodes = match object.get("nodes") {
        None => {
            violations.push(violation("/lock/nodes", ViolationRule::MissingField));
            Vec::new()
        }
        Some(value) => match value.as_array() {
            None => {
                violations.push(violation("/lock/nodes", ViolationRule::InvalidType));
                Vec::new()
            }
            Some(values) => {
                if values.is_empty() {
                    violations.push(violation("/lock/nodes", ViolationRule::InvalidValue));
                }
                values
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| {
                        locked_node(value, &format!("/lock/nodes/{index}"), &mut violations)
                    })
                    .collect()
            }
        },
    };
    if !violations.is_empty() {
        return Err(Box::new(invalid_lock(violations)));
    }
    Ok(LocatedLock {
        value: LockedGraph {
            root: root.expect("valid lock has a root"),
            nodes: nodes.iter().map(|node| node.value.clone()).collect(),
        },
        nodes,
    })
}

fn locked_node(
    value: &Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<LocatedNode> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(
        object,
        pointer,
        &[
            "release",
            "irPackageName",
            "manifestDigest",
            "contentDigest",
            "bindings",
        ],
        violations,
    );
    let release = object
        .get("release")
        .map(|value| release_id(value, &format!("{pointer}/release"), violations))
        .unwrap_or_else(|| {
            violations.push(violation(
                format!("{pointer}/release"),
                ViolationRule::MissingField,
            ));
            None
        });
    let bindings = match object.get("bindings") {
        None => {
            violations.push(violation(
                format!("{pointer}/bindings"),
                ViolationRule::MissingField,
            ));
            Vec::new()
        }
        Some(value) => match value.as_array() {
            None => {
                violations.push(violation(
                    format!("{pointer}/bindings"),
                    ViolationRule::InvalidType,
                ));
                Vec::new()
            }
            Some(values) => values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    binding(value, &format!("{pointer}/bindings/{index}"), violations)
                })
                .collect(),
        },
    };
    let ir_package_name = validated_field(
        object,
        "irPackageName",
        pointer,
        IrPackageName::parse,
        violations,
    );
    let manifest_digest = validated_field(
        object,
        "manifestDigest",
        pointer,
        ResolutionDigest::parse,
        violations,
    );
    let content_digest = validated_field(
        object,
        "contentDigest",
        pointer,
        ResolutionDigest::parse,
        violations,
    );
    Some(LocatedNode {
        value: LockedNode {
            release: release?,
            ir_package_name: ir_package_name?,
            manifest_digest: manifest_digest?,
            content_digest: content_digest?,
            bindings,
        },
        pointer: pointer.to_owned(),
    })
}

fn binding(value: &Value, pointer: &str, violations: &mut Vec<Violation>) -> Option<Binding> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(object, pointer, &["irPackageName", "target"], violations);
    let target = object
        .get("target")
        .map(|value| release_id(value, &format!("{pointer}/target"), violations))
        .unwrap_or_else(|| {
            violations.push(violation(
                format!("{pointer}/target"),
                ViolationRule::MissingField,
            ));
            None
        });
    Some(Binding {
        ir_package_name: validated_field(
            object,
            "irPackageName",
            pointer,
            IrPackageName::parse,
            violations,
        )?,
        target: target?,
    })
}

fn release_id(value: &Value, pointer: &str, violations: &mut Vec<Violation>) -> Option<ReleaseId> {
    let object = expect_object(value, pointer, violations)?;
    unknown_fields(object, pointer, &["packagePath", "version"], violations);
    let package_path = validated_field(
        object,
        "packagePath",
        pointer,
        PackagePath::parse,
        violations,
    );
    let version = validated_field(object, "version", pointer, StableVersion::parse, violations);
    Some(ReleaseId {
        package_path: package_path?,
        version: version?,
    })
}

fn validated_field<T>(
    object: &Map<String, Value>,
    field: &str,
    parent: &str,
    parse: impl FnOnce(&str) -> Result<T, ResolutionValueError>,
    violations: &mut Vec<Violation>,
) -> Option<T> {
    let pointer = format!("{parent}/{field}");
    let Some(value) = object.get(field) else {
        violations.push(violation(pointer, ViolationRule::MissingField));
        return None;
    };
    let Some(value) = value.as_str() else {
        violations.push(violation(pointer, ViolationRule::InvalidType));
        return None;
    };
    match parse(value) {
        Ok(value) => Some(value),
        Err(error) => {
            violations.push(violation(pointer, value_rule(error)));
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

fn expect_object<'a>(
    value: &'a Value,
    pointer: &str,
    violations: &mut Vec<Violation>,
) -> Option<&'a Map<String, Value>> {
    value.as_object().or_else(|| {
        violations.push(violation(pointer, ViolationRule::InvalidType));
        None
    })
}

fn unknown_fields(
    object: &Map<String, Value>,
    pointer: &str,
    allowed: &[&str],
    violations: &mut Vec<Violation>,
) {
    for key in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
        violations.push(violation(
            format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
            ViolationRule::UnknownField,
        ));
    }
}
