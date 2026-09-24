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

impl From<morphir_host::HostError> for DaemonError {
    fn from(error: morphir_host::HostError) -> Self {
        match error {
            morphir_host::HostError::Json(error) => DaemonError::Json(error),
            morphir_host::HostError::Io(error) => DaemonError::Io(error),
            morphir_host::HostError::Channel {
                message,
                cause: morphir_host::ChannelCause::Io | morphir_host::ChannelCause::Json,
                ..
            } => DaemonError::Other(anyhow::anyhow!(message)),
            other => DaemonError::Extension(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_host::{ChannelCause, ChannelState, HostError};

    #[test]
    fn a_channel_error_with_an_io_cause_prints_unchanged() {
        let error = HostError::Channel {
            message: "IO error: broken pipe".into(),
            state: ChannelState::Stopped,
            cause: ChannelCause::Io,
        };

        assert_eq!(
            DaemonError::from(error).to_string(),
            "IO error: broken pipe"
        );
    }

    #[test]
    fn a_channel_error_with_a_json_cause_prints_unchanged() {
        let error = HostError::Channel {
            message: "JSON error: unexpected end of input".into(),
            state: ChannelState::Stopped,
            cause: ChannelCause::Json,
        };

        assert_eq!(
            DaemonError::from(error).to_string(),
            "JSON error: unexpected end of input"
        );
    }

    #[test]
    fn a_channel_error_with_a_transport_cause_keeps_the_extension_prefix() {
        let error = HostError::Channel {
            message: "IO error: broken pipe".into(),
            state: ChannelState::Stopped,
            cause: ChannelCause::Transport,
        };

        assert_eq!(
            DaemonError::from(error).to_string(),
            "Extension error: IO error: broken pipe"
        );
    }
}
