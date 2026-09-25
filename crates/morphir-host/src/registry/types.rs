//! The types a registry resolves to, and the source trait it resolves over.

use crate::{GuestConnection, HostError, MaybeSend};
use async_trait::async_trait;
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    NativeExtension,
};
use std::path::Path;
use std::sync::Arc;

/// The source that registered an extension provider.
///
/// Installed providers have higher selection precedence than built-ins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub enum CapabilityMetadataScope {
    /// The capability snapshot includes every member reported by the provider.
    Complete,
    /// Only populated frontend/backend members persisted by installed state are represented.
    ///
    /// A missing member is unknown, not proof that the provider omits that capability.
    PersistedFrontendBackend,
}

/// A provider the registry can resolve and connect to.
///
/// The registry reads [`GuestSource::info`] and [`GuestSource::capabilities`]
/// once, when the source registers, and resolves against that snapshot. A
/// source must return the same values for its whole life.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait GuestSource: MaybeSend + Sync {
    /// The provider's identity.
    fn info(&self) -> &ExtensionInfo;

    /// The provider's capability metadata.
    fn capabilities(&self) -> &ExtensionCapabilities;

    /// Where the provider was registered from.
    fn origin(&self) -> ProviderOrigin;

    /// Which capability members [`GuestSource::capabilities`] represents.
    fn capability_metadata_scope(&self) -> CapabilityMetadataScope;

    /// The mode this source uses under `policy`.
    fn invocation_mode(&self, policy: InvocationPolicy) -> InvocationMode;

    /// The built-in extension behind a direct or native protocol source.
    fn native(&self) -> Option<&NativeExtension> {
        None
    }

    /// A value that tells apart two builds registered under the same id and
    /// version, such as a digest of an installed extension's catalog record.
    ///
    /// `None`, the default, when the id and version already name one build.
    /// [`Resolved::fingerprint`] includes it, so a pool keyed by the
    /// fingerprint opens a new guest when the build changes.
    fn incarnation(&self) -> Option<&str> {
        None
    }

    /// Start the guest and return a checked, unopened connection.
    async fn connect(&self, workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError>;
}

/// The capability a provider was resolved for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Frontend,
    Backend,
}

/// Immutable metadata for one registered provider.
#[derive(Clone)]
pub struct ProviderMetadata {
    pub(super) source: Arc<dyn GuestSource>,
}

impl std::fmt::Debug for ProviderMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderMetadata")
            .field("info", self.info())
            .field("origin", &self.origin())
            .field(
                "capability_metadata_scope",
                &self.capability_metadata_scope(),
            )
            .field(
                "preferred_invocation_mode",
                &self.preferred_invocation_mode(),
            )
            .finish_non_exhaustive()
    }
}

impl ProviderMetadata {
    /// Return the provider's immutable SDK identity snapshot.
    pub fn info(&self) -> &ExtensionInfo {
        self.source.info()
    }

    /// Return the provider's immutable capability metadata.
    ///
    /// [`Self::capability_metadata_scope`] reports whether this is a complete
    /// native snapshot or only populated frontend/backend members persisted
    /// for an installed provider. Missing members in the latter scope are
    /// unknown rather than known to be absent.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        self.source.capabilities()
    }

    /// Return which capability members are represented by [`Self::capabilities`].
    pub fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.source.capability_metadata_scope()
    }

    /// Return where the provider was registered from.
    pub fn origin(&self) -> ProviderOrigin {
        self.source.origin()
    }

    /// Return the provider's default invocation mode, the mode it uses under
    /// [`InvocationPolicy::PreferDirect`].
    pub fn preferred_invocation_mode(&self) -> InvocationMode {
        self.source.invocation_mode(InvocationPolicy::PreferDirect)
    }
}

/// A provider proven to support one enabled frontend or backend capability.
///
/// The value remembers which capability it was resolved for:
/// [`Resolved::frontend`] is `Some` only for a frontend resolution and
/// [`Resolved::backend`] only for a backend resolution.
#[derive(Clone)]
pub struct Resolved {
    pub(super) source: Arc<dyn GuestSource>,
    pub(super) invocation_mode: InvocationMode,
    pub(super) role: Role,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Resolved");
        debug.field("info", self.info());
        match self.role {
            Role::Frontend => debug.field("frontend", &self.frontend()),
            Role::Backend => debug.field("backend", &self.backend()),
        };
        debug
            .field("origin", &self.origin())
            .field("invocation_mode", &self.invocation_mode)
            .finish_non_exhaustive()
    }
}

impl Resolved {
    /// Return the provider's immutable SDK identity snapshot.
    pub fn info(&self) -> &ExtensionInfo {
        self.source.info()
    }

    /// Return the provider's immutable capability metadata.
    ///
    /// [`Self::capability_metadata_scope`] distinguishes complete native
    /// metadata from persisted installed frontend/backend metadata, where a
    /// missing member is unknown rather than known to be absent.
    pub fn capabilities(&self) -> &ExtensionCapabilities {
        self.source.capabilities()
    }

    /// Return the enabled frontend capability used for resolution, when this
    /// is a frontend resolution.
    pub fn frontend(&self) -> Option<&FrontendCapability> {
        match self.role {
            Role::Frontend => self.capabilities().frontend.as_ref(),
            Role::Backend => None,
        }
    }

    /// Return the enabled backend capability used for resolution, when this
    /// is a backend resolution.
    pub fn backend(&self) -> Option<&BackendCapability> {
        match self.role {
            Role::Backend => self.capabilities().backend.as_ref(),
            Role::Frontend => None,
        }
    }

    /// Return which capability members are represented by [`Self::capabilities`].
    pub fn capability_metadata_scope(&self) -> CapabilityMetadataScope {
        self.source.capability_metadata_scope()
    }

    /// Return where the provider was registered from.
    pub fn origin(&self) -> ProviderOrigin {
        self.source.origin()
    }

    /// Return the selected invocation mode.
    pub fn invocation_mode(&self) -> InvocationMode {
        self.invocation_mode
    }

    /// Return the built-in extension whose typed handles the caller invokes
    /// directly.
    ///
    /// This is `Some` only under [`InvocationMode::NativeDirect`]. Under every
    /// other mode, [`InvocationMode::NativeMep`] included, it is `None`, so a
    /// caller that asked for [`InvocationPolicy::ProtocolOnly`] goes through
    /// [`Self::connect`] and gets MEP negotiation and result checks.
    pub fn native(&self) -> Option<&NativeExtension> {
        match self.invocation_mode {
            InvocationMode::NativeDirect => self.source.native(),
            _ => None,
        }
    }

    /// Whether this provider declares workspace discovery, and so can
    /// synthesize a project from an explicit source selection.
    ///
    /// A built-in's complete native metadata states the capability and the
    /// protocol versions it accepts. An installed provider persists only the
    /// capability kind its release declared; the protocol version is settled
    /// when a session negotiates, so the kind alone answers here.
    pub fn supports_workspace_discovery(&self) -> bool {
        match self.capability_metadata_scope() {
            CapabilityMetadataScope::Complete => self
                .capabilities()
                .workspace
                .as_ref()
                .is_some_and(|workspace| {
                    workspace.discover
                        && workspace
                            .protocol_versions
                            .iter()
                            .any(morphir_workspace::speaks_workspace_discovery_protocol)
                }),
            CapabilityMetadataScope::PersistedFrontendBackend => {
                self.info().types.contains(&ExtensionType::Workspace)
            }
        }
    }

    /// A key that changes when the provider's identity, origin, mode, or
    /// build changes: `"{id}@{version}:{origin:?}:{mode:?}"`, followed by
    /// `":{incarnation}"` when [`GuestSource::incarnation`] is `Some`.
    ///
    /// The incarnation makes a reinstall under the same id and version, with
    /// another artifact, args, or claims, a new key.
    pub fn fingerprint(&self) -> String {
        let info = self.info();
        let identity = format!(
            "{}@{}:{:?}:{:?}",
            info.id,
            info.version,
            self.origin(),
            self.invocation_mode
        );
        match self.source.incarnation() {
            Some(incarnation) => format!("{identity}:{incarnation}"),
            None => identity,
        }
    }

    /// Start the guest and return a checked, unopened connection.
    ///
    /// `workspace` is the directory the guest works in. A source that runs
    /// in the host process may ignore it.
    pub async fn connect(&self, workspace: &Path) -> Result<Box<dyn GuestConnection>, HostError> {
        self.source.connect(workspace).await
    }
}
