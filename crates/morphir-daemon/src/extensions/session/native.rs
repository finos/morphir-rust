//! Native in-process transport for typestate MEP sessions.

use super::{ExpectedExtension, Loaded, MepTransport, Session, TransportError, TransportState};
use crate::DaemonError;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;
use morphir_extension_sdk::{NativeExtension, NativeProtocol};
use std::sync::Arc;

/// Factory for native extension typestate sessions.
pub struct NativeMepSession;

impl NativeMepSession {
    /// Create a loaded MEP session over an in-process native extension.
    pub fn connect(extension: NativeExtension) -> Session<NativeMepTransport, Loaded> {
        Session::loaded(NativeMepTransport::new(extension))
    }
}

/// In-process implementation of the object-safe MEP transport.
pub struct NativeMepTransport {
    expected: ExpectedExtension,
    /// This session's own protocol endpoint: clones of one provider share
    /// handlers, not lifecycle, so sessions over it do not interfere.
    protocol: Arc<dyn NativeProtocol>,
    stopped: bool,
}

impl NativeMepTransport {
    /// Create a transport that locks the extension's discovery metadata.
    pub fn new(extension: NativeExtension) -> Self {
        let expected = ExpectedExtension::discovered_with_capabilities(
            extension.info().clone(),
            extension.capabilities(),
        );
        Self {
            expected,
            protocol: extension.open_protocol(),
            stopped: false,
        }
    }
}

#[async_trait]
impl MepTransport for NativeMepTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.expected.clone()
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        if self.stopped {
            return Err(TransportError::new(
                DaemonError::Extension("Native extension transport is stopped".into()),
                TransportState::Stopped,
            ));
        }

        let protocol = Arc::clone(&self.protocol);
        tokio::task::spawn_blocking(move || protocol.handle(request))
            .await
            .map_err(|error| {
                TransportError::new(
                    DaemonError::Extension(format!(
                        "Native extension protocol worker failed: {error}"
                    )),
                    TransportState::Indeterminate,
                )
            })
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        self.stopped = true;
        Ok(TransportState::Stopped)
    }
}
