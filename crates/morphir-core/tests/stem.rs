//! Unit tests for the document-tree file-stem escape and truncation, cross-checked
//! against a value pulled from the shared naming conformance corpus.

use morphir_core::naming::{Name, file_stem, truncate_stem};

#[test]
fn truncate_stem_matches_the_corpus_value() {
    assert_eq!(
        truncate_stem("customer-relationship-management-record", 25),
        "customer-relati__44a101f8"
    );
}

#[test]
fn file_stem_escapes_reserved_device_names() {
    assert_eq!(file_stem(&Name::from("aux")), "aux_");
    assert_eq!(file_stem(&Name::from("CON")), "_con");
}
