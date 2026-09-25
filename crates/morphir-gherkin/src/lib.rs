//! A Gherkin document model for Morphir.
//!
//! It reads `.feature` files and `.feature.md` files (Markdown with Gherkin) into one model. Every
//! node has a source span. Descriptions keep their prose and fenced blocks as parsed Markdown.
//! Extensions read tags, fences and prose into a typed context. This crate does not run scenarios.

pub mod convert;
pub mod cursor;
pub mod error;
pub mod extension;
pub mod feature_reader;
pub mod markdown;
pub mod mdg_reader;
pub mod model;
pub mod path;
pub mod span;
pub mod visit;

use std::path::{Path, PathBuf};

pub use cursor::Cursor;
pub use error::ReadError;
pub use model::*;
pub use path::{NodePath, NodePathError, Segment};
pub use span::{LineCol, SourceText, Span};

/// Reads a `.feature` or `.feature.md` document from text. The path decides the format:
/// `*.feature.md` is Markdown with Gherkin, `*.feature` is plain Gherkin. Any other path gives
/// `ReadError::UnknownFormat`.
pub fn read_str(path: impl Into<PathBuf>, text: &str) -> Result<(Document, SourceText), ReadError> {
    let path = path.into();
    let source = SourceText::new(text);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let document = if name.ends_with(".feature.md") {
        mdg_reader::read(&path, &source)?
    } else if name.ends_with(".feature") {
        feature_reader::read(&path, &source)?
    } else {
        return Err(ReadError::UnknownFormat { path });
    };
    Ok((document, source))
}

/// Reads a `.feature` or `.feature.md` file.
pub fn read_document(path: &Path) -> Result<(Document, SourceText), ReadError> {
    let text = std::fs::read_to_string(path).map_err(|source| ReadError::Io {
        path: path.to_owned(),
        source,
    })?;
    read_str(path, &text)
}
