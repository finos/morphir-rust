use morphir_core::metadata::{
    Assertion, AssertionKey, Carrier, DocumentId, Fact, GraphIndex, GraphName, MetadataError,
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

fn predicate(name: &str) -> NodeUri {
    NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/{name}"
    ))
    .unwrap()
}

fn document() -> DocumentId {
    DocumentId::new("orders/spec.json").unwrap()
}

fn fact() -> Fact {
    Fact::new(
        node("submit-order"),
        predicate("aliases"),
        ObjectTerm::value(json!("placeOrder")),
        GraphName::Default,
    )
}

#[test]
fn metadata_three_carriers_one_fact_three_assertions() {
    let subject = node("submit-order");
    let mut graph = GraphIndex::new();
    for carrier in [
        Carrier::AttributesFacts(subject.clone()),
        Carrier::AnnotationsFacts(subject.clone()),
        Carrier::DocumentGraph,
    ] {
        graph
            .insert(Assertion::new(
                AssertionKey::new(document(), carrier, fact()).unwrap(),
            ))
            .unwrap();
    }

    assert_eq!(graph.facts().len(), 1);
    assert_eq!(graph.assertions().len(), 3);
    assert_eq!(graph.outgoing(&subject), vec![&fact()]);
    assert_eq!(
        graph.outgoing_with_predicate(&subject, &predicate("aliases")),
        vec![&fact()]
    );
}

#[test]
fn metadata_repeated_alias_same_carrier_one_assertion() {
    let key = AssertionKey::new(
        document(),
        Carrier::AttributesFacts(node("submit-order")),
        fact(),
    )
    .unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(Assertion::new(key.clone())).unwrap();
    graph.insert(Assertion::new(key)).unwrap();

    assert_eq!(graph.facts().len(), 1);
    assert_eq!(graph.assertions().len(), 1);
}

#[test]
fn metadata_node_link_is_queryable_incoming() {
    let target = node("submit-order-v2");
    let link = Fact::new(
        node("submit-order"),
        predicate("replacement"),
        ObjectTerm::NodeRef(target.clone()),
        GraphName::Default,
    );
    let mut graph = GraphIndex::new();
    graph
        .insert(Assertion::new(
            AssertionKey::new(document(), Carrier::DocumentGraph, link.clone()).unwrap(),
        ))
        .unwrap();
    assert_eq!(graph.incoming(&target), vec![&link]);
    assert!(graph.incoming(&node("other")).is_empty());
}

#[test]
fn metadata_typed_json_uri_string_is_literal() {
    let uri = node("submit-order-v2").to_string();
    let value = ObjectTerm::typed_json(
        json!({"replacement": uri.clone()}),
        predicate("target-names"),
    );
    let literal = Fact::new(
        node("submit-order"),
        predicate("target-names"),
        value,
        GraphName::Default,
    );
    let mut graph = GraphIndex::new();
    graph
        .insert(Assertion::new(
            AssertionKey::new(document(), Carrier::DocumentGraph, literal).unwrap(),
        ))
        .unwrap();
    assert!(graph.incoming(&node("submit-order-v2")).is_empty());
}

#[test]
fn metadata_named_graph_is_unsupported() {
    let named = Fact::new(
        node("submit-order"),
        predicate("aliases"),
        ObjectTerm::value(json!("placeOrder")),
        GraphName::Named(node("audit")),
    );
    let mut graph = GraphIndex::new();
    let result = graph.insert(Assertion::new(
        AssertionKey::new(document(), Carrier::DocumentGraph, named).unwrap(),
    ));
    assert_eq!(result, Err(MetadataError::NamedGraphUnsupported));
    assert!(graph.facts().is_empty());
}

#[test]
fn metadata_typed_objects_ignore_member_order_but_keep_array_order() {
    let make_fact = |value| {
        Fact::new(
            node("submit-order"),
            predicate("target-names"),
            ObjectTerm::typed_json(value, predicate("target-names")),
            GraphName::Default,
        )
    };
    let values = [
        json!({"frontend": {"gleam": "submit", "typescript": "submitOrder"}, "order": ["a", "b"]}),
        json!({"order": ["a", "b"], "frontend": {"typescript": "submitOrder", "gleam": "submit"}}),
        json!({"order": ["b", "a"], "frontend": {"gleam": "submit", "typescript": "submitOrder"}}),
    ];
    let mut graph = GraphIndex::new();
    for value in values {
        graph
            .insert(Assertion::new(
                AssertionKey::new(document(), Carrier::DocumentGraph, make_fact(value)).unwrap(),
            ))
            .unwrap();
    }
    assert_eq!(graph.facts().len(), 2);
    assert_eq!(graph.assertions().len(), 2);
}

#[test]
fn metadata_structured_fact_equality_ignores_member_order() {
    let first = Fact::new(
        node("submit-order"),
        predicate("target-names"),
        ObjectTerm::typed_json(
            serde_json::from_str(r#"{"frontend":{"gleam":"submit"},"backend":{"sql":"SUBMIT"}}"#)
                .unwrap(),
            predicate("target-names"),
        ),
        GraphName::Default,
    );
    let second = Fact::new(
        node("submit-order"),
        predicate("target-names"),
        ObjectTerm::typed_json(
            serde_json::from_str(r#"{"backend":{"sql":"SUBMIT"},"frontend":{"gleam":"submit"}}"#)
                .unwrap(),
            predicate("target-names"),
        ),
        GraphName::Default,
    );
    assert_eq!(first, second);
    assert_eq!(
        AssertionKey::new(document(), Carrier::DocumentGraph, first).unwrap(),
        AssertionKey::new(document(), Carrier::DocumentGraph, second).unwrap()
    );
}

#[test]
fn metadata_node_local_carrier_requires_subject_match() {
    let result = AssertionKey::new(document(), Carrier::AttributesFacts(node("other")), fact());
    assert_eq!(result, Err(MetadataError::CarrierSubjectMismatch));
}

#[test]
fn metadata_assertions_for_fact_keep_owners_and_carriers() {
    let mut graph = GraphIndex::new();
    let shared = fact();
    for (owner, carrier) in [
        (document(), Carrier::AttributesFacts(node("submit-order"))),
        (document(), Carrier::DocumentGraph),
        (
            DocumentId::new("orders/other.json").unwrap(),
            Carrier::DocumentGraph,
        ),
    ] {
        graph
            .insert(Assertion::new(
                AssertionKey::new(owner, carrier, shared.clone()).unwrap(),
            ))
            .unwrap();
    }
    assert_eq!(graph.assertions_for_fact(&shared).len(), 3);
    assert_eq!(graph.facts().len(), 1);
}

#[test]
fn metadata_sidecar_carrier_retains_entry_identity() {
    let subject = node("submit-order");
    let entry_point = predicate("aliases");
    let key = AssertionKey::new(
        DocumentId::new("decorations/naming.json").unwrap(),
        Carrier::Sidecar {
            target: subject,
            entry_point,
        },
        fact(),
    )
    .unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(Assertion::new(key)).unwrap();
    assert_eq!(graph.assertions().len(), 1);
    assert_eq!(graph.facts().len(), 1);
}
