//! A `morphir_host::Channel` over an in-process native extension, and the
//! registry source for built-in extensions.

use crate::CheckedConnection;
use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionRequest, ExtensionResponse};
use morphir_extension_sdk::{
    ExtensionCapabilities, ExtensionInfo, NativeExtension, NativeProtocol,
};
use morphir_host::{
    CapabilityMetadataScope, Channel, ChannelCause, ChannelError, ChannelState, ExpectedChecks,
    ExpectedExtension, GuestConnection, GuestSource, HostError, InvocationMode, InvocationPolicy,
    JsonRpcConnection, Outgoing, ProviderOrigin,
};
use std::path::Path;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// A guest that runs in the host process and speaks MEP through
/// [`NativeProtocol::handle`].
///
/// `send(Notification)` is a no-op: the SDK's native protocol takes only
/// requests, so a notification such as `morphir.exit` has nothing to
/// deliver to. `close` and `abort` mark the channel stopped without
/// touching the guest; reusing a stopped channel fails with
/// `"Native extension transport is stopped"` and `Stopped`, the text the
/// CLI has always reported for a stopped built-in.
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
            cause: ChannelCause::Transport,
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
                cause: ChannelCause::Transport,
            });
        };
        handle.await.map_err(|error| ChannelError {
            message: format!("Native extension protocol worker failed: {error}"),
            state: ChannelState::Indeterminate,
            cause: ChannelCause::Transport,
        })
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        self.stopped = true;
        Ok(ChannelState::Stopped)
    }
}

/// A built-in extension as a registry source.
///
/// Its origin is `Builtin` and its capability metadata is complete. Under
/// `PreferDirect` the caller invokes the extension's typed handles through
/// [`GuestSource::native`]; under `ProtocolOnly` it connects over a
/// [`NativeChannel`], with the same checks as any other guest.
pub struct NativeSource {
    extension: NativeExtension,
    capabilities: ExtensionCapabilities,
}

impl NativeSource {
    /// Offer `extension` to a registry.
    pub fn new(extension: NativeExtension) -> Self {
        let capabilities = extension.capabilities();
        Self {
            extension,
            capabilities,
        }
    }
}

#[async_trait]
impl GuestSource for NativeSource {
    fn info(&self) -> &ExtensionInfo {
        self.extension.info()
    }

    fn capabilities(&self) -> &ExtensionCapabilities {
        &self.capabilities
    }

    fn origin(&self) -> ProviderOrigin {
        ProviderOrigin::Builtin
    }

    fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        CapabilityMetadataScope::Complete
    }

    fn invocation_mode(&self, policy: InvocationPolicy) -> InvocationMode {
        match policy {
            InvocationPolicy::PreferDirect => InvocationMode::NativeDirect,
            InvocationPolicy::ProtocolOnly => InvocationMode::NativeMep,
        }
    }

    fn native(&self) -> Option<&NativeExtension> {
        Some(&self.extension)
    }

    async fn connect(&self, _workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError> {
        let channel = NativeChannel::new(&self.extension);
        let checks = ExpectedChecks::new(channel.expectation());
        Ok(Box::new(CheckedConnection::new(JsonRpcConnection::new(
            channel, checks,
        ))))
    }
}
