//! A `morphir_host::Channel` over an in-process native extension.

use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionRequest, ExtensionResponse};
use morphir_extension_sdk::{NativeExtension, NativeProtocol};
use morphir_host::{Channel, ChannelError, ChannelState, ExpectedExtension, Outgoing};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// A guest that runs in the host process and speaks MEP through
/// [`NativeProtocol::handle`].
///
/// `send(Notification)` is a no-op: the SDK's native protocol takes only
/// requests, so a notification such as `morphir.exit` has nothing to
/// deliver to. `close` and `abort` mark the channel stopped without
/// touching the guest; reusing a stopped channel fails with
/// `"Native extension transport is stopped"` and `Stopped`, matching the
/// daemon's `NativeMepTransport`.
pub struct NativeChannel {
    expectation: ExpectedExtension,
    protocol: Arc<dyn NativeProtocol>,
    pending: Option<JoinHandle<ExtensionResponse>>,
    stopped: bool,
}

impl NativeChannel {
    /// Open the extension's native protocol endpoint as a channel.
    pub fn new(extension: &NativeExtension) -> Self {
        let expectation = ExpectedExtension::discovered_with_capabilities(
            extension.info().clone(),
            extension.capabilities(),
        );
        Self {
            expectation,
            protocol: extension.open_protocol(),
            pending: None,
            stopped: false,
        }
    }

    /// What the host should expect of the guest this channel wraps.
    pub fn expectation(&self) -> ExpectedExtension {
        self.expectation.clone()
    }

    fn stopped_error() -> ChannelError {
        ChannelError {
            message: "Native extension transport is stopped".into(),
            state: ChannelState::Stopped,
        }
    }
}

#[async_trait]
impl Channel for NativeChannel {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        let request: ExtensionRequest = match message {
            Outgoing::Request(request) => request,
            Outgoing::Notification(_) => return Ok(()),
        };
        if self.stopped {
            return Err(Self::stopped_error());
        }
        let protocol = Arc::clone(&self.protocol);
        self.pending = Some(tokio::task::spawn_blocking(move || {
            protocol.handle(request)
        }));
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        if self.stopped {
            return Err(Self::stopped_error());
        }
        let Some(handle) = self.pending.take() else {
            return Err(ChannelError {
                message: "Native extension channel has no request pending".into(),
                state: ChannelState::Indeterminate,
            });
        };
        handle.await.map_err(|error| ChannelError {
            message: format!("Native extension protocol worker failed: {error}"),
            state: ChannelState::Indeterminate,
        })
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        self.stopped = true;
        Ok(ChannelState::Stopped)
    }
}
