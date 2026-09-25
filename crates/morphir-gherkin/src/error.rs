//! Why a document could not be read.

use std::path::PathBuf;

use crate::span::LineCol;

/// Why a document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The file could not be read from disk.
    #[error("{path}: cannot read the file: {source}")]
    Io {
        /// The path that was read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The text is not valid Gherkin, or not valid Markdown with Gherkin.
    #[error("{path}:{line}:{col}: {message}", line = position.line, col = position.col)]
    Syntax {
        /// The path the text was read from.
        path: PathBuf,
        /// Where in the file the error was found.
        position: LineCol,
        /// What was wrong.
        message: String,
    },
    /// The path is neither `*.feature` nor `*.feature.md`.
    #[error("{path}: not a .feature or .feature.md file")]
    UnknownFormat {
        /// The path that was given.
        path: PathBuf,
    },
}
