//! Errors reported by the host library.

use morphir_extension_sdk::protocol::RpcError;

/// What a transport failure proves about the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// The guest cannot accept more requests.
    Stopped,
    /// The host cannot prove whether the guest accepted the last request.
    Indeterminate,
}

/// What a transport failure began as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ChannelCause {
    /// The transport itself failed: a timeout, a closed pipe the host
    /// detected, a stopped guest.
    #[default]
    Transport,
    /// An I/O error. The message starts with `IO error: `.
    Io,
    /// A JSON encoding or decoding error. The message starts with `JSON error: `.
    Json,
}

impl ChannelCause {
    /// What `error` began as, before it became a channel failure.
    pub fn of(error: &HostError) -> Self {
        match error {
            HostError::Io(_) => ChannelCause::Io,
            HostError::Json(_) => ChannelCause::Json,
            HostError::Channel { cause, .. } => *cause,
            _ => ChannelCause::Transport,
        }
    }
}

/// A failure in the MEP session, the negotiation, or the transport.
///
/// The variants that existed as daemon messages before this crate existed
/// keep the daemon's `Display` text: `VersionNotOffered`, the envelope texts
/// carried by `Invalid`, `Rpc`, the guard texts carried by `Rejected`, and
/// `Io`, which keeps the daemon's `"IO error: {0}"` text so error strings
/// stay the same across the `From<HostError> for DaemonError` boundary.
/// Callers and tests match on these messages. `State` is new to this crate
/// and has no daemon precedent.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The guest chose a protocol version that the host did not offer.
    #[error("Extension selected protocol version '{selected}' that the host did not offer")]
    VersionNotOffered {
        /// The version the guest chose.
        selected: String,
        /// The versions the host offered.
        offered: Vec<String>,
    },
    /// The guest broke the protocol. The session cannot be trusted.
    #[error("{0}")]
    Invalid(String),
    /// The guest answered with a JSON-RPC error.
    #[error("RPC error {}: {}", .0.code, .0.message)]
    Rpc(RpcError),
    /// The host refused a call before it reached the guest.
    #[error("{0}")]
    Rejected(String),
    /// The transport failed.
    #[error("{message}")]
    Channel {
        /// What went wrong.
        message: String,
        /// What the failure proves about the guest.
        state: ChannelState,
        /// What the failure began as.
        cause: ChannelCause,
    },
    /// The session is in a state that does not allow the operation.
    #[error("Session cannot {action} while {state}")]
    State {
        /// The operation that was refused.
        action: &'static str,
        /// The state of the session.
        state: &'static str,
    },
    /// A value could not be encoded or decoded.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// An I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<crate::channel::ChannelError> for HostError {
    fn from(error: crate::channel::ChannelError) -> Self {
        HostError::Channel {
            message: error.message,
            state: error.state,
            cause: error.cause,
        }
    }
}
