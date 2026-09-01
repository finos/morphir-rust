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
    Extension(String),

    /// An extension session ended and can no longer serve operations.
    ///
    /// This is distinct from an extension rejecting one operation: the session
    /// itself is gone, so retrying on the same handle can never succeed. A
    /// caller holding a cached session handle should discard it and start a new
    /// session. The boxed cause keeps the original failure's variant, so an
    /// `Io` transport failure is still recognisably an `Io` failure.
    #[error("Extension session is no longer available: {0}")]
    SessionLost(#[source] Box<DaemonError>),

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
