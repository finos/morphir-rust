//! Error types for the Morphir daemon

use thiserror::Error;

/// Result type for daemon operations
pub type Result<T> = std::result::Result<T, DaemonError>;

/// Errors that can occur in daemon operations
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Workspace-related errors
    #[error("Workspace error: {0}")]
    Workspace(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),

    /// Project errors
    #[error("Project error: {0}")]
    Project(String),

    /// Build errors
    #[error("Build error: {0}")]
    Build(String),

    /// Extension errors
    #[error("Extension error: {0}")]
    Extension(#[from] morphir_wit_extension::ExtensionError),

    /// IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML parsing errors
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    /// File watching errors
    #[error("Watch error: {0}")]
    Watch(#[from] notify::Error),

    /// Generic errors
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
