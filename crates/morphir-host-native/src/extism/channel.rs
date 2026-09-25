//! A `morphir_host::Channel` over an Extism-hosted WebAssembly guest.

use crate::extism::ExtensionContainer;
use async_trait::async_trait;
use morphir_extension_sdk::protocol::ExtensionResponse;
use morphir_host::{
    Channel, ChannelCause, ChannelError, ChannelState, ExpectedExtension, HostError, Outgoing,
};

/// A guest that runs inside an Extism plugin and speaks MEP through its
/// exported `handle` function.
///
/// `close` and `abort` do not unload the plugin: they report `Stopped`
/// without touching the container. Every transport failure, including a
/// malformed response, is `Indeterminate`: the container cannot prove the
/// guest released its state.
pub struct ExtismChannel {
    container: ExtensionContainer,
    expected: ExpectedExtension,
    response: Option<Vec<u8>>,
}

impl ExtismChannel {
    /// Wrap a loaded container with what the host expects of its guest.
    pub fn new(container: ExtensionContainer, expected: ExpectedExtension) -> Self {
        Self {
            container,
            expected,
            response: None,
        }
    }

    /// What the host should expect of the guest this channel wraps.
    pub fn expectation(&self) -> ExpectedExtension {
        self.expected.clone()
    }
}

fn indeterminate(error: HostError) -> ChannelError {
    let cause = ChannelCause::of(&error);
    ChannelError {
        message: error.to_string(),
        state: ChannelState::Indeterminate,
        cause,
    }
}

fn indeterminate_message(message: impl Into<String>) -> ChannelError {
    ChannelError {
        message: message.into(),
        state: ChannelState::Indeterminate,
        cause: ChannelCause::Transport,
    }
}

#[async_trait]
impl Channel for ExtismChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        let request = match message {
            Outgoing::Request(request) => request,
            // The SDK's Extism guests answer only requests through `handle`;
            // a notification has nowhere to go.
            Outgoing::Notification(_) => return Ok(()),
        };
        let bytes =
            serde_json::to_vec(&request).map_err(|error| indeterminate(HostError::from(error)))?;
        let output = self
            .container
            .call_raw("handle", &bytes)
            .await
            .map_err(indeterminate)?;
        self.response = Some(output);
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        let output = self.response.take().ok_or_else(|| {
            indeterminate_message("Extism extension channel has no response ready")
        })?;
        serde_json::from_slice(&output).map_err(|error| indeterminate(HostError::from(error)))
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        Ok(ChannelState::Stopped)
    }
}
