//! Pure, expanded linked-metadata facts and their authored assertions.
//!
//! This module models the default graph after context expansion. It does not
//! interpret authored keys, resolve declarations, or load documents.

mod assertion;
mod graph;
mod term;

pub use assertion::{Assertion, AssertionKey, AssertionSource, Carrier, DocumentId};
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
    /// Named graph identity is modeled but cannot be inserted into this executor.
    #[error("named graphs are not supported by the first metadata graph executor")]
    NamedGraphUnsupported,
}
