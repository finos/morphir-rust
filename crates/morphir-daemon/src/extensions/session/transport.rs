//! Transport boundary and lifecycle knowledge shared by session controllers.

use crate::DaemonError;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse};
use async_trait::async_trait;
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, FrontendCapability,
};

/// Capability members persisted by installed-extension metadata.
///
/// The distribution manifest currently persists frontend and backend metadata,
/// but not every member of [`ExtensionCapabilities`]. Negotiation locks these
/// persisted members while validating other declared members normally. A
/// `None` member was not persisted and remains unlocked during negotiation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedExtensionCapabilities {
    frontend: Option<FrontendCapability>,
    backend: Option<BackendCapability>,
}

impl PersistedExtensionCapabilities {
    /// Create a persisted capability expectation from its stored members.
    pub fn new(frontend: Option<FrontendCapability>, backend: Option<BackendCapability>) -> Self {
        Self { frontend, backend }
    }

    /// Return the persisted frontend member, when present.
    pub fn frontend(&self) -> Option<&FrontendCapability> {
        self.frontend.as_ref()
    }

    /// Return the persisted backend member, when present.
    pub fn backend(&self) -> Option<&BackendCapability> {
        self.backend.as_ref()
    }

    pub(in crate::extensions) fn is_empty(&self) -> bool {
        self.frontend.is_none() && self.backend.is_none()
    }
}

/// Capability metadata known before negotiation.
#[derive(Debug, Clone)]
pub(in crate::extensions) enum CapabilityExpectation {
    /// Every advertised capability member is known and must match.
    Exact(ExtensionCapabilities),
    /// Persisted frontend and backend members must match; other members remain negotiable.
    Persisted(PersistedExtensionCapabilities),
    /// Only the backend member was persisted and must match.
    Backend(BackendCapability),
}

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
    pub(super) capabilities: Option<CapabilityExpectation>,
    pub(super) allows_legacy_backend: bool,
}

impl ExpectedExtension {
    /// Expect only a stable extension identifier.
    pub fn identified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            discovered: None,
            capabilities: None,
            allows_legacy_backend: false,
        }
    }

    /// Require initialization metadata to agree with discovery metadata.
    pub fn discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: None,
            allows_legacy_backend: false,
        }
    }

    /// Preserve schema-v1 backend behavior for installed legacy metadata.
    pub(in crate::extensions) fn legacy_discovered(info: ExtensionInfo) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: None,
            allows_legacy_backend: true,
        }
    }

    /// Require initialization metadata and every capability to agree with discovery.
    pub fn discovered_with_capabilities(
        info: ExtensionInfo,
        capabilities: ExtensionCapabilities,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
        }
    }

    /// Require persisted frontend and backend metadata to agree with discovery.
    ///
    /// Capability members not represented by installed metadata remain
    /// negotiable and are still checked against the extension's declared types.
    pub fn discovered_with_persisted_capabilities(
        info: ExtensionInfo,
        capabilities: PersistedExtensionCapabilities,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Persisted(capabilities)),
            allows_legacy_backend: false,
        }
    }

    /// Require exact backend metadata while leaving unpersisted members negotiable.
    pub fn discovered_with_backend_capability(
        info: ExtensionInfo,
        backend: BackendCapability,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(CapabilityExpectation::Backend(backend)),
            allows_legacy_backend: false,
        }
    }

    pub(in crate::extensions) fn discovered_with_expectation(
        info: ExtensionInfo,
        capabilities: CapabilityExpectation,
    ) -> Self {
        Self {
            id: info.id.clone(),
            discovered: Some(info),
            capabilities: Some(capabilities),
            allows_legacy_backend: false,
        }
    }

    /// Return the stable extension identifier known before negotiation.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return exact identity metadata obtained during discovery, when available.
    pub fn extension_info(&self) -> Option<&ExtensionInfo> {
        self.discovered.as_ref()
    }

    /// Return complete discovery capabilities, when every member is locked.
    pub fn capabilities(&self) -> Option<&ExtensionCapabilities> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => Some(capabilities),
            Some(CapabilityExpectation::Persisted(_))
            | Some(CapabilityExpectation::Backend(_))
            | None => None,
        }
    }

    /// Return persisted installed capability members, when those members are locked.
    pub fn persisted_capabilities(&self) -> Option<&PersistedExtensionCapabilities> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Persisted(capabilities)) => Some(capabilities),
            Some(CapabilityExpectation::Exact(_))
            | Some(CapabilityExpectation::Backend(_))
            | None => None,
        }
    }

    /// Return the locked frontend capability, whether the lock is exact or persisted.
    pub fn frontend_capability(&self) -> Option<&FrontendCapability> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => capabilities.frontend.as_ref(),
            Some(CapabilityExpectation::Persisted(capabilities)) => capabilities.frontend(),
            Some(CapabilityExpectation::Backend(_)) | None => None,
        }
    }

    /// Return the locked backend capability, whether the lock is exact or partial.
    pub fn backend_capability(&self) -> Option<&BackendCapability> {
        match self.capabilities.as_ref() {
            Some(CapabilityExpectation::Exact(capabilities)) => capabilities.backend.as_ref(),
            Some(CapabilityExpectation::Persisted(capabilities)) => capabilities.backend(),
            Some(CapabilityExpectation::Backend(backend)) => Some(backend),
            None => None,
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
