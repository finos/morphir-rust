//! The default OKF profile must carry the exact OKF v0.2 and producer key
//! sets from `KbModel.scala`.

use morphir_okf::OkfProfile;

#[test]
fn default_profile_carries_the_okf_v02_keys() {
    let p = OkfProfile::default();
    for key in [
        "type",
        "title",
        "description",
        "resource",
        "tags",
        "sources",
        "generated",
        "verified",
        "status",
        "stale_after",
        "runtime",
        "parameters",
        "computation",
        "executor",
        "attester",
        "okf_version",
        "sync",
        "kb_upstream",
    ] {
        assert!(p.known_keys.contains(key), "missing OKF key: {key}");
    }
    assert_eq!(p.known_keys.len(), 18);
}

#[test]
fn default_profile_carries_the_producer_keys() {
    let p = OkfProfile::default();
    for key in [
        "state",
        "kind",
        "breaking",
        "created",
        "state_since",
        "issue",
        "capability",
        "superseded_by",
        "reason",
        "artifacts",
        "implementation_baselines",
        "intent",
        "system",
        "capability_bundle",
        "stale_after_days",
        "decided",
        "supersedes",
    ] {
        assert!(
            p.producer_known_keys.contains(key),
            "missing producer key: {key}"
        );
    }
    assert_eq!(p.producer_known_keys.len(), 17);
}

#[test]
fn recognition_spans_both_sets_and_nothing_else() {
    let p = OkfProfile::default();
    assert!(p.is_recognized("type"));
    assert!(p.is_recognized("state_since"));
    assert!(!p.is_recognized("banana"));
}

#[test]
fn owned_types_match_case_insensitively_and_trimmed() {
    let p = OkfProfile::default();
    assert!(p.owns_type("Decision Record"));
    assert!(p.owns_type("  decision record  "));
    assert!(!p.owns_type("Design Note"));
}

#[test]
fn statuses_are_the_okf_maturity_values() {
    let p = OkfProfile::default();
    for s in ["draft", "stable", "deprecated"] {
        assert!(p.is_known_status(s));
    }
    assert!(!p.is_known_status("wip"));
}
