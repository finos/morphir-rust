use morphir_core::metadata::{
    Assertion, AssertionKey, AssertionSource, Carrier, ContextResources, DocumentId, Fact,
    GraphIndex, GraphName, MetadataError, ObjectTerm, SourceRecord, resolve_context,
};
use morphir_core::node_address::NodeUri;
use serde_json::json;

fn node(path: &str) -> NodeUri {
    NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/{path}"
    ))
    .unwrap()
}

fn assertion() -> Assertion {
    let subject = node("submit-order");
    let predicate = node("deprecated");
    let owner = DocumentId::new("orders/spec.json").unwrap();
    let fact = Fact::new(
        subject,
        predicate,
        ObjectTerm::value(json!(true)),
        GraphName::Default,
    );
    Assertion::new(AssertionKey::new(owner, Carrier::DocumentGraph, fact).unwrap())
}

fn owner() -> DocumentId {
    DocumentId::new("orders/spec.json").unwrap()
}

fn compiler_source() -> AssertionSource {
    AssertionSource::Compiler {
        producer: "morphir-gleam".to_owned(),
        reference: Some("src/Orders.gleam".to_owned()),
    }
}

fn changed_fact() -> Fact {
    Fact::new(
        node("submit-order"),
        node("deprecated"),
        ObjectTerm::value(json!(false)),
        GraphName::Default,
    )
}

#[test]
fn metadata_optional_detail_same_graph() {
    let plain = assertion();
    let mut detailed = assertion();
    detailed
        .add_detail(AssertionSource::Compiler {
            producer: "morphir-gleam".to_owned(),
            reference: Some("src/Orders.gleam".to_owned()),
        })
        .unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(plain).unwrap();
    graph.insert(detailed).unwrap();

    assert_eq!(graph.facts().len(), 1);
    assert_eq!(graph.assertions().len(), 1);
    assert_eq!(graph.assertions()[0].sources().len(), 2);
    assert_eq!(
        graph.assertions()[0].sources()[0],
        AssertionSource::Document(DocumentId::new("orders/spec.json").unwrap())
    );
}

#[test]
fn metadata_repeated_detail_claim_is_one_source() {
    let mut assertion = assertion();
    let source = AssertionSource::Author {
        reference: "review/deprecation".to_owned(),
    };
    assertion.add_detail(source.clone()).unwrap();
    assertion.add_detail(source).unwrap();
    assert_eq!(assertion.detail().len(), 1);
}

#[test]
fn metadata_document_provenance_cannot_be_added_as_detail() {
    let mut assertion = assertion();
    let result = assertion.add_detail(AssertionSource::Document(
        DocumentId::new("somewhere/else.json").unwrap(),
    ));
    assert!(result.is_err());
    assert_eq!(assertion.sources().len(), 1);
}

#[test]
fn metadata_alias_rename_preserves_assertion_key() {
    let resources = ContextResources::new("contexts");
    let uri = node("deprecated").to_string();
    let first = resolve_context(None, &json!({"retired": uri}), &resources, None).unwrap();
    let renamed = resolve_context(None, &json!({"deprecated": uri}), &resources, None).unwrap();
    let key_for = |predicate: NodeUri| {
        let fact = Fact::new(
            node("submit-order"),
            predicate,
            ObjectTerm::value(json!(true)),
            GraphName::Default,
        );
        AssertionKey::new(owner(), Carrier::DocumentGraph, fact).unwrap()
    };
    assert_eq!(
        key_for(first.expand_key("retired").unwrap().uri().clone()),
        key_for(renamed.expand_key("deprecated").unwrap().uri().clone())
    );
}

#[test]
fn metadata_source_record_replaces_document_default_and_deduplicates() {
    let key = assertion().key().clone();
    let record = SourceRecord::new(key, vec![compiler_source(), compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    graph.apply_source_records(&owner(), &[record]).unwrap();

    assert_eq!(graph.assertions()[0].sources(), vec![compiler_source()]);
    assert_eq!(graph.facts().len(), 1);
}

#[test]
fn metadata_empty_source_record_is_rejected() {
    assert_eq!(
        SourceRecord::new(assertion().key().clone(), vec![]),
        Err(MetadataError::EmptyAssertionSources)
    );
}

#[test]
fn metadata_duplicate_selector_rejected_without_partial_application() {
    let record = SourceRecord::new(assertion().key().clone(), vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    let result = graph.apply_source_records(&owner(), &[record.clone(), record]);

    assert_eq!(
        result,
        Err(MetadataError::DuplicateSourceSelector(Box::new(
            assertion().key().clone()
        )))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
}

#[test]
fn metadata_changed_fact_unmatched_selector_is_error() {
    let stale = SourceRecord::new(assertion().key().clone(), vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    let current = AssertionKey::new(owner(), Carrier::DocumentGraph, changed_fact()).unwrap();
    graph.insert(Assertion::new(current)).unwrap();

    assert_eq!(
        graph.apply_source_records(&owner(), &[stale]),
        Err(MetadataError::UnmatchedSourceSelector(Box::new(
            assertion().key().clone()
        )))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
}

#[test]
fn metadata_source_record_cannot_select_other_owner() {
    let other_owner = DocumentId::new("orders/other.json").unwrap();
    let key = assertion().key().clone();
    let record = SourceRecord::new(key, vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    assert_eq!(
        graph.apply_source_records(&other_owner, &[record]),
        Err(MetadataError::SourceSelectorOwnerMismatch(Box::new(
            assertion().key().clone()
        )))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
}

#[test]
fn metadata_source_record_cannot_select_sidecar() {
    let fact = assertion().key().fact().clone();
    let key = AssertionKey::new(
        owner(),
        Carrier::Sidecar {
            target: node("submit-order"),
            entry_point: node("deprecated"),
        },
        fact,
    )
    .unwrap();
    let record = SourceRecord::new(key.clone(), vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(Assertion::new(key.clone())).unwrap();

    assert_eq!(
        graph.apply_source_records(&owner(), &[record]),
        Err(MetadataError::SourceSelectorCarrierUnsupported(Box::new(
            key
        )))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
}

#[test]
fn metadata_source_record_requires_exact_carrier() {
    let key = AssertionKey::new(
        owner(),
        Carrier::AttributesFacts(node("submit-order")),
        assertion().key().fact().clone(),
    )
    .unwrap();
    let record = SourceRecord::new(key.clone(), vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();

    assert_eq!(
        graph.apply_source_records(&owner(), &[record]),
        Err(MetadataError::UnmatchedSourceSelector(Box::new(key)))
    );
}

#[test]
fn metadata_late_bad_record_does_not_apply_earlier_record() {
    let valid = SourceRecord::new(assertion().key().clone(), vec![compiler_source()]).unwrap();
    let stale_key = AssertionKey::new(owner(), Carrier::DocumentGraph, changed_fact()).unwrap();
    let stale = SourceRecord::new(stale_key.clone(), vec![compiler_source()]).unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();

    assert_eq!(
        graph.apply_source_records(&owner(), &[valid, stale]),
        Err(MetadataError::UnmatchedSourceSelector(Box::new(stale_key)))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
}

#[test]
fn metadata_rewrite_preserves_existing_detail() {
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    let original = assertion().key().clone();
    let record = SourceRecord::new(original.clone(), vec![compiler_source()]).unwrap();
    graph.apply_source_records(&owner(), &[record]).unwrap();

    graph.rewrite_assertion(&original, changed_fact()).unwrap();
    assert_eq!(graph.facts(), &[changed_fact()]);
    assert_eq!(graph.assertions()[0].key().fact(), &changed_fact());
    assert_eq!(graph.assertions()[0].sources(), vec![compiler_source()]);
}

#[test]
fn metadata_unknown_owner_prevents_source_based_removal() {
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();

    assert_eq!(
        graph.remove_source_from_owner(&owner(), &compiler_source()),
        Err(MetadataError::UnknownSourceOwnership)
    );
    assert_eq!(graph.assertions().len(), 1);
}

#[test]
fn metadata_known_source_removal_keeps_other_source_and_fact() {
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    let record = SourceRecord::new(
        assertion().key().clone(),
        vec![compiler_source(), AssertionSource::Document(owner())],
    )
    .unwrap();
    graph.apply_source_records(&owner(), &[record]).unwrap();

    assert_eq!(
        graph.remove_source_from_owner(&owner(), &compiler_source()),
        Ok(1)
    );
    assert_eq!(graph.facts().len(), 1);
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Document(owner())]
    );
    assert!(graph.assertions()[0].source_override().is_some());
}

#[test]
fn metadata_last_known_source_removal_removes_fact() {
    let mut graph = GraphIndex::new();
    graph.insert(assertion()).unwrap();
    let record = SourceRecord::new(assertion().key().clone(), vec![compiler_source()]).unwrap();
    graph.apply_source_records(&owner(), &[record]).unwrap();

    assert_eq!(
        graph.remove_source_from_owner(&owner(), &compiler_source()),
        Ok(1)
    );
    assert!(graph.assertions().is_empty());
    assert!(graph.facts().is_empty());
}
