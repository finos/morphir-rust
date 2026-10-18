//! Portable Morphir extension host.
//!
//! The host negotiates the Morphir Extension Protocol (MEP) with a guest,
//! keeps the session state, and moves requests over a transport that the
//! caller supplies. The crate has no process or runtime code, so it compiles
//! for `wasm32-unknown-unknown`.

//! Rich model transport is opt-in through [`HostConfig::with_model_transport`].
//! After initialization, [`Session::compile_model`] and [`Session::generate_model`]
//! use the frozen bilateral selection. Generic [`Session::call`] applies the same
//! admission checks. See `morphir_extension_sdk::model` for structural admission
//! and the default refusal of required remote guarantees.
//!
//! ```
//! use morphir_host::HostConfig;
//! use morphir_extension_sdk::model::*;
//! use morphir_extension_sdk::protocol::PeerInfo;
//! let offer = ModelTransportSupport {
//!     tuples: vec![ModelTransportTuple {
//!         operation: ModelOperation::GenerateModel,
//!         profile: "morphir-model".into(),
//!         codec: ModelCodec::IonBinary,
//!         envelope_version: MODEL_ENVELOPE_VERSION.into(),
//!         ir_version: "4.0.0".into(),
//!     }],
//!     limits: ModelTransportLimits {
//!         max_encoded_bytes: 1_000_000,
//!         max_decoded_bytes: 750_000,
//!         max_depth: 128,
//!         max_nodes: 100_000,
//!     },
//!     guarantees: vec![],
//! };
//! let peer = PeerInfo { kind: Default::default(), name: "example".into(), version: "1".into() };
//! let config = HostConfig::new(peer).with_model_transport(offer);
//! assert_eq!(config.protocol_versions()[0], MODEL_MEP_VERSION);
//! ```

mod channel;
mod config;
mod connection;
mod envelope;
mod error;
mod expected;
mod jsonrpc;
mod negotiated;
/// Warm guest reuse, one session per key.
mod pool;
/// The install probe: describe a guest before trusting its claims.
mod probe;
mod registry;
mod response_policy;
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
pub use pool::Pool;
pub use probe::{Description, DescriptionSource, describe, describe_with};
pub use registry::{
    CapabilityMetadataScope, GuestSource, InvocationMode, InvocationPolicy, ProviderMetadata,
    ProviderOrigin, Registry, Resolved,
};
pub use response_policy::{GuestResponsePolicy, ResponseByteLimit};
pub use send::MaybeSend;
pub use session::{Session, call_once, compile_once, generate_once};
pub use session_core::{Action, BasicChecks, Event, SessionChecks, SessionCore};
