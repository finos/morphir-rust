//! Set-valued default-graph indexing over separately owned assertions.

use super::{Assertion, Fact, GraphName, MetadataError, ObjectTerm};
use crate::node_address::NodeUri;
use std::collections::{BTreeMap, BTreeSet};

/// An in-memory default graph with distinct facts and authored assertions.
///
/// ```
/// use morphir_core::metadata::{Assertion, AssertionKey, Carrier, DocumentId,
///     Fact, GraphIndex, GraphName, ObjectTerm};
/// use morphir_core::node_address::NodeUri;
/// use serde_json::json;
///
/// let subject = NodeUri::parse(
///     "morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/submit-order"
/// ).unwrap();
/// let predicate = NodeUri::parse(
///     "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/aliases"
/// ).unwrap();
/// let fact = Fact::new(subject.clone(), predicate, ObjectTerm::value(json!("placeOrder")),
///     GraphName::Default);
/// let owner = DocumentId::new("orders/spec.json").unwrap();
/// let key = AssertionKey::new(owner, Carrier::AttributesFacts(subject.clone()), fact).unwrap();
/// let mut graph = GraphIndex::new();
/// graph.insert(Assertion::new(key)).unwrap();
/// assert_eq!(graph.outgoing(&subject).len(), 1);
/// ```
#[derive(Debug, Default)]
pub struct GraphIndex {
    facts: Vec<Fact>,
    assertions: Vec<Assertion>,
    fact_ids: BTreeMap<String, usize>,
    assertion_ids: BTreeMap<String, usize>,
    outgoing: BTreeMap<String, BTreeSet<usize>>,
    incoming: BTreeMap<String, BTreeSet<usize>>,
    outgoing_predicate: BTreeMap<(String, String), BTreeSet<usize>>,
    fact_assertions: BTreeMap<String, BTreeSet<usize>>,
}

impl GraphIndex {
    /// Create an empty default graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one assertion, coalescing equal facts and repeated same-carrier assertions.
    ///
    /// Named graphs remain representable in [`Fact`] but are rejected here.
    pub fn insert(&mut self, assertion: Assertion) -> Result<(), MetadataError> {
        if !matches!(assertion.key().fact().graph(), GraphName::Default) {
            return Err(MetadataError::NamedGraphUnsupported);
        }
        let assertion_identity = assertion.key().identity();
        if let Some(&index) = self.assertion_ids.get(&assertion_identity) {
            for source in assertion.detail() {
                self.assertions[index].add_detail(source.clone())?;
            }
            return Ok(());
        }

        let fact = assertion.key().fact();
        let fact_identity = fact.identity();
        if !self.fact_ids.contains_key(&fact_identity) {
            let index = self.facts.len();
            self.fact_ids.insert(fact_identity.clone(), index);
            let subject = fact.subject().to_string();
            let predicate = fact.predicate().to_string();
            self.outgoing
                .entry(subject.clone())
                .or_default()
                .insert(index);
            self.outgoing_predicate
                .entry((subject, predicate))
                .or_default()
                .insert(index);
            if let ObjectTerm::NodeRef(uri) = fact.object() {
                self.incoming
                    .entry(uri.to_string())
                    .or_default()
                    .insert(index);
            }
            self.facts.push(fact.clone());
        }
        let assertion_index = self.assertions.len();
        self.assertion_ids
            .insert(assertion_identity, assertion_index);
        self.fact_assertions
            .entry(fact_identity)
            .or_default()
            .insert(assertion_index);
        self.assertions.push(assertion);
        Ok(())
    }

    /// Distinct facts in first-insertion order.
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    /// Distinct document-and-carrier assertions in first-insertion order.
    pub fn assertions(&self) -> &[Assertion] {
        &self.assertions
    }

    /// All document-and-carrier assertions for one expanded fact.
    pub fn assertions_for_fact(&self, fact: &Fact) -> Vec<&Assertion> {
        self.fact_assertions
            .get(&fact.identity())
            .into_iter()
            .flat_map(|ids| ids.iter())
            .map(|&index| &self.assertions[index])
            .collect()
    }

    /// All distinct default-graph facts whose subject is this node.
    pub fn outgoing(&self, subject: &NodeUri) -> Vec<&Fact> {
        self.get(self.outgoing.get(&subject.to_string()))
    }

    /// Outgoing facts for one expanded predicate declaration.
    pub fn outgoing_with_predicate(&self, subject: &NodeUri, predicate: &NodeUri) -> Vec<&Fact> {
        self.get(
            self.outgoing_predicate
                .get(&(subject.to_string(), predicate.to_string())),
        )
    }

    /// All distinct default-graph facts that link to this node.
    ///
    /// Strings inside typed data are never interpreted as incoming links.
    pub fn incoming(&self, object: &NodeUri) -> Vec<&Fact> {
        self.get(self.incoming.get(&object.to_string()))
    }

    fn get(&self, ids: Option<&BTreeSet<usize>>) -> Vec<&Fact> {
        ids.into_iter()
            .flat_map(|ids| ids.iter())
            .map(|&index| &self.facts[index])
            .collect()
    }
}
