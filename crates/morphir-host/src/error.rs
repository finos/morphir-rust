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

/// A failure in the MEP session, the negotiation, or the transport.
///
/// The variants that existed as daemon messages before this crate existed
/// keep the daemon's `Display` text: `VersionNotOffered`, the envelope texts
/// carried by `Invalid`, `Rpc`, and the guard texts carried by `Rejected`.
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
}

impl From<crate::channel::ChannelError> for HostError {
    fn from(error: crate::channel::ChannelError) -> Self {
        HostError::Channel {
            message: error.message,
            state: error.state,
        }
    }
}
