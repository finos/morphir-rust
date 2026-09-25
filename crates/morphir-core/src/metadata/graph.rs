//! Set-valued default-graph indexing over separately owned assertions.

use super::{
    Assertion, AssertionKey, AssertionSource, Carrier, DocumentId, Fact, GraphName, MetadataError,
    ObjectTerm, SourceRecord,
};
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

/// A failed graph-wide URI binding; the input graph remains unchanged.
#[derive(Debug, thiserror::Error)]
pub enum GraphMapError<E> {
    #[error("node URI binding failed: {0}")]
    Map(E),
    #[error(transparent)]
    Model(#[from] MetadataError),
}

impl GraphIndex {
    /// Create an empty default graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one assertion, coalescing equal facts and repeated same-carrier assertions.
    ///
    /// A duplicate key with mixed explicit and implicit source knowledge is
    /// rejected because merging it could hide an unknown contributor. Named
    /// graphs remain representable in [`Fact`] but are rejected here.
    pub fn insert(&mut self, assertion: Assertion) -> Result<(), MetadataError> {
        if !matches!(assertion.key().fact().graph(), GraphName::Default) {
            return Err(MetadataError::NamedGraphUnsupported);
        }
        let assertion_identity = assertion.key().identity();
        if let Some(&index) = self.assertion_ids.get(&assertion_identity) {
            self.assertions[index].merge_sources(&assertion)?;
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

    /// Rebind every addressed term and carrier in one transaction. Source
    /// detail travels with its assertion, so a published selector cannot be
    /// left referring to the authoring identity. Literal data is never scanned
    /// for URI-looking strings.
    pub fn try_map_node_uris<E>(
        &self,
        mut map: impl FnMut(&NodeUri) -> Result<NodeUri, E>,
    ) -> Result<Self, GraphMapError<E>> {
        let mut output = Self::new();
        for assertion in &self.assertions {
            let fact = assertion.key().fact();
            let object = match fact.object() {
                ObjectTerm::NodeRef(uri) => {
                    ObjectTerm::NodeRef(map(uri).map_err(GraphMapError::Map)?)
                }
                ObjectTerm::Value(value) => match value.datatype() {
                    Some(datatype) => ObjectTerm::typed_json(
                        value.value().clone(),
                        map(datatype).map_err(GraphMapError::Map)?,
                    ),
                    None => ObjectTerm::value(value.value().clone()),
                },
            };
            let graph = match fact.graph() {
                GraphName::Default => GraphName::Default,
                GraphName::Named(uri) => GraphName::Named(map(uri).map_err(GraphMapError::Map)?),
            };
            let mapped_fact = Fact::new(
                map(fact.subject()).map_err(GraphMapError::Map)?,
                map(fact.predicate()).map_err(GraphMapError::Map)?,
                object,
                graph,
            );
            let carrier = match assertion.key().carrier() {
                Carrier::AttributesFacts(uri) => {
                    Carrier::AttributesFacts(map(uri).map_err(GraphMapError::Map)?)
                }
                Carrier::AnnotationsFacts(uri) => {
                    Carrier::AnnotationsFacts(map(uri).map_err(GraphMapError::Map)?)
                }
                Carrier::DocumentGraph => Carrier::DocumentGraph,
                Carrier::Sidecar {
                    target,
                    entry_point,
                } => Carrier::Sidecar {
                    target: map(target).map_err(GraphMapError::Map)?,
                    entry_point: map(entry_point).map_err(GraphMapError::Map)?,
                },
            };
            let key = AssertionKey::new(assertion.key().owner().clone(), carrier, mapped_fact)
                .map_err(GraphMapError::Model)?;
            output
                .insert(assertion.clone().with_key(key))
                .map_err(GraphMapError::Model)?;
        }
        Ok(output)
    }

    /// Apply one document's optional detailed source table after all selectors validate.
    ///
    /// Selectors use the already-expanded assertion identity. Any invalid row
    /// rejects the entire table without changing source ownership.
    pub fn apply_source_records(
        &mut self,
        owner: &DocumentId,
        records: &[SourceRecord],
    ) -> Result<(), MetadataError> {
        let mut seen = BTreeSet::new();
        let mut matches = Vec::with_capacity(records.len());
        for record in records {
            if record.selector().owner() != owner {
                return Err(MetadataError::SourceSelectorOwnerMismatch(Box::new(
                    record.selector().clone(),
                )));
            }
            if matches!(record.selector().carrier(), Carrier::Sidecar { .. }) {
                return Err(MetadataError::SourceSelectorCarrierUnsupported(Box::new(
                    record.selector().clone(),
                )));
            }
            let identity = record.selector().identity();
            if !seen.insert(identity.clone()) {
                return Err(MetadataError::DuplicateSourceSelector(Box::new(
                    record.selector().clone(),
                )));
            }
            let Some(&index) = self.assertion_ids.get(&identity) else {
                return Err(MetadataError::UnmatchedSourceSelector(Box::new(
                    record.selector().clone(),
                )));
            };
            matches.push((index, record.sources()));
        }
        for (index, sources) in matches {
            self.assertions[index].set_source_override(sources.to_vec())?;
        }
        Ok(())
    }

    /// Replace one authored fact and its selector in one graph update.
    ///
    /// Existing explicit sources move with the assertion. A failed replacement
    /// leaves both the graph and its source table association untouched.
    pub fn rewrite_assertion(
        &mut self,
        old: &AssertionKey,
        replacement: Fact,
    ) -> Result<(), MetadataError> {
        let Some(&index) = self.assertion_ids.get(&old.identity()) else {
            return Err(MetadataError::AssertionNotFound);
        };
        let key = AssertionKey::new(old.owner().clone(), old.carrier().clone(), replacement)?;
        if key.identity() != old.identity() && self.assertion_ids.contains_key(&key.identity()) {
            return Err(MetadataError::AssertionCollision(Box::new(key)));
        }
        let mut assertions = self.assertions.clone();
        assertions[index] = assertions[index].clone().with_key(key);
        *self = Self::from_assertions(assertions)?;
        Ok(())
    }

    /// Remove a known source's contribution to one document's assertions.
    ///
    /// An assertion with only the implicit document source has unknown detailed
    /// ownership, so source-dependent removal fails before changing anything.
    /// Assertions without remaining sources are removed from the graph.
    pub fn remove_source_from_owner(
        &mut self,
        owner: &DocumentId,
        source: &AssertionSource,
    ) -> Result<usize, MetadataError> {
        if self.assertions.iter().any(|assertion| {
            assertion.key().owner() == owner && assertion.source_override().is_none()
        }) {
            return Err(MetadataError::UnknownSourceOwnership);
        }
        let mut removed = 0;
        let assertions = self
            .assertions
            .iter()
            .filter_map(|assertion| {
                if assertion.key().owner() != owner || !assertion.sources().contains(source) {
                    return Some(assertion.clone());
                }
                removed += 1;
                let remaining = assertion
                    .sources()
                    .into_iter()
                    .filter(|candidate| candidate != source)
                    .collect::<Vec<_>>();
                if remaining.is_empty() {
                    None
                } else {
                    let mut next = assertion.clone();
                    next.set_source_override(remaining)
                        .expect("removing one known source keeps valid owner sources");
                    Some(next)
                }
            })
            .collect();
        *self = Self::from_assertions(assertions)?;
        Ok(removed)
    }

    fn from_assertions(assertions: Vec<Assertion>) -> Result<Self, MetadataError> {
        let mut graph = Self::new();
        for assertion in assertions {
            graph.insert(assertion)?;
        }
        Ok(graph)
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
