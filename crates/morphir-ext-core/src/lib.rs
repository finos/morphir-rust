//! Morphir Extension Core crate
//!
//! Core types and protocols for Morphir extensions.

pub mod abi;
pub mod envelope;

// Re-export main types for convenience
pub use envelope::{Envelope, Header, EnvelopeError, encode_envelope, decode_envelope};
