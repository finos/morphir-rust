use super::super::model::*;
use super::super::validate::{OuterInput, normalize};
use super::{LocatedLock, violation};
use std::collections::BTreeMap;

pub(super) fn selected_metadata(
    input: &OuterInput,
    lock: &LocatedLock,
) -> Result<(), Box<ResolutionDiagnostic>> {
    let records = record_map(input);
    let mut missing: Vec<_> = lock
        .nodes
        .iter()
        .filter(|node| node.value.release != input.root.value.release)
        .filter(|node| !records.contains_key(&node.value.release))
        .map(|node| MissingItem::Release {
            release: node.value.release.clone(),
        })
        .collect();
    missing.sort_by(|left, right| match (left, right) {
        (MissingItem::Release { release: left }, MissingItem::Release { release: right }) => left
            .package_path
            .cmp(&right.package_path)
            .then_with(|| right.version.cmp(&left.version)),
        _ => std::cmp::Ordering::Equal,
    });
    missing.dedup();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Box::new(ResolutionDiagnostic::IncompleteInput { missing }))
    }
}

pub(super) fn lock_metadata(input: &OuterInput, lock: &LocatedLock) -> Vec<Violation> {
    let records = record_map(input);
    let nodes: BTreeMap<_, _> = lock
        .nodes
        .iter()
        .map(|node| (node.value.release.clone(), node))
        .collect();
    let mut violations = Vec::new();
    for node in &lock.nodes {
        let metadata = if node.value.release == input.root.value.release {
            &input.root.value
        } else {
            records[&node.value.release]
        };
        if node.value.ir_package_name != metadata.ir_package_name {
            violations.push(violation(
                format!("{}/irPackageName", node.pointer),
                ViolationRule::IdentityMismatch,
            ));
        }
        if node.value.manifest_digest != metadata.manifest_digest {
            violations.push(violation(
                format!("{}/manifestDigest", node.pointer),
                ViolationRule::DigestMismatch,
            ));
        }
        if node.value.content_digest != metadata.content_digest {
            violations.push(violation(
                format!("{}/contentDigest", node.pointer),
                ViolationRule::DigestMismatch,
            ));
        }
        let bindings: BTreeMap<_, _> = node
            .value
            .bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| (&binding.ir_package_name, (index, binding)))
            .collect();
        for requirement in &metadata.dependencies {
            let Some((index, binding)) = bindings.get(&requirement.ir_package_name).copied() else {
                violations.push(violation(
                    format!("{}/bindings", node.pointer),
                    ViolationRule::BindingMismatch,
                ));
                continue;
            };
            let target = &nodes[&binding.target].value;
            let pointer = format!("{}/bindings/{index}/target", node.pointer);
            if binding.target.package_path != requirement.package_path
                || target.ir_package_name != requirement.ir_package_name
            {
                violations.push(violation(pointer, ViolationRule::BindingMismatch));
            } else if !requirement.version_range.contains(&binding.target.version) {
                violations.push(violation(pointer, ViolationRule::RequirementMismatch));
            }
        }
        for (index, binding) in node.value.bindings.iter().enumerate() {
            if !metadata
                .dependencies
                .iter()
                .any(|requirement| requirement.ir_package_name == binding.ir_package_name)
            {
                violations.push(violation(
                    format!("{}/bindings/{index}/irPackageName", node.pointer),
                    ViolationRule::BindingMismatch,
                ));
            }
        }
    }
    normalize(violations)
}

fn record_map(input: &OuterInput) -> BTreeMap<ReleaseId, &ReleaseRecord> {
    let mut records = BTreeMap::new();
    records.insert(input.root.value.release.clone(), &input.root.value);
    for record in input
        .releases
        .iter()
        .chain(input.catalogs.iter().flat_map(|catalog| &catalog.records))
    {
        records.insert(record.value.release.clone(), &record.value);
    }
    records
}
