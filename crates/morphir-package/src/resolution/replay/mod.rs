mod components;
mod metadata;
mod shape;
mod topology;

use super::execution::enforce_selected_release_limit;
use super::model::*;
use super::order;
use super::validate::{OuterInput, normalize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub(super) struct LocatedNode {
    pub(super) value: LockedNode,
    pub(super) pointer: String,
}

#[derive(Debug, Clone)]
pub(super) struct LocatedLock {
    pub(super) value: LockedGraph,
    pub(super) nodes: Vec<LocatedNode>,
}

pub(super) fn enforce_lock_size(input: &OuterInput) -> Result<(), ResolutionExecutionError> {
    let Some(nodes) = input
        .raw_lock
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|lock| lock.get("nodes"))
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    enforce_selected_release_limit(nodes.len())
}

pub(super) fn validate_lock(input: &OuterInput) -> Result<LockedGraph, Box<ResolutionDiagnostic>> {
    let lock = shape::lock_shape(input)?;
    reject_lock(lock_identities(&lock))?;
    reject_lock(topology::lock_topology(input, &lock))?;
    metadata::selected_metadata(input, &lock)?;
    reject_lock(metadata::lock_metadata(input, &lock))?;
    Ok(order::normalize_graph(lock.value))
}

fn lock_identities(lock: &LocatedLock) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut releases = HashSet::new();
    let mut paths = HashSet::new();
    let mut names = HashMap::new();
    for node in &lock.nodes {
        if !releases.insert(node.value.release.clone()) {
            violations.push(violation(
                format!("{}/release", node.pointer),
                ViolationRule::DuplicateIdentity,
            ));
            continue;
        }
        if !paths.insert(node.value.release.package_path.clone()) {
            violations.push(violation(
                format!("{}/release/packagePath", node.pointer),
                ViolationRule::DuplicateIdentity,
            ));
            continue;
        }
        if names
            .insert(
                node.value.ir_package_name.clone(),
                node.value.release.clone(),
            )
            .is_some()
        {
            violations.push(violation(
                format!("{}/irPackageName", node.pointer),
                ViolationRule::UnsupportedFlatBinding,
            ));
        }
        let mut bindings = HashSet::new();
        for (index, binding) in node.value.bindings.iter().enumerate() {
            if !bindings.insert(binding.ir_package_name.clone()) {
                violations.push(violation(
                    format!("{}/bindings/{index}/irPackageName", node.pointer),
                    ViolationRule::DuplicateIdentity,
                ));
            }
        }
    }
    normalize(violations)
}

pub(super) fn reject_lock(violations: Vec<Violation>) -> Result<(), Box<ResolutionDiagnostic>> {
    if violations.is_empty() {
        Ok(())
    } else {
        Err(Box::new(invalid_lock(violations)))
    }
}

pub(super) fn invalid_lock(violations: Vec<Violation>) -> ResolutionDiagnostic {
    ResolutionDiagnostic::InvalidLock {
        violations: normalize(violations),
    }
}

pub(super) fn violation(pointer: impl Into<String>, rule: ViolationRule) -> Violation {
    Violation {
        pointer: pointer.into(),
        rule,
    }
}
