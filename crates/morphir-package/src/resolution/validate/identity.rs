use super::super::model::*;
use super::{LocatedRecord, OuterInput, normalize};
use std::collections::HashSet;

pub(crate) fn outer_identities(input: &mut OuterInput) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut releases = HashSet::new();
    releases.insert(input.root.value.release.clone());
    validate_requirements(&input.root, &mut violations);

    let mut catalog_paths = HashSet::new();
    for catalog in &mut input.catalogs {
        if !catalog_paths.insert(catalog.value.package_path.clone()) {
            catalog.suppressed = true;
            push(
                format!("{}/packagePath", catalog.pointer),
                ViolationRule::DuplicateIdentity,
                &mut violations,
            );
            continue;
        }
        for record in &mut catalog.records {
            if !releases.insert(record.value.release.clone()) {
                record.suppressed = true;
                push(
                    format!("{}/release", record.pointer),
                    ViolationRule::DuplicateIdentity,
                    &mut violations,
                );
                continue;
            }
            if record.value.release.package_path != catalog.value.package_path {
                push(
                    format!("{}/release/packagePath", record.pointer),
                    ViolationRule::IdentityMismatch,
                    &mut violations,
                );
            }
            validate_requirements(record, &mut violations);
        }
    }

    for record in &mut input.releases {
        if !releases.insert(record.value.release.clone()) {
            record.suppressed = true;
            push(
                format!("{}/release", record.pointer),
                ViolationRule::DuplicateIdentity,
                &mut violations,
            );
            continue;
        }
        validate_requirements(record, &mut violations);
    }

    let mut target_paths = HashSet::new();
    for target in &input.targets {
        if !target_paths.insert(target.value.package_path().clone()) {
            push(
                format!("{}/packagePath", target.pointer),
                ViolationRule::DuplicateIdentity,
                &mut violations,
            );
        }
    }
    normalize(violations)
}

fn validate_requirements(record: &LocatedRecord, violations: &mut Vec<Violation>) {
    let mut names = HashSet::new();
    for (index, requirement) in record.value.dependencies.iter().enumerate() {
        let pointer = format!("{}/dependencies/{index}", record.pointer);
        if !names.insert(requirement.ir_package_name.clone()) {
            push(
                format!("{pointer}/irPackageName"),
                ViolationRule::DuplicateIdentity,
                violations,
            );
            continue;
        }
        if requirement.version_range.minimum_inclusive
            >= requirement.version_range.maximum_exclusive
        {
            push(
                format!("{pointer}/versionRange"),
                ViolationRule::InvalidInterval,
                violations,
            );
        }
    }
}

fn push(pointer: impl Into<String>, rule: ViolationRule, violations: &mut Vec<Violation>) {
    violations.push(Violation {
        pointer: pointer.into(),
        rule,
    });
}
