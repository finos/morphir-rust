//! Storage-neutral Morphir IR layouts.

mod document_tree;
mod single_file;

pub use document_tree::{read_document_tree, write_document_tree};
pub use single_file::{ClassicV3ModuleVisitor, visit_classic_v3};
