use std::path::PathBuf;

use crate::span::LineCol;

/// Why a document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{path}: cannot read the file: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}:{line}:{col}: {message}", line = position.line, col = position.col)]
    Syntax {
        path: PathBuf,
        position: LineCol,
        message: String,
    },
    #[error("{path}: not a .feature or .feature.md file")]
    UnknownFormat { path: PathBuf },
}
