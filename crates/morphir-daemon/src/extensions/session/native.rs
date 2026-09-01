//! Native in-process transport for typestate MEP sessions.

use super::{ExpectedExtension, Loaded, MepTransport, Session, TransportError, TransportState};
use crate::DaemonError;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;
use morphir_extension_sdk::NativeExtension;

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
    extension: NativeExtension,
    stopped: bool,
}

impl NativeMepTransport {
    /// Create a transport that locks the extension's discovery metadata.
    pub fn new(extension: NativeExtension) -> Self {
        let expected = ExpectedExtension::discovered_with_capabilities(
            extension.info().clone(),
            extension.capabilities().clone(),
        );
        Self {
            expected,
            extension,
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

        let extension = self.extension.clone();
        tokio::task::spawn_blocking(move || extension.protocol().handle(request))
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
