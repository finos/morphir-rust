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
mod jsonrpc;
mod negotiated;
mod send;
mod session_core;
pub mod testing;

pub use channel::{Channel, ChannelError, Outgoing};
pub use config::HostConfig;
pub use connection::{CallError, GuestConnection};
pub use envelope::{EnvelopeError, validate_envelope};
pub use error::{ChannelState, HostError};
pub use jsonrpc::JsonRpcConnection;
pub use negotiated::Negotiated;
pub use send::MaybeSend;
pub use session_core::{Action, BasicChecks, Event, SessionChecks, SessionCore};
