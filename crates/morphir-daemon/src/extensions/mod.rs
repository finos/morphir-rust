//! Extension system for morphir-daemon
//!
//! This module provides the Extism-based plugin runtime for loading
//! and executing Morphir extensions.

pub mod connected;
pub mod container;
pub mod host_functions;
pub mod loader;
pub mod process;
pub mod protocol;
pub mod registry;
pub mod session;
pub mod virtual_paths;

pub use connected::{ConnectedDaemonSession, DaemonConnection};
pub use container::ExtensionContainer;
pub use loader::ExtensionLoader;
pub use process::{ProcessLaunch, SpawnedProcessSession};
pub use protocol::{ExtensionRequest, ExtensionResponse};
pub use registry::ExtensionRegistry;
pub use session::{
    ExpectedExtension, ExtensionSession, ExtensionSessionState, ExtismSession, FailedSession,
    Indeterminate, InvokeOutcome, Loaded, MepTransport, NegotiatedSession, Ready, Session, Stopped,
    TransportError, TransportState,
};
