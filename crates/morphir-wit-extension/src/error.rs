//! Error types for the Morphir extension system

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

    /// Invalid extension manifest
    #[error("Invalid extension manifest: {0}")]
    InvalidManifest(String),

    /// WASM runtime error
    #[error("WASM runtime error: {0}")]
    WasmRuntime(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic error
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
