use std::collections::BTreeMap;

use morphir_core::ir::v4;

use crate::model::{EntryPointKind, EntryPointMetadata};

use super::NormalizeError;

pub(super) fn validate_entry_points(
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

fn normalize_entry_point_kind(kind: v4::EntryPointKind) -> EntryPointKind {
    match kind {
        v4::EntryPointKind::Main => EntryPointKind::Main,
        v4::EntryPointKind::Command => EntryPointKind::Command,
        v4::EntryPointKind::Handler => EntryPointKind::Handler,
    }
}
