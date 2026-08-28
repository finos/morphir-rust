//! Transport boundary and lifecycle knowledge shared by session controllers.

use crate::DaemonError;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;
use morphir_extension_sdk::ExtensionInfo;

/// The transport's knowledge after an exchange or termination attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// The transport proved that the peer cannot accept more requests.
    Stopped,
    /// The transport cannot prove whether the peer accepted the last request.
    Indeterminate,
}

/// A transport failure together with the resulting lifecycle knowledge.
#[derive(Debug)]
pub struct TransportError {
    pub(super) error: DaemonError,
    pub(super) state: TransportState,
}

impl TransportError {
    /// Record a transport failure and what is known about the peer afterwards.
    pub fn new(error: DaemonError, state: TransportState) -> Self {
        Self { error, state }
    }
}

/// Identity known before protocol negotiation.
#[derive(Debug, Clone)]
pub struct ExpectedExtension {
    pub(super) id: String,
    pub(super) discovered: Option<ExtensionInfo>,
}

impl ExpectedExtension {
    /// Expect only a stable extension identifier.
    pub fn identified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            discovered: None,
        }
    }

    /// Require initialization metadata to agree with discovery metadata.
    pub fn discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
        }
    }
}

/// Object-safe I/O boundary used by the MEP session controller.
#[async_trait]
pub trait MepTransport: Send {
    /// Return the identity or discovery record used to load the extension.
    fn expected_extension(&self) -> ExpectedExtension;

    /// Exchange one untrusted JSON-RPC request and response.
    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError>;

    /// Abort the transport after a protocol or negotiation failure.
    async fn abort(&mut self) -> std::result::Result<TransportState, TransportError> {
        self.terminate().await
    }

    /// Stop the transport after the peer acknowledges MEP shutdown.
    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError>;
}

#[async_trait]
impl<T: MepTransport + ?Sized> MepTransport for Box<T> {
    fn expected_extension(&self) -> ExpectedExtension {
        (**self).expected_extension()
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        (**self).exchange(request).await
    }

    async fn abort(&mut self) -> std::result::Result<TransportState, TransportError> {
        (**self).abort().await
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        (**self).terminate().await
    }
}
