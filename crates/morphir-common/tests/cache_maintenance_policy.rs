use morphir_common::cache_maintenance::{
    CacheDecisionReason, CacheEntry, CachePolicy, CleanupMode, plan_cache_cleanup,
};
use std::time::Duration;

const DAY: u64 = 24 * 60 * 60;

fn a_disposable_entry(namespace: &str, path: &str, bytes: u64, last_used_day: u64) -> CacheEntry {
    CacheEntry::disposable(namespace, path, bytes, last_used_day * DAY).unwrap()
}

#[test]
fn policy_cleanup_selects_expired_entries_then_lru_until_known_usage_is_bounded() {
    let entries = vec![
        a_disposable_entry("downloads", "old.pkg", 60, 1),
        a_disposable_entry("downloads", "recent.pkg", 80, 99),
        a_disposable_entry("indexes", "older-index.json", 70, 98),
        CacheEntry::leased("desktop", "active/session", 90, DAY).unwrap(),
        CacheEntry::unclassified("desktop", "unexpected-link", 50).unwrap(),
    ];

    let plan = plan_cache_cleanup(
        entries,
        CachePolicy::new(Duration::from_secs(30 * DAY), 180),
        100 * DAY,
        CleanupMode::Policy,
    )
    .unwrap();

    assert_eq!(plan.known_bytes_before(), 300);
    assert_eq!(plan.unclassified_bytes(), 50);
    assert_eq!(plan.reclaimable_bytes(), 130);
    assert_eq!(plan.known_bytes_after(), 170);
    assert_eq!(
        plan.decisions()
            .iter()
            .map(|decision| (
                decision.entry().namespace(),
                decision.entry().path(),
                decision.reason()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "desktop",
                "active/session",
                CacheDecisionReason::ActiveLease
            ),
            (
                "desktop",
                "unexpected-link",
                CacheDecisionReason::Unclassified
            ),
            ("downloads", "old.pkg", CacheDecisionReason::Expired),
            ("downloads", "recent.pkg", CacheDecisionReason::WithinPolicy),
            (
                "indexes",
                "older-index.json",
                CacheDecisionReason::SizeLimit
            ),
        ]
    );
    assert_eq!(
        plan.decision("downloads", "old.pkg").unwrap().reason(),
        CacheDecisionReason::Expired
    );
    assert_eq!(
        plan.decision("indexes", "older-index.json")
            .unwrap()
            .reason(),
        CacheDecisionReason::SizeLimit
    );
    assert_eq!(
        plan.decision("downloads", "recent.pkg").unwrap().reason(),
        CacheDecisionReason::WithinPolicy
    );
    assert_eq!(
        plan.decision("desktop", "active/session").unwrap().reason(),
        CacheDecisionReason::ActiveLease
    );
    assert_eq!(
        plan.decision("desktop", "unexpected-link")
            .unwrap()
            .reason(),
        CacheDecisionReason::Unclassified
    );
}

#[test]
fn all_mode_still_protects_leased_and_unclassified_entries() {
    let entries = vec![
        a_disposable_entry("downloads", "artifact.pkg", 60, 99),
        CacheEntry::leased("desktop", "active/session", 90, DAY).unwrap(),
        CacheEntry::unclassified("desktop", "unexpected-link", 50).unwrap(),
    ];

    let plan = plan_cache_cleanup(
        entries,
        CachePolicy::new(Duration::from_secs(30 * DAY), 180),
        100 * DAY,
        CleanupMode::All,
    )
    .unwrap();

    assert_eq!(plan.reclaimable_bytes(), 60);
    assert_eq!(
        plan.decision("downloads", "artifact.pkg").unwrap().reason(),
        CacheDecisionReason::RemoveAll
    );
    assert_eq!(
        plan.decision("desktop", "active/session").unwrap().reason(),
        CacheDecisionReason::ActiveLease
    );
    assert_eq!(
        plan.decision("desktop", "unexpected-link")
            .unwrap()
            .reason(),
        CacheDecisionReason::Unclassified
    );
}
