//! Extension system for morphir-daemon
//!
//! This module provides the Extism-based plugin runtime for loading
//! and executing Morphir extensions.

pub mod activation;
pub mod connected;
pub mod container;
pub mod host_functions;
pub mod loader;
pub mod process;
pub mod protocol;
pub mod registry;
pub mod session;
pub mod virtual_paths;

pub use activation::{BoxedMepTransport, activate_transport};
pub use connected::{ConnectedDaemonSession, ConnectedDaemonTransport, DaemonConnection};
pub use container::ExtensionContainer;
pub use loader::ExtensionLoader;
pub use process::{ProcessLaunch, SpawnedProcessSession, SpawnedProcessTransport};
pub use protocol::{ExtensionRequest, ExtensionResponse};
pub use registry::{
    CapabilityMetadataScope, ExtensionRegistry, InvocationMode, InvocationPolicy, ProviderMetadata,
    ProviderOrigin, ResolvedBackend, ResolvedFrontend,
};
pub use session::{
    ExpectedExtension, ExtensionSession, ExtensionSessionState, ExtismSession, FailedSession,
    Indeterminate, InvokeOutcome, Loaded, MepTransport, NativeMepSession, NativeMepTransport,
    NegotiatedSession, PersistedExtensionCapabilities, Ready, Session, Stopped, TransportError,
    TransportState,
};
