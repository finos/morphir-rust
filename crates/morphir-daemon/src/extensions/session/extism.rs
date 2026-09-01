//! Extism transport for typestate MEP sessions.

use super::{
    ExpectedExtension, Loaded, MepTransport, PersistedExtensionCapabilities, Session,
    TransportError, TransportState,
};
use crate::extensions::ExtensionContainer;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};

/// Factory for Extism-backed typestate sessions.
pub struct ExtismSession;

impl ExtismSession {
    /// Create a loaded MEP session around an Extism container.
    pub fn connect(container: ExtensionContainer) -> Session<ExtismTransport, Loaded> {
        Session::loaded(ExtismTransport {
            container,
            locked_extension: None,
        })
    }
}

/// Extism implementation of the object-safe MEP transport.
pub struct ExtismTransport {
    container: ExtensionContainer,
    locked_extension: Option<ExpectedExtension>,
}

impl ExtismTransport {
    /// Attach exact installed identity and complete capabilities to a container.
    pub fn new_with_expected_capabilities(
        container: ExtensionContainer,
        info: ExtensionInfo,
        capabilities: ExtensionCapabilities,
    ) -> Self {
        Self {
            container,
            locked_extension: Some(ExpectedExtension::discovered_with_capabilities(
                info,
                capabilities,
            )),
        }
    }

    /// Attach installed identity and persisted capability members to a container.
    pub fn new_with_persisted_capabilities(
        container: ExtensionContainer,
        info: ExtensionInfo,
        capabilities: PersistedExtensionCapabilities,
    ) -> Self {
        Self {
            container,
            locked_extension: Some(ExpectedExtension::discovered_with_persisted_capabilities(
                info,
                capabilities,
            )),
        }
    }

    pub(crate) fn new_with_legacy_expected_extension(
        container: ExtensionContainer,
        info: ExtensionInfo,
    ) -> Self {
        Self {
            container,
            locked_extension: Some(ExpectedExtension::legacy_discovered(info)),
        }
    }
}

#[async_trait]
impl MepTransport for ExtismTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.locked_extension
            .clone()
            .unwrap_or_else(|| ExpectedExtension::discovered(self.container.info().clone()))
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
