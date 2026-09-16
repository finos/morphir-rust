//! Unit tests for the document-tree file-stem escape and truncation, cross-checked
//! against a value pulled from the shared naming conformance corpus.

use morphir_core::naming::{MIN_TRUNCATED_STEM_BUDGET, Name, file_stem, truncate_stem};

#[test]
fn truncate_stem_matches_the_corpus_value() {
    assert_eq!(
        truncate_stem("customer-relationship-management-record", 25).as_deref(),
        Some("customer-relati__44a101f8")
    );
}

#[test]
fn truncate_stem_refuses_a_budget_below_the_hash_suffix() {
    // Ten characters hold `__` and eight hex digits and nothing of the name, so there is no
    // truncation to offer and the function says so rather than overrunning the budget.
    assert_eq!(MIN_TRUNCATED_STEM_BUDGET, 11);
    assert_eq!(
        truncate_stem("customer-relationship-management-record", 10),
        None
    );
}

#[test]
fn truncate_stem_fits_the_smallest_budget_it_accepts() {
    let truncated = truncate_stem("customer-relationship-management-record", 11)
        .expect("the smallest accepted budget");
    assert_eq!(truncated, "c__44a101f8");
    assert!(truncated.chars().count() <= 11, "{truncated}");
}

#[test]
fn file_stem_escapes_reserved_device_names() {
    assert_eq!(file_stem(&Name::from("aux")), "aux_");
    assert_eq!(file_stem(&Name::from("CON")), "_con");
}
