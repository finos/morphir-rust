//! Authored ownership and descriptive source claims.

use super::{Fact, MetadataError};
use crate::node_address::NodeUri;

/// Stable identity of the containing document, supplied by its reader.
///
/// This is deliberately separate from a [`NodeUri`]: a graph statement may
/// describe a node in another artifact while its own document retains ownership.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocumentId(String);

impl DocumentId {
    /// Create a document identity from a nonempty reader-provided ID.
    pub fn new(id: impl Into<String>) -> Result<Self, MetadataError> {
        let id = id.into();
        if id.is_empty() {
            Err(MetadataError::EmptyDocumentId)
        } else {
            Ok(Self(id))
        }
    }

    /// The reader-provided stable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The semantic container where a document authored a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Carrier {
    /// A Type or Value node's `attributes.facts` container.
    AttributesFacts(NodeUri),
    /// A specification node's `annotations.facts` container.
    AnnotationsFacts(NodeUri),
    /// The containing document's `$meta.@graph` container.
    DocumentGraph,
    /// One V3/V4 decorator sidecar entry after upstream target and value validation.
    Sidecar {
        /// The entry's addressed target node.
        target: NodeUri,
        /// The entry point declaration that supplies its projected predicate.
        entry_point: NodeUri,
    },
}

impl Carrier {
    fn identity(&self) -> String {
        match self {
            Self::AttributesFacts(uri) => serde_json::to_string(&("attributes", uri.to_string())),
            Self::AnnotationsFacts(uri) => serde_json::to_string(&("annotations", uri.to_string())),
            Self::DocumentGraph => serde_json::to_string(&("documentGraph",)),
            Self::Sidecar {
                target,
                entry_point,
            } => serde_json::to_string(&("sidecar", target.to_string(), entry_point.to_string())),
        }
        .expect("carrier identities serialize")
    }
}

/// An authored assertion's alias-independent identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionKey {
    owner: DocumentId,
    carrier: Carrier,
    fact: Fact,
}

impl AssertionKey {
    /// Bind an expanded fact to its owning document and semantic carrier.
    ///
    /// Node-local facts must use their enclosing node as subject. A document
    /// graph can assert a fact about any explicitly addressed node.
    pub fn new(owner: DocumentId, carrier: Carrier, fact: Fact) -> Result<Self, MetadataError> {
        match &carrier {
            Carrier::AttributesFacts(uri) | Carrier::AnnotationsFacts(uri)
                if uri != fact.subject() =>
            {
                Err(MetadataError::CarrierSubjectMismatch)
            }
            Carrier::Sidecar { target, .. } if target != fact.subject() => {
                Err(MetadataError::CarrierSubjectMismatch)
            }
            _ => Ok(Self {
                owner,
                carrier,
                fact,
            }),
        }
    }

    /// The document that contains this assertion.
    pub fn owner(&self) -> &DocumentId {
        &self.owner
    }

    /// The authored semantic container.
    pub fn carrier(&self) -> &Carrier {
        &self.carrier
    }

    /// The expanded graph fact.
    pub fn fact(&self) -> &Fact {
        &self.fact
    }

    pub(crate) fn identity(&self) -> String {
        serde_json::to_string(&(
            self.owner.as_str(),
            self.carrier.identity(),
            self.fact.identity(),
        ))
        .expect("assertion identities serialize")
    }
}

/// A descriptive source claim; these labels do not prove authorship.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssertionSource {
    /// Ordinary provenance from the containing document.
    Document(DocumentId),
    /// An optional compiler producer and source reference.
    Compiler {
        /// Producer name supplied by the writer.
        producer: String,
        /// Optional source location or producer reference.
        reference: Option<String>,
    },
    /// An optional human author reference.
    Author {
        /// Writer-supplied reference, not an authenticated identity.
        reference: String,
    },
}

/// One authored assertion and its optional complete source override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    key: AssertionKey,
    source_override: Option<Vec<AssertionSource>>,
}

impl Assertion {
    /// Create an assertion with ordinary document provenance only.
    pub fn new(key: AssertionKey) -> Self {
        Self {
            key,
            source_override: None,
        }
    }

    /// Its document, carrier and expanded fact identity.
    pub fn key(&self) -> &AssertionKey {
        &self.key
    }

    /// Persisted claims other than the document source.
    pub fn detail(&self) -> Vec<&AssertionSource> {
        self.source_override
            .iter()
            .flatten()
            .filter(|source| !matches!(source, AssertionSource::Document(_)))
            .collect()
    }

    /// The explicit source override, if one was recorded in the document.
    pub fn source_override(&self) -> Option<&[AssertionSource]> {
        self.source_override.as_deref()
    }

    /// Add a compiler or author claim once. Document provenance is derived
    /// solely from the assertion owner and cannot be supplied as extra detail.
    pub fn add_detail(&mut self, source: AssertionSource) -> Result<(), MetadataError> {
        if matches!(source, AssertionSource::Document(_)) {
            return Err(MetadataError::DocumentProvenanceIsImplicit);
        }
        let mut sources = self.sources();
        sources.push(source);
        self.set_source_override(sources)?;
        Ok(())
    }

    /// Sources visible to a query. The document is implicit only without an override.
    pub fn sources(&self) -> Vec<AssertionSource> {
        self.source_override
            .clone()
            .unwrap_or_else(|| vec![AssertionSource::Document(self.key.owner.clone())])
    }

    pub(crate) fn set_source_override(
        &mut self,
        mut sources: Vec<AssertionSource>,
    ) -> Result<(), MetadataError> {
        if sources.is_empty() {
            return Err(MetadataError::EmptyAssertionSources);
        }
        if sources.iter().any(|source| {
            matches!(source, AssertionSource::Document(owner) if owner != self.key.owner())
        }) {
            return Err(MetadataError::DocumentSourceOwnerMismatch);
        }
        sources.sort();
        sources.dedup();
        self.source_override = Some(sources);
        Ok(())
    }

    pub(crate) fn merge_sources(&mut self, other: &Self) -> Result<(), MetadataError> {
        if let Some(sources) = &other.source_override {
            let mut merged = self.source_override.clone().unwrap_or_default();
            merged.extend(sources.iter().cloned());
            self.set_source_override(merged)?;
        }
        Ok(())
    }

    pub(crate) fn with_key(mut self, key: AssertionKey) -> Self {
        self.key = key;
        self
    }
}

/// One optional detailed-source table row selected by an expanded assertion key.
///
/// Codecs expand aliases before constructing this row; no serialized offset or
/// compact alias participates in matching.
///
/// ```
/// use morphir_core::metadata::{Assertion, AssertionKey, AssertionSource, Carrier,
///     DocumentId, Fact, GraphIndex, GraphName, ObjectTerm, SourceRecord};
/// use morphir_core::node_address::NodeUri;
/// use serde_json::json;
///
/// let subject = NodeUri::parse(
///     "morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/submit-order"
/// ).unwrap();
/// let predicate = NodeUri::parse(
///     "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated"
/// ).unwrap();
/// let owner = DocumentId::new("orders/spec.json").unwrap();
/// let fact = Fact::new(subject, predicate, ObjectTerm::value(json!(true)), GraphName::Default);
/// let key = AssertionKey::new(owner.clone(), Carrier::DocumentGraph, fact).unwrap();
/// let source = AssertionSource::Author { reference: "review/42".into() };
/// let record = SourceRecord::new(key.clone(), vec![source.clone()]).unwrap();
/// let mut graph = GraphIndex::new();
/// graph.insert(Assertion::new(key)).unwrap();
/// graph.apply_source_records(&owner, &[record]).unwrap();
/// assert_eq!(graph.assertions()[0].sources(), vec![source]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecord {
    selector: AssertionKey,
    sources: Vec<AssertionSource>,
}

impl SourceRecord {
    /// Create a row with at least one tagged source.
    pub fn new(
        selector: AssertionKey,
        sources: Vec<AssertionSource>,
    ) -> Result<Self, MetadataError> {
        let mut assertion = Assertion::new(selector.clone());
        assertion.set_source_override(sources)?;
        Ok(Self {
            selector,
            sources: assertion.sources(),
        })
    }

    /// The owner, semantic carrier and expanded fact selected by this row.
    pub fn selector(&self) -> &AssertionKey {
        &self.selector
    }

    /// The complete source set, replacing the implicit document default.
    pub fn sources(&self) -> &[AssertionSource] {
        &self.sources
    }
}
