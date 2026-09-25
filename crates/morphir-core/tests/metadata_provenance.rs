use morphir_core::metadata::{
    Assertion, AssertionKey, AssertionSource, Carrier, DocumentId, Fact, GraphIndex, GraphName,
    ObjectTerm,
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
