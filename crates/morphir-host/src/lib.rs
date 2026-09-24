//! Portable Morphir extension host.
//!
//! The host negotiates the Morphir Extension Protocol (MEP) with a guest,
//! keeps the session state, and moves requests over a transport that the
//! caller supplies. The crate has no process or runtime code, so it compiles
//! for `wasm32-unknown-unknown`.

mod channel;
mod config;
mod connection;
mod envelope;
mod error;
mod expected;
mod jsonrpc;
mod negotiated;
/// The install probe: describe a guest before trusting its claims.
mod probe;
mod registry;
mod send;
mod session;
mod session_core;
/// Test doubles for clients of this crate. Enabled by the `testing` feature.
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use channel::{Channel, ChannelError, Outgoing};
pub use config::HostConfig;
pub use connection::{CallError, GuestConnection};
pub use envelope::{EnvelopeError, validate_envelope};
pub use error::{ChannelCause, ChannelState, HostError};
pub use expected::{
    CapabilityExpectation, ExpectedChecks, ExpectedExtension, PersistedExtensionCapabilities,
    validate_negotiation,
};
pub use jsonrpc::JsonRpcConnection;
pub use negotiated::Negotiated;
pub use probe::{Description, DescriptionSource, describe, describe_with};
pub use registry::{
    CapabilityMetadataScope, GuestSource, InvocationMode, InvocationPolicy, ProviderMetadata,
    ProviderOrigin, Registry, Resolved,
};
pub use send::MaybeSend;
pub use session::{Session, call_once, compile_once, generate_once};
pub use session_core::{Action, BasicChecks, Event, SessionChecks, SessionCore};
