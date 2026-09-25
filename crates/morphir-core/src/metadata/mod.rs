//! Pure, expanded linked-metadata facts and their authored assertions.
//!
//! This module models the default graph after context expansion. It does not
//! interpret authored keys, resolve declarations, or load documents.

mod assertion;
mod context;
mod graph;
mod term;

pub use assertion::{Assertion, AssertionKey, AssertionSource, Carrier, DocumentId, SourceRecord};
pub use context::{
    Coercion, ContextError, ContextResources, EffectiveContext, ExpandedKey, expand_object,
    resolve_context,
};
pub use graph::GraphIndex;
pub use term::{Fact, GraphName, ObjectTerm, TypedValue};

/// A metadata model or first-increment graph-execution error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataError {
    /// A document identity cannot be empty.
    #[error("document identity must not be empty")]
    EmptyDocumentId,
    /// A node-local carrier must describe its enclosing node.
    #[error("node-local carrier does not match the fact subject")]
    CarrierSubjectMismatch,
    /// Document provenance comes only from the assertion owner.
    #[error("document provenance is implicit and cannot be added as detail")]
    DocumentProvenanceIsImplicit,
    /// An explicit source list must contain at least one source.
    #[error("assertion source override must not be empty")]
    EmptyAssertionSources,
    /// A document source tag must name the assertion owner.
    #[error("document source does not match the assertion owner")]
    DocumentSourceOwnerMismatch,
    /// One table must not select the same assertion twice.
    #[error("duplicate assertion source selector: {0:?}")]
    DuplicateSourceSelector(Box<AssertionKey>),
    /// The selector must name an assertion authored by this document.
    #[error("assertion source selector has no matching authored assertion: {0:?}")]
    UnmatchedSourceSelector(Box<AssertionKey>),
    /// The table's owner and the selector's owner must agree.
    #[error("assertion source selector names another document: {0:?}")]
    SourceSelectorOwnerMismatch(Box<AssertionKey>),
    /// Document-level source tables cannot override sidecar assertions.
    #[error("assertion source selector cannot name a sidecar: {0:?}")]
    SourceSelectorCarrierUnsupported(Box<AssertionKey>),
    /// The old assertion must exist before an atomic rewrite.
    #[error("assertion to rewrite was not found")]
    AssertionNotFound,
    /// Source-dependent edits require explicit ownership for every assertion in scope.
    #[error("assertion source ownership is unknown")]
    UnknownSourceOwnership,
    /// Named graph identity is modeled but cannot be inserted into this executor.
    #[error("named graphs are not supported by the first metadata graph executor")]
    NamedGraphUnsupported,
}
