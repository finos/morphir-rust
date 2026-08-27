//! Extism transport for typestate MEP sessions.

use super::{ExpectedExtension, Loaded, MepTransport, Session, TransportError, TransportState};
use crate::extensions::ExtensionContainer;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;

/// Factory for Extism-backed typestate sessions.
pub struct ExtismSession;

impl ExtismSession {
    /// Create a loaded MEP session around an Extism container.
    pub fn connect(container: ExtensionContainer) -> Session<ExtismTransport, Loaded> {
        Session::loaded(ExtismTransport { container })
    }
}

/// Extism implementation of the object-safe MEP transport.
pub struct ExtismTransport {
    container: ExtensionContainer,
}

#[async_trait]
impl MepTransport for ExtismTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        ExpectedExtension::discovered(self.container.info().clone())
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        let bytes = serde_json::to_vec(&request)
            .map_err(|error| TransportError::new(error.into(), TransportState::Indeterminate))?;
        let output = self
            .container
            .call_raw("handle", &bytes)
            .await
            .map_err(|error| TransportError::new(error, TransportState::Indeterminate))?;
        serde_json::from_slice(&output)
            .map_err(|error| TransportError::new(error.into(), TransportState::Indeterminate))
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        Ok(TransportState::Stopped)
    }
}
