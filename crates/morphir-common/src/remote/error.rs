//! Error types for remote source operations.

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during remote source operations.
#[derive(Debug, Error)]
pub enum RemoteSourceError {
    /// Invalid source format
    #[error("Invalid source format: {0}")]
    InvalidFormat(String),

    /// Source not allowed by configuration
    #[error("Source not allowed by configuration: {0}")]
    NotAllowed(String),

    /// Remote sources are disabled
    #[error("Remote sources are disabled in configuration")]
    Disabled,

    /// HTTP error
    #[error("HTTP error: {status} - {message}")]
    HttpError {
        /// HTTP status code
        status: u16,
        /// Error message
        message: String,
    },

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Git error
    #[error("Git error: {0}")]
    GitError(String),

    /// Cache error
    #[error("Cache error: {0}")]
    CacheError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Source not found
    #[error("Source not found: {0}")]
    NotFound(String),

    /// Timeout error
    #[error("Timeout after {seconds}s fetching {url}")]
    Timeout {
        /// URL being fetched
        url: String,
        /// Timeout in seconds
        seconds: u64,
    },

    /// Archive extraction error
    #[error("Failed to extract archive: {0}")]
    ArchiveError(String),

    /// Path not found in archive or repository
    #[error("Path not found: {path} in {location}")]
    PathNotFound {
        /// Path that was not found
        path: String,
        /// Location where path was expected
        location: String,
    },

    /// JSON parsing error
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Cache directory creation failed
    #[error("Failed to create cache directory: {path}")]
    CacheDirectoryError {
        /// Path that could not be created
        path: PathBuf,
    },
}

/// Result type for remote source operations.
pub type Result<T> = std::result::Result<T, RemoteSourceError>;
