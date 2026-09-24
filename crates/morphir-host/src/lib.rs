//! Portable Morphir extension host.
//!
//! The host negotiates the Morphir Extension Protocol (MEP) with a guest,
//! keeps the session state, and moves requests over a transport that the
//! caller supplies. The crate has no process or runtime code, so it compiles
//! for `wasm32-unknown-unknown`.

mod config;
mod envelope;
mod error;
mod negotiated;
mod send;

pub use config::HostConfig;
pub use envelope::{EnvelopeError, validate_envelope};
pub use error::{ChannelState, HostError};
pub use negotiated::Negotiated;
pub use send::MaybeSend;
