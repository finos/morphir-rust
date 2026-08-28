//! Error types for OKF loading.
//!
//! Parse problems inside a document are data, not errors: a malformed
//! frontmatter block lands in [`crate::model::Doc::frontmatter_error`] so that
//! checks can report every problem in one pass. This enum covers only the
//! failures that stop loading altogether.

use thiserror::Error;

/// Result type alias for OKF operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that abort loading a knowledge base from disk.
#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// A directory was identified as a bundle root but its `index.md`
    /// disappeared between discovery and loading.
    #[error("{0} has no root index.md")]
    MissingRootIndex(String),
}
