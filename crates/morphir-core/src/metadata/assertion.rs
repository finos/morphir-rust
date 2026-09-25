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

/// One authored assertion and its optional extra source claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    key: AssertionKey,
    detail: Vec<AssertionSource>,
}

impl Assertion {
    /// Create an assertion with ordinary document provenance only.
    pub fn new(key: AssertionKey) -> Self {
        Self {
            key,
            detail: Vec::new(),
        }
    }

    /// Its document, carrier and expanded fact identity.
    pub fn key(&self) -> &AssertionKey {
        &self.key
    }

    /// Additional persisted claims, excluding implicit document provenance.
    pub fn detail(&self) -> &[AssertionSource] {
        &self.detail
    }

    /// Add a compiler or author claim once. Document provenance is derived
    /// solely from the assertion owner and cannot be supplied as extra detail.
    pub fn add_detail(&mut self, source: AssertionSource) -> Result<(), MetadataError> {
        if matches!(source, AssertionSource::Document(_)) {
            return Err(MetadataError::DocumentProvenanceIsImplicit);
        }
        if !self.detail.contains(&source) {
            self.detail.push(source);
            self.detail.sort();
        }
        Ok(())
    }

    /// Sources visible to a query, including the containing document.
    pub fn sources(&self) -> Vec<AssertionSource> {
        let mut sources = vec![AssertionSource::Document(self.key.owner.clone())];
        for source in &self.detail {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
        sources
    }
}
