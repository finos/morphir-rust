use super::{
    CacheDecision, CacheDecisionReason, CacheEntry, CacheEntryState, CachePolicy, CleanupMode,
    CleanupPlan,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// An inconsistent or unrepresentable cache inventory.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CachePlanError {
    /// Each namespace/path identity may occur only once.
    #[error("cache inventory contains duplicate entry {namespace}/{path}")]
    DuplicateEntry {
        /// Registered cache namespace.
        namespace: String,
        /// Portable entry path within that namespace.
        path: String,
    },
    /// Inventory totals exceeded the representable byte range.
    #[error("cache inventory byte total exceeds the supported range")]
    ByteCountOverflow,
}

/// Build one deterministic cleanup plan without changing the filesystem.
pub fn plan_cache_cleanup(
    mut entries: Vec<CacheEntry>,
    policy: CachePolicy,
    now: u64,
    mode: CleanupMode,
) -> Result<CleanupPlan, CachePlanError> {
    entries.sort_by(|left, right| {
        (left.namespace(), left.path()).cmp(&(right.namespace(), right.path()))
    });
    reject_duplicates(&entries)?;

    let known_bytes_before = sum_bytes(
        entries
            .iter()
            .filter(|entry| !matches!(entry.state(), CacheEntryState::Unclassified)),
    )?;
    let unclassified_bytes = sum_bytes(
        entries
            .iter()
            .filter(|entry| matches!(entry.state(), CacheEntryState::Unclassified)),
    )?;

    let mut removals = match mode {
        CleanupMode::All => entries
            .iter()
            .filter(|entry| matches!(entry.state(), CacheEntryState::Disposable { .. }))
            .map(|entry| (identity(entry), CacheDecisionReason::RemoveAll))
            .collect::<BTreeMap<_, _>>(),
        CleanupMode::Policy => expired_entries(&entries, policy, now),
    };

    if mode == CleanupMode::Policy {
        let expired_bytes = sum_bytes(
            entries
                .iter()
                .filter(|entry| removals.contains_key(&identity(entry))),
        )?;
        let mut remaining = known_bytes_before - expired_bytes;
        let mut lru = entries
            .iter()
            .filter(|entry| {
                matches!(entry.state(), CacheEntryState::Disposable { .. })
                    && !removals.contains_key(&identity(entry))
            })
            .collect::<Vec<_>>();
        lru.sort_by(|left, right| {
            (
                left.last_used().expect("disposable entry has last use"),
                left.namespace(),
                left.path(),
            )
                .cmp(&(
                    right.last_used().expect("disposable entry has last use"),
                    right.namespace(),
                    right.path(),
                ))
        });

        for entry in lru {
            if remaining <= policy.max_size_bytes() {
                break;
            }
            removals.insert(identity(entry), CacheDecisionReason::SizeLimit);
            remaining -= entry.bytes();
        }
    }

    let decisions = entries
        .into_iter()
        .map(|entry| {
            let reason =
                removals
                    .get(&identity(&entry))
                    .copied()
                    .unwrap_or_else(|| match entry.state() {
                        CacheEntryState::Disposable { .. } => CacheDecisionReason::WithinPolicy,
                        CacheEntryState::ActiveLease { .. } => CacheDecisionReason::ActiveLease,
                        CacheEntryState::Unclassified => CacheDecisionReason::Unclassified,
                    });
            CacheDecision::new(entry, reason)
        })
        .collect::<Vec<_>>();
    let reclaimable_bytes = sum_bytes(
        decisions
            .iter()
            .filter(|decision| decision.will_remove())
            .map(CacheDecision::entry),
    )?;

    Ok(CleanupPlan::new(
        policy,
        mode,
        known_bytes_before,
        known_bytes_before - reclaimable_bytes,
        unclassified_bytes,
        reclaimable_bytes,
        decisions,
    ))
}

fn expired_entries(
    entries: &[CacheEntry],
    policy: CachePolicy,
    now: u64,
) -> BTreeMap<(String, String), CacheDecisionReason> {
    entries
        .iter()
        .filter(|entry| {
            matches!(entry.state(), CacheEntryState::Disposable { .. })
                && now.saturating_sub(entry.last_used().expect("disposable entry has last use"))
                    > policy.max_age_seconds()
        })
        .map(|entry| (identity(entry), CacheDecisionReason::Expired))
        .collect()
}

fn reject_duplicates(entries: &[CacheEntry]) -> Result<(), CachePlanError> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        let identity = identity(entry);
        if !seen.insert(identity.clone()) {
            return Err(CachePlanError::DuplicateEntry {
                namespace: identity.0,
                path: identity.1,
            });
        }
    }
    Ok(())
}

fn identity(entry: &CacheEntry) -> (String, String) {
    (entry.namespace().to_owned(), entry.path().to_owned())
}

fn sum_bytes<'a>(entries: impl IntoIterator<Item = &'a CacheEntry>) -> Result<u64, CachePlanError> {
    entries.into_iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.bytes())
            .ok_or(CachePlanError::ByteCountOverflow)
    })
}

#[cfg(test)]
mod tests {
    use super::{CachePlanError, plan_cache_cleanup};
    use crate::cache_maintenance::{CacheEntry, CachePolicy, CleanupMode};
    use std::time::Duration;

    #[test]
    fn duplicate_identities_are_rejected_before_planning() {
        let entry = CacheEntry::disposable("downloads", "same.pkg", 1, 1).unwrap();
        let error = plan_cache_cleanup(
            vec![entry.clone(), entry],
            CachePolicy::new(Duration::from_secs(10), 10),
            20,
            CleanupMode::Policy,
        )
        .unwrap_err();

        assert_eq!(
            error,
            CachePlanError::DuplicateEntry {
                namespace: "downloads".to_owned(),
                path: "same.pkg".to_owned(),
            }
        );
    }

    #[test]
    fn byte_totals_fail_closed_on_overflow() {
        let error = plan_cache_cleanup(
            vec![
                CacheEntry::disposable("downloads", "first", u64::MAX, 1).unwrap(),
                CacheEntry::disposable("downloads", "second", 1, 1).unwrap(),
            ],
            CachePolicy::new(Duration::from_secs(10), 10),
            20,
            CleanupMode::Policy,
        )
        .unwrap_err();

        assert_eq!(error, CachePlanError::ByteCountOverflow);
    }
}
