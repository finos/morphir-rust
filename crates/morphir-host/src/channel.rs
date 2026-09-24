//! Moving MEP messages between the host and one guest.

use crate::{ChannelState, MaybeSend};
use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionNotification, ExtensionRequest, ExtensionResponse};

/// One message the host sends to a guest.
#[derive(Debug, Clone)]
pub enum Outgoing {
    /// A request that expects an answer.
    Request(ExtensionRequest),
    /// A notification that expects no answer, such as `morphir.exit`.
    Notification(ExtensionNotification),
}

/// A transport failure, and what it proves about the guest.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ChannelError {
    /// What went wrong.
    pub message: String,
    /// What the failure proves about the guest.
    pub state: ChannelState,
}

/// Moves MEP messages between the host and one guest.
///
/// A channel knows no MEP method names. Process stdio, HTTP, and a browser
/// worker are channels.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Channel: MaybeSend {
    /// Deliver one message.
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError>;

    /// Wait for the answer to the last request.
    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError>;

    /// Release the guest after shutdown, or abort it after a failure.
    async fn close(&mut self) -> Result<ChannelState, ChannelError>;
}
