//! Storage-neutral Morphir IR layouts.

mod codec;
mod diagnostic;
mod document_tree;
mod json;
mod options;
mod pipeline;
mod single_file;
mod yaml;

pub use codec::{CodecRegistry, EventSink, EventSource, IrCodec};
pub use diagnostic::{Severity, SourceSpan, Stage, TransportDiagnostic};
pub use document_tree::{read_document_tree, write_document_tree};
pub use json::JsonCodec;
pub use options::{
    CodecOptions, FormatId, IdentifierError, IrVersion, Layout, NormalizationPolicy, VocabularyId,
};
pub use pipeline::{EventTransform, Retention};
pub use single_file::{ClassicV3ModuleVisitor, visit_classic_v3};
pub use yaml::YamlCodec;
