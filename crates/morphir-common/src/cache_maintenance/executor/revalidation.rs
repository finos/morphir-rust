use super::filesystem::RecoveryBudget;
use super::{CacheExecutionError, CacheExecutionLimits};
use crate::cache_maintenance::inventory::{PinnedCacheEntry, inventory_cache_namespace_pinned};
use crate::cache_maintenance::{
    CacheEntry, CacheEntryState, CacheInventoryLimits, CacheNamespace, CleanupPlan,
};
use crate::home::MorphirHome;
use cap_std::fs::Dir;
use same_file::Handle;
use std::collections::{BTreeMap, BTreeSet};

pub(super) enum RevalidatedEntry<'a> {
    Missing,
    ActiveLease {
        observed_bytes: u64,
    },
    Unclassified {
        observed_bytes: u64,
    },
    Unregistered,
    Stale {
        observed_bytes: u64,
    },
    Ready {
        handle: &'a Handle,
        fingerprint: u64,
    },
}

pub(super) fn inventory_for_execution(
    home: &MorphirHome,
    home_dir: &Dir,
    ownership: &[CacheNamespace],
    limits: CacheInventoryLimits,
    plan: &CleanupPlan,
    execution_limits: CacheExecutionLimits,
    recovered: RecoveryBudget,
) -> Result<BTreeMap<String, Vec<PinnedCacheEntry>>, CacheExecutionError> {
    let mut pinned_paths = BTreeMap::<String, BTreeSet<String>>::new();
    let remaining_removals = execution_limits
        .max_removals
        .saturating_sub(recovered.removals);
    let mut budgeted_bytes = recovered.bytes;
    for decision in plan
        .decisions()
        .iter()
        .filter(|decision| decision.will_remove())
        .take(remaining_removals)
    {
        let entry = decision.entry();
        let next_budgeted_bytes = budgeted_bytes
            .checked_add(entry.bytes())
            .ok_or(CacheExecutionError::ByteCountOverflow)?;
        if next_budgeted_bytes > execution_limits.max_bytes {
            break;
        }
        budgeted_bytes = next_budgeted_bytes;
        pinned_paths
            .entry(entry.namespace().to_owned())
            .or_default()
            .insert(entry.path().to_owned());
    }
    let mut inventories = BTreeMap::new();
    for namespace in ownership {
        let Some(requested) = pinned_paths.get(namespace.name()).cloned() else {
            continue;
        };
        if inventories.contains_key(namespace.name()) {
            return Err(CacheExecutionError::DuplicateNamespace {
                namespace: namespace.name().to_owned(),
            });
        }
        inventories.insert(
            namespace.name().to_owned(),
            inventory_cache_namespace_pinned(home, home_dir, namespace, limits, &requested)?,
        );
    }
    Ok(inventories)
}

pub(super) fn revalidate_entry<'a>(
    inventories: &'a BTreeMap<String, Vec<PinnedCacheEntry>>,
    planned: &CacheEntry,
) -> RevalidatedEntry<'a> {
    let Some(inventory) = inventories.get(planned.namespace()) else {
        return RevalidatedEntry::Unregistered;
    };
    if let Some(observed) = inventory
        .iter()
        .find(|entry| entry.entry().path() == planned.path())
    {
        let observed_entry = observed.entry();
        return match observed_entry.state() {
            CacheEntryState::Disposable { .. }
                if observed_entry.bytes() == planned.bytes()
                    && observed_entry.state() == planned.state() =>
            {
                match (observed.handle(), observed.fingerprint()) {
                    (Some(handle), Some(fingerprint)) => RevalidatedEntry::Ready {
                        handle,
                        fingerprint,
                    },
                    _ => RevalidatedEntry::Unclassified {
                        observed_bytes: observed_entry.bytes(),
                    },
                }
            }
            CacheEntryState::Disposable { .. } => RevalidatedEntry::Stale {
                observed_bytes: observed_entry.bytes(),
            },
            CacheEntryState::ActiveLease { .. } => RevalidatedEntry::ActiveLease {
                observed_bytes: observed_entry.bytes(),
            },
            CacheEntryState::Unclassified => RevalidatedEntry::Unclassified {
                observed_bytes: observed_entry.bytes(),
            },
        };
    }
    let unsafe_ancestor = inventory.iter().find(|entry| {
        entry.entry().state() == CacheEntryState::Unclassified
            && planned
                .path()
                .strip_prefix(entry.entry().path())
                .is_some_and(|rest| rest.starts_with('/'))
    });
    match unsafe_ancestor {
        Some(entry) => RevalidatedEntry::Unclassified {
            observed_bytes: entry.entry().bytes(),
        },
        None => RevalidatedEntry::Missing,
    }
}
