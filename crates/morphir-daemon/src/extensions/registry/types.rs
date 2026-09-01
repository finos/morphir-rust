use crate::extensions::{Loaded, NativeMepSession, NativeMepTransport, Session};
use morphir_core::format_version::ReleaseTriplet;
use morphir_distribution::{ArtifactRuntime, InstalledExtensionSnapshot};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, FrontendCapability, NativeBackend,
    NativeExtension, NativeFrontend,
};
use std::sync::Arc;

/// The source that registered an extension provider.
///
/// Installed providers have higher selection precedence than built-ins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderOrigin {
    /// A provider linked into the host process.
    Builtin,
    /// A provider selected from the installed extension catalog.
    Installed,
}

/// The caller's transport preference for native providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationPolicy {
    /// Invoke native typed handles directly when available.
    PreferDirect,
    /// Use the Morphir Extension Protocol for every provider.
    ProtocolOnly,
}

/// The transport selected for one resolved provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    /// Invoke an in-process typed native handle.
    NativeDirect,
    /// Invoke an in-process provider through MEP.
    NativeMep,
    /// Invoke an installed child process through MEP.
    ProcessMep,
    /// Invoke an installed WebAssembly module through MEP.
    WasmMep,
}

/// How much of a provider's capability metadata the registry knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMetadataScope {
    /// The capability snapshot includes every member reported by the provider.
    Complete,
    /// Only populated frontend/backend members persisted by installed state are represented.
    ///
    /// A missing member is unknown, not proof that the provider omits that capability.
    PersistedFrontendBackend,
}

pub(super) enum ProviderRuntime {
    Native(NativeExtension),
    Installed(InstalledExtensionSnapshot),
}

impl ProviderRuntime {
    pub(super) fn preferred_invocation_mode(&self) -> InvocationMode {
        match self {
            Self::Native(_) => InvocationMode::NativeDirect,
            Self::Installed(snapshot) => match snapshot.installed().runtime() {
                ArtifactRuntime::Process => InvocationMode::ProcessMep,
                ArtifactRuntime::Wasm => InvocationMode::WasmMep,
            },
        }
    }

    pub(super) fn invocation_mode(&self, policy: InvocationPolicy) -> InvocationMode {
        match (self, policy) {
            (Self::Native(_), InvocationPolicy::PreferDirect) => InvocationMode::NativeDirect,
            (Self::Native(_), InvocationPolicy::ProtocolOnly) => InvocationMode::NativeMep,
            (Self::Installed(_), _) => self.preferred_invocation_mode(),
        }
    }
}

pub(super) struct RegisteredFrontend {
    pub(super) capability: FrontendCapability,
    pub(super) releases: Vec<ReleaseTriplet>,
}

pub(super) struct RegisteredBackend {
    pub(super) capability: BackendCapability,
    pub(super) releases: Vec<ReleaseTriplet>,
}

pub(super) struct RegisteredProvider {
    pub(super) info: ExtensionInfo,
    pub(super) capabilities: ExtensionCapabilities,
    pub(super) origin: ProviderOrigin,
    pub(super) capability_metadata_scope: CapabilityMetadataScope,
    pub(super) runtime: ProviderRuntime,
    pub(super) frontend: Option<RegisteredFrontend>,
    pub(super) backend: Option<RegisteredBackend>,
}

/// Immutable metadata for one registered provider.
#[derive(Clone)]
pub struct ProviderMetadata {
    pub(super) provider: Arc<RegisteredProvider>,
}

impl std::fmt::Debug for ProviderMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderMetadata")
            .field("info", &self.provider.info)
            .field("origin", &self.provider.origin)
            .field(
                "capability_metadata_scope",
                &self.provider.capability_metadata_scope,
            )
            .field(
                "preferred_invocation_mode",
                &self.provider.runtime.preferred_invocation_mode(),
            )
            .finish_non_exhaustive()
    }
}

impl ProviderMetadata {
    /// Return the provider's immutable SDK identity snapshot.
    pub fn info(&self) -> &ExtensionInfo {
        &self.provider.info
    }

    /// Return the provider's immutable capability metadata.
    ///
    /// [`Self::capability_metadata_scope`] reports whether this is a complete
    /// native snapshot or only populated frontend/backend members persisted
    /// for an installed provider. Missing members in the latter scope are
    /// unknown rather than known to be absent.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.provider.capabilities
    }

    /// Return which capability members are represented by [`Self::capabilities`].
    pub fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.provider.capability_metadata_scope
    }

    /// Return where the provider was registered from.
    pub fn origin(&self) -> ProviderOrigin {
        self.provider.origin
    }

    /// Return the provider's default invocation mode.
    pub fn preferred_invocation_mode(&self) -> InvocationMode {
        self.provider.runtime.preferred_invocation_mode()
    }
}

/// A provider proven to support one enabled frontend capability.
#[derive(Clone)]
pub struct ResolvedFrontend {
    pub(super) provider: Arc<RegisteredProvider>,
    pub(super) invocation_mode: InvocationMode,
}

impl std::fmt::Debug for ResolvedFrontend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedFrontend")
            .field("info", &self.provider.info)
            .field("capability", self.capability())
            .field("origin", &self.provider.origin)
            .field("invocation_mode", &self.invocation_mode)
            .finish_non_exhaustive()
    }
}

impl ResolvedFrontend {
    /// Return the provider's immutable SDK identity snapshot.
    pub fn info(&self) -> &ExtensionInfo {
        &self.provider.info
    }

    /// Return the enabled frontend capability used for resolution.
    pub fn capability(&self) -> &FrontendCapability {
        &self
            .provider
            .frontend
            .as_ref()
            .expect("resolved frontend retains normalized frontend metadata")
            .capability
    }

    /// Return the provider's immutable capability metadata.
    ///
    /// [`Self::capability_metadata_scope`] distinguishes complete native
    /// metadata from persisted installed frontend/backend metadata, where a
    /// missing member is unknown rather than known to be absent.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.provider.capabilities
    }

    /// Return which capability members are represented by [`Self::capabilities`].
    pub fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.provider.capability_metadata_scope
    }

    /// Return where the provider was registered from.
    pub fn origin(&self) -> ProviderOrigin {
        self.provider.origin
    }

    /// Return the selected invocation mode.
    pub fn invocation_mode(&self) -> InvocationMode {
        self.invocation_mode
    }

    /// Return the native provider only when direct invocation was selected.
    pub fn native_extension(&self) -> Option<&NativeExtension> {
        if self.invocation_mode != InvocationMode::NativeDirect {
            return None;
        }
        match &self.provider.runtime {
            ProviderRuntime::Native(native) => Some(native),
            ProviderRuntime::Installed(_) => None,
        }
    }

    /// Return the direct native frontend handle when this is a built-in.
    pub fn native_frontend(&self) -> Option<&dyn NativeFrontend> {
        self.native_extension().and_then(NativeExtension::frontend)
    }

    /// Create a loaded native protocol session only when protocol invocation was selected.
    pub fn native_mep_session(&self) -> Option<Session<NativeMepTransport, Loaded>> {
        match (&self.provider.runtime, self.invocation_mode) {
            (ProviderRuntime::Native(native), InvocationMode::NativeMep) => {
                Some(NativeMepSession::connect(native.clone()))
            }
            _ => None,
        }
    }

    /// Return the exact installed snapshot when this is an installed provider.
    pub fn installed_snapshot(&self) -> Option<&InstalledExtensionSnapshot> {
        match &self.provider.runtime {
            ProviderRuntime::Native(_) => None,
            ProviderRuntime::Installed(snapshot) => Some(snapshot),
        }
    }
}

/// A provider proven to support one enabled backend capability.
#[derive(Clone)]
pub struct ResolvedBackend {
    pub(super) provider: Arc<RegisteredProvider>,
    pub(super) invocation_mode: InvocationMode,
}

impl std::fmt::Debug for ResolvedBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedBackend")
            .field("info", &self.provider.info)
            .field("capability", self.capability())
            .field("origin", &self.provider.origin)
            .field("invocation_mode", &self.invocation_mode)
            .finish_non_exhaustive()
    }
}

impl ResolvedBackend {
    /// Return the provider's immutable SDK identity snapshot.
    pub fn info(&self) -> &ExtensionInfo {
        &self.provider.info
    }

    /// Return the enabled backend capability used for resolution.
    pub fn capability(&self) -> &BackendCapability {
        &self
            .provider
            .backend
            .as_ref()
            .expect("resolved backend retains normalized backend metadata")
            .capability
    }

    /// Return the provider's immutable capability metadata.
    ///
    /// [`Self::capability_metadata_scope`] distinguishes complete native
    /// metadata from persisted installed frontend/backend metadata, where a
    /// missing member is unknown rather than known to be absent.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        &self.provider.capabilities
    }

    /// Return which capability members are represented by [`Self::capabilities`].
    pub fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.provider.capability_metadata_scope
    }

    /// Return where the provider was registered from.
    pub fn origin(&self) -> ProviderOrigin {
        self.provider.origin
    }

    /// Return the selected invocation mode.
    pub fn invocation_mode(&self) -> InvocationMode {
        self.invocation_mode
    }

    /// Return the native provider only when direct invocation was selected.
    pub fn native_extension(&self) -> Option<&NativeExtension> {
        if self.invocation_mode != InvocationMode::NativeDirect {
            return None;
        }
        match &self.provider.runtime {
            ProviderRuntime::Native(native) => Some(native),
            ProviderRuntime::Installed(_) => None,
        }
    }

    /// Return the direct native backend handle when this is a built-in.
    pub fn native_backend(&self) -> Option<&dyn NativeBackend> {
        self.native_extension().and_then(NativeExtension::backend)
    }

    /// Create a loaded native protocol session only when protocol invocation was selected.
    pub fn native_mep_session(&self) -> Option<Session<NativeMepTransport, Loaded>> {
        match (&self.provider.runtime, self.invocation_mode) {
            (ProviderRuntime::Native(native), InvocationMode::NativeMep) => {
                Some(NativeMepSession::connect(native.clone()))
            }
            _ => None,
        }
    }

    /// Return the exact installed snapshot when this is an installed provider.
    pub fn installed_snapshot(&self) -> Option<&InstalledExtensionSnapshot> {
        match &self.provider.runtime {
            ProviderRuntime::Native(_) => None,
            ProviderRuntime::Installed(snapshot) => Some(snapshot),
        }
    }
}
