//! A Gherkin document model for Morphir.
//!
//! It reads `.feature` files and `.feature.md` files (Markdown with Gherkin) into one model. Every
//! node has a source span. Descriptions keep their prose and fenced blocks as parsed Markdown.
//! Extensions read tags, fences and prose into a typed context. This crate does not run scenarios.
//!
//! ```
//! use morphir_gherkin::read_str;
//!
//! let text = "Feature: Greeting\n  Scenario: Say hello\n    Given a name\n    Then a greeting\n";
//! let (doc, _source) = read_str("greeting.feature", text).unwrap();
//! assert_eq!(doc.feature.unwrap().name, "Greeting");
//! ```

#![warn(missing_docs)]

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
///
/// ```
/// use morphir_gherkin::read_str;
///
/// let (doc, _source) = read_str("f.feature", "Feature: F\n  Scenario: S\n    Given a step\n")
///     .unwrap();
/// assert_eq!(doc.feature.unwrap().scenarios[0].name, "S");
///
/// let error = read_str("f.txt", "not gherkin").unwrap_err();
/// assert!(error.to_string().contains("not a .feature or .feature.md file"));
/// ```
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
///
/// ```
/// use std::io::Write;
///
/// use morphir_gherkin::read_document;
///
/// let mut path = std::env::temp_dir();
/// path.push(format!("morphir-gherkin-doctest-{}.feature", std::process::id()));
/// std::fs::File::create(&path)
///     .unwrap()
///     .write_all(b"Feature: F\n  Scenario: S\n    Given a step\n")
///     .unwrap();
///
/// let (doc, _source) = read_document(&path).unwrap();
/// assert_eq!(doc.feature.unwrap().name, "F");
///
/// std::fs::remove_file(&path).unwrap();
/// ```
pub fn read_document(path: &Path) -> Result<(Document, SourceText), ReadError> {
    let text = std::fs::read_to_string(path).map_err(|source| ReadError::Io {
        path: path.to_owned(),
        source,
    })?;
    read_str(path, &text)
}
