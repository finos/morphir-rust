//! Validated Morphir Extension Protocol sessions.
//!
//! Wire messages are untrusted data. A [`Session`] validates JSON-RPC envelopes,
//! negotiation, capabilities, and lifecycle transitions once for every transport.

mod compatibility;
mod controller;
mod extism;
mod transport;
mod validation;

pub use compatibility::{ExtensionSession, ExtensionSessionState};
pub use controller::{
    FailedSession, Indeterminate, InvokeOutcome, Loaded, NegotiatedSession, Ready, Session, Stopped,
};
pub use extism::{ExtismSession, ExtismTransport};
pub(in crate::extensions) use transport::CapabilityExpectation;
pub use transport::{ExpectedExtension, MepTransport, TransportError, TransportState};
pub(in crate::extensions) use validation::{validate_method_result_async, validate_negotiation};

#[cfg(test)]
mod tests;
