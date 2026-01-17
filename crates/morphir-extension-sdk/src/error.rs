//! Error types for the Morphir extension SDK

use thiserror::Error;

/// Result type for extension operations
pub type Result<T> = std::result::Result<T, ExtensionError>;

/// Errors that can occur in extension operations
#[derive(Debug, Error)]
pub enum ExtensionError {
    /// Extension not found
    #[error("Extension not found: {0}")]
    NotFound(String),

    /// Extension loading failed
    #[error("Failed to load extension: {0}")]
    LoadFailed(String),

    /// Extension initialization failed
    #[error("Failed to initialize extension: {0}")]
    InitFailed(String),

    /// Extension does not support required capability
    #[error("Extension '{extension}' does not support capability: {capability}")]
    UnsupportedCapability {
        extension: String,
        capability: String,
    },

    /// Extension execution failed
    #[error("Extension execution failed: {0}")]
    ExecutionFailed(String),

    /// Invalid response from extension
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl ExtensionError {
    /// Create a new execution error
    pub fn execution(msg: impl Into<String>) -> Self {
        ExtensionError::ExecutionFailed(msg.into())
    }

    /// Create a new invalid response error
    pub fn invalid_response(msg: impl Into<String>) -> Self {
        ExtensionError::InvalidResponse(msg.into())
    }
}
