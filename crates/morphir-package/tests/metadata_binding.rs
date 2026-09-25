use morphir_core::metadata::{
    Assertion, AssertionKey, AssertionSource, Carrier, DocumentId, Fact, GraphIndex, GraphName,
    ObjectTerm,
};
use morphir_core::node_address::{ArtifactRevision, NodeUri, Sha256Digest};
use morphir_package::authoring::{AuthoredLibrary, PublicationBindings};
use serde_json::{Value, json};

fn ir(package: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "formatVersion":"4.1.0",
        "distribution":{"Library":{"packageName":package,"dependencies":{},
            "def":{"modules":{"api":{"Public":{"types":{},"values":{}}}}}}}
    }))
    .unwrap()
}

fn library() -> AuthoredLibrary {
    AuthoredLibrary::create(
        &serde_json::to_vec(
            &json!({"packagePath":"example.com/orders","version":"1.0.0",
            "dependencies":{},"exports":{}}),
        )
        .unwrap(),
        &ir("acme/orders"),
    )
    .unwrap()
}

fn uri(package: &str) -> NodeUri {
    NodeUri::parse(&format!(
        "morphir://ir/pkg/{package}?format=4.1.0#/module/api"
    ))
    .unwrap()
}

#[test]
fn publication_keeps_archive_self_and_pins_exact_external_provider() {
    let mut bindings = PublicationBindings::new(&library()).unwrap();
    let external = ir("acme/metadata");
    let digest = Sha256Digest::from_bytes(&external);
    bindings.add_v4_provider(&external, &digest).unwrap();
    assert_eq!(
        bindings.bind_uri(&uri("acme/orders")).unwrap(),
        uri("acme/orders")
    );
    let bound = bindings.bind_uri(&uri("acme/metadata")).unwrap();
    assert_eq!(bound.revision(), &ArtifactRevision::Pinned(digest));
    assert!(bindings.bind_uri(&uri("acme/missing")).is_err());
    assert!(
        bindings
            .add_v4_provider(&external, &Sha256Digest::from_bytes(b"wrong"))
            .is_err()
    );
}

#[test]
fn publication_remaps_source_selectors_with_facts_and_leaves_json_text() {
    let mut bindings = PublicationBindings::new(&library()).unwrap();
    let external = ir("acme/metadata");
    bindings
        .add_v4_provider(&external, &Sha256Digest::from_bytes(&external))
        .unwrap();
    let owner = DocumentId::new("acme/orders:api").unwrap();
    let subject = uri("acme/orders");
    let predicate = uri("acme/metadata");
    let text = predicate.to_string();
    let fact = Fact::new(
        subject.clone(),
        predicate,
        ObjectTerm::value(json!(text)),
        GraphName::Default,
    );
    let key = AssertionKey::new(owner.clone(), Carrier::AttributesFacts(subject), fact).unwrap();
    let mut assertion = Assertion::new(key);
    assertion
        .add_detail(AssertionSource::Author {
            reference: "author:damian".into(),
        })
        .unwrap();
    let mut graph = GraphIndex::new();
    graph.insert(assertion).unwrap();

    let bound = bindings.bind_graph(&graph).unwrap();
    assert_eq!(bound.facts().len(), 1);
    assert_eq!(bound.assertions().len(), 1);
    assert_eq!(bound.assertions()[0].detail().len(), 1);
    assert_eq!(
        bound.facts()[0].predicate().revision(),
        &ArtifactRevision::Pinned(Sha256Digest::from_bytes(&external))
    );
    let ObjectTerm::Value(value) = bound.facts()[0].object() else {
        panic!("typed data must stay data")
    };
    assert_eq!(value.value(), &Value::String(text));
}

#[test]
fn publication_rejects_wrong_revision_and_false_self_binding() {
    let mut bindings = PublicationBindings::new(&library()).unwrap();
    let external = ir("acme/metadata");
    bindings
        .add_v4_provider(&external, &Sha256Digest::from_bytes(&external))
        .unwrap();
    let wrong = NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/metadata?format=4.1.0&rev={}#/module/api",
        Sha256Digest::from_bytes(b"wrong"),
    ))
    .unwrap();
    assert!(bindings.bind_uri(&wrong).is_err());
    let false_self =
        NodeUri::parse("morphir://ir/pkg/acme/orders?format=4.1.0#/module/missing").unwrap();
    assert!(bindings.bind_uri(&false_self).is_err());
}
